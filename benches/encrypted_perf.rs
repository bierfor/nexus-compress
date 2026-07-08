use nexus_compress::encrypted::{compress_encrypted, decompress_encrypted, EncryptOptions, RecoveryLevel};
use std::time::Instant;
use std::fs;

fn main() {
    let data = fs::read("/tmp/perf_in.bin").expect("read");
    println!("Input: {} bytes ({:.1} MiB)", data.len(), data.len() as f64 / 1024.0 / 1024.0);
    let opts = EncryptOptions::new(b"perf-test", RecoveryLevel::Low);

    // Compress
    let t0 = Instant::now();
    let encrypted = compress_encrypted(&data, &opts, |_ev| {}).expect("compress");
    let compress_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let comp_mbps = (data.len() as f64 / 1024.0 / 1024.0) / (compress_ms / 1000.0);
    println!("Compress: {:.1} ms ({:.1} MB/s), output {} bytes", compress_ms, comp_mbps, encrypted.len());

    // Decompress (clean)
    let t0 = Instant::now();
    let decrypted = decompress_encrypted(&encrypted, b"perf-test", |_ev| {}).expect("decompress");
    let decompress_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let decomp_mbps = (data.len() as f64 / 1024.0 / 1024.0) / (decompress_ms / 1000.0);
    println!("Decompress: {:.1} ms ({:.1} MB/s)", decompress_ms, decomp_mbps);
    assert_eq!(decrypted, data);
    println!("OK: roundtrip byte-identical");
}
