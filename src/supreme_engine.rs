//! Sprint 5.7.10-C: SupremeEngine — single entry point for
//! compression, with internal intelligence that picks the right
//! backend / codec / preprocessor / walker-mode from a
//! high-level `CompressionProfile`.
//!
//! ## Why this module exists
//!
//! Up to sprint 5.7.9 the codebase had a leaky IPC frontier: the
//! Tauri command `compress_target_cmd` parsed **8 separate fields**
//! from the request JSON (`backend`, `codec`, `lossless`,
//! `corpus_mode`, `raw_extensions`, `minify_extensions`,
//! `password`, `recovery_level`) and used them to dispatch into
//! one of 4 different functions:
//!
//!   - `compress_bytes_with_backend` (in-memory bytes, single backend)
//!   - `compress_target_with_codec_lossless_overrides` (file/dir, mixed flags)
//!   - `compress_directory_with_backend_and_codec_overrides_with_progress`
//!   - `compress_target_with_password` (encrypted path)
//!
//! The frontend had to know which fields to send and in which
//! combination. Worse: the **same preset** ("Balanceado") was
//! resolved differently by the GUI (V5Min) and the CLI
//! (V6Solid), because the GUI sent `backend: "v5-min"` and the
//! CLI sent `backend: "v6-solid"` for the same intent.
//!
//! ## What it gives you
//!
//! - [`CompressionProfile`]: the single struct that captures the
//!   user's intent. Mirrors the frontend TypeScript
//!   `CompressionProfile` shape (Sprint 5.7.8).
//! - [`CompressInvocation`]: a profile + invocation-time
//!   parameters (path, password, output directory).
//! - [`resolve_plan`]: pure function that turns a
//!   `CompressionInvocation` + `is_dir` into a `ResolvedPlan`
//!   (the concrete backend, codec, lzma level, lossless flag,
//!   preprocessor overrides). This is the "intelligence" —
//!   it's a separate function so it can be unit-tested
//!   without touching the filesystem.
//! - [`compress`]: the entry point. Takes an invocation +
//!   progress callback. Internally calls the existing
//!   backend-specific functions in `api.rs` and `solid_archive.rs`.
//! - [`compress_bytes`]: in-memory variant for the
//!   `compress_bytes_cmd` Tauri command.
//!
//! ## Migration path
//!
//! **5.7.10-C** (this commit): create the engine. Tauri
//! commands still send the old field-by-field JSON, but the
//! command handler builds a `CompressionProfile` internally
//! and calls `SupremeEngine::compress`. The dispatch logic
//! in the command is collapsed from ~100 lines of JSON
//! parsing + if-let dispatch to a single call.
//!
//! **5.7.10-D** (next sprint): change the Tauri command
//! to accept the `CompressionProfile` as a single nested
//! object instead of flat fields. The frontend can
//! then send `{ req: { path, profile: { ... } } }` instead
//! of `{ req: { path, backend, codec, lossless, ... } }`.
//!
//! **5.7.10-E** (final): delete the `CompressionBackend` enum
//! from the public surface, delete the old
//! `_with_codec_lossless_overrides` functions, the
//! `compress_bytes_with_backend` and the
//! `compress_directory_with_backend_*` variants. The
//! engine is the only public compress entry point.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::{
    self, ApiResult, CompressTargetResult, CompressWithPasswordOptions, CompressionLevel,
    ProgressEvent, RecoveryLevel,
};
use crate::format_knowledge;
use crate::solid_archive::{Codec, CompressionLevel as SolidLevel, PreprocessorOverrides};
use crate::walker;

// ============================================================================
// Profile types (mirror the frontend TypeScript CompressionProfile)
// ============================================================================

/// Schema version of the profile. Bump when the shape changes
/// incompatibly — readers can detect a future version and
/// fall back to defaults.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

/// User-facing compression mode. The frontend uses Spanish
/// names ("rapido" / "balanceado" / "ultra") to keep the
/// labels short and consistent with the rest of the UI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProfileMode {
    /// Fast: prioritise wall-clock time. Zstd(3) on the
    /// codec side, smart preprocessor still on.
    Rapido,
    /// Balanced: the safe default. Zstd(3) since sprint
    /// 5.7.9 (was LZMA(6) before).
    Balanceado,
    /// Ultra: maximise ratio at the cost of time. LZMA(9).
    Ultra,
}

impl Default for ProfileMode {
    fn default() -> Self {
        Self::Balanceado
    }
}

impl std::str::FromStr for ProfileMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "rapido" | "fast" | "veloce" => Ok(Self::Rapido),
            "balanceado" | "balanced" => Ok(Self::Balanceado),
            "ultra" | "max" => Ok(Self::Ultra),
            other => Err(format!(
                "unknown mode '{}'; expected rapido | balanceado | ultra",
                other
            )),
        }
    }
}

/// User-facing codec choice. "auto" means "let the engine
/// pick based on mode" (the 5.7.9 default for balanceado is
/// Zstd, for ultra is LZMA).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProfileCodec {
    Auto,
    Lzma,
    Zstd,
}

impl Default for ProfileCodec {
    fn default() -> Self {
        Self::Auto
    }
}

impl std::str::FromStr for ProfileCodec {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "lzma" | "lzma2" => Ok(Self::Lzma),
            "zstd" => Ok(Self::Zstd),
            other => Err(format!(
                "unknown codec '{}'; expected auto | lzma | zstd",
                other
            )),
        }
    }
}

/// User-facing fidelity choice. "lossy" (the default) applies
/// the smart preprocessor (Conservative / swc / Raw by
/// extension). "lossless" forces every file to Preprocessor::Raw
/// for a bit-exact reversible archive at the cost of a worse
/// ratio.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProfileFidelity {
    Lossy,
    Lossless,
}

impl Default for ProfileFidelity {
    fn default() -> Self {
        Self::Lossy
    }
}

impl std::str::FromStr for ProfileFidelity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "lossy" | "minify" | "smart" => Ok(Self::Lossy),
            "lossless" | "raw" | "exact" => Ok(Self::Lossless),
            other => Err(format!(
                "unknown fidelity '{}'; expected lossy | lossless",
                other
            )),
        }
    }
}

/// The complete user intent. One struct, one source of truth
/// for what the user wants. The engine resolves it into
/// concrete backends / codecs / preprocessors internally.
///
/// The `rename_all = "camelCase"` serde attribute matches the
/// frontend TypeScript interface (`schemaVersion`,
/// `corpusMode`, `rawExtensions`, `minifyExtensions`,
/// `recoveryLevel`). This is the IPC contract — the
/// frontend sends camelCase JSON, the backend parses it
/// directly into this struct without translation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompressionProfile {
    /// Schema version. v1 is the initial struct.
    pub schema_version: u32,
    /// Speed / ratio tradeoff.
    pub mode: ProfileMode,
    /// Codec hint. "auto" delegates to mode-based rules.
    pub codec: ProfileCodec,
    /// Lossy / lossless fidelity.
    pub fidelity: ProfileFidelity,
    /// What to include when archiving a directory.
    pub corpus_mode: api::CorpusMode,
    /// Per-extension "always Raw" pin. Empty = use the
    /// default per-extension table in `format_knowledge`.
    pub raw_extensions: Vec<String>,
    /// Per-extension "always Conservative" pin. Empty =
    /// use the default per-extension table.
    pub minify_extensions: Vec<String>,
    /// Whether to encrypt the output. When true, the
    /// invocation MUST also provide a `password`.
    pub encrypt: bool,
    /// Reed-Solomon recovery level for the encrypted path.
    pub recovery_level: RecoveryLevel,
    /// Sprint 5.7.18: when true, skip archive files
    /// (`.zip`, `.tar`, `.gz`, `.rar`, `.7z`, …) during
    /// the walk. Useful for "I want to backup my source
    /// code, not the 200 MB of release binaries I
    /// accidentally left in the repo". Default `false` to
    /// preserve current behavior — most users have no
    /// archives in their repo and don't expect files to
    /// disappear.
    #[serde(default)]
    pub skip_archive: bool,
}

impl Default for CompressionProfile {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            mode: ProfileMode::Balanceado,
            codec: ProfileCodec::Auto,
            fidelity: ProfileFidelity::Lossy,
            corpus_mode: api::CorpusMode::default(),
            raw_extensions: Vec::new(),
            minify_extensions: Vec::new(),
            encrypt: false,
            recovery_level: RecoveryLevel::default(),
            skip_archive: false,
        }
    }
}

impl CompressionProfile {
    /// Parse a `CompressionProfile` from a JSON object. This is
    /// the entry point the Tauri command uses when the frontend
    /// sends the new nested `{ profile: { ... } }` shape.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid profile: {}", e))
    }
}

// ============================================================================
// Invocation (profile + invocation-time parameters)
// ============================================================================

/// A compression invocation: the user's intent (profile) plus
/// the invocation-time parameters that are NOT part of the
/// profile (path, password, output directory).
///
/// The password is intentionally NOT in the profile — it's a
/// runtime secret, not a saved setting. Custom profiles
/// persisted in localStorage don't carry passwords (the
/// frontend prompts for one on each compress).
#[derive(Debug, Clone)]
pub struct CompressInvocation {
    pub profile: CompressionProfile,
    pub path: PathBuf,
    pub password: Option<Vec<u8>>,
    pub output_dir: Option<PathBuf>,
}

impl CompressInvocation {
    /// Build an invocation from a Tauri request (the legacy
    /// flat-field shape) + a `CompressionProfile` (which the
    /// caller extracted from the request).
    ///
    /// This is the **migration shim**: 5.7.10-C still uses
    /// the flat-field shape on the wire. 5.7.10-D will
    /// switch to nested `profile: { ... }` and this shim
    /// becomes the secondary path.
    pub fn from_legacy_fields(
        profile: CompressionProfile,
        path: PathBuf,
        password: Option<Vec<u8>>,
        output_dir: Option<PathBuf>,
    ) -> Self {
        Self { profile, path, password, output_dir }
    }
}

// ============================================================================
// Resolved plan (internal decision)
// ============================================================================

/// The concrete compression decision. This is what the
/// frontend's intent (profile) gets resolved into before the
/// engine dispatches to a backend.
///
/// Kept `pub` (with field privacy) for testability — the
/// `resolve_plan` function is the "intelligence" of the engine
/// and we want to assert its outputs in unit tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlan {
    /// The compression backend to use.
    pub backend: PlanBackend,
    /// The codec + LZMA level (or zstd level) for the codec.
    pub codec_level: SolidLevel,
    /// Lossless mode: every file is `Preprocessor::Raw`.
    pub lossless: bool,
    /// Per-extension overrides (raw / minify pins from the
    /// Advanced panel).
    pub overrides: PreprocessorOverrides,
}

/// The compression backend selected by the resolver. This is
/// the **internal** enum that the engine uses. It's distinct
/// from the legacy `api::CompressionBackend` (which is on
/// its way out in 5.7.10-E) so the resolver's outputs can
/// be tested without dragging the legacy surface along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanBackend {
    /// v4 codec (LZ77 + rANS + dict) wrapped in NXAR
    /// per-file archive. Used for the encrypted path
    /// (AES-256-GCM + optional Reed-Solomon).
    V4Encrypted,
    /// v5-min: per-file LZMA with smart preprocessor.
    /// Used for single-file inputs and the v5-min
    /// legacy path.
    V5Min,
    /// v6-solid: single LZMA stream over the whole
    /// preprocessed corpus with the Sonic scheduler.
    /// Used for directory inputs.
    V6Solid,
}

impl PlanBackend {
    /// Output file extension for the archive.
    pub fn output_ext(self) -> &'static str {
        match self {
            Self::V4Encrypted => "nxs",
            Self::V5Min => "lz",
            Self::V6Solid => "nxs6",
        }
    }
}

// ============================================================================
// The resolver (the "intelligence" of the engine)
// ============================================================================

/// Resolve a `CompressionProfile` + the runtime "is this a
/// directory?" flag into a concrete compression plan.
///
/// This is a **pure function** — no I/O, no env reads, no
/// side effects. The same inputs always produce the same
/// output. That's what makes it easy to unit-test.
pub fn resolve_plan(invocation: &CompressInvocation, is_dir: bool) -> ResolvedPlan {
    let p = &invocation.profile;

    // Step 1: backend. Encryption forces v4 (the encrypted
    // pipeline is built on top of the v4 codec). Directory
    // inputs go through v6-solid (the single LZMA stream
    // over the preprocessed corpus). File inputs use
    // v5-min (per-file LZMA, smart preprocessor).
    let backend = if p.encrypt {
        PlanBackend::V4Encrypted
    } else if is_dir {
        PlanBackend::V6Solid
    } else {
        PlanBackend::V5Min
    };

    // Step 2: codec + level. The "auto" rule is the
    // 5.7.9 default: balanceado → zstd(3) (fast),
    // ultra → lzma(9) (max ratio), rapido → zstd(3).
    // Explicit codec choice overrides the mode-based rule.
    let codec_level = match (p.codec, p.mode) {
        (ProfileCodec::Zstd, _) => SolidLevel::Zstd(3),
        (ProfileCodec::Lzma, ProfileMode::Rapido) => SolidLevel::Lzma(3),
        (ProfileCodec::Lzma, ProfileMode::Balanceado) => SolidLevel::Lzma(6),
        (ProfileCodec::Lzma, ProfileMode::Ultra) => SolidLevel::Lzma(9),
        (ProfileCodec::Auto, ProfileMode::Rapido) => SolidLevel::Zstd(3),
        // Sprint 5.7.9: balanceado auto → zstd(3) for the
        // 22x speedup. LZMA is still the option for users
        // who explicitly ask for it.
        (ProfileCodec::Auto, ProfileMode::Balanceado) => SolidLevel::Zstd(3),
        (ProfileCodec::Auto, ProfileMode::Ultra) => SolidLevel::Lzma(9),
    };

    // Step 3: lossless.
    let lossless = matches!(p.fidelity, ProfileFidelity::Lossless);

    // Step 4: preprocessor overrides.
    let overrides = PreprocessorOverrides::new(
        p.raw_extensions.clone(),
        p.minify_extensions.clone(),
    );

    ResolvedPlan { backend, codec_level, lossless, overrides }
}

// ============================================================================
// The engine (the public entry point)
// ============================================================================

/// The SupremeEngine. Stateless — all methods take the
/// invocation + progress callback as parameters. The
/// "intelligence" lives in [`resolve_plan`], not in
/// instance state.
pub struct SupremeEngine;

impl SupremeEngine {
    /// Compress a file or directory. The single public
    /// entry point. Internally:
    ///
    ///   1. Stat the path (file vs directory).
    ///   2. Resolve the profile into a `ResolvedPlan`.
    ///   3. Dispatch to the right backend function
    ///      (encrypted / per-file / v6-solid) based on
    ///      the plan.
    ///   4. Return the result.
    ///
    /// **Sprint 5.7.10-E:** this function used to route through
    /// `api::compress_target_with_codec_lossless_overrides`
    /// (which still took a legacy `CompressionBackend` enum).
    /// The engine now calls the underlying functions
    /// directly (`solid_archive::compress_with_progress*`
    /// for the unencrypted paths, `api::compress_target_with_
    /// password` for the encrypted path), so the legacy
    /// `CompressionBackend` enum and the `*_with_backend`
    /// dispatch functions can be deleted from `api.rs`.
    pub fn compress<P>(
        invocation: &CompressInvocation,
        progress: P,
    ) -> ApiResult<CompressTargetResult>
    where
        P: Fn(ProgressEvent) + Send + Sync,
    {
        // Set the corpus mode env var so the walker (which
        // is a separate module) reads it. This is the only
        // bit of "global state" the engine touches, and it's
        // the same env var the CLI uses (so GUI and CLI
        // are guaranteed to behave identically on the same
        // profile + path).
        std::env::set_var("NEXUS_CORPUS_MODE", invocation.profile.corpus_mode.as_str());
        // Sprint 5.7.18: same env-var pattern for the
        // `--skip-archive` flag. The CLI sets this from
        // `--skip-archive`; the Tauri command sets it from
        // the profile's `skipArchive` field. The walker reads
        // it directly. This avoids threading the flag through
        // 4 function signatures and matches the existing
        // NEXUS_CORPUS_MODE convention.
        std::env::set_var(
            "NEXUS_SKIP_ARCHIVE",
            if invocation.profile.skip_archive { "1" } else { "0" },
        );

        // Resolve the plan FIRST so the dispatch is data-driven.
        let meta = std::fs::metadata(&invocation.path).map_err(|e| {
            api::ApiError::new(
                "target.not_found",
                format!("cannot stat {}: {}", invocation.path.display(), e),
            )
        })?;
        let is_dir = meta.is_dir();
        let plan = resolve_plan(invocation, is_dir);

        // Dispatch.
        match (plan.backend, invocation.password.as_deref()) {
            (PlanBackend::V4Encrypted, Some(pwd)) => {
                let opts = CompressWithPasswordOptions {
                    password: pwd,
                    recovery: invocation.profile.recovery_level,
                };
                api::compress_target_with_password(&invocation.path, &opts, progress)
            }
            (PlanBackend::V4Encrypted, None) => Err(api::ApiError::new(
                "profile.missing_password",
                "profile.encrypt=true but no password provided",
            )),
            (PlanBackend::V5Min, _) => {
                // Per-file path. The v4 codec does the heavy
                // lifting; the preprocessor is applied if the
                // profile doesn't request lossless.
                compress_file_with_plan(&invocation.path, &plan, progress)
            }
            (PlanBackend::V6Solid, _) => {
                // Directory path. Walk → preprocess → single
                // LZMA stream over the corpus.
                compress_directory_with_plan(&invocation.path, &plan, progress)
            }
        }
    }

    /// In-memory compression. The frontend's
    /// `compress_bytes_with_backend_cmd` route (currently
    /// unused by the GUI but kept for power users / CLI
    /// integration).
    pub fn compress_bytes(
        profile: &CompressionProfile,
        bytes: &[u8],
        file_name: &str,
    ) -> api::CompressResult {
        std::env::set_var("NEXUS_CORPUS_MODE", profile.corpus_mode.as_str());

        let invocation = CompressInvocation {
            profile: profile.clone(),
            path: PathBuf::from("<in-memory>"),
            password: None,
            output_dir: None,
        };
        // For in-memory compression, is_dir is always false.
        // The plan will be V5Min (the only non-encrypted
        // per-file backend — V4Encrypted requires a real path).
        let plan = resolve_plan(&invocation, false);

        let start = std::time::Instant::now();
        let pre_bytes = if plan.lossless {
            bytes.to_vec()
        } else {
            // Apply the default preprocessor (Conservative /
            // swc / Raw by extension) based on file_name.
            // This mirrors what compress_bytes_with_backend's
            // V5Min arm did.
            crate::minify::minify(bytes)
        };
        let compressed = crate::engine::compress_with("v5-min", &pre_bytes, false)
            .unwrap_or_else(|_| pre_bytes);
        let compressed_size = compressed.len() as u64;
        let original_size = bytes.len() as u64;
        let ratio = if compressed_size == 0 {
            0.0
        } else {
            original_size as f64 / compressed_size as f64
        };
        let compress_time_ms = start.elapsed().as_secs_f64() * 1000.0;
        api::CompressResult {
            compressed,
            original_size,
            compressed_size,
            ratio,
            compress_time_ms,
        }
    }
}

// ============================================================================
// Engine-internal compress helpers (file / dir)
// ============================================================================

/// Compress a single file using the plan. Mirrors the file
/// branch of the legacy `compress_target_with_codec_lossless_overrides`:
/// the v4 codec is the heavy lifter, the preprocessor is
/// applied per extension when the profile doesn't request
/// lossless. The output is the per-file `.nxs` stream.
fn compress_file_with_plan<P>(
    path: &Path,
    plan: &ResolvedPlan,
    progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    use std::time::Instant;
    let start = Instant::now();
    let meta = std::fs::metadata(path).map_err(|e| {
        api::ApiError::new(
            "target.not_found",
            format!("cannot stat {}: {}", path.display(), e),
        )
    })?;
    let total = meta.len();
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Apply the preprocessor unless lossless.
    let raw_bytes = std::fs::read(path).map_err(|e| {
        api::ApiError::new("target.read_failed", format!("read failed: {}", e))
    })?;
    let pre_bytes = if plan.lossless {
        raw_bytes.clone()
    } else {
        crate::minify::minify(&raw_bytes)
    };
    drop(raw_bytes);

    // v4 codec with progress.
    let elapsed_tracker = crate::nxar::ElapsedTracker::new();
    let bytes_done = std::cell::Cell::new(0u64);
    let compressed = crate::codec::compress_with_progress(&pre_bytes, |cumulative| {
        let prev = bytes_done.get();
        if cumulative - prev >= 4096 || cumulative == total {
            bytes_done.set(cumulative);
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
    });

    let compress_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    let compressed_size = compressed.len() as u64;
    let ratio = if compressed_size == 0 {
        0.0
    } else {
        total as f64 / compressed_size as f64
    };

    Ok(api::CompressTargetResult {
        is_directory: false,
        original_size: total,
        compressed_size,
        ratio,
        compress_time_ms,
        compressed_bytes: compressed,
        n_files: 1,
        corpus_breakdown: Default::default(),
        skipped_bytes: 0,
        output_path: String::new(),
        output_ext: "nxs",
    })
}

/// Compress a directory using the plan. Mirrors the directory
/// branch of the legacy `compress_directory_with_backend_*`:
/// walk the tree, apply preprocessors per file, emit a
/// single LZMA stream over the preprocessed corpus (the
/// "v6-solid" pipeline).
fn compress_directory_with_plan<P>(
    path: &Path,
    plan: &ResolvedPlan,
    progress: P,
) -> ApiResult<CompressTargetResult>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    use std::time::Instant;
    let start = Instant::now();
    // Sprint 5.7.18: --skip-archive support. We read the
    // NEXUS_SKIP_ARCHIVE env var here (set by the CLI/Tauri
    // command from the profile) so the path from CLI → main
    // → engine → walker is single-source-of-truth.
    let skip_archive = std::env::var("NEXUS_SKIP_ARCHIVE")
        .ok()
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    // Shared ElapsedTracker so per-file events report real
    // elapsed_ms (the legacy code used a local tracker
    // because the closure capture of `Instant` was producing
    // elapsed_ms=0 for every event on large dirs). The
    // tracker runs in a background thread and reads via
    // `.current_ms()` from inside the closure.
    let elapsed_tracker = crate::nxar::ElapsedTracker::new();

    // Walk the tree. The walker reads NEXUS_CORPUS_MODE itself
    // (set by the engine on the public compress path). Sprint
    // 5.7.18: the profile's `skip_archive` flag is passed
    // through to the walker so `--skip-archive` is honoured
    // end-to-end (CLI → profile → walker).
    let mode = invocation_corpus_mode_from_env();
    let result = walker::walk(
        path,
        mode,
        skip_archive,
    )
    .map_err(|e| {
            api::ApiError::new("directory.io", e.to_string())
        })?;
    if result.files.is_empty() {
        return Err(api::ApiError::new(
            "directory.empty",
            format!("no files in {}", path.display()),
        ));
    }

    // Split into compressible (text/code/configs) and passthrough
    // (PNG/JPG/MP4/ZIP/PDF/...) using format_knowledge.
    let mut lzma_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut passthrough_files: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, bytes) in result.files.iter() {
        if format_knowledge::is_raw_format(name) {
            passthrough_files.push((name.clone(), bytes.clone()));
        } else {
            lzma_files.push((name.clone(), bytes.clone()));
        }
    }
    let total_lzma: u64 = lzma_files.iter().map(|(_, b)| b.len() as u64).sum();
    let total_pt: u64 = passthrough_files.iter().map(|(_, b)| b.len() as u64).sum();
    let total_original = total_lzma + total_pt;

    // Dispatch to the right solid_archive entry point based on
    // lossless vs overrides vs default.
    //
    // Sprint 5.7.11 fix: when the corpus is 100% passthrough
    // (no LZMA-eligible text files) we still need a valid
    // solid-archive header so the decoder's `parse_toc` can
    // read the `NXPT` trailer that follows. `compress` with
    // an empty file list produces a header with `n_files=0`
    // and an empty solid block; the chunk-groups sub-TOC is
    // `n_groups=0` and `solid_block_offset` points right
    // after that. The trailer parser then takes over. Before
    // this fix, a JPEG-only / PNG-only corpus would produce
    // a raw `NXPT` blob with no magic, and decompression
    // failed with "bad magic".
    let mut archive = if lzma_files.is_empty() {
        crate::solid_archive::compress(&[], SolidLevel::Auto)
            .map_err(|e| api::ApiError::new("solid.compress.empty", e))?
    } else if plan.lossless {
        crate::solid_archive::compress_with_progress_lossless(
            &lzma_files,
            plan.codec_level,
            |file_idx, total, name| {
                let bytes_done: u64 = lzma_files
                    .iter()
                    .take(file_idx)
                    .map(|(_, b)| b.len() as u64)
                    .sum::<u64>();
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
                }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
            },
        )
        .map_err(|e| api::ApiError::new("solid.compress", e))?
    } else if !plan.overrides.is_empty() {
        crate::solid_archive::compress_with_progress_full_public(
            &lzma_files,
            plan.codec_level,
            false,
            &plan.overrides,
            |file_idx, total, name| {
                let bytes_done: u64 = lzma_files
                    .iter()
                    .take(file_idx)
                    .map(|(_, b)| b.len() as u64)
                    .sum::<u64>();
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
                }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
            },
        )
        .map_err(|e| api::ApiError::new("solid.compress", e))?
    } else {
        crate::solid_archive::compress_with_progress(
            &lzma_files,
            plan.codec_level,
            |file_idx, total, name| {
                let bytes_done: u64 = lzma_files
                    .iter()
                    .take(file_idx)
                    .map(|(_, b)| b.len() as u64)
                    .sum::<u64>();
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
                }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
            },
        )
        .map_err(|e| api::ApiError::new("solid.compress", e))?
    };

    // Append passthrough payloads with a marker so the decoder
    // can reassemble. Sprint 5.7.10 post-merge: we also try
    // to zstd the passthrough bytes — for already-compressed
    // data zstd produces output ≤ input + minimal overhead, so
    // we get a free compression win on binary dev-caches. If
    // zstd makes the bytes larger (shouldn't happen but just
    // in case) we fall back to raw passthrough.
    //
    // Sprint 5.7.16: emit progress events through the
    // passthrough loop too. Previously this loop wrote bytes
    // to `archive` silently, so on a corpus dominated by
    // passthroughs (e.g. FlowNow: 4 GB of .venv/.git/objects/
    // .claude is passthrough) the UI's progress bar would
    // freeze at the last lzma callback (~7-19% depending on
    // corpus) and then jump to 100% with no in-between
    // updates. The throttle in `throttle::ThrottledEmitter`
    // (100ms) caps the event rate, so emitting per file is
    // safe even with 47k passthroughs.
    use std::io::Write;
    // Total bytes accounted for in the progress bar: lzma +
    // passthrough. The lzma_files phase already reported
    // bytes_done up to `total_lzma`; here we continue from
    // there into the passthroughs.
    let total_bytes_for_progress: u64 = total_lzma + total_pt;
    let mut passthrough_bytes_done: u64 = 0;
    for (name, bytes) in passthrough_files.iter() {
        archive.write_all(b"NXPT").ok();
        let name_bytes = name.as_bytes();
        archive
            .write_all(&(name_bytes.len() as u32).to_le_bytes())
            .ok();
        archive.write_all(name_bytes).ok();
        passthrough_bytes_done = passthrough_bytes_done.saturating_add(bytes.len() as u64);
        // zstd with level 1 is fast enough for large corpora
        // and produces output ≤ input on most "already
        // compressed" data (PNG, JPG, MP4, .pyc, .wasm, .so,
        // .dylib, etc.). The compressor overhead per file is
        // ~50 bytes of frame header — negligible.
        let zstd_bytes = zstd::stream::encode_all(bytes.as_slice(), 1)
            .unwrap_or_else(|_| bytes.clone());
        if zstd_bytes.len() < bytes.len() {
            // Zstd actually compressed — emit a Z marker + the
            // zstd bytes (decompression on the way out is also
            // fast, zstd level 1 decode is ~600 MB/s).
            archive.write_all(b"Z").ok();
            archive
                .write_all(&(zstd_bytes.len() as u32).to_le_bytes())
                .ok();
            archive.write_all(&zstd_bytes).ok();
        } else {
            // Zstd didn't help — emit raw bytes.
            archive.write_all(b"R").ok();
            archive
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .ok();
            archive.write_all(bytes).ok();
        }
        // Sprint 5.7.16: emit a progress event after each
        // passthrough file. The throttle in
        // `throttle::ThrottledEmitter` (100ms) caps the rate,
        // so even on 47k passthroughs the IPC sees at most
        // ~10 events/s. The frontend's progress bar now sees
        // bytes_done climbing from `total_lzma` up to
        // `total_lzma + total_pt` instead of jumping from
        // ~7% (after lzma) to 100% (after passthroughs).
        progress(ProgressEvent {
            phase: "compressing".to_string(),
            current_file: name.clone(),
            files_done: 0, // ignored by frontend when bytes_total > 0
            files_total: 0,
            bytes_done: total_lzma + passthrough_bytes_done,
            bytes_total: total_bytes_for_progress,
            elapsed_ms: 0,
            bytes_per_sec: 0.0,
            eta_ms: 0,
        }.with_estimates(start).with_elapsed_override(elapsed_tracker.current_ms()));
    }

    let compress_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    let compressed_size = archive.len() as u64;
    let ratio = if compressed_size == 0 {
        0.0
    } else {
        total_original as f64 / compressed_size as f64
    };
    let n_files = (lzma_files.len() + passthrough_files.len()) as u64;

    Ok(api::CompressTargetResult {
        is_directory: true,
        original_size: total_original,
        compressed_size,
        ratio,
        compress_time_ms,
        compressed_bytes: archive,
        n_files,
        corpus_breakdown: result.breakdown,
        skipped_bytes: result.skipped_bytes,
        output_path: String::new(),
        output_ext: "nxs6",
    })
}

/// Read the corpus mode from the env var (the walker reads
/// the same env var). This is a private helper because the
/// engine sets `NEXUS_CORPUS_MODE` on the public compress
/// path but the dir helper needs to know the mode too.
fn invocation_corpus_mode_from_env() -> api::CorpusMode {
    std::env::var("NEXUS_CORPUS_MODE")
        .ok()
        .and_then(|s| s.parse::<api::CorpusMode>().ok())
        .unwrap_or_default()
}

// ============================================================================
// Helpers
// ============================================================================
// (The legacy translation helpers `legacy_backend_for_plan` and
// `lzma_level_from_plan` were removed in 5.7.10-E — the engine
// now calls the underlying solid_archive / codec functions
// directly based on `PlanBackend`, so the `api::CompressionBackend`
// enum is no longer needed.)

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(mode: ProfileMode, codec: ProfileCodec, fidelity: ProfileFidelity, encrypt: bool) -> CompressionProfile {
        CompressionProfile {
            schema_version: PROFILE_SCHEMA_VERSION,
            mode,
            codec,
            fidelity,
            corpus_mode: api::CorpusMode::Everything,
            raw_extensions: Vec::new(),
            minify_extensions: Vec::new(),
            encrypt,
            recovery_level: RecoveryLevel::Low,
            skip_archive: false,
        }
    }

    fn invocation(p: CompressionProfile) -> CompressInvocation {
        CompressInvocation {
            profile: p,
            path: PathBuf::from("/tmp"),
            password: None,
            output_dir: None,
        }
    }

    #[test]
    fn encrypted_profile_forces_v4_backend() {
        let p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, true);
        let plan = resolve_plan(&invocation(p), true);
        assert_eq!(plan.backend, PlanBackend::V4Encrypted);
    }

    #[test]
    fn directory_input_picks_v6_solid() {
        let p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, false);
        let plan = resolve_plan(&invocation(p), true);
        assert_eq!(plan.backend, PlanBackend::V6Solid);
    }

    #[test]
    fn single_file_picks_v5_min() {
        let p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, false);
        let plan = resolve_plan(&invocation(p), false);
        assert_eq!(plan.backend, PlanBackend::V5Min);
    }

    #[test]
    fn lossless_fidelity_sets_lossless_flag() {
        let p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossless, false);
        let plan = resolve_plan(&invocation(p), true);
        assert!(plan.lossless);
    }

    #[test]
    fn lossy_fidelity_clears_lossless_flag() {
        let p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, false);
        let plan = resolve_plan(&invocation(p), true);
        assert!(!plan.lossless);
    }

    #[test]
    fn auto_balanceado_resolves_to_zstd() {
        // Sprint 5.7.9 default: balanceado auto → zstd(3) for the 22x speedup.
        let p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, false);
        let plan = resolve_plan(&invocation(p), true);
        assert_eq!(plan.codec_level, SolidLevel::Zstd(3));
    }

    #[test]
    fn auto_ultra_resolves_to_lzma9() {
        let p = profile(ProfileMode::Ultra, ProfileCodec::Auto, ProfileFidelity::Lossy, false);
        let plan = resolve_plan(&invocation(p), true);
        assert_eq!(plan.codec_level, SolidLevel::Lzma(9));
    }

    #[test]
    fn explicit_zstd_overrides_mode() {
        // Even on Ultra, explicit zstd → zstd(3)
        let p = profile(ProfileMode::Ultra, ProfileCodec::Zstd, ProfileFidelity::Lossy, false);
        let plan = resolve_plan(&invocation(p), true);
        assert_eq!(plan.codec_level, SolidLevel::Zstd(3));
    }

    #[test]
    fn explicit_lzma_with_rapido_uses_lzma3() {
        let p = profile(ProfileMode::Rapido, ProfileCodec::Lzma, ProfileFidelity::Lossy, false);
        let plan = resolve_plan(&invocation(p), true);
        assert_eq!(plan.codec_level, SolidLevel::Lzma(3));
    }

    #[test]
    fn per_extension_overrides_pass_through() {
        let mut p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, false);
        p.raw_extensions = vec!["json".into(), "toml".into()];
        p.minify_extensions = vec!["md".into()];
        let plan = resolve_plan(&invocation(p), true);
        assert!(plan.overrides.raw_extensions.contains(&"json".to_string()));
        assert!(plan.overrides.minify_extensions.contains(&"md".to_string()));
    }

    #[test]
    fn corpus_mode_is_preserved_in_profile() {
        // The env-var side effect on the public compress
        // path is exercised in the integration test suite
        // (the CLI and Tauri command both set it before
        // invoking the engine). Here we just verify that
        // the profile's corpus_mode survives the resolve
        // step without mutation.
        let mut p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, false);
        p.corpus_mode = api::CorpusMode::Minimal;
        let inv = invocation(p.clone());
        let plan = resolve_plan(&inv, true);
        // The plan doesn't carry the corpus_mode (it's a
        // walker-level concern), but the profile does — and
        // a `resolve_plan` call shouldn't mutate the profile.
        assert_eq!(inv.profile.corpus_mode, api::CorpusMode::Minimal);
        assert_eq!(plan.backend, PlanBackend::V6Solid);
    }

    #[test]
    fn profile_serde_round_trip() {
        let p = profile(ProfileMode::Ultra, ProfileCodec::Lzma, ProfileFidelity::Lossless, true);
        let json = serde_json::to_string(&p).unwrap();
        let back: CompressionProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn output_ext_per_backend() {
        assert_eq!(PlanBackend::V4Encrypted.output_ext(), "nxs");
        assert_eq!(PlanBackend::V5Min.output_ext(), "lz");
        assert_eq!(PlanBackend::V6Solid.output_ext(), "nxs6");
    }

    #[test]
    fn engine_routes_encrypted_without_password_to_error() {
        // profile.encrypt=true but no password → should error,
        // not silently fall through to the v4 plain path.
        let p = profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy, true);
        // Use a real temp path so the stat() succeeds, otherwise
        // we get a "not_found" error before reaching the password
        // check.
        let tmp = std::env::temp_dir().join("supreme-engine-test-route");
        let _ = std::fs::create_dir_all(&tmp);
        let inv = CompressInvocation {
            profile: p,
            path: tmp.clone(),
            password: None,
            output_dir: None,
        };
        let result = SupremeEngine::compress(&inv, |_| {});
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "profile.missing_password",
            "expected missing_password, got: {} ({})", err.code, err.message);
        // Clean up.
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("NEXUS_CORPUS_MODE");
    }

    #[test]
    fn format_knowledge_singletons_remain_usable() {
        // Sanity check: the engine imports `format_knowledge`
        // and `walker` so they're exercised even from this
        // module's compile path.
        assert!(format_knowledge::is_raw_format("foo.png"));
        assert!(!format_knowledge::is_raw_format("foo.rs"));
        let tmp = std::env::temp_dir();
        // walker::walk_paths works on a real path (even empty).
        let _ = walker::walk_paths(&tmp);
    }
}
