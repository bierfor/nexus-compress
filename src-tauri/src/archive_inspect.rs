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
        "nxs6" | "nxs" => Ok("nxs6"),
        _ => Err(format!(
            "unsupported archive format: .{} (supported: .tar, .nxs6)",
            ext
        )),
    }
}

/// List all entries in the archive's central directory.
pub fn list_entries(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    match detect_format(path)? {
        "tar" => list_tar_entries(path),
        "nxs6" => list_nxs6_entries(path),
        other => Err(format!("unsupported format: {}", other)),
    }
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
        "nxs6" => extract_nxs6_entries(path, output_dir, selected),
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

fn list_nxs6_entries(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let entries = nexus_compress::api::peek_archive_file(path)
        .map_err(|e| format!("peek nxs6: {}", e))?;
    Ok(entries
        .into_iter()
        .map(|e| {
            let name = e.path.clone();
            ArchiveEntry {
                name: e.path,
                size: e.original_size,
                is_dir: name.ends_with('/'),
            }
        })
        .collect())
}

fn extract_nxs6_entries(
    path: &Path,
    output_dir: &Path,
    selected: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("read nxs6: {}", e))?;
    // deserialize_nxar returns the manifest and the raw
    // bytes of every entry in one pass. For selective
    // extraction we only write the entries the caller asked
    // for; for full extraction we write them all.
    let (entries, payload_bytes) = nexus_compress::nxar::deserialize_nxar(&bytes)
        .map_err(|e| format!("deserialize nxs6: {}", e))?;
    let selected_set: Option<std::collections::HashSet<String>> =
        selected.map(|v| v.into_iter().collect());
    let mut written = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let entry_path = entry.path.clone();
        if let Some(ref set) = selected_set {
            if !set.contains(&entry_path) {
                continue;
            }
        }
        if entry_path.starts_with('/') || entry_path.contains("..") {
            return Err(format!("unsafe path in nxs6: {}", entry_path));
        }
        let dest = output_dir.join(&entry_path);
        if entry_path.ends_with('/') {
            std::fs::create_dir_all(&dest)
                .map_err(|e| format!("mkdir: {}", e))?;
            written.push(entry_path);
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir parent: {}", e))?;
            }
            std::fs::write(&dest, &payload_bytes[i])
                .map_err(|e| format!("write {}: {}", dest.display(), e))?;
            written.push(entry_path);
        }
    }
    Ok(written)
}