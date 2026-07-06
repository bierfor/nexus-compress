// Debug: print the ops for the basic roundtrip test case.

use nexus_compress::lz77::{MatchFinder, MatchDecoder};

fn main() {
    let data = b"hello world hello world hello world abcdef hello world";
    let mut enc = MatchFinder::new();
    let ops = enc.encode_optimal(data);
    println!("Data ({} bytes): {:?}", data.len(), std::str::from_utf8(data).unwrap());
    println!("Ops ({} total):", ops.len());
    for (i, op) in ops.iter().enumerate() {
        match op {
            nexus_compress::lz77::Op::Lit(b) => println!("  [{}] Lit 0x{:02x} ({:?})", i, b, *b as char),
            nexus_compress::lz77::Op::Match { dist, len } => println!("  [{}] Match dist={} len={}", i, dist, len),
        }
    }

    let mut dec = MatchDecoder::new();
    let out = dec.decode(&ops);
    println!("Decoded ({} bytes): {:?}", out.len(), std::str::from_utf8(&out).unwrap_or("<binary>"));
    println!("Match: {}", out == data);
}