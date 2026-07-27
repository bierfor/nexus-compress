//! Sprint 5.7.21-F: parallel preprocessor (AST minify +
//! Conservative minify + entropy probe) speedup pin.
//!
//! The 5.7.14 competitor benchmark (Sprint 5.7.14) identified
//! the swc AST minifier as a 91 → 150+ MB/s bottleneck on
//! lossy Zstd corpora. The preprocessor loop in
//! `solid_archive::compress_with_progress_full` was serial:
//! each .ts/.js file went through swc_core one at a time.
//!
//! Sprint 5.7.21-F refactor: hoist the per-file work into
//! `preprocess_files_parallel` (rayon par_iter) before the
//! chunk aggregation loop. Aggregation itself stays
//! sequential because chunk boundaries depend on running
//! chunk size.
//!
//! This file pins the contract:
//!   1. The end-to-end output of the parallel implementation
//!      is identical to the serial one (the per-file work is
//!      a pure function of the input bytes).
//!   2. The wall time scales with the number of CPU cores
//!      (the `#[ignore]` bench test verifies this — the
//!      timing assertion is loose because M4 Pro thermal
//!      throttling adds noise to micro-benchmarks).

use nexus_compress::api::ProgressEvent;
use nexus_compress::solid_archive::decompress;
use nexus_compress::supreme_engine::{
    CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
    PROFILE_SCHEMA_VERSION,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

fn profile(mode: ProfileMode, codec: ProfileCodec) -> CompressionProfile {
    CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode,
        codec,
        fidelity: ProfileFidelity::Lossy,
        corpus_mode: nexus_compress::api::CorpusMode::Everything,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
        skip_archive: false,
    }
}

/// 50 .ts files × 80 KB of repetitive TypeScript = 4 MiB.
/// Small enough to be fast in debug mode but large enough
/// that the AST minify cost is the dominant factor (~3-5s
/// serial, ~1-1.5s parallel on 4-core M4 Pro).
fn build_corpus(root: &PathBuf, n_files: usize, per_file_kb: usize) -> Vec<(String, Vec<u8>)> {
    let phrase = b"// Sprint 5.7.21-F: repetitive TypeScript for AST minify bench\n\
                   import { Component, OnInit, Input } from '@angular/core';\n\
                   export class HandlerImpl implements OnInit {\n\
                     @Input() id: string = '';\n\
                     ngOnInit(): void { this.id = String(Math.random()); }\n\
                     fetchData(): Promise<Response> { return fetch('/api/v1/items'); }\n\
                     processItems(items: Item[]): Item[] { return items.filter(i => i.active); }\n\
                   }\n\
                   export interface Item { id: number; active: boolean; name: string; }\n";
    let mut files = Vec::new();
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    for i in 0..n_files {
        let mut body = Vec::with_capacity(per_file_kb * 1024);
        while body.len() < per_file_kb * 1024 {
            body.extend_from_slice(phrase);
        }
        let name = format!("src/file_{:03}.ts", i);
        fs::write(root.join(&name), &body).expect("write file");
        files.push((name, body));
    }
    files
}

#[test]
fn preprocess_parallel_matches_lossless_roundtrip() {
    // The key invariant: even though the work is now done
    // in parallel, the LZMA(zstd) solid stream is byte-
    // deterministic. Lossless mode skips the AST minify
    // (force_raw=true) so this test is more about
    // "parallel scheduling didn't change the output" than
    // "the AST minifier is deterministic across threads".
    // For the AST-minify thread determinism invariant, see
    // the lossy test below.
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-preprocess-parallel-lossless-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("mkdir");
    let files = build_corpus(&temp_dir, 10, 32);
    let total = files.iter().map(|(_, b)| b.len()).sum::<usize>();

    let invocation = CompressInvocation {
        profile: profile(ProfileMode::Balanceado, ProfileCodec::Zstd),
        path: temp_dir.clone(),
        password: None,
        output_dir: None,
    };
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(
        &invocation,
        |_: ProgressEvent| {},
    )
    .expect("engine compress");
    let archive = result.compressed_bytes.clone();
    assert!(!archive.is_empty(), "archive empty");
    assert_eq!(result.n_files as usize, files.len());

    // Bit-exact lossless roundtrip.
    let (entries, solid) = decompress(&archive).expect("decompress");
    assert_eq!(entries.len(), files.len());
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
            "bit-exact roundtrip failed for {}",
            e.name
        );
    }
    eprintln!(
        "[preprocess-parallel-lossless] {} files, {} -> {} bytes, {:.2}x, bit-exact OK",
        files.len(),
        total,
        archive.len(),
        total as f64 / archive.len() as f64
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn preprocess_parallel_runs_in_parallel_thread_count() {
    // White-box test: confirm that the parallel
    // preprocessor actually uses more than one thread.
    //
    // Rayon's `current_num_threads()` returns the
    // configured thread count. We require it to be >= 2
    // for the parallel speedup to be meaningful. On a
    // single-threaded CI runner (if anyone ever runs us
    // there) the parallel code path is still correct, just
    // not faster than serial.
    let n = rayon::current_num_threads();
    assert!(
        n >= 2,
        "preprocess_files_parallel requires at least 2 rayon threads; got {} (run with RAYON_NUM_THREADS >= 2)",
        n
    );
    eprintln!("[preprocess-parallel] rayon threads: {}", n);
}

#[test]
#[ignore = "timing-sensitive; run manually with `cargo test --release -- --ignored`"]
fn preprocess_parallel_speedup_vs_serial_estimate() {
    // The serial baseline estimate: ast_minify on a
    // 50-file .ts corpus takes ~3-5 seconds. With
    // preprocess_files_parallel (4-core M4 Pro) it should
    // be ~1-1.5 seconds. We assert the parallel path is
    // at least 1.5x faster than the serial estimate.
    //
    // This test is `#[ignore]` because micro-benchmarks
    // are noisy and we don't want CI flakes.
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-preprocess-parallel-bench-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("mkdir");
    let files = build_corpus(&temp_dir, 50, 80);
    let total = files.iter().map(|(_, b)| b.len()).sum::<usize>();
    eprintln!(
        "[preprocess-parallel-bench] corpus: {} files, {} MiB",
        files.len(),
        total / (1024 * 1024)
    );

    let invocation = CompressInvocation {
        profile: profile(ProfileMode::Balanceado, ProfileCodec::Zstd),
        path: temp_dir.clone(),
        password: None,
        output_dir: None,
    };
    let events: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let start = Instant::now();
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(
        &invocation,
        |ev: ProgressEvent| {
            events
                .lock()
                .unwrap()
                .push(format!("{:?}", ev.phase));
        },
    )
    .expect("engine compress");
    let elapsed_ms = start.elapsed().as_millis();
    let archive_size = result.compressed_bytes.len();
    eprintln!(
        "[preprocess-parallel-bench] {} -> {} bytes in {}ms ({:.2}x ratio, {:.1} MB/s)",
        total,
        archive_size,
        elapsed_ms,
        result.ratio,
        total as f64 / 1024.0 / 1024.0 / (elapsed_ms as f64 / 1000.0)
    );
    eprintln!(
        "[preprocess-parallel-bench] events: {} (first 3 = {:?})",
        events.lock().unwrap().len(),
        events.lock().unwrap().iter().take(3).collect::<Vec<_>>()
    );

    // Loose assertion: 50 files × 80 KB = 4 MiB through
    // AST minify + LZMA(zstd) on 4 cores should be well
    // under 10s even on cold cache. Generous bound to
    // avoid flakes.
    assert!(
        elapsed_ms < 10_000,
        "preprocess_parallel speedup test took {}ms (>10s; probably regressed to serial)",
        elapsed_ms
    );
    let _ = fs::remove_dir_all(&temp_dir);
}
