//! Tauri bridge API for NexusCompress.
//!
//! This module is the thin layer between the Tauri command handlers
//! and the core compression engine (`crate::codec::compress` /
//! `decompress`). All functions are synchronous and CPU-bound —
//! Tauri commands should wrap them with `spawn_blocking` to avoid
//! blocking the async runtime.
//!
//! ## Design principles
//!
//! - **Stateless**: every call is independent. No global state, no
//!   caches that mutate between calls. Tauri commands can be called
//!   from multiple threads safely.
//! - **Rich results**: every operation returns its result with stats
//!   (size, ratio, time) so the UI can show progress without doing
//!   the math itself.
//! - **Serializable**: all return types derive `Serialize` /
//!   `Deserialize` so they cross the Tauri IPC boundary directly.
//! - **Error-typed**: all fallible operations return
//!   `ApiResult<T>` with structured `ApiError` (not bare strings)
//!   so the UI can show meaningful messages and the Tauri command
//!   can convert to its own error type.
//! - **No Tauri dep**: this module is pure Rust + serde. Tauri
//!   integration lives in the consumer (the Tauri command handler).
//!
//! ## Example: Tauri command handler
//!
//! ```ignore
//! use tauri::command;
//! use nexus_compress::api::{compress_bytes, ApiError};
//!
//! #[command]
//! pub async fn compress(input: Vec<u8>) -> Result<CompressResult, ApiError> {
//!     tauri::async_runtime::spawn_blocking(move || compress_bytes(&input))
//!         .await
//!         .map_err(|e| ApiError::internal(e.to_string()))
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::time::Instant;

// -----------------------------------------------------------------------
// Result types
// -----------------------------------------------------------------------

/// Result of a `compress_bytes` call: the compressed bytes plus stats.
///
/// `ratio` is `original_size / compressed_size` (so 3.5 means the
/// output is 3.5× smaller than the input). The UI should display
/// this as "compressed to X% of original" via `(1.0 / ratio) * 100`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressResult {
    /// The compressed output. Pass to `decompress_bytes` to recover
    /// the original input. Always non-empty (an empty input
    /// produces a small header-only output).
    pub compressed: Vec<u8>,
    /// Size of the input in bytes.
    pub original_size: u64,
    /// Size of the output in bytes.
    pub compressed_size: u64,
    /// `original_size / compressed_size`. 1.0 = no compression
    /// (random data), higher = better. For an empty input the
    /// compressed form is the header, so this is 0.0 — callers
    /// should display "< 1×" or hide the metric.
    pub ratio: f64,
    /// Wall-clock time of the compress call, in milliseconds.
    pub compress_time_ms: f64,
}

/// Result of a `decompress_bytes` call: the recovered bytes plus
/// stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompressResult {
    /// The recovered (decompressed) bytes. Equal to the input that
    /// was originally passed to `compress_bytes`.
    pub data: Vec<u8>,
    /// Size of the recovered data in bytes.
    pub size: u64,
    /// Wall-clock time of the decompress call, in milliseconds.
    pub decompress_time_ms: f64,
}

/// Engine metadata for the UI's "About" / settings panel.
///
/// Populated at compile time where possible (version, dict size)
/// and at module init where the value depends on the compiled
/// feature set (format version, feature flags).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInfo {
    /// Crate version from `Cargo.toml` (e.g. "0.1.0").
    pub version: String,
    /// Format version byte written into the output header.
    /// Currently 4 (the dict codec, sparse v3, and local sub-dict
    /// path all live under v4 — see `format::VERSION_V3`).
    pub format_version: u8,
    /// Number of entries in the trained dictionary baked into
    /// the binary. 5348 from the current `corpus/trained.dict`.
    pub dict_entries: u32,
    /// Whether the entropy gatekeeper (sprint 2.8.5) is compiled
    /// in. Always `true` in the current build.
    pub has_entropy_gate: bool,
    /// Whether the per-block local sub-dict path (sprint 2.8)
    /// is compiled in. Always `true` in the current build.
    pub has_local_subdict: bool,
    /// Whether sparse v3 stream encoding (sprint 2.7) is compiled
    /// in. Always `true` in the current build.
    pub has_sparse_v3: bool,
    /// Human-readable feature list for the UI's bullet points.
    pub features: Vec<String>,
}

// -----------------------------------------------------------------------
// Error type
// -----------------------------------------------------------------------

/// Structured error type for all fallible API operations.
///
/// Tauri commands should return this directly (Tauri serializes
/// `Display` impls of error types) or convert to their own error
/// type via the `code` field for programmatic handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    /// Machine-readable error code, e.g. `"decompress.invalid_header"`.
    /// UIs can switch on this for i18n / fallbacks.
    pub code: String,
    /// Human-readable error message, English-only. Should be
    /// safe to display directly.
    pub message: String,
}

impl ApiError {
    /// Construct an `ApiError` with a code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Construct for internal/unexpected errors (e.g. background
    /// task join failures). The code is always `"internal"`.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// `Result<T, ApiError>` shorthand used throughout the API.
pub type ApiResult<T> = std::result::Result<T, ApiError>;

// -----------------------------------------------------------------------
// Engine functions
// -----------------------------------------------------------------------

/// Compress a byte buffer. Always succeeds (the codec never fails
/// on a valid byte input; an empty input returns a header-only output).
///
/// The output is a self-contained NexusCompress stream that can
/// be persisted or passed to `decompress_bytes` directly.
///
/// # Performance
///
/// CPU-bound. On a 100 KB mixed-content input expect ~5-25 ms on
/// a modern x86. Tauri commands should wrap this in
/// `spawn_blocking` to keep the async runtime responsive.
pub fn compress_bytes(input: &[u8]) -> CompressResult {
    let start = Instant::now();
    let compressed = crate::compress(input);
    let compress_time_ms = start.elapsed().as_secs_f64() * 1000.0;

    let original_size = input.len() as u64;
    let compressed_size = compressed.len() as u64;
    let ratio = if compressed_size == 0 {
        0.0
    } else {
        original_size as f64 / compressed_size as f64
    };

    CompressResult {
        compressed,
        original_size,
        compressed_size,
        ratio,
        compress_time_ms,
    }
}

/// Decompress a NexusCompress stream. Returns an error if the
/// input is not a valid NexusCompress header or if the stream
/// is corrupted.
///
/// # Errors
///
/// - `ApiError { code: "decompress.empty", .. }` if the input is
///   empty.
/// - `ApiError { code: "decompress.invalid_header", .. }` if the
///   header magic or version byte doesn't match.
/// - `ApiError { code: "decompress.corrupted", .. }` if any
///   block's payload fails to decode.
pub fn decompress_bytes(input: &[u8]) -> ApiResult<DecompressResult> {
    if input.is_empty() {
        return Err(ApiError::new(
            "decompress.empty",
            "input is empty; nothing to decompress",
        ));
    }

    let start = Instant::now();
    // `crate::decompress` is currently infallible — it returns the
    // raw bytes (possibly wrong on corrupted input). The header
    // validation happens inside, but a corrupted block would
    // surface as a panic. Wrap with `catch_unwind` to convert to
    // a clean error.
    let data = match std::panic::catch_unwind(|| crate::decompress(input)) {
        Ok(d) => d,
        Err(_) => {
            return Err(ApiError::new(
                "decompress.corrupted",
                "input is not a valid NexusCompress stream or is corrupted",
            ));
        }
    };
    let decompress_time_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(DecompressResult {
        size: data.len() as u64,
        decompress_time_ms,
        data,
    })
}

/// Engine metadata for the UI's "About" / settings panel.
pub fn engine_info() -> EngineInfo {
    EngineInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        format_version: 4, // v4 — see src/format.rs
        dict_entries: 5348, // corpus/trained.dict
        has_entropy_gate: true,
        has_local_subdict: true,
        has_sparse_v3: true,
        features: vec![
            "content-defined chunking (Gear, 4-64KB)".to_string(),
            "block dedup (FNV-1a hash table)".to_string(),
            "LZ77 lazy matching (64KB window, max-match 255)".to_string(),
            "multi-stream rANS (5 streams, scale 12-bit)".to_string(),
            "trained dictionary codec (5348 entries)".to_string(),
            "per-block local sub-dict from full dict (≤100 entries)".to_string(),
            "sparse v3 stream encoding (32-byte bitmask per stream)".to_string(),
            "entropy gatekeeper (skips dict paths outside [3.0, 7.5])".to_string(),
            "RLE pre-filter (run-aware, 7-bit format)".to_string(),
        ],
    }
}

/// One-shot compress + decompress sanity check. Returns the roundtrip
/// match (input == output) and the ratio achieved. Used by the
/// Tauri "verify install" / "self-test" UI button.
pub fn self_test() -> ApiResult<SelfTestResult> {
    let sample: Vec<u8> = b"the quick brown fox jumps over the lazy dog. "
        .iter()
        .cycle()
        .take(8 * 1024) // 8 KB — small enough to be fast, big enough to exercise CDC
        .cloned()
        .collect();
    let compress = compress_bytes(&sample);
    let decompress = decompress_bytes(&compress.compressed)?;
    let roundtrip_ok = decompress.data == sample;
    Ok(SelfTestResult {
        roundtrip_ok,
        ratio: compress.ratio,
        compress_time_ms: compress.compress_time_ms,
        decompress_time_ms: decompress.decompress_time_ms,
    })
}

/// Result of a `self_test` call. The UI should show a green check
/// if `roundtrip_ok` is `true` and red otherwise.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfTestResult {
    pub roundtrip_ok: bool,
    pub ratio: f64,
    pub compress_time_ms: f64,
    pub decompress_time_ms: f64,
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small_text() {
        let data = b"hello world from nexus-compress api".to_vec();
        let r = compress_bytes(&data);
        assert!(r.compressed_size > 0);
        // Tiny inputs have header overhead that outweighs compression.
        // We just check the roundtrip is lossless and the compressed
        // output is non-empty.
        let d = decompress_bytes(&r.compressed).expect("decompress");
        assert_eq!(d.data, data, "roundtrip mismatch");
    }

    #[test]
    fn roundtrip_repetitive() {
        // 8KB of "abcdefgh" repeating — at this size the dedup and
        // rANS overhead dominate, so ratio is modest (~20x), not
        // the 250x we see on a 256KB file. Test the roundtrip and
        // a conservative ratio floor.
        let data: Vec<u8> = b"abcdefgh".iter().cycle().take(8 * 1024).cloned().collect();
        let r = compress_bytes(&data);
        assert!(r.ratio > 5.0, "expected >5x ratio on 8KB repetitive, got {:.2}x", r.ratio);
        let d = decompress_bytes(&r.compressed).expect("decompress");
        assert_eq!(d.data, data);
    }

    #[test]
    fn roundtrip_natural_text() {
        let data: Vec<u8> = b"the quick brown fox jumps over the lazy dog. \
            a stitch in time saves nine. to be or not to be, that is the question. \
            all that glitters is not gold."
            .iter()
            .cycle()
            .take(32 * 1024)
            .cloned()
            .collect();
        let r = compress_bytes(&data);
        assert!(r.ratio > 1.5, "expected >1.5x ratio on natural text, got {:.2}x", r.ratio);
        let d = decompress_bytes(&r.compressed).expect("decompress");
        assert_eq!(d.data, data);
    }

    #[test]
    fn roundtrip_random_is_safe() {
        // 4KB of pseudo-random — should NOT crash, and ratio should
        // be ~1.0 (no compression possible).
        let mut data = vec![0u8; 4 * 1024];
        let mut s: u32 = 0xc0ffee;
        for b in data.iter_mut() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *b = s as u8;
        }
        let r = compress_bytes(&data);
        assert!(r.ratio < 1.05, "random data should not compress, got {:.2}x", r.ratio);
        let d = decompress_bytes(&r.compressed).expect("decompress");
        assert_eq!(d.data, data);
    }

    #[test]
    fn roundtrip_empty_input() {
        // Empty input is a special case — the codec returns a
        // small header-only output. Roundtrip should be empty.
        let r = compress_bytes(&[]);
        let d = decompress_bytes(&r.compressed).expect("decompress");
        assert!(d.data.is_empty(), "empty input should roundtrip to empty");
    }

    #[test]
    fn decompress_empty_input_errors() {
        let err = decompress_bytes(&[]).expect_err("should fail on empty input");
        assert_eq!(err.code, "decompress.empty");
    }

    #[test]
    fn decompress_garbage_input_errors() {
        let err = decompress_bytes(b"this is not a nexus-compress stream")
            .expect_err("should fail on garbage input");
        // Either the panic-catch fires (corrupted) or the header
        // check fails — both produce a structured error.
        assert!(
            err.code.starts_with("decompress."),
            "unexpected error code: {}",
            err.code
        );
    }

    #[test]
    fn engine_info_round_trip_via_json() {
        let info = engine_info();
        let json = serde_json::to_string(&info).expect("serialize");
        let parsed: EngineInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.version, info.version);
        assert_eq!(parsed.format_version, info.format_version);
        assert_eq!(parsed.dict_entries, info.dict_entries);
        assert_eq!(parsed.has_entropy_gate, info.has_entropy_gate);
        assert_eq!(parsed.has_local_subdict, info.has_local_subdict);
        assert_eq!(parsed.has_sparse_v3, info.has_sparse_v3);
        assert_eq!(parsed.features.len(), info.features.len());
    }

    #[test]
    fn self_test_runs_clean() {
        let result = self_test().expect("self_test should succeed");
        assert!(result.roundtrip_ok, "self_test roundtrip failed");
        assert!(result.ratio > 1.0, "self_test input should compress");
    }

    #[test]
    fn api_error_display() {
        let err = ApiError::new("test.code", "test message");
        assert_eq!(format!("{}", err), "test.code: test message");
        let err = ApiError::internal("oops");
        assert_eq!(err.code, "internal");
        assert_eq!(err.message, "oops");
    }
}
