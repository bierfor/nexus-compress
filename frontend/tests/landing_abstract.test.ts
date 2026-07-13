// Sprint 5.7.21-B-Abstract: LandingPage contract.
//
// The home view is now abstract — just greeting + headline + 3
// action cards + recent activity + shortcuts. The previous
// version had 5 feature pills, a drag-drop CTA card, a gradient
// headline, a rotating "tip of the day", and 5 colorful stat
// cards. All gone.
//
// This test pins the structural invariants so that refactors
// don't accidentally re-introduce the visual noise.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SRC = join(
  import.meta.dirname,
  "..",
  "src",
  "components",
  "LandingPage.tsx",
);
const I18N = join(import.meta.dirname, "..", "src", "lib", "i18n.ts");
const src = readFileSync(SRC, "utf-8");
const i18n = readFileSync(I18N, "utf-8");

function has_key(key: string): boolean {
  return (i18n.match(new RegExp(`"${key}":`, "g")) ?? []).length >= 3;
}

test("LandingPage: file is small (abstract)", () => {
  // The previous version was 674 lines. The abstract version
  // should be under 400 lines. We pin a generous ceiling of
  // 400 to allow future additions.
  const lineCount = src.split("\n").length;
  assert.ok(
    lineCount < 400,
    `LandingPage should be <400 lines (was 674); got ${lineCount}`
  );
});

test("LandingPage: no 5 feature pills", () => {
  // The previous version had 5 feature pills (P2P, Compress,
  // Encrypt, No counts, No track) in a row at the top. Those
  // are gone — the home view is a clean landing.
  assert.ok(
    !src.includes("FEATURE_PILLS"),
    "must NOT define a FEATURE_PILLS array (5 feature pills removed)"
  );
  for (const key of [
    "home.feature.p2p.title",
    "home.feature.compress.title",
    "home.feature.encrypt.title",
    "home.feature.nocounts.title",
    "home.feature.notrack.title",
  ]) {
    assert.ok(
      !src.includes(key),
      `LandingPage must NOT reference ${key} (5 feature pills removed)`
    );
  }
});

test("LandingPage: no drag-drop CTA card (Compress view has it)", () => {
  // The previous version had a big drag-drop CTA card on the
  // right of the hero. That's redundant — the Compress view
  // already has the drop zone. The home is just navigation.
  assert.ok(
    !src.includes("home.dropzone.title"),
    "must NOT render the drag-drop CTA card"
  );
  assert.ok(
    !src.includes("home.dropzone.cta"),
    "must NOT render the drop-zone CTA button"
  );
});

test("LandingPage: no gradient headline", () => {
  // The previous version used bg-gradient-to-r from-cyan-300
  // to-emerald-300 for the headline. That's removed — the
  // abstract version uses plain text-white.
  assert.ok(
    !src.includes("bg-gradient-to-r from-cyan-300"),
    "must NOT use a gradient on the headline"
  );
});

test("LandingPage: 3 action cards (no gradient, single accent)", () => {
  // The 3 main action cards (Comprimir/Extraer/Compartir) are
  // kept but simplified. They no longer have a `from-X-500/15`
  // gradient background — just a neutral border + hover accent.
  assert.ok(
    src.includes("const ACTIONS"),
    "must define ACTIONS array"
  );
  assert.ok(
    src.match(/ACTIONS\.map/),
    "must render the ACTIONS array"
  );
  // The action button must have a `data-testid` for tests.
  assert.ok(
    src.includes('data-testid={`home-action-${a.id}`}'),
    "each action card must have a data-testid"
  );
  // No gradient backgrounds on the action cards.
  assert.ok(
    !src.includes("a.gradient"),
    "action cards must NOT use gradient backgrounds (per-item accent is gone)"
  );
});

test("LandingPage: no 5-stat bar with different colors per stat", () => {
  // The previous version had a 5-stat bar (files processed,
  // bytes saved, ratio, shared, last activity) with one color
  // per stat. That's gone — the recent activity list gives the
  // user the same info without the noise.
  assert.ok(
    !src.includes("StatCard"),
    "must NOT render a StatCard component (5-stat bar removed)"
  );
  assert.ok(
    !src.includes("home.stats.title"),
    "must NOT render the stats bar title"
  );
});

test("LandingPage: no 'Consejo del día' (rotating tip)", () => {
  // The previous version rotated a tip of the day every 12s.
  // Visual noise. The tip is gone.
  assert.ok(
    !src.includes("TIPS"),
    "must NOT define a TIPS array (rotating tip removed)"
  );
  assert.ok(
    !src.includes("home.tip.title"),
    "must NOT render the 'Consejo del día' section"
  );
});

test("LandingPage: single recent activity section (not duplicated)", () => {
  // The previous version had TWO recent-activity sections:
  // - "Actividad" (left column, 3 items, onClick → compress/share)
  // - "Reciente" (right column, 5 items, with "See all" link)
  // Both rendered the same data. The abstract version has ONE
  // section, with 5 items and a "See all" link if there are
  // more than 5.
  const homeActivityRefs = (src.match(/home\.activity\./g) ?? []).length;
  assert.ok(
    homeActivityRefs === 0,
    `must NOT reference home.activity.* (duplicated recent section removed); got ${homeActivityRefs}`
  );
});

test("LandingPage: trilingual i18n keys exist in all 3 locales", () => {
  // Every user-visible string must come from t(). Pin a few
  // critical keys to make sure they exist in ES, EN, IT.
  for (const key of [
    "home.greeting",
    "home.subheadline",
    "action.compress.title",
    "action.decompress.title",
    "action.share.title",
    "home.recent.title",
    "home.recent.empty",
    "home.shortcuts.title",
    "home.shortcuts.cmdk",
  ]) {
    assert.ok(has_key(key), `i18n key ${key} must be in all 3 locales`);
  }
});
