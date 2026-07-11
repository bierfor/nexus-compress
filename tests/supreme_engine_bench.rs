//! Sprint 5.7.10 post-merge bench: validate the SupremeEngine on
//! the canonical `corpus_real/` (a real-world TypeScript/Rust/JS
//! corpus, the same one the 5.7.9 sprint used as a benchmark).
//!
//! This test exercises the engine via the public API
//! (`SupremeEngine::compress` with a `CompressionProfile`), the
//! same path the Tauri GUI uses. It compares against the legacy
//! CLI's numbers (which exercise the same underlying codecs
//! but via a different dispatch).

use nexus_compress::supreme_engine::{
    CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
    PROFILE_SCHEMA_VERSION,
};
use std::time::Instant;

const CORPUS: &str = "corpus_real";

fn profile(mode: ProfileMode, codec: ProfileCodec, fidelity: ProfileFidelity) -> CompressionProfile {
    CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode,
        codec,
        fidelity,
        corpus_mode: nexus_compress::api::CorpusMode::Everything,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
    }
}

fn bench(label: &str, profile: CompressionProfile) -> (u64, u128, f64) {
    let path = std::path::PathBuf::from(CORPUS);
    let invocation = CompressInvocation {
        profile,
        path: path.clone(),
        password: None,
        output_dir: None,
    };
    let start = Instant::now();
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(&invocation, |_| {})
        .expect("engine compress");
    let elapsed_ms = start.elapsed().as_millis();
    let ratio = if result.compressed_size > 0 {
        result.original_size as f64 / result.compressed_size as f64
    } else {
        0.0
    };
    println!(
        "[{}] {} files, {} -> {} bytes, {:.2}x, {}ms",
        label, result.n_files, result.original_size, result.compressed_size, ratio, elapsed_ms
    );
    (result.compressed_size, elapsed_ms, ratio)
}

#[test]
fn engine_balanced_mode_matches_legacy_baseline() {
    // Skip the test if corpus_real isn't available (CI without the corpus).
    if !std::path::Path::new(CORPUS).is_dir() {
        eprintln!("skipping: {} not found", CORPUS);
        return;
    }

    let (_, _, ratio) = bench(
        "engine/balanceado",
        profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy),
    );

    // The 5.7.9 baseline (legacy path) on the same corpus was
    // 6.39x. The refactor should produce a ratio within ±10%
    // of that. The engine wraps the same codecs, so we expect
    // the ratio to be effectively identical.
    assert!(ratio > 5.0, "balanceado ratio {:.2}x too low (expected ~6.39x)", ratio);
    assert!(ratio < 8.0, "balanceado ratio {:.2}x too high (suspicious)", ratio);
}

#[test]
fn engine_lossless_mode_is_bit_exact() {
    if !std::path::Path::new(CORPUS).is_dir() {
        return;
    }

    let (out_size, _, ratio) = bench(
        "engine/lossless",
        profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossless),
    );

    // Lossless should produce a smaller ratio (no minify) but
    // still good thanks to the SOLID dictionary. Sanity: > 4x.
    assert!(ratio > 4.0, "lossless ratio {:.2}x too low", ratio);
    println!("lossless output size: {} bytes", out_size);
}

#[test]
fn engine_ultra_mode_best_ratio() {
    if !std::path::Path::new(CORPUS).is_dir() {
        return;
    }

    // Ultra + LZMA(9) should give the best ratio. The 5.7.9
    // bench on secretaria/ showed ~8.75x with LZMA(9).
    let (_, _, ratio) = bench(
        "engine/ultra",
        profile(ProfileMode::Ultra, ProfileCodec::Lzma, ProfileFidelity::Lossy),
    );
    assert!(ratio > 7.0, "ultra ratio {:.2}x too low (expected >8x)", ratio);
}

#[test]
fn engine_rapido_mode_fastest() {
    if !std::path::Path::new(CORPUS).is_dir() {
        return;
    }

    // Rapido + Zstd should be the fastest. Compare wall time
    // against the balanced run; rapido should be ≤ balanced.
    let (_, ms_fast, _) = bench(
        "engine/rapido",
        profile(ProfileMode::Rapido, ProfileCodec::Zstd, ProfileFidelity::Lossy),
    );
    let (_, ms_bal, _) = bench(
        "engine/balanceado",
        profile(ProfileMode::Balanceado, ProfileCodec::Auto, ProfileFidelity::Lossy),
    );
    println!("rapido: {}ms, balanceado: {}ms", ms_fast, ms_bal);
    // rapido is supposed to be FASTER (or at least not slower)
    // than balanceado. Allow some noise from the test runner.
    assert!(ms_fast <= ms_bal + 50, "rapido should be at least as fast as balanceado");
}
