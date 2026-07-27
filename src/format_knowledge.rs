//! Sprint 5.7.10-A: single source of truth for "already-compressed /
//! high-entropy" file extensions.
//!
//! ## Why this module exists
//!
//! Up to sprint 5.7.9 the codebase maintained **four separate copies**
//! of the same ~80-extension list:
//!
//! 1. `solid_archive::is_incompressible_ext` (extension pre-check)
//! 2. `solid_archive::pick_preprocessor_with_overrides` (default Raw rule)
//! 3. `nxar::PASSTHROUGH_EXTS` (per-file archive passthrough)
//! 4. `api::compress_directory_with_backend_and_codec_overrides_with_progress`
//!    (V6Solid passthrough filter)
//!
//! They drifted. Sprint 5.7.9 part 4 added `.pyc` / `.db` / `.jar` /
//! `.nxs6` to `nxar.rs` and then had to add the same entries to the
//! other lists in follow-up hotfixes. The `.ts` entry has been
//! oscillating between "include for MPEG-TS passthrough" and "exclude
//! to keep TypeScript AST minify" since 5.7.2 hotfix #38.
//!
//! **This module is the only place that knows which extensions should
//! bypass preprocessing.** Every other call site imports from here.
//!
//! ## What it gives you
//!
//! - [`RAW_FORMATS`]: the master sorted slice of "always Raw"
//!   extensions. `binary_search` is O(log n) so the runtime cost
//!   is negligible even on a 200k-file directory.
//! - [`is_raw_format`]: the single function the encoder calls. It
//!   uses `Path::extension()` (the last `.xxx` segment) so it
//!   doesn't false-positive on files like `foopng` or `foo.png.txt`,
//!   unlike the old `ends_with` checks in `nxar.rs` and `api.rs`.
//!
//! ## What it deliberately does NOT do
//!
//! - **`.ts` is excluded.** The extension is ambiguous between
//!   TypeScript source (which we AST-minify via `pick_preprocessor`'s
//!   SwcAst path) and MPEG Transport Stream (which we want to
//!   passthrough). The runtime Shannon entropy check
//!   (`shannon_entropy_incompressible` in `solid_archive.rs`) catches
//!   MPEG-TS at the data level, so TypeScript files keep their AST
//!   minify path even though MPEG-TS would have liked to be in
//!   this list.
//! - **It does not include "user-pinned" extensions.** Those flow
//!   through `PreprocessorOverrides::raw_extensions` (per-extension
//!   pin in the GUI's Advanced panel) and are checked before the
//!   default rule.
//! - **It does not include "raw" rules based on file content.**
//!   Binary detection is the runtime entropy check's job.

use std::path::Path;

/// All file extensions that should bypass any preprocessing (text
/// minify, swc AST minify) and go straight to the codec verbatim.
///
/// Sorted ascending. The sort is asserted at test time so the
/// `binary_search` inside [`is_raw_format`] stays valid.
///
/// Categories covered (see the commit log for additions):
/// - Native binaries / pre-built libraries (.dylib, .so, .o, .a, ...)
/// - Compiled bytecode (.class, .pyc, .pyo, .bc)
/// - Raster images (.png, .jpg, .heic, .avif, .jxl, ...)
/// - RAW camera formats (.dng, .cr2, .nef, .arw, ...)
/// - Vector design files (.ai, .eps, .psd, .xd, .sketch, ...)
/// - 3D models (.stl, .blend, .gltf, .glb, ...)
/// - Video containers (.mp4, .mov, .webm, .mkv, ...)
/// - Audio (.mp3, .flac, .opus, .m4a, ...)
/// - Fonts (.woff, .woff2, .ttf, .otf, ...)
/// - Archives (.zip, .7z, .tar, .gz, .xz, .zst, ...)
/// - Disk images / VMs (.iso, .vmdk, .vhd, .qcow2, ...)
/// - Our own formats (.nxs, .nxs6, .nxar, ...)
/// - Office / PDF / iWork / e-books (.pdf, .docx, .xlsx, .epub, ...)
/// - Databases / scientific data (.sqlite, .db, .sst, .hdf5, ...)
/// - Encrypted / certificate formats (.keystore, .p12, .pem, ...)
///
/// The list is intentionally a SUPERSET of every historical
/// passthrough list in the codebase. The previous four lists were
/// each a partial subset that drifted over time. Today any
/// extension that was ever a passthrough anywhere stays a
/// passthrough everywhere.
pub const RAW_FORMATS: &[&str] = &[
    "3ds", "3g2", "3gp", "7z",
    "a", "aab", "aac", "accdb", "ai", "aif", "aiff", "alac",
    "apk", "arw", "avi", "avif", "azw", "azw3",
    "bc", "blend", "bmp", "br", "bz2",
    "cab", "cer", "class", "cr2", "cr3", "crt",
    "db", "deb", "dll", "dmg", "dng", "doc", "docm", "docx",
    "dotm", "dotx", "dylib",
    "ear", "eot", "eps", "epub", "exe",
    "fb2", "fbx", "fig", "fits", "flac", "flv",
    "gif", "glb", "gltf", "gz",
    "hdf5", "heic", "heif",
    "icns", "ico", "idx", "img", "ipa", "iso",
    "jar", "jks", "jpeg", "jpg", "jxl",
    "key", "keystore",
    "ldb", "lib", "lz", "lz4",
    "m2ts", "m4a", "m4v", "mdb", "mka", "mkv", "mobi",
    "mov", "mp3", "mp4", "msi", "msp", "mts",
    "nc", "nc4", "nef", "node", "numbers",
    "nxar", "nxe", "nxr", "nxs", "nxs6",
    "o", "obj", "odb", "odf", "odg", "odm", "odp", "ods", "odt",
    "ogg", "ogv", "opus", "orf", "otf", "ova", "ovf",
    "p12", "pages", "pdb", "pdf", "pem", "pfx", "pkg", "png",
    "potm", "potx", "ppsm", "ppsx", "ppt", "pptm", "pptx",
    "psb", "psd", "pyc", "pyo",
    "qcow2",
    "raf", "rar", "rocksdb", "rpm", "rw2",
    "sketch", "so", "sqlite", "sqlite3", "srt", "sst", "stl",
    "sub", "svgz",
    "tar", "tgz", "tif", "tiff", "ttf",
    "usdz",
    "vdi", "vhd", "vmdk", "vob", "vtt",
    "war", "wav", "webm", "webp", "wma", "wmv",
    "woff", "woff2",
    "xd", "xls", "xlsb", "xlsm", "xlsx", "xltm", "xltx",
    "xz", "zip", "zst",
];

/// Returns `true` if the file's extension (case-insensitive, last
/// `.` segment of the filename) is in [`RAW_FORMATS`].
///
/// `name` can be a full path or just a filename — `Path::extension`
/// only looks at the last component so the result is the same.
///
/// # Examples
///
/// ```
/// use nexus_compress::format_knowledge::is_raw_format;
///
/// assert!(is_raw_format("foo.png"));
/// assert!(is_raw_format("FOO.PNG"));   // case-insensitive
/// assert!(is_raw_format("/abs/path/to/foo.tar.gz")); // returns true (gz is in list)
/// assert!(!is_raw_format("foo.rs"));   // not on the list
/// assert!(!is_raw_format("foo"));      // no extension
/// assert!(!is_raw_format("foopng"));   // no dot, no extension
/// ```
pub fn is_raw_format(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .map(|ext| {
            // O(log n) lookup. RAW_FORMATS is sorted — that's
            // asserted in the tests below so a future contributor
            // who appends an out-of-order entry gets a loud failure.
            RAW_FORMATS
                .binary_search(&ext.as_str())
                .is_ok()
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_formats_is_sorted() {
        // binary_search requires a sorted slice. The runtime check
        // in is_raw_format would silently misbehave on an unsorted
        // slice (return false for entries that ARE in the list).
        // This test makes the failure loud.
        let mut sorted = RAW_FORMATS.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            RAW_FORMATS.to_vec(),
            "RAW_FORMATS must be sorted ascending (binary_search invariant)"
        );
    }

    #[test]
    fn raw_formats_has_no_duplicates() {
        let mut sorted = RAW_FORMATS.to_vec();
        sorted.sort();
        let len_before = sorted.len();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            len_before,
            "RAW_FORMATS has duplicate entries"
        );
    }

    #[test]
    fn is_raw_format_case_insensitive() {
        assert!(is_raw_format("foo.PNG"));
        assert!(is_raw_format("foo.Png"));
        assert!(is_raw_format("foo.png"));
        assert!(is_raw_format("FOO.ZIP"));
    }

    #[test]
    fn is_raw_format_path_or_filename() {
        assert!(is_raw_format("/abs/path/to/foo.png"));
        assert!(is_raw_format("relative/foo.png"));
        assert!(is_raw_format("foo.png"));
        // Compound extensions: only the last segment matters.
        // `foo.tar.gz` has extension "gz" (in the list).
        assert!(is_raw_format("foo.tar.gz"));
    }

    #[test]
    fn is_raw_format_negative_cases() {
        // Code/script source files are NOT raw — they get the
        // smart preprocessor (Conservative or SwcAst).
        assert!(!is_raw_format("foo.rs"));
        assert!(!is_raw_format("foo.js"));
        assert!(!is_raw_format("foo.ts")); // see module docstring
        assert!(!is_raw_format("foo.py"));
        assert!(!is_raw_format("foo.go"));
        // No extension
        assert!(!is_raw_format("foo"));
        assert!(!is_raw_format(""));
        // Config files
        assert!(!is_raw_format("foo.json"));
        assert!(!is_raw_format("foo.toml"));
        assert!(!is_raw_format("foo.yaml"));
    }

    #[test]
    fn is_raw_format_marks_documented_formats() {
        // Spot-check the categories that motivated the
        // single-source refactor in the first place.
        // Sprint 5.7.9 part 4 added these as passthrough
        // for nxar.rs; they should be Raw everywhere now.
        assert!(is_raw_format("module.pyc"), "pyc (Python bytecode)");
        assert!(is_raw_format("data.db"), "db (SQLite database)");
        assert!(is_raw_format("lib.jar"), "jar (Java archive)");
        assert!(is_raw_format("archive.nxs6"), "nxs6 (our own format)");
        // Previously only in nxar.rs and api.rs (drift bug).
        assert!(is_raw_format("app.apk"), "apk (Android package)");
        assert!(is_raw_format("bundle.aab"), "aab (Android App Bundle)");
        assert!(is_raw_format("disk.vhd"), "vhd (virtual hard disk)");
        assert!(is_raw_format("compiled.bc"), "bc (LLVM bitcode)");
    }

    #[test]
    fn is_raw_format_does_not_false_positive() {
        // The old `ends_with(ext)` check in nxar.rs/api.rs would
        // match these because the basename ends with the
        // substring. Path::extension() correctly rejects them.
        assert!(!is_raw_format("foopng")); // no dot, no extension
        assert!(!is_raw_format("foo.somethingpng")); // ext is "somethingpng"
        assert!(!is_raw_format("foo.png.txt")); // ext is "txt"
        assert!(!is_raw_format("something.zip.bak")); // ext is "bak"
    }
}
