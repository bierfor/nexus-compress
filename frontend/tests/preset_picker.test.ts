// Sprint 5.7.21-B-Abstract: PresetPicker contract.
//
// The picker is a single chip in the main view that
// opens a popover with all built-in presets + custom
// profiles + save-as action. It must:
//   - Exist as a separate file
//   - Be visible as a chip with the active preset
//   - Open a popover with all 7 built-in presets
//   - Close on Esc, click-outside, and selecting a preset
//   - Use trilingual strings

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SRC_DIR = join(import.meta.dirname, "..", "src", "components");
const I18N_PATH = join(import.meta.dirname, "..", "src", "lib", "i18n.ts");

const picker_src = readFileSync(join(SRC_DIR, "PresetPicker.tsx"), "utf-8");
const i18n_src = readFileSync(I18N_PATH, "utf-8");

function has_key(key: string): boolean {
  const count = (i18n_src.match(new RegExp(`"${key}":`, "g")) ?? []).length;
  return count >= 3;
}

test("PresetPicker: file exists and exports the component", () => {
  assert.ok(picker_src.length > 500, "file should be >500B");
  assert.ok(
    picker_src.includes("export function PresetPicker"),
    "must export PresetPicker"
  );
  assert.ok(
    picker_src.includes("useLocale"),
    "must import useLocale for trilingual strings"
  );
});

test("PresetPicker: chip button has data-testid for tests", () => {
  // The chip in the main view is the only entry point
  // to change the preset (the PresetCard grid is gone).
  // It must be discoverable in tests and screen readers.
  assert.ok(
    picker_src.includes('data-testid="preset-picker-chip"'),
    "must have data-testid on the chip button"
  );
  assert.ok(
    picker_src.includes("aria-haspopup"),
    "must have aria-haspopup for screen reader users"
  );
});

test("PresetPicker: popover lists all 7 built-in presets", () => {
  // We don't enumerate the 7 ids literally here (they
  // live in profiles.ts BUILTIN_PRESETS) — we just
  // verify the popover renders BUILTIN_PRESETS.map().
  assert.ok(
    picker_src.includes("BUILTIN_PRESETS.map"),
    "popover must render BUILTIN_PRESETS.map()"
  );
  assert.ok(
    picker_src.includes('role="listbox"'),
    "popover must have role=listbox for ARIA"
  );
});

test("PresetPicker: closes on Esc and click-outside", () => {
  // The popover is the most common user interaction
  // for picking a preset. It must close on Esc and
  // click-outside (the standard popover pattern).
  assert.ok(
    picker_src.includes('addEventListener("keydown"'),
    "must register a keydown listener"
  );
  assert.ok(
    /"Escape"[\s\S]{0,80}setOpen\(false\)/.test(picker_src),
    "Escape key must call setOpen(false) in the handler"
  );
  assert.ok(
    picker_src.includes('addEventListener("mousedown"'),
    "must register a mousedown listener for click-outside"
  );
  assert.ok(
    picker_src.includes("contains"),
    "mousedown handler must check container.contains()"
  );
});

test("PresetPicker: i18n keys it uses exist in all 3 locales", () => {
  // Every user-visible string in the picker must come
  // from t(). We pin a few critical keys to make sure
  // they exist in ES, EN, IT.
  for (const key of [
    "compress.profile.title",
    "compress.profile.save",
    "compress.profile.save_as_placeholder",
    "compress.settings.preset.custom_section",
    "compress.settings.preset.delete",
    "compress.configure",
  ]) {
    assert.ok(has_key(key), `i18n key ${key} must be in all 3 locales`);
  }
});
