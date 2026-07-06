use nexus_compress::rans::{rans_encode, rans_decode, FreqTable};

fn main() {
    let mut counts = [1u32; 256];
    for &b in b"the quick brown fox jumps over the lazy dog" {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    // Show table for 't', 'h', 'e'
    for &c in b"the " {
        let idx = table.sym_to_idx[c as usize];
        let freq = table.freq[idx as usize];
        let cum = table.cum[idx as usize];
        println!("sym {} ({}): idx={} freq={} cum={}", c as char, c, idx, freq, cum);
    }
    println!("table scale={} max_freq={}", table.scale, table.freq.iter().max().unwrap());
    println!("lut[0..30]: {:?}", &table.lut[..30]);
    println!("lut[100..130]: {:?}", &table.lut[100..130]);
    
    // Test 5-byte input
    let data = b"the q";
    let enc = rans_encode(data, &table);
    println!("\nenc of {:?}: {:?}", data, &enc);
    let dec = rans_decode(&enc, &table, data.len());
    println!("dec: {:?}", &dec);
    println!("matches: {}", dec == data);
    
    // Test 1-byte input
    let data = b"t";
    let enc = rans_encode(data, &table);
    println!("\nenc of {:?}: {:?}", data, &enc);
    let dec = rans_decode(&enc, &table, data.len());
    println!("dec: {:?}", &dec);
    println!("matches: {}", dec == data);
}
