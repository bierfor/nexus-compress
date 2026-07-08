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

/// Encode the table as a byte stream for storage.
/// Format: [scale_bits:u8][n_symbols:u16 LE][cum_0:u32 LE][cum_1:u32 LE]...[cum_n:u32 LE]
pub fn encode_table(table: &FreqTable) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + table.cum.len() * 4);
    out.push(table.scale_bits as u8);
    out.extend_from_slice(&(table.n_symbols() as u16).to_le_bytes());
    for &c in &table.cum {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

/// Decode a table from bytes (inverse of `encode_table`).
pub fn decode_table(bytes: &[u8]) -> FreqTable {
    assert!(bytes.len() >= 3, "table bytes too short");
    let scale_bits = bytes[0] as u32;
    let n = u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
    assert!(bytes.len() >= 3 + (n + 1) * 4, "table bytes truncated");
    let mut cum = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let off = 3 + i * 4;
        let v = u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        cum.push(v);
    }
    let total = *cum.last().unwrap();
    FreqTable {
        cum,
        total,
        scale_bits,
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
            panic!(
                "symbol id {} out of range for table with {} symbols",
                sym,
                table.n_symbols()
            );
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

// ---------------------------------------------------------------------
// Sparse rANS encoding (sprint 2.7)
// ---------------------------------------------------------------------
//
// Standard rANS forces every symbol in the 256-entry alphabet to
// have freq>=1. For a 32KB block of text where only ~30 distinct
// bytes are used, the other 226 entries are "ghost" symbols that
// cost 4 bytes each in the cum[] table. The 5 streams (lit / len /
// dist_lo / dist_hi / dict-id) each pay this overhead.
//
// Sparse encoding collapses the alphabet to the present symbols:
//   - 32-byte bitmask (256 bits), one per symbol
//   - Densified rANS table (n+1 cum entries where n = popcount)
//   - Densified rANS stream (encodes dense indices 0..n-1)
//   - Decoder reads bitmask, builds remap[dense_idx] -> original,
//     decodes the densified stream, then translates each symbol.
//
// The densified rANS table uses the SAME `encode_table` / `decode_table`
// format as the dense table — only n_symbols changes.

/// Bitmask size: 256 bits = 32 bytes. One bit per u8 symbol.
pub const SPARSE_BITMASK_BYTES: usize = 32;

/// Build the bitmask + remap + densified data for a u8 stream.
pub fn sparse_bitmask_remap(data: &[u8]) -> ([u8; SPARSE_BITMASK_BYTES], Vec<u16>, Vec<u32>) {
    let mut bitmask = [0u8; SPARSE_BITMASK_BYTES];
    for &b in data {
        let byte = (b / 8) as usize;
        let bit = b % 8;
        bitmask[byte] |= 1 << bit;
    }
    // Remap: dense_idx -> original_value, in value-ascending order.
    let mut remap: Vec<u16> = Vec::new();
    for v in 0..256u16 {
        let byte = (v / 8) as usize;
        let bit = v % 8;
        if bitmask[byte as usize] & (1 << bit) != 0 {
            remap.push(v);
        }
    }
    // Reverse remap: value -> dense_idx.
    let mut reverse = [0u16; 256];
    for (dense_idx, &orig) in remap.iter().enumerate() {
        reverse[orig as usize] = dense_idx as u16;
    }
    // Densified data: each original byte becomes its dense index.
    let dense_data: Vec<u32> = data.iter().map(|&b| reverse[b as usize] as u32).collect();
    (bitmask, remap, dense_data)
}

/// Build the densified rANS table for a dense_data stream.
fn build_sparse_table(dense_data: &[u32], n_symbols: usize, scale_bits: u32) -> FreqTable {
    let mut counts = vec![0u32; n_symbols];
    for &d in dense_data {
        if (d as usize) < n_symbols {
            counts[d as usize] += 1;
        }
    }
    let mut table = FreqTable::from_counts(&counts);
    if (1u32 << scale_bits) >= table.total && scale_bits > 0 {
        table.scale_bits = scale_bits;
    }
    table
}

/// Encode a u8 stream as a sparse rANS stream.
///
/// Returns `(table_section_bytes, stream_bytes)`. The `table_section_bytes`
/// is `[32 bytes bitmask][densified rANS table]` — the decoder
/// splits these by reading the first 32 bytes as the bitmask and the
/// remainder as the rANS table.
pub fn sparse_rans_encode_u8(data: &[u8], scale_bits: u32) -> (Vec<u8>, Vec<u8>) {
    let (bitmask, _remap, dense_data) = sparse_bitmask_remap(data);
    if dense_data.is_empty() {
        // Empty stream: 32-byte bitmask + 0 table + 0 stream.
        return (bitmask.to_vec(), Vec::new());
    }
    let n_symbols = _remap.len();
    let table = build_sparse_table(&dense_data, n_symbols, scale_bits);
    let table_bytes = encode_table(&table);
    let stream_bytes = rans_encode(&dense_data, &table);

    // Combine bitmask + table into one section.
    let mut section = Vec::with_capacity(SPARSE_BITMASK_BYTES + table_bytes.len());
    section.extend_from_slice(&bitmask);
    section.extend_from_slice(&table_bytes);
    (section, stream_bytes)
}

/// Decode a sparse rANS stream. `n` is the number of symbols to
/// decode (the caller knows this from the op-flags count).
///
/// Returns the original (un-densified) u8 values.
pub fn sparse_rans_decode_u8(table_section: &[u8], stream_bytes: &[u8], n: usize) -> Vec<u8> {
    if n == 0 {
        return Vec::new();
    }
    if table_section.len() < SPARSE_BITMASK_BYTES {
        panic!(
            "sparse rANS table section too short: {} bytes (need at least {})",
            table_section.len(),
            SPARSE_BITMASK_BYTES
        );
    }
    let mut bitmask = [0u8; SPARSE_BITMASK_BYTES];
    bitmask.copy_from_slice(&table_section[..SPARSE_BITMASK_BYTES]);
    let dense_table_bytes = &table_section[SPARSE_BITMASK_BYTES..];

    // Build remap from bitmask.
    let mut remap: Vec<u16> = Vec::new();
    for v in 0..256u16 {
        let byte = (v / 8) as usize;
        let bit = v % 8;
        if bitmask[byte] & (1 << bit) != 0 {
            remap.push(v);
        }
    }
    if remap.is_empty() {
        return Vec::new();
    }

    // Decode densified table and stream.
    let table = decode_table(dense_table_bytes);
    let dense_values = rans_decode(stream_bytes, &table, n);

    // Remap dense -> original.
    dense_values
        .iter()
        .map(|&d| {
            if (d as usize) < remap.len() {
                remap[d as usize] as u8
            } else {
                panic!("dense idx {} out of range (remap size {})", d, remap.len())
            }
        })
        .collect()
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
            (0u32, 100u32),
            (100, 130),
            (130, 140),
            (140, 145),
            (145, 147),
            (147, 148),
            (148, 149),
            (149, 150),
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
