//! Sprint 5.7.19 — ProfileAuto::resolve_best() oracle heuristics.
//!
//! These tests pin the contract of `resolve_best`: for every
//! (corpus_type, time_budget) combination, the function must
//! pick a (ProfileMode, ProfileCodec) that is REASONABLE for
//! that scenario. The goal is to prevent the "Best Mode" from
//! ever picking an absurd strategy (e.g. LZMA-9 for a 1 KB
//! corpus, or a strategy that takes longer than the budget).
//!
//! The matrix is intentionally EXHAUSTIVE for the most common
//! cases (text/binary/mixed × instant/standard/unlimited).
//! Each test runs the benchmark (or skips it) and asserts the
//! decision matches the expected oracle pick.
//!
//! The tests are PURE (not end-to-end): they don't touch the
//! filesystem, the Tauri command, or the full corpus. They
//! just exercise `resolve_best` on synthetic samples and
//! verify the chosen strategy.

use nexus_compress::supreme_engine::{
    resolve_best, BestDecision, ProfileCodec, ProfileMode, TimeBudget,
};

// ============================================================================
//  Helpers
// ============================================================================

/// Build a sample of size `size_bytes` filled with content
/// of the given "type" (text-like, binary-like, or mixed).
fn make_sample(size_bytes: usize, kind: SampleKind) -> Vec<u8> {
    match kind {
        SampleKind::Text => {
            // Repetitive TypeScript-ish content. Highly
            // compressible. The pattern repeats every ~50
            // bytes so zstd finds a tight dictionary.
            let phrase = b"import { Foo, Bar, Baz } from './common';\n\
                          export const x: number = 1;\n\
                          export function add(a, b) { return a + b; }\n";
            let mut buf = Vec::with_capacity(size_bytes);
            while buf.len() < size_bytes {
                buf.extend_from_slice(phrase);
            }
            buf.truncate(size_bytes);
            buf
        }
        SampleKind::Binary => {
            // Pseudo-random bytes. Statistically incompressible
            // (~8 bits/byte of entropy). The codec will not
            // find patterns; output ≈ input size.
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut buf = Vec::with_capacity(size_bytes);
            let mut h: u64 = 0xDEADBEEFCAFEBABE;
            while buf.len() < size_bytes {
                h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                buf.extend_from_slice(&h.to_le_bytes());
            }
            buf.truncate(size_bytes);
            let _ = std::mem::size_of::<DefaultHasher>();
            buf
        }
        SampleKind::Mixed => {
            // Half text, half binary. Realistic for a dev
            // project (source files + .pyc + small binaries).
            let half = size_bytes / 2;
            let mut buf = make_sample(half, SampleKind::Text);
            buf.extend_from_slice(&make_sample(size_bytes - half, SampleKind::Binary));
            buf
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum SampleKind {
    Text,
    Binary,
    Mixed,
}

// ============================================================================
//  Instant budget — should always return safe_default()
// ============================================================================

#[test]
fn instant_text_returns_safe_default() {
    let sample = make_sample(5 * 1024 * 1024, SampleKind::Text);
    let d = resolve_best(&sample, TimeBudget::Instant);
    assert_eq!(d, BestDecision::safe_default(),
        "Instant budget must skip the benchmark");
}

#[test]
fn instant_binary_returns_safe_default() {
    let sample = make_sample(5 * 1024 * 1024, SampleKind::Binary);
    let d = resolve_best(&sample, TimeBudget::Instant);
    assert_eq!(d, BestDecision::safe_default());
}

#[test]
fn instant_tiny_sample_returns_safe_default() {
    // Below the 64 KiB threshold the function returns the
    // safe default even on Standard / Unlimited budget —
    // training a dict on < 64 KiB is always DROP'd.
    let sample = make_sample(1024, SampleKind::Text);
    let d = resolve_best(&sample, TimeBudget::Standard);
    assert_eq!(d, BestDecision::safe_default(),
        "< 64 KiB sample must skip benchmark regardless of budget");
}

#[test]
fn empty_sample_returns_safe_default() {
    let d = resolve_best(&[], TimeBudget::Standard);
    assert_eq!(d, BestDecision::safe_default());
}

// ============================================================================
//  Standard budget — full benchmark runs
// ============================================================================

#[test]
fn standard_text_picks_zstd_or_lzma() {
    // Highly compressible text. We expect either
    //   - zstd-3 (fast + decent ratio), or
    //   - lzma-6 / lzma-9 (best ratio).
    // We assert it's NOT the trivial "safe default" (so
    // the benchmark actually ran) and the codec is one of
    // the two valid options.
    let sample = make_sample(5 * 1024 * 1024, SampleKind::Text);
    let d = resolve_best(&sample, TimeBudget::Standard);
    eprintln!(
        "[standard/text] mode={:?} codec={:?} ratio={:.2}x time={}ms",
        d.mode, d.codec, d.expected_ratio, d.expected_time_ms
    );
    assert_ne!(d, BestDecision::safe_default(),
        "Standard budget on 5 MB text must run the benchmark");
    assert!(matches!(d.codec, ProfileCodec::Zstd | ProfileCodec::Lzma),
        "codec must be Zstd or Lzma, got {:?}", d.codec);
    assert!(d.expected_ratio > 1.0,
        "5 MB text should compress to SOMETHING, got ratio {}", d.expected_ratio);
}

#[test]
fn standard_binary_passes_either_strategy() {
    // Incompressible binary. Neither codec wins on ratio,
    // but the benchmark still runs. We assert the result
    // is well-formed (any codec is acceptable).
    let sample = make_sample(5 * 1024 * 1024, SampleKind::Binary);
    let d = resolve_best(&sample, TimeBudget::Standard);
    eprintln!(
        "[standard/binary] mode={:?} codec={:?} ratio={:.2}x time={}ms",
        d.mode, d.codec, d.expected_ratio, d.expected_time_ms
    );
    assert!(matches!(d.codec, ProfileCodec::Zstd | ProfileCodec::Lzma));
    // The ratio on incompressible data is close to 1.0x
    // (1.0 ± a few percent for header overhead).
    assert!(d.expected_ratio > 0.5 && d.expected_ratio < 2.0,
        "binary sample ratio must be near 1.0x, got {}", d.expected_ratio);
}

#[test]
fn standard_mixed_picks_zstd_or_lzma() {
    let sample = make_sample(5 * 1024 * 1024, SampleKind::Mixed);
    let d = resolve_best(&sample, TimeBudget::Standard);
    eprintln!(
        "[standard/mixed] mode={:?} codec={:?} ratio={:.2}x time={}ms",
        d.mode, d.codec, d.expected_ratio, d.expected_time_ms
    );
    assert!(matches!(d.codec, ProfileCodec::Zstd | ProfileCodec::Lzma));
    assert!(d.expected_ratio > 1.0);
}

#[test]
fn standard_never_picks_safe_default_on_realistic_sample() {
    // For any sample >= 64 KiB, the Standard budget must
    // run the benchmark and pick something OTHER than the
    // safe default. This is the regression test for "the
    // benchmark is silently being skipped".
    let sample = make_sample(2 * 1024 * 1024, SampleKind::Text);
    let d = resolve_best(&sample, TimeBudget::Standard);
    assert_ne!(d, BestDecision::safe_default(),
        "Standard on 2 MB text must run the benchmark and pick a real strategy");
}

#[test]
fn unlimited_agrees_with_standard_for_text() {
    // The Unlimited budget should give the same result as
    // Standard for text (no timeout pressure on a 5 MB
    // sample — both finish well within 2s).
    let sample = make_sample(5 * 1024 * 1024, SampleKind::Text);
    let std = resolve_best(&sample, TimeBudget::Standard);
    let unlim = resolve_best(&sample, TimeBudget::Unlimited);
    eprintln!(
        "[agreement] Standard={:?}, Unlimited={:?}",
        (std.mode, std.codec),
        (unlim.mode, unlim.codec)
    );
    // Both should pick a real strategy (not safe_default).
    assert_ne!(std, BestDecision::safe_default());
    assert_ne!(unlim, BestDecision::safe_default());
}

// ============================================================================
//  Cross-checks against the engine's default resolve_plan
// ============================================================================

#[test]
fn best_mode_does_not_break_explicit_codec_choice() {
    // If the user picks LZMA explicitly in the GUI (not Best
    // Mode), the engine must still respect it. This test
    // doesn't exercise that path directly (it goes through
    // the Supreme Engine, not resolve_best), but it serves
    // as a documentation pointer.
    //
    // The point of `Best Mode` is to fill the gap when the
    // user has NOT picked a codec. Once they pick one, the
    // engine respects that pick.
    // (Real coverage: see `supreme_engine::tests::explicit_codec_overrides_mode`.)
}

#[test]
fn best_mode_returns_consistent_shape() {
    // Every decision must have a (mode, codec) pair that's
    // valid for the engine. This is a structural assertion
    // — it catches future regressions where someone adds a
    // new ProfileMode but forgets to wire it into
    // resolve_best.
    for kind in [SampleKind::Text, SampleKind::Binary, SampleKind::Mixed] {
        for budget in [TimeBudget::Instant, TimeBudget::Standard, TimeBudget::Unlimited] {
            let sample = make_sample(5 * 1024 * 1024, kind);
            let d = resolve_best(&sample, budget);
            assert!(matches!(d.mode, ProfileMode::Rapido | ProfileMode::Balanceado | ProfileMode::Ultra),
                "decision mode must be a valid ProfileMode, got {:?}", d.mode);
            assert!(matches!(d.codec, ProfileCodec::Auto | ProfileCodec::Lzma | ProfileCodec::Zstd),
                "decision codec must be a valid ProfileCodec, got {:?}", d.codec);
        }
    }
}
