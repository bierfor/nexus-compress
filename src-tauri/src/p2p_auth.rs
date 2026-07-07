//! Pre-handshake HMAC auth for the P2P tunnel.
//!
//! ## Why this exists (Sprint 5.5)
//!
//! The SPAKE2 handler at `/spake` runs an Ed25519 group
//! operation per request (~500μs). A scraper bot that has
//! found the Quick Tunnel URL can flood the sender with
//! garbage `T_a` values, forcing thousands of group
//! operations per second. The sender's per-request cost is
//! NOT Argon2id (Argon2id runs once at session start) —
//! it's `spake.finish` (R0 in the threat model: cost
//! location clarification). This module adds a cheap HMAC
//! challenge that filters scrapers before they reach
//! `spake.finish`.
//!
//! ## Protocol
//!
//! Both sides derive `pre_key = HKDF-SHA256(KEK,
//! "nx-pre-auth-v1")` at session start. The KEK is
//! `Argon2id(code, salt)`, so anyone who has the token
//! (code + salt) can derive pre_key. Anyone who doesn't
//! (i.e. scrapers who found the URL but don't have the
//! token) gets a uniform 401.
//!
//! The receiver sends the HMAC in the header:
//!
//! ```text
//! X-Nexus-Auth: <unix_ts>|<hex_sig>
//! ```
//!
//! where `hex_sig = hex(HMAC-SHA256(pre_key,
//! "<ts>|<METHOD>|<path>"))`. The method+path binding means
//! a header built for `/spake` can't be replayed against
//! `/file` (R1: binding to request).
//!
//! ## Replay protection (R1)
//!
//! The unix timestamp in the canonical request gives a
//! 60-second validity window (`TIMESTAMP_WINDOW_SECS`). After
//! that, the auth header is invalid. An attacker who
//! records one valid request from the legitimate receiver
//! can replay it for up to 60s, but SPAKE2's per-handshake
//! randomness makes the replay useless for deriving new
//! session keys — each `start_b` produces a different `T_b`
//! and the attacker would need to also re-do the SPAKE2
//! dance to derive a valid session key.
//!
//! ## Error uniformity (R2)
//!
//! Every auth failure — missing header, malformed,
//! timestamp out of window, signature mismatch — returns
//! the same `401 Unauthorized` with the same body. The
//! attacker can't tell which check rejected them, so
//! fingerprinting the protocol stack is impossible.

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

/// The HTTP header carrying the pre-auth HMAC. Lowercase per
/// HTTP/2 spec; HTTP/1.1 is case-insensitive so this works
/// for both.
pub const HEADER_NAME: &str = "x-nexus-auth";

/// HKDF info string. Bump on protocol changes to prevent
/// cross-version key reuse.
const PRE_AUTH_INFO: &[u8] = b"nx-pre-auth-v1";

/// Max clock skew (in seconds) between sender and receiver.
/// 60s is generous — real NTP-synced clocks drift <1s.
/// Larger values = larger replay window.
pub const TIMESTAMP_WINDOW_SECS: i64 = 60;

/// The single uniform 401 body. We return this for ANY
/// failure mode (missing, malformed, expired, signature
/// mismatch). The byte length and content are part of the
/// protocol — changing them would let an attacker
/// distinguish "no header" from "wrong sig".
pub const UNIFORM_401_BODY: &str = "p2p: authentication failed";

// ============================================================================
//  Public API — key derivation, header building
// ============================================================================

/// Derive the pre-auth HMAC key from the sender's KEK.
/// HKDF-SHA256 with domain-separation info. The KEK itself
/// is the Argon2id-stretched (code, salt) pair, computed
/// at session start. Output: a 32-byte HMAC-SHA256 key.
pub fn derive_pre_auth_key(kek: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, kek);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(PRE_AUTH_INFO, &mut *out)
        .expect("hkdf expand: PRE_AUTH_INFO is 16 bytes, well under 255*32 max");
    out
}

/// Build the canonical request string the HMAC covers.
/// Includes timestamp (replay protection) + method + path
/// (binding to the specific request).
fn canonical_request(timestamp_unix: i64, method: &str, path: &str) -> String {
    format!("{}|{}|{}", timestamp_unix, method, path)
}

/// Compute the `X-Nexus-Auth` header value for a given
/// (pre_key, timestamp, method, path). The receiver calls
/// this before sending the request.
pub fn build_auth_header(
    pre_key: &[u8; 32],
    timestamp_unix: i64,
    method: &str,
    path: &str,
) -> String {
    let canonical = canonical_request(timestamp_unix, method, path);
    let mut mac = <HmacSha256 as Mac>::new_from_slice(pre_key)
        .expect("HMAC key");
    mac.update(canonical.as_bytes());
    let sig = mac.finalize().into_bytes();
    format!("{}|{}", timestamp_unix, hex::encode(sig))
}

/// Returns true if the supplied `now` is within
/// `TIMESTAMP_WINDOW_SECS` of the auth-header's timestamp.
/// Public so tests can drive it directly.
pub fn timestamp_within_window(auth_ts: i64, now_unix: i64) -> bool {
    (now_unix - auth_ts).abs() <= TIMESTAMP_WINDOW_SECS
}

/// Current Unix timestamp in seconds. Wrapped in a function
/// so tests can mock the clock.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ============================================================================
//  AuthState — what the middleware needs
// ============================================================================

/// The state the pre-auth middleware reads. Contains the
/// pre-auth HMAC key, derived once from the KEK at session
/// start.
#[derive(Clone)]
pub struct AuthState {
    pub pre_key: Arc<Zeroizing<[u8; 32]>>,
}

impl AuthState {
    /// Build an AuthState from the sender's KEK.
    pub fn new(kek: &[u8; 32]) -> Self {
        AuthState {
            pre_key: Arc::new(derive_pre_auth_key(kek)),
        }
    }
}

// ============================================================================
//  Inner verifier — pure function, easy to test
// ============================================================================

/// Parse the auth header's `ts|sig` parts. Returns None on
/// any malformation (no pipe, bad ts, bad hex).
fn parse_auth_header(value: &str) -> Option<(i64, Vec<u8>)> {
    let (ts_str, sig_hex) = value.split_once('|')?;
    let ts: i64 = ts_str.parse().ok()?;
    let sig = hex::decode(sig_hex).ok()?;
    Some((ts, sig))
}

/// Verify an incoming request's `X-Nexus-Auth` header.
/// Returns Ok(()) if authenticated, Err(()) otherwise.
///
/// Pure function: takes &AuthState and &Request, no I/O,
/// no panics. The middleware is a thin wrapper that maps
/// Err to the uniform 401.
pub fn verify_request(state: &AuthState, req: &Request) -> Result<(), ()> {
    // 1. Extract the X-Nexus-Auth header.
    let header_value = req
        .headers()
        .get(HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .ok_or(())?;
    // 2. Parse "ts|sig".
    let (auth_ts, sig) = parse_auth_header(header_value).ok_or(())?;
    // 3. Check timestamp window.
    if !timestamp_within_window(auth_ts, now_unix()) {
        return Err(());
    }
    // 4. Recompute HMAC over the canonical request and
    //    compare in constant time (HMAC's verify_slice is
    //    constant-time internally).
    let canonical = canonical_request(
        auth_ts,
        req.method().as_str(),
        req.uri().path(),
    );
    let mut mac = <HmacSha256 as Mac>::new_from_slice(state.pre_key.as_slice())
        .map_err(|_| ())?;
    mac.update(canonical.as_bytes());
    mac.verify_slice(&sig).map_err(|_| ())?;
    Ok(())
}

// ============================================================================
//  axum middleware
// ============================================================================

/// axum middleware: requires a valid `X-Nexus-Auth` header
/// on the request. Returns 401 (uniform body) for any
/// failure: missing, malformed, expired, signature
/// mismatch.
pub async fn require_pre_auth(
    State(state): State<AuthState>,
    req: Request,
    next: Next,
) -> Response {
    if verify_request(&state, &req).is_err() {
        return uniform_401();
    }
    next.run(req).await
}

/// Build the uniform 401 Response. Used by the middleware
/// and exported for handlers that want defense-in-depth
/// (e.g. the SPAKE2 / file handlers return this for any
/// post-auth failure so the attacker can't distinguish
/// between layers).
pub fn uniform_401() -> Response {
    (StatusCode::UNAUTHORIZED, UNIFORM_401_BODY).into_response()
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request as HttpRequest, StatusCode},
    };

    fn ke() -> [u8; 32] {
        [0x42u8; 32]
    }

    fn state() -> AuthState {
        AuthState::new(&ke())
    }

    /// Build a test request with an optional auth header.
    /// Header name is `&'static str` (required by
    /// `HeaderName::from_static`). Header value is `String`
    /// (owned) so the test can pass dynamically-built
    /// headers like the HMAC output.
    fn build_request(
        method: &'static str,
        path: &'static str,
        header: Option<(&'static str, String)>,
    ) -> Request {
        let mut builder = HttpRequest::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        if let Some((name, value)) = header {
            builder.headers_mut().insert(
                axum::http::HeaderName::from_static(name),
                axum::http::HeaderValue::from_str(&value).unwrap(),
            );
        }
        builder
    }

    // --- Pure crypto ----------------------------------------------------

    #[test]
    fn pre_auth_key_derivation_is_stable() {
        let k1 = derive_pre_auth_key(&ke());
        let k2 = derive_pre_auth_key(&ke());
        assert_eq!(&*k1, &*k2);
    }

    #[test]
    fn pre_auth_key_derivation_changes_with_input() {
        let k1 = derive_pre_auth_key(&ke());
        let k2 = derive_pre_auth_key(&[0x99u8; 32]);
        assert_ne!(&*k1, &*k2);
    }

    #[test]
    fn pre_auth_key_is_32_bytes() {
        let k = derive_pre_auth_key(&ke());
        assert_eq!(k.len(), 32);
    }

    // --- Header building ------------------------------------------------

    #[test]
    fn build_auth_header_format_is_ts_pipe_hex() {
        let h = build_auth_header(&ke(), 1234567890, "POST", "/spake");
        let (ts, sig) = h.split_once('|').expect("contains pipe");
        assert_eq!(ts, "1234567890");
        // Sig is 32 bytes hex-encoded = 64 chars.
        assert_eq!(sig.len(), 64);
        assert!(hex::decode(sig).is_ok());
    }

    #[test]
    fn build_auth_header_changes_with_method() {
        let k = ke();
        let h1 = build_auth_header(&k, 100, "GET", "/file");
        let h2 = build_auth_header(&k, 100, "POST", "/file");
        // Same ts + path but different method → different sig.
        assert_ne!(h1, h2);
    }

    #[test]
    fn build_auth_header_changes_with_path() {
        let k = ke();
        let h1 = build_auth_header(&k, 100, "POST", "/spake");
        let h2 = build_auth_header(&k, 100, "POST", "/file");
        assert_ne!(h1, h2);
    }

    #[test]
    fn build_auth_header_changes_with_timestamp() {
        let k = ke();
        let h1 = build_auth_header(&k, 100, "POST", "/spake");
        let h2 = build_auth_header(&k, 101, "POST", "/spake");
        assert_ne!(h1, h2);
    }

    // --- Timestamp window ----------------------------------------------

    #[test]
    fn timestamp_within_window_accepts_now() {
        assert!(timestamp_within_window(1000, 1000));
    }

    #[test]
    fn timestamp_within_window_accepts_just_inside() {
        assert!(timestamp_within_window(1000, 1000 + 60));
        assert!(timestamp_within_window(1000, 1000 - 60));
    }

    #[test]
    fn timestamp_within_window_rejects_too_old() {
        assert!(!timestamp_within_window(1000, 1000 + 61));
        assert!(!timestamp_within_window(1000, 1000 + 600));
    }

    #[test]
    fn timestamp_within_window_rejects_too_future() {
        assert!(!timestamp_within_window(1000, 1000 - 61));
    }

    // --- verify_request: happy path ------------------------------------

    #[test]
    fn verify_request_accepts_valid() {
        let s = state();
        let now = now_unix();
        let h = build_auth_header(&*s.pre_key, now, "POST", "/spake");
        let req = build_request("POST", "/spake", Some((HEADER_NAME, h)));
        assert!(verify_request(&s, &req).is_ok());
    }

    // --- verify_request: rejection modes -------------------------------

    #[test]
    fn verify_request_rejects_missing_header() {
        let s = state();
        let req = build_request("POST", "/spake", None);
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_malformed_header() {
        let s = state();
        // No pipe.
        let req = build_request("POST", "/spake", Some((HEADER_NAME, String::from("not-a-pipe"))));
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_bad_ts() {
        let s = state();
        // Pipe + non-numeric ts.
        let req = build_request("POST", "/spake", Some((HEADER_NAME, String::from("abc|deadbeef"))));
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_bad_sig_hex() {
        let s = state();
        // Valid ts, invalid hex.
        let req = build_request("POST", "/spake", Some((HEADER_NAME, String::from("1000|xyz"))));
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_expired_timestamp() {
        let s = state();
        let expired_ts = now_unix() - TIMESTAMP_WINDOW_SECS - 1;
        let h = build_auth_header(&*s.pre_key, expired_ts, "POST", "/spake");
        let req = build_request("POST", "/spake", Some((HEADER_NAME, h)));
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_future_timestamp() {
        let s = state();
        let future_ts = now_unix() + TIMESTAMP_WINDOW_SECS + 1;
        let h = build_auth_header(&*s.pre_key, future_ts, "POST", "/spake");
        let req = build_request("POST", "/spake", Some((HEADER_NAME, h)));
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_wrong_sig() {
        let s = state();
        // Build a valid header but with a different pre_key.
        let other_key = derive_pre_auth_key(&[0xffu8; 32]);
        let h = build_auth_header(&*other_key, now_unix(), "POST", "/spake");
        let req = build_request("POST", "/spake", Some((HEADER_NAME, h)));
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_method_swap() {
        // Header built for POST, sent as GET.
        let s = state();
        let now = now_unix();
        let h = build_auth_header(&*s.pre_key, now, "POST", "/spake");
        let req = build_request("GET", "/spake", Some((HEADER_NAME, h)));
        assert!(verify_request(&s, &req).is_err());
    }

    #[test]
    fn verify_request_rejects_path_swap() {
        // Header built for /spake, sent on /file.
        let s = state();
        let now = now_unix();
        let h = build_auth_header(&*s.pre_key, now, "POST", "/spake");
        let req = build_request("POST", "/file", Some((HEADER_NAME, h)));
        assert!(verify_request(&s, &req).is_err());
    }

    // --- Error uniformity (R2) -----------------------------------------

    #[test]
    fn uniform_401_is_uniform() {
        // All 401s return the same status and body, regardless
        // of which check failed. This is the core of R2: no
        // fingerprinting.
        let r1 = uniform_401();
        let r2 = uniform_401();
        assert_eq!(r1.status(), r2.status());
        assert_eq!(r1.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(UNIFORM_401_BODY, "p2p: authentication failed");
    }

    #[test]
    fn uniform_401_body_is_byte_for_byte_stable() {
        // The exact body is part of the protocol contract —
        // changing it would let an attacker distinguish
        // "no header" from "wrong sig" via response size.
        assert_eq!(UNIFORM_401_BODY.len(), 26);
        assert!(UNIFORM_401_BODY.starts_with("p2p: "));
    }

    // --- end-to-end: axum middleware call -----------------------------
    //
    // The p2p_tunnel integration tests in p2p_tunnel.rs (the
    // `p2p_localhost_roundtrip` test) cover the full
    // end-to-end middleware path. Here we keep only the
    // pure-Rust unit tests for the verifier and the header
    // builder, which is the meaty correctness logic. The
    // remaining e2e axum tests are not worth the extra
    // tower dep + lifetime gymnastics in this file.
}
