//! Sprint 5.7.2 hotfix #27: test that solid_archive::compress_with_progress
//! emits progress events DURING the LZMA encoding phase (not just per-file).
//! This prevents the bar from getting stuck at 100% for several minutes
//! during the LZMA encode of large solid streams.

use std::time::Instant;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use nexus_compress::solid_archive::CompressionLevel;

#[test]
fn v6solid_emits_progress_during_lzma_encoding() {
    // Build a 32 MiB solid stream — large enough that LZMA encoding
    // takes noticeable time so we can detect the progress events.
    let mut files = Vec::new();
    for i in 0..16 {
        // Each file is 2 MiB. 16 files × 2 MiB = 32 MiB.
        let mut bytes = vec![0u8; 2 * 1024 * 1024];
        for j in 0..bytes.len() {
            // Semi-random data so LZMA has work to do (no compression
            // would make the test trivially pass with 0 progress).
            bytes[j] = ((i * 31 + j * 7) % 251) as u8;
        }
        files.push((format!("file_{:02}.dat", i), bytes));
    }
    let progress_count = Arc::new(AtomicUsize::new(0));
    let progress_count_inner = progress_count.clone();
    let start = Instant::now();
    let archive = nexus_compress::solid_archive::compress_with_progress(
        &files,
        CompressionLevel::Lzma(1), // ultra-fast LZMA level for the test
        move |file_idx, total, name| {
            progress_count_inner.fetch_add(1, Ordering::Relaxed);
            // Print progress events so we can see in --nocapture
            eprintln!(
                "[V6SOLID-PROGRESS] file_idx={}/{} name={}",
                file_idx, total, name
            );
        },
    )
    .expect("compress should succeed");
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    let total_progress = progress_count.load(Ordering::Relaxed);
    eprintln!("\n[V6SOLID-TEST] compressed {} MiB in {:.0} ms", files.iter().map(|(_, b)| b.len()).sum::<usize>() / (1024*1024), elapsed);
    eprintln!("[V6SOLID-TEST] total progress events: {}", total_progress);
    eprintln!("[V6SOLID-TEST] output archive size: {} bytes", archive.len());
    // 16 files = 16 per-file events + at least 8 LZMA-chunk events
    // (32 MiB / 4 MiB chunk = 8 chunks).
    assert!(total_progress >= 16 + 8,
        "expected at least 24 progress events (16 files + 8 chunks), got {}",
        total_progress);
}
