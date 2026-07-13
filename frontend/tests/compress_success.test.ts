// Sprint 5.7.21-B-Cleanup: source-level contract tests for
// the new CompressSuccess component.
//
// We don't have a React testing library wired up, so
// instead we pin the invariants via:
//   - source-file existence + size sanity
//   - i18n key coverage (all 3 locales)
//   - hardcoded-text absence (no Spanish / English strings
//     in the JSX that aren't going through `t()`)

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SRC_DIR = join(import.meta.dirname, "..", "src", "components");
const I18N_PATH = join(import.meta.dirname, "..", "src", "lib", "i18n.ts");

const i18n_src = readFileSync(I18N_PATH, "utf-8");
const success_src = readFileSync(join(SRC_DIR, "CompressSuccess.tsx"), "utf-8");

function has_key(key: string): boolean {
  // True if the key exists in all 3 locale blocks.
  const count = (i18n_src.match(new RegExp(`"${key}":`, "g")) ?? []).length;
  return count >= 3;
}

test("CompressSuccess source exists and is a React component", () => {
  assert.ok(success_src.length > 1000, "file should be >1KB");
  assert.ok(
    success_src.includes("export function CompressSuccess"),
    "must export CompressSuccess"
  );
  assert.ok(
    success_src.includes("useLocale"),
    "must import useLocale for trilingual strings"
  );
});

test("CompressSuccess: all i18n strings it uses exist in 3 locales", () => {
  // The component uses these i18n keys. Each must be in
  // ES, EN, IT (3 occurrences) or the build fails.
  for (const key of [
    "compress.success.title",
    "compress.success.smaller",
    "compress.success.reveal",
    "compress.success.another",
    "compress.success.skipped.title",
    "compress.success.skipped.body",
    "compress.success.breakdown.title",
    "compress.success.breakdown.bytes",
    "compress.success.breakdown.explainer",
    "compress.success.breakdown.suggest",
  ]) {
    assert.ok(has_key(key), `missing ${key} in all 3 locales`);
  }
});

test("CompressSuccess: no hardcoded Spanish/English text in JSX strings", () => {
  // The component should route all user-visible strings
  // through t(). We scan the JSX for any string literal
  // containing a Spanish or English common word that's not
  // an i18n key, type name, or import.
  //
  // Skip: type declarations (compressedSize, etc.), imports,
  // i18n keys (compress.*), className strings.
  const suspicious = [
    "completado",
    "comprimir",
    "comprimido",
    "archivo",
    "carpeta",
    "extract",
    "decrypted",
  ];
  for (const word of suspicious) {
    const lines = success_src.split("\n");
    for (const line of lines) {
      // Skip lines that are clearly not user-visible text:
      // imports, comments, className, i18n keys, type
      // declarations, or "compressed" / "decompressed" as
      // variable names.
      if (
        line.trim().startsWith("import ") ||
        line.trim().startsWith("//") ||
        line.trim().startsWith("/*") ||
        line.trim().startsWith("*") ||
        line.includes("className=") ||
        line.includes("compress.") ||
        line.includes(`: number`) ||
        line.includes(`: string`) ||
        line.includes(`compressed`)
      ) {
        continue;
      }
      if (line.includes(word)) {
        assert.fail(
          `hardcoded '${word}' in CompressSuccess: ${line.trim()}`
        );
      }
    }
  }
});

test("CompressSuccess: ratio is the visual hero", () => {
  // The 64px ratio number is the dominant visual element.
  // We pin the className so the test fails if someone shrinks
  // it back to the old 24px subtitle size.
  assert.ok(
    success_src.includes('text-[64px]'),
    "ratio must be rendered at text-[64px] for hero status"
  );
  // The title row "COMPRIMIDO" should be a small uppercase
  // label (10-12px) — the ratio number is the visual hero,
  // the title is a quiet caption above it. Allow decimal
  // sizes (e.g. text-[11.5px]) since the design system
  // uses half-step increments for the small caption tier.
  assert.ok(
    success_src.match(/text-\[1[0-2](?:\.\d+)?px\].*uppercase.*tracking-\[0\.2em\]/),
    "title row should be a small uppercase label (10-12px) with wide tracking"
  );
});

test("CompressSuccess: only TWO primary actions (reveal + another)", () => {
  // The view has exactly two CTAs in a flex row. Any more
  // than two buttons means we're back to the old busy
  // success card.
  const buttonCount = (success_src.match(/<button\b/g) ?? []).length;
  assert.ok(
    buttonCount === 2,
    `expected exactly 2 buttons (reveal + another), found ${buttonCount}`
  );
});
