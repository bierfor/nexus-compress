//! Sprint 5.7.19 — bench of resolve_best() on real corpora.
//!
//! Validates that the Best Mode orchestrator picks a
//! reasonable strategy on the canonical corpora we use for
//! benchmarks (corpus_real and synthetic mixes). The output
//! is informational; the oracle_heuristics tests pin the
//! REGRESSION behavior.

use nexus_compress::supreme_engine::{resolve_best, TimeBudget};

fn make_text_sample(size_bytes: usize) -> Vec<u8> {
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

fn make_binary_sample(size_bytes: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(size_bytes);
    let mut h: u64 = 0xDEADBEEFCAFEBABE;
    while buf.len() < size_bytes {
        h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        buf.extend_from_slice(&h.to_le_bytes());
    }
    buf.truncate(size_bytes);
    buf
}

#[test]
#[ignore = "bench: 5 MB sample, ~1s; run with --ignored"]
fn bench_resolve_best_on_corpus_real_sample() {
    // corpus_real/ is 9.25 MiB of TS/JSON/d.ts. We synthesize
    // a 5 MB sample that matches its profile (TypeScript).
    let sample = make_text_sample(5 * 1024 * 1024);
    eprintln!("[bench] corpus_real-style sample: {} bytes", sample.len());
    let d = resolve_best(&sample, TimeBudget::Standard);
    eprintln!(
        "[bench] Best picked: mode={:?} codec={:?} ratio={:.2}x time={}ms",
        d.mode, d.codec, d.expected_ratio, d.expected_time_ms
    );
}

#[test]
#[ignore = "bench: 5 MB sample, ~1s; run with --ignored"]
fn bench_resolve_best_on_incompressible_sample() {
    let sample = make_binary_sample(5 * 1024 * 1024);
    eprintln!("[bench] binary sample: {} bytes", sample.len());
    let d = resolve_best(&sample, TimeBudget::Standard);
    eprintln!(
        "[bench] Best picked: mode={:?} codec={:?} ratio={:.2}x time={}ms",
        d.mode, d.codec, d.expected_ratio, d.expected_time_ms
    );
}

#[test]
#[ignore = "bench: 5 MB sample, ~1s; run with --ignored"]
fn bench_resolve_best_speed() {
    use std::time::Instant;
    let sample = make_text_sample(5 * 1024 * 1024);
    let start = Instant::now();
    let _ = resolve_best(&sample, TimeBudget::Standard);
    let elapsed = start.elapsed();
    eprintln!(
        "[bench] resolve_best took {:.2}s (Standard budget, 4 strategies in parallel)",
        elapsed.as_secs_f64()
    );
    // The benchmark should finish well within 2 seconds on
    // an M4 Pro. We allow up to 5s for slow CI / debug builds.
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "resolve_best took {:.2}s, expected < 5s",
        elapsed.as_secs_f64()
    );
}
