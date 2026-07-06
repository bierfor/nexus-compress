//! Honest corpus benchmark suite.
//!
//! For each file in `./corpus/`, measure:
//!   - NexusCompress v4: size, ratio, compress ms, decompress ms, roundtrip OK
//!   - gzip -9           : size, ratio, compress ms, decompress ms
//!   - zstd -19          : size, ratio, compress ms, decompress ms
//!
//! Output: a markdown table printed to stdout + a final gap-to-zstd summary.
//!
//! Run with: `cargo run --release --bin corpus_suite`

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use nexus_compress::{compress, decompress};

#[derive(Clone)]
struct Row {
    file: String,
    size: usize,
    nexus_size: usize,
    nexus_compress_ms: f64,
    nexus_decompress_ms: f64,
    nexus_ok: bool,
    gzip_size: usize,
    gzip_compress_ms: f64,
    gzip_decompress_ms: f64,
    gzip_ok: bool,
    zstd_size: usize,
    zstd_compress_ms: f64,
    zstd_decompress_ms: f64,
    zstd_ok: bool,
}

fn timed<F: FnOnce() -> R, R>(f: F) -> (R, f64) {
    let t0 = Instant::now();
    let r = f();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    (r, ms)
}

fn main() {
    let corpus_dir = Path::new("corpus");
    if !corpus_dir.exists() {
        eprintln!("missing ./corpus/ — create it first (e.g. with scripts/gen_corpus.py)");
        std::process::exit(2);
    }

    eprintln!("[corpus_suite] NexusCompress v4 vs gzip -9 vs zstd -19\n");
    eprintln!("Running benchmarks... (this may take a few seconds)\n");

    // Collect per-file results. Skip files without an extension or
    // dot-prefix — the corpus script `gen_corpus.py` only writes
    // extensioned names, so anything bare-named (e.g. `code`, `data`)
    // is a stray duplicate from older test runs that would
    // double-count the aggregate. Dotfiles (`.DS_Store`) and
    // properly extensioned files (`.rs`, `.json`, ...) pass through.
    let entries: Vec<_> = std::fs::read_dir(corpus_dir)
        .expect("read corpus")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.contains('.')
        })
        .collect();
    let mut paths: Vec<_> = entries;
    paths.sort();

    let mut rows: Vec<Row> = Vec::new();

    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let data = match std::fs::read(path) {
            Ok(d) if !d.is_empty() => d,
            _ => continue,
        };

        eprint!("  {} ({} B)...", name, data.len());
        let _ = std::io::stderr().flush();

        // NexusCompress
        let (compressed, c_ms) = timed(|| compress(&data));
        let (decompressed, d_ms) = timed(|| decompress(&compressed));
        let nexus_ok = decompressed == data;

        // gzip
        let gzip_path = path.with_extension("gz");
        let gzip_c_ms = {
            let t0 = Instant::now();
            let child = Command::new("gzip")
                .args(&["-9", "-c", "-n"])
                .arg(&path)
                .stdout(Stdio::piped())
                .spawn()
                .expect("gzip");
            let out = child.wait_with_output().expect("gzip out");
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let _ = std::fs::write(&gzip_path, &out.stdout);
            ms
        };
        let gzip_size = std::fs::metadata(&gzip_path).map(|m| m.len() as usize).unwrap_or(0);
        let gzip_d_ms = {
            let t0 = Instant::now();
            let child = Command::new("gzip")
                .args(&["-d", "-c", "-n"])
                .arg(&gzip_path)
                .stdout(Stdio::piped())
                .spawn()
                .expect("gzip -d");
            let _ = child.wait_with_output();
            t0.elapsed().as_secs_f64() * 1000.0
        };
        let gzip_ok = {
            let out = Command::new("gzip")
                .args(&["-d", "-c", "-n"])
                .arg(&gzip_path)
                .stdout(Stdio::piped())
                .output()
                .expect("gzip -d");
            out.stdout == data
        };
        let _ = std::fs::remove_file(&gzip_path);

        // zstd
        let zstd_path = path.with_extension("zst");
        let zstd_c_ms = {
            let t0 = Instant::now();
            let mut child = Command::new("zstd")
                .args(&["-19", "-q", "-f"])
                .arg(path)
                .arg("-o")
                .arg(&zstd_path)
                .stdout(Stdio::null())
                .spawn()
                .expect("zstd");
            let _ = child.wait();
            t0.elapsed().as_secs_f64() * 1000.0
        };
        let zstd_size = std::fs::metadata(&zstd_path).map(|m| m.len() as usize).unwrap_or(0);
        let zstd_d_ms = {
            let t0 = Instant::now();
            let mut child = Command::new("zstd")
                .args(&["-d", "-q", "-f"])
                .arg(&zstd_path)
                .stdout(Stdio::null())
                .spawn()
                .expect("zstd -d");
            let _ = child.wait();
            t0.elapsed().as_secs_f64() * 1000.0
        };
        let zstd_ok = {
            let out = Command::new("zstd")
                .args(&["-d", "-q", "-c", "-f"])
                .arg(&zstd_path)
                .stdout(Stdio::piped())
                .output()
                .expect("zstd -d");
            out.stdout == data
        };
        let _ = std::fs::remove_file(&zstd_path);

        eprintln!(
            " nexus={:.2}x ({} B, {:.1}ms / {:.1}ms){}",
            data.len() as f64 / compressed.len().max(1) as f64,
            compressed.len(),
            c_ms, d_ms,
            if nexus_ok { "" } else { " ⚠️ MISMATCH" }
        );

        rows.push(Row {
            file: name,
            size: data.len(),
            nexus_size: compressed.len(),
            nexus_compress_ms: c_ms,
            nexus_decompress_ms: d_ms,
            nexus_ok,
            gzip_size,
            gzip_compress_ms: gzip_c_ms,
            gzip_decompress_ms: gzip_d_ms,
            gzip_ok,
            zstd_size,
            zstd_compress_ms: zstd_c_ms,
            zstd_decompress_ms: zstd_d_ms,
            zstd_ok,
        });
    }

    // Markdown table.
    println!("\n# NexusCompress v4 benchmark — honest comparison");
    println!("\n## Compression ratio (data size / compressed size)");
    println!("\n| File | Size | NexusCompress v4 | gzip -9 | zstd -19 | Nexus / zstd |");
    println!("|---|---:|---:|---:|---:|---:|");
    for r in &rows {
        let n_ratio = r.size as f64 / r.nexus_size.max(1) as f64;
        let g_ratio = r.size as f64 / r.gzip_size.max(1) as f64;
        let z_ratio = r.size as f64 / r.zstd_size.max(1) as f64;
        let gap = if z_ratio > 0.0 { n_ratio / z_ratio } else { 0.0 };
        println!(
            "| {} | {} B | **{:.2}x** ({} B) | {:.2}x ({} B) | {:.2}x ({} B) | {:.2}x |",
            r.file, r.size, n_ratio, r.nexus_size,
            g_ratio, r.gzip_size,
            z_ratio, r.zstd_size,
            gap,
        );
    }

    println!("\n## Compress time (ms, single-threaded)");
    println!("\n| File | NexusCompress v4 | gzip -9 | zstd -19 |");
    println!("|---|---:|---:|---:|");
    for r in &rows {
        println!("| {} | {:.1} | {:.1} | {:.1} |", r.file, r.nexus_compress_ms, r.gzip_compress_ms, r.zstd_compress_ms);
    }

    println!("\n## Decompress time (ms, single-threaded)");
    println!("\n| File | NexusCompress v4 | gzip -9 | zstd -19 |");
    println!("|---|---:|---:|---:|");
    for r in &rows {
        println!("| {} | {:.1} | {:.1} | {:.1} |", r.file, r.nexus_decompress_ms, r.gzip_decompress_ms, r.zstd_decompress_ms);
    }

    println!("\n## Roundtrip verification");
    let mut all_ok = true;
    for r in &rows {
        if !r.nexus_ok { println!("  ❌ NexusCompress {}: MISMATCH", r.file); all_ok = false; }
        if !r.gzip_ok { println!("  ❌ gzip {}: MISMATCH", r.file); all_ok = false; }
        if !r.zstd_ok { println!("  ❌ zstd {}: MISMATCH", r.file); all_ok = false; }
    }
    if all_ok {
        println!("  ✅ All compressors roundtrip OK on all corpus files.");
    }

    // Aggregate summary
    let total_size: usize = rows.iter().map(|r| r.size).sum();
    let total_nexus: usize = rows.iter().map(|r| r.nexus_size).sum();
    let total_gzip: usize = rows.iter().map(|r| r.gzip_size).sum();
    let total_zstd: usize = rows.iter().map(|r| r.zstd_size).sum();
    let total_nexus_ms: f64 = rows.iter().map(|r| r.nexus_compress_ms).sum();
    let total_gzip_ms: f64 = rows.iter().map(|r| r.gzip_compress_ms).sum();
    let total_zstd_ms: f64 = rows.iter().map(|r| r.zstd_compress_ms).sum();

    println!("\n## Aggregate (sum across all files)");
    println!("\n| Tool | Total Compressed | Aggregate Ratio | Compress ms |");
    println!("|---|---:|---:|---:|");
    for (name, total, ms) in [
        ("NexusCompress v4", total_nexus, total_nexus_ms),
        ("gzip -9", total_gzip, total_gzip_ms),
        ("zstd -19", total_zstd, total_zstd_ms),
    ] {
        let ratio = total_size as f64 / total.max(1) as f64;
        println!("| {} | {} B | {:.2}x | {:.1} |", name, total, ratio, ms);
    }
}
