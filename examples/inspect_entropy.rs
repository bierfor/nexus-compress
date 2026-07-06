// Check entropy per CDC chunk for code.rs

use nexus_compress::cdc;
use nexus_compress::classifier::compute_stats;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "corpus/code.rs".to_string());
    let data = std::fs::read(&path).unwrap();
    let chunks = cdc::chunkify(&data, 4 * 1024, 64 * 1024, 15);
    println!("{}: {} chunks", path, chunks.len());
    println!("code.rs: {} chunks", chunks.len());
    for (i, (off, len)) in chunks.iter().enumerate() {
        let block = &data[*off..*off + *len];
        let stats = compute_stats(block);
        println!(
            "  chunk {:3}: off={:7} len={:7} entropy={:.2} printability={:.2}",
            i, off, len, stats.entropy, stats.printable_ratio
        );
    }
}