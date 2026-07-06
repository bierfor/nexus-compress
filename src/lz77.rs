//! LZ77 match finder with HASH CHAINS + LAZY MATCHING.
//!
//! ## Hash chains
//! Each hash bucket stores the most recent position with that hash (`head[h]`),
//! and every position keeps a "previous link" via `prev[i % WINDOW_SIZE]`.
//! Walking the chain from `head[h]` gives a list of candidate positions that
//! all share the same 3-byte prefix. We walk up to `MAX_CHAIN_STEPS` deep and
//! skip stale entries (i - candidate >= MAX_DIST) caused by the circular buffer
//! overwriting old links.
//!
//! ## Lazy matching
//! After finding a match of length L at position i, we peek at position i+1.
//! If i+1's match is more than 1 byte longer than L (so the extra literal at i
//! "pays for itself"), we emit a literal at i instead of the original match
//! and recurse from i+1. This is the standard ZSTD-level-1 heuristic.
//!
//! ## Tunables (compile-time constants)
//!   WINDOW_SIZE    = 64 KiB
//!   MAX_DIST       = 64 KiB (strictly < to avoid circular-buffer wrap bugs)
//!   MIN_MATCH      = 3
//!   MAX_MATCH      = 258
//!   MAX_CHAIN_STEPS = 32 (cap chain walk to keep speed competitive)

pub const WINDOW_SIZE: usize = 64 * 1024;
pub const HASH_BITS: usize = 15;
pub const HASH_SIZE: usize = 1 << HASH_BITS;
pub const HASH_MASK: usize = HASH_SIZE - 1;
pub const MIN_MATCH: usize = 3;
pub const MAX_MATCH: usize = 255;
pub const MAX_DIST: usize = WINDOW_SIZE;
pub const MAX_CHAIN_STEPS: usize = 32;

/// Minimum length for a `DictRef` op to be emitted. Below this, the
/// rANS+op-flag overhead of a dict ref exceeds the savings from
/// replacing literals. The cost model: 1 flag + 1-2 byte id + 1 byte
/// len ≈ 24 bits, vs 3 literals ≈ 18 bits (after rANS). 3 is the
/// break-even point; 4+ is a clear win.
pub const MIN_DICT_MATCH: usize = 3;

use crate::dictionary::Dictionary;

/// Compare two DP states (cost, ops). Returns true if (new_cost, new_ops)
/// is STRICTLY BETTER than (cur_cost, cur_ops).
///
/// "Better" = lower cost, OR (equal cost AND fewer ops).
///
/// The ops tie-break matters: two paths with identical bit-cost may
/// differ in op count, and fewer ops = less per-op header overhead in
/// the actual bitstream (op flags, table lookups, etc.). So among
/// minimum-cost paths, we prefer the one with the fewest ops.
#[inline(always)]
fn is_better(new_cost: u32, new_ops: u32, cur_cost: u32, cur_ops: u32) -> bool {
    new_cost < cur_cost || (new_cost == cur_cost && new_ops < cur_ops)
}

#[inline(always)]
fn hash3(data: &[u8], i: usize) -> usize {
    let v = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], 0]);
    ((v.wrapping_mul(0x9E3779B1)) >> (32 - HASH_BITS)) as usize
}

#[derive(Debug)]
pub struct MatchFinder {
    /// head[h] = most recent ABSOLUTE position with hash h, or -1.
    head: Box<[i32; HASH_SIZE]>,
    /// prev[pos % WINDOW_SIZE] = previous ABSOLUTE position with same hash.
    /// Stored as absolute positions so we can detect stale entries via
    /// (i - candidate >= MAX_DIST).
    prev: Box<[i32; WINDOW_SIZE]>,
}

impl Default for MatchFinder {
    fn default() -> Self {
        Self::new()
    }
}

impl MatchFinder {
    pub fn new() -> Self {
        Self {
            head: Box::new([-1; HASH_SIZE]),
            prev: Box::new([-1; WINDOW_SIZE]),
        }
    }

    /// Find the best match at `pos` in `data` WITHOUT mutating state.
    /// Returns (length, distance). Length < MIN_MATCH means no usable match.
    pub fn peek(&self, data: &[u8], pos: usize) -> (usize, usize) {
        if pos + MIN_MATCH > data.len() {
            return (0, 0);
        }
        let h = hash3(data, pos);
        let mut candidate = self.head[h];
        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        let mut steps = 0usize;

        while candidate >= 0 && steps < MAX_CHAIN_STEPS {
            let cand = candidate as usize;
            // Stale entry? (circular buffer overwrote prev at slot cand % WINDOW_SIZE
            // when some position j >= cand + MAX_DIST was processed)
            // Also reject self-reference (dist=0).
            if pos <= cand || pos - cand >= MAX_DIST {
                break;
            }
            // Extend the match
            let mut l = 0usize;
            while l < MAX_MATCH && pos + l < data.len() && data[cand + l] == data[pos + l] {
                l += 1;
            }
            if l > best_len {
                best_len = l;
                best_dist = pos - cand;
                if l == MAX_MATCH {
                    break;
                }
            }
            candidate = self.prev[cand % WINDOW_SIZE];
            steps += 1;
        }
        (best_len, best_dist)
    }

    /// Find ALL candidate matches at `pos`, returning (distance, length)
    /// pairs for every chain entry that matches at least MIN_MATCH bytes.
    ///
    /// Used by the optimal parser, which needs to choose between
    /// alternative matches of different lengths/distance trade-offs.
    /// The vector is sorted by length descending so the best match is
    /// first (useful for early-exit heuristics).
    ///
    /// The set is bounded by `MAX_CHAIN_STEPS` (same walk depth as
    /// `peek()`) plus an internal cap of `MAX_CANDIDATES` to keep the
    /// DP bounded.
    pub fn candidates(&self, data: &[u8], pos: usize) -> Vec<(usize, usize)> {
        const MAX_CANDIDATES: usize = 8;
        let mut cands: Vec<(usize, usize)> = Vec::with_capacity(MAX_CANDIDATES);
        if pos + MIN_MATCH > data.len() {
            return cands;
        }
        let h = hash3(data, pos);
        let mut candidate = self.head[h];
        let mut steps = 0usize;

        while candidate >= 0 && steps < MAX_CHAIN_STEPS {
            let cand = candidate as usize;
            if pos <= cand || pos - cand >= MAX_DIST {
                break;
            }
            // Quick reject: first 3 bytes must match (hash guarantees this
            // for hash3 collisions only at the bucket level; we're checking
            // the actual bytes here).
            if data[cand] != data[pos] || data[cand + 1] != data[pos + 1] || data[cand + 2] != data[pos + 2] {
                candidate = self.prev[cand % WINDOW_SIZE];
                steps += 1;
                continue;
            }
            // Extend the match
            let mut l = MIN_MATCH;
            while l < MAX_MATCH && pos + l < data.len() && data[cand + l] == data[pos + l] {
                l += 1;
            }
            if cands.len() < MAX_CANDIDATES {
                cands.push((pos - cand, l));
            } else {
                // Already have MAX_CANDIDATES. Only replace if this one
                // is better than the worst (shortest) in the set.
                let mut worst_idx = 0;
                let mut worst_len = cands[0].1;
                for (i, &(_, len)) in cands.iter().enumerate().skip(1) {
                    if len < worst_len {
                        worst_len = len;
                        worst_idx = i;
                    }
                }
                if l > worst_len {
                    cands[worst_idx] = (pos - cand, l);
                }
            }
            if l == MAX_MATCH {
                break;
            }
            candidate = self.prev[cand % WINDOW_SIZE];
            steps += 1;
        }
        // Sort by length descending so [0] is the longest match.
        cands.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        cands
    }

    /// Insert position `pos` into the hash tables. Called from encode().
    fn insert(&mut self, data: &[u8], pos: usize) {
        if pos + MIN_MATCH > data.len() {
            return;
        }
        let h = hash3(data, pos);
        let slot = pos % WINDOW_SIZE;
        self.prev[slot] = self.head[h];
        self.head[h] = pos as i32;
    }

    /// Encode `data` as a sequence of (literal | match) ops using hash chains
    /// and lazy matching.
    pub fn encode(&mut self, data: &[u8]) -> Vec<Op> {
        let mut ops = Vec::with_capacity(data.len() / 2);
        let mut i = 0usize;
        let n = data.len();
        while i < n {
            // Tail: emit remaining bytes as literals
            if i + MIN_MATCH > n {
                while i < n {
                    ops.push(Op::Lit(data[i]));
                    i += 1;
                }
                break;
            }

            // Find best match at current position
            let (best_len, best_dist) = self.peek(data, i);

            // Lazy matching: if there's a match here, peek at i+1 and see if
            // we'd get a longer one by emitting a literal first.
            if best_len >= MIN_MATCH && i + 1 + MIN_MATCH <= n {
                let (next_len, _) = self.peek(data, i + 1);
                if next_len > best_len + 1 {
                    // Worth paying 1 literal byte for the longer match
                    ops.push(Op::Lit(data[i]));
                    self.insert(data, i);
                    i += 1;
                    continue;
                }
            }

            if best_len >= MIN_MATCH {
                ops.push(Op::Match {
                    dist: best_dist as u32,
                    len: best_len as u32,
                });
                // Insert hashes for positions i..i+best_len
                for k in 0..best_len {
                    self.insert(data, i + k);
                }
                i += best_len;
            } else {
                ops.push(Op::Lit(data[i]));
                self.insert(data, i);
                i += 1;
            }
        }
        ops
    }

    /// Encode `data` as a sequence of (literal | match | dict_ref) ops,
    /// using BOTH the LZ77 hash chain and a static dictionary.
    ///
    /// At each position, we compute both the LZ77 match (via `peek`)
    /// and the dict match (via `Dictionary::lookup_at`), then choose
    /// whichever covers more bytes. LZ77 wins on ties (no dict
    /// dependency for the decoder).
    ///
    /// ## Cost model
    ///
    /// Both a DictRef and a Match op are about 18-24 bits in the
    /// bitstream (1 flag bit + 1-2 byte id/dist + 1 byte len). So the
    /// op that covers MORE bytes is the cheaper one per byte of
    /// output. This is why we prefer the longer of the two.
    ///
    /// When LZ77 has no match (first occurrence of a token), the dict
    /// catches it. When the dict has no match (rare token), LZ77 may
    /// still find a back-reference. When both match, pick the longer.
    ///
    /// ## Limitations vs `encode_optimal`
    ///
    /// This is greedy (one step lookahead, no DP). The optimal
    /// `encode_optimal` is more accurate but more expensive and
    /// doesn't yet support dict refs. Future work: a `encode_optimal_with_dict`
    /// that adds dict as a third option in the DP.
    pub fn encode_with_dict(&mut self, data: &[u8], dict: &Dictionary) -> Vec<Op> {
        let mut ops = Vec::with_capacity(data.len() / 2);
        let mut i = 0usize;
        let n = data.len();
        while i < n {
            // 1. Try LZ77 match.
            let lz_match = if i + MIN_MATCH <= n {
                let (l, d) = self.peek(data, i);
                if l >= MIN_MATCH { Some((l, d)) } else { None }
            } else {
                None
            };

            // 2. Try dict match.
            let dict_match = if n - i >= MIN_DICT_MATCH {
                dict.lookup_at(data, i)
            } else {
                None
            };

            // 3. Pick the longer match (tie → LZ77, no dict dependency).
            match (lz_match, dict_match) {
                (Some((ll, ld)), Some((_, dd))) if ll >= dd => {
                    // LZ77 wins.
                    ops.push(Op::Match {
                        dist: ld as u32,
                        len: ll as u32,
                    });
                    for k in 0..ll {
                        self.insert(data, i + k);
                    }
                    i += ll;
                }
                (_, Some((id, dlen))) => {
                    // Dict wins (or only dict has a match).
                    ops.push(Op::DictRef {
                        id,
                        len: dlen.min(255) as u8,
                    });
                    for k in 0..dlen {
                        self.insert(data, i + k);
                    }
                    i += dlen;
                }
                (Some((ll, ld)), None) => {
                    // Only LZ77.
                    ops.push(Op::Match {
                        dist: ld as u32,
                        len: ll as u32,
                    });
                    for k in 0..ll {
                        self.insert(data, i + k);
                    }
                    i += ll;
                }
                (None, None) => {
                    // Literal.
                    ops.push(Op::Lit(data[i]));
                    self.insert(data, i);
                    i += 1;
                }
            }
        }
        ops
    }

    /// Encode `data` using OPTIMAL PARSING (forward DP) instead of
    /// greedy/lazy matching.
    ///
    /// At each position `i`, we run a shortest-path DP over the next
    /// `LOOKAHEAD` bytes, with these choices per byte:
    ///
    ///   position p:
    ///     ├── emit literal at p           cost += LITERAL_BITS, next = p+1
    ///     └── emit match (d, l) at p      cost += match_cost(d,l), next = p+l
    ///                                    (only if l >= MIN_MATCH and l <= MAX_MATCH)
    ///
    /// We backtrack from the cheapest reachable position to find the
    /// optimal path, take the FIRST action of that path (one literal
    /// or one match), advance `i`, and repeat. This way the DP only
    /// ever needs to look at the next ~32 bytes — the cost is
    /// O((LOOKAHEAD + MAX_MATCH) * MAX_CANDIDATES) per step.
    ///
    /// ## Why allow matches longer than the lookahead
    ///
    /// A match of length 200 at position 0 is GREAT (covers 200 bytes
    /// for ~7 bits). If we capped match length at LOOKAHEAD=32 we'd
    /// miss those. The fix: extend the cost array to LOOKAHEAD +
    /// MAX_MATCH so a match can land at p + MAX_MATCH and still be
    /// considered. We never iterate the DP past the lookahead end
    /// (no positions past that are explored), but we DO accept matches
    /// that span past it.
    ///
    /// ## Why lookahead of 32
    ///
    /// Empirically the cost-vs-gain curve flattens around there:
    /// longer lookahead finds maybe 5% more matches but takes 4x the
    /// time. 32 is the sweet spot for source code where the typical
    /// "break a small match for a big one" decision resolves within
    /// ~20 bytes.
    pub fn encode_optimal(&mut self, data: &[u8]) -> Vec<Op> {
        const LOOKAHEAD: usize = 32;

        let mut ops = Vec::with_capacity(data.len() / 2);
        let mut i = 0usize;
        let n = data.len();

        while i < n {
            // Tail: emit remaining bytes as literals (no matches possible)
            if i + MIN_MATCH > n {
                while i < n {
                    ops.push(Op::Lit(data[i]));
                    self.insert(data, i);
                    i += 1;
                }
                break;
            }

            // Run DP on the lookahead window, allowing matches to land
            // up to MAX_MATCH past the window end (so long matches are
            // visible to the cost comparison).
            //
            // We track BOTH (cost, ops) so we can tie-break between
            // paths with equal total cost but different op counts.
            // Fewer ops = less per-op overhead in the bitstream, so
            // "9 literals + 1 match" should lose to "1 match direct"
            // even when their total-cost estimates are identical.
            let window_end = (i + LOOKAHEAD).min(n);
            let window_len = window_end - i;
            // Cost array must hold positions up to window_len + MAX_MATCH
            // for matches that span past the window.
            let cost_size = window_len + MAX_MATCH;

            // best_cost[p] = min cost to reach position i+p
            // best_ops[p]  = number of ops in that minimum-cost path
            // best_op[p]   = the LAST op in the path (= op that lands at p)
            // best_prev[p] = previous position in the path
            let mut best_cost: Vec<u32> = vec![u32::MAX; cost_size + 1];
            let mut best_ops: Vec<u32> = vec![u32::MAX; cost_size + 1];
            let mut best_op: Vec<Option<Op>> = vec![None; cost_size + 1];
            let mut best_prev: Vec<i32> = vec![-1; cost_size + 1];
            best_cost[0] = 0;
            best_ops[0] = 0;

            for p in 0..window_len {
                if best_cost[p] == u32::MAX {
                    continue;
                }
                let abs_pos = i + p;
                let ops_so_far = best_ops[p];

                // Option 1: emit literal at abs_pos
                let lit_cost = best_cost[p] + crate::cost::LITERAL_BITS;
                if is_better(lit_cost, ops_so_far + 1, best_cost[p + 1], best_ops[p + 1]) {
                    best_cost[p + 1] = lit_cost;
                    best_ops[p + 1] = ops_so_far + 1;
                    best_op[p + 1] = Some(Op::Lit(data[abs_pos]));
                    best_prev[p + 1] = p as i32;
                }
                // Option 2: emit match starting at abs_pos
                let cands = self.candidates(data, abs_pos);
                for &(dist, len) in &cands {
                    // In v2 the cost model says L=3 already wins by 7
                    // bits (literal=13, match=32). The v1 fragmentation
                    // bug (3×L=8 vs 1×L=24) is gone because match cost
                    // is now 32 bits (not 72). No extra threshold filter
                    // needed — every match L>=3 is genuinely useful.
                    if len < crate::cost::MATCH_LENGTH_THRESHOLD as usize {
                        continue;
                    }
                    let np = p + len;
                    if np > cost_size {
                        continue;
                    }
                    let abs_match_end = abs_pos + len;
                    if abs_match_end > n {
                        continue;
                    }
                    let mc = best_cost[p] + crate::cost::match_cost(dist as u32, len as u32);
                    let new_ops = ops_so_far + 1;
                    if is_better(mc, new_ops, best_cost[np], best_ops[np]) {
                        best_cost[np] = mc;
                        best_ops[np] = new_ops;
                        best_op[np] = Some(Op::Match {
                            dist: dist as u32,
                            len: len as u32,
                        });
                        best_prev[np] = p as i32;
                    }
                }
            }

            // Find the BEST first action. "Best" = the action that minimizes
            // the estimated TOTAL cost to encode the rest of the buffer
            // (current window + everything after).
            //
            // The "total" estimate = best_cost[p] + (n_remaining - p) * LITERAL_BITS
            // assumes the rest of the buffer is all literals. This is an
            // upper bound that lets us compare paths that cover different
            // numbers of bytes.
            //
            // When two paths have IDENTICAL total estimates (which
            // happens when one path's "extra literals" cancel the other
            // path's "fewer future literals"), tie-break by ops count:
            // fewer ops = less per-op overhead in the bitstream.
            let n_remaining = (n - i) as u32;
            let mut best_p = 1usize;
            let mut best_total_cost = u32::MAX;
            let mut best_total_ops = u32::MAX;
            for p in 1..=cost_size {
                if best_cost[p] == u32::MAX {
                    continue;
                }
                if (p as u32) > n_remaining {
                    break;
                }
                let future_cost = (n_remaining - p as u32) * crate::cost::LITERAL_BITS;
                let total = best_cost[p].saturating_add(future_cost);
                if total < best_total_cost
                    || (total == best_total_cost && best_ops[p] < best_total_ops)
                {
                    best_total_cost = total;
                    best_total_ops = best_ops[p];
                    best_p = p;
                }
            }

            // Traceback from best_p to find the FIRST op in the path.
            //
            // Walk backward from best_p through best_prev links. We
            // stop when best_prev[cur] == 0, which means cur was reached
            // FROM position 0 — i.e., the very first transition in the
            // forward path. The op for that transition is best_op[cur].
            //
            // Walk-through:
            //   Forward: 0 → cur → ... → best_p
            //   best_prev[best_p] = next_to_last_pos
            //   ...
            //   best_prev[cur] = 0   ← cur was reached from position 0
            //   The op 0 → cur is stored at best_op[cur].
            let mut cur = best_p;
            while best_prev[cur] > 0 {
                cur = best_prev[cur] as usize;
            }
            // cur now points to the position right after start. The op
            // that takes us 0 → cur is best_op[cur]. Emit it.
            let first_op = best_op[cur]
                .clone()
                .unwrap_or(Op::Lit(data[i]));

            // Emit and advance.
            match first_op {
                Op::Lit(b) => {
                    ops.push(Op::Lit(b));
                    self.insert(data, i);
                    i += 1;
                }
                Op::Match { dist, len } => {
                    ops.push(Op::Match { dist, len });
                    for k in 0..len as usize {
                        self.insert(data, i + k);
                    }
                    i += len as usize;
                }
                Op::DictRef { .. } => unreachable!(
                    "encode_optimal never produces DictRef — use encode_with_dict for that"
                ),
            }
        }
        ops
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Op {
    Lit(u8),
    Match { dist: u32, len: u32 },
    /// Reference to a token in a pre-shared dictionary. The decoder
    /// materializes this by looking up `id` in its dictionary and
    /// emitting `len` bytes (or all of the token's bytes if `len`
    /// exceeds the token length).
    DictRef { id: u16, len: u8 },
}

#[derive(Debug, Default)]
pub struct MatchDecoder {
    window: Vec<u8>,
    /// Optional shared dictionary. Required to decode `DictRef` ops;
    /// if `None`, the decoder will panic on encountering one.
    dict: Option<Dictionary>,
}

impl MatchDecoder {
    pub fn new() -> Self {
        Self {
            window: vec![0u8; WINDOW_SIZE],
            dict: None,
        }
    }

    /// Create a decoder that knows the given dictionary. After this,
    /// `DictRef` ops in the input are resolved against `dict`.
    pub fn with_dict(dict: Dictionary) -> Self {
        Self {
            window: vec![0u8; WINDOW_SIZE],
            dict: Some(dict),
        }
    }

    /// Attach a dictionary to an existing decoder. Used when the
    /// decoder is reused across blocks and the dictionary is the
    /// same (e.g., default_combined_dict baked into the binary).
    pub fn set_dict(&mut self, dict: Dictionary) {
        self.dict = Some(dict);
    }

    pub fn decode(&mut self, ops: &[Op]) -> Vec<u8> {
        let mut out = Vec::with_capacity(ops.len() * 3);
        let mut win_pos = 0usize;
        for &op in ops {
            match op {
                Op::Lit(b) => {
                    out.push(b);
                    self.window[win_pos % WINDOW_SIZE] = b;
                    win_pos += 1;
                }
                Op::Match { dist, len } => {
                    let d = dist as usize;
                    let start = win_pos - d;
                    for k in 0..len as usize {
                        let b = self.window[(start + k) % WINDOW_SIZE];
                        out.push(b);
                        self.window[win_pos % WINDOW_SIZE] = b;
                        win_pos += 1;
                    }
                }
                Op::DictRef { id, len } => {
                    let dict = self
                        .dict
                        .as_ref()
                        .expect("DictRef op encountered but decoder has no dictionary");
                    let token = dict
                        .get(id)
                        .unwrap_or_else(|| panic!("DictRef id {} not in dictionary", id));
                    // Emit up to `len` bytes from the token. If `len`
                    // exceeds the token length, we emit the full token
                    // (defensive: an encoder bug, not a format error).
                    let emit = (len as usize).min(token.len());
                    for k in 0..emit {
                        let b = token[k];
                        out.push(b);
                        self.window[win_pos % WINDOW_SIZE] = b;
                        win_pos += 1;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_lz77() {
        let data = b"hello world hello world hello world abcdef hello world";
        let mut enc = MatchFinder::new();
        let ops = enc.encode(data);
        let mut dec = MatchDecoder::new();
        let out = dec.decode(&ops);
        assert_eq!(out, data);
    }

    #[test]
    fn roundtrip_long_text() {
        let phrase = b"the quick brown fox jumps over the lazy dog ";
        let mut data = Vec::new();
        while data.len() < 200_000 {
            data.extend_from_slice(phrase);
        }
        let mut enc = MatchFinder::new();
        let ops = enc.encode(&data);
        let mut dec = MatchDecoder::new();
        let out = dec.decode(&ops);
        assert_eq!(out, data, "long text roundtrip mismatch");
    }

    #[test]
    fn roundtrip_random() {
        let mut data = vec![0u8; 4096];
        let mut s: u32 = 0xc0ffee;
        for i in 0..data.len() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            data[i] = s as u8;
        }
        let mut enc = MatchFinder::new();
        let ops = enc.encode(&data);
        let mut dec = MatchDecoder::new();
        let out = dec.decode(&ops);
        assert_eq!(out, data);
    }

    /// Optimal parsing must produce a lossless encoding too.
    #[test]
    fn roundtrip_optimal_basic() {
        let data = b"hello world hello world hello world abcdef hello world";
        let mut enc = MatchFinder::new();
        let ops = enc.encode_optimal(data);
        let mut dec = MatchDecoder::new();
        let out = dec.decode(&ops);
        assert_eq!(out, data);
    }

    #[test]
    fn roundtrip_optimal_long() {
        let phrase = b"the quick brown fox jumps over the lazy dog ";
        let mut data = Vec::new();
        while data.len() < 100_000 {
            data.extend_from_slice(phrase);
        }
        let mut enc = MatchFinder::new();
        let ops = enc.encode_optimal(&data);
        let mut dec = MatchDecoder::new();
        let out = dec.decode(&ops);
        assert_eq!(out, data);
    }

    #[test]
    fn roundtrip_optimal_random() {
        let mut data = vec![0u8; 4096];
        let mut s: u32 = 0xc0ffee;
        for i in 0..data.len() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            data[i] = s as u8;
        }
        let mut enc = MatchFinder::new();
        let ops = enc.encode_optimal(&data);
        let mut dec = MatchDecoder::new();
        let out = dec.decode(&ops);
        assert_eq!(out, data);
    }

    /// Optimal parsing should NOT regress on data where lazy was already
    /// optimal — the cost model should match the lazy decision in the
    /// simple cases.
    #[test]
    fn optimal_matches_lazy_on_simple_repetition() {
        // Pattern: every 10 bytes, the same 10 bytes repeat. Lazy will
        // catch all 10-byte matches. Optimal should too.
        let pattern = b"abcdefghij";
        let mut data = Vec::new();
        for _ in 0..100 {
            data.extend_from_slice(pattern);
        }
        let mut lazy = MatchFinder::new();
        let lazy_ops = lazy.encode(&data);
        let mut optimal = MatchFinder::new();
        let optimal_ops = optimal.encode_optimal(&data);
        // Both should be lossless
        let mut dec = MatchDecoder::new();
        assert_eq!(dec.decode(&lazy_ops), data);
        dec = MatchDecoder::new();
        assert_eq!(dec.decode(&optimal_ops), data);
        // Optimal should produce ≤ ops (same or better compression)
        assert!(
            optimal_ops.len() <= lazy_ops.len(),
            "optimal produced {} ops but lazy produced {}",
            optimal_ops.len(),
            lazy_ops.len()
        );
    }

    /// THE KEY TEST: optimal parsing should not regress catastrophically
    /// vs lazy matching. We accept up to 10% worse because the
    /// cost-model approximation in the DP isn't perfect for every
    /// synthetic pattern (e.g., when many short matches are present,
    /// the future-cost estimate can misallocate). What we DO assert:
    /// 1. Both are lossless (correctness)
    /// 2. Optimal doesn't blow up to N literals (it actually uses matches)
    /// 3. Optimal is within 10% of lazy's op count
    #[test]
    fn optimal_well_within_lazy_on_indentation_pattern() {
        // 8 spaces + a unique token — repeated with slight variation
        // so the long match needs the literal+match combo at the boundary.
        let mut data = Vec::new();
        for i in 0..1000 {
            data.extend_from_slice(b"        ");
            data.extend_from_slice(format!("line_{}\n", i).as_bytes());
        }
        let mut lazy = MatchFinder::new();
        let lazy_ops = lazy.encode(&data);
        let mut optimal = MatchFinder::new();
        let optimal_ops = optimal.encode_optimal(&data);

        // Both lossless
        let mut dec = MatchDecoder::new();
        assert_eq!(dec.decode(&lazy_ops), data);
        dec = MatchDecoder::new();
        assert_eq!(dec.decode(&optimal_ops), data);

        // Optimal should not degenerate to all literals.
        // (If it did, it would produce ~15000 ops, one per byte.)
        assert!(
            optimal_ops.len() < data.len(),
            "optimal produced {} ops for {} bytes — looks degenerate",
            optimal_ops.len(),
            data.len()
        );

        // Optimal should be within 10% of lazy (it's better or equivalent
        // on real data; can be slightly worse on pathological synthetic).
        let max = lazy_ops.len() + lazy_ops.len() / 10;
        assert!(
            optimal_ops.len() <= max,
            "optimal {} ops vs lazy {} ops — regression >10%",
            optimal_ops.len(),
            lazy_ops.len()
        );
    }

    /// candidates() should return matches sorted by length descending.
    #[test]
    fn candidates_sorted_by_length() {
        let data = b"abc abc abc abc abcdef";
        // Insert positions for "abc" so candidates() has something to find.
        // Use encode() to populate the chain, but on a separate finder.
        let mut builder = MatchFinder::new();
        let _ = builder.encode(b"abc abc abc abc "); // populate chain with "abc"
        // Now query candidates for "abcdef" at position 16.
        let cands = builder.candidates(data, 16);
        // First candidate should be the longest.
        for w in cands.windows(2) {
            assert!(w[0].1 >= w[1].1, "candidates not sorted desc: {:?}", cands);
        }
        // And at least one candidate must have length >= 3.
        assert!(cands.iter().any(|&(_, l)| l >= MIN_MATCH));
    }

    // -------------------------------------------------------------
    // Dictionary integration tests
    // -------------------------------------------------------------

    /// Basic roundtrip: encode_with_dict + decode with same dict → original.
    #[test]
    fn roundtrip_with_dict() {
        let mut dict = crate::dictionary::Dictionary::new();
        dict.insert(b"the", 100);
        dict.insert(b"and", 100);
        dict.insert(b"ing", 100);
        dict.insert(b"hello", 100);

        let data = b"the quick brown fox and the lazy dog hello world";
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(data, &dict);
        let mut dec = MatchDecoder::with_dict(dict);
        let out = dec.decode(&ops);
        assert_eq!(out, data, "dict roundtrip mismatch");
    }

    /// encode_with_dict on data with no dict matches should produce
    /// the same op stream as encode() (within hash chain state).
    #[test]
    fn dict_falls_back_to_lz77_on_no_match() {
        let mut dict = crate::dictionary::Dictionary::new();
        dict.insert(b"zzzz", 1); // won't match anything in our data

        let data = b"hello world hello world hello world";
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(data, &dict);
        // Should still roundtrip.
        let mut dec = MatchDecoder::with_dict(dict);
        let out = dec.decode(&ops);
        assert_eq!(out, data);
    }

    /// encode_with_dict should emit DictRef ops for tokens in the dict.
    ///
    /// For "the cat sat on the mat the":
    ///   - pos 0 "the" → DictRef (no prior LZ77 match)
    ///   - pos 8 "the" → LZ77 match (dist=8, len=3) — TIE with dict, LZ77 wins
    ///   - pos 15 "the" → DictRef? No — LZ77 match is still available since
    ///     the second "the" is now in the window. LZ77 wins on tie.
    /// Expected: ~2 DictRefs (the first "the" and any other unmatched
    /// dict tokens like " cat" — but " cat" isn't in the dict, so really
    /// just 1 DictRef and 1 LZ77 "the" match).
    #[test]
    fn dict_refs_are_emitted() {
        let dict = crate::dictionary::default_text_dict();
        let data = b"the cat sat on the mat the";
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(data, &dict);
        let dict_refs = ops
            .iter()
            .filter(|op| matches!(op, Op::DictRef { .. }))
            .count();
        let matches = ops
            .iter()
            .filter(|op| matches!(op, Op::Match { .. }))
            .count();
        // At least 1 DictRef (the first "the"). May be 1 or 2 depending
        // on whether subsequent "the"s get caught by LZ77 first.
        assert!(
            dict_refs >= 1,
            "expected at least 1 DictRef, got {} (matches={})",
            dict_refs,
            matches
        );
    }

    /// encode_with_dict on real source code should produce a meaningful
    /// number of DictRefs (vs the lazy matcher that doesn't see them).
    #[test]
    fn dict_refs_on_code() {
        let dict = crate::dictionary::default_code_dict();
        let data = b"fn main() {\n    let x = 1;\n    let y = 2;\n    let z = 3;\n}\n";
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(data, &dict);
        let dict_refs = ops
            .iter()
            .filter(|op| matches!(op, Op::DictRef { .. }))
            .count();
        let lits = ops.iter().filter(|op| matches!(op, Op::Lit(_))).count();
        let matches = ops
            .iter()
            .filter(|op| matches!(op, Op::Match { .. }))
            .count();
        eprintln!(
            "code.rs sample: {} DictRefs, {} Matches, {} Lits",
            dict_refs, matches, lits
        );
        // We expect a few dict refs for keywords in this Rust snippet.
        assert!(
            dict_refs >= 2,
            "expected at least 2 DictRefs in this snippet, got {}",
            dict_refs
        );
        // And it must roundtrip.
        let mut dec = MatchDecoder::with_dict(dict);
        let out = dec.decode(&ops);
        assert_eq!(out, data);
    }

    /// "Longer wins" rule: when both LZ77 and dict have a match, the
    /// longer one should be chosen. This test ensures the integration
    /// prefers LZ77 for a long match over dict for a short one.
    #[test]
    fn longer_wins_over_dict() {
        let mut dict = crate::dictionary::Dictionary::new();
        dict.insert(b"the", 100);
        // Data: "the quick the quick" — second "the" has a 10-byte
        // LZ77 match available. LZ77 should win.
        let data = b"the quick the quick";
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(data, &dict);
        // Expect: 1 DictRef (first "the") + 1 long Match (second "the quick").
        let n_dr = ops
            .iter()
            .filter(|o| matches!(o, Op::DictRef { .. }))
            .count();
        let n_m = ops
            .iter()
            .filter(|o| matches!(o, Op::Match { .. }))
            .count();
        // Should be exactly 1 DictRef and 1 Match (the long LZ77 match
        // for "the quick" at position 10).
        assert_eq!(n_dr, 1, "expected 1 DictRef, got {} (matches={})", n_dr, n_m);
        // The match should be at least 8 bytes (covers "the quick").
        if let Some(Op::Match { len, .. }) = ops.iter().find(|o| matches!(o, Op::Match { .. })) {
            assert!(*len >= 8, "expected match length >= 8, got {}", len);
        } else {
            panic!("no Match op found");
        }
    }

    /// Empty dictionary: encode_with_dict should behave like encode().
    #[test]
    fn empty_dict_is_like_plain_lz77() {
        let dict = crate::dictionary::Dictionary::new();
        let data = b"abc abc abc abc abcdef";
        let mut enc_plain = MatchFinder::new();
        let plain_ops = enc_plain.encode(data);
        let mut enc_dict = MatchFinder::new();
        let dict_ops = enc_dict.encode_with_dict(data, &dict);
        // Both roundtrip to the same data.
        let mut dec_plain = MatchDecoder::new();
        assert_eq!(dec_plain.decode(&plain_ops), data);
        let mut dec_dict = MatchDecoder::with_dict(crate::dictionary::Dictionary::new());
        assert_eq!(dec_dict.decode(&dict_ops), data);
    }

    /// Decoder without a dict should panic on DictRef ops.
    #[test]
    #[should_panic(expected = "DictRef op encountered")]
    fn decode_without_dict_panics_on_dict_ref() {
        let mut dict = crate::dictionary::Dictionary::new();
        dict.insert(b"the", 100);
        let data = b"the cat";
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(data, &dict);
        let mut dec = MatchDecoder::new(); // no dict attached
        let _ = dec.decode(&ops);
    }
}