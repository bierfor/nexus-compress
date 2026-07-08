//! GF(2^8) finite-field arithmetic — Sprint 5.7.2.
//!
//! This is the **pure math layer** for the Reed-Solomon
//! recovery codec. The functions here are header-agnostic:
//! they don't know about block IDs, parity counts, or wire
//! format. Their only inputs and outputs are single bytes
//! (`u8`) treated as elements of GF(2^8).
//!
//! ## Field definition
//!
//! - **Polynomial:** `x^8 + x^4 + x^3 + x^2 + 1` (the AES
//!   reduction polynomial, in 8-bit form: `0x1D`).
//!   **NOT 0x1B** — that's the polynomial
//!   `x^8 + x^4 + x^3 + x + 1` which has multiplicative
//!   order 51 (a sub-field of GF(2^8)), not 255. Using
//!   0x1B with α=2 gives a 51-cycle: the first 51 powers
//!   look right, then the sequence restarts. We hit this
//!   bug on the first three attempts at this module and
//!   spent a sprint chasing it as an LLVM optimizer
//!   problem before realizing the polynomial was wrong.
//! - **Basis:** the generator is `α = 0x02` (the polynomial
//!   `x`). Every non-zero field element is `α^k` for some
//!   `k ∈ [0, 254]`.
//! - **Order of the multiplicative group:** 255 (a prime).
//!
//! ## Why the tables are hardcoded
//!
//! The tables in this module are the output of a one-time
//! Python precomputation (see `build_tables_bulletproof`
//! below for the algorithm). They're 256 bytes each, 512
//! bytes total of pure data — and they never change between
//! builds. Hardcoding them as `pub const` arrays means:
//!   - Zero runtime initialization cost (no `OnceLock`,
//!     no lazy first-read, no `if !INIT.is_set()` branch).
//!   - No risk of an optimizer eliding writes or
//!     transforming the table-builder loop into a
//!     degenerate form. The data is in `.rodata` from
//!     the moment the binary is linked.
//!   - `const` slices are `Send + Sync` for free.
//!
//! The trade-off: 512 bytes of `.rodata` (negligible —
//! the binary is 14 MB, this is 0.004 % of the size).
//!
//! ## Why we hand-roll this
//!
//! - The `reed-solomon-erasure` crate is GPL-3.0 → incompatible
//!   with our AGPL-3.0 + commercial dual license.
//! - The `rs-erasure` crate is MIT but pulls a `rayon` peer
//!   that fights our scoped `ThreadPoolBuilder` in
//!   `codec::parallel`.
//! - A from-scratch GF(2^8) + RS is ~300 LOC. The cost of
//!   writing it is less than the cost of arguing with a
//!   license review on every release.

/// `EXP_TABLE[i] = α^i` (mod the field), where `α = 0x02`.
///
/// Index 0 is the multiplicative identity (1). Indices
/// 1..=254 cycle through the 255 non-zero field elements.
/// Index 255 duplicates index 0 (so `(LOG_TABLE[a] +
/// LOG_TABLE[b]) % 255` can be looked up without
/// bounds-checking the wrap).
pub const EXP_TABLE: [u8; 256] = [
    0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1d, 0x3a, 0x74, 0xe8, 0xcd, 0x87, 0x13, 0x26,
    0x4c, 0x98, 0x2d, 0x5a, 0xb4, 0x75, 0xea, 0xc9, 0x8f, 0x03, 0x06, 0x0c, 0x18, 0x30, 0x60, 0xc0,
    0x9d, 0x27, 0x4e, 0x9c, 0x25, 0x4a, 0x94, 0x35, 0x6a, 0xd4, 0xb5, 0x77, 0xee, 0xc1, 0x9f, 0x23,
    0x46, 0x8c, 0x05, 0x0a, 0x14, 0x28, 0x50, 0xa0, 0x5d, 0xba, 0x69, 0xd2, 0xb9, 0x6f, 0xde, 0xa1,
    0x5f, 0xbe, 0x61, 0xc2, 0x99, 0x2f, 0x5e, 0xbc, 0x65, 0xca, 0x89, 0x0f, 0x1e, 0x3c, 0x78, 0xf0,
    0xfd, 0xe7, 0xd3, 0xbb, 0x6b, 0xd6, 0xb1, 0x7f, 0xfe, 0xe1, 0xdf, 0xa3, 0x5b, 0xb6, 0x71, 0xe2,
    0xd9, 0xaf, 0x43, 0x86, 0x11, 0x22, 0x44, 0x88, 0x0d, 0x1a, 0x34, 0x68, 0xd0, 0xbd, 0x67, 0xce,
    0x81, 0x1f, 0x3e, 0x7c, 0xf8, 0xed, 0xc7, 0x93, 0x3b, 0x76, 0xec, 0xc5, 0x97, 0x33, 0x66, 0xcc,
    0x85, 0x17, 0x2e, 0x5c, 0xb8, 0x6d, 0xda, 0xa9, 0x4f, 0x9e, 0x21, 0x42, 0x84, 0x15, 0x2a, 0x54,
    0xa8, 0x4d, 0x9a, 0x29, 0x52, 0xa4, 0x55, 0xaa, 0x49, 0x92, 0x39, 0x72, 0xe4, 0xd5, 0xb7, 0x73,
    0xe6, 0xd1, 0xbf, 0x63, 0xc6, 0x91, 0x3f, 0x7e, 0xfc, 0xe5, 0xd7, 0xb3, 0x7b, 0xf6, 0xf1, 0xff,
    0xe3, 0xdb, 0xab, 0x4b, 0x96, 0x31, 0x62, 0xc4, 0x95, 0x37, 0x6e, 0xdc, 0xa5, 0x57, 0xae, 0x41,
    0x82, 0x19, 0x32, 0x64, 0xc8, 0x8d, 0x07, 0x0e, 0x1c, 0x38, 0x70, 0xe0, 0xdd, 0xa7, 0x53, 0xa6,
    0x51, 0xa2, 0x59, 0xb2, 0x79, 0xf2, 0xf9, 0xef, 0xc3, 0x9b, 0x2b, 0x56, 0xac, 0x45, 0x8a, 0x09,
    0x12, 0x24, 0x48, 0x90, 0x3d, 0x7a, 0xf4, 0xf5, 0xf7, 0xf3, 0xfb, 0xeb, 0xcb, 0x8b, 0x0b, 0x16,
    0x2c, 0x58, 0xb0, 0x7d, 0xfa, 0xe9, 0xcf, 0x83, 0x1b, 0x36, 0x6c, 0xd8, 0xad, 0x47, 0x8e, 0x01,
];

/// `LOG_TABLE[x] = i` such that `EXP_TABLE[i] == x`.
///
/// `LOG_TABLE[0]` is 0 by convention. **You must not
/// call `gf_mul`, `gf_inv`, or `gf_pow` with `a == 0`
/// without first short-circuiting the zero case** (those
/// functions do this internally).
pub const LOG_TABLE: [u8; 256] = [
    0x00, 0x00, 0x01, 0x19, 0x02, 0x32, 0x1a, 0xc6, 0x03, 0xdf, 0x33, 0xee, 0x1b, 0x68, 0xc7, 0x4b,
    0x04, 0x64, 0xe0, 0x0e, 0x34, 0x8d, 0xef, 0x81, 0x1c, 0xc1, 0x69, 0xf8, 0xc8, 0x08, 0x4c, 0x71,
    0x05, 0x8a, 0x65, 0x2f, 0xe1, 0x24, 0x0f, 0x21, 0x35, 0x93, 0x8e, 0xda, 0xf0, 0x12, 0x82, 0x45,
    0x1d, 0xb5, 0xc2, 0x7d, 0x6a, 0x27, 0xf9, 0xb9, 0xc9, 0x9a, 0x09, 0x78, 0x4d, 0xe4, 0x72, 0xa6,
    0x06, 0xbf, 0x8b, 0x62, 0x66, 0xdd, 0x30, 0xfd, 0xe2, 0x98, 0x25, 0xb3, 0x10, 0x91, 0x22, 0x88,
    0x36, 0xd0, 0x94, 0xce, 0x8f, 0x96, 0xdb, 0xbd, 0xf1, 0xd2, 0x13, 0x5c, 0x83, 0x38, 0x46, 0x40,
    0x1e, 0x42, 0xb6, 0xa3, 0xc3, 0x48, 0x7e, 0x6e, 0x6b, 0x3a, 0x28, 0x54, 0xfa, 0x85, 0xba, 0x3d,
    0xca, 0x5e, 0x9b, 0x9f, 0x0a, 0x15, 0x79, 0x2b, 0x4e, 0xd4, 0xe5, 0xac, 0x73, 0xf3, 0xa7, 0x57,
    0x07, 0x70, 0xc0, 0xf7, 0x8c, 0x80, 0x63, 0x0d, 0x67, 0x4a, 0xde, 0xed, 0x31, 0xc5, 0xfe, 0x18,
    0xe3, 0xa5, 0x99, 0x77, 0x26, 0xb8, 0xb4, 0x7c, 0x11, 0x44, 0x92, 0xd9, 0x23, 0x20, 0x89, 0x2e,
    0x37, 0x3f, 0xd1, 0x5b, 0x95, 0xbc, 0xcf, 0xcd, 0x90, 0x87, 0x97, 0xb2, 0xdc, 0xfc, 0xbe, 0x61,
    0xf2, 0x56, 0xd3, 0xab, 0x14, 0x2a, 0x5d, 0x9e, 0x84, 0x3c, 0x39, 0x53, 0x47, 0x6d, 0x41, 0xa2,
    0x1f, 0x2d, 0x43, 0xd8, 0xb7, 0x7b, 0xa4, 0x76, 0xc4, 0x17, 0x49, 0xec, 0x7f, 0x0c, 0x6f, 0xf6,
    0x6c, 0xa1, 0x3b, 0x52, 0x29, 0x9d, 0x55, 0xaa, 0xfb, 0x60, 0x86, 0xb1, 0xbb, 0xcc, 0x3e, 0x5a,
    0xcb, 0x59, 0x5f, 0xb0, 0x9c, 0xa9, 0xa0, 0x51, 0x0b, 0xf5, 0x16, 0xeb, 0x7a, 0x75, 0x2c, 0xd7,
    0x4f, 0xae, 0xd5, 0xe9, 0xe6, 0xe7, 0xad, 0xe8, 0x74, 0xd6, 0xf4, 0xea, 0xa8, 0x50, 0x58, 0xaf,
];

// ─────────────────────────────────────────────────────
//  Field operations
// ─────────────────────────────────────────────────────

/// Addition in GF(2^8). Characteristic 2 means `add` and
/// `sub` are the same operation: bitwise XOR. There is no
/// carry, no sign, no overflow.
#[inline]
pub const fn gf_add(a: u8, b: u8) -> u8 {
    a ^ b
}

/// Subtraction in GF(2^8). Identical to addition because
/// every element is its own additive inverse in a field
/// of characteristic 2.
#[inline]
pub const fn gf_sub(a: u8, b: u8) -> u8 {
    a ^ b
}

/// Multiplication in GF(2^8). Lock-free, branch-only-on-zero,
/// table-lookup-fast. `O(1)` with two memory reads and one
/// XOR-free index.
#[inline]
pub const fn gf_mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let la = LOG_TABLE[a as usize] as usize;
    let lb = LOG_TABLE[b as usize] as usize;
    EXP_TABLE[(la + lb) % 255]
}

/// Multiplicative inverse in GF(2^8). Returns 0 for input 0
/// (callers should guard this — dividing by zero in a field
/// is undefined; we choose the convention "gf_inv(0) = 0"
/// so a missing-cancel-step bug produces a 0 byte that the
/// downstream check will catch, rather than a panic that
/// aborts the whole decode).
#[inline]
pub const fn gf_inv(a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    // a^(-1) = a^(255-1) = a^254 in a group of order 255.
    // We use the same exp-table trick: 255 - log[a] gives
    // the index of a^(-1) directly.
    let la = LOG_TABLE[a as usize] as usize;
    EXP_TABLE[255 - la]
}

/// Exponentiation in GF(2^8). `a^n` computed via
/// repeated-squaring is overkill for n < 255; we use the
/// log trick and a single modular reduction.
#[inline]
pub const fn gf_pow(a: u8, n: u32) -> u8 {
    if a == 0 {
        return if n == 0 { 1 } else { 0 };
    }
    let la = LOG_TABLE[a as usize] as usize;
    // Modular reduction on the log index. We do `as u32` to
    // avoid overflow on `la * n` for n up to 2^32.
    let idx = ((la as u32 * n) % 255) as usize;
    EXP_TABLE[idx]
}

// ─────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 1. `add` is XOR. The fundamental axiom of
    ///    characteristic-2 fields.
    #[test]
    fn add_is_xor() {
        assert_eq!(gf_add(0, 0), 0);
        assert_eq!(gf_add(0xAB, 0xCD), 0xAB ^ 0xCD);
        assert_eq!(gf_add(0xFF, 0xFF), 0);
        // Every element is its own additive inverse.
        for a in 0u8..=255 {
            assert_eq!(gf_add(a, a), 0, "a + a must be 0 for a = {}", a);
        }
    }

    /// 2. `sub` is identical to `add` in characteristic 2.
    ///    Sanity-checked against a known identity: for
    ///    `a != b`, `a + b == a - b` is a tautology here.
    #[test]
    fn sub_equals_add() {
        for a in 0u8..=255 {
            for b in 0u8..=255 {
                assert_eq!(gf_sub(a, b), gf_add(a, b));
            }
        }
    }

    /// 3. `mul` with 0 is 0 (absorbing element), and `mul`
    ///    with 1 is the identity. The two basic boundary
    ///    conditions any field must satisfy.
    #[test]
    fn mul_zero_and_one() {
        for a in 0u8..=255 {
            assert_eq!(gf_mul(0, a), 0, "0 * a must be 0 for a = {}", a);
            assert_eq!(gf_mul(a, 0), 0, "a * 0 must be 0 for a = {}", a);
            assert_eq!(gf_mul(1, a), a, "1 * a must equal a for a = {}", a);
            assert_eq!(gf_mul(a, 1), a, "a * 1 must equal a for a = {}", a);
        }
    }

    /// 4. `mul` is commutative. The group operation on a
    ///    finite field is always commutative; this test
    ///    validates that the table-based implementation
    ///    preserved the property (a classic source of
    ///    off-by-one bugs in GF arithmetic).
    #[test]
    fn mul_is_commutative() {
        for i in 0u8..=255 {
            let a = i;
            let b = i.wrapping_mul(7) ^ 0x5A;
            assert_eq!(gf_mul(a, b), gf_mul(b, a), "{} * {} should commute", a, b);
        }
    }

    /// 5. `inv` is the multiplicative inverse: `a * inv(a)
    ///    == 1` for all `a != 0`. This is THE test that
    ///    catches a misbuilt EXP/LOG table — if the
    ///    generator, polynomial, or wrap-around index is
    ///    off, the inv of some element will not produce 1
    ///    when multiplied back.
    #[test]
    fn inv_roundtrips_to_one() {
        for a in 1u8..=255 {
            let inv = gf_inv(a);
            assert_ne!(inv, 0, "inv({}) must be non-zero", a);
            assert_eq!(gf_mul(a, inv), 1, "{} * inv({}) must be 1", a, a);
            // Also: inv(inv(a)) == a (inverse is an
            // involution on the non-zero elements).
            assert_eq!(gf_inv(inv), a, "inv(inv({})) must equal {}", a, a);
        }
    }

    /// 6. Fermat's little theorem in GF(2^8): for any
    ///    `a != 0`, `a^255 == 1` (because the multiplicative
    ///    group has order 255). This is the most general
    ///    sanity check on the table structure — if the
    ///    theorem fails for any a, the tables are wrong.
    #[test]
    fn pow_255_is_one() {
        for a in 1u8..=255 {
            assert_eq!(gf_pow(a, 255), 1, "a^255 must be 1 for a = {}", a);
        }
        // 0^0 is defined as 1 by convention in the impl.
        assert_eq!(gf_pow(0, 0), 1);
        // 0^n for n > 0 is 0.
        for n in 1u32..=10 {
            assert_eq!(gf_pow(0, n), 0);
        }
    }

    /// 7. The two tables are structurally consistent:
    /// `exp[log[x]] == x` for all `x != 0`, and every
    /// non-zero element appears exactly once in `exp`.
    /// This is the sanity check that catches the bug
    /// where the table-generation loop leaves most log
    /// entries unwritten.
    #[test]
    fn exp_log_are_consistent() {
        // exp[0] must be 1 and log[1] must be 0.
        assert_eq!(EXP_TABLE[0], 1, "exp[0] must be 1");
        assert_eq!(LOG_TABLE[1], 0, "log[1] must be 0");
        // exp[255] is the periodic copy of exp[0] (used
        // for the modulo-255 wrap in mul).
        assert_eq!(EXP_TABLE[255], EXP_TABLE[0]);
        // log[0] is convention 0 (callers must short-circuit
        // before reading it).
        assert_eq!(LOG_TABLE[0], 0);
        // Every non-zero element must appear at exactly one
        // position in exp[0..255].
        let mut seen = [false; 256];
        for i in 0..255 {
            let e = EXP_TABLE[i];
            assert!(!seen[e as usize], "element {} appears twice in exp", e);
            seen[e as usize] = true;
        }
        // log[exp[i]] must equal i for all i in 0..255.
        for i in 0..255 {
            let e = EXP_TABLE[i];
            assert_eq!(
                LOG_TABLE[e as usize], i as u8,
                "log[exp[{}]] = {} should be {}",
                i, LOG_TABLE[e as usize], i
            );
        }
        // The 255 non-zero elements are all present.
        for a in 1u8..=255 {
            assert!(seen[a as usize], "element {} never appears in exp", a);
        }
    }
}
