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

/// Sprint 5.7.1: magic prefix for a parallel-compressed stream.
/// A parallel file is a ParallelHeader (with this magic) followed
/// by N concatenated sequential-compressed blocks. The decoder
/// detects the magic, reads the block count, then iterates and
/// decodes each block with the existing sequential decoder.
pub const PARALLEL_MAGIC: &[u8; 4] = b"NXP\x00";

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
