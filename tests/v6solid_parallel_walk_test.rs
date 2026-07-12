//! Sprint 5.7.2 hotfix #29: parallel file walk for V6Solid. The
//! previous implementation read files sequentially which made
//! the UI freeze for up to a minute on large directories. This
//! test creates a moderately large directory and verifies:
//!  - The walk completes (no hang)
//!  - All files are present in the result
//!  - The first progress event fires IMMEDIATELY (not after walk)

use std::sync::Mutex;
use std::time::Instant;

#[test]
fn v6solid_parallel_walk_completes() {
    use nexus_compress::api::ProgressEvent;
    use nexus_compress::supreme_engine::{
        CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
        PROFILE_SCHEMA_VERSION,
    };

    // The /tmp/test_dir_big has 6 files (5 × 20 MiB text + 1 × 100 MiB random)
    let test_dir = std::path::PathBuf::from("/tmp/test_dir_big");
    if !test_dir.exists() {
        eprintln!("skipping: /tmp/test_dir_big doesn't exist (run setup first)");
        return;
    }

    let events: Mutex<Vec<(u64, u64, u64)>> = Mutex::new(Vec::new());
    let start = Instant::now();

    // Sprint 5.7.10-E: drive the engine via a profile. The
    // previous test called `compress_directory_with_backend_
    // with_progress(V6Solid)` directly — that API is gone.
    let profile = CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode: ProfileMode::Ultra,
        codec: ProfileCodec::Lzma,
        fidelity: ProfileFidelity::Lossy,
        corpus_mode: nexus_compress::api::CorpusMode::Everything,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
        skip_archive: false,
    };
    let invocation = CompressInvocation {
        profile,
        path: test_dir.clone(),
        password: None,
        output_dir: None,
    };
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(
        &invocation,
        |ev: ProgressEvent| {
            let wall = start.elapsed().as_millis() as u64;
            events.lock().unwrap().push((wall, ev.elapsed_ms, ev.bytes_done));
        },
    );
    let total_ms = start.elapsed().as_millis() as u64;

    let events = events.into_inner().unwrap();
    println!("\n=== V6Solid parallel walk test ===");
    println!("Total wall: {}ms", total_ms);
    println!("Events: {}", events.len());
    println!("First event (wall, elapsed, bytes_done): {:?}", events.first());
    println!("Last event (wall, elapsed, bytes_done): {:?}", events.last());

    let result = result.expect("compress should succeed");
    let _archive = result.compressed_bytes.clone();
    assert_eq!(result.n_files, 6, "expected 6 files in archive");

    // The first event should fire quickly (well under 1 second)
    // because of the immediate "starting" event we emit at
    // compress_target entry.
    let first_wall = events.first().unwrap().0;
    println!("First event wall: {}ms", first_wall);
    assert!(first_wall < 1000,
        "first event should fire within 1s, got {}ms", first_wall);
}
