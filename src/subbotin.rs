//! Byte-aligned range coder (Subbotin-style).
//!
//! Both `low` and `range` are u64. Renorm writes the top byte of low.
//! Decoder reads bytes MSB-first and shifts them into `code`.
//!
//! Stream layout:
//!   [preload: 4 bytes of 0x00]   decoder reads these as initial `code`
//!   [renorm bytes]               written by encoder during encoding
//!   [finish bytes: 4]            commits the residual range
//!
//! Key invariant: encoder's `low` and decoder's `code` are mirrors — both
//! represent the encoded message's position. They start at 0 and evolve in
//! lock-step via the renorm bytes.

#![allow(dead_code)]

const TOP: u64 = 0xFFFF_FFFF;
const THRESHOLD: u64 = 1u64 << 31;
const PRELOAD_BYTES: usize = 4;
const FINISH_BYTES: usize = 4;

// ============================================================================
// Frequency table
// ============================================================================

#[derive(Debug, Clone)]
pub struct FreqTable {
    pub cum: Vec<u32>,
    pub total: u32,
}

impl FreqTable {
    pub fn from_counts(counts: &[u32]) -> Self {
        let total_in: u64 = counts.iter().map(|&c| c as u64).sum();
        let scale = (1u64 << 24) - 1;
        let scaled: Vec<u32> = if total_in <= (1u64 << 24) {
            counts.to_vec()
        } else {
            counts
                .iter()
                .map(|&c| ((c as u64 * scale) / total_in).max(1) as u32)
                .collect()
        };
        let mut cum = Vec::with_capacity(scaled.len() + 1);
        cum.push(0u32);
        for c in &scaled {
            cum.push(cum.last().unwrap().wrapping_add(*c));
        }
        let total = *cum.last().unwrap();
        Self { cum, total }
    }

    pub fn uniform(n: u32) -> Self {
        Self::from_counts(&vec![1u32; n as usize])
    }

    pub fn n_symbols(&self) -> u32 {
        (self.cum.len() - 1) as u32
    }

    pub fn lookup(&self, scaled: u32) -> (u32, u32, u32) {
        let n = self.cum.len() - 1;
        for i in 0..n {
            if scaled < self.cum[i + 1] {
                return (i as u32, self.cum[i], self.cum[i + 1]);
            }
        }
        let last = n - 1;
        (last as u32, self.cum[last], self.cum[n])
    }
}

// ============================================================================
// Encoder
// ============================================================================

pub struct Encoder {
    low: u64,
    range: u64,
    bytes: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        let mut e = Self {
            low: 0,
            range: TOP,
            bytes: Vec::new(),
        };
        for _ in 0..PRELOAD_BYTES {
            e.bytes.push(0);
        }
        e
    }

    pub fn encode_symbol(&mut self, table: &FreqTable, sym: u32) {
        let i = sym as usize;
        let cum_low = table.cum[i];
        let cum_high = table.cum[i + 1];
        let freq = cum_high - cum_low;
        let total = table.total;

        let r = self.range / total as u64;
        self.low = self.low.wrapping_add(r * cum_low as u64);
        self.range = r * freq as u64;

        // Renormalize: while range < THRESHOLD, write top byte of low and shift.
        while self.range < THRESHOLD {
            self.bytes.push((self.low >> 56) as u8);
            self.low <<= 8;
            self.range <<= 8;
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        for _ in 0..FINISH_BYTES {
            self.bytes.push((self.low >> 56) as u8);
            self.low <<= 8;
            self.range <<= 8;
        }
        self.bytes
    }
}

// ============================================================================
// Decoder
// ============================================================================

pub struct Decoder<'a> {
    code: u64,
    low: u64,
    range: u64,
    bytes: &'a [u8],
    read_pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(stream: &'a [u8]) -> Self {
        assert!(stream.len() >= PRELOAD_BYTES);
        let mut code: u64 = 0;
        for i in 0..PRELOAD_BYTES {
            code = (code << 8) | (stream[i] as u64);
        }
        Self {
            code,
            low: 0,
            range: TOP,
            bytes: stream,
            read_pos: PRELOAD_BYTES,
        }
    }

    fn read_byte(&mut self) -> u8 {
        let b = if self.read_pos < self.bytes.len() {
            self.bytes[self.read_pos]
        } else {
            0
        };
        self.read_pos += 1;
        b
    }

    pub fn decode_symbol(&mut self, table: &FreqTable) -> u32 {
        let total = table.total;
        let range = self.range;
        // Find sym: scaled = floor((code - low) * total / range)
        // Note: code and low are mirrors of the encoded message.
        let scaled = self
            .code
            .wrapping_sub(self.low)
            .wrapping_mul(total as u64)
            .wrapping_div(range) as u32;
        let (sym, cum_low, cum_high) = table.lookup(scaled);

        // Update low and range.
        let r = range / total as u64;
        self.low = self.low.wrapping_add(r * cum_low as u64);
        self.range = r * (cum_high - cum_low) as u64;

        // Renormalize: read bytes when range < THRESHOLD
        while self.range < THRESHOLD {
            let b = self.read_byte() as u64;
            self.code = (self.code << 8) | b;
            self.range <<= 8;
            self.low <<= 8;
        }
        sym
    }
}

// ============================================================================
// Top-level wrappers
// ============================================================================

pub fn encode_symbols(symbols: &[u32], table: &FreqTable) -> Vec<u8> {
    let mut enc = Encoder::new();
    for &sym in symbols {
        enc.encode_symbol(table, sym);
    }
    enc.finish()
}

pub fn decode_symbols(stream: &[u8], table: &FreqTable, n: usize) -> Vec<u32> {
    let mut dec = Decoder::new(stream);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
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
        assert_eq!(decoded, symbols, "roundtrip mismatch ({})", name);
    }

    #[test]
    fn one_symbol_uniform() {
        roundtrip(&[0], &FreqTable::uniform(1), "1-sym uniform");
    }

    #[test]
    fn five_same_zero() {
        roundtrip(&[0, 0, 0, 0, 0], &FreqTable::uniform(2), "5x sym 0");
    }

    #[test]
    fn five_same_one() {
        roundtrip(&[1, 1, 1, 1, 1], &FreqTable::uniform(2), "5x sym 1");
    }

    #[test]
    fn alternating_two_syms() {
        roundtrip(&[0, 1, 0, 1, 0, 1, 0, 1], &FreqTable::uniform(2), "alternating");
    }

    #[test]
    fn skewed_one_at_end() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut v = vec![0u32; 99];
        v.push(1);
        roundtrip(&v, &table, "99:1 rare at end");
    }

    #[test]
    fn skewed_one_at_start() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut v = vec![1u32];
        v.extend(std::iter::repeat(0).take(99));
        roundtrip(&v, &table, "99:1 rare at start");
    }

    #[test]
    fn skewed_one_in_middle() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut v = vec![0u32; 50];
        v.push(1);
        v.extend(std::iter::repeat(0).take(49));
        roundtrip(&v, &table, "99:1 rare in middle");
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
        roundtrip(&symbols, &table, "1000 random uniform");
    }

    #[test]
    fn long_random_skewed() {
        let table = FreqTable::from_counts(&[100, 30, 10, 5, 2, 1, 1, 1]);
        let total = 150u32;
        let intervals = [
            (0u32, 100u32), (100, 130), (130, 140), (140, 145),
            (145, 147), (147, 148), (148, 149), (149, 150),
        ];
        let mut s: u32 = 0xcafebabe;
        let mut symbols = Vec::with_capacity(500);
        for _ in 0..500 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let v = (s % total) as u32;
            let mut sym = 7u32;
            for (i, &(lo, hi)) in intervals.iter().enumerate() {
                if v >= lo && v < hi {
                    sym = i as u32;
                    break;
                }
            }
            symbols.push(sym);
        }
        roundtrip(&symbols, &table, "500 random skewed");
    }

    #[test]
    fn long_skewed_500() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut symbols = vec![0u32; 499];
        symbols.push(1);
        roundtrip(&symbols, &table, "500 skewed 99:1");
    }
}