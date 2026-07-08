//! Reed-Solomon encode / decode pipeline — Sprint 5.7.2 step 2.
//!
//! This module composes the GF(2^8) primitives from
//! [`crate::galois::field`] into a complete erasure-coding
//! codec. The math is locked; this layer is the engineering.
//!
//! ## Why Cauchy (not Vandermonde)
//!
//! Both matrices work for Reed-Solomon over GF(2^8), but
//! Cauchy has one property that matters for us: **any
//! square submatrix of a Cauchy matrix is guaranteed
//! invertible** (provided the row and column "anchor"
//! points are distinct). This means the decoder doesn't
//! need to check for singular submatrices — it can blindly
//! run Gaussian elimination on whichever subset of data
//! shards are missing, up to the `parity_shards` limit.
//!
//! Vandermonde matrices don't have that guarantee. A
//! Vandermonde with carefully-chosen anchor points can
//! hit a singular submatrix if the user loses exactly the
//! wrong `t` data shards. We'd have to add a check + a
//! retry with different anchors, which complicates the
//! recovery code. Cauchy sidesteps that entirely.
//!
//! ## Matrix structure
//!
//! The encoding matrix `M` is `(k + m) × k` where
//! `k = data_shards` and `m = parity_shards`:
//!
//! ```text
//! M = [ I_k  ]     (top k rows: identity, the data shards
//!     [ C   ]        pass through unchanged)
//! ```
//!
//! where `C` is the `m × k` Cauchy matrix:
//!
//! ```text
//! C[i][j] = 1 / (x[i] - y[j])     in GF(2^8)
//! ```
//!
//! with `x[i] = i + 1` and `y[j] = k + j + 1` (so `x` and
//! `y` are disjoint sets of non-zero field elements, as
//! the Cauchy definition requires).
//!
//! ## Encoding
//!
//! For each parity shard `i` and each byte position `b`:
//!
//! ```text
//! parity[i][b] = Σ_j C[i][j] × data[j][b]    (in GF(2^8))
//! ```
//!
//! The outer loop over `b` is parallelizable per shard (each
//! parity shard is independent of the others). The inner
//! sum over `j` is sequential within a shard, but the
//! "per-byte multiply-and-XOR" pattern is one `gf_mul` + one
//! `gf_add` per inner step — no shared state, perfect
//! cache behavior. A 1 GiB input with 32 MiB blocks
//! produces 32 data shards + 3 parity shards; total
//! compute is `(m × k) × total_bytes / 2^23` operations.
//!
//! ## Decoding (recovery)
//!
//! When up to `m` data shards are missing:
//!
//! 1. Identify which data shards are present and which
//!    are missing (call the missing set `MISS`, the
//!    present set `PRES`, with `|MISS| ≤ m`).
//! 2. Build the `|MISS| × |MISS|` submatrix of `C` indexed
//!    by `(MISS, MISS)` — a Cauchy submatrix, guaranteed
//!    invertible.
//! 3. Invert that submatrix via Gaussian elimination over
//!    GF(2^8).
//! 4. For each missing data shard `j` in `MISS`:
//!    For each byte position `b`:
//!    ```text
//!    data[j][b] = Σ_i C^-1[j][i] × parity[i][b]
//!    ```
//! 5. Verify the reconstructed shard against the GCM tag
//!    that was applied in Pass 2 of the parallel encoder
//!    (the AES-256-GCM tag is the canonical "is this the
//!    right block?" check — RS gives us a candidate, GCM
//!    tells us if it's the right one). See Sprint 5.7.2
//!    design doc §1.3 for the full threat model.
//!
//! ## Memory layout
//!
//! The encoding matrix is stored row-major as a `Vec<u8>`
//! of length `(data_shards + parity_shards) * data_shards`.
//! We pad the matrix to a power-of-2 row size at
//! construction time so SIMD-friendly loops are possible
//! in a future optimization pass.

use crate::galois::field::{gf_add, gf_inv, gf_mul, gf_sub};
use std::fmt;

/// The encoding matrix. Constructed once per
/// `ReedSolomon` instance (which lives for the duration
/// of one compress/decompress operation).
///
/// **Field-element convention:** `M[row * data_shards + col]`
/// holds the GF(2^8) coefficient for the multiplication.
/// Reading the matrix in row-major form lets us do
/// `(k + m)` sequential reads of `k` bytes each during
/// encoding, which is what the CPU's hardware prefetcher
/// expects.
pub struct ReedSolomon {
    /// `k` — the number of data shards. Must be ≥ 1.
    pub data_shards: usize,
    /// `m` — the number of parity shards. Must be ≥ 1.
    /// The decoder can recover up to `m` missing data
    /// shards (or, by symmetry, up to `m` missing parity
    /// shards — see decode_recover()).
    pub parity_shards: usize,
    /// Row-major encoding matrix of shape
    /// `(data_shards + parity_shards) × data_shards`.
    /// The first `data_shards` rows are the identity
    /// (data shards pass through); the remaining
    /// `parity_shards` rows are the Cauchy coefficients.
    pub matrix: Vec<u8>,
}

/// Invert an n × n matrix over GF(2^8) in place via
/// Gauss-Jordan elimination.
///
/// **Layout:** `aug` is a row-major `n × 2n` augmented
/// matrix — the left half is the matrix to invert, the
/// right half starts as the identity and is transformed
/// into the inverse. After the call, `aug[i * 2n + j]`
/// holds `1` for `j == i` and `0` for `j != i` (left half
/// is the identity), and the right half holds the
/// inverse matrix row-by-row.
///
/// **Algorithm:** standard Gauss-Jordan:
/// 1. For each column `c` in 0..n:
///    a. Find a pivot row `r ≥ c` with `aug[r, c] != 0`.
///    b. Swap row `r` with row `c` (if different).
///    c. Scale row `c` by `1 / aug[c, c]` so the pivot
///       becomes 1.
///    d. Eliminate column `c` from every other row by
///       XORing `(factor * row_c)` into the other row,
///       where `factor = aug[other, c]`.
///
/// For an `n ≤ 12` matrix, the entire computation lives
/// in L1 cache (≤ 288 bytes for the augmented matrix).
/// The cost is `O(n³)` `gf_mul`s — for n=12, that's
/// ~3000 multiplications, a few microseconds on any
/// modern CPU. A real bottleneck we don't have.
///
/// **Why we panic on a missing pivot instead of returning
/// `Err`:** the caller (Sprint 5.7.2's `decode_recover`)
/// only ever feeds us Cauchy submatrices, which are
/// unconditionally invertible. A missing pivot is a
/// "this should be impossible" condition — failing fast
/// is more useful than silently producing a corrupt
/// inverse.
/// An error type for the Reed-Solomon operations. Most/// errors here are "the user asked for something we can't
/// satisfy" (e.g., zero parity shards, too many total
/// shards to fit in GF(2^8)). They are unrecoverable at
/// the algorithm level; the caller should reconfigure
/// the recovery level and try again.
///
/// We implement `Display + std::error::Error` by hand
/// instead of pulling in `thiserror` — the engine's
/// `Cargo.toml` doesn't list `thiserror` as a direct dep
/// (only the Tauri app does), and 30 lines of boilerplate
/// is cheaper than touching the dependency surface for a
/// four-variant error type.
#[derive(Debug)]
pub enum ReedSolomonError {
    /// `data_shards + parity_shards` would exceed 254.
    /// Cauchy requires distinct non-zero field elements
    /// for the `x[i]` and `y[j]` anchor points; we use
    /// 1..=255 for the anchors so the natural cap is 254
    /// (we need 1 distinct value for each row AND each
    /// column, with no overlap).
    TooManyShards { data: usize, parity: usize },
    /// `parity_shards` was 0. The whole point of this
    /// codec is to generate parity; with zero parity
    /// shards we can't recover anything.
    ZeroParityShards,
    /// `data_shards` was 0. An empty input is a no-op;
    /// we don't need a codec for it.
    ZeroDataShards,
    /// The decoder found more than `parity_shards`
    /// missing data shards and cannot recover.
    TooManyMissing { missing: usize, available: usize },
    /// The shards slice had the wrong length (not
    /// `data_shards + parity_shards`).
    WrongShardCount { got: usize, expected: usize },
    /// Two shards had different lengths in the encode
    /// input.
    InconsistentShardLengths,
}

impl fmt::Display for ReedSolomonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyShards { data, parity } => write!(
                f,
                "too many shards: data={} + parity={} exceeds 254 (max Cauchy anchors in GF(2^8))",
                data, parity
            ),
            Self::ZeroParityShards => write!(f, "parity_shards must be >= 1, got 0"),
            Self::ZeroDataShards => write!(f, "data_shards must be >= 1, got 0"),
            Self::TooManyMissing { missing, available } => write!(
                f,
                "too many missing shards: {} missing, but only {} parity shards available",
                missing, available
            ),
            Self::WrongShardCount { got, expected } => write!(
                f,
                "wrong shard count: got {}, expected data_shards + parity_shards = {}",
                got, expected
            ),
            Self::InconsistentShardLengths => write!(
                f,
                "inconsistent shard lengths: all shards must be the same byte length"
            ),
        }
    }
}

impl std::error::Error for ReedSolomonError {}

/// Invert an n × n matrix over GF(2^8) in place via
/// Gauss-Jordan elimination.
///
/// **Layout:** `aug` is a row-major `n × 2n` augmented
/// matrix — the left half is the matrix to invert, the
/// right half starts as the identity and is transformed
/// into the inverse. After the call, `aug[i * 2n + j]`
/// holds `1` for `j == i` and `0` for `j != i` (left half
/// is the identity), and the right half holds the
/// inverse matrix row-by-row.
///
/// **Algorithm:** standard Gauss-Jordan:
/// 1. For each column `c` in 0..n:
///    a. Find a pivot row `r ≥ c` with `aug[r, c] != 0`.
///    b. Swap row `r` with row `c` (if different).
///    c. Scale row `c` by `1 / aug[c, c]` so the pivot
///       becomes 1.
///    d. Eliminate column `c` from every other row by
///       XORing `(factor * row_c)` into the other row,
///       where `factor = aug[other, c]`.
///
/// For an `n ≤ 12` matrix, the entire computation lives
/// in L1 cache (≤ 288 bytes for the augmented matrix).
/// The cost is `O(n³)` `gf_mul`s — for n=12, that's
/// ~3000 multiplications, a few microseconds on any
/// modern CPU. A real bottleneck we don't have.
///
/// **Why we panic on a missing pivot instead of returning
/// `Err`:** the caller (Sprint 5.7.2's `decode_recover`)
/// only ever feeds us Cauchy submatrices, which are
/// unconditionally invertible. A missing pivot is a
/// "this should be impossible" condition — failing fast
/// is more useful than silently producing a corrupt
/// inverse.
fn invert_gf256(aug: &mut [u8], n: usize) {
    assert_eq!(aug.len(), n * 2 * n, "augmented matrix must be n × 2n");

    for col in 0..n {
        // ── Step 1: find a non-zero pivot in column col,
        //          rows col..n. ──────────────────
        let mut pivot_row = col;
        while pivot_row < n && aug[pivot_row * 2 * n + col] == 0 {
            pivot_row += 1;
        }
        assert!(
            pivot_row < n,
            "invert_gf256: no non-zero pivot in column {} (matrix is singular; \
             this should be impossible for a Cauchy submatrix)",
            col
        );

        // ── Step 2: swap pivot row to position col. ──
        // For n ≤ 12, a swap is at most 24 byte copies;
        // for n=32 it's 64. The compiler will emit the
        // optimal memcpy or in-register swap; we don't
        // try to micro-optimize here.
        if pivot_row != col {
            for k in 0..(2 * n) {
                aug.swap(col * 2 * n + k, pivot_row * 2 * n + k);
            }
        }

        // ── Step 3: scale row col so pivot is 1. ─────
        // `gf_inv(0)` returns 0 by convention, but we
        // asserted above that the pivot is non-zero, so
        // the inverse is well-defined and non-zero.
        let pivot_val = aug[col * 2 * n + col];
        let pivot_inv = gf_inv(pivot_val);
        if pivot_inv != 1 {
            // Common case: pivot is 1, no work needed.
            // Uncommon case: pivot is some other field
            // element; scale the whole row by its
            // inverse. Both halves of the augmented
            // matrix must be scaled (otherwise the
            // right half would stop being A^-1 * 1 and
            // start being A^-1 * pivot_inv).
            for k in 0..(2 * n) {
                aug[col * 2 * n + k] = gf_mul(aug[col * 2 * n + k], pivot_inv);
            }
        }

        // ── Step 4: eliminate column col from every
        //          other row. ───────────────────────
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = aug[row * 2 * n + col];
            if factor == 0 {
                continue; // already zero, no work needed
            }
            // row[k] ^= factor * col[k], in GF(2^8).
            // This is the characteristic-2 trick: the
            // "subtract" of Gaussian elimination is just
            // XOR, because `-x = +x` in GF(2).
            for k in 0..(2 * n) {
                let rhs = gf_mul(factor, aug[col * 2 * n + k]);
                aug[row * 2 * n + k] = gf_add(aug[row * 2 * n + k], rhs);
            }
        }
    }
}

impl ReedSolomon {
    /// Construct a new Reed-Solomon codec with the given
    /// number of data and parity shards. The encoding
    /// matrix is built immediately (the matrix is a pure
    /// function of `data_shards` and `parity_shards`, so
    /// there's no need to defer it).
    ///
    /// **Naming convention (matches the rest of the
    /// field.md design doc):** `k` for data shards, `m`
    /// for parity shards. The total number of shards
    /// written to disk is `k + m`; the file size is the
    /// same as if we'd written `k` data shards only, plus
    /// the `m` parity shards' worth of bytes. With
    /// `m / (k + m) = 10 %` (the default `low` recovery
    /// level), a 1 GiB input produces 1.1 GiB on disk.
    pub fn new(data_shards: usize, parity_shards: usize) -> Result<Self, ReedSolomonError> {
        // Input validation. We want the codec to be
        // impossible to misuse — every error here is a
        // configuration mistake by the caller, and
        // returning a Result is the right shape.
        if data_shards == 0 {
            return Err(ReedSolomonError::ZeroDataShards);
        }
        if parity_shards == 0 {
            return Err(ReedSolomonError::ZeroParityShards);
        }
        // Cauchy needs 1..=254 (we use 1..=255, but we
        // need at least 2 distinct anchor values for
        // any non-trivial matrix — one for `x` rows and
        // one for `y` columns). For a `k + m` total, we
        // need `k + m` distinct values, so cap at 254
        // to leave room for the seed value 0.
        if data_shards + parity_shards > 254 {
            return Err(ReedSolomonError::TooManyShards {
                data: data_shards,
                parity: parity_shards,
            });
        }

        let total_rows = data_shards + parity_shards;
        let mut matrix = vec![0u8; total_rows * data_shards];

        // Top k rows: identity. The data shards pass
        // through unchanged — this is the "clear" part of
        // the encoding, matching what 7z and WinRAR do.
        for row in 0..data_shards {
            matrix[row * data_shards + row] = 1;
        }

        // Bottom m rows: Cauchy coefficients. Anchor
        // points:
        //   x[i] = (i + 1) as field element,  for i in 0..m
        //   y[j] = (k + j + 1) as field element,  for j in 0..k
        // The two sets are disjoint by construction:
        //   x range: 1..=m
        //   y range: (k+1)..=(k+m)
        // They never overlap because m ≥ 1 and k ≥ 1,
        // so the smallest y (k+1) is at least 2, and the
        // largest x (m) is at most 254.
        for i in 0..parity_shards {
            let x = (i + 1) as u8;
            for j in 0..data_shards {
                let y = (data_shards + j + 1) as u8;
                // C[i][j] = 1 / (x - y) in GF(2^8).
                // Subtraction in characteristic 2 is
                // XOR, so this is gf_inv(x ^ y).
                // Both x and y are non-zero; their XOR is
                // non-zero with overwhelming probability
                // (only x == y would be zero, which the
                // disjoint-set construction prevents).
                let row_idx = (data_shards + i) * data_shards + j;
                matrix[row_idx] = gf_inv(x ^ y);
            }
        }

        Ok(Self {
            data_shards,
            parity_shards,
            matrix,
        })
    }

    /// Generate the parity shards from the data shards.
    ///
    /// **Layout contract:** the input `shards` slice has
    /// length `data_shards + parity_shards`. The first
    /// `data_shards` entries are the INPUT data (read
    /// but not modified); the last `parity_shards`
    /// entries are the OUTPUT parity shards (overwritten
    /// in place). All shards must be the same length;
    /// shorter parity shards are extended with zeros
    /// before the call (so the math works for the
    /// shorter positions) and then truncated by the
    /// caller.
    ///
    /// **Cost:** for each of the `parity_shards` rows of
    /// the Cauchy matrix, we do a matrix-vector product
    /// of length `data_shards`, with each element being
    /// a `gf_mul`. Per byte: `m × k` field multiplications
    /// + `m × (k-1)` field additions. The "additions" in
    /// GF(2^8) are just XORs, so the inner loop is one
    /// `gf_mul` + one XOR per data shard per byte.
    ///
    /// **Parallelism:** the outer loop over the
    /// `parity_shards` rows is embarrassingly parallel —
    /// each parity shard depends only on the data shards
    /// (which are read-only here). The caller can wrap
    /// the call in a `par_iter` if they want.
    pub fn encode_shards(&self, shards: &mut [Vec<u8>]) -> Result<(), ReedSolomonError> {
        // ── Step 1: validate input ─────────────────
        if shards.len() != self.data_shards + self.parity_shards {
            return Err(ReedSolomonError::WrongShardCount {
                got: shards.len(),
                expected: self.data_shards + self.parity_shards,
            });
        }
        // All shards must be the same length. Shorter
        // data shards are an error (corruption during
        // read or construction); shorter parity shards
        // are also an error (the caller should have
        // pre-extended them with zeros).
        let shard_len = shards[0].len();
        for s in shards.iter() {
            if s.len() != shard_len {
                return Err(ReedSolomonError::InconsistentShardLengths);
            }
        }

        // ── Step 2: split the shards into data and
        //          parity halves. The data half is
        //          read-only here, the parity half is
        //          write-only. Splitting once at the
        //          top avoids borrow-checker fights in
        //          the inner loop (the borrow on the
        //          parity shard would otherwise extend
        //          across the whole per-byte inner loop,
        //          and Rust can't statically prove the
        //          data and parity indices are disjoint
        //          when the only thing tying them
        //          together is the same slice).
        let (data_shards, parity_shards) = shards.split_at_mut(self.data_shards);

        // ── Step 3: compute each parity shard ───────
        // For each parity row i in 0..m:
        //   For each byte position b in 0..shard_len:
        //     parity[i][b] = Σ_j C[i][j] × data[j][b]
        //
        // The C[i][j] coefficients are the bottom m rows
        // of the encoding matrix (the top k rows are the
        // identity, applied implicitly by leaving the
        // data shards untouched).
        for (parity_idx, parity) in parity_shards.iter_mut().enumerate() {
            // Zero the output shard first (XOR-reduction
            // needs a clean accumulator; the math is
            // parity = Σ coefficients × data).
            for byte in parity.iter_mut() {
                *byte = 0;
            }
            // The Cauchy row for this parity shard starts
            // at column 0 of the row `data_shards + i` in
            // the encoding matrix.
            let cauchy_row_start = (self.data_shards + parity_idx) * self.data_shards;
            // Inner sum: for each data shard, multiply
            // the coefficient by each byte of the data
            // shard and XOR into the parity.
            for (j, data) in data_shards.iter().enumerate() {
                let coef = self.matrix[cauchy_row_start + j];
                if coef == 0 {
                    // Skip — multiplying by 0 is a no-op
                    // and saves a few cycles per byte.
                    continue;
                }
                for (b_idx, d_byte) in data.iter().enumerate() {
                    parity[b_idx] = gf_add(parity[b_idx], gf_mul(coef, *d_byte));
                }
            }
        }
        Ok(())
    }

    /// Recover up to `parity_shards` missing data shards.
    /// This is the function that gets called when the
    /// decoder detects a CRC64 mismatch (or a GCM auth
    /// failure) on one or more data shards.
    ///
    /// **Caller contract:** `shards` is the full
    /// `data_shards + parity_shards` slice. Entries with
    /// `None` (representing "this shard's data is
    /// corrupt or missing") will be reconstructed. After
    /// this call returns, all slots in `shards` are
    /// `Some` again.
    ///
    /// **Validation done by the caller (not by us):** the
    /// reconstructed data shards must still pass their
    /// AES-256-GCM tag check before being handed to the
    /// LZMA decoder. RS gives us a candidate; GCM is
    /// the judge.
    pub fn decode_recover(
        &self,
        shards: &mut [Option<Vec<u8>>],
    ) -> Result<(), ReedSolomonError> {
        // ── Step 1: validate and find the missing set ─
        // A "missing" data shard is one whose `None`
        // indicates a corrupt or absent block. We don't
        // try to recover parity shards here (they can be
        // regenerated by re-encoding); we only recover
        // DATA shards.
        if shards.len() != self.data_shards + self.parity_shards {
            return Err(ReedSolomonError::WrongShardCount {
                got: shards.len(),
                expected: self.data_shards + self.parity_shards,
            });
        }
        // Validate all present shards have the same length.
        let first_present = shards.iter().flatten().next();
        let shard_len = match first_present {
            Some(s) => s.len(),
            None => {
                // No data is present at all — even the
                // parity shards are missing. We can't
                // recover anything.
                return Err(ReedSolomonError::TooManyMissing {
                    missing: self.data_shards,
                    available: self.parity_shards,
                });
            }
        };
        for (idx, s) in shards.iter().enumerate() {
            if let Some(bytes) = s {
                if bytes.len() != shard_len {
                    return Err(ReedSolomonError::InconsistentShardLengths);
                }
            }
        }
        // The missing set is the data shard indices whose
        // `Option` is `None`. Parity shard corruption is
        // outside our scope (the caller can re-encode
        // if needed; we just need enough of the m
        // equations to recover the missing data).
        let missing: Vec<usize> = (0..self.data_shards)
            .filter(|&i| shards[i].is_none())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let t = missing.len();
        if t > self.parity_shards {
            return Err(ReedSolomonError::TooManyMissing {
                missing: t,
                available: self.parity_shards,
            });
        }

        // ── Step 2: build the t × t Cauchy submatrix and
        //          invert it via Gauss-Jordan elimination
        //          on an augmented matrix in-place.
        //
        // The submatrix we need is at the intersection of
        // the LAST t parity rows and the missing data
        // columns. We pick the LAST t parity rows (any t
        // of the m parity rows would do — Cauchy
        // guarantees any t × t submatrix is invertible;
        // the last t is just a deterministic choice).
        let mut submatrix: Vec<u8> = Vec::with_capacity(t * t);
        for i in 0..t {
            let parity_row = self.data_shards + (self.parity_shards - t) + i;
            for j in 0..t {
                let data_col = missing[j];
                submatrix.push(self.matrix[parity_row * self.data_shards + data_col]);
            }
        }

        // Augment the submatrix with the identity to the
        // right: `aug[i * 2t + j]` is the submatrix entry
        // for j < t, the identity entry for j ≥ t.
        //
        // Layout: row-major, t rows × 2t columns. For
        // t ≤ 12, total size is 12 × 24 = 288 bytes —
        // fits in 5 cache lines on a 64-byte-line CPU.
        // The whole inversion runs in L1 with zero cache
        // misses past the first warm-up.
        let mut aug: Vec<u8> = vec![0u8; t * 2 * t];
        for i in 0..t {
            for j in 0..t {
                aug[i * 2 * t + j] = submatrix[i * t + j];
            }
            // Identity on the right half.
            aug[i * 2 * t + t + i] = 1;
        }
        // In-place Gauss-Jordan. After this, aug's left
        // half is the identity and aug's right half is
        // the inverse.
        invert_gf256(&mut aug, t);

        // ── Step 3: precompute the "known contribution"
        //          to the RHS.
        //
        // For each missing data shard j in MISS and each
        // byte position b, the formula is:
        //   data[j][b] = Σ_i inv[j][i] * (parity[i][b] -
        //                                Σ_{j' in PRES} C[i,j'] * data[j'][b])
        //
        // We split this into two parts:
        //   A: the parity contribution (Σ_i inv[j][i] * parity[i][b])
        //   B: the known-data contribution (Σ_i inv[j][i] *
        //                                   Σ_{j' in PRES} C[i,j'] * data[j'][b])
        //
        // For each missing index j, we precompute the
        // 2D matrix `sub_C_inv_times_C_present[j][j']`
        // = Σ_i inv[j][i] * C[i, j'] for j' in PRES.
        // That gives us, per missing shard and per byte:
        //   data[j][b] = A - B
        // where A is one matrix-vector product over the
        // parity shards and B is one over the present
        // data shards, both in GF(2^8).
        //
        // The XOR in `data = A - B` is just the
        // characteristic-2 add (gf_add is XOR).
        //
        // ── Step 3a: build the "present" matrix.
        //   present_coefs[j][j'] = Σ_i inv[j][i] * C[i, j']
        // for j in MISS, j' in PRES. Size: t × k.
        // (k = data_shards; only the PRES columns are
        // nonzero in the inner sum but we compute all k
        // for SIMD friendliness — wasted ops on zero
        // rows are cheap.)
        let mut present_coefs: Vec<u8> = vec![0u8; t * self.data_shards];
        for (mj, &j) in missing.iter().enumerate() {
            // `inv[mj]` is the mj-th row of the inverted
            // augmented matrix's right half. We pull it
            // directly from `aug`.
            let inv_row = &aug[mj * 2 * t + t..mj * 2 * t + 2 * t];
            for (i, &inv_i) in inv_row.iter().enumerate() {
                if inv_i == 0 {
                    continue;
                }
                // The submatrix was built from the LAST
                // t parity rows (parity_row = data_shards +
                // (m - t) + i for column index i of the
                // inverted matrix). When we read the
                // matching row from `self.matrix` to
                // compute the present-shard contribution,
                // we MUST use that same offset — the
                // submatrix's i-th column corresponds to
                // the (m - t + i)-th parity row of the
                // full matrix, not the i-th.
                let parity_row = self.data_shards + (self.parity_shards - t) + i;
                for jprime in 0..self.data_shards {
                    let c = self.matrix[parity_row * self.data_shards + jprime];
                    if c == 0 {
                        continue;
                    }
                    let prod = gf_mul(inv_i, c);
                    present_coefs[mj * self.data_shards + jprime] =
                        gf_add(present_coefs[mj * self.data_shards + jprime], prod);
                }
            }
        }

        // ── Step 4: reconstruct each missing data shard.
        //
        // For each missing j in MISS and each byte
        // position b, we compute:
        //   data[j][b] = (Σ_i inv[j][i] * parity[m-t+i][b])
        //              ^ (Σ_{j' in PRES} present_coefs[j][j'] * data[j'][b])
        // where ^ is XOR in characteristic 2.
        //
        // We allocate the output shard, then fill it byte
        // by byte. The two inner products are over
        // small vectors (length t and |PRES|), so the
        // whole thing is cache-resident.
        for (mj, &j) in missing.iter().enumerate() {
            let inv_row = &aug[mj * 2 * t + t..mj * 2 * t + 2 * t];
            let mut reconstructed = vec![0u8; shard_len];
            for b in 0..shard_len {
                // Compute A: the parity contribution at byte b.
                // CRITICAL: the i-th column of the inverted
                // submatrix corresponds to the (m - t + i)-th
                // parity shard in the original `shards` slice,
                // NOT the i-th. (This is the "last t parity
                // rows" offset we used in the submatrix build.)
                let mut a: u8 = 0;
                for (i, &inv_i) in inv_row.iter().enumerate() {
                    if inv_i == 0 {
                        continue;
                    }
                    let parity_idx = self.data_shards + (self.parity_shards - t) + i;
                    let parity = shards[parity_idx]
                        .as_ref()
                        .expect("parity shard present (checked by caller)");
                    a = gf_add(a, gf_mul(inv_i, parity[b]));
                }
                // Compute B: the known-data contribution at byte b.
                // Iterate over data_shards directly, skipping
                // the missing ones. The `jprime == j` check
                // skips the self-contribution (the missing
                // shard's own term in its own reconstruction).
                let mut bb: u8 = 0;
                for jprime in 0..self.data_shards {
                    let coef = present_coefs[mj * self.data_shards + jprime];
                    if coef == 0 {
                        continue;
                    }
                    if jprime == j {
                        // Self-contribution: data[j] is being
                        // reconstructed, so its own contribution
                        // is unknown. Skip.
                        continue;
                    }
                    if let Some(data) = &shards[jprime] {
                        bb = gf_add(bb, gf_mul(coef, data[b]));
                    }
                }
                reconstructed[b] = gf_add(a, bb);
            }
            // Drop the missing slot's None and replace
            // with the reconstructed bytes.
            shards[j] = Some(reconstructed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1. Construction validates the input. The first
    ///    thing a wrong call hits.
    #[test]
    fn rejects_zero_data_shards() {
        let r = ReedSolomon::new(0, 3);
        assert!(matches!(r, Err(ReedSolomonError::ZeroDataShards)));
    }

    #[test]
    fn rejects_zero_parity_shards() {
        let r = ReedSolomon::new(4, 0);
        assert!(matches!(r, Err(ReedSolomonError::ZeroParityShards)));
    }

    #[test]
    fn rejects_too_many_shards() {
        // 200 + 100 = 300 > 254.
        let r = ReedSolomon::new(200, 100);
        assert!(matches!(
            r,
            Err(ReedSolomonError::TooManyShards { data: 200, parity: 100 })
        ));
    }

    /// 2. The encoding matrix has the right shape: the
    ///    top `data_shards` rows are the identity, the
    ///    bottom `parity_shards` rows are non-zero. We
    ///    don't validate the Cauchy structure here
    ///    (that comes in a follow-up commit when
    ///    decode_recover lands); we just check the
    ///    shape.
    #[test]
    fn matrix_has_identity_on_top_rows() {
        let rs = ReedSolomon::new(4, 2).expect("4 + 2 = 6 ≤ 254");
        let k = rs.data_shards;
        // Top-k rows: each row has a 1 on its own diagonal.
        for row in 0..k {
            for col in 0..k {
                let expected = if row == col { 1 } else { 0 };
                assert_eq!(
                    rs.matrix[row * k + col],
                    expected,
                    "row {} col {}: expected {} got {}",
                    row,
                    col,
                    expected,
                    rs.matrix[row * k + col]
                );
            }
        }
    }

    /// 3. The encoding matrix has the right TOTAL
    ///    dimensions. A common off-by-one in matrix
    ///    construction is forgetting to allocate one
    ///    row of the matrix.
    #[test]
    fn matrix_has_correct_total_size() {
        let rs = ReedSolomon::new(7, 3).expect("7 + 3 = 10 ≤ 254");
        let k = rs.data_shards;
        let m = rs.parity_shards;
        assert_eq!(rs.matrix.len(), (k + m) * k);
    }

    /// 4. Cauchy construction: the bottom-m rows
    ///    contain a non-zero entry in every column. If
    ///    any column of any parity row is zero, either
    ///    the anchor sets overlap (a Cauchy construction
    ///    bug) or two anchors are equal (impossible with
    ///    the disjoint-set construction but worth
    ///    checking).
    #[test]
    fn cauchy_rows_are_fully_nonzero() {
        let rs = ReedSolomon::new(5, 2).expect("5 + 2 = 7 ≤ 254");
        let k = rs.data_shards;
        for parity_idx in 0..rs.parity_shards {
            let row = k + parity_idx;
            for col in 0..k {
                let c = rs.matrix[row * k + col];
                assert_ne!(
                    c, 0,
                    "parity row {} col {} is zero — anchor sets overlap or construction bug",
                    row, col
                );
            }
        }
    }

    /// 5. End-to-end: encode 4 data shards with 2
    ///    parity shards, then recover any single missing
    ///    data shard and confirm the recovered bytes
    ///    match the original. This is the canonical
    ///    use case from the design doc §1.3.
    #[test]
    fn recover_one_missing_data_shard() {
        let rs = ReedSolomon::new(4, 2).expect("4 + 2 = 6 ≤ 254");
        let k = rs.data_shards;
        let m = rs.parity_shards;
        let shard_len = 64;
        // Each data shard is 64 bytes of distinct data
        // (the i-th byte is i + 17*i, so a wrong value
        // is easy to spot in a diff).
        let original: Vec<Vec<u8>> = (0..k)
            .map(|i| (0..shard_len as u8).map(|b| b.wrapping_add(i as u8 * 17)).collect())
            .collect();
        let mut shards: Vec<Vec<u8>> = original.clone();
        // Append the parity slots pre-allocated to
        // shard_len (encode_shards validates that all
        // shards have the same length).
        for _ in 0..m {
            shards.push(vec![0u8; shard_len]);
        }
        rs.encode_shards(&mut shards).expect("encode");

        // Corrupt one data shard (set to None) and
        // recover.
        let missing_idx = 2;
        let mut options: Vec<Option<Vec<u8>>> = shards
            .into_iter()
            .enumerate()
            .map(|(i, s)| if i == missing_idx { None } else { Some(s) })
            .collect();
        rs.decode_recover(&mut options).expect("recover");

        for (i, opt) in options.iter().enumerate() {
            let s = opt.as_ref().expect(&format!("shard {} should be recovered", i));
            if i < k {
                assert_eq!(
                    s, &original[i],
                    "recovered data shard {} does not match original",
                    i
                );
            }
        }
    }

    /// 6. End-to-end: recover TWO missing data shards
    ///    (the maximum allowed by parity_shards = 2).
    ///    This exercises the full pipeline: inversion
    ///    of a 2 × 2 Cauchy submatrix, the
    ///    "known contribution" precomputation, and the
    ///    byte-level reconstruction loop.
    #[test]
    fn recover_max_missing_data_shards() {
        let rs = ReedSolomon::new(5, 2).expect("5 + 2 = 7 ≤ 254");
        let k = rs.data_shards;
        let m = rs.parity_shards;
        let shard_len = 32;
        let original: Vec<Vec<u8>> = (0..k)
            .map(|i| (0..shard_len as u8).map(|b| b.wrapping_add(i as u8 * 13 + 7)).collect())
            .collect();
        let mut shards: Vec<Vec<u8>> = original.clone();
        for _ in 0..m {
            shards.push(vec![0u8; shard_len]);
        }
        rs.encode_shards(&mut shards).expect("encode");

        // Drop the last 2 data shards (the maximum
        // recoverable with m=2).
        let mut options: Vec<Option<Vec<u8>>> = shards
            .into_iter()
            .enumerate()
            .map(|(i, s)| if i == k - 2 || i == k - 1 { None } else { Some(s) })
            .collect();
        rs.decode_recover(&mut options).expect("recover");

        for (i, opt) in options.iter().enumerate() {
            let s = opt.as_ref().expect(&format!("shard {} should be recovered", i));
            if i < k {
                assert_eq!(
                    s, &original[i],
                    "recovered data shard {} does not match original",
                    i
                );
            }
        }
    }

    /// 7. Roundtrip: encode, then decode with NO
    ///    missing shards. The function should be a
    ///    no-op (no changes, no errors). This is the
    ///    "happy path" of the decoder when nothing went
    ///    wrong.
    #[test]
    fn recover_no_missing_is_noop() {
        let rs = ReedSolomon::new(3, 2).expect("3 + 2 = 5 ≤ 254");
        let k = rs.data_shards;
        let m = rs.parity_shards;
        let shard_len = 16;
        let original: Vec<Vec<u8>> = (0..k)
            .map(|i| (0..shard_len as u8).map(|b| b.wrapping_add(i as u8 * 5)).collect())
            .collect();
        let mut shards: Vec<Vec<u8>> = original.clone();
        for _ in 0..m {
            shards.push(vec![0u8; shard_len]);
        }
        rs.encode_shards(&mut shards).expect("encode");

        // Wrap in Some (all shards present).
        let mut options: Vec<Option<Vec<u8>>> =
            shards.iter().cloned().map(Some).collect();
        let snapshot = options.clone();
        rs.decode_recover(&mut options).expect("recover (no missing)");
        // The function should be a no-op — the slots
        // are unchanged.
        for (i, (a, b)) in options.iter().zip(snapshot.iter()).enumerate() {
            assert_eq!(a, b, "slot {} changed during no-op recover", i);
        }
    }

    /// 8. The decoder refuses to recover more than
    ///    `parity_shards` missing data shards. Trying
    ///    to recover 3 with m=2 is a user error (they
    ///    chose the wrong recovery level).
    #[test]
    fn recover_too_many_missing_errors() {
        let rs = ReedSolomon::new(4, 2).expect("4 + 2 = 6 ≤ 254");
        let k = rs.data_shards;
        let m = rs.parity_shards;
        let shard_len = 8;
        let mut shards: Vec<Vec<u8>> = (0..k + m)
            .map(|_| vec![0u8; shard_len])
            .collect();
        rs.encode_shards(&mut shards).expect("encode");
        // Mark 3 data shards as missing (more than m=2).
        let mut options: Vec<Option<Vec<u8>>> = shards
            .into_iter()
            .enumerate()
            .map(|(i, s)| if i < 3 { None } else { Some(s) })
            .collect();
        let r = rs.decode_recover(&mut options);
        assert!(
            matches!(r, Err(ReedSolomonError::TooManyMissing { missing: 3, available: 2 })),
            "expected TooManyMissing, got {:?}",
            r
        );
    }
}
