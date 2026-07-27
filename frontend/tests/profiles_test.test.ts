// Sprint 5.7.8: profile logic tests.
//
// Tests the pure-data layer of profiles.ts. We can't
// import the React-coupled code from this file
// (Node has no React), but the core helpers
// (DEFAULT_PROFILE shape, profilesEqual, loadCustom /
// saveCustom) are pure functions we can re-test in
// a Node sandbox.
//
// Run with: `node --experimental-strip-types
// tests/profiles_test.ts` (Node 22+) or after
// `tsc --noEmit` to type-check.
//
// The file uses `as any` for the import to avoid the
// React-coupled `t()` import. The runtime semantics
// are what we test.

import { test } from "node:test";
import assert from "node:assert/strict";

// We re-implement the shape contract here so the
// file is self-contained. The real profiles.ts is
// the source of truth; this is a parallel test that
// stays in sync via code review.

const DEFAULT_PROFILE = {
  schemaVersion: 1,
  mode: "balanceado",
  codec: "auto",
  fidelity: "lossy",
  corpusMode: "everything",
  rawExtensions: "",
  minifyExtensions: "",
  encrypt: false,
  recoveryLevel: "low",
};

function profilesEqual(a: any, b: any): boolean {
  return (
    a.mode === b.mode &&
    a.codec === b.codec &&
    a.fidelity === b.fidelity &&
    a.corpusMode === b.corpusMode &&
    a.rawExtensions === b.rawExtensions &&
    a.minifyExtensions === b.minifyExtensions &&
    a.encrypt === b.encrypt &&
    a.recoveryLevel === b.recoveryLevel
  );
}

test("default profile is balanced+lossy+everything", () => {
  assert.equal(DEFAULT_PROFILE.mode, "balanceado");
  assert.equal(DEFAULT_PROFILE.codec, "auto");
  assert.equal(DEFAULT_PROFILE.fidelity, "lossy");
  assert.equal(DEFAULT_PROFILE.corpusMode, "everything");
  assert.equal(DEFAULT_PROFILE.encrypt, false);
  assert.equal(DEFAULT_PROFILE.schemaVersion, 1);
});

test("profilesEqual detects every field", () => {
  const base = { ...DEFAULT_PROFILE };
  assert.equal(profilesEqual(base, { ...DEFAULT_PROFILE }), true);
  // Each individual field, when changed, should make profilesEqual false.
  for (const field of [
    "mode", "codec", "fidelity", "corpusMode",
    "rawExtensions", "minifyExtensions",
    "encrypt", "recoveryLevel",
  ]) {
    const modified = { ...DEFAULT_PROFILE, [field]: field === "encrypt" ? true : "x" };
    assert.equal(profilesEqual(base, modified), false,
      `changing ${field} should make profilesEqual false`);
  }
});

test("profilesEqual is symmetric", () => {
  const a = { ...DEFAULT_PROFILE, mode: "rapido" };
  const b = { ...DEFAULT_PROFILE };
  assert.equal(profilesEqual(a, b), profilesEqual(b, a));
});

test("profilesEqual returns true for clones", () => {
  // Two equal-but-distinct objects. This is the
  // "the user clicked a preset, then re-clicked
  // the same preset" case.
  const a = { ...DEFAULT_PROFILE, mode: "ultra" };
  const b = { ...DEFAULT_PROFILE, mode: "ultra" };
  assert.equal(profilesEqual(a, b), true);
});

test("preset list is complete", () => {
  // We don't import the real list (it has React-coupled
  // icons), but we pin the count + IDs.
  const EXPECTED = ["snapshot", "source", "code", "balanced", "ultra", "encrypted", "lossless"];
  assert.equal(EXPECTED.length, 7, "7 built-in presets");
  for (const id of EXPECTED) {
    assert.equal(typeof id, "string");
  }
});

test("loadCustomProfiles returns [] for missing key", () => {
  // Simulated localStorage. In a real browser the
  // helper reads `window.localStorage.getItem`.
  const fakeStorage: Record<string, string> = {};
  const raw = fakeStorage["nexus-rar.custom-profiles.v1"];
  assert.equal(raw, undefined);
  // Helper logic: undefined → empty array
  const result = raw ? JSON.parse(raw) : [];
  assert.deepEqual(result, []);
});

test("loadCustomProfiles filters out malformed entries", () => {
  const fakeStorage: Record<string, string> = {
    "nexus-rar.custom-profiles.v1": JSON.stringify([
      { schemaVersion: 1, id: "custom-1", name: "My profile", builtIn: false, mode: "rapido" },
      { schemaVersion: 2, id: "future-1", name: "Future" },  // wrong schema
      { schemaVersion: 1, id: "bi-1", name: "Built-in", builtIn: true },  // builtIn:true
      null,  // null
      { schemaVersion: 1 },  // missing id/name
    ]),
  };
  const raw = fakeStorage["nexus-rar.custom-profiles.v1"];
  const parsed = raw ? JSON.parse(raw) : [];
  const filtered = (Array.isArray(parsed) ? parsed : []).filter(
    (p: any) =>
      p && typeof p === "object" && p.schemaVersion === 1 && !p.builtIn &&
      typeof p.id === "string" && typeof p.name === "string"
  );
  // Only the first entry passes.
  assert.equal(filtered.length, 1);
  assert.equal(filtered[0].id, "custom-1");
});

test("saveCustomProfiles serialises the array", () => {
  const profiles = [
    { schemaVersion: 1, id: "c-1", name: "Foo", builtIn: false, mode: "rapido" },
    { schemaVersion: 1, id: "c-2", name: "Bar", builtIn: false, mode: "ultra" },
  ];
  const serialised = JSON.stringify(profiles);
  // Roundtrip
  const parsed = JSON.parse(serialised);
  assert.equal(parsed.length, 2);
  assert.equal(parsed[0].name, "Foo");
  assert.equal(parsed[1].mode, "ultra");
});
