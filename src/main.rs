//! nexus CLI — `nexus c FILE` / `nexus d FILE` / `nexus bench`.
//!
//! Usage:
//!   nexus c <input> <output.nexus>
//!   nexus d <input.nexus> <output>
//!   nexus bench   # run internal benchmark vs zstd/gzip/xz on ./corpus/*

use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use nexus_compress::{compress, decompress};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage:\n  nexus c <input> <output.nexus>      # input is a file\n  \
             nexus c <dir>  <output.nxar>      # input is a directory (auto-detected)\n  \
             nexus d <input.nexus|nxar> <output>\n  \
             nexus bench"
        );
        std::process::exit(2);
    }
    match args[1].as_str() {
        "c" | "compress" => {
            if args.len() < 4 {
                eprintln!("usage: nexus c <input> <output>");
                std::process::exit(2);
            }
            let input = std::path::Path::new(&args[2]);
            let output = &args[3];
            if input.is_dir() {
                // Directory compression → NXAR archive.
                let (result, archive) = nexus_compress::api::compress_directory(
                    input,
                    nexus_compress::api::CompressionLevel::Fast,
                )
                .expect("compress directory");
                fs::write(output, &archive).expect("write output");
                eprintln!(
                    "{} -> {} ({:.2}x), {} files, {} bytes -> {} bytes",
                    args[2],
                    args[3],
                    result.aggregate_ratio,
                    result.n_files,
                    result.total_original_size,
                    result.total_compressed_size
                );
            } else {
                let bytes = fs::read(input).expect("read input");
                let out = compress(&bytes);
                fs::write(output, &out).expect("write output");
                eprintln!(
                    "{} -> {} ({:.2}x), {} blocks",
                    args[2],
                    args[3],
                    bytes.len() as f64 / out.len().max(1) as f64,
                    (bytes.len() + 256 * 1024 - 1) / (256 * 1024)
                );
            }
        }
        "d" | "decompress" => {
            if args.len() < 4 {
                eprintln!("usage: nexus d <input.nexus|nxar> <output>");
                std::process::exit(2);
            }
            let input = fs::read(&args[2]).expect("read input");
            // Auto-detect NXAR vs .nexus by magic.
            if input.len() >= 4 && &input[0..4] == b"NXAR" {
                let out_path = std::path::Path::new(&args[3]);
                std::fs::create_dir_all(out_path).expect("create output dir");
                let result = nexus_compress::api::decompress_directory(&input, out_path)
                    .expect("decompress directory");
                eprintln!(
                    "{} -> {} ({} files, {:.2}x)",
                    args[2], args[3], result.n_files, result.aggregate_ratio
                );
            } else {
                let out = decompress(&input);
                fs::write(&args[3], &out).expect("write output");
                eprintln!("{} -> {} ({} bytes)", args[2], args[3], out.len());
            }
        }
        "bench" => {
            run_bench();
        }
        _ => {
            eprintln!("unknown subcommand: {}", args[1]);
            std::process::exit(2);
        }
    }
}

fn run_bench() {
    let corpus_dir = Path::new("corpus");
    if !corpus_dir.exists() {
        eprintln!("missing ./corpus/ — create it with test files first");
        std::process::exit(2);
    }
    let mut entries: Vec<_> = fs::read_dir(corpus_dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    entries.sort();

    println!(
        "{:<22} {:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "file", "size B", "nexus B", "ratio", "t C ms", "t D ms", "zstd B", "zstd r", "gzip B"
    );
    println!("{}", "-".repeat(110));

    for path in entries {
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if data.is_empty() {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        // nexus compress
        let t0 = Instant::now();
        let compressed = compress(&data);
        let tc = t0.elapsed().as_secs_f64() * 1000.0;
        // nexus decompress
        let t1 = Instant::now();
        let decompressed = decompress(&compressed);
        let td = t1.elapsed().as_secs_f64() * 1000.0;

        assert_eq!(
            decompressed, data,
            "ROUNDTRIP MISMATCH for {}",
            name
        );

        let n_ratio = data.len() as f64 / compressed.len().max(1) as f64;

        // zstd -19 (max ratio)
        let zstd_out = run_external(
            "zstd",
            &["-19", "-q", "-f"],
            &data,
            &format!("/tmp/{}.zst", name),
        );
        let zstd_size = zstd_out.unwrap_or(0);
        let z_ratio = if zstd_size > 0 {
            data.len() as f64 / zstd_size as f64
        } else {
            0.0
        };

        // gzip -9
        let gzip_out = run_external("gzip", &["-9", "-c"], &data, &format!("/tmp/{}.gz", name));
        let gzip_size = gzip_out.unwrap_or(0);

        println!(
            "{:<22} {:>10} {:>9} {:>8.2}x {:>9.1} {:>9.1} {:>9} {:>8.2}x {:>9}",
            name,
            data.len(),
            compressed.len(),
            n_ratio,
            tc,
            td,
            zstd_size,
            z_ratio,
            gzip_size
        );
    }
}

fn run_external(cmd: &str, args: &[&str], input: &[u8], _hint: &str) -> Option<usize> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut sin) = child.stdin.take() {
        let _ = sin.write_all(input);
    }
    let output = child.wait_with_output().ok()?;
    Some(output.stdout.len())
}