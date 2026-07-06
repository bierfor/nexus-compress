use nexus_compress::rans::{FreqTable};

fn main() {
    let mut counts = [1u32; 256];
    for &b in b"the quick brown fox jumps over the lazy dog" {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);
    
    // Manually encode "the q"
    let data: &[u8] = b"the q";
    let scale = table.scale as u64;
    let mut state: u64 = 8388608; // RANS_BYTE_L
    let mut out: Vec<u8> = Vec::new();
    
    for &b in data.iter() {
        let idx = table.sym_to_idx[b as usize] as usize;
        let freq = table.freq[idx] as u64;
        let cum = table.cum[idx] as u64;
        let x_max = (8388608u64 / scale) * scale * freq;
        
        // Renorm
        let mut pushes = Vec::new();
        while state >= x_max {
            pushes.push(state & 0xFF);
            state >>= 8;
        }
        
        // Encode
        state = (state / freq) * scale + (state % freq) + cum;
        println!("sym '{}' idx={} freq={} cum={} x_max={} pushes={:?} state_after={}", b as char, idx, freq, cum, x_max, pushes, state);
    }
    
    // Final flush
    let mut final_pushes = Vec::new();
    while state >= scale {
        final_pushes.push(state & 0xFF);
        state >>= 8;
    }
    while state >= 256 {
        final_pushes.push(state & 0xFF);
        state >>= 8;
    }
    final_pushes.push(state);
    println!("final state={} pushes={:?}", state, final_pushes);
    
    // Total encoded bytes (in order pushed)
    let mut all_pushes = Vec::new();
    // Need to redo encoding to collect all pushes
    state = 8388608;
    for &b in data.iter() {
        let idx = table.sym_to_idx[b as usize] as usize;
        let freq = table.freq[idx] as u64;
        let cum = table.cum[idx] as u64;
        let x_max = (8388608u64 / scale) * scale * freq;
        while state >= x_max {
            all_pushes.push(state & 0xFF);
            state >>= 8;
        }
        state = (state / freq) * scale + (state % freq) + cum;
    }
    while state >= scale {
        all_pushes.push(state & 0xFF);
        state >>= 8;
    }
    while state >= 256 {
        all_pushes.push(state & 0xFF);
        state >>= 8;
    }
    all_pushes.push(state);
    println!("\nAll pushes (in order): {:?}", all_pushes);
    println!("Actual enc: [80, 54, 218, 227, 151, 0, 102]");
}
