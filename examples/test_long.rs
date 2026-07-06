use nexus_compress::rans::{rans_encode, rans_decode, FreqTable};

fn main() {
    let mut counts = [1u32; 256];
    for &b in b"the quick brown fox jumps over the lazy dog" {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    let phrase = b"the quick brown fox jumps over the lazy dog ";
    
    for &n_copies in [10, 100, 1000, 2000].iter() {
        let mut data = Vec::new();
        for _ in 0..n_copies {
            data.extend_from_slice(phrase);
        }
        let enc = rans_encode(&data, &table);
        let dec = rans_decode(&enc, &table, data.len());
        let matches = dec == data;
        println!("n_copies={} data_len={} enc_len={} matches={}", n_copies, data.len(), enc.len(), matches);
        if !matches {
            let first_diff = dec.iter().zip(data.iter()).position(|(a, b)| a != b);
            if let Some(i) = first_diff {
                println!("  first diff at byte {}", i);
                let lo = i.saturating_sub(2);
                let hi = (i+10).min(data.len());
                println!("  expected [{:?}]", &data[lo..hi]);
                println!("  got      [{:?}]", &dec[lo..hi]);
            }
            break;
        }
    }
}
