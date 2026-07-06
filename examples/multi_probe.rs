use std::io::Read;
fn main() {
    for name in &["code.rs", "text.txt", "data.json", "mixed.bin", "trained.dict"] {
        let data = std::fs::read(format!("corpus/{}", name)).unwrap();
        let streams = nexus_compress::cost_probe::probe::stream_bits_per_symbol(&data);
        eprintln!("\n{} ({} B):", name, data.len());
        for s in streams {
            eprintln!("  {:10}  syms={:8}  bits/sym={:.2}", s.name, s.symbols, s.bits_per_symbol);
        }
    }
}
