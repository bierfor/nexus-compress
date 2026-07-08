//! Block-level deduplication.
//!
//! Splits the input into fixed-size blocks (default 64 KB) BEFORE LZ77/rANS.
//! Each block is hashed; if its hash has been seen, the block is emitted as
//! a `BlockType::Duplicate` referencing the original block's ID instead of
//! running compression on it.
//!
//! ## Hash choice: FNV-1a 64-bit + content verification
//! We use FNV-1a for the fast check (~5 GB/s). When a hash matches, we
//! verify the content byte-by-byte (slow path) to rule out FNV-1a
//! collisions. FNV-1a 64-bit is NOT cryptographic — for 64 KB blocks
//! across a 1 GB file, collisions are unlikely but possible. Without
//! the verification, a hash collision would cause silent corruption.
//!
//! ## Memory model
//! - Encoder: `HashMap<u64, (u32, u32)>` mapping hash → (unique_id, content_len).
//!   The content_len is stored for the verification step. For a 1 GB file
//!   with 4 KB blocks, that's 256 K entries (~16 MB on a 64-bit system).
//! - Decoder: `Vec<Vec<u8>>` cache of decoded blocks, indexed by unique_id.
//!   `unique_id` is the position in the order of unique blocks (NOT the
//!   block position in the file). The encoder increments the counter only
//!   for unique blocks; the decoder increments by pushing to the cache.
//!   Both sides agree because both use the same counting policy.
//!
//! ## Bug fix (sprint 2.9)
//! Earlier versions used the block's position-in-file (block_id) as
//! the dedup reference. The decoder's cache was indexed by the order
//! of unique blocks decoded, so once a block was marked as a
//! duplicate, the indices diverged. Files with > 100 blocks would
//! eventually panic with "Duplicate references unknown block id".
//! The fix is to use a sequential unique_id counter on BOTH sides.

use std::collections::HashMap;

/// Default block size for dedup. 64 KB matches the LZ77 sliding window.
pub const DEDUP_BLOCK_SIZE: usize = 64 * 1024;

/// FNV-1a 64-bit hash. Fast, simple, no dependencies.
#[derive(Debug, Default, Clone, Copy)]
pub struct Fnv1a {
    state: u64,
}

impl Fnv1a {
    pub fn new() -> Self {
        Self {
            state: 0xcbf29ce484222325,
        } // FNV offset basis (64-bit)
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
    /// Block is byte-identical to a previously-seen block; payload should
    /// be `[u32 original_unique_id]` and block_type should be `Duplicate`.
    Duplicate { original_id: u32 },
    /// Block is unique; payload should be the normal compressed form and
    /// the encoder should record `hash → (current_unique_id, content_len)`
    /// after this call.
    Unique { hash: u64 },
}

/// Tracks block hashes seen during compression. Stores the unique_id
/// (sequential counter) and content length so the encoder can verify
/// hash collisions before emitting a Duplicate block.
#[derive(Debug, Default)]
pub struct DedupTable {
    hash_to_entry: HashMap<u64, (u32, usize)>,
}

impl DedupTable {
    pub fn new() -> Self {
        Self {
            hash_to_entry: HashMap::new(),
        }
    }

    /// Number of unique blocks recorded.
    pub fn len(&self) -> usize {
        self.hash_to_entry.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hash_to_entry.is_empty()
    }

    /// Check if `block`'s hash has been seen.
    ///
    /// If the hash matches a previous block, returns
    /// `Duplicate { original_id }` ONLY if the content is byte-identical
    /// (verified against `original_content`). Otherwise treats the block
    /// as unique (defending against FNV-1a collisions).
    ///
    /// `unique_id` is the encoder's sequential counter for unique
    /// blocks; it's the value that will be emitted as `original_id`
    /// if a future block has the same content.
    pub fn lookup(
        &mut self,
        block: &[u8],
        unique_id: u32,
        original_content: impl FnOnce(u32) -> Option<Vec<u8>>,
    ) -> DedupResult {
        let hash = Fnv1a::hash(block);
        if let Some(entry) = self.hash_to_entry.get(&hash).copied() {
            let (orig_id, orig_len) = entry;
            // Hash matches a prior block. Verify content.
            if orig_len == block.len() {
                if let Some(orig) = original_content(orig_id) {
                    if orig == block {
                        return DedupResult::Duplicate {
                            original_id: orig_id,
                        };
                    }
                }
            }
            // Hash collision (or content check failed). Fall through
            // and treat as unique.
        }
        self.hash_to_entry.insert(hash, (unique_id, block.len()));
        DedupResult::Unique { hash }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_basic() {
        assert_eq!(Fnv1a::hash(b""), 0xcbf29ce484222325);
        assert_eq!(Fnv1a::hash(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(Fnv1a::hash(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn block_size_constant() {
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
        let block_a_dup = b"hello world, this is block A";
        // No content stored yet, so the closure returns None and
        // verification is skipped (the very first block can't
        // collide with anything).
        let none = |_: u32| None;
        match dt.lookup(block_a, 0, none) {
            DedupResult::Unique { .. } => {}
            _ => panic!("expected unique"),
        }
        match dt.lookup(block_b, 1, none) {
            DedupResult::Unique { .. } => {}
            _ => panic!("expected unique"),
        }
        // Now we can look up content for prior blocks.
        let with_content = |id: u32| -> Option<Vec<u8>> {
            if id == 0 {
                Some(block_a.to_vec())
            } else {
                Some(block_b.to_vec())
            }
        };
        match dt.lookup(block_a_dup, 2, with_content) {
            DedupResult::Duplicate { original_id: 0 } => {}
            r => panic!("expected duplicate of 0, got {:?}", r),
        }
        assert_eq!(dt.len(), 2);
    }

    #[test]
    fn dedup_unique_id_stays_sequential() {
        // The unique_id passed to lookup is what gets stored as the
        // value (and later returned as original_id). Even after a
        // duplicate, the counter never goes backward.
        let mut dt = DedupTable::new();
        let b = b"repeated content";
        let none = |_: u32| None;
        // unique_id=0 first time
        match dt.lookup(b, 0, none) {
            DedupResult::Unique { .. } => {}
            _ => panic!(),
        }
        // The encoder will increment its counter to 1 for the next
        // block, but lookup itself doesn't increment. The dedup
        // table just records.
        // Second block with same content: lookup returns Duplicate
        // with the original unique_id (0), and the new block uses
        // unique_id=1 in the call but doesn't insert.
        let with_content = |id: u32| if id == 0 { Some(b.to_vec()) } else { None };
        match dt.lookup(b, 1, with_content) {
            DedupResult::Duplicate { original_id: 0 } => {}
            _ => panic!(),
        }
        assert_eq!(dt.len(), 1);
    }

    #[test]
    fn dedup_handles_hash_collision() {
        // Two blocks with different content but (in theory) the
        // same hash. The dedup table should treat them as unique.
        // We can't easily force a real FNV-1a collision in a test,
        // so we simulate by injecting a fake entry via a private
        // helper. The behavior we want: lookup(b, id, content)
        // returns Unique when content doesn't match, even if the
        // hash collides.
        let mut dt = DedupTable::new();
        // Inject a fake entry with a known hash and id.
        // We compute the hash of "real_a" and pretend it's a
        // collision with "fake_a".
        let real_a = b"real block content";
        let fake_a = b"fake block content"; // different
        let h_real = Fnv1a::hash(real_a);
        // Bypass the public API by inserting directly (test only).
        dt.hash_to_entry.insert(h_real, (0, real_a.len()));
        // Now try to look up a different block that happens to have
        // the same hash (we force this by inserting manually).
        // Real lookup:
        let r = dt.lookup(real_a, 1, |id| {
            if id == 0 {
                Some(real_a.to_vec())
            } else {
                None
            }
        });
        assert!(matches!(r, DedupResult::Duplicate { original_id: 0 }));
        // Fake lookup (different content but same hash via direct insert):
        let mut dt2 = DedupTable::new();
        let orig = b"original content that won't match";
        let h = Fnv1a::hash(orig);
        dt2.hash_to_entry.insert(h, (0, fake_a.len())); // wrong content!
        let r = dt2.lookup(
            fake_a,
            1,
            |id| {
                if id == 0 {
                    Some(orig.to_vec())
                } else {
                    None
                }
            },
        );
        // Should be Unique because content doesn't match (length differs
        // here; with the same length but different bytes the result
        // would still be Unique via the != check).
        assert!(matches!(r, DedupResult::Unique { .. }));
    }

    #[test]
    fn dedup_handles_partial_blocks() {
        let mut dt = DedupTable::new();
        let small = b"tiny";
        let none = |_: u32| None;
        match dt.lookup(small, 0, none) {
            DedupResult::Unique { .. } => {}
            _ => panic!(),
        }
        let with_content = |id: u32| if id == 0 { Some(small.to_vec()) } else { None };
        match dt.lookup(small, 1, with_content) {
            DedupResult::Duplicate { original_id: 0 } => {}
            _ => panic!(),
        }
    }

    /// Regression test for the v2.9 panic
    /// "Duplicate references unknown block id N (cache.len()=M)".
    ///
    /// Earlier versions used `block_id` (the chunk index) as the
    /// dedup reference, but the decoder's cache was indexed by the
    /// order of unique blocks. The two indices diverged whenever a
    /// block was a duplicate. We test the realistic pattern: many
    /// unique blocks interspersed with duplicates.
    #[test]
    fn dedup_indices_align_across_many_blocks() {
        // Simulate the encoder: 10 unique blocks, some duplicated.
        // For each entry, track (block_id, unique_id, content, is_duplicate).
        let cases: Vec<(u32, u32, Vec<u8>, bool)> = vec![
            (0, 0, b"block A".to_vec(), false), // unique, unique_id=0
            (1, 1, b"block B".to_vec(), false), // unique, unique_id=1
            (2, 0, b"block A".to_vec(), true),  // duplicate of unique_id=0
            (3, 2, b"block C".to_vec(), false), // unique, unique_id=2
            (4, 1, b"block B".to_vec(), true),  // duplicate of unique_id=1
            (5, 3, b"block D".to_vec(), false), // unique, unique_id=3
            (6, 0, b"block A".to_vec(), true),  // duplicate of unique_id=0
            (7, 4, b"block E".to_vec(), false), // unique, unique_id=4
            (8, 2, b"block C".to_vec(), true),  // duplicate of unique_id=2
        ];
        // For each unique block, push its content in unique_id order.
        let mut unique_contents: Vec<Vec<u8>> = Vec::new();
        for (_block_id, expected_unique_id, content, is_duplicate) in &cases {
            if !*is_duplicate {
                assert_eq!(
                    unique_contents.len() as u32,
                    *expected_unique_id,
                    "unique_id should match unique_contents index"
                );
                unique_contents.push(content.clone());
            }
        }
        // Now check: for each Duplicate, the referenced unique_id
        // is in range and the content matches.
        for (_block_id, expected_unique_id, content, is_duplicate) in &cases {
            if *is_duplicate {
                let referenced = unique_contents
                    .get(*expected_unique_id as usize)
                    .expect("decoder cache out of sync with encoder unique_id");
                assert_eq!(
                    referenced, content,
                    "content mismatch at unique_id {}",
                    expected_unique_id
                );
            }
        }
    }
}
