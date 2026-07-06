use nexus_compress::codec;

fn main() {
    for size in [5000, 50000, 100000].iter() {
        let data: Vec<u8> = (0..*size).map(|i| (i % 256) as u8).collect();
        let compressed = codec::compress(&data);
        let decompressed = codec::decompress(&compressed);
        let matches = decompressed == data;
        println!("size={} compressed={} matches={}", size, compressed.len(), matches);
        if !matches {
            // Find first difference
            for i in 0..(if *size < 10000 { *size } else { 10000 }) {
                if data[i] != decompressed[i] {
                    println!("  first diff at byte {}: expected {} got {}", i, data[i], decompressed[i]);
                    // Show neighborhood
                    let lo = i.saturating_sub(5);
                    let hi = (i+10).min(*size);
                    println!("  expected: {:?}", &data[lo..hi]);
                    println!("  got:      {:?}", &decompressed[lo..hi]);
                    break;
                }
            }
            // Check totals
            let total_diff = data.iter().zip(decompressed.iter()).filter(|(a, b)| a != b).count();
            println!("  total bytes differ: {}", total_diff);
            println!("  decompressed len: {}, expected: {}", decompressed.len(), data.len());
        }
    }
}
