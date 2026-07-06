// Debug: compare optimal vs lazy on real source code.

use nexus_compress::lz77::{MatchFinder, MatchDecoder};

fn main() {
    let data = std::fs::read("corpus/code.rs").unwrap();
    println!("code.rs: {} bytes", data.len());

    // Take a 4KB sample from middle of file
    let sample = &data[1000..5000];
    println!("Sample ({} bytes):\n  first 80: {:?}",
        sample.len(),
        std::str::from_utf8(&sample[..80.min(sample.len())]).unwrap_or("<bin>"));

    let mut enc = MatchFinder::new();
    let lazy_ops = enc.encode(sample);
    println!("Lazy: {} ops", lazy_ops.len());

    let mut enc2 = MatchFinder::new();
    let opt_ops = enc2.encode_optimal(sample);
    println!("Optimal: {} ops", opt_ops.len());

    // Count matches and literals in each
    let lazy_lits = lazy_ops.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Lit(_))).count();
    let lazy_mats = lazy_ops.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Match { .. })).count();
    let opt_lits = opt_ops.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Lit(_))).count();
    let opt_mats = opt_ops.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Match { .. })).count();
    println!("Lazy: {} lits, {} matches", lazy_lits, lazy_mats);
    println!("Optimal: {} lits, {} matches", opt_lits, opt_mats);

    // Histogram of optimal match lengths
    let mut lens: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for op in &opt_ops {
        if let nexus_compress::lz77::Op::Match { len, .. } = op {
            *lens.entry(*len).or_insert(0) += 1;
        }
    }
    let mut sorted_lens: Vec<_> = lens.iter().collect();
    sorted_lens.sort_by_key(|(k, _)| **k);
    println!("Optimal match length histogram:");
    for (len, count) in sorted_lens {
        println!("  L={:3}: {}", len, count);
    }

    // Show first 30 optimal ops
    println!("First 30 optimal ops:");
    for (i, op) in opt_ops.iter().take(30).enumerate() {
        match op {
            nexus_compress::lz77::Op::Lit(b) => println!("  [{}] Lit 0x{:02x} ({:?})", i, b, *b as char),
            nexus_compress::lz77::Op::Match { dist, len } => println!("  [{}] Match dist={} len={}", i, dist, len),
        }
    }
}