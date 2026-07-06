use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    
    // Build table from data itself
    let mut counts = [1u32; 256];
    for &b in &data {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    println!("freq: {:?}", &table.freq[..20]);
    println!("cum[256]={}", table.cum[256]);
    
    let enc = rans_encode_streamed(&data, &table);
    let dec = rans_decode_streamed(&enc, &table, data.len());
    println!("matches: {}", dec == data);
    println!("first 20 expected: {:?}", &data[..20]);
    println!("first 20 got:      {:?}", &dec[..20]);
    
    // Try with non-uniform distribution
    let data2: Vec<u8> = (0..256).map(|i| if i < 10 { 0 } else { i as u8 }).collect();
    let mut counts2 = [1u32; 256];
    for &b in &data2 {
        counts2[b as usize] += 1;
    }
    let table2 = FreqTable::from_counts(&counts2, 12);
    
    println!("\nSkewed data test:");
    println!("data2 counts: 0 appears {} times", data2.iter().filter(|&&b| b == 0).count());
    println!("table2 freq[0]={} cum[0]={}", table2.freq[0], table2.cum[0]);
    println!("table2 freq[1]={} cum[1]={} cum[2]={}", table2.freq[1], table2.cum[1], table2.cum[2]);
    
    let enc = rans_encode_streamed(&data2, &table2);
    let dec = rans_decode_streamed(&enc, &table2, data2.len());
    println!("matches: {}", dec == data2);
    if dec != data2 {
        for i in 0..20 {
            if dec[i] != data2[i] {
                println!("  diff at {}: exp {} got {}", i, data2[i], dec[i]);
                break;
            }
        }
    }
}
