use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    let table = FreqTable::uniform(12);
    // 1000 bytes of uniform (cycling 0..256)
    let data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
    let enc = rans_encode_streamed(&data, &table);
    println!("encoded len: {} (n_chunks={})", enc.len(), u32::from_le_bytes(enc[0..4].try_into().unwrap()));
    // Skip n_chunks
    let mut p = 4;
    let mut i = 0;
    while p < enc.len() {
        let cl = u32::from_le_bytes(enc[p..p+4].try_into().unwrap()) as usize;
        println!("  chunk {}: {} bytes", i, cl);
        p += 4 + cl;
        i += 1;
    }
    let dec = rans_decode_streamed(&enc, &table, 1000);
    println!("decoded len: {}", dec.len());
    println!("matches: {}", dec == data);
    if dec != data {
        for j in 0..20 {
            if dec[j] != data[j] {
                println!("  diff at {}: expected {} got {}", j, data[j], dec[j]);
            }
        }
    }
}
