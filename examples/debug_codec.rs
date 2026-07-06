// Compare the actual bitstream cost: lazy vs optimal on a code chunk.

use nexus_compress::codec;

fn main() {
    let data = std::fs::read("corpus/code.rs").unwrap();
    // Take a chunk that's typical source code (not edge of file)
    let sample: Vec<u8> = data[50000..54000].to_vec();
    println!("Sample: {} bytes", sample.len());

    let compressed_lazy = codec::compress(&sample);
    let compressed_v14_lazy = compressed_lazy.len();

    // We don't have a direct flag, but optimal is what we want to test.
    // For now just print the compressed size with current code (optimal).
    println!("Compressed (current = optimal): {} bytes", compressed_v14_lazy);
    println!("Ratio: {:.2}x", sample.len() as f64 / compressed_v14_lazy as f64);

    // Inspect first few blocks
    use nexus_compress::format::{NexusHeader, BlockHeader, BlockType};
    use std::io::Cursor;
    let mut cur = Cursor::new(&compressed_lazy);
    let header = NexusHeader::read(&mut cur).unwrap();
    println!("Header: blocks={}, uncompressed={}", header.block_count, header.uncompressed_total_size);

    for i in 0..header.block_count.min(3) {
        let bh = BlockHeader::read(&mut cur).unwrap();
        let start = cur.position() as usize;
        println!("Block {}: type={:?}, uncomp={}, comp={}",
            i, bh.block_type, bh.uncompressed_size, bh.compressed_size);
        // Skip payload
        cur.set_position((start + bh.compressed_size as usize) as u64);
    }
}