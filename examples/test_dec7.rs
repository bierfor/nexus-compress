use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    let data_full: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    let mut counts = [1u32; 256];
    for &b in &data_full {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    println!("table.cum[256]={} scale={}", table.cum[256], table.scale);
    println!("freq distribution: {} 16s, {} 17s, {} 15s",
        table.freq.iter().filter(|&&f| f == 16).count(),
        table.freq.iter().filter(|&&f| f == 17).count(),
        table.freq.iter().filter(|&&f| f == 15).count());
    
    let sub: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let enc = rans_encode_streamed(&sub, &table);
    let dec = rans_decode_streamed(&enc, &table, sub.len());
    println!("matches: {}", dec == sub);
    println!("first 20 expected: {:?}", &sub[..20]);
    println!("first 20 got:      {:?}", &dec[..20]);
    
    // Try with a clean uniform table
    let table_u = FreqTable::uniform(12);
    println!("\nuniform table:");
    println!("table.cum[256]={} scale={}", table_u.cum[256], table_u.scale);
    let enc = rans_encode_streamed(&sub, &table_u);
    let dec = rans_decode_streamed(&enc, &table_u, sub.len());
    println!("matches: {}", dec == sub);
}
