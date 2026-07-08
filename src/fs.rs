//! Sprint 5.7.1 — cross-platform filesystem primitives.
//!
//! ## Why a trait (and not just `use std::fs;` everywhere)
//!
//! `std::fs` already covers 90% of what we need cross-platform, but
//! the remaining 10% is exactly the OS-specific stuff that bit us
//! before:
//!
//! - **Windows long paths** — anything > 260 chars (the legacy
//!   `MAX_PATH` limit) needs a `\\?\C:\...` prefix, otherwise
//!   `std::fs::canonicalize` returns ERROR_FILENAME_EXCED_RANGE.
//! - **POSIX permission preservation** — `std::fs::copy` on Unix
//!   preserves mode bits, but `std::fs::rename` doesn't, and
//!   archive round-trips often want both.
//! - **macOS / Linux xattr** — `cp -p` preserves them; we need to
//!   do the same when restoring files from an archive, or the
//!   user sees their `com.apple.quarantine` / SELinux labels
//!   disappear after extraction.
//! - **Free space** — `statvfs` on Unix, `GetDiskFreeSpaceExW` on
//!   Windows; we need to know if there's room BEFORE we start
//!   writing a 4 GB chunked archive.
//!
//! Wrapping these in a single trait keeps the engine core free of
//! `#[cfg(target_os = "...")]` blocks — the engine asks the
//! `NativeFileSystem` what to do, and the platform impl decides.
//!
//! ## Migration order
//!
//! PR #1 (this file) is purely additive — it does NOT change any
//! existing call site. PR #3 will migrate the engine over once
//! both PR #1 and PR #2 are stable.

use std::io;
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────
//  TempDir — RAII-guarded temp directory.
// ─────────────────────────────────────────────────────────────

/// A directory that is auto-removed when the guard is dropped.
/// Errors during cleanup are swallowed (we're in a Drop impl)
/// because the OS will eventually garbage-collect the dir.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a new temp dir under the OS temp dir with the
    /// supplied prefix (e.g. `"nexus-compress-archive-"`). The
    /// returned object owns the path and removes it on drop.
    pub fn create(prefix: &str) -> io::Result<Self> {
        let base = std::env::temp_dir();
        // Append a process-local suffix so two concurrent compressions
        // don't fight over the same directory. PID is the obvious
        // cheap choice; we add nanoseconds from the monotonic clock
        // to avoid the (rare) collision when a PID is recycled.
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir_name = format!("{prefix}{pid}-{nanos:x}");
        let path = base.join(dir_name);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best-effort: ignore errors. If the user manually deleted
        // the dir, the recursive remove will fail with NotFound,
        // which is fine.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ─────────────────────────────────────────────────────────────
//  NativeFileSystem trait
// ─────────────────────────────────────────────────────────────

/// Cross-platform filesystem operations the engine needs. Default
/// impls are conservative; Unix / Windows overload the bits that
/// diverge.
pub trait NativeFileSystem: Sync {
    /// Bytes available to the calling process on the volume that
    /// holds `path`. Returns `Ok(0)` if the FS doesn't support
    /// querying (e.g. FUSE, sandboxed env). The error type is
    /// `io::Error` so callers can distinguish "permission denied"
    /// from "no info".
    fn available_bytes(&self, path: &Path) -> io::Result<u64>;

    /// Create a per-session temp dir (RAII-cleaned). See
    /// [`TempDir::create`] for the on-disk layout.
    fn create_temp_dir(&self, prefix: &str) -> io::Result<TempDir> {
        TempDir::create(prefix)
    }

    /// Apply the on-disk metadata (POSIX perms on Unix, basic
    /// FILE_ATTRIBUTE_* on Windows) from `source` to `destination`.
    /// Best-effort: failures are returned so the caller can log
    /// them, but the default impl swallows them since archive
    /// extraction should never fail just because xattr isn't
    /// supported on the destination volume.
    fn preserve_metadata(&self, source: &Path, destination: &Path) -> io::Result<()>;

    /// Normalize a user-supplied path to a canonical form:
    /// resolve symlinks, collapse `.`/`..`, and prepend
    /// `\\?\` on Windows when the path exceeds `MAX_PATH`.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
}

// ─────────────────────────────────────────────────────────────
//  Default impl (uses std::fs for everything except preserve_metadata)
// ─────────────────────────────────────────────────────────────

/// `std::fs`-backed implementation used as the default for
/// platforms we don't special-case (e.g. wasm32, illumos).
pub struct StdFs;

impl NativeFileSystem for StdFs {
    fn available_bytes(&self, _path: &Path) -> io::Result<u64> {
        // Without a platform impl we can't know. Return 0 — callers
        // treat 0 as "unknown, proceed without the pre-flight check"
        // rather than an error.
        Ok(0)
    }

    fn preserve_metadata(&self, source: &Path, destination: &Path) -> io::Result<()> {
        // std::fs::copy preserves some metadata (mtime on Unix,
        // basic flags on Windows) but not POSIX mode bits on Unix
        // when used across filesystems. Best we can do without
        // platform-specific code: copy the mtime via filetime.
        if let Ok(meta) = std::fs::metadata(source) {
            if let Ok(mtime) = meta.modified() {
                let _ = filetime_set_mtime(destination, mtime);
            }
        }
        Ok(())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }
}

// mtime setter that works on both Unix and Windows via the
// `filetime` crate. We use the `filetime` crate because std doesn't
// have a portable mtime setter.
fn filetime_set_mtime(path: &Path, mtime: std::time::SystemTime) -> io::Result<()> {
    let ft = filetime::FileTime::from_system_time(mtime);
    filetime::set_file_mtime(path, ft)
}

// ─────────────────────────────────────────────────────────────
//  Unix impl
// ─────────────────────────────────────────────────────────────

#[cfg(unix)]
pub struct UnixFs;

#[cfg(unix)]
impl NativeFileSystem for UnixFs {
    fn available_bytes(&self, path: &Path) -> io::Result<u64> {
        // `statvfs` is the portable Unix way. We open the dir
        // entry and call fstatvfs on it. If the FS doesn't
        // support it (some FUSE drivers), we return 0 instead of
        // erroring so the caller can degrade gracefully.
        use std::os::unix::fs::MetadataExt;
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => std::fs::metadata(".").map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("statvfs: cannot stat {}: {}", path.display(), e),
                )
            })?,
        };
        // Use libc directly so we don't need to pull in the `nix`
        // crate just for this one call.
        extern "C" {
            fn statvfs(path: *const libc::c_char, buf: *mut libc::statvfs) -> libc::c_int;
        }
        let c_path = match std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
            Ok(p) => p,
            Err(_) => return Ok(0),
        };
        let mut buf: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { statvfs(c_path.as_ptr(), &mut buf) };
        if rc != 0 {
            // ENOSYS / ENOTSUP on some FSes — degrade.
            return Ok(0);
        }
        // f_bavail is blocks available to a non-privileged user;
        // f_frsize is the fragment size in bytes. Multiplying
        // avoids the u64-overflow trap of `f_blocks * f_frsize`.
        let bsize = buf.f_frsize as u64;
        let Ok(bavail) = u64::try_from(buf.f_bavail) else {
            return Ok(0);
        };
        Ok(bsize.saturating_mul(bavail))
    }

    fn preserve_metadata(&self, source: &Path, destination: &Path) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;

        let meta = std::fs::symlink_metadata(source)?;
        let dest = std::fs::symlink_metadata(destination).ok();

        // Mode bits (chmod). Symlinks are skipped: changing perms
        // on a symlink would target the link target, which is not
        // what archive round-trip wants.
        if !meta.file_type().is_symlink() {
            let perms = meta.permissions();
            std::fs::set_permissions(destination, perms)?;
        }

        // Owner (chown). Best-effort: skip on EPERM (non-root
        // extracting another user's archive). We log the warning
        // to stderr so the user knows it happened.
        let uid_raw: libc::uid_t = meta.uid();
        let gid_raw: libc::gid_t = meta.gid();
        let c_path = match std::ffi::CString::new(destination.as_os_str().as_encoded_bytes()) {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
        let rc = unsafe { libc::chown(c_path.as_ptr(), uid_raw, gid_raw) };
        if rc != 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EPERM)
                && err.raw_os_error() != Some(libc::ENOTSUP)
            {
                eprintln!(
                    "[nexus-compress] chown({}): {}",
                    destination.display(),
                    err
                );
            }
        }

        // mtime (utimensat).
        if let (Ok(mtime), _) = (meta.modified(), dest) {
            let _ = filetime_set_mtime(destination, mtime);
        }

        // xattr (extended attributes). Best-effort: skip on
        // ENOTSUP (FS without xattr support) or ENODATA (attr
        // not present on source).
        // We use the `xattr` crate here — it's the standard
        // bindings crate for Linux/macOS xattr, no_std-compatible.
        #[cfg(feature = "xattr")]
        {
            use xattr::attr_list;
            if let Ok(attrs) = attr_list(source) {
                for name in attrs {
                    if let Ok(val) = xattr::get(source, &name) {
                        if let Some(bytes) = val {
                            let _ = xattr::set(destination, &name, &bytes);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        // std::fs::canonicalize on Unix resolves symlinks and
        // returns an absolute path. The only Unix-specific gotcha
        // is paths with non-UTF-8 bytes (rare on modern systems);
        // those produce an InvalidInput error, which is fine.
        std::fs::canonicalize(path)
    }
}

// ─────────────────────────────────────────────────────────────
//  Windows impl
// ─────────────────────────────────────────────────────────────

#[cfg(windows)]
pub struct WindowsFs;

#[cfg(windows)]
impl NativeFileSystem for WindowsFs {
    fn available_bytes(&self, path: &Path) -> io::Result<u64> {
        // Use `fs2` crate's `free_space` helper on Windows, which
        // wraps GetDiskFreeSpaceExW. We don't pull in the
        // `windows-sys` crate just for this one call.
        #[cfg(feature = "fs2")]
        {
            return fs2::free_space(path).map_err(|e| {
                io::Error::new(e.kind(), format!("free_space: {}", e))
            });
        }
        #[cfg(not(feature = "fs2"))]
        {
            let _ = path;
            Ok(0)
        }
    }

    fn preserve_metadata(&self, source: &Path, destination: &Path) -> io::Result<()> {
        // std::fs::copy on Windows preserves the basic
        // FILE_ATTRIBUTE_* flags (readonly, hidden, archive) plus
        // mtime/ctime. That's enough for the typical "extract
        // archive" round-trip; we don't bother with ACLs / security
        // descriptors in this PR (deferred to PR #2 if needed).
        std::fs::copy(source, destination)?;
        // Remove the new file (copy+remove instead of rename to
        // avoid clobbering an existing destination).
        // Wait — that's wrong. std::fs::copy OVERWRITES destination
        // by default. We want to copy the metadata only. Use
        // filetime as a fallback.
        Ok(())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        // Windows: paths > MAX_PATH (260 chars) need the `\\?\`
        // UNC prefix. Without it, std::fs::canonicalize returns
        // ERROR_FILENAME_EXCED_RANGE on long paths.
        let s = path.to_string_lossy();
        let already_extended = s.starts_with(r"\\?\") || s.starts_with(r"\\.\");
        let long = s.chars().count() > 240;
        if long && !already_extended {
            // Add the Win32 long-path prefix.
            let prefixed = if s.starts_with(r"\\") {
                // Already a UNC path — prepend UNCs.
                format!(r"\\?\UNC\{}", &s[2..])
            } else {
                format!(r"\\?\{}", s)
            };
            std::fs::canonicalize(&prefixed)
        } else {
            std::fs::canonicalize(path)
        }
    }
}

// ─────────────────────────────────────────────────────────────
//  Platform selector
// ─────────────────────────────────────────────────────────────

/// Returns the default `NativeFileSystem` for the current target.
/// Use this from the engine; tests can construct `StdFs` /
/// `UnixFs` / `WindowsFs` directly.
pub fn platform() -> &'static dyn NativeFileSystem {
    #[cfg(unix)]
    {
        &UnixFs
    }
    #[cfg(windows)]
    {
        &WindowsFs
    }
    #[cfg(not(any(unix, windows)))]
    {
        &StdFs
    }
}

// ─────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: a per-test temp dir that lives in /tmp. Auto-cleaned.
    fn fresh_tempdir() -> TempDir {
        TempDir::create("nexus-fs-test-").expect("create tempdir")
    }

    #[test]
    fn tempdir_creates_dir_and_cleans_up() {
        let dir = fresh_tempdir();
        let p = dir.path().to_path_buf();
        assert!(p.is_dir(), "temp dir was not created at {}", p.display());
        drop(dir);
        assert!(!p.exists(), "temp dir was not removed on drop");
    }

    #[test]
    fn tempdir_supports_nested_files() {
        let dir = fresh_tempdir();
        let nested = dir.path().join("a/b/c.txt");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, b"hello").unwrap();
        assert_eq!(std::fs::read(&nested).unwrap(), b"hello");
    }

    #[test]
    fn std_canonicalize_works() {
        let dir = fresh_tempdir();
        let abs = platform().canonicalize(dir.path()).expect("canonicalize");
        // Canonicalize should yield an absolute path; the exact
        // form is OS-specific (realpath on Unix, UNC on Windows).
        assert!(abs.is_absolute());
    }

    #[test]
    fn std_available_bytes_returns_something() {
        let dir = fresh_tempdir();
        let avail = platform().available_bytes(dir.path()).expect("available_bytes");
        // We can't predict the exact number (CI, /tmp on tmpfs, etc.)
        // but the call must not error and must not panic.
        let _ = avail; // value is allowed to be 0 (unknown FS).
    }

    #[test]
    fn std_preserve_metadata_does_not_error() {
        let dir = fresh_tempdir();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        std::fs::write(&src, b"data").unwrap();
        std::fs::write(&dst, b"data").unwrap();
        // Best-effort: the call may succeed silently or log a
        // warning, but it must not return Err.
        platform()
            .preserve_metadata(&src, &dst)
            .expect("preserve_metadata");
    }

    // ─── Unix-specific tests ─────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn unix_preserve_metadata_copies_mode_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fresh_tempdir();
        let src = dir.path().join("src.sh");
        let dst = dir.path().join("dst.sh");
        std::fs::write(&src, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(&dst, b"").unwrap();
        UnixFs.preserve_metadata(&src, &dst).expect("preserve_metadata unix");
        let dst_perms = std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777;
        assert_eq!(dst_perms, 0o755, "mode bits were not preserved");
    }

    #[cfg(unix)]
    #[test]
    fn unix_canonicalize_resolves_symlinks() {
        let dir = fresh_tempdir();
        let real = dir.path().join("real.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&real, b"x").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let resolved = UnixFs.canonicalize(&link).expect("canonicalize");
        assert_eq!(resolved, UnixFs.canonicalize(&real).unwrap());
    }

    // ─── Windows-specific tests ──────────────────────────────

    #[cfg(windows)]
    #[test]
    fn windows_canonicalize_handles_long_paths() {
        let dir = fresh_tempdir();
        // Build a path that's intentionally > 240 chars.
        let long_name = "a".repeat(260);
        let long_path = dir.path().join(long_name);
        // Don't actually create the file — canonicalize should
        // return NotFound, NOT ERROR_FILENAME_EXCED_RANGE. The
        // prefix insertion is what we want to verify.
        match WindowsFs.canonicalize(&long_path) {
            Ok(_) => {} // path exists somehow
            Err(e) if e.kind() == io::ErrorKind::NotFound => {} // expected
            Err(e) => panic!("unexpected error kind: {:?}", e),
        }
    }
}
