use nexus_compress::rans::FreqTable;

fn main() {
    let data_full: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    let mut counts = [1u32; 256];
    for &b in &data_full {
        counts[b as usize] += 1;
    }
    
    let total: u32 = counts.iter().sum();
    println!("total counts: {}", total);
    
    let scale: u64 = 4096;
    for sym in 0..5 {
        let v = (counts[sym] as u64 * scale) / total as u64;
        println!("counts[{}]={} → scaled={} → max(1, {})={}", sym, counts[sym], v, v, std::cmp::max(1, v));
    }
    
    let table = FreqTable::from_counts(&counts, 12);
    println!("\ntable.cum[256] = {}", table.cum[256]);
    println!("table.scale = {}", table.scale);
    println!("table.n_symbols = {}", table.n_symbols);
}
