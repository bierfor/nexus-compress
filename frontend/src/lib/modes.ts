// Sprint 5.7.21-G: extracted the MODES array from
// CompressView.tsx so it can be unit-tested without
// pulling in React + Tauri runtime. The mirror contract
// between this file and src/supreme_engine.rs::resolve_plan
// is the testable invariant: the chip badge must show
// what the engine will actually emit.
//
// Backend `resolve_plan` table (codec=Auto default):
//   (Auto, Rapido)     → SolidLevel::Zstd(3)
//   (Auto, Balanceado) → SolidLevel::Zstd(3)  (5.7.9 default, 22x speedup)
//   (Auto, Ultra)      → SolidLevel::Lzma(9)
//
// The `defaultCodec` strings below are formatted for the
// UI chip and MUST match these values. If you change one,
// change both.

export type Mode = "rapido" | "balanceado" | "ultra";

export interface ModeMeta {
  id: Mode;
  icon: string;
  stars: number;
  /// Displayed as a chip badge. Mirrors
  /// `src/supreme_engine.rs::resolve_plan` (the codec the
  /// engine picks when `profile.codec === "auto"`).
  defaultCodec: string;
}

export const MODES: ModeMeta[] = [
  {
    id: "rapido",
    icon: "⚡",
    stars: 4,
    defaultCodec: "zstd-3",
  },
  {
    id: "balanceado",
    icon: "⚖",
    stars: 5,
    // Sprint 5.7.9 part 4: balanceado switched from v5-min
    // to v6-solid (5.7.10). The default codec is zstd-3 for
    // the 22x speedup; users can override with `codec: lzma`
    // for higher ratio (5.7.9 hotfix).
    defaultCodec: "zstd-3",
  },
  {
    id: "ultra",
    icon: "💎",
    stars: 3,
    defaultCodec: "lzma-9",
  },
];

/// Look up the chip metadata for a mode. Returns undefined
/// if the mode id is unknown (caller must handle).
export function getMode(id: Mode): ModeMeta | undefined {
  return MODES.find((m) => m.id === id);
}
