//! Debug rANS roundtrip on uniform data.

use nexus_compress::rans::{FreqTable, rans_decode, rans_encode};

fn main() {
    let table = FreqTable::uniform(12);
    println!("Uniform table: scale={}, n_symbols={}", table.scale, table.n_symbols);
    println!("First 8 freqs: {:?}", &table.freq[..8]);
    println!("First 8 cum: {:?}", &table.cum[..8]);
    println!("lut[0..32] = {:?}", &table.lut[..32]);
    println!("lut[4080..] = {:?}", &table.lut[4080..]);

    let data: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();

    let enc = rans_encode(&data, &table);
    println!("Encoded size: {}", enc.len());
    println!("First 10 bytes: {:?}", &enc[..10.min(enc.len())]);
    println!("Last 10 bytes: {:?}", &enc[enc.len().saturating_sub(10)..]);

    let dec = rans_decode(&enc, &table, data.len());
    println!("Decoded: first 20 = {:?}", &dec[..20.min(dec.len())]);
    println!("Decoded: last 20 = {:?}", &dec[dec.len().saturating_sub(20)..]);

    // Also test with a small data
    let small = vec![0u8, 1, 2, 3, 4];
    let enc2 = rans_encode(&small, &table);
    println!("\nSmall data [0..5]:");
    println!("Encoded: {:?}", enc2);
    let dec2 = rans_decode(&enc2, &table, small.len());
    println!("Decoded: {:?}", dec2);
}