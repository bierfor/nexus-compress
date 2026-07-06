use nexus_compress::rans::{rans_encode, FreqTable};

fn main() {
    let table = FreqTable::uniform(12);
    let data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let enc = rans_encode(&data, &table);
    println!("encoded {} bytes for 256 symbols", enc.len());
    println!("first 20: {:?}", &enc[..20]);
    println!("last 20: {:?}", &enc[enc.len()-20..]);
    
    // Now compare with what we expect
    println!("\nWhat we expect:");
    println!("- initial renorm push: 0");
    println!("- per-sym renorm pushes: (state after sym) & 0xFF");
    println!("- final flush pushes");
}
