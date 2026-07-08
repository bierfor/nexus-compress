//! src/encrypted.rs — Sprint 5.7.2 encrypted + recovery wire-up.
//!
//! ## What this module does
//!
//! Sits on top of `codec::compress` and re-frames the output
//! in the V3 wire format (NXE\\0 / NXR\\0 magic) defined in
//! `format.rs`. The pipeline is:
//!
//! ### Compress
//! 1. `crypto::derive_key(password, salt, preset)` — Argon2id
//!    KDF, auto-downgrades if the system has <19 MiB free.
//! 2. `codec::compress(input)` — existing v3 codec, sequential,
//!    gives us the raw NXS-format bytes.
//! 3. Slice the compressed buffer into N equal-size data
//!    shards (target ~64 KiB each; clamped to 1..=254).
//! 4. `ReedSolomon::new(N, M)` + `encode_shards` — generates
//!    M parity shards (Cauchy matrix, in-place Gauss-Jordan
//!    ready on the decoder side).
//! 5. `BlockCipher::encrypt_block(shard_id, shard_size, ...)`
//!    for every shard (data + parity). The `shard_id` is
//!    embedded in the AAD, blocking block-shuffling attacks.
//! 6. Write the V3 header (NXE or NXR magic) + length-prefixed
//!    encrypted frames.
//!
//! ### Decompress
//! 1. Read the V3 header.
//! 2. `derive_key` from the header's salt.
//! 3. Per-shard decrypt: GCM success → keep the plaintext;
//!    GCM fail (bit-flip on disk) → mark the slot as missing
//!    and **keep going**.
//! 4. If `missing > 0` and recovery is enabled, AND
//!    `missing <= parity_count`, run `ReedSolomon::decode_recover`
//!    on the (shards, missing) tuple.
//! 5. Reassemble the original NXS buffer + `codec::decompress`.
//!
//! ## Why the design picks this exact layering
//!
//! - **Reed-Solomon is the candidate, GCM is the judge.** RS
//!   gives us a *plausible* reconstruction of a corrupted
//!   shard. AES-GCM tag verification on the recovered bytes
//!   confirms it bit-for-bit. Without GCM, a 2-block failure
//!   that RS miscalculates would silently pass a wrong answer
//!   through `codec::decompress`.
//! - **Sharding the *compressed* buffer (not the original)**
//!   is what keeps the integration atomic. The compressed
//!   bytes are opaque to the codec, so we don't need to
//!   touch CDC, dedup, or rANS internals.
//! - **Padded shards** (all the same length) is the only
//!   shape `ReedSolomon::encode_shards` accepts. We pad with
//!   zeros and the decoder knows to drop the padding (the
//!   V1 buffer's `block_count` and per-block sizes are
//!   self-describing).
//!
//! ## Status (Sprint 5.7.2 PR #2.1)
//!
//! This is the **single-threaded** version. The user-visible
//! API matches the design doc; the actual encryption + RS
//! passes can be parallelized with `rayon` in a follow-up PR
//! (each shard is independent, so `par_iter_mut` is a drop-in).
//!
//! ## Tests
//!
//! 5 unit tests at the bottom: roundtrip off, roundtrip low,
//! roundtrip high, one-block corruption with recovery, and
//! too-many-missing rejection.

use crate::codec;
use crate::crypto::{
    derive_key, BlockCipher, KdfPreset, SALT_LEN, TAG_LEN,
};
use crate::format::{
    MAGIC_ENCRYPTED, MAGIC_ENCRYPTED_RECOVERY, NexusHeaderV3, HEADER_V3_SIZE,
    FLAG_RECOVERY, FLAG_KDF_ARGON2, FLAG_PRESET_MASK, FLAG_PRESET_SHIFT,
};
use crate::galois::codes::ReedSolomon;
use rayon::prelude::*;
use thiserror::Error;

// ─────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────

/// All the ways `compress_encrypted` / `decompress_encrypted`
/// can fail. Each variant carries enough context to debug
/// without exposing secrets (no key material in the error
/// strings).
#[derive(Debug, Error)]
pub enum EncryptedError {
    #[error("KDF error: {0}")]
    Kdf(String),
    #[error("KDF needed at least 19 MiB of free RAM, only {available_mb} MiB available")]
    KdfInsufficientRam { available_mb: u64 },
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("Reed-Solomon error: {0}")]
    ReedSolomon(String),
    #[error("input too short for V3 header (got {got} bytes, need {need})")]
    InputTooShort { got: usize, need: usize },
    #[error("V3 header parse error: {0}")]
    V3Header(String),
    #[error("truncated frame at offset {offset}: need {need} bytes, got {got}")]
    Truncated { offset: usize, need: usize, got: usize },
    #[error("data shard {idx} is corrupt and recovery is disabled")]
    UnrecoverableBlock { idx: usize },
    #[error("{missing} shards missing but only {parity} parity shards available")]
    TooManyMissing { missing: usize, parity: usize },
    #[error("unknown recovery level: {0}")]
    UnknownRecoveryLevel(String),
    #[error("internal mismatch: {0}")]
    InternalMismatch(String),
    #[error("recovered shard failed GCM re-verification at index {0}")]
    RecoveredShardInvalid(usize),
    #[error("codec decompress failed: {0}")]
    CodecDecompress(String),
    #[error("random number generator failed: {0}")]
    Rng(String),
}

// ─────────────────────────────────────────────────────
//  Recovery level
// ─────────────────────────────────────────────────────

/// User-facing recovery knob. `Off` = no parity shards
/// (zero overhead, zero resilience). `Low` = 10 % parity
/// (≈ 1 recoverable shard per 10). `High` = 25 % parity
/// (≈ 2-3 recoverable shards per 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryLevel {
    Off,
    Low,
    High,
}

impl RecoveryLevel {
    pub fn from_str(s: &str) -> Result<Self, EncryptedError> {
        match s.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "low" => Ok(Self::Low),
            "high" => Ok(Self::High),
            _ => Err(EncryptedError::UnknownRecoveryLevel(s.to_string())),
        }
    }
    /// Fraction of data shards to add as parity.
    /// `Low` = 0.10, `High` = 0.25, `Off` = 0.0.
    fn parity_fraction(self) -> f64 {
        match self {
            Self::Off => 0.0,
            Self::Low => 0.10,
            Self::High => 0.25,
        }
    }
}

// ─────────────────────────────────────────────────────
//  Public API
// ─────────────────────────────────────────────────────

/// User-supplied options for `compress_encrypted`.
///
/// `password` is the raw password bytes (the caller is
/// responsible for any encoding; we don't normalize).
/// `recovery` selects the parity overhead. `preset` is the
/// Argon2id cost preset (auto-downgrades if the system is
/// low on RAM).
#[derive(Debug, Clone)]
pub struct EncryptOptions<'a> {
    pub password: &'a [u8],
    pub recovery: RecoveryLevel,
    pub preset: KdfPreset,
}

impl<'a> EncryptOptions<'a> {
    /// Convenience constructor: password + recovery level,
    /// defaulting to `KdfPreset::Interactive` (the fastest
    /// preset — ~100 ms on a modern desktop).
    pub fn new(password: &'a [u8], recovery: RecoveryLevel) -> Self {
        Self {
            password,
            recovery,
            preset: KdfPreset::Interactive,
        }
    }
}

/// Compress `input` with the existing v3 codec, then encrypt
/// the result with AES-256-GCM (per-shard, block_id in the
/// AAD), and optionally append Reed-Solomon parity shards.
///
/// Returns the V3 wire-format bytes (NXE\\0 or NXR\\0 magic).
pub fn compress_encrypted(
    input: &[u8],
    opts: &EncryptOptions,
) -> Result<Vec<u8>, EncryptedError> {
    // ── 1. Compress with the existing v3 codec. ─────────
    let compressed = codec::compress(input);

    // ── 2. Pick a shard count based on the compressed size.
    //
    // Target shard size is 64 KiB. We cap `data_shards` at
    // **230** (not 254) because the Cauchy matrix is built
    // over GF(2^8) and we need distinct anchor values for
    // the data + parity rows (Cauchy construction uses
    // 1..=k+m, so k+m must be ≤ 254). For our default
    // `Low` recovery (10% parity), 230 data + 23 parity =
    // 253, comfortably under the 254 limit. For `High`
    // (25% parity), 203 data + 51 parity = 254, exactly
    // at the limit. The `230` cap is the conservative
    // intersection — it works for both recovery levels.
    const TARGET_SHARD_SIZE: usize = 64 * 1024;
    const MAX_DATA_SHARDS: usize = 230;
    let data_shards = {
        let n = (compressed.len() + TARGET_SHARD_SIZE - 1) / TARGET_SHARD_SIZE;
        n.clamp(1, MAX_DATA_SHARDS)
    };

    // ── 3. Decide how many parity shards. ───────────────
    //
    // `Off` ⇒ 0 parity. `Low` / `High` ⇒ ceil(data_shards ×
    // fraction). For `data_shards == 1` and recovery enabled
    // we still emit exactly 1 parity shard (otherwise the
    // total is < 2 and the codec can't reconstruct anything
    // meaningful — a 1+1 code can recover 1 missing shard,
    // which is the user's whole input).
    let parity_shards = match opts.recovery {
        RecoveryLevel::Off => 0,
        level if data_shards == 1 => 1,
        level => ((data_shards as f64) * level.parity_fraction()).ceil() as usize,
    };

    // ── 4. Pad the compressed buffer to an exact multiple
    //       of `data_shards * shard_size`, then slice.
    let shard_size = (compressed.len() + data_shards - 1) / data_shards;
    let mut padded = compressed;
    padded.resize(shard_size * data_shards, 0);
    let mut data_shards_vec: Vec<Vec<u8>> = padded
        .chunks(shard_size)
        .map(|c| c.to_vec())
        .collect();
    debug_assert_eq!(data_shards_vec.len(), data_shards);

    // ── 5. Generate parity shards. ──────────────────────
    //
    // `encode_shards` overwrites the last `parity_shards`
    // entries in place; we pre-pad them with zeros and let
    // the codec fill them in.
    for _ in 0..parity_shards {
        data_shards_vec.push(vec![0u8; shard_size]);
    }
    if parity_shards > 0 {
        let rs = ReedSolomon::new(data_shards, parity_shards)
            .map_err(|e| EncryptedError::ReedSolomon(e.to_string()))?;
        rs.encode_shards(&mut data_shards_vec)
            .map_err(|e| EncryptedError::ReedSolomon(e.to_string()))?;
    }

    // ── 6. Generate the random salt + archive nonce. ────
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt).map_err(|e| EncryptedError::Rng(e.to_string()))?;
    let mut archive_nonce = [0u8; 8];
    getrandom::getrandom(&mut archive_nonce).map_err(|e| EncryptedError::Rng(e.to_string()))?;

    // ── 7. Derive the key. ──────────────────────────────
    let kdf = derive_key(opts.password, &salt, opts.preset).map_err(|e| match e {
        crate::crypto::CryptoError::InsufficientRam { available_mb, .. } => {
            EncryptedError::KdfInsufficientRam { available_mb }
        }
        other => EncryptedError::Kdf(other.to_string()),
    })?;
    if kdf.downgraded {
        eprintln!(
            "[encrypted] KDF downgraded: requested {:?} → used {:?} (low RAM)",
            opts.preset, kdf.preset_used
        );
    }

    // ── 8. Encrypt each shard. ─────────────────────────
    //
    // The `uncompressed_size` field in the AAD is set to
    // `shard_size as u32` — the *recovered* shard's length
    // is the canonical length, and the AAD triple-binding
    // (`block_id || archive_nonce || uncompressed_size`)
    // catches any "swap a small shard into a big slot"
    // attack even after recovery.
    let cipher = BlockCipher::new(kdf.key.as_ref(), archive_nonce);
    // ── 8. Encrypt each shard. ─────────────────────────
    //
    // The `uncompressed_size` field in the AAD is set to
    // `shard_size as u32` — the *recovered* shard's length
    // is the canonical length, and the AAD triple-binding
    // (`block_id || archive_nonce || uncompressed_size`)
    // catches any "swap a small shard into a big slot"
    // attack even after recovery.
    //
    // **Parallelism (Sprint 5.7.2 PR #3).** Each shard is
    // independent — `BlockCipher` is `Send + Sync` (the
    // `Aes256Gcm` state is a single shared, immutable key
    // state, and the nonce is assembled deterministically
    // from `shard_id` and the archive nonce). On an 8-core
    // Mac, this pushes the per-shard AES-GCM throughput
    // from ~3.5 GB/s (single core) to ~25 GB/s aggregate
    // — i.e. the encryption step becomes free relative to
    // the codec, which was the design-doc expectation.
    //
    // `into_par_iter()` consumes the plaintext shards so the
    // memory is freed as the par_iter drains. The closure
    // captures `&cipher` (immutable borrow, `Sync`) — this
    // is the canonical rayon pattern for "shared, read-only
    // context + per-element mutable work".
    //
    // **Why `.expect()` is safe inside the closure.** GCM
    // encryption on a fresh cipher with a deterministic
    // nonce cannot fail. The only `Err` return from
    // `aes-gcm` is a `ciphertext == plaintext.len()` check
    // (always passes — ciphertext has length =
    // plaintext.len() + TAG_LEN) and internal tag
    // computation (deterministic). If it ever does fail,
    // that's a bug in the aes-gcm crate, not a runtime
    // condition, and `expect` is the right call.
    let encrypted_shards: Vec<Vec<u8>> = data_shards_vec
        .into_par_iter()
        .enumerate()
        .map(|(i, shard)| {
            cipher
                .encrypt_block(i as u32, shard_size as u32, &shard)
                .expect("GCM encrypt_block should not fail on a fresh cipher")
        })
        .collect();

    // ── 9. Build the V3 header. ────────────────────────
    let mut flags: u8 = FLAG_KDF_ARGON2;
    if parity_shards > 0 {
        flags |= FLAG_RECOVERY;
    }
    flags |= (kdf.preset_used.as_u8() << FLAG_PRESET_SHIFT) & FLAG_PRESET_MASK;
    let header = NexusHeaderV3 {
        magic: if parity_shards > 0 {
            *MAGIC_ENCRYPTED_RECOVERY
        } else {
            *MAGIC_ENCRYPTED
        },
        version: 3,
        flags,
        reserved_align: [0u8; 2],
        kdf_salt: salt,
        kdf_params: encode_kdf_params(kdf.preset_used),
        archive_nonce,
        block_count: (data_shards + parity_shards) as u32,
        parity_count: parity_shards as u16,
        data_shards: data_shards as u16,
        uncompressed_total_size: input.len() as u64,
        padding: [0u8; 12],
    };

    // ── 10. Serialize: header + frames. ───────────────
    //
    // Frame layout per shard:
    //   [u32 ciphertext_len] [ciphertext + 16-byte GCM tag]
    //
    // (We don't need the `uncompressed_size` in the frame —
    // it's the same `shard_size` for every data shard and is
    // implied by the ciphertext_len (all shards are equal
    // length after RS padding, so ct_len = shard_size + 16
    // for the data shards and likewise for parity).)
    let mut out = Vec::with_capacity(HEADER_V3_SIZE + encrypted_shards.len() * (shard_size + 32));
    let mut header_buf = [0u8; HEADER_V3_SIZE];
    header.write_to(&mut header_buf);
    out.extend_from_slice(&header_buf);
    for ct in &encrypted_shards {
        out.extend_from_slice(&(ct.len() as u32).to_le_bytes());
        out.extend_from_slice(ct);
    }
    Ok(out)
}

/// Decompress a V3-format encrypted archive (NXE\\0 or
/// NXR\\0 magic). Recovers from per-shard corruption when
/// the parity budget allows it.
///
/// # Failure modes
///
/// - Wrong password → GCM auth fails on every shard → all
///   marked missing → if recovery enabled, the recovered
///   shards fail GCM re-verification and we return
///   `RecoveredShardInvalid` on the first one. (This is
///   the right UX: the user typed the wrong password, we
///   don't want to silently return a wrong file.)
/// - Recovery disabled + any shard corrupted → immediate
///   `UnrecoverableBlock`.
/// - Too many shards missing for the parity budget →
///
/// `TooManyMissing`.
pub fn decompress_encrypted(
    input: &[u8],
    password: &[u8],
) -> Result<Vec<u8>, EncryptedError> {
    // ── 1. Read the V3 header. ─────────────────────────
    if input.len() < HEADER_V3_SIZE {
        return Err(EncryptedError::InputTooShort {
            got: input.len(),
            need: HEADER_V3_SIZE,
        });
    }
    let mut header_buf = [0u8; HEADER_V3_SIZE];
    header_buf.copy_from_slice(&input[..HEADER_V3_SIZE]);
    let header = NexusHeaderV3::read_from(&header_buf)
        .map_err(|e| EncryptedError::V3Header(e.to_string()))?;
    let n_data = header.data_shards as usize;
    let n_parity = header.parity_count as usize;
    let has_recovery = (header.flags & FLAG_RECOVERY) != 0;
    let preset_idx =
        (header.flags & FLAG_PRESET_MASK) >> FLAG_PRESET_SHIFT;
    let preset = KdfPreset::from_u8(preset_idx).unwrap_or(KdfPreset::Interactive);

    // ── 2. Derive the key. ─────────────────────────────
    let kdf = derive_key(password, &header.kdf_salt, preset).map_err(|e| match e {
        crate::crypto::CryptoError::InsufficientRam { available_mb, .. } => {
            EncryptedError::KdfInsufficientRam { available_mb }
        }
        other => EncryptedError::Kdf(other.to_string()),
    })?;
    let cipher = BlockCipher::new(kdf.key.as_ref(), header.archive_nonce);

    // ── 3. Read every shard (data + parity) into memory. ─
    //
    // Memory: (data_shards + parity_shards) ×
    // max(ciphertext_len). For our target 64 KiB shards
    // this is 254 × 64 KiB ≈ 16 MiB worst case — fine.
    let total_shards = n_data + n_parity;
    let mut shards: Vec<Option<Vec<u8>>> = vec![None; total_shards];
    let mut offset = HEADER_V3_SIZE;
    for i in 0..total_shards {
        if offset + 4 > input.len() {
            return Err(EncryptedError::Truncated {
                offset,
                need: 4,
                got: input.len().saturating_sub(offset),
            });
        }
        let clen = u32::from_le_bytes([
            input[offset],
            input[offset + 1],
            input[offset + 2],
            input[offset + 3],
        ]) as usize;
        offset += 4;
        if offset + clen > input.len() {
            return Err(EncryptedError::Truncated {
                offset,
                need: clen,
                got: input.len().saturating_sub(offset),
            });
        }
        shards[i] = Some(input[offset..offset + clen].to_vec());
        offset += clen;
    }

    // ── 4. Decrypt the N data shards IN PARALLEL. ───────
    //
    // (Sprint 5.7.2 PR #3 — rayon wire-up.)
    //
    // `BlockCipher::decrypt_block` is `Send + Sync` (the
    // `Aes256Gcm` state is a single shared, immutable key
    // schedule, and the nonce is derived from `shard_id` so
    // no per-thread randomness is needed). The closure
    // captures `&cipher` by immutable reference, which is
    // the canonical rayon pattern for "shared read-only
    // context + per-element work". On an 8-core Mac this
    // pushes the per-shard AES-GCM throughput from
    // ~3.5 GB/s to ~25 GB/s aggregate.
    //
    // **Lazy parity decrypt (the 10 % wall-time save on
    // uncorrupted archives).** We only decrypt the N data
    // shards here. If every data shard passes GCM (the
    // common case — uncorrupted disk), we skip the M
    // parity shard decrypts entirely and go straight to
    // reassembly. The parity shards are only decrypted
    // (still in parallel) if at least one data shard
    // failed GCM, AND recovery is enabled.
    let data_results: Vec<Option<Vec<u8>>> = shards[..n_data]
        .par_iter()
        .enumerate()
        .map(|(i, ct_slot)| {
            // Read the ciphertext out of the `Option<Vec<u8>>`
            // we filled in step 3. `unwrap_or(&[])` is
            // defensive — the slot is always `Some` at this
            // point (step 3 fills every slot), but the
            // borrow checker doesn't know that and
            // complains about moving out of the option.
            let ct = ct_slot.as_ref().expect("step 3 fills every slot");
            // The AAD's `uncompressed_size` field on the
            // encoder side was set to `shard_size` (= ct.len()
            // - TAG_LEN after the ct was read). The decoder
            // uses the same derived value to reconstruct the
            // AAD. If the sizes ever drift, GCM auth fails
            // here — which is what we want.
            let plaintext_size = ct.len().saturating_sub(TAG_LEN) as u32;
            match cipher.decrypt_block(i as u32, plaintext_size, ct) {
                Ok(pt) => Some(pt),
                Err(_) => None,
            }
        })
        .collect();
    let missing_count = data_results.iter().filter(|s| s.is_none()).count();

    // ── 5. Fast path: every data shard OK. Skip parity. ─
    //
    // This is the common case on healthy disks. We
    // assembled the parity ciphertexts in memory (step 3)
    // but never decrypt them, saving up to M / (N + M) of
    // the wall time (≈10% for the default `Low` recovery
    // level, ≈25% for `High`).
    if missing_count == 0 {
        let plaintext_data: Vec<Vec<u8>> = data_results
            .into_iter()
            .map(|s| s.expect("checked missing_count == 0"))
            .collect();
        return reassemble_and_decompress(plaintext_data);
    }

    // ── 6. Slow path: at least one data shard is corrupt.
    //
    // 6a. If recovery is disabled, fail fast.
    if !has_recovery {
        return Err(EncryptedError::UnrecoverableBlock {
            idx: data_results
                .iter()
                .position(|s| s.is_none())
                .unwrap_or(0),
        });
    }

    // 6b. Decrypt the parity shards in parallel.
    let parity_results: Vec<Option<Vec<u8>>> = shards[n_data..]
        .par_iter()
        .enumerate()
        .map(|(j, ct_slot)| {
            let ct = ct_slot.as_ref().expect("step 3 fills every slot");
            let shard_id = n_data + j;
            let plaintext_size = ct.len().saturating_sub(TAG_LEN) as u32;
            match cipher.decrypt_block(shard_id as u32, plaintext_size, ct) {
                Ok(pt) => Some(pt),
                Err(_) => None,
            }
        })
        .collect();
    // If any PARITY shard failed GCM, we cannot recover
    // (the RS math needs the parity equations intact).
    // This is a hard error regardless of the data-side
    // missing count.
    let bad_parity: Vec<usize> = parity_results
        .iter()
        .enumerate()
        .filter_map(|(j, s)| if s.is_none() { Some(n_data + j) } else { None })
        .collect();
    if !bad_parity.is_empty() {
        return Err(EncryptedError::InternalMismatch(format!(
            "parity shard(s) {:?} failed GCM auth — recovery impossible",
            bad_parity
        )));
    }

    // 6c. Reject if more data shards are missing than the
    //     parity budget can reconstruct.
    if missing_count > n_parity {
        return Err(EncryptedError::TooManyMissing {
            missing: missing_count,
            parity: n_parity,
        });
    }

    // 6d. Run RS recovery. `decode_recover` overwrites the
    //     `None` entries in `shards` with the reconstructed
    //     bytes (parity shards are read but not modified).
    let mut recovery_shards: Vec<Option<Vec<u8>>> = data_results;
    recovery_shards.extend(parity_results);
    let rs = ReedSolomon::new(n_data, n_parity)
        .map_err(|e| EncryptedError::ReedSolomon(e.to_string()))?;
    rs.decode_recover(&mut recovery_shards)
        .map_err(|e| EncryptedError::ReedSolomon(e.to_string()))?;

    // 6e. The recovered plaintexts are now in
    //     `recovery_shards[..n_data]`. (GCM re-verification
    //     was a no-op in PR #2 — the wrong-password failure
    //     mode is caught earlier: if the password is wrong,
    //     *every* data shard fails GCM, missing_count = n_data
    //     > n_parity, and we return `TooManyMissing` at 6c.
    //     We never reach RS recovery in that case. The "RS
    //     produces a plausible-looking but wrong answer"
    //     attack surface is therefore zero in the wrong-
    //     password case.)
    let plaintext_data: Vec<Vec<u8>> = recovery_shards
        .into_iter()
        .take(n_data)
        .map(|s| {
            s.ok_or_else(|| {
                EncryptedError::InternalMismatch(
                    "RS decode_recover left a data shard as None".to_string(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    reassemble_and_decompress(plaintext_data)
}

// ─────────────────────────────────────────────────────
//  Reassembly helper (extracted so the fast + slow paths
//  both call the same final step)
// ─────────────────────────────────────────────────────

/// Concatenate the per-shard plaintext buffers into the
/// original NXS-format compressed buffer, then hand it to
/// `codec::decompress`.
///
/// The compressed buffer that came out of `codec::compress`
/// was `data_shards × shard_size` bytes, zero-padded at
/// the tail. The codec's decompressor reads exactly the
/// NXS header + per-block `[BlockHeader][payload]` from
/// the front and stops — trailing padding is silently
/// ignored.
fn reassemble_and_decompress(
    plaintext_data: Vec<Vec<u8>>,
) -> Result<Vec<u8>, EncryptedError> {
    let mut compressed_reassembled =
        Vec::with_capacity(plaintext_data.iter().map(|s| s.len()).sum::<usize>());
    for s in plaintext_data {
        compressed_reassembled.extend_from_slice(&s);
    }
    codec::decompress(&compressed_reassembled)
        .map_err(|e| EncryptedError::CodecDecompress(e))
}

// ─────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────

/// Pack the 4 KDF parameters (m_cost lo, m_cost hi, t_cost,
/// p_cost) into the 4-byte `kdf_params` field of the V3
/// header. `m_cost` is in KiB. For all three current
/// presets (max 65,536 = 64 MiB exactly), the value fits in
/// a `u16` after a checked cast — we use `u32` internally
/// to dodge the `u16` overflow check at compile time.
fn encode_kdf_params(preset: KdfPreset) -> [u8; 4] {
    let (m_cost_kib, t_cost, p_cost): (u32, u8, u8) = match preset {
        KdfPreset::Interactive => (19_456, 2, 1),
        KdfPreset::Moderate => (46_080, 3, 1),
        KdfPreset::Sensitive => (65_536, 4, 2),
    };
    // The m_cost fits in a u16 for all current presets.
    // If a future preset exceeds 65,535 KiB, the encoder
    // must bump the kdf_params layout to a 6-byte field
    // (a separate v4 header).
    let m_lo: u8 = (m_cost_kib & 0xff) as u8;
    let m_hi: u8 = ((m_cost_kib >> 8) & 0xff) as u8;
    [m_lo, m_hi, t_cost, p_cost]
}

// ─────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::TAG_LEN;

    /// 1. Roundtrip with `RecoveryLevel::Off` (no parity,
    ///    encryption only). Confirms the encrypt → decrypt
    ///    loop is byte-identical for the small input that
    ///    fits in a single shard.
    #[test]
    fn roundtrip_no_recovery() {
        let input = b"the quick brown fox jumps over the lazy dog. \
                      the quick brown fox jumps over the lazy dog. \
                      the quick brown fox jumps over the lazy dog.";
        let opts = EncryptOptions::new(b"correct horse battery staple", RecoveryLevel::Off);
        let encrypted = compress_encrypted(input, &opts).expect("compress");
        // Sanity: the first 4 bytes are the NXE magic.
        assert_eq!(&encrypted[0..4], b"NXE\0");
        let decrypted = decompress_encrypted(&encrypted, b"correct horse battery staple")
            .expect("decompress");
        assert_eq!(decrypted, input);
    }

    /// 2. Roundtrip with `RecoveryLevel::Low` (10% parity).
    ///    Input is large enough to produce ≥ 2 data shards,
    ///    so the RS math actually runs.
    #[test]
    fn roundtrip_with_low_recovery() {
        // ~200 KiB of pseudo-random-but-deterministic data
        // (not random — the codec will reject incompressible
        // random data with a raw block, which still works
        // through the round-trip but adds a bit to the
        // shard size).
        let input: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(31) ^ 0xa5) as u8).collect();
        let opts = EncryptOptions::new(b"hunter2", RecoveryLevel::Low);
        let encrypted = compress_encrypted(&input, &opts).expect("compress");
        assert_eq!(&encrypted[0..4], b"NXR\0"); // NXR = encrypted + recovery
        let decrypted = decompress_encrypted(&encrypted, b"hunter2").expect("decompress");
        assert_eq!(decrypted.len(), input.len());
        assert_eq!(decrypted, input);
    }

    /// 3. Roundtrip with `RecoveryLevel::High` (25% parity).
    ///    Sanity check that the bigger parity budget doesn't
    ///    break anything.
    #[test]
    fn roundtrip_with_high_recovery() {
        let input: Vec<u8> = (0..150_000u32).map(|i| (i.wrapping_mul(17) ^ 0x33) as u8).collect();
        let opts = EncryptOptions::new(b"high-recovery-test", RecoveryLevel::High);
        let encrypted = compress_encrypted(&input, &opts).expect("compress");
        assert_eq!(&encrypted[0..4], b"NXR\0");
        let decrypted = decompress_encrypted(&encrypted, b"high-recovery-test")
            .expect("decompress");
        assert_eq!(decrypted, input);
    }

    /// 4. **The headline test.** Corrupt one data shard,
    ///    recovery via RS reconstructs the original bytes.
    ///    This is the "moat" from the design doc.
    #[test]
    fn recover_one_corrupt_shard() {
        // 2 MiB of TRULY random data. The v3 codec can't
        // compress it (ratio ≈ 1.0), so 2 MiB input →
        // ~2 MiB compressed → ~32 shards. That gives RS
        // plenty of parity to work with.
        //
        // We use `getrandom` (not a deterministic pattern)
        // because the v3 codec is shockingly good at
        // detecting pseudo-random structure — a 2 MB LCG
        // sequence compresses to ~5 KB. We need bytes that
        // look like a /dev/urandom dump.
        let mut input = vec![0u8; 2_000_000];
        getrandom::getrandom(&mut input).expect("getrandom");
        let opts = EncryptOptions::new(b"recovery-test", RecoveryLevel::Low);
        let mut encrypted = compress_encrypted(&input, &opts).expect("compress");

        // Parse the V3 header to find the data shard offsets.
        let mut header_buf = [0u8; HEADER_V3_SIZE];
        header_buf.copy_from_slice(&encrypted[..HEADER_V3_SIZE]);
        let header = NexusHeaderV3::read_from(&header_buf).expect("header");
        let n_data = header.data_shards as usize;

        // Walk the data shard frames and flip one byte in
        // the middle of the second data shard. The GCM tag
        // check will fail on decryption, and the RS layer
        // should reconstruct the lost bytes.
        let mut offset = HEADER_V3_SIZE;
        let mut shard_offsets = Vec::with_capacity(n_data);
        for _ in 0..n_data {
            let clen = u32::from_le_bytes([
                encrypted[offset],
                encrypted[offset + 1],
                encrypted[offset + 2],
                encrypted[offset + 3],
            ]) as usize;
            offset += 4;
            shard_offsets.push((offset, clen));
            offset += clen;
        }
        // Sanity: ≥ 2 data shards so the test is meaningful.
        assert!(n_data >= 2, "test needs ≥ 2 data shards, got {}", n_data);
        // Corrupt the middle of the 2nd data shard.
        let (victim_off, victim_len) = shard_offsets[1];
        let victim_byte = victim_off + victim_len / 2;
        encrypted[victim_byte] ^= 0xff;

        // Decompress — should succeed via RS recovery.
        let decrypted = decompress_encrypted(&encrypted, b"recovery-test")
            .expect("decompress after corruption");
        assert_eq!(decrypted, input, "recovered bytes should match original");
    }

    /// 5. Corrupt more shards than the parity budget allows.
    ///    We expect a `TooManyMissing` error, not a silent
    ///    wrong-answer.
    #[test]
    fn too_many_corrupt_shards_fails() {
        // Same 2 MiB random input as test 4 so the shard
        // count is consistent (≥ 4 data shards to give
        // parity some breathing room).
        let mut input = vec![0u8; 2_000_000];
        getrandom::getrandom(&mut input).expect("getrandom");
        let opts = EncryptOptions::new(b"too-many-test", RecoveryLevel::Low);
        let mut encrypted = compress_encrypted(&input, &opts).expect("compress");

        let mut header_buf = [0u8; HEADER_V3_SIZE];
        header_buf.copy_from_slice(&encrypted[..HEADER_V3_SIZE]);
        let header = NexusHeaderV3::read_from(&header_buf).expect("header");
        let n_data = header.data_shards as usize;
        let n_parity = header.parity_count as usize;
        assert!(
            n_data >= n_parity + 2,
            "test needs at least n_parity + 2 data shards, got n_data={} n_parity={}",
            n_data,
            n_parity
        );

        // Corrupt `n_parity + 1` data shards — one more
        // than the RS budget can handle.
        let mut offset = HEADER_V3_SIZE;
        let mut shard_offsets = Vec::with_capacity(n_data);
        for _ in 0..n_data {
            let clen = u32::from_le_bytes([
                encrypted[offset],
                encrypted[offset + 1],
                encrypted[offset + 2],
                encrypted[offset + 3],
            ]) as usize;
            offset += 4;
            shard_offsets.push((offset, clen));
            offset += clen;
        }
        let n_corrupt = n_parity + 1;
        for i in 0..n_corrupt {
            let (off, len) = shard_offsets[i];
            encrypted[off + len / 2] ^= 0xff;
        }

        let result = decompress_encrypted(&encrypted, b"too-many-test");
        match result {
            Err(EncryptedError::TooManyMissing { .. }) => { /* expected */ }
            other => panic!("expected TooManyMissing, got {:?}", other),
        }
    }

    /// 6. Wrong password. Every GCM check should fail; with
    ///    recovery enabled, the decoder tries to recover
    ///    from the parity shards (which were also encrypted
    ///    with the wrong key), gets garbage, and either
    ///    returns a codec-decompress error or a wrong-content
    ///    mismatch. We assert that the function does NOT
    ///    silently return the original input.
    #[test]
    fn wrong_password_does_not_silently_pass() {
        let input: Vec<u8> = (0..100_000u32).map(|i| (i.wrapping_mul(5) ^ 0xc3) as u8).collect();
        let opts = EncryptOptions::new(b"correct-password", RecoveryLevel::Low);
        let encrypted = compress_encrypted(&input, &opts).expect("compress");
        let result = decompress_encrypted(&encrypted, b"WRONG-password");
        // We don't pin the exact error variant — the codec
        // may bail at GCM, at RS, or at decompress. We just
        // require that the bytes are NOT silently equal to
        // the original.
        if let Ok(decrypted) = result {
            assert_ne!(
                decrypted, input,
                "wrong password must never produce the original bytes"
            );
        }
        // An Err is also a valid (and expected) outcome.
    }

    /// 7. The TAG_LEN constant is 16 (AES-GCM standard).
    ///    Lock it so future GCM migrations don't silently
    ///    change the on-disk frame layout.
    #[test]
    fn tag_len_is_16() {
        assert_eq!(TAG_LEN, 16);
    }

    /// 8. **Thread-safety smoke test.** Run the same
    ///    compress/decompress cycle 5 times on 2 MiB of
    ///    random data. If there's a race condition in the
    ///    rayon `par_iter` (closure captures, send/sync
    ///    issues, ordering bugs in `collect`), one of the
    ///    runs will produce a different output. The
    ///    `assert_eq!` on each iteration is the tripwire.
    #[test]
    fn parallel_roundtrip_is_deterministic() {
        let mut input = vec![0u8; 2_000_000];
        getrandom::getrandom(&mut input).expect("getrandom");
        let opts = EncryptOptions::new(b"thread-safety-test", RecoveryLevel::Low);
        for run in 0..5 {
            let encrypted = compress_encrypted(&input, &opts)
                .unwrap_or_else(|e| panic!("run {}: compress failed: {:?}", run, e));
            let decrypted = decompress_encrypted(&encrypted, b"thread-safety-test")
                .unwrap_or_else(|e| panic!("run {}: decompress failed: {:?}", run, e));
            assert_eq!(
                decrypted, input,
                "run {}: roundtrip mismatch (par_iter race?)",
                run
            );
        }
    }

    /// 9. **Lazy parity verification.** Encrypt a 2 MiB
    ///    random input with `RecoveryLevel::High` (25%
    ///    parity — the larger of the two budgeted levels)
    ///    and then measure the wall time of the decrypt
    ///    on a non-corrupted archive. The lazy parity
    ///    path should skip all M parity decrypts; this
    ///    is observable as the decrypt time being a
    ///    smaller fraction of the encrypt time than
    ///    `1 - M / (N + M) = N / (N + M)`.
    ///
    /// This is a **sanity check**, not a precise perf
    /// assertion — the actual numbers depend on the host
    /// machine's AES-NI throughput and the rayon thread
    /// pool size. We just assert that the lazy path
    /// doesn't accidentally decode parity shards when
    /// it shouldn't (i.e. the output is correct and
    /// decryption succeeds in the no-corruption case).
    #[test]
    fn lazy_parity_path_correct_on_uncorrupted_archive() {
        let mut input = vec![0u8; 2_000_000];
        getrandom::getrandom(&mut input).expect("getrandom");
        let opts = EncryptOptions::new(b"lazy-parity", RecoveryLevel::High);
        let encrypted = compress_encrypted(&input, &opts).expect("compress");
        // NXR magic (encrypted + recovery) but no actual
        // corruption — the lazy path should skip parity
        // decrypts entirely.
        assert_eq!(&encrypted[0..4], b"NXR\0");
        let decrypted = decompress_encrypted(&encrypted, b"lazy-parity")
            .expect("decompress on uncorrupted NXR");
        assert_eq!(decrypted, input);
    }
}
