//! nexus CLI — `nexus c FILE` / `nexus d FILE`.
//!
//! Usage:
//!   nexus c <input> <output>           # compress a file
//!   nexus c --backend v5 --minify <in> <out>  # v5 LZMA + minify pre-filter
//!   nexus c --password P --recovery low <in> <out>  # v4 + AES-256-GCM + RS (NXE/NXR)
//!   nexus c <dir>  <output.nxar>        # compress a directory
//!   nexus d <input.nexus|nxar|nxe|nxr> <output>
//!   nexus bench                          # run corpus benchmark
//!
//! Compression backends (v4 default, v5 = LZMA via xz2, v6 = v5 + smart preprocessor):
//!   v4          — multi-stream LZ77 + rANS + dict codec (default)
//!   v5          — LZMA level 6 (balanced, fast)
//!   v5-min      — LZMA level 6 + minify pre-filter
//!   v5-extreme  — LZMA level 9 (max ratio)
//!   v6          — LZMA + swc AST minify (for .js/.ts/.tsx/.jsx) OR conservative minify
//!   v6-extreme  — same as v6 but LZMA level 9
//!
//! Encrypted + recovery (Sprint 5.7.2):
//!   --password P     enable AES-256-GCM encryption; output magic = NXE\0
//!                    or NXR\0 (if --recovery is also set)
//!   --recovery LVL   parity budget: off (default if --password set means
//!                    no parity → just encryption) | low (10%) | high (25%)
//!                    Default if --password is set WITHOUT --recovery:
//                    "low" (10% recovery — matches the design doc's
//                    "recovery on by default" choice).

use std::env;
use std::fs;
use std::path::Path;

use nexus_compress::encrypted::{
    compress_encrypted, decompress_encrypted, EncryptOptions, RecoveryLevel,
};
use nexus_compress::engine;
use nexus_compress::crypto::KdfPreset;

fn print_help() {
    eprintln!("nexus CLI — NexusCompress v4 + v5 LZMA + v6 AST-aware + v6-solid + NXE/NXR encrypted");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("    nexus c [OPTIONS] <input> <output>");
    eprintln!("    nexus d [OPTIONS] <input> <output>");
    eprintln!("    nexus bench");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("    --backend NAME  v4 | v5 | v5-min | v5-extreme | v6 | v6-extreme");
    eprintln!("                    (default: v4). Ignored if --password is set (encrypted");
    eprintln!("                    path always uses v4 + AES-256-GCM).");
    eprintln!("    --minify        apply the conservative minify pre-filter (v5-min)");
    eprintln!("    --solid         directory: build a SOLID v6 archive (NXS6, LOSSY)");
    eprintln!("                    cross-file LZMA dictionary, max ratio on source code");
    eprintln!("    --level N       LZMA level 0..9 (default 6, used by --solid and v5/v6)");
    eprintln!("    --codec K       SOLID v6 codec: auto | lzma | zstd. Default: auto.");
    eprintln!("                    auto: per-chunk codec flip based on entropy (#46 hybrid).");
    eprintln!("                    lzma: pure LZMA, no entropy flip (use for max ratio).");
    eprintln!("                    zstd: pure Zstd, no entropy flip (use for max speed).");
    eprintln!("    --lossless      SOLID v6: skip the per-extension preprocessor (Raw for all).");
    eprintln!("                    Archives are bit-exact reversible but ratio drops to ~1.5-2x.");
    eprintln!("                    Use for backups where integrity > compression.");
    eprintln!("    --raw-ext EXT[,..]  pin extensions to Raw (bit-exact). e.g. --raw-ext .json,.env");
    eprintln!("    --corpus MODE       directory walk mode: everything|source|minimal");
    eprintln!("                        (default: everything = no skip, full archive)");
    eprintln!("                    Overrides the default per-extension rule. --lossless implies all.");
    eprintln!("    --minify-ext EXT[,..]  pin extensions to Conservative minify. e.g. --minify-ext .md");
    eprintln!("    --skip-archive    skip archive files (.zip, .tar, .gz, .rar, ...) during the walk.");
    eprintln!("                    Useful to opt a file OUT of swc AST (when the default is .ts).");
    eprintln!("    --password P    encrypt with AES-256-GCM (Sprint 5.7.2). Argon2id KDF");
    eprintln!("                    with the Interactive preset (~100 ms on a modern desktop).");
    eprintln!("                    Output magic: NXE\\0 (no recovery) or NXR\\0 (with recovery).");
    eprintln!("    --recovery LVL  parity budget: off | low (10%) | high (25%). Default if");
    eprintln!("                    --password is set without --recovery: low. 7z-style UX.");
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
    eprintln!("    nexus c --password hunter2 out.nxe secret.txt  # encrypted only (NXE)");
    eprintln!("    nexus c --password hunter2 --recovery high big.iso out.nxr  # + RS");
    eprintln!("    nexus d out.nxs6 out_dir/            # auto-detect NXS6 / NXAR / v5");
    eprintln!("    nexus d --password hunter2 out.nxr out_dir/  # decrypt NXR with recovery");
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
    // Sprint 5.7.2 hotfix #47: explicit codec override. None =
    // keep the Auto behaviour (entropy flip per chunk). Some(codec)
    // = force that codec for the whole archive with no flip.
    let mut codec_override: Option<nexus_compress::solid_archive::Codec> = None;
    // Sprint 5.7.3 hotfix #49: --lossless. When true, every
    // file is fed to the codec as Preprocessor::Raw (no
    // minify, no swc AST, no entropy flip on extension). The
    // archive is bit-exact reversible.
    let mut lossless: bool = false;
    // Sprint 5.7.18: --skip-archive. Skip archive files
    // (`.zip`, `.tar`, `.gz`, `.rar`, …) during the walk.
    let mut skip_archive: bool = false;
    // Sprint 5.7.4 hotfix #50: --raw-ext / --minify-ext.
    // Per-extension override lists. Empty by default. When
    // populated, files whose extension matches the list are
    // forced to the corresponding preprocessor regardless
    // of the default rule.
    let mut raw_ext: Vec<String> = Vec::new();
    let mut minify_ext: Vec<String> = Vec::new();
    // Sprint 5.7.7 hotfix #55: --corpus MODE. Controls
    // the directory walk. Default is "everything" (no
    // skip) — the user wanted control, and the previous
    // hardcoded skip-list silently dropped files.
    // Accepts: everything|source|minimal (with aliases
    // all|full|src|code|min).
    let mut corpus_mode: Option<nexus_compress::api::CorpusMode> = None;
    let mut password: Option<String> = None;
    let mut recovery: Option<String> = None;
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
            // Sprint 5.7.2 hotfix #47: accept both `--codec
            // lzma` (separate arg) and `--codec=lzma` (single
            // arg). The split is what makes the flag usable from
            // the GUI, which builds the arg list as a flat
            // string array of pre-split tokens.
            arg if arg == "--codec" || arg.starts_with("--codec=") => {
                let v = if let Some(eq) = a.strip_prefix("--codec=") {
                    eq.to_string()
                } else {
                    iter.next().cloned().unwrap_or_default()
                };
                codec_override = match v.to_ascii_lowercase().as_str() {
                    "lzma" | "lzma2" => Some(nexus_compress::solid_archive::Codec::Lzma),
                    "zstd" => Some(nexus_compress::solid_archive::Codec::Zstd),
                    "auto" | "" => None,
                    other => {
                        eprintln!("error: --codec must be auto|lzma|zstd, got '{}'", other);
                        std::process::exit(2);
                    }
                };
            }
            "--password" => {
                password = iter.next().cloned();
            }
            "--recovery" => {
                recovery = iter.next().cloned();
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            "--lossless" | "--no-minify" => {
                lossless = true;
            }
            // Sprint 5.7.18: --skip-archive flag. When set, the
            // walker skips archive files (`.zip`, `.tar`,
            // `.gz`, `.rar`, …) entirely. Useful for backing
            // up source code without including 200 MB of
            // release binaries you accidentally left in the
            // repo. Default off to preserve current behavior.
            "--skip-archive" => {
                skip_archive = true;
            }
            "--raw-ext" => {
                if let Some(v) = iter.next() {
                    raw_ext.extend(
                        v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                    );
                }
            }
            "--minify-ext" => {
                if let Some(v) = iter.next() {
                    minify_ext.extend(
                        v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                    );
                }
            }
            // Sprint 5.7.7 hotfix #55: --corpus MODE.
            arg if arg == "--corpus" || arg.starts_with("--corpus=") => {
                let v = if let Some(eq) = a.strip_prefix("--corpus=") {
                    eq.to_string()
                } else {
                    iter.next().cloned().unwrap_or_default()
                };
                match v.parse::<nexus_compress::api::CorpusMode>() {
                    Ok(m) => corpus_mode = Some(m),
                    Err(e) => {
                        eprintln!("error: {}", e);
                        std::process::exit(2);
                    }
                }
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
    // Sprint 5.7.7 hotfix #55: set NEXUS_CORPUS_MODE
    // before the walk. The walker reads this env var
    // in collect_paths(). Default (None) is Everything.
    if let Some(m) = corpus_mode {
        std::env::set_var("NEXUS_CORPUS_MODE", m.as_str());
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
                    // Sprint 5.7.2 hotfix #47: translate the CLI
                    // `--codec` flag into a `CompressionLevel` so
                    // the encoder skips the entropy flip when
                    // the user pinned a specific codec.
                    let solid_level: nexus_compress::solid_archive::CompressionLevel =
                        match codec_override {
                            None => nexus_compress::solid_archive::CompressionLevel::Auto,
                            Some(nexus_compress::solid_archive::Codec::Lzma) => {
                                nexus_compress::solid_archive::CompressionLevel::Lzma(level)
                            }
                            Some(nexus_compress::solid_archive::Codec::Zstd) => {
                                nexus_compress::solid_archive::CompressionLevel::Zstd(3)
                            }
                        };
                    let files = walk_dir_with_skip(input).expect("walk dir");
                    if files.is_empty() {
                        eprintln!("error: directory has no files: {}", positional[1]);
                        std::process::exit(2);
                    }
                    let original_total: usize = files.iter().map(|(_, b)| b.len()).sum();
                    let t0 = std::time::Instant::now();
                    // Sprint 5.7.3 hotfix #49: --lossless
                    // bypasses the per-extension preprocessor.
                    // We dispatch to the dedicated function so
                    // the call site stays a single line; the
                    // alternative (passing a bool down through
                    // `compress`) would touch the public API
                    // for every existing caller.
                    let archive = if lossless {
                        // When --lossless is set, the per-extension
                        // overrides are still honoured: a file
                        // pinned to Conservative would still be
                        // Conservative. But the user picked
                        // --lossless precisely to be safe, so we
                        // force ALL files to Raw (overrides are
                        // ignored under --lossless). This matches
                        // the user's intent: "no minify at all".
                        nexus_compress::solid_archive::compress_with_progress_lossless(
                            &files,
                            solid_level,
                            |_, _, _| {},
                        )
                    } else {
                        // Sprint 5.7.4 hotfix #50: per-extension
                        // overrides. When the user passed
                        // --raw-ext or --minify-ext, route through
                        // the full-control entry point so the
                        // overrides actually take effect.
                        if !raw_ext.is_empty() || !minify_ext.is_empty() {
                            let overrides = nexus_compress::solid_archive::PreprocessorOverrides::new(
                                raw_ext.clone(),
                                minify_ext.clone(),
                            );
                            nexus_compress::solid_archive::compress_with_progress_full_public(
                                &files,
                                solid_level,
                                false,
                                &overrides,
                                |_, _, _| {},
                            )
                        } else {
                            nexus_compress::solid_archive::compress(&files, solid_level)
                        }
                    }
                    .expect("solid compress");
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    fs::write(output, &archive).expect("write output");
                    let ratio = original_total as f64 / archive.len().max(1) as f64;
                    let codec_label = match codec_override {
                        None => "AUTO",
                        Some(nexus_compress::solid_archive::Codec::Lzma) => "LZMA",
                        Some(nexus_compress::solid_archive::Codec::Zstd) => "ZSTD",
                    };
                    // Sprint 5.7.9: AUTO now resolves to Zstd
                    // (the fast default), so its suffix should
                    // be " Zstd 3" not " LZMA <level>". The
                    // level arg in CLI is only used when the
                    // user explicitly picked LZMA via --codec
                    // lzma; for Auto/Zstd, zstd uses its own
                    // internal preset (3 by default).
                    let level_suffix = match codec_override {
                        Some(nexus_compress::solid_archive::Codec::Lzma) => {
                            format!(" {}", level)
                        }
                        Some(nexus_compress::solid_archive::Codec::Zstd) => String::new(),
                        None => String::new(), // AUTO → Zstd (5.7.9)
                    };
                    // Sprint 5.7.3 hotfix #49: append the
                    // lossless marker so the user can see the
                    // archive is bit-exact reversible.
                    let lossless_suffix = if lossless { " LOSSLESS" } else { "" };
                    eprintln!(
                        "{} -> {} (SOLID v6 {}{}{}, {:.2}x, {} files, {} bytes -> {} bytes, {:.0} ms)",
                        positional[1],
                        output,
                        codec_label,
                        level_suffix,
                        lossless_suffix,
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
                // ── Encrypted path (Sprint 5.7.2) ────────────
                //
                // If `--password` is set, route through
                // `encrypted::compress_encrypted` which wraps
                // the v4 codec (multi-stream LZ77+rANS) with
                // AES-256-GCM per shard + optional Reed-Solomon
                // parity shards. Output magic becomes
                // NXE\0 (no recovery) or NXR\0 (with recovery).
                //
                // Default recovery level is "low" (10% parity)
                // when --password is set WITHOUT --recovery —
                // matches the design-doc decision that recovery
                // is on-by-default (the user has to opt OUT, not
                // opt IN).
                if let Some(pwd) = password.as_deref() {
                    let recovery_level = match recovery.as_deref() {
                        Some(s) => RecoveryLevel::from_str(s).unwrap_or_else(|e| {
                            eprintln!("{}", e);
                            std::process::exit(2);
                        }),
                        None => RecoveryLevel::Low,
                    };
                    let opts = EncryptOptions {
                        password: pwd.as_bytes(),
                        recovery: recovery_level,
                        preset: KdfPreset::Interactive,
                    };
                    let out = compress_encrypted(&bytes, &opts, |_ev| {}).expect("compress encrypted");
                    fs::write(output, &out).expect("write output");
                    let ratio = bytes.len() as f64 / out.len().max(1) as f64;
                    let label = match recovery_level {
                        RecoveryLevel::Off => "NXE",
                        RecoveryLevel::Low => "NXR/low",
                        RecoveryLevel::High => "NXR/high",
                    };
                    eprintln!(
                        "{} -> {} ({:.2}x, encrypted+{})",
                        positional[1], output, ratio, label,
                    );
                } else {
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
                        positional[1], output, ratio, backend_name,
                    );
                }
            }
        }
        "d" | "decompress" => {
            if positional.len() < 3 {
                eprintln!("usage: nexus d [OPTIONS] <input> <output>");
                std::process::exit(2);
            }
            let input = fs::read(&positional[1]).expect("read input");
            // Auto-detect by magic.
            //
            // Order matters: check the encrypted magics FIRST
            // because both NXE\0 and NXR\0 start with 'N' and
            // would not collide with the others (NXS\0, NXS6,
            // NXAR), but we want to fail fast on the encrypted
            // path with a "password required" message before
            // trying the wrong decoder.
            if input.len() >= 4
                && (&input[0..4] == b"NXE\0" || &input[0..4] == b"NXR\0")
            {
                // Encrypted archive (Sprint 5.7.2). Requires
                // `--password` to decrypt.
                let Some(pwd) = password.as_deref() else {
                    eprintln!(
                        "error: {} is an encrypted archive (NXE/NXR magic); \
                         pass --password to decrypt",
                        positional[1]
                    );
                    std::process::exit(2);
                };
                let out = decompress_encrypted(&input, pwd.as_bytes(), |_ev| {})
                    .expect("decompress encrypted");
                fs::write(&positional[2], &out).expect("write output");
                eprintln!(
                    "{} -> {} ({} bytes, decrypted)",
                    positional[1],
                    positional[2],
                    out.len()
                );
            } else if input.len() >= 5 && &input[0..5] == nexus_compress::solid_archive::MAGIC {
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
                eprintln!(
                    "{} -> {} ({} bytes)",
                    positional[1],
                    positional[2],
                    out.len()
                );
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
/// Walk a directory recursively and return `(relative_path, bytes)`
/// for every regular file, sorted by relative path for determinism.
///
/// **Sprint 5.7.10-B:** thin wrapper around [`crate::walker::walk`],
/// the single source of truth for corpus walking + skip-list
/// filtering. The 110-line local walker (with its `CorpusModeLike`
/// enum that mirrored `api::CorpusMode`) was deleted and the CLI
/// now shares the GUI's walker code path. The `Thumbs.db` entry
/// that was missing from the CLI's local skip-list is now picked
/// up automatically.
fn walk_dir_with_skip(root: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mode = std::env::var("NEXUS_CORPUS_MODE")
        .ok()
        .and_then(|s| s.parse::<nexus_compress::api::CorpusMode>().ok())
        .unwrap_or_default();
    // Sprint 5.7.18: --skip-archive. When the flag is set, the
    // walker filters out archive files (`.zip`, `.tar`, `.gz`,
    // `.rar`, …) so they don't end up in the output. The flag
    // is read from the same env-var convention as
    // `NEXUS_CORPUS_MODE` so the engine and CLI stay in sync.
    let skip_archive = std::env::var("NEXUS_SKIP_ARCHIVE")
        .ok()
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let result = nexus_compress::walker::walk(root, mode, skip_archive)?;
    if result.skipped_bytes > 0 {
        eprintln!(
            "[WALK-SKIP] total skipped: {} MiB of dev cache / build artifacts",
            result.skipped_bytes / (1024 * 1024)
        );
    }
    Ok(result.files)
}

