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
use std::path::Path;
use std::time::Instant;

use crate::engine;

// Re-export so the Tauri commands and the UI both see the same
// `DirectoryResult` shape (defined in `src/nxar.rs`).
pub use crate::nxar::{ArchiveEntry, DirectoryResult};

/// Compression level — controls the LZ77 strategy and which codec
/// paths the encoder tries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompressionLevel {
    /// Lazy LZ77 matching. Default. ~8× faster than `Premium` with
    /// no measurable ratio loss on the corpus (3.15× aggregate).
    /// The cost model in `src/cost.rs` says match=17 bits, literal=8
    /// bits, so lazy + 1-lookahead matches the DP's per-position
    /// decisions well enough that the optimal DP's 8.8× cost buys
    /// ~0% gain.
    Fast,
    /// Optimal LZ77 DP (`encode_optimal`). ~8.8× slower than
    /// `Fast` for marginal or zero ratio gain. The DP's
    /// all-literals future-cost estimate is too coarse to
    /// consistently beat lazy on natural text. Kept as an
    /// experimental option; the UI should hide it behind an
    /// "Advanced / max compression" toggle.
    Premium,
}

impl Default for CompressionLevel {
    fn default() -> Self {
        Self::Fast
    }
}

impl std::str::FromStr for CompressionLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "fast" => Ok(Self::Fast),
            "premium" => Ok(Self::Premium),
            other => Err(format!(
                "unknown compression level '{}'; expected 'fast' or 'premium'",
                other
            )),
        }
    }
}

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
    compress_bytes_with_level(input, CompressionLevel::Fast)
}

/// Like `compress_bytes` but lets the caller pick the LZ77 strategy.
///
/// `Fast` (default): lazy matching. ~8× faster than `Premium`
/// with no measurable ratio loss on the corpus.
///
/// `Premium`: optimal DP. ~8.8× slower for marginal or zero
/// ratio gain. Keep as an experimental option.
pub fn compress_bytes_with_level(input: &[u8], level: CompressionLevel) -> CompressResult {
    let start = Instant::now();
    let compressed = match level {
        CompressionLevel::Fast => crate::compress(input),
        CompressionLevel::Premium => crate::compress_premium(input),
    };
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

/// Compression backend selector for the GUI.
///
/// Each backend has a different preprocessor + entropy coder:
/// - `V4`: the original lossless codec (multi-stream LZ77 + rANS
///   + trained dict). 100% reversible. LOSSLESS.
/// - `V5Min`: LZMA + conservative text minify. Lossless on
///   non-comment text, lossy when comments are present. ~102%
///   of 7z on the corpus. Also strong on minified bundles
///   (strips embedded source-map comments).
/// - `V6`: swc AST minify (drops types, comments, formatting on
///   `.js`/`.ts`/`.tsx`) + LZMA. LOSSY. Best on TypeScript
///   source where type annotations are 70-80% of the bytes
///   (~117% of 7z on the corpus).
/// - `V6Solid`: same as `V6` but for a directory — single
///   LZMA stream over the whole preprocessed corpus so the
///   dictionary survives across files. +0.7% on the current
///   corpus, larger wins on diverse source trees.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CompressionBackend {
    V4,
    V5Min,
    V6,
    /// Directory-only. Single LZMA stream over the whole corpus.
    /// The CLI's `--solid` flag uses this; the Tauri UI exposes
    /// it under the directory mode.
    V6Solid,
}

impl std::str::FromStr for CompressionBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "v4" => Ok(Self::V4),
            "v5-min" | "v5min" | "v5" => Ok(Self::V5Min),
            "v6" => Ok(Self::V6),
            "v6-solid" | "v6solid" | "solid" => Ok(Self::V6Solid),
            other => Err(format!(
                "unknown backend '{other}' (use v4 | v5-min | v6 | v6-solid)"
            )),
        }
    }
}

impl Default for CompressionBackend {
    fn default() -> Self {
        Self::V4
    }
}

/// `CompressionBackend` info sent to the UI so the ConfigPanel can
/// render descriptions and ratio hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendInfo {
    pub id: String,
    pub name: &'static str,
    pub tagline: &'static str,
    /// Honest one-liner about the lossy / lossless contract.
    pub contract: &'static str,
    /// Approximate ratio as % of 7z from `bench_v6` (corpus_real).
    /// -1 means "not measured" (e.g. v4 uses its own bench).
    pub pct_of_7z: f64,
    /// Whether the backend is lossy.
    pub lossy: bool,
}

pub fn backend_info() -> Vec<BackendInfo> {
    vec![
        BackendInfo {
            id: "v4".to_string(),
            name: "v4",
            tagline: "Lossless · LZ77 + rANS + dict",
            contract: "100% byte-identical roundtrip. Trained dict (5348 entries).",
            pct_of_7z: 137.0, // v4 raw on the corpus_suite is 137% of 7z
            lossy: false,
        },
        BackendInfo {
            id: "v5-min".to_string(),
            name: "v5 Text-Min",
            tagline: "Text minify + LZMA",
            contract: "Strips comments, collapses whitespace. Loses comments.",
            pct_of_7z: 162.0, // bench_v6 v5-min aggregate (driven by main-app.js bundle)
            lossy: true,
        },
        BackendInfo {
            id: "v6".to_string(),
            name: "v6 AST",
            tagline: "swc AST minify + LZMA (single file)",
            contract: "Drops types, comments, formatting from .js/.ts. Reversible in semantics only.",
            pct_of_7z: 117.0, // bench_v6 v6 aggregate on ts_source category
            lossy: true,
        },
        BackendInfo {
            id: "v6-solid".to_string(),
            name: "v6 Solid-AST",
            tagline: "swc + LZMA · single stream over whole corpus",
            contract: "Same as v6 but the LZMA dictionary spans all files. Best on multi-file source trees.",
            pct_of_7z: 117.0, // bench_solid SOLID v6 = 8.78x, 7z = 7.48x = 117%
            lossy: true,
        },
    ]
}

/// Compress in-memory bytes with the chosen backend.
///
/// `file_name` is used by `V6` to pick the right preprocessor
/// (`.js`/`.ts`/`.tsx` get swc AST minify, everything else gets
/// the conservative text minify). For `V4` and `V5Min` it's
/// only used for stats.
///
/// `lzma_level` is the LZMA preset (0..=9). 6 = balanced
/// (apples-to-apples with `xz -6`), 9 = max (apples-to-apples
/// with `7z -mx=9`). Ignored by `V4` (which has its own LZ77
/// strategy selector — see `CompressionLevel`).
pub fn compress_bytes_with_backend(
    input: &[u8],
    file_name: &str,
    backend: CompressionBackend,
    lzma_level: u32,
) -> CompressResult {
    let start = Instant::now();
    let compressed: Vec<u8> = match backend {
        CompressionBackend::V4 => {
            // Default to Fast LZ77 — the GUI's level slider can
            // upgrade to Premium via compress_bytes_with_level.
            crate::compress(input)
        }
        CompressionBackend::V5Min => {
            // LZMA + conservative text minify.
            let pre = crate::minify::minify(input);
            crate::engine::compress_with("v5-min", &pre, false).unwrap_or_else(|_| pre)
        }
        CompressionBackend::V6 => {
            // swc AST minify (when file_name ends in .js/.ts/etc.)
            // + LZMA. engine::compress_v6 picks the right pre.
            let ext = std::path::Path::new(file_name)
                .extension()
                .and_then(|e| e.to_str());
            crate::engine::compress_v6(input, ext, lzma_level)
        }
        CompressionBackend::V6Solid => {
            // For a single in-memory buffer, v6-solid is the same
            // as v6 (no other files to share the dictionary with).
            // The CLI uses the directory walker; the Tauri GUI
            // should call compress_directory_with_backend instead.
            let ext = std::path::Path::new(file_name)
                .extension()
                .and_then(|e| e.to_str());
            crate::engine::compress_v6(input, ext, lzma_level)
        }
    };
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

/// Unified result returned by `compress_target` — covers BOTH
/// single-file inputs and directory inputs under the same shape so
/// the UI doesn't have to branch on `is_directory`.
///
/// The compressed bytes are always included, so the frontend can
/// save them to disk (or stream them to the user) without an extra
/// round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressTargetResult {
    /// `true` if `input` was a directory, `false` if a single file.
    pub is_directory: bool,
    /// Size of the input on disk, in bytes (recursive for dirs).
    pub original_size: u64,
    /// Size of the compressed output, in bytes.
    pub compressed_size: u64,
    /// `original_size / compressed_size`. 0.0 if no compression.
    pub ratio: f64,
    /// Wall-clock time of the compress call, in milliseconds.
    pub compress_time_ms: f64,
    /// The compressed payload. For files this is a single-stream
    /// v4/v5/v6 output. For directories it's an NXS6 (SOLID v6) or
    /// NXAR archive depending on the backend.
    pub compressed_bytes: Vec<u8>,
    /// Number of files in the archive (1 for a single-file input).
    pub n_files: u64,
    /// Path the compressed output was written to (next to the input
    /// file or inside the input directory). Empty if auto-save was
    /// skipped (e.g. permission error).
    pub output_path: String,
    /// Suggested extension for the compressed output (`.nxs` for
    /// v4, `.lz` for v5/v5-min, `.nxs6` for SOLID v6).
    pub output_ext: &'static str,
}

/// Compress a single file or directory by path. Auto-detects whether
/// `path` is a file or directory and dispatches accordingly. This
/// is the entry point the Tauri GUI uses when the user drops a file
/// or picks one via the native dialog — it doesn't need to know the
/// type ahead of time.
///
/// `lzma_level` is the LZMA preset (0..=9). Ignored by `V4`. For
/// directories the LZMA level still applies to the underlying v6
/// solid LZMA block.
///
/// The compressed bytes are **auto-saved** next to the input so the
/// user can verify the output exists on disk:
/// - For a file `/foo/bar.txt` with backend V4 → writes
///   `/foo/bar.txt.nxs` (or `.lz` / `.nxs6` for v5 / v6-solid).
/// - For a directory `/foo/bar/` with V6Solid → writes
///   `/foo/bar.nxs6` (a sibling).
///
/// If `output_dir` is `Some`, the output filename is preserved but
/// written into that directory instead of next to the input.
///
/// `progress` is called as the work advances (per-file for solid
/// archives; phase transitions for single files). Pass
/// `|_,_,_| {}` if you don't care.
///
/// If writing fails (permission error, etc.) the function still
/// returns the bytes — the `output_path` field will be empty.
pub fn compress_target<P>(
    input_path: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    output_dir: Option<&Path>,
    mut progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: FnMut(ProgressEvent),
{
    let start = Instant::now();
    let meta = std::fs::metadata(input_path).map_err(|e| {
        ApiError::new(
            "target.not_found",
            format!("cannot stat {}: {}", input_path.display(), e),
        )
    })?;

    let (compressed_bytes, total_original, n_files, is_dir): (Vec<u8>, u64, u64, bool) =
        if meta.is_file() {
            progress(ProgressEvent {
                phase: "reading".to_string(),
                current_file: input_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                files_done: 0,
                files_total: 1,
                bytes_done: 0,
                bytes_total: meta.len(),
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            let bytes = std::fs::read(input_path)
                .map_err(|e| ApiError::new("target.read_failed", format!("read failed: {}", e)))?;
            let file_name = input_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let r = compress_bytes_with_backend(&bytes, &file_name, backend, lzma_level);
            progress(ProgressEvent {
                phase: "compressing".to_string(),
                current_file: file_name.clone(),
                files_done: 0,
                files_total: 1,
                bytes_done: meta.len(),
                bytes_total: meta.len(),
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            let compressed = r.compressed;
            progress(ProgressEvent {
                phase: "done".to_string(),
                current_file: file_name.clone(),
                files_done: 1,
                files_total: 1,
                bytes_done: meta.len(),
                bytes_total: meta.len(),
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            (compressed, meta.len(), 1u64, false)
        } else if meta.is_dir() {
            // Walk into the directory and use the directory backend.
            // For V6Solid (the path that exercises the solid LZMA
            // stream) we get per-file progress via the
            // `solid_archive::compress_with_progress` hook. For other
            // backends the directory backend dispatches per-file
            // archiving (less interesting to show progress on, but we
            // still emit a coarse-grain event so the UI gets something).
            let total_size = walk_dir_total_bytes(input_path);
            let (dir_result, archive) =
                compress_directory_with_backend(input_path, backend, lzma_level)?;
            progress(ProgressEvent {
                phase: "done".to_string(),
                current_file: String::new(),
                files_done: dir_result.n_files,
                files_total: dir_result.n_files,
                bytes_done: total_size,
                bytes_total: total_size,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            (
                archive,
                dir_result.total_original_size,
                dir_result.n_files,
                true,
            )
        } else {
            return Err(ApiError::new(
                "target.not_file_or_dir",
                format!("{} is neither file nor directory", input_path.display()),
            ));
        };

    let compress_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    let compressed_size = compressed_bytes.len() as u64;
    let ratio = if compressed_size == 0 {
        0.0
    } else {
        total_original as f64 / compressed_size as f64
    };

    // Auto-save the output. If the user picked a custom output
    // directory, write there. Otherwise, write next to the input
    // so the user can verify on disk.
    let (mut output_path, output_ext) = compute_output_path(input_path, backend, is_dir);
    if let Some(out_dir) = output_dir {
        // Rebuild the path inside the user's chosen directory.
        let file_name = match is_dir {
            true => match input_path.file_name() {
                Some(n) => format!("{}.{}", n.to_string_lossy(), output_ext),
                None => format!("archive.{}", output_ext),
            },
            false => match input_path.file_name() {
                Some(n) => format!("{}.{}", n.to_string_lossy(), output_ext),
                None => format!("file.{}", output_ext),
            },
        };
        output_path = out_dir.join(file_name).to_string_lossy().into_owned();
    }
    let written = std::fs::write(&output_path, &compressed_bytes)
        .map(|_| output_path.clone())
        .map_err(|e| {
            // Surface as a warning rather than a hard error — the
            // bytes are still returned in the response.
            eprintln!("warn: could not write output to {}: {}", output_path, e);
            e
        })
        .ok()
        .unwrap_or_default();

    Ok(CompressTargetResult {
        is_directory: is_dir,
        original_size: total_original,
        compressed_size,
        ratio,
        compress_time_ms,
        compressed_bytes,
        n_files,
        output_path: written,
        output_ext,
    })
}

/// Result of decompressing an archive by path. Mirrors the compress
/// flow's uniform shape so the GUI doesn't have to branch on the
/// archive kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompressTargetResult {
    /// What kind of archive we detected from the file's magic.
    /// One of `nxs6`, `nxar`, `v4`, `v5-v6-single`.
    pub archive_kind: &'static str,
    /// Total bytes restored to disk (recursive for archives).
    pub restored_size: u64,
    /// Number of files written. 1 for a single-file archive.
    pub n_files: u64,
    /// Whether the output is a directory (multi-file archive) or
    /// a single file. The GUI picks the right reveal/open affordance.
    pub is_directory: bool,
    /// Path the output was written to (a directory for archives, a
    /// file for single-file decompression).
    pub output_path: String,
    /// Wall-clock time of the decompress + write, milliseconds.
    pub decompress_time_ms: f64,
}

/// Decompress an archive by path. Auto-detects the format from the
/// file's magic bytes and dispatches:
///
/// - `NXS6\n` → SOLID v6 archive. Restores all files to
///   `<parent>/<archive_stem>.extracted/` (or `output_dir` if
///   given).
///
/// - `NXAR\n` → per-file nxar (lossless v4 codec). Restores all
///   files to `<parent>/<archive_stem>/` (or `output_dir`).
///
/// - `NXS\x00` → single-file v4 stream. Restores the bytes to
///   `<parent>/<archive_stem>.out` (or `output_dir`).
///
/// - `0x05` → single-file v5/v6 LZMA stream. Restores to
///   `<parent>/<archive_stem>.out` (or `output_dir`).
///
/// `progress` is called per-file as the restore lands on disk so
/// the GUI can drive a WinRAR-style progress bar.
pub fn decompress_target_with_progress<P>(
    input_path: &Path,
    output_dir: Option<&Path>,
    mut progress: P,
) -> ApiResult<DecompressTargetResult>
where
    P: FnMut(ProgressEvent),
{
    use std::io::Read;
    let start = Instant::now();
    // **Sprint 5.7.2 hotfix #23:** if the user passed a DIRECTORY
    // (e.g. they double-clicked an extracted folder, or the file
    // picker returned a folder path), bail with a clear error
    // BEFORE trying to read the magic bytes. The old code would
    // `File::open` the directory (which on macOS/Linux returns a
    // valid fd), then fail at `read` with a confusing
    // `Is a directory (os error 21)` because directories don't
    // support `read()`. The user sees a cryptic error pointing at
    // the wrong line; this fix surfaces the real cause.
    if let Ok(meta) = std::fs::metadata(input_path) {
        if meta.is_dir() {
            return Err(ApiError::new(
                "decompress.is_directory",
                format!(
                    "{} is a directory, not an archive. \
                     Pick a .nxs / .nxs6 / .lz / .nxar file.",
                    input_path.display()
                ),
            ));
        }
    }
    // Read the first 8 bytes to dispatch on magic.
    let mut file = std::fs::File::open(input_path).map_err(|e| {
        ApiError::new(
            "decompress.open_failed",
            format!("open {}: {}", input_path.display(), e),
        )
    })?;
    let mut head = [0u8; 8];
    let n = file
        .read(&mut head)
        .map_err(|e| ApiError::new("decompress.read_failed", format!("read header: {}", e)))?;
    if n < 5 {
        return Err(ApiError::new(
            "decompress.too_short",
            format!("file too short ({} bytes)", n),
        ));
    }

    let parent = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let stem = input_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());

    // Dispatch on magic.
    let (kind, output_path, restored_size, n_files, is_dir) =
        if &head[..5] == crate::solid_archive::MAGIC {
            // NXS6: SOLID v6.
            let bytes = std::fs::read(input_path)
                .map_err(|e| ApiError::new("decompress.read_failed", format!("read: {}", e)))?;
            let (entries, solid) = crate::solid_archive::decompress(&bytes)
                .map_err(|e| ApiError::new("solid.decompress_failed", e))?;
            // Output dir: user override or sibling <stem>.extracted/.
            let out_dir = output_dir
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| parent.join(format!("{}.extracted", stem)));
            std::fs::create_dir_all(&out_dir).map_err(|e| {
                ApiError::new(
                    "decompress.mkdir_failed",
                    format!("mkdir {}: {}", out_dir.display(), e),
                )
            })?;
            let total: u64 = entries.iter().map(|e| e.original_size).sum();
            let n = entries.len() as u64;
            progress(ProgressEvent {
                phase: "compressing".to_string(),
                current_file: String::new(),
                files_done: 0,
                files_total: n,
                bytes_done: 0,
                bytes_total: total,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            let mut restored: u64 = 0;
                for (i, e) in entries.iter().enumerate() {
                    // Note: rename the inner `start` to `solid_offset`
                    // so it doesn't shadow the outer `start: Instant`
                    // used by `ProgressEvent::with_estimates`. The
                    // outer `start` is the wall-clock start of
                    // `decompress_target_with_progress`; the inner
                    // one was the byte offset into the SOLID blob.
                    let solid_offset = e.solid_offset as usize;
                    let end = solid_offset + e.pre_size as usize;
                    let file_bytes = &solid[solid_offset..end];
                let target = out_dir.join(&e.name);
                if let Some(p) = target.parent() {
                    std::fs::create_dir_all(p).map_err(|err| {
                        ApiError::new(
                            "decompress.mkdir_failed",
                            format!("mkdir {}: {}", p.display(), err),
                        )
                    })?;
                }
                std::fs::write(&target, file_bytes).map_err(|err| {
                    ApiError::new(
                        "decompress.write_failed",
                        format!("write {}: {}", target.display(), err),
                    )
                })?;
                restored += file_bytes.len() as u64;
                progress(ProgressEvent {
                    phase: "compressing".to_string(),
                    current_file: e.name.clone(),
                    files_done: (i + 1) as u64,
                    files_total: n,
                    bytes_done: restored,
                    bytes_total: total,
                    elapsed_ms: 0,
                    bytes_per_sec: 0.0,
                    eta_ms: 0,
                }.with_estimates(start));
            }
            (
                "nxs6",
                out_dir.to_string_lossy().into_owned(),
                restored,
                n,
                true,
            )
        } else if &head[..4] == b"NXAR" {
            // NXAR: per-file v4 archive.
            let bytes = std::fs::read(input_path)
                .map_err(|e| ApiError::new("decompress.read_failed", format!("read: {}", e)))?;
            let out_dir = output_dir
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| parent.join(&stem));
            std::fs::create_dir_all(&out_dir).map_err(|e| {
                ApiError::new(
                    "decompress.mkdir_failed",
                    format!("mkdir {}: {}", out_dir.display(), e),
                )
            })?;
            let result = decompress_directory(&bytes, &out_dir).map_err(|e| {
                ApiError::new(
                    "nxar.decompress_failed",
                    format!("{}/{}", e.code, e.message),
                )
            })?;
            progress(ProgressEvent {
                phase: "compressing".to_string(),
                current_file: String::new(),
                files_done: result.n_files,
                files_total: result.n_files,
                bytes_done: result.total_original_size,
                bytes_total: result.total_original_size,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            (
                "nxar",
                out_dir.to_string_lossy().into_owned(),
                result.total_original_size,
                result.n_files,
                true,
            )
        } else if &head[..4] == b"NXS\x00" {
            // Single-file v4 stream.
            let bytes = std::fs::read(input_path)
                .map_err(|e| ApiError::new("decompress.read_failed", format!("read: {}", e)))?;
            let out = decompress_bytes(&bytes)?;
            let out_path = match output_dir {
                Some(d) => d.join(format!("{}.out", stem)),
                None => parent.join(format!("{}.out", stem)),
            };
            std::fs::write(&out_path, &out.data).map_err(|e| {
                ApiError::new(
                    "decompress.write_failed",
                    format!("write {}: {}", out_path.display(), e),
                )
            })?;
            progress(ProgressEvent {
                phase: "done".to_string(),
                current_file: stem.clone(),
                files_done: 1,
                files_total: 1,
                bytes_done: out.data.len() as u64,
                bytes_total: out.data.len() as u64,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            (
                "v4",
                out_path.to_string_lossy().into_owned(),
                out.data.len() as u64,
                1,
                false,
            )
        } else if head[0] == 0x05 {
            // Single-file v5/v6 LZMA stream. (Format byte 0x05
            // followed by the LZMA2 stream inside the .lzma
            // framing that `engine::compress_v5` / `compress_v6`
            // produce.)
            //
            // **Sprint 5.7.2 hotfix #22:** the previous code
            // called `decompress_bytes` here, which only knows
            // the V3 NXS format and would reject any LZMA byte
            // with the confusing `decompress.invalid_header:
            // not a .nexus file (bad magic)` error. Use
            // `engine::decompress_any` instead — it dispatches
            // on the first byte (0x05 → LZMA) and decodes
            // correctly.
            let bytes = std::fs::read(input_path)
                .map_err(|e| ApiError::new("decompress.read_failed", format!("read: {}", e)))?;
            let out = engine::decompress_any(&bytes).map_err(|e| {
                ApiError::new("decompress.lzma_failed", format!("lzma decode: {}", e))
            })?;
            let out_path = match output_dir {
                Some(d) => d.join(format!("{}.out", stem)),
                None => parent.join(format!("{}.out", stem)),
            };
            std::fs::write(&out_path, &out).map_err(|e| {
                ApiError::new(
                    "decompress.write_failed",
                    format!("write {}: {}", out_path.display(), e),
                )
            })?;
            progress(ProgressEvent {
                phase: "done".to_string(),
                current_file: stem.clone(),
                files_done: 1,
                files_total: 1,
                bytes_done: out.len() as u64,
                bytes_total: out.len() as u64,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            (
                "v5-v6-single",
                out_path.to_string_lossy().into_owned(),
                out.len() as u64,
                1,
                false,
            )
        } else {
            return Err(ApiError::new(
                "decompress.unknown_format",
                format!(
                    "unknown archive format (magic: {:02x}{:02x}{:02x}{:02x}…)",
                    head[0], head[1], head[2], head[3]
                ),
            ));
        };

    progress(ProgressEvent {
        phase: "done".to_string(),
        current_file: String::new(),
        files_done: n_files,
        files_total: n_files,
        bytes_done: restored_size,
        bytes_total: restored_size,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    }.with_estimates(start));

    let decompress_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(DecompressTargetResult {
        archive_kind: kind,
        restored_size,
        n_files,
        is_directory: is_dir,
        output_path,
        decompress_time_ms,
    })
}

/// Backward-compatible no-progress wrapper.
pub fn decompress_target(input_path: &Path) -> ApiResult<DecompressTargetResult> {
    decompress_target_with_progress(input_path, None, |_| {})
}

/// One entry in the archive preview — a file or directory inside
/// the archive, with its uncompressed size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivePreviewEntry {
    pub path: String,
    pub size: u64,
    /// True if this entry is a directory (folder entry); false for
    /// a regular file. Multi-file archives may have directory
    /// entries for free, but most don't — we infer from the
    /// entry size being 0 or from a trailing slash on the path.
    pub is_dir: bool,
}

/// Result of peeking at an archive. Lets the GUI render a file
/// list (WinRAR-style) before the user commits to extracting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeekResult {
    pub archive_kind: &'static str,
    pub n_files: u64,
    pub total_uncompressed: u64,
    pub compressed_size: u64,
    pub files: Vec<ArchivePreviewEntry>,
}

/// Peek at an archive's contents without decompressing. Reads just
/// enough of the file to discover the TOC:
///
/// - `NXS6\n` → parses the solid header + TOC. Does NOT
///   decompress the LZMA block.
/// - `NXAR\n` → reuses the existing per-file peek.
/// - `NXS\x00` (single v4) / `0x05` (single v5/v6) → returns a
///   synthetic "1 file" entry with no path.
///
/// Unknown formats return an error.
pub fn peek_archive_target(input_path: &Path) -> ApiResult<PeekResult> {
    let meta = std::fs::metadata(input_path).map_err(|e| {
        ApiError::new(
            "peek.not_found",
            format!("cannot stat {}: {}", input_path.display(), e),
        )
    })?;
    let compressed_size = meta.len();
    let bytes = std::fs::read(input_path)
        .map_err(|e| ApiError::new("peek.read_failed", format!("read: {}", e)))?;

    if &bytes[..5] == crate::solid_archive::MAGIC {
        // NXS6: parse the TOC inline (no LZMA decode).
        let (entries, total) = crate::solid_archive::peek_toc(&bytes)
            .map_err(|e| ApiError::new("solid.peek_failed", e))?;
        let files: Vec<ArchivePreviewEntry> = entries
            .into_iter()
            .map(|e| ArchivePreviewEntry {
                path: e.name,
                size: e.original_size,
                is_dir: false,
            })
            .collect();
        Ok(PeekResult {
            archive_kind: "nxs6",
            n_files: files.len() as u64,
            total_uncompressed: total,
            compressed_size,
            files,
        })
    } else if &bytes[..4] == b"NXAR" {
        let result =
            peek_archive(&bytes).map_err(|e| ApiError::new("nxar.peek_failed", e.message))?;
        let total = result.total_original_size;
        let files: Vec<ArchivePreviewEntry> = result
            .entries
            .into_iter()
            .map(|e| ArchivePreviewEntry {
                path: e.path,
                size: e.original_size,
                is_dir: false,
            })
            .collect();
        let n = files.len() as u64;
        Ok(PeekResult {
            archive_kind: "nxar",
            n_files: n,
            total_uncompressed: total,
            compressed_size,
            files,
        })
    } else if &bytes[..4] == b"NXS\x00" || bytes[0] == 0x05 {
        // Single-file archives: no TOC, no file list.
        Ok(PeekResult {
            archive_kind: if bytes[0] == 0x05 {
                "v5-v6-single"
            } else {
                "v4"
            },
            n_files: 1,
            total_uncompressed: compressed_size, // best guess = compressed size
            compressed_size,
            files: vec![ArchivePreviewEntry {
                path: input_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "output".to_string()),
                size: compressed_size,
                is_dir: false,
            }],
        })
    } else {
        Err(ApiError::new(
            "peek.unknown_format",
            format!(
                "unknown archive format (magic: {:02x}{:02x}{:02x}{:02x}…)",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ),
        ))
    }
}

/// Compute the output path for an auto-saved compressed file.
///
/// - Single file input `/foo/bar.txt` →
///   `/foo/bar.txt.nxs` (v4) / `.lz` (v5) / `.nxs6` (v6-solid)
/// - Directory input `/foo/bar/` →
///   `/foo/bar.nxs6` (v6-solid) or `/foo/bar.nxs` (v4 per-file)
fn compute_output_path(
    input_path: &Path,
    backend: CompressionBackend,
    is_dir: bool,
) -> (String, &'static str) {
    let ext = match backend {
        CompressionBackend::V4 => "nxs",
        CompressionBackend::V5Min | CompressionBackend::V6 => "lz",
        CompressionBackend::V6Solid => "nxs6",
    };
    let parent = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let stem = if is_dir {
        // /foo/bar → /foo/bar.nxs6
        input_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".to_string())
    } else {
        // /foo/bar.txt → /foo/bar.txt.nxs (stems off the original name)
        input_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string())
    };
    let base = if is_dir {
        parent.join(format!("{}.{}", stem, ext))
    } else {
        // /foo/bar.txt → /foo/bar.txt.nxs (append ext to full name)
        parent.join(format!("{}.{}", stem, ext))
    };
    (base.to_string_lossy().into_owned(), ext)
}

/// Progress event emitted during `compress_target`.
///
/// The Tauri command hooks this into `app.emit("compress-progress", …)`
/// so the frontend can drive a real progress bar (WinRAR-style).
/// The `phase` is one of:
/// - `"reading"`     – input bytes are being read from disk
/// - `"compressing"` – per-file LZMA encoding in progress
/// - `"done"`        – emit always fires at the end so the UI can
///                       clear the bar
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub current_file: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Milliseconds elapsed since `compress_target` (or
    /// `decompress_target_with_progress`) started. The frontend
    /// uses this to render the elapsed time. Reset to 0 between
    /// calls; within a single call, monotonically increasing.
    pub elapsed_ms: u64,
    /// Throughput, in bytes/second, computed as
    /// `bytes_done / (elapsed_ms / 1000)`. Smoothed via the
    /// instantaneous value at each emit (no EMA — the codec
    /// emits at most a few hundred events per file so a single
    /// sample per emit is fine). 0 during the first 100 ms
    /// when we don't have a reliable throughput yet.
    pub bytes_per_sec: f64,
    /// Estimated milliseconds remaining until `bytes_done ==
    /// bytes_total`. 0 when done, and 0 during the warm-up
    /// window (<100 ms) when we can't estimate yet.
    pub eta_ms: u64,
}

impl ProgressEvent {
    /// Populate `elapsed_ms`, `bytes_per_sec`, and `eta_ms` from
    /// a start time. Use this at every emit site so the frontend
    /// has real data for the progress bar's time / ETA / throughput
    /// display.
    ///
    /// Warm-up window: if `elapsed_ms < 100`, we set
    /// `bytes_per_sec = 0.0` and `eta_ms = 0` (no estimate
    /// possible yet). After 100 ms we have at least one codec
    /// chunk's worth of data and the estimates become meaningful.
    pub fn with_estimates(mut self, start: std::time::Instant) -> Self {
        self.elapsed_ms = start.elapsed().as_millis() as u64;
        if self.elapsed_ms < 100 {
            self.bytes_per_sec = 0.0;
            self.eta_ms = 0;
            return self;
        }
        let elapsed_s = self.elapsed_ms as f64 / 1000.0;
        self.bytes_per_sec = self.bytes_done as f64 / elapsed_s;
        if self.bytes_total > self.bytes_done {
            let remaining = self.bytes_total - self.bytes_done;
            self.eta_ms = if self.bytes_per_sec > 0.0 {
                (remaining as f64 / self.bytes_per_sec * 1000.0) as u64
            } else {
                0
            };
        } else {
            self.eta_ms = 0;
        }
        self
    }
}

/// Walk a directory and sum the byte sizes of every regular file.
/// Used for the bytes_total in the progress bar.
fn walk_dir_total_bytes(root: &Path) -> u64 {
    fn walk(p: &Path) -> std::io::Result<u64> {
        let mut total = 0u64;
        for entry in std::fs::read_dir(p)? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            if ft.is_dir() {
                total += walk(&path)?;
            } else if ft.is_file() {
                if let Ok(md) = entry.metadata() {
                    total += md.len();
                }
            }
        }
        Ok(total)
    }
    walk(root).unwrap_or(0)
}

/// Compress a directory using the chosen backend. The directory
/// is walked recursively and each file is fed through the right
/// preprocessor. For `V6Solid` the preprocessed bytes are
/// concatenated and a single LZMA stream is written. For `V4`
/// (the existing per-file nxar) and `V5Min` / `V6` the original
/// per-file v4 archive is used.
pub fn compress_directory_with_backend(
    input_dir: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
) -> ApiResult<(DirectoryResult, Vec<u8>)> {
    match backend {
        CompressionBackend::V6Solid => {
            // Walk the directory, build the solid archive.
            let files = walk_dir_for_solid(input_dir)?;
            if files.is_empty() {
                return Err(ApiError::new(
                    "directory.empty",
                    format!("no files in {}", input_dir.display()),
                ));
            }
            let archive = crate::solid_archive::compress(&files, lzma_level)
                .map_err(|e| ApiError::new("solid.compress", e))?;
            // Build a synthetic DirectoryResult (the solid format
            // doesn't have the same per-file metadata as nxar, so
            // we report the aggregate only).
            let total_original: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
            let entries: Vec<ArchiveEntry> = files
                .iter()
                .map(|(name, bytes)| ArchiveEntry {
                    path: name.clone(),
                    original_size: bytes.len() as u64,
                    compressed_size: 0, // solid doesn't track per-file
                    compress_time_ms: 0.0,
                })
                .collect();
            let result = DirectoryResult {
                root: input_dir.to_string_lossy().into_owned(),
                n_files: files.len() as u64,
                total_original_size: total_original,
                total_compressed_size: archive.len() as u64,
                aggregate_ratio: total_original as f64 / archive.len().max(1) as f64,
                total_time_ms: 0.0,
                entries,
            };
            Ok((result, archive))
        }
        // The other backends fall through to the existing
        // per-file nxar (lossless v4 codec).
        _ => compress_directory(input_dir, CompressionLevel::Fast),
    }
}

/// Walk a directory recursively into (relative_path, bytes) pairs.
/// Shared with the CLI's `walk_dir` helper — duplicated here to
/// keep the Tauri boundary self-contained.
fn walk_dir_for_solid(root: &Path) -> ApiResult<Vec<(String, Vec<u8>)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("read_dir({}) failed: {}", dir.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry failed: {}", e))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("file_type({}) failed: {}", path.display(), e))?;
            if file_type.is_dir() {
                walk(root, &path, out)?;
            } else if file_type.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("read({}) failed: {}", path.display(), e))?;
                out.push((rel, bytes));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out).map_err(|e| ApiError::new("directory.io", e))?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
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
    // Sprint 5.6.4: codec::decompress now returns Result
    // directly. Still wrap with catch_unwind as a safety net for
    // any other panics deeper in the decode path.
    let data = match std::panic::catch_unwind(|| crate::decompress(input)) {
        Ok(Ok(d)) => d,
        Ok(Err(e)) => {
            return Err(ApiError::new("decompress.invalid_header", e));
        }
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
        format_version: 4,  // v4 — see src/format.rs
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
// Directory compression (NXAR archive)
// -----------------------------------------------------------------------

/// Compress a directory into an NXAR archive.
///
/// `input_dir` is walked recursively (symlinks skipped). Each
/// regular file is compressed independently with the v4 engine
/// and concatenated under the NXAR header. Returns aggregate
/// stats and the archive bytes.
///
/// # Errors
///
/// - `code: "directory.not_found"` if the path doesn't exist or
///   isn't a directory.
/// - `code: "directory.empty"` if no regular files are found.
/// - `code: "directory.io"` for filesystem errors during walk.
pub fn compress_directory(
    input_dir: &Path,
    level: CompressionLevel,
) -> ApiResult<(DirectoryResult, Vec<u8>)> {
    if !input_dir.is_dir() {
        return Err(ApiError::new(
            "directory.not_found",
            format!("path is not a directory: {}", input_dir.display()),
        ));
    }
    crate::nxar::compress_directory(input_dir)
        .map(|(mut result, archive)| {
            // The `level` parameter is currently a no-op for
            // directory compression: each file uses the default
            // `crate::compress` (Fast). A future v2 could pipe
            // the level into the per-file compression call.
            let _ = level;
            // Add an entry-size histogram to the result so the UI
            // can show per-file ratios in the file list.
            result.total_compressed_size = result.entries.iter().map(|e| e.compressed_size).sum();
            result.aggregate_ratio = if result.total_compressed_size == 0 {
                0.0
            } else {
                result.total_original_size as f64 / result.total_compressed_size as f64
            };
            (result, archive)
        })
        .map_err(|e| ApiError::new("directory.io", e))
}

/// Compress multiple directories into a single NXAR archive.
///
/// Each root directory's files are stored under a top-level
/// folder named after the root's leaf name. For example, if
/// the user picks `/path/to/a` and `/path/to/b`, the archive
/// contains entries like `a/inner/file.txt` and `b/other.txt`.
///
/// If any path in `input_dirs` doesn't exist or isn't a
/// directory, the whole call fails. An empty list is also
/// an error.
///
/// # Errors
///
/// - `code: "directory.not_found"` if any path doesn't exist
///   or isn't a directory.
/// - `code: "directory.empty"` if no regular files are found
///   across all directories.
/// - `code: "directory.io"` for filesystem errors.
pub fn compress_directories(
    input_dirs: &[&Path],
    level: CompressionLevel,
) -> ApiResult<(DirectoryResult, Vec<u8>)> {
    if input_dirs.is_empty() {
        return Err(ApiError::new("directory.empty", "no directories provided"));
    }
    for d in input_dirs {
        if !d.is_dir() {
            return Err(ApiError::new(
                "directory.not_found",
                format!("path is not a directory: {}", d.display()),
            ));
        }
    }
    crate::nxar::compress_directories(input_dirs.iter().copied())
        .map(|(mut result, archive)| {
            let _ = level;
            result.total_compressed_size = result.entries.iter().map(|e| e.compressed_size).sum();
            result.aggregate_ratio = if result.total_compressed_size == 0 {
                0.0
            } else {
                result.total_original_size as f64 / result.total_compressed_size as f64
            };
            (result, archive)
        })
        .map_err(|e| ApiError::new("directory.io", e))
}

/// Decompress an NXAR archive into `output_dir`.
///
/// Each entry's relative path is created under `output_dir` with
/// the same directory structure. Existing files are overwritten.
///
/// # Errors
///
/// - `code: "directory.io"` for filesystem errors (mkdir, write).
/// - `code: "archive.malformed"` if the archive header is
///   invalid or the magic doesn't match.
/// - `code: "archive.size_mismatch"` if a recovered file's size
///   doesn't match the recorded size (signals corruption).
pub fn decompress_directory(archive: &[u8], output_dir: &Path) -> ApiResult<DirectoryResult> {
    crate::nxar::decompress_directory(archive, output_dir).map_err(|e| {
        // Try to categorize the error for the UI.
        if e.contains("magic") {
            ApiError::new("archive.malformed", e)
        } else if e.contains("size mismatch") {
            ApiError::new("archive.size_mismatch", e)
        } else {
            ApiError::new("directory.io", e)
        }
    })
}

/// Peek at an NXAR archive's manifest without loading per-file
/// payloads. Used by the UI to populate the "archive contents"
/// preview before the user commits to extracting.
///
/// Returns a `DirectoryResult`-shaped value with the entry list
/// (path, original_size, compressed_size). The
/// `total_original_size` and `aggregate_ratio` are populated
/// from the entry sizes.
pub fn peek_archive(archive: &[u8]) -> ApiResult<crate::nxar::DirectoryResult> {
    let entries =
        crate::nxar::peek_archive(archive).map_err(|e| ApiError::new("archive.malformed", e))?;
    let n_files = entries.len() as u64;
    let total_original: u64 = entries.iter().map(|e| e.original_size).sum();
    let total_compressed: u64 = entries.iter().map(|e| e.compressed_size).sum();
    let aggregate_ratio = if total_compressed == 0 {
        0.0
    } else {
        total_original as f64 / total_compressed as f64
    };
    Ok(crate::nxar::DirectoryResult {
        root: "<archive>".to_string(),
        n_files,
        total_original_size: total_original,
        total_compressed_size: total_compressed,
        aggregate_ratio,
        total_time_ms: 0.0,
        entries,
    })
}

/// Peek an NXAR archive by reading from a file path. The file is
/// read from disk in Rust, so the entire archive bytes never
/// cross the IPC boundary (avoids the multi-second JSON
/// serialization for large archives).
pub fn peek_archive_file(path: &Path) -> ApiResult<Vec<crate::nxar::ArchiveEntry>> {
    crate::nxar::peek_archive_file(path).map_err(|e| ApiError::new("archive.io", e))
}

/// Peek + extract in one call. Reads the archive from `path`,
/// peeks the manifest, and (if `extract_to` is Some) extracts
/// to that directory. The combined operation reads the file
/// from disk only once.
pub fn peek_and_extract_file(
    path: &Path,
    extract_to: Option<&Path>,
) -> ApiResult<(
    Vec<crate::nxar::ArchiveEntry>,
    Option<crate::nxar::DirectoryResult>,
)> {
    crate::nxar::peek_and_extract_file(path, extract_to).map_err(|e| ApiError::new("archive.io", e))
}

/// Open a path in the OS file manager (Finder on macOS, Explorer
/// on Windows, xdg-open on Linux). Used by the UI's
/// "open extracted folder" button.
///
/// Returns Ok(()) if the OS command spawned successfully;
/// the actual app launch is fire-and-forget.
pub fn open_path(path: &str) -> ApiResult<()> {
    use std::process::Command;
    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg(path).spawn()
    } else if cfg!(target_os = "windows") {
        Command::new("explorer").arg(path).spawn()
    } else {
        // Linux / BSD — xdg-open is the de-facto standard.
        Command::new("xdg-open").arg(path).spawn()
    };
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(ApiError::new(
            "open_path.failed",
            format!("could not open {}: {}", path, e),
        )),
    }
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
        assert!(
            r.ratio > 5.0,
            "expected >5x ratio on 8KB repetitive, got {:.2}x",
            r.ratio
        );
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
        assert!(
            r.ratio > 1.5,
            "expected >1.5x ratio on natural text, got {:.2}x",
            r.ratio
        );
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
        assert!(
            r.ratio < 1.05,
            "random data should not compress, got {:.2}x",
            r.ratio
        );
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
    fn compression_level_default_is_fast() {
        assert_eq!(CompressionLevel::default(), CompressionLevel::Fast);
    }

    #[test]
    fn compress_with_level_fast_works() {
        // The default `Fast` level should be the same as the
        // existing `compress_bytes` (lazy LZ77).
        let data: Vec<u8> = b"the quick brown fox jumps over the lazy dog"
            .iter()
            .cycle()
            .take(4096)
            .cloned()
            .collect();
        let r_fast = compress_bytes_with_level(&data, CompressionLevel::Fast);
        let r_default = compress_bytes(&data);
        // Fast is the default — the two calls produce the same
        // output (and the same stats).
        assert_eq!(r_fast.compressed_size, r_default.compressed_size);
        assert_eq!(r_fast.compress_time_ms >= 0.0, true);
        // Roundtrip must hold.
        let d = decompress_bytes(&r_fast.compressed).expect("decompress");
        assert_eq!(d.data, data);
    }

    #[test]
    fn compress_with_level_premium_runs() {
        // The `Premium` level currently delegates to the same
        // codec (the optimal DP was found to be 8.8× slower with
        // ~0% ratio gain — see README sprint 2.9). The function
        // exists and produces a valid roundtrip.
        let data: Vec<u8> = b"the quick brown fox jumps over the lazy dog"
            .iter()
            .cycle()
            .take(4096)
            .cloned()
            .collect();
        let r = compress_bytes_with_level(&data, CompressionLevel::Premium);
        let d = decompress_bytes(&r.compressed).expect("decompress");
        assert_eq!(d.data, data);
    }

    #[test]
    fn compression_level_serde_roundtrip() {
        // The level must survive a serde roundtrip so the Tauri
        // command param works over the IPC boundary.
        let json_fast = serde_json::to_string(&CompressionLevel::Fast).unwrap();
        let json_premium = serde_json::to_string(&CompressionLevel::Premium).unwrap();
        assert_eq!(json_fast, "\"fast\"");
        assert_eq!(json_premium, "\"premium\"");
        let parsed: CompressionLevel = serde_json::from_str(&json_premium).unwrap();
        assert_eq!(parsed, CompressionLevel::Premium);
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
