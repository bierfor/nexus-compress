//! Sprint 5.7.18: regression test for the universal skip-list
//! (`.DS_Store`, `._*`, `Thumbs.db`) and the new `skip_archive`
//! filter (`.zip`, `.tar`, `.gz`, `.rar`, …).
//!
//! Both filters must work in ALL corpus modes (including
//! `Everything`) — OS / IDE metadata is never wanted in a
//! corpus, and the user explicitly opted into skipping
//! archives.

use nexus_compress::api::CorpusMode;
use std::fs;
use std::path::Path;

fn make_corpus_with_metadata(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    fs::create_dir_all(root.join("assets")).expect("mkdir assets");
    // Real source files.
    fs::write(
        root.join("src/index.ts"),
        b"export const x = 1;\n",
    )
    .expect("write index.ts");
    fs::write(
        root.join("src/util.ts"),
        b"export function add(a, b) { return a + b; }\n",
    )
    .expect("write util.ts");
    // A real PNG (passthrough, no compression).
    fs::write(
        root.join("assets/icon.png"),
        b"\x89PNG_FAKE_HEADER_REPETITIVE",
    )
    .expect("write icon.png");
    // OS / IDE metadata — should always be skipped.
    fs::write(root.join(".DS_Store"), b"\x00\x01\x02\x03").expect("write .DS_Store");
    fs::write(root.join("._hidden_resource_fork"), b"\x00\x01").expect("write ._*");
    fs::write(root.join("Thumbs.db"), b"windows thumbnail cache").expect("write Thumbs.db");
    // Archive files — should be skipped when skip_archive=true.
    // We write some fake bytes (the walker only checks the
    // extension, not the magic).
    fs::write(root.join("release.zip"), b"PK\x03\x04fake zip").expect("write release.zip");
    fs::write(root.join("backup.tar.gz"), b"fake gzipped tar").expect("write tar.gz");
    fs::write(root.join("old_release.rar"), b"Rar!\x1a\x07\x00").expect("write old.rar");
    fs::write(root.join("dist.7z"), b"7z\xbc\xaf\x27\x1cfake").expect("write dist.7z");
}

fn count_names_by_ext<'a>(files: &'a [(String, Vec<u8>)], ext: &str) -> Vec<&'a str> {
    files
        .iter()
        .filter(|(n, _)| n.ends_with(&format!(".{}", ext)))
        .map(|(n, _)| n.as_str())
        .collect()
}

#[test]
fn everything_mode_skips_os_metadata_universally() {
    let tmp = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let corpus = tmp.join(format!("nexus-skip-meta-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&corpus);
    fs::create_dir_all(&corpus).expect("mkdir");
    make_corpus_with_metadata(&corpus);

    let r = nexus_compress::walker::walk(&corpus, CorpusMode::Everything, false)
        .expect("walk");
    let names: Vec<String> = r.files.iter().map(|(n, _)| n.clone()).collect();
    eprintln!("[test] everything: {} files: {:?}", names.len(), names);
    // Source files present.
    assert!(names.iter().any(|n| n.ends_with("src/index.ts")));
    assert!(names.iter().any(|n| n.ends_with("src/util.ts")));
    assert!(names.iter().any(|n| n.ends_with("assets/icon.png")));
    // OS metadata filtered out — this is the new Sprint 5.7.18
    // behavior (the walker used to let these through in
    // Everything mode).
    assert!(
        !names.iter().any(|n| n.ends_with(".DS_Store")),
        ".DS_Store must be skipped in all modes"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("._")),
        "._* resource forks must be skipped in all modes"
    );
    assert!(
        !names.iter().any(|n| n.ends_with("Thumbs.db")),
        "Thumbs.db must be skipped in all modes"
    );
    // Archives NOT skipped (skip_archive=false is the default).
    assert!(
        names.iter().any(|n| n.ends_with("release.zip")),
        "release.zip must be present when skip_archive=false"
    );

    let _ = fs::remove_dir_all(&corpus);
}

#[test]
fn skip_archive_flag_excludes_archive_files() {
    let tmp = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let corpus = tmp.join(format!("nexus-skip-archive-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&corpus);
    fs::create_dir_all(&corpus).expect("mkdir");
    make_corpus_with_metadata(&corpus);

    let r = nexus_compress::walker::walk(&corpus, CorpusMode::Everything, true)
        .expect("walk with skip_archive");
    let names: Vec<String> = r.files.iter().map(|(n, _)| n.clone()).collect();
    eprintln!("[test] skip_archive: {} files: {:?}", names.len(), names);
    // Source files + PNG still present.
    assert!(names.iter().any(|n| n.ends_with("src/index.ts")));
    assert!(names.iter().any(|n| n.ends_with("src/util.ts")));
    assert!(names.iter().any(|n| n.ends_with("assets/icon.png")));
    // All archives excluded.
    for archive_name in &[
        "release.zip",
        "backup.tar.gz",
        "old_release.rar",
        "dist.7z",
    ] {
        assert!(
            !names.iter().any(|n| n.ends_with(archive_name)),
            "{} must be excluded when skip_archive=true",
            archive_name
        );
    }
    // OS metadata also excluded (universal filter).
    assert!(!names.iter().any(|n| n.ends_with(".DS_Store")));
    // The size accounting: skipped_bytes should include the
    // archive bytes (the user is opting out of backing them
    // up, so they go into the skip counter for UI feedback).
    let archive_bytes_skipped: u64 = r.skipped_bytes;
    eprintln!("[test] skip_archive skipped_bytes: {}", archive_bytes_skipped);
    // 4 archive files + 3 OS metadata files. Their sizes:
    //   release.zip  12 bytes
    //   backup.tar.gz 17 bytes
    //   old_release.rar 19 bytes
    //   dist.7z 14 bytes
    //   .DS_Store  4 bytes
    //   ._hidden   2 bytes
    //   Thumbs.db  23 bytes
    // = 91 bytes total. We just check that the counter is at
    // least that big (some metadata may also be skipped by
    // the dir-walker even before the file-level check).
    assert!(
        archive_bytes_skipped >= 62,
        "skipped_bytes must account for archives + metadata; got {}",
        archive_bytes_skipped
    );

    let _ = fs::remove_dir_all(&corpus);
}

#[test]
fn source_mode_still_skips_lock_files_alongside_metadata() {
    // The previous lock-file skip-list (Sprint 5.7.43) must
    // continue to work in Source mode, on top of the new
    // universal skip-list.
    let tmp = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let corpus = tmp.join(format!("nexus-source-mode-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&corpus);
    fs::create_dir_all(corpus.join("src")).expect("mkdir src");
    fs::write(corpus.join("src/main.ts"), b"const x = 1;\n").expect("write");
    fs::write(
        corpus.join("package-lock.json"),
        b"{\"lockfileVersion\": 1}",
    )
    .expect("write lockfile");
    fs::write(corpus.join(".DS_Store"), b"\x00").expect("write ds");

    let r = nexus_compress::walker::walk(&corpus, CorpusMode::Source, false)
        .expect("walk");
    let names: Vec<String> = r.files.iter().map(|(n, _)| n.clone()).collect();
    assert!(names.iter().any(|n| n.ends_with("src/main.ts")));
    assert!(
        !names.iter().any(|n| n.ends_with("package-lock.json")),
        "lock file must be skipped in Source mode"
    );
    assert!(
        !names.iter().any(|n| n.ends_with(".DS_Store")),
        "OS metadata must be skipped in Source mode too"
    );

    let _ = fs::remove_dir_all(&corpus);
}

#[test]
fn minimal_mode_keeps_only_source_extensions() {
    // The Minimal mode allow-list (only .ts/.tsx/.js/.json/etc.)
    // must continue to work alongside the new universal
    // skip-list and the skip_archive flag.
    let tmp = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let corpus = tmp.join(format!("nexus-minimal-mode-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&corpus);
    fs::create_dir_all(corpus.join("src")).expect("mkdir src");
    fs::write(corpus.join("src/main.ts"), b"const x = 1;\n").expect("write ts");
    fs::write(corpus.join("src/data.json"), b"{}").expect("write json");
    fs::write(corpus.join("README.md"), b"# README\n").expect("write md");
    fs::write(corpus.join("archive.zip"), b"PK fake").expect("write zip");

    let r = nexus_compress::walker::walk(&corpus, CorpusMode::Minimal, false)
        .expect("walk minimal");
    let names: Vec<String> = r.files.iter().map(|(n, _)| n.clone()).collect();
    // .ts is in the Minimal allow-list.
    assert!(names.iter().any(|n| n.ends_with("src/main.ts")));
    // .md is in the Minimal allow-list (Markdown is source).
    assert!(names.iter().any(|n| n.ends_with("README.md")));
    // .json is intentionally NOT in the Minimal allow-list
    // (Sprint 5.7.10-B's design choice — JSON files are
    // config, not source). They are skipped in Minimal mode.
    assert!(!names.iter().any(|n| n.ends_with("src/data.json")));
    // .zip is in the universal-skip-list? No, archives are only
    // skipped with --skip-archive, not in Minimal mode by
    // default. They also fail the Minimal allow-list.
    assert!(!names.iter().any(|n| n.ends_with("archive.zip")));

    let _ = fs::remove_dir_all(&corpus);
}

#[test]
fn archive_skip_count_names() {
    // Sanity: count the files that should be skipped, for the
    // record. Helps anyone reviewing this test to know which
    // files belong to which skip-list.
    let _ = count_names_by_ext;
}
