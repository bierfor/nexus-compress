//! Compare two dictionaries: the hand-curated default and a trained
//! one. For each corpus file, prints DictRef count for both.
//!
//! Usage:
//!   dict_compare <trained-dict-file> [corpus-file...]

use nexus_compress::dictionary::{default_combined_dict, Dictionary};
use nexus_compress::lz77::{MatchFinder, Op};
use std::env;
use std::fs;

fn count_dict_refs(dict: &Dictionary, data: &[u8]) -> (usize, usize, usize) {
    let mut enc = MatchFinder::new();
    let ops = enc.encode_with_dict(data, dict);
    let mut n_lits = 0;
    let mut n_match = 0;
    let mut n_dr = 0;
    for op in &ops {
        match op {
            Op::Lit(_) => n_lits += 1,
            Op::Match { .. } => n_match += 1,
            Op::DictRef { .. } => n_dr += 1,
        }
    }
    (n_lits, n_match, n_dr)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: dict_compare <trained-dict-file> [corpus-file...]");
        std::process::exit(1);
    }
    let trained_path = &args[0];
    let trained_bytes = fs::read(trained_path).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {}", trained_path, e);
        std::process::exit(1);
    });
    let trained = Dictionary::from_bytes(&trained_bytes).unwrap_or_else(|| {
        eprintln!("{} is not a valid .dict file", trained_path);
        std::process::exit(1);
    });
    let combined = default_combined_dict();

    eprintln!(
        "Default combined: {} entries ({} bytes token payload)",
        combined.len(),
        combined.iter().map(|(_, t)| t.len()).sum::<usize>()
    );
    eprintln!(
        "Trained ({}): {} entries ({} bytes token payload)\n",
        trained_path,
        trained.len(),
        trained.iter().map(|(_, t)| t.len()).sum::<usize>()
    );

    let corpus: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
    let inputs: Vec<&str> = if corpus.is_empty() {
        // Default: use the 3 main corpus files.
        vec![
            "corpus/code.rs",
            "corpus/text.txt",
            "corpus/data.json",
            "corpus/mixed.bin",
            "corpus/random.bin",
            "corpus/repetitive.bin",
        ]
    } else {
        corpus
    };

    println!(
        "{:<20} {:>10} {:>15} {:>15} {:>10}",
        "file", "size", "default_dr", "trained_dr", "ratio"
    );
    println!("{}", "-".repeat(72));
    for path in inputs {
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(_) => {
                eprintln!("skip {}", path);
                continue;
            }
        };
        let (_, _, dr_default) = count_dict_refs(&combined, &data);
        let (_, _, dr_trained) = count_dict_refs(&trained, &data);
        let ratio = if dr_default > 0 {
            dr_trained as f64 / dr_default as f64
        } else if dr_trained > 0 {
            f64::INFINITY
        } else {
            0.0
        };
        let ratio_str = if ratio.is_infinite() {
            "INF".to_string()
        } else {
            format!("{:.1}x", ratio)
        };
        println!(
            "{:<20} {:>10} {:>15} {:>15} {:>10}",
            std::path::Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path),
            data.len(),
            dr_default,
            dr_trained,
            ratio_str,
        );
    }
}
