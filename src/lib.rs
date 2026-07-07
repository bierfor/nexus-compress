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
pub mod cost;
pub mod dedup;
pub mod dict_codec;
pub mod dictionary;
pub mod engine;
pub mod format;
pub mod lz77;
pub mod minify;
pub mod nxar;
pub mod rans_v4;
pub mod rle;
pub mod solid_archive;

pub use codec::{compress, compress_premium, decompress};
pub use format::{BlockType, NexusHeader};
pub use api::{compress_directory_with_backend, CompressionBackend};