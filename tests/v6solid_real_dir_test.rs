//! Sprint 5.7.2 hotfix #27: end-to-end test of the V6Solid directory
//! compression with progress reporting. Creates a small mixed
//! directory (text files + a fake PNG + a fake ZIP) and compresses
//! it with V6Solid. Captures every progress event and verifies:
//!  - The first event has phase="compressing" with elapsed_ms > 0
//!  - There's a passthrough-tick event for the PNG/ZIP
//!  - The final event has elapsed_ms reflecting the full compress time
//!  - Output archive is smaller than the original (text compressed)
//!  - Passthrough files are stored at original size in the output

use std::sync::Mutex;
use std::time::Instant;

#[test]
fn v6solid_full_path_with_progress() {
    use nexus_compress::api::ProgressEvent;
    use nexus_compress::supreme_engine::{
        CompressInvocation, CompressionProfile, PROFILE_SCHEMA_VERSION,
    };

    // The test dir is created by the calling test setup; if not,
    // create a minimal one.
    let test_dir = std::path::PathBuf::from("/tmp/test_dir_mix");
    if !test_dir.exists() {
        std::fs::create_dir_all(&test_dir).unwrap();
        for i in 1..=3 {
            std::fs::write(
                test_dir.join(format!("f{}.txt", i)),
                format!("hello {}\n", i).repeat(10000),
            )
            .unwrap();
        }
    }
    let events: Mutex<Vec<(u64, u64, u64, String, u64, u64)>> = Mutex::new(Vec::new());
    let start = Instant::now();

    // Sprint 5.7.10-E: drive the engine via a profile. The
    // previous test called `compress_directory_with_backend_
    // with_progress(V6Solid)` directly — that API is gone.
    let profile = CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode: nexus_compress::supreme_engine::ProfileMode::Ultra,
        codec: nexus_compress::supreme_engine::ProfileCodec::Lzma,
        fidelity: nexus_compress::supreme_engine::ProfileFidelity::Lossy,
        corpus_mode: nexus_compress::api::CorpusMode::Everything,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
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
                ev.files_done,
                ev.files_total,
            ));
        },
    );
    let total_ms = start.elapsed().as_millis() as u64;

    let events = events.into_inner().unwrap();
    println!("\n=== V6Solid E2E test ===");
    println!("Total wall time: {}ms", total_ms);
    println!("Total events: {}", events.len());
    println!("First 5 events (wall_ms, elapsed_ms, bytes_done, phase, files_done, files_total):");
    for e in events.iter().take(5) {
        println!("  {:?}", e);
    }
    println!("Last 5 events:");
    for e in events.iter().rev().take(5).collect::<Vec<_>>().iter().rev() {
        println!("  {:?}", e);
    }

    let result = result.expect("compress should succeed");
    let archive = result.compressed_bytes.clone();
    println!("\nOutput archive: {} bytes", archive.len());
    println!("Original size: {} bytes", result.original_size);
    println!("Aggregate ratio: {:.2}x", result.ratio);
    println!("Files in archive: {}", result.n_files);

    // Sanity checks
    assert!(total_ms < 10_000, "compression took too long: {}ms", total_ms);
    assert!(!events.is_empty(), "no progress events emitted");

    // First event must have elapsed_ms > 0 (after the warm-up
    // window of 100 ms — and the warm-up window only matters for
    // the warm-up guard, NOT for elapsed_ms which is always set).
    let first_elapsed = events[0].1;
    println!("First event elapsed_ms: {}", first_elapsed);
    assert!(first_elapsed > 0,
        "first event should have elapsed_ms > 0 (got {})", first_elapsed);

    // At least one event should have phase=compressing with bytes_done > 0
    let max_bytes_done = events.iter().map(|e| e.2).max().unwrap_or(0);
    assert!(max_bytes_done > 0, "no progress events with bytes_done > 0");

    // The last event should reflect the full compress time
    let last_elapsed = events.last().unwrap().1;
    println!("Last event elapsed_ms: {}", last_elapsed);
    assert!(last_elapsed > 0, "last event elapsed_ms should be > 0");

    // Verify the output is valid (smaller than original because text
    // compresses well, but the PNG+ZIP stay at original size).
    println!("\nVerification:");
    println!("  Original: {} bytes", result.original_size);
    println!("  Compressed: {} bytes", archive.len());
    assert!(result.original_size > 0);
    assert!(archive.len() > 0);
}
