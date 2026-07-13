// Sprint 5.7.21-B-Abstract: SettingsDrawer contract.
//
// The drawer is a slide-in panel from the right that
// absorbs the previous 5-section accordion. It must:
//   - Exist as a separate file (not inlined in CompressView)
//   - Render all 5 sections (Speed / Quality / Content /
//     Advanced / Security)
//   - Close on Esc, click-outside, and X click
//   - Lock body scroll while open
//   - Use trilingual strings (no hardcoded text)

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SRC_DIR = join(import.meta.dirname, "..", "src", "components");
const I18N_PATH = join(import.meta.dirname, "..", "src", "lib", "i18n.ts");

const drawer_src = readFileSync(join(SRC_DIR, "SettingsDrawer.tsx"), "utf-8");
const i18n_src = readFileSync(I18N_PATH, "utf-8");

function has_key(key: string): boolean {
  const count = (i18n_src.match(new RegExp(`"${key}":`, "g")) ?? []).length;
  return count >= 3;
}

test("SettingsDrawer: file exists and exports the component", () => {
  assert.ok(drawer_src.length > 1000, "file should be >1KB");
  assert.ok(
    drawer_src.includes("export function SettingsDrawer"),
    "must export SettingsDrawer"
  );
  assert.ok(
    drawer_src.includes("useLocale"),
    "must import useLocale for trilingual strings"
  );
});

test("SettingsDrawer: contains all 5 sections (Speed / Quality / Content / Advanced / Security)", () => {
  // The 5 sections are rendered as <section> blocks with
  // their respective SectionHeader calls. We pin the
  // section title keys (each must be a `compress.section.*`
  // key from i18n.ts).
  for (const title of [
    "compress.section.speed",
    "compress.section.quality",
    "compress.section.corpus",
    "compress.section.advanced",
    "compress.section.security",
  ]) {
    assert.ok(
      drawer_src.includes(title),
      `must reference section title key ${title}`
    );
    assert.ok(has_key(title), `i18n key ${title} must be in all 3 locales`);
  }
});

test("SettingsDrawer: closes on Esc key", () => {
  // We pin the presence of a keydown listener that
  // closes on Esc. This is the keyboard accessibility
  // invariant. The check accepts both inline (one-line)
  // and multi-line implementations. The listener is
  // registered in a useEffect that runs while `open` is
  // true; the callback checks `e.key === "Escape"` and
  // calls `onClose()`.
  assert.ok(
    drawer_src.includes('addEventListener("keydown"'),
    "must register a keydown listener"
  );
  assert.ok(
    /"Escape"[\s\S]{0,80}onClose/.test(drawer_src),
    "Escape key must trigger onClose (in the same handler)"
  );
});

test("SettingsDrawer: locks body scroll while open", () => {
  // Sprint 5.7.21-B-Abstract: while the drawer is open
  // we want to prevent the user from scrolling the page
  // underneath. The body overflow is restored on close.
  assert.ok(
    drawer_src.includes("document.body.style.overflow"),
    "must lock body scroll while drawer is open"
  );
});

test("SettingsDrawer: X close button + backdrop close", () => {
  // The drawer must have both an explicit X button (for
  // mouse users) and a clickable backdrop (for power
  // users who want to click outside).
  assert.ok(
    drawer_src.includes('aria-label={t("compress.settings.close")}'),
    "must have an X close button with aria-label"
  );
  assert.ok(
    drawer_src.match(/onClick=\{onClose\}/),
    "must have a backdrop with onClick={onClose}"
  );
});
