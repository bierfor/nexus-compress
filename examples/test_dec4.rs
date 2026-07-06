use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    let data: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    
    let mut counts = [1u32; 256];
    for &b in &data {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    let sub = &data[..256];
    println!("input first 20: {:?}", &sub[..20]);
    let enc = rans_encode_streamed(sub, &table);
    println!("encoded len: {}", enc.len());
    
    let dec = rans_decode_streamed(&enc, &table, sub.len());
    println!("decoded first 20: {:?}", &dec[..20]);
}
