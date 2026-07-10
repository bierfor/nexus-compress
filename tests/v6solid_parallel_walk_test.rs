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
    use nexus_compress::api::{self, CompressionBackend, ProgressEvent};

    // The /tmp/test_dir_big has 6 files (5 × 20 MiB text + 1 × 100 MiB random)
    let test_dir = std::path::PathBuf::from("/tmp/test_dir_big");
    if !test_dir.exists() {
        eprintln!("skipping: /tmp/test_dir_big doesn't exist (run setup first)");
        return;
    }

    let events: Mutex<Vec<(u64, u64, u64)>> = Mutex::new(Vec::new());
    let start = Instant::now();

    let result = api::compress_directory_with_backend_with_progress(
        &test_dir,
        CompressionBackend::V6Solid,
        1,
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

    let (result, archive) = result.expect("compress should succeed");
    assert_eq!(result.n_files, 6, "expected 6 files in archive");

    // The first event should fire quickly (well under 1 second)
    // because of the immediate "starting" event we emit at
    // compress_target entry.
    let first_wall = events.first().unwrap().0;
    println!("First event wall: {}ms", first_wall);
    assert!(first_wall < 1000,
        "first event should fire within 1s, got {}ms", first_wall);
}
