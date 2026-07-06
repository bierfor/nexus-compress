use nexus_compress::rans::{rans_encode, rans_decode, FreqTable};

fn main() {
    let table = FreqTable::uniform(12);
    // Try small data
    for n in [1, 5, 10, 50, 100, 200, 300, 400, 500].iter() {
        let data: Vec<u8> = (0..*n).map(|i| (i % 256) as u8).collect();
        let enc = rans_encode(&data, &table);
        let dec = rans_decode(&enc, &table, *n);
        let matches = dec == data;
        println!("n={} enc_len={} matches={}", n, enc.len(), matches);
        if !matches {
            for j in 0..(if *n < 10 { *n } else { 10 }) {
                if dec[j] != data[j] {
                    println!("    diff at {}: exp {} got {}", j, data[j], dec[j]);
                }
            }
        }
    }
}
