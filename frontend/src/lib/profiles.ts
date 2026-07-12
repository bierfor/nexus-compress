// Sprint 5.7.8: Compression profiles.
//
// A profile bundles every compression setting the user
// can tweak into a single named bundle. The UI exposes
// three flows:
//
// 1. Pick a built-in preset (Snapshot, Source, Code,
//    Balanced, Ultra, Encrypted). One click.
//
// 2. Edit a preset's settings — the profile becomes
//    "Custom" (with a "save as" affordance).
//
// 3. Save a custom profile with a name, stored in
//    localStorage. Future sessions can re-apply it.
//
// The same shape serialises to a Tauri command field
// (so the backend can receive a profile_id or accept
// the full struct) and to localStorage. The struct is
// versioned: future fields default safely.

export type Mode = "rapido" | "balanceado" | "ultra";
export type CodecChoice = "auto" | "lzma" | "zstd";
export type FidelityChoice = "lossy" | "lossless";
export type CorpusModeChoice = "everything" | "source" | "minimal";

/// Every user-tweakable compression setting in one
/// struct. Adding a new toggle = adding a field here
/// + adding it to the default profile + adding the
/// matching control in CompressView. The version
/// field makes future migrations safe.
export interface CompressionProfile {
  /// Schema version. Bump when the shape changes
  /// incompatibly. v1 = the initial struct.
  schemaVersion: 1;

  // The seven toggles the user controls.
  mode: Mode;
  codec: CodecChoice;
  fidelity: FidelityChoice;
  corpusMode: CorpusModeChoice;
  rawExtensions: string;        // comma-separated
  minifyExtensions: string;     // comma-separated
  encrypt: boolean;
  recoveryLevel: "off" | "low" | "high";
}

/// Built-in presets. Each one is a full profile that
/// the user can apply with a click. The icons match
/// the visual language used in CompressView.
export interface PresetMeta {
  id: string;            // stable identifier
  icon: string;          // emoji for the chip
  builtIn: true;         // can't be deleted
}

export interface CompressionProfilePreset extends CompressionProfile {
  id: string;
  builtIn: true;
  /// Emoji icon shown on the chip. Keeps the chip
  /// recognisable at a glance.
  icon: string;
}

export interface CustomProfile extends CompressionProfile {
  id: string;
  builtIn: false;
  /// User-given name shown in the chip and the picker.
  name: string;
}

export type AnyProfile = CompressionProfilePreset | CustomProfile;

export const DEFAULT_PROFILE: CompressionProfile = {
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

export const BUILTIN_PRESETS: CompressionProfilePreset[] = [
  {
    id: "snapshot",
    icon: "📦",
    builtIn: true,
    schemaVersion: 1,
    // Snapshot: full directory, balanced speed, max ratio.
    // The default for "I want to back up my project as it
    // is right now."
    mode: "balanceado",
    codec: "auto",
    fidelity: "lossy",
    corpusMode: "everything",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: false,
    recoveryLevel: "low",
  },
  // Sprint 5.7.19 (v0.3.0): "Best" preset. The user clicks
  // this once and the engine runs 4 micro-benchmarks on a
  // 5 MB sample of the corpus, picks the best (mode, codec)
  // combination, and applies it. Like WinRAR's "Best" mode.
  // The frontend shows the resolution: "Estrategia
  // recomendada: Zstd-3 Lossy (ratio 6.4x, 80 MB/s)".
  //
  // The mode and codec fields are placeholders — they get
  // overwritten by the engine's `resolve_best` call when
  // the user actually compresses. The UI displays "Best" as
  // the active preset until that resolution happens.
  {
    id: "best",
    icon: "🎯",
    builtIn: true,
    schemaVersion: 1,
    mode: "balanceado",
    codec: "auto",
    fidelity: "lossy",
    corpusMode: "everything",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: false,
    recoveryLevel: "low",
  },
  {
    id: "source",
    icon: "📝",
    builtIn: true,
    schemaVersion: 1,
    // Source: skip dev caches, lossy. The pre-5.7.7 default
    // for "I want to back up just my code, not the
    // node_modules and .next garbage."
    mode: "balanceado",
    codec: "auto",
    fidelity: "lossy",
    corpusMode: "source",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: false,
    recoveryLevel: "low",
  },
  {
    id: "code",
    icon: "✨",
    builtIn: true,
    schemaVersion: 1,
    // Code: minimal corpus (source only), lossy. The
    // "I want the highest ratio on a code project,
    // the archive is for source recovery only" preset.
    mode: "ultra",
    codec: "lzma",
    fidelity: "lossy",
    corpusMode: "minimal",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: false,
    recoveryLevel: "low",
  },
  {
    id: "balanced",
    icon: "⚖️",
    builtIn: true,
    schemaVersion: 1,
    // Balanced: source corpus, auto codec, lossy. The
    // "good default" for everyday work.
    mode: "balanceado",
    codec: "auto",
    fidelity: "lossy",
    corpusMode: "source",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: false,
    recoveryLevel: "low",
  },
  {
    id: "ultra",
    icon: "💎",
    builtIn: true,
    schemaVersion: 1,
    // Ultra: full corpus, LZMA -9, lossy. Slowest,
    // best ratio. For archival where time doesn't
    // matter.
    mode: "ultra",
    codec: "lzma",
    fidelity: "lossy",
    corpusMode: "everything",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: false,
    recoveryLevel: "low",
  },
  {
    id: "encrypted",
    icon: "🔐",
    builtIn: true,
    schemaVersion: 1,
    // Encrypted: source corpus, lossy, AES-256-GCM
    // with low Reed-Solomon recovery. The "send to
    // someone over the network safely" preset.
    mode: "balanceado",
    codec: "auto",
    fidelity: "lossy",
    corpusMode: "source",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: true,
    recoveryLevel: "low",
  },
  {
    id: "lossless",
    icon: "🔒",
    builtIn: true,
    schemaVersion: 1,
    // Lossless: full corpus, every file Preprocessor::Raw,
    // bit-exact reversible. The "I need a true backup,
    // don't change my files" preset.
    mode: "balanceado",
    codec: "auto",
    fidelity: "lossless",
    corpusMode: "everything",
    rawExtensions: "",
    minifyExtensions: "",
    encrypt: false,
    recoveryLevel: "low",
  },
];

// -----------------------------------------------------------------------
// localStorage persistence for custom profiles
// -----------------------------------------------------------------------

const CUSTOM_KEY = "nexus-rar.custom-profiles.v1";

/// Load custom profiles from localStorage. Returns []
/// if storage is unavailable (SSR, private mode, etc).
export function loadCustomProfiles(): CustomProfile[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(CUSTOM_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    // Schema-version sanity check: drop anything
    // from a future schema we don't understand.
    return parsed.filter(
      (p): p is CustomProfile =>
        p &&
        typeof p === "object" &&
        p.schemaVersion === 1 &&
        !p.builtIn &&
        typeof p.id === "string" &&
        typeof p.name === "string"
    );
  } catch {
    return [];
  }
}

/// Save custom profiles to localStorage. Best-effort:
/// quota errors are swallowed (the user keeps the
/// in-memory copy for this session).
export function saveCustomProfiles(profiles: CustomProfile[]): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(CUSTOM_KEY, JSON.stringify(profiles));
  } catch {
    // localStorage might be full or disabled. The
    // in-memory copy still works for this session.
  }
}

/// Diff two profiles. Returns true if any user-
/// tweakable field differs. Used to detect "the
/// user edited a preset" → offer to save as custom.
export function profilesEqual(
  a: CompressionProfile,
  b: CompressionProfile
): boolean {
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
