//! Pluggable compression backends.
//!
//! ## Why a trait?
//!
//! NexusCompress v4 is the default backend — the multi-stream LZ77 +
//! rANS + dict codec you've been iterating on for 4 sprints. v5 is a
//! separate backend (LZMA via the `xz2` crate) that's strictly better
//! at ratio on most inputs but slower. Rather than rewrite v4, we
//! expose both as implementations of the same trait and let the
//! caller (CLI flag, Tauri option, or bench selector) pick.
//!
//! ## The trait contract
//!
//! A `CompressionEngine` is a whole-input compressor. You hand it a
//! `&[u8]` and get a `Vec<u8>` back. The format is engine-specific —
//! each engine tags its output with a small prefix (a single byte
//! for the engine ID) so `decompress` can dispatch to the right
//! decoder. The archive layer (`nxar`) wraps the per-file compressed
//! bytes in its own container, so the engine prefix is preserved
//! per entry.
//!
//! The `name()` and `bench_label()` are for the bench suite output
//! and the Tauri settings panel. They should be short (1-2 words)
//! and human-readable.

use crate::codec;

/// A whole-input compression backend.
pub trait CompressionEngine: Send + Sync {
    /// One-word identifier. Used in CLI flags (`--backend v4`),
    /// the bench table, and as the format prefix byte so the
    /// decompressor can dispatch.
    fn name(&self) -> &'static str;

    /// Pretty label for the bench suite markdown table.
    /// Should include any prefix flags (e.g. "v4 (fast)", "v5 LZMA -9").
    fn bench_label(&self) -> &'static str;

    /// Compress `input` and return the encoded bytes. The first
    /// byte MUST be a format identifier that `decompress` knows
    /// how to dispatch on.
    fn compress(&self, input: &[u8]) -> Vec<u8>;

    /// Decompress `encoded`. The first byte determines which engine
    /// is used (so callers don't have to specify — the format is
    /// self-describing). Returns an error string for invalid input.
    fn decompress(&self, encoded: &[u8]) -> Result<Vec<u8>, String>;

    /// Engine-specific format byte. The same value is written as
    /// the first byte of `compress()` output and read at the start
    /// of `decompress()`.
    fn format_byte(&self) -> u8;
}

// ---------------------------------------------------------------------------
// Engine IDs (first byte of every compressed stream)
// ---------------------------------------------------------------------------

/// NexusCompress v4 multi-stream format. The codec writes
/// `NexusHeader` (4-byte magic + version + ...) followed by per-block
/// headers. We tag the engine as `0x04` (v4's format version).
pub const V4_FORMAT_BYTE: u8 = 0x04;

/// NexusCompress v5 LZMA format. The codec writes a single LZMA2
/// stream prefixed by a 1-byte engine tag. Simple and self-describing.
pub const V5_LZMA_FORMAT_BYTE: u8 = 0x05;

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Decompress any compressed stream produced by either engine.
/// Reads the first byte (or the v4 magic for legacy compatibility)
/// and dispatches to the right engine. This is the single entry
/// point the archive layer (`nxar`) and the CLI use — they don't
/// need to know which engine produced the bytes.
///
/// The v4 engine uses the 4-byte magic `NXS\x00` (0x4E 0x58 0x53
/// 0x00) as its own identifier. The v5 LZMA engine uses a single
/// 0x05 tag byte. We detect both.
pub fn decompress_any(encoded: &[u8]) -> Result<Vec<u8>, String> {
    if encoded.is_empty() {
        return Err("empty compressed stream".into());
    }
    // v4 magic: "NXS\x00" (little-endian "NXS" + null terminator)
    if encoded.len() >= 4 && &encoded[0..4] == b"NXS\x00" {
        return V4Engine.decompress(encoded);
    }
    // v5 LZMA: single-byte tag
    if encoded[0] == V5_LZMA_FORMAT_BYTE {
        return LzmaEngine::new(9).decompress(encoded);
    }
    Err(format!(
        "unknown engine format byte 0x{:02x} (only v4 NXS magic and v5 LZMA=0x05 supported)",
        encoded[0]
    ))
}

/// Compress with an engine chosen by name (`"v4"`, `"v5"`, `"v5-min"`).
/// Returns the tagged compressed bytes. Used by the CLI's
/// `--backend` flag.
pub fn compress_with(backend: &str, input: &[u8], pre_minify: bool) -> Result<Vec<u8>, String> {
    // Optional pre-filter: minify code/text before compression.
    // The minifier knows about .js/.ts/.json/.md; for other content
    // it returns the input unchanged. The pre-filter doesn't know
    // what extension the bytes came from (the CLI/UI passes raw
    // bytes), so it always applies conservative text-stripping
    // when the content looks like text (UTF-8 printable) and skips
    // when it looks binary.
    let payload: Vec<u8> = if pre_minify {
        crate::minify::minify(input)
    } else {
        input.to_vec()
    };
    match backend {
        "v4" => Ok(V4Engine.compress(&payload)),
        "v5" => Ok(LzmaEngine::new(6).compress(&payload)),
        "v5-min" => Ok(LzmaEngine::new(9).compress(&payload)),
        "v5-extreme" => Ok(LzmaEngine::new(9).compress(&payload)),
        "v6" => Ok(compress_v6(input, None, 6)),
        "v6-extreme" => Ok(compress_v6(input, None, 9)),
        other => Err(format!(
            "unknown backend '{other}' (use v4, v5, v5-min, v5-extreme, v6, v6-extreme)"
        )),
    }
}

/// v6 backend: LZMA + smart preprocessor.
///
/// The preprocessor is chosen by file extension when `ext_hint` is
/// `Some`, otherwise by content sniffing:
/// - `.js` / `.jsx` / `.mjs` / `.cjs` / `.ts` / `.tsx`:
///   `swc_core` AST minify (lossy — drops comments, formatting,
///   identifier names; preserves runtime semantics).
/// - Anything else that looks like text: conservative
///   `crate::minify` (lossless — strips comments, collapses
///   whitespace, but keeps all original characters).
/// - Binary content: pass through unchanged.
///
/// **Honest contract:** v6 output is NOT byte-identical to the
/// input. After decompression you get the minified source (or the
/// original if the file was binary). The `ext_hint` lets the CLI
/// pass the file extension from the path so we can pick the right
/// preprocessor without sniffing.
pub fn compress_v6(input: &[u8], ext_hint: Option<&str>, lzma_level: u32) -> Vec<u8> {
    let ext = ext_hint.map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    let is_js_family = matches!(ext.as_str(), "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx");
    let pre: Vec<u8> = if is_js_family {
        // AST minify. Lossy. If parsing fails (rare for .js/.ts),
        // the ast_minify module falls back to passing the input
        // through unchanged.
        crate::ast_minify::minify(input).bytes
    } else {
        // Conservative text minify. Returns input unchanged for
        // binary content (detected via NUL bytes / non-UTF-8).
        crate::minify::minify(input)
    };
    LzmaEngine::new(lzma_level).compress(&pre)
}

/// All backends registered with their public names. Used by the
/// CLI's `--help` and the Tauri settings panel.
pub fn list_backends() -> &'static [&'static str] {
    &["v4", "v5", "v5-min", "v5-extreme", "v6", "v6-extreme"]
}

// ---------------------------------------------------------------------------
// v4 engine (wraps the existing `crate::codec::compress`)
// ---------------------------------------------------------------------------

/// The current NexusCompress v4 backend. Multi-block LZ77 + rANS +
/// dict codec. `compress()` writes a `NexusHeader` (4 bytes) + per-
/// block headers, so it already has a self-describing format — we
/// re-emit that as-is.
pub struct V4Engine;

impl CompressionEngine for V4Engine {
    fn name(&self) -> &'static str {
        "v4"
    }
    fn bench_label(&self) -> &'static str {
        "NexusCompress v4"
    }
    fn format_byte(&self) -> u8 {
        V4_FORMAT_BYTE
    }

    fn compress(&self, input: &[u8]) -> Vec<u8> {
        // The v4 codec doesn't prefix the format byte (the magic
        // `NXS\x00` is its own identifier). The dispatch in
        // `decompress_any` matches on the magic too, so we don't
        // need to inject the 0x04 byte here. The MAGIC is the
        // format byte for v4.
        codec::compress(input)
    }

    fn decompress(&self, encoded: &[u8]) -> Result<Vec<u8>, String> {
        // Sprint 5.6.4: codec::decompress now returns Result
        // directly. Still wrap with catch_unwind as a belt-and-
        // suspenders safety net for any other panics deeper in
        // the decode path.
        std::panic::catch_unwind(|| codec::decompress(encoded))
            .map_err(|_| "v4 decompress panicked (corrupted stream?)".to_string())?
    }
}

// ---------------------------------------------------------------------------
// v5 LZMA engine (xz2-based)
// ---------------------------------------------------------------------------

/// LZMA2 backend (via the `xz2` crate). Wraps a single LZMA2 stream
/// with a 1-byte engine tag prefix so the dispatch layer can
/// recognize it. The `level` parameter is the LZMA preset:
/// - 0..=3: fast (matches `xz -0`..`xz -3`)
/// - 4..=6: balanced
/// - 7..=9: max ratio (matches `xz -7`..`xz -9`)
///
/// The `LZMABlock` defaults to `xz -9` (max ratio) which is the
/// apples-to-apples comparison for 7z in the bench suite.
pub struct LzmaEngine {
    level: u32,
}

impl LzmaEngine {
    pub fn new(level: u32) -> Self {
        Self {
            level: level.clamp(0, 9),
        }
    }
}

impl CompressionEngine for LzmaEngine {
    fn name(&self) -> &'static str {
        "v5-lzma"
    }
    fn bench_label(&self) -> &'static str {
        match self.level {
            0..=3 => "NexusCompress v5 LZMA (fast)",
            4..=6 => "NexusCompress v5 LZMA (balanced)",
            _ => "NexusCompress v5 LZMA (extreme)",
        }
    }
    fn format_byte(&self) -> u8 {
        V5_LZMA_FORMAT_BYTE
    }

    fn compress(&self, input: &[u8]) -> Vec<u8> {
        // xz2's stream API expects Read/Write streams. For whole-
        // input compression, the simplest is the `compress` helper
        // which writes to a Vec<u8>. The level maps to the LZMA
        // preset (0=easy, 9=best).
        let mut out = Vec::with_capacity(input.len() / 2);
        out.push(self.format_byte()); // tag
                                      // xz2's stream::write takes a compression level. The API
                                      // also takes an `xz::Write` writer — we use Vec<u8>.
        let mut writer = xz2::write::XzEncoder::new(&mut out, self.level);
        use std::io::Write;
        if writer.write_all(input).is_err() {
            // Should be impossible (Vec write never fails), but
            // the trait expects Vec, not Result. Surface the error
            // by returning an empty stream — the bench will catch
            // it via roundtrip mismatch.
            return vec![self.format_byte()];
        }
        if writer.finish().is_err() {
            return vec![self.format_byte()];
        }
        out
    }

    fn decompress(&self, encoded: &[u8]) -> Result<Vec<u8>, String> {
        if encoded.is_empty() || encoded[0] != self.format_byte() {
            return Err("v5 LZMA: bad format byte".into());
        }
        let payload = &encoded[1..];
        let mut decoder = xz2::read::XzDecoder::new(payload);
        let mut out = Vec::with_capacity(payload.len() * 2);
        use std::io::Read;
        decoder
            .read_to_end(&mut out)
            .map_err(|e| format!("v5 LZMA decompress failed: {}", e))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(engine: &dyn CompressionEngine, data: &[u8]) -> Vec<u8> {
        let compressed = engine.compress(data);
        let recovered = engine.decompress(&compressed).expect("decompress");
        assert_eq!(recovered, data, "roundtrip mismatch");
        recovered
    }

    #[test]
    fn v4_engine_roundtrip() {
        let data = b"the quick brown fox jumps over the lazy dog. \
                     repetitive text repeated text repeated text";
        roundtrip(&V4Engine, data);
    }

    #[test]
    fn lzma_engine_roundtrip_small() {
        let data = b"hello nexuscompress lzma test";
        roundtrip(&LzmaEngine::new(6), data);
    }

    #[test]
    fn lzma_engine_roundtrip_large_repetitive() {
        let mut data = Vec::with_capacity(100_000);
        for _ in 0..1000 {
            data.extend_from_slice(b"the quick brown fox jumps over the lazy dog\n");
        }
        roundtrip(&LzmaEngine::new(9), &data);
    }

    #[test]
    fn lzma_levels_roundtrip() {
        for level in 0..=9 {
            roundtrip(&LzmaEngine::new(level), b"some data to compress");
        }
    }

    #[test]
    fn dispatch_dispatches_to_right_engine() {
        let v4_data = b"v4 test data";
        let v4_compressed = V4Engine.compress(v4_data);
        let recovered = decompress_any(&v4_compressed).expect("dispatch v4");
        assert_eq!(recovered, v4_data);

        let v5_data = b"v5 test data";
        let v5_compressed = LzmaEngine::new(6).compress(v5_data);
        let recovered = decompress_any(&v5_compressed).expect("dispatch v5");
        assert_eq!(recovered, v5_data);
    }

    #[test]
    fn dispatch_rejects_unknown_format() {
        let bogus = vec![0x99, 0x00, 0x00];
        let r = decompress_any(&bogus);
        assert!(r.is_err());
    }

    #[test]
    fn compress_with_dispatches() {
        let data = b"compress_with test";
        let v4 = compress_with("v4", data, false).expect("v4");
        let recovered = decompress_any(&v4).expect("decompress v4");
        assert_eq!(recovered, data);

        let v5 = compress_with("v5", data, false).expect("v5");
        let recovered = decompress_any(&v5).expect("decompress v5");
        assert_eq!(recovered, data);
    }

    #[test]
    fn lzma_beats_v4_on_repetitive_data() {
        // The v5 LZMA backend should be better (smaller) than v4 on
        // highly-repetitive data. This is a coarse smoke test, not
        // a benchmark — the bench_suite gives the precise numbers.
        let mut data = Vec::with_capacity(200_000);
        for _ in 0..2000 {
            data.extend_from_slice(b"abcde");
        }
        let v4_size = V4Engine.compress(&data).len();
        let v5_size = LzmaEngine::new(9).compress(&data).len();
        assert!(
            v5_size < v4_size,
            "v5 LZMA ({} B) should beat v4 ({} B) on repetitive data",
            v5_size,
            v4_size
        );
    }
}
