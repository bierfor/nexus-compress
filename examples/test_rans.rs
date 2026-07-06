// Standalone rANS test to debug

use nexus_compress::rans::{FreqTable, rans_encode, rans_decode};

fn main() {
    let table = FreqTable::uniform(12);
    println!("scale = {}, n_symbols = {}", table.scale, table.n_symbols);

    // Try data = [0, 1, 2, 3, 4]
    let data: Vec<u8> = vec![0, 1, 2, 3, 4];
    let enc = rans_encode(&data, &table);
    println!("encoded = {:?}", enc);
    println!("encoded size = {}", enc.len());

    let dec = rans_decode(&enc, &table, data.len());
    println!("decoded = {:?}", dec);
    println!("match = {}", dec == data);

    // Manual trace of the encoder
    let mut state: u32 = table.scale;
    println!("\nManual encode trace:");
    for &b in data.iter().rev() {
        let idx = table.sym_to_idx[b as usize] as usize;
        let freq = table.freq[idx];
        let cum = table.cum[idx];
        let max_state = (table.scale / freq) * freq;
        if state >= max_state {
            println!("  renorm: state {} -> {} (push {})", state, state >> 8, state & 0xFF);
            state >>= 8;
        }
        let old_state = state;
        state = ((state / freq) * table.scale) + (state % freq) + cum;
        println!("  b={} (idx={}, freq={}, cum={}): state {} -> {}", b, idx, freq, cum, old_state, state);
    }
    // Final renorm
    while state >= table.scale {
        println!("  final renorm: state {} -> {} (push {})", state, state >> 8, state & 0xFF);
        state >>= 8;
    }
    println!("  final state (after renorm): {}", state);
    println!("  final state bytes: {}, {}", state & 0xFF, (state >> 8) & 0xFF);
    println!();

    // Manual trace of the decoder
    println!("Manual decode trace:");
    let mut p = enc.len();
    let mut state: u32 = 0;
    while state < table.scale {
        p -= 1;
        state = (state << 8) | (enc[p] as u32);
        println!("  read byte {} -> state = {}", enc[p], state);
    }
    println!("Initial state for decoding: {}", state);

    for i in 0..data.len() {
        let slot = state & table.scale_mask;
        let idx = table.lut[slot as usize] as usize;
        let freq = table.freq[idx];
        let cum = table.cum[idx];
        let old_state = state;
        state = freq * (state / table.scale) + (state % table.scale) - cum;
        println!("  iter {}: slot={} -> sym={} (idx={}, freq={}, cum={}): state {} -> {}",
            i, slot, table.idx_to_sym[idx], idx, freq, cum, old_state, state);
    }
}