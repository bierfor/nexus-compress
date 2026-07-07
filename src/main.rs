//! nexus CLI — `nexus c FILE` / `nexus d FILE`.
//!
//! Usage:
//!   nexus c <input> <output>           # compress a file
//!   nexus c --backend v5 --minify <in> <out>  # v5 LZMA + minify pre-filter
//!   nexus c <dir>  <output.nxar>        # compress a directory
//!   nexus d <input.nexus|nxar> <output>
//!   nexus bench                          # run corpus benchmark
//!
//! Compression backends (v4 default, v5 = LZMA via xz2):
//!   v4          — multi-stream LZ77 + rANS + dict codec (default)
//!   v5          — LZMA level 6 (balanced, fast)
//!   v5-min      — LZMA level 6 + minify pre-filter
//!   v5-extreme  — LZMA level 9 (max ratio)

use std::env;
use std::fs;
use std::path::Path;

use nexus_compress::engine;

fn print_help() {
    eprintln!("nexus CLI — NexusCompress v4 + v5 LZMA");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("    nexus c [OPTIONS] <input> <output>");
    eprintln!("    nexus d [OPTIONS] <input> <output>");
    eprintln!("    nexus bench");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("    --backend NAME  v4 | v5 | v5-min | v5-extreme   (default: v4)");
    eprintln!("    --minify        apply the conservative minify pre-filter");
    eprintln!("    -h, --help      show this help");
    eprintln!();
    eprintln!("EXAMPLES:");
    eprintln!("    nexus c big.txt out.nxs              # default v4");
    eprintln!("    nexus c --backend v5 code.js out.lz   # LZMA balanced");
    eprintln!("    nexus c --backend v5-min src out.lz  # LZMA + minify (lossless)");
    eprintln!("    nexus c --backend v5-extreme big.nxs tiny.lz  # LZMA -9 (max ratio)");
    eprintln!("    nexus d out.nxs big.txt              # auto-detect v4/v5");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_help();
        std::process::exit(2);
    }
    let mut iter = args.iter().skip(1);
    let mut backend: Option<String> = None;
    let mut minify = false;
    let mut positional: Vec<String> = Vec::new();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--backend" => {
                backend = iter.next().cloned();
            }
            "--minify" => {
                minify = true;
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            other if other.starts_with("--") => {
                eprintln!("unknown option: {}", other);
                std::process::exit(2);
            }
            other => {
                positional.push(other.to_string());
            }
        }
    }
    if positional.is_empty() {
        print_help();
        std::process::exit(2);
    }
    let sub = positional[0].as_str();
    match sub {
        "c" | "compress" => {
            if positional.len() < 3 {
                eprintln!("usage: nexus c [OPTIONS] <input> <output>");
                std::process::exit(2);
            }
            let input = Path::new(&positional[1]);
            let output = &positional[2];
            let backend_name = backend.as_deref().unwrap_or("v4");
            if input.is_dir() {
                // Directory compression → NXAR archive.
                // The directory path uses the per-file v4 pipeline
                // (multi-stream, dedup, dict). The --backend flag
                // applies to single-file mode.
                if backend.is_some() {
                    eprintln!("warn: --backend is ignored for directory compression (NXAR uses v4 per-file)");
                }
                let (result, archive) = nexus_compress::api::compress_directory(
                    input,
                    nexus_compress::api::CompressionLevel::Fast,
                )
                .expect("compress directory");
                fs::write(output, &archive).expect("write output");
                eprintln!(
                    "{} -> {} ({:.2}x), {} files, {} bytes -> {} bytes",
                    positional[1],
                    output,
                    result.aggregate_ratio,
                    result.n_files,
                    result.total_original_size,
                    result.total_compressed_size
                );
            } else {
                let bytes = fs::read(input).expect("read input");
                let out =
                    engine::compress_with(backend_name, &bytes, minify).expect("compress");
                fs::write(output, &out).expect("write output");
                let ratio = bytes.len() as f64 / out.len().max(1) as f64;
                eprintln!(
                    "{} -> {} ({:.2}x, {}, minify={})",
                    positional[1],
                    output,
                    ratio,
                    backend_name,
                    minify
                );
            }
        }
        "d" | "decompress" => {
            if positional.len() < 3 {
                eprintln!("usage: nexus d [OPTIONS] <input> <output>");
                std::process::exit(2);
            }
            let input = fs::read(&positional[1]).expect("read input");
            // Auto-detect NXAR vs v4/v5 by magic.
            if input.len() >= 4 && &input[0..4] == b"NXAR" {
                let out_path = Path::new(&positional[2]);
                std::fs::create_dir_all(out_path).expect("create output dir");
                let result = nexus_compress::api::decompress_directory(&input, out_path)
                    .expect("decompress directory");
                eprintln!(
                    "{} -> {} ({} files, {:.2}x)",
                    positional[1], positional[2], result.n_files, result.aggregate_ratio
                );
            } else {
                let out = engine::decompress_any(&input).expect("decompress");
                fs::write(&positional[2], &out).expect("write output");
                eprintln!("{} -> {} ({} bytes)", positional[1], positional[2], out.len());
            }
        }
        "bench" => {
            // Delegate to the bin/bench_suite binary — but if we're
            // invoked as `nexus bench` from the lib, we just print
            // a hint.
            eprintln!("`nexus bench` is moved to the bench_suite binary. Run:");
            eprintln!("    cargo run --release --bin bench_suite");
        }
        _ => {
            eprintln!("unknown subcommand: {}", sub);
            print_help();
            std::process::exit(2);
        }
    }
}
