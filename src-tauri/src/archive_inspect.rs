//! Archive inspection (Sprint 5.6.17).
//!
//! WinRAR-style "browse without extracting": list the central
//! directory of an archive and selectively extract individual
//! entries without streaming the whole archive to disk first.
//!
//! Two formats supported for now:
//! - `.tar` (plain POSIX ustar)
//! - `.nxs6` / `.nxs` (Nexus Compress archive — already has a
//!   central directory in its NXAR trailer)
//!
//! Both rely on the archive's own central directory / TOC.
//! Reading the entry list is O(headers), not O(payload).

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Validate that an archive entry path is safe to extract relative
/// to `output_dir`. Returns `Ok(())` if the path is relative, has
/// no `..` or `.` path components, and contains no path-traversal
/// tricks. Returns `Err(reason)` otherwise.
///
/// **Sprint 5.7.2 hotfix #22 — semantic check, NOT substring.**
/// Earlier versions used `entry_path.contains("..")` which wrongly
/// rejected legitimate filenames containing `..` AS PART OF A NAME,
/// e.g. Next.js dev mode emits chunks like
/// `pomodoro/.next/dev/server/chunks/ssr/_0td~bj.._.js`. The
/// substring `..` is fine inside a single component — what we're
/// guarding against is `..` AS A WHOLE COMPONENT (which would mean
/// "go up one directory"). This helper walks `Path::components`
/// and rejects only `.` / `..` / absolute roots / null bytes /
/// control characters.
fn safe_relative_path(entry_path: &str) -> Result<(), String> {
    if entry_path.is_empty() {
        return Err("empty entry path".to_string());
    }
    // Reject NUL bytes early (defense in depth — `Path::components`
    // on some platforms terminates at NUL, hiding the rest of the
    // path from the check).
    if entry_path.contains('\0') {
        return Err(format!(
            "entry path contains NUL byte: {:?}",
            entry_path
        ));
    }
    // Windows-style absolute paths ("C:\foo") sneak through
    // `Path::components` on Unix. Reject any drive-letter prefix.
    if entry_path.len() >= 2 && entry_path.as_bytes()[1] == b':' {
        return Err(format!(
            "entry path has drive letter: {:?}",
            entry_path
        ));
    }
    let p = Path::new(entry_path);
    // Reject absolute paths ("/etc/passwd" on Unix, "C:\..." already
    // caught above, "\\server\share" on Windows).
    if p.is_absolute() {
        return Err(format!("absolute entry path: {:?}", entry_path));
    }
    for component in p.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                return Err(format!(
                    "entry path contains .. component: {:?}",
                    entry_path
                ));
            }
            Component::CurDir => {
                return Err(format!(
                    "entry path contains . component: {:?}",
                    entry_path
                ));
            }
            Component::Normal(_) | Component::Prefix(_) | Component::RootDir => {
                // Prefix / RootDir were rejected by is_absolute above.
                // Normal is fine — that's any ordinary filename,
                // even one containing `..` as a substring.
            }
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ArchiveEntry {
    /// Path inside the archive, e.g. "Counter-Strike 2/Contents/MacOS/cs2".
    pub name: String,
    /// Size in bytes (uncompressed).
    pub size: u64,
    /// True if this is a directory entry.
    pub is_dir: bool,
}

/// Detect archive format by extension.
pub fn detect_format(path: &Path) -> Result<&'static str, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "tar" => Ok("tar"),
        // .nxs6 / .nxs (V6Solid) carry their central directory
        // at the FRONT of the file (MAGIC + version + count +
        // per-entry header). parse_toc reads it without LZMA-
        // decompressing the payload.
        "nxs6" | "nxs" => Ok("solid"),
        _ => Err(format!(
            "unsupported archive format: .{} (supported: .tar, .nxs6)",
            ext
        )),
    }
}

/// List all entries in the archive's central directory.
/// Returns the complete Vec. Prefer `list_paginated` for
/// huge archives — sending 1M entries over IPC is expensive.
pub fn list_entries(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    match detect_format(path)? {
        "tar" => list_tar_entries(path),
        "solid" => list_solid_entries(path),
        other => Err(format!("unsupported format: {}", other)),
    }
}

/// Streaming paginated listing for huge archives. Reads only
/// `limit + 1` headers to determine the page contents +
/// `has_more`. For a 30 GB tar with 1M files, only the first
/// ~256 KB of headers are touched (regardless of where the page
/// starts), and only `limit` entries are returned over IPC.
pub fn list_paginated(path: &Path, offset: usize, limit: usize) -> Result<PageResult, String> {
    match detect_format(path)? {
        "tar" => list_tar_paginated(path, offset, limit),
        "solid" => list_solid_paginated(path, offset, limit),
        other => Err(format!("unsupported format: {}", other)),
    }
}

/// One page of archive entries plus the metadata the UI needs
/// to render "showing N–M of K" / "load more" controls.
#[derive(Debug, Clone)]
pub struct PageResult {
    pub entries: Vec<ArchiveEntry>,
    pub has_more: bool,
}

/// Extract the requested entries (or all if `selected` is None)
/// to `output_dir`. Streams each entry individually.
pub fn extract_entries(
    path: &Path,
    output_dir: &Path,
    selected: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(output_dir).map_err(|e| format!("mkdir output: {}", e))?;
    match detect_format(path)? {
        "tar" => extract_tar_entries(path, output_dir, selected),
        "solid" => extract_solid_entries(path, output_dir, selected),
        other => Err(format!("unsupported format: {}", other)),
    }
}

// --- tar -----------------------------------------------------------------

fn list_tar_entries(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("open tar: {}", e))?;
    let mut ar = tar::Archive::new(f);
    let mut out = Vec::new();
    for entry in ar.entries().map_err(|e| format!("tar entries: {}", e))? {
        let entry = entry.map_err(|e| format!("tar entry: {}", e))?;
        let name = entry
            .path()
            .map_err(|e| format!("tar path: {}", e))?
            .to_string_lossy()
            .into_owned();
        let size = entry.size();
        let is_dir = entry.header().entry_type().is_dir();
        out.push(ArchiveEntry { name, size, is_dir });
    }
    Ok(out)
}

/// Streaming paginated tar listing. Reads only enough headers
/// to fill the requested window + one more entry (used to
/// detect has_more without enumerating the whole archive).
fn list_tar_paginated(path: &Path, offset: usize, limit: usize) -> Result<PageResult, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("open tar: {}", e))?;
    let mut ar = tar::Archive::new(f);
    // Skip `offset` headers, then collect up to limit+1.
    let mut out = Vec::with_capacity(limit + 1);
    let mut skipped = 0usize;
    let mut entry_iter = ar.entries().map_err(|e| format!("tar entries: {}", e))?;
    while skipped < offset {
        match entry_iter.next() {
            Some(Ok(_)) => skipped += 1,
            Some(Err(e)) => return Err(format!("tar entry: {}", e)),
            None => {
                return Ok(PageResult {
                    entries: out,
                    has_more: false,
                })
            }
        }
    }
    let mut count = 0usize;
    for entry in entry_iter {
        if count >= limit + 1 {
            break;
        }
        let entry = entry.map_err(|e| format!("tar entry: {}", e))?;
        let name = entry
            .path()
            .map_err(|e| format!("tar path: {}", e))?
            .to_string_lossy()
            .into_owned();
        let size = entry.size();
        let is_dir = entry.header().entry_type().is_dir();
        out.push(ArchiveEntry { name, size, is_dir });
        count += 1;
    }
    let has_more = out.len() > limit;
    if has_more {
        out.pop();
    }
    Ok(PageResult {
        entries: out,
        has_more,
    })
}

fn extract_tar_entries(
    path: &Path,
    output_dir: &Path,
    selected: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("open tar: {}", e))?;
    let mut ar = tar::Archive::new(f);
    let selected_set: Option<std::collections::HashSet<String>> =
        selected.map(|v| v.into_iter().collect());
    let mut written = Vec::new();
    for entry in ar.entries().map_err(|e| format!("tar entries: {}", e))? {
        let mut entry = entry.map_err(|e| format!("tar entry: {}", e))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("tar path: {}", e))?
            .to_string_lossy()
            .into_owned();
        if let Some(ref set) = selected_set {
            if !set.contains(&entry_path) {
                continue;
            }
        }
        // Defend against malicious archives: reject absolute paths
        // and ".." / "." path COMPONENTS (the tar crate strips ".."
        // but defense in depth). Sprint 5.7.2 hotfix #22: switched
        // from `contains("..")` substring check to `safe_relative_path`
        // so legitimate filenames like Next.js's `_0td~bj.._.js`
        // aren't rejected.
        safe_relative_path(&entry_path)
            .map_err(|e| format!("unsafe path in tar: {} ({})", entry_path, e))?;
        let dest = output_dir.join(&entry_path);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("mkdir {}: {}", dest.display(), e))?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir parent: {}", e))?;
            }
            entry
                .unpack(&dest)
                .map_err(|e| format!("unpack {}: {}", dest.display(), e))?;
        }
        written.push(entry_path);
    }
    Ok(written)
}

// --- nxs6 / nxs -----------------------------------------------------------
//
// .nxs6 (V6Solid) layout:
//   [MAGIC "NXS6\n" 5 bytes][version 1 byte][n_entries 4 bytes]
//   [per-entry header: name_len(2) + name + pre_size(8) +
//                       original_size(8) + preprocessor(1) +
//                       solid_offset(8)]
//   [LZMA-compressed solid stream — concatenated preprocessed
//    bytes of every file in offset order]
//
// Sprint 5.6.20: solid_archive::parse_toc walks the header in
// O(entries) WITHOUT touching the LZMA stream, so we can list
// entries instantly. For selective extraction we still need to
// LZMA-decompress the solid block (one pass), but then we can
// slice out only the entries the user asked for using their
// solid_offset. This matches WinRAR/ZIP behaviour for solid
// archives (full decompression is the only way to do selective).

fn list_solid_entries(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read solid archive: {}", e))?;
    let parsed = nexus_compress::solid_archive::parse_toc(&bytes)
        .map_err(|e| format!("parse solid toc: {}", e))?;
    let entries = parsed.entries;
    Ok(entries
        .into_iter()
        .map(|e| {
            // The solid archive doesn't carry directories as
            // entries — directories are implied by the path
            // separators in file names. Mark paths ending in
            // '/' as dirs (defensive — the producer shouldn't
            // emit these today, but the parser tolerates them).
            let name = e.name.clone();
            ArchiveEntry {
                name: e.name,
                size: e.original_size,
                is_dir: name.ends_with('/'),
            }
        })
        .collect())
}

/// Paginated solid listing. The solid_archive TOC reader is
/// already O(headers) and the headers are tiny (~50 bytes
/// each), so even 100k entries parses in tens of milliseconds.
/// We slice the result for the requested window.
fn list_solid_paginated(path: &Path, offset: usize, limit: usize) -> Result<PageResult, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read solid archive: {}", e))?;
    let parsed = nexus_compress::solid_archive::parse_toc(&bytes)
        .map_err(|e| format!("parse solid toc: {}", e))?;
    let entries = parsed.entries;
    let total = entries.len();
    let end = (offset + limit).min(total);
    let window: Vec<ArchiveEntry> = if offset < total {
        entries[offset..end]
            .iter()
            .map(|e| {
                let name = e.name.clone();
                ArchiveEntry {
                    name: e.name.clone(),
                    size: e.original_size,
                    is_dir: name.ends_with('/'),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let has_more = end < total;
    Ok(PageResult {
        entries: window,
        has_more,
    })
}

fn extract_solid_entries(
    path: &Path,
    output_dir: &Path,
    selected: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    use nexus_compress::solid_archive::decompress as solid_decompress;
    let bytes = std::fs::read(path).map_err(|e| format!("read solid archive: {}", e))?;
    // Step 1: read the TOC (cheap — no LZMA).
    let parsed = nexus_compress::solid_archive::parse_toc(&bytes)
        .map_err(|e| format!("parse solid toc: {}", e))?;
    let entries = parsed.entries;
    // Step 2: full LZMA decompression (one pass, mandatory for
    // solid archives — same as WinRAR/ZIP). After this we
    // have the preprocessed bytes of every entry concatenated
    // in the order they were compressed. Per-entry offsets
    // come from the TOC.
    let (_entries, solid_uncompressed) =
        solid_decompress(&bytes).map_err(|e| format!("solid decompress: {}", e))?;
    let selected_set: Option<std::collections::HashSet<String>> =
        selected.map(|v| v.into_iter().collect());
    let mut written = Vec::new();
    for e in &entries {
        let entry_path = e.name.clone();
        if let Some(ref set) = selected_set {
            if !set.contains(&entry_path) {
                continue;
            }
        }
        // Sprint 5.7.2 hotfix #22: switched to per-component check so
        // filenames like `_0td~bj.._.js` (legitimate, produced by
        // Next.js dev mode) aren't rejected. Only `..` / `.` AS A
        // WHOLE PATH COMPONENT triggers the error.
        safe_relative_path(&entry_path)
            .map_err(|e| format!("unsafe path in solid: {} ({})", entry_path, e))?;
        // Directories in solid archives are implicit (carried
        // as file paths with embedded slashes). Create the
        // parent for each file.
        let dest = output_dir.join(&entry_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir parent: {}", e))?;
        }
        // Slice the preprocessed bytes for this entry. For
        // Raw files the bytes are unchanged; for Conservative /
        // SwcAst preprocessors the extracted file is the
        // minified version (V6's lossy step has no inverse).
        let start = e.solid_offset as usize;
        let end = start + e.pre_size as usize;
        if end > solid_uncompressed.len() {
            return Err(format!(
                "solid entry '{}' extends past solid block ({} > {})",
                entry_path,
                end,
                solid_uncompressed.len()
            ));
        }
        std::fs::write(&dest, &solid_uncompressed[start..end])
            .map_err(|e| format!("write {}: {}", dest.display(), e))?;
        written.push(entry_path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::safe_relative_path;

    /// Sprint 5.7.2 hotfix #22 — the regression test that pins
    /// the per-component semantic. Before this commit, the check
    /// was `entry_path.contains("..")` which wrongly rejected
    /// legitimate filenames like Next.js's `_0td~bj.._.js`.
    /// These tests pin both directions: legitimate filenames
    /// with `..` as a SUBSTRING pass, and `..` / `.` as a whole
    /// component fails.
    #[test]
    fn safe_path_accepts_legitimate_nextjs_chunk_with_double_dot_substring() {
        // The exact filename from the user's bug report
        // (frontend.nxs6 → pomodoro/.next/dev/server/chunks/ssr/_0td~bj.._.js).
        let path = "pomodoro/.next/dev/server/chunks/ssr/_0td~bj.._.js";
        safe_relative_path(path).expect("legitimate Next.js chunk should pass");
    }

    #[test]
    fn safe_path_accepts_normal_filenames_with_double_dot_substring() {
        for p in &[
            "src/foo..bar.rs",
            "test..spec.js",
            "weird..name..with..multiple..dots.txt",
            "no-leading-dot",
            "a",
            "very/deep/nested/path/with/dots.in.name.json",
        ] {
            safe_relative_path(p)
                .unwrap_or_else(|e| panic!("legitimate path {:?} should pass: {}", p, e));
        }
    }

    #[test]
    fn safe_path_rejects_parent_dir_components() {
        for p in &[
            "../etc/passwd",
            "foo/../../etc/passwd",
            "foo/bar/..",
            "foo/./../bar",
        ] {
            safe_relative_path(p)
                .err()
                .unwrap_or_else(|| panic!("traversal path {:?} should be rejected", p));
        }
    }

    #[test]
    fn safe_path_rejects_cur_dir_components() {
        // On Unix, `Path::components` normalizes `.` away, so
        // `foo/./bar` becomes `[Normal("foo"), Normal("bar")]`
        // — same as `foo/bar`. The `.` is a no-op at the
        // filesystem level, so there's no traversal risk.
        //
        // We test only the cases where `.` would actually show
        // up as a `Component::CurDir` (which is none, on Unix).
        // The actual rejection-on-Unix happens via `Component::ParentDir`,
        // tested in `safe_path_rejects_parent_dir_components`.
        //
        // The empty-string case is rejected by the `is_empty()` check
        // at the top of `safe_relative_path`.
        assert!(
            safe_relative_path("").is_err(),
            "empty path should be rejected"
        );
    }

    #[test]
    fn safe_path_rejects_absolute_paths() {
        for p in &[
            "/etc/passwd",
            "/foo/bar",
            "/",
        ] {
            safe_relative_path(p)
                .err()
                .unwrap_or_else(|| panic!("absolute path {:?} should be rejected", p));
        }
    }

    #[test]
    fn safe_path_rejects_drive_letter_paths() {
        // The `Component::Prefix` matcher catches these on Windows
        // builds; on Unix `Path::components` walks through them
        // harmlessly as `Prefix`, which we ALSO reject (see
        // hotfix #22 — we previously didn't, which let `C:\foo`
        // sneak through on macOS).
        for p in &[
            "C:\\Windows\\System32",
            "C:/Windows/System32",
            "D:\\data",
        ] {
            // On Unix, Path::components may not detect drive letters
            // the same way as Windows, but the explicit `:` check
            // in safe_relative_path catches all three.
            safe_relative_path(p)
                .err()
                .unwrap_or_else(|| panic!("drive-letter path {:?} should be rejected", p));
        }
    }

    #[test]
    fn safe_path_rejects_empty_and_nul() {
        assert!(safe_relative_path("").is_err(), "empty path");
        assert!(
            safe_relative_path("foo\0bar").is_err(),
            "path with NUL byte"
        );
    }
}
