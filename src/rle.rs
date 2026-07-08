//! Simple RLE pre-filter for the compression pipeline.
//!
//! Format: each "chunk" is 1 or 2 bytes.
//!   - 0b0_VVVVVVV (0x00..=0x7F): literal byte value (lower 7 bits, 0..=127).
//!   - 0b1_CCCCCCC (0x80..=0xFF): run of (lower 7 bits + 1) copies of the next byte.
//!
//! Properties:
//!   - Bytes 0..=127 (typical ASCII): 1 byte each when alone (literal).
//!   - Bytes 128..=255: emitted as a "run of 1" → 2 bytes per occurrence (a 1-byte
//!     expansion vs raw). Use only when the input actually has runs.
//!   - Run of N same bytes (N >= 2): 2 bytes total. ~N/2 compression.
//!   - For all-same input: 2 bytes per 128-byte run. ~64x compression on 64KB.
//!
//! NOTE: this pre-filter is not currently wired into the main codec pipeline
//! because empirical benchmarks on our 6-file corpus (repetitive.bin is
//! "abcdefgh" repeated — no runs of 2+; code.rs / text.txt / data.json are
//! mostly unique bytes; random.bin is uniformly distributed) showed that RLE
//! is neutral at best and slightly harmful at worst. Kept here as a
//! building block for future optimization (e.g., RLE + LZ77 + rANS for
//! different data shapes like log files, run-length-heavy binary formats).

#![allow(dead_code)]

/// Apply RLE to `data`. Output is at most 2x the input size (in the
/// pathological all-unique case). Never smaller than `data.len() / 128`
/// (in the all-same case).
pub fn compress_rle(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        // Count run length (cap at 128 since 7-bit count encodes 0..=127 = 1..=128).
        let mut run = 1usize;
        while i + run < data.len() && data[i + run] == b && run < 128 {
            run += 1;
        }
        if b <= 0x7F && run == 1 {
            // Single-byte literal: high bit clear, value in lower 7 bits.
            out.push(b);
            i += 1;
        } else if run >= 2 {
            // Run: 0b1_CCCCCCC value. C = run - 1 (so 0..=127 maps to 1..=128).
            out.push(0x80 | ((run - 1) as u8));
            out.push(b);
            i += run;
        } else {
            // Single high byte (>= 0x80) or a 1-byte "run of 1": emit as 2 bytes.
            out.push(0x80); // count = 0 → 1 copy
            out.push(b);
            i += 1;
        }
    }
    out
}

/// Reverse of `compress_rle`. Returns the original data.
pub fn decompress_rle(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b & 0x80 != 0 {
            // Run chunk. Length = (b & 0x7F) + 1.
            let count = (b & 0x7F) as usize + 1;
            i += 1;
            if i >= data.len() {
                break; // malformed: missing value byte
            }
            let value = data[i];
            i += 1;
            for _ in 0..count {
                out.push(value);
            }
        } else {
            // Literal chunk (7-bit value).
            out.push(b);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8], name: &str) {
        let rle = compress_rle(data);
        let decoded = decompress_rle(&rle);
        assert_eq!(decoded, data, "RLE roundtrip failed for {}", name);
    }

    #[test]
    fn empty() {
        roundtrip(&[], "empty");
    }

    #[test]
    fn single_byte_low() {
        roundtrip(&[0x42], "single low");
    }

    #[test]
    fn single_byte_high() {
        roundtrip(&[0xAA], "single high");
    }

    #[test]
    fn all_different_low() {
        roundtrip(&(0..=127u8).collect::<Vec<_>>(), "all different low");
    }

    #[test]
    fn all_different_high() {
        let data: Vec<u8> = (128..=255u8).collect();
        roundtrip(&data, "all different high");
    }

    #[test]
    fn all_different_mixed() {
        let mut data = vec![0u8; 256];
        for (i, b) in data.iter_mut().enumerate() {
            *b = i as u8;
        }
        roundtrip(&data, "all different mixed");
    }

    #[test]
    fn all_same_short() {
        roundtrip(&[0xAA; 10], "10x same");
    }

    #[test]
    fn all_same_long() {
        roundtrip(&[0xCC; 1000], "1000x same");
    }

    #[test]
    fn all_same_exact_chunk() {
        roundtrip(&[0xDD; 128], "128x same (max run)");
    }

    #[test]
    fn run_above_chunk() {
        roundtrip(&[0xEE; 256], "256x same (two runs)");
    }

    #[test]
    fn alternating() {
        let data: Vec<u8> = (0..100u8)
            .map(|i| if i % 2 == 0 { 0x55 } else { 0xAA })
            .collect();
        roundtrip(&data, "alternating low/high");
    }

    #[test]
    fn mixed() {
        let data: Vec<u8> = b"the quick brown fox jumps over the lazy dog. ".repeat(100);
        roundtrip(&data, "mixed text");
    }

    #[test]
    fn runs_compress() {
        let data = vec![0xAA; 1000];
        let rle = compress_rle(&data);
        assert!(
            rle.len() < data.len() / 4,
            "run of 1000 same bytes should compress well: {} -> {}",
            data.len(),
            rle.len()
        );
    }

    #[test]
    fn random_doesnt_explode() {
        // Pseudo-random data with full byte range.
        let mut data = Vec::with_capacity(1024);
        let mut s: u32 = 0xdeadbeef;
        for _ in 0..1024 {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            data.push((s >> 16) as u8);
        }
        let rle = compress_rle(&data);
        // Worst case 2x for all-unique. Real data should be close to 1.5-2x.
        assert!(
            rle.len() <= data.len() * 2,
            "RLE exploded: {} bytes -> {} (max should be {} = 2x)",
            data.len(),
            rle.len(),
            data.len() * 2
        );
    }
}
