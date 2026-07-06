//! Direct test of rans_v4 wrapper on repetitive streams from the codec.
use nexus_compress::rans_v4::{decode_table, encode_table, rans_decode, rans_encode, FreqTable};

#[test]
fn repetitive_lit_stream() {
    // 4500 literals, all 'a' (97). Reproduces what happens with repetitive
    // data in the codec.
    const N: usize = 4500;
    let data: Vec<u32> = vec![97; N];

    let counts = {
        let mut c = [0u32; 256];
        for &s in &data {
            c[s as usize] += 1;
        }
        c
    };
    let table = FreqTable::from_counts(&counts);
    let stream = rans_encode(&data, &table);
    let decoded = rans_decode(&stream, &table, N);
    assert_eq!(decoded, data, "rans_v4 repetitive roundtrip failed ({} bytes)", stream.len());
}

#[test]
fn repetitive_long_dist_lo() {
    // Match distances in repetitive data are uniform: typically dist = phrase_len.
    // For "the quick brown fox jumps over the lazy dog. " (45 chars), all matches
    // have dist_lo=44 (0x2C) and dist_hi=0.
    const N: usize = 4500;
    let data: Vec<u32> = vec![44; N];

    let counts = {
        let mut c = [0u32; 256];
        for &s in &data {
            c[s as usize] += 1;
        }
        c
    };
    let table = FreqTable::from_counts(&counts);
    let stream = rans_encode(&data, &table);
    let decoded = rans_decode(&stream, &table, N);
    assert_eq!(decoded, data, "rans_v4 dist_lo roundtrip failed");
}

#[test]
fn repetitive_long_len() {
    // Match lengths in repetitive data are uniform: typically len = 45.
    const N: usize = 4500;
    let data: Vec<u32> = vec![45; N];

    let counts = {
        let mut c = [0u32; 256];
        for &s in &data {
            c[s as usize] += 1;
        }
        c
    };
    let table = FreqTable::from_counts(&counts);
    let stream = rans_encode(&data, &table);
    let decoded = rans_decode(&stream, &table, N);
    assert_eq!(decoded, data, "rans_v4 len roundtrip failed");
}