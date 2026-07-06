//! v4.5 dictionary-aware codec — extends v3 with a 5th rANS stream for
//! `Op::DictRef` ids and 2-bit packed op-flags.
//!
//! ## Why a separate module
//!
//! The v3 codec is a self-contained, well-tested piece of code. The v4.5
//! extension has a different op-flag encoding (2 bits instead of 1 byte)
//! and adds a new rANS stream, so mixing the two would clutter the
//! primary codec. Keeping them in separate modules means the v3 path
//! stays untouched, the v4.5 path is independently testable, and the
//! dispatcher in `codec.rs` can pick the right one per block.
//!
//! ## Format (TAG_V3_MULTISTREAM_DICT = 8 payload)
//!
//! ```text
//! [tag=8]                                 (handled by caller)
//! [u8 dict_select]                        (2-bit field, lower bits used)
//!   0 = No dict (lit + match only)
//!   1 = Trained (corpus/trained.dict)
//!   2 = Code   (default_code_dict)
//!   3 = JSON   (default_json_dict)
//! [u32 lit_table_len] [u32 lit_stream_len]
//! [u32 len_table_len] [u32 len_stream_len]
//! [u32 dist_lo_table_len] [u32 dist_lo_stream_len]
//! [u32 dist_hi_table_len] [u32 dist_hi_stream_len]
//! [u32 dict_table_len]  [u32 dict_stream_len]
//! [u32 op_flags_count]
//! [lit_table][lit_stream]
//! [len_table][len_stream]
//! [dist_lo_table][dist_lo_stream]
//! [dist_hi_table][dist_hi_stream]
//! [dict_table][dict_stream]
//! [op_flags_bytes: 2 bits per op, packed 4 per byte]
//! ```
//!
//! ## Op-flags (2 bits)
//!
//!   00 = Lit    (consume 1 byte from lit stream)
//!   01 = Match  (consume 1 byte each from len, dist_lo, dist_hi streams)
//!   10 = DictRef(consume 1 byte from dict stream, resolve via sub-dict)
//!   11 = reserved (decoder panics on this — invalid)
//!
//! Packing: 4 ops per byte, high bits first. For `n` ops, the byte
//! count is `(n + 3) / 4`. The last byte may have 1-3 unused trailing
//! bits which the decoder must NOT read.
//!
//! ## Sub-dict selection
//!
//! Per-block heuristic on the first 500 bytes:
//!   - entropy > 7.5   → 0 (None, high-entropy noise)
//!   - struct_ratio > 0.10 AND printable > 0.85 → 3 (JSON)
//!   - code-keyword hits >= 2 → 2 (Code)
//!   - else → 1 (Trained)
//!
//! The 500-byte scan is O(N) and adds ~microseconds per block. The
//! heuristic is intentionally simple — its job is to pick the right
//! sub-dict for the block content, not to be clever. Empirically
//! the four categories cover >95% of our corpus.

use crate::dictionary::Dictionary;
use crate::lz77::{MatchDecoder, MatchFinder, Op};
use crate::rans_v4::{decode_table, encode_table, rans_decode, rans_encode, FreqTable};

/// Pre-scan sample size (bytes). Short blocks scan the whole block.
const PRESCAN_BYTES: usize = 500;

/// Maximum dict entries the codec can encode. The rANS alphabet for
/// the dict-ids stream is 0..=MAX_DICT_ENTRIES-1, so dict_ids fit in
/// one u8 per reference. Larger dicts (the trained.dict has 5348
/// entries) are clipped to their first MAX_DICT_ENTRIES entries,
/// which are the highest-score (most common) ones.
pub const MAX_DICT_ENTRIES: usize = 256;

/// Baked-in trained dictionary (from sprint 2.5, trained on
/// `corpus/{code,text,data.json}`). 60KB on disk, ~5348 entries.
///
/// The bytes are embedded at compile time via `include_bytes!` so the
/// binary is self-contained — no need to ship a separate .dict file.
/// To update the dict, re-run `cargo run --release --bin dict_train -- -o
/// corpus/trained.dict corpus/code.rs corpus/text.txt corpus/data.json`
/// and rebuild.
const TRAINED_DICT_BYTES: &[u8] = include_bytes!("../corpus/trained.dict");

/// Get the trained dictionary, parsing it on first access. Cached.
/// Returns the FULL 5348-entry dict (for inspection / training).
pub fn trained_dict() -> Dictionary {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Dictionary> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            Dictionary::from_bytes(TRAINED_DICT_BYTES)
                .expect("corpus/trained.dict is corrupt or wrong version")
        })
        .clone()
}

/// Get the COMPACT trained dictionary (first MAX_DICT_ENTRIES entries).
/// The compact dict is what the codec actually uses for LZ77 matching
/// and decoding. The full dict is preserved for inspection.
///
/// The first MAX_DICT_ENTRIES entries of the trained dict are the
/// highest-score ones (insertion order = score-descending), so the
/// 256-entry compact version captures the most-frequent tokens.
pub fn trained_dict_compact() -> Dictionary {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Dictionary> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let full = trained_dict();
            let mut compact = Dictionary::new();
            for (i, (_, token)) in full.iter().enumerate() {
                if i >= MAX_DICT_ENTRIES {
                    break;
                }
                compact.insert(token, 1);
            }
            compact
        })
        .clone()
}

/// 2-bit op-flag values.
pub mod flag {
    pub const LIT: u8 = 0b00;
    pub const MATCH: u8 = 0b01;
    pub const DICTREF: u8 = 0b10;
    pub const RESERVED: u8 = 0b11; // must never appear in valid bitstreams
}

/// Dict-select values.
pub mod select {
    pub const NONE: u8 = 0;
    pub const TRAINED: u8 = 1;
    pub const CODE: u8 = 2;
    pub const JSON: u8 = 3;
    pub const _TEXT: u8 = 4; // reserved; trained.dict covers text too
}

/// Heuristic: pick the best sub-dict for a block based on its first
/// 500 bytes. Returns one of the `select::*` constants.
pub fn dict_select_for_block(block: &[u8]) -> u8 {
    if block.is_empty() {
        return select::NONE;
    }
    let sample = &block[..block.len().min(PRESCAN_BYTES)];

    let mut counts = [0u32; 256];
    let mut struct_chars = 0u32;
    let mut printable = 0u32;
    for &b in sample {
        counts[b as usize] += 1;
        if matches!(b, b'{' | b'}' | b':' | b',' | b'[' | b']' | b'"') {
            struct_chars += 1;
        }
        if (0x20..0x7f).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t' {
            printable += 1;
        }
    }
    let n = sample.len() as f32;
    let struct_ratio = struct_chars as f32 / n;
    let printable_ratio = printable as f32 / n;
    let entropy = shannon_entropy(&counts, sample.len() as u32);

    // 1. High-entropy random data → no dict (would only add noise).
    if entropy > 7.5 {
        return select::NONE;
    }

    // 2. Mixed-content blocks (some ASCII, some binary) — printable
    // ratio < 0.6 means too much non-ASCII noise for a static dict
    // to help. The trained dict would add matches but with
    // low signal-to-noise.
    if printable_ratio < 0.6 {
        return select::NONE;
    }

    // 3. Highly repetitive content (a few 4-byte patterns repeated
    // many times): dedup handles these. v4.5 would just waste work
    // on the first block. Examples: "abcdefgh" repeated, all-zero,
    // all-space, etc. Threshold: < 20 unique 4-byte windows in 500
    // bytes (real text/code/JSON have hundreds).
    if sample.len() >= 4 {
        let mut unique_4byte = std::collections::HashSet::with_capacity(64);
        for w in sample.windows(4) {
            let key = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
            unique_4byte.insert(key);
        }
        if unique_4byte.len() < 20 {
            return select::NONE;
        }
    }

    // 4. JSON-like: lots of {", ,", :", [", "].
    if struct_ratio > 0.10 && printable_ratio > 0.85 {
        return select::JSON;
    }

    // 5. Source code: presence of Rust-style keywords.
    let code_keywords: &[&[u8]] = &[
        b"fn ", b"let ", b"mut ", b"impl ", b"pub ", b"struct ", b"->", b"=>", b"::",
    ];
    let mut code_hits = 0;
    for kw in code_keywords {
        if sample.windows(kw.len()).any(|w| w == *kw) {
            code_hits += 1;
        }
    }
    if code_hits >= 2 {
        return select::CODE;
    }

    // 6. Default: trained dictionary (covers text + mixed content).
    select::TRAINED
}

fn shannon_entropy(counts: &[u32; 256], total: u32) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let mut h = 0f32;
    for &c in counts.iter() {
        if c == 0 {
            continue;
        }
        let p = c as f32 / total as f32;
        h -= p * p.log2();
    }
    h
}

/// Load the sub-dict for a given `select` value. Returns `None` for
/// `select::NONE` (no dict refs in this block).
pub fn load_sub_dict(select: u8) -> Option<Dictionary> {
    match select {
        select::NONE => None,
        select::TRAINED => Some(trained_dict_compact()),
        select::CODE => Some(crate::dictionary::default_code_dict()),
        select::JSON => Some(crate::dictionary::default_json_dict()),
        _ => None,
    }
}

/// Quick check used by the codec to decide whether to attempt the v4.5
/// dict-aware path on a block. Returns `true` iff the pre-scan selects
/// a non-NONE dictionary. Cheap (~microseconds per call) — runs the
/// same heuristic as `dict_select_for_block` but short-circuits on
/// `select::NONE`.
pub fn should_try_v45(block: &[u8]) -> bool {
    dict_select_for_block(block) != select::NONE
}

// ---------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------

/// Build the v4.5 dict multistream payload (without the leading tag
/// byte — caller adds it). The payload includes the `dict_select`
/// field as the first byte after the (caller's) tag.
///
/// Returns `None` if the encoded form is not smaller than the input,
/// in which case the caller should fall back to the standard v3 path
/// or the raw block.
pub fn encode_v45_multistream(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }

    // 1. Pick the sub-dict for this block.
    let dict_select = dict_select_for_block(data);
    let sub_dict = match load_sub_dict(dict_select) {
        Some(d) => d,
        None => return None, // No dict: caller falls back to v3.
    };

    // 2. LZ77 with dict integration.
    let mut mf = MatchFinder::new();
    let ops = mf.encode_with_dict(data, &sub_dict);

    // 3. Split ops into FIVE independent streams (4 + dict).
    let mut literals: Vec<u8> = Vec::new();
    let mut lengths: Vec<u8> = Vec::new();
    let mut dist_lows: Vec<u8> = Vec::new();
    let mut dist_highs: Vec<u8> = Vec::new();
    let mut dict_ids: Vec<u32> = Vec::new(); // u16 ids stored as u32 for rANS
    for op in &ops {
        match op {
            Op::Lit(b) => literals.push(*b),
            Op::Match { dist, len } => {
                lengths.push(*len as u8);
                dist_lows.push((*dist & 0xFF) as u8);
                dist_highs.push(((*dist >> 8) & 0xFF) as u8);
            }
            Op::DictRef { id, len: _ } => {
                // The `len` field is determined by the dict token's
                // length (looked up at decode time). The encoder
                // doesn't need to store it — the LZ77 matchfinder
                // already advanced past those bytes.
                dict_ids.push(*id as u32);
            }
        }
    }

    // 4. Build frequency tables. v4.7: the 4 base streams now use
    //    SPARSE encoding (32-byte bitmask + densified rANS table) —
    //    same principle as the dict-ids stream (v4.6) generalized
    //    to all u8 streams. This drops the per-table overhead from
    //    ~1027 bytes to ~50-200 bytes per stream.
    let (lit_table_section, lit_stream) = crate::rans_v4::sparse_rans_encode_u8(&literals, 12);
    let (len_table_section, len_stream) = crate::rans_v4::sparse_rans_encode_u8(&lengths, 12);
    let (dist_lo_table_section, dist_lo_stream) = crate::rans_v4::sparse_rans_encode_u8(&dist_lows, 12);
    let (dist_hi_table_section, dist_hi_stream) = crate::rans_v4::sparse_rans_encode_u8(&dist_highs, 12);
    // Sparse dict-ids table (v4.6): the alphabet is collapsed from
    // 256 to just the set bits in the bitmask. For most blocks this
    // shrinks the table from ~1027 bytes to ~50-200 bytes.
    let (dict_bitmask, _dict_remap, dict_dense_ids) = sparse_dict_encode(&dict_ids);
    let dict_table = build_sparse_dict_table(&dict_dense_ids, _dict_remap.len());
    let dict_stream = rans_encode(&dict_dense_ids, &dict_table);

    // 6. Table bytes. Each base-stream section is now:
    //   [32 bytes bitmask][densified rANS table]
    // The dict section is the same shape: [bitmask][densified rANS table].
    let mut dict_table_bytes = Vec::with_capacity(DICT_BITMASK_BYTES + 64);
    dict_table_bytes.extend_from_slice(&dict_bitmask);
    dict_table_bytes.extend_from_slice(&encode_table(&dict_table));

    // 7. Op-flags (2 bits per op, packed 4 per byte).
    let op_flags_bytes = pack_op_flags(&ops);

    // 8. Assemble payload.
    //    Layout:
    //      [u8 dict_select]
    //      [u32 n_ops]                       (needed to count rANS
    //                                          symbols per stream)
    //      [u32 lit_table_len] [u32 lit_stream_len]
    //      [u32 len_table_len] [u32 len_stream_len]
    //      [u32 dist_lo_table_len] [u32 dist_lo_stream_len]
    //      [u32 dist_hi_table_len] [u32 dist_hi_stream_len]
    //      [u32 dict_table_len]  [u32 dict_stream_len]
    //      [u32 op_flags_len]
    //      [tables][streams][op_flags]
    let mut out = Vec::with_capacity(
        1 + 11 * 4
            + lit_table_section.len()
            + lit_stream.len()
            + len_table_section.len()
            + len_stream.len()
            + dist_lo_table_section.len()
            + dist_lo_stream.len()
            + dist_hi_table_section.len()
            + dist_hi_stream.len()
            + dict_table_bytes.len()
            + dict_stream.len()
            + op_flags_bytes.len(),
    );
    out.push(dict_select);
    push_u32(&mut out, ops.len() as u32); // n_ops
    push_u32(&mut out, lit_table_section.len() as u32);
    push_u32(&mut out, lit_stream.len() as u32);
    push_u32(&mut out, len_table_section.len() as u32);
    push_u32(&mut out, len_stream.len() as u32);
    push_u32(&mut out, dist_lo_table_section.len() as u32);
    push_u32(&mut out, dist_lo_stream.len() as u32);
    push_u32(&mut out, dist_hi_table_section.len() as u32);
    push_u32(&mut out, dist_hi_stream.len() as u32);
    push_u32(&mut out, dict_table_bytes.len() as u32);
    push_u32(&mut out, dict_stream.len() as u32);
    push_u32(&mut out, op_flags_bytes.len() as u32);
    out.extend_from_slice(&lit_table_section);
    out.extend_from_slice(&lit_stream);
    out.extend_from_slice(&len_table_section);
    out.extend_from_slice(&len_stream);
    out.extend_from_slice(&dist_lo_table_section);
    out.extend_from_slice(&dist_lo_stream);
    out.extend_from_slice(&dist_hi_table_section);
    out.extend_from_slice(&dist_hi_stream);
    out.extend_from_slice(&dict_table_bytes);
    out.extend_from_slice(&dict_stream);
    out.extend_from_slice(&op_flags_bytes);

    if out.len() >= data.len() {
        None
    } else {
        Some(out)
    }
}

// ---------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------

/// Decode a TAG_V3_MULTISTREAM_DICT payload. Caller has already verified
/// the tag byte. The dict used here MUST be the same one the encoder
/// used (selected by `dict_select` in the payload).
pub fn decode_v45_multistream(payload: &[u8]) -> Vec<u8> {
    let mut off = 0usize;
    let dict_select = payload[off];
    off += 1;
    let sub_dict = load_sub_dict(dict_select).expect("dict_select refers to unknown dict");

    let n_ops = read_u32(payload, &mut off) as usize;
    // Read 10 u32 sizes.
    let lit_table_len = read_u32(payload, &mut off) as usize;
    let lit_stream_len = read_u32(payload, &mut off) as usize;
    let len_table_len = read_u32(payload, &mut off) as usize;
    let len_stream_len = read_u32(payload, &mut off) as usize;
    let dist_lo_table_len = read_u32(payload, &mut off) as usize;
    let dist_lo_stream_len = read_u32(payload, &mut off) as usize;
    let dist_hi_table_len = read_u32(payload, &mut off) as usize;
    let dist_hi_stream_len = read_u32(payload, &mut off) as usize;
    let dict_table_len = read_u32(payload, &mut off) as usize;
    let dict_stream_len = read_u32(payload, &mut off) as usize;
    let op_flags_len = read_u32(payload, &mut off) as usize;

    // Slice the sections.
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
    let dict_table_bytes = &payload[off..off + dict_table_len];
    off += dict_table_len;
    let dict_stream = &payload[off..off + dict_stream_len];
    off += dict_stream_len;

    // Decode the dict table (sparse v4.6 format):
    //   [32 bytes bitmask][rANS table for the dense alphabet]
    if dict_table_bytes.len() < DICT_BITMASK_BYTES {
        panic!(
            "dict table section too short: {} bytes (need at least {} for bitmask)",
            dict_table_bytes.len(),
            DICT_BITMASK_BYTES
        );
    }
    let mut dict_bitmask = [0u8; DICT_BITMASK_BYTES];
    dict_bitmask.copy_from_slice(&dict_table_bytes[..DICT_BITMASK_BYTES]);
    let dict_table_inner_bytes = &dict_table_bytes[DICT_BITMASK_BYTES..];
    let dict_table = decode_table(dict_table_inner_bytes);
    let dict_remap = sparse_dict_decode_remap(&dict_bitmask);
    let op_flags_bytes = &payload[off..off + op_flags_len];

    // Decode the rANS streams. v4.7: the 4 base streams use the
    // sparse format (32-byte bitmask + densified rANS table). The
    // shared `sparse_rans_decode_u8` helper handles all four.
    // (dict table already decoded above with the bitmask.)

    // Count how many of each op type there are by scanning op_flags.
    // We use n_ops to avoid reading phantom bits in the last byte.
    let (n_lits, n_matches, n_dict_refs) = count_ops(op_flags_bytes, n_ops);

    // Decode the 4 base streams via sparse_rans_decode_u8.
    let lit_values = crate::rans_v4::sparse_rans_decode_u8(lit_table_bytes, lit_stream, n_lits);
    let len_values = crate::rans_v4::sparse_rans_decode_u8(len_table_bytes, len_stream, n_matches);
    let dist_lo_values = crate::rans_v4::sparse_rans_decode_u8(dist_lo_table_bytes, dist_lo_stream, n_matches);
    let dist_hi_values = crate::rans_v4::sparse_rans_decode_u8(dist_hi_table_bytes, dist_hi_stream, n_matches);
    // The dict stream encodes dense indices (0..N-1). We need to
    // remap each dense index to the original dict id via the bitmask.
    let dict_dense_values = rans_decode(dict_stream, &dict_table, n_dict_refs);
    let dict_values: Vec<u32> = dict_dense_values
        .iter()
        .map(|&d| {
            if (d as usize) < dict_remap.len() {
                dict_remap[d as usize] as u32
            } else {
                panic!("dense dict idx {} out of range (remap size {})", d, dict_remap.len())
            }
        })
        .collect();

    // Reconstruct ops by interleaving per the flags.
    let mut ops: Vec<Op> = Vec::with_capacity(n_ops);
    let mut lit_iter = lit_values.into_iter().map(|v| v as u8);
    let mut len_iter = len_values.into_iter();
    let mut dlo_iter = dist_lo_values.into_iter();
    let mut dhi_iter = dist_hi_values.into_iter();
    let mut dict_iter = dict_values.into_iter();

    for flag_byte in iter_op_flags(op_flags_bytes).take(n_ops) {
        match flag_byte {
            flag::LIT => {
                ops.push(Op::Lit(lit_iter.next().expect("ran out of literals")));
            }
            flag::MATCH => {
                let l = len_iter.next().expect("ran out of lengths") as u32;
                let lo = dlo_iter.next().expect("ran out of dist_lows") as u32;
                let hi = dhi_iter.next().expect("ran out of dist_highs") as u32;
                ops.push(Op::Match {
                    dist: lo | (hi << 8),
                    len: l,
                });
            }
            flag::DICTREF => {
                let id = dict_iter.next().expect("ran out of dict ids") as u16;
                let token = sub_dict
                    .get(id)
                    .unwrap_or_else(|| panic!("dict id {} not in dict", id));
                // The decoder materializes the full token. We push
                // one Op::DictRef with len=token.len() so MatchDecoder
                // emits the right number of bytes. (len is implicit
                // in the dict token, but the Op struct requires it
                // — we use the token length.)
                ops.push(Op::DictRef {
                    id,
                    len: token.len() as u8,
                });
            }
            flag::RESERVED => panic!("reserved op flag 0b11 in bitstream"),
            other => panic!("unknown op flag 0b{:02b}", other),
        }
    }

    let mut dec = MatchDecoder::with_dict(sub_dict);
    dec.decode(&ops)
}

// ---------------------------------------------------------------
// Op-flags packing / unpacking
// ---------------------------------------------------------------

/// Pack ops into 2-bit flags, 4 ops per byte (high bits first).
pub fn pack_op_flags(ops: &[Op]) -> Vec<u8> {
    let byte_count = (ops.len() + 3) / 4;
    let mut bytes = vec![0u8; byte_count];
    for (i, op) in ops.iter().enumerate() {
        let byte_idx = i / 4;
        let bit_off = 6 - 2 * (i % 4);
        let f = match op {
            Op::Lit(_) => flag::LIT,
            Op::Match { .. } => flag::MATCH,
            Op::DictRef { .. } => flag::DICTREF,
        };
        bytes[byte_idx] |= f << bit_off;
    }
    bytes
}

/// Iterator over the op-flag values, unpacking 2-bit fields from a
/// packed byte stream.
pub fn iter_op_flags(bytes: &[u8]) -> impl Iterator<Item = u8> + '_ {
    bytes
        .iter()
        .enumerate()
        .flat_map(|(i, b)| {
            let n_valid = if i == bytes.len() - 1 {
                // Last byte: 1-4 valid ops (we don't know exactly how
                // many; the caller has the op count separately).
                4
            } else {
                4
            };
            (0..n_valid).map(move |k| (b >> (6 - 2 * k)) & 0b11)
        })
}

/// Count the number of ops of each type in a packed op_flags stream,
/// up to `n_ops` total (the rest of the bits in the last byte are
/// don't-care padding).
pub fn count_ops(bytes: &[u8], n_ops: usize) -> (usize, usize, usize) {
    let mut n_lit = 0;
    let mut n_match = 0;
    let mut n_dict = 0;
    for f in iter_op_flags(bytes).take(n_ops) {
        match f {
            flag::LIT => n_lit += 1,
            flag::MATCH => n_match += 1,
            flag::DICTREF => n_dict += 1,
            _ => {}
        }
    }
    (n_lit, n_match, n_dict)
}

// ---------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------

fn build_table_with_precision(data: &[u8], scale_bits: u32) -> FreqTable {
    let mut counts = [0u32; 256];
    for &b in data {
        counts[b as usize] += 1;
    }
    let mut table = FreqTable::from_counts(&counts);
    if (1u32 << scale_bits) >= table.total && scale_bits > 0 {
        table.scale_bits = scale_bits;
    }
    table
}

/// Build a rANS table for a dict-ids stream. The alphabet is 0..=255
/// (MAX_DICT_ENTRIES). Used for the 5th stream.
fn build_table_with_precision_dict(data: &[u32]) -> FreqTable {
    let mut counts = [0u32; 256];
    for &v in data {
        if (v as usize) < 256 {
            counts[v as usize] += 1;
        }
    }
    let mut table = FreqTable::from_counts(&counts);
    if (1u32 << 12) >= table.total && 12 > 0 {
        table.scale_bits = 12;
    }
    table
}

// ---------------------------------------------------------------------
// Sparse table encoding for the 5th (dict-id) stream (v4.6).
//
// The dense 256-entry dict table costs ~1027 bytes regardless of how
// many ids are actually used in a block. For most blocks only ~10-50
// ids are referenced; the rest are "ghost" entries that rANS forces
// to freq≥1 to maintain the alphabet invariant. Sparse encoding
// collapses the alphabet to the actually-used subset:
//
//   - 32-byte bitmask (256 bits) marks which ids are present in the
//     block. Bit i = 1 means id i appears at least once.
//   - The rANS table is built only for the set bits, in id order.
//     Dense index 0 = smallest set id, dense index 1 = next, etc.
//   - The rANS stream encodes dense indices, not original ids.
//   - The decoder reads the bitmask, rebuilds the remap table, and
//     translates dense indices back to original ids after decoding.
//
// Overhead for a block with K distinct dict ids:
//   bitmask  : 32 bytes (constant)
//   table    : 7 + 4*(K+1) bytes
//   stream   : K*ceil(log2(K)) bits ≈ 1 byte/symbol minimum
// vs. dense  : 7 + 4*257 = 1035 bytes (table) + 0 (empty stream if 0 refs)
// So for K=10: ~80 bytes vs. 1035 bytes — saves ~950 bytes per block.
// For K=0 (no dict refs): 32 + 0 + 0 = 32 bytes vs. 1035 bytes.
// ---------------------------------------------------------------------

/// Bitmask size: 256 bits = 32 bytes. One bit per dict id.
pub const DICT_BITMASK_BYTES: usize = 32;

/// Build the sparse representation of a dict_ids stream.
///
/// Returns:
///   - `bitmask`: 32-byte presence bitmask (bit i = id i present)
///   - `remap`: dense_idx → original_id, in id-ascending order
///   - `dense_ids`: the input remapped through the dense alphabet
pub fn sparse_dict_encode(ids: &[u32]) -> ([u8; DICT_BITMASK_BYTES], Vec<u16>, Vec<u32>) {
    let mut bitmask = [0u8; DICT_BITMASK_BYTES];
    for &id in ids {
        let id_u16 = id as u16;
        if (id_u16 as usize) < 256 {
            let byte = (id_u16 / 8) as usize;
            let bit = id_u16 % 8;
            bitmask[byte] |= 1 << bit;
        }
    }
    // Build remap: for each set bit, dense_idx = count of set bits up to here.
    let mut remap: Vec<u16> = Vec::new();
    for id in 0..256u16 {
        let byte = (id / 8) as usize;
        let bit = id % 8;
        if bitmask[byte] & (1 << bit) != 0 {
            remap.push(id);
        }
    }
    // Build reverse remap (id -> dense_idx) for fast lookup.
    let mut reverse = [0u16; 256];
    for (dense_idx, &orig) in remap.iter().enumerate() {
        reverse[orig as usize] = dense_idx as u16;
    }
    let dense_ids: Vec<u32> = ids
        .iter()
        .map(|&id| reverse[id as usize] as u32)
        .collect();
    (bitmask, remap, dense_ids)
}

/// Inverse of `sparse_dict_encode`: given a bitmask, build the
/// dense_idx → original_id remap.
pub fn sparse_dict_decode_remap(bitmask: &[u8; DICT_BITMASK_BYTES]) -> Vec<u16> {
    let mut remap = Vec::new();
    for id in 0..256u16 {
        let byte = (id / 8) as usize;
        let bit = id % 8;
        if bitmask[byte] & (1 << bit) != 0 {
            remap.push(id);
        }
    }
    remap
}

/// Build a rANS table for the dense (sparse-mapped) dict-id stream.
/// The alphabet is 0..N-1 where N = number of set bits in the bitmask.
fn build_sparse_dict_table(dense_ids: &[u32], n_symbols: usize) -> FreqTable {
    let mut counts = vec![0u32; n_symbols];
    for &v in dense_ids {
        if (v as usize) < n_symbols {
            counts[v as usize] += 1;
        }
    }
    let mut table = FreqTable::from_counts(&counts);
    if (1u32 << 12) >= table.total && 12 > 0 {
        table.scale_bits = 12;
    }
    table
}

fn as_u32(data: &[u8]) -> Vec<u32> {
    data.iter().map(|&b| b as u32).collect()
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

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prescan_detects_json() {
        let data = b"{\"key\": \"value\", \"arr\": [1, 2, 3], \"nested\": {\"a\": \"b\"}}";
        let s = dict_select_for_block(data);
        assert_eq!(s, select::JSON);
    }

    #[test]
    fn prescan_detects_code() {
        let data = b"fn main() {\n    let x = 1;\n    let y = 2;\n    pub struct Foo {}\n}\n";
        let s = dict_select_for_block(data);
        assert_eq!(s, select::CODE);
    }

    #[test]
    fn prescan_detects_random() {
        // Pseudo-random data with high entropy.
        let mut data = Vec::with_capacity(2000);
        let mut s: u32 = 0xdeadbeef;
        for _ in 0..2000 {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            data.push((s >> 16) as u8);
        }
        let sel = dict_select_for_block(&data);
        assert_eq!(sel, select::NONE, "random data should be select::NONE");
    }

    #[test]
    fn prescan_defaults_to_trained() {
        // Plain English text without strong code or JSON markers.
        let data = b"the quick brown fox jumps over the lazy dog and then it ran away into the dark night never to be seen again by anyone";
        let sel = dict_select_for_block(data);
        assert_eq!(sel, select::TRAINED);
    }

    #[test]
    fn pack_unpack_roundtrip() {
        let ops = vec![
            Op::Lit(b'a'),
            Op::Match { dist: 5, len: 3 },
            Op::DictRef { id: 42, len: 4 },
            Op::Lit(b'b'),
            Op::Match { dist: 1, len: 2 },
        ];
        let packed = pack_op_flags(&ops);
        let unpacked: Vec<u8> = iter_op_flags(&packed).take(ops.len()).collect();
        let expected: Vec<u8> = ops
            .iter()
            .map(|op| match op {
                Op::Lit(_) => flag::LIT,
                Op::Match { .. } => flag::MATCH,
                Op::DictRef { .. } => flag::DICTREF,
            })
            .collect();
        assert_eq!(unpacked, expected);
    }

    #[test]
    fn pack_unpack_with_unaligned_count() {
        // 1, 2, 3, 5, 6, 7 ops (not multiples of 4) — make sure the
        // last byte's extra bits don't break anything.
        for n in &[1, 2, 3, 5, 6, 7, 9, 10, 13] {
            let ops: Vec<Op> = (0..*n).map(|i| {
                match i % 3 {
                    0 => Op::Lit(i as u8),
                    1 => Op::Match { dist: i as u32, len: 3 },
                    _ => Op::DictRef { id: i as u16, len: 4 },
                }
            }).collect();
            let packed = pack_op_flags(&ops);
            let unpacked: Vec<u8> = iter_op_flags(&packed).take(*n).collect();
            let expected: Vec<u8> = ops.iter().map(|op| match op {
                Op::Lit(_) => flag::LIT,
                Op::Match { .. } => flag::MATCH,
                Op::DictRef { .. } => flag::DICTREF,
            }).collect();
            assert_eq!(unpacked, expected, "roundtrip failed for n={}", n);
        }
    }

    #[test]
    fn count_ops_basic() {
        let ops = vec![
            Op::Lit(b'x'),
            Op::Match { dist: 1, len: 3 },
            Op::DictRef { id: 7, len: 2 },
            Op::Lit(b'y'),
            Op::Match { dist: 2, len: 4 },
        ];
        let packed = pack_op_flags(&ops);
        let (l, m, d) = count_ops(&packed, ops.len());
        assert_eq!(l, 2);
        assert_eq!(m, 2);
        assert_eq!(d, 1);
    }

    #[test]
    fn roundtrip_with_dict_in_bitstream() {
        // Use the full v4.5 encode/decode path. Data must be large
        // enough that the 5-stream overhead (5 tables ≈ 5KB) pays
        // for itself.
        let phrase = b"the cat sat on the mat ";
        let mut data = Vec::new();
        for _ in 0..2000 {
            data.extend_from_slice(phrase);
        }
        let payload = encode_v45_multistream(&data).expect("v4.5 encode should succeed on large data");
        let decoded = decode_v45_multistream(&payload);
        assert_eq!(decoded, data, "v4.5 roundtrip failed");
    }

    #[test]
    fn roundtrip_json_pattern() {
        // Build a JSON-like payload large enough for v4.5 to be
        // smaller than the raw input.
        let mut data = Vec::new();
        for i in 0..2000 {
            data.extend_from_slice(
                format!("{{\"id\": {}, \"name\": \"user_{}\", \"tags\": [\"admin\", \"user\"]}},\n", i, i)
                    .as_bytes(),
            );
        }
        let payload = encode_v45_multistream(&data).expect("v4.5 encode should succeed on JSON");
        let decoded = decode_v45_multistream(&payload);
        assert_eq!(decoded, data);
    }

    #[test]
    fn roundtrip_code_pattern() {
        // Larger Rust snippet so v4.5's 5-stream overhead pays off.
        let mut data = Vec::new();
        for i in 0..2000 {
            data.extend_from_slice(
                format!("    let x_{} = {};\n", i, i).as_bytes(),
            );
        }
        data.extend_from_slice(b"fn main() {\n");
        data.extend_from_slice(format!("    let total = {};\n", 2000).as_bytes());
        let payload = encode_v45_multistream(&data).expect("v4.5 encode should succeed on code");
        let decoded = decode_v45_multistream(&payload);
        assert_eq!(decoded, data);
    }

    #[test]
    fn roundtrip_random_falls_back_to_none() {
        // Pseudo-random data — should select dict::NONE, so the
        // encoder returns None (caller falls back to v3).
        let mut data = Vec::with_capacity(2000);
        let mut s: u32 = 0xc0ffee;
        for _ in 0..2000 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            data.push(s as u8);
        }
        let result = encode_v45_multistream(&data);
        assert!(result.is_none(), "random data should fall back to v3 (None)");
    }

    // ---------------------------------------------------------------
    // Sparse table encoding (v4.6)
    // ---------------------------------------------------------------

    #[test]
    fn sparse_bitmask_basic() {
        let ids = vec![0u32, 1, 5, 100, 255];
        let (bitmask, remap, dense_ids) = sparse_dict_encode(&ids);
        // Verify bitmask: bit 0, 1, 5, 100, 255 set
        assert_eq!(bitmask[0] & 0b00000011, 0b00000011); // bits 0, 1
        assert_eq!(bitmask[0] & 0b00100000, 0b00100000); // bit 5
        assert_eq!(bitmask[100 / 8] & (1 << (100 % 8)), 1 << (100 % 8));
        assert_eq!(bitmask[255 / 8] & (1 << (255 % 8)), 1 << (255 % 8));
        // Remap in id-ascending order
        assert_eq!(remap, vec![0, 1, 5, 100, 255]);
        // Dense ids map 0->0, 1->1, 5->2, 100->3, 255->4
        assert_eq!(dense_ids, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn sparse_bitmask_empty() {
        let ids: Vec<u32> = vec![];
        let (bitmask, remap, dense_ids) = sparse_dict_encode(&ids);
        assert_eq!(bitmask, [0u8; 32]);
        assert_eq!(remap, Vec::<u16>::new());
        assert_eq!(dense_ids, Vec::<u32>::new());
    }

    #[test]
    fn sparse_bitmask_all_set() {
        let ids: Vec<u32> = (0..256u32).collect();
        let (bitmask, remap, dense_ids) = sparse_dict_encode(&ids);
        for byte in 0..32 {
            assert_eq!(bitmask[byte], 0xFF);
        }
        assert_eq!(remap.len(), 256);
        // All 256 ids present → dense == original
        assert_eq!(dense_ids, ids);
    }

    #[test]
    fn sparse_decode_remap_inverts_encode() {
        let ids = vec![3u32, 7, 11, 200];
        let (bitmask, remap_encoded, _) = sparse_dict_encode(&ids);
        let remap_decoded = sparse_dict_decode_remap(&bitmask);
        assert_eq!(remap_encoded, remap_decoded);
    }

    #[test]
    fn sparse_roundtrip_via_v45() {
        // End-to-end: encode then decode via the full v4.5 path.
        // Data must be large enough that the 5-stream overhead pays.
        let phrase = b"the cat sat on the mat ";
        let mut data = Vec::new();
        for _ in 0..2000 {
            data.extend_from_slice(phrase);
        }
        let payload = encode_v45_multistream(&data).expect("encode should succeed");
        let decoded = decode_v45_multistream(&payload);
        assert_eq!(decoded, data, "sparse roundtrip via v4.5 failed");
    }

    #[test]
    fn sparse_table_smaller_than_dense() {
        // For a block with only a few dict refs, the sparse table
        // should be MUCH smaller than the dense 256-entry table.
        // Build a payload that produces ~10 distinct dict refs.
        let mut data = Vec::new();
        for _ in 0..2000 {
            data.extend_from_slice(b"the cat sat on the mat ");
        }
        let payload = encode_v45_multistream(&data).expect("encode should succeed");
        // Find the dict table section. The dict table is the
        // second-to-last sub-section in the payload. We can't easily
        // parse it from outside, but we know: if sparse works, the
        // payload is much smaller than 5*1KB = 5KB tables.
        assert!(
            payload.len() < 5000,
            "sparse payload suspiciously large: {} bytes",
            payload.len()
        );
    }
}
