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
//! v0 format spec lives in `format.rs`.

pub mod cdc;
pub mod classifier;
pub mod cm;
pub mod codec;
pub mod cost;
pub mod dedup;
pub mod format;
pub mod lz77;
pub mod range2;
pub mod rans;

pub use codec::{compress, decompress};
pub use format::{BlockType, NexusHeader};