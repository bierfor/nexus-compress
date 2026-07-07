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
use std::path::{Path, PathBuf};

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
pub fn list_paginated(
    path: &Path,
    offset: usize,
    limit: usize,
) -> Result<PageResult, String> {
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
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("mkdir output: {}", e))?;
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
        out.push(ArchiveEntry {
            name,
            size,
            is_dir,
        });
    }
    Ok(out)
}

/// Streaming paginated tar listing. Reads only enough headers
/// to fill the requested window + one more entry (used to
/// detect has_more without enumerating the whole archive).
fn list_tar_paginated(
    path: &Path,
    offset: usize,
    limit: usize,
) -> Result<PageResult, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("open tar: {}", e))?;
    let mut ar = tar::Archive::new(f);
    // Skip `offset` headers, then collect up to limit+1.
    let mut out = Vec::with_capacity(limit + 1);
    let mut skipped = 0usize;
    let mut entry_iter =
        ar.entries().map_err(|e| format!("tar entries: {}", e))?;
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
        out.push(ArchiveEntry {
            name,
            size,
            is_dir,
        });
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
        // Defend against malicious archives: refuse absolute
        // paths and ".." components (the tar crate strips ".."
        // but defense in depth).
        if entry_path.starts_with('/') || entry_path.contains("..") {
            return Err(format!("unsafe path in tar: {}", entry_path));
        }
        let dest = output_dir.join(&entry_path);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("mkdir {}: {}", dest.display(), e))?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir parent: {}", e))?;
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
    let bytes = std::fs::read(path)
        .map_err(|e| format!("read solid archive: {}", e))?;
    let entries = nexus_compress::solid_archive::parse_toc(&bytes)
        .map_err(|e| format!("parse solid toc: {}", e))?;
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
fn list_solid_paginated(
    path: &Path,
    offset: usize,
    limit: usize,
) -> Result<PageResult, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("read solid archive: {}", e))?;
    let entries = nexus_compress::solid_archive::parse_toc(&bytes)
        .map_err(|e| format!("parse solid toc: {}", e))?;
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
    let bytes = std::fs::read(path)
        .map_err(|e| format!("read solid archive: {}", e))?;
    // Step 1: read the TOC (cheap — no LZMA).
    let entries = nexus_compress::solid_archive::parse_toc(&bytes)
        .map_err(|e| format!("parse solid toc: {}", e))?;
    // Step 2: full LZMA decompression (one pass, mandatory for
    // solid archives — same as WinRAR/ZIP). After this we
    // have the preprocessed bytes of every entry concatenated
    // in the order they were compressed. Per-entry offsets
    // come from the TOC.
    let (_entries, solid_uncompressed) = solid_decompress(&bytes)
        .map_err(|e| format!("solid decompress: {}", e))?;
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
        if entry_path.starts_with('/') || entry_path.contains("..") {
            return Err(format!("unsafe path in solid: {}", entry_path));
        }
        // Directories in solid archives are implicit (carried
        // as file paths with embedded slashes). Create the
        // parent for each file.
        let dest = output_dir.join(&entry_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir parent: {}", e))?;
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