//! rANS wrapper using the `rans` crate (ryg_rans implementation).
//!
//! Drop-in replacement for the buggy v3 `rans.rs`. Uses the well-tested
//! `rans` crate under the hood.
//!
//! Provides:
//!   - `encode(symbols, &FreqTable) -> Vec<u8>` — same signature as v3.
//!   - `decode(stream, &FreqTable, n) -> Vec<u32>` — same signature as v3.
//!
//! Internally: build per-symbol rANS encoders from cum/freq, encode in
//! order, flush. To decode: read flushed state, decode in REVERSE order
//! (ryg_rans' decoder requires this).

use rans::byte_decoder::{ByteRansDecSymbol, ByteRansDecoder};
use rans::byte_encoder::{ByteRansEncSymbol, ByteRansEncoder};
use rans::{RansDecSymbol, RansDecoder, RansEncSymbol, RansEncoder, RansEncoderMulti};

/// Frequency table: cum[i] is the cumulative count for symbol i.
/// cum[0] = 0, cum[n] = total.
#[derive(Debug, Clone)]
pub struct FreqTable {
    pub cum: Vec<u32>,
    pub total: u32,
    pub scale_bits: u32,
}

impl FreqTable {
    /// Build from raw counts. Picks smallest scale_bits s.t. 2^scale_bits >= total.
    pub fn from_counts(counts: &[u32]) -> Self {
        let total: u64 = counts.iter().map(|&c| c as u64).sum();
        let scale_bits = if total <= 1 {
            1
        } else {
            // ceil(log2(total))
            let mut bits = 0u32;
            let mut t = total;
            while t > 1 {
                t = (t + 1) / 2;
                bits += 1;
            }
            bits.max(1)
        };
        let mut cum = Vec::with_capacity(counts.len() + 1);
        cum.push(0u32);
        for &c in counts {
            cum.push(cum.last().unwrap().wrapping_add(c));
        }
        // Final total = last cum
        Self {
            cum,
            total: total as u32,
            scale_bits,
        }
    }

    pub fn n_symbols(&self) -> u32 {
        (self.cum.len() - 1) as u32
    }

    /// Look up the symbol whose cum interval contains `target` (in [0, total)).
    pub fn lookup(&self, target: u32) -> u32 {
        let n = self.cum.len() - 1;
        for i in 0..n {
            if target < self.cum[i + 1] {
                return i as u32;
            }
        }
        // Shouldn't reach for valid target < total.
        n as u32 - 1
    }
}

/// Encode symbols using the table. Returns a byte stream that the decoder
/// can read back.
pub fn rans_encode(symbols: &[u32], table: &FreqTable) -> Vec<u8> {
    let mut enc = ByteRansEncoder::new(symbols.len() * 4 + 64);

    // Build a small cache of symbols indexed by symbol id (to avoid recreating
    // for every put). Each symbol needs (cum, freq, scale_bits).
    // ryg_rans requires that the cumulative frequency passed to the symbol
    // MATCHES the start of the symbol's interval.
    let mut enc_symbols: Vec<Option<ByteRansEncSymbol>> = Vec::with_capacity(table.cum.len());
    for i in 0..table.cum.len() - 1 {
        let cum = table.cum[i];
        let freq = table.cum[i + 1] - table.cum[i];
        let sym = ByteRansEncSymbol::new(cum, freq, table.scale_bits);
        enc_symbols.push(Some(sym));
    }

    for &sym in symbols {
        if let Some(s) = enc_symbols.get(sym as usize).and_then(|o| o.as_ref()) {
            enc.put(s);
        } else {
            panic!("symbol id {} out of range for table with {} symbols",
                   sym, table.n_symbols());
        }
    }

    enc.flush();
    enc.data().to_owned()
}

/// Decode a stream produced by `rans_encode`. Returns `n` symbols.
pub fn rans_decode(stream: &[u8], table: &FreqTable, n: usize) -> Vec<u32> {
    let mut dec = ByteRansDecoder::new(stream.to_vec());

    // Build decoder symbols (one per symbol id).
    let mut dec_symbols: Vec<Option<ByteRansDecSymbol>> = Vec::with_capacity(table.cum.len());
    for i in 0..table.cum.len() - 1 {
        let cum = table.cum[i];
        let freq = table.cum[i + 1] - table.cum[i];
        let sym = ByteRansDecSymbol::new(cum, freq);
        dec_symbols.push(Some(sym));
    }

    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let cum_freq = dec.get(table.scale_bits);
        let sym = table.lookup(cum_freq);
        out.push(sym);
        if let Some(s) = dec_symbols.get(sym as usize).and_then(|o| o.as_ref()) {
            dec.advance(s, table.scale_bits);
        }
    }

    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(symbols: &[u32], table: &FreqTable, name: &str) {
        let stream = rans_encode(symbols, table);
        let decoded = rans_decode(&stream, table, symbols.len());
        assert_eq!(decoded, symbols, "roundtrip mismatch ({})", name);
    }

    #[test]
    fn one_symbol() {
        roundtrip(&[0], &FreqTable::from_counts(&[1]), "1-sym");
    }

    #[test]
    fn uniform_2() {
        let table = FreqTable::from_counts(&[1, 1]);
        roundtrip(&[0, 1, 0, 1, 0, 1, 0, 1], &table, "alternating 2-sym");
    }

    #[test]
    fn uniform_8() {
        let table = FreqTable::from_counts(&[1, 1, 1, 1, 1, 1, 1, 1]);
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
    fn skewed_99_1_short() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut v = vec![0u32; 99];
        v.push(1);
        roundtrip(&v, &table, "99:1 skewed 100 syms");
    }

    #[test]
    fn skewed_99_1_long() {
        let table = FreqTable::from_counts(&[99, 1]);
        let mut symbols = vec![0u32; 499];
        symbols.push(1);
        roundtrip(&symbols, &table, "99:1 skewed 500 syms");
    }

    #[test]
    fn skewed_long_random() {
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
}