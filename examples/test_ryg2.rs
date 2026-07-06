use nexus_compress::rans::{rans_encode, rans_decode, FreqTable};

fn main() {
    let table = FreqTable::uniform(12);
    let data: Vec<u8> = vec![0, 1, 2, 3, 4];
    let enc = rans_encode(&data, &table);
    println!("enc: {:?}", &enc);
    println!("enc len: {}", enc.len());
    
    // Manually decode to see what happens
    let max_freq = 16u64;
    let scale = 4096u64;
    let x_max_init = (8388608u64 / scale) * scale * max_freq;
    println!("x_max_init: {} (2^{})", x_max_init, (x_max_init as f64).log2());
    
    let mut state: u64 = 0;
    let mut p = enc.len();
    while state < x_max_init && p > 0 {
        p -= 1;
        state = (state << 8) | (enc[p] as u64);
        println!("init read: p={} byte={} state={}", p, enc[p], state);
    }
    println!("after init: state={}", state);
    
    let mut out = Vec::new();
    for iter in 0..5 {
        let slot = (state & 4095) as u32;
        let idx = table.lut[slot as usize] as usize;
        let freq = table.freq[idx] as u64;
        let x_max = (8388608u64 / scale) * scale * freq;
        while state < x_max && p > 0 {
            p -= 1;
            state = (state << 8) | (enc[p] as u64);
            println!("  iter {} refill: p={} byte={} state={}", iter, p, enc[p], state);
        }
        let cum = table.cum[idx] as u64;
        state = freq * (state / scale) + (state % scale) - cum;
        println!("iter {}: slot={} idx={} cum={} freq={} state={} pushed={}", iter, slot, idx, cum, freq, state, table.idx_to_sym[idx]);
        out.push(table.idx_to_sym[idx]);
    }
    println!("out (pre-reverse): {:?}", &out);
    out.reverse();
    println!("out (post-reverse): {:?}", &out);
}
