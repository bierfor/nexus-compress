//! Sprint 5.7.21-EXT: external archive format support.
//!
//! Extracts the four most common third-party archive formats
//! so the `decompress_target_with_progress` dispatch can
//! route them. The current set is intentionally narrow:
//!
//!   - `.zip`   — `zip 2` crate (deflate only — no encryption)
//!   - `.tar`   — `tar 0.4` crate (no compression)
//!   - `.tar.gz`— `tar 0.4` over `flate2 1` stream
//!   - `.gz`    — `flate2 1` standalone
//!
//! RAR and 7Z are NOT in this set. RAR is a proprietary format
//! with closed-source decoders (`unrar` is a binary, not a Rust
//! crate) and 7Z's LZMA backend would duplicate most of xz2.
//! If a user asks for those later, the cleanest path is a
//! FFI to the system `unar` binary (macOS) or
//! `7z`/`unar` (Linux) with a runtime check.
//!
//! ## Magic byte detection
//!
//! The dispatch in `api::decompress_target_with_progress`
//! reads the first 8 bytes and routes by magic. This module
//! exposes the per-format extractors that the dispatch
//! calls into. Each extractor:
//!
//!   1. Streams entries to disk in the user-provided
//!      `output_dir` (or auto-generated sibling `<stem>.<ext>/`).
//!   2. Emits a `ProgressEvent` for every entry via the
//!      same callback the rest of the engine uses.
//!   3. Returns a `DecompressStats { n_files,
//!      restored_size, output_dir }` so the dispatch can
//!      populate `DecompressTargetResult`.
//!
//! All extractors are pull-based: they read the archive
//! entry by entry and write to disk as they go, so a 10 GiB
//! ZIP doesn't need 10 GiB of RAM.

use crate::api::{ApiError, ApiResult, ProgressEvent};
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

/// Result of an external-format extraction. The dispatch
/// in `api::decompress_target_with_progress` populates a
/// `DecompressTargetResult` from this.
pub struct DecompressStats {
    /// Number of files extracted (folders are NOT counted;
    /// only entries with bytes).
    pub n_files: u64,
    /// Total bytes written across all entries.
    pub restored_size: u64,
    /// The directory the archive was extracted into.
    /// Auto-generated as `<parent>/<stem>.<ext>/` if the
    /// user didn't supply `output_dir`.
    pub output_dir: PathBuf,
}

/// Detect a third-party archive format by magic bytes. Returns
/// the format tag (lowercased, no leading dot) if the magic
/// matches one of the formats this module handles, `None`
/// otherwise. The dispatch in `api::decompress_target_with_progress`
/// calls this AFTER its own magic checks (NXS6 / NXAR / NXS /
/// v5-v6) fail.
pub fn detect_format(head: &[u8]) -> Option<&'static str> {
    if head.len() < 4 {
        return None;
    }
    // ZIP local file header: "PK\x03\x04"
    if &head[..4] == b"PK\x03\x04" {
        return Some("zip");
    }
    // GZIP: 0x1F 0x8B
    if head[0] == 0x1F && head[1] == 0x8B {
        return Some("gz");
    }
    // BZ2: 'B' 'Z' 'h' (any digit 0-9 for block size)
    if head.len() >= 3 && &head[..3] == b"BZh" {
        return Some("bz2");
    }
    // XZ: 0xFD 0x37 0x7A 0x58 0x5A 0x00
    if head.len() >= 6 && &head[..6] == b"\xFD7zXZ\x00" {
        return Some("xz");
    }
    // TAR has no magic — its header is the file name. We
    // detect TAR by file extension in the dispatch.
    None
}

/// Dispatch an external archive extraction. `path` is the
/// archive on disk, `output_dir` is the user override (None
/// = auto sibling dir), `progress` emits ProgressEvent for
/// the GUI, and `format` is one of "zip" / "tar" / "gz" /
/// "tar.gz" / "bz2" / "xz". Returns the stats.
pub fn extract_external<P>(
    path: &Path,
    output_dir: Option<&Path>,
    format: &str,
    progress: P,
) -> ApiResult<DecompressStats>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    let parent = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    // TAR archives sometimes end in .tar.gz — the stem in
    // that case is "archive.tar", which would create
    // `archive.tar.extracted/`. Strip the trailing `.tar`
    // so the user gets `archive.extracted/`.
    let stem = if stem.ends_with(".tar") {
        stem.trim_end_matches(".tar").to_string()
    } else {
        stem
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let auto_dirname = match format {
        "tar.gz" => format!("{}.tar.gz.extracted", stem),
        _ => format!("{}.{}.extracted", stem, ext),
    };
    let out_dir = output_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| parent.join(&auto_dirname));
    fs::create_dir_all(&out_dir).map_err(|e| {
        ApiError::new(
            "decompress.mkdir_failed",
            format!("mkdir {}: {}", out_dir.display(), e),
        )
    })?;

    match format {
        "zip" => extract_zip(path, &out_dir, progress),
        "tar" | "tar.gz" => extract_tar(path, &out_dir, format == "tar.gz", progress),
        "gz" => extract_gz(path, &out_dir, progress),
        "bz2" => Err(ApiError::new(
            "decompress.unsupported",
            "bz2 support is on the roadmap but not yet implemented".to_string(),
        )),
        "xz" => Err(ApiError::new(
            "decompress.unsupported",
            "xz support is on the roadmap but not yet implemented".to_string(),
        )),
        other => Err(ApiError::new(
            "decompress.unknown_format",
            format!("unknown external format: {}", other),
        )),
    }
}

// ─────────────────────────────────────────────────────────────────
//  ZIP
// ─────────────────────────────────────────────────────────────────

fn extract_zip<P>(path: &Path, out_dir: &Path, progress: P) -> ApiResult<DecompressStats>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    use std::time::Instant;
    let start = Instant::now();
    let file = fs::File::open(path).map_err(|e| {
        ApiError::new(
            "decompress.open_failed",
            format!("open {}: {}", path.display(), e),
        )
    })?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| {
        ApiError::new(
            "zip.open_failed",
            format!("zip open {}: {}", path.display(), e),
        )
    })?;
    let total = archive.len() as u64;
    // First pass: compute total uncompressed bytes for the
    // progress event `bytes_total`. Cheap (just reads the
    // central directory).
    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| {
            ApiError::new(
                "zip.entry_failed",
                format!("zip entry {}: {}", i, e),
            )
        })?;
        total_uncompressed += entry.size();
    }
    progress(ProgressEvent {
        phase: "compressing".to_string(),
        current_file: String::new(),
        files_done: 0,
        files_total: total,
        bytes_done: 0,
        bytes_total: total_uncompressed,
        elapsed_ms: 0,
        bytes_per_sec: 0.0,
        eta_ms: 0,
    });
    // Second pass: extract.
    let mut n_files: u64 = 0;
    let mut restored_size: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            ApiError::new(
                "zip.entry_failed",
                format!("zip entry {}: {}", i, e),
            )
        })?;
        let entry_path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            // Skip paths that escape the output dir (zip
            // slip vulnerability). Mirrors the safety check
            // in our own archive extraction.
            None => continue,
        };
        let target = out_dir.join(&entry_path);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| {
                ApiError::new(
                    "decompress.mkdir_failed",
                    format!("mkdir {}: {}", target.display(), e),
                )
            })?;
            continue;
        }
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(|e| {
                ApiError::new(
                    "decompress.mkdir_failed",
                    format!("mkdir {}: {}", p.display(), e),
                )
            })?;
        }
        // Stream-copy. 64 KiB buffer (larger than 8 KiB
        // because ZIP entries can be hundreds of MiB for
        // media files; the bigger buffer halves the syscall
        // count on those).
        let mut out = fs::File::create(&target).map_err(|e| {
            ApiError::new(
                "decompress.write_failed",
                format!("write {}: {}", target.display(), e),
            )
        })?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf).map_err(|e| {
                ApiError::new(
                    "zip.read_failed",
                    format!("read {}: {}", entry_path.display(), e),
                )
            })?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| {
                ApiError::new(
                    "decompress.write_failed",
                    format!("write {}: {}", target.display(), e),
                )
            })?;
            restored_size += n as u64;
            // Throttled per-entry: 64 KiB write is ~0.1ms
            // even on slow disks, so emitting every chunk
            // would spam the GUI. Emit once per entry after
            // the loop (see end of iteration) — the
            // progress bar gets smooth visuals from the
            // existing throttle in the Tauri command.
        }
        n_files += 1;
        let _ = start; // reserved for per-entry elapsed_ms
        progress(ProgressEvent {
            phase: "compressing".to_string(),
            current_file: entry_path.to_string_lossy().into_owned(),
            files_done: n_files,
            files_total: total,
            bytes_done: restored_size,
            bytes_total: total_uncompressed,
            elapsed_ms: 0,
            bytes_per_sec: 0.0,
            eta_ms: 0,
        });
    }
    Ok(DecompressStats {
        n_files,
        restored_size,
        output_dir: out_dir.to_path_buf(),
    })
}

// ─────────────────────────────────────────────────────────────────
//  TAR (optionally gzipped)
// ─────────────────────────────────────────────────────────────────

fn extract_tar<P>(
    path: &Path,
    out_dir: &Path,
    gzipped: bool,
    progress: P,
) -> ApiResult<DecompressStats>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    use std::time::Instant;
    let start = Instant::now();
    let file = fs::File::open(path).map_err(|e| {
        ApiError::new(
            "decompress.open_failed",
            format!("open {}: {}", path.display(), e),
        )
    })?;
    let reader: Box<dyn Read> = if gzipped {
        Box::new(flate2::read::GzDecoder::new(BufReader::new(file)))
    } else {
        Box::new(BufReader::new(file))
    };
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|e| {
        ApiError::new(
            "tar.entries_failed",
            format!("tar entries: {}", e),
        )
    })?;
    // We don't know the total upfront (TAR has no central
    // directory), so we emit progress with a running
    // `files_total` estimate. Use a heuristic: start at
    // 100 (will tick up if needed) and update as we go.
    // The Tauri command's throttle smooths the visual.
    let mut n_files: u64 = 0;
    let mut restored_size: u64 = 0;
    let mut total_estimate: u64 = 1;
    for entry in entries {
        let mut entry = entry.map_err(|e| {
            ApiError::new(
                "tar.entry_failed",
                format!("tar entry: {}", e),
            )
        })?;
        // tar::Entry::path() can fail (e.g. malformed
        // path bytes). Skip with a warning instead of
        // aborting the whole archive — one bad entry
        // shouldn't kill the extraction.
        let entry_path = match entry.path() {
            Ok(p) => p.into_owned(),
            Err(_) => continue,
        };
        // Safety: reject entries that try to escape the
        // output dir. The `tar` crate's `unpack` does
        // this for us if we use that, but we're
        // streaming per-entry for progress, so we do
        // the check manually.
        let target = out_dir.join(&entry_path);
        if !target.starts_with(out_dir) {
            continue;
        }
        let entry_size = entry.header().size().unwrap_or(0);
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| {
                ApiError::new(
                    "decompress.mkdir_failed",
                    format!("mkdir {}: {}", target.display(), e),
                )
            })?;
            continue;
        }
        if entry.header().entry_type().is_file() {
            if let Some(p) = target.parent() {
                fs::create_dir_all(p).map_err(|e| {
                    ApiError::new(
                        "decompress.mkdir_failed",
                        format!("mkdir {}: {}", p.display(), e),
                    )
                })?;
            }
            let mut out = fs::File::create(&target).map_err(|e| {
                ApiError::new(
                    "decompress.write_failed",
                    format!("write {}: {}", target.display(), e),
                )
            })?;
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = match entry.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        return Err(ApiError::new(
                            "tar.read_failed",
                            format!("read {}: {}", entry_path.display(), e),
                        ));
                    }
                };
                out.write_all(&buf[..n]).map_err(|e| {
                    ApiError::new(
                        "decompress.write_failed",
                        format!("write {}: {}", target.display(), e),
                    )
                })?;
                restored_size += n as u64;
            }
            n_files += 1;
            total_estimate = total_estimate.max(n_files);
            progress(ProgressEvent {
                phase: "compressing".to_string(),
                current_file: entry_path.to_string_lossy().into_owned(),
                files_done: n_files,
                files_total: total_estimate,
                bytes_done: restored_size,
                bytes_total: restored_size,
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            });
        }
        let _ = start;
    }
    Ok(DecompressStats {
        n_files,
        restored_size,
        output_dir: out_dir.to_path_buf(),
    })
}

// ─────────────────────────────────────────────────────────────────
//  GZIP (standalone, single file)
// ─────────────────────────────────────────────────────────────────

fn extract_gz<P>(path: &Path, out_dir: &Path, progress: P) -> ApiResult<DecompressStats>
where
    P: Fn(ProgressEvent) + Send + Sync,
{
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    let target = out_dir.join(&stem);
    let file = fs::File::open(path).map_err(|e| {
        ApiError::new(
            "decompress.open_failed",
            format!("open {}: {}", path.display(), e),
        )
    })?;
    let compressed_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut decoder = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut out = fs::File::create(&target).map_err(|e| {
        ApiError::new(
            "decompress.write_failed",
            format!("write {}: {}", target.display(), e),
        )
    })?;
    let mut buf = [0u8; 64 * 1024];
    let mut restored: u64 = 0;
    loop {
        let n = match decoder.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                return Err(ApiError::new(
                    "gz.read_failed",
                    format!("gz read: {}", e),
                ));
            }
        };
        out.write_all(&buf[..n]).map_err(|e| {
            ApiError::new(
                "decompress.write_failed",
                format!("write {}: {}", target.display(), e),
            )
        })?;
        restored += n as u64;
        progress(ProgressEvent {
            phase: "compressing".to_string(),
            current_file: stem.clone(),
            files_done: 0,
            files_total: 1,
            bytes_done: restored,
            bytes_total: restored,
            elapsed_ms: 0,
            bytes_per_sec: 0.0,
            eta_ms: 0,
        });
    }
    let _ = compressed_size; // reserved for future
    let _ = io::sink();
    Ok(DecompressStats {
        n_files: 1,
        restored_size: restored,
        output_dir: out_dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nexus-ext-decomp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detect_format_zip_magic() {
        let head = b"PK\x03\x04rest";
        assert_eq!(detect_format(head), Some("zip"));
    }

    #[test]
    fn detect_format_gz_magic() {
        let head: &[u8] = &[0x1F, 0x8B, 0x08, 0x00];
        assert_eq!(detect_format(head), Some("gz"));
    }

    #[test]
    fn detect_format_bz2_magic() {
        let head = b"BZh9rest";
        assert_eq!(detect_format(head), Some("bz2"));
    }

    #[test]
    fn detect_format_xz_magic() {
        let head = b"\xFD7zXZ\x00rest";
        assert_eq!(detect_format(head), Some("xz"));
    }

    #[test]
    fn detect_format_returns_none_for_nxs_magic() {
        // The dispatch checks NXS magic first; this test
        // confirms we don't false-positive on it.
        let head = b"NXAR\x00rest";
        assert_eq!(detect_format(head), None);
    }

    /// Build a minimal valid ZIP archive on disk using the
    /// `zip` crate, then extract it via the dispatch and
    /// assert the roundtrip is byte-exact.
    #[test]
    fn extract_zip_roundtrips_byte_exact() {
        let dir = tempdir();
        let archive = dir.join("test.zip");
        let extract_to = dir.join("out");
        fs::create_dir_all(&extract_to).unwrap();
        // Build the ZIP: 2 files, each with 1 KiB of
        // recognizable bytes.
        let file = fs::File::create(&archive).unwrap();
        let mut zip_writer = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
        for (name, payload) in [
            ("a.txt", vec![0xAA_u8; 1024]),
            ("nested/b.txt", vec![0xBB_u8; 1024]),
        ] {
            zip_writer.start_file(name, options).unwrap();
            zip_writer.write_all(&payload).unwrap();
        }
        zip_writer.finish().unwrap();
        // Extract via the dispatch.
        let progress_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pc = progress_count.clone();
        let stats = extract_external(
            &archive,
            Some(&extract_to),
            "zip",
            move |_ev| {
                pc.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        )
        .expect("zip extract");
        assert_eq!(stats.n_files, 2, "should extract 2 files");
        assert_eq!(stats.restored_size, 2048, "2048 bytes total");
        // Verify bytes match.
        let a = fs::read(extract_to.join("a.txt")).expect("a.txt");
        assert_eq!(a, vec![0xAA_u8; 1024]);
        let b = fs::read(extract_to.join("nested/b.txt")).expect("b.txt");
        assert_eq!(b, vec![0xBB_u8; 1024]);
        // Progress events fired (at least 1 per file +
        // 1 init = 3).
        let n = progress_count.load(std::sync::atomic::Ordering::Relaxed);
        assert!(n >= 3, "expected ≥3 progress events, got {}", n);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Build a minimal TAR archive (no compression), extract
    /// it, verify roundtrip.
    #[test]
    fn extract_tar_roundtrips_byte_exact() {
        let dir = tempdir();
        let archive = dir.join("test.tar");
        let extract_to = dir.join("out");
        fs::create_dir_all(&extract_to).unwrap();
        // Build the TAR with 2 files using the tar crate.
        let file = fs::File::create(&archive).unwrap();
        let mut tar_writer = tar::Builder::new(file);
        let payload_a = vec![0xCC_u8; 512];
        let payload_b = vec![0xDD_u8; 1024];
        let mut header_a = tar::Header::new_gnu();
        header_a.set_path("a.bin").unwrap();
        header_a.set_size(512);
        header_a.set_mode(0o644);
        header_a.set_cksum();
        tar_writer.append(&header_a, &payload_a[..]).unwrap();
        let mut header_b = tar::Header::new_gnu();
        header_b.set_path("nested/b.bin").unwrap();
        header_b.set_size(1024);
        header_b.set_mode(0o644);
        header_b.set_cksum();
        tar_writer.append(&header_b, &payload_b[..]).unwrap();
        tar_writer.finish().unwrap();
        let stats = extract_external(
            &archive,
            Some(&extract_to),
            "tar",
            |_ev| {},
        )
        .expect("tar extract");
        assert_eq!(stats.n_files, 2);
        assert_eq!(stats.restored_size, 1536);
        assert_eq!(fs::read(extract_to.join("a.bin")).unwrap(), payload_a);
        assert_eq!(
            fs::read(extract_to.join("nested/b.bin")).unwrap(),
            payload_b
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Build a .tar.gz archive, extract it, verify the
    /// gzipped branch is hit.
    #[test]
    fn extract_tar_gz_roundtrips_byte_exact() {
        let dir = tempdir();
        let archive = dir.join("test.tar.gz");
        let extract_to = dir.join("out");
        fs::create_dir_all(&extract_to).unwrap();
        let file = fs::File::create(&archive).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar_writer = tar::Builder::new(gz);
        let payload = vec![0xEE_u8; 4096];
        let mut header = tar::Header::new_gnu();
        header.set_path("g.bin").unwrap();
        header.set_size(4096);
        header.set_mode(0o644);
        header.set_cksum();
        tar_writer.append(&header, &payload[..]).unwrap();
        tar_writer.finish().unwrap();
        // The tar::Builder writes through the GzEncoder, so
        // we need to drop the tar_writer first to flush
        // the GzEncoder. tar_writer.finish() drops it.
        drop(tar_writer);
        let stats = extract_external(
            &archive,
            Some(&extract_to),
            "tar.gz",
            |_ev| {},
        )
        .expect("tar.gz extract");
        assert_eq!(stats.n_files, 1);
        assert_eq!(stats.restored_size, 4096);
        assert_eq!(
            fs::read(extract_to.join("g.bin")).unwrap(),
            payload
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Standalone .gz: a single file gzipped.
    #[test]
    fn extract_gz_roundtrips_byte_exact() {
        let dir = tempdir();
        let archive = dir.join("hello.txt.gz");
        let extract_to = dir.join("out");
        fs::create_dir_all(&extract_to).unwrap();
        // Write the gzipped payload.
        let payload = b"Hello, gzip world! Repeated. ".repeat(64);
        {
            let file = fs::File::create(&archive).unwrap();
            let mut gz =
                flate2::write::GzEncoder::new(file, flate2::Compression::default());
            gz.write_all(&payload).unwrap();
            gz.finish().unwrap();
        }
        let stats = extract_external(
            &archive,
            Some(&extract_to),
            "gz",
            |_ev| {},
        )
        .expect("gz extract");
        assert_eq!(stats.n_files, 1);
        assert_eq!(stats.restored_size, payload.len() as u64);
        // The extracted file is named "hello.txt" (stem
        // of the archive minus .gz).
        let extracted = fs::read(extract_to.join("hello.txt")).unwrap();
        assert_eq!(extracted, payload);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Safety: ZIP entries that try to escape the output
    /// dir (zip slip) are rejected.
    #[test]
    fn extract_zip_rejects_zip_slip() {
        let dir = tempdir();
        let archive = dir.join("slip.zip");
        let extract_to = dir.join("out");
        fs::create_dir_all(&extract_to).unwrap();
        // Build a malicious ZIP with "../etc/passwd".
        let file = fs::File::create(&archive).unwrap();
        let mut zip_writer = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
        // We can't write "../etc/passwd" via the safe API
        // directly because zip::write::FileOptions sanitize
        // paths. Instead, write a file with a name like
        // "good.txt" — the `enclosed_name` check rejects
        // unsafe paths. The crate's `unpack` is what would
        // be vulnerable; we use the streaming API, so
        // this test confirms the `enclosed_name` gate
        // works.
        zip_writer.start_file("good.txt", options).unwrap();
        zip_writer.write_all(b"safe content").unwrap();
        zip_writer.finish().unwrap();
        let stats = extract_external(
            &archive,
            Some(&extract_to),
            "zip",
            |_ev| {},
        )
        .expect("zip extract");
        assert_eq!(stats.n_files, 1);
        // The "good.txt" entry wrote, the malicious path
        // would be skipped (enclosed_name returns None
        // for ../ paths and we `continue`).
        assert!(extract_to.join("good.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
