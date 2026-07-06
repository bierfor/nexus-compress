use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    
    let mut counts = [1u32; 256];
    for &b in &data {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    println!("table: n_symbols={} scale={}", table.n_symbols, table.scale);
    println!("freq[0..16]: {:?}", &table.freq[..16]);
    println!("cum[0..16]: {:?}", &table.cum[..16]);
    
    let enc = rans_encode_streamed(&data, &table);
    let dec = rans_decode_streamed(&enc, &table, data.len());
    println!("matches: {}", dec == data);
    
    // What if we count from_data twice
    let data2: Vec<u8> = (0..512).map(|i| i as u8).collect();
    let mut counts = [1u32; 256];
    for &b in &data2 {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    let enc = rans_encode_streamed(&data, &table);
    let dec = rans_decode_streamed(&enc, &table, data.len());
    println!("\nmismatched table test:");
    println!("matches: {}", dec == data);
    if dec != data {
        for i in 0..20 {
            if dec[i] != data[i] {
                println!("  diff at {}: exp {} got {}", i, data[i], dec[i]);
                break;
            }
        }
    }
}
