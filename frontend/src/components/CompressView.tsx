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

import { useEffect, useState, useCallback, useRef } from "react";
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
  ChevronDown,
  ChevronRight,
  Save,
  Trash2,
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
const CODECS: { id: CodecChoice; icon: string }[] = [
  { id: "auto", icon: "🪄" },
  { id: "lzma", icon: "💎" },
  { id: "zstd", icon: "⚡" },
];

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

const CORPUS_MODES: { id: CorpusModeChoice; icon: string; desc: string }[] = [
  { id: "everything", icon: "📦", desc: "compress.corpus.everything.desc" },
  { id: "source",     icon: "📝", desc: "compress.corpus.source.desc" },
  { id: "minimal",    icon: "✨", desc: "compress.corpus.minimal.desc" },
];

const FIDELITY: { id: FidelityChoice; icon: string }[] = [
  { id: "lossy", icon: "✨" },
  { id: "lossless", icon: "🔒" },
];

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
  // Which sections of the accordion are open. The
  // active preset pre-opens relevant sections so the
  // user can see what the preset changes.
  const [openSections, setOpenSections] = useState<Set<string>>(
    new Set(["presets"])
  );
  const toggleSection = (id: string) => {
    setOpenSections((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };
  // Editable name for the "save as custom" dialog.
  const [saveAsName, setSaveAsName] = useState("");
  const [showSaveAs, setShowSaveAs] = useState(false);
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
    // Pre-open the Advanced and Security sections if
    // the preset touches them. Keeps the default
    // collapsed view tidy for users who only want
    // the preset's defaults.
    const open = new Set<string>(["presets"]);
    if (p.rawExtensions || p.minifyExtensions) open.add("advanced");
    if (p.encrypt) open.add("security");
    setOpenSections(open);
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
      setToast({
        kind: "ok",
        msg: `✓ ${lockEmoji}${inputFilename} → ${prettyBytes(lastResult.compressed_size)} (${savingsPct}% más pequeño) en ${(durationMs / 1000).toFixed(1)}s`,
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
          setError("Compression completed but auto-save path was empty. Check console for details.");
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

  // Mode title/desc helpers
  const getModeTitle = (id: Mode) => {
    if (id === "rapido") return t("mode.fast.title");
    if (id === "balanceado") return t("mode.balanced.title");
    return t("mode.ultra.title");
  };
  const getModeDesc = (id: Mode) => {
    if (id === "rapido") return t("mode.fast.desc");
    if (id === "balanceado") return t("mode.balanced.desc");
    return t("mode.ultra.desc");
  };

  // Estimated savings for the preview
  const estSavings =
    profile.mode === "rapido" ? 0.15 : profile.mode === "balanceado" ? 0.35 : 0.5;

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

      <div className="max-w-4xl mx-auto px-8 pt-12 pb-20">
        {/* Header */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
            <button
              onClick={() => onNavigate("landing")}
              className="hover:text-zinc-300 transition-colors"
            >
              {t("back")}
            </button>
          </div>
          <h1 className="text-white text-[36px] font-semibold tracking-tight mb-3">
            {t("compress.title")}
          </h1>
          <p className="text-zinc-400 text-[14px] leading-relaxed max-w-2xl">
            {t("compress.desc")}
          </p>
        </div>

        {/* Sprint 5.7.2 hotfix #24: success card. Shows WHERE the
            output file was saved and gives the user a way to
            reveal it in Finder or kick off another compression
            without leaving the view. Replaces the old behavior
            of silently clearing everything the moment the
            toast fired. */}
        {lastSuccess && !busy && (
          <div className="mb-10 p-6 rounded-2xl bg-gradient-to-br from-emerald-500/[0.08] to-cyan-500/[0.04] border border-emerald-500/20">
            <div className="flex items-start gap-4">
              <div className="flex-shrink-0 w-12 h-12 rounded-full bg-emerald-500/20 flex items-center justify-center text-emerald-400 text-[24px]">
                ✓
              </div>
              <div className="flex-1 min-w-0">
                <div className="text-emerald-300 text-[15px] font-semibold tracking-tight mb-1">
                  {t("compress.success.title")} {lastSuccess.encrypted && "🔒"}
                </div>
                <div className="text-zinc-300 text-[13px] mb-3">
                  <span className="font-mono">{lastSuccess.inputFilename}</span>
                  {" → "}
                  <span className="text-emerald-300 font-medium">
                    {prettyBytes(lastSuccess.compressedSize)}
                  </span>
                  <span className="text-zinc-500">
                    {" "}({Math.round((1 - lastSuccess.compressedSize / lastSuccess.originalSize) * 100)}% {t("compress.success.smaller")}){" "}
                    {t("compress.success.in")} {(lastSuccess.durationMs / 1000).toFixed(1)}s
                  </span>
                </div>
                <div className="text-zinc-500 text-[11px] mb-4 font-mono break-all">
                  📁 {lastSuccess.outputPath}
                </div>

                {/* Sprint 5.7.2 hotfix #44: show the user how many bytes
                    were skipped by the dev-cache filter, so a low
                    ratio on a corpus like secretaria/ (50% .next cache)
                    doesn't look like a bug. */}
                {lastSuccess.skippedBytes && lastSuccess.skippedBytes > 0 && (
                  <div className="text-amber-300/90 text-[11.5px] mb-3 flex items-start gap-2 bg-amber-500/[0.06] border border-amber-500/15 rounded-lg px-3 py-2">
                    <span className="flex-shrink-0 text-[14px] leading-none">ℹ️</span>
                    <span>
                      <span className="font-semibold">Nota de rendimiento:</span>{" "}
                      Se saltaron{" "}
                      <span className="font-mono font-semibold">
                        {prettyBytes(lastSuccess.skippedBytes)}
                      </span>{" "}
                      de caché de desarrollo (`.next`, `node_modules`, etc.).
                      El ratio aplica solo a archivos de código fuente.
                    </span>
                  </div>
                )}
                {/* Sprint 5.7.9 part 6: corpus breakdown by category.
                    Shown when the build-artifact share is high (the
                    most common case where compression ratio is
                    honestly low because the corpus is mostly
                    already-compressed binaries). The user is
                    pointed at Source mode which skips build
                    artifacts. */}
                {lastSuccess.corpusBreakdown && (() => {
                  const b = lastSuccess.corpusBreakdown!;
                  const total = b.sourceBytes + b.buildArtifactBytes + b.otherBytes;
                  if (total === 0) return null;
                  const buildPct = (b.buildArtifactBytes / total) * 100;
                  if (buildPct < 50) return null;
                  return (
                    <div className="text-amber-300/90 text-[11.5px] mb-3 flex items-start gap-2 bg-amber-500/[0.06] border border-amber-500/15 rounded-lg px-3 py-2">
                      <span className="flex-shrink-0 text-[14px] leading-none">💡</span>
                      <span>
                        <span className="font-semibold">Tu corpus es {buildPct.toFixed(0)}% artefactos de build</span>{" "}
                        (<span className="font-mono">{prettyBytes(b.buildArtifactBytes)}</span>{" "}
                        en <code className="text-amber-200/80">.next/</code>, <code className="text-amber-200/80">node_modules/</code>, <code className="text-amber-200/80">venv/</code>, etc.).
                        Esos archivos ya están comprimidos — ningún codec puede reducir el ratio.
                        Para mejor ratio en el código, cambiá a{" "}
                        <span className="font-semibold">📝 Solo fuentes</span> o <span className="font-semibold">✨ Mínimo</span> arriba.
                      </span>
                    </div>
                  );
                })()}
                <div className="flex items-center gap-2">
                  <button
                    onClick={async () => {
                      if (!isTauri) return;
                      try {
                        const { invoke } = await import("@tauri-apps/api/core");
                        await invoke("reveal_in_finder_cmd", { path: lastSuccess.outputPath });
                      } catch (e) {
                        console.error("reveal failed:", e);
                        setToast({ kind: "err", msg: `✗ ${e}` });
                      }
                    }}
                    className="px-4 py-2 rounded-lg bg-cyan-500/15 hover:bg-cyan-500/25 border border-cyan-500/30 text-cyan-300 text-[12.5px] font-medium transition-colors"
                  >
                    📂 {t("compress.success.reveal")}
                  </button>
                  <button
                    onClick={() => setLastSuccess(null)}
                    className="px-4 py-2 rounded-lg bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.08] text-zinc-300 text-[12.5px] font-medium transition-colors"
                  >
                    🔄 {t("compress.success.another")}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Big drop zone */}
        <div className="mb-10">
          {files.length === 0 ? (
            <div
              className={`relative p-16 text-center ${
                dragOver ? "dropzone is-dragover" : "dropzone"
              }`}
            >
              <div className="text-cyan-400 mb-6 select-none inline-block transition-transform duration-300" style={{ transform: dragOver ? "scale(1.15) rotate(-6deg)" : "scale(1)" }}>
                <Archive size={64} strokeWidth={1.4} />
              </div>
              <h3 className="text-white text-[20px] font-medium mb-2">
                {dragOver ? t("compress.drop.active") : t("compress.drop")}
              </h3>
              <p className="text-zinc-500 text-[13px] mb-6">
                {t("compress.drop.hint")}
              </p>
              <div className="flex items-center gap-2 max-w-2xl mx-auto flex-wrap">
                <input
                  type="text"
                  value={pathInput}
                  onChange={(e) => setPathInput(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && onAddPath()}
                  placeholder={t("compress.placeholder")}
                  className="input flex-1 min-w-[200px]"
                />
                {/* Archivos — multi-select file picker with no
                    extension filter (the previous `["*"]` filter
                    was restricting on macOS). */}
                <button
                  onClick={onBrowseFiles}
                  className="btn btn-ghost"
                  title={t("compress.browse.files")}
                >
                  <FilePlus size={14} />
                  {t("compress.browse.files")}
                </button>
                {/* Carpetas — multi-select directory picker.
                    The pipeline (`compress_target_cmd`) walks
                    folders recursively, so any directory just
                    works. */}
                <button
                  onClick={onBrowseFolders}
                  className="btn btn-ghost"
                  title={t("compress.browse.folders")}
                >
                  <FolderOpen size={14} />
                  {t("compress.browse.folders")}
                </button>
                <button
                  onClick={onAddPath}
                  disabled={!pathInput.trim()}
                  className="btn btn-primary"
                >
                  <Plus size={14} />
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


        {/* Sprint 5.7.8: profile + accordion UI.
            Replaces the 5 separate horizontal selector
            blocks (Mode / Codec / Fidelity / Corpus /
            Advanced / Encryption) with:
              1. A horizontal preset bar at the top
                 (chips: built-in presets + custom).
              2. An accordion of 5 sections: Speed,
                 Quality, Corpus, Advanced, Security.
                 Each section is collapsed by default;
                 clicking the header toggles it.
            The active preset is highlighted. When the
            user edits a field inside the accordion, the
            chip loses its active state and a "Save as
            custom" button appears. */}

        {/* Preset bar */}
        <div className="mb-6">
          <div className="flex items-baseline gap-3 mb-3">
            <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase">
              {t("compress.profile.title")}
            </div>
            <div className="text-zinc-600 text-[11px]">
              {t("compress.profile.desc")}
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            {BUILTIN_PRESETS.map((p) => {
              const active = activePresetId === p.id;
              return (
                <button
                  key={p.id}
                  onClick={() => applyPreset(p)}
                  disabled={busy}
                  className={`group relative px-3.5 py-2 rounded-full border text-[12px] transition-all disabled:opacity-50 ${
                    active
                      ? "border-cyan-400/50 bg-cyan-400/10 text-cyan-100 shadow-[0_0_0_1px_rgba(34,211,238,0.15)]"
                      : "border-white/[0.08] bg-white/[0.02] text-zinc-300 hover:border-white/[0.16] hover:bg-white/[0.04]"
                  }`}
                >
                  <span className="mr-1.5">{p.icon}</span>
                  <span className="font-medium">{t(`compress.preset.${p.id}` as "compress.preset.snapshot")}</span>
                </button>
              );
            })}
            {customProfiles.map((p) => {
              const active = activePresetId === p.id;
              return (
                <div key={p.id} className="relative group">
                  <button
                    onClick={() => applyPreset(p)}
                    disabled={busy}
                    className={`pl-3.5 pr-7 py-2 rounded-full border text-[12px] transition-all disabled:opacity-50 ${
                      active
                        ? "border-emerald-400/50 bg-emerald-400/10 text-emerald-100 shadow-[0_0_0_1px_rgba(52,211,153,0.15)]"
                        : "border-white/[0.08] bg-white/[0.02] text-zinc-300 hover:border-white/[0.16] hover:bg-white/[0.04]"
                    }`}
                  >
                    <span className="mr-1.5">⭐</span>
                    <span className="font-medium">{p.name}</span>
                  </button>
                  <button
                    onClick={() => deleteCustom(p.id)}
                    disabled={busy}
                    aria-label={t("compress.profile.delete")}
                    className="absolute right-1 top-1/2 -translate-y-1/2 w-5 h-5 rounded-full text-zinc-500 hover:text-rose-400 hover:bg-rose-400/10 opacity-0 group-hover:opacity-100 transition-all flex items-center justify-center"
                  >
                    <Trash2 size={11} />
                  </button>
                </div>
              );
            })}
            {/* Save current as custom */}
            {profileIsCustom && (
              <button
                onClick={() => {
                  setSaveAsName("");
                  setShowSaveAs(true);
                }}
                disabled={busy}
                className="px-3 py-2 rounded-full border border-dashed border-amber-400/40 bg-amber-400/[0.04] text-amber-200 hover:bg-amber-400/[0.08] text-[12px] transition-all disabled:opacity-50 flex items-center gap-1.5"
              >
                <Save size={12} />
                {t("compress.profile.save_as_custom")}
              </button>
            )}
          </div>
        </div>

        {/* Accordion of detailed controls */}
        <div className="mb-10 rounded-2xl border border-white/[0.06] bg-white/[0.015] divide-y divide-white/[0.06] overflow-hidden">
          {(
            [
              {
                id: "speed",
                title: t("compress.section.speed"),
                desc: t("compress.section.speed.desc"),
                icon: "⚡",
                body: (
                  <div className="space-y-5">
                    <div>
                      <div className="text-zinc-400 text-[11px] uppercase tracking-wider mb-2">
                        {t("compress.mode")}
                      </div>
                      <div className="grid grid-cols-3 gap-2">
                        {MODES.map((m) => {
                          const active = profile.mode === m.id;
                          return (
                            <button
                              key={m.id}
                              onClick={() => updateProfile({ mode: m.id })}
                              disabled={busy}
                              title={t(`compress.mode.${m.id}.tooltip`)}
                              className={`text-left p-3 rounded-xl border transition-all disabled:opacity-50 ${
                                active
                                  ? "border-cyan-500/40 bg-cyan-500/[0.06]"
                                  : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                              }`}
                            >
                              <div className="flex items-center gap-2 mb-1">
                                <span className="text-[15px]">{m.icon}</span>
                                <span className="text-white text-[13px] font-medium">
                                  {getModeTitle(m.id)}
                                </span>
                                <span className="ml-auto text-[9px] text-zinc-500 font-mono">
                                  {m.defaultCodec}
                                </span>
                              </div>
                              <div className="text-zinc-500 text-[10.5px] leading-snug">
                                {t(`compress.mode.${m.id}.desc`)}
                              </div>
                            </button>
                          );
                        })}
                      </div>
                    </div>
                    <div>
                      <div className="text-zinc-400 text-[11px] uppercase tracking-wider mb-2">
                        {t("compress.codec")}
                      </div>
                      <div className="grid grid-cols-3 gap-2">
                        {CODECS.map((c) => {
                          const active = profile.codec === c.id;
                          return (
                            <button
                              key={c.id}
                              onClick={() => updateProfile({ codec: c.id })}
                              disabled={busy}
                              title={t(`compress.codec.${c.id}.tooltip`)}
                              className={`p-3 rounded-xl border transition-all disabled:opacity-50 ${
                                active
                                  ? "border-violet-500/40 bg-violet-500/[0.06]"
                                  : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                              }`}
                            >
                              <div className="flex items-center gap-2">
                                <span className="text-[15px]">{c.icon}</span>
                                <span className="text-white text-[13px] font-medium">
                                  {t(`compress.codec.${c.id}`)}
                                </span>
                              </div>
                              <div className="text-zinc-500 text-[10.5px] leading-snug mt-1">
                                {t(`compress.codec.${c.id}.desc`)}
                              </div>
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  </div>
                ),
              },
              {
                id: "quality",
                title: t("compress.section.quality"),
                desc: t("compress.section.quality.desc"),
                icon: "✨",
                body: (
                  <div className="space-y-4">
                    <div className="text-zinc-400 text-[11px] uppercase tracking-wider mb-2">
                      {t("compress.fidelity")}
                    </div>
                    <div className="grid grid-cols-2 gap-2">
                      {FIDELITY.map((f) => {
                        const active = profile.fidelity === f.id;
                        return (
                          <button
                            key={f.id}
                            onClick={() => updateProfile({ fidelity: f.id })}
                            disabled={busy}
                            className={`text-left p-3 rounded-xl border transition-all disabled:opacity-50 ${
                              active
                                ? "border-amber-500/40 bg-amber-500/[0.06]"
                                : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                            }`}
                          >
                            <div className="flex items-center gap-2 mb-1">
                              <span className="text-[15px]">{f.icon}</span>
                              <span className="text-white text-[13px] font-medium">
                                {t(`compress.fidelity.${f.id}`)}
                              </span>
                            </div>
                            <div className="text-zinc-500 text-[10.5px] leading-snug">
                              {t(`compress.fidelity.${f.id}.desc`)}
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ),
              },
              {
                id: "corpus",
                title: t("compress.section.corpus"),
                desc: t("compress.section.corpus.desc"),
                icon: "📂",
                body: (
                  <div className="space-y-3">
                    {CORPUS_MODES.map((m) => {
                      const active = profile.corpusMode === m.id;
                      return (
                        <button
                          key={m.id}
                          onClick={() => updateProfile({ corpusMode: m.id })}
                          disabled={busy}
                          className={`w-full text-left p-3 rounded-xl border transition-all disabled:opacity-50 ${
                            active
                              ? "border-emerald-500/40 bg-emerald-500/[0.06]"
                              : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                          }`}
                        >
                          <div className="flex items-center gap-2 mb-1">
                            <span className="text-[15px]">{m.icon}</span>
                            <span className="text-white text-[13px] font-medium">
                              {t(`compress.corpus.${m.id}`)}
                            </span>
                            {active && (
                              <span className="ml-auto w-1.5 h-1.5 rounded-full bg-emerald-400" />
                            )}
                          </div>
                          <div className="text-zinc-500 text-[10.5px] leading-snug">
                            {t(`compress.corpus.${m.id}.desc`)}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                ),
              },
              {
                id: "advanced",
                title: t("compress.section.advanced"),
                desc: t("compress.section.advanced.desc"),
                icon: "🛠",
                body: (
                  <div className="space-y-3">
                    <div>
                      <label className="block text-zinc-400 text-[11px] uppercase tracking-wider mb-1.5">
                        {t("compress.advanced.raw.label")}
                      </label>
                      <input
                        type="text"
                        value={profile.rawExtensions}
                        onChange={(e) =>
                          updateProfile({ rawExtensions: e.target.value })
                        }
                        placeholder={t("compress.advanced.raw.placeholder")}
                        disabled={busy || profile.fidelity === "lossless"}
                        className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 disabled:opacity-40"
                      />
                    </div>
                    <div>
                      <label className="block text-zinc-400 text-[11px] uppercase tracking-wider mb-1.5">
                        {t("compress.advanced.minify.label")}
                      </label>
                      <input
                        type="text"
                        value={profile.minifyExtensions}
                        onChange={(e) =>
                          updateProfile({ minifyExtensions: e.target.value })
                        }
                        placeholder={t("compress.advanced.minify.placeholder")}
                        disabled={busy || profile.fidelity === "lossless"}
                        className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 disabled:opacity-40"
                      />
                    </div>
                    {profile.fidelity === "lossless" && (
                      <div className="text-zinc-500 text-[10.5px] italic">
                        {t("compress.advanced.disabled_lossless")}
                      </div>
                    )}
                  </div>
                ),
              },
              {
                id: "security",
                title: t("compress.section.security"),
                desc: t("compress.section.security.desc"),
                icon: "🔐",
                body: (
                  <div className="space-y-3">
                    <label className="flex items-center justify-between p-3 rounded-xl border border-white/[0.06] bg-white/[0.02] cursor-pointer">
                      <div>
                        <div className="text-white text-[12.5px] font-medium">
                          {t("compress.encrypt.label")}
                        </div>
                        <div className="text-zinc-500 text-[10.5px] leading-snug">
                          {t("compress.encrypt.desc")}
                        </div>
                      </div>
                      <input
                        type="checkbox"
                        checked={profile.encrypt}
                        onChange={(e) =>
                          updateProfile({ encrypt: e.target.checked })
                        }
                        disabled={busy}
                        className="w-4 h-4 accent-cyan-500 cursor-pointer disabled:opacity-50"
                      />
                    </label>
                    {profile.encrypt && (
                      <>
                        <div>
                          <label className="block text-zinc-400 text-[11px] uppercase tracking-wider mb-1.5">
                            {t("compress.password.label")}
                          </label>
                          <div className="relative">
                            <input
                              type={showPwd ? "text" : "password"}
                              value={password}
                              onChange={(e) => setPassword(e.target.value)}
                              placeholder={t("compress.password.placeholder")}
                              autoComplete="new-password"
                              spellCheck={false}
                              disabled={busy}
                              className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 pr-9 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 disabled:opacity-50"
                            />
                            <button
                              type="button"
                              onClick={() => setShowPwd((s) => !s)}
                              tabIndex={-1}
                              disabled={busy}
                              className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-200 text-[10px] uppercase tracking-wider px-1.5 py-0.5"
                            >
                              {showPwd ? t("compress.password.hide") : t("compress.password.show")}
                            </button>
                          </div>
                        </div>
                        <div>
                          <div className="text-zinc-400 text-[11px] uppercase tracking-wider mb-1.5">
                            {t("compress.recovery.label")}
                          </div>
                          <div className="grid grid-cols-3 gap-1.5">
                            {(
                              [
                                { v: "off", label: "off", desc: "0% overhead" },
                                { v: "low", label: "low", desc: "1 / 10 files" },
                                { v: "high", label: "high", desc: "2-3 / 8 files" },
                              ] as const
                            ).map((opt) => {
                              const active = profile.recoveryLevel === opt.v;
                              return (
                                <button
                                  key={opt.v}
                                  onClick={() =>
                                    updateProfile({ recoveryLevel: opt.v })
                                  }
                                  disabled={busy}
                                  className={`p-2.5 rounded-lg border transition-all disabled:opacity-50 ${
                                    active
                                      ? "border-cyan-500/40 bg-cyan-500/[0.06]"
                                      : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                                  }`}
                                >
                                  <div className="text-white text-[12px] font-medium">
                                    {opt.label}
                                  </div>
                                  <div className="text-zinc-500 text-[10px]">
                                    {opt.desc}
                                  </div>
                                </button>
                              );
                            })}
                          </div>
                        </div>
                      </>
                    )}
                  </div>
                ),
              },
            ] as const
          ).map((section) => {
            const isOpen = openSections.has(section.id);
            return (
              <div key={section.id}>
                <button
                  onClick={() => toggleSection(section.id)}
                  className="w-full flex items-center gap-3 px-5 py-3.5 text-left hover:bg-white/[0.02] transition-colors"
                >
                  <span className="text-[15px]">{section.icon}</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-zinc-200 text-[13px] font-medium">
                      {section.title}
                    </div>
                    <div className="text-zinc-500 text-[10.5px] truncate">
                      {section.desc}
                    </div>
                  </div>
                  {isOpen ? (
                    <ChevronDown size={14} className="text-zinc-500" />
                  ) : (
                    <ChevronRight size={14} className="text-zinc-500" />
                  )}
                </button>
                {isOpen && (
                  <div className="px-5 pb-5">{section.body}</div>
                )}
              </div>
            );
          })}
        </div>

        {/* Save-as-custom modal (inline, not a real modal — simpler
            than building a portal) */}
        {showSaveAs && (
          <div className="mb-6 p-4 rounded-xl border border-amber-400/30 bg-amber-400/[0.04] flex items-center gap-3">
            <input
              type="text"
              value={saveAsName}
              onChange={(e) => setSaveAsName(e.target.value)}
              placeholder={t("compress.profile.save_as_placeholder")}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  saveAsCustom(saveAsName);
                  setShowSaveAs(false);
                } else if (e.key === "Escape") {
                  setShowSaveAs(false);
                }
              }}
              className="flex-1 bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 text-[12.5px] text-zinc-200 placeholder-zinc-500 focus:outline-none focus:border-amber-400"
            />
            <button
              onClick={() => {
                saveAsCustom(saveAsName);
                setShowSaveAs(false);
              }}
              className="px-4 py-2 rounded-lg bg-amber-500/20 border border-amber-500/40 text-amber-100 text-[12px] font-medium hover:bg-amber-500/30"
            >
              {t("compress.profile.save")}
            </button>
            <button
              onClick={() => setShowSaveAs(false)}
              className="px-3 py-2 rounded-lg text-zinc-400 hover:text-zinc-200 text-[12px]"
            >
              {t("compress.profile.cancel")}
            </button>
          </div>
        )}

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
                  ? "✓ completado"
                  : progress.eta_ms > 0
                    ? `ETA ${formatMs(progress.eta_ms)}`
                    : "ETA —"}
              </span>
            </div>
          </div>
        )}

        {/* Estimated savings preview (when files are loaded and not busy) */}
        {files.length > 0 && !busy && (
          <div className="mb-10 p-6 rounded-2xl bg-gradient-to-br from-cyan-500/[0.06] to-emerald-500/[0.04] border border-white/[0.08]">
            <div className="text-zinc-400 text-[11px] tracking-[0.2em] uppercase mb-4">
              {t("compress.estimate.title")} ({getModeTitle(profile.mode)})
            </div>
            <div className="grid grid-cols-3 gap-6">
              <Stat label={t("compress.estimate.saving")} value={`${(estSavings * 100).toFixed(0)}%`} />
              <Stat
                label={t("compress.estimate.speed")}
                value={
                  profile.mode === "rapido"
                    ? t("compress.speed.fast")
                    : profile.mode === "balanceado"
                    ? t("compress.speed.medium")
                    : t("compress.speed.slow")
                }
              />
              <Stat
                label={t("compress.estimate.best")}
                value={
                  profile.mode === "rapido"
                    ? t("compress.best.video")
                    : profile.mode === "balanceado"
                    ? t("compress.best.general")
                    : t("compress.best.files")
                }
              />
            </div>
            {/* Sprint 5.7.19: el "Estrategia" muestra el códec
                que el motor va a usar, según el profile actual.
                Es el momento de "transparencia radical" del
                roadmap v0.3.0 — el usuario entiende qué va a
                pasar antes de hacer clic. */}
            <div className="mt-4 px-4 py-3 rounded-xl bg-white/[0.04] border border-white/[0.08]">
              <div className="text-zinc-500 text-[10px] tracking-[0.15em] uppercase mb-1">
                {t("compress.estimate.strategy")}
              </div>
              <div className="text-white text-[12px] font-mono">
                {(() => {
                  // Resolver el códec efectivo según el profile.
                  // Esta lógica MIRA la misma tabla que
                  // ProfileMode::resolve_plan() en el engine
                  // (Sprint 5.7.10-C). Si cambia el engine,
                  // cambiar aquí también.
                  const codec = profile.codec === "auto"
                    ? (profile.mode === "ultra" ? "LZMA-9" : "Zstd-3")
                    : profile.codec === "lzma"
                    ? `LZMA-${profile.mode === "rapido" ? 3 : profile.mode === "balanceado" ? 6 : 9}`
                    : "Zstd-3";
                  const preproc = profile.fidelity === "lossless"
                    ? "Raw (bit-exact)"
                    : profile.mode === "ultra"
                    ? "swc AST + Conservative"
                    : "swc AST";
                  const dict = profile.fidelity === "lossy" && profile.mode === "balanceado"
                    ? " + dict (≥16 MiB)"
                    : "";
                  return `${codec} + ${preproc}${dict}`;
                })()}
              </div>
            </div>
          </div>
        )}

        {/* Action button */}
        <button
          onClick={onCompress}
          disabled={files.length === 0 || busy || !destDir}
          className="w-full py-4 rounded-2xl bg-gradient-to-b from-cyan-500 to-cyan-600 hover:from-cyan-400 hover:to-cyan-500 disabled:from-zinc-800 disabled:to-zinc-800 disabled:text-zinc-600 text-white text-[15px] font-semibold tracking-tight transition-all shadow-lg shadow-cyan-500/20 disabled:shadow-none"
        >
          {busy
            ? (progress?.phase === "writing"
              ? `💾 ${t("compress.btn.writing")}`
              : progress?.phase === "done"
                ? `✓ ${t("compress.btn.done")}`
                : `${t("compress.btn.busy")} ${progressPct.toFixed(0)}%`)
            : lastSuccess
              ? `✓ ${t("compress.btn.done")}`
              : t("compress.btn")}
        </button>

        {error && (
          <div className="mt-4 p-4 rounded-xl bg-red-500/[0.08] border border-red-500/20 text-red-400 text-[13px]">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div>
      <div className="text-zinc-500 text-[11px] uppercase tracking-wider mb-1">
        {label}
      </div>
      <div className="text-[20px] font-semibold tabular-nums tracking-tight text-white">
        {value}
      </div>
    </div>
  );
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
