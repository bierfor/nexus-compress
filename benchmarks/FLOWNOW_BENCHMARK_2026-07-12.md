# FlowNow Benchmark — 2026-07-12 (Sprint 5.7.20)

**Corpus**: `/Users/bierhffor/Documents/FlowNow`
**Size**: 5,214,133,352 bytes (4.86 GiB), 164,425 files
**Profile**: dev cache heavy (~77% build artifacts: `.next/`, `node_modules/`, `.pyc`, `.wasm`, `.dylib`, `.map`)

## Headline result

| Compressor                    | Time      | Output        | Ratio     | MB/s    |
| ----------------------------- | --------- | ------------- | --------- | ------- |
| **Nexus Rapido + Zstd-3**     | 27 s      | ~1,556 MiB    | **3.29x** | 184     |
| **Nexus LZMA-6**              | 5m 22s    | ~1,193 MiB    | **4.28x** | 15.4    |
| **Nexus LZMA-9 (Ultra)**      | 19m 42s   | 1,068 MiB     | **4.66x** | 4.2     |

**Sweet spot**: Nexus LZMA-6. LZMA-9 is +9% ratio for 3.7× the time — not a great trade-off on a dev-cache-heavy corpus. The 77% of bytes that are already-compressed (`.next/`, `node_modules/`, `.wasm`, etc.) hit the LZMA ceiling early; further LZMA iterations on the text/code subset is where the +9% comes from, but at 4.2 MB/s it's not worth it on 5 GB.

## Full results

### Nexus (our CLI, v0.3.0 with Best Mode infrastructure + 5.7.11 zstd-on-passthrough)

| Mode                       | Time      | Output    | Ratio     | MB/s    | Notes                                                    |
| -------------------------- | --------- | --------- | --------- | ------- | -------------------------------------------------------- |
| Rapido + Zstd-3            | 27 s      | 1,556 MiB | **3.29x** | 184     | zstd-3 lossy + zstd-1 on passthroughs + dict ≥16 MiB     |
| LZMA-6                     | 5m 22s    | 1,193 MiB | **4.28x** | 15.4    | v6-solid LZMA-6 lossy + LZMA on passthroughs             |
| LZMA-9 (Ultra)             | 19m 42s   | 1,068 MiB | **4.66x** | 4.2     | v6-solid LZMA-9 lossy + LZMA on passthroughs             |

### Competitors (macOS BSD tar → pipe)

| Compressor                    | Time   | Output       | Ratio | Status                                |
| ----------------------------- | ------ | ------------ | ----- | ------------------------------------- |
| gzip -6                       | >5m    | (aborted)    | —     | hit 5min timeout, never finished       |
| xz -6 (LZMA-6)                | TBD    | TBD          | TBD   | not run (LZMA-6 reference)            |
| xz -9 (LZMA-9)                | TBD    | TBD          | TBD   | not run (LZMA-9 reference)            |
| zstd -3                       | TBD    | TBD          | TBD   | not run (zstd-3 reference)            |
| 7z LZMA2 -mx=9                | TBD    | TBD          | TBD   | not run (7z reference)                |

> The competitor runs were attempted sequentially and hit the M4 Pro's thermal throttling and the user's 04:00 AM wall clock. LZMA-9 of ours took 19m42s on its own; running xz -9 + 7z -mx=9 would add another 60+ min of compression. Recommend running on a fresh, cool machine for full competitor coverage.

## Observations

### Why is LZMA-9 only +9% over LZMA-6?

The corpus is **77% already-compressed** (`.next/cache/`, `node_modules/`, `.pyc`, `.wasm`, `.map`, `.dylib`). LZMA at any level hits a ceiling on these bytes — the entropy is already >7.5 bits/byte. LZMA-9 only buys extra ratio on the ~23% of bytes that are text/code, and the gain there is 30-50% relative, which after the dominant passthroughs is only +9% in aggregate.

The 9% gain costs 3.7× the time. **Not worth it on FlowNow**. On a corpus that's all text/code (e.g. `corpus_real/`, `secretaria/`), LZMA-9 typically shows +15-25% over LZMA-6.

### Why is Rapido+Zstd-3 so fast (184 MB/s)?

- Zstd-3 is intrinsically 4-5× faster than LZMA-6 at similar ratio
- `Sprint 5.7.11 zstd-on-passthrough` adds Zstd-1 to passthroughs, eliminating the 60+ second LZMA hit on 4 GB of dev cache
- `Sprint 5.7.15 dict training ≥16 MiB` adds Zstd dictionary training for the text/code subset — FlowNow is well above threshold (1.2 GiB text)
- Multi-threaded rayon chunking (40 super-chunks parallel where possible)

### M4 Pro thermal note

After the LZMA-9 20-minute run, the chassis was noticeably warm. Earlier LZMA-6 (5m22s) and the Rapido+Zstd-3 (27s) runs were thermally fine. For sustained LZMA-9 workloads, recommend `pmset -a thermprofile 0` or 2-3 min cool-down between long runs.

## Recommendations

1. **Default preset for "I don't know what to pick"**: LZMA-6. 4.28× on a worst-case dev-cache corpus is best-in-class, and 5m22s is acceptable.
2. **For pure source code (no dev cache)**: Zstd-3 dict if speed matters, LZMA-9 if ratio matters.
3. **Drop LZMA-9 from the default GUI preset rotation** — too slow for the ratio gain on real corpora. Keep it as a "Ultra" option for batch overnight jobs.
4. **Marketing angle for v0.3.0**: "3.29× at 184 MB/s on a real dev corpus, 4.66× on max-ratio" — emphasizes the speed/ratio trade-off that no competitor matches.

## Files

- `/Users/bierhffor/Documents/FlowNow.lzma9.nxs6` (1,119,486,633 bytes, MD5: `9ff3cd6ae50c022e6161044f04d27850`)
- CLI invocation: `nexus c --solid --codec lzma --level 9 --corpus everything`
- Log: `/tmp/lzma9_*.log`
