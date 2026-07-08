//! Dictionary trainer — extracts common substrings from a corpus and
//! writes them to a `.dict` binary file.
//!
//! Usage:
//!   dict_train [--max-size N] [--min-len N] [--max-len N] [--min-freq N]
//!              [--output FILE] [--quiet] <input-file>...
//!
//! Defaults:
//!   --max-size  32768  (32 KB target — same as zstd's default)
//!   --min-len   3
//!   --max-len   8
//!   --min-freq  4
//!   --output    trained.dict
//!
//! Algorithm:
//!   1. Slide a window of min_len..=max_len over each input file.
//!   2. HashMap<Vec<u8>, u32> for frequency counting.
//!   3. Score = frequency * (length - 1). A 6-byte token at freq 100
//!      outscores a 2-byte token at freq 200 (300 > 200).
//!   4. Sort by score descending.
//!   5. Greedy top-down pick until --max-size is reached.

use nexus_compress::dictionary::Dictionary;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process;
use std::time::Instant;

struct Args {
    max_size: usize,
    max_entries: usize,
    min_len: usize,
    max_len: usize,
    min_freq: u32,
    output: PathBuf,
    quiet: bool,
    inputs: Vec<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut max_size = 32 * 1024usize;
    let mut max_entries = usize::MAX;
    let mut min_len = 3usize;
    let mut max_len = 8usize;
    let mut min_freq = 4u32;
    let mut output = PathBuf::from("trained.dict");
    let mut quiet = false;
    let mut inputs: Vec<PathBuf> = Vec::new();

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--max-size" => {
                max_size = iter
                    .next()
                    .ok_or("--max-size requires a value")?
                    .parse()
                    .map_err(|_| "--max-size must be a positive integer")?;
            }
            "--max-entries" => {
                max_entries = iter
                    .next()
                    .ok_or("--max-entries requires a value")?
                    .parse()
                    .map_err(|_| "--max-entries must be a positive integer")?;
            }
            "--min-len" => {
                min_len = iter
                    .next()
                    .ok_or("--min-len requires a value")?
                    .parse()
                    .map_err(|_| "--min-len must be a positive integer")?;
            }
            "--max-len" => {
                max_len = iter
                    .next()
                    .ok_or("--max-len requires a value")?
                    .parse()
                    .map_err(|_| "--max-len must be a positive integer")?;
            }
            "--min-freq" => {
                min_freq = iter
                    .next()
                    .ok_or("--min-freq requires a value")?
                    .parse()
                    .map_err(|_| "--min-freq must be a positive integer")?;
            }
            "--output" | "-o" => {
                output = PathBuf::from(iter.next().ok_or("--output requires a value")?);
            }
            "--quiet" | "-q" => {
                quiet = true;
            }
            "--help" | "-h" => {
                print_help();
                process::exit(0);
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option: {}", other));
            }
            other => {
                inputs.push(PathBuf::from(other));
            }
        }
    }

    if inputs.is_empty() {
        return Err("no input files".to_string());
    }
    if min_len > max_len {
        return Err(format!("--min-len ({}) > --max-len ({})", min_len, max_len));
    }
    if max_size == 0 {
        return Err("--max-size must be > 0".to_string());
    }

    Ok(Args {
        max_size,
        max_entries,
        min_len,
        max_len,
        min_freq,
        output,
        quiet,
        inputs,
    })
}

fn print_help() {
    eprintln!("dict_train — train a static literal dictionary from a corpus");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("    dict_train [OPTIONS] <input-file>...");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("    --max-size N    Target dict size in bytes (default: 32768 = 32 KB)");
    eprintln!("    --max-entries N Hard cap on number of entries (default: unlimited)");
    eprintln!("    --min-len N     Minimum token length (default: 3)");
    eprintln!("    --max-len N     Maximum token length (default: 8)");
    eprintln!("    --min-freq N    Minimum token frequency (default: 4)");
    eprintln!("    -o, --output F  Output .dict file (default: trained.dict)");
    eprintln!("    -q, --quiet     Suppress per-file progress");
    eprintln!("    -h, --help      Print this help");
    eprintln!();
    eprintln!("EXAMPLE:");
    eprintln!("    dict_train -o code.dict corpus/code.rs corpus/code2.rs");
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {}", e);
            eprintln!();
            print_help();
            process::exit(2);
        }
    };

    if !args.quiet {
        eprintln!(
            "dict_train: max_size={} max_entries={} min_len={} max_len={} min_freq={} output={}",
            args.max_size,
            args.max_entries,
            args.min_len,
            args.max_len,
            args.min_freq,
            args.output.display()
        );
    }

    // Read all input files.
    let start = Instant::now();
    let mut corpora: Vec<Vec<u8>> = Vec::new();
    let mut total_input_bytes: u64 = 0;
    for path in &args.inputs {
        match fs::read(path) {
            Ok(data) => {
                total_input_bytes += data.len() as u64;
                if !args.quiet {
                    eprintln!("  read {} ({} bytes)", path.display(), data.len());
                }
                corpora.push(data);
            }
            Err(e) => {
                eprintln!("error: cannot read {}: {}", path.display(), e);
                process::exit(1);
            }
        }
    }
    let corpora_refs: Vec<&[u8]> = corpora.iter().map(|v| v.as_slice()).collect();
    let read_elapsed = start.elapsed();
    if !args.quiet {
        eprintln!(
            "  total: {} files, {} bytes read in {:?}",
            args.inputs.len(),
            total_input_bytes,
            read_elapsed
        );
    }

    // Train.
    let train_start = Instant::now();
    let (dict, stats) = Dictionary::train_from_corpus_capped(
        &corpora_refs,
        args.max_size,
        args.min_len,
        args.max_len,
        args.min_freq,
        args.max_entries,
    );
    let train_elapsed = train_start.elapsed();
    let serial = dict.to_bytes();

    // Write.
    let write_start = Instant::now();
    let mut f = match fs::File::create(&args.output) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: cannot create {}: {}", args.output.display(), e);
            process::exit(1);
        }
    };
    if let Err(e) = f.write_all(&serial) {
        eprintln!("error: write failed: {}", e);
        process::exit(1);
    }
    let write_elapsed = write_start.elapsed();

    eprintln!();
    eprintln!("Dictionary trained in {:?}", train_elapsed);
    eprintln!(
        "  scanned          : {} substrings",
        stats.total_substrings_scanned
    );
    eprintln!("  tokens kept      : {} entries", stats.unique_tokens_kept);
    eprintln!(
        "  token bytes used : {} / {} (max)",
        stats.dict_bytes, args.max_size
    );
    eprintln!(
        "  written to       : {} ({} bytes on disk)",
        args.output.display(),
        serial.len()
    );
    eprintln!(
        "  total elapsed    : read {:?} + train {:?} + write {:?}",
        read_elapsed, train_elapsed, write_elapsed
    );
    eprintln!();
    eprintln!("Top 20 entries:");
    let mut entries: Vec<(Vec<u8>,)> = dict.iter().map(|(_, t)| (t.to_vec(),)).collect();
    // Sort by length desc for display (longer first, since they're more interesting)
    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (i, (tok,)) in entries.iter().take(20).enumerate() {
        let display = String::from_utf8_lossy(tok);
        eprintln!("  [{:>2}] len={} {:?}", i + 1, tok.len(), display);
    }
}
