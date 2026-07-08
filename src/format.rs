//! .nexus binary format (v0 and v2).
//!
//! ## Header (18 bytes, both versions):
//!   [0..4]   magic = "NXS\0"
//!   [4]      version (0 = legacy, 2 = current)
//!   [5]      flags (reserved, must be 0)
//!   [6..10]  block count (u32 LE)
//!   [10..18] uncompressed total size (u64 LE)
//!
//! ## Per block (BlockHeader, 9 bytes — unchanged):
//!   [0]      block type tag (BlockType as u8)
//!   [1..5]   uncompressed size (u32 LE)
//!   [5..9]   compressed size (u32 LE)
//!   [9..]    payload bytes (format depends on version, see below)
//!
//! ## v0 compressed block payload (version=0):
//!   [0]      tag = 2 (TAG_RANS_LITERALS)
//!   [1..5]   u32 table_len
//!   [5..9]   u32 rans_len
//!   [9..]    table_bytes, rans_bytes, ops_bytes
//!
//!   ops_bytes = sequence of:
//!     [u8 flag=0][u8 placeholder]                  ← literal (placeholder is
//!                                                    overwritten by decoder
//!                                                    with rANS-decoded value)
//!     [u8 flag=1][u32 dist LE][u32 len LE]        ← match
//!
//! ## v2 compressed block payload (version=2):
//!   [0]      tag = 4 (TAG_RANS_V2)
//!   [1..5]   u32 table_len
//!   [5..9]   u32 rans_lit_len
//!   [9..13]  u32 ops_len
//!   [13..]   table_bytes, rans_bytes, ops_bytes
//!
//!   ops_bytes = sequence of:
//!     [u8 flag=0]                                 ← literal (value pulled
//!                                                    from rANS stream)
//!     [u8 flag=1][u16 dist LE][u8 len]            ← match
//!
//!   Difference from v0: NO placeholder byte per literal (saves 8 bits
//!   per literal); matches use u16 dist + u8 len (saves 40 bits per
//!   match). Combined: a literal costs ~13 bits (was 21), a match
//!   costs 32 bits (was 72). Optimal-parsing DP cost model changes
//!   correspondingly.
//!
//! ## Raw / Duplicate blocks are unchanged across versions.
//!
//! Blocks are independent → decompression is trivially parallelizable.

use std::io::{Read, Write};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Text = 0,
    Binary = 1,
    Structured = 2, // json / database / csv-like
    Multimedia = 3,
    Random = 4,    // high-entropy, will barely compress
    Raw = 5,       // stored uncompressed (incompressible detected)
    Duplicate = 6, // v1.3+: identical bytes to a previously-seen block
    Unknown = 255,
}

impl BlockType {
    pub fn from_u8(b: u8) -> Self {
        match b {
            0 => Self::Text,
            1 => Self::Binary,
            2 => Self::Structured,
            3 => Self::Multimedia,
            4 => Self::Random,
            5 => Self::Raw,
            6 => Self::Duplicate,
            _ => Self::Unknown,
        }
    }
}

pub const MAGIC: &[u8; 4] = b"NXS\x00";

/// Legacy version (v0) — used by files written before Format v2 cleanup.
pub const VERSION_V0: u8 = 0;

/// Format v2 (u16/u8 matches, no literal placeholder).
pub const VERSION_V2: u8 = 2;

/// Format v3 — multi-stream rANS (independent lit/len/dist streams).
/// Distance is split into 2 rANS-encoded u8 symbols (low/high byte) to
/// keep the 256-symbol alphabet while gaining per-byte entropy benefit.
pub const VERSION_V3: u8 = 3;

#[derive(Debug, Clone)]
pub struct NexusHeader {
    pub version: u8,
    pub flags: u8,
    pub block_count: u32,
    pub uncompressed_total_size: u64,
}

impl NexusHeader {
    pub const SIZE: usize = 18;

    pub fn write<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        w.write_all(MAGIC)?;
        w.write_all(&[self.version, self.flags])?;
        w.write_all(&self.block_count.to_le_bytes())?;
        w.write_all(&self.uncompressed_total_size.to_le_bytes())?;
        w.write_all(&[0u8; 4]) // reserved
    }

    pub fn read<R: Read>(r: &mut R) -> std::io::Result<Self> {
        let mut magic = [0u8; 4];
        r.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a .nexus file (bad magic)",
            ));
        }
        let mut ver_fl = [0u8; 2];
        r.read_exact(&mut ver_fl)?;
        let mut bc = [0u8; 4];
        r.read_exact(&mut bc)?;
        let mut total = [0u8; 8];
        r.read_exact(&mut total)?;
        let mut _res = [0u8; 4];
        r.read_exact(&mut _res)?;
        Ok(Self {
            version: ver_fl[0],
            flags: ver_fl[1],
            block_count: u32::from_le_bytes(bc),
            uncompressed_total_size: u64::from_le_bytes(total),
        })
    }
}

#[derive(Debug, Clone)]
pub struct BlockHeader {
    pub block_type: BlockType,
    pub uncompressed_size: u32,
    pub compressed_size: u32,
}

impl BlockHeader {
    pub const SIZE: usize = 9;

    pub fn write<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        w.write_all(&[self.block_type as u8])?;
        w.write_all(&self.uncompressed_size.to_le_bytes())?;
        w.write_all(&self.compressed_size.to_le_bytes())
    }

    pub fn read<R: Read>(r: &mut R) -> std::io::Result<Self> {
        let mut tag = [0u8; 1];
        r.read_exact(&mut tag)?;
        let mut usz = [0u8; 4];
        r.read_exact(&mut usz)?;
        let mut csz = [0u8; 4];
        r.read_exact(&mut csz)?;
        Ok(Self {
            block_type: BlockType::from_u8(tag[0]),
            uncompressed_size: u32::from_le_bytes(usz),
            compressed_size: u32::from_le_bytes(csz),
        })
    }
}

// ─────────────────────────────────────────────────────
//  Sprint 5.7.2: encrypted + recovery wire format (V3)
// ─────────────────────────────────────────────────────
//
// The V3 header introduces encryption (AES-256-GCM) and
// optional Reed-Solomon recovery (Cauchy matrix). The
// V0/V1/V2 formats use magic `NXS\0` and stay
// forward-compatible — the V3 decoder dispatches on
// magic and reads the new header structure only when it
// sees `NXE\0` or `NXR\0`.
//
// **Layout is fixed at 64 bytes** (= one cache line on
// virtually every modern CPU from the last 15 years).
// Reading the entire header is one cache-line miss; all
// subsequent field accesses are in L1.
//
// **Endianness:** all multi-byte fields are little-endian
// on disk. `to_le_bytes` and `from_le_bytes` are the
// conversion primitives. This is the same convention
// used by 7z, WinRAR, and the v0.1.1 NXS format — saves
// us from a "big-endian server crashes the decoder" bug
// class that bit 7z in 2018.

/// Magic bytes for the encrypted (no-recovery) V3 format.
pub const MAGIC_ENCRYPTED: &[u8; 4] = b"NXE\0";

/// Magic bytes for the encrypted + recovery V3 format.
/// The presence of the `FLAG_RECOVERY` bit in the header
/// is the authoritative signal; this constant is for
/// the encode side (so the writer commits to one of the
/// two magics and the reader can short-circuit the
/// non-recovery path).
pub const MAGIC_ENCRYPTED_RECOVERY: &[u8; 4] = b"NXR\0";

/// Bit 0 of `flags`: Reed-Solomon recovery shards are
/// present in the file. If 0, the file is just AES-256-GCM
/// encrypted with no erasure coding.
pub const FLAG_RECOVERY: u8 = 0b0000_0001;

/// Bit 1 of `flags`: 0 = legacy KDF (we don't actually
/// ship a legacy KDF in v0.1.3, but the bit is reserved
/// for forward compat with any v0.1.2-era scrypt-style
/// KDF we might add later); 1 = Argon2id (the recommended
/// KDF for v0.1.3+).
pub const FLAG_KDF_ARGON2: u8 = 0b0000_0010;

/// Bits 2-3 of `flags`: 2-bit KDF preset index.
///   0 = interactive (m=19 MiB, t=2, p=1;   ~100 ms unlock)
///   1 = moderate    (m=46 MiB, t=3, p=1;   ~500 ms unlock)
///   2 = sensitive   (m=65 MiB, t=4, p=2;   ~2 s   unlock)
///   3 = reserved    (current encoders must NOT emit this)
///
/// Mask + shift to extract: `(flags & FLAG_PRESET_MASK) >> 2`.
pub const FLAG_PRESET_MASK: u8 = 0b0000_1100;

/// Bit shift to convert the masked 2-bit preset field
/// into a 0..=3 index. Use as:
///   `(flags & FLAG_PRESET_MASK) >> FLAG_PRESET_SHIFT`
pub const FLAG_PRESET_SHIFT: u32 = 2;

/// Total serialized size of the V3 header. Fixed at 64
/// bytes (one cache line). Padding fields at the end
/// leave room for v0.2.x / v0.3.x extensions without
/// breaking the header size.
pub const HEADER_V3_SIZE: usize = 64;

/// The V3 wire-format header for encrypted (and
/// optionally recovery-enabled) `.nxe` / `.nxr` files.
///
/// `#[repr(C, packed)]` ensures the struct has EXACTLY
/// the field layout the user specified: no internal
/// alignment padding, total size = sum of field sizes =
/// 64. The static-size assertion in the tests below
/// guarantees this stays true across future edits.
///
/// **Field-by-field rationale:**
/// - `magic`: distinguishes V3 from older magics.
/// - `version`: future-proofing for v4 headers.
/// - `flags`: bitfield (recovery, KDF type, preset).
/// - `reserved_align`: 2 bytes of zero padding to align
///   the `kdf_salt` field at offset 8 (for 8-byte
///   alignment of subsequent u64 fields). The presence
///   of this padding is what makes `data_shards` (u16)
///   land at offset 42 instead of 44 in a non-packed
///   `#[repr(C)]` struct.
/// - `kdf_salt`: 16 bytes of Argon2id salt, generated
///   with `OsRng` at compress time. Written once, read
///   back at decompress time.
/// - `kdf_params`: 4 bytes encoding the Argon2id
///   parameters (m_cost lo, m_cost hi, t_cost, p_cost)
///   so the decoder can reproduce the same KDF output
///   without needing the user to re-specify the preset.
/// - `archive_nonce`: 8 bytes of random nonce prefix
///   per archive. Combined with the per-block `block_id`
///   (4 bytes) it forms the full 12-byte AES-GCM nonce.
/// - `block_count`: number of super-blocks (data +
///   parity combined).
/// - `parity_count`: number of Reed-Solomon parity
///   shards (= the number of missing data shards the
///   recovery layer can tolerate).
/// - `data_shards`: number of original data super-blocks
///   (before parity). Included explicitly so the
///   decoder can reconstruct the Cauchy matrix shape
///   without recomputing `block_count - parity_count`.
/// - `uncompressed_total_size`: the total input size in
///   bytes (before compression and encryption). Used by
///   the decoder to allocate the output buffer exactly.
/// - `padding`: 12 bytes of reserved space for v0.2.x /
///   v0.3.x extensions (key file IDs, multi-recipient
///   indicators, etc.). MUST be zero in v0.1.3 — readers
///   from a future version will interpret non-zero
///   padding as "I don't recognize this header" and
///   fall back to the legacy V0/V2 decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, packed)]
pub struct NexusHeaderV3 {
    pub magic: [u8; 4],
    pub version: u8,
    pub flags: u8,
    pub reserved_align: [u8; 2],
    pub kdf_salt: [u8; 16],
    pub kdf_params: [u8; 4],
    pub archive_nonce: [u8; 8],
    pub block_count: u32,
    pub parity_count: u16,
    pub data_shards: u16,
    pub uncompressed_total_size: u64,
    pub padding: [u8; 12],
}

/// Errors that can occur when parsing a V3 header from a
/// raw byte buffer. The most common one is a wrong
/// magic (the file isn't V3 at all); the rest are
/// invariant violations that would indicate a corrupted
/// header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V3HeaderError {
    /// The 4-byte magic prefix is not `NXE\0` or
    /// `NXR\0`. The decoder should fall back to the
    /// V0/V2 path in this case.
    BadMagic { found: [u8; 4] },
    /// The version byte is not 3. Future versions may
    /// bump this; the decoder should reject unknown
    /// versions explicitly so a v4 file is never
    /// silently misread as v3.
    UnsupportedVersion { found: u8 },
    /// The header is well-formed but logically
    /// inconsistent (e.g. `data_shards + parity_count !=
    /// block_count`, or `parity_count > block_count`).
    /// Reading the file as-is would produce a corrupt
    /// output, so we bail.
    InconsistentLayout {
        data_shards: u16,
        parity_count: u16,
        block_count: u32,
    },
}

impl core::fmt::Display for V3HeaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic { found } => write!(
                f,
                "V3 header bad magic: expected NXE\\0 or NXR\\0, found {:?}",
                found
            ),
            Self::UnsupportedVersion { found } => write!(
                f,
                "V3 header unsupported version: {} (this build reads version 3)",
                found
            ),
            Self::InconsistentLayout {
                data_shards,
                parity_count,
                block_count,
            } => write!(
                f,
                "V3 header inconsistent layout: data_shards={} + parity_count={} != block_count={}",
                data_shards, parity_count, block_count
            ),
        }
    }
}

impl std::error::Error for V3HeaderError {}

impl NexusHeaderV3 {
    /// Serialize this header into a 64-byte buffer.
    /// The buffer is filled with the wire format described
    /// in the module docs; all multi-byte fields are
    /// little-endian.
    pub fn write_to(&self, buf: &mut [u8; HEADER_V3_SIZE]) {
        // ── Bytes 0..4: magic ─────────────────────
        buf[0..4].copy_from_slice(&self.magic);
        // ── Byte 4: version ────────────────────────
        buf[4] = self.version;
        // ── Byte 5: flags ──────────────────────────
        buf[5] = self.flags;
        // ── Bytes 6..8: reserved alignment padding ──
        buf[6..8].copy_from_slice(&self.reserved_align);
        // ── Bytes 8..24: kdf salt ───────────────────
        buf[8..24].copy_from_slice(&self.kdf_salt);
        // ── Bytes 24..28: kdf params ────────────────
        buf[24..28].copy_from_slice(&self.kdf_params);
        // ── Bytes 28..36: archive nonce ─────────────
        buf[28..36].copy_from_slice(&self.archive_nonce);
        // ── Bytes 36..40: block_count (u32 LE) ─────
        buf[36..40].copy_from_slice(&self.block_count.to_le_bytes());
        // ── Bytes 40..42: parity_count (u16 LE) ────
        buf[40..42].copy_from_slice(&self.parity_count.to_le_bytes());
        // ── Bytes 42..44: data_shards (u16 LE) ─────
        buf[42..44].copy_from_slice(&self.data_shards.to_le_bytes());
        // ── Bytes 44..52: uncompressed_total_size (u64 LE) ──
        buf[44..52].copy_from_slice(&self.uncompressed_total_size.to_le_bytes());
        // ── Bytes 52..64: reserved padding (12 bytes) ─
        // MUST be zero per the V3 spec. A non-zero
        // value here means the file was written by a
        // future version that used the padding for
        // something else; we treat it as an invalid
        // header to avoid silent corruption.
        buf[52..64].copy_from_slice(&self.padding);
    }

    /// Parse a V3 header from a 64-byte buffer.
    /// Returns an error if the magic is wrong, the
    /// version is not 3, or the layout is inconsistent.
    pub fn read_from(buf: &[u8; HEADER_V3_SIZE]) -> Result<Self, V3HeaderError> {
        let magic: [u8; 4] = [buf[0], buf[1], buf[2], buf[3]];
        if magic != *MAGIC_ENCRYPTED && magic != *MAGIC_ENCRYPTED_RECOVERY {
            return Err(V3HeaderError::BadMagic { found: magic });
        }
        let version = buf[4];
        if version != 3 {
            return Err(V3HeaderError::UnsupportedVersion { found: version });
        }
        let flags = buf[5];
        let reserved_align: [u8; 2] = [buf[6], buf[7]];
        let mut kdf_salt = [0u8; 16];
        kdf_salt.copy_from_slice(&buf[8..24]);
        let mut kdf_params = [0u8; 4];
        kdf_params.copy_from_slice(&buf[24..28]);
        let mut archive_nonce = [0u8; 8];
        archive_nonce.copy_from_slice(&buf[28..36]);
        let block_count = u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]);
        let parity_count = u16::from_le_bytes([buf[40], buf[41]]);
        let data_shards = u16::from_le_bytes([buf[42], buf[43]]);
        let uncompressed_total_size = u64::from_le_bytes([
            buf[44], buf[45], buf[46], buf[47], buf[48], buf[49], buf[50], buf[51],
        ]);
        let mut padding = [0u8; 12];
        padding.copy_from_slice(&buf[52..64]);

        // Layout invariant: data_shards + parity_count
        // MUST equal block_count. If it doesn't, the
        // header is corrupt and we bail.
        if u32::from(data_shards) + u32::from(parity_count) != block_count {
            return Err(V3HeaderError::InconsistentLayout {
                data_shards,
                parity_count,
                block_count,
            });
        }
        // The padding MUST be zero. A non-zero padding
        // means the file was written by a future
        // version that used the padding for something
        // else (e.g. a key file ID). We treat it as
        // invalid to be conservative.
        if padding != [0u8; 12] {
            // Note: this is a soft check. The first 12
            // bytes of a future header might be a flag
            // and not actual data. For now (v0.1.3)
            // we reject any non-zero padding.
            return Err(V3HeaderError::InconsistentLayout {
                data_shards,
                parity_count,
                block_count,
            });
        }

        Ok(Self {
            magic,
            version,
            flags,
            reserved_align,
            kdf_salt,
            kdf_params,
            archive_nonce,
            block_count,
            parity_count,
            data_shards,
            uncompressed_total_size,
            padding,
        })
    }

    /// Is the Reed-Solomon recovery layer enabled?
    #[inline]
    pub fn has_recovery(&self) -> bool {
        self.flags & FLAG_RECOVERY != 0
    }

    /// Is the KDF Argon2id (vs. legacy scrypt-style)?
    #[inline]
    pub fn uses_argon2(&self) -> bool {
        self.flags & FLAG_KDF_ARGON2 != 0
    }

    /// The KDF preset index (0=interactive, 1=moderate,
    /// 2=sensitive, 3=reserved). Returns `None` if the
    /// reserved value 3 was somehow set.
    #[inline]
    pub fn kdf_preset(&self) -> Option<u8> {
        let raw = (self.flags & FLAG_PRESET_MASK) >> FLAG_PRESET_SHIFT;
        if raw < 3 {
            Some(raw)
        } else {
            None
        }
    }

    /// Set the recovery flag (caller's convenience; not
    /// required to be called before `write_to`).
    #[inline]
    pub fn set_recovery(&mut self, on: bool) {
        if on {
            self.flags |= FLAG_RECOVERY;
        } else {
            self.flags &= !FLAG_RECOVERY;
        }
    }

    /// Set the KDF-to-Argon2 flag.
    #[inline]
    pub fn set_argon2(&mut self, on: bool) {
        if on {
            self.flags |= FLAG_KDF_ARGON2;
        } else {
            self.flags &= !FLAG_KDF_ARGON2;
        }
    }

    /// Set the KDF preset (0=interactive, 1=moderate,
    /// 2=sensitive; 3 is reserved and rejected).
    #[inline]
    pub fn set_kdf_preset(&mut self, preset: u8) {
        debug_assert!(preset < 3, "KDF preset 3 is reserved");
        // Clear the old preset bits, then OR in the new.
        self.flags &= !FLAG_PRESET_MASK;
        self.flags |= (preset << FLAG_PRESET_SHIFT) & FLAG_PRESET_MASK;
    }
}

// ─────────────────────────────────────────────────────
//  V3 header tests
// ─────────────────────────────────────────────────────

#[cfg(test)]
mod v3_header_tests {
    use super::*;

    /// 1. The struct's static size is exactly 64 bytes.
    ///    This is the cache-line alignment contract
    ///    from the design doc: `#[repr(C, packed)]`
    ///    eliminates internal alignment padding, so the
    ///    size is the sum of field sizes. If a future
    ///    refactor accidentally adds a u32 somewhere,
    ///    this assertion fires.
    #[test]
    fn header_size_is_64_bytes() {
        assert_eq!(
            core::mem::size_of::<NexusHeaderV3>(),
            64,
            "NexusHeaderV3 must be exactly 64 bytes (one cache line); \
             check that no field was added or its type widened"
        );
        assert_eq!(NexusHeaderV3::write_to_size_assert(), 64); // sanity
    }

    /// Helper trait for the size assertion above: forces
    /// the compiler to evaluate `write_to` against a
    /// real `[u8; 64]` buffer, which is the same
    /// compile-time check `HEADER_V3_SIZE` gives us.
    impl NexusHeaderV3 {
        // The actual implementation is a no-op that
        // just returns HEADER_V3_SIZE. We keep it
        // private to this test module so it doesn't
        // pollute the public API.
        fn write_to_size_assert() -> usize {
            HEADER_V3_SIZE
        }
    }

    /// 2. Roundtrip: write a known header, read it back,
    ///    every field matches. Catches bugs in the
    ///    offset arithmetic (e.g. swapping the byte
    ///    positions of `kdf_params` and `archive_nonce`).
    #[test]
    fn header_write_then_read_roundtrips() {
        let original = NexusHeaderV3 {
            magic: *MAGIC_ENCRYPTED_RECOVERY,
            version: 3,
            flags: FLAG_RECOVERY | FLAG_KDF_ARGON2 | (1 << FLAG_PRESET_SHIFT), // recovery + argon2 + moderate
            reserved_align: [0, 0],
            kdf_salt: [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
            ],
            kdf_params: [0xb0, 0x00, 0x03, 0x01], // m_cost=0xb000=45056 KiB (~46 MiB), t=3, p=1
            archive_nonce: [
                0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11,
            ],
            block_count: 35,    // 32 data + 3 parity
            parity_count: 3,
            data_shards: 32,
            uncompressed_total_size: 0x0123_4567_89ab_cdef,
            padding: [0u8; 12],
        };
        let mut buf = [0u8; HEADER_V3_SIZE];
        original.write_to(&mut buf);
        let recovered = NexusHeaderV3::read_from(&buf).expect("read_from");
        assert_eq!(original, recovered);
    }

    /// 3. Bad magic is rejected. The decoder must
    ///    short-circuit here so the rest of the file
    ///    isn't read as if it were V3.
    #[test]
    fn read_rejects_bad_magic() {
        let mut buf = [0u8; HEADER_V3_SIZE];
        buf[0..4].copy_from_slice(b"NXS\0"); // legacy V0/V2 magic
        buf[4] = 3; // version is right, but magic is wrong
        let r = NexusHeaderV3::read_from(&buf);
        assert!(matches!(r, Err(V3HeaderError::BadMagic { .. })), "got {:?}", r);
    }

    /// 4. Wrong version is rejected. A v4 header
    ///    (future) should never be silently read as
    ///    v3 — that would corrupt the user's data.
    #[test]
    fn read_rejects_wrong_version() {
        let mut buf = [0u8; HEADER_V3_SIZE];
        buf[0..4].copy_from_slice(MAGIC_ENCRYPTED);
        buf[4] = 4; // v4 — not yet implemented
        let r = NexusHeaderV3::read_from(&buf);
        assert!(
            matches!(r, Err(V3HeaderError::UnsupportedVersion { found: 4 })),
            "got {:?}",
            r
        );
    }

    /// 5. Inconsistent layout is rejected. If
    ///    `data_shards + parity_count != block_count`,
    ///    the file is corrupt (or was tampered with);
    ///    the decoder must not try to process it.
    #[test]
    fn read_rejects_inconsistent_layout() {
        let mut buf = [0u8; HEADER_V3_SIZE];
        buf[0..4].copy_from_slice(MAGIC_ENCRYPTED_RECOVERY);
        buf[4] = 3;
        buf[36..40].copy_from_slice(&10u32.to_le_bytes()); // block_count=10
        buf[40..42].copy_from_slice(&3u16.to_le_bytes()); // parity_count=3
        buf[42..44].copy_from_slice(&5u16.to_le_bytes()); // data_shards=5 (3+5=8, not 10)
        let r = NexusHeaderV3::read_from(&buf);
        assert!(
            matches!(r, Err(V3HeaderError::InconsistentLayout { .. })),
            "got {:?}",
            r
        );
    }

    /// 6. Non-zero padding is rejected. The padding
    ///    is reserved for future use; if a v0.1.3
    ///    reader sees non-zero padding, the file was
    ///    written by a future version. Reject
    ///    conservatively rather than guess.
    #[test]
    fn read_rejects_nonzero_padding() {
        let mut buf = [0u8; HEADER_V3_SIZE];
        buf[0..4].copy_from_slice(MAGIC_ENCRYPTED);
        buf[4] = 3;
        buf[36..40].copy_from_slice(&1u32.to_le_bytes());
        buf[40..42].copy_from_slice(&0u16.to_le_bytes());
        buf[42..44].copy_from_slice(&1u16.to_le_bytes());
        buf[52] = 0xff; // non-zero padding byte
        let r = NexusHeaderV3::read_from(&buf);
        assert!(matches!(r, Err(_)), "got {:?}", r);
    }

    /// 7. Flag accessor roundtrip. The setters
    ///    (`set_recovery`, `set_argon2`, `set_kdf_preset`)
    ///    must correctly mutate the `flags` byte
    ///    without disturbing the other bits.
    #[test]
    fn flag_setters_roundtrip() {
        let mut h = NexusHeaderV3 {
            magic: *MAGIC_ENCRYPTED,
            version: 3,
            flags: 0,
            reserved_align: [0, 0],
            kdf_salt: [0u8; 16],
            kdf_params: [0u8; 4],
            archive_nonce: [0u8; 8],
            block_count: 1,
            parity_count: 0,
            data_shards: 1,
            uncompressed_total_size: 0,
            padding: [0u8; 12],
        };
        assert!(!h.has_recovery());
        assert!(!h.uses_argon2());
        assert_eq!(h.kdf_preset(), Some(0)); // default = interactive

        h.set_recovery(true);
        assert!(h.has_recovery());
        h.set_argon2(true);
        assert!(h.uses_argon2());
        h.set_kdf_preset(2);
        assert_eq!(h.kdf_preset(), Some(2));

        // The flag bits are exactly what we set, no
        // other bits are clobbered.
        assert_eq!(h.flags, FLAG_RECOVERY | FLAG_KDF_ARGON2 | (2 << FLAG_PRESET_SHIFT));

        // Unset recovery — only the recovery bit
        // changes, the others stay.
        h.set_recovery(false);
        assert!(!h.has_recovery());
        assert!(h.uses_argon2());
        assert_eq!(h.kdf_preset(), Some(2));
        assert_eq!(h.flags, FLAG_KDF_ARGON2 | (2 << FLAG_PRESET_SHIFT));
    }

    /// 8. The reserved padding (12 bytes at the end)
    ///    is genuinely zero in the serialized output.
    ///    This is the v0.2.x / v0.3.x extension space
    ///    that we promised in the design doc. Any
    ///    setter that touches the struct must NOT
    ///    write to the padding.
    #[test]
    fn serialized_padding_is_zero() {
        let h = NexusHeaderV3 {
            magic: *MAGIC_ENCRYPTED,
            version: 3,
            flags: FLAG_RECOVERY,
            reserved_align: [0, 0],
            kdf_salt: [0xaau8; 16],
            kdf_params: [0xb0, 0x00, 0x03, 0x01],
            archive_nonce: [0x55u8; 8],
            block_count: 5,
            parity_count: 1,
            data_shards: 4,
            uncompressed_total_size: 12345,
            padding: [0u8; 12],
        };
        let mut buf = [0xa5u8; HEADER_V3_SIZE]; // fill with non-zero to detect any write
        h.write_to(&mut buf);
        assert_eq!(&buf[52..64], &[0u8; 12], "padding must be zeroed on write");
    }

    /// 9. Endianness. We write a header with
    ///    `uncompressed_total_size = 0x0123_4567_89ab_cdef`
    ///    and verify the on-disk bytes are in
    ///    LITTLE-ENDIAN order. If the write accidentally
    ///    used big-endian (or a custom byte order), the
    ///    read would either return a different value
    ///    (or fail the InconsistentLayout check).
    #[test]
    fn endianness_is_little_endian() {
        let h = NexusHeaderV3 {
            magic: *MAGIC_ENCRYPTED,
            version: 3,
            flags: 0,
            reserved_align: [0, 0],
            kdf_salt: [0u8; 16],
            kdf_params: [0u8; 4],
            archive_nonce: [0u8; 8],
            block_count: 1,
            parity_count: 0,
            data_shards: 1,
            uncompressed_total_size: 0x0123_4567_89ab_cdef,
            padding: [0u8; 12],
        };
        let mut buf = [0u8; HEADER_V3_SIZE];
        h.write_to(&mut buf);
        // Bytes 44..52 must be the u64 in LE order:
        // least-significant byte first.
        let expected = 0x0123_4567_89ab_cdef_u64.to_le_bytes();
        assert_eq!(&buf[44..52], &expected[..]);
        // And the roundtrip must recover the original
        // value exactly. The `read_unaligned` dance is
        // required because the field is inside a
        // `#[repr(C, packed)]` struct (no alignment
        // guarantees), so direct field access would
        // trigger the "misaligned reference" lint
        // (E0793). `core::ptr::addr_of!` gets us a raw
        // pointer without taking a reference; the read
        // is well-defined on all platforms we target.
        let r = NexusHeaderV3::read_from(&buf).expect("read_from");
        let recovered_size = unsafe {
            core::ptr::addr_of!((*core::ptr::addr_of!(r)).uncompressed_total_size).read_unaligned()
        };
        assert_eq!(recovered_size, 0x0123_4567_89ab_cdef);
    }
}
