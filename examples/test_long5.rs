use nexus_compress::rans::FreqTable;

fn main() {
    let mut counts = [1u32; 256];
    for &b in b"the quick brown fox jumps over the lazy dog" {
        counts[b as usize] += 1;
    }
    let table = FreqTable::from_counts(&counts, 12);

    let q_idx = table.sym_to_idx['q' as usize];
    println!("'q' idx={} cum={} freq={}", q_idx, table.cum[q_idx as usize], table.freq[q_idx as usize]);
    let slot: u32 = 2019;
    if table.cum[q_idx as usize] <= slot && slot < table.cum[q_idx as usize + 1] {
        println!("slot 2019 IS in 'q' range!");
    }

    let state: u64 = 1711314915;
    let scale: u64 = 4096;
    let freq: u64 = 28;
    let cum: u64 = 1998;
    let new_state = freq * (state / scale) + (state % scale) - cum;
    println!("Iter 1: state={} slot=2019 (q) state_new={}", state, new_state);

    let x_max: u64 = 1040187392;
    let mut s = new_state;
    while s < x_max {
        break;
    }
    println!("Iter 2: state after refill={}", s);
}