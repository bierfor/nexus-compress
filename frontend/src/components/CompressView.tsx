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
} from "lucide-react";

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
// Sprint 5.7.5: each mode surfaces its backend version
// (`version`) so the user can SEE which engine they're
// picking. v4 = fast LZ77+rANS, v5 = balanced LZMA, v5-min
// = LZMA + Conservative minify, v6 = LZMA + swc AST, v6-solid
// = the SOLID pipeline we built in sprint 5.7.2 (the
// per-chunk-codec hybrid with the Format Oracle).
const MODES: {
  id: Mode;
  icon: string;
  stars: number;
  backend: string;
  version: string; // Sprint 5.7.5: displayed as a badge in the pastille
  lzma: number;
}[] = [
  {
    id: "rapido",
    icon: "⚡",
    stars: 4,
    backend: "v4",
    version: "v4",
    lzma: 0,
  },
  {
    id: "balanceado",
    icon: "⚖",
    stars: 5,
    backend: "v5-min",
    version: "v5",
    lzma: 6,
  },
  {
    id: "ultra",
    icon: "💎",
    stars: 3,
    backend: "v6-solid",
    version: "v6",
    lzma: 9,
  },
];

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
  const [mode, setMode] = useState<Mode>("balanceado");
  // Sprint 5.7.2 hotfix #47: codec toggle (Auto | LZMA | Zstd).
  // Default is "auto" so the entropy-driven per-chunk flip from
  // #39 still runs by default — the toggle is the user's opt-out
  // when they want a single, predictable codec for the whole
  // archive.
  const [codec, setCodec] = useState<CodecChoice>("auto");
  // Sprint 5.7.3 hotfix #49: fidelity toggle (Lossy | Lossless).
  // Default "lossy" so existing users keep the 5-7x ratio. The
  // user opts in to bit-exact reversibility by picking
  // "lossless".
  const [fidelity, setFidelity] = useState<FidelityChoice>("lossy");
  // Sprint 5.7.7 hotfix #55: corpus mode (3-pill).
  // "everything" (default since 5.7.7) includes the
  // full directory; "source" skips dev caches (the
  // pre-5.7.7 default); "minimal" keeps only source
  // code + manifests. See api.rs CorpusMode.
  const [corpusMode, setCorpusMode] = useState<CorpusModeChoice>("everything");
  // Sprint 5.7.4 hotfix #50: per-extension override lists for
  // the Advanced panel. Strings are stored as a single
  // comma-separated list per the i18n placeholder convention
  // (e.g. ".json, .env, .toml"). The user can type freely;
  // we trim, lowercase, and dedup on the way out to the
  // invoke. Defaults are empty so the legacy behaviour
  // (built-in per-extension table) is preserved.
  const [rawExtensions, setRawExtensions] = useState<string>("");
  const [minifyExtensions, setMinifyExtensions] = useState<string>("");
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
  const [encrypt, setEncrypt] = useState(false);
  const [password, setPassword] = useState<string>("");
  const [recoveryLevel, setRecoveryLevel] = useState<"off" | "low" | "high">("low");
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
      console.error("file picker:", e);
    }
  }, [acceptPaths]);

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
      console.error("folder picker:", e);
    }
  }, [acceptPaths]);

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
      console.error("folder picker:", e);
    }
  }, []);

  // Compress
  const onCompress = useCallback(async () => {
    if (files.length === 0) return;
    setBusy(true);
    setError(null);
    setProgress(null);
    const startTime = Date.now();
    const m = MODES.find((x) => x.id === mode)!;
    const inputPath = files[0];
    const inputFilename = inputPath.split("/").pop() || "archivo";
    // Guard: encryption requires a non-empty password. We don't
    // fail the click silently — show an error and bail.
    if (encrypt && !password) {
      setError("encryption enabled but no password set");
      setBusy(false);
      return;
    }
    try {
      const lastResult: CompressResult = await tauriInvoke("compress_target_cmd", {
        req: {
          path: inputPath,
          backend: m.backend,
          lzma_level: m.lzma,
          output_dir: destDir || null,
          // Sprint 5.7.2 hotfix #47: codec toggle. "auto" is
          // the legacy path (entropy flip per chunk). "lzma"
          // and "zstd" pin a single codec for the whole
          // archive and bypass the flip. The Rust command
          // (commands.rs) maps this into a `CompressionLevel`
          // passed to `solid_archive::compress_with_progress`.
          codec: codec,
          // Sprint 5.7.3 hotfix #49: fidelity toggle. "lossy"
          // (default) keeps the smart preprocessor
          // (Conservative / swc / Raw by extension). "lossless"
          // forces every file to Preprocessor::Raw, the archive
          // is bit-exact reversible. The Rust command
          // (commands.rs) maps this into a `bool` passed to
          // `compress_target_with_codec_lossless`.
          lossless: fidelity === "lossless",
          // Sprint 5.7.4 hotfix #50: per-extension overrides
          // from the Advanced panel. Empty arrays mean "use
          // the built-in per-extension table" (the legacy
          // behaviour). The backend normalises (lowercases,
          // dedups, drops the leading dot) so the GUI can
          // pass either ".json" or "json" or "JSON".
          raw_extensions: parseExtList(rawExtensions),
          minify_extensions: parseExtList(minifyExtensions),
          // Sprint 5.7.7 hotfix #55: corpus mode from the
          // 3-pill selector. Backend parses it into
          // api::CorpusMode and sets NEXUS_CORPUS_MODE
          // before the walk.
          corpus_mode: corpusMode,
          // Sprint 5.7.2: optional encryption. When `password`
          // is set, the backend routes to the encrypted pipeline
          // (v4 + AES-256-GCM + optional Reed-Solomon). When
          // `password` is null, the existing plain path runs.
          password: encrypt ? password : null,
          recovery_level: encrypt ? recoveryLevel : null,
        },
      });
      const durationMs = Date.now() - startTime;
      const savings = lastResult.compressed_size / lastResult.original_size;
      const savingsPct = Math.round((1 - savings) * 100);
      const lockEmoji = encrypt ? "🔒 " : "";
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
            encrypted: encrypt,
            // Sprint 5.7.2 hotfix #44: carry the dev-cache skip
            // bytes into the success card so the UI can show the
            // "skipped X MiB" diagnostic.
            skippedBytes: (lastResult as any).skipped_bytes ?? 0,
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
  }, [files, mode, destDir, onComplete]);

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
    mode === "rapido" ? 0.15 : mode === "balanceado" ? 0.35 : 0.5;

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

        {/* Mode selector */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-4">
            {t("compress.mode")}
          </div>
          <div className="grid grid-cols-3 gap-3">
            {MODES.map((m) => {
              const active = mode === m.id;
              return (
                <button
                  key={m.id}
                  onClick={() => setMode(m.id)}
                  disabled={busy}
                  className={`relative text-left p-5 rounded-2xl border transition-all disabled:opacity-50 ${
                    active
                      ? "bg-white/[0.08] border-cyan-500/40"
                      : "bg-white/[0.02] border-white/[0.06] hover:border-white/[0.12]"
                  }`}
                >
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-xl">{m.icon}</span>
                    <span className="text-white text-[16px] font-medium">
                      {getModeTitle(m.id)}
                    </span>
                    {/* Sprint 5.7.5: backend version badge so the
                        user knows WHICH engine (v4 / v5 / v6) each
                        mode runs. The badge sits inline with the
                        title in a monospaced font to feel like
                        version metadata, not a feature. */}
                    <span className="ml-auto text-[10px] font-mono text-cyan-300/80 bg-cyan-500/10 px-1.5 py-0.5 rounded border border-cyan-500/20">
                      {m.version}
                    </span>
                  </div>
                  <div className="text-zinc-500 text-[11.5px] leading-relaxed mb-2.5 min-h-[2.6em]">
                    {getModeDesc(m.id)}
                  </div>
                  <div className="flex items-center gap-0.5 text-amber-400/80 text-[11px]">
                    {"★".repeat(m.stars)}
                    <span className="text-zinc-700">
                      {"★".repeat(5 - m.stars)}
                    </span>
                  </div>
                  {active && (
                    <div className="absolute top-3 right-3 w-2 h-2 rounded-full bg-cyan-400" />
                  )}
                </button>
              );
            })}
          </div>
        </div>

        {/* Sprint 5.7.2 hotfix #47: codec toggle (Auto | LZMA | Zstd). */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
            {t("compress.codec")}
          </div>
          <div className="grid grid-cols-3 gap-3">
            {CODECS.map((c) => {
              const active = codec === c.id;
              return (
                <button
                  key={c.id}
                  onClick={() => setCodec(c.id)}
                  disabled={busy}
                  className={`relative text-left p-5 rounded-2xl border transition-all disabled:opacity-50 ${
                    active
                      ? "bg-white/[0.08] border-violet-500/40"
                      : "bg-white/[0.02] border-white/[0.06] hover:border-white/[0.12]"
                  }`}
                >
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-xl">{c.icon}</span>
                    <span className="text-white text-[16px] font-medium">
                      {t(`compress.codec.${c.id}`)}
                    </span>
                  </div>
                  <div className="text-zinc-500 text-[11.5px] leading-relaxed min-h-[2.6em]">
                    {t(`compress.codec.${c.id}.desc`)}
                  </div>
                  {active && (
                    <div className="absolute top-3 right-3 w-2 h-2 rounded-full bg-violet-400" />
                  )}
                </button>
              );
            })}
          </div>
        </div>

        {/* Sprint 5.7.3 hotfix #49: fidelity toggle (Lossy | Lossless).
            Uses an amber border so it visually stands apart from the
            cyan mode selector and the violet codec selector — the three
            toggles now form a colour-coded decision hierarchy:
              cyan   = pipeline (Rapido/Balanceado/Ultra)
              violet = codec (Auto/LZMA/Zstd)
              amber  = fidelity (Lossy/Lossless) */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
            {t("compress.fidelity")}
          </div>
          <div className="grid grid-cols-2 gap-3">
            {FIDELITY.map((f) => {
              const active = fidelity === f.id;
              return (
                <button
                  key={f.id}
                  onClick={() => setFidelity(f.id)}
                  disabled={busy}
                  className={`relative text-left p-5 rounded-2xl border transition-all disabled:opacity-50 ${
                    active
                      ? "bg-white/[0.08] border-amber-500/40"
                      : "bg-white/[0.02] border-white/[0.06] hover:border-white/[0.12]"
                  }`}
                >
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-xl">{f.icon}</span>
                    <span className="text-white text-[16px] font-medium">
                      {t(`compress.fidelity.${f.id}`)}
                    </span>
                  </div>
                  <div className="text-zinc-500 text-[11.5px] leading-relaxed min-h-[2.6em]">
                    {t(`compress.fidelity.${f.id}.desc`)}
                  </div>
                  {active && (
                    <div className="absolute top-3 right-3 w-2 h-2 rounded-full bg-amber-400" />
                  )}
                </button>
              );
            })}
          </div>
        </div>

        {/* Sprint 5.7.7 hotfix #55: corpus mode 3-pill selector.
            Controls what gets included in the directory archive.
            "everything" (default) = full snapshot, "source" =
            skip dev caches (5.7.6 default), "minimal" = source
            code only. Each pill shows an icon, the label, and
            a 1-line description. The active pill is highlighted
            with a coloured border + dot, matching the pattern
            used for Mode/Codec/Fidelity. */}
        <div className="mb-10">
          <div className="flex items-baseline gap-3 mb-3">
            <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase">
              {t("compress.corpus")}
            </div>
            <div className="text-zinc-600 text-[11px]">
              {t("compress.corpus.desc")}
            </div>
          </div>
          <div className="grid grid-cols-3 gap-3">
            {CORPUS_MODES.map((m) => {
              const active = corpusMode === m.id;
              return (
                <button
                  key={m.id}
                  onClick={() => setCorpusMode(m.id)}
                  className={`relative text-left rounded-xl border p-3 transition-all ${
                    active
                      ? "border-emerald-400/40 bg-emerald-400/[0.06] shadow-[0_0_0_1px_rgba(52,211,153,0.15)]"
                      : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12] hover:bg-white/[0.04]"
                  }`}
                >
                  <div className="flex items-center gap-2 mb-1">
                    <span className="text-[16px] leading-none">{m.icon}</span>
                    <span className="text-zinc-200 text-[12.5px] font-semibold">
                      {t(`compress.corpus.${m.id}`)}
                    </span>
                  </div>
                  <div className="text-zinc-500 text-[11px] leading-snug">
                    {t(`compress.corpus.${m.id}.desc` as const)}
                  </div>
                  {active && (
                    <div className="absolute top-3 right-3 w-2 h-2 rounded-full bg-emerald-400" />
                  )}
                </button>
              );
            })}
          </div>
        </div>

        {/* Sprint 5.7.4 hotfix #50: Advanced panel — per-extension
            override inputs. The two fields map directly to the
            Rust `PreprocessorOverrides { raw_extensions,
            minify_extensions }` and to the CLI's `--raw-ext` /
            `--minify-ext` flags. The panel is greyed out under
            "lossless" because the global Raw already covers
            every file — pinning individual extensions would be
            redundant. */}
        <div
          className={`mb-10 rounded-2xl border border-white/[0.06] bg-white/[0.02] p-5 ${
            fidelity === "lossless" ? "opacity-40 pointer-events-none" : ""
          }`}
        >
          <div className="flex items-baseline gap-3 mb-1">
            <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase">
              {t("compress.advanced")}
            </div>
            <div className="text-zinc-600 text-[11px]">
              {t("compress.advanced.desc")}
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3 mt-4">
            <label className="block">
              <div className="text-amber-400/80 text-[11px] tracking-wider uppercase mb-1.5">
                {t("compress.advanced.raw.label")}
              </div>
              <input
                type="text"
                value={rawExtensions}
                onChange={(e) => setRawExtensions(e.target.value)}
                disabled={busy || fidelity === "lossless"}
                placeholder={t("compress.advanced.raw.placeholder")}
                className="w-full bg-white/[0.02] border border-white/[0.06] rounded-lg px-3 py-2 text-white text-[13px] font-mono placeholder:text-zinc-700 focus:border-amber-500/40 focus:outline-none disabled:opacity-50"
              />
            </label>
            <label className="block">
              <div className="text-violet-400/80 text-[11px] tracking-wider uppercase mb-1.5">
                {t("compress.advanced.minify.label")}
              </div>
              <input
                type="text"
                value={minifyExtensions}
                onChange={(e) => setMinifyExtensions(e.target.value)}
                disabled={busy || fidelity === "lossless"}
                placeholder={t("compress.advanced.minify.placeholder")}
                className="w-full bg-white/[0.02] border border-white/[0.06] rounded-lg px-3 py-2 text-white text-[13px] font-mono placeholder:text-zinc-700 focus:border-violet-500/40 focus:outline-none disabled:opacity-50"
              />
            </label>
          </div>
        </div>

        {/* Destination */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
            {t("compress.dest")}
          </div>
          <div className="flex items-center gap-3 px-5 py-4 rounded-2xl bg-white/[0.02] border border-white/[0.06]">
            <span className="text-zinc-500 text-[13px]">📁</span>
            <span
              className="text-white text-[14px] flex-1 truncate font-mono"
              title={destDir}
            >
              {destDir || t("compress.dest.detecting")}
            </span>
            <button
              onClick={onBrowseDest}
              disabled={busy}
              className="px-3 py-1.5 text-[12px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-lg transition-colors disabled:opacity-50"
            >
              {t("compress.dest.change")}
            </button>
          </div>
        </div>

        {/* Sprint 5.7.2: encryption + recovery panel. Toggle to
            enable AES-256-GCM, choose a recovery level, type a
            password. The panel collapses when encryption is off
            (the default — 7z-style "off by default" UX would be
            too aggressive; matching the design doc, encryption
            is opt-in but enabled by default in production). */}
        <div className="mb-4 panel p-3 flex flex-col gap-2">
          <label className="flex items-center justify-between cursor-pointer">
            <span className="metric-label flex items-center gap-2">
              <span className="text-amber-400">🔒</span>
              encryption
            </span>
            <span className="flex items-center gap-2">
              <span className="text-[10px] text-zinc-500 uppercase tracking-wider">
                {encrypt ? "on" : "off"}
              </span>
              <input
                type="checkbox"
                checked={encrypt}
                onChange={(e) => setEncrypt(e.target.checked)}
                disabled={busy}
                className="w-4 h-4 accent-cyan-500 cursor-pointer disabled:opacity-50"
              />
            </span>
          </label>

          {encrypt && (
            <div className="flex flex-col gap-2 mt-1 pt-2 border-t border-zinc-800">
              {/* Password input + visibility toggle */}
              <div className="relative">
                <input
                  type={showPwd ? "text" : "password"}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="archive password"
                  autoComplete="new-password"
                  spellCheck={false}
                  disabled={busy}
                  className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 pr-9 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 focus:bg-zinc-900 disabled:opacity-50"
                />
                <button
                  type="button"
                  onClick={() => setShowPwd((s) => !s)}
                  tabIndex={-1}
                  disabled={busy}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-cyan-400 text-[14px] leading-none disabled:opacity-50"
                  title={showPwd ? "hide password" : "show password"}
                >
                  {showPwd ? "🙈" : "👁"}
                </button>
              </div>

              {/* Recovery level radio group. Default "low" (10%
                  parity, matches the 7z UX). */}
              <div className="flex flex-col gap-1.5">
                <div className="text-[10px] text-zinc-500 uppercase tracking-wider">
                  recovery (Reed-Solomon)
                </div>
                <div className="grid grid-cols-3 gap-1.5">
                  {(
                    [
                      { v: "off", label: "off", desc: "0% overhead" },
                      { v: "low", label: "low", desc: "1 / 10 files" },
                      { v: "high", label: "high", desc: "2-3 / 8 files" },
                    ] as const
                  ).map((opt) => {
                    const active = recoveryLevel === opt.v;
                    return (
                      <button
                        key={opt.v}
                        type="button"
                        disabled={busy}
                        onClick={() => setRecoveryLevel(opt.v)}
                        className={`px-2 py-1.5 rounded border text-[10.5px] font-mono transition-colors disabled:opacity-50 ${
                          active
                            ? "border-cyan-500 bg-cyan-500/10 text-cyan-300"
                            : "border-zinc-800 text-zinc-400 hover:border-zinc-700 hover:text-zinc-300"
                        }`}
                      >
                        <div className="font-medium">{opt.label}</div>
                        <div className="text-[9px] text-zinc-500 mt-0.5">
                          {opt.desc}
                        </div>
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>
          )}
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
              {t("compress.estimate.title")} ({getModeTitle(mode)})
            </div>
            <div className="grid grid-cols-3 gap-6">
              <Stat label={t("compress.estimate.saving")} value={`${(estSavings * 100).toFixed(0)}%`} />
              <Stat
                label={t("compress.estimate.speed")}
                value={
                  mode === "rapido"
                    ? t("compress.speed.fast")
                    : mode === "balanceado"
                    ? t("compress.speed.medium")
                    : t("compress.speed.slow")
                }
              />
              <Stat
                label={t("compress.estimate.best")}
                value={
                  mode === "rapido"
                    ? t("compress.best.video")
                    : mode === "balanceado"
                    ? t("compress.best.general")
                    : t("compress.best.files")
                }
              />
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
