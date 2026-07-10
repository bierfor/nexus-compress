// Regression test for the elapsed_ms=0 bug observed on real
// directory compressions (5 GB / 163k files): every progress
// event showed elapsed_ms=0 in the JSON wire format, except the
// very first one (api.rs compress_target pre-event). The bug was
// that the inner per-chunk closure captured `total_start` by
// value, and `with_estimates` set elapsed_ms from THAT copy — but
// due to a clippy-style lint or capture-mode edge case, the
// closure got a captured value that was effectively `Instant::now()`
// at call time, not at function entry.
//
// This test simulates the EXACT pattern from nxar.rs:
//   let total_start = Instant::now();
//   for (idx, abs) in files.iter().enumerate() {
//       // ... read file ...
//       let compressed = codec::compress_with_progress(&bytes, |cumulative| {
//           progress(ProgressEvent { ... }.with_estimates(total_start));
//       });
//   }
use std::time::Instant;
use nexus_compress::codec;
use nexus_compress::api::ProgressEvent;

#[test]
fn directory_progress_events_have_real_elapsed_ms() {
    // Simulate a small directory with 10 files of 4 KiB each.
    let files: Vec<Vec<u8>> = (0..10).map(|i| {
        let mut v = vec![0u8; 4096];
        v[0] = i as u8;
        v
    }).collect();

    let total_start = Instant::now();
    let mut events: Vec<(u64, u64, u64)> = Vec::new(); // (idx, cumulative_in_file, elapsed_ms)
    let mut total_original: u64 = 0;

    for (idx, bytes) in files.iter().enumerate() {
        // Per-file pre-event (simulates nxar.rs:182)
        let pre_ev = ProgressEvent {
            phase: "compressing".to_string(),
            current_file: format!("file_{}.bin", idx),
            files_done: idx as u64,
            files_total: files.len() as u64,
            bytes_done: total_original,
            bytes_total: files.iter().map(|f| f.len() as u64).sum(),
            elapsed_ms: 0,
            bytes_per_sec: 0.0,
            eta_ms: 0,
        }.with_estimates(total_start);

        // Per-chunk events (simulates nxar.rs:200)
        let compressed = codec::compress_with_progress(bytes, |cumulative| {
            let ev = ProgressEvent {
                phase: "compressing".to_string(),
                current_file: format!("file_{}.bin", idx),
                files_done: idx as u64,
                files_total: files.len() as u64,
                bytes_done: total_original + cumulative,
                bytes_total: files.iter().map(|f| f.len() as u64).sum(),
                elapsed_ms: 0,
                bytes_per_sec: 0.0,
                eta_ms: 0,
            }.with_estimates(total_start);
            events.push((idx as u64, cumulative, ev.elapsed_ms));
        });

        // Track for assertions
        events.push((idx as u64, u64::MAX, pre_ev.elapsed_ms));
        total_original += bytes.len() as u64;
    }

    println!("=== Directory progress simulation ===");
    println!("Total events: {}", events.len());
    println!("First 10 events (idx, cumulative, elapsed_ms):");
    for e in events.iter().take(10) {
        println!("  {:?}", e);
    }

    // Every event's elapsed_ms should reflect total_start.elapsed() at
    // construction time. For 10 files of 4 KiB each, the codec runs
    // in microseconds, but total_start.elapsed() should be > 0 even
    // after the first event (loop overhead alone takes > 1ms on macOS).
    let max_elapsed = events.iter().map(|(_, _, e)| *e).max().unwrap_or(0);
    let nonzero_count = events.iter().filter(|(_, _, e)| *e > 0).count();
    println!("Max elapsed_ms observed: {}", max_elapsed);
    println!("Events with elapsed_ms > 0: {} / {}", nonzero_count, events.len());
    assert!(max_elapsed > 0,
        "at least one event should have elapsed_ms > 0, got max={}", max_elapsed);
    // The final pre-event should have a non-trivial elapsed_ms because
    // the codec processed all 10 files before it.
    let last = events.last().unwrap();
    assert!(last.2 > 0,
        "final pre-event elapsed_ms should be > 0, got {} (this is the bug pattern)",
        last.2);
}