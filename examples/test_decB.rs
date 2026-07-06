use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    
    // Counts[0] = 1000, others = 1 → freq[0] high, others low
    let mut counts = [1u32; 256];
    counts[0] = 1000;
    let table = FreqTable::from_counts(&counts, 12);
    println!("Skewed: freq[0]={} freq[1]={} cum[0]={} cum[2]={} cum[256]={}",
        table.freq[0], table.freq[1], table.cum[0], table.cum[2], table.cum[256]);
    
    let enc = rans_encode_streamed(&data, &table);
    let dec = rans_decode_streamed(&enc, &table, data.len());
    println!("matches: {}", dec == data);
    if dec != data {
        for i in 0..20 {
            if dec[i] != data[i] {
                println!("  diff at {}: exp {} got {}", i, data[i], dec[i]);
                break;
            }
        }
    }
    
    // Now use the same skewed table with mostly-0 data (matches table)
    let data_skewed: Vec<u8> = (0..256).map(|i| if i < 100 { 0 } else { 1 }).collect();
    let mut counts2 = [1u32; 256];
    counts2[0] = 100;
    counts2[1] = 50;
    let table2 = FreqTable::from_counts(&counts2, 12);
    println!("\nSkewed data: freq[0]={} freq[1]={} freq[2]={} cum[256]={}",
        table2.freq[0], table2.freq[1], table2.freq[2], table2.cum[256]);
    let enc = rans_encode_streamed(&data_skewed, &table2);
    let dec = rans_decode_streamed(&enc, &table2, data_skewed.len());
    println!("matches: {}", dec == data_skewed);
}
