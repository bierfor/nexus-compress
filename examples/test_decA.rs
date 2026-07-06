use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    // Build a table where all symbols have freq=15 (uniform skew)
    let mut counts = [100u32; 256];  // all 100 → scaled = 100 * 4096 / 25600 = 16 → all 16
    // To get freq=15, use counts = 93 each: 93*4096/23808 = 16. Hmm
    
    // Let me try: counts = 100, expect all 16
    let table_16 = FreqTable::from_counts(&counts, 12);
    let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let enc = rans_encode_streamed(&data, &table_16);
    let dec = rans_decode_streamed(&enc, &table_16, data.len());
    println!("uniform 16: matches={}", dec == data);
    
    // Counts = 50, expect all 16 (50*4096/12800=16)
    let counts = [50u32; 256];
    let table_16b = FreqTable::from_counts(&counts, 12);
    let enc = rans_encode_streamed(&data, &table_16b);
    let dec = rans_decode_streamed(&enc, &table_16b, data.len());
    println!("uniform 16 (counts 50): matches={}", dec == data);
    
    // Counts = 100, expect all 16 (same as above)
    // Counts = 17, expect... 17*4096/4352=16. Hmm
    
    // Counts = 30, expect all 16 (30*4096/7680=16)
    let counts = [30u32; 256];
    let table_16c = FreqTable::from_counts(&counts, 12);
    let enc = rans_encode_streamed(&data, &table_16c);
    let dec = rans_decode_streamed(&enc, &table_16c, data.len());
    println!("uniform 16 (counts 30): matches={}", dec == data);
    
    // Try freq=17 for ALL symbols: need cum[256]=4096, freq[i]=17 for all → cum[256]=17*256=4352 ≠ 4096.
    // Impossible without drift.
    // Try with mixed: 240 freq=17, 16 freq=1 (above test_dec9)
    // Or 256 freq=16, plus drift correction adds freq=17 somewhere
    
    // Let me try with counts = 200, all sym → scaled = 200*4096/51200=16
    let counts = [200u32; 256];
    let table_d = FreqTable::from_counts(&counts, 12);
    println!("\ncounts=200: freq[0..5]={:?} cum[256]={}", &table_d.freq[..5], table_d.cum[256]);
    let enc = rans_encode_streamed(&data, &table_d);
    let dec = rans_decode_streamed(&enc, &table_d, data.len());
    println!("uniform 16 (counts 200): matches={}", dec == data);
}
