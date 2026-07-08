//! Top-level codec — pipeline orchestrator.
//!
//! ## v2 pipeline (Format v2 — current)
//! 1. Split input via CDC (Gear hash, min=4K avg=32K max=64K).
//! 2. For each chunk:
//!    a. Compute FNV-1a hash, check dedup table.
//!    b. If duplicate: emit `BlockType::Duplicate` referencing original.
//!    c. If unique: classify, then LZ77 with optimal-parsing DP (v2 cost
//!       model: literal=13 bits, match=32 bits → optimal wins on L>=3).
//!    d. rANS-encode the literal bytes.
//!    e. Write op stream: 1 byte flag per literal (no placeholder),
//!       4 bytes per match (u16 dist + u8 len).
//!    f. If compressed payload >= raw block size: fall back to `Raw`.
//! 3. Emit header with version=3.
//!
//! ## Decoder
//! Reads header version. Handles v0, v2, and v3 compressed payloads.
//! Maintains `Vec<Vec<u8>>` cache for Duplicate lookups.
//!
//! ## v3 multi-stream rANS (the current default)
//!
//! Instead of one shared rANS stream for everything, v3 splits the
//! compressed payload into three independent rANS streams with their
//! own frequency tables:
//!
//! ## Sprint 5.7.1 — parallel compression (PR #2)
//!
//! PR #2 adds `compress_parallel` and a `compress_auto` heuristic
//! that picks sequential vs parallel based on input size, thread
//! count, and whether the shared-dict will pay off. See
//! [`compress_auto`] for the exact heuristic and [`parallel`] for
//! the implementation.
//!
//! The parallel encoder uses the SAME wire format as the sequential
//! one — a single `NexusHeader` followed by concatenated CDC chunks.
//! The decoder does NOT need to know which encoder produced the
//! output, so PR #2 is format-compatible. PR #3 (or later) will add
//! the dict-chained format that lets the decoder apply block 0's
//! trailing bytes as block N's preset dict; that's a v2 format
//! bump.

use crate::format::{NexusHeader, PARALLEL_MAGIC};
use std::time::Instant;

pub mod parallel;
pub mod sequential;
// shared_dict is reserved for the LZMA-based path (engine.rs,
// solid_archive.rs). The current custom LZ77+rANS pipeline in
// `sequential.rs` doesn't have a preset_dict equivalent, so
// enabling it here would just bloat the binary. Kept on disk
// for future use; not wired into `compress_parallel` yet.
#[allow(dead_code)]
pub mod shared_dict;

pub use parallel::compress_parallel;
pub use sequential::{compress, compress_premium, decompress};

// ─────────────────────────────────────────────────────────────
//  compress_auto — heuristic that picks sequential vs parallel.
// ─────────────────────────────────────────────────────────────

/// Threshold (in bytes) above which `compress_auto` will consider
/// using the parallel pipeline. Below this the rayon task spawn
/// overhead exceeds the parallel speedup, so sequential wins.
///
/// Set to **64 MiB** (2 super-blocks at 32 MiB each) because:
/// 1. One super-block has no cross-block work to parallelize.
/// 2. The shared-dict benefit only kicks in at 2+ blocks.
/// 3. The sequential pipeline already saturates one core; below
///    this threshold the per-block work fits comfortably in the
///    user's "I clicked the button" perception window.
pub const PARALLEL_INPUT_THRESHOLD: usize = 64 * 1024 * 1024;

/// Pick the best pipeline for `input` based on size + thread
/// count + shared-dict benefit.
///
/// Heuristic (in order):
/// 1. **size**: must be ≥ `PARALLEL_INPUT_THRESHOLD` (64 MiB).
///    Below that, the rayon task spawn + sync cost dominates.
/// 2. **threads**: must be ≥ 2 (we have at least one other worker
///    to share the work with). Single-core machines always go
///    sequential.
/// 3. **shared-dict benefit**: if the input splits into a single
///    super-block, the parallel encoder runs the same code path
///    as sequential — no benefit, just overhead. We skip parallel
///    in that case even if (1) and (2) are met. (This is what the
///    user flagged as the "reset the shared dict between threads"
///    concern — see PR #2 design notes.)
///
/// `num_threads_hint` lets the caller override the actual thread
/// count (e.g. the engine passes `rayon::current_num_threads()`,
/// tests can pass a fixed value to test the boundary conditions).
pub fn compress_auto_with_threads(input: &[u8], num_threads_hint: usize) -> Vec<u8> {
    use rayon::current_num_threads;
    let n_threads = if num_threads_hint == 0 {
        current_num_threads()
    } else {
        num_threads_hint
    };
    let n_blocks = parallel::num_super_blocks(input);
    if input.len() >= PARALLEL_INPUT_THRESHOLD && n_threads >= 2 && n_blocks >= 2 {
        let t0 = Instant::now();
        let out = compress_parallel(input);
        eprintln!(
            "[nexus-compress] compress_auto: parallel ({} MiB input, {} super-blocks, {} threads) in {:?}",
            input.len() / (1024 * 1024),
            n_blocks,
            n_threads,
            t0.elapsed()
        );
        out
    } else {
        if input.len() >= PARALLEL_INPUT_THRESHOLD {
            // Big enough for parallel but skipped — surface why.
            eprintln!(
                "[nexus-compress] compress_auto: sequential (parallel skipped: \
                 {} threads, {} super-blocks)",
                n_threads, n_blocks
            );
        }
        compress(input)
    }
}

/// Production entry point — uses the actual rayon thread pool.
pub fn compress_auto(input: &[u8]) -> Vec<u8> {
    compress_auto_with_threads(input, 0)
}

// Re-export the header type so downstream code (api.rs, tests) can
// use `codec::NexusHeader` instead of reaching into `format`.
pub use crate::format::NexusHeader as Header;

// ─────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_check(input: &[u8]) {
        let out = compress(input);
        let back = decompress(&out).expect("decompress sequential");
        assert_eq!(back, input, "sequential roundtrip failed for {} bytes", input.len());
    }

    fn roundtrip_check_parallel(input: &[u8]) {
        let out = compress_parallel(input);
        let back = decompress(&out).expect("decompress parallel");
        assert_eq!(back, input, "parallel roundtrip failed for {} bytes", input.len());
    }

    #[test]
    fn sequential_roundtrip_short() {
        roundtrip_check(b"hello world");
    }

    #[test]
    fn sequential_roundtrip_random() {
        let mut v = vec![0u8; 1000];
        for (i, b) in v.iter_mut().enumerate() {
            *b = (i * 7 ^ (i >> 3)) as u8;
        }
        roundtrip_check(&v);
    }

    #[test]
    fn parallel_roundtrip_repetitive_large() {
        // 128 MiB of repeating data — exercises the parallel path
        // and verifies the shared_dict extraction doesn't break
        // roundtrip.
        let unit = b"nexus-compress-sprint-5-7-1-dict-chaining-".repeat(1024);
        // unit is ~44 KiB; we need 128 MiB so repeat 3072 times.
        let mut input = Vec::with_capacity(unit.len() * 3072);
        for _ in 0..3072 {
            input.extend_from_slice(&unit);
        }
        assert!(input.len() >= PARALLEL_INPUT_THRESHOLD, "test input too small: {} < {}", input.len(), PARALLEL_INPUT_THRESHOLD);
        roundtrip_check_parallel(&input);
    }

    #[test]
    fn auto_chooses_sequential_for_small_input() {
        let input = b"small payload".to_vec();
        let out = compress_auto(&input);
        let back = decompress(&out).expect("decompress");
        assert_eq!(back, input);
    }

    #[test]
    fn auto_chooses_parallel_for_large_repetitive_input() {
        // 128 MiB of highly compressible content — the test that
        // proves `compress_auto` actually picks the parallel path
        // (single-threaded = forever, parallel = fast).
        let unit = vec![b'A'; 4096];
        let mut input = Vec::with_capacity(unit.len() * 32 * 1024);
        for _ in 0..(32 * 1024) {
            input.extend_from_slice(&unit);
        }
        assert!(input.len() >= PARALLEL_INPUT_THRESHOLD);
        let out = compress_auto_with_threads(&input, 8);
        let back = decompress(&out).expect("decompress");
        assert_eq!(back, input);
    }

    #[test]
    fn auto_falls_back_to_sequential_when_threads_is_one() {
        let unit = vec![b'B'; 4096];
        let mut input = Vec::with_capacity(unit.len() * 32 * 1024);
        for _ in 0..(32 * 1024) {
            input.extend_from_slice(&unit);
        }
        // Force num_threads=1 → must NOT go parallel.
        let out = compress_auto_with_threads(&input, 1);
        let back = decompress(&out).expect("decompress");
        assert_eq!(back, input);
    }

    #[test]
    fn auto_falls_back_to_sequential_for_single_super_block() {
        // Force num_threads=8 (so the "≥ 2 threads" check passes) but
        // make the input small enough to fit in 1 super-block. The
        // shared-dict benefit is 0, so the parallel path should
        // be skipped per the user's design note.
        let input = vec![b'C'; 16 * 1024 * 1024]; // 16 MiB = 1 super-block
        assert!(input.len() < PARALLEL_INPUT_THRESHOLD);
        let out = compress_auto_with_threads(&input, 8);
        let back = decompress(&out).expect("decompress");
        assert_eq!(back, input);
    }
}
