//! Block-level deduplication.
//!
//! Splits the input into fixed-size blocks (default 4 KB) BEFORE LZ77/rANS.
//! Each block is hashed; if its hash has been seen, the block is emitted as
//! a `BlockType::Duplicate` referencing the original block's ID instead of
//! running compression on it.
//!
//! ## Hash choice: FNV-1a 64-bit
//! We use FNV-1a (no external dependency, ~5 GB/s throughput) instead of
//! xxHash. xxHash is faster but pulls in a C dependency. For MVP the
//! simpler FNV is fine.
//!
//! ## Memory model
//! - Encoder: `HashMap<u64, u32>` mapping hash → block_id. For a 1 GB file
//!   with 4 KB blocks, that's 256 K entries (~16 MB on a 64-bit system).
//! - Decoder: `Vec<Vec<u8>>` cache of decoded blocks, indexed by block_id.
//!   Memory grows with unique blocks, not with file size.
//!
//! ## Limitations (deferred to v2)
//! - Fixed 4 KB blocks; misaligned duplicates (e.g., block shifted by 1 byte
//!   from the previous occurrence) won't dedup. CDC fixes this.
//! - No sliding window on the dedup hashmap; huge files keep all entries
//!   forever. Could add LRU eviction later.

use std::collections::HashMap;

/// Default block size for dedup. 64 KB matches the LZ77 sliding window,
/// so the trade-off is:
/// - For files < 64 KB: behaves like the old v1.2 single-block pipeline.
///   LZ77 sees the whole file; dedup is moot (no prior block to match).
/// - For files >= 64 KB with redundant regions: dedup catches repeats
///   between separate 64 KB blocks. LZ77 within each block still handles
///   intra-block repetition.
/// - Hashmap memory: ~1 MB per 1 GB of input.
pub const DEDUP_BLOCK_SIZE: usize = 64 * 1024;

/// FNV-1a 64-bit hash. Fast, simple, no dependencies.
#[derive(Debug, Default, Clone, Copy)]
pub struct Fnv1a {
    state: u64,
}

impl Fnv1a {
    pub fn new() -> Self {
        Self {
            state: 0xcbf29ce484222325, // FNV offset basis (64-bit)
        }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= b as u64;
            self.state = self.state.wrapping_mul(0x100000001b3); // FNV prime
        }
    }

    pub fn finish(self) -> u64 {
        self.state
    }

    pub fn hash(bytes: &[u8]) -> u64 {
        let mut h = Self::new();
        h.update(bytes);
        h.finish()
    }
}

/// Result of trying to dedup a block during encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupResult {
    /// Block is identical to a previously-seen block; payload should be
    /// `[u32 original_id]` and block_type should be `Duplicate`.
    Duplicate { original_id: u32 },
    /// Block is unique; payload should be the normal compressed form and
    /// the encoder should record `hash` -> `current_block_id` after.
    Unique { hash: u64 },
}

/// Tracks block hashes seen during compression.
#[derive(Debug, Default)]
pub struct DedupTable {
    hash_to_id: HashMap<u64, u32>,
}

impl DedupTable {
    pub fn new() -> Self {
        Self {
            hash_to_id: HashMap::new(),
        }
    }

    /// Check if `block`'s hash has been seen before.
    pub fn lookup(&mut self, block: &[u8], block_id: u32) -> DedupResult {
        let hash = Fnv1a::hash(block);
        if let Some(&original_id) = self.hash_to_id.get(&hash) {
            DedupResult::Duplicate { original_id }
        } else {
            self.hash_to_id.insert(hash, block_id);
            DedupResult::Unique { hash }
        }
    }

    pub fn len(&self) -> usize {
        self.hash_to_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hash_to_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_basic() {
        // Known test vector for FNV-1a 64-bit
        let h = Fnv1a::hash(b"");
        assert_eq!(h, 0xcbf29ce484222325);
        let h = Fnv1a::hash(b"a");
        assert_eq!(h, 0xaf63dc4c8601ec8c);
        let h = Fnv1a::hash(b"foobar");
        assert_eq!(h, 0x85944171f73967e8);
    }

    #[test]
    fn block_size_constant() {
        // Sanity: must equal LZ77 window
        assert_eq!(DEDUP_BLOCK_SIZE, 64 * 1024);
    }

    #[test]
    fn fnv1a_incremental_matches() {
        let data = b"the quick brown fox jumps over the lazy dog";
        let h1 = Fnv1a::hash(data);
        let mut h = Fnv1a::new();
        h.update(&data[..10]);
        h.update(&data[10..]);
        assert_eq!(h.finish(), h1);
    }

    #[test]
    fn dedup_detects_duplicates() {
        let mut dt = DedupTable::new();
        let block_a = b"hello world, this is block A";
        let block_b = b"this is a completely different block";
        let block_a_dup = b"hello world, this is block A"; // identical to A

        // First occurrence of A: unique, recorded
        match dt.lookup(block_a, 0) {
            DedupResult::Unique { .. } => {}
            _ => panic!("expected unique"),
        }
        // B is new
        match dt.lookup(block_b, 1) {
            DedupResult::Unique { .. } => {}
            _ => panic!("expected unique"),
        }
        // A again → duplicate
        match dt.lookup(block_a_dup, 2) {
            DedupResult::Duplicate { original_id: 0 } => {}
            r => panic!("expected duplicate of 0, got {:?}", r),
        }
        assert_eq!(dt.len(), 2);
    }

    #[test]
    fn dedup_handles_partial_blocks() {
        let mut dt = DedupTable::new();
        // Smaller than DEDUP_BLOCK_SIZE
        let small = b"tiny";
        match dt.lookup(small, 0) {
            DedupResult::Unique { .. } => {}
            _ => panic!("expected unique"),
        }
        match dt.lookup(small, 1) {
            DedupResult::Duplicate { original_id: 0 } => {}
            _ => panic!("expected duplicate"),
        }
    }
}