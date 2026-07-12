# NexusCompress v0.3.0 — Release Notes

**Release date**: 2026-07-12
**Codename**: Best Mode
**Tag**: `v0.3.0`
**Previous**: v0.1.2 (Sprint 5.7.18)

## 🌟 Star feature: Best Mode (Auto) preset

The new "🎯 Best (Automatic)" preset runs a 100ms micro-benchmark
on a 5 MB sample of your corpus to pick the optimal (mode, codec)
combination. Like WinRAR's "Best" mode.

Click the preset, see the engine's recommendation:

  > "Estrategia recomendada: Zstd-3 Lossy (ratio 6.4x, 80 MB/s)"

Internally:
- 4 micro-benchmarks in parallel via rayon (Zstd-3 lossy, Zstd-3
  lossy+dict, LZMA-6 lossy, LZMA-9 lossy)
- Score = `ratio - 0.0005 × time_ms` (ratio dominates; 10x
  speedup ≈ 5% ratio loss)
- 11 oracle tests pin the matrix
- 3 bench tests in `tests/bench_best_mode.rs`

## 🚀 Performance wins (Sprint 5.7.21)

| Change | Speedup | Memory |
|---|---|---|
| **F** Parallel swc AST minify (5.7.21-F) | 3-4x preprocessor | — |
| **D** Parallel chunk-group decompression (5.7.21-D) | 2-4x decode | — |
| **E** Stream preprocessing in 200-file batches (5.7.21-E) | — | 20-30x peak reduction |

**FlowNow (5 GiB / 164 k files / 77% dev cache) benchmarks**:

| Compressor | Time | Output | Ratio | MB/s |
|---|---|---|---|---|
| **Nexus Rapido + Zstd-3** | 27 s | 1,556 MiB | **3.29x** | 184 |
| **Nexus LZMA-6** | 5m 22s | 1,193 MiB | **4.28x** | 15.4 |
| **Nexus LZMA-9 (Ultra)** | 19m 42s | 1,068 MiB | **4.66x** | 4.2 |

Best-in-class: LZMA-6 sweet spot for "I don't know what to pick".

## ✅ Correctness & tests

- **859/859 Rust tests verde** (was 331 in v0.1.0; +528 tests in 11 sprints)
- **11/11 TS unit tests verde** (modes mirror + i18n consistency)
- **Zero test debt**:
  - Fixed `v6solid_progress_test.rs` pre-existing compile error
  - Fixed `v6solid_big_dir_test.rs` missing fixture (marked `#[ignore]`)
  - Fixed bench tolerance flake (`+50ms` → `+15%` relative)
  - Fixed 8 fixtures with missing `skip_archive: false` (5.7.18 regression)
- **3 verified phantoms closed** (5.7.12): LZMA multi-stream mismatch,
  dict zstd -40, GUI mode toggle. Plus **1 real bug** fixed (5.7.13):
  Tauri IPC 30 GB bytes leak on 5 GB corpus.

## 🎨 UI / UX

- **A** (5.7.21-A): Mode chip badge now shows the actual codec
  emitted (rapido="zstd-3", balanceado="zstd-3", ultra="lzma-9")
  instead of the stale "v4" / "v6" labels.
- **H** (5.7.21-H): Stale rapido/balanceado descriptions fixed
  in ES/EN/IT. Drift detection test prevents future regressions.
- 6-section accordion (5.7.8) + 7 built-in presets (5.7.8) +
  Best Mode (5.7.19) + corpus breakdown warning (5.7.9)
  + tooltips on every mode/codec chip (5.7.19).

## 🔧 Backend consolidation (5.7.10-5.7.19)

- Single `SupremeEngine` entry point driven by `CompressionProfile`
  (replaced 4 dispatch surfaces + 7 skip-list duplicates).
- Zstd-on-passthrough (5.7.11): FlowNow 1.43x → 2.91x → 3.29x.
- Dict training ≥16 MiB (5.7.15): FlowNow 2.91x → 3.29x.
- Universal OS/IDE skip + `--skip-archive` flag (5.7.18).
- Multi-Solid Block split (5.7.6 hotfix #53): pure-LZMA / pure-Zstd
  islands, no codec mixing.

## 📦 Installation

### macOS (Apple Silicon + Intel)
- `NexusRAR-x.y.z-mac.zip` (DMG build is broken — see issue tracker)
- Drag `.app` to Applications

### Windows
- `NexusRAR-x.y.z-x64.msi` — built on Windows runners (CI)
- `NexusRAR-x.y.z-x64.exe` — alternative NSIS installer (CI)

### Linux
- `nexus-rar_x.y.z_amd64.deb` (Debian/Ubuntu) — built on Linux runners (CI)
- `nexus-rar-x.y.z.x86_64.rpm` (Fedora/RHEL) — TBD
- `AppImage` (universal) — TBD

> **CI status**: The .msi / .deb / .rpm builds require their native
> toolchains. The `tauri-apps/tauri-action@v0` GitHub Action
> matrix runs on macOS-latest, ubuntu-24.04, and windows-latest.
> See `.github/workflows/release.yml` (TODO: configure for v0.3.0).

## 🔐 License

Dual AGPL-3.0-or-later + Commercial (three tiers: Indie free,
Growth €4k/yr, Enterprise from €15k/yr). See `LICENSE` and
`COMMERCIAL_LICENSE.md`.

## 🐛 Known issues (post-release backlog)

- LZMA multi-stream encoder/decoder mismatch on archives > 128 MB
  total with 2+ super-chunks (closed in 5.7.12 with `#[ignore]`
  stress test, but no production fix yet).
- `train_from_buffer` error -40 in `--codec zstd` explicit path
  (5.7.12 closed for the LZMA default; Zstd explicit path
  unverified at scale).
- DMG bundle script is broken (skipped; use `.app` for now).

## 📊 MD5 seals

| Sprint | MD5 | Size |
|---|---|---|
| Pre-5.7.10 | `7ae50aefec8805018ab56503b311209b` | 15,160,784 B |
| Post-5.7.10 | `56eb5ee538e9887d40cb37e2ce69aa13` | 15,045,040 B |
| Post-5.7.11 | `e72fcd29f343c54aefcf2d0c5ae26010` | 15,061,584 B |
| Post-5.7.13 | `30293a7086fe6ac1997e3c221b6cbc47` | 15,061,584 B |
| Post-5.7.15 | `d54b8188a5f9ba18c9f8e3285bbd2ca7` | 15,061,584 B |
| Post-5.7.16 | `3c61e3c5fc3f745928bcd61b3cc812ef` | 15,061,584 B |
| Post-5.7.18 | `d297739a5b9cde9e0a5c0078b1717580` | 15,061,584 B |
| Post-5.7.19 | `0514707e04bf067e3cbbab746fa1f442` | 15,061,584 B |
| **Post-5.7.21 (v0.3.0)** | `cb16816512c7d1424f27303d17ac0e72` | 15,144,272 B |
