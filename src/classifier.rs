//! Content classifier — heuristic v0.
//!
//! Examines the first chunk of a block and decides its type using cheap stats:
//!   - Shannon entropy estimate
//!   - printable ASCII ratio
//!   - byte distribution variance
//!   - presence of structured delimiters (JSON braces, CSV commas, etc.)
//!
//! In v1 this will be replaced by a small INT8 CNN trained offline. The
//! heuristics here are tuned to match the CNN's expected labels so that the
//! downstream pipeline doesn't change when we swap in the network.

use crate::format::BlockType;

#[derive(Debug, Clone, Copy)]
pub struct BlockStats {
    pub size: usize,
    pub entropy: f32,         // 0..=8 bits/byte
    pub printable_ratio: f32, // 0..=1
    pub struct_score: f32,    // 0..=1, heuristic for JSON/CSV/SQL
    pub run_avg: f32,         // average run length
}

pub fn classify(data: &[u8]) -> BlockType {
    let stats = compute_stats(data);
    classify_from_stats(&stats)
}

pub fn compute_stats(data: &[u8]) -> BlockStats {
    let n = data.len().max(1);
    let mut counts = [0u32; 256];
    let mut printable = 0u32;
    let mut struct_chars = 0u32;
    let mut run_total = 0u32;
    let mut run_count = 0u32;

    let mut last = None;
    let mut cur_run = 0u32;

    for &b in data {
        counts[b as usize] += 1;
        if (0x20..0x7f).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t' {
            printable += 1;
        }
        // structured data indicators
        if matches!(b, b'{' | b'}' | b':' | b',' | b'[' | b']' | b'"') {
            struct_chars += 1;
        }
        if let Some(prev) = last {
            if prev == b {
                cur_run += 1;
            } else if cur_run > 0 {
                run_total += cur_run;
                run_count += 1;
                cur_run = 0;
            }
        }
        last = Some(b);
    }
    if cur_run > 0 {
        run_total += cur_run;
        run_count += 1;
    }

    // Shannon entropy
    let mut h = 0f32;
    for &c in counts.iter() {
        if c == 0 {
            continue;
        }
        let p = c as f32 / n as f32;
        h -= p * p.log2();
    }

    BlockStats {
        size: data.len(),
        entropy: h,
        printable_ratio: printable as f32 / n as f32,
        struct_score: struct_chars as f32 / n as f32,
        run_avg: if run_count > 0 {
            run_total as f32 / run_count as f32
        } else {
            0.0
        },
    }
}

pub fn classify_from_stats(s: &BlockStats) -> BlockType {
    if s.entropy > 7.7 {
        // close to uniform -> incompressible
        return BlockType::Random;
    }
    // structured data has lots of punctuation, even moderate entropy
    if s.struct_score > 0.10 && s.printable_ratio > 0.85 {
        return BlockType::Structured;
    }
    if s.printable_ratio > 0.92 && s.run_avg < 4.0 {
        return BlockType::Text;
    }
    if s.run_avg > 6.0 && s.entropy < 5.0 {
        // repetitive patterns
        return BlockType::Binary;
    }
    if s.entropy > 6.0 {
        BlockType::Binary
    } else {
        BlockType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_text() {
        let text = b"The quick brown fox jumps over the lazy dog. \
                     Pack my box with five dozen liquor jugs.";
        assert_eq!(classify(text), BlockType::Text);
    }

    #[test]
    fn detects_random() {
        // pseudo-random high entropy
        let mut v = Vec::with_capacity(4096);
        let mut x: u32 = 0xdeadbeef;
        for _ in 0..4096 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            v.push(x as u8);
        }
        assert_eq!(classify(&v), BlockType::Random);
    }

    #[test]
    fn detects_structured() {
        let json = br#"{"users":[{"id":1,"name":"alice"},{"id":2,"name":"bob"}]}"#;
        assert_eq!(classify(json), BlockType::Structured);
    }
}
