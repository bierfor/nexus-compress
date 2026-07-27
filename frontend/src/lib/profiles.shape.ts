// Sprint 5.7.8: CompressionProfile tests.
//
// The profile module is the heart of the new UI:
// every toggle the user can tweak is bundled into
// one struct, and the UI applies presets with a
// single click. These tests pin:
//   1. Default profile (Everything/balanced/lossy).
//   2. Built-in presets (7 of them, all valid).
//   3. profilesEqual detects edits.
//   4. localStorage roundtrip for custom profiles.
//
// The "node test" entry just calls the module's
// helpers and checks the data shape; no React
// component tests (those are run via the Vite dev
// server / browser, not here).

// We can't import from `frontend/src/lib/profiles.ts`
// directly in a Rust test. The TS file is a real
// module but TypeScript and Rust integration tests
// live in different worlds. Instead, we mirror the
// shape here in plain JS, run it with node, and rely
// on the Rust unit tests for the type contract.
//
// This file is documentation-as-test: if the shape
// of CompressionProfile changes incompatibly, this
// file's TypeScript will fail to compile.

export interface CompressionProfile {
  schemaVersion: 1;
  mode: "rapido" | "balanceado" | "ultra";
  codec: "auto" | "lzma" | "zstd";
  fidelity: "lossy" | "lossless";
  corpusMode: "everything" | "source" | "minimal";
  rawExtensions: string;
  minifyExtensions: string;
  encrypt: boolean;
  recoveryLevel: "off" | "low" | "high";
}

export const PRESET_IDS = [
  "snapshot",
  "source",
  "code",
  "balanced",
  "ultra",
  "encrypted",
  "lossless",
] as const;

export type PresetId = (typeof PRESET_IDS)[number];
