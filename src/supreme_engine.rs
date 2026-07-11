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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    ///      (encrypted / v5-min / v6-solid) based on
    ///      the plan.
    ///   4. Return the result.
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

        // Resolve the plan FIRST so the dispatch is data-driven
        // (the only `if` is "does the profile encrypt?" and
        // "is the path a directory?" — both checked via the
        // plan).
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
                // Single-file path: legacy `compress_target_with_codec_lossless_overrides`
                // covers both file and directory cases. For a
                // single file, the backend is forced to per-file
                // by the `is_dir` flag in the plan.
                let legacy_backend = legacy_backend_for_plan(plan.backend);
                let codec_override = match plan.codec_level.codec() {
                    Codec::Lzma => None, // v5-min uses the LZMA level, not the toggle
                    Codec::Zstd => Some(Codec::Zstd),
                };
                api::compress_target_with_codec_lossless_overrides(
                    &invocation.path,
                    legacy_backend,
                    lzma_level_from_plan(&plan.codec_level),
                    invocation.output_dir.as_deref(),
                    codec_override,
                    plan.lossless,
                    &plan.overrides,
                    progress,
                )
            }
            (PlanBackend::V6Solid, _) => {
                let legacy_backend = legacy_backend_for_plan(plan.backend);
                let codec_override = match plan.codec_level {
                    SolidLevel::Zstd(_) => Some(Codec::Zstd),
                    _ => None, // LZMA explicit → use the v6-solid LZMA path
                };
                api::compress_target_with_codec_lossless_overrides(
                    &invocation.path,
                    legacy_backend,
                    lzma_level_from_plan(&plan.codec_level),
                    invocation.output_dir.as_deref(),
                    codec_override,
                    plan.lossless,
                    &plan.overrides,
                    progress,
                )
            }
        }
    }

    /// In-memory compression. The frontend's
    /// `compress_bytes_cmd` route.
    pub fn compress_bytes(
        profile: &CompressionProfile,
        bytes: &[u8],
        file_name: &str,
    ) -> api::CompressResult {
        // Set the corpus mode so any future walker call
        // (e.g. for in-memory dictionary training) sees the
        // same mode the profile specifies.
        std::env::set_var("NEXUS_CORPUS_MODE", profile.corpus_mode.as_str());

        let invocation = CompressInvocation {
            profile: profile.clone(),
            path: PathBuf::from("<in-memory>"),
            password: None,
            output_dir: None,
        };
        // For in-memory compression, is_dir is always false.
        // The plan will be V5Min or V4Encrypted depending on
        // profile.encrypt.
        let plan = resolve_plan(&invocation, false);

        let backend = legacy_backend_for_plan(plan.backend);
        let lzma_level = lzma_level_from_plan(&plan.codec_level);
        api::compress_bytes_with_backend(bytes, file_name, backend, lzma_level)
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Translate the engine's `PlanBackend` into the legacy
/// `api::CompressionBackend` so we can call the existing
/// dispatch functions during the migration. This mapping
/// goes away in 5.7.10-E when the legacy enum is deleted.
fn legacy_backend_for_plan(plan_backend: PlanBackend) -> api::CompressionBackend {
    match plan_backend {
        PlanBackend::V4Encrypted => api::CompressionBackend::V4,
        PlanBackend::V5Min => api::CompressionBackend::V5Min,
        PlanBackend::V6Solid => api::CompressionBackend::V6Solid,
    }
}

/// Extract the LZMA preset (0..=9) from a `SolidLevel`. Returns
/// 6 (the balanced default) for non-LZMA levels — the legacy
/// `lzma_level` field is only used by the LZMA path.
fn lzma_level_from_plan(level: &SolidLevel) -> u32 {
    match level {
        SolidLevel::Lzma(n) => *n,
        _ => 6,
    }
}

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
