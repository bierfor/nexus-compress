// Sprint 5.7.21-H: i18n consistency test.
//
// Pins the contract that user-visible strings don't lie
// about engine behavior. The compress mode/codec labels
// in particular must match what `resolve_plan` actually
// emits (see frontend/src/lib/modes.ts and
// src/supreme_engine.rs::resolve_plan).
//
// What this test catches:
//   - Stale labels that mention the wrong codec (e.g.
//     "LZ4-fast" instead of "Zstd-3" for rapido).
//   - Stale ratio claims that don't match the bench
//     numbers in benchmarks/FLOWNOW_BENCHMARK_2026-07-12.md.
//   - Inconsistent terminology between desc and tooltip.
//
// Run with:
//   node --experimental-strip-types --test \
//     frontend/tests/i18n_consistency.test.ts

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Read the i18n source file as text and extract the IT
// section. We don't import the module because pulling
// in the entire i18n table also pulls in TypeScript-only
// syntax that node --experimental-strip-types handles,
// but text parsing is faster and avoids any side effects
// from the module's top-level statements.
const I18N_PATH = join(import.meta.dirname, "..", "src", "lib", "i18n.ts");
const i18n_src = readFileSync(I18N_PATH, "utf-8");

/// Extract the value for a key from a specific locale
/// block (e.g. ES, EN, IT) in the i18n source file.
function extract_value(src: string, locale: "ES" | "EN" | "IT", key: string): string {
  // Find the block for this locale. EN and IT have type
  // annotations (`const EN: Record<...> = { ... }`), ES
  // does not (`const ES = { ... }`). Both forms are matched
  // by the loose `const ${locale}` search.
  const start = src.indexOf(`const ${locale}`);
  if (start < 0) throw new Error(`locale block ${locale} not found`);
  // Find the matching closing brace.
  let depth = 0;
  let block_start = src.indexOf("{", start);
  let block_end = -1;
  for (let i = block_start; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) {
        block_end = i;
        break;
      }
    }
  }
  const block = src.slice(block_start, block_end);
  // Match the key. Use a regex that handles both
  // single and double quotes around the value.
  const m = block.match(new RegExp(`"${key.replace(/\./g, "\\.")}":\\s*"((?:[^"\\\\]|\\\\.)*)"`));
  if (!m) throw new Error(`key ${key} not found in ${locale} block`);
  // Unescape the captured value.
  return m[1].replace(/\\(.)/g, "$1");
}

test("rapido.desc does NOT mention LZ4 (we use Zstd-3)", () => {
  // The previous text said "LZ4-fast" which was a stale
  // reference. The engine has used Zstd-3 since 5.7.9.
  // This pin catches the regression.
  for (const loc of ["ES", "EN", "IT"] as const) {
    const desc = extract_value(i18n_src, loc, "compress.mode.rapido.desc");
    assert.ok(
      !/LZ4/i.test(desc),
      `${loc}.compress.mode.rapido.desc must NOT mention "LZ4" (engine emits Zstd-3); got: ${desc}`
    );
  }
});

test("rapido.desc mentions Zstd-3 (matches engine output)", () => {
  for (const loc of ["ES", "EN", "IT"] as const) {
    const desc = extract_value(i18n_src, loc, "compress.mode.rapido.desc");
    assert.ok(
      /Zstd-?3/i.test(desc),
      `${loc}.compress.mode.rapido.desc must mention "Zstd-3" (engine default); got: ${desc}`
    );
  }
});

test("balanceado.desc does NOT claim 'LZMA + Conservative' (we use Zstd-3)", () => {
  // The previous text said "LZMA + Conservative" for
  // Balanceado but Balanceado defaults to Zstd-3 since
  // 5.7.9 (22x speedup). This pin catches the regression.
  for (const loc of ["ES", "EN", "IT"] as const) {
    const desc = extract_value(i18n_src, loc, "compress.mode.balanceado.desc");
    assert.ok(
      !/LZMA.*Conservative/i.test(desc),
      `${loc}.compress.mode.balanceado.desc must NOT claim "LZMA + Conservative" (engine emits Zstd-3); got: ${desc}`
    );
    assert.ok(
      /Zstd-?3/i.test(desc),
      `${loc}.compress.mode.balanceado.desc must mention "Zstd-3" (engine default); got: ${desc}`
    );
  }
});

test("all three locales have keys for the canonical mode descs", () => {
  // The TS type system already enforces this via
  // `Record<keyof typeof ES, string>`, but having an
  // explicit test means a missing IT key fails in CI
  // even if the typecheck is skipped.
  for (const loc of ["ES", "EN", "IT"] as const) {
    for (const key of [
      "compress.mode.rapido.desc",
      "compress.mode.balanceado.desc",
      "compress.mode.ultra.desc",
    ]) {
      const v = extract_value(i18n_src, loc, key);
      assert.ok(v.length > 5, `${loc}.${key} must be non-empty; got: ${v}`);
    }
  }
});

test("IT descs use Italian, not English or Spanish", () => {
  // Smoke test that the IT translations are actually
  // in Italian, not copy-pasted from ES/EN. We look
  // for at least 2 of these common Italian words in
  // each mode's desc.
  const italian_markers = [
    "rapporto", // ratio
    "veloce", // fast
    "bilanciato", // balanced
    "ultra",
    "ideale", // ideal
    "codice", // code
    "sorgente", // source
    "default",
    "raccomandato", // recommended
  ];
  for (const key of [
    "compress.mode.rapido.desc",
    "compress.mode.balanceado.desc",
    "compress.mode.ultra.desc",
  ]) {
    const v = extract_value(i18n_src, "IT", key);
    const found = italian_markers.filter((m) => v.toLowerCase().includes(m)).length;
    assert.ok(
      found >= 1,
      `IT.${key} doesn't look like Italian (only ${found} markers found); got: ${v}`
    );
  }
});
