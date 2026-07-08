//! GF(2^8) and Reed-Solomon primitives — Sprint 5.7.2.
//!
//! Split into two sub-modules:
//!
//! - [`field`]: pure GF(2^8) arithmetic (tables + add/sub/
//!   mul/inv/pow). Header-agnostic; this is the math layer.
//!   The whole `field.rs` is `const`-constructable and
//!   depends on no other crate.
//! - [`codes`]: (Sprint 5.7.2 follow-up) the Reed-Solomon
//!   encode/decode pipeline that uses the field primitives
//!   to build the parity matrix and run the Berlekamp–
//!   Massey error locator. This module WILL depend on the
//!   locked wire-format header (it needs to know the
//!   `parity_count` field width).
//!
//! The split is intentional: the math layer is testable in
//! isolation and is invariant under any future wire-format
//! change. Only `codes` is coupled to the format.

pub mod field;

// `codes` lands in a follow-up commit once the wire-format
// header is locked (Sprint 5.7.2 step 2). Keeping the
// module declaration here as a placeholder so downstream
// `use crate::galois::codes::*` paths are stable once we
// add the file.
#[allow(dead_code)]
pub mod codes;
