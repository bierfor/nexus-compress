//! Like v6solid_full_path_with_progress but on a 219 MB directory
//! (100 MB text + 100 MB random + 19 MB LZMA overhead). Verifies
//! progress events fire DURING the LZMA encoding phase.
//!
//! Marked `#[ignore]` because the fixture (`/tmp/test_dir_big`,
//! 219 MB) is not self-bootstrapping and the smaller
//! `v6solid_progress_test` already covers the same invariant at
//! 32 MB. Run manually with:
//!   cargo test --test v6solid_big_dir_test -- --ignored

use std::sync::Mutex;
use std::time::Instant;

#[test]
#[ignore = "requires /tmp/test_dir_big (219 MB) fixture; use v6solid_progress_test for CI"]
fn v6solid_big_dir_with_progress() {
    use nexus_compress::api::ProgressEvent;
    use nexus_compress::supreme_engine::{
        CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
        PROFILE_SCHEMA_VERSION,
    };

    let test_dir = std::path::PathBuf::from("/tmp/test_dir_big");
    if !test_dir.exists() {
        panic!("test dir /tmp/test_dir_big missing");
    }
    let events: Mutex<Vec<(u64, u64, u64, String)>> = Mutex::new(Vec::new());
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
            let wall_ms = start.elapsed().as_millis() as u64;
            events.lock().unwrap().push((
                wall_ms,
                ev.elapsed_ms,
                ev.bytes_done,
                ev.phase.clone(),
            ));
        },
    );
    let total_ms = start.elapsed().as_millis() as u64;

    let events = events.into_inner().unwrap();
    println!("\n=== V6Solid BIG E2E test (219 MB dir) ===");
    println!("Total wall time: {}ms", total_ms);
    println!("Total events: {}", events.len());

    // Print every event so we can verify the LZMA phase progress
    for (i, e) in events.iter().enumerate() {
        println!("  #{:3}  wall={:5}ms  elapsed={:5}ms  bytes_done={:>10}  phase={}",
            i, e.0, e.1, e.2, e.3);
    }

    let result = result.expect("compress should succeed");
    let archive = result.compressed_bytes.clone();
    println!("\nOutput archive: {} bytes ({:.1} KB)", archive.len(), archive.len() as f64 / 1024.0);
    println!("Original size: {} bytes ({:.1} KB)", result.original_size, result.original_size as f64 / 1024.0);
    println!("Aggregate ratio: {:.2}x", result.ratio);

    // All events should have elapsed_ms > 0
    let zero_count = events.iter().filter(|e| e.1 == 0).count();
    println!("\nEvents with elapsed_ms=0: {}", zero_count);
    assert_eq!(zero_count, 0,
        "no event should have elapsed_ms=0 (got {} events with elapsed_ms=0)", zero_count);

    // Should have many events (>= 5 per file + 10 LZMA chunks for 100 MB)
    assert!(events.len() >= 20,
        "expected >= 20 progress events for 219 MB dir, got {}", events.len());

    // Last event should be near the wall time
    let last_wall = events.last().unwrap().0;
    assert!((total_ms as i64 - last_wall as i64).abs() < 5000,
        "last event wall={}ms should be near total_ms={}ms", last_wall, total_ms);
}
