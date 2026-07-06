use nexus_compress::rans::{rans_encode, rans_decode, FreqTable};

fn main() {
    let table = FreqTable::uniform(12);
    for n in [1, 5, 32, 256].iter() {
        let data: Vec<u8> = (0..*n).map(|i| (i % 256) as u8).collect();
        let enc = rans_encode(&data, &table);
        let dec = rans_decode(&enc, &table, data.len());
        let matches = dec == data;
        println!("n={} enc_len={} matches={}", n, enc.len(), matches);
        if !matches {
            println!("  expected: {:?}", &data[..data.len().min(10)]);
            println!("  got:      {:?}", &dec[..dec.len().min(10)]);
            println!("  enc[:20]: {:?}", &enc[..enc.len().min(20)]);
        }
    }
}
