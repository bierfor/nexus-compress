//! NXAR — multi-file archive container for the NexusCompress engine.
//!
//! When the user wants to compress a whole directory, we compress
//! each file independently (so partial extraction is possible and
//! per-file dedup + dict encoding stay optimal) and then concatenate
//! the per-file `.nxr` streams with a small header that records
//! the path + sizes of each entry.
//!
//! ## Container format (v1)
//!
//! ```text
//! offset  size   field
//! ------  ----   -------------------------------------------------------
//! 0       4      magic b"NXAR"     (NEXUS ARchive)
//! 4       1      version = 0x01
//! 5       3      reserved (zero)
//! 8       4      n_files   (u32 LE)
//! 12      …      per-file entries (n_files of them):
//!                  [u16 name_len][name_bytes UTF-8]
//!                  [u64 original_size]
//!                  [u64 compressed_size]
//!                  [u8 × compressed_size]   ← a complete .nxr stream
//! ```
//!
//! Each per-file entry is a self-contained `.nxr` stream produced
//! by `crate::compress`. The container itself is NOT additionally
//! compressed at the NXAR level — the trade-off is: simple
//! per-file extraction vs. a small overhead (~30 bytes/entry)
//! that doesn't amortize into a meaningful win for typical
//! folder sizes (10-1000 files). A future v2 could re-compress
//! the concatenation to gain inter-file dedup.

use crate::codec::{compress, decompress};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const NXAR_MAGIC: &[u8; 4] = b"NXAR";
const NXAR_VERSION: u8 = 0x01;

/// Per-file entry in the archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// Path relative to the archive root, using '/' separator.
    pub path: String,
    pub original_size: u64,
    pub compressed_size: u64,
    pub compress_time_ms: f64,
}

/// Result of compressing a directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryResult {
    /// Absolute path of the directory that was compressed (or
    /// extracted to, in the decompress case).
    pub root: String,
    /// Number of files in the archive.
    pub n_files: u64,
    /// Total size of the original files in bytes.
    pub total_original_size: u64,
    /// Total size of the compressed payload in bytes.
    pub total_compressed_size: u64,
    /// `total_original / total_compressed`. 0.0 if no files.
    pub aggregate_ratio: f64,
    /// Wall-clock time of the operation, in milliseconds.
    pub total_time_ms: f64,
    /// Per-file breakdown.
    pub entries: Vec<ArchiveEntry>,
}

/// Recursively walk a directory and collect all regular files.
/// Symlinks are skipped (we don't follow them to avoid loops and
/// to keep the archive self-contained).
pub fn walk(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        let md = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(_) => continue, // skip unreadable entries
        };
        if md.is_file() {
            out.push(p);
        } else if md.is_dir() {
            let rd = match fs::read_dir(&p) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                stack.push(e.path());
            }
        }
        // symlinks and other types are skipped
    }
    out.sort();
    Ok(out)
}

/// Read a file, returning its bytes. Used by the per-file
/// compression loop. Returns an error if the file can't be read.
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut f = fs::File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Compress a directory into an NXAR archive (returned as bytes).
///
/// `root` is the directory the user picked. Each regular file
/// under it is compressed independently with the v4 engine; the
/// per-file `.nxr` streams are concatenated under the NXAR
/// header. Returns aggregate stats and the archive bytes.
pub fn compress_directory(root: &Path) -> Result<(DirectoryResult, Vec<u8>), String> {
    let files = walk(root).map_err(|e| format!("walk failed: {}", e))?;
    if files.is_empty() {
        return Err("directory is empty (no regular files)".into());
    }

    let total_start = Instant::now();
    let mut entries: Vec<ArchiveEntry> = Vec::with_capacity(files.len());
    let mut payloads: Vec<Vec<u8>> = Vec::with_capacity(files.len());
    let mut total_original: u64 = 0;
    let mut total_compressed: u64 = 0;

    for abs in &files {
        // Compute path relative to root, with '/' separators
        // (NXAR uses a single canonical separator for cross-platform
        // archives).
        let rel = abs
            .strip_prefix(root)
            .unwrap_or(abs)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = match read_file(abs) {
            Ok(b) => b,
            Err(_e) => {
                // Skip unreadable files but record 0 bytes so the
                // user sees them in the entry list.
                entries.push(ArchiveEntry {
                    path: rel,
                    original_size: 0,
                    compressed_size: 0,
                    compress_time_ms: 0.0,
                });
                payloads.push(Vec::new());
                continue;
            }
        };
        let original_size = bytes.len() as u64;
        let t0 = Instant::now();
        let compressed = compress(&bytes);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        let compressed_size = compressed.len() as u64;
        total_original += original_size;
        total_compressed += compressed_size;
        entries.push(ArchiveEntry {
            path: rel,
            original_size,
            compressed_size,
            compress_time_ms: dt,
        });
        payloads.push(compressed);
    }

    // Serialize the NXAR container.
    let archive = serialize_nxar(&entries, &payloads)?;
    let total_time = total_start.elapsed().as_secs_f64() * 1000.0;
    let aggregate_ratio = if total_compressed == 0 {
        0.0
    } else {
        total_original as f64 / total_compressed as f64
    };
    let result = DirectoryResult {
        root: root.to_string_lossy().into_owned(),
        n_files: entries.len() as u64,
        total_original_size: total_original,
        total_compressed_size: total_compressed,
        aggregate_ratio,
        total_time_ms: total_time,
        entries,
    };
    Ok((result, archive))
}

/// Decompress an NXAR archive into `output_dir`.
///
/// Each entry's path (relative to the archive root) is created
/// under `output_dir` with the same directory structure. Existing
/// files are overwritten.
pub fn decompress_directory(archive: &[u8], output_dir: &Path) -> Result<DirectoryResult, String> {
    let (entries, payloads) = deserialize_nxar(archive)?;
    fs::create_dir_all(output_dir).map_err(|e| format!("create output dir: {}", e))?;

    let total_start = Instant::now();
    let mut total_original: u64 = 0;
    let mut total_compressed: u64 = 0;
    let mut out_entries: Vec<ArchiveEntry> = Vec::with_capacity(entries.len());

    for (entry, payload) in entries.iter().zip(payloads.iter()) {
        if payload.is_empty() && entry.original_size == 0 {
            // Skipped file during the original compress (unreadable).
            out_entries.push(ArchiveEntry {
                path: entry.path.clone(),
                original_size: 0,
                compressed_size: 0,
                compress_time_ms: 0.0,
            });
            continue;
        }
        let t0 = Instant::now();
        let bytes = decompress(payload);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        // Sanity: recovered length matches what we recorded.
        if bytes.len() as u64 != entry.original_size {
            return Err(format!(
                "size mismatch for {}: expected {} got {}",
                entry.path,
                entry.original_size,
                bytes.len()
            ));
        }
        // Write to disk, creating parent dirs as needed.
        let out_path = output_dir.join(&entry.path);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!("create parent for {}: {}", entry.path, e)
            })?;
        }
        fs::write(&out_path, &bytes).map_err(|e| format!("write {}: {}", entry.path, e))?;
        total_original += entry.original_size;
        total_compressed += entry.compressed_size;
        out_entries.push(ArchiveEntry {
            path: entry.path.clone(),
            original_size: entry.original_size,
            compressed_size: entry.compressed_size,
            compress_time_ms: dt,
        });
    }

    let total_time = total_start.elapsed().as_secs_f64() * 1000.0;
    let aggregate_ratio = if total_compressed == 0 {
        0.0
    } else {
        total_original as f64 / total_compressed as f64
    };
    let result = DirectoryResult {
        root: output_dir.to_string_lossy().into_owned(),
        n_files: out_entries.len() as u64,
        total_original_size: total_original,
        total_compressed_size: total_compressed,
        aggregate_ratio,
        total_time_ms: total_time,
        entries: out_entries,
    };
    Ok(result)
}

/// Serialize the (entries, payloads) into NXAR bytes.
fn serialize_nxar(entries: &[ArchiveEntry], payloads: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.write_all(NXAR_MAGIC).map_err(|e| e.to_string())?;
    out.write_all(&[NXAR_VERSION]).map_err(|e| e.to_string())?;
    out.write_all(&[0u8; 3]).map_err(|e| e.to_string())?; // reserved
    out.write_all(&(entries.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    for (e, p) in entries.iter().zip(payloads.iter()) {
        let name = e.path.as_bytes();
        if name.len() > u16::MAX as usize {
            return Err(format!("path too long: {}", e.path));
        }
        out.write_all(&(name.len() as u16).to_le_bytes())
            .map_err(|e| e.to_string())?;
        out.write_all(name).map_err(|e| e.to_string())?;
        out.write_all(&e.original_size.to_le_bytes())
            .map_err(|e| e.to_string())?;
        out.write_all(&e.compressed_size.to_le_bytes())
            .map_err(|e| e.to_string())?;
        out.write_all(p).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Peek at an NXAR archive's manifest without loading the
/// per-file payloads. Returns the entry list (path, sizes)
/// parsed from the header. The payload bytes are NOT copied —
/// this is the cheap "show me what's inside" call that the UI
/// uses to populate the archive contents preview before the
/// user commits to extracting.
pub fn peek_archive(archive: &[u8]) -> Result<Vec<ArchiveEntry>, String> {
    if archive.len() < 12 {
        return Err("archive too short".into());
    }
    if &archive[0..4] != NXAR_MAGIC {
        return Err("bad magic (not an NXAR archive)".into());
    }
    if archive[4] != NXAR_VERSION {
        return Err(format!("unsupported NXAR version: {}", archive[4]));
    }
    let n_files = u32::from_le_bytes(
        archive[8..12]
            .try_into()
            .map_err(|_| "bad header")?,
    ) as usize;
    let mut entries = Vec::with_capacity(n_files);
    let mut cursor = 12usize;
    for _ in 0..n_files {
        if cursor + 2 + 8 + 8 > archive.len() {
            return Err("truncated entry header".into());
        }
        let name_len = u16::from_le_bytes(
            archive[cursor..cursor + 2]
                .try_into()
                .map_err(|_| "bad name len")?,
        ) as usize;
        cursor += 2;
        if cursor + name_len + 16 > archive.len() {
            return Err("truncated name".into());
        }
        let name = String::from_utf8(archive[cursor..cursor + name_len].to_vec())
            .map_err(|_| "non-utf8 name")?;
        cursor += name_len;
        let original_size = u64::from_le_bytes(
            archive[cursor..cursor + 8]
                .try_into()
                .map_err(|_| "bad original_size")?,
        );
        cursor += 8;
        let compressed_size = u64::from_le_bytes(
            archive[cursor..cursor + 8]
                .try_into()
                .map_err(|_| "bad compressed_size")?,
        );
        cursor += 8;
        let end = cursor
            .checked_add(compressed_size as usize)
            .ok_or("compressed_size overflow")?;
        if end > archive.len() {
            return Err("truncated payload".into());
        }
        cursor = end; // skip the payload without copying
        entries.push(ArchiveEntry {
            path: name,
            original_size,
            compressed_size,
            compress_time_ms: 0.0,
        });
    }
    Ok(entries)
}

/// Read a file from disk and peek at its NXAR manifest. This is the
/// fast path used by the UI when the user picks a .nxar via the
/// Tauri dialog (which returns a path) instead of the HTML5 file
/// input (which returns bytes and forces a slow IPC transfer).
pub fn peek_archive_file(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read failed: {}", e))?;
    peek_archive(&bytes)
}

/// Read a file from disk, peek the manifest, and (if `extract_to`
/// is Some) extract to that directory. Combines peek + extract
/// into a single disk-side operation so the entire archive bytes
/// never cross the IPC boundary.
pub fn peek_and_extract_file(
    path: &Path,
    extract_to: Option<&Path>,
) -> Result<(Vec<ArchiveEntry>, Option<DirectoryResult>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read failed: {}", e))?;
    let entries = peek_archive(&bytes)?;
    let result = match extract_to {
        Some(out) => Some(decompress_directory(&bytes, out)?),
        None => None,
    };
    Ok((entries, result))
}

/// Deserialize NXAR bytes into (entries, payloads).
fn deserialize_nxar(archive: &[u8]) -> Result<(Vec<ArchiveEntry>, Vec<Vec<u8>>), String> {
    if archive.len() < 12 {
        return Err("archive too short".into());
    }
    if &archive[0..4] != NXAR_MAGIC {
        return Err("bad magic (not an NXAR archive)".into());
    }
    if archive[4] != NXAR_VERSION {
        return Err(format!("unsupported NXAR version: {}", archive[4]));
    }
    let n_files = u32::from_le_bytes(
        archive[8..12]
            .try_into()
            .map_err(|_| "bad header")?,
    ) as usize;
    let mut entries = Vec::with_capacity(n_files);
    let mut payloads = Vec::with_capacity(n_files);
    let mut cursor = 12usize;
    for _ in 0..n_files {
        if cursor + 2 + 8 + 8 > archive.len() {
            return Err("truncated entry header".into());
        }
        let name_len = u16::from_le_bytes(
            archive[cursor..cursor + 2]
                .try_into()
                .map_err(|_| "bad name len")?,
        ) as usize;
        cursor += 2;
        if cursor + name_len + 16 > archive.len() {
            return Err("truncated name".into());
        }
        let name = String::from_utf8(archive[cursor..cursor + name_len].to_vec())
            .map_err(|_| "non-utf8 name")?;
        cursor += name_len;
        let original_size = u64::from_le_bytes(
            archive[cursor..cursor + 8]
                .try_into()
                .map_err(|_| "bad original_size")?,
        );
        cursor += 8;
        let compressed_size = u64::from_le_bytes(
            archive[cursor..cursor + 8]
                .try_into()
                .map_err(|_| "bad compressed_size")?,
        );
        cursor += 8;
        let end = cursor
            .checked_add(compressed_size as usize)
            .ok_or("compressed_size overflow")?;
        if end > archive.len() {
            return Err("truncated payload".into());
        }
        let payload = archive[cursor..end].to_vec();
        cursor = end;
        entries.push(ArchiveEntry {
            path: name,
            original_size,
            compressed_size,
            compress_time_ms: 0.0, // not stored in the archive
        });
        payloads.push(payload);
    }
    Ok((entries, payloads))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_temp_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        // Small file
        let mut f = std::fs::File::create(dir.path().join("a.txt")).unwrap();
        f.write_all(b"the quick brown fox jumps over the lazy dog. ").unwrap();
        // Medium file
        let mut f = std::fs::File::create(dir.path().join("b.txt")).unwrap();
        for _ in 0..200 {
            f.write_all(b"the quick brown fox jumps over the lazy dog. ")
                .unwrap();
        }
        // Subdir with a file
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let mut f = std::fs::File::create(dir.path().join("sub/c.txt")).unwrap();
        f.write_all(b"another file in a subdir").unwrap();
        dir
    }

    #[test]
    fn walk_finds_all_files() {
        let dir = make_temp_tree();
        let files = walk(dir.path()).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"b.txt".to_string()));
        assert!(names.contains(&format!("sub{}c.txt", std::path::MAIN_SEPARATOR)));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn roundtrip_nxar() {
        let dir = make_temp_tree();
        let (result, archive) = compress_directory(dir.path()).unwrap();
        assert_eq!(result.n_files, 3);
        assert!(result.total_original_size > 0);
        assert!(result.total_compressed_size > 0);
        assert!(result.aggregate_ratio > 1.0, "should compress");
        assert_eq!(archive[0..4], *b"NXAR");

        // Decompress into a new dir and verify byte equality.
        let out_dir = tempfile::tempdir().unwrap();
        let dec = decompress_directory(&archive, out_dir.path()).unwrap();
        assert_eq!(dec.n_files, 3);
        assert_eq!(
            dec.total_original_size, result.total_original_size,
            "total_original mismatch after roundtrip"
        );
        // Each file recovered identically.
        for e in &result.entries {
            let orig_bytes = std::fs::read(dir.path().join(&e.path)).unwrap();
            let recovered_bytes = std::fs::read(out_dir.path().join(&e.path)).unwrap();
            assert_eq!(orig_bytes, recovered_bytes, "file {} differs", e.path);
        }
    }

    #[test]
    fn roundtrip_nxar_empty_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        let r = compress_directory(dir.path());
        assert!(r.is_err());
    }

    #[test]
    fn peek_matches_full_roundtrip() {
        let dir = make_temp_tree();
        let (_result, archive) = compress_directory(dir.path()).unwrap();
        let peeked = peek_archive(&archive).unwrap();
        assert_eq!(peeked.len(), 3);
        for p in &peeked {
            assert!(p.original_size > 0 || p.path.contains("missing"));
            assert!(!p.path.is_empty());
        }
    }

    #[test]
    fn peek_rejects_bad_magic() {
        let bogus = b"JUNK\x01\x00\x00\x00\x00\x00\x00\x00";
        let r = peek_archive(bogus);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("magic"));
    }

    #[test]
    fn deserialize_rejects_bad_magic() {
        let bogus = b"JUNK\x01\x00\x00\x00\x00\x00\x00\x00";
        let r = deserialize_nxar(bogus);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("magic"));
    }
}
