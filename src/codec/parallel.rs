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

    // Compress all blocks in parallel via rayon.
    let compressed: Vec<Vec<u8>> = blocks
        .par_iter()
        .map(|block| crate::codec::compress(block))
        .collect();

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
}
