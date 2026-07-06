use nexus_compress::rans::FreqTable;

fn main() {
    let mut counts = [1u32; 256];
    for &b in b"the quick brown fox jumps over the lazy dog" {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    // Find which sym has cum <= 3811
    let target_slot = 3811;
    let mut found = false;
    for idx in 0..table.n_symbols as usize {
        let cum = table.cum[idx];
        let freq = table.freq[idx];
        if cum <= target_slot && target_slot < cum + freq {
            println!("slot {} maps to idx {} (sym {}) cum={} freq={}", target_slot, idx, table.idx_to_sym[idx], cum, freq);
            found = true;
            break;
        }
    }
    if !found {
        println!("slot {} not found in any cum range", target_slot);
    }
    
    // Also dump cum near target
    println!("\ncum near 3811:");
    for idx in 0..table.n_symbols as usize {
        let cum = table.cum[idx];
        let cum_next = table.cum[idx+1];
        if cum_next > 3500 && cum < 4500 {
            println!("  idx={} (sym {}) cum={} cum_next={} freq={}", idx, table.idx_to_sym[idx], cum, cum_next, table.freq[idx]);
        }
    }
    
    // Verify decoder state computation
    let bytes = vec![102u8, 0, 151, 227, 218, 54, 80];
    let mut state: u64 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        state = state * 256 + b as u64;
        println!("after {} bytes: state={} slot={}", i+1, state, state & 4095);
    }
    
    println!("\nencoder pushes: [80, 54, 218, 227, 151, 0, 102]");
    println!("decoder reads from end: {:?}", bytes);
}
