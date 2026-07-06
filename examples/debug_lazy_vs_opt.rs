// Compare lazy vs optimal on real code at the codec level.

use nexus_compress::codec::{self};
use nexus_compress::lz77::MatchFinder;

fn main() {
    let data = std::fs::read("corpus/code.rs").unwrap();
    let sample: Vec<u8> = data[50000..54000].to_vec();
    println!("Sample: {} bytes", sample.len());

    // Use the LZ77 level directly to see op count vs payload size.
    let mut finder = MatchFinder::new();
    let ops_lazy = finder.encode(&sample);
    let ops_opt = finder.encode_optimal(&sample);

    let lazy_lits = ops_lazy.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Lit(_))).count();
    let lazy_mats = ops_lazy.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Match { .. })).count();
    let opt_lits = ops_opt.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Lit(_))).count();
    let opt_mats = ops_opt.iter().filter(|o| matches!(o, nexus_compress::lz77::Op::Match { .. })).count();

    let lazy_op_bytes = lazy_lits * 2 + lazy_mats * 9;
    let opt_op_bytes = opt_lits * 2 + opt_mats * 9;

    println!("\nLazy:   {} lits, {} matches. Op stream ~{} bytes ({} bytes/match avg)",
        lazy_lits, lazy_mats, lazy_op_bytes,
        if lazy_mats > 0 { sample.len() / lazy_mats } else { 0 });
    println!("Optimal: {} lits, {} matches. Op stream ~{} bytes ({} bytes/match avg)",
        opt_lits, opt_mats, opt_op_bytes,
        if opt_mats > 0 { sample.len() / opt_mats } else { 0 });

    // rANS estimate (5 bits per ASCII literal)
    let lazy_rans_bits = lazy_lits * 5;
    let opt_rans_bits = opt_lits * 5;
    println!("\nEstimated rANS payload: lazy={} bits, optimal={} bits",
        lazy_rans_bits, opt_rans_bits);

    // Total estimated payload
    println!("Estimated total payload: lazy={} bytes, optimal={} bytes",
        lazy_op_bytes + lazy_rans_bits/8 + 120,
        opt_op_bytes + opt_rans_bits/8 + 120);

    // Actual compression
    let compressed = codec::compress(&sample);
    println!("\nActual compressed (optimal): {} bytes (ratio {:.2}x)",
        compressed.len(), sample.len() as f64 / compressed.len() as f64);
}