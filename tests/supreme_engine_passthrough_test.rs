//! Sprint 5.7.11: end-to-end passthrough roundtrip via the
//! SupremeEngine. Builds a synthetic corpus that mixes compressible
//! text files with a passthrough PNG, runs the engine, and then
//! decompresses the resulting archive with the new `NXPT` trailer
//! parser to confirm the PNG bytes survive bit-exact.
//!
//! This is the integration-level companion to the unit tests in
//! `src/solid_archive.rs` (which build the archive by hand). The
//! unit tests pin the wire format; this test pins the end-to-end
//! behavior of the engine + the decoder working together.

use nexus_compress::solid_archive::decompress as solid_decompress;
use nexus_compress::supreme_engine::{
    CompressInvocation, CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode,
    PROFILE_SCHEMA_VERSION,
};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn profile(corpus_mode: nexus_compress::api::CorpusMode) -> CompressionProfile {
    CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode: ProfileMode::Balanceado,
        codec: ProfileCodec::Auto,
        fidelity: ProfileFidelity::Lossy,
        corpus_mode,
        raw_extensions: vec![],
        minify_extensions: vec![],
        encrypt: false,
        recovery_level: nexus_compress::api::RecoveryLevel::Low,
    }
}

fn write_synthetic_corpus(root: &std::path::Path) -> (Vec<u8>, Vec<u8>) {
    // 200 KB of repetitive TypeScript so the LZMA/zstd block is
    // real (not just a tiny stub) and clearly compressible.
    let ts_body = b"import { foo } from './bar';\n\
                    export function handler(req: Request): Response {\n\
                      return new Response('hello world', { status: 200 });\n\
                    }\n"
        .repeat(800);
    fs::write(root.join("src/handler.ts"), &ts_body).expect("write handler.ts");

    // 80 KB of "PNG" with a recognizable repeating pattern. We
    // don't care about the header being valid PNG — we only need
    // the bytes to roundtrip. The engine should route this file
    // through passthrough because `.png` is in
    // `format_knowledge::RAW_FORMATS`, then zstd-compress the
    // passthrough payload because the body is repetitive.
    let mut png_body: Vec<u8> = Vec::with_capacity(80 * 1024);
    let pattern: Vec<u8> = (0..255_u8).collect();
    while png_body.len() < 80 * 1024 {
        png_body.extend_from_slice(&pattern);
    }
    png_body.truncate(80 * 1024);
    fs::write(root.join("assets/logo.png"), &png_body).expect("write logo.png");
    (ts_body, png_body)
}

#[test]
fn end_to_end_roundtrip_preserves_passthrough_bytes() {
    let tmp = std::env::temp_dir().join(format!(
        "nexus-supreme-passthrough-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("mkdir tmp");
    let corpus_root = tmp.join("corpus");
    fs::create_dir_all(corpus_root.join("src")).expect("mkdir src");
    fs::create_dir_all(corpus_root.join("assets")).expect("mkdir assets");
    let (ts_body, png_body) = write_synthetic_corpus(&corpus_root);

    let invocation = CompressInvocation {
        profile: profile(nexus_compress::api::CorpusMode::Everything),
        path: corpus_root.clone(),
        password: None,
        output_dir: Some(tmp.clone()),
    };
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(&invocation, |_| {})
        .expect("SupremeEngine compress");
    assert!(result.compressed_size > 0);
    assert!(result.n_files >= 2);

    // The engine returns the archive bytes inline
    // (`compressed_bytes`); the Tauri command / CLI write them
    // to disk. We do the same here so we can feed the file
    // back through the public decompress path.
    let archive_path: PathBuf = tmp.join("corpus.nxs6");
    let mut f = fs::File::create(&archive_path).expect("create archive file");
    f.write_all(&result.compressed_bytes).expect("write archive bytes");
    drop(f);
    let bytes = fs::read(&archive_path).expect("read archive");

    // Roundtrip through the new trailer-aware decompress.
    let (entries, solid) = solid_decompress(&bytes).expect("solid_decompress");
    let png_entry = entries
        .iter()
        .find(|e| e.name == "assets/logo.png")
        .expect("PNG entry present in decompressed archive");
    assert_eq!(
        png_entry.preprocessor,
        nexus_compress::solid_archive::Preprocessor::Raw,
        "PNG must be marked as Raw passthrough"
    );
    let start = png_entry.solid_offset as usize;
    let end = start + png_entry.pre_size as usize;
    assert_eq!(
        &solid[start..end],
        png_body.as_slice(),
        "PNG bytes must roundtrip bit-exact through the engine + decoder"
    );
    assert_eq!(png_entry.pre_size as usize, png_body.len());
    assert_eq!(png_entry.original_size, png_entry.pre_size);

    // The TypeScript source should also be present. It is
    // minified (swc) by the Lossy preprocessor, so we don't
    // expect byte-equality with the input — just that the
    // entry exists and is shorter than the input.
    let ts_entry = entries
        .iter()
        .find(|e| e.name == "src/handler.ts")
        .expect("TS entry present in decompressed archive");
    assert_eq!(
        ts_entry.preprocessor,
        nexus_compress::solid_archive::Preprocessor::SwcAst,
        "TS must use swc (Lossy default)"
    );
    let ts_start = ts_entry.solid_offset as usize;
    let ts_end = ts_start + ts_entry.pre_size as usize;
    let ts_recovered = &solid[ts_start..ts_end];
    assert!(
        ts_recovered.len() <= ts_body.len(),
        "swc-minified TS ({}) must be <= source ({})",
        ts_recovered.len(),
        ts_body.len()
    );

    // Cleanup.
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn end_to_end_archive_with_only_passthroughs_still_roundtrips() {
    // Edge case: a corpus with no compressible text at all. The
    // engine should produce a valid archive (empty LZMA block,
    // full `NXPT` trailer) and the decoder must still recover
    // every passthrough bit-exact.
    let tmp = std::env::temp_dir().join(format!(
        "nexus-supreme-passthrough-only-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("mkdir tmp");
    let corpus_root = tmp.join("corpus");
    fs::create_dir_all(corpus_root.join("img")).expect("mkdir img");

    let mut jpg_body: Vec<u8> = Vec::with_capacity(40 * 1024);
    let pattern: Vec<u8> = (0..255_u8).collect();
    while jpg_body.len() < 40 * 1024 {
        jpg_body.extend_from_slice(&pattern);
    }
    jpg_body.truncate(40 * 1024);
    fs::write(corpus_root.join("img/photo.jpg"), &jpg_body).expect("write photo.jpg");

    let invocation = CompressInvocation {
        profile: profile(nexus_compress::api::CorpusMode::Everything),
        path: corpus_root.clone(),
        password: None,
        output_dir: Some(tmp.clone()),
    };
    let result = nexus_compress::supreme_engine::SupremeEngine::compress(&invocation, |_| {})
        .expect("SupremeEngine compress (passthrough-only)");
    assert!(result.compressed_size > 0);
    assert!(result.n_files >= 1);

    let archive_path: PathBuf = tmp.join("corpus.nxs6");
    let mut f = fs::File::create(&archive_path).expect("create archive file");
    f.write_all(&result.compressed_bytes).expect("write archive bytes");
    drop(f);
    let bytes = fs::read(&archive_path).expect("read archive");
    let (entries, solid) = solid_decompress(&bytes).expect("solid_decompress passthrough-only");
    let jpg_entry = entries
        .iter()
        .find(|e| e.name == "img/photo.jpg")
        .expect("JPG entry present");
    let start = jpg_entry.solid_offset as usize;
    let end = start + jpg_entry.pre_size as usize;
    assert_eq!(&solid[start..end], jpg_body.as_slice());

    let _ = fs::remove_dir_all(&tmp);
}
