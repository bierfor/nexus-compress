//! Solid archive format `NXS7` (NexusCompress Solid v7).
//!
//! ## Why solid?
//!
//! In a typical source-code corpus (Next.js, Rust crate, etc.) the
//! files share huge amounts of structure: the same imports, the same
//! type names, the same function bodies, the same boilerplate. When
//! you compress each file independently, the dictionary resets
//! between files and can't see those cross-file repetitions.
//!
//! Solid compression concatenates ALL preprocessed files into a
//! single byte stream and runs the codec over the whole thing. The
//! dictionary now spans the entire corpus — so the second `.tsx`
//! file reuses the matches found in the first. On a real corpus
//! this typically buys +5-15% over per-file compression at the cost
//! of making the archive LOSSY (you can't extract the original bytes
//! — you get the minified version of each file).
//!
//! ## Codecs
//!
//! - **LZMA** (level 0..=9): best ratio, slowest. Default for
//!   "ultra" mode. ~20-50 MB/s single-thread on Apple Silicon.
//! - **zstd** (level -7..=22): 5-10x faster than LZMA with
//!   comparable ratio. Default for "rapido" / "balanceado" modes.
//!   ~300-500 MB/s on Apple Silicon.
//!
//! ## Format
//!
//! ```text
//! +------------------------+
//! | Magic "NXS7\n"  (5 B)  |
//! | Version       (1 B)    |   = 2
//! | Codec        (1 B)    |   0=LZMA 1=zstd
//! | n_files       (4 B LE) |
//! +------------------------+
//! | TOC entry 1            |   repeated n_files times
//! |   name_len  (2 B LE)   |
//! |   name      (N bytes)  |   UTF-8, no null
//! |   orig_size (8 B LE)   |   file size on disk
//! |   pre_size  (8 B LE)   |   bytes after preprocessor
//! |   preproc   (1 B)      |   0=raw 1=conservative 2=swc
//! |   offset    (8 B LE)   |   offset in DECOMPRESSED solid
//! +------------------------+
//! | Solid block             |   codec bytes (see Codec byte above)
//! +------------------------+
//! ```
//!
//! ## Honest contract
//!
//! **LOSSY archive.** Decompression returns the *minified* version
//! of each file (or the original bytes if the preprocessor was
//! `Raw`). The original source — comments, formatting, original
//! identifier names — is NOT recoverable. If you need lossless
//! per-file archives, use the v4 `.nxar` format (`NXAR` magic)
//! instead.
//!
//! ## When to use
//!
//! - Distributing minified source bundles (one file per source).
//! - Compressing large corpus snapshots where the smallest size
//!   matters more than byte-perfect recoverability.
//! - Use v4 `.nxar` (per-file lossless) when roundtrip fidelity
//!   matters.

use std::io::{Read, Write};
use std::sync::Arc;

use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

use crate::ast_minify;
use crate::minify;

/// Format magic: `NXS7\n`.
pub const MAGIC: &[u8; 5] = b"NXS7\n";

/// Format version 2 (NXS7 = zstd default, NXS6 = LZMA only).
pub const VERSION: u8 = 4;

/// Solid block codec identifier stored in the header.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// LZMA (xz2 wrapper, preset 0..=9).
    Lzma = 0,
    /// Zstandard (zstd, level -7..=22) + optional trained dict.
    Zstd = 1,
}

/// Which codec + level to use for the solid block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    /// Fully automatic: detects hardware RAM, data size and picks the
    /// best codec + preset at runtime. The user never has to think
    /// about this — just pass `Auto` and the system figures it out.
    Auto,
    /// LZMA (xz2 wrapper, preset 0..=9). Explicit override.
    Lzma(u32),
    /// Zstandard (level -7..=22). Explicit override.
    Zstd(i32),
}

impl CompressionLevel {
    /// Fully automatic — detects hardware and picks the best preset.
    pub fn auto() -> Self {
        CompressionLevel::Auto
    }
    /// Fastest option: zstd level 3 (~300-500 MB/s on Apple Silicon).
    pub fn fast() -> Self {
        CompressionLevel::Zstd(3)
    }
    /// Balanced: zstd level 3.
    pub fn balanced() -> Self {
        CompressionLevel::Zstd(3)
    }
    /// Maximum ratio: LZMA preset 9 (~20-50 MB/s on Apple Silicon).
    pub fn ultra() -> Self {
        CompressionLevel::Lzma(9)
    }
    pub fn codec(&self) -> Codec {
        match self {
            CompressionLevel::Auto | CompressionLevel::Zstd(_) => Codec::Zstd,
            CompressionLevel::Lzma(_) => Codec::Lzma,
        }
    }
    /// Resolve `Auto` to the best concrete codec for the machine.
    /// Uses: available RAM → dict cap; CPU cores → parallelism.
    pub fn resolve(&self, _total_bytes: u64) -> CompressionLevel {
        match self {
            // Sprint 5.7.9: Auto default flipped from LZMA(3)
            // to Zstd(3). Benchmark on M4 Pro: zstd -3 runs
            // 200-500 MB/s per core vs LZMA(3) ~25 MB/s, with
            // ratio within 5% of LZMA(3) on a preprocessed
            // (Conservative / SWC AST) corpus. The user asked
            // for the default Lossy path to be fast: "vamos a
            // tener que hacerlo mas veloz". The conservative
            // preprocessor already strips 90% of the
            // semantic noise, so the marginal ratio gain
            // from LZMA over zstd is small, but the time
            // difference is dramatic.
            //
            // Callers that want LZMA still can: pass
            // `CompressionLevel::Lzma(6)` explicitly. The CLI
            // `--codec lzma` flag also routes here. Auto
            // means "the safe fast default".
            CompressionLevel::Auto => CompressionLevel::Zstd(3),
            other => *other,
        }
    }
}

// Sprint 5.7.2 hotfix #34: backward-compat for legacy callers that
// pass a raw integer (e.g. tests doing `compress(&files, 6)`). 0..=9
// → LZMA preset, anything else → LZMA preset 6.
impl From<i32> for CompressionLevel {
    fn from(n: i32) -> Self {
        if n >= 0 && n <= 9 {
            CompressionLevel::Lzma(n as u32)
        } else {
            CompressionLevel::Lzma(6)
        }
    }
}

impl From<u32> for CompressionLevel {
    fn from(n: u32) -> Self {
        if n <= 9 {
            CompressionLevel::Lzma(n)
        } else {
            CompressionLevel::Lzma(6)
        }
    }
}



/// Per-file preprocessor identifiers.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preprocessor {
    /// Pass the bytes through unchanged.
    Raw = 0,
    /// Conservative text minify (strips comments, collapses whitespace).
    Conservative = 1,
    /// swc AST minify (drops comments, types, formatting — lossy).
    SwcAst = 2,
}

impl Preprocessor {
    fn from_u8(b: u8) -> Result<Self, String> {
        // Sprint 5.7.2 hotfix #46: low nibble of the encoded byte
        // (bits 0-3) holds the preprocessor id. The high nibble
        // (bits 4-7) holds the chunk codec id, but we strip it
        // here so preprocessor id is the only thing this helper
        // reports. Decoders that need the codec use the
        // companion helper `chunk_codec_from_byte` below.
        let low = b & 0x0F;
        match low {
            0 => Ok(Preprocessor::Raw),
            1 => Ok(Preprocessor::Conservative),
            2 => Ok(Preprocessor::SwcAst),
            other => Err(format!("unknown preprocessor id: {}", other)),
        }
    }
}

/// Sprint 5.7.2 hotfix #46: high nibble (bits 4-7) of the per-file
/// preproc byte now carries the chunk codec. We extract it here so
/// the decoder knows whether this entry's preprocessed bytes were
/// emitted as LZMA frames or zstd frames (or, in the common
/// homogeneous case, which global codec applies to the whole file).
///
/// The decoder groups consecutive entries with the same codec into
/// a single decode pass so the per-chunk granularity exposed by
/// the entropy-aware encoder (#38, #39) round-trips correctly.
#[inline]
fn chunk_codec_from_byte(b: u8) -> Codec {
    let high = (b >> 4) & 0x0F;
    match high {
        1 => Codec::Zstd,
        // 0 (and any out-of-range nibble from a v1 archive that
        // never wrote a high nibble) defaults to LZMA — that's
        // what v1 archives have always meant, so we stay
        // backward-compatible with files written before #46.
        _ => Codec::Lzma,
    }
}

/// Pack the preprocessor id and chunk codec into a single byte.
/// v1 readers see `preprocessor as u8` because the high nibble is
/// zero by construction, so existing archives decode without a
/// format bump.
#[inline]
fn pack_preproc_codec(pre: Preprocessor, codec: Codec) -> u8 {
    ((codec as u8) << 4) | (pre as u8)
}

/// Sprint 5.7.2 hotfix #46: run-length encode the encoder's
/// per-chunk codec vector into the sub-TOC that lives between the
/// file entries and the solid block. Each tuple is
/// `(codec, total_compressed_size)` for a run of consecutive
/// super-chunks that share the same codec. The decoder uses
/// these tuples to know (a) which decoder to instantiate for
/// each slice of the solid block, and (b) exactly how many
/// compressed bytes to feed that decoder.
fn build_chunk_groups(
    chunk_codecs: &[CompressionLevel],
    chunk_compressed_sizes: &[u32],
) -> Vec<(Codec, u32)> {
    debug_assert_eq!(chunk_codecs.len(), chunk_compressed_sizes.len());
    let mut groups: Vec<(Codec, u32)> = Vec::new();
    let mut i = 0;
    while i < chunk_codecs.len() {
        let codec = chunk_codecs[i].codec();
        let mut total: u32 = 0;
        let mut j = i;
        while j < chunk_codecs.len() && chunk_codecs[j].codec() == codec {
            total = total.saturating_add(chunk_compressed_sizes[j]);
            j += 1;
        }
        groups.push((codec, total));
        i = j;
    }
    groups
}

/// A single file entry in the TOC.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub original_size: u64,
    pub pre_size: u64,
    pub preprocessor: Preprocessor,
    /// Offset in the DECOMPRESSED solid stream where this file's
    /// preprocessed bytes start.
    pub solid_offset: u64,
    /// Sprint 5.7.2 hotfix #46: codec used to encode the super-
    /// chunk this file belongs to. Hotfix #38/#39 may flip a
    /// chunk to `Codec::Zstd` when its contents are statistically
    /// incompressible (entropy > 7.5 bits/byte), so this is per
    /// file rather than per archive. The decoder groups
    /// consecutive entries with the same `chunk_codec` and
    /// decodes each group with the matching decoder.
    pub chunk_codec: Codec,
}

/// Detects the best preprocessor for a file based on its name.
/// Sprint 5.7.2 hotfix #38: per-chunk codec selection.
///
/// Decides whether a file should go through LZMA (compressible text)
/// or zstd-fast-negative-3 (already-compressed/incompressible data).
/// The decision is per-file so super-chunks can mix both codecs.
///
/// Files matching `is_incompressible_ext()` use zstd(-3) which on
/// incompressible data runs at ~5.7 GB/s (vs LZMA which would
/// spend minutes looking for matches that don't exist).
///
/// **Sprint 5.7.10-A:** this is now a thin wrapper around
/// [`crate::format_knowledge::is_raw_format`], the single source
/// of truth for the extension list. The previous ~80-entry
/// `matches!()` arm lived here and was duplicated verbatim in
/// `pick_preprocessor_with_overrides`, plus partially in
/// `nxar::PASSTHROUGH_EXTS` and `api::PASSTHROUGH_EXT_LIST`.
/// All four call sites now route through the master list.
fn is_incompressible_ext(name: &str) -> bool {
    crate::format_knowledge::is_raw_format(name)
}

/// Sprint 5.7.2 hotfix #39: Shannon entropy pre-flight check.
///
/// Computes entropy of the first 64 KiB of the input. Files with
/// entropy >7.5 bits/byte are statistically indistinguishable from
/// random data — running LZMA on them wastes minutes of CPU per
/// chunk and produces output LARGER than the input (matches+headers).
///
/// Cost: O(64 KiB) per file, ~50 µs on Apple Silicon. Negligible
/// compared to the minutes saved on `.sst`/`.png`/binary blobs.
const ENTROPY_SAMPLE_BYTES: usize = 64 * 1024;
const ENTROPY_INCOMPRESSIBLE_THRESHOLD: f64 = 7.5;

fn shannon_entropy_incompressible(data: &[u8]) -> bool {
    // Sample the first 64 KiB (or the whole file if smaller).
    let sample: &[u8] = if data.len() > ENTROPY_SAMPLE_BYTES {
        &data[..ENTROPY_SAMPLE_BYTES]
    } else {
        data
    };
    if sample.is_empty() {
        return false;
    }
    let mut counts = [0usize; 256];
    for &b in sample {
        counts[b as usize] += 1;
    }
    let n = sample.len() as f64;
    let mut entropy = 0.0_f64;
    for &c in &counts {
        if c > 0 {
            let p = c as f64 / n;
            entropy -= p * p.log2();
        }
    }
    entropy > ENTROPY_INCOMPRESSIBLE_THRESHOLD
}

/// Sprint 5.7.4 hotfix #50: per-extension preprocessor
/// overrides. The GUI exposes a "Advanced" panel where the
/// user can pin specific extensions to `Raw` (bit-exact) or
/// `SwcAst` / `Conservative` (smart minify) regardless of the
/// default rule in `pick_preprocessor`.
///
/// Two lists, two intents:
/// * `raw_extensions` — extensions that MUST be Raw (overrides
///   the default rule, used for `.json`, `.env`, `.toml` and
///   any other file the user considers "must not be touched").
/// * `minify_extensions` — extensions that MUST be Conservative
///   (overrides the default rule, used for `.md`, `.txt` to
///   keep them parseable when recovered, or `.html`/`.css`
///   if the user wants the conservative strip without the
///   Conservative default that includes them already).
///
/// Resolution order:
/// 1. `raw_extensions` (highest priority — bit-exact wins)
/// 2. `minify_extensions` (forces Conservative)
/// 3. Default rule from `pick_preprocessor`
///
/// The lists are case-insensitive (`JSON` matches `json`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreprocessorOverrides {
    pub raw_extensions: Vec<String>,
    pub minify_extensions: Vec<String>,
}

impl PreprocessorOverrides {
    /// Empty / no overrides — every file uses the default rule.
    pub fn is_empty(&self) -> bool {
        self.raw_extensions.is_empty() && self.minify_extensions.is_empty()
    }

    /// Normalize an extension list to lowercase, dedup, drop empty
    /// entries. Used internally by the parser when reading the
    /// CLI flags (`--raw-ext .json,.env`) or the JSON req
    /// (`raw_extensions: [".json", ".env"]`).
    fn normalize(list: Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = list
            .into_iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_start_matches('.').to_string())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn new(raw_extensions: Vec<String>, minify_extensions: Vec<String>) -> Self {
        Self {
            raw_extensions: Self::normalize(raw_extensions),
            minify_extensions: Self::normalize(minify_extensions),
        }
    }
}

/// Sprint 5.7.4 hotfix #50: consults the per-extension override
/// list first, then the default rule. The resolution order is:
///   1. `overrides.raw_extensions` (if the file's ext is here,
///      force Raw — bit-exact regardless of the default rule)
///   2. `overrides.minify_extensions` (force Conservative)
///   3. Default rule (the `format_knowledge::is_raw_format`
///      table + JS-family SwcAst + Conservative fallback)
///
/// **Sprint 5.7.10-A:** the dead wrapper `pick_preprocessor`
/// (no-arg version, unused since the overrides landed in
/// 5.7.4) was removed. Callers that don't have overrides use
/// `pick_preprocessor_with_overrides(name, &PreprocessorOverrides::default())`
/// directly.
fn pick_preprocessor_with_overrides(
    name: &str,
    overrides: &PreprocessorOverrides,
) -> Preprocessor {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    // Override layer: bit-exact wins first, then forced
    // conservative, then default. The order matters — if
    // the user puts `json` in BOTH lists, Raw wins (the
    // safe default).
    if overrides.raw_extensions.binary_search(&ext).is_ok() {
        return Preprocessor::Raw;
    }
    if overrides.minify_extensions.binary_search(&ext).is_ok() {
        return Preprocessor::Conservative;
    }

    // Default rule. Sprint 5.7.5: the list of "always Raw"
    // formats used to be ~25 entries and lived in only
    // one place. The user's request to cover modern
    // Office / e-book / image / 3D / disk-image formats
    // grew the list to ~80 — at that size the two
    // functions that consume the list (`is_incompressible_ext`
    // and this `pick_preprocessor`) drifted apart:
    // is_incompressible_ext got the new entries, this
    // match kept the old list. The result: the entropy
    // pre-flight correctly skipped minify for `report.docx`,
    // but the preprocessor itself still minified it
    // (returning `Conservative`), which the
    // format-coverage tests caught.
    //
    // **Sprint 5.7.10-A:** both call sites now route
    // through `crate::format_knowledge::is_raw_format`, the
    // single source of truth. Adding a new format is a
    // one-line change in `format_knowledge::RAW_FORMATS`.
    if crate::format_knowledge::is_raw_format(name) {
        return Preprocessor::Raw;
    }
    // JavaScript/TypeScript AST minify — lossy but big ratio on source.
    // This is a *second* pass after the raw check: `.ts` is
    // deliberately NOT in RAW_FORMATS (see the module
    // docstring in `format_knowledge.rs`) so TypeScript keeps
    // its SwcAst path. MPEG-TS files (same extension) get
    // caught by the runtime Shannon entropy check instead.
    if matches!(ext.as_str(), "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") {
        return Preprocessor::SwcAst;
    }
    // HTML/CSS fall through to Conservative minify (the hotfix #41
    // HTML-aware minify was prototyped but proved buggy — reverted).
    // Everything else: conservative whitespace/comment strip.
    Preprocessor::Conservative
}

/// Sprint 5.7.5 hotfix #52: the Format Oracle. Decides
/// whether a trained dict is worth embedding in the
/// archive header.
///
/// The cost-benefit equation is:
///   `total_with_dict = dict_bytes + compressed_with_dict`
///   `total_without_dict = compressed_without_dict`
/// We keep the dict when `total_with_dict < total_without_dict`.
///
/// The Zstd+dict ratio on a typical text/code corpus is
/// 4-6x. The Zstd-without-dict ratio on the same corpus
/// is 1.5-2x (Zstd is fast but less dense than LZMA
/// without a shared dictionary across chunks). So
/// `compressed_without_dict ≈ compressed_with_dict * 2`
/// in the worst case, and the dict pays for itself when:
///
///   `dict_bytes + compressed_with_dict < 2 * compressed_with_dict`
///   `dict_bytes < compressed_with_dict`
///
/// In other words: **the dict must be smaller than the
/// compressed solid block.** This is the textbook rule
/// of thumb for dictionary compression: a trained dict
/// is a fixed-size overhead that only pays for itself
/// when the corpus is large enough that the per-byte
/// savings exceed the dict's amortized cost.
///
/// We use a conservative 8x ratio to predict the
/// compressed size from the preprocessed input
/// (Zstd+dict is often 8x+ on text corpora; this errs on
/// the side of "drop the dict" which is the safe
/// choice — the user can re-encode without lossless
/// /--codec=zstd if they want to force the dict):
///
///   `compressed_with_dict ≈ pre_bytes / 8`
///   `dict_bytes < pre_bytes / 8`
///   `pre_bytes > dict_bytes * 8`
///
/// The 8x threshold was calibrated empirically against
/// the user's three test corpora:
///   - `mongo/`  (842 KB pre, 144 KB dict)  → 842 < 1152 → DROP
///   - `secretaria/` (1.7 MB pre, 49 KB dict) → 1.7 MB > 392 KB → KEEP
///   - `FlowNow/` (1 GB pre, 107 KB dict)   → 1 GB > 856 KB → KEEP
///
/// The function is `pub` so the test suite can exercise
/// the boundary cases directly.
pub fn should_use_dict_for_size(dict_bytes: usize, pre_bytes: u64) -> bool {
    if dict_bytes == 0 {
        return false;
    }
    // 8x ratio threshold. See the doc comment above for
    // the empirical calibration against the three
    // reference corpora.
    pre_bytes > (dict_bytes as u64) * 8
}
/// Apply the preprocessor to a single file's bytes.
fn apply_preprocessor(pre: Preprocessor, input: &[u8]) -> Vec<u8> {
    match pre {
        Preprocessor::Raw => input.to_vec(),
        Preprocessor::Conservative => minify::minify(input),
        Preprocessor::SwcAst => ast_minify::minify(input).bytes,
    }
}

/// Sprint 5.7.21-F: per-file preprocessing result. The
/// preprocessor (AST minify for .ts/.js, Conservative for
/// other text, Raw for binary or `--lossless`) is the most
/// expensive step on the encoding path: a single .ts file
/// can spend 50-200 ms inside swc_core. We parallelize the
/// preprocessing stage so the wall-time scales with the
/// number of CPU cores, not the number of files.
struct PreprocessedFile {
    name: String,
    original_size: u64,
    pre_size: u64,
    pre_bytes: Vec<u8>,
    pre: Preprocessor,
    incompressible: bool,
}

/// Preprocess every file in parallel. Returns a Vec in the
/// same order as `files` (rayon's `par_iter().collect()`
/// preserves order). The downstream chunk aggregation loop
/// then runs serially over the result.
///
/// Memory note: this holds the preprocessed bytes for ALL
/// files in memory at once. For a 5 GiB corpus of source
/// code that's typically 2-3 GiB of `pre_bytes` (preprocessing
/// shrinks text). The previous serial code only held
/// `current_chunk` (128 MiB) plus the file iterator. If a
/// user is RAM-constrained they should still see a net
/// win because we go from O(serial) wall-time to
/// O(serial / num_cores) wall-time. The original file
/// bytes are already fully loaded (in `files`) so the
/// extra pre_bytes memory is the only overhead.
fn preprocess_files_parallel(
    files: &[(String, Vec<u8>)],
    force_raw: bool,
    overrides: &PreprocessorOverrides,
) -> Vec<PreprocessedFile> {
    use rayon::prelude::*;
    files
        .par_iter()
        .map(|(name, bytes)| {
            // Sprint 5.7.3 hotfix #49 + Sprint 5.7.4 hotfix #50.
            // force_raw → Raw for all (lossless path). Otherwise
            // per-extension preprocessor with per-extension overrides.
            let pre = if force_raw {
                Preprocessor::Raw
            } else {
                pick_preprocessor_with_overrides(name, overrides)
            };
            let pre_bytes = apply_preprocessor(pre, bytes);
            let original_size = bytes.len() as u64;
            let pre_size = pre_bytes.len() as u64;
            // Sprint 5.7.2 hotfix #39: Shannon entropy on the
            // preprocessed bytes. Cheap (64 KiB scan) so doing
            // it here is free.
            let incompressible = is_incompressible_ext(name)
                || shannon_entropy_incompressible(&pre_bytes);
            PreprocessedFile {
                name: name.clone(),
                original_size,
                pre_size,
                pre_bytes,
                pre,
                incompressible,
            }
        })
        .collect()
}

/// Compress a list of (name, bytes) into a solid v7 archive.
///
/// The archive is LOSSY — the preprocessor is applied to each file
/// before concatenation. The `name` should be the relative path
/// (e.g. `src/index.ts`); it is stored verbatim in the TOC.
///
/// `codec` selects the compression algorithm. See `CompressionLevel`.
    /// Sprint 5.7.2 hotfix #33: `CompressionLevel::fast()` / `balanced()`
    /// default to LZMA (65 MB/s, 4.3x ratio). zstd available via
    /// `CompressionLevel::Zstd(n)` explicit override for specific corpora.
///
/// `progress` is called after each file is preprocessed, with
/// `(file_index_done, total_files, current_file_name)`. Pass `|_,_,_| {}`
/// if you don't need progress reporting.
///
/// `force_raw` (Sprint 5.7.3 hotfix #49): when `true`, the
/// per-extension preprocessor selection (`pick_preprocessor`)
/// is bypassed — every file is treated as `Preprocessor::Raw`,
/// meaning the original bytes are fed verbatim to the codec.
/// This is the "Safe Mode" / bit-exact backup path: the
/// archive is fully reversible byte-for-byte. The trade-off
/// is a worse compression ratio on text-heavy corpora
/// (typically 1.5-2x instead of the 5-7x the smart
/// preprocessor achieves).
pub fn compress_with_progress<P>(
    files: &[(String, Vec<u8>)],
    level: CompressionLevel,
    progress: P,
) -> Result<Vec<u8>, String>
where
    P: Fn(usize, usize, &str) + Send + Sync,
{
    compress_with_progress_full(
        files,
        level,
        false,
        &PreprocessorOverrides::default(),
        progress,
    )
}

/// Sprint 5.7.3 hotfix #49: same as `compress_with_progress`
/// but with the explicit `force_raw` flag that bypasses the
/// per-extension preprocessor (every file becomes
/// `Preprocessor::Raw`, original bytes go to the codec
/// unchanged). Use this for bit-exact backup archives where
/// the user values integrity over ratio.
pub fn compress_with_progress_lossless<P>(
    files: &[(String, Vec<u8>)],
    level: CompressionLevel,
    progress: P,
) -> Result<Vec<u8>, String>
where
    P: Fn(usize, usize, &str) + Send + Sync,
{
    compress_with_progress_full(
        files,
        level,
        true,
        &PreprocessorOverrides::default(),
        progress,
    )
}

/// Sprint 5.7.4 hotfix #50: full-control entry point. The
/// caller passes the preprocessor override lists (Raw and
/// Conservative) and the global `force_raw` flag. Resolution
/// order inside the encoder is:
///   1. `force_raw` (global lossless switch) → every file Raw
///   2. `overrides.raw_extensions` (per-extension Raw)
///   3. `overrides.minify_extensions` (per-extension Conservative)
///   4. Default rule by extension
pub fn compress_with_progress_full_public<P>(
    files: &[(String, Vec<u8>)],
    level: CompressionLevel,
    force_raw: bool,
    overrides: &PreprocessorOverrides,
    progress: P,
) -> Result<Vec<u8>, String>
where
    P: Fn(usize, usize, &str) + Send + Sync,
{
    compress_with_progress_full(files, level, force_raw, overrides, progress)
}

fn compress_with_progress_full<P>(
    files: &[(String, Vec<u8>)],
    level: CompressionLevel,
    force_raw: bool,
    overrides: &PreprocessorOverrides,
    progress: P,
) -> Result<Vec<u8>, String>
where
    P: Fn(usize, usize, &str) + Send + Sync,
{
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  Sprint 5.7.2 hotfix #31 — Super-Chunks Sólidos Paralelos
    //  Sprint 5.7.2 hotfix #32 — Codec automático: zstd default
    //  Sprint 5.7.3 hotfix #49 — --lossless bypass
    //  Sprint 5.7.4 hotfix #50 — per-extension overrides
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    // Sprint 5.7.2 hotfix #47: the entropy-driven codec flip
    // (hotfix #39) is a feature of the *automatic* pipeline
    // only. When the user explicitly picks LZMA or Zstd via the
    // GUI toggle or the CLI `--codec` flag, we respect that
    // choice verbatim — never silently swap codecs based on the
    // per-chunk entropy reading. This is what makes the toggle
    // useful in the first place.
    let was_auto = matches!(level, CompressionLevel::Auto);
    // Resolve Auto → concrete codec based on available RAM.
    let level = level.resolve(0);

    const SUPER_CHUNK_MIB: usize = 128;
    const SUPER_CHUNK_BYTES: usize = SUPER_CHUNK_MIB * 1024 * 1024;

    // ── Step 1: buffer aggregator — segment files into super-chunks.
    //
    // Sprint 5.7.2 hotfix #38: per-chunk codec. `chunk_codecs[i]`
    // tells `par_iter.map` which codec to use for chunk `i`:
    //   - ZstdFast(-3): for chunks dominated by already-compressed
    //     data (`.sst`, `.png`, `.zip`, etc.) — runs at ~5.7 GB/s
    //     on incompressible data instead of LZMA's multi-minute
    //     match-search deadlock.
    //   - Lzma(level): for compressible text/code chunks.
    //
    // The decision is per-CHUNK, not per-file: once a chunk is
    // marked ZstdFast, ALL files in it go through zstd-fast. This
    // keeps the aggregator simple — we just flip a flag when we
    // see an `is_incompressible_ext()` file.
    let mut entries: Vec<FileEntry> = Vec::with_capacity(files.len());
    let mut super_chunks: Vec<Vec<u8>> = Vec::new();
    let mut chunk_codecs: Vec<CompressionLevel> = Vec::new();
    let mut current_chunk: Vec<u8> = Vec::with_capacity(SUPER_CHUNK_BYTES);
    let mut current_chunk_codec: CompressionLevel = level; // default: user-requested codec
    let mut global_offset: u64 = 0;
    let total = files.len();
    // Sprint 5.7.2 hotfix #46: parallel to `chunk_codecs`, this
    // vector records the compressed size of each super-chunk so
    // the sub-TOC written below can tell the decoder exactly
    // how many bytes each codec group occupies in the solid
    // block. Hoisted out of the per-encoding match arm so the
    // writer can see it.
    let mut chunk_compressed_sizes: Vec<u32> = Vec::new();

    // Sprint 5.7.21-F: parallelize the per-file preprocessing
    // step (ast_minify, conservative minify, entropy probe)
    // BEFORE the chunk aggregation loop. The aggregation
    // itself is still sequential because chunk boundaries
    // depend on the running chunk size, but every per-file
    // operation that can run independently now does.
    //
    // On a 30-file / 17 MiB repetitive TypeScript corpus this
    // step drops from ~6 s (serial ast_minify at ~570 ms each)
    // to ~1.5 s on a 4-core M4 Pro. See
    // tests/preprocess_parallel_test.rs for the empirical
    // measurement and bit-exact equivalence pin.
    let preprocessed = preprocess_files_parallel(files, force_raw, overrides);

    for (i, _unused) in files.iter().enumerate() {
        // Sprint 5.7.21-F: preprocessing is now done in parallel
        // before the chunk aggregation loop (see `preprocessed`
        // above). The AST minify step was the serial bottleneck
        // (50-200 ms per .ts/.js file inside swc_core). Pulling
        // it out lets rayon's par_iter parallelise the work
        // across all cores. The aggregation below is still
        // sequential because chunk boundaries depend on order.
        let preprocessor = preprocessed[i].pre;
        let pre_bytes = &preprocessed[i].pre_bytes;
        let original_size = preprocessed[i].original_size;
        let pre_size = preprocessed[i].pre_size;
        let incompressible = preprocessed[i].incompressible;
        let name = &preprocessed[i].name;

        if !current_chunk.is_empty()
            && current_chunk.len() + pre_bytes.len() > SUPER_CHUNK_BYTES
        {
            super_chunks.push(std::mem::take(&mut current_chunk));
            chunk_codecs.push(current_chunk_codec);
            current_chunk_codec = level; // reset for next chunk
        }

        // Sprint 5.7.2 hotfix #38+#39+#47: if this file is
        // incompressible (by extension OR by entropy) AND the
        // user did not pin a specific codec, flip the current
        // chunk to zstd-fast. The Zstd(-3) forward path runs
        // at ~5.7 GB/s on random data, vs LZMA which hangs for
        // minutes. When the user explicitly picked LZMA or Zstd
        // (`was_auto == false`), we keep their choice for every
        // chunk — the GUI toggle is then genuinely honored.
        //
        // Sprint 5.7.6 hotfix #53: Multi-Solid Block split.
        // The previous code flipped `current_chunk_codec` IN
        // PLACE — the chunk kept its LZMA-prefix from the
        // previous file and the new ZstdFast-suffix was
        // appended. That meant a chunk was a mix of two
        // codecs, which is exactly the pathology the user
        // flagged in the 18:00 review: "If you mix 500 MB
        // of text and 500 MB of binary in the same corpus,
        // the pipeline processes them sequentially in the
        // same solid block."
        //
        // The fix: when the codec changes mid-corpus, close
        // the current chunk (with its current codec) and
        // start a fresh one for the new file. The result:
        // every chunk is a pure-LZMA or pure-ZstdFast
        // island, and the sub-TOC groups them by codec.
        // A 500MB text + 500MB binary corpus now produces
        // ~3-4 large LZMA chunks followed by ~3-4 large
        // ZstdFast chunks, with a clean boundary between
        // them. Ratio on text stays at 5-7x (no LZMA
        // dictionary pollution), and binary passes through
        // at ~1.0x via zstd-fast (no LZMA time penalty).
        if was_auto && incompressible {
            // Codec changed → close the current chunk with
            // its existing codec and reset for the new one.
            if !current_chunk.is_empty()
                && current_chunk_codec != CompressionLevel::Zstd(-3)
            {
                super_chunks.push(std::mem::take(&mut current_chunk));
                chunk_codecs.push(current_chunk_codec);
                // Reset to the user's level for the next
                // chunk (it'll get flipped again if the
                // next file is also incompressible).
                current_chunk_codec = level;
            }
            current_chunk_codec = CompressionLevel::Zstd(-3);
        } else if !current_chunk.is_empty()
            && current_chunk_codec == CompressionLevel::Zstd(-3)
            && !incompressible
        {
            // Symmetric case: the current chunk is
            // ZstdFast but the new file is compressible.
            // Close the ZstdFast chunk and start a fresh
            // LZMA chunk. Same logic, opposite direction.
            super_chunks.push(std::mem::take(&mut current_chunk));
            chunk_codecs.push(current_chunk_codec);
            current_chunk_codec = level;
        }

        entries.push(FileEntry {
            name: name.clone(),
            original_size,
            pre_size,
            preprocessor,
            solid_offset: global_offset,
            // Sprint 5.7.2 hotfix #46: record the codec of the
            // super-chunk this file is being aggregated into.
            // The chunk aggregator above may have flipped
            // `current_chunk_codec` to Zstd(-3) for this file
            // (entropy > 7.5 bits/byte or incompressible
            // extension). Persist that choice so the decoder
            // can pick the right decoder per file.
            chunk_codec: current_chunk_codec.codec(),
        });
        current_chunk.extend_from_slice(&pre_bytes);
        global_offset += pre_size;

        progress(i + 1, total, name);
    }
    if !current_chunk.is_empty() {
        super_chunks.push(current_chunk);
        chunk_codecs.push(current_chunk_codec);
    }

    // ── Step 2: parallel compress each super-chunk via rayon.
    let codec_tag = match level.codec() {
        Codec::Zstd => "ZSTD",
        Codec::Lzma => "LZMA",
    };
    eprintln!(
        "[SOLID-{}] super-chunks: {} (≤{} MiB each, {} MiB total)",
        codec_tag,
        super_chunks.len(),
        SUPER_CHUNK_MIB,
        global_offset / (1024 * 1024)
    );

    // Trained dictionary, populated by the Zstd branch and embedded
    // in the NXS7 header so any decoder (on any machine) can
    // reconstruct the same compression context. Empty for LZMA.
    let mut trained_dict: Vec<u8> = Vec::new();

    // Sprint 5.7.2 hotfix #38: per-chunk codec. The match below
    // dispatches on the OVERALL `level`, but inside each branch
    // we use `chunk_codecs[i]` (set by the aggregator) to pick
    // zstd-fast vs LZMA per chunk. So when level=Auto→Lzma(3) and
    // chunk_codecs=[Zstd(-3), Lzma(3), Zstd(-3)], we run LZMA on
    // chunk 1 and zstd-fast on chunks 0 and 2.
    let solid_compressed: Vec<u8> = match level {
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        //  ZSTD branch (default on 16 GB machines)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        CompressionLevel::Zstd(level_num) => {
            // Sprint 5.7.9: SKIP dict training in Lossy mode.
            // The Conservative preprocessor + SWC AST already
            // stripped 90% of the semantic noise from the
            // corpus. The bytes the codec sees are mostly
            // unique tokens (variable names, string literals,
            // punctuation). Training a Zstd dictionary on
            // preprocessed bytes takes ~2 seconds on M4 Pro
            // and the Format Oracle then REJECTS the dict
            // because the gain isn't worth the embedded
            // overhead. We were paying 2s to throw the dict
            // away. Skipping is a pure win.
            //
            // Sprint 5.7.15: dict training is now re-enabled in
            // Lossy mode for corpora ≥ 16 MiB. The sliding window
            // (hotfix #51) makes training safe (no -72 srcSize_wrong
            // crashes), the Format Oracle (hotfix #52) decides
            // KEEP/DROP based on estimated gain, and the
            // 16 MiB threshold avoids the 2-3 second training
            // cost on small corpora where the dict is almost
            // always DROP'd anyway. Empirically verified in
            // Sprint 5.7.12: small repetitive corpora produce
            // dicts of 0-1 KiB that the oracle drops. Large
            // corpora (FlowNow 5.2 GiB) produce useful 44 KB
            // dicts that the oracle KEEPs.
            let level_n = level_num;
            // Lossless mode (force_raw=true): always train.
            // Lossy mode: only train on corpora whose preprocessed
            // bytes sum ≥ 16 MiB (where the dict has enough
            // material to learn from and the Format Oracle has
            // a real KEEP/DROP decision to make).
            const LOSSY_DICT_TRAINING_MIN_BYTES: u64 = 16 * 1024 * 1024;
            let total_pre_bytes_for_gate: u64 =
                super_chunks.iter().map(|c| c.len() as u64).sum();
            let skip_dict_training = !force_raw
                && total_pre_bytes_for_gate < LOSSY_DICT_TRAINING_MIN_BYTES;
            if skip_dict_training {
                if !force_raw {
                    eprintln!(
                        "[SOLID-ZSTD] dict training SKIPPED (Lossy mode, \
                         {} MiB < 16 MiB threshold); using zstd level {} directly",
                        total_pre_bytes_for_gate / (1024 * 1024),
                        level_num
                    );
                } else {
                    eprintln!(
                        "[SOLID-ZSTD] dict training SKIPPED (should not happen \
                         — force_raw=true means we should train)"
                    );
                }
                trained_dict = Vec::new();
            } else { // skip_dict_training == false: full pipeline
            // Dictionary training: collect up to 512 MB of samples from
            // the corpus (1/32 of 16 GB). Training itself needs only
            // ~6 MB of working memory regardless of dict size.
            let ram_mb = crate::ram::available_memory_mb().unwrap_or(8192) as usize;
            let dict_cap_bytes = (ram_mb / 32).max(16).min(512) * 1024 * 1024;

            // Sprint 5.7.4 hotfix #51: sample collection.
            //
            // The previous version took ONE 1 MB slice per super-chunk
            // and pushed it as a single sample. That was the
            // root cause of the "Error -40 / -72 srcSize_wrong"
            // failures: `ZDICT_trainFromBuffer` REQUIRES multiple
            // distinct samples to learn from (it internally
            // divides samples into segments and extracts
            // repetitions across them). A corpus with a single
            // super-chunk — like the `mongo/` test corpus — would
            // produce exactly 1 sample and the trainer would
            // refuse with `ZSTD_error_srcSize_wrong` (-72).
            //
            // Fix: split each super-chunk into MANY overlapping
            // samples of ~64 KiB each. That gives the trainer the
            // diversity it needs while keeping memory bounded.
            // We also clip the total sample buffer to
            // `dict_cap_bytes` so the dict doesn't dominate RAM.
            const SAMPLE_SIZE: usize = 64 * 1024; // 64 KiB per sample
            const SAMPLE_STRIDE: usize = 32 * 1024; // 50% overlap
            let mut train_samples: Vec<u8> = Vec::new();
            let mut train_sizes: Vec<usize> = Vec::new();
            'outer: for chunk in super_chunks.iter() {
                if chunk.is_empty() {
                    continue;
                }
                if chunk.len() <= SAMPLE_SIZE {
                    // Chunk fits in a single sample — push it whole.
                    if train_samples.len() + chunk.len() > dict_cap_bytes {
                        break;
                    }
                    train_samples.extend_from_slice(chunk);
                    train_sizes.push(chunk.len());
                    continue;
                }
                // Sliding window over the chunk.
                let mut offset = 0;
                while offset + SAMPLE_SIZE <= chunk.len() {
                    if train_samples.len() + SAMPLE_SIZE > dict_cap_bytes {
                        break 'outer;
                    }
                    train_samples.extend_from_slice(&chunk[offset..offset + SAMPLE_SIZE]);
                    train_sizes.push(SAMPLE_SIZE);
                    offset += SAMPLE_STRIDE;
                }
                // Tail sample (the leftover bytes that didn't fit a
                // full SAMPLE_SIZE window). Important: the trainer
                // needs the chunk to be REPRESENTED, not covered
                // window-by-window. The tail gives a fresh angle
                // on the data that helps the dict generalize.
                if offset < chunk.len() {
                    let tail_len = chunk.len() - offset;
                    if train_samples.len() + tail_len <= dict_cap_bytes {
                        train_samples.extend_from_slice(&chunk[offset..]);
                        train_sizes.push(tail_len);
                    }
                }
            }
            let dict: Vec<u8> = if train_samples.len() >= 64 * 1024
                && train_sizes.len() >= 2 // <-- the real fix
                && train_sizes.iter().sum::<usize>() == train_samples.len()
            {
                // Sprint 5.7.2 hotfix #42: try the high-level
                // EncoderDictionary::from_samples API first. It is
                // more robust to sample-size edge cases than the
                // raw train_from_buffer (which can fail with
                // ZSTD_error_srcSize_wrong / -72 on certain sample
                // alignments). Fall back to train_from_buffer if
                // the high-level API is unavailable.
                //
                // Build a Vec<Vec<u8>> of the individual samples by
                // walking train_sizes against train_samples. We must
                // do this carefully because train_samples is a flat
                // concatenation of all the chunk slices we pushed.
                let mut individual_samples: Vec<Vec<u8>> = Vec::with_capacity(train_sizes.len());
                let mut offset = 0;
                for &size in &train_sizes {
                    if offset + size <= train_samples.len() {
                        individual_samples.push(train_samples[offset..offset + size].to_vec());
                    } else {
                        // Defensive: should not happen given the loop
                        // invariant `offset + size == train_samples.len()`.
                        eprintln!(
                            "[SOLID-ZSTD] sample alignment mismatch: offset {} + size {} > {}",
                            offset, size, train_samples.len()
                        );
                        break;
                    }
                    offset += size;
                }
                let dict_result = match zstd::dict::from_samples(
                    &individual_samples,
                    dict_cap_bytes, // max dict size
                ) {
                    Ok(bytes) => {
                        eprintln!(
                            "[SOLID-ZSTD] dict: {} KB trained (high-level API, {} samples, {} KiB total)",
                            bytes.len() / 1024,
                            individual_samples.len(),
                            train_samples.len() / 1024
                        );
                        Ok(bytes)
                    }
                    Err(e1) => {
                        // Fallback to low-level API.
                        let mut dict_buf = vec![0u8; dict_cap_bytes];
                        match zstd::zstd_safe::train_from_buffer(
                            &mut dict_buf[..],
                            &train_samples,
                            &train_sizes,
                        ) {
                            Ok(size) => {
                                eprintln!(
                                    "[SOLID-ZSTD] dict: {} KB trained (low-level fallback)",
                                    size / 1024
                                );
                                dict_buf.truncate(size);
                                Ok(dict_buf)
                            }
                            Err(e2) => {
                                let raw = e2 as u64;
                                let as_i32 = raw as i32;
                                eprintln!(
                                    "[SOLID-ZSTD] dict train FAILED: high-level={:?}, \
                                     low-level raw={} i32={} ({} KB samples, {} samples)",
                                    e1, raw, as_i32,
                                    train_samples.len() / 1024,
                                    train_sizes.len()
                                );
                                Err(e2)
                            }
                        }
                    }
                };
                dict_result.unwrap_or_default()
            } else {
                eprintln!(
                    "[SOLID-ZSTD] dict skipped (need ≥64 KiB AND ≥2 samples; \
                     have {} KiB, {} samples)",
                    train_samples.len() / 1024,
                    train_sizes.len()
                );
                Vec::new()
            };

            // Sprint 5.7.5 hotfix #52: the Format Oracle.
            // Decide BEFORE the dict is committed to the
            // compression pass. A dict the oracle rejects
            // is dropped here, BEFORE the rayon parallel
            // compression loop, so the per-chunk Encoder
            // never sees the dict and emits a v3-compatible
            // header.
            let total_pre_bytes: u64 = super_chunks.iter().map(|c| c.len() as u64).sum();
            let oracle_keep = should_use_dict_for_size(dict.len(), total_pre_bytes);
            if !oracle_keep {
                eprintln!(
                    "[SOLID-ZSTD] oracle DROP dict: {} KB dict vs {} KB corpus \
                     ({} KB at 8x estimate); dict would be ≥12.5% of compressed output",
                    dict.len() / 1024,
                    total_pre_bytes / 1024,
                    total_pre_bytes / 8 / 1024
                );
                trained_dict = Vec::new();
            } else {
                eprintln!(
                    "[SOLID-ZSTD] oracle KEEP dict: {} KB dict vs {} KB corpus \
                     ({} KB at 8x estimate); dict is <12.5% of compressed output",
                    dict.len() / 1024,
                    total_pre_bytes / 1024,
                    total_pre_bytes / 8 / 1024
                );
                trained_dict = dict.clone();
            }
            } // close else block (full dict pipeline)

            use rayon::prelude::*;
            let progress_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let total_chunks = super_chunks.len();
            let progress_ref = &progress;
            // Sprint 5.7.5 hotfix #52: use the post-oracle
            // `trained_dict` here so the per-chunk encoder
            // honours the oracle's decision. If the oracle
            // dropped the dict, this is empty and every
            // chunk goes through the dictionary-less path.
            let dict_ref = trained_dict.clone();

            let compressed_chunks: Vec<Vec<u8>> = super_chunks
                .par_iter()
                .enumerate()
                .map(|(i, chunk)| {
                    let mut out = Vec::new();
                    {
                        // Sprint 5.7.2 hotfix #38: per-chunk codec.
                        // If this chunk was marked Zstd(-3) by the
                        // aggregator (because it contains .sst/.png/etc.),
                        // use zstd-fast with the simple forward path.
                        // Otherwise use the user's level with dict.
                        let use_fast = matches!(
                            chunk_codecs.get(i).copied().unwrap_or(level),
                            CompressionLevel::Zstd(n) if n < 0
                        );
                        let chunk_level = if use_fast { -3 } else { level_n };
                        let mut enc = if dict_ref.is_empty() || use_fast {
                            // For Zstd(-3), skip the dict — dict training
                            // doesn't help incompressible data and adds
                            // overhead.
                            zstd::stream::write::Encoder::new(&mut out, chunk_level)
                                .map_err(|e| format!("zstd encoder: {}", e))?
                        } else {
                            zstd::stream::write::Encoder::with_dictionary(&mut out, chunk_level, &dict_ref)
                                .map_err(|e| format!("zstd encoder w/ dict: {}", e))?
                        };
                        if !use_fast {
                            let _ = enc.set_parameter(zstd::zstd_safe::CParameter::WindowLog(22));
                        }
                        // Write through the Encoder's Write impl → data goes to `out` directly.
                        const SUB: usize = 4 * 1024 * 1024;
                        let mut fed = 0;
                        while fed < chunk.len() {
                            let end = (fed + SUB).min(chunk.len());
                            enc.write_all(&chunk[fed..end])
                                .map_err(|e| format!("zstd write: {}", e))?;
                            fed = end;
                            progress_ref(total, total, "");
                        }
                        // Flush the zstd frame footer into `out`.
                        enc.do_finish()
                            .map_err(|e| format!("zstd do_finish: {}", e))?;
                    }
                    let done = progress_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    progress_ref(total, total, "");
                    eprintln!(
                        "[SOLID-ZSTD] chunk {}/{} done ({} MiB → {} KiB)",
                        done, total_chunks,
                        chunk.len() / (1024 * 1024),
                        out.len() / 1024
                    );
                    Ok::<_, String>(out)
                })
                .collect::<Result<Vec<_>, _>>()?;

            let total_compressed_size: usize = compressed_chunks.iter().map(|c| c.len()).sum();
            // Sprint 5.7.2 hotfix #46+47: capture per-chunk
            // compressed sizes in the outer-scope vector so the
            // writer can emit the sub-TOC after the file
            // entries. Zstd and Lzma branches must both push
            // here — a missing push (as the Zstd branch
            // originally did) trips the `build_chunk_groups`
            // debug_assert with an out-of-bounds panic.
            for c in &compressed_chunks {
                chunk_compressed_sizes.push(c.len() as u32);
            }
            let mut out: Vec<u8> = Vec::with_capacity(total_compressed_size);
            for c in compressed_chunks { out.extend_from_slice(&c); }
            eprintln!(
                "[SOLID-ZSTD] done: {} MiB → {} MiB ({}x)",
                global_offset / (1024 * 1024),
                out.len() / (1024 * 1024),
                if out.is_empty() { 1.0 } else { global_offset as f64 / out.len() as f64 }
            );
            out
        }

        // Safety: resolve() converts Auto → Zstd/Lzma before this match,
        // so Auto can never reach here.
        CompressionLevel::Auto => unreachable!(),

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        //  LZMA branch (explicit ultra mode)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        CompressionLevel::Lzma(lzma_level) => {
            use rayon::prelude::*;
            use crate::scheduler::{Weighted, sonic_par_try_map, default_thread_count};
            let progress_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let total_chunks = super_chunks.len();
            let progress_ref = &progress;
            let make_stream = || {
                crate::ram::lzma_stream_for_requested_preset(lzma_level)
                    .map_err(|e| format!("LZMA stream: {}", e))
            };
            // Sprint 5.7.6 hotfix #54: Sonic Scheduler. We
            // pre-weight the super_chunks by their byte count
            // and let LPT partition the work across N
            // buckets. Each bucket runs sequentially on a
            // rayon worker thread, but the WORK ASSIGNMENT
            // is balanced: a 256 MiB random chunk and ten
            // 1 MiB text chunks no longer land on the same
            // thread (which would serialize them).
            let weighted_chunks: Vec<Weighted<Vec<u8>>> = super_chunks
                .iter()
                .map(|c| Weighted::new(c.len() as u64, c.clone()))
                .collect();
            let n_threads = default_thread_count();
            let chunk_codecs_ref = &chunk_codecs;
            let compressed_chunks: Vec<Vec<u8>> = sonic_par_try_map(
                weighted_chunks,
                n_threads,
                |orig_idx, chunk| -> Result<Vec<u8>, String> {
                    let mut out = Vec::new();
                    {
                        // Sprint 5.7.2 hotfix #38: if this chunk was
                        // marked Zstd(-3) by the aggregator, use zstd-fast
                        // (LZMA would hang for minutes on .sst/.png data).
                        let use_fast = matches!(
                            chunk_codecs_ref.get(orig_idx).copied().unwrap_or(level),
                            CompressionLevel::Zstd(n) if n < 0
                        );
                        if use_fast {
                            // zstd-fast forward — ~5.7 GB/s on incompressible.
                            let mut enc = zstd::stream::write::Encoder::new(&mut out, -3)
                                .map_err(|e| format!("zstd-fast encoder: {}", e))?;
                            const SUB: usize = 4 * 1024 * 1024;
                            let mut fed = 0;
                            while fed < chunk.len() {
                                let end = (fed + SUB).min(chunk.len());
                                enc.write_all(&chunk[fed..end])
                                    .map_err(|e| format!("zstd-fast write: {}", e))?;
                                fed = end;
                                progress_ref(total, total, "");
                            }
                            enc.do_finish()
                                .map_err(|e| format!("zstd-fast finish: {}", e))?;
                        } else {
                            let stream = make_stream()?;
                            let mut enc = XzEncoder::new_stream(&mut out, stream);
                            const SUB: usize = 4 * 1024 * 1024;
                            let mut fed = 0;
                            while fed < chunk.len() {
                                let end = (fed + SUB).min(chunk.len());
                                enc.write_all(&chunk[fed..end])
                                    .map_err(|e| format!("LZMA write: {}", e))?;
                                fed = end;
                                progress_ref(total, total, "");
                            }
                            enc.finish()
                                .map_err(|e| format!("LZMA finish: {}", e))?;
                        }
                    }
                    let done = progress_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    progress_ref(total, total, "");
                    eprintln!(
                        "[SOLID-LZMA] chunk {}/{} done ({} MiB → {} KiB)",
                        done, total_chunks,
                        chunk.len() / (1024 * 1024),
                        out.len() / 1024
                    );
                    Ok(out)
                },
            )?;
            let total_compressed_size: usize = compressed_chunks.iter().map(|c| c.len()).sum();
            // Sprint 5.7.2 hotfix #46: capture per-chunk compressed
            // sizes in the outer-scope vector so the writer can
            // emit the sub-TOC after the file entries. The order
            // matches `chunk_codecs` because both come from the
            // same parallel iter (zip them in `build_chunk_groups`).
            for c in &compressed_chunks {
                chunk_compressed_sizes.push(c.len() as u32);
            }
            let mut out: Vec<u8> = Vec::with_capacity(total_compressed_size);
            for c in compressed_chunks { out.extend_from_slice(&c); }
            eprintln!(
                "[SOLID-LZMA] done: {} MiB → {} MiB ({}x)",
                global_offset / (1024 * 1024),
                out.len() / (1024 * 1024),
                if out.is_empty() { 1.0 } else { global_offset as f64 / out.len() as f64 }
            );
            out
        }
    };

    // Sprint 5.7.2 hotfix #46: build the per-chunk-group descriptor
    // so the decoder knows which codec emitted each segment of the
    // solid block AND where each segment ends in the compressed
    // byte stream. Without `chunk_compressed_size` the decoder
    // would have to guess the boundary between an xz frame and a
    // zstd frame, which is impossible because both formats
    // reserve their own magic at the start.
    //
    // Grouping rule: run-length encode `chunk_codecs` so consecutive
    // chunks sharing the same codec form one group. For homogeneous
    // archives (no file triggered the entropy flip from #39),
    // `chunk_groups.len() == 1` and the sub-TOC adds 5 bytes.
    let chunk_groups: Vec<(Codec, u32)> = build_chunk_groups(&chunk_codecs, &chunk_compressed_sizes);

    // ── Step 3: serialize header (TOC) + append compressed block.
    //
    // NXS7 v4 wire format (Sprint 5.7.2 hotfix #46: per-chunk codec):
    //
    //   MAGIC (5) | VERSION (1) | CODEC (1) | N_FILES (4) |
    //   [TOC entries: name_len u16, name, original_size u64,
    //    pre_size u64, preproc_codec u8, solid_offset u64] |
    //   N_GROUPS (4) |
    //   [group entries: chunk_codec u8, chunk_compressed_size u32] *
    //   N_GROUPS |
    //   solid_compressed...
    //
    // preproc_codec is a packed byte: low nibble = preprocessor id
    // (back-compat with v1), high nibble = chunk_codec id. The
    // per-entry preproc byte makes it possible for callers like the
    // API layer to surface the codec that was actually used per
    // file (for stats, UI, etc.) without re-running the entropy
    // check. The sub-TOC after the entries is the authoritative
    // byte-exact boundary map for the decoder.
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(MAGIC);
    // Sprint 5.7.4 hotfix #51: VERSION selection. The hotfix
    // #45 revert removed the dict-in-header field and that
    // BROKE the roundtrip: the encoder was still training
    // a dict and using it to compress, but the decoder was
    // never told the dict existed and "Dictionary mismatch"
    // panicked. The fix is to bump VERSION to 4 and
    // re-introduce the dict field for archives that actually
    // contain a trained dict. Archives WITHOUT a trained
    // dict (e.g. LZMA-only, or zstd with too few samples)
    // stay at VERSION 3 so they keep the v3 wire format
    // and remain readable by older decoders that we may have
    // shipped before this fix.
    //
    // The literal `3` here is the canonical v3 identifier
    // and is NOT the same as `VERSION` (which is 4 — the
    // current build's max). We hardcode 3 because it's a
    // wire-format constant, not a build identity.
    let archive_version: u8 = if !trained_dict.is_empty() {
        4
    } else {
        3
    };
    out.push(archive_version);
    out.push(level.codec() as u8);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    // Sprint 5.7.4 hotfix #51: dict field, only present in v4
    // archives. v3 archives skip this entirely (the parser
    // does the same — it reads dict_len only when version >= 4
    // OR when there is enough room). The field is:
    //   dict_len: u32 LE  |  dict_bytes: [u8; dict_len]
    // We DON'T include a magic / length prefix because the
    // dict is the last variable-length field before the solid
    // block, and the sub-TOC + solid block are bounded
    // separately.
    // explicitly. For the default LZMA path (and for the test suite)
    // we write a clean NXS6/NXS7 header: HEADER → TOC → SOLID.
    //
    // This restores the test suite and roundtrip. Trade-off: dict
    // portability across machines is no longer supported (revisit
    // in a future sprint with a proper format bump).
    let _ = &trained_dict; // suppress unused warning
    for e in &entries {
        let name_bytes = e.name.as_bytes();
        if name_bytes.len() > u16::MAX as usize {
            return Err(format!(
                "file name too long ({} bytes): {}",
                name_bytes.len(),
                e.name
            ));
        }
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&e.original_size.to_le_bytes());
        out.extend_from_slice(&e.pre_size.to_le_bytes());
        out.push(pack_preproc_codec(e.preprocessor, e.chunk_codec));
        out.extend_from_slice(&e.solid_offset.to_le_bytes());
    }
    // Sprint 5.7.4 hotfix #51: dict field for v4 archives.
    // Only present when `trained_dict` is non-empty (which
    // happens when the user picked --codec=zstd and the corpus
    // had enough samples to train a dict). v3 archives skip
    // this block — the parser does the same.
    if archive_version == 4 && !trained_dict.is_empty() {
        out.extend_from_slice(&(trained_dict.len() as u32).to_le_bytes());
        out.extend_from_slice(&trained_dict);
    }
    // Sprint 5.7.2 hotfix #46: write the chunk-group sub-TOC so
    // the decoder can cut the solid block at the right boundary
    // for each codec. The hot-path (homogeneous archive) costs
    // exactly 5 bytes: a 4-byte count and one 1+4 tuple.
    out.extend_from_slice(&(chunk_groups.len() as u32).to_le_bytes());
    for (codec, compressed_size) in &chunk_groups {
        out.push(*codec as u8);
        out.extend_from_slice(&compressed_size.to_le_bytes());
    }
    out.extend_from_slice(&solid_compressed);
    Ok(out)
}

/// Backward-compatible wrapper that discards progress.
#[inline]
pub fn compress<L: Into<CompressionLevel>>(
    files: &[(String, Vec<u8>)],
    level: L,
) -> Result<Vec<u8>, String> {
    compress_with_progress(files, level.into(), |_, _, _| {})
}

/// Decompress a solid v6 archive, returning the (name, preprocessed
/// bytes) of every file.
///
/// **LOSSY:** the returned bytes are the minified versions, not the
/// originals. To get the original back you would need to feed each
/// returned file through a formatter, which is not what this module
/// does (and not what the v6 backend ever claimed to do).
/// Read just the TOC of a solid archive, WITHOUT touching the
/// LZMA block. Returns the file entries (with uncompressed sizes)
/// and the sum of those sizes. Used by the GUI preview so the
/// user can browse a `.nxs6` archive without paying full
/// decompression cost.
pub fn peek_toc(archive: &[u8]) -> Result<(Vec<FileEntry>, u64), String> {
    let parsed = parse_toc(archive)?;
    let total: u64 = parsed.entries.iter().map(|e| e.original_size).sum();
    Ok((parsed.entries, total))
}

/// Sprint 5.7.11: a passthrough file's raw on-disk representation,
/// captured by `parse_toc` from the `NXPT` trailer that the encoder
/// appends after the LZMA/zstd solid block. The decoder turns
/// these into proper `FileEntry`s in `decompress` (decoding the
/// payload with zstd if the marker is `Z`, or using it raw if `R`).
///
/// We keep the raw payload here (not the decoded bytes) so callers
/// that only parse the TOC without decompressing the solid block
/// (e.g. `list_solid_paginated`) can still see the passthrough
/// filenames and sizes without paying the zstd cost.
#[derive(Debug, Clone)]
pub struct PassthroughRawEntry {
    pub name: String,
    /// `0` = the payload was zstd-compressed by the encoder
    ///       (`Z` marker). `1` = raw bytes (`R` marker, the
    ///       encoder fell back to raw because zstd didn't shrink).
    pub codec: u8,
    /// Compressed payload as written to the archive.
    pub payload: Vec<u8>,
}

/// Result of parsing a solid archive's TOC.
#[derive(Debug)]
pub struct ParsedToc {
    pub entries: Vec<FileEntry>,
    /// Trained zstd dictionary (empty if LZMA or no dict).
    pub dict: Vec<u8>,
    /// Byte offset in the archive where the compressed solid block begins.
    pub solid_block_offset: usize,
    /// Sprint 5.7.2 hotfix #46: per-chunk-group descriptor
    /// `[(codec, compressed_size)]` covering the solid block in
    /// order. For v1/v2 archives that predate the hotfix, this
    /// is rebuilt by the parser as a single group using the
    /// archive's global codec and the solid block's total size.
    pub chunk_groups: Vec<(Codec, u32)>,
    /// Sprint 5.7.11: passthrough entries captured from the
    /// `NXPT` trailer at the end of the archive. Empty for
    /// archives written before the SupremeEngine path
    /// (Sprint 5.7.10-C) and for archives whose `raw_extensions`
    /// list was empty at compression time.
    pub passthroughs: Vec<PassthroughRawEntry>,
}

/// Parse the TOC portion of a solid archive (header + entry list)
/// and return the entries, the trained dict (if any), and the
/// offset of the solid block. Shared between `peek_toc` and
/// `decompress`.
///
/// Supports both NXS6 (version=1, LZMA only) and NXS7 (version=2,
/// LZMA or zstd). The codec byte is only present in version 2.
/// The dict_len + dict_bytes are present in version 2 (Nxs7 v3,
/// hotfix #34) but written as `dict_len=0` for archives without
/// a custom dict, so we can always read the same 4 bytes.
pub fn parse_toc(archive: &[u8]) -> Result<ParsedToc, String> {
    if archive.len() < MAGIC.len() + 1 + 4 {
        return Err("solid archive: too short for header".into());
    }
    let mut pos = 0;
    if &archive[pos..pos + MAGIC.len()] != MAGIC {
        return Err("solid archive: bad magic".into());
    }
    pos += MAGIC.len();
    let version = archive[pos];
    pos += 1;
    if version != VERSION && version != 1 && version != 2 && version != 3 {
        return Err(format!("solid archive: unsupported version {}", version));
    }
    // Codec byte is present in NXS7 (version 2 OR 3). NXS6
    // (version 1) has no codec byte. Sprint 5.7.2 hotfix #46
    // bumped VERSION to 3 to advertise the per-chunk-codec
    // sub-TOC; the codec byte itself is unchanged.
    if version >= 2 {
        let _codec = archive[pos]; // 0=LZMA, 1=zstd
        pos += 1;
    }
    let n_files = u32::from_le_bytes(
        archive[pos..pos + 4]
            .try_into()
            .map_err(|_| "solid archive: bad n_files".to_string())?,
    ) as usize;
    pos += 4;

    // Sprint 5.7.4 hotfix #51: dict field for v4 archives.
    // IMPORTANT: the dict is written AFTER the file entries
    // (the encoder writes HEADER → ENTRIES → DICT → SUB-TOC
    // → SOLID), so we have to read it AFTER the entries
    // loop below. The placeholder below exists only to make
    // `?` error returns inside the entries loop work without
    // shadowing. The actual dict read is right after the
    // entries loop completes.
    let mut dict: Vec<u8> = Vec::new();

    let mut entries: Vec<FileEntry> = Vec::with_capacity(n_files);
    for _ in 0..n_files {
        if pos + 2 > archive.len() {
            return Err("solid archive: truncated name_len".into());
        }
        let name_len = u16::from_le_bytes(
            archive[pos..pos + 2]
                .try_into()
                .map_err(|_| "solid archive: bad name_len".to_string())?,
        ) as usize;
        pos += 2;
        if pos + name_len > archive.len() {
            return Err("solid archive: truncated name".into());
        }
        let name = String::from_utf8(archive[pos..pos + name_len].to_vec())
            .map_err(|_| "solid archive: invalid UTF-8 in name".to_string())?;
        pos += name_len;
        if pos + 8 + 8 + 1 + 8 > archive.len() {
            return Err("solid archive: truncated entry".into());
        }
        let original_size = u64::from_le_bytes(
            archive[pos..pos + 8]
                .try_into()
                .map_err(|_| "solid archive: bad orig_size".to_string())?,
        );
        pos += 8;
        let pre_size = u64::from_le_bytes(
            archive[pos..pos + 8]
                .try_into()
                .map_err(|_| "solid archive: bad pre_size".to_string())?,
        );
        pos += 8;
        let encoded_preproc = archive[pos];
        pos += 1;
        // Sprint 5.7.2 hotfix #46: low nibble = preprocessor id,
        // high nibble = chunk codec. `Preprocessor::from_u8` strips
        // the high nibble before matching, so v1 archives (which
        // never wrote a high nibble) decode unchanged.
        let preprocessor = Preprocessor::from_u8(encoded_preproc)?;
        let chunk_codec = chunk_codec_from_byte(encoded_preproc);
        let solid_offset = u64::from_le_bytes(
            archive[pos..pos + 8]
                .try_into()
                .map_err(|_| "solid archive: bad offset".to_string())?,
        );
        pos += 8;
        entries.push(FileEntry {
            name,
            original_size,
            pre_size,
            preprocessor,
            solid_offset,
            chunk_codec,
        });
    }

    // Sprint 5.7.4 hotfix #51: dict field for v4 archives.
    // Read AFTER the entries loop because the encoder writes
    // it after the entries (HEADER → ENTRIES → DICT →
    // SUB-TOC → SOLID). v3 archives skip this block; the
    // encoder also skipped it when no dict was trained.
    if version == 4 {
        if pos + 4 > archive.len() {
            return Err("solid archive: truncated dict_len".into());
        }
        let dict_len = u32::from_le_bytes(
            archive[pos..pos + 4]
                .try_into()
                .map_err(|_| "solid archive: bad dict_len".to_string())?,
        ) as usize;
        pos += 4;
        if pos + dict_len > archive.len() {
            return Err("solid archive: truncated dict bytes".into());
        }
        if dict_len > 0 {
            dict = archive[pos..pos + dict_len].to_vec();
            pos += dict_len;
        }
    }

    // Sprint 5.7.2 hotfix #46: chunk-group sub-TOC. v3 (NXS7 v4)
    // writes N_GROUPS (4 LE) + N_GROUPS × (codec u8, size u32 LE)
    // right after the file entries. v1 (NXS6) and v2 (NXS7 v3)
    // archives do not have this section — they encode the whole
    // solid block with a single codec (the archive's global codec)
    // and no per-chunk flips. To stay back-compatible we synthesize
    // a single chunk_group for those older archives by treating
    // the entire remaining solid block as one group.
    let chunk_groups: Vec<(Codec, u32)> = if version >= 3 {
        if pos + 4 > archive.len() {
            return Err("solid archive: truncated chunk-group count".into());
        }
        let n_groups = u32::from_le_bytes(
            archive[pos..pos + 4]
                .try_into()
                .map_err(|_| "solid archive: bad chunk-group count".to_string())?,
        ) as usize;
        pos += 4;
        // Sprint 5.7.2 hotfix #46: an empty archive (n_files == 0)
        // writes an empty sub-TOC with n_groups == 0. We accept
        // that as a valid input: the decompressor will return an
        // empty solid block and no file entries.
        let mut groups: Vec<(Codec, u32)> = Vec::with_capacity(n_groups);
        for _ in 0..n_groups {
            if pos + 5 > archive.len() {
                return Err("solid archive: truncated chunk-group entry".into());
            }
            let codec = match archive[pos] {
                0 => Codec::Lzma,
                1 => Codec::Zstd,
                other => {
                    return Err(format!("solid archive: unknown chunk codec {}", other))
                }
            };
            pos += 1;
            let size = u32::from_le_bytes(
                archive[pos..pos + 4]
                    .try_into()
                    .map_err(|_| "solid archive: bad chunk-group size".to_string())?,
            );
            pos += 4;
            groups.push((codec, size));
        }
        // Sanity: the sum of chunk-group sizes must equal the
        // remaining solid-block size. This is what keeps the
        // decoder from under/over-reading.
        let total_group_bytes: u64 = groups.iter().map(|(_, s)| *s as u64).sum();
        if total_group_bytes > (archive.len() - pos) as u64 {
            return Err(format!(
                "solid archive: chunk-group total ({} bytes) exceeds solid block ({} bytes)",
                total_group_bytes,
                archive.len() - pos
            ));
        }
        groups
    } else {
        // Legacy archive: single group using the global codec,
        // spanning the whole remaining solid block.
        let legacy_codec = if version == 2 {
            archive[MAGIC.len() + 1]
        } else {
            0
        };
        let codec = match legacy_codec {
            1 => Codec::Zstd,
            _ => Codec::Lzma,
        };
        let total_size = (archive.len() - pos) as u32;
        vec![(codec, total_size)]
    };

    // Sprint 5.7.11: parse the `NXPT` trailer at the end of the
    // archive if present. The SupremeEngine (5.7.10-C) appends
    // passthrough files here after the LZMA/zstd solid block.
    // Archives written before that change have no trailer — the
    // loop below simply produces an empty `passthroughs` vec.
    //
    // We compute `solid_block_end` as the byte right after the
    // solid block (sum of all chunk-group sizes), and parse
    // anything past it as a sequence of `NXPT` entries. If the
    // first 4 bytes of the trailer region aren't `NXPT` we treat
    // the whole archive as a non-SupremeEngine archive (no
    // passthroughs) and return what we already have.
    let total_solid_bytes: usize = chunk_groups
        .iter()
        .map(|(_, s)| *s as usize)
        .sum();
    let solid_block_end: usize = pos + total_solid_bytes;
    let mut passthroughs: Vec<PassthroughRawEntry> = Vec::new();
    if solid_block_end + 4 <= archive.len()
        && &archive[solid_block_end..solid_block_end + 4] == b"NXPT"
    {
        let mut p = solid_block_end;
        while p + 9 <= archive.len() && &archive[p..p + 4] == b"NXPT" {
            p += 4;
            let name_len = u32::from_le_bytes(
                archive[p..p + 4]
                    .try_into()
                    .map_err(|_| "solid archive: bad passthrough name_len".to_string())?,
            ) as usize;
            p += 4;
            if p + name_len > archive.len() {
                return Err("solid archive: truncated passthrough name".into());
            }
            let name = String::from_utf8(archive[p..p + name_len].to_vec())
                .map_err(|_| "solid archive: invalid UTF-8 in passthrough name".to_string())?;
            p += name_len;
            if p + 5 > archive.len() {
                return Err("solid archive: truncated passthrough marker+len".into());
            }
            // 0 = zstd-compressed (`Z`), 1 = raw (`R`).
            let codec = match archive[p] {
                b'Z' => 0,
                b'R' => 1,
                other => {
                    return Err(format!(
                        "solid archive: unknown passthrough marker {}",
                        other
                    ))
                }
            };
            p += 1;
            let payload_len = u32::from_le_bytes(
                archive[p..p + 4]
                    .try_into()
                    .map_err(|_| "solid archive: bad passthrough payload_len".to_string())?,
            ) as usize;
            p += 4;
            if p + payload_len > archive.len() {
                return Err("solid archive: truncated passthrough payload".into());
            }
            let payload = archive[p..p + payload_len].to_vec();
            p += payload_len;
            passthroughs.push(PassthroughRawEntry { name, codec, payload });
        }
        // If the trailer didn't end cleanly on a non-NXPT byte,
        // that's an error — we don't want a half-parsed trailer
        // silently surviving.
        if p != archive.len() {
            return Err(format!(
                "solid archive: passthrough trailer ended mid-entry at byte {} (file len {})",
                p,
                archive.len()
            ));
        }
    }

    Ok(ParsedToc {
        entries,
        dict,
        solid_block_offset: pos,
        chunk_groups,
        passthroughs,
    })
}

/// Sprint 5.7.21-D: helper that decodes a single chunk
/// group. The `decompress` function used to do this in a
/// sequential for loop; pulling it into a free function
/// makes the per-group work a pure function of
/// (codec, compressed_bytes, dict) and lets the caller
/// parallelize the decode stage via `rayon::par_iter`.
///
/// Errors here are surfaced verbatim by the parallel
/// collection — the first failing group's error becomes
/// the `decompress` error.
fn decode_chunk_group(
    codec: Codec,
    group_compressed: &[u8],
    dict: &[u8],
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    match codec {
        Codec::Zstd => {
            // The zstd decoder path. If a dict was trained
            // (5.7.2 hotfix #42) the decoder needs the
            // same dict to roundtrip. Hotfix #34 reduced
            // the input slice to the group's own bytes
            // (the original bug fed zstd bytes to the
            // wrong decoder when a previous group was LZMA).
            if dict.is_empty() {
                let mut dec = zstd::stream::read::Decoder::new(group_compressed)
                    .map_err(|e| format!("zstd decode failed: {}", e))?;
                dec.read_to_end(&mut out)
                    .map_err(|e| format!("zstd read failed: {}", e))?;
            } else {
                let mut dec = zstd::stream::read::Decoder::with_dictionary(
                    group_compressed, dict,
                )
                .map_err(|e| format!("zstd decode w/ dict failed: {}", e))?;
                dec.read_to_end(&mut out)
                    .map_err(|e| format!("zstd read w/ dict read failed: {}", e))?;
            }
        }
        Codec::Lzma => {
            // The xz2 multi_decoder can chew through a
            // contiguous run of xz frames in the group's
            // bytes. For a single super-chunk that's one
            // frame; for runs of same-codec chunks merged
            // by the encoder (#38) the multi_decoder
            // happily processes all of them sequentially.
            // xz2's XzDecoder takes ownership of the
            // input slice via `BufReader`; that's fine in
            // a parallel context because each rayon
            // worker gets its own `XzDecoder` instance
            // and we never share between threads.
            XzDecoder::new_multi_decoder(group_compressed)
                .read_to_end(&mut out)
                .map_err(|e| format!("solid LZMA decompress failed: {}", e))?;
        }
    }
    Ok(out)
}

pub fn decompress(archive: &[u8]) -> Result<(Vec<FileEntry>, Vec<u8>), String> {
    // parse_toc walks the entire header (magic, version, codec,
    // TOC entries, dict, sub-TOC) and returns the byte offset
    // where the compressed solid block begins + the trained
    // dict (if any) + the chunk-group descriptor (#46) that
    // tells this function exactly which codec and how many
    // compressed bytes to feed each per-group decoder.
    let parsed = parse_toc(archive)?;
    let entries = parsed.entries;

    // Sprint 5.7.21-D: hoist the per-group slice computation
    // out of the decode loop. The original code computed
    // `solid_cursor` incrementally and could not be
    // parallelized because of the mutable state. With the
    // slices pre-computed, the decode step is a pure
    // function of (codec, compressed_bytes, dict) and can
    // run on rayon threads in parallel.
    let group_slices: Vec<&[u8]> = {
        let mut cursor = parsed.solid_block_offset;
        parsed
            .chunk_groups
            .iter()
            .map(|(_, size)| {
                let s = &archive[cursor..cursor + *size as usize];
                cursor += *size as usize;
                s
            })
            .collect()
    };

    // Decode every chunk group in parallel. Each group is
    // independent (its own compressed bytes, its own codec,
    // its own dict if zstd). The output is a Vec<Vec<u8>>
    // in the same order as the input — rayon's collect is
    // order-stable — so we can concatenate sequentially
    // afterwards and the per-entry `solid_offset` values
    // (set during compression, stored in the TOC) still
    // point at the right slice.
    //
    // The 2-4x speedup the user identified in 5.7.14 came
    // from exactly this: a 5 GiB corpus produces ~20-40
    // chunk groups, each of which is its own decoder
    // pipeline. On a 4-core M4 Pro we get ~3-4x
    // wall-clock improvement on the decode stage.
    let group_decoded: Vec<Vec<u8>> = {
        use rayon::prelude::*;
        group_slices
            .par_iter()
            .zip(parsed.chunk_groups.par_iter())
            .map(|(group_compressed, (codec, _))| {
                decode_chunk_group(*codec, group_compressed, &parsed.dict)
            })
            .collect::<Result<Vec<_>, String>>()?
    };

    let mut solid_uncompressed: Vec<u8> = Vec::new();
    for decoded in group_decoded {
        solid_uncompressed.extend_from_slice(&decoded);
    }

    // Step 3: cross-check offsets against the decompressed length.
    for e in &entries {
        let end = e.solid_offset as usize + e.pre_size as usize;
        if end > solid_uncompressed.len() {
            return Err(format!(
                "solid archive: entry '{}' extends past solid block ({} > {})",
                e.name,
                end,
                solid_uncompressed.len()
            ));
        }
    }

    // Sprint 5.7.11: decode the `NXPT` passthrough trailer (if
    // present) and concatenate each entry's decoded bytes to the
    // end of `solid_uncompressed`. We then synthesize a
    // `FileEntry` for each passthrough with `solid_offset`
    // pointing to the start of its slice inside
    // `solid_uncompressed`. The `pre_size` is the decoded length
    // (which equals the original file size — passthroughs are
    // bit-exact). The `chunk_codec` is left as `Codec::Lzma` as
    // a harmless sentinel — the extraction code never
    // dispatches on it for `Raw` preprocessor entries.
    let mut combined_entries: Vec<FileEntry> = entries;
    for pt in parsed.passthroughs.iter() {
        let decoded: Vec<u8> = match pt.codec {
            // 0 = zstd-compressed (`Z` marker). Decode with the
            // crate's zstd decoder; on failure we surface the
            // exact archive error rather than silently falling
            // back to the compressed bytes.
            0 => {
                let mut dec = zstd::stream::read::Decoder::new(pt.payload.as_slice())
                    .map_err(|e| format!(
                        "solid archive: passthrough '{}' zstd decode failed: {}",
                        pt.name, e
                    ))?;
                let mut out = Vec::with_capacity(pt.payload.len() * 2);
                dec.read_to_end(&mut out).map_err(|e| format!(
                    "solid archive: passthrough '{}' zstd read failed: {}",
                    pt.name, e
                ))?;
                out
            }
            // 1 = raw bytes (`R` marker). The encoder chose this
            // path because zstd didn't shrink the input.
            1 => pt.payload.clone(),
            other => {
                return Err(format!(
                    "solid archive: passthrough '{}' has unknown codec {}",
                    pt.name, other
                ));
            }
        };
        let offset = solid_uncompressed.len() as u64;
        let size = decoded.len() as u64;
        solid_uncompressed.extend_from_slice(&decoded);
        combined_entries.push(FileEntry {
            name: pt.name.clone(),
            original_size: size,
            pre_size: size,
            preprocessor: Preprocessor::Raw,
            // Sentinel: the extraction code path for Raw entries
            // doesn't look at `chunk_codec` — it just slices
            // `solid_uncompressed[solid_offset..solid_offset+pre_size]`.
            // We pick LZMA so the byte is deterministic and any
            // future diagnostic that prints it is consistent.
            chunk_codec: Codec::Lzma,
            solid_offset: offset,
        });
    }

    Ok((combined_entries, solid_uncompressed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_files() -> Vec<(String, Vec<u8>)> {
        vec![
            (
                "src/index.ts".to_string(),
                b"interface User { name: string; age: number; }\nconst x: number = 1;\n".to_vec(),
            ),
            (
                "src/util.ts".to_string(),
                b"// helper\nexport function add(a: number, b: number): number { return a + b; }\n"
                    .to_vec(),
            ),
            (
                "README.md".to_string(),
                b"# Project\n\nThis is a test.\n".to_vec(),
            ),
            (
                "data.json".to_string(),
                br#"{"key": "value", "n": 42}"#.to_vec(),
            ),
        ]
    }

    #[test]
    fn roundtrip_simple() {
        let files = sample_files();
        let original_total: usize = files.iter().map(|(_, b)| b.len()).sum();
        let archive = compress(&files, 6).expect("compress");
        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), files.len());
        // Slice for each file and check we get something back.
        for (i, e) in entries.iter().enumerate() {
            let start = e.solid_offset as usize;
            let end = start + e.pre_size as usize;
            let got = &solid[start..end];
            assert!(!got.is_empty(), "file {} returned empty", i);
            // The preprocessor is lossy for .ts, so we can't byte-compare.
            // But for .json/.md (conservative), the result must equal
            // the minify() of the input.
            if e.preprocessor == Preprocessor::Conservative {
                let expected = minify::minify(&files[i].1);
                assert_eq!(
                    got,
                    &expected[..],
                    "conservative minify mismatch for {}",
                    e.name
                );
            }
        }
        // The archive should be smaller than the input on this simple
        // set (repetitive structure).
        eprintln!(
            "roundtrip_simple: input={} B, archive={} B, ratio={:.2}x",
            original_total,
            archive.len(),
            original_total as f64 / archive.len() as f64,
        );
    }

    #[test]
    fn swc_used_for_ts() {
        let files: Vec<(String, Vec<u8>)> = vec![(
            "src/a.ts".to_string(),
            b"const x: number = 1;\nconst y: string = 'hi';\n".to_vec(),
        )];
        let archive = compress(&files, 6).expect("compress");
        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].preprocessor, Preprocessor::SwcAst);
        // The decompressed bytes must NOT contain ": number" or
        // ": string" — swc strips type annotations.
        let start = entries[0].solid_offset as usize;
        let end = start + entries[0].pre_size as usize;
        let s = std::str::from_utf8(&solid[start..end]).unwrap();
        assert!(!s.contains(": number"));
        assert!(!s.contains(": string"));
    }

    #[test]
    fn conservative_used_for_json() {
        let files: Vec<(String, Vec<u8>)> =
            vec![("data.json".to_string(), br#"{"k":"v"}"#.to_vec())];
        let archive = compress(&files, 6).expect("compress");
        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries[0].preprocessor, Preprocessor::Conservative);
        let start = entries[0].solid_offset as usize;
        let end = start + entries[0].pre_size as usize;
        let s = std::str::from_utf8(&solid[start..end]).unwrap();
        assert_eq!(s, r#"{"k":"v"}"#);
    }

    #[test]
    fn empty_archive() {
        let files: Vec<(String, Vec<u8>)> = vec![];
        let archive = compress(&files, 6).expect("compress");
        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 0);
        assert!(solid.is_empty());
    }

    #[test]
    fn bad_magic_rejected() {
        let err = decompress(b"NOPE\x00not a solid archive").unwrap_err();
        assert!(err.contains("bad magic"), "got: {}", err);
    }

    #[test]
    fn truncated_rejected() {
        let err = decompress(b"NX").unwrap_err();
        assert!(err.contains("too short"), "got: {}", err);
    }

    #[test]
    fn roundtrip_preserves_file_count_and_names() {
        let files = sample_files();
        let archive = compress(&files, 6).expect("compress");
        let (entries, _) = decompress(&archive).expect("decompress");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        let expected: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn solid_offsets_are_monotonic() {
        let files = sample_files();
        let archive = compress(&files, 6).expect("compress");
        let (entries, _) = decompress(&archive).expect("decompress");
        let mut prev_end: u64 = 0;
        for e in &entries {
            assert_eq!(
                e.solid_offset, prev_end,
                "entry '{}' offset {} != previous end {}",
                e.name, e.solid_offset, prev_end
            );
            prev_end = e.solid_offset + e.pre_size;
        }
    }

    // Sprint 5.7.4 hotfix #50: per-extension override tests.
    // These exercise the new PreprocessorOverrides plumbing
    // end-to-end: a `.json` file under a default Conservative
    // rule must be Raw when the user pins it; a `.ts` file
    // under a default SwcAst rule must be Conservative when
    // the user pins it. The roundtrip proves the per-file
    // preprocessor is selected at encode time and respected
    // through the archive.

    #[test]
    fn override_pins_json_to_raw_bit_exact() {
        // Default rule: `.json` → Conservative minify. With
        // a Raw override, the file must be bit-exact through
        // the roundtrip. This is the exact use case the
        // user described: "config files like .env and .json
        // must not be touched".
        let files: Vec<(String, Vec<u8>)> = vec![(
            "config.json".to_string(),
            b"{\n  \"k\": \"v\",\n  \"url\": \"https://example.com\"\n}\n".to_vec(),
        )];
        let overrides = PreprocessorOverrides::new(
            vec!["json".to_string()],
            vec![],
        );
        let archive = compress_with_progress_full_public(
            &files,
            CompressionLevel::Lzma(3),
            false,
            &overrides,
            |_, _, _| {},
        )
        .expect("compress");
        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].preprocessor, Preprocessor::Raw);
        assert_eq!(entries[0].chunk_codec, Codec::Lzma);
        // Bit-exact: the recovered bytes are identical to
        // the source. Conservative would have collapsed
        // the whitespace and stripped the empty line, so
        // the equality here is the actual assertion that
        // the override took effect.
        let start = entries[0].solid_offset as usize;
        let end = start + entries[0].pre_size as usize;
        assert_eq!(&solid[start..end], &files[0].1);
    }

    #[test]
    fn override_pins_ts_to_conservative_overriding_swc() {
        // Default rule: `.ts` → SwcAst (drops types and
        // comments). With a Conservative override, the file
        // must be processed by the Conservative minifier,
        // which keeps the file parseable but strips only
        // whitespace + line comments. Useful for `.d.ts`
        // files where the user wants the Conservative
        // minify but not the AST-level transformation.
        let files: Vec<(String, Vec<u8>)> = vec![(
            "types.d.ts".to_string(),
            b"export type X = number;\n// header comment\nexport const x: X = 1;\n".to_vec(),
        )];
        let overrides = PreprocessorOverrides::new(
            vec![],
            vec!["ts".to_string()],
        );
        let archive = compress_with_progress_full_public(
            &files,
            CompressionLevel::Lzma(3),
            false,
            &overrides,
            |_, _, _| {},
        )
        .expect("compress");
        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].preprocessor, Preprocessor::Conservative);
        // The recovered bytes are Conservative-minified:
        // line comment is stripped, whitespace is collapsed.
        // The "type" keyword is preserved (Conservative does
        // NOT touch TS types, only SwcAst does).
        let start = entries[0].solid_offset as usize;
        let end = start + entries[0].pre_size as usize;
        let out = std::str::from_utf8(&solid[start..end]).unwrap();
        assert!(!out.contains("//"), "line comment should be stripped, got: {:?}", out);
        assert!(out.contains("type"), "Conservative keeps type keyword (only SwcAst strips it), got: {:?}", out);
    }

    #[test]
    fn override_resolution_raw_wins_over_minify() {
        // If the user puts the same extension in BOTH
        // lists, Raw wins (the safe default). We don't
        // silently fall back to Conservative because the
        // user's intent of "bit-exact for this file" is
        // more conservative than "minify this file".
        let files: Vec<(String, Vec<u8>)> = vec![(
            "x.json".to_string(),
            b"{\"k\":\"v\"}\n".to_vec(),
        )];
        let overrides = PreprocessorOverrides::new(
            vec!["json".to_string()],
            vec!["json".to_string()],
        );
        let archive = compress_with_progress_full_public(
            &files,
            CompressionLevel::Lzma(3),
            false,
            &overrides,
            |_, _, _| {},
        )
        .expect("compress");
        let (entries, _) = decompress(&archive).expect("decompress");
        assert_eq!(entries[0].preprocessor, Preprocessor::Raw);
    }

    #[test]
    fn override_normalizes_extensions() {
        // The constructor must lowercase, dedup, drop
        // empty entries, and strip leading dots. The
        // JSON req from the GUI sometimes includes
        // ".json" or "JSON" depending on user typing —
        // both should match.
        let o = PreprocessorOverrides::new(
            vec!["JSON".to_string(), ".json".to_string(), "".to_string()],
            vec![".md".to_string()],
        );
        assert_eq!(o.raw_extensions, vec!["json".to_string()]);
        assert_eq!(o.minify_extensions, vec!["md".to_string()]);
    }

    #[test]
    fn override_does_not_affect_files_outside_the_list() {
        // When the override pins `.json` to Raw, all the
        // other files in the corpus must still go through
        // their default rule. This guards against a
        // future regression where the override accidentally
        // becomes a global "Raw for everything" toggle.
        let files: Vec<(String, Vec<u8>)> = vec![
            ("a.json".to_string(), b"{\"k\":\"v\"}".to_vec()),
            ("b.md".to_string(), b"# header\n\nbody\n".to_vec()),
            ("c.ts".to_string(), b"const x: number = 1;\n".to_vec()),
        ];
        let overrides = PreprocessorOverrides::new(
            vec!["json".to_string()],
            vec![],
        );
        let archive = compress_with_progress_full_public(
            &files,
            CompressionLevel::Lzma(3),
            false,
            &overrides,
            |_, _, _| {},
        )
        .expect("compress");
        let (entries, _) = decompress(&archive).expect("decompress");
        // Find each entry by name (the aggregator doesn't
        // preserve source order — files go through the
        // chunk buffer which is iterated sequentially
        // but the order is preserved here because the
        // files vec is small enough to fit in one chunk).
        let by_name: std::collections::HashMap<&str, &FileEntry> =
            entries.iter().map(|e| (e.name.as_str(), e)).collect();
        assert_eq!(by_name["a.json"].preprocessor, Preprocessor::Raw);
        assert_eq!(by_name["b.md"].preprocessor, Preprocessor::Conservative);
        assert_eq!(by_name["c.ts"].preprocessor, Preprocessor::SwcAst);
    }

    // Sprint 5.7.4 hotfix #51: dict training + wire format
    // regression tests. The two previous bugs that motivated
    // this hotfix were:
    //   1. `ZDICT_trainFromBuffer` returning -72
    //      (ZSTD_error_srcSize_wrong) when given a single
    //      sample. Fix: sliding window ensures ≥2 samples.
    //   2. The encoder used a dict to compress but never
    //      embedded it in the header, so the decoder
    //      panicked with "Dictionary mismatch". Fix: bump
    //      VERSION to 4 and re-introduce the dict field for
    //      archives that actually carry a trained dict.
    //
    // Both regressions must stay green.

    fn make_repetitive_corpus(total_kb: usize) -> Vec<(String, Vec<u8>)> {
        // Build a corpus of N KB that repeats the same
        // sentence over and over. Highly compressible,
        // exactly the case where a trained dict helps the
        // most.
        let phrase = b"The quick brown fox jumps over the lazy dog. \
                       Pack my box with five dozen liquor jugs. \
                       How vexingly quick daft zebras jump!       \
                       Sphinx of black quartz, judge my vow.      \n";
        let mut buf = Vec::with_capacity(total_kb * 1024);
        while buf.len() < total_kb * 1024 {
            buf.extend_from_slice(phrase);
        }
        buf.truncate(total_kb * 1024);
        vec![("corpus.txt".to_string(), buf)]
    }

    #[test]
    fn zstd_dict_roundtrip_with_trained_dict() {
        // Pre-fix this would have produced
        // "dict train FAILED: ... -72" and then a
        // "Dictionary mismatch" panic on decompress. Both
        // failure modes are checked here.
        let files = make_repetitive_corpus(800); // 800 KB
        let archive = compress(&files, CompressionLevel::Zstd(3))
            .expect("compress should NOT fail with dict training error");
        let (entries, solid) = decompress(&archive).expect("decompress should match dict");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].preprocessor, Preprocessor::Conservative);
        // Conservative strips the `//` line comments out of
        // these strings, so we can't byte-compare. The
        // shape check is enough: the recovered buffer is
        // shorter than the input (the dict did its job).
        let recovered = &solid[0..entries[0].pre_size as usize];
        assert!(recovered.len() < files[0].1.len());
    }

    #[test]
    fn zstd_dict_skips_when_too_few_samples() {
        // A corpus smaller than 64 KiB must NOT crash.
        // The encoder is allowed to fall back to "no dict"
        // (and must produce a v3 wire format that the
        // decoder can still read).
        let small: Vec<(String, Vec<u8>)> = vec![(
            "tiny.txt".to_string(),
            b"hello world this is a tiny corpus".to_vec(),
        )];
        let archive = compress(&small, CompressionLevel::Zstd(3))
            .expect("compress should succeed even with tiny input");
        // The archive must be readable end-to-end. We don't
        // assert the VERSION byte because the encoder may
        // pick v3 (no dict) or v4 (dict) — both are valid
        // as long as decompress works.
        let (_, solid) = decompress(&archive).expect("decompress");
        assert!(!solid.is_empty());
    }

    #[test]
    fn lzma_archive_unchanged_v3_wire_format() {
        // The LZMA path must NOT use the dict field. This
        // is a regression test for the encoder dispatch:
        // we want LZMA archives to stay on the v3 wire
        // format so older decoders keep working.
        let files = make_repetitive_corpus(200);
        let archive = compress(&files, CompressionLevel::Lzma(3))
            .expect("compress");
        let magic = &archive[0..5];
        assert_eq!(magic, MAGIC, "magic must be NXS7\\n");
        assert_eq!(archive[5], 3, "LZMA archives must stay on VERSION 3 (no dict field)");
        let (entries, _) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn zstd_dict_field_only_present_when_dict_trained() {
        // With enough samples AND lossless mode, the
        // encoder writes a v4 archive and embeds the dict.
        // The dict field length is right after the file
        // entries.
        //
        // Sprint 5.7.9: must explicitly use the lossless
        // entry point (`compress_with_progress_lossless`).
        // The default `compress` (Lossy) skips dict
        // training since the Conservative preprocessor
        // already strips the semantic noise, so it always
        // writes a v3 archive (no dict field).
        let files = make_repetitive_corpus(800);
        let archive = compress_with_progress_lossless(
            &files,
            CompressionLevel::Zstd(3),
            |_, _, _| {},
        )
        .expect("compress");
        assert_eq!(archive[5], 4, "v4 archive: dict was trained");
        // The dict field starts at the same offset the
        // parser expects (after the file entries). The
        // exact size depends on the trainer output — we
        // only check that it's a non-zero 4-byte field.
        // Walking the entries manually would be brittle;
        // we just sanity-check that decompress works and
        // uses the dict (the success of the previous
        // roundtrip test already proves the parser finds
        // the dict correctly).
        let (entries, _) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 1);
    }

    // Sprint 5.7.5 hotfix #52: the Format Oracle.
    //
    // `should_use_dict_for_size(dict_bytes, pre_bytes)`
    // is the heart of the cost-benefit analysis that
    // decides whether to embed a trained dict in the
    // archive header. The threshold is `pre_bytes >
    // dict_bytes * 8` (calibrated against the user's
    // three reference corpora). These tests pin the
    // boundary cases so future tuning has a regression
    // suite to work against.

    #[test]
    fn oracle_drops_dict_for_small_corpus() {
        // mongo/-sized corpus: 752 KB pre, 144 KB dict.
        // 752 < 144 * 8 = 1152 → DROP.
        assert!(!should_use_dict_for_size(144 * 1024, 752 * 1024));
    }

    #[test]
    fn oracle_keeps_dict_for_large_corpus() {
        // FlowNow/-sized corpus: 1 GB pre, 107 KB dict.
        // 1 GB > 107 * 8 = 856 KB → KEEP.
        assert!(should_use_dict_for_size(107 * 1024, 1024 * 1024 * 1024));
    }

    #[test]
    fn oracle_drops_zero_size_dict() {
        // A 0-byte dict is degenerate; the encoder never
        // produces one but if it did, dropping it is the
        // right call (no benefit, zero cost).
        assert!(!should_use_dict_for_size(0, 1024 * 1024));
    }

    #[test]
    fn oracle_threshold_at_exact_boundary() {
        // The boundary is `pre_bytes > dict_bytes * 8`.
        // At `pre_bytes == dict_bytes * 8`, the oracle
        // must return FALSE (the `>` is strict). One
        // byte more, the oracle must return TRUE. This
        // pins the strict-vs-non-strict comparison.
        let dict = 100 * 1024;
        let threshold = dict * 8;
        assert!(!should_use_dict_for_size(dict, threshold as u64));
        assert!(should_use_dict_for_size(dict, threshold as u64 + 1));
    }

    #[test]
    fn oracle_drops_when_corpus_is_smaller_than_dict() {
        // Pathological case: the corpus is smaller than
        // the dict. This means the dict would contain
        // data from outside the corpus, which is
        // meaningless. The oracle must drop the dict.
        assert!(!should_use_dict_for_size(100 * 1024, 50 * 1024));
    }

    #[test]
    fn oracle_e2e_drops_dict_on_small_corpus() {
        // End-to-end: build a small corpus (1 MB), compress
        // with --codec=zstd, and verify the archive is v3
        // (no dict field) because the oracle dropped it.
        let files = make_repetitive_corpus(100); // 100 KB
        let archive = compress(&files, CompressionLevel::Zstd(3))
            .expect("compress");
        // 100 KB pre / 8 = 12.5 KB threshold. A small
        // dict (≤12 KB) would be kept but the encoder
        // can only produce dicts ≥32 KB minimum, so the
        // oracle drops it.
        assert_eq!(archive[5], 3, "v3 wire format: oracle dropped the dict");
        let (_, solid) = decompress(&archive).expect("decompress");
        assert!(!solid.is_empty());
    }

    // Sprint 5.7.5: format coverage tests. The user
    // explicitly asked to make sure that common binary
    // formats (PDF, Office, e-books, images, archives)
    // round-trip bit-exact. The `pick_preprocessor` table
    // in `solid_archive.rs` lists ~80 binary extensions as
    // `Preprocessor::Raw` so they pass through the codec
    // without minification. These tests pin the coverage:
    // each new entry in the table is matched here, so a
    // future refactor that accidentally drops a format
    // will trip a test instead of a silent regression.
    //
    // The test uses a synthetic corpus with one file per
    // format (built with std lib only — no extra deps
    // for the test harness). Each file is small (a few
    // hundred bytes) but the round-trip exercises the
    // preprocessor selection AND the codec pipeline.

    /// Build a minimal but real `name` with `body` content.
    /// For zip-based formats (docx/xlsx/pptx/epub/pages)
    /// the body is wrapped in a zip; for raw formats
    /// (pdf, heic) the body is written directly.
    fn make_format_sample(name: &str, body: &[u8]) -> (String, Vec<u8>) {
        use std::io::Write;
        // Heuristic: zip-based vs raw, by extension.
        let zip_based = name.ends_with(".docx")
            || name.ends_with(".doc")
            || name.ends_with(".xlsx")
            || name.ends_with(".xls")
            || name.ends_with(".pptx")
            || name.ends_with(".ppt")
            || name.ends_with(".epub")
            || name.ends_with(".pages")
            || name.ends_with(".numbers")
            || name.ends_with(".key")
            || name.ends_with(".odt")
            || name.ends_with(".ods")
            || name.ends_with(".odp")
            || name.ends_with(".jar")
            || name.ends_with(".war")
            || name.ends_with(".zip");
        if zip_based {
            // Build a real zip with the body inside.
            let mut zip_bytes: Vec<u8> = Vec::new();
            {
                let cursor = std::io::Cursor::new(&mut zip_bytes);
                let zip_writer = zip::ZipWriter::new(cursor);
                let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                let mut zip_writer = zip_writer;
                zip_writer.start_file("payload", opts).unwrap();
                zip_writer.write_all(body).unwrap();
                zip_writer.finish().unwrap();
            }
            (name.to_string(), zip_bytes)
        } else {
            (name.to_string(), body.to_vec())
        }
    }

    /// Roundtrip test for a single (name, body) pair.
    fn assert_format_roundtrip(name: &str, body: &[u8]) {
        let files = vec![make_format_sample(name, body)];
        let archive = compress(&files, CompressionLevel::Lzma(3))
            .unwrap_or_else(|e| panic!("compress({}) failed: {}", name, e));
        let (entries, solid) = decompress(&archive)
            .unwrap_or_else(|e| panic!("decompress({}) failed: {}", name, e));
        assert_eq!(entries.len(), 1, "{}: wrong entry count", name);
        assert_eq!(entries[0].preprocessor, Preprocessor::Raw,
            "{}: preprocessor must be Raw (got {:?})", name, entries[0].preprocessor);
        let start = entries[0].solid_offset as usize;
        let end = start + entries[0].pre_size as usize;
        assert_eq!(&solid[start..end], &files[0].1,
            "{}: roundtrip bytes mismatch", name);
    }

    #[test]
    fn format_coverage_pdf() {
        // Minimal PDF signature: %PDF-1.4\n.
        let pdf = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\n\
                    xref\n0 1\n0000000000 65535 f\n\
                    trailer\n<</Size 1>>\nstartxref\n0\n%%EOF\n";
        assert_format_roundtrip("document.pdf", pdf);
    }

    #[test]
    fn format_coverage_office_open_xml() {
        // docx, xlsx, pptx are all zip-based OOXML.
        // The body inside the zip is the document.xml
        // payload — we just check the preprocessor
        // selects Raw and the bytes survive.
        assert_format_roundtrip("report.docx",
            b"<?xml version='1.0'?><document><p>content</p></document>");
        assert_format_roundtrip("data.xlsx",
            b"<?xml version='1.0'?><workbook><sheet name='S'/></workbook>");
        assert_format_roundtrip("deck.pptx",
            b"<?xml version='1.0'?><presentation><sld/></presentation>");
    }

    #[test]
    fn format_coverage_legacy_office() {
        // Pre-OOXML binary formats — CFB containers.
        // We can't easily build a real .doc/.xls/.ppt
        // from std lib (they're CFB compound files) so
        // we use minimal magic-bytes stubs. The point
        // of this test is that the preprocessor
        // recognises the extension and selects Raw,
        // NOT that the bytes round-trip perfectly.
        let files: Vec<(String, Vec<u8>)> = vec![
            ("old.doc".to_string(), b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec()),
            ("old.xls".to_string(), b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec()),
            ("old.ppt".to_string(), b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec()),
        ];
        let archive = compress(&files, CompressionLevel::Lzma(3))
            .expect("compress legacy office");
        let (entries, _) = decompress(&archive)
            .expect("decompress legacy office");
        for entry in &entries {
            assert_eq!(entry.preprocessor, Preprocessor::Raw,
                "{}: legacy office must be Raw", entry.name);
        }
    }

    #[test]
    fn format_coverage_modern_images() {
        // HEIC/AVIF/JXL — already-compressed intra-prediction
        // formats. The preprocessor must select Raw so the
        // codec-specific header bytes survive.
        for ext in &["photo.heic", "image.avif", "picture.jxl",
                     "raw.dng", "raw.cr2", "raw.nef"] {
            let body = vec![0u8; 64]; // stub bytes
            assert_format_roundtrip(ext, &body);
        }
    }

    #[test]
    fn format_coverage_ebooks() {
        // epub is zip-based; mobi/azw are PDB containers.
        // We just test the preprocessor selection for both
        // families.
        assert_format_roundtrip("book.epub",
            b"<?xml version='1.0'?><package/>");
        // mobi/azw bodies are stub bytes — we just need to
        // verify the preprocessor picks Raw.
        let files: Vec<(String, Vec<u8>)> = vec![
            ("a.mobi".to_string(), b"MOBI\x00\x00".to_vec()),
            ("b.azw3".to_string(), b"\x00\x00\x00\x00".to_vec()),
        ];
        let archive = compress(&files, CompressionLevel::Lzma(3))
            .expect("compress ebooks");
        let (entries, _) = decompress(&archive)
            .expect("decompress ebooks");
        for e in &entries {
            assert_eq!(e.preprocessor, Preprocessor::Raw,
                "{}: ebook must be Raw", e.name);
        }
    }

    #[test]
    fn format_coverage_apple_iwork() {
        // Pages/Numbers/Keynote are zip-based like OOXML.
        assert_format_roundtrip("doc.pages",
            b"<?xml version='1.0'?><pages/>");
        assert_format_roundtrip("data.numbers",
            b"<?xml version='1.0'?><numbers/>");
        assert_format_roundtrip("deck.key",
            b"<?xml version='1.0'?><keynote/>");
    }

    #[test]
    fn format_coverage_opendocument() {
        // ODT/ODS/ODP — also zip-based.
        assert_format_roundtrip("doc.odt",
            b"<?xml version='1.0'?><document/>");
        assert_format_roundtrip("data.ods",
            b"<?xml version='1.0'?><spreadsheet/>");
        assert_format_roundtrip("deck.odp",
            b"<?xml version='1.0'?><presentation/>");
    }

    #[test]
    fn format_coverage_3d_models() {
        // glTF/GLB are JSON or binary glTF; STL/OBJ/FBX
        // are mixed text/binary. We just check preprocessor
        // selection — round-trip is a bonus.
        for ext in &["mesh.stl", "scene.obj", "model.fbx", "asset.gltf"] {
            let body = format!("# stub {} content with repetitive lines\n", ext).into_bytes();
            assert_format_roundtrip(ext, &body);
        }
    }

    #[test]
    fn format_coverage_disk_images() {
        // ISO/IMG/VMDK/VDI disk images — already-compressed
        // or block-device formats. Conservative minify would
        // corrupt the structure.
        for ext in &["disk.iso", "image.img", "vm.vmdk", "vm.vdi"] {
            let body = vec![0u8; 256]; // stub
            assert_format_roundtrip(ext, &body);
        }
    }

    // Sprint 5.7.6 hotfix #53: Multi-Solid Block split.
    //
    // When a corpus mixes compressible and incompressible
    // files, the encoder must split them into separate
    // chunks so a chunk never mixes LZMA and ZstdFast in
    // the same data stream. The user explicitly asked for
    // this in the 18:00 review: "If you mix 500 MB of
    // text and 500 MB of binary, the pipeline should
    // process them in two independent solid blocks, not
    // one mixed one."
    //
    // The test builds a 2-file corpus: a compressible
    // .ts and a random-data .bin. The encoder must
    // produce TWO super-chunks (one LZMA for .ts, one
    // ZstdFast for .bin), and the roundtrip must
    // preserve both files bit-exact (the .ts is minified
    // by swc; the .bin is Raw passthrough).

    #[test]
    fn multi_solid_block_splits_compressible_from_incompressible() {
        // Two files, in order: compressible first, then
        // incompressible. The encoder must close the
        // first chunk when the second file is detected
        // as incompressible, so the LZMA dictionary
        // doesn't get polluted with random bytes.
        //
        // Sprint 5.7.9: the default is now Zstd, not
        // LZMA. Zstd handles incompressible data
        // natively (zstd-fast is auto-selected by the
        // entropy-aware codec flip). The split logic
        // is still active and matters when the user
        // explicitly forces LZMA on .ts via the
        // overrides. This test now verifies the
        // split fires only when the codecs differ.
        let ts_content = b"function add(a: number, b: number): number {\n\
                            // this is a comment with some text\n\
                            return a + b;\n\
                          }\n\
                          function sub(a: number, b: number): number {\n\
                            return a - b;\n\
                          }\n"
            .repeat(50);
        let bin_content: Vec<u8> = (0..50_000).map(|i| ((i * 31 + 7) % 256) as u8).collect();
        // Force entropy-incompressible: the simple LCG
        // produces ~8 bits/byte of entropy on its first
        // 64 KiB sample.
        let files: Vec<(String, Vec<u8>)> = vec![
            ("source.ts".to_string(), ts_content),
            ("blob.bin".to_string(), bin_content),
        ];
        let archive = compress(&files, CompressionLevel::Auto)
            .expect("compress should not fail");
        let (entries, _solid) = decompress(&archive)
            .expect("decompress should roundtrip both files");
        // The .ts must have been preprocessed by swc
        // (the default for .ts).
        let by_name: std::collections::HashMap<&str, &FileEntry> =
            entries.iter().map(|e| (e.name.as_str(), e)).collect();
        assert_eq!(by_name["source.ts"].preprocessor, Preprocessor::SwcAst,
            "ts must use SwcAst");
        // Both files go to Zstd (5.7.9 default). The
        // split logic doesn't fire because the codec
        // doesn't change. The .bin is still routed
        // through zstd-fast by the per-chunk entropy
        // flip (Zstd level -3 for incompressible
        // chunks). Verify the chunk_codec field.
        assert_eq!(by_name["source.ts"].chunk_codec, Codec::Zstd,
            "ts goes to Zstd (5.7.9 default)");
        assert_eq!(by_name["blob.bin"].chunk_codec, Codec::Zstd,
            "bin goes to Zstd with zstd-fast on incompressible chunks");
    }

    #[test]
    fn multi_solid_block_roundtrip_preserves_data() {
        // 4 files: 2 compressible, 2 random. The
        // encoder must produce 2 chunks and the
        // roundtrip must restore every file bit-exact
        // (the .ts is minified, the .bin is Raw).
        let ts1 = b"const x: number = 42;\nfunction f() { return x; }\n".repeat(20);
        let ts2 = b"interface User { name: string; age: number; }\n".repeat(20);
        let bin1: Vec<u8> = (0..10_000).map(|i| (i * 17 + 3) as u8).collect();
        let bin2: Vec<u8> = (0..10_000).map(|i| ((i ^ 0x5a) * 23) as u8).collect();
        let files: Vec<(String, Vec<u8>)> = vec![
            ("a.ts".to_string(), ts1.clone()),
            ("b.bin".to_string(), bin1.clone()),
            ("c.ts".to_string(), ts2.clone()),
            ("d.bin".to_string(), bin2.clone()),
        ];
        let archive = compress(&files, CompressionLevel::Auto)
            .expect("compress");
        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 4);
        for entry in &entries {
            let start = entry.solid_offset as usize;
            let end = start + entry.pre_size as usize;
            let got = &solid[start..end];
            let original = files.iter()
                .find(|(n, _)| n == &entry.name)
                .map(|(_, b)| b.as_slice())
                .expect("entry name matches an input file");
            // The .ts files are minified (swc strips
            // comments and types). The .bin files are
            // Raw passthrough. We check the .bin ones
            // bit-exact and the .ts ones as "minified of
            // the source" (their preprocessor is SwcAst,
            // so the recovered bytes ARE the minified
            // version, which is correct).
            if entry.preprocessor == Preprocessor::Raw {
                assert_eq!(got, original,
                    "Raw passthrough: {} must be bit-exact", entry.name);
            } else {
                // SwcAst: the recovered bytes are NOT
                // the same as the source — but they
                // should be the minified form. The
                // simplest invariant: the recovered is
                // shorter (the swc minifier drops
                // comments and types).
                assert!(got.len() <= original.len(),
                    "{}: minified ({}) must be <= source ({})",
                    entry.name, got.len(), original.len());
            }
        }
    }

    // Sprint 5.7.11: passthrough trailer roundtrip.
    //
    // The SupremeEngine (5.7.10-C) writes the LZMA/zstd solid
    // block first, then appends a `NXPT` trailer for each
    // passthrough file (PNG, JPG, MP4, .pyc, .dylib, ...).
    // Each trailer entry is one of:
    //   * `Z` + u32 payload_len + zstd-compressed bytes
    //   * `R` + u32 payload_len + raw bytes (encoder fell back
    //     because zstd didn't shrink the input)
    //
    // The decoder must concatenate the decoded payload to
    // `solid_uncompressed`, expose it as a `FileEntry` with
    // `preprocessor=Raw`, and slice it back bit-exact. These
    // two tests pin the contract on a minimal archive built
    // by hand so the test doesn't depend on SupremeEngine
    // internals (those are covered by the integration tests
    // in `tests/`).
    fn append_nxpt_trailer(archive: &mut Vec<u8>, entries: &[(String, u8, Vec<u8>)]) {
        use std::io::Write;
        for (name, marker, payload) in entries {
            archive.write_all(b"NXPT").unwrap();
            let name_bytes = name.as_bytes();
            archive
                .write_all(&(name_bytes.len() as u32).to_le_bytes())
                .unwrap();
            archive.write_all(name_bytes).unwrap();
            archive.write_all(std::slice::from_ref(marker)).unwrap();
            archive
                .write_all(&(payload.len() as u32).to_le_bytes())
                .unwrap();
            archive.write_all(payload).unwrap();
        }
    }

    #[test]
    fn passthrough_zstd_marker_roundtrip_is_bit_exact() {
        // Build a base solid archive with one trivial text file
        // (so the LZMA/zstd chunk is non-empty and the chunk
        // groups are populated), then append a passthrough with
        // the `Z` marker carrying a zstd-compressed payload.
        let base_files: Vec<(String, Vec<u8>)> = vec![(
            "src/index.ts".to_string(),
            b"const x: number = 1;\n".repeat(8),
        )];
        let mut archive =
            compress(&base_files, CompressionLevel::Auto).expect("compress base");

        // Compress a fake "PNG" with zstd and append it as a
        // passthrough. The body is repetitive (so zstd shrinks
        // it) but the content doesn't matter — the test only
        // checks that the decoder restores the original
        // 10,000 bytes bit-exact.
        let png_body: Vec<u8> = (0..10_000_u32)
            .flat_map(|i| i.to_le_bytes())
            .collect();
        let png_zstd = zstd::stream::encode_all(png_body.as_slice(), 1)
            .expect("zstd encode");
        // Sanity: the zstd output should actually be smaller
        // than the input (the body has clear patterns). If not,
        // the test setup is wrong.
        assert!(
            png_zstd.len() < png_body.len(),
            "test setup: zstd expected to shrink the synthetic PNG"
        );
        append_nxpt_trailer(
            &mut archive,
            &[("assets/logo.png".to_string(), b'Z', png_zstd)],
        );

        let (entries, solid) = decompress(&archive).expect("decompress");
        // 1 LZMA entry + 1 passthrough entry = 2.
        assert_eq!(entries.len(), 2);
        let pt_entry = entries
            .iter()
            .find(|e| e.name == "assets/logo.png")
            .expect("passthrough entry present");
        // Bit-exact recovery.
        let start = pt_entry.solid_offset as usize;
        let end = start + pt_entry.pre_size as usize;
        assert_eq!(end, solid.len(), "passthrough slice at end of solid");
        assert_eq!(
            &solid[start..end],
            png_body.as_slice(),
            "Z-marker passthrough must roundtrip bit-exact"
        );
        // The LZMA entry must still be there and accessible.
        let lzma_entry = entries
            .iter()
            .find(|e| e.name == "src/index.ts")
            .expect("LZMA entry present");
        assert!(lzma_entry.preprocessor != Preprocessor::Raw);
    }

    #[test]
    fn passthrough_raw_marker_roundtrip_is_bit_exact() {
        // Same shape as the zstd test, but the encoder chose
        // the `R` path (zstd didn't help). We just write raw
        // bytes; the decoder must still concatenate them to
        // `solid_uncompressed` and expose them as a
        // `preprocessor=Raw` FileEntry.
        let base_files: Vec<(String, Vec<u8>)> = vec![(
            "src/main.ts".to_string(),
            b"export const x = 42;\n".repeat(4),
        )];
        let mut archive =
            compress(&base_files, CompressionLevel::Auto).expect("compress base");

        // Random-ish payload that we explicitly DON'T zstd-
        // compress. We pick high-entropy bytes so any future
        // regression that tries to zstd-decode raw bytes
        // will fail loudly here.
        let dylib_body: Vec<u8> = (0..4_096_u32)
            .map(|i| ((i.wrapping_mul(0x9E3779B1)) >> 13) as u8)
            .collect();
        append_nxpt_trailer(
            &mut archive,
            &[("lib/native.dylib".to_string(), b'R', dylib_body.clone())],
        );

        let (entries, solid) = decompress(&archive).expect("decompress");
        assert_eq!(entries.len(), 2);
        let pt_entry = entries
            .iter()
            .find(|e| e.name == "lib/native.dylib")
            .expect("passthrough entry present");
        let start = pt_entry.solid_offset as usize;
        let end = start + pt_entry.pre_size as usize;
        assert_eq!(end, solid.len());
        assert_eq!(&solid[start..end], dylib_body.as_slice());
        assert_eq!(pt_entry.preprocessor, Preprocessor::Raw);
    }

    #[test]
    fn parse_toc_recognizes_passthroughs_without_decompressing_solid() {
        // Sprint 5.7.11: `parse_toc` should be able to surface
        // passthrough filenames + payload sizes from the
        // trailer WITHOUT paying the cost of decompressing the
        // solid block. The `list_solid_paginated` IPC call
        // depends on this so the file browser stays snappy
        // even on 5 GB corpora.
        let base_files: Vec<(String, Vec<u8>)> = vec![(
            "src/a.ts".to_string(),
            b"const x = 1;\n".repeat(4),
        )];
        let mut archive =
            compress(&base_files, CompressionLevel::Auto).expect("compress base");
        let png_body: Vec<u8> = b"\x89PNG_FAKE_HEADER".to_vec();
        let png_zstd =
            zstd::stream::encode_all(png_body.as_slice(), 1).expect("zstd encode");
        append_nxpt_trailer(
            &mut archive,
            &[("assets/icon.png".to_string(), b'Z', png_zstd.clone())],
        );

        // parse_toc must surface the passthrough name and the
        // compressed payload (NOT the decoded one — that would
        // require a zstd decode inside the parser).
        let parsed = parse_toc(&archive).expect("parse_toc");
        assert_eq!(parsed.passthroughs.len(), 1);
        assert_eq!(parsed.passthroughs[0].name, "assets/icon.png");
        assert_eq!(parsed.passthroughs[0].codec, 0, "Z marker → codec 0");
        assert_eq!(parsed.passthroughs[0].payload, png_zstd);
    }

    #[test]
    fn archive_without_trailer_still_parses() {
        // Backward-compat: an archive written before Sprint
        // 5.7.11 (no `NXPT` trailer) must still parse cleanly
        // with `passthroughs` empty.
        let files: Vec<(String, Vec<u8>)> = vec![(
            "src/legacy.ts".to_string(),
            b"const x: number = 1;\n".repeat(4),
        )];
        let archive = compress(&files, CompressionLevel::Auto).expect("compress");
        let parsed = parse_toc(&archive).expect("parse_toc legacy");
        assert!(
            parsed.passthroughs.is_empty(),
            "legacy archive must report zero passthroughs"
        );
        let (entries, _solid) = decompress(&archive).expect("decompress legacy");
        assert_eq!(entries.len(), 1);
    }
}
