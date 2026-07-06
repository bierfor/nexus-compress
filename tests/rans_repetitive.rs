//! Test rANS crate directly on repetitive data to isolate codec bugs.
use rans::byte_decoder::{ByteRansDecSymbol, ByteRansDecoder};
use rans::byte_encoder::{ByteRansEncSymbol, ByteRansEncoder};
use rans::{RansDecSymbol, RansDecoder, RansEncSymbol, RansEncoder, RansEncoderMulti};

#[test]
fn repetitive_rans_only() {
    // 1000 syms of just 'a' (ascii 97) - extreme repetitive.
    const SCALE_BITS: u32 = 8;
    const N: usize = 1000;

    let data: Vec<u8> = vec![97; N];

    let sym_a = ByteRansEncSymbol::new(97, 1, SCALE_BITS);

    let mut encoder = ByteRansEncoder::new(N * 4);
    for _ in 0..N {
        encoder.put(&sym_a);
    }
    encoder.flush();
    let encoded = encoder.data().to_owned();
    println!("encoded {} syms of 'a' to {} bytes", N, encoded.len());

    let mut decoder = ByteRansDecoder::new(encoded);
    let dec_a = ByteRansDecSymbol::new(97, 1);

    let mut decoded: Vec<u8> = Vec::with_capacity(N);
    for _ in 0..N {
        let cum = decoder.get(SCALE_BITS);
        let s = cum; // for uniform alphabet of 256, cum == symbol
        decoded.push(s as u8);
        decoder.advance(&dec_a, SCALE_BITS);
    }
    decoded.reverse();
    assert_eq!(decoded, data);
}