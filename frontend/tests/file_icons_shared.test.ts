// Sprint 5.7.21-B-FileIcons-Shared: shared file-icon helpers.
//
// Extracted from LandingPage.tsx in the Sprint 5.7.21-B-Home-Icons
// refactor so EVERY view that renders file rows can use the same
// icon mapping. This test pins:
//   - The shared lib file exists and exports getFileKind + getKindBadge
//   - The icon map covers common file types
//   - The 4 main views (LandingPage, RecentView, ShareView,
//     DecompressView) all import from the shared lib

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SRC = join(import.meta.dirname, "..", "src", "lib", "fileIcons.ts");
const src = readFileSync(SRC, "utf-8");

test("fileIcons.ts: file exists and exports the helpers", () => {
  assert.ok(src.length > 1000, "file should be >1KB");
  assert.ok(
    src.includes("export function getFileKind"),
    "must export getFileKind"
  );
  assert.ok(
    src.includes("export function getKindBadge"),
    "must export getKindBadge"
  );
  assert.ok(
    src.includes("export const DEFAULT_FILE_KIND"),
    "must export DEFAULT_FILE_KIND"
  );
});

test("fileIcons.ts: icon map covers common file types", () => {
  // The icon map must cover at least the common file types
  // a real user would work with: code, images, video, audio,
  // archives, documents, and spreadsheets.
  for (const ext of [
    // Code
    "js", "ts", "py", "rs", "go", "html", "css",
    // Images
    "jpg", "png", "svg", "webp",
    // Video
    "mp4", "mov", "mkv", "webm",
    // Audio
    "mp3", "wav", "flac",
    // Archives
    "zip", "tar", "gz", "bz2", "xz", "nxs", "nxs6", "nxe",
    // Documents
    "pdf", "doc", "txt", "md",
    // Spreadsheets
    "xls", "xlsx", "csv",
    // JSON
    "json",
  ]) {
    assert.ok(
      new RegExp(`\\b${ext}:\\s*\\{`).test(src),
      `FILE_ICON_MAP must include the "${ext}" extension`
    );
  }
});

test("fileIcons.ts: handles compound extensions (.tar.gz etc.)", () => {
  // The function must check for .tar.gz, .tar.bz2, .tar.xz
  // BEFORE the simple .gz / .bz2 / .xz suffix check, otherwise
  // a .tar.gz file would get the generic archive icon instead
  // of the tar-specific one.
  assert.ok(
    /tar\.gz[\s\S]{0,200}tar\.bz2[\s\S]{0,200}tar\.xz/.test(src),
    "must check compound extensions in order (tar.gz, tar.bz2, tar.xz)"
  );
});

test("fileIcons.ts: getKindBadge handles all 3 kinds", () => {
  // The kind badge must handle all 3 op kinds: compress
  // (cyan Archive), decompress (amber FolderOpen), share
  // (emerald Send).
  assert.ok(
    /getKindBadge[\s\S]{0,500}compress[\s\S]{0,200}decompress[\s\S]{0,200}share/.test(
      src
    ),
    "getKindBadge must handle all 3 kinds (compress / decompress / share)"
  );
});

// ── All views must use the shared lib ────────────────────────

const VIEWS = [
  "LandingPage.tsx",
  "RecentView.tsx",
  "ShareView.tsx",
  "DecompressView.tsx",
];

for (const view of VIEWS) {
  test(`${view}: imports getFileKind from @/lib/fileIcons`, () => {
    const viewSrc = readFileSync(
      join(import.meta.dirname, "..", "src", "components", view),
      "utf-8",
    );
    assert.ok(
      viewSrc.includes("from \"@/lib/fileIcons\""),
      `${view} must import the shared file-icon helpers from @/lib/fileIcons`
    );
  });
}
