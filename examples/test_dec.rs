use nexus_compress::rans::{rans_encode, rans_decode, FreqTable};

fn main() {
    let table = FreqTable::uniform(12);
    let data: Vec<u8> = (0..32).map(|i| i as u8).collect();
    let enc = rans_encode(&data, &table);
    let dec = rans_decode(&enc, &table, data.len());
    println!("expected: {:?}", &data);
    println!("got:      {:?}", &dec);
    println!("matches: {}", dec == data);
    
    // Also try 256
    let data2: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let enc2 = rans_encode(&data2, &table);
    let dec2 = rans_decode(&enc2, &table, data2.len());
    let first_wrong = dec2.iter().zip(data2.iter()).position(|(a, b)| a != b);
    println!("\n256 bytes:");
    println!("matches: {}", dec2 == data2);
    if let Some(i) = first_wrong {
        println!("first diff at {}: exp {} got {}", i, data2[i], dec2[i]);
        let lo = i.saturating_sub(3);
        let hi = (i+10).min(256);
        println!("expected: {:?}", &data2[lo..hi]);
        println!("got:      {:?}", &dec2[lo..hi]);
    }
}
