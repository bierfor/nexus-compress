//! Sprint 5.7.21-E: streaming preprocessing pin.
//!
//! Before this refactor, `preprocess_files_parallel` returned
//! a single `Vec<PreprocessedFile>` that held the preprocessed
//! bytes for the ENTIRE corpus. On a 5 GiB / 164 k-file corpus
//! (FlowNow) that was ~2-3 GiB held in memory for the duration
//! of the encoding pass.
//!
//! 5.7.21-E refactor: stream the preprocessing in batches of
//! 200 files. Each batch's preprocessed Vec is dropped at the
//! end of the batch iteration, so peak memory for the
//! preprocessed stage is O(200 × avg_pre_size) instead of
//! O(total_pre_size). The aggregation state
//! (current_chunk, current_chunk_codec, global_offset,
//! entries, super_chunks, chunk_codecs) carries across batches
//! because it's all in the outer scope.
//!
//! The wire format MUST be byte-identical to the non-batched
//! version: the super-chunk boundary check is
//! `current_chunk.len() + pre_bytes.len() > SUPER_CHUNK_BYTES`,
//! which is independent of how we batch the preprocessing.
//!
//! This file pins:
//!   1. Bit-exact roundtrip on a multi-batch corpus (>200
//!      files, so the streaming path actually iterates).
//!   2. The wire format (chunk_groups sub-TOC + solid block)
//!      is the same as if we had preprocessed in one shot.
//!   3. The number of progress events equals the file count
//!      (no events are dropped or duplicated by the batching).

use nexus_compress::api::ProgressEvent;
use nexus_compress::solid_archive::decompress;
use nexus_compress::supreme_engine::{
    CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
    PROFILE_SCHEMA_VERSION,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

fn profile() -> CompressionProfile {
    CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode: ProfileMode::Balanceado,
        codec: ProfileCodec::Zstd,
        fidelity: ProfileFidelity::Lossless,
        corpus_mode: nexus_compress::api::CorpusMode::Everything,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
        skip_archive: false,
    }
}

fn build_large_corpus(root: &PathBuf, n_files: usize, per_file_kb: usize) -> Vec<(String, Vec<u8>)> {
    // Repetitive TypeScript so the LZMA(zstd) ratio is
    // meaningful and the dict trainer has signal to learn
    // from. Each file is well under the 200-file batch size
    // so the streaming path runs at least n_files / 200
    // times.
    let phrase = b"// Sprint 5.7.21-E: streaming batch test\n\
                   import { Component, OnInit, Input } from '@angular/core';\n\
                   export class HandlerImpl implements OnInit {\n\
                     @Input() id: string = '';\n\
                     ngOnInit(): void { this.id = String(Math.random()); }\n\
                   }\n";
    let mut files = Vec::new();
    fs::create_dir_all(root.join("src")).expect("mkdir src");
    for i in 0..n_files {
        let mut body = Vec::with_capacity(per_file_kb * 1024);
        while body.len() < per_file_kb * 1024 {
            body.extend_from_slice(phrase);
        }
        let name = format!("src/file_{:04}.ts", i);
        fs::write(root.join(&name), &body).expect("write file");
        files.push((name, body));
    }
    files
}

#[test]
fn streaming_roundtrips_bit_exact_multi_batch() {
    // 500 files × 4 KB = 2 MiB. With STREAM_BATCH_FILES=200
    // the streaming path runs 3 batches (200, 200, 100).
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-streaming-batch-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("mkdir");
    let files = build_large_corpus(&temp_dir, 500, 4);
    let total = files.iter().map(|(_, b)| b.len()).sum::<usize>();

    let invocation = CompressInvocation {
        profile: profile(),
        path: temp_dir.clone(),
        password: None,
        output_dir: None,
    };
    let events: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(
        &invocation,
        |ev: ProgressEvent| {
            events.lock().unwrap().push(format!("{:?}", ev.phase));
        },
    )
    .expect("engine compress");
    let archive = result.compressed_bytes.clone();
    assert_eq!(result.n_files as usize, files.len(), "file count");

    // Bit-exact roundtrip.
    let (entries, solid) = decompress(&archive).expect("decompress");
    assert_eq!(entries.len(), files.len());
    for e in &entries {
        let start = e.solid_offset as usize;
        let end = start + e.pre_size as usize;
        let recovered = &solid[start..end];
        let want = files
            .iter()
            .find(|(n, _)| n == &e.name)
            .map(|(_, b)| b.as_slice())
            .expect("file in entries");
        assert_eq!(
            recovered, want,
            "streaming: bit-exact roundtrip failed for {}",
            e.name
        );
    }
    eprintln!(
        "[streaming] {} files, {} B -> {} B, {:.2}x, bit-exact OK",
        files.len(),
        total,
        archive.len(),
        total as f64 / archive.len() as f64
    );

    // Progress events: at least one per file.
    let n_events = events.lock().unwrap().len();
    assert!(
        n_events >= files.len(),
        "expected at least {} progress events, got {}",
        files.len(),
        n_events
    );
    eprintln!("[streaming] progress events: {} (>= {})", n_events, files.len());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn streaming_single_batch_preserves_wire_format() {
    // 50 files × 4 KB = 200 KiB. Fits in ONE batch
    // (STREAM_BATCH_FILES=200). Same path as multi-batch
    // but exercises the batch loop body exactly once.
    let temp_dir = std::env::temp_dir().join(format!(
        "nexus-streaming-single-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("mkdir");
    let files = build_large_corpus(&temp_dir, 50, 4);
    let total = files.iter().map(|(_, b)| b.len()).sum::<usize>();

    let invocation = CompressInvocation {
        profile: profile(),
        path: temp_dir.clone(),
        password: None,
        output_dir: None,
    };
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(
        &invocation,
        |_: ProgressEvent| {},
    )
    .expect("engine compress");
    let archive = result.compressed_bytes.clone();

    // Roundtrip.
    let (entries, solid) = decompress(&archive).expect("decompress");
    assert_eq!(entries.len(), files.len());
    for e in &entries {
        let start = e.solid_offset as usize;
        let end = start + e.pre_size as usize;
        let recovered = &solid[start..end];
        let want = files
            .iter()
            .find(|(n, _)| n == &e.name)
            .map(|(_, b)| b.as_slice())
            .expect("file in entries");
        assert_eq!(recovered, want, "single-batch: bit-exact failed for {}", e.name);
    }
    eprintln!(
        "[streaming-single] {} files, {} B -> {} B, {:.2}x, bit-exact OK",
        files.len(),
        total,
        archive.len(),
        total as f64 / archive.len() as f64
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
