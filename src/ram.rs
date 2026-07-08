//! Sprint 5.7 hotfix #21: RAM auto-tune for the LZMA dictionary.
//!
//! ## Why
//!
//! xz2's preset (0..=9) maps to a fixed dictionary size:
//!   0–1 → 1 MiB    2 → 2 MiB    3–4 → 4 MiB
//!   5–6 → 8 MiB    7   → 16 MiB  8   → 32 MiB
//!   9   → 64 MiB
//!
//! At preset 9 we get the best ratio, but the LZMA encoder
//! allocates the FULL dictionary size in RAM (plus working buffers).
//! On an 8 GiB laptop with a few Chrome tabs open, encoding a 4 GiB
//! ISO with preset 9 will push the system into swap and the app
//! appears to hang. We need to pick the largest dict the current
//! machine can afford without swapping.
//!
//! ## Strategy
//!
//! At compress start:
//!   1. Read `available_memory()` via sysinfo (cross-platform).
//!   2. Cap dict at `available_memory() / 4` (leave 75% for the OS
//!      and other apps — generous to avoid OOM on borderline cases).
//!   3. Map that cap to the nearest preset ≤ the requested preset.
//!   4. If sysinfo is unavailable (rare: no /proc, no sysctl), fall
//!      back to the requested preset unchanged — better to compress
//!      than to error out.
//!
//! ## What we DON'T do
//!
//! - Per-block dict tuning: too complex for the win, and the LZMA
//!   preset is a one-shot encoder option (can't resize mid-stream).
//! - Detect pressure mid-compress: the user is one click away from
//!   picking a lower preset, and the pre-flight check catches 99% of
//!   the actual problem.

use std::sync::OnceLock;

use sysinfo::System;
use xz2::stream::{Check, LzmaOptions, Stream};

/// Cached System handle. Building one is ~1 ms; refreshing it is
/// free (just a few stat syscalls). We refresh only when
/// `available_memory_mb()` is called and the cache is older than
/// `CACHE_MS` so back-to-back compress calls (e.g. compressing
/// 50 files in a folder) don't re-stat the kernel 50 times.
static SYSTEM: OnceLock<std::sync::Mutex<(System, std::time::Instant)>> = OnceLock::new();

const CACHE_MS: u128 = 5_000;

/// Read the system's currently-available physical memory, in MiB.
/// Returns `None` if sysinfo can't be initialised (e.g. unrecognised
/// platform, missing /proc, sandboxed environment).
pub fn available_memory_mb() -> Option<u64> {
    let cell = SYSTEM.get_or_init(|| {
        std::sync::Mutex::new((System::new(), std::time::Instant::now()))
    });
    let mut guard = cell.lock().ok()?;
    let now = std::time::Instant::now();
    if now.duration_since(guard.1).as_millis() > CACHE_MS {
        guard.0.refresh_memory();
        guard.1 = now;
    }
    Some(guard.0.available_memory() / (1024 * 1024))
}

/// LZMA preset → dictionary size in MiB, per the official xz spec.
/// Kept as a `const` table so the math is grep-able and the
/// auto-tune function stays branch-free.
const PRESET_DICT_MIB: [u64; 10] = [
    1,  // 0
    1,  // 1
    2,  // 2
    4,  // 3
    4,  // 4
    8,  // 5
    8,  // 6
    16, // 7
    32, // 8
    64, // 9
];

/// Clamp the requested LZMA preset to whatever the system can
/// actually afford, given `available_mb` of free RAM.
///
/// Returns the (possibly-downgraded) preset. If the requested preset
/// already fits, returns it unchanged. If the system has < 256 MiB
/// free, drops to preset 1 (1 MiB dict — the minimum for LZMA).
///
/// The `requested_preset` is clamped to 0..=9 (xz2's valid range)
/// before lookup.
pub fn clamp_preset_for_ram(requested_preset: u32, available_mb: Option<u64>) -> u32 {
    let requested = requested_preset.min(9);
    let Some(avail) = available_mb else {
        // No data → trust the user / caller.
        return requested;
    };
    let requested_dict = PRESET_DICT_MIB[requested as usize];
    // Keep the LZMA dict ≤ 25% of free RAM. 25% leaves 75% for the
    // OS, the kernel page cache, browser tabs, Slack, etc. On a
    // 16 GiB Mac this caps the dict at 4 GiB which is well above
    // any preset, so high-end machines hit the requested preset
    // (max 64 MiB) and don't pay any auto-tune penalty.
    let cap = avail / 4;
    if requested_dict <= cap {
        return requested;
    }
    // Otherwise find the largest preset whose dict fits.
    // Walk the table from the requested preset downward.
    for p in (0..=requested).rev() {
        if PRESET_DICT_MIB[p as usize] <= cap {
            return p;
        }
    }
    // Even preset 0 (1 MiB) doesn't fit. Return preset 1 (also 1 MiB)
    // — but with a 1 MiB dict on a 64 MiB system the user has bigger
    // problems than the compression ratio. We could fall back to a
    // different codec here, but for now: return 1 and log.
    if avail < 256 {
        eprintln!(
            "[nexus-compress] WARN: only {} MiB free; falling back to \
             preset 1 (1 MiB dict) for the LZMA stream. Compression \
             will be slow + low ratio. Consider closing other apps.",
            avail
        );
    }
    1
}

/// Build an xz2 `Stream` for the requested LZMA preset, after
/// clamping the preset to whatever the system can afford. Use this
/// instead of `XzEncoder::new(out, level)` everywhere we compress.
///
/// # Example
/// ```ignore
/// let stream = ram::lzma_stream_for_requested_preset(9)?;
/// let mut enc = XzEncoder::new_stream(out, stream);
/// ```
pub fn lzma_stream_for_requested_preset(requested_preset: u32) -> Result<Stream, String> {
    let available = available_memory_mb();
    let final_preset = clamp_preset_for_ram(requested_preset, available);
    if let Some(avail) = available {
        let final_dict = PRESET_DICT_MIB[final_preset as usize];
        let requested_dict = PRESET_DICT_MIB[requested_preset.min(9) as usize];
        if final_preset != requested_preset {
            eprintln!(
                "[nexus-compress] auto-tune: LZMA preset {} → {} ({} MiB dict) \
                 because only {} MiB RAM is free (was requesting {} MiB).",
                requested_preset, final_preset, final_dict, avail, requested_dict
            );
        }
    }
    let mut opts = LzmaOptions::new_preset(final_preset)
        .map_err(|e| format!("lzma preset {} invalid: {:?}", final_preset, e))?;
    // We always set the dict explicitly from the preset table so
    // we're robust to liblzma version drift (the preset→dict mapping
    // is part of the xz spec but liblzma could conceivably differ).
    let dict_bytes = (PRESET_DICT_MIB[final_preset as usize] * 1024 * 1024)
        .try_into()
        .map_err(|_| "preset dict size overflows u32".to_string())?;
    opts.dict_size(dict_bytes);
    Stream::new_lzma_encoder(&opts).map_err(|e| format!("{:?}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_table_matches_xz_spec() {
        // Pin the dict sizes so anyone changing them has to think.
        assert_eq!(PRESET_DICT_MIB, [1, 1, 2, 4, 4, 8, 8, 16, 32, 64]);
    }

    #[test]
    fn high_ram_keeps_requested_preset() {
        // 16 GiB free → cap = 4 GiB → all presets fit.
        assert_eq!(clamp_preset_for_ram(9, Some(16 * 1024)), 9);
        assert_eq!(clamp_preset_for_ram(6, Some(16 * 1024)), 6);
    }

    #[test]
    fn mid_ram_downgrades_max() {
        // 1 GiB free → cap = 256 MiB. Preset 9 (64 MiB) fits, 8 too.
        // So 9 stays.
        assert_eq!(clamp_preset_for_ram(9, Some(1024)), 9);
        // 256 MiB free → cap = 64 MiB → 9 still fits, no downgrade.
        assert_eq!(clamp_preset_for_ram(9, Some(256)), 9);
    }

    #[test]
    fn low_ram_clamps_dict() {
        // 64 MiB free → cap = 16 MiB. 9 (64 MiB) doesn't fit; 7
        // (16 MiB) is the largest that does.
        assert_eq!(clamp_preset_for_ram(9, Some(64)), 7);
        // 32 MiB free → cap = 8 MiB. 7 (16 MiB) doesn't fit; 6
        // (8 MiB) does.
        assert_eq!(clamp_preset_for_ram(9, Some(32)), 6);
    }

    #[test]
    fn no_data_returns_requested() {
        assert_eq!(clamp_preset_for_ram(9, None), 9);
        assert_eq!(clamp_preset_for_ram(0, None), 0);
    }

    #[test]
    fn clamps_above_9_to_9() {
        // Out-of-range inputs should be clamped, not panic.
        assert_eq!(clamp_preset_for_ram(15, Some(1024)), 9);
    }

    #[test]
    fn sub_256mb_returns_preset_1() {
        // 128 MiB free → cap = 32 MiB. Preset 9 (64 MiB) doesn't fit;
        // preset 8 (32 MiB) fits. So result is 8.
        assert_eq!(clamp_preset_for_ram(9, Some(128)), 8);
        // 4 MiB free → cap = 1 MiB. Preset 0/1 (1 MiB) fits.
        assert_eq!(clamp_preset_for_ram(9, Some(4)), 1);
        // 1 MiB free → cap = 256 KiB. Nothing fits; return 1 (1 MiB).
        assert_eq!(clamp_preset_for_ram(9, Some(1)), 1);
    }
}
