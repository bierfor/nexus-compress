//! Top-level codec — pipeline orchestrator.
//!
//! ## v2 pipeline (Format v2 — current)
//! 1. Split input via CDC (Gear hash, min=4K avg=32K max=64K).
//! 2. For each chunk:
//!    a. Compute FNV-1a hash, check dedup table.
//!    b. If duplicate: emit `BlockType::Duplicate` referencing original.
//!    c. If unique: classify, then LZ77 with optimal-parsing DP (v2 cost
//!       model: literal=13 bits, match=32 bits → optimal wins on L>=3).
//!    d. rANS-encode the literal bytes.
//!    e. Write op stream: 1 byte flag per literal (no placeholder),
//!       4 bytes per match (u16 dist + u8 len).
//!    f. If compressed payload >= raw block size: fall back to `Raw`.
//! 3. Emit header with version=3.
//!
//! ## Decoder
//! Reads header version. Handles v0, v2, and v3 compressed payloads.
//! Maintains `Vec<Vec<u8>>` cache for Duplicate lookups.
//!
//! ## v3 multi-stream rANS (the current default)
//!
//! Instead of one shared rANS stream for everything, v3 splits the
//! compressed payload into three independent rANS streams with their
//! own frequency tables:
//!
//! 1. **Literals stream** — raw byte values from `Op::Lit(b)` entries.
//! 2. **Lengths stream** — match lengths (u8, 1..=255).
//! 3. **Distances stream** — match distances, encoded as TWO u8
//!    symbols per match (low byte, then high byte) so we keep the
//!    256-symbol rANS alphabet while gaining entropy benefit on the
//!    high byte (which is heavily skewed toward small values for
//!    local references in structured data).
//!
//! An op-flags stream (one byte per op: 0 = literal, 1 = match) tells
//! the decoder how to interleave the three streams.
//!
//! Why this wins even without changing LZ77:
//! - Distance's high byte is non-uniform (most distances < 256 for
//!   source code) → rANS gives big wins.
//! - Length has its own skewed distribution (3, 4, 8, 16, ... are
//!   common) → rANS picks this up.
//! - Literals stay where they were (already optimal in v2).
//! - Total: lazy matching finds matches that the v3 format can encode
//!   more cheaply.

use crate::cdc;
use crate::classifier::{classify, BlockStats};
use crate::dedup::{DedupResult, DedupTable};
use crate::format::{
    BlockHeader, BlockType, NexusHeader, VERSION_V0, VERSION_V2, VERSION_V3,
};
use crate::lz77::{MatchDecoder, MatchFinder, Op};
use crate::rans_v4::{decode_table, rans_decode, rans_encode, encode_table, FreqTable};
use std::io::{Cursor, Read};

const TAG_RAW: u8 = 1;
const TAG_RANS_LITERALS_V0: u8 = 2;
const TAG_DUPLICATE: u8 = 3;
const TAG_RANS_LITERALS_V2: u8 = 4;
const TAG_V3_MULTISTREAM: u8 = 5;
const TAG_V3_MULTISTREAM_RLE: u8 = 7;

/// CDC chunk size parameters.
///
/// `max = 64 KB` is the sweet spot:
/// - On uniform/repetitive data, the Gear hash never finds a natural
///   boundary, so we hit max_chunk → chunks align at 64K offsets →
///   dedup matches them across copies. This preserves fixed-block
///   performance on the pathological cases.
/// - On real data with content variation, the Gear hash finds natural
///   boundaries every ~32 KB (avg). When a file is modified between
///   versions (the CDC use case), the boundaries before the
///   modification point stay aligned → dedup matches across versions.
/// - Matches the LZ77 window size, so dedup and LZ77 cooperate: a
///   duplicate chunk never loses intra-block compression quality
///   versus treating it as a normal block.
const CDC_MIN_CHUNK: usize = 4 * 1024; // 4 KB
const CDC_AVG_BITS: u32 = 15; // 2^15 = 32 KB average
const CDC_MAX_CHUNK: usize = 64 * 1024; // 64 KB (== LZ77 window)

pub fn compress(input: &[u8]) -> Vec<u8> {
    let total_uncompressed = input.len() as u64;

    // Content-defined chunking: variable-size blocks aligned to content
    // boundaries. Each chunk is then dedup'd + LZ77 + rANS compressed.
    let chunk_specs: Vec<(usize, usize)> = if input.is_empty() {
        vec![]
    } else {
        cdc::chunkify(input, CDC_MIN_CHUNK, CDC_MAX_CHUNK, CDC_AVG_BITS)
    };

    // Slice the input into chunks per the CDC boundaries
    let chunks: Vec<&[u8]> = chunk_specs
        .iter()
        .map(|&(off, len)| &input[off..off + len])
        .collect();

    let mut out = Vec::with_capacity(input.len() + 64);
    let header = NexusHeader {
        version: VERSION_V3,
        flags: 0,
        block_count: chunks.len() as u32,
        uncompressed_total_size: total_uncompressed,
    };
    header.write(&mut out).unwrap();

    let mut dedup = DedupTable::new();
    for (block_id, block) in chunks.iter().enumerate() {
        let block_id_u32 = block_id as u32;

        // Dedup check
        if !block.is_empty() {
            match dedup.lookup(block, block_id_u32) {
                DedupResult::Duplicate { original_id } => {
                    // Emit Duplicate block: payload = [tag][u32 original_id]
                    let mut payload = Vec::with_capacity(5);
                    payload.push(TAG_DUPLICATE);
                    payload.extend_from_slice(&original_id.to_le_bytes());
                    let bh = BlockHeader {
                        block_type: BlockType::Duplicate,
                        uncompressed_size: block.len() as u32,
                        compressed_size: payload.len() as u32,
                    };
                    bh.write(&mut out).unwrap();
                    out.extend_from_slice(&payload);
                    continue;
                }
                DedupResult::Unique { .. } => {
                    // Continue to normal compression
                }
            }
        }

        // Normal compression path
        let mut block_type = if block.is_empty() {
            BlockType::Unknown
        } else {
            classify(block)
        };
        let stats = if block.is_empty() {
            BlockStats {
                size: 0,
                entropy: 0.0,
                printable_ratio: 0.0,
                struct_score: 0.0,
                run_avg: 0.0,
            }
        } else {
            crate::classifier::compute_stats(block)
        };
        let payload = encode_block(block, &mut block_type, &stats);
        let bh = BlockHeader {
            block_type,
            uncompressed_size: block.len() as u32,
            compressed_size: payload.len() as u32,
        };
        bh.write(&mut out).unwrap();
        out.extend_from_slice(&payload);
    }
    out
}

fn encode_block(block: &[u8], block_type: &mut BlockType, stats: &BlockStats) -> Vec<u8> {
    if block.is_empty() {
        return vec![];
    }

    // Try the standard v3 multistream path.
    let standard = encode_v3_multistream(block).map(|mut payload| {
        payload.insert(0, TAG_V3_MULTISTREAM);
        payload
    });

    // Try the RLE pre-filter path. RLE shrinks input when there are runs
    // of ≥2 same bytes; it expands when bytes are all unique AND have
    // values >= 0x80 (because each high byte becomes a 2-byte "run of 1").
    //
    // Gating heuristic: only try RLE when the block has a non-trivial
    // average run length (>= 1.1 means at least 10% of adjacent byte pairs
    // are equal). On blocks where this isn't true, the standard path is
    // faster and produces equivalent output — RLE would just be wasted
    // LZ77+rans work to confirm the same answer.
    let rle_path = if stats.run_avg >= 1.1 {
        let rle_block = crate::rle::compress_rle(block);
        if rle_block.len() < block.len() {
            encode_v3_multistream(&rle_block).map(|mut payload| {
                payload.insert(0, TAG_V3_MULTISTREAM_RLE);
                payload
            })
        } else {
            None
        }
    } else {
        None
    };

    match (standard, rle_path) {
        (Some(s), Some(r)) if r.len() < s.len() => r,
        (Some(s), _) => s,
        (None, Some(r)) => r,
        (None, None) => {
            *block_type = BlockType::Raw;
            let mut raw = Vec::with_capacity(1 + block.len());
            raw.push(TAG_RAW);
            raw.extend_from_slice(block);
            raw
        }
    }
}

/// Build the v3 multistream payload (without the leading tag byte) for
/// the given input data. Returns None if the encoded form is not smaller
/// than the input (incompressible).
fn encode_v3_multistream(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }

    // 1. LZ77 with hash chains + lazy matching.
    //
    //    v3 still uses lazy matching (same as v2). Optimal parsing is
    //    now in better shape thanks to per-byte rANS distance (the
    //    close-distance bias is real now) but we keep lazy default
    //    because it still wins on source code (lazy finds 30.30x on
    //    code.rs; optimal on v2 found 2.23x). Re-enabling optimal is
    //    a follow-up after v3 baseline lands.
    let mut mf = MatchFinder::new();
    let ops = mf.encode(data);

    // 2. Split ops into THREE independent streams for v3.
    //
    //    Why three: LZ77 op data has different statistics per field.
    //    - Literals: full byte range (0..=255), skewed toward ASCII.
    //    - Lengths: small range (3..=255), highly skewed toward 3-16.
    //    - Distances: 1..=65535, with high byte biased toward 0
    //      (most matches are local in structured data).
    //
    //    Mixing them in one rANS table destroys each field's local
    //    entropy. Separate tables let rANS exploit each field's
    //    specific distribution.
    let mut literals: Vec<u8> = Vec::new();
    let mut lengths: Vec<u8> = Vec::new();
    let mut dist_lows: Vec<u8> = Vec::new();
    let mut dist_highs: Vec<u8> = Vec::new();
    for op in &ops {
        match op {
            Op::Lit(b) => literals.push(*b),
            Op::Match { dist, len } => {
                lengths.push(*len as u8);
                dist_lows.push((*dist & 0xFF) as u8);
                dist_highs.push(((*dist >> 8) & 0xFF) as u8);
            }
            Op::DictRef { .. } => unreachable!(
                "DictRef ops are only produced by encode_with_dict, which the codec \
                 does not call yet. This is a bug."
            ),
        }
    }

    // 3. Build frequency tables and rANS-encode each stream.
    let lit_table = build_table_with_precision(&literals, 12);
    let len_table = build_table_with_precision(&lengths, 12);
    let dist_lo_table = build_table_with_precision(&dist_lows, 12);
    let dist_hi_table = build_table_with_precision(&dist_highs, 12);

    // Convert Vec<u8> streams to Vec<u32> for rans_v4.
    let lit_u32: Vec<u32> = literals.iter().map(|&x| x as u32).collect();
    let len_u32: Vec<u32> = lengths.iter().map(|&x| x as u32).collect();
    let dist_lo_u32: Vec<u32> = dist_lows.iter().map(|&x| x as u32).collect();
    let dist_hi_u32: Vec<u32> = dist_highs.iter().map(|&x| x as u32).collect();

    let lit_stream = rans_encode(&lit_u32, &lit_table);
    let len_stream = rans_encode(&len_u32, &len_table);
    let dist_lo_stream = rans_encode(&dist_lo_u32, &dist_lo_table);
    let dist_hi_stream = rans_encode(&dist_hi_u32, &dist_hi_table);

    let lit_table_bytes = encode_table(&lit_table);
    // Dense encoding for all tables. v3 dense has ~260 bytes per table × 4
    // tables ≈ 1KB overhead per block — significant on small blocks
    // (e.g., code.rs blocks are 2-4KB compressed). Sparse encoding was
    // tried but it conflicts with rANS's "every symbol has freq≥1"
    // invariant: from_counts forces freq=1 for unused symbols, so no
    // symbol is truly "absent" in the table. We'd need a different
    // rANS variant to allow freq=0; that's a v4 task.
    let len_table_bytes = encode_table(&len_table);
    let dist_lo_table_bytes = encode_table(&dist_lo_table);
    let dist_hi_table_bytes = encode_table(&dist_hi_table);

    // 4. Op-flags stream (one byte per op: 0 = literal, 1 = match).
    let mut ops_bytes = Vec::with_capacity(ops.len() + 4);
    ops_bytes.extend_from_slice(&(ops.len() as u32).to_le_bytes());
    for op in &ops {
        match op {
            Op::Lit(_) => ops_bytes.push(0),
            Op::Match { .. } => ops_bytes.push(1),
            Op::DictRef { .. } => unreachable!(
                "DictRef ops not handled by codec yet (see lz77.rs::encode_with_dict)"
            ),
        }
    }

    // 5. Compressed payload (v3 format, no leading tag here — caller adds it).
    //    Layout:
    //      [u32 lit_table_len][u32 lit_stream_len]
    //      [u32 len_table_len][u32 len_stream_len]
    //      [u32 dist_lo_table_len][u32 dist_lo_stream_len]
    //      [u32 dist_hi_table_len][u32 dist_hi_stream_len]
    //      [u32 ops_len]
    //      [lit_table][lit_stream]
    //      [len_table][len_stream]
    //      [dist_lo_table][dist_lo_stream]
    //      [dist_hi_table][dist_hi_stream]
    //      [ops_bytes]
    let mut out = Vec::with_capacity(
        8 * 8 + lit_table_bytes.len()
            + lit_stream.len()
            + len_table_bytes.len()
            + len_stream.len()
            + dist_lo_table_bytes.len()
            + dist_lo_stream.len()
            + dist_hi_table_bytes.len()
            + dist_hi_stream.len()
            + ops_bytes.len(),
    );
    push_u32(&mut out, lit_table_bytes.len() as u32);
    push_u32(&mut out, lit_stream.len() as u32);
    push_u32(&mut out, len_table_bytes.len() as u32);
    push_u32(&mut out, len_stream.len() as u32);
    push_u32(&mut out, dist_lo_table_bytes.len() as u32);
    push_u32(&mut out, dist_lo_stream.len() as u32);
    push_u32(&mut out, dist_hi_table_bytes.len() as u32);
    push_u32(&mut out, dist_hi_stream.len() as u32);
    push_u32(&mut out, ops_bytes.len() as u32);
    out.extend_from_slice(&lit_table_bytes);
    out.extend_from_slice(&lit_stream);
    out.extend_from_slice(&len_table_bytes);
    out.extend_from_slice(&len_stream);
    out.extend_from_slice(&dist_lo_table_bytes);
    out.extend_from_slice(&dist_lo_stream);
    out.extend_from_slice(&dist_hi_table_bytes);
    out.extend_from_slice(&dist_hi_stream);
    out.extend_from_slice(&ops_bytes);

    if out.len() >= data.len() {
        None
    } else {
        Some(out)
    }
}

/// Build a rANS frequency table with custom precision. Lower precision
/// (e.g., 8 bits) gives smaller tables at the cost of slightly less
/// accurate probability estimates. For small-alphabet streams (match
/// fields in v3) 8 bits is plenty and saves ~150 bytes per table.
fn build_table_with_precision(data: &[u8], scale_bits: u32) -> FreqTable {
    let mut counts = [0u32; 256];
    for &b in data {
        counts[b as usize] += 1;
    }
    let mut table = FreqTable::from_counts(&counts);
    // Override the auto-picked scale_bits with the requested one (if it fits).
    // If the requested precision is too small for the total, fall back to
    // auto-picked. rANS needs scale_bits s.t. 2^scale_bits >= total.
    if (1u32 << scale_bits) >= table.total && scale_bits > 0 {
        table.scale_bits = scale_bits;
    }
    table
}

#[inline]
fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn read_u32(buf: &[u8], off: &mut usize) -> u32 {
    let v = u32::from_le_bytes(buf[*off..*off + 4].try_into().unwrap());
    *off += 4;
    v
}

pub fn decompress(input: &[u8]) -> Vec<u8> {
    let mut cursor = Cursor::new(input);
    let header = NexusHeader::read(&mut cursor).expect("invalid .nexus header");
    assert!(
        header.version == VERSION_V0
            || header.version == VERSION_V2
            || header.version == VERSION_V3
            || header.version == VERSION_V3,
        "unsupported .nexus version {} (expected {}, {}, or {})",
        header.version, VERSION_V0, VERSION_V2, VERSION_V3
    );
    let mut out = Vec::with_capacity(header.uncompressed_total_size as usize);

    // Cache of decoded blocks for Duplicate lookup
    let mut cache: Vec<Vec<u8>> = Vec::new();

    for block_idx in 0..header.block_count {
        let bh = BlockHeader::read(&mut cursor).expect("invalid block header");
        let mut payload = vec![0u8; bh.compressed_size as usize];
        cursor.read_exact(&mut payload).unwrap();

        let block = decode_block(&payload, bh.uncompressed_size as usize, bh.block_type, &mut cache, header.version);
        eprintln!("[decompress] block {} of {}: type={:?} uncompressed={} compressed={} decoded={}",
            block_idx, header.block_count, bh.block_type, bh.uncompressed_size, bh.compressed_size, block.len());
        out.extend_from_slice(&block);
    }
    out
}

fn decode_block(payload: &[u8], uncompressed_size: usize, block_type: BlockType, cache: &mut Vec<Vec<u8>>, _version: u8) -> Vec<u8> {
    if uncompressed_size == 0 {
        return vec![];
    }
    assert!(!payload.is_empty(), "empty payload for non-empty block");

    // Duplicate block: payload = [tag][u32 original_id]
    if payload[0] == TAG_DUPLICATE || block_type == BlockType::Duplicate {
        let original_id =
            u32::from_le_bytes(payload[1..5].try_into().unwrap()) as usize;
        let original = cache
            .get(original_id)
            .unwrap_or_else(|| panic!(
                "Duplicate references unknown block id {} (cache.len()={}, block_type={:?})",
                original_id, cache.len(), block_type
            ));
        return original.clone();
    }

    if payload[0] == TAG_RAW {
        let raw = payload[1..].to_vec();
        cache.push(raw.clone());
        return raw;
    }

    // v3: multi-stream rANS — split into lit / len / dist_lo / dist_hi
    if payload[0] == TAG_V3_MULTISTREAM {
        return decode_block_v3(payload, cache, false);
    }
    // v3 + RLE pre-filter: same as above, but RLE-decode the LZ77 output.
    if payload[0] == TAG_V3_MULTISTREAM_RLE {
        return decode_block_v3(payload, cache, true);
    }

    // v0 / v2: single rANS stream
    let is_v2 = payload[0] == TAG_RANS_LITERALS_V2;
    if !is_v2 {
        assert_eq!(
            payload[0], TAG_RANS_LITERALS_V0,
            "unsupported block tag {}",
            payload[0]
        );
    }

    // RANS-compressed block. Layout differs between v0 and v2:
    //   v0: [tag=2][u32 table_len][u32 rans_len][table][rans][ops]
    //   v2: [tag=4][u32 table_len][u32 rans_len][table][rans][ops]
    //      (same header layout, different op-stream encoding)
    let table_len = u32::from_le_bytes(payload[1..5].try_into().unwrap()) as usize;
    let rans_len = u32::from_le_bytes(payload[5..9].try_into().unwrap()) as usize;
    let table_bytes = &payload[9..9 + table_len];
    let rans_payload = &payload[9 + table_len..9 + table_len + rans_len];
    let ops_bytes = &payload[9 + table_len + rans_len..];

    let table = decode_table(table_bytes);

    let n_ops = u32::from_le_bytes(ops_bytes[0..4].try_into().unwrap()) as usize;
    let mut ops: Vec<Op> = Vec::with_capacity(n_ops);
    let mut p = 4usize;
    let mut n_literals = 0usize;
    for _ in 0..n_ops {
        let flag = ops_bytes[p];
        p += 1;
        match flag {
            0 => {
                // Literal: v0 has placeholder byte, v2 does not.
                if is_v2 {
                    ops.push(Op::Lit(0)); // placeholder, overwritten by rANS later
                } else {
                    ops.push(Op::Lit(ops_bytes[p]));
                    p += 1;
                }
                n_literals += 1;
            }
            1 => {
                let (dist, len) = if is_v2 {
                    // v2: u16 distance + u8 length
                    let d = u16::from_le_bytes(ops_bytes[p..p + 2].try_into().unwrap()) as u32;
                    p += 2;
                    let l = ops_bytes[p] as u32;
                    p += 1;
                    (d, l)
                } else {
                    // v0: u32 distance + u32 length
                    let d = u32::from_le_bytes(ops_bytes[p..p + 4].try_into().unwrap());
                    p += 4;
                    let l = u32::from_le_bytes(ops_bytes[p..p + 4].try_into().unwrap());
                    p += 4;
                    (d, l)
                };
                ops.push(Op::Match { dist, len });
            }
            _ => panic!("unknown op flag {}", flag),
        }
    }

    let decoded_lits = rans_decode(rans_payload, &table, n_literals);

    let mut lit_iter = decoded_lits.into_iter();
    for op in ops.iter_mut() {
        if let Op::Lit(slot) = op {
            *slot = lit_iter.next().expect("ran out of decoded literals") as u8;
        }
    }

    let mut md = MatchDecoder::new();
    let decoded = md.decode(&ops);

    // Add to cache for future Duplicate references
    cache.push(decoded.clone());

    decoded
}

/// v3 multi-stream decoder.
///
/// Reads 4 rANS streams (literals, lengths, dist_low, dist_high) and
/// reconstructs the op stream from the interleave flags. Each rANS
/// stream has its own frequency table.
///
/// `was_rle`: if true, the input was RLE-pre-filtered before LZ77, so
/// the LZ77 output must be RLE-decoded to recover the original block.
fn decode_block_v3(payload: &[u8], cache: &mut Vec<Vec<u8>>, was_rle: bool) -> Vec<u8> {
    let mut off = 1usize; // skip TAG_V3_MULTISTREAM or TAG_V3_MULTISTREAM_RLE

    // Read the 9 u32 sizes
    let lit_table_len = read_u32(payload, &mut off) as usize;
    let lit_stream_len = read_u32(payload, &mut off) as usize;
    let len_table_len = read_u32(payload, &mut off) as usize;
    let len_stream_len = read_u32(payload, &mut off) as usize;
    let dist_lo_table_len = read_u32(payload, &mut off) as usize;
    let dist_lo_stream_len = read_u32(payload, &mut off) as usize;
    let dist_hi_table_len = read_u32(payload, &mut off) as usize;
    let dist_hi_stream_len = read_u32(payload, &mut off) as usize;
    let ops_len = read_u32(payload, &mut off) as usize;

    // Slice each section
    let lit_table_bytes = &payload[off..off + lit_table_len];
    off += lit_table_len;
    let lit_stream = &payload[off..off + lit_stream_len];
    off += lit_stream_len;

    let len_table_bytes = &payload[off..off + len_table_len];
    off += len_table_len;
    let len_stream = &payload[off..off + len_stream_len];
    off += len_stream_len;

    let dist_lo_table_bytes = &payload[off..off + dist_lo_table_len];
    off += dist_lo_table_len;
    let dist_lo_stream = &payload[off..off + dist_lo_stream_len];
    off += dist_lo_stream_len;

    let dist_hi_table_bytes = &payload[off..off + dist_hi_table_len];
    off += dist_hi_table_len;
    let dist_hi_stream = &payload[off..off + dist_hi_stream_len];
    off += dist_hi_stream_len;

    let ops_bytes = &payload[off..off + ops_len];

    // Decode the rANS streams
    let lit_table = decode_table(lit_table_bytes);
    let len_table = decode_table(len_table_bytes);
    let dist_lo_table = decode_table(dist_lo_table_bytes);
    let dist_hi_table = decode_table(dist_hi_table_bytes);

    let n_ops = u32::from_le_bytes(ops_bytes[0..4].try_into().unwrap()) as usize;
    let flags: Vec<u8> = ops_bytes[4..4 + n_ops].to_vec();

    // Count how many literals vs matches so we know how much to decode from each
    let n_lits = flags.iter().filter(|&&f| f == 0).count();
    let n_matches = n_ops - n_lits;

    let lit_values = rans_decode(lit_stream, &lit_table, n_lits);
    let len_values = rans_decode(len_stream, &len_table, n_matches);
    let dist_lo_values = rans_decode(dist_lo_stream, &dist_lo_table, n_matches);
    let dist_hi_values = rans_decode(dist_hi_stream, &dist_hi_table, n_matches);

    // Reconstruct ops by interleaving per the flags
    let mut ops: Vec<Op> = Vec::with_capacity(n_ops);
    let mut lit_iter = lit_values.into_iter().map(|v| v as u8);
    let mut len_iter = len_values.into_iter();
    let mut dlo_iter = dist_lo_values.into_iter();
    let mut dhi_iter = dist_hi_values.into_iter();
    for &flag in &flags {
        match flag {
            0 => {
                ops.push(Op::Lit(lit_iter.next().expect("ran out of literals")));
            }
            1 => {
                let l = len_iter.next().expect("ran out of lengths") as u32;
                let lo = dlo_iter.next().expect("ran out of dist_lows") as u32;
                let hi = dhi_iter.next().expect("ran out of dist_highs") as u32;
                ops.push(Op::Match {
                    dist: lo | (hi << 8),
                    len: l,
                });
            }
            _ => panic!("unknown op flag {}", flag),
        }
    }

    let mut md = MatchDecoder::new();
    let mut decoded = md.decode(&ops);
    if was_rle {
        decoded = crate::rle::decompress_rle(&decoded);
    }
    cache.push(decoded.clone());
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small_text() {
        let data = b"The quick brown fox jumps over the lazy dog. \
                     The quick brown fox jumps over the lazy dog. \
                     The quick brown fox jumps over the lazy dog.";
        let c = compress(data);
        let d = decompress(&c);
        assert_eq!(d, data, "roundtrip mismatch");
    }

    #[test]
    fn roundtrip_empty() {
        let c = compress(b"");
        let d = decompress(&c);
        assert_eq!(d, b"");
    }

    #[test]
    fn roundtrip_random() {
        let mut v = Vec::with_capacity(8192);
        let mut x: u32 = 0xc0ffee;
        for _ in 0..8192 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            v.push(x as u8);
        }
        let c = compress(&v);
        let d = decompress(&c);
        assert_eq!(d, v);
    }

    #[test]
    fn roundtrip_repetitive() {
        // Force dedup to kick in
        let phrase = b"the quick brown fox jumps over the lazy dog ";
        let mut data = Vec::new();
        while data.len() < 1024 {
            data.extend_from_slice(phrase);
        }
        let c = compress(&data);
        let d = decompress(&c);
        assert_eq!(d, data, "repetitive roundtrip mismatch");
    }
}