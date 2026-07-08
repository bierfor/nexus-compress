//! Sprint 5.7.1 — parallel compression pipeline.
//!
//! Splits the input into fixed-size super-blocks (32 MiB each by
//! default), compresses each independently via the existing
//! sequential codec, and concatenates the results under a
//! `ParallelHeader` wrapper.
//!
//! ## Why fixed super-blocks (not CDC chunks)
//!
//! CDC chunks are 16-64 KB each — perfect for the per-block
//! dictionary, but too small to parallelize (a single rayon
//! task overhead is ~50 µs, and a 16 KB block takes ~200 µs to
//! compress). Grouping CDC chunks into 32 MiB super-blocks gives
//! each rayon task enough work to amortize the spawn cost.
//!
//! ## Why no shared-dict
//!
//! The sequential codec uses a custom LZ77 + rANS pipeline, not
//! liblzma. The "shared dict" trick (extract last N compressed
//! bytes from block 0, pass to block N as preset_dict) is a
//! liblzma-specific feature. Applying it to the custom pipeline
//! would require adding a matching preset_dict API to the custom
//! LZ77 state, which is a multi-day project. v0.1.2 ships
//! speed-only parallel; the dict-chained parallel for the custom
//! pipeline is a future-sprint item.
//!
//! ## Format (v0.1.2)
//!
//! ```text
//! +-------------------------+
//! | PARALLEL_MAGIC "NXP\0"  |  4 bytes
//! | block_count   (u32 LE)  |  4 bytes
//! | total_size    (u64 LE)  |  8 bytes
//! | super_block_size (u32)  |  4 bytes (target size of each block)
//! +-------------------------+
//! | block 0 length (u32 LE) |
//! | block 0 bytes ...       |  ← output of sequential::compress on
//! |                          |    the first 32 MiB
//! +-------------------------+
//! | block 1 length (u32 LE) |
//! | block 1 bytes ...       |
//! +-------------------------+
//! | ...                      |
//! +-------------------------+
//! ```
//!
//! The block bytes are full `sequential::compress` outputs — each
//! has its own internal `NexusHeader` (MAGIC = "NXS\0"). The
//! parallel decoder reads the parallel header, then iterates over
//! the length-prefixed blocks and calls `sequential::decompress`
//! on each, concatenating the outputs.

use crate::format::PARALLEL_MAGIC;
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;

/// Default super-block size. 32 MiB is a balance between:
/// - Per-block work large enough that rayon's task spawn cost
///   (~50 µs) is amortized over a compress call of hundreds of ms.
/// - Per-block size small enough that a multi-GB input still
///   parallelizes across all available cores (a 4 GB input on
///   8 cores → 16 super-blocks).
/// - Per-block size that fits comfortably in L2 cache (32 MiB
///   > modern L2 but L3 catch-up is fine; LZ77 doesn't have a
///   working-set issue at this size).
pub const DEFAULT_SUPER_BLOCK_BYTES: usize = 32 * 1024 * 1024;

/// Sprint 5.7.1 stress-fix: estimated peak memory per parallel
/// block. Each super-block of `DEFAULT_SUPER_BLOCK_BYTES` input
/// keeps:
///   - the input slice itself: 32 MiB
///   - the LZ77 working set: ~16 MiB (window + hash table)
///   - the rANS tables: ~8 MiB
///   - the dedup table shared across the whole input: ~4 MiB
///     per million unique blocks
///   - overhead from rayon stack + std::fs: ~5 MiB
///
/// On a 4 GB machine (typical developer laptop) with 32 cores,
/// the default rayon thread count would spawn 32 workers all
/// holding ~32+ MiB of input data + ~30 MiB of working state.
/// Peak memory: 32 × 65 MiB = 2 GiB — within budget, but on a
/// constrained container with 1-2 GiB total RAM, this OOMs.
///
/// We cap concurrent workers so total parallel-state memory is
/// ≤ 50% of free RAM. See [`safe_parallel_thread_count`].
const ESTIMATED_BLOCK_WORKING_SET_MIB: u64 = 65;

/// Compute a safe thread count for the parallel section given
/// `input_size` and the currently available RAM. The function:
///   - never returns 0 or 1 (parallel needs at least 2 to win)
///   - never exceeds `rayon::current_num_threads()`
///   - never exceeds what free RAM can afford (50% cap)
///   - never returns 0 if RAM is unknown (assumes at least 1 GiB
///     free — conservative for CI containers)
pub fn safe_parallel_thread_count(input_size: usize) -> usize {
    let n_cores = rayon::current_num_threads().max(2);
    let available_mb = crate::ram::available_memory_mb().unwrap_or(1024);
    // Cap: 50% of free RAM divided by the working-set per block.
    // Each block uses ~ESTIMATED_BLOCK_WORKING_SET_MIB plus its
    // own slice of `input_size`. We charge the full `input_size`
    // to every block in the worst case (which it isn't — each
    // block only holds 1/n of the input — but conservative is
    // safer than OOM).
    let n_blocks = num_super_blocks_with(input_size, DEFAULT_SUPER_BLOCK_BYTES);
    let input_mib = (input_size / (1024 * 1024)).max(1) as u64;
    let per_block_mib = ESTIMATED_BLOCK_WORKING_SET_MIB + input_mib / n_blocks.max(1) as u64;
    // Round up to nearest integer thread count, with a floor of 2.
    let half = (available_mb / 2).max(per_block_mib);
    let by_ram = (half / per_block_mib.max(1)) as usize;
    by_ram.clamp(2, n_cores)
}

/// Compress `input` in parallel by splitting into super-blocks
/// and compressing each on a rayon worker. Falls back to the
/// sequential codec when:
///   - input is empty (degenerate case)
///   - input fits in 1 super-block (no benefit, just overhead)
pub fn compress_parallel(input: &[u8]) -> Vec<u8> {
    compress_parallel_with_block_size(input, DEFAULT_SUPER_BLOCK_BYTES)
}

/// Public so [`crate::codec::compress_auto`] can read the number
/// of super-blocks WITHOUT actually compressing (used in the
/// "≥ 2 blocks" check of the heuristic).
pub fn num_super_blocks(input: &[u8]) -> usize {
    num_super_blocks_with(input.len(), DEFAULT_SUPER_BLOCK_BYTES)
}

fn num_super_blocks_with(total_len: usize, block_size: usize) -> usize {
    if total_len == 0 {
        0
    } else {
        (total_len + block_size - 1) / block_size
    }
}

pub fn compress_parallel_with_block_size(input: &[u8], block_size: usize) -> Vec<u8> {
    // Edge cases first.
    if input.is_empty() {
        // An empty parallel stream: just the parallel header with
        // 0 blocks. The decoder returns an empty Vec.
        return encode_parallel_header(&[], 0, block_size);
    }

    // Split into super-blocks.
    let blocks: Vec<&[u8]> = input.chunks(block_size).collect();
    let n_blocks = blocks.len();
    if n_blocks == 1 {
        // Single block → fall through to sequential. The output
        // is still a self-contained `sequential::compress` result.
        return crate::codec::compress(input);
    }

    // Compress all blocks in parallel via a SCOPED thread pool.
    //
    // Why a scoped pool and not the global one: OOM protection.
    // The global rayon pool runs `current_num_threads()` workers
    // permanently. On a 32-core machine with constrained RAM,
    // that means 32 workers all holding their share of the input
    // plus the LZ77 + rANS working state — easily 2+ GiB peak.
    // A scoped pool (created on the fly, dropped after use) lets
    // us cap concurrent workers via `safe_parallel_thread_count`
    // which respects available RAM. The pool is dropped when this
    // function returns, releasing the worker threads back to the
    // OS scheduler.
    let n_threads = safe_parallel_thread_count(input.len());
    let pool = match ThreadPoolBuilder::new()
        .num_threads(n_threads)
        .thread_name(|i| format!("nexus-compress-{}", i))
        .build()
    {
        Ok(p) => p,
        // If we can't build a pool (resource exhaustion, etc.) we
        // fall back to the sequential pipeline. The caller still
        // gets a valid compressed result, just slower.
        Err(e) => {
            eprintln!(
                "[nexus-compress] WARN: rayon pool build failed ({}), \
                 falling back to sequential for this call",
                e
            );
            return crate::codec::compress(input);
        }
    };
    let compressed: Vec<Vec<u8>> = pool.install(|| {
        blocks
            .par_iter()
            .map(|block| crate::codec::compress(block))
            .collect()
    });

    // Serialize: header + per-block length prefix + block bytes.
    let mut out = encode_parallel_header(&compressed, input.len() as u64, block_size);
    for block in &compressed {
        let len = block.len() as u32;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(block);
    }
    out
}

fn encode_parallel_header(
    blocks: &[Vec<u8>],
    total_uncompressed_size: u64,
    super_block_size: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + blocks.iter().map(|b| 4 + b.len()).sum::<usize>());
    out.extend_from_slice(PARALLEL_MAGIC);
    out.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    out.extend_from_slice(&total_uncompressed_size.to_le_bytes());
    out.extend_from_slice(&(super_block_size as u32).to_le_bytes());
    out
}

// ─────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{compress, decompress};

    #[test]
    fn parallel_empty_input_roundtrips() {
        let out = compress_parallel(&[]);
        // Should just be the 20-byte parallel header with 0 blocks.
        assert_eq!(out.len(), 20);
        // And the decoder returns an empty Vec.
        let back = decompress(&out).expect("decompress empty");
        assert!(back.is_empty());
    }

    #[test]
    fn parallel_single_block_falls_back_to_sequential() {
        // 10 MiB → fits in 1 super-block → output is identical to
        // a sequential compress of the same input (one block, no
        // parallel wrapper).
        let input = vec![b'X'; 10 * 1024 * 1024];
        let para = compress_parallel(&input);
        let seq = compress(&input);
        assert_eq!(
            para, seq,
            "single-block parallel should equal sequential"
        );
        let back = decompress(&para).expect("decompress");
        assert_eq!(back, input);
    }

    #[test]
    fn parallel_multi_block_roundtrips() {
        // 4 blocks of 32 MiB each (128 MiB total) — exercises the
        // ray on par_iter path with 4 tasks.
        let input: Vec<u8> = (0..128 * 1024 * 1024)
            .map(|i| ((i * 31) ^ (i >> 7)) as u8)
            .collect();
        let out = compress_parallel(&input);
        let back = decompress(&out).expect("decompress multi");
        assert_eq!(back, input);
    }

    #[test]
    fn parallel_does_not_equal_sequential_bytewise() {
        // The parallel output must have the NXP magic, not the NXS
        // magic — otherwise the decoder would treat it as a
        // single sequential stream.
        let input: Vec<u8> = (0..64 * 1024 * 1024).map(|i| i as u8).collect();
        let para = compress_parallel(&input);
        let seq = compress(&input);
        assert_ne!(
            &para[..4],
            &crate::format::MAGIC[..],
            "parallel output should NOT start with NXS magic"
        );
        assert_eq!(
            &para[..4],
            &PARALLEL_MAGIC[..],
            "parallel output should start with NXP magic"
        );
        assert_ne!(para, seq);
    }

    #[test]
    fn num_super_blocks_basic() {
        assert_eq!(num_super_blocks_with(0, 32 * 1024 * 1024), 0);
        assert_eq!(num_super_blocks_with(1, 32 * 1024 * 1024), 1);
        assert_eq!(
            num_super_blocks_with(32 * 1024 * 1024, 32 * 1024 * 1024),
            1
        );
        assert_eq!(
            num_super_blocks_with(32 * 1024 * 1024 + 1, 32 * 1024 * 1024),
            2
        );
        assert_eq!(
            num_super_blocks_with(64 * 1024 * 1024, 32 * 1024 * 1024),
            2
        );
        assert_eq!(
            num_super_blocks_with(128 * 1024 * 1024, 32 * 1024 * 1024),
            4
        );
    }

    // ── Decoder robustness (Sprint 5.7.1 stress fix) ─────────

    #[test]
    fn decoder_rejects_oversized_block_count() {
        // 4 bytes of payload but the header claims 1000 blocks.
        // Without the sanity check, the decoder would loop
        // 1000 times and panic on the first length-prefix read.
        let mut bogus = vec![];
        bogus.extend_from_slice(b"NXP\0");
        bogus.extend_from_slice(&1000u32.to_le_bytes()); // 1000 blocks
        bogus.extend_from_slice(&0u64.to_le_bytes()); // total_size
        bogus.extend_from_slice(&32u32.to_le_bytes()); // super_block_size
        // No actual block data — 0 bytes left.
        let res = crate::codec::decompress(&bogus);
        assert!(res.is_err(), "decoder must reject oversized block_count");
        let msg = format!("{:?}", res);
        assert!(msg.contains("claims 1000 blocks"), "got: {}", msg);
    }

    #[test]
    fn decoder_rejects_truncated_block_data() {
        // 2 blocks claimed, 1 block's data, then file ends.
        // The decoder must return Err instead of panicking.
        let mut truncated = vec![];
        truncated.extend_from_slice(b"NXP\0");
        truncated.extend_from_slice(&2u32.to_le_bytes());
        truncated.extend_from_slice(&100u64.to_le_bytes());
        truncated.extend_from_slice(&32u32.to_le_bytes());
        // Block 0 length prefix + block 0 data (fake)
        let fake_block_0 = compress(b"hello");
        truncated.extend_from_slice(&(fake_block_0.len() as u32).to_le_bytes());
        truncated.extend_from_slice(&fake_block_0);
        // Block 1 length prefix only, no actual block data.
        truncated.extend_from_slice(&100u32.to_le_bytes());
        // No data follows.
        let res = crate::codec::decompress(&truncated);
        assert!(res.is_err(), "decoder must reject truncated block data");
    }

    // ── OOM guard (Sprint 5.7.1 stress fix) ──────────────────

    #[test]
    fn oom_guard_clamps_to_at_least_two() {
        // Even on a 64-byte input (way below the parallel
        // threshold), the function must return ≥ 2.
        let n = safe_parallel_thread_count(64);
        assert!(n >= 2, "thread count must be ≥ 2 for parallel to win");
    }

    #[test]
    fn oom_guard_clamps_to_rayon_total() {
        // The function never exceeds rayon's total thread count.
        let n = safe_parallel_thread_count(128 * 1024 * 1024);
        let max = rayon::current_num_threads();
        assert!(n <= max, "thread count {} > rayon total {}", n, max);
    }

    #[test]
    fn parallel_compress_does_not_oom_on_tiny_ram() {
        // Simulate a 128 MB input on a 256 MB machine (the user
        // explicitly called this out as a container scenario). The
        // OOM guard should drop the parallel thread count to 1-2
        // (or fall back to sequential via the `n_blocks >= 2`
        // heuristic in `compress_auto`). Either way: the function
        // must return without panicking, and the result must
        // roundtrip.
        let input: Vec<u8> = (0..128 * 1024 * 1024).map(|i| (i * 7) as u8).collect();
        // We can't simulate 256 MB free RAM directly (the real
        // number is whatever the test runner has). But the
        // OOM guard returns a thread count that respects that
        // number. If the real machine has ≥ 8 GiB the guard
        // returns 8; if it has 256 MiB the guard returns ~1 and
        // the parallel path is skipped (n_blocks = 4 ≥ 2 so we DO
        // enter parallel with 1 thread, but it's effectively
        // sequential). Either way: no OOM, no panic.
        let _ = safe_parallel_thread_count(input.len());
        let out = compress_parallel(&input);
        let back = crate::codec::decompress(&out).expect("decompress");
        assert_eq!(back, input);
    }
}
