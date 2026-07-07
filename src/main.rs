//! nexus CLI — `nexus c FILE` / `nexus d FILE`.
//!
//! Usage:
//!   nexus c <input> <output>           # compress a file
//!   nexus c --backend v5 --minify <in> <out>  # v5 LZMA + minify pre-filter
//!   nexus c <dir>  <output.nxar>        # compress a directory
//!   nexus d <input.nexus|nxar> <output>
//!   nexus bench                          # run corpus benchmark
//!
//! Compression backends (v4 default, v5 = LZMA via xz2, v6 = v5 + smart preprocessor):
//!   v4          — multi-stream LZ77 + rANS + dict codec (default)
//!   v5          — LZMA level 6 (balanced, fast)
//!   v5-min      — LZMA level 6 + minify pre-filter
//!   v5-extreme  — LZMA level 9 (max ratio)
//!   v6          — LZMA + swc AST minify (for .js/.ts/.tsx/.jsx) OR conservative minify
//!   v6-extreme  — same as v6 but LZMA level 9

use std::env;
use std::fs;
use std::path::Path;

use nexus_compress::engine;

fn print_help() {
    eprintln!("nexus CLI — NexusCompress v4 + v5 LZMA + v6 AST-aware + v6-solid");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("    nexus c [OPTIONS] <input> <output>");
    eprintln!("    nexus d [OPTIONS] <input> <output>");
    eprintln!("    nexus bench");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("    --backend NAME  v4 | v5 | v5-min | v5-extreme | v6 | v6-extreme");
    eprintln!("                    (default: v4)");
    eprintln!("    --minify        apply the conservative minify pre-filter (v5-min)");
    eprintln!("    --solid         directory: build a SOLID v6 archive (NXS6, LOSSY)");
    eprintln!("                    cross-file LZMA dictionary, max ratio on source code");
    eprintln!("    --level N       LZMA level 0..9 (default 6, used by --solid and v5/v6)");
    eprintln!("    -h, --help      show this help");
    eprintln!();
    eprintln!("EXAMPLES:");
    eprintln!("    nexus c big.txt out.nxs              # default v4");
    eprintln!("    nexus c --backend v5 code.js out.lz   # LZMA balanced");
    eprintln!("    nexus c --backend v5-min src out.lz  # LZMA + minify (lossless)");
    eprintln!("    nexus c --backend v6 app.tsx out.lz  # LZMA + swc AST minify (lossy)");
    eprintln!("    nexus c --backend v6-extreme code.js out.lz  # LZMA -9 + swc");
    eprintln!("    nexus c --solid src/ out.nxs6        # SOLID v6 (lossy, max ratio)");
    eprintln!("    nexus c --solid --level 9 src/ out.nxs6  # SOLID v6, LZMA -9");
    eprintln!("    nexus d out.nxs6 out_dir/            # auto-detect NXS6 / NXAR / v5");
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
    let mut solid = false;
    let mut lzma_level: Option<u32> = None;
    let mut positional: Vec<String> = Vec::new();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--backend" => {
                backend = iter.next().cloned();
            }
            "--minify" => {
                minify = true;
            }
            "--solid" => {
                solid = true;
            }
            "--level" => {
                lzma_level = iter.next().and_then(|s| s.parse().ok());
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
                if solid {
                    // SOLID v6 archive: NXS6 magic, single LZMA
                    // stream over the entire preprocessed corpus.
                    // LOSSY — decompress returns minified files.
                    let level = lzma_level
                        .or_else(|| {
                            if backend.as_deref() == Some("v6-extreme") {
                                Some(9)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(6);
                    let files = walk_dir(input).expect("walk dir");
                    if files.is_empty() {
                        eprintln!("error: directory has no files: {}", positional[1]);
                        std::process::exit(2);
                    }
                    let original_total: usize = files.iter().map(|(_, b)| b.len()).sum();
                    let t0 = std::time::Instant::now();
                    let archive = nexus_compress::solid_archive::compress(&files, level)
                        .expect("solid compress");
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    fs::write(output, &archive).expect("write output");
                    let ratio = original_total as f64 / archive.len().max(1) as f64;
                    eprintln!(
                        "{} -> {} (SOLID v6 LZMA {}, {:.2}x, {} files, {} bytes -> {} bytes, {:.0} ms)",
                        positional[1],
                        output,
                        level,
                        ratio,
                        files.len(),
                        original_total,
                        archive.len(),
                        ms,
                    );
                } else {
                    // Per-file NXAR archive (lossless v4).
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
                }
            } else {
                let bytes = fs::read(input).expect("read input");
                let out = if matches!(backend_name, "v6" | "v6-extreme") {
                    // v6 needs the file extension to pick the right
                    // preprocessor. engine::compress_with only does
                    // conservative minify unconditionally, so we
                    // call the v6 entry point directly.
                    let ext = input.extension().and_then(|e| e.to_str());
                    let level = if backend_name == "v6-extreme" { 9 } else { 6 };
                    engine::compress_v6(&bytes, ext, level)
                } else {
                    engine::compress_with(backend_name, &bytes, minify).expect("compress")
                };
                fs::write(output, &out).expect("write output");
                let ratio = bytes.len() as f64 / out.len().max(1) as f64;
                eprintln!(
                    "{} -> {} ({:.2}x, {})",
                    positional[1],
                    output,
                    ratio,
                    backend_name,
                );
            }
        }
        "d" | "decompress" => {
            if positional.len() < 3 {
                eprintln!("usage: nexus d [OPTIONS] <input> <output>");
                std::process::exit(2);
            }
            let input = fs::read(&positional[1]).expect("read input");
            // Auto-detect by magic.
            if input.len() >= 5 && &input[0..5] == nexus_compress::solid_archive::MAGIC {
                // NXS6 solid archive. Output is a directory; we
                // write the preprocessed bytes of each file. LOSSY.
                let out_path = Path::new(&positional[2]);
                std::fs::create_dir_all(out_path).expect("create output dir");
                let (entries, solid) =
                    nexus_compress::solid_archive::decompress(&input).expect("solid decompress");
                let mut n = 0;
                for e in &entries {
                    let start = e.solid_offset as usize;
                    let end = start + e.pre_size as usize;
                    let bytes = &solid[start..end];
                    let target = out_path.join(&e.name);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).expect("create parent");
                    }
                    std::fs::write(&target, bytes).expect("write extracted file");
                    n += 1;
                }
                eprintln!(
                    "{} -> {} (SOLID v6, {} files, lossy)",
                    positional[1], positional[2], n
                );
            } else if input.len() >= 4 && &input[0..4] == b"NXAR" {
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

/// Walk a directory recursively and return `(relative_path, bytes)`
/// for every regular file, sorted by relative path for determinism.
fn walk_dir(root: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut Vec<(String, Vec<u8>)>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                walk(root, &path, out)?;
            } else if file_type.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&path)?;
                out.push((rel, bytes));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}
