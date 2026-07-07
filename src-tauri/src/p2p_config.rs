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
use sha2::{Digest, Sha256};
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
//  mDNS service constants (Sprint 5.5.2 Direct Mode)
// ============================================================================

/// mDNS service type for Direct Mode. Browsers filter by this
/// (`_nexus-share._tcp.local`) to avoid noise from other mDNS
/// services on the LAN.
pub const SERVICE_TYPE: &str = "_nexus-share._tcp.local";

/// Wire prefix for v2 tokens. v1 uses `nx:1:` + base64url JSON
/// (Quick/Named Tunnel URL). v2 uses `nx:2:` + a colon-delimited
/// payload (Direct Mode mDNS coordinates). The receiver reads
/// the prefix to decide which decode path to take.
pub const TOKEN_PREFIX_V2: &str = "nx:2:";

/// Length (in hex chars) of the service-name hash. 6 hex chars
/// = 24 bits = ~16M unique codes — collision probability for
/// one sender session is effectively zero.
pub const SERVICE_HASH_LEN: usize = 6;

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
//  v2 token + service-name derivation (Sprint 5.5.2 Direct Mode)
// ============================================================================
//
// v1 tokens (`nx:1:<base64url-json>`) carry a Cloudflare URL.
// v2 tokens (`nx:2:direct:<service_hash>:<code>`) carry mDNS
// coordinates — no URL, no Cloudflare, no public IP. The
// receiver browses mDNS for the service name derived from the
// hash and gets the sender's IP+port via the SRV record.
//
// Design choice (from the pre-flight discussion):
//   - Service hash is derived from the 4-word code itself
//     (no separate random string for the user to type).
//   - Hash is `SHA-256(code)` truncated to 6 hex chars
//     (24 bits — collision-free for one sender session).
//   - Token format is plain colon-delimited text, NOT base64.
//     Easier for humans to eyeball, easier to grep in logs.

/// Derive the short service-name from a 4-word code. Returns
/// `nx-<6hexchars>`, suitable for use as an mDNS service name
/// (`nx-abc123._nexus-share._tcp.local`).
pub fn derive_service_short_name(code: &str) -> String {
    let mut h = Sha256::new();
    h.update(code.as_bytes());
    let hash = h.finalize();
    // Take the first 3 bytes = 6 hex chars.
    let hex: String = hash[..3]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    format!("nx-{}", hex)
}

/// Derive the full mDNS service name (with type suffix). The
/// receiver uses this string to filter browse results.
pub fn derive_mdns_service_name(code: &str) -> String {
    format!(
        "{}.{}",
        derive_service_short_name(code),
        SERVICE_TYPE
    )
}

/// v2 token, parsed. `service_hash` is the 6-hex-char hash and
/// `code` is the 4-word passphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2pTokenV2 {
    pub service_hash: String,
    pub code: String,
}

/// Format a v2 token from its parts. Pure constructor — no
/// validation. Use `parse_v2_token` if you need validation.
pub fn format_v2_token(service_hash: &str, code: &str) -> String {
    format!("{}direct:{}:{}", TOKEN_PREFIX_V2, service_hash, code)
}

/// Parse a v2 token string. Returns the parsed parts on success
/// or a human-readable error on failure.
pub fn parse_v2_token(s: &str) -> Result<P2pTokenV2, String> {
    let body = s
        .strip_prefix(TOKEN_PREFIX_V2)
        .ok_or_else(|| {
            format!(
                "P2P v2 token must start with '{}', got: {}",
                TOKEN_PREFIX_V2,
                &s.chars().take(8).collect::<String>()
            )
        })?;
    // nx:2:direct:<hash>:<code>
    // The code can contain ':' in theory (4-word codes don't
    // but defense in depth), so we splitn(3, ':') after the
    // prefix and treat the last segment as the full code.
    let parts: Vec<&str> = body.splitn(3, ':').collect();
    if parts.len() != 3 {
        return Err(format!(
            "P2P v2 token must have format 'nx:2:<mode>:<hash>:<code>' (got {} parts)",
            parts.len()
        ));
    }
    let mode = parts[0];
    if mode != "direct" {
        return Err(format!(
            "P2P v2 token mode must be 'direct' (got '{}')",
            mode
        ));
    }
    let service_hash = parts[1];
    if service_hash.len() != SERVICE_HASH_LEN
        || !service_hash.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(format!(
            "P2P v2 service_hash must be {} hex chars (got '{}')",
            SERVICE_HASH_LEN, service_hash
        ));
    }
    let code = parts[2];
    validate_v2_code(code)?;
    Ok(P2pTokenV2 {
        service_hash: service_hash.to_string(),
        code: code.to_string(),
    })
}

/// Validate the 4-word code in a v2 token. We don't constrain
/// to CODE_WORDS here (the dictionary is in `p2p_tunnel`) —
/// just the format: exactly 4 lowercase-alpha segments joined
/// by `-`. The receiver will use the code to derive the KEK
/// for SPAKE2; if it's wrong, the SPAKE2 handshake fails.
fn validate_v2_code(code: &str) -> Result<(), String> {
    if code.is_empty() {
        return Err("P2P v2 code is empty".to_string());
    }
    let parts: Vec<&str> = code.split('-').collect();
    if parts.len() != 4 {
        return Err(format!(
            "P2P v2 code must be 4 words separated by '-' (got {} segments)",
            parts.len()
        ));
    }
    for w in &parts {
        if w.is_empty() {
            return Err("P2P v2 code contains empty word".to_string());
        }
        if !w.chars().all(|c| c.is_ascii_lowercase()) {
            return Err(format!(
                "P2P v2 code word '{}' contains non-lowercase chars",
                w
            ));
        }
    }
    Ok(())
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

    // --- v2 token format (Sprint 5.5.2 Direct Mode) -------------------

    #[test]
    fn derive_service_short_name_is_six_hex() {
        let name = derive_service_short_name("alpha-bear-cosmic-delta");
        assert!(name.starts_with("nx-"));
        let hex = name.strip_prefix("nx-").unwrap();
        assert_eq!(hex.len(), 6);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn derive_service_short_name_is_deterministic() {
        // Same code → same service name. The receiver's
        // derived name must match the sender's exactly or the
        // mDNS browse filter rejects all results.
        let code = "alpha-bear-cosmic-delta";
        let n1 = derive_service_short_name(code);
        let n2 = derive_service_short_name(code);
        assert_eq!(n1, n2);
    }

    #[test]
    fn derive_service_short_name_differs_per_code() {
        // Different code → different service name. Hash
        // collisions are possible but extremely unlikely for
        // 6-hex-char (24-bit) output on a small input space.
        // We don't assert uniqueness exhaustively — we just
        // check that two different codes produce different
        // outputs in practice (the SHA-256 first 3 bytes of
        // these two inputs are virtually never equal).
        let n1 = derive_service_short_name("alpha-bear-cosmic-delta");
        let n2 = derive_service_short_name("delta-cosmic-bear-alpha");
        assert_ne!(n1, n2);
    }

    #[test]
    fn derive_mdns_service_name_appends_type() {
        let full = derive_mdns_service_name("alpha-bear-cosmic-delta");
        assert!(full.ends_with("._nexus-share._tcp.local"));
        assert!(full.starts_with("nx-"));
    }

    #[test]
    fn parse_v2_token_accepts_well_formed() {
        let token = format_v2_token("abc123", "alpha-bear-cosmic-delta");
        assert!(token.starts_with("nx:2:direct:abc123:"));
        let parsed = parse_v2_token(&token).expect("parse");
        assert_eq!(parsed.service_hash, "abc123");
        assert_eq!(parsed.code, "alpha-bear-cosmic-delta");
    }

    #[test]
    fn parse_v2_token_roundtrip() {
        let token = format_v2_token("deadbe", "tiger-river-mountain-cloud");
        let parsed = parse_v2_token(&token).expect("parse");
        let reserialized = format_v2_token(&parsed.service_hash, &parsed.code);
        assert_eq!(token, reserialized);
    }

    #[test]
    fn parse_v2_token_rejects_wrong_prefix() {
        assert!(parse_v2_token("nx:1:foo").is_err());
        assert!(parse_v2_token("garbage").is_err());
        assert!(parse_v2_token("").is_err());
    }

    #[test]
    fn parse_v2_token_rejects_wrong_mode() {
        // Only "direct" is supported in v2 right now.
        let bad = format!("nx:2:quick:abc123:alpha-bear-cosmic-delta");
        assert!(parse_v2_token(&bad).is_err());
    }

    #[test]
    fn parse_v2_token_rejects_bad_hash() {
        // Too short.
        let bad = format!("nx:2:direct:abc:alpha-bear-cosmic-delta");
        assert!(parse_v2_token(&bad).is_err());
        // Too long.
        let bad = format!("nx:2:direct:abcdef12:alpha-bear-cosmic-delta");
        assert!(parse_v2_token(&bad).is_err());
        // Non-hex chars.
        let bad = format!("nx:2:direct:zzzzzz:alpha-bear-cosmic-delta");
        assert!(parse_v2_token(&bad).is_err());
    }

    #[test]
    fn parse_v2_token_rejects_bad_code() {
        // Not 4 words.
        assert!(parse_v2_token("nx:2:direct:abc123:alpha-bear").is_err());
        assert!(parse_v2_token("nx:2:direct:abc123:alpha-bear-cosmic").is_err());
        // Empty word.
        assert!(parse_v2_token("nx:2:direct:abc123:alpha--cosmic-delta").is_err());
        // Uppercase.
        assert!(parse_v2_token("nx:2:direct:abc123:Alpha-Bear-Cosmic-Delta").is_err());
    }

    #[test]
    fn derive_and_parse_match() {
        // End-to-end: derive service hash from a code, format a
        // token from those parts, parse it back, and verify the
        // receiver would look up the same mDNS name.
        let code = "alpha-bear-cosmic-delta";
        let service_short = derive_service_short_name(code);
        let hash = service_short.strip_prefix("nx-").unwrap();
        let token = format_v2_token(hash, code);
        let parsed = parse_v2_token(&token).expect("parse");
        let recovered_short = derive_service_short_name(&parsed.code);
        assert_eq!(service_short, recovered_short);
        // And the mDNS fullname matches what the receiver expects.
        assert_eq!(
            derive_mdns_service_name(&parsed.code),
            format!("{}.{}", recovered_short, SERVICE_TYPE)
        );
    }
}
