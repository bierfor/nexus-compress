// Sprint 5.7.21-G: TypeScript unit test pinning the
// `MODES.defaultCodec` mirror contract.
//
// Why this test exists:
//
//   - The backend's `resolve_plan` (src/supreme_engine.rs)
//     has a table:
//
//       (Auto, Rapido)     → SolidLevel::Zstd(3)
//       (Auto, Balanceado) → SolidLevel::Zstd(3)
//       (Auto, Ultra)      → SolidLevel::Lzma(9)
//
//   - The frontend displays a chip badge showing
//     `MODES[i].defaultCodec`. The user reads this
//     badge to know which codec the engine will pick.
//
//   - If a backend or frontend change updates one side
//     but not the other, the user sees a misleading badge.
//     This test catches that drift.
//
// Run with:
//   node --experimental-strip-types --test \
//     frontend/tests/modes_mirror.test.ts
//
// (Node 24 has stable --experimental-strip-types; no
//  transpiler, no vitest, no npm install required.)

import { test } from "node:test";
import assert from "node:assert/strict";
import { MODES, getMode, type Mode } from "../src/lib/modes.ts";

/// Backend `resolve_plan` table for `codec === "auto"`.
/// If the Rust side changes these values, this table
/// MUST be updated to match (and the chip badge in
/// `MODES` must be updated too).
const EXPECTED_DEFAULT_CODEC: Record<Mode, string> = {
  rapido: "zstd-3",
  balanceado: "zstd-3",
  ultra: "lzma-9",
};

test("MODES has exactly the three canonical modes", () => {
  assert.equal(MODES.length, 3, "MODES must have exactly 3 entries");
  const ids = MODES.map((m) => m.id).sort();
  assert.deepEqual(
    ids,
    ["balanceado", "rapido", "ultra"],
    "MODES ids must be the canonical set"
  );
});

test("rapido.defaultCodec mirrors resolve_plan(codec=Auto, mode=Rapido)", () => {
  const m = getMode("rapido");
  assert.ok(m, "mode rapido must be present in MODES");
  assert.equal(
    m!.defaultCodec,
    EXPECTED_DEFAULT_CODEC.rapido,
    `frontend chip for rapido must say "${EXPECTED_DEFAULT_CODEC.rapido}"`
  );
});

test("balanceado.defaultCodec mirrors resolve_plan(codec=Auto, mode=Balanceado)", () => {
  const m = getMode("balanceado");
  assert.ok(m, "mode balanceado must be present in MODES");
  assert.equal(
    m!.defaultCodec,
    EXPECTED_DEFAULT_CODEC.balanceado,
    `frontend chip for balanceado must say "${EXPECTED_DEFAULT_CODEC.balanceado}"`
  );
});

test("ultra.defaultCodec mirrors resolve_plan(codec=Auto, mode=Ultra)", () => {
  const m = getMode("ultra");
  assert.ok(m, "mode ultra must be present in MODES");
  assert.equal(
    m!.defaultCodec,
    EXPECTED_DEFAULT_CODEC.ultra,
    `frontend chip for ultra must say "${EXPECTED_DEFAULT_CODEC.ultra}"`
  );
});

test("each mode has a non-empty icon and stars (UI sanity)", () => {
  for (const m of MODES) {
    assert.ok(m.icon.length > 0, `${m.id} has empty icon`);
    assert.ok(m.stars >= 1 && m.stars <= 5, `${m.id} stars out of range: ${m.stars}`);
  }
});

test("defaultCodec strings use the canonical N-K format (e.g. zstd-3, lzma-9)", () => {
  // Pin the display format. If someone changes
  // "zstd-3" to "zstd (level 3)" the chip text changes
  // and downstream formatters that grep for the level
  // digit break.
  for (const m of MODES) {
    assert.match(
      m.defaultCodec,
      /^(zstd|lzma)-\d+$/,
      `${m.id}.defaultCodec must match {zstd|lzma}-{N}, got "${m.defaultCodec}"`
    );
  }
});
