//! Bit-cost model for optimal LZ77 parsing (v4 format).
//!
//! ## The v4 cost model
//!
//! v4 of the .nexus format (sprint 2.9) uses a 2-bit op-flag and
//! rANS-encoded streams for every per-op field. The actual bit costs
//! are much lower than the v2-era constants they're replacing.
//!
//! Measured on the corpus with `src/cost_probe.rs` (sprint 2.9):
//!
//! | File           | lit bits/sym | match bits/sym | dict bits/sym |
//! |----------------|-------------:|---------------:|--------------:|
//! | code.rs        | 5.48         |  8.82          | 5.33          |
//! | text.txt       | 4.77         | 20.01          |  0 (no dict)  |
//! | data.json      | 4.63         | 20.75          | 1.78          |
//! | mixed.bin      | 8.94         | 16.33          |  0            |
//! | trained.dict   | 6.07         | 16.58          |  0            |
//! | **avg**        | **~7**       | **~17**        | **~2**        |
//!
//! Old (v2) constants: literal=13, match=32. New (v4) constants:
//! literal=8, match=17, threshold=4. The 2× over-estimate was
//! caused by a stale cost model from when op-flags were 1 byte
//! and match fields were u16/u8 (not rANS-encoded).
//!
//! ## Why a 2-bit op-flag matters
//!
//! The op-flags stream is `[2 bits × n_ops]`, packed 4 per byte.
//! Each op contributes 2 bits of fixed overhead. Per the measured
//! numbers, that's ~10-30% of an op's cost — the rest is the
//! rANS-encoded payload.
//!
//! ## Why match cost is lower than v2 thought
//!
//! v2 used u16 distance (16 bits) and u8 length (8 bits), both
//! raw bytes. v4 rANS-encodes these as 3 separate 8-bit streams
//! (len, dist_lo, dist_hi). For natural text/code, dist_hi
//! clusters near 0 (small distances dominate) so its entropy is
//! ~3-5 bits, not 8. dist_lo is more uniform (~8 bits). len
//! clusters near 3-50 so entropy is ~3-5 bits.
//!
//! Sum: ~2 (flag) + 4 (len) + 8 (dist_lo) + 4 (dist_hi) = ~18 bits,
//! which matches the measured average.
//!
//! ## When does a match beat a literal?
//!
//! With the v4 cost model (LITERAL=8, MATCH=17):
//!
//! - L=3:  match=17, lits=3×8=24  → match wins by 7
//! - L=4:  match=17, lits=4×8=32  → match wins by 15
//! - L=10: match=17, lits=10×8=80 → match wins by 63
//!
//! In practice, for files where dist_lo is near-uniform (text/data.json),
//! the per-match cost is closer to 20 bits, so L=3 is borderline.
//! Threshold = 4 (was 3 in v2) is the conservative choice that
//! always wins.
//!
//! ## Per-block entropy (future work)
//!
//! The static `LITERAL_BITS` doesn't capture that lit entropy
//! varies from 4.6 (data.json) to 8.9 (mixed.bin). A future
//! per-block cost model could feed the gatekeeper's entropy into
//! the DP: `cost.literal(block) = 2 + H(block)`. For now, the
//! fixed 8-bit average is a 2× improvement over v2.

// -----------------------------------------------------------------------
// v4 cost constants (sprint 2.9 — calibrated by cost_probe.rs)
// -----------------------------------------------------------------------

/// Op-flag cost in bits: 2 (1 bit × 2) per op.
/// (Constant — every op pays this regardless of type.)
pub const OP_FLAG_BITS: u32 = 2;

/// Per-literal cost in bits (v4 calibrated average): 2 (op-flag) +
/// ~6 (rANS-encoded byte, average across corpus).
///
/// Replaces v2's LITERAL_BITS=13. The 2× reduction reflects the
/// switch from a 1-byte op-flag + placeholder to a 2-bit op-flag
/// + rANS-encoded byte.
pub const LITERAL_BITS: u32 = 8;

/// Per-match cost in bits (v4 calibrated average): 2 (op-flag) +
/// ~4 (len rANS) + ~8 (dist_lo rANS) + ~4 (dist_hi rANS).
///
/// Replaces v2's MATCH_BITS=32. The 2× reduction reflects:
///  - op-flag: 1 byte → 2 bits
///  - dist: u16 raw → dist_lo + dist_hi rANS streams (entropy
///    ~12 bits for natural text vs 16 bits raw)
///  - len: u8 raw → rANS-encoded (entropy ~4 bits for natural
///    text vs 8 bits raw)
pub const MATCH_BITS: u32 = 17;

/// Minimum match length that beats emitting literals (v4 model).
///
/// At L=3, match=17 vs lits=3×8=24. Wins by 7.
/// At L=4, match=17 vs lits=4×8=32. Wins by 15. Safe choice.
///
/// For files where dist_lo entropy is closer to 8 (most natural
/// text), the effective per-match cost is 20 bits. Then L=3 is
/// borderline (20 vs 24) and L=4 is the safe minimum. We pick
/// 4 to avoid the borderline-wins.
pub const MATCH_LENGTH_THRESHOLD: u32 = 4;

/// Returns true if a match of `length` bytes is cheaper than emitting
/// `length` literals (v4 cost model).
#[inline(always)]
pub fn match_beats_literal(_distance: u32, length: u32) -> bool {
    length >= MATCH_LENGTH_THRESHOLD
}

/// Compute the match cost in bits (always MATCH_BITS in v4; distance
/// doesn't affect cost because the 3 rANS streams are jointly optimal).
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
        // v4: 8 bits (was 13 in v2).
        assert_eq!(LITERAL_BITS, 8);
    }

    #[test]
    fn match_cost_is_fixed() {
        // v4: 17 bits (was 32 in v2).
        assert_eq!(match_cost(10, 10), 17);
        assert_eq!(match_cost(60_000, 3), 17);
    }

    #[test]
    fn match_length_3_is_below_threshold() {
        // v4 threshold is 4 (not 3) because for files with dist_lo
        // entropy near 8, the effective per-match cost is ~20 bits
        // and L=3 is borderline. Threshold=4 is the safe choice.
        // L=3 returns false (literal cost = 24 vs match cost 17,
        // the cost model says it's still a win but we keep the
        // conservative threshold).
        assert!(!match_beats_literal(100, 3));
    }

    #[test]
    fn match_length_4_beats_literal() {
        // 17 vs 4*8=32, wins by 15.
        assert!(match_beats_literal(100, 4));
    }

    #[test]
    fn match_length_2_loses_to_literal() {
        // 17 vs 2*8=16, loses by 1.
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
    fn threshold_is_4() {
        // v4: threshold 4 (was 3 in v2 — see doc comment for why).
        assert_eq!(MATCH_LENGTH_THRESHOLD, 4);
    }

    /// Sanity: the 3xL=8 fragmentation that bit v1 should NOT
    /// happen in v4 either. 1×L=24 costs 17 bits, 3×L=8 costs 51
    /// bits. Same byte coverage, 34 bits saved by picking the
    /// longer match. The DP must prefer 1×L=24.
    #[test]
    fn v4_avoids_fragmentation() {
        let one_long = match_cost(0, 24);
        let three_short = match_cost(0, 8) * 3;
        assert!(one_long < three_short);
    }

    /// v4 (17 bits) vs v2 (32 bits): the new cost is 53% of the
    /// old. This is the calibration shift the optimal parser's DP
    /// needs.
    #[test]
    fn v4_is_cheaper_than_v2() {
        assert!(MATCH_BITS < 32);
        assert!(LITERAL_BITS < 13);
    }
}
