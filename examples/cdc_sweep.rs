//! Sweep CDC parameters to see where boundaries fall on each corpus file.
//! Used to pick parameters that minimize the regression on repetitive data.

use nexus_compress::cdc;

fn main() {
    let corpus_files = [
        "corpus/code.rs",
        "corpus/data.json",
        "corpus/mixed.bin",
        "corpus/random.bin",
        "corpus/repetitive.bin",
        "corpus/text.txt",
    ];

    let configs: &[(usize, usize, u32, &str)] = &[
        (16 * 1024, 256 * 1024, 14, "min=16K max=256K avg=16K"),
        (16 * 1024, 256 * 1024, 16, "min=16K max=256K avg=64K"),
        (16 * 1024, 256 * 1024, 18, "min=16K max=256K avg=256K"),
        (4 * 1024, 64 * 1024, 14, "min=4K  max=64K  avg=16K"),
        (8 * 1024, 64 * 1024, 16, "min=8K  max=64K  avg=64K"),
        (4 * 1024, 32 * 1024, 13, "min=4K  max=32K  avg=8K"),
        (2 * 1024, 16 * 1024, 12, "min=2K  max=16K  avg=4K"),
    ];

    println!(
        "{:<35} {:>6} {:>6} {:>10} {:>10}",
        "config", "files", "chunks", "min_size", "max_size"
    );
    for &(min, max, bits, label) in configs {
        for f in &corpus_files {
            let Ok(data) = std::fs::read(f) else { continue };
            let chunks = cdc::chunkify(&data, min, max, bits);
            let sizes: Vec<usize> = chunks.iter().map(|&(_, l)| l).collect();
            let mn = *sizes.iter().min().unwrap_or(&0);
            let mx = *sizes.iter().max().unwrap_or(&0);
            println!(
                "{:<35} {:>6} {:>6} {:>10} {:>10}",
                format!("{} on {}", label, f),
                f.len(),
                chunks.len(),
                mn,
                mx
            );
        }
        println!();
    }
}