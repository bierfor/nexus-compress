use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    // Test 1: 256 bytes cycling with table built from those 256 bytes
    let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let mut counts = [1u32; 256];
    for &b in &data {
        counts[b as usize] += 1;
    }
    let table1 = FreqTable::from_counts(&counts, 12);
    let enc1 = rans_encode_streamed(&data, &table1);
    let dec1 = rans_decode_streamed(&enc1, &table1, data.len());
    println!("test1: enc_len={} dec_matches={}", enc1.len(), dec1 == data);
    
    // Test 2: 256 bytes cycling with table built from 50000 bytes cycling
    let data_full: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    let mut counts = [1u32; 256];
    for &b in &data_full {
        counts[b as usize] += 1;
    }
    let table2 = FreqTable::from_counts(&counts, 12);
    let enc2 = rans_encode_streamed(&data, &table2);
    let dec2 = rans_decode_streamed(&enc2, &table2, data.len());
    println!("test2: enc_len={} dec_matches={}", enc2.len(), dec2 == data);
    
    // Print first 20 bytes of each encoded stream
    println!("\ntest1 enc[:30]: {:?}", &enc1[..30.min(enc1.len())]);
    println!("test2 enc[:30]: {:?}", &enc2[..30.min(enc2.len())]);
    
    // Print tables
    println!("\ntable1 freq first 10: {:?}", &table1.freq[..10]);
    println!("table1 cum first 10: {:?}", &table1.cum[..10]);
    println!("table2 freq first 10: {:?}", &table2.freq[..10]);
    println!("table2 cum first 10: {:?}", &table2.cum[..10]);
}
