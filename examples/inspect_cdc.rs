//! Quick inspector: prints CDC chunk boundaries for the corpus files.

use nexus_compress::cdc;

fn main() {
    let corpus = std::env::args().nth(1).unwrap_or_else(|| "corpus/repetitive.bin".to_string());
    let data = std::fs::read(&corpus).unwrap();
    let chunks = cdc::chunkify(&data, 16 * 1024, 256 * 1024, 16);
    println!("File: {} ({} bytes)", corpus, data.len());
    println!("CDC chunks: {}", chunks.len());
    let mut sizes: Vec<usize> = chunks.iter().map(|&(_, l)| l).collect();
    sizes.sort();
    let n = sizes.len();
    println!(
        "Chunk sizes: min={}, median={}, max={}",
        sizes[0],
        sizes[n / 2],
        sizes[n - 1]
    );
    let total_chunk_bytes: usize = chunks.iter().map(|&(_, l)| l).sum();
    println!(
        "Total bytes chunked: {} (matches input: {})",
        total_chunk_bytes,
        total_chunk_bytes == data.len()
    );
    for (i, (off, len)) in chunks.iter().enumerate() {
        let end = (*off + len).min(*off + 8);
        let preview = &data[*off..end];
        println!(
            "  chunk {:3}: off={:7} len={:7} preview={:02x?}",
            i, off, len, preview
        );
        if i >= 15 {
            println!("  ... ({} more)", chunks.len() - i - 1);
            break;
        }
    }
}