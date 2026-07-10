//! NexusCompress — hybrid file compressor MVP.
//!
//! Pipeline:
//!   input bytes
//!     -> classifier (block type)
//!     -> preprocessor (delta/exec filter per type)
//!     -> LZ77 match finder
//!     -> residual -> Context Mixing model -> probabilities
//!     -> rANS encoder -> .nexus bytes
//!
//! v4 format spec lives in `format.rs`. The entropy coder (`rans_v4`) is a
//! thin wrapper over the well-tested `rans` crate (ryg_rans).

pub mod api;
pub mod ast_minify;
pub mod cdc;
pub mod classifier;
pub mod codec;
/// Sprint 5.7.2 — Block-level AES-256-GCM encryption with
/// Argon2id KDF. The `derive_key` entry point produces a
/// `KdfResult` whose `preset_used` field goes straight
/// into the V3 header's `kdf_params` field, so the
/// decoder reproduces the same KDF output bit-for-bit.
pub mod crypto;
/// Sprint 5.7.2 — End-to-end encrypted + recovery wire-up.
/// Sits on top of `codec::compress` / `codec::decompress`
/// (the existing v3 format) and re-frames the output in the
/// V3 (NXE/NXR) format: AES-256-GCM per block, optional
/// Reed-Solomon parity shards. Decoder catches GCM auth
/// failures and falls back to the Gauss-Jordan recovery.
pub mod encrypted;
/// Sprint 5.7.2 — Reed-Solomon recovery codec. The `galois`
/// submodule holds the GF(2^8) finite-field arithmetic
/// (EXP/LOG tables + the four primitive operations);
/// `galois::codes` will hold the matrix-based encode/decode
/// pipeline that consumes the field primitives.
pub mod galois;
pub mod cost;
pub mod dedup;
pub mod dict_codec;
pub mod dictionary;
pub mod engine;
pub mod format;
pub mod fs;
pub mod lz77;
pub mod minify;
pub mod nxar;
pub mod ram;
pub mod rans_v4;
pub mod rle;
pub mod solid_archive;
pub mod scheduler;
pub mod stats;

pub use api::{compress_directory_with_backend, CompressionBackend};
pub use codec::{compress, compress_premium, decompress};
pub use format::{BlockType, NexusHeader};
