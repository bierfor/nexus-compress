//! Dict ref smoke test — how many DictRef ops are emitted per corpus
//! file, and what fraction of the data they cover.
//!
//! Usage:
//!   cargo run --release --bin dict_smoke -- corpus/code.rs corpus/text.txt corpus/data.json
//!
//! For each file, runs MatchFinder::encode_with_dict with
//! default_combined_dict, and prints:
//!   - total ops (lit, match, dict_ref)
//!   - bytes covered by each op type
//!   - % of file covered by dict refs

use nexus_compress::dictionary::default_combined_dict;
use nexus_compress::lz77::{MatchFinder, Op};
use std::env;
use std::fs;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: dict_smoke <file1> [file2 ...]");
        std::process::exit(1);
    }
    let dict = default_combined_dict();
    println!("Dictionary: {} entries", dict.len());
    println!();
    println!(
        "{:<20} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "file", "size_B", "n_lits", "n_match", "n_dictrf", "dict_cover", "lit_cover"
    );
    println!("{}", "-".repeat(82));
    for path in &args {
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("skip {}: {}", path, e);
                continue;
            }
        };
        let mut enc = MatchFinder::new();
        let ops = enc.encode_with_dict(&data, &dict);
        let mut n_lits = 0usize;
        let mut n_match = 0usize;
        let mut n_dictrf = 0usize;
        let mut lit_cover = 0usize;
        let mut match_cover = 0usize;
        let mut dict_cover = 0usize;
        for op in &ops {
            match op {
                Op::Lit(b) => {
                    n_lits += 1;
                    lit_cover += 1;
                    let _ = b;
                }
                Op::Match { len, .. } => {
                    n_match += 1;
                    match_cover += *len as usize;
                }
                Op::DictRef { len, .. } => {
                    n_dictrf += 1;
                    dict_cover += *len as usize;
                }
            }
        }
        let total_cover = lit_cover + match_cover + dict_cover;
        let dict_pct = if total_cover > 0 {
            100.0 * dict_cover as f64 / total_cover as f64
        } else {
            0.0
        };
        let lit_pct = if total_cover > 0 {
            100.0 * lit_cover as f64 / total_cover as f64
        } else {
            0.0
        };
        println!(
            "{:<20} {:>10} {:>10} {:>10} {:>10} {:>9.1}% {:>9.1}%",
            std::path::Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path),
            data.len(),
            n_lits,
            n_match,
            n_dictrf,
            dict_pct,
            lit_pct,
        );
    }
}
