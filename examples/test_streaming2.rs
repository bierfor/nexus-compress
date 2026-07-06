use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    // Simulate what codec does
    let data: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    
    // Build a table that mimics what the codec would build for literals of cycling data
    let mut counts = [1u32; 256];
    for &b in &data {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    // Encode the first 256 bytes (which is what the literal stream would be for the 50K test)
    let literals = &data[..256];
    let enc = rans_encode_streamed(literals, &table);
    println!("encoded len: {} (input was {})", enc.len(), literals.len());
    let dec = rans_decode_streamed(&enc, &table, literals.len());
    println!("decoded len: {} (expected {})", dec.len(), literals.len());
    if dec != literals {
        let n_diff = dec.iter().zip(literals.iter()).filter(|(a, b)| a != b).count();
        println!("DIFFS: {}", n_diff);
        for i in 0..dec.len().min(20) {
            if dec[i] != literals[i] {
                println!("  diff at {}: exp {} got {}", i, literals[i], dec[i]);
            }
        }
    } else {
        println!("MATCH ✓");
    }
    
    // Now try with 450 bytes (chunk boundary)
    let sub = &data[..450];
    let enc = rans_encode_streamed(sub, &table);
    let dec = rans_decode_streamed(&enc, &table, sub.len());
    println!("\n450 bytes:");
    println!("encoded len: {} (input was {})", enc.len(), sub.len());
    println!("decoded len: {} (expected {})", dec.len(), sub.len());
    if dec != sub {
        let n_diff = dec.iter().zip(sub.iter()).filter(|(a, b)| a != b).count();
        println!("DIFFS: {}", n_diff);
    } else {
        println!("MATCH ✓");
    }
    
    // Try 1000 bytes
    let sub = &data[..1000];
    let enc = rans_encode_streamed(sub, &table);
    let dec = rans_decode_streamed(&enc, &table, sub.len());
    println!("\n1000 bytes:");
    println!("encoded len: {} (input was {})", enc.len(), sub.len());
    println!("decoded len: {} (expected {})", dec.len(), sub.len());
    if dec != sub {
        let n_diff = dec.iter().zip(sub.iter()).filter(|(a, b)| a != b).count();
        println!("DIFFS: {}", n_diff);
    } else {
        println!("MATCH ✓");
    }
}
