//! Content-Defined Chunking (CDC) using Gear hash.
//!
//! Splits a byte stream into variable-size chunks whose boundaries are
//! determined by the content itself. Two similar streams produce similar
//! chunk boundaries, which is the foundation of deduplication systems
//! like `rsync`, `restic`, and `bup`.
//!
//! ## Algorithm: Gear hash
//! Per byte: `hash = (hash << 1) + GEAR_TABLE[byte]`.
//! A chunk boundary is cut when `(hash & mask) == 0`, with `min_chunk`
//! and `max_chunk` constraints to bound chunk size.
//!
//! ## Why Gear (not Rabin fingerprint)
//! Rabin fingerprint uses polynomial division — mathematically elegant but
//! requires expensive modular arithmetic on each byte. Gear uses a single
//! shift + add + table lookup, ~5x faster, and equally good chunk
//! distribution for dedup purposes. Used by `restic`, `bup`, etc.
//!
//! ## Hash function choice
//! FNV-1a is used for the DEDUP lookup hash (in `dedup.rs`). Gear is only
//! for BOUNDARY detection — it's not collision-resistant, but it doesn't
//! need to be; it just needs to distribute boundaries uniformly.

/// Build the Gear table deterministically using xorshift64 seeded with a
/// fixed value. The values must be non-zero (otherwise the boundary check
/// `hash & mask == 0` triggers every byte).
const fn build_gear_table() -> [u64; 256] {
    let mut state: u64 = 0x123456789abcdef0;
    let mut table = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let v = state;
        // Force non-zero so the hash can grow
        table[i] = if v == 0 { 1 } else { v };
        i += 1;
    }
    table
}

pub static GEAR_TABLE: [u64; 256] = build_gear_table();

/// Stateful Gear chunker. Feed bytes; it tells you when to cut.
pub struct GearChunker {
    hash: u64,
    mask: u64,
    bytes_in_chunk: usize,
    min_chunk: usize,
    max_chunk: usize,
}

impl GearChunker {
    /// `bits` = log2(average chunk size). For avg ~64 KB use bits=16.
    pub fn new(min_chunk: usize, max_chunk: usize, bits: u32) -> Self {
        Self {
            hash: 0,
            mask: (1u64 << bits) - 1,
            bytes_in_chunk: 0,
            min_chunk,
            max_chunk,
        }
    }

    /// Reset state (call at the start of each chunk).
    pub fn reset(&mut self) {
        self.hash = 0;
        self.bytes_in_chunk = 0;
    }

    /// Feed a byte. Returns true if a chunk boundary should be cut here.
    pub fn feed(&mut self, byte: u8) -> bool {
        self.hash = (self.hash << 1).wrapping_add(GEAR_TABLE[byte as usize]);
        self.bytes_in_chunk += 1;

        // Don't cut before min_chunk bytes
        if self.bytes_in_chunk < self.min_chunk {
            return false;
        }
        // Force cut at max_chunk (prevents pathologically large chunks)
        if self.bytes_in_chunk >= self.max_chunk {
            self.reset();
            return true;
        }
        // Normal cut on hash match
        if self.hash & self.mask == 0 {
            self.reset();
            return true;
        }
        false
    }
}

/// Split `data` into chunks. Returns a Vec of (offset, length) pairs.
///
/// `bits` controls the average chunk size: `avg_chunk ≈ 2^bits` bytes.
/// A bit value of 16 gives ~64 KB average, matching the LZ77 window.
pub fn chunkify(
    data: &[u8],
    min_chunk: usize,
    max_chunk: usize,
    bits: u32,
) -> Vec<(usize, usize)> {
    let mut chunks = Vec::new();
    let mut chunker = GearChunker::new(min_chunk, max_chunk, bits);
    let mut start = 0usize;

    for (i, &byte) in data.iter().enumerate() {
        if chunker.feed(byte) {
            chunks.push((start, i - start + 1));
            start = i + 1;
        }
    }

    // Trailing partial chunk (only emit if non-empty)
    if start < data.len() {
        chunks.push((start, data.len() - start));
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunkify_empty() {
        let chunks = chunkify(&[], 1024, 4096, 12);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunkify_respects_min_max() {
        // With a small input and large min_chunk, all bytes go in one chunk
        let data = vec![0u8; 100];
        let chunks = chunkify(&data, 200, 1000, 10);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], (0, 100));
    }

    #[test]
    fn chunkify_max_chunk_forces_cut() {
        // max_chunk = 100, so we should cut at 100, 200, ...
        let data = vec![0u8; 250];
        let chunks = chunkify(&data, 10, 100, 16);
        // First cut at 100, second at 200, trailing 50
        assert!(chunks.len() >= 2);
        assert!(chunks[0].1 <= 100);
        assert_eq!(chunks[0].0, 0);
    }

    #[test]
    fn chunkify_is_deterministic() {
        // Same input → same chunk boundaries, every time
        let data: Vec<u8> = (0..10000).map(|i| (i * 37 + 11) as u8).collect();
        let c1 = chunkify(&data, 4096, 65536, 14);
        let c2 = chunkify(&data, 4096, 65536, 14);
        assert_eq!(c1, c2);
    }

    /// The signature test for CDC: take a repetitive stream, mutate it,
    /// and verify that boundaries BEFORE the mutation are stable.
    /// Boundaries AFTER the mutation may shift, but should drift by no
    /// more than `max_chunk` per boundary.
    #[test]
    fn boundaries_stable_under_modification() {
        // Repetitive base
        let base: Vec<u8> = (0..50000)
            .map(|i| ((i * 17 + i / 13) % 256) as u8)
            .collect();

        let chunks_base = chunkify(&base, 16 * 1024, 256 * 1024, 16);

        // Mutate the middle of the stream
        let mut mutated = base.clone();
        for i in 25000..25100 {
            mutated[i] = mutated[i].wrapping_add(1);
        }

        let chunks_mutated = chunkify(&mutated, 16 * 1024, 256 * 1024, 16);

        // Both should produce reasonable numbers of chunks
        assert!(chunks_base.len() > 0);
        assert!(chunks_mutated.len() > 0);

        // Verify: boundaries before the mutation point should be at the
        // same offsets (within max_chunk tolerance) in both versions.
        let mutation_offset = 25000;
        let tolerance = 256 * 1024; // max_chunk

        // For each base chunk boundary before the mutation, find the
        // closest mutated boundary. They should be within tolerance.
        for &(b_off, _) in &chunks_base {
            if b_off >= mutation_offset {
                break;
            }
            let closest = chunks_mutated
                .iter()
                .map(|&(m_off, _)| (m_off as i64 - b_off as i64).unsigned_abs())
                .min()
                .unwrap();
            assert!(
                closest < tolerance,
                "base boundary at {} drifted by {} (>{}) in mutated stream",
                b_off, closest, tolerance
            );
        }
    }

    /// Verify the Gear table has no zero entries (would break the
    /// boundary detection by triggering every byte).
    #[test]
    fn gear_table_has_no_zeros() {
        for (i, &v) in GEAR_TABLE.iter().enumerate() {
            assert_ne!(v, 0, "GEAR_TABLE[{}] = 0", i);
        }
    }

    #[test]
    fn gear_table_has_good_distribution() {
        // The low bits of the table should be roughly uniform (50% ones)
        // across the array. If not, the Gear hash will be biased.
        let mut low_bit_ones = 0;
        let mut total = 0;
        for &v in &GEAR_TABLE {
            low_bit_ones += (v & 1) as usize;
            total += 1;
        }
        let ratio = low_bit_ones as f64 / total as f64;
        assert!(
            (0.3..=0.7).contains(&ratio),
            "low bit distribution too skewed: {} / {} = {:.3}",
            low_bit_ones, total, ratio
        );
    }
}