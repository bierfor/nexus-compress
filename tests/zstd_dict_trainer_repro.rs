//! Sprint 5.7.12: dict trainer empirical verification.
//!
//! The Sprint 5.7.2 memory snapshot flagged "`train_from_buffer`
//! error -40 (afecta path zstd explícito, no el default LZMA)".
//! That bug was a phantom: the dict trainer has been hardened
//! in two waves:
//!
//! - Sprint 5.7.2 hotfix #42: prefer high-level API
//!   `zstd::dict::from_samples` and fall back to the low-level
//!   `train_from_buffer` only on failure.
//! - Sprint 5.7.4 hotfix #51: sliding window of 64 KiB with
//!   50% overlap, requiring `train_sizes.len() >= 2` before
//!   calling the trainer (the old "1 sample per super-chunk"
//!   approach triggered `ZSTD_error_srcSize_wrong` (-72)).
//!
//! In addition, Sprint 5.7.9 made the dict trainer **skip**
//! entirely in Lossy mode (the default for the SupremeEngine),
//! so the trainer only runs when `force_raw=true` (CLI
//! `--lossless`, or preprocessor overrides that force the
//! full-control entry point).
//!
//! The empirical observation in this sprint:
//!   * The trainer executes (the high-level API is called with
//!     all the samples).
//!   * The trainer returns a `Vec<u8>` that is sometimes
//!     empty (the API decides no useful dict can be learned).
//!   * An empty dict is NOT an error — it falls through to a
//!     VERSION=3 wire format and a normal roundtrip.
//!   * There is NO error -40 or error -72 reproduced on the
//!     corpora we tested (small repetitive, medium repetitive,
//!     many-small-files, low-entropy).
//!
//! This file pins the contract: the lossless path with explicit
//! zstd codec must always succeed end-to-end, regardless of
//! whether the dict trainer returns bytes or an empty Vec.

use nexus_compress::solid_archive::{
    compress_with_progress_lossless, decompress, parse_toc, CompressionLevel,
};

fn make_repetitive_corpus(total_kb: usize) -> Vec<(String, Vec<u8>)> {
    // Highly compressible (same phrase repeated). This is
    // the case where a trained dict helps the most.
    let phrase = b"The quick brown fox jumps over the lazy dog. \
                   Pack my box with five dozen liquor jugs. \
                   How vexingly quick daft zebras jump!       \
                   Sphinx of black quartz, judge my vow.      \n";
    let mut buf = Vec::with_capacity(total_kb * 1024);
    while buf.len() < total_kb * 1024 {
        buf.extend_from_slice(phrase);
    }
    buf.truncate(total_kb * 1024);
    vec![("corpus.txt".to_string(), buf)]
}

fn assert_lossless_roundtrip(files: &[(String, Vec<u8>)], archive: &[u8], label: &str) {
    // The VERSION may be 3 (no dict) or 4 (dict trained) —
    // both are valid lossless wire formats. We don't pin a
    // specific version because the trainer may decide the
    // corpus doesn't benefit from a dict (returns 0 bytes).
    let version = archive[5];
    assert!(
        matches!(version, 3 | 4),
        "[{label}] version must be 3 or 4; got {version}"
    );
    let parsed = parse_toc(archive).expect("parse_toc");
    eprintln!(
        "[{label}] version={version}, dict={} KB, archive={} B",
        parsed.dict.len() / 1024,
        archive.len()
    );

    // Roundtrip must be bit-exact (lossless + force_raw).
    let (entries, solid) = decompress(archive).expect("decompress");
    assert_eq!(
        entries.len(),
        files.len(),
        "[{label}] entry count mismatch"
    );
    for e in &entries {
        let start = e.solid_offset as usize;
        let end = start + e.pre_size as usize;
        let recovered = &solid[start..end];
        let want = files
            .iter()
            .find(|(n, _)| n == &e.name)
            .map(|(_, b)| b.as_slice())
            .expect("file in entries");
        assert_eq!(
            recovered, want,
            "[{label}] lossless bit-exact roundtrip failed for {}",
            e.name
        );
    }
}

#[test]
fn lossless_zstd_200kb_repetitive_corpus() {
    let files = make_repetitive_corpus(200);
    let archive = compress_with_progress_lossless(
        &files,
        CompressionLevel::Zstd(3),
        |_, _, _| {},
    )
    .expect("lossless 200KB compress should NOT fail");
    assert_lossless_roundtrip(&files, &archive, "200KB repetitive");
}

#[test]
fn lossless_zstd_2mb_repetitive_corpus() {
    let files = make_repetitive_corpus(2 * 1024);
    let archive = compress_with_progress_lossless(
        &files,
        CompressionLevel::Zstd(3),
        |_, _, _| {},
    )
    .expect("lossless 2MB compress should NOT fail");
    assert_lossless_roundtrip(&files, &archive, "2MB repetitive");
}

#[test]
fn lossless_zstd_many_small_files() {
    // 50 files of ~8 KB each = ~400 KB total. Each file is
    // big enough to be its own sample (≥64 KiB threshold is
    // for the SAMPLE_SIZE window, not the file size). The
    // sliding window collects many samples → trainer has
    // the diversity it needs.
    let mut files = Vec::new();
    for i in 0..50 {
        // Build ~8 KB per file by repeating the line.
        let line = format!(
            "src/file_{i:03}.ts: import {{ Foo, Bar }} from \"./common\";\n\
             export const x_{i}: number = {i};\n\
             export function helper_{i}(a: number, b: number): number {{ return a + b; }}\n"
        );
        let mut body = String::with_capacity(8 * 1024);
        while body.len() < 8 * 1024 {
            body.push_str(&line);
        }
        files.push((format!("src/file_{i:03}.ts"), body.into_bytes()));
    }
    let total_bytes: usize = files.iter().map(|(_, b)| b.len()).sum();
    assert!(
        total_bytes > 64 * 1024,
        "test setup: total corpus must exceed 64 KiB to trigger dict training; got {} bytes",
        total_bytes
    );
    eprintln!(
        "[setup] 50 files, total {} bytes ({} KiB)",
        total_bytes,
        total_bytes / 1024
    );
    let archive = compress_with_progress_lossless(
        &files,
        CompressionLevel::Zstd(3),
        |_, _, _| {},
    )
    .expect("lossless many-small-files compress should NOT fail");
    assert_lossless_roundtrip(&files, &archive, "50x8KB files");
}

#[test]
fn lossless_zstd_low_entropy_corpus() {
    // Low-entropy 1 MB corpus (header + zeros). Stresses the
    // trainer because there's little repetition to learn
    // from. The trainer may return an empty dict; we don't
    // care — roundtrip must still work.
    let mut buf = Vec::with_capacity(1024 * 1024);
    let header = b"HEADER\x00\x00\x00\x00";
    for _ in 0..(1024 * 1024 / header.len()) {
        buf.extend_from_slice(header);
    }
    let files = vec![("low_entropy.bin".to_string(), buf)];
    let archive = compress_with_progress_lossless(
        &files,
        CompressionLevel::Zstd(3),
        |_, _, _| {},
    )
    .expect("lossless low-entropy compress should NOT fail");
    assert_lossless_roundtrip(&files, &archive, "low-entropy 1MB");
}
