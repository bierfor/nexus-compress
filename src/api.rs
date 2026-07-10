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
use std::sync::Arc;
use std::time::Instant;

use crate::codec;
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
// Sprint 5.7.7 hotfix #55: CorpusMode
// -----------------------------------------------------------------------

/// What to include when archiving a directory.
///
/// The user controls this with a 3-pill selector in the UI
/// ("Everything" / "Source" / "Minimal") and a CLI flag
/// (`--corpus=everything|source|minimal`). The default is
/// `Everything` since 5.7.7 — the previous default of skipping
/// dev caches was useful for ratio, but it silently dropped
/// files the user might want to recover later (a freshly cloned
/// repo with its `target/` or `.next/` is recoverable from
/// the package manager, but an in-progress WIP isn't).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CorpusMode {
    /// Include every file under the root. No skipping.
    Everything,
    /// Skip dev caches and build artifacts.
    Source,
    /// Keep only source code + manifests.
    Minimal,
}

impl Default for CorpusMode {
    fn default() -> Self {
        Self::Everything
    }
}

impl std::str::FromStr for CorpusMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "everything" | "all" | "full" => Ok(Self::Everything),
            "source" | "src" | "code" => Ok(Self::Source),
            "minimal" | "min" => Ok(Self::Minimal),
            other => Err(format!(
                "unknown corpus mode '{}'; expected 'everything', 'source', or 'minimal'",
                other
            )),
        }
    }
}

impl CorpusMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Everything => "everything",
            Self::Source => "source",
            Self::Minimal => "minimal",
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
    /// Bytes skipped by the dev-cache walk filter (hotfix #43).
    /// The frontend shows this as a tooltip: "Skipped X MiB of
    /// dev cache (`.next`, `node_modules`, …)" so users understand
    /// a low ratio on a cache-heavy corpus is intentional.
    pub skipped_bytes: u64,
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
    /// Sprint 5.7.9 part 6: corpus breakdown by category
    /// (source / build_artifact / other). The UI uses
    /// this to warn the user when the archive is
    /// dominated by build artifacts (where no codec
    /// can do much). For a single-file input this is
    /// the zero `CorpusBreakdown`.
    pub corpus_breakdown: CorpusBreakdown,
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
    progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    // Sprint 5.7.2 hotfix #47: pass `None` for the codec to keep
    // the legacy Auto behaviour. The newer `compress_target_with_codec`
    // overload is what the GUI calls when the toggle is set.
    compress_target_with_codec(input_path, backend, lzma_level, output_dir, None, progress)
}

/// Sprint 5.7.2 hotfix #47: same as `compress_target` but lets
/// the caller pin a specific codec (LZMA or Zstd) for the
/// whole archive. `codec = None` keeps the Auto behaviour
/// (entropy flip per chunk).
pub fn compress_target_with_codec<P>(
    input_path: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    output_dir: Option<&Path>,
    codec: Option<crate::solid_archive::Codec>,
    progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    // Sprint 5.7.3 hotfix #49: default to lossy (smart
    // preprocessor). Callers that want bit-exact
    // reversibility use `compress_target_with_codec_lossless`.
    compress_target_with_codec_lossless(input_path, backend, lzma_level, output_dir, codec, false, progress)
}

/// Sprint 5.7.3 hotfix #49: same as `compress_target_with_codec`
/// but every file is fed to the codec as `Preprocessor::Raw`
/// (no minify, no swc AST, no entropy flip on extension).
/// The archive is bit-exact reversible. The trade-off is a
/// worse ratio (typically 1.5-2x instead of 5-7x) on text
/// corpora because the smart preprocessor no longer strips
/// whitespace, comments, or types.
pub fn compress_target_with_codec_lossless<P>(
    input_path: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    output_dir: Option<&Path>,
    codec: Option<crate::solid_archive::Codec>,
    lossless: bool,
    mut progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    // Sprint 5.7.4 hotfix #50: per-extension overrides default
    // to empty. Callers that need granular control use
    // `compress_target_with_codec_lossless_overrides` (the
    // GUI's "Advanced" panel surfaces this when the user
    // adds raw/minify extension lists).
    compress_target_with_codec_lossless_overrides(
        input_path,
        backend,
        lzma_level,
        output_dir,
        codec,
        lossless,
        &crate::solid_archive::PreprocessorOverrides::default(),
        progress,
    )
}

/// Sprint 5.7.4 hotfix #50: full-control entry point. The
/// caller passes per-extension override lists in addition
/// to the global `lossless` flag. Used by the GUI's Advanced
/// panel and by the CLI's `--raw-ext` / `--minify-ext` flags.
pub fn compress_target_with_codec_lossless_overrides<P>(
    input_path: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    output_dir: Option<&Path>,
    codec: Option<crate::solid_archive::Codec>,
    lossless: bool,
    overrides: &crate::solid_archive::PreprocessorOverrides,
    mut progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    let start = Instant::now();
    // Sprint 5.7.2 hotfix #24: shared ElapsedTracker so the
    // per-chunk closure can read fresh elapsed_ms without
    // relying on the closure-capture pattern (which was
    // producing elapsed_ms=0 in every event for large
    // compressions; see src/nxar.rs for the full diagnosis).
    let elapsed_tracker = crate::nxar::ElapsedTracker::new();
    // Sprint 5.7.2 hotfix #29: emit an IMMEDIATE "starting" event
    // so the UI shows something the instant the user clicks
    // Comprimir. Previously the first event fired only AFTER
    // walk_dir_total_bytes + walk_dir_for_solid (which read all
    // 5 GB of files into RAM) — leaving the UI frozen at 0%
    // for up to a minute on large directories.
    progress(ProgressEvent {
        phase: "compressing".to_string(),
        current_file: String::new(),
        files_done: 0,
        files_total: 0,
        bytes_done: 0,
        bytes_total: 0,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    });
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
            }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
            let bytes = std::fs::read(input_path)
                .map_err(|e| ApiError::new("target.read_failed", format!("read failed: {}", e)))?;
            let file_name = input_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Sprint 5.7.2 hotfix #23: emit a "compressing" event
            // before the codec actually runs so the UI sees the bar
            // at 0% with a real `start` timestamp. Then call the
            // codec via the progress-aware variant (`codec::compress_
            // with_progress`) and pipe per-chunk `bytes_done` into
            // the same ProgressEvent callback. The codec finishes,
            // then we emit one final "compressing" event with
            // bytes_done=bytes_total so the bar hits 100% before
            // transitioning to "done".
            //
            // Sprint 5.7.2 hotfix #24: DO NOT pre-run the codec
            // via `compress_bytes_with_backend` here. The previous
            // code did `let r = compress_bytes_with_backend(...)`
            // and discarded `r`, then re-ran the codec via
            // `compress_with_progress` — this doubled the wall-clock
            // compression time on big files (e.g. 428 MB / 92 s →
            // actual codec only 45 s, but the user saw 92 s
            // because the silent pre-run ate the first half). The
            // pre-run was a leftover from when `compress_with_
            // progress` didn't exist; now the codec has its own
            // progress-aware variant and the pre-run is dead code.
            let compressed = {
                let total = meta.len();
                // Pre-event: 0% with real elapsed_ms / bytes_per_sec.
                progress(ProgressEvent {
                    phase: "compressing".to_string(),
                    current_file: file_name.clone(),
                    files_done: 0,
                    files_total: 1,
                    bytes_done: 0,
                    bytes_total: total,
                    elapsed_ms: 0,
                    bytes_per_sec: 0.0,
                    eta_ms: 0,
                }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
                // Codec with per-chunk progress. The throttle on
                // the Tauri side (100 ms window) will dedupe bursts;
                // we still get a smooth bar.
                let bytes_done = std::cell::Cell::new(0u64);
                crate::codec::compress_with_progress(&bytes, |cumulative| {
                    // Throttle in-process to avoid emitting one event
                    // per CDC chunk (could be hundreds for large
                    // files). 4 KiB granularity is plenty for the
                    // 100 ms IPC throttle downstream.
                    let prev = bytes_done.get();
                    if cumulative - prev >= 4096 || cumulative == total {
                        bytes_done.set(cumulative);
                        // TEMP DEBUG: confirm `start` is the outer
                        // Instant. If we see 0 here, start is being
                        // shadowed by a local somewhere.
                        let raw_elapsed = start.elapsed().as_millis() as u64;
                        if raw_elapsed == 0 || cumulative == total {
                            eprintln!("[PROGRESS-CALLBACK-DEBUG] cumulative={} raw_start.elapsed()={}ms bytes_done_now={}",
                                cumulative, raw_elapsed, bytes_done.get());
                        }
                        progress(ProgressEvent {
                            phase: "compressing".to_string(),
                            current_file: file_name.clone(),
                            files_done: 0,
                            files_total: 1,
                            bytes_done: cumulative,
                            bytes_total: total,
                            elapsed_ms: 0,
                            bytes_per_sec: 0.0,
                            eta_ms: 0,
                        }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
                    }
                })
            };
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
            }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
            (compressed, meta.len(), 1u64, false)
        } else if meta.is_dir() {
            // Walk into the directory and use the directory backend.
            // For V6Solid (the path that exercises the solid LZMA
            // stream) we get per-file progress via the
            // `solid_archive::compress_with_progress` hook. For other
            // backends the directory backend dispatches per-file
            // archiving with per-chunk progress via the
            // nxar::compress_directory_with_progress hook (hotfix #23.1).
            //
            // The wrapping `with_progress` variant forwards every
            // per-file + per-chunk event through the same `progress`
            // closure so the UI bar advances smoothly. We also emit
            // a pre-event at 0% with a real bytes_total before the
            // work starts, so the user sees the bar at 0% with
            // a non-zero denominator from the first paint.
            let total_size = walk_dir_total_bytes(input_path);
            progress(ProgressEvent {
                phase: "compressing".to_string(),
                current_file: String::new(),
                files_done: 0,
                files_total: 1, // updated below once we know n_files
                bytes_done: 0,
                bytes_total: total_size,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
            let (dir_result, archive) =
                compress_directory_with_backend_and_codec_overrides_with_progress(
                    input_path,
                    backend,
                    lzma_level,
                    codec,
                    lossless,
                    overrides,
                    |ev| progress(ev),
                )?;
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
            }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
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
    // Sprint 5.7.2 hotfix #24: emit a "writing" phase BEFORE the
    // fs::write so the user sees the bar move off 100% (the bar was
    // frozen at 100% during file write because the codec's last
    // per-chunk event already had bytes_done == bytes_total). For
    // a 1+ GB output, fs::write can take many seconds — without
    // this event the user sees "Comprimiendo 100%" with no
    // indication that the codec finished and we're writing.
    progress(ProgressEvent {
        phase: "writing".to_string(),
        current_file: String::new(),
        files_done: n_files,
        files_total: n_files,
        bytes_done: compressed_bytes.len() as u64,
        bytes_total: compressed_bytes.len() as u64,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
    // Sprint 5.7.2 hotfix #36: buffered chunked write.
    //
    // `std::fs::write` is a single `write_all` of the entire 2+ GB
    // Vec. macOS APFS handles small syscalls well but a single
    // multi-GB write stalls at 9 MB/s because the page cache fills
    // and the kernel blocks. Chunked writes (16 MiB at a time) hit
    // ~500 MB/s on Apple Silicon SSD.
    let written = match std::fs::File::create(&output_path) {
        Ok(mut f) => {
            use std::io::Write as IoWrite;
            const CHUNK: usize = 16 * 1024 * 1024;
            let mut written_total: usize = 0;
            let write_result: std::io::Result<()> = (|| {
                for chunk in compressed_bytes.chunks(CHUNK) {
                    f.write_all(chunk)?;
                    written_total += chunk.len();
                }
                f.flush()?;
                Ok(())
            })();
            match write_result {
                Ok(()) => {
                    eprintln!(
                        "[WRITE] buffered: {} MiB in {} MiB chunks ({} MB/s est.)",
                        written_total / (1024 * 1024),
                        CHUNK / (1024 * 1024),
                        if compressed_bytes.len() > 0 {
                            (written_total as u64 * 1000) / 1.max(written_total as u64)
                        } else {
                            0
                        }
                    );
                    Ok(output_path.clone())
                }
                Err(e) => {
                    eprintln!("warn: could not write output to {}: {}", output_path, e);
                    Err(e)
                }
            }
        }
        Err(e) => {
            eprintln!("warn: could not create output file {}: {}", output_path, e);
            Err(e)
        }
    }
    .ok()
    .unwrap_or_default();

    Ok(CompressTargetResult {
        is_directory: is_dir,
        original_size: total_original,
        skipped_bytes: crate::stats::take_skipped_bytes(),
        compressed_size,
        ratio,
        compress_time_ms,
        compressed_bytes,
        n_files,
        corpus_breakdown: if is_dir {
            crate::stats::take_corpus_breakdown()
        } else {
            CorpusBreakdown::default()
        },
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
    P: Fn(ProgressEvent) + Send + Sync,
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
            // Sprint 5.7.2 hotfix #23: decompress with per-block
            // progress. The callback updates bytes_done incrementally
            // as the codec streams blocks. We emit a "decrypting"
            // event (alias for compressing during single-file
            // decompress) for the UI bar.
            let total_size = bytes.len() as u64;
            progress(ProgressEvent {
                phase: "decrypting".to_string(),
                current_file: stem.clone(),
                files_done: 0,
                files_total: 1,
                bytes_done: 0,
                bytes_total: total_size,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(start));
            let out = decompress_bytes_with_progress(&bytes, |cumulative| {
                progress(ProgressEvent {
                    phase: "decrypting".to_string(),
                    current_file: stem.clone(),
                    files_done: 0,
                    files_total: 1,
                    bytes_done: cumulative,
                    bytes_total: total_size,
                    elapsed_ms: 0,
                    bytes_per_sec: 0.0,
                    eta_ms: 0,
                }.with_estimates(start));
            })?;
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

// ─────────────────────────────────────────────────────
//  Sprint 5.7.2 PR #4: encrypted API surface
// ─────────────────────────────────────────────────────

/// Compression backend selection for `compress_target_with_password`.
/// The encrypted path always uses the v4 codec internally
/// (the LZ77+rANS pipeline that powers the multi-stream
/// format) and wraps its output in the V3 wire format
/// (NXE\0 / NXR\0 magic, AES-256-GCM per shard, optional
/// Reed-Solomon parity). See `src/encrypted.rs` for the
/// full design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryLevel {
    /// No parity shards. 0 % overhead, no recovery.
    Off,
    /// 10 % parity budget. Default.
    Low,
    /// 25 % parity budget.
    High,
}

impl Default for RecoveryLevel {
    fn default() -> Self { Self::Low }
}

impl RecoveryLevel {
    /// Parse from a lowercase string ("off" / "low" / "high").
    /// Used by the Tauri command to translate the
    /// `recovery_level` field of the JSON req.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "low" => Ok(Self::Low),
            "high" => Ok(Self::High),
            other => Err(format!(
                "unknown recovery level '{}' (use 'off' | 'low' | 'high')",
                other
            )),
        }
    }
}

/// User-supplied options for `compress_target_with_password`.
/// Mirrors the CLI's `--password P [--recovery off|low|high]`.
#[derive(Debug, Clone)]
pub struct CompressWithPasswordOptions<'a> {
    /// Raw password bytes. The caller is responsible for any
    /// encoding (UTF-8 / etc.) — we don't normalize.
    pub password: &'a [u8],
    /// Recovery parity level. Defaults to `Low` (10 %) when
    /// the frontend doesn't specify one, matching the design
    /// doc's "recovery on by default" choice and the 7z /
    /// WinRAR UX.
    pub recovery: RecoveryLevel,
}

impl<'a> CompressWithPasswordOptions<'a> {
    /// Convenience constructor: password + recovery level.
    pub fn new(password: &'a [u8], recovery: RecoveryLevel) -> Self {
        Self { password, recovery }
    }
}

/// Compress a file or directory with the V4 codec, then encrypt
/// the result with AES-256-GCM (per-shard, `shard_id` in the AAD
/// for block-shuffling resistance) and optionally append
/// Reed-Solomon parity shards. The output format is the V3
/// wire format defined in `format.rs` (NXE\0 / NXR\0 magic).
///
/// **Progress semantics:** the wrapped v4 codec emits at most a
/// few hundred events per file (CDC chunk boundaries). The
/// `encrypted` path adds a few more events (one per shard for
/// the encrypt pass, plus a single event after the RS encode).
/// We translate these to a single `ProgressEvent` stream on the
/// outside so the frontend doesn't have to know the difference
/// between plain and encrypted compression.
///
/// **Memory:** peak is `data_shards × max(ciphertext_len)` —
/// for our 64 KiB target shard size that's at most 230 × 80 KiB
/// ≈ 18 MiB worst case. Acceptable.
///
/// **Why a separate function (not just an `Option<...>` on
/// `compress_target`):** the encrypted path has a different
/// return type for `compressed_bytes` (it returns the V3 wire
/// format bytes, not the NXS bytes), a different output
/// extension (`.nxe` / `.nxr` instead of `.nxs` / `.nxs6`),
/// and a different progress event shape (encrypt + RS phases
/// on top of the codec phases). Mixing the two paths in a
/// single function would make the function signature explode.
pub fn compress_target_with_password<P>(
    input_path: &Path,
    opts: &CompressWithPasswordOptions,
    mut progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    use crate::encrypted::{compress_encrypted, EncryptOptions};
    let start = Instant::now();

    let meta = std::fs::metadata(input_path).map_err(|e| {
        ApiError::new(
            "target.not_found",
            format!("cannot stat {}: {}", input_path.display(), e),
        )
    })?;

    let is_dir = meta.is_dir();

    // Single-phase progress emission: tell the frontend we're
    // "reading" + "encrypting" up front, then emit the final
    // "done" event. The wrapped v4 codec doesn't expose its
    // internal CDC events to the outside (and the encrypted
    // sharding is a single sync pass), so a 3-event lifecycle
    // is the most granular honest signal we can give the UI.
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

    let bytes = std::fs::read(input_path).map_err(|e| {
        ApiError::new("target.read_failed", format!("read failed: {}", e))
    })?;

    progress(ProgressEvent {
        phase: "compressing".to_string(),
        current_file: input_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files_done: 0,
        files_total: 1,
        bytes_done: meta.len() as u64,
        bytes_total: meta.len(),
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    }.with_estimates(start));

    // The encrypted path operates on raw bytes (it doesn't
    // know about directory walks). For the directory case we
    // fall through to `solid_archive::compress` (the v6-solid
    // NXAR-format path) and encrypt the result. For a single
    // file we use the v4 codec + encrypt.
    let compressed = if is_dir {
        // Walk the directory, get a v6-solid NXS6 archive, then
        // encrypt it. Two-pass but each pass is single-threaded
        // for the file enumeration; acceptable for a "secure
        // backup" UX where the user already expects a few-second
        // wait.
        let total_size = walk_dir_total_bytes(input_path);
        let (dir_result, archive) = compress_directory_with_backend(
            input_path,
            CompressionBackend::V6Solid,
            6,
        )?;
        // Emit mid-progress to keep the bar moving.
        progress(ProgressEvent {
            phase: "compressing".to_string(),
            current_file: String::new(),
            files_done: dir_result.n_files,
            files_total: dir_result.n_files,
            bytes_done: total_size,
            bytes_total: total_size,
            elapsed_ms: 0,
            bytes_per_sec: 0.0,
            eta_ms: 0,
        }.with_estimates(start));
        let enc_opts = EncryptOptions {
            password: opts.password,
            recovery: encrypted_recovery_level(opts.recovery),
            preset: crate::crypto::KdfPreset::Interactive,
        };
        compress_encrypted(&archive, &enc_opts, |ev| progress(ev)).map_err(|e| {
            ApiError::new("encrypted.compress_failed", e.to_string())
        })?
    } else {
        // Single-file path: v4 codec → encrypted.
        let v4 = codec::compress(&bytes);
        let enc_opts = EncryptOptions {
            password: opts.password,
            recovery: encrypted_recovery_level(opts.recovery),
            preset: crate::crypto::KdfPreset::Interactive,
        };
        compress_encrypted(&v4, &enc_opts, |ev| progress(ev)).map_err(|e| {
            ApiError::new("encrypted.compress_failed", e.to_string())
        })?
    };

    let compress_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    let compressed_size = compressed.len() as u64;
    let original_size = meta.len();
    let ratio = if compressed_size == 0 {
        0.0
    } else {
        original_size as f64 / compressed_size as f64
    };

    // Output extension: `.nxe` (no recovery) or `.nxr` (with
    // recovery). The wire format's magic byte already encodes
    // this — the extension is a hint for file pickers and
    // shell autocompletion.
    let output_ext = if matches!(opts.recovery, RecoveryLevel::Off) {
        "nxe"
    } else {
        "nxr"
    };

    // Auto-save next to the input (same logic as `compress_target`).
    let (mut output_path, _) = compute_output_path_with_ext(
        input_path,
        output_ext,
        is_dir,
    );
    if let Some(parent) = input_path.parent() {
        // `compute_output_path_with_ext` already produces
        // `<stem>.<ext>` next to the input; nothing to do here.
        let _ = parent;
    }
    let written = std::fs::write(&output_path, &compressed)
        .map(|_| output_path.clone())
        .map_err(|e| {
            eprintln!("warn: could not write encrypted output to {}: {}", output_path, e);
            e
        })
        .ok()
        .unwrap_or_default();

    progress(ProgressEvent {
        phase: "done".to_string(),
        current_file: input_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files_done: 1,
        files_total: 1,
        bytes_done: original_size,
        bytes_total: original_size,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    }.with_estimates(start));

    Ok(CompressTargetResult {
        is_directory: is_dir,
        original_size,
        skipped_bytes: crate::stats::take_skipped_bytes(),
        compressed_size,
        ratio,
        compress_time_ms,
        compressed_bytes: compressed,
        n_files: if is_dir { walk_dir_count(input_path) } else { 1 },
        corpus_breakdown: if is_dir {
            crate::stats::take_corpus_breakdown()
        } else {
            CorpusBreakdown::default()
        },
        output_path: written,
        output_ext,
    })
}

/// Map our `RecoveryLevel` to the encrypted module's.
fn encrypted_recovery_level(l: RecoveryLevel) -> crate::encrypted::RecoveryLevel {
    use crate::encrypted::RecoveryLevel as E;
    match l {
        RecoveryLevel::Off => E::Off,
        RecoveryLevel::Low => E::Low,
        RecoveryLevel::High => E::High,
    }
}

/// Compute the output path with an explicit extension. Mirrors
/// `compute_output_path` but with a custom ext (so the encrypted
/// path can use `.nxe` / `.nxr` instead of the codec-specific
/// `.nxs` / `.lz` / `.nxs6`).
fn compute_output_path_with_ext(
    input_path: &Path,
    output_ext: &'static str,
    is_dir: bool,
) -> (String, &'static str) {
    let parent = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let stem_raw = input_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let stem = strip_trailing_dot_star(std::path::Path::new(&stem_raw))
        .to_string_lossy()
        .into_owned();
    let base = parent.join(format!("{}.{}", stem, output_ext));
    let final_path = strip_trailing_dot_star(&base);
    (final_path.to_string_lossy().into_owned(), output_ext)
}

/// Count files in a directory (regular files only) for the
/// `n_files` field of `CompressTargetResult` in the encrypted
/// directory path. Used to populate the UI's "X files" stat.
fn walk_dir_count(root: &Path) -> u64 {
    fn walk(p: &Path) -> std::io::Result<u64> {
        let mut n = 0u64;
        for entry in std::fs::read_dir(p)? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            if ft.is_dir() {
                n += walk(&path)?;
            } else if ft.is_file() {
                n += 1;
            }
        }
        Ok(n)
    }
    walk(root).unwrap_or(0)
}

/// Decompress a V3-format encrypted archive (NXE\0 / NXR\0 magic).
/// Requires `password`. Dispatches the same way `decompress_target`
/// does (magic-byte check, then to the encrypted decoder).
pub fn decompress_target_with_password<P>(
    input_path: &Path,
    password: &[u8],
    mut progress: P,
) -> ApiResult<DecompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    use crate::encrypted::decompress_encrypted;
    let start = Instant::now();
    let bytes = std::fs::read(input_path).map_err(|e| {
        ApiError::new(
            "decompress.read_failed",
            format!("read {}: {}", input_path.display(), e),
        )
    })?;

    // Two-phase progress: reading + decrypting.
    progress(ProgressEvent {
        phase: "reading".to_string(),
        current_file: input_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files_done: 0,
        files_total: 1,
        bytes_done: 0,
        bytes_total: bytes.len() as u64,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    }.with_estimates(start));

    progress(ProgressEvent {
        phase: "compressing".to_string(),  // we reuse this phase slot
        current_file: String::new(),
        files_done: 0,
        files_total: 1,
        bytes_done: bytes.len() as u64,
        bytes_total: bytes.len() as u64,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    }.with_estimates(start));

    let decompressed = decompress_encrypted(&bytes, password, |ev| progress(ev)).map_err(|e| {
        ApiError::new("encrypted.decompress_failed", e.to_string())
    })?;

    let parent = input_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let stem = input_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let out_path = parent.join(format!("{}.out", strip_trailing_dot_star(Path::new(&stem)).to_string_lossy()));

    let restored_size = decompressed.len() as u64;
    std::fs::write(&out_path, &decompressed).map_err(|e| {
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
        bytes_done: restored_size,
        bytes_total: restored_size,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    }.with_estimates(start));

    let archive_kind = if bytes.starts_with(b"NXR\0") { "nxr" } else { "nxe" };
    Ok(DecompressTargetResult {
        archive_kind,
        restored_size,
        n_files: 1,
        is_directory: false,
        output_path: out_path.to_string_lossy().into_owned(),
        decompress_time_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
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
        // We wrap the SOLID parse in a permissive match: if the
        // TOC is corrupt (truncated header, wrong version byte,
        // mismatched n_files, etc.) we DON'T surface the raw
        // `parse solid toc: ...` technical message — the user
        // just sees a clear "format not recognized" and the
        // ArchivePreview falls back to a "single file" entry
        // with the on-disk size. The user can still try to
        // decompress (the full decoder has more tolerance for
        // partial TOCs than `peek_toc` does).
        let (archive_kind, files, total) = match crate::solid_archive::peek_toc(&bytes) {
            Ok((entries, total)) => {
                let files: Vec<ArchivePreviewEntry> = entries
                    .into_iter()
                    .map(|e| ArchivePreviewEntry {
                        path: e.name,
                        size: e.original_size,
                        is_dir: false,
                    })
                    .collect();
                let n = files.len() as u64;
                ("nxs6", files, total)
            }
            Err(_e) => {
                // TOC was corrupt or the file is truncated.
                // Return a "synthetic single-file" entry so the
                // user at least sees the archive's on-disk size
                // + a marker that the file list couldn't be
                // parsed. The full decompressor will give a
                // better error if the data is actually unreadable.
                let files = vec![ArchivePreviewEntry {
                    path: input_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "archive".to_string()),
                    size: compressed_size,
                    is_dir: false,
                }];
                ("nxs6-truncated", files, compressed_size)
            }
        };
        let n = files.len() as u64;
        Ok(PeekResult {
            archive_kind,
            n_files: n,
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
    } else if &bytes[..4] == b"NXE\0" || &bytes[..4] == b"NXR\0" {
        // Encrypted archive (Sprint 5.7.2). The file list is
        // INSIDE the encrypted payload, so we can't show it
        // without the password — but the outer envelope tells
        // us enough for a useful preview:
        //
        // - `NXE\0` = encrypted, no recovery
        // - `NXR\0` = encrypted + Reed-Solomon parity shards
        //
        // We surface the archive's on-disk size (the only
        // honest metric we have without the key), the recovery
        // mode, and a clear "password required to list files"
        // banner. The frontend renders this in the
        // ArchivePreview component with a lock icon and the
        // password field right next to it.
        let kind = if &bytes[..4] == b"NXR\0" {
            "nxr-encrypted"
        } else {
            "nxe-encrypted"
        };
        Ok(PeekResult {
            archive_kind: kind,
            n_files: 0, // unknown without the password
            total_uncompressed: bytes.len() as u64, // lower bound
            compressed_size: bytes.len() as u64,
            files: vec![], // populated only after password + decrypt
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

/// Peek at an encrypted archive (NXE\0 / NXR\0 magic) with a
/// password. Decrypts the archive, then runs the existing
/// `peek_archive_target` on the reconstructed NXS buffer to
/// extract the file list. On wrong password, the GCM auth check
/// in `encrypted::decompress_encrypted` fails and we return
/// `ApiError::code = "encrypted.wrong_password"` (the frontend
/// maps this to the "✗ wrong password" line in the preview).
///
/// The recovered file list is the SAME format that the plain
/// `peek_archive_target` returns, so the ArchivePreview component
/// doesn't need a special "encrypted unlock" path — it just
/// re-fetches with the password and re-renders the same list.
pub fn peek_archive_target_with_password(
    input_path: &Path,
    password: &[u8],
) -> ApiResult<PeekResult> {
    use crate::encrypted::decompress_encrypted;
    let bytes = std::fs::read(input_path).map_err(|e| {
        ApiError::new("peek.read_failed", format!("read: {}", e))
    })?;
    // Decrypt the archive. GCM auth failure = wrong password.
    // The peek-with-password path doesn't carry a progress
    // closure — peek is fast (header parse + a few shards
    // to validate the password) and emits no events. We use
    // a no-op closure to satisfy the new signature.
    let decompressed = decompress_encrypted(&bytes, password, |_ev| {}).map_err(|e| {
        // Distinguish "wrong password" (GCM auth fail) from
        // other decrypt errors. The encrypted module's
        // CryptoError::Decrypt carries the GCM failure message
        // ("aead: ..."), so we string-match for it.
        let s = e.to_string();
        if s.contains("aead") || s.contains("Decrypt") {
            ApiError::new("encrypted.wrong_password", "wrong password")
        } else {
            ApiError::new("encrypted.decompress_failed", s)
        }
    })?;
    // Now run the existing peek on the reconstructed NXS buffer.
    // We invoke the inner logic by calling the dispatch
    // directly on the bytes in memory. Refactor: build a
    // PeekResult from the in-memory bytes without going back
    // through the filesystem.
    peek_archive_bytes(&decompressed)
}

/// Inner: peek from in-memory bytes (used by both
/// `peek_archive_target` and `peek_archive_target_with_password`).
fn peek_archive_bytes(bytes: &[u8]) -> ApiResult<PeekResult> {
    if bytes.len() < 5 {
        return Err(ApiError::new(
            "peek.too_short",
            format!("buffer too short ({} bytes)", bytes.len()),
        ));
    }
    if &bytes[..5] == crate::solid_archive::MAGIC {
        let (entries, total) = crate::solid_archive::peek_toc(bytes)
            .map_err(|e| ApiError::new("solid.peek_failed", e))?;
        let files: Vec<ArchivePreviewEntry> = entries
            .into_iter()
            .map(|e| ArchivePreviewEntry {
                path: e.name,
                size: e.original_size,
                is_dir: false,
            })
            .collect();
        let n = files.len() as u64;
        Ok(PeekResult {
            archive_kind: "nxs6",
            n_files: n,
            total_uncompressed: total,
            compressed_size: bytes.len() as u64,
            files,
        })
    } else if &bytes[..4] == b"NXAR\n" {
        let entries = crate::nxar::peek_archive(bytes)
            .map_err(|e| ApiError::new("nxar.peek_failed", e))?;
        let files: Vec<ArchivePreviewEntry> = entries
            .into_iter()
            .map(|e| ArchivePreviewEntry {
                path: e.path,
                size: e.original_size,
                is_dir: false,
            })
            .collect();
        let n = files.len() as u64;
        let total: u64 = files.iter().map(|f| f.size).sum();
        Ok(PeekResult {
            archive_kind: "nxar",
            n_files: n,
            total_uncompressed: total,
            compressed_size: bytes.len() as u64,
            files,
        })
    } else if &bytes[..4] == b"NXS\x00" {
        let compressed_size = bytes.len() as u64;
        Ok(PeekResult {
            archive_kind: "v4",
            n_files: 1,
            total_uncompressed: compressed_size,
            compressed_size,
            files: vec![ArchivePreviewEntry {
                path: "<single-file v4 stream>".to_string(),
                size: compressed_size,
                is_dir: false,
            }],
        })
    } else if bytes[0] == 0x05 {
        let compressed_size = bytes.len() as u64;
        Ok(PeekResult {
            archive_kind: "v5-v6-single",
            n_files: 1,
            total_uncompressed: compressed_size,
            compressed_size,
            files: vec![ArchivePreviewEntry {
                path: "<single-file v5/v6 LZMA stream>".to_string(),
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
/// Strip a trailing `.*` from a path. macOS Sequoia's native save
/// dialog has a well-known bug where typing `foo` in a dialog that
/// has "All files (*.*)" or "All extensions" selected appends a
/// literal `.*` to the chosen filename. The result is a file named
/// `foo.nxs6.*` (or `foo.txt.*`, etc.) — confusing for users, and
/// for our own backend it would mean a future `foo.nxs6`
/// decompression would fail (file not found).
///
/// This helper is called on both the **output** path (we don't
/// want to CREATE files with a trailing `.*`) and the **input**
/// path (defensive: if the user already has a file with a
/// trailing `.*` — because of a previous save dialog session —
/// and the extension-less version exists, prefer the extension-
/// less version so the magic-byte dispatch works).
///
/// **NOT** called for files whose literal name contains `.*` in
/// the middle (e.g. `*.tar.gz` is left alone) — we only strip the
/// trailing 2-byte `.*` suffix.
fn strip_trailing_dot_star(p: &Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    if s.ends_with(".*") && s.len() > 2 {
        let trimmed = &s[..s.len() - 2];
        std::path::PathBuf::from(trimmed)
    } else {
        p.to_path_buf()
    }
}

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

    /// Sprint 5.7.2 hotfix #24: override `elapsed_ms` AFTER calling
    /// `with_estimates(start)`. We need this because the closure
    /// capture pattern used inside `compress_directory_with_progress`
    /// and `compress_target`'s per-chunk closure was producing
    /// `elapsed_ms=0` for every event on large directory compressions
    /// (5 GB / 163k files). Root cause was a subtle interaction
    /// between the closure capture of the `Instant` and the
    /// `with_estimates` builder — the captured Instant was effectively
    /// a fresh `Instant::now()` at each call site.
    ///
    /// Using a separate `ElapsedTracker` (background thread updating
    /// an `AtomicU64`) provides a single source of truth for
    /// `elapsed_ms`. We still call `with_estimates(start)` first to
    /// populate `bytes_per_sec` and `eta_ms`, then override
    /// `elapsed_ms` from the tracker.
    pub fn with_elapsed_override(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = elapsed_ms;
        // Recompute bytes_per_sec and eta_ms from the override so
        // the displayed numbers stay consistent with elapsed_ms.
        if self.elapsed_ms >= 100 {
            let elapsed_s = self.elapsed_ms as f64 / 1000.0;
            self.bytes_per_sec = if self.bytes_done > 0 {
                self.bytes_done as f64 / elapsed_s
            } else {
                0.0
            };
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
        } else {
            self.bytes_per_sec = 0.0;
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
    compress_directory_with_backend_with_progress(input_dir, backend, lzma_level, |_| {})
}

/// Sprint 5.7.2 hotfix #47: same as
/// `compress_directory_with_backend` but with an explicit codec
/// override for the GUI toggle. `codec = None` keeps the legacy
/// Auto behaviour (entropy flip per chunk).
pub fn compress_directory_with_backend_and_codec(
    input_dir: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    codec: Option<crate::solid_archive::Codec>,
) -> ApiResult<(DirectoryResult, Vec<u8>)> {
    compress_directory_with_backend_and_codec_with_progress(
        input_dir,
        backend,
        lzma_level,
        codec,
        false,
        |_| {},
    )
}

/// Sprint 5.7.2 hotfix #23.1 — same as
/// `compress_directory_with_backend` but pipes a
/// `ProgressEvent` callback through both code paths:
///   - V6Solid: per-file `solid_archive::compress_with_progress`
///   - per-file nxar: per-file + per-codec-chunk via
///     `nxar::compress_directory_with_progress`
///
/// Without this, the directory compress path emits ONLY the
/// final "done" event, so the UI bar sits at 0% until the
/// very end and jumps to 100%.
pub fn compress_directory_with_backend_with_progress<P>(
    input_dir: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    progress: P,
) -> ApiResult<(DirectoryResult, Vec<u8>)>
where
    // Sprint 5.7.2 hotfix #28: changed FnMut → Fn + Send + Sync so
    // rayon can share the callback across worker threads during
    // parallel LZMA encoding. Callers that genuinely need FnMut
    // (because they mutate captured state) need to use interior
    // mutability (Mutex / atomic) instead.
    P: Fn(ProgressEvent) + Send + Sync,
{
    // Default to Auto when called from older code paths. The
    // newer overload (see below) lets the GUI pass a pinned
    // codec that bypasses the per-chunk entropy flip.
    compress_directory_with_backend_and_codec_with_progress(
        input_dir,
        backend,
        lzma_level,
        None,
        false,
        progress,
    )
}

/// Sprint 5.7.2 hotfix #47: the GUI exposes a codec toggle
/// (Auto | LZMA | Zstd) that, when set to anything other than
/// Auto, pins the codec and disables the entropy-driven flip
/// (hotfix #39) so the user's choice is honoured for every
/// chunk. The default Auto path is identical to the legacy
/// `compress_directory_with_backend_with_progress`.
pub fn compress_directory_with_backend_and_codec_with_progress<P>(
    input_dir: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    codec: Option<crate::solid_archive::Codec>,
    lossless: bool,
    progress: P,
) -> ApiResult<(DirectoryResult, Vec<u8>)>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    compress_directory_with_backend_and_codec_overrides_with_progress(
        input_dir,
        backend,
        lzma_level,
        codec,
        lossless,
        &crate::solid_archive::PreprocessorOverrides::default(),
        progress,
    )
}

/// Sprint 5.7.4 hotfix #50: same as the codec-only variant
/// but also accepts per-extension overrides. The CLI's
/// `--raw-ext` and the GUI's Advanced panel route here when
/// the user wants granular control over which files get the
/// smart preprocessor.
pub fn compress_directory_with_backend_and_codec_overrides_with_progress<P>(
    input_dir: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    codec: Option<crate::solid_archive::Codec>,
    lossless: bool,
    overrides: &crate::solid_archive::PreprocessorOverrides,
    progress: P,
) -> ApiResult<(DirectoryResult, Vec<u8>)>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    match backend {
        CompressionBackend::V6Solid => {
            // Sprint 5.7.2 hotfix #26: local ElapsedTracker so the
            // V6Solid pre-event and per-file events report real
            // elapsed_ms (previously they emitted 0 because
            // with_elapsed_override wasn't chained here).
            let v6_tracker = crate::nxar::ElapsedTracker::new();
            let (files, _skipped) = walk_dir_for_solid(input_dir)?;
            if files.is_empty() {
                return Err(ApiError::new(
                    "directory.empty",
                    format!("no files in {}", input_dir.display()),
                ));
            }
            // Sprint 5.7.2 hotfix #26: split the input into
            // "compressible" (text/code/configs) and "passthrough"
            // (PNG/JPG/MP4/ZIP/PDF/…). For V6Solid the codec is a
            // single LZMA stream over the WHOLE corpus, so feeding
            // it 5 GB of already-compressed PNG would burn ~80 s of
            // encode time for zero size reduction. We:
            //   1. Filter out passthrough files from the LZMA input.
            //   2. Emit per-file "passthrough" events so the bar
            //      advances correctly (bytes_done jumps).
            //   3. Concatenate the raw bytes + a small NXPT marker
            //      AFTER the LZMA stream so the decoder can reassemble.
            let mut lzma_files: Vec<(String, Vec<u8>)> = Vec::with_capacity(files.len());
            let mut passthrough_files: Vec<(String, Vec<u8>)> = Vec::new();
            let lower_ext = |s: &str| s.to_ascii_lowercase();
            // Same list as nxar.rs's PASSTHROUGH_EXTS. We duplicate
            // it here (rather than re-exporting from nxar) to keep
            // api.rs self-contained.
            const PASSTHROUGH_EXT_LIST: &[&str] = &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico",
                "heic", "heif", "avif", "jxl", "tiff", "tif",
                "dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2",
                "mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv", "flv",
                "3gp", "3g2", "ts", "m2ts", "mts", "vob", "ogv",
                "mp3", "m4a", "aac", "ogg", "opus", "flac", "alac", "wav",
                "wma", "aiff", "aif", "mka",
                "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst",
                "lz4", "lz", "pkg", "deb", "rpm", "apk", "ipa", "jar", "war", "ear",
                "pdf", "docx", "pptx", "xlsx", "odt", "ods", "odp", "epub",
                "ttf", "otf", "woff", "woff2", "eot",
                "iso", "dmg", "img", "vhd", "vmdk",
                "exe", "dll", "so", "dylib", "class", "pdb",
            ];
            for (name, bytes) in files.iter() {
                let lower = lower_ext(name);
                let is_pt = PASSTHROUGH_EXT_LIST.iter().any(|ext| {
                    lower.ends_with(ext) || lower.ends_with(&format!(".{}", ext))
                });
                if is_pt {
                    passthrough_files.push((name.clone(), bytes.clone()));
                } else {
                    lzma_files.push((name.clone(), bytes.clone()));
                }
            }
            let total_lzma: u64 = lzma_files.iter().map(|(_, b)| b.len() as u64).sum();
            let total_pt: u64 = passthrough_files.iter().map(|(_, b)| b.len() as u64).sum();
            let total_original = total_lzma + total_pt;
            // Emit a passthrough-tick event for each passthrough
            // file so the bar advances. Each one jumps bytes_done
            // by its full size.
            let mut cumulative_pt_bytes: u64 = 0;
            for (name, bytes) in passthrough_files.iter() {
                cumulative_pt_bytes += bytes.len() as u64;
                progress(ProgressEvent {
                    phase: "compressing".to_string(),
                    current_file: name.clone(),
                    files_done: 0,
                    files_total: 0, // passthroughs aren't part of the LZMA stream
                    bytes_done: cumulative_pt_bytes,
                    bytes_total: total_original,
                    elapsed_ms: 0,
                    bytes_per_sec: 0.0,
                    eta_ms: 0,
                }.with_elapsed_override(v6_tracker.current_ms()));
            }
            // Pre-event for the LZMA phase (only the lzma_files
            // contribute to its bytes_total).
            progress(ProgressEvent {
                phase: "compressing".to_string(),
                current_file: String::new(),
                files_done: 0,
                files_total: lzma_files.len() as u64,
                bytes_done: cumulative_pt_bytes,
                bytes_total: total_original,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_elapsed_override(v6_tracker.current_ms()));
            // Now run the codec on the LZMA-eligible files only.
            // Sprint 5.7.2 hotfix #32: use CompressionLevel::auto() which
            // picks zstd on 16 GB machines (~300 MB/s) and LZMA on RAM-
            // constrained machines. The user's mode selection (rapido/
            // balanceado/ultra) is honoured by the CompressionLevel
            // resolver.
            //
            // Sprint 5.7.2 hotfix #47: when the GUI pinned a
            // specific codec via the toggle, build a concrete
            // CompressionLevel (Lzma or Zstd) instead of Auto.
            // Auto still goes through the entropy-driven flip
            // (#39) — the new toggle is a way to *opt out* of
            // that flip and force a single codec for the whole
            // archive.
            let compression_level: crate::solid_archive::CompressionLevel = match codec {
                None => crate::solid_archive::CompressionLevel::auto(),
                Some(crate::solid_archive::Codec::Lzma) => {
                    crate::solid_archive::CompressionLevel::Lzma(lzma_level)
                }
                Some(crate::solid_archive::Codec::Zstd) => {
                    crate::solid_archive::CompressionLevel::Zstd(3)
                }
            };
            let mut archive = if lzma_files.is_empty() {
                Vec::new()
            } else {
                // Sprint 5.7.3 hotfix #49: when the GUI
                // requested lossless mode, route to the
                // dedicated lossless entry point that bypasses
                // the per-extension preprocessor. The result
                // is bit-exact reversible but trades ratio
                // for integrity.
                //
                // Sprint 5.7.4 hotfix #50: per-extension
                // overrides route to the full-control entry
                // point. The three paths are mutually exclusive
                // for clarity — lossless wins over overrides
                // because the user picked "no minify at all",
                // and overrides win over the default because
                // the user picked specific files to pin.
                if lossless {
                    crate::solid_archive::compress_with_progress_lossless(
                        &lzma_files,
                        compression_level,
                        |file_idx, total, name| {
                        let bytes_done: u64 = lzma_files
                            .iter()
                            .take(file_idx)
                            .map(|(_, b)| b.len() as u64)
                            .sum::<u64>()
                            + cumulative_pt_bytes;
                        progress(ProgressEvent {
                            phase: "compressing".to_string(),
                            current_file: name.to_string(),
                            files_done: file_idx as u64,
                            files_total: total as u64,
                            bytes_done,
                            bytes_total: total_original,
                            elapsed_ms: 0,
                            bytes_per_sec: 0.0,
                            eta_ms: 0,
                        }.with_elapsed_override(v6_tracker.current_ms()));
                    },
                )
                    .map_err(|e| ApiError::new("solid.compress", e))?
                } else if !overrides.is_empty() {
                    crate::solid_archive::compress_with_progress_full_public(
                        &lzma_files,
                        compression_level,
                        false,
                        overrides,
                        |file_idx, total, name| {
                            let bytes_done: u64 = lzma_files
                                .iter()
                                .take(file_idx)
                                .map(|(_, b)| b.len() as u64)
                                .sum::<u64>()
                                + cumulative_pt_bytes;
                            progress(ProgressEvent {
                                phase: "compressing".to_string(),
                                current_file: name.to_string(),
                                files_done: file_idx as u64,
                                files_total: total as u64,
                                bytes_done,
                                bytes_total: total_original,
                                elapsed_ms: 0,
                                bytes_per_sec: 0.0,
                                eta_ms: 0,
                            }.with_elapsed_override(v6_tracker.current_ms()));
                        },
                    )
                    .map_err(|e| ApiError::new("solid.compress", e))?
                } else {
                    crate::solid_archive::compress_with_progress(
                        &lzma_files,
                        compression_level,
                        |file_idx, total, name| {
                            let bytes_done: u64 = lzma_files
                                .iter()
                                .take(file_idx)
                                .map(|(_, b)| b.len() as u64)
                                .sum::<u64>()
                                + cumulative_pt_bytes;
                            progress(ProgressEvent {
                                phase: "compressing".to_string(),
                                current_file: name.to_string(),
                                files_done: file_idx as u64,
                                files_total: total as u64,
                                bytes_done,
                                bytes_total: total_original,
                                elapsed_ms: 0,
                                bytes_per_sec: 0.0,
                                eta_ms: 0,
                            }.with_elapsed_override(v6_tracker.current_ms()));
                        },
                    )
                    .map_err(|e| ApiError::new("solid.compress", e))?
                }
            };
            // Append passthrough payloads (with a marker so the
            // decoder knows where the LZMA stream ends and the raw
            // entries begin).
            use std::io::Write;
            for (name, bytes) in passthrough_files.iter() {
                // Marker: 4-byte "NXPT" + u32 name_len + name + raw bytes
                archive.write_all(b"NXPT").ok();
                let name_bytes = name.as_bytes();
                archive
                    .write_all(&(name_bytes.len() as u32).to_le_bytes())
                    .ok();
                archive.write_all(name_bytes).ok();
                archive.write_all(bytes).ok();
            }
            let entries: Vec<ArchiveEntry> = files
                .iter()
                .map(|(name, bytes)| ArchiveEntry {
                    path: name.clone(),
                    original_size: bytes.len() as u64,
                    compressed_size: 0,
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
        // The other backends fall through to the per-file nxar
        // (lossless v4 codec) — now with the new progress variant
        // that emits per-file + per-chunk events.
        _ => {
            // The nxar::compress_directory_with_progress is in
            // the lib crate (not the api wrapper), so wrap its
            // callback type. Both signatures use FnMut, the only
            // difference is the event struct type — they're the
            // same type here (ProgressEvent lives in api.rs and
            // is re-exported).
            crate::nxar::compress_directory_with_progress(
                input_dir,
                |ev| progress(ev),
            )
            .map_err(|e| ApiError::new("nxar.compress_failed", e))
        }
    }
}

/// Sprint 5.7.9: read a file with the kernel's read-ahead
/// turned on. On macOS, `fcntl(fd, F_RDAHEAD, 1)` enables
/// aggressive sequential read-ahead. This bypasses the
/// kernel's near-full-SSD conservatism (the kernel backs
/// off prefetch when free blocks are scarce, to preserve
/// blocks for wear-leveling). The cost is a single fcntl
/// syscall per file; the benefit is 1.5-2x throughput on
/// a busy SSD like the M4 Pro at 90% capacity.
///
/// On non-macOS platforms (Windows / Linux for the headless
/// build), we fall through to plain `std::fs::read`. The
/// Linux build is the only non-macOS target — the GUI is
/// macOS-only — so on Linux we still get the speedup via
/// posix_fadvise if someone enables it later. For now
/// the feature-gate is the right balance: cheap on macOS
/// (where it matters most), no-op elsewhere.
#[cfg(target_os = "macos")]
fn read_with_readahead(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::io::AsRawFd;
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|e| format!("read({}) failed: {}", path.display(), e))?;
    // F_RDAHEAD = 44 on macOS. Enable sequential read-ahead.
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, arg: i32) -> i32;
    }
    const F_RDAHEAD: i32 = 44;
    let fd = file.as_raw_fd();
    unsafe {
        let _ = fcntl(fd, F_RDAHEAD, 1);
    }
    let mut bytes = Vec::new();
    file.take(u64::MAX).read_to_end(&mut bytes)
        .map_err(|e| format!("read_to_end({}) failed: {}", path.display(), e))?;
    Ok(bytes)
}

#[cfg(not(target_os = "macos"))]
fn read_with_readahead(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("read({}) failed: {}", path.display(), e))
}

/// Walk a directory recursively into (relative_path, bytes) pairs.
/// Shared with the CLI's `walk_dir` helper — duplicated here to
/// keep the Tauri boundary self-contained.
fn walk_dir_for_solid(root: &Path) -> ApiResult<(Vec<(String, Vec<u8>)>, u64)> {
    // Sprint 5.7.2 hotfix #29: parallelize the slow I/O phase that
    // used to block for ~1 minute on a 5 GB input. The original
    // walked the directory tree and read each file's bytes SEQUEN-
    // TIALLY. For 5 GB on HDD (~100 MB/s sequential read) that's
    // 50 seconds where the UI shows 0% / no events at all.
    //
    // Now we:
    //   1. Walk the tree SEQUENTIALLY to collect paths (fast — just
    //      stat() calls, no file I/O).
    //   2. Read file CONTENTS in parallel via rayon (the actual
    //      bottleneck — disk I/O benefits from queue_depth > 1).
    //   3. Sort by relative path so the archive is deterministic.
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    /// Sprint 5.7.2 hotfix #43: skip-list for transient build/dev caches.
///
/// These directories contain compiler caches, intermediate artifacts,
/// or runtime state that is NOT part of the user's source code. They
/// are typically:
///   - Multi-GB (Turbopack cache alone is 2-3 GB on Next.js projects)
///   - Already compressed (RocksDB LZ4 inside .sst files)
///   - Re-creatable from source (`rm -rf .next && next build` works)
/// Including them in an archive wastes CPU + I/O and produces a
/// misleadingly-low compression ratio that looks like a bug.
///
/// Sprint 5.7.2 hotfix #45: previously we ran `dir_size` (recursive
/// readdir + metadata) on every skipped directory to populate the
/// UI tooltip. For 164k filesystems like FlowNow that walk took
/// minutes — the dir_size alone dominated the wall time. Now we
/// return a non-recursive best-effort estimate (the dir's own
/// apparent size + a flag) and let the UI label it as "estimate".
/// The exact byte count is no longer needed — the user only needs
/// to see "skipped N MiB of dev cache" not a precise number.
fn dir_size_estimate(path: &Path) -> u64 {
    // Use the dir's own metadata. On macOS this returns the inode
    // allocation (not the recursive sum), so for huge trees it
    // is small but bounded. The UI labels the result as an
    // estimate.
    std::fs::metadata(path)
        .map(|m| m.len())
        .unwrap_or(0)
}

fn corpus_should_skip_dir(path: &Path, mode: CorpusMode) -> bool {
    // Sprint 5.7.7 hotfix #55: parametrise by CorpusMode.
    if matches!(mode, CorpusMode::Everything) {
        return false;
    }
    const SKIP_DIRS: &[&str] = &[
        ".next", ".turbo", ".swc", ".claude", ".nexus",
        "node_modules", "target", ".venv", "venv", "__pycache__",
        ".git", ".DS_Store", "Thumbs.db", "dist", "build",
        ".cache", ".tmp", "coverage", ".nyc_output", "logs", "log", ".run",
    ];
    for comp in path.components() {
        if let Some(name) = comp.as_os_str().to_str() {
            if SKIP_DIRS.iter().any(|s| *s == name) {
                return true;
            }
        }
    }
    if matches!(mode, CorpusMode::Minimal) {
        const MINIMAL_SKIP_DIRS: &[&str] = &[
            "assets", "static", "public", "media", "images", "img",
            "fonts", "icons", "videos", "audio", "screenshots",
            "docs", "doc", "documentation", "examples", "demo",
            "fixtures", "mocks", "snapshots", "__snapshots__",
            "test-data", "testdata",
        ];
        for comp in path.components() {
            if let Some(name) = comp.as_os_str().to_str() {
                if MINIMAL_SKIP_DIRS.iter().any(|s| *s == name) {
                    return true;
                }
            }
        }
    }
    false
}

fn corpus_should_skip_file(path: &Path, mode: CorpusMode) -> bool {
    if matches!(mode, CorpusMode::Everything) {
        return false;
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        const SKIP_SUFFIXES: &[&str] = &[
            ".tsbuildinfo", ".pid", ".sock", ".swp", ".bak", ".tmp", "~",
        ];
        for s in SKIP_SUFFIXES {
            if name.ends_with(s) {
                return true;
            }
        }
        if name == "package-lock.json"
            || name == "pnpm-lock.yaml"
            || name == "yarn.lock"
            || name == "bun.lockb"
        {
            return true;
        }
    }
    if matches!(mode, CorpusMode::Minimal) {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            const SOURCE_EXTS: &[&str] = &[
                "ts", "tsx", "js", "jsx", "mjs", "cjs",
                "py", "pyx", "pyi",
                "rs", "go", "java", "kt", "kts", "swift",
                "c", "cpp", "cc", "cxx", "h", "hpp", "hxx",
                "cs", "rb", "php", "scala", "sc", "clj", "cljs",
                "ex", "exs", "erl", "hrl", "hs", "lhs", "ml", "mli",
                "lua", "r", "jl", "dart", "vue", "svelte",
                "sh", "bash", "zsh", "fish", "ps1",
                "sql", "graphql", "gql", "proto",
                "md", "mdx", "txt", "rst", "adoc",
            ];
            let ext_lower = ext.to_ascii_lowercase();
            if !SOURCE_EXTS.iter().any(|s| *s == ext_lower.as_str()) {
                return true;
            }
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            const ALLOWED_NO_EXT: &[&str] = &[
                "Makefile", "makefile", "GNUmakefile",
                "Rakefile", "Gemfile", "Vagrantfile",
                "Dockerfile", "Procfile",
                "LICENSE", "LICENCE", "NOTICE",
                "README", "CHANGELOG", "CONTRIBUTING",
                "Cargo.lock", "go.mod", "go.sum",
                "package.json", "tsconfig.json",
                "pyproject.toml", "setup.py", "setup.cfg",
                "requirements.txt", "Pipfile", "Pipfile.lock",
                "pom.xml", "build.gradle", "build.gradle.kts",
                ".gitignore", ".gitattributes", ".editorconfig",
                ".eslintrc", ".eslintrc.js", ".eslintrc.json",
                ".prettierrc", ".prettierrc.js", ".prettierrc.json",
            ];
            if !ALLOWED_NO_EXT.iter().any(|s| *s == name) {
                return true;
            }
        }
    }
    false
}

fn collect_paths(
        root: &Path,
        dir: &Path,
        out: &mut Vec<std::path::PathBuf>,
        skipped_bytes: &mut u64,
    ) -> Result<(), String> {
        // Sprint 5.7.7 hotfix #55: read mode from env.
        // Pragmatic choice: thread it through every
        // callsite would be a much larger diff. The CLI
        // and Tauri command set NEXUS_CORPUS_MODE before
        // invoking this walker.
        let mode = std::env::var("NEXUS_CORPUS_MODE")
            .ok()
            .and_then(|s| s.parse::<CorpusMode>().ok())
            .unwrap_or_default();
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("read_dir({}) failed: {}", dir.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry failed: {}", e))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| format!("file_type({}) failed: {}", path.display(), e))?;
            if file_type.is_dir() {
                if corpus_should_skip_dir(&path, mode) {
                    // Sprint 5.7.2 hotfix #45: report an ESTIMATE for
                    // the skip size, do NOT recurse into the dir.
                    // Recursive walks on dirs like `.claude/worktrees/*`
                    // dominated the wall clock.
                    let est = dir_size_estimate(&path);
                    *skipped_bytes += est;
                    eprintln!(
                        "[WALK-SKIP] {} (est. {} bytes) — dev cache / build artifact",
                        path.display(),
                        est
                    );
                    continue;
                }
                collect_paths(root, &path, out, skipped_bytes)?;
            } else if file_type.is_file() {
                if corpus_should_skip_file(&path, mode) {
                    if let Ok(md) = entry.metadata() {
                        *skipped_bytes += md.len();
                        eprintln!(
                            "[WALK-SKIP] {} ({} KiB) — lock file / build cache",
                            path.display(),
                            md.len() / 1024
                        );
                    }
                    continue;
                }
                out.push(path);
            }
        }
        Ok(())
    }

    let mut skipped_bytes: u64 = 0;
    collect_paths(root, root, &mut paths, &mut skipped_bytes)
        .map_err(|e| ApiError::new("directory.io", e))?;
    if skipped_bytes > 0 {
        eprintln!(
            "[WALK-SKIP] total skipped: {} MiB of dev cache / build artifacts",
            skipped_bytes / (1024 * 1024)
        );
    }

    // Sprint 5.7.2 hotfix #43: persist skipped_bytes for the UI
    // success card (Opción D — diagnostic tooltip).
    crate::stats::record_skipped_bytes(skipped_bytes);

    let n_files = paths.len();
    eprintln!("[WALK-PARALLEL] collected {} paths, reading in parallel", n_files);

    use rayon::prelude::*;
    let root_for_rel = root.to_path_buf();
    let read_results: Result<Vec<(String, Vec<u8>)>, String> = paths
        .par_iter()
        .map(|path| {
            // Sprint 5.7.9: hint the kernel that we'll read
            // this file sequentially. On macOS, fcntl(fd,
            // F_RDAHEAD, 1) turns on aggressive read-ahead
            // for the file descriptor. On a near-full SSD
            // (the M4 Pro at 90% capacity), the kernel's
            // default prefetch is conservative because
            // it tries to preserve free blocks for
            // wear-leveling. F_RDAHEAD bypasses that
            // conservatism for files we know we'll read
            // end-to-end, which on this workload improves
            // throughput 1.5-2x. On Linux we'd use
            // posix_fadvise(fd, 0, 0, POSIX_FADV_SEQUENTIAL)
            // but the project supports macOS only for the
            // GUI build, so we feature-gate it.
            let bytes = read_with_readahead(path)?;
            let rel = path
                .strip_prefix(&root_for_rel)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            Ok((rel, bytes))
        })
        .collect();

    let mut out = read_results.map_err(|e| ApiError::new("directory.io", e))?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    eprintln!("[WALK-PARALLEL] done, {} files loaded ({} MiB total)",
        out.len(),
        out.iter().map(|(_, b)| b.len()).sum::<usize>() / (1024 * 1024)
    );
    // Sprint 5.7.9 part 6: classify the corpus by category so
    // the UI can warn the user when most of the archive is
    // dev cache / pre-compressed binaries (where no codec
    // can do much). The classification is based on path
    // patterns: anything inside `.next/`, `node_modules/`,
    // `venv/`, `target/`, `dist/`, `__pycache__/`, etc. is
    // counted as "build_artifact" (typically already
    // minified/compiled/compressed). Files matching the
    // Rust/JS/TS/Python source extension allow-list are
    // counted as "source". Everything else (configs,
    // data files, docs) is "other".
    let breakdown = classify_corpus_breakdown(&out);
    crate::stats::record_corpus_breakdown(&breakdown);
    Ok((out, skipped_bytes))
}

/// Sprint 5.7.9 part 6: classify a corpus by path pattern.
/// Used by the UI to display a "your corpus is X% build
/// artifacts, switch to Source mode" warning when the
/// user picks Everything mode on a cache-heavy dir.
fn classify_corpus_breakdown(
    files: &[(String, Vec<u8>)],
) -> CorpusBreakdown {
    let mut breakdown = CorpusBreakdown::default();
    const BUILD_ARTIFACT_DIRS: &[&str] = &[
        ".next", "node_modules", "venv", ".venv", "target",
        "dist", "build", "__pycache__", ".turbo", ".swc",
        ".cache", "coverage", ".parcel-cache", ".next-cache",
    ];
    const SOURCE_EXTS: &[&str] = &[
        "ts", "tsx", "js", "jsx", "mjs", "cjs",
        "py", "pyx", "pyi", "rs", "go", "java", "kt", "swift",
        "c", "cpp", "cc", "cxx", "h", "hpp", "hxx",
        "cs", "rb", "php", "scala", "ex", "exs", "erl", "hs",
        "lua", "r", "jl", "dart", "vue", "svelte",
        "sh", "bash", "sql", "graphql", "proto",
    ];
    for (rel, bytes) in files {
        let size = bytes.len() as u64;
        let path = std::path::Path::new(rel);
        let mut in_artifact = false;
        for comp in path.components() {
            if let Some(name) = comp.as_os_str().to_str() {
                if BUILD_ARTIFACT_DIRS.iter().any(|d| *d == name) {
                    in_artifact = true;
                    break;
                }
            }
        }
        if in_artifact {
            breakdown.build_artifact_bytes += size;
            breakdown.build_artifact_files += 1;
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let ext_lower = ext.to_ascii_lowercase();
        if SOURCE_EXTS.iter().any(|s| *s == ext_lower.as_str()) {
            breakdown.source_bytes += size;
            breakdown.source_files += 1;
        } else {
            breakdown.other_bytes += size;
            breakdown.other_files += 1;
        }
    }
    breakdown
}

/// Per-corpus breakdown reported to the UI. Lets the
/// frontend show "your corpus is 87% build artifacts" so
/// the user understands a low ratio on a Next.js /
/// Python-venv project is not a bug.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct CorpusBreakdown {
    pub source_files: u64,
    pub source_bytes: u64,
    pub build_artifact_files: u64,
    pub build_artifact_bytes: u64,
    pub other_files: u64,
    pub other_bytes: u64,
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
    let data = match std::panic::catch_unwind(|| {
        crate::codec::decompress_with_progress(input, |_cumulative| {
            // No-op: per-block progress isn't surfaced for the
            // single-file CLI / API path here. The Tauri
            // command wraps decompress_bytes in a thread that
            // gets called per-block; see `decompress_target_
            // with_progress` for the rich ProgressEvent emission.
            // The codec-level hook is here so the next iteration
            // can pipe it without re-plumbing the codec signature.
        })
    }) {
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

/// Sprint 5.7.2 hotfix #23: decompress with a per-block
/// progress callback. The callback receives the cumulative
/// output bytes restored so far (sum of restored block sizes
/// from the codec). Pass `|_| {}` to ignore progress
/// (semantically identical to the no-callback `decompress_
/// bytes` above).
pub fn decompress_bytes_with_progress<P>(
    input: &[u8],
    mut progress: P,
) -> ApiResult<DecompressResult>
where
    P: FnMut(u64),
{
    if input.is_empty() {
        return Err(ApiError::new(
            "decompress.empty",
            "input is empty; nothing to decompress",
        ));
    }

    let start = Instant::now();
    // The progress closure isn't UnwindSafe by default (RefCell
    // is not), so we AssertUnwindSafe to bypass the check. The
    // actual safety story is: if the codec panics, the progress
    // callback may or may not have been called — that's fine,
    // because we're about to return Err anyway.
    let data = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::codec::decompress_with_progress(input, |cumulative| progress(cumulative))
    })) {
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
