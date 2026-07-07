//! Tunnel transport configuration (Sprint 5.5.1).
//!
//! Persists the *non-secret* part of the tunnel config (mode +
//! public hostname) in `nexus_config.json` inside the app data
//! dir, and routes the *secret* part (Cloudflare tunnel token)
//! through `keyring`, the OS-native encrypted credential store.
//!
//! ## Why two stores?
//!
//! - The **hostname** is public (anyone who connects to the
//!   tunnel needs to know it). It belongs in a regular JSON
//!   file the user can read/edit.
//! - The **token** is a credential that authenticates the
//!   sender to Cloudflare. If leaked, an attacker could
//!   impersonate the tunnel. It belongs in keyring (Keychain
//!   on macOS, Credential Manager on Windows, Secret Service
//!   on Linux), encrypted at rest, accessible only to the
//!   process that wrote it.
//!
//! ## Why a trait?
//!
//! The OS keyring is process-specific and platform-specific.
//! Testing against a real keyring would (a) require a
//! graphical session on macOS, (b) leave credentials behind
//! that the test can't clean up, and (c) make CI brittle.
//! `TokenStore` is a one-method trait that the production
//! `KeyringTokenStore` implements, and `MockTokenStore`
//! implements the same trait for tests. The two are
//! interchangeable from the call site.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Which transport to use for the P2P tunnel.
///
/// `Quick` is the default (anonymous `--url` Quick Tunnel,
/// rate-limited by Cloudflare after 3-5 connections). `Named`
/// uses a per-user Cloudflare account + DNS (no rate limit).
/// `Direct` is reserved for Sprint 5.5.2 and not yet
/// implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportMode {
    /// Anonymous Quick Tunnel via `cloudflared --url`. No
    /// account needed but Cloudflare throttles the API after
    /// a handful of requests (Error 1015).
    Quick,
    /// Per-account Named Tunnel via `cloudflared tunnel run
    /// --token X`. Requires a Cloudflare account, a tunnel
    /// configured in the dashboard, and a DNS record
    /// pointing to it. No rate limit.
    Named,
    /// Direct IP mode (Sprint 5.5.2). mDNS discovery + port
    /// forwarding + raw TCP. No Cloudflare involvement.
    /// Currently returns `unimplemented!()`.
    Direct,
}

impl Default for TransportMode {
    fn default() -> Self {
        TransportMode::Quick
    }
}

/// Non-secret part of the tunnel config. Persisted as JSON in
/// `nexus_config.json` inside the app data dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    pub mode: TransportMode,
    /// Public hostname for `Named` mode (e.g.,
    /// `p2p.example.com`). Stripped of scheme and path before
    /// storage. `None` for `Quick` and `Direct` modes.
    pub hostname: Option<String>,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        TunnelConfig {
            mode: TransportMode::Quick,
            hostname: None,
        }
    }
}

const TUNNEL_CONFIG_FILE: &str = "nexus_config.json";
const KEYRING_SERVICE: &str = "com.nexus-rar.tunnel";
const KEYRING_USER: &str = "tunnel-token";

// ============================================================================
//  TokenStore trait + production impl
// ============================================================================

/// Abstraction over the secret-storage backend. Production uses
/// the OS keyring; tests use an in-memory mock.
pub trait TokenStore: Send + Sync {
    fn get_token(&self) -> Result<Option<String>, String>;
    fn set_token(&self, token: &str) -> Result<(), String>;
    fn clear_token(&self) -> Result<(), String>;
}

/// Production TokenStore backed by the OS keyring.
pub struct KeyringTokenStore {
    service: String,
    user: String,
}

impl KeyringTokenStore {
    pub fn new(service: impl Into<String>, user: impl Into<String>) -> Self {
        KeyringTokenStore {
            service: service.into(),
            user: user.into(),
        }
    }
}

impl TokenStore for KeyringTokenStore {
    fn get_token(&self) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(&self.service, &self.user)
            .map_err(|e| format!("keyring entry: {}", e))?;
        match entry.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keyring get: {}", e)),
        }
    }
    fn set_token(&self, token: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(&self.service, &self.user)
            .map_err(|e| format!("keyring entry: {}", e))?;
        entry
            .set_password(token)
            .map_err(|e| format!("keyring set: {}", e))
    }
    fn clear_token(&self) -> Result<(), String> {
        let entry = keyring::Entry::new(&self.service, &self.user)
            .map_err(|e| format!("keyring entry: {}", e))?;
        match entry.delete_password() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keyring delete: {}", e)),
        }
    }
}

/// Default `TokenStore` for production: OS keyring.
pub fn default_token_store() -> KeyringTokenStore {
    KeyringTokenStore::new(KEYRING_SERVICE, KEYRING_USER)
}

// ============================================================================
//  MockTokenStore — for tests
// ============================================================================

/// In-memory TokenStore for tests. NOT for production — the
/// token is plaintext in RAM and lost on process exit.
pub struct MockTokenStore {
    inner: Arc<Mutex<Option<String>>>,
}

impl MockTokenStore {
    pub fn new() -> Self {
        MockTokenStore {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    /// Pre-populate the mock with a token. Useful for tests
    /// that need to simulate a "token already saved" state.
    pub fn with_token(token: impl Into<String>) -> Self {
        MockTokenStore {
            inner: Arc::new(Mutex::new(Some(token.into()))),
        }
    }
}

impl Default for MockTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for MockTokenStore {
    fn get_token(&self) -> Result<Option<String>, String> {
        Ok(self.inner.lock().unwrap().clone())
    }
    fn set_token(&self, token: &str) -> Result<(), String> {
        *self.inner.lock().unwrap() = Some(token.to_string());
        Ok(())
    }
    fn clear_token(&self) -> Result<(), String> {
        *self.inner.lock().unwrap() = None;
        Ok(())
    }
}

// ============================================================================
//  Config persistence (non-secret part only)
// ============================================================================

/// Load the non-secret part of the tunnel config from disk.
/// Returns defaults if the file doesn't exist.
pub fn load_tunnel_config(app_data_dir: &Path) -> Result<TunnelConfig, String> {
    let path = app_data_dir.join(TUNNEL_CONFIG_FILE);
    if !path.exists() {
        return Ok(TunnelConfig::default());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("read config: {}", e))?;
    let cfg: TunnelConfig = serde_json::from_slice(&bytes)
        .map_err(|e| format!("parse config: {}", e))?;
    Ok(cfg)
}

/// Persist the non-secret part of the tunnel config to disk.
/// The token (if any) must be persisted via
/// `TokenStore::set_token`.
pub fn save_tunnel_config(
    app_data_dir: &Path,
    cfg: &TunnelConfig,
) -> Result<(), String> {
    std::fs::create_dir_all(app_data_dir).map_err(|e| format!("mkdir: {}", e))?;
    let path = app_data_dir.join(TUNNEL_CONFIG_FILE);
    let json = serde_json::to_vec_pretty(cfg).map_err(|e| format!("serialize: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("write config: {}", e))
}

// ============================================================================
//  Validation
// ============================================================================

/// Validate that `token` is a plausible Cloudflare tunnel
/// token. Cloudflare tokens are base64-encoded JSON, typically
/// 200+ chars. We accept anything that's:
///   - non-empty
///   - at least 100 chars
///   - all base64 chars (alphanumeric + `+/=`)
pub fn validate_token(token: &str) -> Result<(), String> {
    if token.is_empty() {
        return Err("token is empty".to_string());
    }
    if token.len() < 100 {
        return Err(format!(
            "token too short ({} chars, expected ≥100)",
            token.len()
        ));
    }
    if !token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        return Err("token contains invalid characters (must be base64)".to_string());
    }
    Ok(())
}

/// Validate that `hostname` is a plausible FQDN. Rejects IPs,
/// `localhost`, empty strings. Allows hostnames with or
/// without scheme. Returns the canonical `https://<host>`
/// form.
pub fn validate_hostname(hostname: &str) -> Result<String, String> {
    let trimmed = hostname.trim();
    if trimmed.is_empty() {
        return Err("hostname is empty".to_string());
    }
    // Strip optional scheme.
    let host = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    // Strip optional path.
    let host = host.split('/').next().unwrap_or(host);
    if host.is_empty() {
        return Err("hostname is empty after stripping scheme/path".to_string());
    }
    // Reject private/local addresses.
    if host == "localhost"
        || host.starts_with("127.")
        || host.starts_with("192.168.")
        || host.starts_with("10.")
    {
        return Err(format!("hostname is a local address: {}", host));
    }
    // Must contain a dot (FQDN).
    if !host.contains('.') {
        return Err(format!("hostname is not a FQDN (no dot): {}", host));
    }
    // Only valid DNS chars.
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(format!(
            "hostname contains invalid characters: {}",
            host
        ));
    }
    Ok(format!("https://{}", host))
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- MockTokenStore -------------------------------------------------

    #[test]
    fn mock_token_store_starts_empty() {
        let s = MockTokenStore::new();
        assert_eq!(s.get_token().unwrap(), None);
    }

    #[test]
    fn mock_token_store_roundtrips_set_get_clear() {
        let s = MockTokenStore::new();
        s.set_token("secret123").unwrap();
        assert_eq!(s.get_token().unwrap().as_deref(), Some("secret123"));
        s.clear_token().unwrap();
        assert_eq!(s.get_token().unwrap(), None);
    }

    #[test]
    fn mock_token_store_with_token_constructor() {
        let s = MockTokenStore::with_token("preset");
        assert_eq!(s.get_token().unwrap().as_deref(), Some("preset"));
    }

    #[test]
    fn mock_token_store_overwrites() {
        let s = MockTokenStore::with_token("first");
        s.set_token("second").unwrap();
        assert_eq!(s.get_token().unwrap().as_deref(), Some("second"));
    }

    #[test]
    fn mock_token_store_default_matches_new() {
        let s = MockTokenStore::default();
        assert_eq!(s.get_token().unwrap(), None);
    }

    // --- validate_token -------------------------------------------------

    #[test]
    fn validate_token_rejects_empty() {
        assert!(validate_token("").is_err());
    }

    #[test]
    fn validate_token_rejects_short() {
        assert!(validate_token("short").is_err());
        assert!(validate_token(&"x".repeat(99)).is_err());
    }

    #[test]
    fn validate_token_rejects_invalid_chars() {
        let bad = format!("{} with spaces", "x".repeat(100));
        assert!(validate_token(&bad).is_err());
        let bad = format!("{}special@chars", "x".repeat(100));
        assert!(validate_token(&bad).is_err());
    }

    #[test]
    fn validate_token_accepts_valid_base64() {
        let valid = "a".repeat(200);
        assert!(validate_token(&valid).is_ok());
        let valid_with_slash = format!("{}/{}", "a".repeat(150), "b".repeat(49));
        assert!(validate_token(&valid_with_slash).is_ok());
        let valid_with_eq = format!("{}==", "a".repeat(150));
        assert!(validate_token(&valid_with_eq).is_ok());
    }

    // --- validate_hostname -----------------------------------------------

    #[test]
    fn validate_hostname_rejects_empty() {
        assert!(validate_hostname("").is_err());
        assert!(validate_hostname("   ").is_err());
    }

    #[test]
    fn validate_hostname_rejects_local() {
        assert!(validate_hostname("localhost").is_err());
        assert!(validate_hostname("192.168.1.1").is_err());
        assert!(validate_hostname("10.0.0.1").is_err());
        assert!(validate_hostname("127.0.0.1").is_err());
    }

    #[test]
    fn validate_hostname_rejects_no_dot() {
        assert!(validate_hostname("nodot").is_err());
        assert!(validate_hostname("localhost").is_err());
    }

    #[test]
    fn validate_hostname_rejects_invalid_chars() {
        assert!(validate_hostname("has space.com").is_err());
        assert!(validate_hostname("with@symbol.com").is_err());
    }

    #[test]
    fn validate_hostname_strips_scheme() {
        assert_eq!(
            validate_hostname("https://p2p.example.com").unwrap(),
            "https://p2p.example.com"
        );
        assert_eq!(
            validate_hostname("http://p2p.example.com").unwrap(),
            "https://p2p.example.com"
        );
        assert_eq!(
            validate_hostname("p2p.example.com").unwrap(),
            "https://p2p.example.com"
        );
    }

    #[test]
    fn validate_hostname_strips_path() {
        assert_eq!(
            validate_hostname("p2p.example.com/some/path").unwrap(),
            "https://p2p.example.com"
        );
    }

    // --- Default + enum ------------------------------------------------

    #[test]
    fn default_mode_is_quick() {
        assert_eq!(TunnelConfig::default().mode, TransportMode::Quick);
        assert_eq!(TransportMode::default(), TransportMode::Quick);
    }

    #[test]
    fn transport_mode_serializes_as_string() {
        // Confirms the wire format is human-readable (e.g.
        // "Quick" in nexus_config.json, not a magic number).
        let json = serde_json::to_string(&TransportMode::Named).unwrap();
        assert_eq!(json, "\"Named\"");
        let round: TransportMode = serde_json::from_str("\"Direct\"").unwrap();
        assert_eq!(round, TransportMode::Direct);
    }
}
