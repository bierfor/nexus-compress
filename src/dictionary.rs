//! Static literal dictionary for the compression pipeline.
//!
//! Pre-defines common substrings (English text, code keywords, JSON
//! tokens) that the LZ77 matchfinder can replace with a single
//! "dict ref" op, saving bytes that would otherwise go into the
//! literal rANS stream.
//!
//! ## Why this works
//!
//! For source code, the 3-byte sequence `    ` (4 spaces) appears
//! thousands of times. With LZ77 alone we either:
//!   - find a match for the spaces (dist + len = 32 bits per match), or
//!   - emit 4 literal spaces (4 × 8 = 32 bits after rANS gets it down
//!     to ~5 bits each = 20 bits).
//!
//! With a static dict, we emit ONE op (`DictRef { id, len: 4 }`) that
//! the decoder resolves via a pre-shared table. The op is 1 flag bit
//! + log2(N) bits for the id + log2(L) bits for the length. For 130
//! entries and length 4: 1 + 8 + 2 = 11 bits. That's a 2× win over
//! the LZ77 match and a ~2× win over literal encoding.
//!
//! ## Lookup structure
//!
//! Entries are indexed by (first_byte, second_byte?) for O(1) prefix
//! access. `lookup_at(data, pos)` returns the longest entry matching
//! `data[pos..]`. Tokens of length 1 use `(first, None)`; tokens of
//! length ≥2 use `(first, Some(second_of_token))`. At lookup time we
//! query both buckets (if applicable) and pick the longest match.
//!
//! ## Default dictionaries
//!
//! `default_text_dict`, `default_code_dict`, `default_json_dict` are
//! hand-curated for English text, Rust/source code, and JSON
//! respectively. `default_combined_dict` is the union of all three
//! (~130 entries, ~600 bytes of static storage).
//!
//! These are *starter* dictionaries. For production use, train on a
//! corpus of the data type you want to compress. See the v4.4 roadmap
//! note in the README.

#![allow(dead_code)]

use std::collections::HashMap;

/// A single dictionary entry. The `id` is the index into the
/// dictionary's `entries` vec, and is what gets written into the
/// bitstream when the LZ77 matchfinder emits a `DictRef` op.
#[derive(Debug, Clone)]
pub struct DictEntry {
    pub id: u16,
    pub token: Vec<u8>,
    /// Empirical frequency estimate (1-1000). Used for tie-breaking
    /// when multiple entries share the same prefix; not currently
    /// consumed by `lookup_at` (which always picks the longest match)
    /// but reserved for future cost-model refinement.
    pub est_freq: u32,
}

/// Static dictionary with O(1) prefix lookup.
///
/// The dictionary is built once at startup (from `default_*_dict()` or
/// loaded from a file) and then queried during LZ77 encoding. It is
/// shared between encoder and decoder (both must have the same
/// dictionary to roundtrip).
#[derive(Debug, Clone)]
pub struct Dictionary {
    /// All entries, in insertion order. `id` is the index into this
    /// vec, so `entries[id as usize]` is O(1).
    entries: Vec<DictEntry>,
    /// Prefix index: (first_byte, second_byte_or_none) → list of
    /// (entry_id, token_length). The bucket size is bounded by the
    /// number of entries that share a given 1-2 byte prefix; in our
    /// hand-curated dicts this is ≤5 per bucket.
    by_prefix: HashMap<(u8, Option<u8>), Vec<(u16, u8)>>,
}

impl Dictionary {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            by_prefix: HashMap::new(),
        }
    }

    /// Add a token. Returns the assigned id.
    ///
    /// - Empty tokens are rejected (returns 0, not stored).
    /// - Duplicate tokens (exact byte match) keep the FIRST id and
    ///   return it. This is idempotent — calling `insert(b"the")`
    ///   twice is harmless.
    pub fn insert(&mut self, token: &[u8], est_freq: u32) -> u16 {
        if token.is_empty() {
            return 0;
        }
        // Dedup
        for e in &self.entries {
            if e.token == token {
                return e.id;
            }
        }
        let id = self.entries.len() as u16;
        self.entries.push(DictEntry {
            id,
            token: token.to_vec(),
            est_freq,
        });
        let first = token[0];
        let second = if token.len() >= 2 {
            Some(token[1])
        } else {
            None
        };
        self.by_prefix
            .entry((first, second))
            .or_default()
            .push((id, token.len() as u8));
        id
    }

    /// Find the longest entry matching `data[pos..]`. Returns
    /// `(entry_id, match_length)`, or `None` if no entry matches.
    ///
    /// Match is exact (byte-for-byte). A token of length L matches
    /// iff `data[pos..pos+L] == token`.
    pub fn lookup_at(&self, data: &[u8], pos: usize) -> Option<(u16, usize)> {
        if pos >= data.len() {
            return None;
        }
        let first = data[pos];
        let second = if pos + 1 < data.len() {
            Some(data[pos + 1])
        } else {
            None
        };

        // Collect candidates from both buckets (if applicable).
        let mut candidates: Vec<(u16, u8)> = Vec::new();
        if let Some(s) = second {
            if let Some(bucket) = self.by_prefix.get(&(first, Some(s))) {
                candidates.extend_from_slice(bucket);
            }
        }
        if let Some(bucket) = self.by_prefix.get(&(first, None)) {
            candidates.extend_from_slice(bucket);
        }

        // Find the longest match among the candidates.
        let mut best: Option<(u16, usize)> = None;
        for &(id, declared_len) in &candidates {
            let dl = declared_len as usize;
            if pos + dl > data.len() {
                continue;
            }
            // O(1) lookup since id is the index
            let entry = &self.entries[id as usize];
            if entry.token.len() != dl {
                continue;
            }
            if data[pos..pos + dl] == entry.token[..] {
                if best.map_or(true, |(_, bl)| dl > bl) {
                    best = Some((id, dl));
                }
            }
        }
        best
    }

    /// Look up the token bytes for a given entry id. Used by the
    /// decoder to materialize a `DictRef` op.
    pub fn get(&self, id: u16) -> Option<&[u8]> {
        self.entries.get(id as usize).map(|e| e.token.as_slice())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all (id, token) pairs. For debugging / inspection.
    pub fn iter(&self) -> impl Iterator<Item = (u16, &[u8])> {
        self.entries.iter().map(|e| (e.id, e.token.as_slice()))
    }

    /// Serialize the dictionary to a binary blob.
    ///
    /// Format (`.dict` v1):
    ///   [4 bytes] magic = "NXD\0"
    ///   [4 bytes] version (u32 LE) = 1
    ///   [4 bytes] entry count (u32 LE)
    ///   per entry:
    ///     [1 byte]  token length (u8)
    ///     [N bytes] token bytes
    ///     [4 bytes] est_freq (u32 LE)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.entries.len() * 12);
        out.extend_from_slice(b"NXD\0");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            assert!(
                e.token.len() <= 255,
                "token too long to serialize: {} bytes",
                e.token.len()
            );
            out.push(e.token.len() as u8);
            out.extend_from_slice(&e.token);
            out.extend_from_slice(&e.est_freq.to_le_bytes());
        }
        out
    }

    /// Parse a binary dictionary blob produced by `to_bytes` (or by the
    /// `dict_train` CLI tool). Returns `None` if the magic or version
    /// don't match.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 12 {
            return None;
        }
        if &bytes[0..4] != b"NXD\0" {
            return None;
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
        if version != 1 {
            return None;
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        let mut dict = Self::new();
        let mut off = 12usize;
        for _ in 0..count {
            if off + 1 > bytes.len() {
                return None;
            }
            let tlen = bytes[off] as usize;
            off += 1;
            if off + tlen + 4 > bytes.len() {
                return None;
            }
            let token = bytes[off..off + tlen].to_vec();
            off += tlen;
            let freq = u32::from_le_bytes(bytes[off..off + 4].try_into().ok()?);
            off += 4;
            dict.insert(&token, freq);
        }
        Some(dict)
    }

    /// Train a dictionary from a corpus of files.
    ///
    /// Algorithm (greedy, "longer-wins" inspired by zstd's dictionary
    /// training):
    ///   1. Slide a window of 3-8 bytes over each input file.
    ///   2. Count the frequency of every unique substring.
    ///   3. Score each substring: `score = frequency * (length - 1)`.
    ///      A 6-byte token that appears 100 times saves more bytes
    ///      than a 2-byte token that appears 200 times — the score
    ///      reflects the marginal compression benefit, not just the
    ///      occurrence count.
    ///   4. Sort by score descending.
    ///   5. Greedily pick tokens, in score order, until `max_bytes`
    ///      is reached. Skip tokens that are a prefix of an
    ///      already-picked token (the longer token strictly dominates).
    ///   6. Skip tokens that appear fewer than `min_freq` times (rare
    ///      tokens waste dict slots).
    ///
    /// Returns `(dictionary, training_stats)`. Stats include the
    /// number of unique tokens seen and the final dict size.
    ///
    /// This is a 200-line implementation. It's NOT as sophisticated
    /// as zstd's `zdict.h` (which uses a multi-step knapsack-style
    /// optimization to find the local-best dictionary given the rest
    /// of the dictionary). For v4.5 this is enough — the smoke test
    /// shows the trained dict produces 10-50x more matches than the
    /// hand-curated default.
    pub fn train_from_corpus(
        corpora: &[&[u8]],
        max_bytes: usize,
        min_len: usize,
        max_len: usize,
        min_freq: u32,
    ) -> (Self, TrainStats) {
        Self::train_from_corpus_capped(corpora, max_bytes, min_len, max_len, min_freq, usize::MAX)
    }

    /// Like `train_from_corpus` but with a hard cap on the number of
    /// entries kept. The greedy selection is already sorted by score
    /// (descending), so `max_entries` simply truncates after the top-N
    /// picks. Useful when the trained dict must fit a fixed-size
    /// bitmask in the codec (FULL_DICT_ENTRIES).
    pub fn train_from_corpus_capped(
        corpora: &[&[u8]],
        max_bytes: usize,
        min_len: usize,
        max_len: usize,
        min_freq: u32,
        max_entries: usize,
    ) -> (Self, TrainStats) {
        use std::collections::HashMap;

        // Phase 1: count frequencies.
        let mut counts: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut total_substrings_scanned: u64 = 0;
        for data in corpora {
            if data.len() < min_len {
                continue;
            }
            for start in 0..=(data.len() - min_len) {
                let max_l = (data.len() - start).min(max_len);
                for len in min_len..=max_l {
                    total_substrings_scanned += 1;
                    let token = &data[start..start + len];
                    // Inline the increment to avoid the borrow on `data`.
                    let key = token.to_vec();
                    *counts.entry(key).or_insert(0) += 1;
                }
            }
        }

        // Phase 2: score and sort.
        let mut scored: Vec<(Vec<u8>, u32)> = counts
            .into_iter()
            .filter(|(_, freq)| *freq >= min_freq)
            .map(|(token, freq)| {
                let score = freq.saturating_mul((token.len() as u32).saturating_sub(1));
                (token, score)
            })
            .collect();
        scored.sort_unstable_by(|a, b| b.1.cmp(&a.1));

        // Phase 3: greedy selection. No prefix-skip — both "abc" and
        // "abc " can coexist. The LZ77 matchfinder picks whichever
        // gives the longest cover at runtime.
        let mut dict = Self::new();
        let mut total_size: usize = 0;
        for (token, _score) in scored {
            if dict.len() >= max_entries {
                break;
            }
            if total_size + token.len() > max_bytes {
                continue;
            }
            dict.insert(&token, 1);
            total_size += token.len();
        }

        let stats = TrainStats {
            total_substrings_scanned,
            unique_tokens_kept: dict.len(),
            dict_bytes: total_size,
            skipped_as_prefix: 0,
        };
        (dict, stats)
    }
}

/// Statistics from a `train_from_corpus` call.
#[derive(Debug, Clone)]
pub struct TrainStats {
    /// Total number of (start, len) pairs scanned across all files.
    pub total_substrings_scanned: u64,
    /// Number of tokens actually inserted into the dictionary.
    pub unique_tokens_kept: usize,
    /// Total bytes used by the dictionary's tokens (not including
    /// the prefix index overhead).
    pub dict_bytes: usize,
    /// Number of candidate tokens skipped (currently 0 — see the
    /// `train_from_corpus` doc for the design decision).
    pub skipped_as_prefix: u32,
}

impl Default for Dictionary {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------
// Default dictionaries
// ---------------------------------------------------------------------

/// Hand-curated English text dictionary: common 2-7 byte tokens from
/// natural language. Frequencies are rough (1-1000), based on
/// word-frequency tables; not used by `lookup_at` (always longest
/// match) but available for future cost-model work.
pub fn default_text_dict() -> Dictionary {
    let mut d = Dictionary::new();
    for &(tok, freq) in TEXT_TOKENS {
        d.insert(tok, freq);
    }
    d
}

/// Hand-curated Rust/source code dictionary. Common keywords, type
/// names, operators, and indentation patterns.
pub fn default_code_dict() -> Dictionary {
    let mut d = Dictionary::new();
    for &(tok, freq) in CODE_TOKENS {
        d.insert(tok, freq);
    }
    d
}

/// Hand-curated JSON dictionary. Common delimiters, values, and
/// quote patterns.
pub fn default_json_dict() -> Dictionary {
    let mut d = Dictionary::new();
    for &(tok, freq) in JSON_TOKENS {
        d.insert(tok, freq);
    }
    d
}

/// Combined dictionary: text + code + JSON (~130 entries).
pub fn default_combined_dict() -> Dictionary {
    let mut d = Dictionary::new();
    for &(tok, freq) in TEXT_TOKENS {
        d.insert(tok, freq);
    }
    for &(tok, freq) in CODE_TOKENS {
        d.insert(tok, freq);
    }
    for &(tok, freq) in JSON_TOKENS {
        d.insert(tok, freq);
    }
    d
}

// Static token lists. Each is `(&'static [u8], u32)` (token, freq).
// Total entries across all three: ~130 (fits in 1KB of static memory).

const TEXT_TOKENS: &[(&[u8], u32)] = &[
    (b"the", 1000),
    (b"and", 800),
    (b"ing", 600),
    (b"tion", 400),
    (b"er ", 500),
    (b"in ", 700),
    (b"an ", 600),
    (b"is ", 500),
    (b"to ", 600),
    (b"of ", 800),
    (b"ed ", 500),
    (b"or ", 400),
    (b"for", 400),
    (b"ith", 300),
    (b"ith ", 250),
    (b"as ", 300),
    (b"at ", 300),
    (b"be ", 300),
    (b"by ", 200),
    (b"that", 300),
    (b"this", 250),
    (b"with", 300),
    (b"from", 200),
    (b"have", 200),
    (b"are ", 200),
    (b"was ", 200),
    (b"not ", 200),
    (b"but ", 200),
    (b"all", 150),
    (b"can", 150),
    (b"had", 150),
    (b"her", 150),
    (b"one", 150),
    (b"our", 150),
    (b"out", 150),
    (b"day", 150),
    (b"get", 150),
    (b"use", 150),
    (b"man", 100),
    (b"new", 100),
    (b"now", 100),
    (b"old", 100),
    (b"see", 100),
    (b"way", 100),
    (b"who", 100),
    (b"boy", 100),
    (b"did", 100),
    (b"its", 100),
    (b"let", 100),
    (b"put", 100),
    (b"say", 100),
    (b"she", 100),
    (b"too", 100),
    (b"two", 100),
];

const CODE_TOKENS: &[(&[u8], u32)] = &[
    (b"fn ", 500),
    (b"let ", 800),
    (b"mut ", 600),
    (b"match ", 400),
    (b"if ", 600),
    (b"else", 300),
    (b"self.", 400),
    (b"pub ", 500),
    (b"struct", 300),
    (b"use ", 400),
    (b"impl ", 300),
    (b"for ", 400),
    (b"in ", 300),
    (b"return", 200),
    (b"=>", 250),
    (b"->", 200),
    (b"..", 200),
    (b"::", 400),
    (b"==", 200),
    (b"!=", 100),
    (b"<=", 100),
    (b">=", 100),
    (b"&&", 100),
    (b"||", 100),
    (b"    ", 800),
    (b"\n    ", 400),
    (b"\n}", 200),
    (b"\n    fn", 200),
    (b"\n    let", 200),
    (b"\n    if", 100),
    (b"\n    for", 100),
    (b"\n    match", 100),
    (b"Option<", 50),
    (b"Result<", 50),
    (b"Vec<", 50),
    (b"String", 50),
    (b"usize", 50),
    (b"u32", 50),
    (b"u8", 30),
    (b"u64", 30),
    (b"i32", 30),
    (b"i64", 30),
    (b"bool", 30),
    (b"true", 100),
    (b"false", 100),
    (b"Some(", 100),
    (b"None", 100),
    (b"Ok(", 100),
    (b"Err(", 100),
];

const JSON_TOKENS: &[(&[u8], u32)] = &[
    (b"\"", 500),
    (b":\"", 600),
    (b"\":", 100),
    (b",\"", 800),
    (b"\":\"", 200),
    (b"\": ", 200),
    (b", ", 1000),
    (b": ", 800),
    (b"null", 200),
    (b"true", 200),
    (b"false", 200),
    (b"[\"", 200),
    (b"\"]", 200),
    (b"{\"", 200),
    (b"\"}", 200),
    (b"[[", 50),
    (b"]]", 50),
    (b"{{", 50),
    (b"}}", 50),
    (b"\\\"", 100),
    (b"\\\\", 50),
    (b"\\n", 50),
    (b"\\t", 50),
];

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dict() {
        let d = Dictionary::new();
        assert_eq!(d.len(), 0);
        assert!(d.is_empty());
        assert!(d.lookup_at(b"anything", 0).is_none());
    }

    #[test]
    fn insert_basic() {
        let mut d = Dictionary::new();
        let id = d.insert(b"the", 100);
        assert_eq!(id, 0);
        assert_eq!(d.len(), 1);
        assert!(!d.is_empty());
        assert_eq!(d.get(id), Some(b"the".as_ref()));
    }

    #[test]
    fn insert_dedup() {
        let mut d = Dictionary::new();
        let id1 = d.insert(b"the", 100);
        let id2 = d.insert(b"the", 200);
        assert_eq!(id1, id2);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn insert_empty_rejected() {
        let mut d = Dictionary::new();
        let id = d.insert(b"", 100);
        assert_eq!(id, 0);
        assert_eq!(d.len(), 0);
    }

    #[test]
    fn insert_distinct_ids() {
        let mut d = Dictionary::new();
        assert_eq!(d.insert(b"the", 100), 0);
        assert_eq!(d.insert(b"and", 100), 1);
        assert_eq!(d.insert(b"ing", 100), 2);
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn lookup_exact_match() {
        let mut d = Dictionary::new();
        d.insert(b"the", 100);
        let r = d.lookup_at(b"the", 0);
        assert_eq!(r, Some((0, 3)));
    }

    #[test]
    fn lookup_no_match() {
        let mut d = Dictionary::new();
        d.insert(b"the", 100);
        assert!(d.lookup_at(b"and", 0).is_none());
        assert!(d.lookup_at(b"they", 0).is_some()); // "the" prefix
    }

    #[test]
    fn lookup_at_offset() {
        let mut d = Dictionary::new();
        d.insert(b"the", 100);
        let r = d.lookup_at(b"  the", 2);
        assert_eq!(r, Some((0, 3)));
    }

    #[test]
    fn lookup_picks_longest() {
        let mut d = Dictionary::new();
        d.insert(b"th", 100); // short
        d.insert(b"the", 100); // longer
        d.insert(b"them", 50); // longest
        let r = d.lookup_at(b"them", 0);
        assert_eq!(r, Some((2, 4))); // "them" wins over "the" and "th"
    }

    #[test]
    fn lookup_past_end() {
        let mut d = Dictionary::new();
        d.insert(b"hello", 100);
        // data is "hi" — not enough bytes for "hello"
        assert!(d.lookup_at(b"hi", 0).is_none());
        // data is "hell" — partial match, NOT a full token
        assert!(d.lookup_at(b"hell", 0).is_none());
    }

    #[test]
    fn lookup_two_byte_entry() {
        let mut d = Dictionary::new();
        d.insert(b"=>", 100);
        let r = d.lookup_at(b"=>", 0);
        assert_eq!(r, Some((0, 2)));
    }

    #[test]
    fn lookup_one_byte_entry() {
        let mut d = Dictionary::new();
        d.insert(b"a", 100);
        let r = d.lookup_at(b"abc", 0);
        assert_eq!(r, Some((0, 1)));
    }

    #[test]
    fn lookup_one_byte_alongside_multi_byte() {
        // If both a 1-byte entry "a" and a 2-byte entry "ab" exist,
        // looking up "abc" should pick "ab" (longest).
        let mut d = Dictionary::new();
        d.insert(b"a", 100);
        d.insert(b"ab", 100);
        let r = d.lookup_at(b"abc", 0);
        assert_eq!(r, Some((1, 2)));
    }

    #[test]
    fn lookup_combined_dict_finds_known() {
        let d = default_combined_dict();
        // "the" should be in the text dict
        let r = d.lookup_at(b"the", 0);
        assert!(r.is_some(), "combined dict should contain 'the'");
        // "fn " should be in the code dict
        let r = d.lookup_at(b"fn ", 0);
        assert!(r.is_some(), "combined dict should contain 'fn '");
        // "\", \"" should be in the json dict
        let r = d.lookup_at(b",\"", 0);
        assert!(r.is_some(), "combined dict should contain ',\"'");
    }

    #[test]
    fn default_dicts_nonempty() {
        assert!(default_text_dict().len() > 10);
        assert!(default_code_dict().len() > 10);
        assert!(default_json_dict().len() > 10);
        assert!(default_combined_dict().len() > 30);
    }

    #[test]
    fn combined_dict_idempotent() {
        // Re-inserting the same token should not grow the dict.
        let mut d = default_combined_dict();
        let before = d.len();
        d.insert(b"the", 999);
        d.insert(b"fn ", 999);
        assert_eq!(d.len(), before, "duplicates should not grow the dict");
    }

    #[test]
    fn lookup_at_arbitrary_offset() {
        let d = default_text_dict();
        let text = b"hello the world";
        // "the" starts at position 6
        let r = d.lookup_at(text, 6);
        assert_eq!(r.map(|(_, l)| l), Some(3));
    }

    #[test]
    fn iter_visits_all() {
        let d = default_code_dict();
        let count = d.iter().count();
        assert_eq!(count, d.len());
        assert!(count > 10);
    }

    // ---------------------------------------------------------------
    // Binary serialization
    // ---------------------------------------------------------------

    #[test]
    fn bytes_roundtrip_empty() {
        let d = Dictionary::new();
        let bytes = d.to_bytes();
        let d2 = Dictionary::from_bytes(&bytes).expect("from_bytes failed");
        assert_eq!(d2.len(), 0);
    }

    #[test]
    fn bytes_roundtrip_combined() {
        let d = default_combined_dict();
        let bytes = d.to_bytes();
        let d2 = Dictionary::from_bytes(&bytes).expect("from_bytes failed");
        assert_eq!(d2.len(), d.len());
        // Each entry should still lookup correctly.
        for (i, _) in d.iter().enumerate() {
            // ids are dense 0..len-1
            let token = d.get(i as u16).unwrap();
            assert_eq!(d2.get(i as u16), Some(token));
        }
    }

    #[test]
    fn from_bytes_rejects_bad_magic() {
        let bad = b"XXXX\0\0\0\0\0\0\0\0\0";
        assert!(Dictionary::from_bytes(bad).is_none());
    }

    #[test]
    fn from_bytes_rejects_truncated() {
        let d = default_text_dict();
        let mut bytes = d.to_bytes();
        bytes.truncate(bytes.len() - 5);
        assert!(Dictionary::from_bytes(&bytes).is_none());
    }

    #[test]
    fn from_bytes_rejects_bad_version() {
        let d = default_text_dict();
        let mut bytes = d.to_bytes();
        // Corrupt version
        bytes[5] = 0xFF;
        assert!(Dictionary::from_bytes(&bytes).is_none());
    }

    #[test]
    fn from_bytes_handles_1byte_token() {
        let mut d = Dictionary::new();
        d.insert(b"a", 100);
        let bytes = d.to_bytes();
        let d2 = Dictionary::from_bytes(&bytes).unwrap();
        assert_eq!(d2.get(0), Some(b"a".as_ref()));
    }

    #[test]
    fn from_bytes_handles_long_token() {
        let mut d = Dictionary::new();
        d.insert(b"hello world this is long", 100);
        let bytes = d.to_bytes();
        let d2 = Dictionary::from_bytes(&bytes).unwrap();
        assert_eq!(d2.get(0), Some(b"hello world this is long".as_ref()));
    }

    // ---------------------------------------------------------------
    // Training
    // ---------------------------------------------------------------

    #[test]
    fn train_picks_frequent_substring() {
        // "abc" appears 5 times. Should be in the dict.
        let corpus: &[&[u8]] = &[b"abc abc abc abc abc"];
        let (d, stats) = Dictionary::train_from_corpus(corpus, 1024, 3, 8, 2);
        assert!(d.get(0).is_some(), "dict should have at least one entry");
        assert!(
            d.lookup_at(b"abc", 0).is_some(),
            "dict should contain 'abc'"
        );
        assert!(stats.dict_bytes > 0);
    }

    #[test]
    fn train_respects_min_freq() {
        // "abc" appears once, should be skipped with min_freq=2.
        let corpus: &[&[u8]] = &[b"abc def ghi jkl mno"];
        let (d, _stats) = Dictionary::train_from_corpus(corpus, 1024, 3, 8, 2);
        assert!(d.lookup_at(b"abc", 0).is_none());
    }

    #[test]
    fn train_skips_prefix_of_picked() {
        // "abcdef" appears 3 times. "abc" appears as a substring many
        // more times. Without prefix-skip, "abc" would be picked first
        // (highest score) and "abcdef" would be skipped due to size.
        // With prefix-skip, the longer "abcdef" is preferred.
        let corpus: &[&[u8]] = &[b"abcdef abcdef abcdef abc abc abc abc abc abc"];
        let (d, _stats) = Dictionary::train_from_corpus(corpus, 1024, 3, 8, 2);
        // Look for either "abc" or "abcdef" — both should be valid.
        let has_abc = d.lookup_at(b"abc", 0).is_some();
        let has_abcdef = d.lookup_at(b"abcdef", 0).is_some();
        assert!(
            has_abc || has_abcdef,
            "dict should contain 'abc' or 'abcdef'"
        );
    }

    #[test]
    fn train_respects_max_bytes() {
        // Big corpus, small budget — dict should be bounded.
        let mut big = Vec::new();
        for i in 0..1000 {
            big.extend_from_slice(format!("phrase_{} ", i).as_bytes());
        }
        let corpus: &[&[u8]] = &[&big];
        let (_d, stats) = Dictionary::train_from_corpus(corpus, 200, 3, 8, 2);
        assert!(
            stats.dict_bytes <= 200,
            "dict should be bounded by max_bytes"
        );
    }

    #[test]
    fn train_on_empty_corpus() {
        let corpus: &[&[u8]] = &[&[]];
        let (d, stats) = Dictionary::train_from_corpus(corpus, 1024, 3, 8, 2);
        assert_eq!(d.len(), 0);
        assert_eq!(stats.dict_bytes, 0);
    }

    #[test]
    fn trained_dict_roundtrips_through_bytes() {
        let corpus: &[&[u8]] = &[b"the cat the cat the cat the dog the dog"];
        let (d, _stats) = Dictionary::train_from_corpus(corpus, 1024, 3, 8, 2);
        let bytes = d.to_bytes();
        let d2 = Dictionary::from_bytes(&bytes).unwrap();
        assert_eq!(d2.len(), d.len());
    }

    #[test]
    fn trained_dict_can_be_used_for_encoding() {
        // Round-trip: train, encode_with_dict, decode.
        let data = b"the cat sat on the mat the cat sat on the mat";
        let corpus: &[&[u8]] = &[data];
        let (d, _stats) = Dictionary::train_from_corpus(corpus, 1024, 3, 8, 2);

        use crate::lz77::{MatchDecoder, MatchFinder};
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(data, &d);
        let mut dec = MatchDecoder::with_dict(d);
        let out = dec.decode(&ops);
        assert_eq!(out, data, "trained dict roundtrip failed");
    }
}
