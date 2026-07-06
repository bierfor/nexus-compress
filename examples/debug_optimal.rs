// Quick debug to see why optimal produces 743 ops instead of 14.

use nexus_compress::lz77::{MatchFinder, MatchDecoder};

fn main() {
    let pattern = b"abcdefghij";
    let mut data = Vec::new();
    for _ in 0..100 {
        data.extend_from_slice(pattern);
    }
    println!("Data size: {}", data.len());

    let mut enc = MatchFinder::new();
    let ops = enc.encode_optimal(&data);
    println!("Optimal ops: {}", ops.len());

    // Decode to verify correctness
    let mut dec = MatchDecoder::new();
    let out = dec.decode(&ops);
    println!("Decoded matches input: {}", out == data);

    // Print first 25 ops
    for (i, op) in ops.iter().take(25).enumerate() {
        match op {
            nexus_compress::lz77::Op::Lit(b) => println!("  [{}] Lit 0x{:02x} ({})", i, b, *b as char),
            nexus_compress::lz77::Op::Match { dist, len } => println!("  [{}] Match dist={} len={}", i, dist, len),
        }
    }
    println!("...");
    for (i, op) in ops.iter().rev().take(5).enumerate() {
        let idx = ops.len() - 5 + i;
        match op {
            nexus_compress::lz77::Op::Lit(b) => println!("  [{}] Lit 0x{:02x} ({})", idx, b, *b as char),
            nexus_compress::lz77::Op::Match { dist, len } => println!("  [{}] Match dist={} len={}", idx, dist, len),
        }
    }
}