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
}

interface ProgressEvent {
  phase: string;
  current_file: string;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
}

type Mode = "rapido" | "balanceado" | "ultra";

// Only backend/static data — titles and descriptions come from t()
const MODES: {
  id: Mode;
  icon: string;
  stars: number;
  backend: string;
  lzma: number;
}[] = [
  {
    id: "rapido",
    icon: "⚡",
    stars: 4,
    backend: "v4",
    lzma: 0,
  },
  {
    id: "balanceado",
    icon: "⚖",
    stars: 5,
    backend: "v5-min",
    lzma: 6,
  },
  {
    id: "ultra",
    icon: "💎",
    stars: 3,
    backend: "v6-solid",
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
  const [destDir, setDestDir] = useState<string>("");
  const [destInitialized, setDestInitialized] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [pathInput, setPathInput] = useState("");
  // FolderOpen + Plus icons for the new btn-ghost / btn-primary buttons
  // are imported at the top of the file.
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);
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
        // No `filters` → the native panel shows ALL files and
        // accepts any extension. This is what users expect for
        // a generic compressor.
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
    try {
      const lastResult: CompressResult = await tauriInvoke("compress_target_cmd", {
        req: {
          path: inputPath,
          backend: m.backend,
          lzma_level: m.lzma,
          output_dir: destDir || null,
        },
      });
      const durationMs = Date.now() - startTime;
      const savings = lastResult.compressed_size / lastResult.original_size;
      const savingsPct = Math.round((1 - savings) * 100);
      setToast({
        kind: "ok",
        msg: `✓ ${inputFilename} → ${prettyBytes(lastResult.compressed_size)} (${savingsPct}% más pequeño) en ${(durationMs / 1000).toFixed(1)}s`,
      });
      onComplete({
        kind: "compress",
        filename: inputFilename,
        originalBytes: lastResult.original_size,
        compressedBytes: lastResult.compressed_size,
        durationMs,
      });
      setFiles([]);
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

        {/* Progress (only when compressing) */}
        {busy && progress && (
          <div className="mb-10 p-6 rounded-2xl bg-cyan-500/[0.06] border border-cyan-500/20">
            <div className="flex items-center justify-between mb-3">
              <div className="text-cyan-300 text-[11px] tracking-[0.2em] uppercase">
                {progress.phase === "reading"
                  ? t("compress.phase.reading")
                  : progress.phase === "compressing"
                  ? t("compress.phase.compressing")
                  : progress.phase === "writing"
                  ? t("compress.phase.writing")
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
            ? `${t("compress.btn.busy")} ${progressPct.toFixed(0)}%`
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
