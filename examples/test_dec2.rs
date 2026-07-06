use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    let table = FreqTable::uniform(12);
    
    for n in [32, 64, 128, 200, 256, 450, 500, 1000].iter() {
        let data: Vec<u8> = (0..*n).map(|i| i as u8).collect();
        let enc = rans_encode_streamed(&data, &table);
        let dec = rans_decode_streamed(&enc, &table, data.len());
        let matches = dec == data;
        println!("n={} matches={}", n, matches);
        if !matches {
            let m = if *n < 20 { *n } else { 20 }; for i in 0..m {
                if dec[i] != data[i] {
                    println!("    diff at {}: exp {} got {}", i, data[i], dec[i]);
                    break;
                }
            }
        }
    }
}
