//! Phase 3 benchmark: SOLID v6 (single LZMA stream across whole corpus)
//! vs per-file v6 (independent LZMA per file) vs 7z (LZMA2 -mx=9)
//! on a directory of real JS/TS source code.
//!
//! Walks `./corpus_real/`, totals the original bytes, then:
//!   1. Compresses as a SOLID v6 archive (level 6 + level 9)
//!   2. Compresses each file independently with v6 (per-file LZMA)
//!   3. Runs `7z a -mx=9` on the directory
//!
//! Reports the aggregate ratio and the SOLID win (the per-file
//! dictionary reset is what solid avoids).
//!
//! Run: `cargo run --release --bin bench_solid`

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use nexus_compress::engine;
use nexus_compress::solid_archive;

fn main() {
    let corpus_dir = Path::new("corpus_real");
    if !corpus_dir.exists() {
        eprintln!("missing ./corpus_real/");
        std::process::exit(2);
    }

    eprintln!("[bench_solid] SOLID v6 vs per-file v6 vs 7z on a directory of real JS/TS\n");

    // Walk the directory.
    let files: Vec<(String, Vec<u8>)> = match walk_dir(corpus_dir) {
        Ok(f) if !f.is_empty() => f,
        _ => {
            eprintln!("corpus_real/ is empty or unreadable");
            std::process::exit(2);
        }
    };
    let total_original: usize = files.iter().map(|(_, b)| b.len()).sum();
    eprintln!(
        "  Corpus: {} files, {} B total ({:.2} MB)\n",
        files.len(),
        total_original,
        total_original as f64 / 1024.0 / 1024.0
    );

    // 1. SOLID v6 (level 6, balanced)
    let t0 = Instant::now();
    let solid_l6 = solid_archive::compress(&files, 6).expect("solid l6");
    let solid_l6_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // 2. SOLID v6 (level 9, max)
    let t0 = Instant::now();
    let solid_l9 = solid_archive::compress(&files, 9).expect("solid l9");
    let solid_l9_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // 3. Per-file v6 (each file compressed independently with v6)
    let t0 = Instant::now();
    let mut per_file_total: usize = 0;
    let mut per_file_max: usize = 0;
    for (name, bytes) in &files {
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str());
        let comp = engine::compress_v6(bytes, ext, 6);
        per_file_total += comp.len();
        if comp.len() > per_file_max {
            per_file_max = comp.len();
        }
    }
    let per_file_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // 4. 7z LZMA2 -mx=9 on the directory
    let sevenz_bin = if Command::new("7z").arg("--help").output().is_ok() {
        "7z"
    } else if Command::new("7za").arg("--help").output().is_ok() {
        "7za"
    } else {
        ""
    };
    let (sevenz_size, sevenz_ms) = if sevenz_bin.is_empty() {
        eprintln!("  (no 7z binary found — skipping 7z comparison)");
        (0, 0.0)
    } else {
        let archive = Path::new("/tmp/bench_solid.7z");
        let t0 = Instant::now();
        let out = Command::new(sevenz_bin)
            .args(&["a", "-mx=9", "-bb0", "-bso0"])
            .arg(archive)
            .arg(corpus_dir)
            .output()
            .expect("7z");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let ok = out.status.success();
        let sz = std::fs::metadata(archive).map(|m| m.len() as usize).unwrap_or(0);
        let _ = std::fs::remove_file(archive);
        if !ok {
            eprintln!("  (7z failed — skipping)");
            (0, 0.0)
        } else {
            (sz, ms)
        }
    };

    // Markdown table
    println!("\n# SOLID v6 vs per-file v6 vs 7z — directory benchmark\n");
    println!("Corpus: `./corpus_real/` ({} files, {} B / {:.2} MB)\n",
        files.len(), total_original, total_original as f64 / 1024.0 / 1024.0);
    println!("| Backend | Compressed | Ratio | % of 7z | Time |");
    println!("|---|---:|---:|---:|---:|");

    let total_sevenz = sevenz_size;
    let sevenz_ratio = if total_sevenz > 0 {
        total_original as f64 / total_sevenz as f64
    } else {
        0.0
    };
    let print_row = |name: &str, size: usize, ms: f64| {
        if size == 0 {
            return;
        }
        let r = total_original as f64 / size as f64;
        let pct = if total_sevenz > 0 {
            format!("{:.0}%", (r / sevenz_ratio) * 100.0)
        } else {
            "n/a".to_string()
        };
        println!(
            "| {} | {} B | **{:.2}x** | {} | {:.0} ms |",
            name,
            size,
            r,
            pct,
            ms,
        );
    };
    print_row("Per-file v6 (LZMA per file, no cross-file dict)", per_file_total, per_file_ms);
    print_row("**SOLID v6 (LZMA -6, one stream over whole corpus)**", solid_l6.len(), solid_l6_ms);
    print_row("**SOLID v6 extreme (LZMA -9)**", solid_l9.len(), solid_l9_ms);
    if total_sevenz > 0 {
        print_row("7z -mx=9 (LZMA2, baseline)", sevenz_size, sevenz_ms);
    }

    // Verdict
    println!("\n## Verdict\n");
    if total_sevenz > 0 && per_file_total > 0 && solid_l6.len() > 0 {
        let per_file_ratio = total_original as f64 / per_file_total as f64;
        let solid_ratio = total_original as f64 / solid_l6.len() as f64;
        let solid9_ratio = total_original as f64 / solid_l9.len() as f64;
        let sevenz_ratio = total_original as f64 / total_sevenz as f64;
        let solid_vs_perfile = solid_ratio - per_file_ratio;
        let solid_vs_7z = (solid_ratio / sevenz_ratio) * 100.0;
        let solid9_vs_7z = (solid9_ratio / sevenz_ratio) * 100.0;
        println!(
            "* Per-file v6: **{:.2}x** ({:.0}% of 7z)",
            per_file_ratio,
            (per_file_ratio / sevenz_ratio) * 100.0,
        );
        println!(
            "* SOLID v6 (LZMA -6): **{:.2}x** ({:.0}% of 7z)",
            solid_ratio, solid_vs_7z,
        );
        println!(
            "* SOLID v6 (LZMA -9): **{:.2}x** ({:.0}% of 7z)",
            solid9_ratio, solid9_vs_7z,
        );
        println!(
            "* **Solid win over per-file: +{:.2}x** ({:.1}% better) — the LZMA dictionary now spans the whole corpus.",
            solid_vs_perfile,
            (solid_vs_perfile / per_file_ratio) * 100.0,
        );
        println!();
        if solid_vs_7z > 100.0 {
            println!("* **SOLID v6 BEATS 7z** by {:.0}% on this corpus.", solid_vs_7z - 100.0);
        } else {
            println!("* 7z is still ahead by {:.0}%.", 100.0 - solid_vs_7z);
        }
    } else {
        println!("* (7z not available — only per-file vs solid comparison)");
        if per_file_total > 0 && solid_l6.len() > 0 {
            let solid_ratio = total_original as f64 / solid_l6.len() as f64;
            let per_file_ratio = total_original as f64 / per_file_total as f64;
            println!(
                "* Solid win over per-file: +{:.2}x ({:.1}% better)",
                solid_ratio - per_file_ratio,
                ((solid_ratio - per_file_ratio) / per_file_ratio) * 100.0,
            );
        }
    }

    // Lossy contract reminder
    println!("\n### Honest contract\n");
    println!("SOLID v6 is **LOSSY**. Decompression yields the *minified* version of each file");
    println!("(swc AST minify for `.js/.ts/.tsx`, conservative text minify for the rest).");
    println!("The original source — comments, formatting, identifier names — is NOT recoverable.");
    println!("Use the v4 `.nxar` (per-file lossless) when roundtrip fidelity matters.");
}

fn walk_dir(root: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> std::io::Result<()> {
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
