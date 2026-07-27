// Sprint 5.7.6 hotfix #54: Sonic Scheduler — speedup
// benchmark across heterogeneous weight distributions.
//
// Compares LPT vs naive round-robin makespan for
// realistic weight distributions a real corpus might
// produce. The numbers here are the THEORETICAL
// makespan assuming work-per-byte is constant. In
// reality LZMA/zstd are not linear, but the
// qualitative result is the same: LPT gets the
// "biggest chunks" out of each thread's queue first,
// which is the right thing for a "do less work per
// byte" codec (compression).

use nexus_compress::scheduler::{lpt_partition, Weighted};

/// Theoretically optimal makespan: total_weight / n_threads
/// (if we could split chunks perfectly). This is a
/// lower bound; LPT gets within 4/3 of it for static
/// lists (Graham 1969).
fn optimal_makespan(weights: &[u64], n_threads: usize) -> u64 {
    let total: u64 = weights.iter().sum();
    (total + n_threads as u64 - 1) / n_threads as u64
}

fn naive_makespan(weights: &[u64], n_threads: usize) -> u64 {
    let mut sums = vec![0u64; n_threads];
    for (i, w) in weights.iter().enumerate() {
        sums[i % n_threads] += w;
    }
    *sums.iter().max().unwrap()
}

fn lpt_makespan(weights: &[u64], n_threads: usize) -> u64 {
    let items: Vec<Weighted<u32>> = weights
        .iter()
        .enumerate()
        .map(|(i, w)| Weighted::new(*w, i as u32))
        .collect();
    let buckets = lpt_partition(items, n_threads);
    let sums: Vec<u64> = buckets
        .iter()
        .map(|b| b.iter().map(|w| w.weight).sum())
        .collect();
    *sums.iter().max().unwrap()
}

#[test]
fn speedup_4_threads_8_chunks_balanced() {
    // 4 threads, 8 chunks, weights 100, 90, 80, 70,
    // 60, 50, 40, 30. Total = 520. Optimal: 130.
    // Naive: chunk i goes to thread (i % 4):
    //   t0 = 100+60 = 160
    //   t1 = 90+50 = 140
    //   t2 = 80+40 = 120
    //   t3 = 70+30 = 100
    //   max = 160.
    // LPT (after sort: 100,90,80,70,60,50,40,30):
    //   100 -> t0
    //   90  -> t1
    //   80  -> t2
    //   70  -> t3
    //   60  -> t3 (sums: 160, 90, 80, 70) -> t3
    //   50  -> t2 (sums: 160, 90, 80, 130) -> t2
    //   40  -> t1 (sums: 160, 90, 130, 130) -> t1
    //   30  -> t2 (sums: 160, 90, 160, 130) -> t1 (90)
    //   30  -> t1 (sums: 160, 120, 160, 130) -> t1
    //   max = 160.
    // Hmm same in this case. Let me try a different
    // distribution.
    let weights = [100u64, 90, 80, 70, 60, 50, 40, 30];
    let n = 4;
    let opt = optimal_makespan(&weights, n);
    let nv = naive_makespan(&weights, n);
    let lpt = lpt_makespan(&weights, n);
    eprintln!("balanced 8/4: optimal={} naive={} lpt={}", opt, nv, lpt);
    // LPT should always be <= naive (or equal in
    // benign cases).
    assert!(lpt <= nv, "LPT should be <= naive in all cases");
}

#[test]
fn speedup_8_threads_4_huge_4_tiny() {
    // 4 huge (50 MiB each) + 4 tiny (1 KB each).
    // Total: 200 MiB + 4 KB. Optimal lower bound: 25 MiB.
    // Achievable makespan: the largest chunk is 50 MiB
    // and chunks are indivisible, so the minimum
    // possible makespan is 50 MiB (one thread has the
    // huge chunk, the others have 0). LPT achieves this
    // trivially: the 4 huge chunks go to 4 different
    // threads, the 4 tiny chunks fill the other 4.
    let mut weights = vec![50 * 1024 * 1024u64; 4];
    weights.extend(std::iter::repeat(1024).take(4));
    let n = 8;
    let opt = optimal_makespan(&weights, n);
    let nv = naive_makespan(&weights, n);
    let lpt = lpt_makespan(&weights, n);
    eprintln!("4huge+4tiny/8: optimal_lb={} naive={} lpt={}", opt, nv, lpt);
    assert!(lpt <= nv, "LPT ({}) should be <= naive ({})", lpt, nv);
    // LPT is optimal here: 50 MiB per thread is the
    // best possible given indivisible chunks.
    assert_eq!(lpt, 50 * 1024 * 1024,
        "LPT should be optimal (50 MiB), got {}", lpt);
}

#[test]
fn speedup_4_threads_3_huge_30_small() {
    // Realistic case: 3 huge random chunks (40 MiB)
    // + 30 small text chunks (1 MiB each). 4 threads.
    // LPT puts each huge chunk on a different thread.
    // Then the 30 small chunks round-robin across the
    // 4 threads (all tied at 0 weight). Each thread
    // gets 7-8 small chunks (~7-8 MiB). The 3 threads
    // with huge chunks end up at ~47-48 MiB. The 4th
    // (no huge) ends up at ~7-8 MiB. Makespan: ~48 MiB.
    let mut weights = vec![40 * 1024 * 1024u64; 3];
    weights.extend(std::iter::repeat(1024 * 1024).take(30));
    let n = 4;
    let opt = optimal_makespan(&weights, n);
    let nv = naive_makespan(&weights, n);
    let lpt = lpt_makespan(&weights, n);
    let speedup = nv as f64 / lpt as f64;
    eprintln!("3huge+30small/4: optimal_lb={} naive={} lpt={} speedup={:.2}x",
        opt, nv, lpt, speedup);
    assert!(lpt <= nv, "LPT should be <= naive");
    // LPT should match the 3 huge chunks on 3 threads
    // (~48 MiB), not the lower bound (39 MiB) which
    // would require splitting a chunk.
    assert!(lpt >= 40 * 1024 * 1024 && lpt <= 50 * 1024 * 1024,
        "LPT ({}) should be in [40 MiB, 50 MiB]", lpt);
}

#[test]
fn speedup_4_threads_2_huge_2_medium_4_tiny() {
    // 2 huge (60 MiB) + 2 medium (10 MiB) + 4 tiny (500 KB)
    let mut weights = vec![60 * 1024 * 1024u64; 2];
    weights.extend(std::iter::repeat(10 * 1024 * 1024).take(2));
    weights.extend(std::iter::repeat(500 * 1024).take(4));
    let n = 4;
    let opt = optimal_makespan(&weights, n);
    let nv = naive_makespan(&weights, n);
    let lpt = lpt_makespan(&weights, n);
    let speedup = nv as f64 / lpt as f64;
    eprintln!("2huge+2med+4tiny/4: optimal={} naive={} lpt={} speedup={:.2}x",
        opt, nv, lpt, speedup);
    assert!(lpt <= nv, "LPT should be <= naive");
}

#[test]
fn speedup_pathological() {
    // Pathological case: 1 huge + 100 tiny on 4 threads.
    // LPT must put the huge chunk alone in one bucket.
    let mut weights = vec![1u64 << 30]; // 1 GiB
    weights.extend(std::iter::repeat(1024).take(100));
    let n = 4;
    let opt = optimal_makespan(&weights, n);
    let nv = naive_makespan(&weights, n);
    let lpt = lpt_makespan(&weights, n);
    eprintln!("1huge+100tiny/4: optimal={} naive={} lpt={}", opt, nv, lpt);
    assert!(lpt <= nv, "LPT should be <= naive");
    // Naive puts 25 small chunks on the huge bucket's
    // thread (round-robin: indices 0, 4, 8, 12, ..., 96
    // are on thread 0 — 25 of them). So naive is
    // 1 GiB + 25 KiB, which is ~1 GiB.
    // LPT keeps the huge bucket at exactly 1 GiB.
    // In practice, 25 KiB is negligible vs 1 GiB so
    // the speedup is ~1.0, but LPT is still
    // theoretically better.
    assert_eq!(lpt, 1u64 << 30, "LPT should keep huge chunk alone");
}

#[test]
fn aggregate_speedup_across_realistic_distributions() {
    // Average speedup over a mix of distributions
    // representing what a real corpus might produce.
    let distributions: Vec<Vec<u64>> = vec![
        // 50/50 text+random mix
        {
            let mut w = vec![10 * 1024 * 1024u64; 5]; // 5 huge random
            w.extend(std::iter::repeat(512 * 1024).take(20)); // 20 small text
            w
        },
        // Mostly small
        (0..50).map(|i| (i as u64 + 1) * 100_000).collect(),
        // Mostly huge
        (0..10).map(|i| (i as u64 + 1) * 5 * 1024 * 1024).collect(),
        // Even spread
        (0..30).map(|i| (i as u64 + 1) * 1024 * 1024).collect(),
        // Skewed (1 huge + many small)
        {
            let mut w = vec![500 * 1024 * 1024u64];
            w.extend(std::iter::repeat(64 * 1024).take(50));
            w
        },
    ];
    let n = 8;
    let mut total_naive: u64 = 0;
    let mut total_lpt: u64 = 0;
    for (i, w) in distributions.iter().enumerate() {
        let nv = naive_makespan(w, n);
        let lpt = lpt_makespan(w, n);
        eprintln!("distribution {}: naive={} lpt={} speedup={:.2}x",
            i, nv, lpt, nv as f64 / lpt as f64);
        total_naive += nv;
        total_lpt += lpt;
        assert!(lpt <= nv, "LPT must be <= naive in every case");
    }
    let agg_speedup = total_naive as f64 / total_lpt as f64;
    eprintln!("AGGREGATE speedup: {:.3}x", agg_speedup);
    // LPT should at least match naive on aggregate.
    assert!(agg_speedup >= 1.0,
        "aggregate LPT speedup {} should be >= 1.0", agg_speedup);
}
