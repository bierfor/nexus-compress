//! Sprint 5.7.10-B: single source of truth for corpus walking +
//! skip-list filtering.
//!
//! ## Why this module exists
//!
//! Up to sprint 5.7.9 the codebase maintained **FOUR separate
//! directory walkers** and **seven separate skip-list constants**:
//!
//! Walkers:
//! 1. `nxar::walk` — recursive, no skip-list, returns paths
//! 2. `main::walk_dir_with_skip` — CLI walker, mode-blind skip-list
//!    + local `CorpusModeLike` enum (paris with `api::CorpusMode`)
//! 3. `api::walk_dir_for_solid` — API walker, mode-aware skip-list
//! 4. `bin::bench_solid::walk_dir` — bench walker, no skip
//!
//! Skip-lists (some duplicated verbatim, some partial):
//! - `main::SKIP_DIRS` (19 entries, missing `Thumbs.db`)
//! - `api::SKIP_DIRS` (21 entries, has `Thumbs.db`)
//! - `api::MINIMAL_SKIP_DIRS` (22 entries)
//! - `api::SKIP_SUFFIXES` (7 entries)
//! - `api::SOURCE_EXTS` (35 entries, duplicated in `classify_corpus_breakdown`)
//! - `api::ALLOWED_NO_EXT` (30 entries)
//! - `api::classify_corpus_breakdown::BUILD_ARTIFACT_DIRS` (14 entries,
//!   duplicated with `corpus_should_skip_dir` indirectly via `SKIP_DIRS`)
//!
//! They drifted. Sprint 5.7.9 part 3 was a hotfix that made the
//! CLI walker honor `NEXUS_CORPUS_MODE` after the user noticed the
//! CLI and the GUI gave different corpora on the same directory.
//! The `Thumbs.db` entry is still missing from the CLI list —
//! that's a Windows UX bug that nobody has hit on a Mac.
//!
//! **This module is the only place that knows how to walk a
//! directory.** Every other call site imports from here.
//!
//! ## What it gives you
//!
//! - **The skip-list constants.** Single source of truth. Adding
//!   `Thumbs.db`-style entries is a one-line change here.
//! - [`walk`]: the main entry point. Returns the file list
//!   (`Vec<(String, Vec<u8>)>`), the total skipped bytes, and
//!   the corpus breakdown (source / build_artifact / other) in
//!   a single pass. The previous code did TWO passes — one for
//!   the walk, one for the breakdown classification — which is
//!   ~2x slower on a 200k-file corpus.
//! - [`walk_paths`]: lightweight path-only walker for code that
//!   doesn't need the bytes (e.g. `nxar`, bench harness).
//! - [`count_files`]: cheap O(n) file counter for the UI's
//!   "X files" stat.
//! - [`total_bytes`]: cheap O(n) byte counter for the UI's
//!   "X MiB total" stat.
//!
//! ## What it deliberately does NOT do
//!
//! - **No `.gitignore` parsing.** The previous design didn't
//!   have it either. The skip-list is the user's contract for
//!   "what to exclude", and we honour the CorpusMode (Source
//!   mode = skip dev caches, Minimal = keep source only, etc.).
//! - **No follow-symlinks.** A symlink loop on a 200k-file
//!   corpus would have been a denial-of-service. We use
//!   `symlink_metadata` and skip anything that isn't a real
//!   file or directory.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::api::{CorpusBreakdown, CorpusMode};

// ============================================================================
// Skip-list constants (the consolidated truth)
// ============================================================================

/// Directories to skip in **Source** and **Minimal** modes. The
/// `Everything` mode (the 5.7.7 default) skips NOTHING.
///
/// This is the **superset** of the previously-duplicated lists:
/// - `main::SKIP_DIRS` (the CLI's list)
/// - `api::SKIP_DIRS` (the GUI's list, had `Thumbs.db` extra)
///
/// The user's pattern of "no drift on hotfix additions" is the
/// reason the lists are now identical. Adding a new entry here
/// automatically applies to GUI, CLI, and the bench.
pub const SKIP_DIRS: &[&str] = &[
    ".DS_Store", ".cache", ".claude", ".git", ".next", ".nexus",
    ".nyc_output", ".parcel-cache", ".run", ".swc", ".tmp", ".turbo",
    ".venv", "Thumbs.db", "__pycache__", "build", "coverage", "dist",
    "log", "logs", "node_modules", "target", "venv",
];

/// Additional directories to skip in **Minimal** mode (assets,
/// fixtures, screenshots, etc. — keep only source + manifests).
/// Adds to `SKIP_DIRS`.
pub const MINIMAL_SKIP_DIRS: &[&str] = &[
    "__snapshots__", "assets", "audio", "demo", "doc", "docs",
    "documentation", "examples", "fixtures", "fonts", "icons",
    "images", "img", "media", "mocks", "public", "screenshots",
    "snapshots", "static", "test-data", "testdata", "videos",
];

/// File **names** to skip in Source / Minimal mode (lock files,
/// package manager metadata).
pub const SKIP_FILES: &[&str] = &[
    "bun.lockb", "package-lock.json", "pnpm-lock.yaml", "yarn.lock",
];

/// File **name suffix** patterns to skip in Source / Minimal
/// mode. A file is skipped if its name ends with any of these.
pub const SKIP_SUFFIXES: &[&str] = &[
    ".bak", ".pid", ".sock", ".swp", ".tmp", ".tsbuildinfo", "~",
];

/// **Minimal** mode allow-list: file extensions that count as
/// "source" (and are therefore included). Files NOT matching
/// one of these are skipped in Minimal mode.
///
/// **Sprint 5.7.10-B:** lifted verbatim from the previous
/// `api::corpus_should_skip_file` `SOURCE_EXTS` const. The
/// list does NOT include `json` — the previous design
/// intentionally kept `package.json` / `tsconfig.json` etc.
/// in `ALLOWED_NO_EXT` (the no-extension allow-list) even
/// though they DO have an extension. This is a known quirk
/// in the original logic that we preserve as-is to avoid a
/// behavior change. Fixing the `json` semantics is a
/// separate concern for a future sprint.
pub const SOURCE_EXTS: &[&str] = &[
    "adoc", "bash", "c", "cc", "cjs", "clj", "cljs", "cpp", "cs",
    "cxx", "dart", "erl", "ex", "exs", "fish", "go", "gql",
    "graphql", "h", "hpp", "hrl", "hs", "hxx", "java", "jl", "js",
    "jsx", "kt", "kts", "lhs", "lua", "md", "mdx", "mjs", "ml",
    "mli", "php", "proto", "ps1", "py", "pyi", "pyx", "r", "rb",
    "rs", "rst", "sc", "scala", "sh", "sql", "svelte", "swift",
    "ts", "tsx", "txt", "vue", "zsh",
];

/// **Minimal** mode allow-list: file **names** that should be
/// kept. The list mixes files without extensions (Makefiles,
/// READMEs) with files that have extensions (package.json,
/// tsconfig.json) — the previous design kept `*.json` config
/// files in the no-extension allow-list to avoid an
/// extension-based check that would skip them. We preserve
/// that quirk here.
pub const ALLOWED_NO_EXT: &[&str] = &[
    ".editorconfig", ".eslintrc", ".eslintrc.js", ".eslintrc.json",
    ".gitattributes", ".gitignore", ".prettierrc", ".prettierrc.js",
    ".prettierrc.json", "CHANGELOG", "CONTRIBUTING", "Cargo.lock",
    "Cargo.toml", "Dockerfile", "GNUmakefile", "Gemfile", "LICENCE",
    "LICENSE", "Makefile", "NOTICE", "Pipfile", "Pipfile.lock",
    "Procfile", "README", "Rakefile", "Vagrantfile", "build.gradle",
    "build.gradle.kts", "go.mod", "go.sum", "package.json", "pom.xml",
    "pyproject.toml", "requirements.txt", "setup.cfg", "setup.py",
    "tsconfig.json",
];

/// Directories that count as **build artifact** in the corpus
/// breakdown classification. Used to label the corpus (e.g. UI
/// warning "your corpus is 87% build artifacts"). The previous
/// version of this list (inline in `classify_corpus_breakdown`)
/// was a partial subset of `SKIP_DIRS`. The consolidated version
/// is the same — kept separate because the **purpose** is
/// different (breakdown classification vs walk filtering), even
/// though the entries happen to overlap.
pub const BUILD_ARTIFACT_DIRS: &[&str] = &[
    ".cache", ".next", ".next-cache", ".parcel-cache", ".swc", ".turbo",
    ".venv", "__pycache__", "build", "coverage", "dist", "node_modules",
    "target", "venv",
];

// ============================================================================
// Walker
// ============================================================================

/// The result of a corpus walk: the file list, the bytes skipped
/// by the skip-list, and a breakdown of the corpus by category.
///
/// The breakdown is computed DURING the walk (single pass over
/// the filesystem) so we don't pay for two traversals on a
/// 200k-file corpus.
#[derive(Debug, Clone, Default)]
pub struct WalkResult {
    /// `(relative_path, bytes)` for every file that passed the
    /// skip filter. Sorted ascending by path for deterministic
    /// archive ordering. Paths use `/` as the separator on all
    /// platforms (the `\\` → `/` rewrite happens here so the
    /// archive format is platform-independent).
    pub files: Vec<(String, Vec<u8>)>,
    /// Total bytes of files / dirs NOT walked. Includes both
    /// skipped directories (size estimated via inode
    /// allocation, not recursive — see `dir_size_estimate`) and
    /// skipped individual files (lock files, etc.).
    pub skipped_bytes: u64,
    /// Classification of the included files by category. Used by
    /// the UI to show the "X% build artifacts" warning.
    pub breakdown: CorpusBreakdown,
}

/// Walk a directory recursively and return every file that
/// passes the skip-list for the given `mode`. The output is
/// sorted by path.
///
/// `mode = Everything` walks every file (no skip). `mode = Source`
/// skips dev caches and lock files. `mode = Minimal` keeps only
/// source code + manifests.
///
/// Symlinks are NOT followed (loop safety on weird filesystems).
/// Unreadable entries are silently skipped (a permission error
/// on one file shouldn't fail the whole walk).
///
/// # Performance
///
/// Single-threaded recursive walk. The `read_dir` is the
/// bottleneck on large filesystems — for a 200k-file corpus
/// the walk itself takes ~6 seconds on an M4 Pro (see sprint
/// 5.7.2 hotfix #45 for the parallel-walk discussion).
pub fn walk(root: &Path, mode: CorpusMode) -> io::Result<WalkResult> {
    let mut result = WalkResult::default();
    walk_recursive(root, root, mode, &mut result)?;
    result
        .files
        .sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

fn walk_recursive(
    root: &Path,
    dir: &Path,
    mode: CorpusMode,
    result: &mut WalkResult,
) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()), // skip unreadable dirs silently
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // skip unreadable entries
        };
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            if should_skip_dir(&path, mode) {
                result.skipped_bytes += dir_size_estimate(&path);
                continue;
            }
            walk_recursive(root, &path, mode, result)?;
        } else if file_type.is_file() {
            if should_skip_file(&path, mode) {
                if let Ok(md) = entry.metadata() {
                    result.skipped_bytes += md.len();
                }
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue, // skip unreadable files
            };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            // Classify the file by path so the UI can warn
            // about build-artifact-heavy corpora.
            result.breakdown.classify(&rel, bytes.len() as u64);
            result.files.push((rel, bytes));
        }
        // symlinks and other types are intentionally skipped
    }
    Ok(())
}

/// Walk a directory and return just the paths (no bytes read,
/// no skip filtering). Cheap O(n) traversal used by code that
/// only needs the file list (e.g. the `nxar` per-file archive
/// path, the bench harness).
pub fn walk_paths(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        let md = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(_) => continue,
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

/// Count regular files recursively. Cheap O(n) walk used for
/// the UI's "X files" stat before the actual compression
/// starts.
pub fn count_files(root: &Path) -> u64 {
    fn walk(p: &Path) -> u64 {
        let mut n = 0u64;
        let entries = match fs::read_dir(p) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                n += walk(&entry.path());
            } else if ft.is_file() {
                n += 1;
            }
        }
        n
    }
    walk(root)
}

/// Compute the total bytes of all regular files recursively.
/// Cheap O(n) walk used for the UI's "X MiB total" stat.
pub fn total_bytes(root: &Path) -> u64 {
    fn walk(p: &Path) -> u64 {
        let mut total = 0u64;
        let entries = match fs::read_dir(p) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                total += walk(&entry.path());
            } else if ft.is_file() {
                if let Ok(md) = entry.metadata() {
                    total += md.len();
                }
            }
        }
        total
    }
    walk(root)
}

// ============================================================================
// Skip-list predicates
// ============================================================================

/// True if `path` should be skipped entirely (its dir is in the
/// skip-list for `mode`).
///
/// `Everything` mode: never skip.
/// `Source` mode: skip any dir in `SKIP_DIRS`.
/// `Minimal` mode: skip any dir in `SKIP_DIRS` OR `MINIMAL_SKIP_DIRS`.
fn should_skip_dir(path: &Path, mode: CorpusMode) -> bool {
    if matches!(mode, CorpusMode::Everything) {
        return false;
    }
    // Any component of the path matching the skip-list is enough
    // to reject the whole sub-tree. Using `components()` instead
    // of substring matching (e.g. `path.to_string_lossy().contains("node_modules")`)
    // so we don't false-positive on a file literally named
    // `node_modules.txt` at the root.
    for comp in path.components() {
        let Some(name) = comp.as_os_str().to_str() else {
            continue;
        };
        if SKIP_DIRS.binary_search(&name).is_ok() {
            return true;
        }
        if matches!(mode, CorpusMode::Minimal)
            && MINIMAL_SKIP_DIRS.binary_search(&name).is_ok()
        {
            return true;
        }
    }
    false
}

/// True if `path` (a file) should be skipped for the given mode.
///
/// `Everything` mode: never skip.
/// `Source` mode: skip lock files and build-cache suffixes.
/// `Minimal` mode: skip everything that isn't in `SOURCE_EXTS`
/// (or `ALLOWED_NO_EXT` for no-extension files).
fn should_skip_file(path: &Path, mode: CorpusMode) -> bool {
    if matches!(mode, CorpusMode::Everything) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // Lock files / package metadata (Source + Minimal).
    if SKIP_FILES.binary_search(&name).is_ok() {
        return true;
    }
    // Suffix patterns (Source + Minimal).
    for s in SKIP_SUFFIXES {
        if name.ends_with(s) {
            return true;
        }
    }
    // Minimal: only keep files whose extension (or special
    // no-extension name) is in the allow-list.
    if matches!(mode, CorpusMode::Minimal) {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => {
                let ext_lower = ext.to_ascii_lowercase();
                if SOURCE_EXTS.binary_search(&ext_lower.as_str()).is_err() {
                    return true;
                }
            }
            None => {
                if ALLOWED_NO_EXT.binary_search(&name).is_err() {
                    return true;
                }
            }
        }
    }
    false
}

/// Non-recursive best-effort estimate of a directory's "size
/// contribution" to the skipped-bytes total. We use the dir's
/// own metadata (`metadata().len()`), which on macOS returns the
/// inode allocation — not the recursive sum. The UI labels the
/// result as an estimate; the user only needs to see "skipped
/// ~5 GiB of dev cache", not a precise byte count.
///
/// The previous recursive `dir_size` on a 164k-file corpus
/// (FlowNow) took 10+ minutes. The estimate is O(1).
fn dir_size_estimate(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

// ============================================================================
// Corpus breakdown (single pass, called from walk_recursive)
// ============================================================================

/// Extension of `CorpusBreakdown` that classifies a file by its
/// path. Lives here (in `walker.rs`) so the walk loop and the
/// classification happen in a single pass.
impl CorpusBreakdown {
    /// Classify a file by its relative path. The caller is the
    /// walker loop, which calls this for every file that passed
    /// the skip filter.
    pub fn classify(&mut self, rel: &str, size: u64) {
        let path = Path::new(rel);
        // "build_artifact" if any path component is a known
        // build dir (.next, node_modules, etc.).
        let mut in_artifact = false;
        for comp in path.components() {
            let Some(name) = comp.as_os_str().to_str() else {
                continue;
            };
            if BUILD_ARTIFACT_DIRS.binary_search(&name).is_ok() {
                in_artifact = true;
                break;
            }
        }
        // "source" if the extension matches the source allow-list
        // AND we're not in a build-artifact dir. Source files
        // accidentally placed in `dist/` count as build_artifact.
        if !in_artifact {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_ascii_lowercase();
                if SOURCE_EXTS.binary_search(&ext_lower.as_str()).is_ok() {
                    self.source_files += 1;
                    self.source_bytes += size;
                    return;
                }
            }
        }
        if in_artifact {
            self.build_artifact_files += 1;
            self.build_artifact_bytes += size;
        } else {
            self.other_files += 1;
            self.other_bytes += size;
        }
    }
}

// ============================================================================
// Sorted-set invariant enforcement
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Lock the sorted-unique invariant on every skip-list. A
    /// future contributor who appends an out-of-order entry or
    /// a duplicate gets a loud failure at test time. The
    /// `binary_search` calls in `should_skip_dir` and
    /// `should_skip_file` would silently misbehave on unsorted
    /// data.
    #[test]
    fn all_skip_lists_are_sorted_and_unique() {
        let lists: &[(&str, &[&str])] = &[
            ("SKIP_DIRS", SKIP_DIRS),
            ("MINIMAL_SKIP_DIRS", MINIMAL_SKIP_DIRS),
            ("SKIP_FILES", SKIP_FILES),
            ("SKIP_SUFFIXES", SKIP_SUFFIXES),
            ("SOURCE_EXTS", SOURCE_EXTS),
            ("ALLOWED_NO_EXT", ALLOWED_NO_EXT),
            ("BUILD_ARTIFACT_DIRS", BUILD_ARTIFACT_DIRS),
        ];
        for (name, list) in lists {
            // Sorted
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                list.to_vec(),
                "{}: list must be sorted ascending (binary_search invariant)",
                name
            );
            // Unique
            let set: HashSet<&&str> = list.iter().collect();
            assert_eq!(
                set.len(),
                list.len(),
                "{}: list has duplicate entries",
                name
            );
        }
    }

    #[test]
    fn everything_mode_skips_nothing() {
        // Walk a temp dir with both dev-cache and source files,
        // assert Everything mode returns all of them.
        let tmp = tempdir();
        write(&tmp.join("node_modules/foo.js"), b"x");
        write(&tmp.join("src/main.rs"), b"y");
        let r = walk(&tmp, CorpusMode::Everything).unwrap();
        assert_eq!(r.files.len(), 2);
        assert_eq!(r.skipped_bytes, 0);
    }

    #[test]
    fn source_mode_skips_node_modules() {
        let tmp = tempdir();
        write(&tmp.join("node_modules/foo.js"), b"x");
        write(&tmp.join("src/main.rs"), b"y");
        let r = walk(&tmp, CorpusMode::Source).unwrap();
        assert_eq!(r.files.len(), 1);
        assert!(r.files[0].0.ends_with("main.rs"));
    }

    #[test]
    fn source_mode_skips_lock_files() {
        let tmp = tempdir();
        write(&tmp.join("package-lock.json"), b"x");
        write(&tmp.join("index.js"), b"y");
        let r = walk(&tmp, CorpusMode::Source).unwrap();
        assert_eq!(r.files.len(), 1);
        assert!(r.files[0].0.ends_with("index.js"));
    }

    #[test]
    fn minimal_mode_keeps_only_source() {
        let tmp = tempdir();
        write(&tmp.join("node_modules/foo.js"), b"x"); // skipped
        write(&tmp.join("assets/logo.png"), b"x");     // skipped
        write(&tmp.join("src/main.rs"), b"y");         // kept
        write(&tmp.join("README.md"), b"y");           // kept
        write(&tmp.join("Makefile"), b"y");            // kept
        let r = walk(&tmp, CorpusMode::Minimal).unwrap();
        let names: Vec<&str> = r.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(r.files.len(), 3, "got: {:?}", names);
        assert!(names.iter().any(|n| n.ends_with("main.rs")));
        assert!(names.iter().any(|n| n.ends_with("README.md")));
        assert!(names.iter().any(|n| n.ends_with("Makefile")));
    }

    #[test]
    fn minimal_mode_rejects_non_source_extensions() {
        // Demonstrates the actual keep / skip contract:
        // - Source-extension files (rs) are kept
        // - No-extension files in ALLOWED_NO_EXT (Makefile) are kept
        // - Non-source extensions (png, zip, mp4) are skipped
        //
        // Note: `tsconfig.json` is in `ALLOWED_NO_EXT` but the
        // original logic checked extensions FIRST and only
        // fell through to the no-extension list when
        // `path.extension()` was None. So `tsconfig.json` is
        // effectively skipped in the original behavior. This
        // is a pre-existing quirk preserved here — fixing
        // the json semantics is a separate concern.
        let tmp = tempdir();
        write(&tmp.join("image.png"), b"x");   // skipped
        write(&tmp.join("archive.zip"), b"x"); // skipped
        write(&tmp.join("video.mp4"), b"x");   // skipped
        write(&tmp.join("src/main.rs"), b"y"); // kept (rs extension)
        write(&tmp.join("Makefile"), b"y");    // kept (no-ext, allow-list)
        let r = walk(&tmp, CorpusMode::Minimal).unwrap();
        let names: Vec<&str> = r.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(r.files.len(), 2, "got: {:?}", names);
        assert!(names.iter().any(|n| n.ends_with("main.rs")));
        assert!(names.iter().any(|n| n.ends_with("Makefile")));
    }

    #[test]
    fn breakdown_classifies_build_artifact() {
        let tmp = tempdir();
        write(&tmp.join("node_modules/foo.js"), b"x"); // build_artifact (dir)
        write(&tmp.join("src/main.rs"), b"y");         // source (rs ext)
        write(&tmp.join("dist/bundle.js"), b"z");      // build_artifact (dir wins)
        let r = walk(&tmp, CorpusMode::Everything).unwrap();
        assert_eq!(r.breakdown.build_artifact_files, 2,
            "node_modules/foo.js + dist/bundle.js");
        assert_eq!(r.breakdown.source_files, 1, "src/main.rs");
        assert_eq!(r.breakdown.other_files, 0);
    }

    #[test]
    fn breakdown_source_in_dist_dir_is_build_artifact() {
        // A `.js` file inside `dist/` is minified output, not
        // source. The classification correctly labels it as
        // build_artifact (dir wins over extension).
        let tmp = tempdir();
        write(&tmp.join("dist/bundle.js"), b"x");
        let r = walk(&tmp, CorpusMode::Everything).unwrap();
        assert_eq!(r.breakdown.build_artifact_files, 1);
        assert_eq!(r.breakdown.source_files, 0);
    }

    // -----------------------------------------------------------------
    // Test helpers (no external test-util dep needed)
    // -----------------------------------------------------------------

    fn tempdir() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "nexus-walker-test-{}-{}",
            std::process::id(),
            rand_u64()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn write(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn rand_u64() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
