//! Byte-aligned arithmetic coder, written from the Schindler/Mark Nelson spec.
//!
//! Three-register model:
//!   - Encoder: `(low, high, range)` where range = high - low + 1.
//!   - Decoder: `(low, high, code)` where code is the "number" the encoder
//!              committed to so far.
//!   - Both sides share the same `(low, high, range)` after each symbol.
//!
//! Precision: 32 bits with a half = `1 << 31` and quarter = `1 << 30`.
//! Renormalize when the range straddles the half (top bit differs).
//! Bit-follow handles the "follow" bits when range collapses to a region
//! where the bit is the inverse of what just got pushed out.
//!
//! Stream layout:
//!   [preamble: 32 bits = code register initial value, MSB first]
//!   [renorm bits, MSB first, packed into bytes]
//!
//! Size note: the preamble is essential so the decoder can start decoding
//! from the BEGINNING of the message. Without it, the decoder wouldn't
//! know which bits correspond to which renorm cycle.

#![allow(dead_code)]

/// Top of the range (inclusive). The range `low`/`high` lives in `[0, TOP_VALUE]`.
const TOP_VALUE: u32 = 0xFFFF_FFFF;
/// Half of the range. If `low` and `high` straddle this, top bit is fixed
/// and can be emitted.
const HALF: u32 = 1u32 << 31;
/// Quarter of the range. Used to detect "follow" cases.
const QUARTER: u32 = 1u32 << 30;

// ============================================================================
// Frequency table
// ============================================================================

/// Cumulative frequency table for the arithmetic coder.
pub struct FreqTable {
    /// `cum[i]` is the cumulative count for symbol `i`. `cum[0] = 0`.
    /// `cum[n] = total`.
    pub cum: Vec<u32>,
    /// Total frequency (sum of all counts).
    pub total: u32,
}

impl FreqTable {
    /// Build from per-symbol counts (all must be >= 1).
    pub fn from_counts(counts: &[u32]) -> Self {
        let total: u64 = counts.iter().map(|&c| c as u64).sum();
        // Cap total so range arithmetic doesn't overflow.
        let target_total = if total <= (1u64 << 24) {
            total
        } else {
            (1u64 << 24) - 1
        };

        let mut cum = Vec::with_capacity(counts.len() + 1);
        cum.push(0);
        let mut acc: u64 = 0;
        for &c in counts {
            let scaled = if total <= (1u64 << 24) {
                c as u64
            } else {
                let v = (c as u64 * target_total) / total;
                v.max(1)
            };
            acc += scaled;
            cum.push(acc as u32);
        }
        let final_total = if total <= (1u64 << 24) {
            total as u32
        } else {
            *cum.last().unwrap()
        };
        Self { cum, total: final_total }
    }

    /// Build a uniform table over `n` symbols (each frequency = 1).
    pub fn uniform(n: u32) -> Self {
        let counts = vec![1u32; n as usize];
        Self::from_counts(&counts)
    }

    pub fn n_symbols(&self) -> u32 {
        (self.cum.len() - 1) as u32
    }

    /// Look up the symbol whose interval contains `count` (which must be
    /// in `[0, total)`).
    pub fn lookup(&self, count: u32) -> (u32, u32, u32) {
        let n = self.cum.len() - 1;
        for i in 0..n {
            if count < self.cum[i + 1] {
                return (i as u32, self.cum[i], self.cum[i + 1]);
            }
        }
        // Shouldn't reach.
        let last = n - 1;
        (last as u32, self.cum[last], self.cum[n])
    }
}

// ============================================================================
// Bit writer/reader (MSB-first, packed into bytes)
// ============================================================================

struct BitWriter {
    buf: Vec<u8>,
    cur: u8,
    nbits: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self { buf: Vec::new(), cur: 0, nbits: 0 }
    }

    fn push_bit(&mut self, bit: u8) {
        self.cur = (self.cur << 1) | (bit & 1);
        self.nbits += 1;
        if self.nbits == 8 {
            self.buf.push(self.cur);
            self.cur = 0;
            self.nbits = 0;
        }
    }

    /// Pad to byte boundary with zeros and return the bytes (without the
    /// partial byte).
    fn finalize(self) -> Vec<u8> {
        self.buf
    }

    /// Like finalize but also flushes any partial byte (zero-padded).
    fn finalize_padded(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.cur <<= 8 - self.nbits;
            self.buf.push(self.cur);
        }
        self.buf
    }

    fn nbits_written(&self) -> u32 {
        (self.buf.len() as u32) * 8 + self.nbits as u32
    }
}

// ============================================================================
// RangeEncoder — produces a byte stream from a sequence of symbols.
// ============================================================================

pub struct RangeEncoder {
    low: u32,
    high: u32,
    bits: BitWriter,
    /// Number of pending "follow" bits.
    bits_to_follow: u32,
}

impl RangeEncoder {
    pub fn new() -> Self {
        Self {
            low: 0,
            high: TOP_VALUE,
            bits: BitWriter::new(),
            bits_to_follow: 0,
        }
    }

    /// Encode a symbol.
    pub fn encode_symbol(&mut self, cum_low: u32, cum_high: u32, total: u32) {
        debug_assert!(cum_low < cum_high);
        debug_assert!(total > 0);

        let r = ((self.high as u64 - self.low as u64) + 1) / (total as u64);
        // new_high = low + r * cum_high - 1     (subtract 1 because inclusive)
        // new_low  = low + r * cum_low
        let new_high = (self.low as u64) + r * (cum_high as u64) - 1;
        let new_low = (self.low as u64) + r * (cum_low as u64);
        self.high = new_high as u32;
        self.low = new_low as u32;

        self.renormalize();
    }

    /// Renormalize `low` and `high` and emit follow bits.
    /// NOTE: This uses a SIMPLIFIED condition (`low >= HALF` rather than
    /// the standard `low >= HALF + QUARTER`). The simpler condition works
    /// for some test cases but breaks for others. Keeping it as a
    /// starting point — see README for what needs to be fixed.
    fn renormalize(&mut self) {
        loop {
            if self.high < HALF {
                // Top bit of (low, high) is 0. Emit 0 + follow.
                self.output_bit_plus_follow(0);
                self.low <<= 1;
                self.high = (self.high << 1) | 1;
            } else if self.low >= HALF {
                // Top bit is 1. Emit 1 + follow.
                self.output_bit_plus_follow(1);
                self.low = (self.low - HALF) << 1;
                self.high = ((self.high - HALF) << 1) | 1;
            } else if self.low >= QUARTER && self.high < HALF + QUARTER {
                // Stuck in the middle.
                self.bits_to_follow += 1;
                self.low = (self.low - QUARTER) << 1;
                self.high = ((self.high - QUARTER) << 1) | 1;
            } else {
                break;
            }
        }
    }

    /// Output the given bit plus all pending follow bits (as the inverse).
    fn output_bit_plus_follow(&mut self, bit: u8) {
        self.bits.push_bit(bit & 1);
        while self.bits_to_follow > 0 {
            self.bits.push_bit((bit & 1) ^ 1);
            self.bits_to_follow -= 1;
        }
    }

    /// Finalize. Emits the boundary bit + any pending follow bits, then
    /// pads with bits from the residual range so the decoder can read
    /// CODE_BITS bits for its initial code register.
    pub fn finish(mut self) -> Vec<u8> {
        const CODE_BITS: u32 = 32;
        // Mark Nelson's flush: increment bits_to_follow by 1, then emit
        // boundary + all pending follow bits.
        self.bits_to_follow += 1;
        let final_bit: u8 = if self.low < HALF { 0u8 } else { 1u8 };
        self.output_bit_plus_follow(final_bit);
        // Now pad with bits from `low` so the decoder's init can read
        // CODE_BITS bits. We pad with the top bit of `low` (the same one
        // already committed by the encoder's renorm cycles).
        let written = self.bits.nbits_written();
        let mut to_add = CODE_BITS.saturating_sub(written);
        while to_add > 0 {
            let b = ((self.low >> 31) & 1) as u8;
            self.bits.push_bit(b);
            self.low = self.low.wrapping_shl(1);
            to_add -= 1;
        }
        self.bits.finalize_padded()
    }
}

// ============================================================================
// RangeDecoder
// ============================================================================

pub struct RangeDecoder<'a> {
    low: u32,
    high: u32,
    /// The "decoded number so far", initialized from the 4-byte preamble.
    code: u32,
    /// The bit stream (after the preamble).
    bytes: &'a [u8],
    /// Bit position in `bytes`.
    bit_pos: u32,
}

impl<'a> RangeDecoder<'a> {
    pub fn new(stream: &'a [u8]) -> Self {
        assert!(stream.len() >= 4, "stream must contain at least a 4-byte preamble");
        let code = u32::from_be_bytes([stream[0], stream[1], stream[2], stream[3]]);
        Self {
            low: 0,
            high: TOP_VALUE,
            code,
            bytes: &stream[4..],
            bit_pos: 0,
        }
    }

    fn read_bit(&mut self) -> u8 {
        let byte_idx = (self.bit_pos / 8) as usize;
        let bit_in_byte = 7 - (self.bit_pos % 8);
        let b = if byte_idx < self.bytes.len() {
            (self.bytes[byte_idx] >> bit_in_byte) & 1
        } else {
            0
        };
        self.bit_pos += 1;
        b
    }

    /// Decode the next symbol. Returns the symbol ID.
    pub fn decode_symbol(&mut self, table: &FreqTable) -> u32 {
        let total = table.total;
        let range = (self.high as u64 - self.low as u64) + 1;
        // count = (code - low) * total / range, clamped to fit in [0, total)
        let count = ((self.code as u64).saturating_sub(self.low as u64))
            .saturating_mul(total as u64) / range;
        let count = count.min((total as u64) - 1) as u32;

        let (sym, cum_low, cum_high) = table.lookup(count);
        let r = range / (total as u64);
        self.high = (self.low as u64 + r * (cum_high as u64) - 1) as u32;
        self.low = (self.low as u64 + r * (cum_low as u64)) as u32;

        self.renormalize();
        sym
    }

    fn renormalize(&mut self) {
        loop {
            if self.high < HALF {
                // Top bit is 0. Shift up and read new bit at the bottom.
                let b = self.read_bit() as u32;
                self.code = (self.code << 1) | b;
                self.low <<= 1;
                self.high = (self.high << 1) | 1;
            } else if self.low >= HALF {
                // Top bit is 1. Subtract HALF, shift up, set top bit back to 1.
                let b = self.read_bit() as u32;
                self.code = ((self.code - HALF) << 1) | b | HALF;
                self.low = (self.low - HALF) << 1;
                self.high = ((self.high - HALF) << 1) | 1 | HALF;
            } else if self.low >= QUARTER && self.high < HALF + QUARTER {
                // Stuck in the middle.
                let b = self.read_bit() as u32;
                self.code = ((self.code - QUARTER) << 1) | b;
                self.low = (self.low - QUARTER) << 1;
                self.high = ((self.high - QUARTER) << 1) | 1;
            } else {
                break;
            }
        }
    }
}

// ============================================================================
// Top-level encode/decode wrappers.
// ============================================================================

/// Encode a sequence of symbol IDs (each in `0..n_symbols`) using the table.
/// Returns a byte stream that the decoder can roundtrip from.
pub fn encode_symbols(symbols: &[u32], table: &FreqTable) -> Vec<u8> {
    let mut enc = RangeEncoder::new();
    for &sym in symbols {
        let i = sym as usize;
        enc.encode_symbol(table.cum[i], table.cum[i + 1], table.total);
    }
    enc.finish()
}

/// Decode a stream produced by `encode_symbols`.
pub fn decode_symbols(stream: &[u8], table: &FreqTable, n_symbols: usize) -> Vec<u32> {
    let mut dec = RangeDecoder::new(stream);
    let mut out = Vec::with_capacity(n_symbols);
    for _ in 0..n_symbols {
        out.push(dec.decode_symbol(table));
    }
    out
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(symbols: &[u32], table: &FreqTable, name: &str) {
        let stream = encode_symbols(symbols, table);
        let decoded = decode_symbols(&stream, table, symbols.len());
        if decoded != symbols {
            // Locate first divergence for clearer diagnostics.
            for (i, (&a, &b)) in symbols.iter().zip(decoded.iter()).enumerate() {
                if a != b {
                    panic!(
                        "roundtrip mismatch ({}): first diff at index {}\n  expected: {:?}\n  got:      {:?}\n  prefix_ok: {:?}",
                        name, i, &symbols[..i.min(symbols.len())], &decoded[..i.min(decoded.len())],
                        symbols[..i] == decoded[..i]
                    );
                }
            }
            panic!("roundtrip mismatch ({}): full streams diverge", name);
        }
    }

    #[test]
    fn one_symbol_uniform() {
        let table = FreqTable::uniform(1);
        roundtrip(&[0], &table, "1-sym uniform");
    }

    #[test]
    fn five_same_symbol() {
        let table = FreqTable::uniform(2);
        roundtrip(&[0, 0, 0, 0, 0], &table, "5x sym 0");
    }

    #[test]
    fn two_syms_uniform_high() {
        let table = FreqTable::uniform(2);
        roundtrip(&[1, 1, 1, 1], &table, "all 1");
    }

    #[test]
    fn alternating_two_syms() {
        let table = FreqTable::uniform(2);
        roundtrip(&[0, 1, 0, 1, 0, 1, 0, 1], &table, "alternating 2-sym");
    }

    #[test]
    fn skewed_99_to_1_at_end() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut symbols = vec![0u32; 99];
        symbols.push(1);
        roundtrip(&symbols, &table, "skewed 99:1, rare at end");
    }

    #[test]
    fn skewed_99_to_1_at_start() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut symbols = vec![1u32];
        symbols.extend(std::iter::repeat(0).take(99));
        roundtrip(&symbols, &table, "skewed 99:1, rare at start");
    }

    #[test]
    fn skewed_99_to_1_in_middle() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut symbols = vec![0u32; 50];
        symbols.push(1);
        symbols.extend(std::iter::repeat(0).take(49));
        roundtrip(&symbols, &table, "skewed 99:1, rare in middle");
    }

    #[test]
    fn long_skewed_500_symbols() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut symbols = vec![0u32; 499];
        symbols.push(1);
        roundtrip(&symbols, &table, "500-symbol skewed 99:1");
    }

    #[test]
    fn long_random_uniform() {
        let table = FreqTable::uniform(8);
        let mut s: u32 = 0xdeadbeef;
        let mut symbols = Vec::with_capacity(1000);
        for _ in 0..1000 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            symbols.push(s % 8);
        }
        roundtrip(&symbols, &table, "1000 random uniform 8-sym");
    }

    #[test]
    fn long_random_skewed() {
        let table = FreqTable::from_counts(&[100, 30, 10, 5, 2, 1, 1, 1]);
        let total = 150u32;
        let mut s: u32 = 0xcafebabe;
        let mut symbols = Vec::with_capacity(500);
        let cum_intervals = [
            (0u32, 100u32),
            (100, 130),
            (130, 140),
            (140, 145),
            (145, 147),
            (147, 148),
            (148, 149),
            (149, 150),
        ];
        for _ in 0..500 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let v = (s % total) as u32;
            let mut sym = 7u32;
            for (i, &(lo, hi)) in cum_intervals.iter().enumerate() {
                if v >= lo && v < hi {
                    sym = i as u32;
                    break;
                }
            }
            symbols.push(sym);
        }
        roundtrip(&symbols, &table, "500 random skewed 8-sym");
    }
}
