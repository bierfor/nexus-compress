//! Cost probe: measure REAL bits-per-symbol in the v4.8 bitstream.
//!
//! Run via `cargo run --release --bin cost_probe -- <file>` or
//! `cargo test --lib --release cost_probe` (the unit test runs
//! against `corpus/text.txt`).
//!
//! This is the "telemetría de costos" step of sprint 2.9 — it
//! measures what the bitstream ACTUALLY costs so we can fix the
//! DP's cost model in `cost::LITERAL_BITS` and `cost::MATCH_BITS`.
//!
//! ## Why this matters
//!
//! The static `cost::LITERAL_BITS = 13` and `cost::MATCH_BITS = 32`
//! are leftover from the v2 era when:
//!
//! - Op-flags were 1 byte (now 2 bits).
//! - Match distance was u16 always (now 2 rANS streams of 8 bits each).
//! - Match length was u8 always (now 1 rANS stream of 8 bits).
//!
//! With v4.7/v4.8 (sparse v3 streams + dict codec), a literal
//! costs ~2 (flag) + H(byte) bits, not 13. A match costs ~2 (flag)
//! + 3 * H(byte) bits, not 32. The optimal parser's DP has been
//! making decisions based on a model that's 2-3x too pessimistic
//! about literals and matches.
//!
//! ## What the probe does
//!
//! 1. Compresses the input via the v4.8 codec.
//! 2. Walks the block stream and counts ops per type (Lit, Match, DictRef).
//! 3. Reports the average bits per op-type.
//!
//! The probe can't perfectly attribute per-op costs (the rANS
//! streams encode the symbols jointly, and the table header is
//! amortized), but the per-symbol average is what the DP needs
//! to make good per-position decisions.

pub mod probe {
    use std::collections::HashMap;

    /// Probe a single file's compressed output and return per-op-type
    /// counts and the encoded stream size.
    pub struct ProbeResult {
        pub file_size: u64,
        pub compressed_size: u64,
        pub total_ops: u64,
        pub lit_ops: u64,
        pub match_ops: u64,
        pub dict_ops: u64,
        /// Encoded op-flags stream size in bytes (2 bits per op, packed).
        pub op_flags_bytes: u64,
        /// Average bits per op, computed as
        /// (op_flags_bytes * 8 + sum_of_stream_bits) / total_ops.
        pub bits_per_op: f64,
    }

    /// Walk the v4.5/v4.8 compressed stream and count ops.
    ///
    /// The stream format (see `src/dict_codec.rs`):
    ///   [u8 tag=TAG_V3_MULTISTREAM_DICT=8]
    ///   [u8 dict_select]
    ///   [32|669 byte dict bitmask]
    ///   [u32 op_flags_len][op_flags_bytes: 2 bits per op, packed 4 per byte]
    ///   [per-stream: u32 table_len, table_bytes, u32 stream_len, stream_bytes] × 5
    ///
    /// We don't fully parse the streams (we don't need to), but
    /// the op-flags ARE parseable and that's what we need.
    pub fn probe(data: &[u8]) -> Option<ProbeResult> {
        // Use the public codec to compress.
        let compressed = crate::compress(data);

        if compressed.is_empty() {
            return None;
        }

        // Walk the blocks. The format is:
        //   [u32 header_len][header_bytes]
        //   blocks...
        // The header is serialized via `serde_cbor` typically — let's
        // just use the public API to iterate via decode.
        let decoded = crate::decompress(&compressed);
        if decoded != data {
            return None; // Sanity: roundtrip must hold.
        }

        // Now count ops by re-running the encode and walking the
        // ops vector. This is duplicative but the only way to get
        // per-op counts without instrumenting the codec.
        // For simplicity, just compress once and re-encode the same
        // data to count ops.
        let mut counts = HashMap::new();
        count_ops_in_encode(data, &mut counts);

        let lit_ops = *counts.get("Lit").unwrap_or(&0);
        let match_ops = *counts.get("Match").unwrap_or(&0);
        let dict_ops = *counts.get("DictRef").unwrap_or(&0);
        let total_ops = lit_ops + match_ops + dict_ops;

        // Op-flags are 2 bits per op, packed 4 per byte.
        let op_flags_bytes = (total_ops * 2).div_ceil(8);

        // Use the codec's encoded size as the "all-in" cost.
        // (Includes 5 rANS stream table headers + 5 rANS stream payloads
        // + dict bitmask + per-block headers. The "bits per op" is
        // an upper bound on per-op contribution.)
        let bits_per_op = if total_ops > 0 {
            (compressed.len() as f64 * 8.0) / total_ops as f64
        } else {
            0.0
        };

        Some(ProbeResult {
            file_size: data.len() as u64,
            compressed_size: compressed.len() as u64,
            total_ops,
            lit_ops,
            match_ops,
            dict_ops,
            op_flags_bytes,
            bits_per_op,
        })
    }

    /// Run the LZ77 encoder on `data` (using the trained dict) and
    /// tally ops by variant. We use the public `compress` indirectly
    /// by calling `MatchFinder::encode_with_dict` — this gives us the
    /// exact ops the codec would have produced for the active dict.
    fn count_ops_in_encode(data: &[u8], counts: &mut HashMap<&'static str, u64>) {
        use crate::dict_codec::{dict_select_for_block, load_sub_dict};
        use crate::lz77::MatchFinder;

        // Per-block, run the same LZ77 the codec would.
        let chunks = crate::cdc::chunkify(data, 4 * 1024, 64 * 1024, 15);
        for i in 0..chunks.len() {
            let (s, len) = chunks[i];
            let block = &data[s..s + len];
            if block.len() < crate::lz77::MIN_MATCH {
                // Tiny tail: all literals.
                *counts.entry("Lit").or_insert(0) += block.len() as u64;
                continue;
            }
            let sel = dict_select_for_block(block);
            let sub = load_sub_dict(sel);
            let mut mf = MatchFinder::new();
            let ops = if let Some(d) = sub {
                mf.encode_with_dict(block, &d)
            } else {
                mf.encode(block)
            };
            for op in &ops {
                let key: &'static str = match op {
                    crate::lz77::Op::Lit(_) => "Lit",
                    crate::lz77::Op::Match { .. } => "Match",
                    crate::lz77::Op::DictRef { .. } => "DictRef",
                };
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    /// Per-stream bits-per-symbol probe.
    ///
    /// Encodes the input via the codec, then for each of the 5 rANS
    /// streams (lit, len, dist_lo, dist_hi, dict_id), measures the
    /// stream payload size (excluding table header) and divides by
    /// the symbol count. This is the actual `H(symbol)` the encoder
    /// achieves, which is what the DP cost model needs.
    pub struct StreamBits {
        pub name: &'static str,
        pub symbols: u64,
        pub payload_bytes: u64, // excluding table header
        pub bits_per_symbol: f64,
    }

    pub fn stream_bits_per_symbol(data: &[u8]) -> Vec<StreamBits> {
        use crate::dict_codec::load_sub_dict;
        use crate::lz77::{MatchFinder, Op};

        let mut out = Vec::new();
        let mut lit_syms: Vec<u8> = Vec::new();
        let mut len_syms: Vec<u8> = Vec::new();
        let mut dist_lo_syms: Vec<u8> = Vec::new();
        let mut dist_hi_syms: Vec<u8> = Vec::new();
        let mut dict_syms: Vec<u32> = Vec::new();

        let chunks = crate::cdc::chunkify(data, 4 * 1024, 64 * 1024, 15);
        for i in 0..chunks.len() {
            let (s, len) = chunks[i];
            let block = &data[s..s + len];
            if block.len() < crate::lz77::MIN_MATCH {
                for &b in block {
                    lit_syms.push(b);
                }
                continue;
            }
            let sel = crate::dict_codec::dict_select_for_block(block);
            let sub = load_sub_dict(sel);
            let mut mf = MatchFinder::new();
            let ops = match sub {
                Some(d) => mf.encode_with_dict(block, &d),
                None => mf.encode(block),
            };
            for op in &ops {
                match op {
                    Op::Lit(b) => lit_syms.push(*b),
                    Op::Match { dist, len } => {
                        len_syms.push(*len as u8);
                        dist_lo_syms.push((*dist & 0xFF) as u8);
                        dist_hi_syms.push(((*dist >> 8) & 0xFF) as u8);
                    }
                    Op::DictRef { id, .. } => dict_syms.push(*id as u32),
                }
            }
        }

        // Use rANS to encode each stream and measure the payload
        // (excluding the table). The payload is the bitstream that
        // rANS produces for these symbols.
        out.push(StreamBits {
            name: "lit",
            symbols: lit_syms.len() as u64,
            payload_bytes: rans_payload_size_u8(&lit_syms, 12),
            bits_per_symbol: bits_per(&lit_syms, 12),
        });
        out.push(StreamBits {
            name: "len",
            symbols: len_syms.len() as u64,
            payload_bytes: rans_payload_size_u8(&len_syms, 12),
            bits_per_symbol: bits_per(&len_syms, 12),
        });
        out.push(StreamBits {
            name: "dist_lo",
            symbols: dist_lo_syms.len() as u64,
            payload_bytes: rans_payload_size_u8(&dist_lo_syms, 12),
            bits_per_symbol: bits_per(&dist_lo_syms, 12),
        });
        out.push(StreamBits {
            name: "dist_hi",
            symbols: dist_hi_syms.len() as u64,
            payload_bytes: rans_payload_size_u8(&dist_hi_syms, 12),
            bits_per_symbol: bits_per(&dist_hi_syms, 12),
        });
        out.push(StreamBits {
            name: "dict_id",
            symbols: dict_syms.len() as u64,
            payload_bytes: rans_payload_size_u32(&dict_syms, 12),
            bits_per_symbol: bits_per_u32(&dict_syms, 12),
        });
        out
    }

    /// Encode a u8 stream with rANS and return the payload size
    /// in bytes (excluding the freq-table header). The bitstream
    /// is what actually gets written to the .nexus file for these
    /// symbols.
    fn rans_payload_size_u8(syms: &[u8], _scale_bits: u32) -> u64 {
        if syms.is_empty() {
            return 0;
        }
        let counts = count_bytes(syms);
        let counts_vec: Vec<u32> = counts.to_vec();
        let table = crate::rans_v4::FreqTable::from_counts(&counts_vec);
        let symbols_u32: Vec<u32> = syms.iter().map(|&b| b as u32).collect();
        let bytes = crate::rans_v4::rans_encode(&symbols_u32, &table);
        bytes.len() as u64
    }

    /// Same for u32 symbols (used by the dict_id stream).
    fn rans_payload_size_u32(syms: &[u32], _scale_bits: u32) -> u64 {
        if syms.is_empty() {
            return 0;
        }
        let counts = count_u32(syms);
        let max_id = counts.keys().max().copied().unwrap_or(0);
        let mut counts_vec = vec![0u32; (max_id as usize) + 1];
        for (k, v) in &counts {
            counts_vec[*k as usize] = *v;
        }
        let table = crate::rans_v4::FreqTable::from_counts(&counts_vec);
        let bytes = crate::rans_v4::rans_encode(syms, &table);
        bytes.len() as u64
    }

    fn bits_per(syms: &[u8], scale_bits: u32) -> f64 {
        if syms.is_empty() {
            return 0.0;
        }
        (rans_payload_size_u8(syms, scale_bits) as f64 * 8.0) / syms.len() as f64
    }

    fn bits_per_u32(syms: &[u32], scale_bits: u32) -> f64 {
        if syms.is_empty() {
            return 0.0;
        }
        (rans_payload_size_u32(syms, scale_bits) as f64 * 8.0) / syms.len() as f64
    }

    fn count_bytes(syms: &[u8]) -> [u32; 256] {
        let mut c = [0u32; 256];
        for &b in syms {
            c[b as usize] += 1;
        }
        c
    }

    fn count_u32(syms: &[u32]) -> std::collections::HashMap<u32, u32> {
        let mut c = std::collections::HashMap::new();
        for &s in syms {
            *c.entry(s).or_insert(0) += 1;
        }
        c
    }

    /// Pretty-print a probe result.
    pub fn print_probe(label: &str, r: &ProbeResult) {
        eprintln!(
            "{label:20}  size={:8}  compressed={:6}  ops={:6} (L={:6} M={:6} D={:6})  bits/op={:5.2}",
            r.file_size,
            r.compressed_size,
            r.total_ops,
            r.lit_ops,
            r.match_ops,
            r.dict_ops,
            r.bits_per_op
        );
    }

    pub fn print_streams(label: &str, streams: &[StreamBits]) {
        eprintln!("{label}: per-stream bits/symbol");
        for s in streams {
            eprintln!(
                "  {:10}  syms={:8}  payload={:6} B  bits/sym={:5.2}",
                s.name, s.symbols, s.payload_bytes, s.bits_per_symbol
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::probe::*;
    use std::io::Read;

    #[test]
    fn cost_probe_runs_on_corpus() {
        // Run the probe on every file in corpus/. Useful for
        // comparing the cost model against the real bitstream.
        let corpus_dir = std::env::current_dir()
            .expect("cwd")
            .join("corpus");
        if !corpus_dir.is_dir() {
            eprintln!("(skipping: no corpus/ dir)");
            return;
        }
        let entries: Vec<_> = std::fs::read_dir(&corpus_dir)
            .expect("read corpus")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.contains('.'))
                        .unwrap_or(false)
            })
            .collect();
        let mut paths = entries;
        paths.sort();

        eprintln!("\n=== Per-file probe (ops + bits/op) ===");
        for p in &paths {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let mut f = std::fs::File::open(p).expect("open");
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            if data.is_empty() {
                continue;
            }
            if let Some(r) = probe(&data) {
                print_probe(&name, &r);
            }
        }

        eprintln!("\n=== Per-stream bits/symbol (text.txt only) ===");
        let text_path = corpus_dir.join("text.txt");
        if text_path.is_file() {
            let data = std::fs::read(&text_path).expect("read text.txt");
            let streams = stream_bits_per_symbol(&data);
            print_streams("text.txt", &streams);
        }

        eprintln!("\n=== Current static cost model (cost.rs) ===");
        eprintln!("  LITERAL_BITS = {}", crate::cost::LITERAL_BITS);
        eprintln!("  MATCH_BITS = {}", crate::cost::MATCH_BITS);
        eprintln!("  MATCH_LENGTH_THRESHOLD = {}", crate::cost::MATCH_LENGTH_THRESHOLD);
    }
}
