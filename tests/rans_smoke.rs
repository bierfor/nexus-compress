//! Smoke test for the `rans` crate (ryg_rans wrapper).
//! Verify roundtrip works before integrating into codec.

use rans::byte_decoder::{ByteRansDecSymbol, ByteRansDecoder};
use rans::byte_encoder::{ByteRansEncSymbol, ByteRansEncoder};
use rans::{RansDecSymbol, RansDecoder, RansEncSymbol, RansEncoder, RansEncoderMulti};

#[test]
fn smoke_roundtrip_alphabet_4() {
    // Alphabet: 4 symbols (0, 1, 2, 3), each with frequency 1.
    // cum = [0, 1, 2, 3, 4], total = 4, scale_bits = 2 (so total fits in 4 = 2^2).
    const SCALE_BITS: u32 = 2;
    const N: usize = 200;

    // Build symbols
    let sym0 = ByteRansEncSymbol::new(0, 1, SCALE_BITS);
    let sym1 = ByteRansEncSymbol::new(1, 1, SCALE_BITS);
    let sym2 = ByteRansEncSymbol::new(2, 1, SCALE_BITS);
    let sym3 = ByteRansEncSymbol::new(3, 1, SCALE_BITS);
    let symbols = [&sym0, &sym1, &sym2, &sym3];

    // Generate a test sequence
    let mut data: Vec<u8> = Vec::with_capacity(N);
    let mut s: u32 = 0xdeadbeef;
    for _ in 0..N {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        data.push(s as u8 % 4);
    }

    // Encode
    let mut encoder = ByteRansEncoder::new(1024);
    for &sym in &data {
        encoder.put(symbols[sym as usize]);
    }
    encoder.flush();
    let encoded = encoder.data().to_owned();

    // Decode (in reverse)
    let mut decoder = ByteRansDecoder::new(encoded);
    let dec0 = ByteRansDecSymbol::new(0, 1);
    let dec1 = ByteRansDecSymbol::new(1, 1);
    let dec2 = ByteRansDecSymbol::new(2, 1);
    let dec3 = ByteRansDecSymbol::new(3, 1);
    let dec_symbols = [&dec0, &dec1, &dec2, &dec3];

    let mut decoded: Vec<u8> = Vec::with_capacity(N);
    for _ in 0..N {
        let cum_freq = decoder.get(SCALE_BITS);
        let sym_idx = (cum_freq) as usize; // cum_freq == sym_idx in our alphabet
        decoded.push(sym_idx as u8);
        decoder.advance(dec_symbols[sym_idx], SCALE_BITS);
    }
    decoded.reverse(); // decoder returns in reverse order

    assert_eq!(decoded, data, "rans crate roundtrip failed");
    println!("rans crate works: encoded {} bytes from {} input symbols",
             encoder.data().len(), N);
}

#[test]
fn smoke_roundtrip_skewed_99_1() {
    // 99 syms of '0' (cum=0, freq=99), 1 sym of '1' (cum=99, freq=1).
    // total = 100, scale_bits = ceil(log2(100)) = 7 (so total = 128).
    const SCALE_BITS: u32 = 7;
    const N: usize = 200;

    let sym0 = ByteRansEncSymbol::new(0, 99, SCALE_BITS);
    let sym1 = ByteRansEncSymbol::new(99, 1, SCALE_BITS);

    let mut data: Vec<u8> = Vec::with_capacity(N);
    // 199 zeros, then 1 one.
    for _ in 0..(N - 1) { data.push(0); }
    data.push(1);

    let mut encoder = ByteRansEncoder::new(1024);
    for &sym in &data {
        if sym == 0 { encoder.put(&sym0); } else { encoder.put(&sym1); }
    }
    encoder.flush();
    let encoded = encoder.data().to_owned();

    let mut decoder = ByteRansDecoder::new(encoded);
    let dec0 = ByteRansDecSymbol::new(0, 99);
    let dec1 = ByteRansDecSymbol::new(99, 1);

    let mut decoded: Vec<u8> = Vec::with_capacity(N);
    for _ in 0..N {
        let cum_freq = decoder.get(SCALE_BITS);
        let sym_idx = if cum_freq < 99 { 0 } else { 99 };
        decoded.push(if sym_idx == 0 { 0 } else { 1 });
        decoder.advance(if sym_idx == 0 { &dec0 } else { &dec1 }, SCALE_BITS);
    }
    decoded.reverse();

    assert_eq!(decoded, data, "rans crate skewed roundtrip failed");
}