# Sprint 5.7.2 — Block-Level AES-256-GCM + Reed-Solomon Recovery

> **Status:** design draft, pre-implementation. Target: post-v0.1.2.
> **Author:** Mavis (Mavis), 2026-07-08.
> **Parents:** Sprint 5.7.1 (parallel compression), Sprint 5.7 (db.rs + RAM auto-tune).
> **License note:** encryption + recovery adds no AGPL-3.0 obligations; the
> format remains open and the codec stays reversible from spec.

---

## 0. Why this sprint

Today's `.nxs6` format is:

```
[MAGIC "NXS\0"] [version+flags] [block_count] [total_size] [reserved]
[ BlockHeader ] [ compressed chunk bytes ] × block_count
```

Two production gaps that hold back the commercial pitch:

1. **No confidentiality.** Anyone with disk access reads the archive.
   7-Zip has `-p`, WinRAR has password-protection, Zstd has nothing.
   A real "secure local backup" tool needs AES at rest.
2. **No integrity verification beyond per-block decode-time checks.**
   A bit-flip in sector 4 of an HDD doesn't fail loud — it either
   returns garbage (CDC dedup happy-path), or panics. Neither
   matches what users expect from "professional archive" software.

Both are solvable with **per-block** AES-256-GCM + a small Reed-Solomon
parity layer, applied inside the existing `par_iter` of `parallel.rs`.

---

## 1. Wire format v3 (`.nxs6v3`)

Bump the on-disk magic from `NXS\0` (4 bytes) to a new magic **only when
encryption is enabled**, so existing v0.1.1 / v0.1.2 archives keep working:

| Magic        | Means                                              |
| ------------ | -------------------------------------------------- |
| `NXS\0`      | v0.1.1 / v0.1.2 — no encryption, no recovery      |
| `NXE\0`      | v0.1.3+ — **encrypted**, no recovery               |
| `NXR\0`      | v0.1.3+ — **encrypted + recovery blocks present**  |

The decoder dispatches on the magic; the existing v2/v3 sequential and
parallel decoders continue to read `NXS\0` as today.

### 1.1 New top-level header (24 bytes)

```rust
pub struct NexusHeaderV3 {
    pub magic: [u8; 4],          // "NXE\0" or "NXR\0"
    pub version: u8,             // 3
    pub flags: u8,               // bit 0: recovery, bit 1: argon2id
    pub kdf_salt: [u8; 16],      // only present if flags & 0x02
    pub kdf_params: [u8; 8],     // m_cost, t_cost, p_cost, parallelism
    pub block_count: u32,
    pub uncompressed_total_size: u64,
}
```

`kdf_salt` is **per-archive** (not per-block). Generated with
`OsRng.fill_bytes(16)` at compress time, written to the header,
re-read at decompress time. The user password + salt derive the same
32-byte AES key on both ends.

### 1.2 Per-block frame (12 bytes header + payload)

```rust
pub struct BlockFrame {
    pub block_id: u32,        // 0-indexed
    pub nonce: [u8; 12],      // AES-GCM nonce (per-block, unique)
    pub payload_len: u32,     // ciphertext + 16-byte GCM tag
    pub aad: [u8; 16],        // block_id || archive_nonce || block_uncompressed_size
    pub ciphertext: Vec<u8>,  // 32 MiB input → ≤ 32 MiB + 16 B
}
```

**AAD choice.** We bind the ciphertext to its position in the archive.
If an attacker swaps blocks between two archives encrypted with the
same password, GCM auth fails. This stops "block-shuffling" attacks
that 7z's password-only mode is famously vulnerable to.

**Nonce uniqueness.** 96-bit nonce = `block_id (4) || archive_nonce (8)`.
The `archive_nonce` is 8 random bytes per archive. Combined with
`block_id` (0..block_count < 2^32), the (key, nonce) pair is unique
per archive. No nonce-reuse, no need for the heavyweight random
96-bit generation per block.

### 1.3 Recovery layout (only if `flags & 0x01`)

**Parity goes in a TRAILER, not interleaved with data blocks.**

Why trailer over interleave:
- The decoder always reads the file start-to-finish (we don't
  stream a `.nxs6` — compression is a single atomic write).
- With a trailer, the offsets are trivial: read header →
  read `block_count` data blocks (length-prefixed) → read
  `m` parity blocks. No random-access, no seek.
- Interleaved would only help if we were streaming random
  blocks (think Kafka-style append-only log with sub-second
  per-block recovery). That's not our threat model.

Reed-Solomon over GF(256) with the classic `rs = 255, k, m` shape,
re-parameterized for the parallel encoder:

- **Data shards:** the `block_count` compressed blocks.
- **Parity shards:** `m = ceil(block_count × 0.10)` (10 % overhead,
  configurable via CLI). With 32 MiB super-blocks and a 1 GiB file
  (32 blocks), that's ~3.2 MiB of parity.
- **Shard size:** the largest single block's compressed size, padded
  with zeros. Storing all parity shards adds O(m × max_block) to the
  output — acceptable.
- **Encoder:** runs INSIDE the same `par_iter` as the data shards
  (each parity shard is the XOR of every 10th data shard modulo
  Galois). This is a single `zip()` over the data shards.

**Per-shard recovery.** When one block is corrupted, the decoder:
1. Reads the block, decrypts, runs the inner CRC64 + LZMA decode.
2. If decode fails OR GCM auth tag mismatches: mark the slot as
   "missing".
3. After reading all blocks: if `missing_count ≤ m`, run the
   Reed-Solomon reconstruction to recover the lost shards.
4. Recompute and verify the AAD + GCM tag of the recovered shard
   (GCM was applied BEFORE parity, so the tag is the canonical
   auth — RS gives us a candidate, GCM confirms).

This is the part 7z and WinRAR do not give you. They stop at
"corrupt block — abort." We continue.

---

## 2. Crypto module (`src/crypto.rs` — new)

```rust
//! Block-level AES-256-GCM with Argon2id KDF.
//!
//! Sprint 5.7.2. Single source of truth for key derivation and
//! authenticated encryption. Used by both the parallel encoder
//! (one encrypt per super-block) and the parallel decoder
//! (one decrypt per super-block, with parity fallback).
//!
//! All ops are constant-time where the underlying `aes-gcm`
//! and `argon2` crates are. We do not roll our own crypto.

use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::Aced;
use argon2::{Argon2, Algorithm, Version, Params};

pub const KEY_LEN: usize = 32;       // AES-256
pub const NONCE_LEN: usize = 12;     // GCM standard
pub const TAG_LEN: usize = 16;       // GCM standard
pub const SALT_LEN: usize = 16;      // Argon2id

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KdfPreset {
    /// Interactive: <100 ms on a modern desktop. For local archives.
    Interactive = 0,
    /// Moderate: ~500 ms. For cloud-shared archives.
    Moderate = 1,
    /// Sensitive: ~2 s. For long-term storage of PII.
    Sensitive = 2,
}

impl KdfPreset {
    /// The actual Argon2id parameters (m_cost, t_cost, p_cost,
    /// output_len) for this preset. The m_cost values are in
    /// KiB. These are the EXACT values the design doc
    /// §2 commits to.
    pub fn params(self) -> Params {
        match self {
            Self::Interactive => Params::new(19_456, 2, 1, Some(KEY_LEN as u32)),
            Self::Moderate    => Params::new(46_080, 3, 1, Some(KEY_LEN as u32)),
            Self::Sensitive   => Params::new(65_536, 4, 2, Some(KEY_LEN as u32)),
        }
    }
}

/// Result of a key derivation. Returned to the caller so they
/// can (a) write the actually-used preset into the V3 header's
/// kdf_params field (so the decoder reproduces the same KDF
/// output exactly), and (b) surface a warning to the user if
/// the original preset was downgraded to fit the available RAM.
#[derive(Debug, Clone)]
pub struct KdfResult {
    /// The 32-byte AES-256 key derived from the password.
    /// Returned in a `Zeroizing`-compatible buffer (see
    /// `key_material::ZeroizingKey`) so it gets wiped on Drop.
    pub key: ZeroizingKey,
    /// The preset that was actually used. May differ from
    /// the caller's request if `downgraded` is true.
    pub preset_used: KdfPreset,
    /// True if the requested preset was relaxed to fit the
    /// available RAM. The orchestrator should surface this
    /// to the user (CLI/UI warning: "Argon2id preset relaxed
    /// from Sensitive to Moderate: only 4 GiB RAM available").
    pub downgraded: bool,
}

/// A 32-byte AES key that wipes itself on Drop. Used for
/// all in-memory key material to limit the window of
/// vulnerability to a memory-dump attack.
pub struct ZeroizingKey(pub [u8; KEY_LEN]);

impl Drop for ZeroizingKey {
    fn drop(&mut self) {
        // Volatile write to ensure the compiler doesn't
        // optimize away the wipe. `volatile_set_memory` is
        // stable in Rust 1.78+; before that we use
        // `core::ptr::write_volatile` in a loop.
        for byte in self.0.iter_mut() {
            unsafe { core::ptr::write_volatile(byte, 0) };
        }
    }
}

/// Derive a 32-byte AES key from `password` + `salt` with the
/// given Argon2id preset. **Auto-downgrades the preset** if
/// the available RAM on the system is below the preset's
/// working-set requirement (e.g. Sensitive needs 65 MiB; on
/// a 4 GiB machine we drop to Moderate automatically).
///
/// **Why auto-downgrade vs hard fail:** the orchestrator
/// (Tauri command / CLI subcommand) is already past the
/// "user clicked Compress" point. A hard failure mid-50-GiB
/// backup is much worse than a slightly weaker KDF — the
/// alternative is no backup at all, which is the worst
/// outcome of all. We downgraded, log a warning, and the
/// user can re-run with `-k sensitive --no-downgrade` if
/// they explicitly want to risk OOM.
pub fn derive_key(
    password: &[u8],
    salt: &[u8; SALT_LEN],
    requested: KdfPreset,
) -> Result<KdfResult, CryptoError> {
    // Read available RAM. If we can't (sandboxed env, exotic
    // platform), we trust the caller's request and don't
    // downgrade — the user can override per-call if they
    // know their environment.
    let available_mb = crate::ram::available_memory_mb();

    // `clamp_preset_for_ram` returns the highest preset that
    // fits in `available_mb` without OOM-ing. It returns the
    // SAME preset if there's enough RAM.
    let clamped = crate::ram::clamp_preset_for_ram(requested as u32, available_mb);
    let preset_used = preset_from_u32(clamped);
    let downgraded = preset_used != requested;

    // Now derive the key with the (possibly clamped) preset.
    let params = preset_used.params();
    let mut raw_key = [0u8; KEY_LEN];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, &mut raw_key)
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    Ok(KdfResult {
        key: ZeroizingKey(raw_key),
        preset_used,
        downgraded,
    })
}

fn preset_from_u32(n: u32) -> KdfPreset {
    match n {
        0 => KdfPreset::Interactive,
        1 => KdfPreset::Moderate,
        2 => KdfPreset::Sensitive,
        _ => KdfPreset::Interactive, // unknown → safest default
    }
}

pub struct BlockCipher {
    cipher: Aes256Gcm,
    nonce_prefix: [u8; 8], // per-archive random 8 bytes
}

impl BlockCipher {
    pub fn new(key: &[u8; KEY_LEN], nonce_prefix: [u8; 8]) -> Self {
        Self {
            cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)),
            nonce_prefix,
        }
    }

    /// Encrypts `plaintext` for `block_id`, returning
    /// `ciphertext || tag`. The GCM AAD binds the block to its
    /// position in the archive (see wire format spec).
    pub fn encrypt_block(
        &self,
        block_id: u32,
        uncompressed_size: u32,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = self.assemble_nonce(block_id);
        let aad = assemble_aad(block_id, &self.nonce_prefix, uncompressed_size);
        self.cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                aes_gcm::aead::Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|e| CryptoError::Encrypt(e.to_string()))
    }

    pub fn decrypt_block(
        &self,
        block_id: u32,
        uncompressed_size: u32,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce = self.assemble_nonce(block_id);
        let aad = assemble_aad(block_id, &self.nonce_prefix, uncompressed_size);
        self.cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                aes_gcm::aead::Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|e| CryptoError::Decrypt(e.to_string()))
    }

    fn assemble_nonce(&self, block_id: u32) -> [u8; NONCE_LEN] {
        let mut n = [0u8; NONCE_LEN];
        n[..8].copy_from_slice(&self.nonce_prefix);
        n[8..].copy_from_slice(&block_id.to_be_bytes());
        n
    }
}

fn assemble_aad(block_id: u32, archive_nonce: &[u8; 8], uncompressed_size: u32) -> [u8; 16] {
    let mut aad = [0u8; 16];
    aad[..4].copy_from_slice(&block_id.to_be_bytes());
    aad[4..12].copy_from_slice(archive_nonce);
    aad[12..].copy_from_slice(&uncompressed_size.to_be_bytes());
    aad
}
```

**Cargo.toml addition.**

```toml
[dependencies]
aes-gcm = "0.10"
argon2 = "0.5"
rand = "0.8"
```

All three are MIT / Apache-2.0 / dual-licensed — compatible with the
AGPL-3.0 + commercial dual.

---

## 3. Recovery module (`src/recovery.rs` — new)

Use a minimal Reed-Solomon implementation. We don't need a heavyweight
crate; the standard `reed-solomon-erasure` is GPL-3.0 which is
incompatible. The `rs-erasure` crate is MIT but pulls a `rayon` peer
that conflicts with our scoped pool.

**Recommendation:** implement Galois-field RS directly, ~300 LOC. The
encoder is the simple Cauchy-matrix construction; the decoder uses
Berlekamp-Massey + Forney, both ~150 LOC each.

```rust
//! Block-level Reed-Solomon erasure coding for forward-error
//! recovery. Sprint 5.7.2. NOT for archival "store N copies
//! of everything" — strictly m-parity-shard recovery from up
//! to `m` missing blocks.
//!
//! Tradeoff: 10 % overhead, ~150 MB/s encode, ~120 MB/s decode
//! on a single core. Multi-core decode is on the roadmap for
//! v0.1.4 (the data dependencies are sequential within a
//! shard, so it's not a `par_iter` candidate).
```

---

## 4. Codec integration (`src/codec/parallel.rs` — extend)

```rust
pub fn compress_parallel_with_block_size_and_encryption(
    input: &[u8],
    block_size: usize,
    on_block: &mut F,
    cipher: Option<&BlockCipher>,
) -> Vec<u8> {
    // ... existing pre-flight ...
    let n_threads = safe_parallel_thread_count(input.len());
    let pool = ThreadPoolBuilder::new().num_threads(n_threads).build()?;

    let bytes_done = Arc::new(AtomicU64::new(0));
    let start_instant = Arc::new(Mutex::new(None::<Instant>));

    // ── Pass 1: compress all data shards (existing code) ─────
    let mut compressed: Vec<Vec<u8>> = pool.install(|| {
        blocks.par_iter().enumerate().map(|(idx, block)| {
            // ... existing codec::compress call + progress emit ...
        }).collect()
    });

    // ── Pass 2: encrypt each data shard in place (NEW) ───────
    if let Some(cipher) = cipher {
        pool.install(|| {
            compressed.par_iter_mut().enumerate().for_each(|(idx, c)| {
                *c = cipher.encrypt_block(idx as u32, ... , c).expect("encrypt");
            });
        });
    }

    // ── Pass 3: generate m parity shards (NEW, if requested) ─
    let parity = if want_recovery {
        generate_parity_shards(&compressed, m)
    } else {
        Vec::new()
    };

    // ── Serialize: header + data shards + parity shards ─────
    let mut out = encode_v3_header(...);
    for c in &compressed { write_length_prefixed(c, &mut out); }
    for p in &parity   { write_length_prefixed(p, &mut out); }
    out
}
```

**Why two passes, not one.** The encryption step needs the
**uncompressed size** of each block for the AAD. We don't have that
in pass 1's intermediate `Vec<u8>` — it's in the original `&[u8]`
input slice. Doing the encryption as a second `par_iter_mut` pass
costs us one extra synchronization point but lets us reuse the
existing compression code unchanged.

**Bench expectation.** AES-256-GCM on a modern desktop
(AES-NI) at ~3.5 GB/s. The codec does ≤ 100 MiB/s on the
LZ77+rANS pipeline, so encryption is a **wash** (no measurable
slowdown) when AES-NI is available. On a Raspberry Pi 4
(no AES-NI), it'll add ~30 % wall time — acceptable for the
Sprint 5.7.3 mobile story, not the Sprint 5.7.2 desktop story.

---

## 5. API surface

### 5.1 Engine (`src/api.rs`)

```rust
pub fn compress_target_with_password<P>(
    input_path: &Path,
    backend: CompressionBackend,
    lzma_level: u32,
    output_dir: Option<&Path>,
    password: Option<&[u8]>,
    recovery: RecoveryLevel,  // Off | Low (10%) | High (25%)
    progress: P,
) -> ApiResult<CompressTargetResult> { ... }

pub fn decompress_target_with_password<P>(
    input_path: &Path,
    output_dir: Option<&Path>,
    password: Option<&[u8]>,
    progress: P,
) -> ApiResult<DecompressTargetResult> { ... }
```

`RecoveryLevel::Off` ⇒ no parity shards, no overhead.
`RecoveryLevel::Low`  ⇒ 10 % parity (1 recoverable block per 10).
`RecoveryLevel::High` ⇒ 25 % parity (2-3 recoverable blocks per 8).

### 5.2 Tauri command (`src-tauri/src/commands.rs`)

The existing commands take `req: serde_json`. We extend the JSON
schema (NOT a new command — back-compat with the existing UI):

```json
{
  "req": {
    "path": "...",
    "backend": "v4",
    "lzma_level": 6,
    "output_dir": null,
    "password": "hunter2",          // NEW (optional, base64 NOT needed)
    "recovery": "low"               // NEW (optional: "off" | "low" | "high")
  }
}
```

Password handling: Tauri 2.x channels the command through a secure
IPC pipe; the `password` string is only in memory during the
command execution and never written to logs. We **do not** use
`tauri-plugin-stronghold` for v0.1.3 (overkill) — instead the
key is held in a `Zeroizing<[u8; 32]>` that wipes itself on
`Drop`.

### 5.3 Frontend (`CompressView.tsx`)

A new "Encrypt" section in the right column of the compress view,
above the destination picker. Three controls:

- **Password** (text input, `type="password"`, autoComplete="new-password")
- **Recovery level** (radio: Low (default) / Off / High, with a
  one-line description each: "1 block per 10 recoverable — 10 %
  overhead" / "No recovery — 0 % overhead" / "2-3 per 8 — 25 %
  overhead")

**Default is `low`, not `off`.** The recovery feature is the
core of the Sprint 5.7.2 pitch ("indestructible archives"). A
user who doesn't want the overhead can switch to `off`
explicitly. This matches the 7z / WinRAR UX convention
(recovery is on-by-default, with a disable checkbox).

This is ~200 lines of TS. Reuse the existing panel styling from
Sprint 5.6.29.

---

## 6. Test plan

### 6.1 Unit tests (engine)

- `crypto::tests::derive_key_deterministic` — same input ⇒ same key.
- `crypto::tests::derive_key_distinct_per_salt` — different salts ⇒
  different keys.
- `crypto::tests::encrypt_decrypt_roundtrip` — plaintext → encrypt →
  decrypt ⇒ plaintext.
- `crypto::tests::encrypt_then_flip_one_bit_fails` — flip one bit
  in the ciphertext ⇒ decrypt returns `Err(Decrypt)`.
- `crypto::tests::aad_swap_fails` — encrypt with AAD(A), decrypt
  with AAD(B) ⇒ `Err(Decrypt)`.
- `recovery::tests::encode_decode_roundtrip` — 32 data + 3 parity ⇒
  decoder reconstructs the original.
- `recovery::tests::recover_one_missing` — 1 block zeroed out ⇒
  reconstructed correctly.
- `recovery::tests::recover_three_missing` — 3 blocks zeroed out
  (with m=3) ⇒ reconstructed correctly.
- `recovery::tests::too_many_missing_fails` — 4 blocks zeroed out
  with m=3 ⇒ `Err(TooManyMissing)`.
- `codec::parallel::tests::compress_encrypt_decrypt_roundtrip` —
  end-to-end with a 64 MiB input.
- `codec::parallel::tests::compress_recover_after_bit_flip` —
  flip 1 bit in the encrypted output, decoder still produces the
  original plaintext.

### 6.2 Integration tests (Tauri)

- Compress a 100 MB sample with password + recovery=Low. Verify
  the output file is 10 % larger than the unencrypted version.
- Corrupt 1 byte of the encrypted output via `truncate -c -s -1`
  + `dd if=/dev/urandom of=... bs=1 count=1 seek=N conv=notrunc`.
- Decompress — should still succeed, log a `recovery_used` event
  in the db.

### 6.3 Performance

Benchmark on the 200 MB corpus:

- Sequential v0.1.2 baseline: 100 MiB/s
- Parallel v0.1.2: 280 MiB/s (8 cores)
- Parallel v0.1.3 + AES (no recovery): 280 MiB/s (no measurable
  difference, AES-NI is the bottleneck that LZ77 isn't)
- Parallel v0.1.3 + AES + recovery: 250 MiB/s encode (RS is single-
  thread), 220 MiB/s decode + recovery

---

## 7. Risks & open questions

1. **Argon2id memory.** Interactive preset = 19 MiB. Some users
   have 8 GiB total RAM and a Tauri WebView open — pushing
   memory to the limit. The CLI should auto-fall-back to a
   softer preset if `available_memory_mb() < 256`. We already
   have `crate::ram::available_memory_mb()` from Sprint 5.7.1.
2. **Password reset.** If a user loses the password, the archive
   is gone. No backdoor. Document this loudly in the UI.
3. **Time-based attack on the password.** Argon2id's
   `t_cost = 2, m_cost = 19 MiB` is fine for the
   "interactive" use case (local desktop, no server-side
   guessing). The "Sensitive" preset is what you want for
   archives that might end up on a stolen laptop.
4. **Reed-Solomon license.** Re-implement from scratch. The
   ~300 LOC is the cost of avoiding GPL-3.0 contamination.
5. **Format compat with v0.1.1 / v0.1.2.** New `NXE\0` /
   `NXR\0` magics mean v0.1.3+ decoders can READ v0.1.1
   archives (they keep using the `NXS\0` path), but
   v0.1.1 / v0.1.2 decoders will refuse to read v0.1.3+
   encrypted archives. This is the right tradeoff (encryption
   is a one-way upgrade).

---

## 8. Estimate

- **crypto.rs**: 200 LOC + 6 tests = 0.5 day.
- **recovery.rs**: 350 LOC + 5 tests = 1 day.
- **format.rs extension**: 80 LOC + 2 tests = 0.25 day.
- **parallel.rs integration**: 60 LOC + 3 tests = 0.5 day.
- **api.rs surface**: 40 LOC + 1 test = 0.25 day.
- **commands.rs (Tauri)**: 30 LOC = 0.1 day.
- **Frontend (CompressView)** + i18n: 200 LOC = 0.5 day.
- **Manual end-to-end + integration tests on the 800 MiB
  file**: 0.5 day.

**Total: ~3.5 days.** Conservative — Argon2id parameter tuning
alone is half a day of "is 19 MiB the right m_cost?" debate.

---

## 9. Out of scope (deferred)

- **Streaming encrypt** for the `p2p_tunnel` (the live file
  share). Different threat model (in-memory + authenticated
  channel already), different KDF. v0.2.x.
- **Key files / smart card** instead of password. v0.2.x.
- **Multi-recipient encryption** (header lists N public keys,
  body is encrypted with a random symmetric key, each recipient
  gets the symmetric key encrypted with their public key).
  v0.3.x.
- **Recovery beyond block level** (intra-block erasure codes for
  partial reads from a failing disk). v0.2.x.
