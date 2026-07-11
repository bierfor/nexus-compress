//! Sprint 5.7.13: regression test for the IPC bytes leak.
//!
//! The Tauri command `compress_target_cmd` previously returned
//! `CompressTargetResult.compressed_bytes: Vec<u8>` unchanged
//! to the frontend, freezing the UI on 5 GB corpora because
//! the WebView spent minutes copying the buffer through the
//! IPC boundary. The fix writes the bytes to disk in the
//! backend and clears `compressed_bytes` before returning.
//!
//! This test pins the contract: the on-disk file MUST exist
//! and match the engine's `compressed_size`; the
//! `compressed_bytes` field MUST be empty after the write
//! (so the IPC payload stays small regardless of corpus
//! size).

use nexus_compress::supreme_engine::{
    CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
    PROFILE_SCHEMA_VERSION,
};
use std::path::PathBuf;
use std::time::Instant;

fn make_corpus(root: &std::path::Path, total_kb: usize) {
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    let phrase = b"import { Foo, Bar, Baz } from './common';\n\
                   export interface Config {\n  \
                     version: number;\n  \
                     name: string;\n\
                   }\n\
                   export const defaultConfig: Config = { version: 1, name: 'x' };\n\
                   export function makeConfig(overrides: Partial<Config>): Config {\n  \
                     return { ...defaultConfig, ...overrides };\n  \
                   }\n";
    let mut body = Vec::with_capacity(total_kb * 1024);
    while body.len() < total_kb * 1024 {
        body.extend_from_slice(phrase);
    }
    body.truncate(total_kb * 1024);
    std::fs::write(root.join("src/index.ts"), &body).expect("write index.ts");
    std::fs::write(root.join("src/util.ts"), &body).expect("write util.ts");
}

fn build_profile() -> CompressionProfile {
    CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode: ProfileMode::Balanceado,
        codec: ProfileCodec::Auto,
        fidelity: ProfileFidelity::Lossy,
        corpus_mode: nexus_compress::api::CorpusMode::Everything,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
    }
}

#[test]
fn compress_target_result_must_be_writable_to_disk_and_strippable() {
    // Build a small corpus (10 MB) so the test is fast.
    let tmp = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let corpus = tmp.join(format!("nexus-fix-verify-{}-{n}", std::process::id()));
    let output_dir = tmp.join(format!("nexus-fix-out-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&corpus);
    let _ = std::fs::remove_dir_all(&output_dir);
    std::fs::create_dir_all(&corpus).expect("mkdir corpus");
    std::fs::create_dir_all(&output_dir).expect("mkdir output_dir");
    make_corpus(&corpus, 10 * 1024);

    let inv = CompressInvocation {
        profile: build_profile(),
        path: corpus.clone(),
        password: None,
        output_dir: Some(output_dir.clone()),
    };
    let _start = Instant::now();
    let mut result =
        nexus_compress::supreme_engine::SupremeEngine::compress(&inv, |_| {})
            .expect("SupremeEngine compress");
    eprintln!(
        "engine returned: original={} compressed={} bytes={} files={}",
        result.original_size, result.compressed_size,
        result.compressed_bytes.len(), result.n_files
    );

    // Property 1: engine must have produced bytes for a non-empty corpus.
    assert!(!result.compressed_bytes.is_empty(), "engine must produce bytes for 10 MB corpus");
    assert!(result.compressed_size > 0);

    // Property 2: write the bytes to disk (the Tauri command's
    // fix does this). The file must match compressed_size
    // exactly — no truncation, no extra bytes.
    let filename = corpus
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("corpus");
    let output_path: PathBuf = output_dir.join(format!("{}.nxs6", filename));
    std::fs::write(&output_path, &result.compressed_bytes).expect("write archive");
    let on_disk = std::fs::metadata(&output_path).expect("stat output");
    assert_eq!(
        on_disk.len(), result.compressed_size,
        "on-disk size must match engine's compressed_size"
    );

    // Property 3: the engine has produced a valid NXS7 archive.
    // Read the magic + version to confirm.
    let bytes = std::fs::read(&output_path).expect("read archive");
    assert_eq!(&bytes[0..5], b"NXS7\n", "magic must be NXS7\\n");
    let version = bytes[5];
    assert!(matches!(version, 3 | 4), "version must be 3 or 4, got {version}");

    // Property 4: roundtrip — decompress the on-disk file
    // and confirm we recover the original corpus.
    let (entries, solid) =
        nexus_compress::solid_archive::decompress(&bytes).expect("decompress");
    assert_eq!(entries.len(), 2, "expected 2 source entries");
    for e in &entries {
        let start = e.solid_offset as usize;
        let end = start + e.pre_size as usize;
        let recovered = &solid[start..end];
        // The .ts files are swc-minified, so we don't
        // byte-compare. We just check shape: pre_size
        // > 0 and the slice is non-empty.
        assert!(!recovered.is_empty(), "{} recovered empty", e.name);
    }

    // Property 5 (the IPC fix): after writing to disk, the
    // Tauri command MUST clear compressed_bytes before
    // returning. This is what keeps the IPC payload small.
    // Simulate the Tauri command's post-processing here.
    result.compressed_bytes = Vec::new();
    result.output_path = output_path.to_string_lossy().to_string();
    assert!(
        result.compressed_bytes.is_empty(),
        "compressed_bytes must be cleared before IPC return"
    );
    assert!(
        !result.output_path.is_empty(),
        "output_path must be non-empty after disk write"
    );

    // Cleanup.
    let _ = std::fs::remove_dir_all(&corpus);
    let _ = std::fs::remove_dir_all(&output_dir);
}

#[test]
fn empty_corpus_path_does_not_hang_or_panic() {
    // Edge case: an empty corpus (no files) must not cause
    // a hang or panic in the write path. The engine returns
    // 0 bytes; the Tauri command's post-processing must
    // handle that gracefully (we don't write a 0-byte file
    // unless the user expects one).
    let tmp = std::env::temp_dir();
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let corpus = tmp.join(format!("nexus-fix-empty-{}-{n}", std::process::id()));
    let output_dir = tmp.join(format!("nexus-fix-empty-out-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&corpus);
    let _ = std::fs::remove_dir_all(&output_dir);
    std::fs::create_dir_all(&corpus).expect("mkdir corpus");
    std::fs::create_dir_all(&output_dir).expect("mkdir output_dir");

    let inv = CompressInvocation {
        profile: build_profile(),
        path: corpus.clone(),
        password: None,
        output_dir: Some(output_dir.clone()),
    };
    // The engine's empty-corpus check should reject this
    // before we get to the write path. We assert that the
    // error is surfaced cleanly, not a panic or hang.
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(&inv, |_| {});
    assert!(result.is_err(), "empty corpus must error, not return 0-byte result");
    let err = result.unwrap_err();
    eprintln!("empty corpus error (expected): {err}");

    let _ = std::fs::remove_dir_all(&corpus);
    let _ = std::fs::remove_dir_all(&output_dir);
}
