use nexus_compress::cdc;

fn main() {
    let data: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    let chunks = cdc::chunkify(&data, 4096, 65536, 15);
    println!("n_chunks: {}", chunks.len());
    for (i, (off, len)) in chunks.iter().enumerate() {
        println!("  chunk {}: offset={} length={}", i, off, len);
    }
}
