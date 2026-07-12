//! Sprint 5.7.10 post-merge diagnose: investigate why the engine
//! is producing a 1.43x ratio on FlowNow (which should be 5-10x
//! since it's mostly text: 47k .py + 47k .pyc + 11k .h files).
//!
//! Skip if FlowNow isn't present (CI without the corpus).

use nexus_compress::supreme_engine::{
    CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
    PROFILE_SCHEMA_VERSION,
};
use std::time::Instant;

const FLOWNOW: &str = "/Users/bierhffor/Documents/FlowNow";

fn profile(mode: ProfileMode, codec: ProfileCodec, fidelity: ProfileFidelity) -> CompressionProfile {
    profile_with_mode(mode, codec, fidelity, nexus_compress::api::CorpusMode::Everything)
}

fn profile_with_mode(
    mode: ProfileMode,
    codec: ProfileCodec,
    fidelity: ProfileFidelity,
    corpus_mode: nexus_compress::api::CorpusMode,
) -> CompressionProfile {
    CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode,
        codec,
        fidelity,
        corpus_mode,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
        skip_archive: false,
    }
}

fn corpus_breakdown_to_string(b: &nexus_compress::api::CorpusBreakdown) -> String {
    let total = b.source_bytes + b.build_artifact_bytes + b.other_bytes;
    if total == 0 {
        return "  (empty)".to_string();
    }
    let pct = |bytes: u64| -> String {
        format!("{} ({:.1}%)", bytes, 100.0 * bytes as f64 / total as f64)
    };
    format!(
        "  source:        {} files, {}\n  build_artifact: {} files, {}\n  other:         {} files, {}",
        b.source_files,
        pct(b.source_bytes),
        b.build_artifact_files,
        pct(b.build_artifact_bytes),
        b.other_files,
        pct(b.other_bytes)
    )
}

#[test]
fn diagnose_flownow_compression_ratio() {
    if !std::path::Path::new(FLOWNOW).is_dir() {
        eprintln!("skipping: {} not found", FLOWNOW);
        return;
    }

    let path = std::path::PathBuf::from(FLOWNOW);

    // Try a few profiles to see which gives the best ratio.
    for (label, prof) in [
        (
            "Everything/Balanceado/Auto/Lossy",
            profile_with_mode(
                ProfileMode::Balanceado,
                ProfileCodec::Auto,
                ProfileFidelity::Lossy,
                nexus_compress::api::CorpusMode::Everything,
            ),
        ),
        (
            "Source/Balanceado/Auto/Lossy",
            profile_with_mode(
                ProfileMode::Balanceado,
                ProfileCodec::Auto,
                ProfileFidelity::Lossy,
                nexus_compress::api::CorpusMode::Source,
            ),
        ),
        (
            "Source/Ultra/LZMA-9/Lossy",
            profile_with_mode(
                ProfileMode::Ultra,
                ProfileCodec::Lzma,
                ProfileFidelity::Lossy,
                nexus_compress::api::CorpusMode::Source,
            ),
        ),
    ] {
        let invocation = CompressInvocation {
            profile: prof,
            path: path.clone(),
            password: None,
            output_dir: None,
        };
        eprintln!("\n=== Engine: FlowNow ({} mode) ===", label);
        let start = Instant::now();
        let result =
            nexus_compress::supreme_engine::SupremeEngine::compress(&invocation, |_| {})
                .expect("engine compress");
        let elapsed_ms = start.elapsed().as_millis();
        let ratio = if result.compressed_size > 0 {
            result.original_size as f64 / result.compressed_size as f64
        } else {
            0.0
        };
        eprintln!(
            "input:  {} bytes ({:.1} MiB)\noutput: {} bytes ({:.1} MiB)\nratio:  {:.2}x\nfiles:  {}\nskipped: {} bytes\nelapsed: {}ms",
            result.original_size,
            result.original_size as f64 / (1024.0 * 1024.0),
            result.compressed_size,
            result.compressed_size as f64 / (1024.0 * 1024.0),
            ratio,
            result.n_files,
            result.skipped_bytes,
            elapsed_ms,
        );
        assert!(result.compressed_size > 0);
        // Pause between runs for thermal recovery
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
