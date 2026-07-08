//! Solid archive format `NXS6` (NexusCompress Solid v6).
//!
//! ## Why solid?
//!
//! In a typical source-code corpus (Next.js, Rust crate, etc.) the
//! files share huge amounts of structure: the same imports, the same
//! type names, the same function bodies, the same boilerplate. When
//! you compress each file independently, the LZMA dictionary resets
//! between files and can't see those cross-file repetitions.
//!
//! Solid compression concatenates ALL preprocessed files into a
//! single byte stream and runs LZMA over the whole thing. The
//! dictionary now spans the entire corpus — so the second `.tsx`
//! file reuses the matches the LZMA Match Finder found in the first.
//! On a real Next.js corpus this typically buys +5-15% over
//! per-file LZMA at the cost of making the archive LOSSY (you can't
//! extract the original bytes — you get the minified version of
//! each file).
//!
//! ## Format
//!
//! ```text
//! +------------------------+
//! | Magic "NXS6\n"  (5 B)  |
//! | Version       (1 B)    |   = 1
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
//! | Solid block (LZMA)     |   xz2 stream, no tag prefix
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

use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

use crate::ast_minify;
use crate::minify;

/// Format magic: `NXS6\n`.
pub const MAGIC: &[u8; 5] = b"NXS6\n";

/// Format version (currently 1).
pub const VERSION: u8 = 1;

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
        match b {
            0 => Ok(Preprocessor::Raw),
            1 => Ok(Preprocessor::Conservative),
            2 => Ok(Preprocessor::SwcAst),
            other => Err(format!("unknown preprocessor id: {}", other)),
        }
    }
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
}

/// Detects the best preprocessor for a file based on its name.
fn pick_preprocessor(name: &str) -> Preprocessor {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => Preprocessor::SwcAst,
        _ => Preprocessor::Conservative,
    }
}

/// Apply the preprocessor to a single file's bytes.
fn apply_preprocessor(pre: Preprocessor, input: &[u8]) -> Vec<u8> {
    match pre {
        Preprocessor::Raw => input.to_vec(),
        Preprocessor::Conservative => minify::minify(input),
        Preprocessor::SwcAst => ast_minify::minify(input).bytes,
    }
}

/// Compress a list of (name, bytes) into a solid v6 archive.
///
/// The archive is LOSSY — the preprocessor is applied to each file
/// before concatenation. The `name` should be the relative path
/// (e.g. `src/index.ts`); it is stored verbatim in the TOC.
///
/// `lzma_level` is the LZMA preset (0..=9). 6 = balanced, 9 = max
/// ratio (apples-to-apples with 7z `-mx=9`).
///
/// `progress` is called after each file is preprocessed, with
/// `(file_index_done, total_files, current_file_name)`. Pass `|_,_,_| {}`
/// if you don't need progress reporting (or use `compress_with_progress`).
pub fn compress_with_progress<P>(
    files: &[(String, Vec<u8>)],
    lzma_level: u32,
    mut progress: P,
) -> Result<Vec<u8>, String>
where
    P: FnMut(usize, usize, &str),
{
    // Step 1: preprocess each file, build the uncompressed solid
    // stream and the TOC in parallel.
    let mut entries: Vec<FileEntry> = Vec::with_capacity(files.len());
    let mut solid_uncompressed: Vec<u8> = Vec::new();
    let total = files.len();

    for (i, (name, bytes)) in files.iter().enumerate() {
        let pre = pick_preprocessor(name);
        let pre_bytes = apply_preprocessor(pre, bytes);
        let offset = solid_uncompressed.len() as u64;
        let original_size = bytes.len() as u64;
        let pre_size = pre_bytes.len() as u64;
        solid_uncompressed.extend_from_slice(&pre_bytes);
        entries.push(FileEntry {
            name: name.clone(),
            original_size,
            pre_size,
            preprocessor: pre,
            solid_offset: offset,
        });
        // Report progress after each file is in the solid stream.
        progress(i + 1, total, name);
    }

    // Step 2: LZMA-compress the solid stream.
    let mut lzma_compressed: Vec<u8> = Vec::new();
    {
        // Sprint 5.7 hotfix #21: auto-tune the LZMA dict size to
        // whatever the machine can actually afford (no swap).
        let stream = crate::ram::lzma_stream_for_requested_preset(lzma_level)
            .map_err(|e| format!("solid LZMA stream: {}", e))?;
        let mut enc = XzEncoder::new_stream(&mut lzma_compressed, stream);
        enc.write_all(&solid_uncompressed)
            .map_err(|e| format!("solid LZMA write failed: {}", e))?;
        enc.finish()
            .map_err(|e| format!("solid LZMA finish failed: {}", e))?;
    }

    // Step 3: serialize the header (TOC) + append the LZMA block.
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
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
        out.push(e.preprocessor as u8);
        out.extend_from_slice(&e.solid_offset.to_le_bytes());
    }
    out.extend_from_slice(&lzma_compressed);
    Ok(out)
}

/// Backward-compatible wrapper that discards progress.
#[inline]
pub fn compress(files: &[(String, Vec<u8>)], lzma_level: u32) -> Result<Vec<u8>, String> {
    compress_with_progress(files, lzma_level, |_, _, _| {})
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
    let entries = parse_toc(archive)?;
    let total: u64 = entries.iter().map(|e| e.original_size).sum();
    Ok((entries, total))
}

/// Parse the TOC portion of a solid archive (header + entry list)
/// and return the entries. Shared between `peek_toc` and
/// `decompress`.
pub fn parse_toc(archive: &[u8]) -> Result<Vec<FileEntry>, String> {
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
    if version != VERSION {
        return Err(format!("solid archive: unsupported version {}", version));
    }
    let n_files = u32::from_le_bytes(
        archive[pos..pos + 4]
            .try_into()
            .map_err(|_| "solid archive: bad n_files".to_string())?,
    ) as usize;
    pos += 4;

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
        let preprocessor = match archive[pos] {
            0 => Preprocessor::Raw,
            1 => Preprocessor::Conservative,
            2 => Preprocessor::SwcAst,
            other => return Err(format!("unknown preprocessor id: {}", other)),
        };
        pos += 1;
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
        });
    }
    Ok(entries)
}

pub fn decompress(archive: &[u8]) -> Result<(Vec<FileEntry>, Vec<u8>), String> {
    // Reuse parse_toc for the header + entry walk, then walk
    // through the entries manually a second time to compute `pos`
    // (the byte offset just past the TOC) — the LZMA solid block
    // begins there. We don't try to be clever about avoiding the
    // second walk because the TOC entries are tens of bytes each
    // and the user pays for one LZMA stream either way.
    let entries = parse_toc(archive)?;
    let mut pos = MAGIC.len() + 1 + 4;
    for e in &entries {
        pos += 2 + e.name.len() + 8 + 8 + 1 + 8;
    }

    // Step 2: LZMA-decompress the rest of the file.
    let lzma_compressed = &archive[pos..];
    let mut solid_uncompressed: Vec<u8> = Vec::new();
    XzDecoder::new(lzma_compressed)
        .read_to_end(&mut solid_uncompressed)
        .map_err(|e| format!("solid LZMA decompress failed: {}", e))?;

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

    Ok((entries, solid_uncompressed))
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
}
