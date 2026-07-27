// Sprint 5.7.21-B-Abstract: CompressView uses the
// abstract layout (no inline accordion, no PresetCard
// grid, no savings preview). The main flow is just
// drop zone + preset chip + Compress action.
//
// The previous version of CompressView had:
//   - A 3-column PresetCard grid
//   - A 5-section accordion (Speed/Quality/Corpus/Advanced/Security)
//   - A "savings preview" with stats
//   - A "save as custom" inline modal
// All of these are now extracted into SettingsDrawer
// and PresetPicker, or removed entirely.
//
// This test pins the structural invariants so that
// refactors don't accidentally re-introduce the
// visual noise.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const COMPRESS_VIEW = join(
  import.meta.dirname,
  "..",
  "src",
  "components",
  "CompressView.tsx",
);
const cv_src = readFileSync(COMPRESS_VIEW, "utf-8");

test("CompressView: imports the new abstract components", () => {
  // SettingsDrawer + PresetPicker are the new pieces.
  // PresetCard is GONE (replaced by PresetPicker).
  assert.ok(
    cv_src.includes('import { SettingsDrawer }'),
    "must import SettingsDrawer"
  );
  assert.ok(
    cv_src.includes('import { PresetPicker }'),
    "must import PresetPicker"
  );
  assert.ok(
    !cv_src.includes('import { PresetCard }'),
    "must NOT import PresetCard (replaced by PresetPicker)"
  );
});

test("CompressView: no inline accordion (Speed/Quality/Corpus/Advanced/Security)", () => {
  // The previous implementation had a `<div
  // className="mb-10 rounded-2xl border ...">` accordion
  // with 5 sections inline. That structure is gone —
  // everything lives in SettingsDrawer now.
  for (const key of [
    "compress.section.speed",
    "compress.section.quality",
    "compress.section.corpus",
    "compress.section.advanced",
    "compress.section.security",
  ]) {
    assert.ok(
      !cv_src.includes(key),
      `CompressView must not reference ${key} inline (it's in SettingsDrawer)`
    );
  }
});

test("CompressView: no savings preview card", () => {
  // The previous version rendered an "estimated savings
  // preview" with 3 stats (saving %, speed label, best
  // for). That was visual noise. It's gone.
  assert.ok(
    !cv_src.includes("compress.estimate.title"),
    "must NOT render the savings preview (compress.estimate.title)"
  );
  assert.ok(
    !cv_src.includes("compress.estimate.saving"),
    "must NOT render the savings percent"
  );
});

test("CompressView: no local Stat helper (it was savings-preview only)", () => {
  // The `Stat` helper was only used by the savings
  // preview. With the preview gone, Stat should be
  // removed.
  assert.ok(
    !cv_src.match(/^function Stat\(/m),
    "must not define a local Stat function (was for savings preview only)"
  );
});

test("CompressView: render PresetPicker + Configure button in main view", () => {
  // The abstract layout replaces the 3-col PresetCard
  // grid with a single PresetPicker chip + a "Configure"
  // button. The button opens the SettingsDrawer.
  assert.ok(
    cv_src.includes("<PresetPicker"),
    "must render <PresetPicker /> in the main view"
  );
  assert.ok(
    cv_src.includes('data-testid="configure-button"'),
    "must render the Configure button (opens the drawer)"
  );
  assert.ok(
    cv_src.includes("<SettingsDrawer"),
    "must render <SettingsDrawer /> at the root"
  );
});

test("CompressView: hero drop zone is more abstract (smaller, tighter)", () => {
  // The 5.7.21-B-Abstract iteration reduces the icon
  // size (80 → 36) and tightens the padding (py-20 →
  // py-12). Pin the classNames so a regression that
  // brings back the big icon is caught.
  assert.ok(
    cv_src.includes('Archive size={36}'),
    "Archive icon must be 36 (was 80)"
  );
  assert.ok(
    cv_src.includes("py-12"),
    "drop zone padding must be py-12 (was py-20)"
  );
  assert.ok(
    !cv_src.match(/Archive size=\{80\}/),
    "Archive icon must NOT be 80 (regression)"
  );
});
