//! End-to-end codec roundtrip test using v4 rANS.
use nexus_compress::{compress, decompress};

#[test]
fn roundtrip_small() {
    let data: Vec<u8> = b"hello world! this is a test of the codec. ".repeat(100);
    let compressed = compress(&data);
    let decompressed = decompress(&compressed);
    assert_eq!(decompressed, data, "roundtrip failed for small text");
}

#[test]
fn roundtrip_random() {
    let mut data = Vec::with_capacity(10000);
    let mut s: u32 = 0xc0ffee;
    for _ in 0..10000 {
        s = s.wrapping_mul(1103515245).wrapping_add(12345);
        data.push((s >> 16) as u8);
    }
    let compressed = compress(&data);
    let decompressed = decompress(&compressed);
    assert_eq!(decompressed, data, "roundtrip failed for random");
}

#[test]
fn roundtrip_repetitive_small() {
    let data: Vec<u8> = b"the quick brown fox jumps over the lazy dog. ".repeat(10);
    let compressed = compress(&data);
    let decompressed = decompress(&compressed);
    assert_eq!(decompressed, data, "roundtrip failed for repetitive 10x");
}

#[test]
fn roundtrip_repetitive_threshold() {
    let data: Vec<u8> = b"the quick brown fox jumps over the lazy dog. ".repeat(97);
    eprintln!("[test] starting 97x");
    let compressed = compress(&data);
    eprintln!("[test] compressed len: {}", compressed.len());
    let decompressed = decompress(&compressed);
    eprintln!("[test] decompressed len: {}", decompressed.len());
    assert_eq!(decompressed, data, "roundtrip failed for repetitive 97x");
}

#[test]
fn roundtrip_repetitive() {
    let data: Vec<u8> = b"the quick brown fox jumps over the lazy dog. ".repeat(1000);
    let compressed = compress(&data);
    let decompressed = decompress(&compressed);
    assert_eq!(decompressed, data, "roundtrip failed for repetitive 1000x");
}