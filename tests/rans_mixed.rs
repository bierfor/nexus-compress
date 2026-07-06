//! Test rANS wrapper with mixed data (like a real codec lit stream).
use nexus_compress::rans_v4::{rans_decode, rans_encode, FreqTable};

#[test]
fn mixed_repetitive() {
    // Like the real codec lit stream: first occurrence of phrase = 41 bytes,
    // then a few more literals from match edges.
    let data: Vec<u32> = b"the quick brown fox jumps over the lazy dog. "
        .iter().map(|&x| x as u32).collect();
    assert_eq!(data.len(), 45);

    let counts = {
        let mut c = [0u32; 256];
        for &s in &data {
            c[s as usize] += 1;
        }
        c
    };
    let table = FreqTable::from_counts(&counts);
    let stream = rans_encode(&data, &table);
    let decoded = rans_decode(&stream, &table, data.len());
    assert_eq!(decoded, data, "mixed 45-byte roundtrip failed");
}

#[test]
fn mixed_repetitive_2x() {
    let base: Vec<u32> = b"the quick brown fox jumps over the lazy dog. "
        .iter().map(|&x| x as u32).collect();
    let data: Vec<u32> = base.iter().chain(base.iter()).cloned().collect();

    let counts = {
        let mut c = [0u32; 256];
        for &s in &data {
            c[s as usize] += 1;
        }
        c
    };
    let table = FreqTable::from_counts(&counts);
    let stream = rans_encode(&data, &table);
    let decoded = rans_decode(&stream, &table, data.len());
    assert_eq!(decoded, data, "2x mixed roundtrip failed");
}