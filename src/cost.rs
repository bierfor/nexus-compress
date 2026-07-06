//! Bit-cost model for optimal LZ77 parsing (v2 format).
//!
//! ## The v2 cost model
//!
//! v2 of the .nexus format eliminates the literal placeholder byte and
//! shrinks match (distance, length) encoding to u16/u8. New costs:
//!
//! - `LITERAL_BITS = 13`: 8 (op-flag) + ~5 (rANS-encoded byte).
//!   No placeholder; the literal VALUE lives only in the rANS stream.
//! - `MATCH_BITS = 32`: 8 (flag) + 16 (u16 distance) + 8 (u8 length).
//!
//! ## When does a match beat a literal?
//!
//! - L=3,  D=any:  32/3  = 10.7 bits/byte → wins vs literal 13
//! - L=4,  D=any:  32/4  = 8 bits/byte → big win
//! - L=10, D=any:  32/10 = 3.2 bits/byte → huge
//! - L=100, D=any: 32/100 = 0.32 bits/byte → gigantic
//!
//! `match_beats_literal(length)` = true iff length >= 3.
//!
//! ## v1 → v2: what changed and why it matters
//!
//! v1 had a literal placeholder byte in the op stream that the decoder
//! threw away (it pulled the actual value from the rANS stream instead).
//! That doubled the op-stream cost of every literal. Worse, v1 used u32
//! for distance (16 bits always zero for our 64 KB window) and u32 for
//! length (8 bits always zero for matches under 256 bytes).
//!
//! Combined v1 cost: literal = 21 bits, match = 72 bits. The optimal
//! parser's DP was mathematically correct but its cost model said
//! "L=4 wins by 12 bits" when the actual codec savings for that L=4
//! match were negative (placeholder byte savings didn't compensate the
//! 9-byte match overhead in the op stream). Net: optimal regressed on
//! real source code.
//!
//! v2 collapses both overheads. The DP can now pick L=3 matches safely
//! (wins by 7 bits), and longer matches are far cheaper to emit.
//!
//! ## Distance cost is still fixed (no rANS-encoded distance yet)
//!
//! Distance uses u16 (= max 65535) regardless of value. Same as v1's
//! u32 minus the unused 16 high bits. Real compressors (zstd) rANS-encode
//! distance to penalize far matches — that's a v3 task.

/// Per-literal cost in bits (v2): 8 (op-flag) + ~5 (rANS-encoded byte).
pub const LITERAL_BITS: u32 = 13;

/// Match cost in bits (v2): 8 (flag) + 16 (u16 distance) + 8 (u8 length).
pub const MATCH_BITS: u32 = 32;

/// Minimum match length that beats a literal (v2).
///
/// At L=3: amortized 11 bits/byte vs literal 13 bits/byte. Win by 2.
/// At L=4: amortized 8 bits/byte. Win by 5.
/// At L=5: amortized 6.4 bits/byte. Win by 6.6.
pub const MATCH_LENGTH_THRESHOLD: u32 = 3;

/// Returns true if a match of `length` bytes is cheaper than emitting
/// `length` literals (v2 cost model).
#[inline(always)]
pub fn match_beats_literal(_distance: u32, length: u32) -> bool {
    length >= MATCH_LENGTH_THRESHOLD
}

/// Compute the match cost in bits (always MATCH_BITS in v2).
#[inline(always)]
pub fn match_cost(_distance: u32, _length: u32) -> u32 {
    MATCH_BITS
}

/// Cost-per-byte of a match.
#[inline(always)]
pub fn match_amortized(_distance: u32, length: u32) -> u32 {
    (MATCH_BITS + length - 1) / length // round-up division
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_baseline() {
        assert_eq!(LITERAL_BITS, 13);
    }

    #[test]
    fn match_cost_is_fixed() {
        assert_eq!(match_cost(10, 10), 32);
        assert_eq!(match_cost(60_000, 3), 32);
    }

    #[test]
    fn match_length_3_beats_literal() {
        // 32/3 = 10.67 < 13.
        assert!(match_beats_literal(100, 3));
    }

    #[test]
    fn match_length_2_loses_to_literal() {
        // 32/2 = 16 > 13.
        assert!(!match_beats_literal(100, 2));
    }

    #[test]
    fn long_match_always_beats_literal() {
        assert!(match_beats_literal(65_535, 100));
    }

    #[test]
    fn amortized_decreases_with_length() {
        assert!(match_amortized(100, 100) < match_amortized(100, 4));
        assert!(match_amortized(100, 4) < match_amortized(100, 3));
    }

    #[test]
    fn threshold_is_3() {
        assert_eq!(MATCH_LENGTH_THRESHOLD, 3);
    }

    /// Sanity: in v2, the 3xL=8 fragmentation that bit v1 should
    /// NOT happen here. 1×L=24 costs 32 bits, 3×L=8 costs 96 bits.
    /// Same byte coverage, 64 bits saved by picking the longer match.
    /// The DP must prefer 1×L=24.
    #[test]
    fn v2_avoids_fragmentation() {
        // 1 match of L=24: cost 32 bits
        // 3 matches of L=8: cost 96 bits
        // Long match is 64 bits cheaper.
        let one_long = match_cost(0, 24);
        let three_short = match_cost(0, 8) * 3;
        assert!(one_long < three_short);
    }
}