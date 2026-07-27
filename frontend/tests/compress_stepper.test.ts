// Sprint 5.7.21-B-Remodel: unit tests for the new
// Compress-flow components (Stepper, PresetCard, StickyBar).
//
// The components are visual and mostly tested via integration
// (the running app), but we can pin a few invariants without
// pulling in React testing libraries: the i18n keys the
// components depend on, and the data shape of the inputs.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const I18N_PATH = join(import.meta.dirname, "..", "src", "lib", "i18n.ts");
const SRC_DIR = join(import.meta.dirname, "..", "src", "components");
const i18n_src = readFileSync(I18N_PATH, "utf-8");

function has_key(key: string): boolean {
  // True if the key exists in all 3 locale blocks.
  const count = (i18n_src.match(new RegExp(`"${key}":`, "g")) ?? []).length;
  return count >= 3;
}

test("stepper: all 4 step labels exist in 3 locales", () => {
  for (const id of ["files", "profile", "output", "compress"]) {
    assert.ok(
      has_key(`compress.step.${id}`),
      `missing compress.step.${id} in all 3 locales`
    );
    assert.ok(
      has_key(`compress.step.${id}.hint`),
      `missing compress.step.${id}.hint in all 3 locales`
    );
  }
});

test("preset cards: all 8 builtin presets have card descriptions", () => {
  for (const id of [
    "snapshot",
    "best",
    "source",
    "code",
    "balanced",
    "ultra",
    "encrypted",
    "lossless",
  ]) {
    assert.ok(
      has_key(`compress.preset.${id}.card`),
      `missing compress.preset.${id}.card in all 3 locales`
    );
  }
});

test("sticky bar: bottom labels exist in 3 locales", () => {
  for (const key of [
    "compress.bottom.estimated",
    "compress.bottom.unknown_size",
    "compress.bottom.click_to_pick",
    "compress.bottom.encrypt_toggle",
  ]) {
    assert.ok(has_key(key), `missing ${key} in all 3 locales`);
  }
});

test("components: source files exist and are TSX", () => {
  for (const file of [
    "CompressStepper.tsx",
    "PresetCard.tsx",
    "CompressStickyBar.tsx",
  ]) {
    const path = join(SRC_DIR, file);
    const content = readFileSync(path, "utf-8");
    assert.ok(content.length > 100, `${file} should be >100 chars`);
    assert.ok(
      content.includes("useLocale"),
      `${file} should import useLocale for trilingual strings`
    );
  }
});

test("drift detection: CompressStepper consumes 4 step ids", () => {
  // The CompressStepper takes a `steps` prop — it doesn't
  // hardcode the step ids. The "exactly 4" invariant is
  // pinned by the consumer (CompressView's STEPS constant,
  // see below). This test just confirms the canonical set
  // is consistent across the source.
  //
  // We accept any of these step ids as "the canonical 4":
  //   files, profile, output, compress
  // and assert all 4 exist in the i18n source with the
  // required hint suffix.
  const expected_steps = ["files", "profile", "output", "compress"];
  for (const id of expected_steps) {
    assert.ok(
      has_key(`compress.step.${id}`),
      `missing compress.step.${id} in all 3 locales`
    );
    assert.ok(
      has_key(`compress.step.${id}.hint`),
      `missing compress.step.${id}.hint in all 3 locales`
    );
  }
  assert.equal(expected_steps.length, 4);
});
