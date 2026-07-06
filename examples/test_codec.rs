use nexus_compress::codec;

fn main() {
    for size in [35, 100, 200, 500, 1000, 5000, 50000] {
        let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
        let compressed = codec::compress(&data);
        let decompressed = codec::decompress(&compressed);
        let matches = decompressed == data;
        println!("size={} compressed={} matches={}", size, compressed.len(), matches);
        if !matches {
            // Show first 20 bytes of diff
            for i in 0..size.min(20) {
                if data[i] != decompressed[i] {
                    println!("  diff at {}: expected {:?} got {:?}", i, data[i], decompressed[i]);
                }
            }
        }
    }
}
