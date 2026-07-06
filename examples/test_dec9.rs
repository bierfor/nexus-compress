use nexus_compress::rans::{rans_encode_streamed, rans_decode_streamed, FreqTable};

fn main() {
    // Build a table where all symbols have the same non-16 freq
    let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    
    // Build counts such that freq ends up uniform 17 for all symbols
    // Total counts = 256 * 17 = 4352. But scale = 4096. So can't have all freq=17.
    // Can have 240*17 + 16*1 = 4096. Hmm 240*17=4080 + 16*1=16 → 4096. 
    // So 240 syms with freq=17, 16 syms with freq=1.
    
    // Test 1: table where 240 syms have freq=17, 16 syms have freq=1
    let mut freq = vec![1u32; 256];
    for i in 0..240 {
        freq[i] = 17;
    }
    // Build cum
    let mut cum = vec![0u32; 257];
    for i in 0..256 {
        cum[i+1] = cum[i] + freq[i];
    }
    println!("cum[256]={} scale={}", cum[256], 4096);
    
    let mut sym_to_idx = [0xFFFFu16; 256];
    let mut idx_to_sym = Vec::new();
    for i in 0..256 {
        sym_to_idx[i] = i as u16;
        idx_to_sym.push(i as u8);
    }
    let lut: Vec<u16> = (0..4096).map(|s| {
        // Find which sym contains slot s
        let mut lo = 0;
        let mut hi = 255;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if cum[mid+1] <= s {
                lo = mid + 1;
            } else if cum[mid] > s {
                hi = mid - 1;
            } else {
                lo = mid;
                hi = mid;
            }
        }
        lo as u16
    }).collect();
    
    let table = FreqTable {
        cum: cum.clone(),
        freq: freq.clone(),
        lut,
        scale_bits: 12,
        scale_mask: 4095,
        scale: 4096,
        n_symbols: 256,
        sym_to_idx,
        idx_to_sym,
    };
    
    let enc = rans_encode_streamed(&data, &table);
    let dec = rans_decode_streamed(&enc, &table, data.len());
    println!("matches: {}", dec == data);
    if dec != data {
        for i in 0..20 {
            if dec[i] != data[i] {
                println!("  diff at {}: exp {} got {}", i, data[i], dec[i]);
                break;
            }
        }
    }
}
