#!/usr/bin/env bash
# Sprint 5.7.14: competitor benchmark.
#
# Compresses the same canonical corpus (`corpus_real/`) with
# every general-purpose compressor available on the M4 Pro
# (gzip, bzip2, xz, zstd, 7z) plus our CLI at several
# profile combinations. Measures wall time, output size,
# ratio, and throughput.
#
# Run from the repo root:
#   bash scripts/bench_competitors.sh
#
# Output: a markdown table on stdout, ready to paste into
# a Sprint changelog or a sales pitch.

set -e

CORPUS="${1:-corpus_real}"
OUT_DIR=$(mktemp -d -t nexus-bench-XXXXXX)
trap "rm -rf '$OUT_DIR'" EXIT

# Pre-compute corpus stats.
# macOS `du` doesn't support `-b`; we sum file sizes with `find`+`stat` instead.
CORPUS_BYTES=$(find "$CORPUS" -type f -exec stat -f %z {} \; | awk '{s+=$1} END {print s}')
CORPUS_FILES=$(find "$CORPUS" -type f | wc -l | tr -d ' ')
CORPUS_MIB=$(echo "scale=2; $CORPUS_BYTES / 1048576" | bc)

echo "corpus: $CORPUS"
echo "size:   ${CORPUS_MIB} MiB (${CORPUS_BYTES} B), ${CORPUS_FILES} files"
echo "tmpdir: $OUT_DIR"
echo ""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

run() {
    # $1 = label, $2 = command...
    local label="$1"
    shift
    local out="$OUT_DIR/$label.bin"
    local start_ns end_ns elapsed_ns
    start_ns=$(date +%s%N)
    if "$@" >"$out" 2>"$OUT_DIR/$label.err"; then
        end_ns=$(date +%s%N)
        elapsed_ns=$((end_ns - start_ns))
        local out_bytes
        out_bytes=$(wc -c <"$out" | tr -d ' ')
        echo "$label $out_bytes $elapsed_ns"
    else
        echo "$label ERR"
    fi
}

# ---------------------------------------------------------------------------
# Competitors
# ---------------------------------------------------------------------------

declare -a RESULTS=()

# gzip
GZ6_OUT="$OUT_DIR/gz6.tar"
GZ6_T_NS=$(date +%s%N)
tar -C "$(dirname "$CORPUS")" -cf - "$(basename "$CORPUS")" | gzip -6 >"$GZ6_OUT"
GZ6_T=$(($(date +%s%N) - GZ6_T_NS))
GZ6_B=$(wc -c <"$GZ6_OUT" | tr -d ' ')
RESULTS+=("gzip -6 (default)|$GZ6_B|$GZ6_T")

GZ9_OUT="$OUT_DIR/gz9.tar"
GZ9_T_NS=$(date +%s%N)
tar -C "$(dirname "$CORPUS")" -cf - "$(basename "$CORPUS")" | gzip -9 >"$GZ9_OUT"
GZ9_T=$(($(date +%s%N) - GZ9_T_NS))
GZ9_B=$(wc -c <"$GZ9_OUT" | tr -d ' ')
RESULTS+=("gzip -9 (max)|$GZ9_B|$GZ9_T")

# bzip2
BZ_OUT="$OUT_DIR/bz.tar"
BZ_T_NS=$(date +%s%N)
tar -C "$(dirname "$CORPUS")" -cf - "$(basename "$CORPUS")" | bzip2 -9 >"$BZ_OUT"
BZ_T=$(($(date +%s%N) - BZ_T_NS))
BZ_B=$(wc -c <"$BZ_OUT" | tr -d ' ')
RESULTS+=("bzip2 -9|$BZ_B|$BZ_T")

# xz
XZ6_OUT="$OUT_DIR/xz6.tar"
XZ6_T_NS=$(date +%s%N)
tar -C "$(dirname "$CORPUS")" -cf - "$(basename "$CORPUS")" | xz -6 >"$XZ6_OUT"
XZ6_T=$(($(date +%s%N) - XZ6_T_NS))
XZ6_B=$(wc -c <"$XZ6_OUT" | tr -d ' ')
RESULTS+=("xz -6 (LZMA-6)|$XZ6_B|$XZ6_T")

XZ9_OUT="$OUT_DIR/xz9.tar"
XZ9_T_NS=$(date +%s%N)
tar -C "$(dirname "$CORPUS")" -cf - "$(basename "$CORPUS")" | xz -9 >"$XZ9_OUT"
XZ9_T=$(($(date +%s%N) - XZ9_T_NS))
XZ9_B=$(wc -c <"$XZ9_OUT" | tr -d ' ')
RESULTS+=("xz -9 (LZMA-9)|$XZ9_B|$XZ9_T")

# zstd (macOS BSD tar doesn't have --zstd; pipe to zstd CLI)
ZS3_OUT="$OUT_DIR/zs3.tar.zst"
ZS3_T_NS=$(date +%s%N)
tar -C "$(dirname "$CORPUS")" -cf - "$(basename "$CORPUS")" | zstd -3 -q -o "$ZS3_OUT"
ZS3_T=$(($(date +%s%N) - ZS3_T_NS))
ZS3_B=$(wc -c <"$ZS3_OUT" | tr -d ' ')
RESULTS+=("zstd -3 (default)|$ZS3_B|$ZS3_T")

ZS19_OUT="$OUT_DIR/zs19.tar.zst"
ZS19_T_NS=$(date +%s%N)
tar -C "$(dirname "$CORPUS")" -cf - "$(basename "$CORPUS")" | zstd -19 -q -o "$ZS19_OUT"
ZS19_T=$(($(date +%s%N) - ZS19_T_NS))
ZS19_B=$(wc -c <"$ZS19_OUT" | tr -d ' ')
RESULTS+=("zstd -19 (max)|$ZS19_B|$ZS19_T")

# 7z with LZMA2 (the typical 7z default)
SEVENZ_OUT="$OUT_DIR/7z.7z"
SEVENZ_T_NS=$(date +%s%N)
7z a -mx=9 -mmt=on "$SEVENZ_OUT" "$CORPUS" >/dev/null
SEVENZ_T=$(($(date +%s%N) - SEVENZ_T_NS))
SEVENZ_B=$(wc -c <"$SEVENZ_OUT" | tr -d ' ')
RESULTS+=("7z LZMA2 -mx=9|$SEVENZ_B|$SEVENZ_T")

# ---------------------------------------------------------------------------
# Our CLI
# ---------------------------------------------------------------------------

NEXUS_BIN="$(dirname "$0")/../target/release/nexus"
if [ ! -x "$NEXUS_BIN" ]; then
    NEXUS_BIN="$(dirname "$0")/../target/debug/nexus"
fi

run_nexus() {
    # $1 = label, $2 = arguments...
    local label="$1"
    shift
    local out="$OUT_DIR/nexus_${label// /_}.nxs6"
    local start_ns end_ns
    start_ns=$(date +%s%N)
    if "$NEXUS_BIN" c "$@" "$CORPUS" "$out" >"$OUT_DIR/nexus_${label// /_}.log" 2>&1; then
        end_ns=$(date +%s%N)
        local out_bytes
        out_bytes=$(wc -c <"$out" | tr -d ' ')
        local elapsed_ns=$((end_ns - start_ns))
        RESULTS+=("nexus $label|$out_bytes|$elapsed_ns")
    else
        RESULTS+=("nexus $label|ERR|ERR")
    fi
}

if [ -x "$NEXUS_BIN" ]; then
    run_nexus "lossless zstd-3" --solid --lossless --codec zstd --level 3
    run_nexus "lossless zstd-9" --solid --lossless --codec zstd --level 9
    run_nexus "lossless lzma-6" --solid --lossless --codec lzma --level 6
    run_nexus "lossless lzma-9" --solid --lossless --codec lzma --level 9
    run_nexus "lossy zstd-3" --solid --codec zstd --level 3
    run_nexus "lossy lzma-6" --solid --codec lzma --level 6
    run_nexus "lossy lzma-9 (ultra)" --solid --codec lzma --level 9
fi

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------

echo ""
echo "## Benchmark results"
echo ""
echo "Corpus: \`$CORPUS\` (${CORPUS_MIB} MiB, ${CORPUS_FILES} files)"
echo ""
printf "| %-32s | %12s | %8s | %10s | %10s |\n" "Compressor" "Size (B)" "Ratio" "Time (s)" "MB/s"
printf "| %-32s | %12s | %8s | %10s | %10s |\n" "---" "---" "---" "---" "---"
for r in "${RESULTS[@]}"; do
    IFS='|' read -r label bytes ns <<<"$r"
    if [ "$bytes" = "ERR" ]; then
        printf "| %-32s | %12s | %8s | %10s | %10s |\n" "$label" "FAIL" "-" "-" "-"
    else
        # ns is in nanoseconds; convert to seconds with 2 decimals.
        sec=$(echo "scale=3; $ns / 1000000000" | bc)
        # Ratio = input / output.
        ratio=$(echo "scale=2; $CORPUS_BYTES / $bytes" | bc)
        # MB/s = (input MiB) / (seconds).
        mbps=$(echo "scale=2; $CORPUS_MIB / $sec" | bc)
        printf "| %-32s | %12d | %6sx | %9ss | %8s |\n" \
            "$label" "$bytes" "$ratio" "$sec" "$mbps"
    fi
done

# ---------------------------------------------------------------------------
# Sanity: roundtrip one of our archives
# ---------------------------------------------------------------------------

if [ -x "$NEXUS_BIN" ] && [ -f "$OUT_DIR/nexus_lossless_lzma-9.nxs6" ]; then
    echo ""
    echo "## Roundtrip check (lossless lzma-9)"
    DEC_OUT="$OUT_DIR/decoded"
    mkdir -p "$DEC_OUT"
    "$NEXUS_BIN" d "$OUT_DIR/nexus_lossless_lzma-9.nxs6" "$DEC_OUT" \
        >"$OUT_DIR/decode.log" 2>&1 || true
    DEC_FILES=$(find "$DEC_OUT" -type f | wc -l | tr -d ' ')
    echo "decompressed files: $DEC_FILES (input had $CORPUS_FILES)"
fi
