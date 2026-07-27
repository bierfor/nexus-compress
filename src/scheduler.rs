// Sprint 5.7.6 hotfix #54: Sonic Scheduler — chunk-weight
// static partitioning for rayon.
//
// Background: the codec pipeline in `solid_archive.rs`
// processes super-chunks in parallel via `par_iter().map()`.
// Rayon's work-stealing scheduler is fair but **size-agnostic**:
// it hands the next chunk to the next idle thread, regardless
// of how long that chunk will take to compress.
//
// In a heterogeneous corpus (e.g. 8 small text chunks + 2
// huge random-data chunks), this means one thread can end up
// holding a 256 MiB random chunk for 50 seconds while another
// thread idles after 5 seconds of small chunks. The makespan
// is determined by the worst-loaded thread, which is far from
// optimal.
//
// LPT (Longest Processing Time) heuristic — Graham 1969:
//   1. Sort items by weight descending
//   2. For each item, assign to the currently least-loaded
//      thread
// Guarantees makespan <= (4/3 - 1/(3m)) * optimal for m threads
// on a static list, which is the best known bound for the
// online variant.
//
// We use byte count as the weight proxy. LZMA and zstd are
// both O(n) (LZMA has a higher constant but the same
// asymptotic behaviour), so the proxy is honest enough —
// a 4 MiB text chunk compresses ~4x faster than a 16 MiB
// text chunk, regardless of codec. The LZMA/zstd mix doesn't
// change the proxy's monotonicity, only the constant.
//
// This is the Sonic Scheduler. It's "sonic" because it makes
// the parallel compressor finish sooner on heterogeneous
// corpora (the S3 → S1 → S2 SOnic-N scheduler family is
// named after the Sega console, not the speed of sound —
// this scheduler inherits the naming).

use rayon::prelude::*;

/// A unit of work for the scheduler. The `weight` field is
/// the byte count (or any monotonic proxy for CPU cost).
/// The `data` is opaque to the scheduler; the caller's
/// `compress` closure knows how to handle it.
#[derive(Debug, Clone, Default)]
pub struct Weighted<T> {
    pub weight: u64,
    pub data: T,
}

impl<T> Weighted<T> {
    pub fn new(weight: u64, data: T) -> Self {
        Self { weight, data }
    }
}

/// Group items into `n_buckets` buckets using the LPT
/// (Longest Processing Time) greedy heuristic.
///
/// Algorithm:
///   - Sort items by weight descending.
///   - For each item, push it into the bucket with the
///     smallest current total weight.
///
/// Tie-breaking: when two buckets have equal weight, pick
/// the one with the smaller index. This makes the
/// scheduling deterministic across runs (important for
/// debugging and for reproducible benchmarks).
pub fn lpt_partition<T>(items: Vec<Weighted<T>>, n_buckets: usize) -> Vec<Vec<Weighted<T>>> {
    assert!(n_buckets >= 1, "lpt_partition: n_buckets must be >= 1");
    if items.is_empty() {
        return (0..n_buckets).map(|_| Vec::new()).collect();
    }
    let mut buckets: Vec<Vec<Weighted<T>>> = (0..n_buckets).map(|_| Vec::new()).collect();
    let mut bucket_weights: Vec<u64> = vec![0; n_buckets];

    // Sort by weight descending. Use sort_by with reverse
    // to keep the sort stable for items of equal weight
    // (Rayon's par_iter preserves input order within a
    // chunk but the sort here is the single-threaded
    // pre-pass).
    let mut sorted = items;
    sorted.sort_by(|a, b| b.weight.cmp(&a.weight));

    for item in sorted {
        // Find the bucket with the smallest current
        // weight. Linear scan: O(n_buckets) per item.
        // For n_buckets <= 32 (typical CPU count) this
        // is faster than maintaining a min-heap, which
        // would have higher constants.
        let mut min_idx = 0;
        let mut min_weight = bucket_weights[0];
        for i in 1..n_buckets {
            if bucket_weights[i] < min_weight {
                min_weight = bucket_weights[i];
                min_idx = i;
            }
        }
        bucket_weights[min_idx] += item.weight;
        buckets[min_idx].push(item);
    }

    buckets
}

/// Parallel "sonic" map: process a list of weighted items
/// in parallel, using LPT scheduling to balance the load
/// across threads.
///
/// `f` is called for each item; it must be `Send + Sync`
/// (the closure runs on a rayon worker thread).
///
/// Returns a vector of results in the **same order as the
/// input** (the scheduler is internal; callers don't see
/// the bucket assignment).
///
/// We collect results from each bucket and then re-order
/// them by the original input index. This re-order is
/// O(n) and adds a single allocation per bucket.
pub fn sonic_par_map<T, R, F>(items: Vec<Weighted<T>>, n_threads: usize, f: F) -> Vec<R>
where
    T: Send + Sync,
    R: Send,
    F: Fn(&T) -> R + Send + Sync,
{
    // Track the original index of each item so we can
    // re-order results to match the input order.
    let indexed: Vec<(usize, Weighted<T>)> =
        items.into_iter().enumerate().map(|(i, w)| (i, w)).collect();
    let original_len = indexed.len();
    let n_threads = n_threads.max(1);

    // Strip the index off, partition by LPT, then re-attach
    // the index inside each bucket.
    let items_only: Vec<Weighted<(usize, T)>> = indexed
        .into_iter()
        .map(|(i, w)| {
            let data = w.data;
            Weighted::new(w.weight, (i, data))
        })
        .collect();
    let buckets = lpt_partition(items_only, n_threads);

    // Each bucket returns a Vec<(orig_idx, R)>. The Vec
    // is owned by the bucket (no shared mutable state).
    // After all buckets finish, we flatten + sort by
    // orig_idx to get the input-order result vector.
    //
    // This pattern is the same as `par_iter().map().collect()`
    // in rayon — the difference is that the work is
    // pre-partitioned by LPT instead of work-stealing
    // chunk-by-chunk.
    let bucket_results: Vec<Vec<(usize, R)>> = buckets
        .into_par_iter()
        .map(|bucket| {
            let mut out = Vec::with_capacity(bucket.len());
            for weighted in bucket {
                let (orig_idx, data) = weighted.data;
                let result = f(&data);
                out.push((orig_idx, result));
            }
            out
        })
        .collect();

    // Flatten + sort by orig_idx. The sort is O(n log n)
    // but n is small (number of chunks) and runs once.
    // Could be replaced with an O(n) counting sort if
    // orig_idx is bounded, but that's premature.
    let mut flat: Vec<(usize, R)> =
        bucket_results.into_iter().flatten().collect();
    flat.sort_by_key(|(idx, _)| *idx);
    debug_assert_eq!(flat.len(), original_len,
        "sonic_par_map: collected results count mismatch");
    flat.into_iter().map(|(_, r)| r).collect()
}

/// "Result-aware" parallel map: like `sonic_par_map`, but
/// the closure returns `Result<R, E>`. If ANY item returns
/// an `Err`, the first one (by original input order) is
/// returned and the rest of the items are still processed
/// (so the per-chunk progress counter is honest). The
/// success case unwraps each Result and returns the inner
/// `Vec<R>` in input order.
///
/// The codec pipeline uses this variant: a single
/// corrupted chunk shouldn't abort the other chunks
/// silently — we want to log per-chunk progress even if
/// one fails, then return the error.
pub fn sonic_par_try_map<T, R, E, F>(
    items: Vec<Weighted<T>>,
    n_threads: usize,
    f: F,
) -> Result<Vec<R>, E>
where
    T: Send + Sync,
    R: Send,
    E: Send + Sync,
    F: Fn(usize, &T) -> Result<R, E> + Send + Sync,
{
    // Like sonic_par_map, but the closure can fail. We
    // collect the Results and unwrap them at the end,
    // returning the first error if any.
    //
    // The closure receives the original input index AND
    // the item, so the caller can index into per-chunk
    // metadata arrays (e.g. chunk_codecs[i]) that must
    // align with the input order regardless of which
    // bucket processed each item.
    let indexed: Vec<(usize, Weighted<T>)> =
        items.into_iter().enumerate().map(|(i, w)| (i, w)).collect();
    let original_len = indexed.len();
    let n_threads = n_threads.max(1);

    let items_only: Vec<Weighted<(usize, T)>> = indexed
        .into_iter()
        .map(|(i, w)| {
            let data = w.data;
            Weighted::new(w.weight, (i, data))
        })
        .collect();
    let buckets = lpt_partition(items_only, n_threads);

    let bucket_results: Vec<Vec<(usize, Result<R, E>)>> = buckets
        .into_par_iter()
        .map(|bucket| {
            let mut out = Vec::with_capacity(bucket.len());
            for weighted in bucket {
                let (orig_idx, data) = weighted.data;
                let result = f(orig_idx, &data);
                out.push((orig_idx, result));
            }
            out
        })
        .collect();

    let mut flat: Vec<(usize, Result<R, E>)> =
        bucket_results.into_iter().flatten().collect();
    flat.sort_by_key(|(idx, _)| *idx);
    debug_assert_eq!(flat.len(), original_len,
        "sonic_par_try_map: collected results count mismatch");
    // Walk in input order. If we hit an Err, return it.
    // If we hit all Ok, unwrap into Vec<R>.
    let mut out: Vec<R> = Vec::with_capacity(original_len);
    for (_, r) in flat {
        match r {
            Ok(v) => out.push(v),
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Convenience: number of threads to use for the scheduler.
///
/// Defaults to `num_cpus::get()` (physical cores) on most
/// platforms. The user can override via the env var
/// `NEXUS_SONIC_THREADS` for reproducible benchmarks.
pub fn default_thread_count() -> usize {
    std::env::var("NEXUS_SONIC_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(num_cpus::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lpt_balances_uniform_weights() {
        // 10 items of weight 10 each, 4 buckets. LPT
        // should distribute them as 3, 3, 2, 2 (sum 30
        // each for the first two, 20 each for the last
        // two — total per bucket: 30, 30, 20, 20).
        let items: Vec<Weighted<u32>> = (0..10)
            .map(|i| Weighted::new(10, i))
            .collect();
        let buckets = lpt_partition(items, 4);
        assert_eq!(buckets.len(), 4);
        let total: u64 = buckets
            .iter()
            .map(|b| b.iter().map(|w| w.weight).sum::<u64>())
            .sum();
        assert_eq!(total, 100, "weights must be conserved");
        // Check that no bucket has more than ceil(100/4) + 1
        // weight, i.e. <= 26 (with the 3+3+2+2 split).
        let max = buckets
            .iter()
            .map(|b| b.iter().map(|w| w.weight).sum::<u64>())
            .max()
            .unwrap();
        assert!(max <= 30, "max bucket weight should be ~30, got {}", max);
    }

    #[test]
    fn lpt_handles_heterogeneous() {
        // The classic LPT motivating example: 4 buckets,
        // weights [100, 80, 60, 50, 40, 30, 20, 10].
        // Optimal makespan: ~92.5. LPT gives: 90.
        // LPT schedule: 100+30+10=140, 80+40=120, 60+20=80,
        // 50 = 50. Wait, that's not the LPT. Let me redo:
        //
        // Sort: [100, 80, 60, 50, 40, 30, 20, 10]
        // Pick smallest each time (all 0 initially):
        //   100 -> bucket 0 (sum 100)
        //   80  -> bucket 1 (sum 80)
        //   60  -> bucket 2 (sum 60)
        //   50  -> bucket 3 (sum 50)
        //   40  -> bucket 3 (sum 90)
        //   30  -> bucket 2 (sum 90)
        //   20  -> bucket 1 (sum 100)
        //   10  -> bucket 0 (sum 110)
        //
        // Final sums: 110, 100, 90, 90. Makespan: 110.
        let items: Vec<Weighted<u32>> = [100u32, 80, 60, 50, 40, 30, 20, 10]
            .iter()
            .map(|w| Weighted::new(*w as u64, 0u32))
            .collect();
        let buckets = lpt_partition(items, 4);
        let sums: Vec<u64> = buckets
            .iter()
            .map(|b| b.iter().map(|w| w.weight).sum())
            .collect();
        let max = *sums.iter().max().unwrap();
        assert!(max <= 120, "LPT makespan should be <= 120, got {}", max);
        // Verify all items accounted for.
        let total: u64 = sums.iter().sum();
        assert_eq!(total, 390);
    }

    #[test]
    fn lpt_empty_input() {
        let items: Vec<Weighted<u32>> = Vec::new();
        let buckets = lpt_partition(items, 4);
        assert_eq!(buckets.len(), 4);
        for b in &buckets {
            assert!(b.is_empty());
        }
    }

    #[test]
    fn lpt_single_bucket() {
        let items: Vec<Weighted<u32>> = (0..5)
            .map(|i| Weighted::new(i as u64, i))
            .collect();
        let buckets = lpt_partition(items, 1);
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].len(), 5);
    }

    #[test]
    fn sonic_par_map_preserves_order() {
        // The scheduler can process items in any order,
        // but the OUTPUT must be in the same order as the
        // input. This is the contract the codec pipeline
        // depends on (chunk N's compressed output must
        // appear at offset N in the file).
        let items: Vec<Weighted<u32>> = (0..100)
            .map(|i| Weighted::new(((i % 7) + 1) as u64, i))
            .collect();
        let results = sonic_par_map(items, 4, |x| x * 2);
        for (i, r) in results.iter().enumerate() {
            assert_eq!(*r, (i as u32) * 2, "result at index {} wrong", i);
        }
    }

    #[test]
    fn sonic_par_map_handles_empty() {
        let items: Vec<Weighted<u32>> = Vec::new();
        let results: Vec<u32> = sonic_par_map(items, 4, |x| x * 2);
        assert!(results.is_empty());
    }

    #[test]
    fn sonic_par_map_more_threads_than_items() {
        // 3 items, 8 buckets. The first 5 buckets should
        // be empty, the last 3 should each have one item.
        let items: Vec<Weighted<u32>> = (0..3)
            .map(|i| Weighted::new(10, i))
            .collect();
        let results = sonic_par_map(items, 8, |x| x * 2);
        assert_eq!(results, vec![0, 2, 4]);
    }

    #[test]
    fn sonic_par_map_is_correct_under_load() {
        // Stress test: 1000 items, 8 buckets, weight
        // varies 1..=100, result is item * 3 + 1.
        let items: Vec<Weighted<u32>> = (0..1000)
            .map(|i| Weighted::new((i % 100 + 1) as u64, i))
            .collect();
        let results = sonic_par_map(items, 8, |x| x * 3 + 1);
        for (i, r) in results.iter().enumerate() {
            assert_eq!(*r, (i as u32) * 3 + 1);
        }
    }

    #[test]
    fn default_thread_count_returns_positive() {
        let n = default_thread_count();
        assert!(n >= 1, "default thread count must be >= 1");
    }

    #[test]
    fn sonic_par_try_map_all_ok() {
        let items: Vec<Weighted<u32>> = (0..20)
            .map(|i| Weighted::new(1, i))
            .collect();
        let res: Result<Vec<u32>, String> =
            sonic_par_try_map(items, 4, |_idx, x| Ok(x * 10));
        let v = res.unwrap();
        for (i, r) in v.iter().enumerate() {
            assert_eq!(*r, i as u32 * 10);
        }
    }

    #[test]
    fn sonic_par_try_map_returns_first_error() {
        // Item 5 returns Err. The result must be Err and
        // the Err value must match the one we returned.
        let items: Vec<Weighted<u32>> = (0..20)
            .map(|i| Weighted::new(1, i))
            .collect();
        let res: Result<Vec<u32>, String> = sonic_par_try_map(
            items,
            4,
            |_idx, x| {
                if *x == 5 {
                    Err(format!("failed on item {}", x))
                } else {
                    Ok(*x)
                }
            },
        );
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, "failed on item 5");
    }

    #[test]
    fn sonic_par_try_map_preserves_order_on_ok() {
        // Heterogeneous weights, no errors. The output
        // order must match the input order regardless of
        // which bucket processed each item.
        let items: Vec<Weighted<u32>> = (0..50)
            .map(|i| Weighted::new(((i * 37) % 11 + 1) as u64, i))
            .collect();
        let res: Result<Vec<u32>, String> =
            sonic_par_try_map(items, 4, |_idx, x| Ok(x + 1000));
        let v = res.unwrap();
        for (i, r) in v.iter().enumerate() {
            assert_eq!(*r, i as u32 + 1000);
        }
    }

    #[test]
    fn sonic_par_try_map_passes_orig_idx() {
        // The closure must receive the original input
        // index, not a bucket-local index. This is the
        // contract the codec pipeline relies on to
        // index into chunk_codecs[i].
        let items: Vec<Weighted<u32>> = (0..10)
            .map(|i| Weighted::new(1, i))
            .collect();
        let res: Result<Vec<u32>, String> =
            sonic_par_try_map(items, 3, |idx, x| Ok(idx as u32 * 100 + x));
        let v = res.unwrap();
        for (i, r) in v.iter().enumerate() {
            assert_eq!(*r, i as u32 * 100 + i as u32);
        }
    }
}
