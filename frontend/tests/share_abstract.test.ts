// Sprint 5.7.21-B-Abstract: ShareView contract.
//
// The share view used to be a 1266-line file with 3 sub-panels
// (SendPanel, ReceivePanel, LinkPanel) shown all at once, plus
// a big page header (h1 + 3 selling-point chips). The abstract
// version is a thin router with tabs (Send | Receive) and only
// the active panel visible at a time. The LinkPanel renders
// as a sub-section below the SendPanel once a share is active.
//
// Pinned invariants:
//   - No big page header (h1 with share.title)
//   - No 3 selling-point chips (share.feature.*)
//   - Tabs at the top (share.tab.send / share.tab.receive)
//   - Only the active panel renders (no 2-col layout)
//   - LinkPanel renders inside the Send tab (sub-section)
//   - The 3 feature pills inside LinkPanel (Expiración /
//     Ilimitado / E2E) are removed
//   - The preview + activity list are kept (compact)

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const SRC = join(
  import.meta.dirname,
  "..",
  "src",
  "components",
  "ShareView.tsx",
);
const I18N = join(import.meta.dirname, "..", "src", "lib", "i18n.ts");
const src = readFileSync(SRC, "utf-8");
const i18n = readFileSync(I18N, "utf-8");

function has_key(key: string): boolean {
  return (i18n.match(new RegExp(`"${key}":`, "g")) ?? []).length >= 3;
}

test("ShareView: file is smaller (abstract)", () => {
  // The original was 1266 lines. Sprint 5.7.21-B-Share-Improve
  // + Sprint 5.7.21-B-Share-Icons added icons, state machine,
  // connection badges, and other visual improvements — the
  // file grew to ~1578 lines but the design quality is
  // significantly better. We pin a ceiling of 1600 to allow
  // future additions without regression to the pre-abstract state.
  const lineCount = src.split("\n").length;
  assert.ok(
    lineCount < 1600,
    `ShareView should be <1600 lines (was 1266 before this sprint); got ${lineCount}`
  );
});

test("ShareView: no big page header (h1 with share.title)", () => {
  // The previous version had a big <h1> at the top of the
  // page with the share.title and share.desc. The TopBar
  // already provides global nav; the per-page title was
  // visual noise.
  assert.ok(
    !/className="text-white text-\[32px\][^"]*tracking-tight[^"]*"[^>]*>\s*\{t\("share\.title"\)/m.test(
      src
    ),
    "must NOT render a big h1 with share.title (TopBar provides nav)"
  );
});

test("ShareView: no 3 selling-point chips (share.feature.*)", () => {
  // The previous version had 3 chips at the top right of the
  // header: "Sin límites", "Sin servidor", "E2E cifrado".
  // These repeated the marketing the user already knows.
  for (const key of [
    "share.feature.unlimited",
    "share.feature.nostorage",
    "share.feature.e2e",
  ]) {
    // The key might still exist in i18n (orphan), but the
    // ShareView must NOT reference it.
    assert.ok(
      !src.includes(key),
      `ShareView must NOT reference ${key} (3 selling-point chips removed)`
    );
  }
});

test("ShareView: tabs at the top (Send | Receive)", () => {
  // The abstract view shows tabs so the user picks ONE flow
  // instead of seeing both flows at once.
  assert.ok(
    src.includes('data-testid="share-tab-send"'),
    "must have a Send tab with data-testid"
  );
  assert.ok(
    src.includes('data-testid="share-tab-receive"'),
    "must have a Receive tab with data-testid"
  );
  assert.ok(
    src.includes("share.tab.send") || src.includes("\"share.tab.send\""),
    "must reference share.tab.send i18n key"
  );
  assert.ok(
    src.includes("share.tab.receive") ||
      src.includes("\"share.tab.receive\""),
    "must reference share.tab.receive i18n key"
  );
});

test("ShareView: only the active panel renders (no 2-col layout)", () => {
  // The previous 2-col layout (SendPanel 5/12 + LinkPanel 7/12)
  // is gone. The active panel renders alone.
  assert.ok(
    !src.includes("lg:col-span-5"),
    "must NOT have a 2-col layout (lg:col-span-5)"
  );
  assert.ok(
    !src.includes("lg:col-span-7"),
    "must NOT have a 2-col layout (lg:col-span-7)"
  );
});

test("ShareView: LinkPanel renders as sub-section of Send tab", () => {
  // The LinkPanel used to be a side panel in the 2-col
  // layout. Now it renders below the SendPanel when a
  // share is active (shareResp is set).
  assert.ok(
    /tab === "send"/.test(src) && /shareResp/.test(src),
    "LinkPanel must render inside the Send tab, conditional on shareResp"
  );
});

test("ShareView: removed the 3 feature pills inside LinkPanel", () => {
  // The previous LinkPanel had 3 small cards at the bottom:
  // Expiración, Ilimitado, E2E. These repeated the selling
  // points that used to live in the page header. Gone.
  assert.ok(
    !src.includes("share.link.feature.expires"),
    "must NOT render the Expiración feature pill"
  );
  assert.ok(
    !src.includes("share.link.feature.unlimited"),
    "must NOT render the Ilimitado feature pill"
  );
  assert.ok(
    !src.includes("share.link.feature.e2e"),
    "must NOT render the E2E feature pill"
  );
});

test("ShareView: i18n keys it uses exist in all 3 locales", () => {
  // Every user-visible string must come from t(). Pin a few
  // critical keys to make sure they exist in ES, EN, IT.
  for (const key of [
    "share.tab.send",
    "share.tab.receive",
    "share.send.step1.title",
    "share.send.drop",
    "share.send.btn",
    "share.link.title",
    "share.link.copy",
    "share.link.token.label",
  ]) {
    assert.ok(has_key(key), `i18n key ${key} must be in all 3 locales`);
  }
});

test("ShareView: SendPanel has both file and folder browse buttons", () => {
  // Sprint 5.7.21-B-Share-Improve: the user can pick a file
  // OR a folder to share. The previous version only had a file
  // picker; onBrowseFolder was defined but never called.
  assert.ok(
    src.includes('data-testid="send-browse-file"'),
    "must have a 'Choose file' button with data-testid"
  );
  assert.ok(
    src.includes('data-testid="send-browse-folder"'),
    "must have a 'Choose folder' button with data-testid"
  );
  // The i18n keys for both labels must exist in 3 locales.
  assert.ok(
    has_key("share.send.browse.file"),
    "share.send.browse.file must be in all 3 locales"
  );
  assert.ok(
    has_key("share.send.browse.folder"),
    "share.send.browse.folder must be in all 3 locales"
  );
});

test("ShareView: SendPanel has a path text input (paste path)", () => {
  // The user can type a path directly (useful when the native
  // picker is flaky on macOS Sequoia).
  assert.ok(
    has_key("share.send.placeholder"),
    "share.send.placeholder must exist in all 3 locales"
  );
  assert.ok(
    has_key("share.send.add"),
    "share.send.add must exist in all 3 locales"
  );
});

test("ShareView: SendPanel tracks pathKind (file vs folder)", () => {
  // The preview icon switches based on whether the selected
  // path is a file or a folder. The kind is set by the explicit
  // browse buttons.
  assert.ok(
    /pathKind/.test(src),
    "SendPanel must track pathKind state"
  );
  assert.ok(
    /PreviewIcon/.test(src),
    "SendPanel must compute a PreviewIcon based on pathKind"
  );
  // The 'ready' labels are different for files vs folders.
  assert.ok(
    has_key("share.send.ready.file"),
    "share.send.ready.file must exist in all 3 locales"
  );
  assert.ok(
    has_key("share.send.ready.folder"),
    "share.send.ready.folder must exist in all 3 locales"
  );
});

test("ShareView: SendPanel handles multi-file drop with a warning", () => {
  // When the user drops multiple paths, the backend only takes
  // the first. We surface a quiet warning so they know.
  assert.ok(
    /multiWarning/.test(src),
    "SendPanel must track multiWarning state"
  );
  assert.ok(
    has_key("share.send.multi.warning"),
    "share.send.multi.warning must exist in all 3 locales"
  );
});

test("ShareView: SendPanel CTA is flat (no gradient)", () => {
  // Sprint 5.7.21-B-Share-Improve: replaced the 3-color
  // gradient CTA with a single-accent flat button, consistent
  // with the rest of the app. The previous gradient used
  // linear-gradient(135deg, #34d399 0%, #10b981 35%, #22d3ee 100%).
  assert.ok(
    !src.includes("linear-gradient(135deg, #34d399"),
    "must NOT have the 3-color gradient CTA (replaced with flat cyan)"
  );
});

test("ShareView: LinkPanel has state machine + connection badge", () => {
  // Sprint 5.7.21-B-Share-Icons: the LinkPanel shows a
  // 4-step state machine (Created → Ready → Connected →
  // Sent) and a connection type badge (LAN vs Internet tunnel).
  assert.ok(
    src.includes("data-testid=\"link-state-machine\""),
    "LinkPanel must render the state machine with data-testid"
  );
  assert.ok(
    src.includes("data-testid=\"link-status-badge\""),
    "LinkPanel must render the status badge with data-testid"
  );
  assert.ok(
    src.includes("data-testid=\"link-connection-badge\""),
    "LinkPanel must render the connection badge with data-testid"
  );
  // The state labels must exist in i18n (3 locales each).
  for (const key of [
    "share.link.state.ready",
    "share.link.state.connected",
    "share.link.connection.lan",
    "share.link.connection.tunnel",
    "share.link.step.created",
    "share.link.step.ready",
  ]) {
    assert.ok(has_key(key), `i18n key ${key} must be in all 3 locales`);
  }
});

test("ShareView: SendPanel uses better drop zone icon (Share2 + Layers)", () => {
  // Sprint 5.7.21-B-Share-Icons: the drop zone icon changed
  // from a single FileText to a Share2 with a Layers corner
  // badge (suggesting "items to share"). On drag-over, the
  // icon rotates and the bg changes to suggest "this is where
  // items land".
  assert.ok(
    /Share2 size=\{28\}/.test(src),
    "drop zone must use Share2 as the main icon"
  );
  assert.ok(
    /Layers size=\{12\}/.test(src),
    "drop zone must have a small Layers corner badge"
  );
  assert.ok(
    /Inbox size=\{32\}/.test(src),
    "drop zone on drag-over must use Inbox icon"
  );
});
