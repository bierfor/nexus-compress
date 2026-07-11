//! Sprint 5.7.10-D integration test: verify the Tauri command
//! can parse the new nested `profile: { ... }` shape that the
//! frontend sends in 5.7.10-D. This exercises the same
//! `CompressionProfile::from_json` entry point that
//! `build_profile_from_legacy_req` uses when it detects a
//! nested profile in the request.

use nexus_compress::supreme_engine::{
    resolve_plan, CompressInvocation, CompressionProfile, PlanBackend,
};
use std::path::PathBuf;

/// The exact JSON the frontend sends in 5.7.10-D (mirrors
/// `CompressView.tsx` CompressView's `tauriInvoke("compress_target_cmd", ...)`).
const NESTED_REQUEST: &str = r#"{
    "path": "/tmp/some-file.txt",
    "output_dir": null,
    "profile": {
        "schemaVersion": 1,
        "mode": "balanceado",
        "codec": "auto",
        "fidelity": "lossy",
        "corpusMode": "source",
        "rawExtensions": ["json", "toml"],
        "minifyExtensions": ["md"],
        "encrypt": false,
        "recoveryLevel": "low"
    },
    "password": null,
    "profile_id": "balanced"
}"#;

#[test]
fn nested_profile_parses_and_resolves_to_v6_solid() {
    let value: serde_json::Value =
        serde_json::from_str(NESTED_REQUEST).expect("valid JSON");
    let profile_value = value
        .get("profile")
        .expect("frontend sent the nested profile");
    let profile =
        CompressionProfile::from_json(profile_value).expect("profile parses");

    assert_eq!(profile.schema_version, 1);
    assert_eq!(profile.mode, nexus_compress::supreme_engine::ProfileMode::Balanceado);
    assert_eq!(profile.codec, nexus_compress::supreme_engine::ProfileCodec::Auto);
    assert_eq!(profile.fidelity, nexus_compress::supreme_engine::ProfileFidelity::Lossy);
    assert_eq!(
        profile.corpus_mode,
        nexus_compress::api::CorpusMode::Source
    );
    assert_eq!(profile.raw_extensions, vec!["json", "toml"]);
    assert_eq!(profile.minify_extensions, vec!["md"]);
    assert!(!profile.encrypt);

    // Resolve the plan. With is_dir=true, the engine picks
    // V6Solid (single LZMA stream over the corpus).
    let inv = CompressInvocation {
        profile: profile.clone(),
        path: PathBuf::from("/tmp/some-dir"),
        password: None,
        output_dir: None,
    };
    let plan = resolve_plan(&inv, true);
    assert_eq!(plan.backend, PlanBackend::V6Solid);
}

#[test]
fn nested_profile_with_encrypt_resolves_to_v4_encrypted() {
    let value = serde_json::json!({
        "profile": {
            "schemaVersion": 1,
            "mode": "balanceado",
            "codec": "auto",
            "fidelity": "lossy",
            "corpusMode": "everything",
            "rawExtensions": [],
            "minifyExtensions": [],
            "encrypt": true,
            "recoveryLevel": "high"
        }
    });
    let profile = CompressionProfile::from_json(&value["profile"]).unwrap();
    let inv = CompressInvocation {
        profile,
        path: PathBuf::from("/tmp"),
        password: Some(b"hunter2".to_vec()),
        output_dir: None,
    };
    let plan = resolve_plan(&inv, true);
    assert_eq!(plan.backend, PlanBackend::V4Encrypted);
}

#[test]
fn nested_profile_with_ultra_resolves_to_lzma9() {
    let value = serde_json::json!({
        "profile": {
            "schemaVersion": 1,
            "mode": "ultra",
            "codec": "auto",
            "fidelity": "lossy",
            "corpusMode": "everything",
            "rawExtensions": [],
            "minifyExtensions": [],
            "encrypt": false,
            "recoveryLevel": "low"
        }
    });
    let profile = CompressionProfile::from_json(&value["profile"]).unwrap();
    let inv = CompressInvocation {
        profile,
        path: PathBuf::from("/tmp"),
        password: None,
        output_dir: None,
    };
    let plan = resolve_plan(&inv, true);
    // Ultra + auto = LZMA(9) per the resolution rules
    assert_eq!(plan.codec_level, nexus_compress::solid_archive::CompressionLevel::Lzma(9));
}
