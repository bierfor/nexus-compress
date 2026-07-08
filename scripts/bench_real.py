#!/usr/bin/env python3
"""
Real-world compression benchmark.

Compares NexusCompress against the actual installed CLI tools:
  - 7z (LZMA2, two presets: default and max)
  - zstd (default and ultra levels)
  - xz (default and max)
  - bzip2, gzip, lz4, brotli (all at their "max" levels)

For each (compressor, file) pair we measure:
  - size_in   : original bytes
  - size_out  : compressed bytes
  - ratio     : size_out / size_in (lower = better)
  - time_ms   : wall-clock time in milliseconds
  - mbps      : size_in / time_s / 1024 / 1024

Output: markdown table suitable for pasting into a blog post.
Optionally runs N repetitions and reports median.

Usage:
  python3 scripts/bench_real.py                 # default: 1 run
  python3 scripts/bench_real.py --runs 3        # median of 3
  python3 scripts/bench_real.py --quiet         # no per-compress chatter
"""

import argparse
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional, Tuple

# ---------------------------------------------------------------------------
# Corpus selection: representative of what users actually compress
# ---------------------------------------------------------------------------

DEFAULT_CORPUS: List[str] = [
    "corpus/code",                       # 105 KB, structured text (Pascal/Go-ish)
    "corpus/data.json",                  # 635 KB, JSON (highly redundant keys)
    "corpus/mixed.bin",                  #  95 KB, mixed (text + binary chunks)
    "corpus/random.bin",                 # 256 KB, incompressible (control)
    "corpus/repetitive.bin",             # 256 KB, near-zero entropy (control)
    "corpus/text.txt",                   # 178 KB, natural text
    "corpus_real/Globe3D.tsx",           #  13 KB, real-world TSX
    "corpus_real/Player.tsx",            #   8 KB, real-world TSX
    "corpus_real/index.d.ts",            # 875 KB, real-world TS declarations
    "corpus_real/lib.dom.d.ts",          #  1.8 MB, real-world TS declarations
]

# ---------------------------------------------------------------------------
# Compressor matrix
# ---------------------------------------------------------------------------
# Each entry: name, command builder. The harness calls build_cmd(in, out) to
# produce the CLI invocation. The harness owns timing: it runs the command
# and measures wall-clock from start to finish. Output size is the size of
# the file at `out` after the command completes.
# ---------------------------------------------------------------------------


@dataclass
class Compressor:
    name: str
    build_cmd: callable  # (in_path: Path, out_path: Path, work_dir: Path) -> List[str]
    out_extension: str   # suffix for the compressed file (used to find it)
    needs_cleanup: callable  # (work_dir: Path) -> None   - remove side files

    def __repr__(self) -> str:
        return self.name


# --- NexusCompress (our engine) ---
# The real CLI is `nexus` (src/main.rs), NOT `nexus-rar` (Tauri GUI app).
# Usage: `nexus c [OPTIONS] <input> <output>`
# Backends: v4 (default, multi-stream LZ77+rANS), v5 (LZMA-6),
#           v5-min (LZMA-6 + minify), v5-extreme (LZMA-9),
#           v6 (LZMA + swc AST minify for .js/.ts/.tsx/.jsx),
#           v6-extreme (v6 + LZMA-9)

def _nexus_cli_path() -> Optional[Path]:
    """Find the `nexus` CLI binary (workspace root bin, NOT nexus-rar)."""
    candidates = [
        Path("target/release/nexus"),
        Path("target/debug/nexus"),
    ]
    existing = [c.resolve() for c in candidates if c.exists()]
    if not existing:
        return None
    existing.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    return existing[0]


def _nexus_compress(binary: Path, backend: str) -> Compressor:
    """Wrap the `nexus c` CLI as a Compressor entry."""
    def build(in_p: Path, out_p: Path, work: Path) -> List[str]:
        cmd = [str(binary), "c", str(in_p), str(out_p)]
        if backend != "v4":
            cmd += ["--backend", backend]
        return cmd

    return Compressor(
        name=f"nexus/{backend}",
        build_cmd=build,
        out_extension=".nxs6",
        needs_cleanup=lambda w: None,
    )


# Sprint 5.7.2: encrypted + RS path.
# Hard-coded password so the bench is reproducible across
# runs. This is BENCH-ONLY — never use a real password
# here. The user is comparing the encrypted path's
# performance to the plain path, not testing the KDF
# timing (which is constant + ~100 ms per file).
BENCH_PASSWORD = "bench-only-not-a-real-password-do-not-use"


def _nexus_encrypted(binary: Path, recovery: str) -> Compressor:
    """Wrap the `nexus c --password ... [--recovery ...]` CLI."""
    def build(in_p: Path, out_p: Path, work: Path) -> List[str]:
        cmd = [
            str(binary), "c",
            "--password", BENCH_PASSWORD,
            str(in_p), str(out_p),
        ]
        if recovery != "default":
            cmd += ["--recovery", recovery]
        return cmd

    name = "nexus/v4+encrypted" if recovery == "off" else f"nexus/v4+encrypted+RS/{recovery}"
    return Compressor(
        name=name,
        build_cmd=build,
        out_extension=".nxe" if recovery == "off" else ".nxr",
        needs_cleanup=lambda w: None,
    )


# --- External CLI wrappers ---

def _7z(level: str, dict_mb: Optional[int] = None) -> Compressor:
    """7-Zip (LZMA2)."""
    if shutil.which("7z") is None:
        raise RuntimeError("7z not installed (brew install p7zip)")

    def build(in_p: Path, out_p: Path, work: Path) -> List[str]:
        # 7z writes <archive>.7z, so we use a temp name and move it.
        arch = work / f"arch_7z_{os.getpid()}"
        cmd = ["7z", "a", "-bb0", "-bd", "-y", f"-mx={level}", f"{arch}.7z", str(in_p)]
        if dict_mb is not None:
            cmd += [f"-md={dict_mb}m"]
        return cmd

    def cleanup(work: Path) -> None:
        for p in work.glob("arch_7z_*.7z"):
            p.unlink(missing_ok=True)

    label = f"7z/mx{level}"
    if dict_mb is not None:
        label += f"/md{dict_mb}M"
    return Compressor(name=label, build_cmd=build, out_extension=".7z", needs_cleanup=cleanup)


def _zstd(level: int) -> Compressor:
    """Zstd (zstandard)."""
    if shutil.which("zstd") is None:
        raise RuntimeError("zstd not installed (brew install zstd)")

    def build(in_p: Path, out_p: Path, work: Path) -> List[str]:
        return ["zstd", f"-{level}", "-q", "-f", str(in_p), "-o", str(out_p)]

    return Compressor(name=f"zstd/-{level}", build_cmd=build, out_extension=".zst", needs_cleanup=lambda w: None)


def _xz(level: int) -> Compressor:
    """XZ Utils (LZMA2 reference impl)."""
    if shutil.which("xz") is None:
        raise RuntimeError("xz not installed")

    def build(in_p: Path, out_p: Path, work: Path) -> List[str]:
        # xz -k keeps the original; -c writes to stdout. We need an output file.
        return ["xz", f"-{level}", "-k", "-c", str(in_p)]

    def cleanup(work: Path) -> None:
        # The harness captures stdout if we set capture_output. But xz is
        # simpler if we just have the harness redirect: handled below.
        pass

    return Compressor(name=f"xz/-{level}", build_cmd=build, out_extension=".xz", needs_cleanup=cleanup)


def _bzip2() -> Compressor:
    if shutil.which("bzip2") is None:
        raise RuntimeError("bzip2 not installed")
    return Compressor(
        name="bzip2/-9",
        build_cmd=lambda i, o, w: ["bzip2", "-9", "-c", str(i)],
        out_extension=".bz2",
        needs_cleanup=lambda w: None,
    )


def _gzip() -> Compressor:
    if shutil.which("gzip") is None:
        raise RuntimeError("gzip not installed")
    return Compressor(
        name="gzip/-9",
        build_cmd=lambda i, o, w: ["gzip", "-9", "-c", str(i)],
        out_extension=".gz",
        needs_cleanup=lambda w: None,
    )


def _lz4() -> Compressor:
    if shutil.which("lz4") is None:
        raise RuntimeError("lz4 not installed")
    return Compressor(
        name="lz4/-9",
        build_cmd=lambda i, o, w: ["lz4", "-9", "-f", str(i), str(o)],
        out_extension=".lz4",
        needs_cleanup=lambda w: None,
    )


def _brotli() -> Compressor:
    if shutil.which("brotli") is None:
        raise RuntimeError("brotli not installed (brew install brotli)")
    return Compressor(
        name="brotli/-q11",
        build_cmd=lambda i, o, w: ["brotli", "-q", "11", "-f", str(i), "-o", str(o)],
        out_extension=".br",
        needs_cleanup=lambda w: None,
    )


def build_compressor_matrix(nexus_binary: Optional[Path]) -> List[Compressor]:
    """Assemble the full list of compressors. Skip ones we don't have."""
    matrix: List[Compressor] = []
    if nexus_binary is not None:
        # 5 plain backends: v4 default, v5 (LZMA-6), v5-extreme (LZMA-9),
        # v6 (LZMA + swc AST minify), v6-extreme (LZMA-9 + swc).
        # v6 is our secret weapon on .js/.ts/.tsx source code.
        for backend in ["v4", "v5", "v5-extreme", "v6", "v6-extreme"]:
            matrix.append(_nexus_compress(nexus_binary, backend))
        # 3 encrypted variants (Sprint 5.7.2):
        #   v4 + AES-256-GCM (NXE, no parity)
        #   v4 + AES-256-GCM + RS-low (NXR, 10% parity — default)
        #   v4 + AES-256-GCM + RS-high (NXR, 25% parity)
        # All three are wired to v4 internally (the encrypted
        # path doesn't have a v5/v6 integration yet — that's
        # PR #4 in the roadmap). This is the fair comparison
        # to assess the overhead of encryption + recovery.
        matrix.append(_nexus_encrypted(nexus_binary, "off"))
        matrix.append(_nexus_encrypted(nexus_binary, "low"))
        matrix.append(_nexus_encrypted(nexus_binary, "high"))
    # try each external tool, skip on missing
    for factory in [_7z, _zstd, _xz, _bzip2, _gzip, _lz4, _brotli]:
        try:
            if factory is _7z:
                matrix.append(_7z(5))
                matrix.append(_7z(9))
            elif factory is _zstd:
                matrix.append(_zstd(3))
                matrix.append(_zstd(19))
            elif factory is _xz:
                matrix.append(_xz(6))
                matrix.append(_xz(9))
            else:
                matrix.append(factory())
        except RuntimeError as e:
            print(f"  [skip] {factory.__name__}: {e}", file=sys.stderr)
    return matrix


# ---------------------------------------------------------------------------
# Benchmark core
# ---------------------------------------------------------------------------


@dataclass
class Result:
    compressor: str
    file: str
    size_in: int
    size_out: int
    ratio: float
    time_ms: float
    mbps: float
    error: Optional[str] = None


def _run_one(c: Compressor, in_path: Path, out_path: Path, work: Path, capture_stdout_to: Optional[Path]) -> float:
    """Run a single compression, return wall-clock time in ms. Capture stdout to file if requested."""
    cmd = c.build_cmd(in_path, out_path, work)
    t0 = time.perf_counter()
    if capture_stdout_to is not None:
        with open(capture_stdout_to, "wb") as fout:
            r = subprocess.run(cmd, stdout=fout, stderr=subprocess.DEVNULL)
    else:
        r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    t1 = time.perf_counter()
    if r.returncode != 0:
        raise RuntimeError(f"{c.name} returned {r.returncode}")
    return (t1 - t0) * 1000.0


def bench_pair(c: Compressor, in_path: Path, runs: int, quiet: bool) -> Result:
    """Benchmark a single (compressor, file) pair. Returns median across runs."""
    size_in = in_path.stat().st_size

    # 7z writes to <work>/arch.7z, not to out_path. xz/bzip2/gzip write to stdout.
    # We unify: build the command for the specific case, capture to capture_file.
    out_path = in_path.with_suffix(c.out_extension)
    capture_file = in_path.parent / f".bench_{c.name.replace('/', '_')}_{in_path.name}.out"

    times: List[float] = []
    sizes: List[int] = []
    last_err: Optional[str] = None

    # work dir for side files (7z)
    work = in_path.parent

    for r in range(runs):
        try:
            # Decide whether we need a stdout capture (xz, bzip2, gzip)
            need_stdout = c.name.startswith(("xz", "bzip2", "gzip"))
            t = _run_one(c, in_path, out_path, work, capture_file if need_stdout else None)
            # The output file is either:
            #   - out_path (most compressors)
            #   - capture_file (xz/bzip2/gzip that write to stdout)
            #   - <work>/arch_7z_*.7z (7z)
            if c.name.startswith("7z"):
                # find the 7z file
                candidates = list(work.glob("arch_7z_*.7z"))
                if not candidates:
                    raise RuntimeError("7z did not produce an archive")
                sz = candidates[0].stat().st_size
                # rename for next iteration cleanliness
                if r == 0:
                    pass  # leave for cleanup
            elif need_stdout:
                if not capture_file.exists():
                    raise RuntimeError(f"{c.name} wrote nothing to stdout")
                sz = capture_file.stat().st_size
            else:
                if not out_path.exists():
                    raise RuntimeError(f"{c.name} did not produce {out_path}")
                sz = out_path.stat().st_size
            times.append(t)
            sizes.append(sz)
        except Exception as e:
            last_err = str(e)
        finally:
            c.needs_cleanup(work)
            # delete any side files
            for p in work.glob(f".bench_{c.name.replace('/', '_')}_*"):
                p.unlink(missing_ok=True)
            if out_path.exists() and not c.name.startswith("7z"):
                # keep for size check on first run, delete on rest
                if r == runs - 1:
                    pass
                out_path.unlink(missing_ok=True)

    if not times:
        return Result(
            compressor=c.name, file=in_path.name,
            size_in=size_in, size_out=0, ratio=0.0, time_ms=0.0, mbps=0.0,
            error=last_err or "no successful run",
        )

    time_med = statistics.median(times)
    size_med = int(statistics.median(sizes))
    ratio = size_med / size_in if size_in > 0 else 0.0
    mbps = (size_in / 1024 / 1024) / (time_med / 1000) if time_med > 0 else 0.0

    if not quiet:
        print(f"  {c.name:24s}  {in_path.name:24s}  in={size_in:>10}  out={size_med:>10}  ratio={ratio:.3f}  time={time_med:8.1f} ms  {mbps:6.2f} MB/s")
    return Result(c.name, in_path.name, size_in, size_med, ratio, time_med, mbps)


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def emit_markdown(results: List[Result], corpus: List[str], runs: int) -> str:
    files = sorted(set(r.file for r in results))
    compressors = sorted(set(r.compressor for r in results), key=lambda n: (
        0 if n.startswith("nexus-rar") else 1,  # ours first
        n,
    ))

    lines: List[str] = []
    lines.append("# NexusCompress real-world benchmark")
    lines.append("")
    lines.append(f"- Tool: `scripts/bench_real.py`")
    lines.append(f"- Runs: median of {runs} repetition(s) per (compressor, file)")
    lines.append(f"- Time: wall-clock, measured with `time.perf_counter()`")
    lines.append(f"- `ratio = size_out / size_in` (lower = better)")
    lines.append(f"- `MB/s = size_in / time_s / 1MiB`")
    lines.append("")

    # Per-file tables (one table per file)
    lines.append("## Per-file comparison")
    lines.append("")
    for f in files:
        rows = [r for r in results if r.file == f and r.error is None]
        if not rows:
            continue
        size_in = rows[0].size_in
        # sort by ratio ascending
        rows.sort(key=lambda r: (r.ratio, r.time_ms))
        # find winner(s)
        best_ratio = rows[0].ratio
        best_speed = min(rows, key=lambda r: r.time_ms).time_ms

        lines.append(f"### `{f}` — {size_in:,} bytes input")
        lines.append("")
        lines.append("| # | Compressor | size_out (B) | ratio | time (ms) | MB/s |")
        lines.append("|---|------------|-------------:|------:|----------:|-----:|")
        for i, r in enumerate(rows, 1):
            mark = ""
            if r.ratio == best_ratio:
                mark = " ← best ratio"
            if r.time_ms == best_speed:
                mark += " ← fastest"
            lines.append(f"| {i} | `{r.compressor}` | {r.size_out:,} | {r.ratio:.3f}{mark} | {r.time_ms:.1f} | {r.mbps:.2f} |")
        lines.append("")

    # Aggregate: total compressed size + avg speed per compressor
    lines.append("## Aggregate (across all files)")
    lines.append("")
    agg: dict = {}
    for r in results:
        if r.error:
            continue
        if r.compressor not in agg:
            agg[r.compressor] = {"size_out": 0, "size_in": 0, "time_ms": 0.0, "count": 0}
        agg[r.compressor]["size_out"] += r.size_out
        agg[r.compressor]["size_in"] += r.size_in
        agg[r.compressor]["time_ms"] += r.time_ms
        agg[r.compressor]["count"] += 1

    lines.append("| Compressor | total out (B) | total ratio | total MB/s |")
    lines.append("|------------|--------------:|------------:|-----------:|")
    for name, d in sorted(agg.items(), key=lambda kv: kv[1]["size_out"] / max(kv[1]["size_in"], 1)):
        ratio = d["size_out"] / max(d["size_in"], 1)
        mbps = (d["size_in"] / 1024 / 1024) / (d["time_ms"] / 1000) if d["time_ms"] > 0 else 0.0
        lines.append(f"| `{name}` | {d['size_out']:,} | {ratio:.3f} | {mbps:.2f} |")
    lines.append("")

    # Pareto frontier
    lines.append("## Pareto frontier (no other tool is both faster AND smaller)")
    lines.append("")
    pareto = []
    for name, d in agg.items():
        ratio = d["size_out"] / max(d["size_in"], 1)
        mbps = (d["size_in"] / 1024 / 1024) / (d["time_ms"] / 1000) if d["time_ms"] > 0 else 0.0
        pareto.append((name, ratio, mbps))
    pareto.sort(key=lambda x: (x[1], -x[2]))
    frontier = []
    best_speed = -1.0
    for name, ratio, mbps in pareto:
        if mbps >= best_speed:
            frontier.append((name, ratio, mbps))
            best_speed = mbps
    lines.append("| Compressor | ratio | MB/s |")
    lines.append("|------------|------:|-----:|")
    for name, ratio, mbps in frontier:
        lines.append(f"| `{name}` | {ratio:.3f} | {mbps:.2f} |")
    lines.append("")

    # Errors
    errs = [r for r in results if r.error]
    if errs:
        lines.append("## Errors")
        lines.append("")
        for r in errs:
            lines.append(f"- `{r.compressor}` on `{r.file}`: {r.error}")
        lines.append("")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--runs", type=int, default=1, help="repetitions per pair (median reported)")
    ap.add_argument("--corpus", nargs="*", default=DEFAULT_CORPUS, help="files to test")
    ap.add_argument("--nexus", type=str, default=None, help="path to nexus CLI binary (auto-detect)")
    ap.add_argument("--out", type=str, default="bench_results.md", help="markdown output file")
    ap.add_argument("--quiet", action="store_true", help="suppress per-pair output")
    args = ap.parse_args()

    repo = Path(__file__).resolve().parent.parent
    os.chdir(repo)

    if args.nexus:
        nexus = Path(args.nexus).resolve()
        if not nexus.exists():
            print(f"nexus CLI not found at {nexus}", file=sys.stderr)
            return 1
    else:
        nexus = _nexus_cli_path()
    if nexus is None:
        print("No nexus CLI found; bench will skip the engine comparison.", file=sys.stderr)
    else:
        print(f"Using nexus CLI: {nexus}  (built {time.strftime('%Y-%m-%d %H:%M', time.localtime(nexus.stat().st_mtime))})", file=sys.stderr)

    if not args.quiet:
        print(f"Corpus: {len(args.corpus)} files", file=sys.stderr)

    # Validate corpus
    for f in args.corpus:
        if not Path(f).exists():
            print(f"Missing corpus file: {f}", file=sys.stderr)
            return 1

    matrix = build_compressor_matrix(nexus)
    print(f"Compressors: {len(matrix)}", file=sys.stderr)
    for c in matrix:
        print(f"  - {c.name}", file=sys.stderr)

    results: List[Result] = []
    total_pairs = len(matrix) * len(args.corpus)
    done = 0
    for f in args.corpus:
        in_path = Path(f).resolve()
        for c in matrix:
            done += 1
            if not args.quiet:
                print(f"[{done}/{total_pairs}] {c.name} on {in_path.name} ...", file=sys.stderr)
            r = bench_pair(c, in_path, args.runs, args.quiet)
            results.append(r)

    md = emit_markdown(results, args.corpus, args.runs)
    out_path = Path(args.out)
    out_path.write_text(md, encoding="utf-8")
    print(f"\nWrote {out_path} ({len(md)} bytes)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
