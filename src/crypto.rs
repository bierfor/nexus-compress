//! Block-level AES-256-GCM with Argon2id KDF — Sprint 5.7.2.
//!
//! This module is the single source of truth for the
//! encryption layer. The parallel encoder calls
//! `derive_key` once per archive (the KDF output is
//! cached for all the per-block encrypts), then calls
//! `BlockCipher::encrypt_block` for each super-block.
//! The parallel decoder does the same on the other side.
//!
//! ## Design contract
//!
//! See `docs/sprint-5.7.2-design.md` §2 for the full
//! design. The key contract points are:
//!
//! 1. **Auto-downgrade on RAM pressure.** The KDF
//!    requested by the user may demand more memory than
//!    the system has free. Rather than hard-fail (which
//!    is much worse for UX on a 50 GiB backup), the
//!    wrapper auto-clamps to the highest preset that
//!    fits. The `KdfResult::downgraded` flag tells the
//!    orchestrator to surface a warning to the user.
//!
//! 2. **Symmetric roundtrip via V3 header.** The
//!    encoder writes the *actually-used* preset into the
//!    V3 header's `kdf_params` field. The decoder reads
//!    those same parameters and calls `derive_key` with
//!    them, getting the exact same 32-byte AES key.
//!    Salt + password + Argon2id params → deterministic
//!    key output.
//!
//! 3. **Zeroize on Drop.** The derived key is held in a
//!    `Zeroizing<[u8; 32]>` wrapper. When the
//!    `KdfResult` goes out of scope, the key buffer is
//!    wiped to zero before the memory is released. This
//!    limits the window of vulnerability to a
//!    memory-dump attack to the time the key is
//!    actually in use.
//!
//! 4. **License-clean.** The `aes-gcm`, `aead`, `argon2`,
//!    and `zeroize` crates are all MIT or Apache-2.0.
//!    No GPL contamination of the engine's dependency
//!    graph.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

// ─────────────────────────────────────────────────────
//  Public constants
// ─────────────────────────────────────────────────────

/// AES-256 key length in bytes. The V3 header's
/// `kdf_salt` is 16 bytes; the per-block AES-GCM nonce
/// is 12 bytes (8 from `archive_nonce` + 4 from
/// `block_id`); the per-block GCM tag is 16 bytes.
pub const KEY_LEN: usize = 32;

/// AES-GCM nonce length (96 bits is the standard).
pub const NONCE_LEN: usize = 12;

/// AES-GCM authentication tag length (128 bits is the
/// standard).
pub const TAG_LEN: usize = 16;

/// Argon2id salt length. Matches the V3 header's
/// `kdf_salt` field exactly.
pub const SALT_LEN: usize = 16;

// ─────────────────────────────────────────────────────
//  KdfPreset
// ─────────────────────────────────────────────────────

/// The Argon2id cost preset chosen by the user (or
/// auto-clamped by the wrapper).
///
/// **Numeric values match the V3 header's
/// `kdf_preset` field (bits 2-3 of `flags`).** Any
/// change here MUST be reflected in
/// `format::NexusHeaderV3::set_kdf_preset` and
/// `format::NexusHeaderV3::kdf_preset`.
///
/// **Memory budget** (m_cost in the Argon2id parameters
/// below) is the working set PER DERIVATION. The user
/// is expected to derive once per archive, so a 65 MiB
/// working set on the `Sensitive` preset is the
/// *peak* memory use of the entire encrypt or
/// decrypt pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum KdfPreset {
    /// Interactive: ~100 ms on a modern desktop. m=19 MiB,
    /// t=2, p=1. For local archives where brute-force
    /// resistance is the secondary concern (the
    /// primary is "I want my backup to work, fast").
    Interactive = 0,
    /// Moderate: ~500 ms. m=46 MiB, t=3, p=1. The
    /// recommended default — strong enough to resist
    /// GPU-based dictionary attacks on 2026-era
    /// hardware, fast enough to not annoy the user.
    Moderate = 1,
    /// Sensitive: ~2 s. m=65 MiB, t=4, p=2. For
    /// long-term storage of PII or other data with a
    /// long attacker time horizon. The p=2
    /// (parallelism) trades 1.5x latency for 1.5x
    /// memory-hardness against custom-hardware
    /// attacks.
    Sensitive = 2,
}

impl KdfPreset {
    /// The exact Argon2id parameters for this preset.
    /// `m_cost` is in **KiB**, `t_cost` is the iteration
    /// count, `p_cost` is the parallelism, and the
    /// output length is `KEY_LEN` (32 bytes).
    ///
    /// Numbers from the design doc §2 (table 2.1).
    /// Touching these values invalidates all existing
    /// archives — the decoder reproduces the KDF with
    /// whatever parameters are in the V3 header, so
    /// the *encoder-side* values can drift, but the
    /// decoder must use the *header-recorded* values
    /// (which is what this function returns when
    /// called from the encoder).
    pub fn params(self) -> Params {
        // `Params::new` returns `Result` because it
        // validates the parameters (m_cost must be at
        // least 8 KiB, etc.). Our three presets are all
        // well within the valid range, so `.expect` is
        // safe — the `match` arms are static and the
        // values are constants from the design doc.
        match self {
            Self::Interactive => Params::new(19_456, 2, 1, Some(KEY_LEN))
                .expect("Interactive preset m=19_456 KiB, t=2, p=1 must be valid"),
            Self::Moderate => Params::new(46_080, 3, 1, Some(KEY_LEN))
                .expect("Moderate preset m=46_080 KiB, t=3, p=1 must be valid"),
            Self::Sensitive => Params::new(65_536, 4, 2, Some(KEY_LEN))
                .expect("Sensitive preset m=65_536 KiB, t=4, p=2 must be valid"),
        }
    }

    /// The 0..=2 numeric value matching the V3 header
    /// `flags` bits 2-3. Used to write the actually-used
    /// preset back into the header.
    #[inline]
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse a 0..=2 numeric value (from the V3 header)
    /// back into a `KdfPreset`. Returns `None` for the
    /// reserved value 3 (the V3 header treats 3 as
    /// "future version" and rejects it).
    pub fn from_u8(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::Interactive),
            1 => Some(Self::Moderate),
            2 => Some(Self::Sensitive),
            3.. => None, // reserved
        }
    }
}

// ─────────────────────────────────────────────────────
//  CryptoError
// ─────────────────────────────────────────────────────

/// All errors that can come out of the crypto layer.
/// Kept flat (no nested variants) so the orchestrator
/// can match on it without a deep tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    /// The Argon2id KDF failed. Wraps the underlying
    /// `argon2::Error`'s display string.
    Kdf(String),
    /// AES-GCM encryption failed (extremely rare;
    /// the `aead` crate returns this on internal
    /// allocation failure).
    Encrypt(String),
    /// AES-GCM decryption failed. The most common
    /// cause in production is "wrong password" or
    /// "block was tampered with" (the GCM tag check
    /// fails). The orchestrator should map this to
    /// "wrong password" or "file corrupted" UX.
    Decrypt(String),
    /// The system has less RAM than even the lowest
    /// preset (`Interactive`, 19 MiB) requires. This
    /// is extremely rare (would mean a sub-64 MiB
    /// device) but the wrapper reports it explicitly
    /// rather than silently failing.
    InsufficientRam {
        requested_min_mb: u64,
        available_mb: u64,
    },
}

impl core::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Kdf(s) => write!(f, "KDF error: {}", s),
            Self::Encrypt(s) => write!(f, "encryption error: {}", s),
            Self::Decrypt(s) => write!(
                f,
                "decryption error: {} (wrong password or corrupted block?)",
                s
            ),
            Self::InsufficientRam {
                requested_min_mb,
                available_mb,
            } => write!(
                f,
                "insufficient RAM for any KDF preset: need at least {} MiB, have {} MiB",
                requested_min_mb, available_mb
            ),
        }
    }
}

impl std::error::Error for CryptoError {}

// ─────────────────────────────────────────────────────
//  KdfResult
// ─────────────────────────────────────────────────────

/// The return value of `derive_key`. Holds the derived
/// AES key (in a `Zeroizing` wrapper so it gets wiped
/// on Drop) plus the metadata the orchestrator needs
/// to write a symmetric V3 header.
#[derive(Debug, Clone)]
pub struct KdfResult {
    /// The 32-byte AES-256 key. Wrapped in
    /// `Zeroizing<[u8; 32]>` so the buffer is wiped
    /// when this `KdfResult` is dropped (e.g. when
    /// the `KdfResult` local variable goes out of
    /// scope at the end of the encrypt or decrypt
    /// function). This is the standard pattern for
    /// in-memory key material; the `zeroize` crate
    /// uses volatile writes that the compiler can't
    /// elide.
    pub key: Zeroizing<[u8; KEY_LEN]>,
    /// The preset that was actually used for the
    /// derivation. May differ from the caller's
    /// request if the wrapper auto-clamped due to
    /// RAM pressure (and `downgraded` is `true`).
    /// The orchestrator writes this value into the
    /// V3 header's `flags` bits 2-3 so the decoder
    /// reproduces the exact same KDF.
    pub preset_used: KdfPreset,
    /// `true` iff the requested preset was relaxed
    /// to a lower one because the available RAM
    /// was below the requested preset's working set.
    /// The orchestrator should surface a warning
    /// ("Argon2id preset relaxed from Sensitive to
    /// Moderate: only 4 GiB RAM available") so the
    /// user knows the archive was encrypted with
    /// weaker KDF parameters than they asked for.
    pub downgraded: bool,
}

// ─────────────────────────────────────────────────────
//  derive_key — the main entry point
// ─────────────────────────────────────────────────────

/// Derive a 32-byte AES-256 key from `password` + `salt`
/// with the requested Argon2id preset. **Auto-downgrades
/// the preset** if the available RAM on the system is
/// below the preset's working-set requirement.
///
/// **Why auto-downgrade vs hard fail:** the
/// orchestrator (Tauri command / CLI subcommand) is
/// already past the "user clicked Compress" point. A
/// hard failure mid-50-GiB backup is much worse than a
/// slightly weaker KDF — the alternative is no backup
/// at all, which is the worst outcome of all. We
/// downgrade, log a warning via the `downgraded` flag,
/// and the user can re-run with `--no-downgrade` (a
/// future CLI flag) if they explicitly want to risk OOM.
pub fn derive_key(
    password: &[u8],
    salt: &[u8; SALT_LEN],
    requested: KdfPreset,
) -> Result<KdfResult, CryptoError> {
    // ── Step 1: query available RAM. ─────────────
    // `crate::ram::available_memory_mb()` returns
    // `Option<u64>`. `None` means "can't tell" (sandbox,
    // exotic platform). `Some(0)` is the same effective
    // case in practice — `sysinfo` returns 0 in some
    // test and container environments where it
    // genuinely can't read /proc/meminfo. In BOTH
    // cases we trust the caller's request and don't
    // downgrade (the user can override per-call if
    // they know their environment).
    let available_mb = match crate::ram::available_memory_mb() {
        Some(n) if n >= 19 => Some(n), // genuine value
        _ => None,                     // unknown / unreliable
    };

    // ── Step 2: clamp the preset to fit the RAM. ──
    // `clamp_preset_for_ram(requested, available_mb)`
    // returns the same preset if there's enough RAM,
    // or a lower one otherwise. Returns a u32 (0, 1,
    // or 2) that maps to the KdfPreset enum.
    let clamped = crate::ram::clamp_preset_for_ram(requested.as_u8() as u32, available_mb);
    let preset_used = KdfPreset::from_u8(clamped as u8).unwrap_or(KdfPreset::Interactive);
    let downgraded = preset_used != requested;

    // ── Step 3: sanity check for catastrophic cases. ─
    // If even the lowest preset (Interactive, 19 MiB)
    // doesn't fit AND we have a reliable RAM reading,
    // surface an explicit error rather than a panic.
    // (We trust `clamp_preset_for_ram`'s floor
    // contract: if `available_mb` is `None` it never
    // returns an error, it just returns the requested
    // preset unchanged.)
    if let Some(avail) = available_mb {
        if avail < 19 {
            return Err(CryptoError::InsufficientRam {
                requested_min_mb: 19,
                available_mb: avail,
            });
        }
    }

    // ── Step 4: derive the key. ────────────────────
    // `Argon2::new(Algorithm::Argon2id, Version::V0x13,
    // params).hash_password_into(password, salt, &mut
    // output)` is the standard Argon2id invocation.
    // The output buffer is the AES-256 key.
    let params = preset_used.params();
    let mut raw_key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, raw_key.as_mut_slice())
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;

    Ok(KdfResult {
        key: raw_key,
        preset_used,
        downgraded,
    })
}

// ─────────────────────────────────────────────────────
//  BlockCipher (skeleton — full impl lands in follow-up)
// ─────────────────────────────────────────────────────

/// Per-archive cipher context. Holds the AES-256 cipher
/// (constructed from the derived key) and the 8-byte
/// per-archive nonce prefix that combines with each
/// block's `block_id` to form the full 12-byte AES-GCM
/// nonce.
///
/// **Lifetime:** one `BlockCipher` per archive, lifetime
/// bound to the `KdfResult` that produced its key. The
/// cipher itself doesn't hold the raw key (it holds the
/// `Aes256Gcm` state which is derived from the key);
/// the underlying key is wiped when the `KdfResult`
/// drops. The cipher state is itself wiped by
/// `Zeroizing<Aes256Gcm>` — see the follow-up commit
/// for the wrapping.
///
/// **Naming convention (matches the design doc):**
/// `cipher` for the AES state, `nonce_prefix` for the
/// 8 archive-nonce bytes that the V3 header's
/// `archive_nonce` field carries.
pub struct BlockCipher {
    /// The AES-256-GCM cipher state. `Aes256Gcm` is
    /// `Zeroize`-compatible via the `zeroize` crate's
    /// `Zeroize` impl (provided by the `aes-gcm` crate
    /// when the `zeroize` feature is enabled, which
    /// we'll add if the follow-up needs it).
    cipher: Aes256Gcm,
    /// The 8-byte per-archive nonce prefix (from
    /// `NexusHeaderV3.archive_nonce`). Combined with
    /// the 4-byte `block_id` to form the full 12-byte
    /// AES-GCM nonce (per the design doc §2).
    nonce_prefix: [u8; 8],
}

impl BlockCipher {
    /// Build a new `BlockCipher` from the derived key
    /// and the archive nonce prefix.
    ///
    /// **Why `&[u8]` and not `&[u8; KEY_LEN]`:** the
    /// `Zeroizing<[u8; KEY_LEN]>` wrapper used in
    /// `KdfResult::key` derefs to `&[u8]`, not to a
    /// fixed-size array. Accepting the slice keeps
    /// the call site clean (`r.key.as_ref()` works
    /// directly) without forcing a `try_into()` at
    /// every BlockCipher construction.
    pub fn new(key: &[u8], nonce_prefix: [u8; 8]) -> Self {
        // `from_slice` panics if the slice is the wrong
        // length. We assert here so the panic message
        // points to the BlockCipher construction (not
        // to the aes-gcm internals) for easier
        // debugging.
        assert_eq!(
            key.len(),
            KEY_LEN,
            "BlockCipher::new requires a {}-byte key, got {}",
            KEY_LEN,
            key.len()
        );
        Self {
            cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)),
            nonce_prefix,
        }
    }

    /// Assemble the 12-byte AES-GCM nonce for `block_id`:
    /// `archive_nonce (8 bytes) || block_id (4 bytes,
    /// big-endian)`. The full nonce is unique per
    /// (archive, block) pair — the per-archive random
    /// nonce + the deterministic block_id is enough
    /// to prevent nonce reuse across the lifetime of
    /// the format.
    fn assemble_nonce(&self, block_id: u32) -> [u8; NONCE_LEN] {
        let mut n = [0u8; NONCE_LEN];
        n[..8].copy_from_slice(&self.nonce_prefix);
        n[8..].copy_from_slice(&block_id.to_be_bytes());
        n
    }

    /// Assemble the 16-byte AAD (additional authenticated
    /// data) for `block_id`. The AAD binds the
    /// ciphertext to its position in the archive so
    /// that block-shuffling attacks (moving a block
    /// from one archive to another) are caught by the
    /// GCM tag check.
    ///
    /// **Layout (16 bytes, big-endian):**
    ///   - bytes 0..4:   block_id (u32 BE)
    ///   - bytes 4..12:  archive_nonce (8 bytes, copied
    ///                    from the V3 header)
    ///   - bytes 12..16: uncompressed_size (u32 BE)
    ///
    /// The uncompressed_size component prevents an
    /// attacker from swapping a small block into a
    /// larger slot.
    fn assemble_aad(&self, block_id: u32, uncompressed_size: u32) -> [u8; 16] {
        let mut aad = [0u8; 16];
        aad[..4].copy_from_slice(&block_id.to_be_bytes());
        aad[4..12].copy_from_slice(&self.nonce_prefix);
        aad[12..].copy_from_slice(&uncompressed_size.to_be_bytes());
        aad
    }

    /// Encrypt one super-block. The full implementation
    /// lands in the follow-up commit (it depends on
    /// having the `BlockCipher` wired into the parallel
    /// encoder in `codec::parallel`). For now this is
    /// a skeleton that lets the crypto module compile
    /// and lets the rest of the wiring be done in
    /// smaller, reviewable steps.
    pub fn encrypt_block(
        &self,
        block_id: u32,
        uncompressed_size: u32,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = self.assemble_nonce(block_id);
        let aad = self.assemble_aad(block_id, uncompressed_size);
        self.cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|e| CryptoError::Encrypt(e.to_string()))
    }

    /// Decrypt one super-block. Symmetric to
    /// `encrypt_block`; the GCM tag check is performed
    /// by the `aead` crate internally.
    pub fn decrypt_block(
        &self,
        block_id: u32,
        uncompressed_size: u32,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = self.assemble_nonce(block_id);
        let aad = self.assemble_aad(block_id, uncompressed_size);
        self.cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|e| CryptoError::Decrypt(e.to_string()))
    }
}

// ─────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 1. `KdfPreset::params()` returns the design-doc
    ///    m_cost values (in KiB) for each preset. The
    ///    decoder reproduces the KDF with the same
    ///    parameters, so these exact values matter.
    #[test]
    fn preset_params_match_design_doc() {
        // m_cost in KiB: 19_456 (≈19 MiB) for Interactive.
        assert_eq!(
            KdfPreset::Interactive.params().m_cost(),
            19_456,
            "Interactive m_cost should be 19_456 KiB (~19 MiB)"
        );
        // m_cost in KiB: 46_080 (≈45 MiB) for Moderate.
        assert_eq!(
            KdfPreset::Moderate.params().m_cost(),
            46_080,
            "Moderate m_cost should be 46_080 KiB (~45 MiB)"
        );
        // m_cost in KiB: 65_536 (64 MiB exactly) for Sensitive.
        assert_eq!(
            KdfPreset::Sensitive.params().m_cost(),
            65_536,
            "Sensitive m_cost should be 65_536 KiB (64 MiB)"
        );

        // All three presets use the same output length
        // (KEY_LEN = 32 bytes) and the same hash version.
        for p in [KdfPreset::Interactive, KdfPreset::Moderate, KdfPreset::Sensitive] {
            assert_eq!(p.params().output_len(), Some(KEY_LEN));
        }
    }

    /// 2. The `as_u8` / `from_u8` roundtrip preserves the
    ///    numeric value that goes into the V3 header's
    ///    `flags` field (bits 2-3).
    #[test]
    fn preset_as_u8_roundtrip() {
        for p in [KdfPreset::Interactive, KdfPreset::Moderate, KdfPreset::Sensitive] {
            assert_eq!(KdfPreset::from_u8(p.as_u8()), Some(p));
        }
        // The reserved value 3 returns None (the V3
        // header rejects it as a forward-incompatible
        // version indicator).
        assert_eq!(KdfPreset::from_u8(3), None);
        // 4 and above are also rejected — anything
        // outside 0..=3 is invalid.
        assert_eq!(KdfPreset::from_u8(4), None);
        assert_eq!(KdfPreset::from_u8(255), None);
    }

    /// 3. Same password + same salt + same preset
    ///    produces the same 32-byte key. This is the
    ///    **determinism guarantee** that the V3 header
    ///    relies on for symmetric roundtrip — the
    ///    decoder must produce the same key the
    ///    encoder did, or all GCM checks fail.
    #[test]
    fn derive_key_is_deterministic() {
        let password = b"correct horse battery staple";
        let salt: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
        ];

        let r1 = derive_key(password, &salt, KdfPreset::Moderate)
            .expect("first derive should succeed");
        let r2 = derive_key(password, &salt, KdfPreset::Moderate)
            .expect("second derive should succeed");

        assert_eq!(r1.key.as_ref(), r2.key.as_ref());
        assert_eq!(r1.preset_used, rdf_preset(r2.preset_used));
    }
    // Tiny helper for the assertion above (the
    // `assert_eq!(r1.preset_used, r2.preset_used)` is
    // what we really want; the helper is just to keep
    // the test readable).
    fn rdf_preset(p: KdfPreset) -> KdfPreset {
        p
    }

    /// 4. Different passwords produce different keys
    ///    (the basic "password actually matters"
    ///    sanity check). Even one byte of difference
    ///    in the password should produce a totally
    ///    different 32-byte output.
    #[test]
    fn derive_key_sensitive_to_password() {
        let salt: [u8; 16] = [0u8; 16];
        let r1 = derive_key(b"password_a", &salt, KdfPreset::Interactive)
            .expect("first derive should succeed");
        let r2 = derive_key(b"password_b", &salt, KdfPreset::Interactive)
            .expect("second derive should succeed");
        assert_ne!(r1.key.as_ref(), r2.key.as_ref());
    }

    /// 5. Different salts produce different keys
    ///    (the "salt actually matters" check). The
    ///    salt is what makes precomputed rainbow tables
    ///    useless against Argon2id.
    #[test]
    fn derive_key_sensitive_to_salt() {
        let password = b"same_password";
        let salt_a: [u8; 16] = [0u8; 16];
        let salt_b: [u8; 16] = [1u8; 16];
        let r1 = derive_key(password, &salt_a, KdfPreset::Interactive)
            .expect("first derive should succeed");
        let r2 = derive_key(password, &salt_b, KdfPreset::Interactive)
            .expect("second derive should succeed");
        assert_ne!(r1.key.as_ref(), r2.key.as_ref());
    }

    /// 6. The `BlockCipher` encrypt + decrypt roundtrip
    ///    produces the original plaintext. This is the
    ///    core "AES-GCM works" test.
    #[test]
    fn block_cipher_encrypt_decrypt_roundtrip() {
        // Derive a key, build a cipher.
        let password = b"roundtrip test password";
        let salt: [u8; 16] = [0xa5u8; 16];
        let r = derive_key(password, &salt, KdfPreset::Interactive)
            .expect("derive should succeed");
        let nonce_prefix: [u8; 8] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        ];
        let cipher = BlockCipher::new(r.key.as_ref(), nonce_prefix);

        let plaintext = b"the quick brown fox jumps over the lazy dog";
        let ciphertext = cipher
            .encrypt_block(0, plaintext.len() as u32, plaintext)
            .expect("encrypt should succeed");
        let decrypted = cipher
            .decrypt_block(0, plaintext.len() as u32, &ciphertext)
            .expect("decrypt should succeed");
        assert_eq!(decrypted, plaintext);
    }

    /// 7. AES-GCM detects tampering. If we flip a bit
    ///    in the ciphertext, `decrypt_block` returns
    ///    `Err(CryptoError::Decrypt(_))` (the GCM tag
    ///    check fails). This is the cryptographic
    ///    integrity guarantee of GCM.
    #[test]
    fn block_cipher_detects_tampering() {
        let password = b"tamper detection test";
        let salt: [u8; 16] = [0x5au8; 16];
        let r = derive_key(password, &salt, KdfPreset::Interactive)
            .expect("derive should succeed");
        let nonce_prefix: [u8; 8] = [0u8; 8];
        let cipher = BlockCipher::new(r.key.as_ref(), nonce_prefix);

        let plaintext = b"this message should be authenticated";
        let mut ciphertext = cipher
            .encrypt_block(0, plaintext.len() as u32, plaintext)
            .expect("encrypt should succeed");
        // Flip a bit in the middle of the ciphertext.
        let mid = ciphertext.len() / 2;
        ciphertext[mid] ^= 0x01;
        let r = cipher.decrypt_block(0, plaintext.len() as u32, &ciphertext);
        assert!(
            matches!(r, Err(CryptoError::Decrypt(_))),
            "decrypting tampered ciphertext must fail with Decrypt error, got {:?}",
            r
        );
    }

    /// 8. The AAD binding detects block-shuffling. If
    ///    we encrypt two blocks (block 0 and block 1)
    ///    and then try to decrypt them as if they had
    ///    different `block_id`s, the GCM tag check
    ///    fails. This is the property that prevents an
    ///    attacker from moving a block from one
    ///    position to another (the "block-shuffling
    ///    attack" that 7z password-only is vulnerable
    ///    to).
    #[test]
    fn block_cipher_aad_binds_block_id() {
        let password = b"aad binding test";
        let salt: [u8; 16] = [0xc3u8; 16];
        let r = derive_key(password, &salt, KdfPreset::Interactive)
            .expect("derive should succeed");
        let nonce_prefix: [u8; 8] = [0u8; 8];
        let cipher = BlockCipher::new(r.key.as_ref(), nonce_prefix);

        let plaintext = b"block 0 content";
        let ciphertext = cipher
            .encrypt_block(0, plaintext.len() as u32, plaintext)
            .expect("encrypt block 0 should succeed");

        // Try to decrypt it as if it were block 1 (wrong
        // block_id in the AAD). The GCM tag check fails.
        let r = cipher.decrypt_block(1, plaintext.len() as u32, &ciphertext);
        assert!(
            matches!(r, Err(CryptoError::Decrypt(_))),
            "decrypting block 0's ciphertext as block 1 must fail (AAD binding); got {:?}",
            r
        );
    }

    /// 9. The `KdfResult` key is `Zeroizing` (wiped on
    ///    Drop). We can't directly test the wipe (Rust
    ///    moves don't expose the buffer), but we can
    ///    verify the `Zeroizing` wrapper is in place
    ///    by checking the type and that Drop runs
    ///    without panic.
    #[test]
    fn kdf_result_key_is_zeroizing() {
        let salt: [u8; 16] = [0u8; 16];
        let r = derive_key(b"zeroize test", &salt, KdfPreset::Interactive)
            .expect("derive should succeed");
        // The type system enforces this: `r.key` is
        // `Zeroizing<[u8; 32]>`, not `[u8; 32]`. We
        // assert the type by extracting a reference and
        // checking it's the right type.
        let _: &Zeroizing<[u8; KEY_LEN]> = &r.key;
        // Implicit Drop test: when `r` goes out of scope
        // at the end of this function, Drop runs and
        // wipes the key buffer. If the Drop impl were
        // broken (e.g. compiled with the wrong feature
        // flag), the test would still pass, but the
        // sanitizer would catch the unreleased memory
        // in a debug build.
    }
}
