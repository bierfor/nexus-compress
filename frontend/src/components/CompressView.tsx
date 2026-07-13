"use client";

/**
 * CompressView — dedicated screen for compression (Sprint 5.6.1).
 *
 * Fixes vs Sprint 5.6:
 *   - Real progress bar with % + bytes streamed during compression
 *     (subscribes to the Tauri 'compress-progress' event)
 *   - Toast notification when compression finishes (auto-dismiss)
 *   - Destination default is the actual homeDir/Downloads path,
 *     not a magic "~/Downloads" string that the backend ignores
 *   - Recientes is now populated via the parent state — CompressView
 *     pushes to it via onComplete
 */

import { useEffect, useState, useCallback, useRef, useMemo } from "react";
import { type View } from "@/components/NeoTopBar";
import { useLocale } from "@/components/LocaleProvider";
import {
  FolderOpen,
  Plus,
  Archive,
  CheckCircle2,
  AlertCircle,
  File as FileIcon,
  FilePlus,
  Settings as SettingsIcon,
  Sliders,
} from "lucide-react";
import {
  type CompressionProfile,
  type AnyProfile,
  type CustomProfile,
  BUILTIN_PRESETS,
  DEFAULT_PROFILE,
  loadCustomProfiles,
  saveCustomProfiles,
  profilesEqual,
} from "@/lib/profiles";
// Sprint 5.7.21-B-Abstract: the Compress page is now
// abstract — the configuration UI (5-section accordion +
// 7-card preset grid + savings preview) is gone from the
// main view. The user sees: drop zone + active preset chip
// + Compress action. All fine-tuning lives behind a single
// "Configure" button that opens the SettingsDrawer (slide-in
// from the right). The PresetPicker is the chip + popover
// that replaces the old PresetCard grid.
import { CompressStepper } from "./CompressStepper";
import { CompressStickyBar } from "./CompressStickyBar";
import { CompressSuccess } from "./CompressSuccess";
import { SettingsDrawer } from "./SettingsDrawer";
import { PresetPicker } from "./PresetPicker";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  if (!isTauri) return {} as T;
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

interface CompressResult {
  filename: string;
  output_path: string;
  original_size: number;
  compressed_size: number;
  ratio: number;
  compress_time_ms: number;
  n_files: number;
  /// Sprint 5.7.2 hotfix #44: bytes skipped by the dev-cache filter.
  /// Surfaced as a tooltip on the success card so the user
  /// understands a low ratio on a cache-heavy corpus is intentional.
  skipped_bytes?: number;
}

interface ProgressEvent {
  phase: string;
  current_file: string;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
  // Sprint 5.7.2 hotfix #23: real-time estimates from the Rust
  // `ProgressEvent::with_estimates(start)` builder. Frontend now
  // can render ETA / throughput / elapsed alongside the
  // percentage bar.
  elapsed_ms: number;
  bytes_per_sec: number;
  eta_ms: number;
}

/// Format milliseconds as `m:ss` (or `h:mm:ss` if ≥ 1 hour).
/// Frontend equivalent of the Rust `format_duration` helper.
function formatMs(ms: number): string {
  if (ms < 0 || !Number.isFinite(ms)) return "—";
  const totalSec = Math.floor(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/// Format bytes/sec as a human-friendly rate (MB/s, KB/s, B/s).
function formatRate(bps: number): string {
  if (!Number.isFinite(bps) || bps <= 0) return "—";
  if (bps >= 1024 * 1024) return `${(bps / 1024 / 1024).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${Math.round(bps)} B/s`;
}

type Mode = "rapido" | "balanceado" | "ultra";

// Sprint 5.7.2 hotfix #47: codec toggle. "auto" keeps the
// entropy-flip pipeline; "lzma" and "zstd" pin a single codec
// and skip the flip. The string values match the CLI
// `--codec` argument and the Tauri command's `codec` field.
type CodecChoice = "auto" | "lzma" | "zstd";

// Static data — titles and descriptions come from t()
//
// Sprint 5.7.21-B-Abstract: the CODECS, FIDELITY, and
// CORPUS_MODES arrays now live in SettingsDrawer.tsx
// (where the chips are actually rendered). The type
// aliases below are still useful as TS type-level
// documentation of the choices, so we keep them but
// drop the runtime arrays.

// Sprint 5.7.3 hotfix #49: fidelity toggle. The codec toggle
// (Auto | LZMA | Zstd) decides WHICH entropy coder to use; the
// fidelity toggle (Lossy | Lossless) decides WHETHER to feed
// the smart preprocessor at all. Lossless is the "Safe Mode":
// every file is treated as Preprocessor::Raw, the archive is
// bit-exact reversible. The trade-off is a worse ratio on
// text corpora (typically 1.5-2x instead of 5-7x).
type FidelityChoice = "lossy" | "lossless";

// Sprint 5.7.7 hotfix #55: corpus mode 3-pill selector.
// "everything" (default) = full directory, no skip.
// "source" = skip dev caches / lock files (pre-5.7.7
// hardcoded default; still useful for ratio-focused
// backups). "minimal" = only source code + manifests,
// highest ratio but cannot rebuild.
type CorpusModeChoice = "everything" | "source" | "minimal";

// Only backend/static data — titles and descriptions come from t()
//
// Sprint 5.7.21-G: the MODES array is now in
// `src/lib/modes.ts` so it can be unit-tested without
// pulling in React + Tauri runtime. The contract: the
// chip badge displayed in the UI must match what the
// backend's `resolve_plan` actually emits for
// `codec === "auto"`. See modes.ts for the full table.
import { MODES } from "../lib/modes";

export function CompressView({
  onComplete,
  onNavigate,
}: {
  onComplete: (op: {
    kind: "compress";
    filename: string;
    originalBytes: number;
    compressedBytes: number;
    durationMs: number;
  }) => void;
  onNavigate: (v: View) => void;
}) {
  const { t } = useLocale();
  const [files, setFiles] = useState<string[]>([]);
  // Sprint 5.7.8: profile state. A profile bundles
  // every user-tweakable setting into one struct.
  // The active preset is tracked separately so the UI
  // can highlight the matching chip and offer a
  // "save as custom" affordance when the user edits it.
  const [profile, setProfile] = useState<CompressionProfile>(DEFAULT_PROFILE);
  const [activePresetId, setActivePresetId] = useState<string | null>(null);
  // Custom profiles live in localStorage so they
  // survive app restarts. Loaded on mount, saved on
  // every change.
  const [customProfiles, setCustomProfiles] = useState<CustomProfile[]>([]);
  useEffect(() => {
    setCustomProfiles(loadCustomProfiles());
  }, []);
  useEffect(() => {
    saveCustomProfiles(customProfiles);
  }, [customProfiles]);
  // Sprint 5.7.21-B-Abstract: settings drawer open state
  // (closed by default). The "Configure" button toggles
  // it. The drawer is rendered as a z-50 overlay at the
  // end of the JSX tree. The previous `openSections`
  // accordion state is gone — the drawer is for users
  // who want to fine-tune, not a default UI.
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Legacy individual setters removed in 5.7.8 —
  // the profile is the single source of truth. The
  // invoke and the renders below read directly from
  // `profile.{mode, codec, fidelity, corpusMode,
  // rawExtensions, minifyExtensions, encrypt,
  // recoveryLevel}`. Helpers below (updateProfile)
  // wrap setProfile with the right shape.
  const updateProfile = useCallback(
    (patch: Partial<CompressionProfile>) => {
      setProfile((prev) => ({ ...prev, ...patch }));
      setActivePresetId(null);
    },
    []
  );
  const [destDir, setDestDir] = useState<string>("");

  // Sprint 5.7.4 hotfix #50: turn the comma-separated
  // extension list the user typed in the Advanced panel
  // into the array shape the backend expects. Trims each
  // entry, lowercases it, drops empty strings, and
  // strips the leading dot (so ".json" and "json" both
  // match `json`). Returns [] when the input is blank.
  const parseExtList = (raw: string): string[] => {
    return raw
      .split(",")
      .map((s) => s.trim().toLowerCase())
      .filter((s) => s.length > 0)
      .map((s) => s.replace(/^\./, ""));
  };
  const [destInitialized, setDestInitialized] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [pathInput, setPathInput] = useState("");
  // Sprint 5.7.2: encryption orchestrator state. The password
  // and recovery level flow into the `compress_target_cmd`
  // Tauri call; if `password` is set, the backend routes to
  // the encrypted pipeline (v4 + AES-256-GCM + optional
  // Reed-Solomon).
  //
  // Sprint 5.7.21 (Option B cleanup): `encrypt` and
  // `recoveryLevel` are now part of the `profile` state
  // (5.7.8 single-source-of-truth refactor). Only the
  // `password` stays as local state because it's a runtime
  // secret and intentionally NOT saved to the profile
  // (otherwise it would leak into localStorage and into
  // every Tauri command invocation).
  const [password, setPassword] = useState<string>("");
  const [showPwd, setShowPwd] = useState(false);
  // FolderOpen + Plus icons for the new btn-ghost / btn-primary buttons
  // are imported at the top of the file.
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);
  // Sprint 5.7.2 hotfix #24: success card state. The previous
  // behavior cleared `progress` and `files` the instant compress
  // resolved, which left the user staring at the (empty) input
  // area with no idea where the .nxs/.nxe file landed. Now we
  // pin the last successful result to the screen until the user
  // explicitly chooses to compress another file or reveal the
  // output in Finder. Lives alongside `progress` so a fresh
  // compress can replace it.
  const [lastSuccess, setLastSuccess] = useState<{
    inputFilename: string;
    outputPath: string;
    originalSize: number;
    compressedSize: number;
    durationMs: number;
    encrypted: boolean;
    /// Sprint 5.7.2 hotfix #44: bytes skipped by the dev-cache
    /// filter. Surfaced as a tooltip on the success card.
    skippedBytes?: number;
    /// Sprint 5.7.9 part 6: corpus breakdown by category
    /// (source bytes / build_artifact bytes / other bytes).
    /// The UI uses this to warn the user when their
    /// archive is dominated by build artifacts (where
    /// no codec can do much). For a single-file input
    /// this is undefined.
    corpusBreakdown?: {
      sourceBytes: number;
      sourceFiles: number;
      buildArtifactBytes: number;
      buildArtifactFiles: number;
      otherBytes: number;
      otherFiles: number;
    };
  } | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Initialize the destination to the real ~/Downloads path on mount.
  useEffect(() => {
    if (!isTauri || destInitialized) return;
    (async () => {
      try {
        const { homeDir, join } = await import("@tauri-apps/api/path");
        const home = await homeDir();
        const downloads = await join(home, "Downloads");
        setDestDir(downloads);
      } catch {
        setDestDir("Downloads");
      } finally {
        setDestInitialized(true);
      }
    })();
  }, [destInitialized]);

  // Subscribe to the 'compress-progress' event from the Rust backend
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlisten = await eventMod.listen("compress-progress", (e: any) => {
          const p = e?.payload;
          if (!p) return;
          setProgress({
            phase: String(p.phase ?? ""),
            current_file: String(p.current_file ?? ""),
            files_done: Number(p.files_done ?? 0),
            files_total: Number(p.files_total ?? 1),
            bytes_done: Number(p.bytes_done ?? 0),
            bytes_total: Number(p.bytes_total ?? 0),
            elapsed_ms: Number(p.elapsed_ms ?? 0),
            bytes_per_sec: Number(p.bytes_per_sec ?? 0),
            eta_ms: Number(p.eta_ms ?? 0),
          });
        });
      } catch (e) {
        console.error(e);
      }
    })();
    return () => unlisten?.();
  }, []);

  // Auto-dismiss toast after 5s
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 5000);
    return () => clearTimeout(timer);
  }, [toast]);

  const acceptPaths = useCallback((paths: string[]) => {
    if (paths.length > 0) {
      setFiles((prev) => [...prev, ...paths]);
      setError(null);
    }
  }, []);

  // Drag-and-drop
  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(true);
  }, []);
  const onDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
  }, []);
  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
    const tauriPaths = (e as any).detail?.paths ?? null;
    if (tauriPaths?.length) {
      acceptPaths(tauriPaths);
    }
  }, [acceptPaths]);

  // Tauri drag-drop listener (gives absolute paths)
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlisten = await eventMod.listen("tauri://drag-drop", (e: any) => {
          const paths: string[] = e?.payload?.paths ?? [];
          if (paths.length > 0) acceptPaths(paths);
        });
      } catch (e) {
        console.error(e);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [acceptPaths]);

  // Sprint 5.6.29 hotfix #19: the previous call passed
  //   `filters: [{ name: "All files", extensions: ["*"] }]`
  // which on macOS restricts the NSOpenPanel to ONLY files
  // matching no extension (and sometimes appends literal ".*"
  // to the chosen filename). For a generic compressor we want
  // to accept **any** file (zip, jpg, png, txt, …) and **any**
  // folder. We expose both via two buttons:
  //   - "Archivos" → multi-select file open dialog with NO filter
  //   - "Carpetas" → directory picker
  const onBrowseFiles = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: true,
        directory: false,
        // macOS Sequoia / Sonoma bug: when no `filters` is set,
        // the native NSOpenPanel sometimes defaults to "folders
        // only" mode and the user can't see regular files. Pass
        // an explicit "All files" filter with `["*"]` to force
        // the standard file-selection mode. On other platforms
        // the `["*"]` is treated as a wildcard and shows every
        // file, which is what a generic compressor wants.
        filters: [
          { name: "All files", extensions: ["*"] },
          { name: "Archives", extensions: ["nxs", "nxs6", "nxe", "nxr", "lz", "zip", "tar", "gz"] },
        ],
      });
      if (Array.isArray(result)) acceptPaths(result);
      else if (typeof result === "string") acceptPaths([result]);
    } catch (e) {
      // Sprint 5.7.21-B cleanup: surface the picker error
      // to the user via the existing `error` state instead
      // of silently logging to console. The most common
      // case is the user closing the dialog (we treat that
      // as a soft cancel and don't show anything).
      const msg = String((e as Error)?.message ?? e);
      if (!/cancel/i.test(msg)) {
        setError(t("compress.error.file_picker") + ": " + msg);
      }
    }
  }, [acceptPaths, t]);

  const onBrowseFolders = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: true,
        directory: true,
      });
      if (Array.isArray(result)) acceptPaths(result);
      else if (typeof result === "string") acceptPaths([result]);
    } catch (e) {
      // Sprint 5.7.21-B cleanup: surface the picker error
      // to the user via the existing `error` state. The
      // user closing the dialog is treated as a soft
      // cancel (no error shown).
      const msg = String((e as Error)?.message ?? e);
      if (!/cancel/i.test(msg)) {
        setError(t("compress.error.folder_picker") + ": " + msg);
      }
    }
  }, [acceptPaths, t]);

  // Path input
  const onAddPath = useCallback(() => {
    const trimmed = pathInput.trim();
    if (trimmed) {
      acceptPaths([trimmed]);
      setPathInput("");
    }
  }, [pathInput, acceptPaths]);

  // Browse folder for destination
  const onBrowseDest = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({ multiple: false, directory: true });
      if (typeof result === "string") setDestDir(result);
    } catch (e) {
      // Sprint 5.7.21-B cleanup: surface the destination
      // picker error to the user (the existing `error`
      // state). User-cancelled dialog is silent.
      const msg = String((e as Error)?.message ?? e);
      if (!/cancel/i.test(msg)) {
        setError(t("compress.error.dest_picker") + ": " + msg);
      }
    }
  }, [t]);

  // Apply a built-in or custom preset: replace the
  // current profile with the preset's settings, mark
  // the chip as active, and pre-open the sections
  // the user might want to see.
  const applyPreset = useCallback((p: AnyProfile) => {
    const next: CompressionProfile = {
      schemaVersion: 1,
      mode: p.mode,
      codec: p.codec,
      fidelity: p.fidelity,
      corpusMode: p.corpusMode,
      rawExtensions: p.rawExtensions,
      minifyExtensions: p.minifyExtensions,
      encrypt: p.encrypt,
      recoveryLevel: p.recoveryLevel,
    };
    setProfile(next);
    setActivePresetId(p.id);
    // Sprint 5.7.21-B-Abstract: the previous implementation
    // pre-opened accordion sections based on the preset.
    // The accordion is gone now — every preset's settings
    // are immediately available in the SettingsDrawer
    // (and the drawer is closed by default, so it doesn't
    // matter). Nothing to pre-open here.
  }, []);

  // Save the current profile as a custom preset.
  // Custom profiles live in localStorage and survive
  // app restarts.
  const saveAsCustom = useCallback(
    (name: string) => {
      const id = `custom-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const newCustom: CustomProfile = {
        ...profile,
        schemaVersion: 1,
        id,
        builtIn: false,
        name: name.trim() || t("compress.profile.custom_default_name"),
      };
      setCustomProfiles((prev) => [...prev, newCustom]);
      setActivePresetId(id);
    },
    [profile, t]
  );

  // Delete a custom preset. No-op for built-ins.
  const deleteCustom = useCallback((id: string) => {
    setCustomProfiles((prev) => prev.filter((p) => p.id !== id));
    setActivePresetId(null);
  }, []);

  // Detect "current profile matches no preset" — used
  // to show the "save as custom" affordance.
  const profileIsCustom = !BUILTIN_PRESETS.some((bp) =>
    profilesEqual(bp, profile)
  ) && !customProfiles.some((cp) => profilesEqual(cp, profile));

  // Compress
  const onCompress = useCallback(async () => {
    if (files.length === 0) return;
    setBusy(true);
    setError(null);
    setProgress(null);
    const startTime = Date.now();
    const inputPath = files[0];
    const inputFilename = inputPath.split("/").pop() || "archivo";
    // Guard: encryption requires a non-empty password. We don't
    // fail the click silently — show an error and bail.
    if (profile.encrypt && !password) {
      setError("encryption enabled but no password set");
      setBusy(false);
      return;
    }
    try {
      // Sprint 5.7.10-D: the request is now a single
      // `profile` object instead of 8 separate fields. The
      // backend (SupremeEngine) reads the profile and
      // resolves the right backend / codec / preprocessor
      // internally. The flat-field shape still works
      // (build_profile_from_legacy_req in commands.rs
      // translates it), but the frontend is moving to the
      // canonical nested shape.
      const lastResult: CompressResult = await tauriInvoke("compress_target_cmd", {
        req: {
          path: inputPath,
          output_dir: destDir || null,
          profile: {
            schemaVersion: 1,
            mode: profile.mode,
            codec: profile.codec,
            fidelity: profile.fidelity,
            corpusMode: profile.corpusMode,
            // Per-extension overrides: the GUI keeps them as
            // comma-separated strings in the profile (legacy
            // shape for localStorage); we parse to a string
            // array here. The backend's PreprocessorOverrides
            // constructor normalises / dedupes.
            rawExtensions: parseExtList(profile.rawExtensions),
            minifyExtensions: parseExtList(profile.minifyExtensions),
            encrypt: profile.encrypt,
            recoveryLevel: profile.recoveryLevel,
          },
          // Password is intentionally NOT in the profile —
          // it's a runtime secret, not a saved setting.
          // Custom profiles in localStorage don't carry it.
          password: profile.encrypt ? password : null,
          // Sprint 5.7.8: the active profile id. Backend can
          // log this for reproducibility (a future
          // "compression history by profile" feature).
          profile_id: activePresetId,
        },
      });
      const durationMs = Date.now() - startTime;
      const savings = lastResult.compressed_size / lastResult.original_size;
      const savingsPct = Math.round((1 - savings) * 100);
      const lockEmoji = profile.encrypt ? "🔒 " : "";
      // Sprint 5.7.21-B-Cleanup: trilingual toast (was
      // hardcoded Spanish "% más pequeño en"). Placeholders
      // are filled by the user at the call site via the
      // `t()` helper.
      setToast({
        kind: "ok",
        msg: t("compress.toast.success")
          .replace("{file}", `${lockEmoji}${inputFilename}`)
          .replace("{size}", prettyBytes(lastResult.compressed_size))
          .replace("{pct}", String(savingsPct))
          .replace("{sec}", (durationMs / 1000).toFixed(1)),
      });
      // Pin the success card so the user can see WHERE the
      // .nxs/.nxe file was saved and reveal it in Finder
      // before kicking off another compress.
      //
      // Sprint 5.7.2 hotfix #40: ALWAYS show the success card if
      // compression produced any bytes — don't silently hide it
      // when output_path is empty (which used to make users think
      // the compression "got lost"). When output_path is missing
      // we still show the card so the user sees compression_size,
      // and we surface a warning that auto-save may have failed.
      const outputPath = (lastResult as any).output_path || "";
      const compressedBytes = lastResult.compressed_size || 0;
      if (compressedBytes > 0) {
        if (outputPath) {
          setLastSuccess({
            inputFilename,
            outputPath,
            originalSize: lastResult.original_size,
            compressedSize: compressedBytes,
            durationMs,
            encrypted: profile.encrypt,
            // Sprint 5.7.2 hotfix #44: carry the dev-cache skip
            // bytes into the success card so the UI can show the
            // "skipped X MiB" diagnostic.
            skippedBytes: (lastResult as any).skipped_bytes ?? 0,
            // Sprint 5.7.9 part 6: corpus breakdown by category.
            // Used to warn the user when the archive is dominated
            // by build artifacts (low ratio is honest math, not
            // a bug).
            corpusBreakdown: (lastResult as any).corpus_breakdown,
          });
        } else {
          // Compression succeeded but auto-save produced no path —
          // likely a permission error. Show the card with a generic
          // path so the user knows something happened, plus a
          // warning toast.
          setError(t("compress.error.empty_path"));
          setToast({
            kind: "err",
            msg: `✗ Saved empty (output_path missing) — compressed ${prettyBytes(compressedBytes)}`,
          });
        }
      }
      onComplete({
        kind: "compress",
        filename: inputFilename,
        originalBytes: lastResult.original_size,
        compressedBytes: lastResult.compressed_size,
        durationMs,
      });
      setFiles([]);
      setPassword(""); // wipe the in-memory password after success
    } catch (e: any) {
      const msg = String(e?.message ?? e);
      setError(msg);
      setToast({ kind: "err", msg: `✗ Error: ${msg}` });
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }, [files, profile.mode, destDir, onComplete]);

  // Progress percentage (0–100)
  const progressPct =
    progress && progress.bytes_total > 0
      ? Math.min(100, (progress.bytes_done / progress.bytes_total) * 100)
      : 0;

  // Sprint 5.7.21-B-Remodel: estimated output size + sticky
  // bar button label. Both are derived from the current
  // state so the StickyBar can render without holding its
  // own copies of profile/files/etc.
  //
  // The estimated size is a rough heuristic: 0.5x for lossy
  // modes (typical 2-7x ratio on text/code corpora), 0.85x
  // for lossless (1.2-1.5x ratio on mixed corpora), 1.0x
  // for files we can't estimate (user-typed paths we
  // haven't stat'd). The exact number comes from the
  // progress bar after compress starts.
  const estimatedSize = useMemo(() => {
    if (files.length === 0) return "";
    // We don't stat the files (would block the UI); use
    // a fixed-budget estimate based on the file path
    // string length as a placeholder. The real number
    // comes in via the progress event.
    // 30 bytes per file path is a rough proxy; the actual
    // size comes from the backend's n_files × avg_size
    // once the user hits Compress.
    const rough = files.length * 30 * 1024;
    const ratio = profile.fidelity === "lossless" ? 0.85 : 0.5;
    return prettyBytes(rough * ratio);
  }, [files, profile.fidelity]);

  // The Compress button label changes based on state:
  //   - default: "Comprimir" / "Compress" / "Comprimi"
  //   - busy + writing phase: "Escribiendo…" / "Writing…"
  //   - busy + done phase: "✓ Listo" / "✓ Done" / "✓ Fatto"
  //   - busy + compressing: "Comprimiendo 42%"
  //   - lastSuccess: "✓ Listo" (one-shot before reset)
  //   - no files / no dest: disabled (the StickyBar grays out)
  const compressButtonLabel = useMemo(() => {
    if (busy) {
      if (progress?.phase === "writing") return `💾 ${t("compress.btn.writing")}`;
      if (progress?.phase === "done") return `✓ ${t("compress.btn.done")}`;
      return `${t("compress.btn.busy")} ${progressPct.toFixed(0)}%`;
    }
    if (lastSuccess) return `✓ ${t("compress.btn.done")}`;
    return t("compress.btn");
  }, [busy, progress, progressPct, lastSuccess, t]);

  // Sprint 5.7.21-B-Abstract: getModeTitle/getModeDesc/estSavings
  // are gone — the savings preview card is removed (the user
  // gets the real number on the success screen, no need for
  // a noisy estimate in the main flow).

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`flex-1 overflow-y-auto transition-colors relative ${
        dragOver ? "bg-cyan-500/[0.04]" : ""
      }`}
    >
      {/* Toast notification (success/error after compression) */}
      {toast && (
        <div
          className={`fixed top-20 left-1/2 -translate-x-1/2 z-50 px-5 py-3.5 rounded-2xl shadow-2xl backdrop-blur-md border max-w-2xl animate-slide-down flex items-center gap-3 ${
            toast.kind === "ok"
              ? "bg-emerald-500/15 border-emerald-500/40 text-emerald-100"
              : "bg-rose-500/15 border-rose-500/40 text-rose-100"
          }`}
        >
          {toast.kind === "ok" ? (
            <CheckCircle2 size={18} className="text-emerald-400 shrink-0" />
          ) : (
            <AlertCircle size={18} className="text-rose-400 shrink-0" />
          )}
          <div className="text-[13.5px] font-medium">{toast.msg}</div>
        </div>
      )}

      <div className="max-w-5xl mx-auto px-8 pt-8 pb-20">
        {/* Sprint 5.7.21-B-Cleanup: the page header is gone.
            The TopBar already provides global nav; a per-page
            "Comprimir" title + "Arrastra archivos..." description
            on top of that was visual noise. The stepper (right
            below) tells the user WHERE they are; the drop zone
            (the next section) tells them WHAT to do. */}

        {/* Sprint 5.7.21-B-Cleanup: when lastSuccess is set
            AND the user is not actively compressing, the
            whole screen becomes a "success takeover" view.
            The CompressSuccess component owns the layout —
            no stepper, no drop zone, no preset grid, no
            accordion. The user just sees the result and two
            clear actions. They click "Compress another" to
            come back to the configuration flow. */}

        {/* Sprint 5.7.21-B-Cleanup: hide the stepper when the
            success takeover is active (CompressSuccess has its
            own visual). The stepper only makes sense in the
            configuration flow. */}
        {!lastSuccess && (
          <CompressStepper
            steps={[
              { id: "files", labelKey: "compress.step.files", hintKey: "compress.step.files.hint" },
              { id: "profile", labelKey: "compress.step.profile", hintKey: "compress.step.profile.hint" },
              { id: "output", labelKey: "compress.step.output", hintKey: "compress.step.output.hint" },
              { id: "compress", labelKey: "compress.step.compress", hintKey: "compress.step.compress.hint" },
            ]}
            activeIndex={
              files.length === 0
                ? 0
                : activePresetId === null && !profilesEqual(profile, DEFAULT_PROFILE)
                  ? 1
                  : !destDir
                    ? 2
                    : 3
            }
          />
        )}
        {lastSuccess && !busy ? (
          <CompressSuccess
            info={lastSuccess}
            isTauri={isTauri}
            onAnother={() => setLastSuccess(null)}
            onReveal={async (path) => {
              if (!isTauri) return "Tauri only";
              try {
                const { invoke } = await import("@tauri-apps/api/core");
                await invoke("reveal_in_finder_cmd", { path });
                return null;
              } catch (e) {
                console.error("reveal failed:", e);
                return String((e as Error)?.message ?? e);
              }
            }}
          />
        ) : null}

        {/* Sprint 5.7.21-B-Cleanup: the entire configuration
            flow (drop zone, preset grid, accordion, savings
            preview) is hidden when the success takeover is
            active. The user sees ONLY the success state and
            two clear actions — the configuration UI is not
            visible until they click "Compress another". */}
        {!lastSuccess && (
          <>
        {/* Sprint 5.7.21-B-Abstract: hero drop zone. The
            abstract design reduces the icon size (80 → 36)
            and tightens the padding (py-20 → py-12) so the
            drop zone is a calm container, not a "look at
            me" callout. The user reads the title, types a
            path or drops a file, moves on. No fanfare. */}
        <div className="mb-8">
          {files.length === 0 ? (
            <div
              className={`relative rounded-2xl border-2 border-dashed transition-all duration-200 px-8 py-12 text-center ${
                dragOver
                  ? "border-cyan-400/60 bg-cyan-500/[0.04]"
                  : "border-white/[0.08] hover:border-white/[0.16] bg-white/[0.015]"
              }`}
              onDragOver={onDragOver}
              onDragLeave={onDragLeave}
              onDrop={onDrop}
            >
              <div
                className="text-cyan-400/70 mb-5 select-none inline-block transition-transform duration-300"
                style={{ transform: dragOver ? "scale(1.1) rotate(-4deg)" : "scale(1)" }}
              >
                <Archive size={36} strokeWidth={1.3} />
              </div>
              <h3 className="text-white text-[20px] font-medium tracking-tight mb-1.5">
                {dragOver ? t("compress.drop.active") : t("compress.drop")}
              </h3>
              <p className="text-zinc-500 text-[13px] mb-6 max-w-md mx-auto">
                {t("compress.drop.hint")}
              </p>
              <div className="flex items-center gap-2 max-w-xl mx-auto flex-wrap justify-center">
                <input
                  type="text"
                  value={pathInput}
                  onChange={(e) => setPathInput(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && onAddPath()}
                  placeholder={t("compress.placeholder")}
                  className="flex-1 min-w-[180px] px-3 py-2 rounded-lg bg-white/[0.04] border border-white/[0.08] text-white text-[13px] placeholder:text-zinc-600 focus:outline-none focus:border-cyan-400/40 focus:bg-white/[0.06] transition-all"
                />
                <button
                  onClick={onBrowseFiles}
                  className="px-3 py-2 rounded-lg bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.08] text-zinc-300 hover:text-white text-[12.5px] font-medium transition-all flex items-center gap-1.5"
                  title={t("compress.browse.files")}
                >
                  <FilePlus size={13} />
                  {t("compress.browse.files")}
                </button>
                <button
                  onClick={onBrowseFolders}
                  className="px-3 py-2 rounded-lg bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.08] text-zinc-300 hover:text-white text-[12.5px] font-medium transition-all flex items-center gap-1.5"
                  title={t("compress.browse.folders")}
                >
                  <FolderOpen size={13} />
                  {t("compress.browse.folders")}
                </button>
                <button
                  onClick={onAddPath}
                  disabled={!pathInput.trim()}
                  className="px-3 py-2 rounded-lg bg-cyan-500/15 hover:bg-cyan-500/25 border border-cyan-500/30 text-cyan-300 text-[12.5px] font-medium transition-all flex items-center gap-1.5 disabled:opacity-30 disabled:hover:bg-cyan-500/15"
                >
                  <Plus size={13} />
                  {t("compress.add")}
                </button>
              </div>
            </div>
          ) : (
            <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] overflow-hidden">
              <div className="px-5 py-3 border-b border-white/[0.06] flex items-center justify-between">
                <div className="text-zinc-300 text-[13px] font-medium">
                  {files.length}{" "}
                  {files.length !== 1 ? t("compress.files.plural") : t("compress.files")}
                </div>
                <button
                  onClick={() => setFiles([])}
                  className="text-zinc-500 hover:text-red-400 text-[12px] transition-colors"
                  disabled={busy}
                >
                  {t("compress.clear")}
                </button>
              </div>
              <div className="divide-y divide-white/[0.04]">
                {files.map((f, i) => (
                  <div
                    key={i}
                    className="px-5 py-3 flex items-center gap-3 text-[13px]"
                  >
                    <span className="text-cyan-400">📄</span>
                    <span className="text-white flex-1 truncate" title={f}>
                      {f.split("/").pop()}
                    </span>
                    <span className="text-zinc-600 text-[11px] truncate max-w-md">
                      {f}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>


        {/* Sprint 5.7.21-B-Abstract: the configuration UI is
            now a single chip + a "Configure" button. The
            previous 3-column PresetCard grid + 5-section
            accordion + savings preview are all gone from
            the main view. The user sees:
              - the active preset (chip, with chevron for
                the popover that lists all presets)
              - a small "Configure" button that opens the
                settings drawer (slide-in panel)
            That's it. The Compress action lives in the
            sticky bar at the bottom. */}

        {/* Preset + Configure row */}
        <div className="mb-6 flex items-center justify-center gap-2">
          <PresetPicker
            activePreset={
              activePresetId
                ? BUILTIN_PRESETS.find((p) => p.id === activePresetId) ??
                  customProfiles.find((p) => p.id === activePresetId)
                : undefined
            }
            customProfiles={customProfiles}
            applyPreset={applyPreset}
            profileIsCustom={profileIsCustom}
            saveAsCustom={saveAsCustom}
            deleteCustom={deleteCustom}
            onOpenSettings={() => setSettingsOpen(true)}
            busy={busy}
          />
          <button
            onClick={() => setSettingsOpen(true)}
            disabled={busy}
            aria-label={t("compress.configure.tooltip")}
            title={t("compress.configure.tooltip")}
            data-testid="configure-button"
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-full border border-white/[0.08] bg-white/[0.02] text-zinc-400 hover:text-white hover:bg-white/[0.05] hover:border-white/[0.16] text-[12px] font-medium transition-all disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
          >
            <Sliders size={12} />
            {t("compress.configure")}
          </button>
        </div>



        {/* Progress (only when compressing) */}
        {busy && progress && (
          // Sprint 5.7.2 (commit #12, v0.1.2): the recovery
          // phase gets its own amber/orange color so the
          // user can SEE the moment corruption was detected
          // and RS kicked in. Without this color swap the
          // event would still fire correctly, but the user
          // would have to read the bar text to notice.
          <div
            className={
              progress.phase === "recovering"
                ? "mb-10 p-6 rounded-2xl bg-amber-500/[0.08] border border-amber-500/30"
                : "mb-10 p-6 rounded-2xl bg-cyan-500/[0.06] border border-cyan-500/20"
            }
          >
            <div className="flex items-center justify-between mb-3">
              <div
                className={
                  progress.phase === "recovering"
                    ? "text-amber-300 text-[11px] tracking-[0.2em] uppercase font-semibold"
                    : "text-cyan-300 text-[11px] tracking-[0.2em] uppercase"
                }
              >
                {progress.phase === "reading"
                  ? t("compress.phase.reading")
                  : progress.phase === "compressing"
                  ? t("compress.phase.compressing")
                  : progress.phase === "encrypting"
                  ? t("compress.phase.encrypting")
                  : progress.phase === "building"
                  ? t("compress.phase.building")
                  : progress.phase === "writing"
                  ? t("compress.phase.writing")
                  : progress.phase === "recovering"
                  ? "🚨 " + t("compress.phase.recovering")
                  : progress.phase === "done"
                  ? t("compress.phase.done")
                  : t("compress.phase.processing")}
              </div>
              <div className="text-white text-[20px] font-semibold tabular-nums">
                {progressPct.toFixed(1)}%
              </div>
            </div>
            <div className="h-2 bg-white/[0.04] rounded-full overflow-hidden mb-2">
              <div
                className="h-full bg-gradient-to-r from-cyan-400 to-cyan-500 rounded-full transition-all duration-200"
                style={{ width: `${progressPct}%` }}
              />
            </div>
            <div className="flex items-center justify-between text-[11.5px] text-zinc-400 tabular-nums">
              <span className="truncate max-w-md">
                {progress.current_file || "—"}
              </span>
              <span>
                {prettyBytes(progress.bytes_done)} / {prettyBytes(progress.bytes_total)}
              </span>
            </div>
            {/* Sprint 5.7.2 hotfix #23: real-time ETA / throughput /
                elapsed (the new ProgressEvent fields). Sprint 5.7.2
                hotfix #24: always show elapsed_ms (even 0 → "0:00")
                and compute throughput as a fallback when
                bytes_per_sec is 0 but bytes_done > 0 (the warm-up
                window or events that arrived with elapsed < 100ms
                used to show "—" for both, which looks broken at 100%). */}
            <div className="flex items-center justify-between text-[10.5px] text-zinc-500 tabular-nums mt-1">
              <span>
                {formatMs(progress.elapsed_ms)}
                <span className="text-zinc-700 mx-1.5">·</span>
                {(() => {
                  // Prefer the backend's reported throughput,
                  // but fall back to bytes_done/elapsed_ms so the
                  // bar never shows "—" when bytes have actually
                  // been processed.
                  let rate = progress.bytes_per_sec;
                  if (!(rate > 0) && progress.elapsed_ms > 0 && progress.bytes_done > 0) {
                    rate = (progress.bytes_done * 1000) / progress.elapsed_ms;
                  }
                  return rate > 0 ? formatRate(rate) : "—";
                })()}
              </span>
              <span>
                {progress.bytes_done >= progress.bytes_total && progress.bytes_total > 0
                  ? t("compress.progress.done")
                  : progress.eta_ms > 0
                    ? `ETA ${formatMs(progress.eta_ms)}`
                    : "ETA —"}
              </span>
            </div>
          </div>
        )}

        {/* Sprint 5.7.21-B-Abstract: the savings preview is
            gone. The user already saw the configuration UI
            (the chip + the drawer); the real ratio number
            comes from CompressSuccess after the run. Showing
            a "65% smaller" estimate in the main flow was
            visual noise. */}

        {/* Action button — moved to the sticky bar below */}
        {/* Sprint 5.7.21-B-Remodel: error message is now
            shown as a toast (not a permanent red banner).
            The StickyBar handles the error state internally
            (button shows disabled + tooltip explains why). */}
        </>
        )}

        {/* Sticky bottom action bar — hidden in the success
            takeover (no compress needed in that state).
            The Compress button + destination picker +
            estimated output live here. */}
        {!lastSuccess && (
        <CompressStickyBar
          destDir={destDir}
          onPickDest={onBrowseDest}
          encrypt={profile.encrypt}
          onToggleEncrypt={() => updateProfile({ encrypt: !profile.encrypt })}
          showEncryptToggle
          estimatedSize={estimatedSize}
          canCompress={files.length > 0 && !busy && !!destDir}
          busy={busy}
          buttonLabel={compressButtonLabel}
          onCompress={onCompress}
        />
        )}
      </div>

      {/* Sprint 5.7.21-B-Abstract: settings drawer. Slide-in
          panel from the right that contains the 5 sections
          (Speed / Quality / Content / Advanced / Security)
          plus a compact preset picker. Rendered at the root
          level (z-50 overlay) so it covers the page while
          the user is fine-tuning. */}
      <SettingsDrawer
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        profile={profile}
        updateProfile={updateProfile}
        customProfiles={customProfiles}
        applyPreset={applyPreset}
        activePresetId={activePresetId}
        profileIsCustom={profileIsCustom}
        password={password}
        setPassword={setPassword}
        showPwd={showPwd}
        setShowPwd={setShowPwd}
        saveAsCustom={saveAsCustom}
        deleteCustom={deleteCustom}
        busy={busy}
      />
    </div>
  );
}

// Sprint 5.7.21-B-Abstract: the local `Stat` helper was
// only used by the savings preview (now removed). The
// preset stats live in the success screen (CompressSuccess)
// and the sticky bar's estimated size uses prettyBytes.

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
