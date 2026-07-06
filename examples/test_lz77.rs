use nexus_compress::lz77::MatchFinder;

fn main() {
    let data: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    let mut enc = MatchFinder::new();
    let ops = enc.encode(&data);
    
    let mut n_lits = 0;
    let mut n_matches = 0;
    let mut n_dict_refs = 0;
    let mut total_match_len = 0u64;
    for op in &ops {
        match op {
            nexus_compress::lz77::Op::Lit(_) => n_lits += 1,
            nexus_compress::lz77::Op::Match { len, .. } => {
                n_matches += 1;
                total_match_len += *len as u64;
            }
            nexus_compress::lz77::Op::DictRef { len, .. } => {
                n_dict_refs += 1;
                total_match_len += *len as u64;
            }
        }
    }
    println!("n_ops={} n_lits={} n_matches={} n_dict_refs={} avg_match_len={}",
        ops.len(), n_lits, n_matches, n_dict_refs,
        if n_matches > 0 { total_match_len / n_matches as u64 } else { 0 });
    
    // Decode
    let mut dec = nexus_compress::lz77::MatchDecoder::new();
    let decoded = dec.decode(&ops);
    println!("decoded len: {} (expected {})", decoded.len(), data.len());
    if decoded != data {
        for i in 0..data.len().min(100) {
            if decoded[i] != data[i] {
                println!("  first diff at {}: exp {} got {}", i, data[i], decoded[i]);
                break;
            }
        }
    }
}
