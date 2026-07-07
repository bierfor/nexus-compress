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

const MODES: {
  id: Mode;
  icon: string;
  title: string;
  description: string;
  stars: number;
  backend: string;
  lzma: number;
}[] = [
  {
    id: "rapido",
    icon: "⚡",
    title: "Rápido",
    description: "Ideal para vídeos. Compresión rápida, ahorra ~10-20%.",
    stars: 4,
    backend: "v4",
    lzma: 0,
  },
  {
    id: "balanceado",
    icon: "⚖",
    title: "Balanceado",
    description: "Recomendado. Mejor relación tiempo/tamaño para la mayoría de archivos.",
    stars: 5,
    backend: "v5",
    lzma: 6,
  },
  {
    id: "ultra",
    icon: "💎",
    title: "Ultra",
    description: "Máxima compresión. Más lento, pero ahorra más espacio.",
    stars: 3,
    backend: "v6",
    lzma: 9,
  },
];

export function CompressView({
  onComplete,
}: {
  onComplete: (op: {
    kind: "compress";
    filename: string;
    originalBytes: number;
    compressedBytes: number;
    durationMs: number;
  }) => void;
}) {
  const [files, setFiles] = useState<string[]>([]);
  const [mode, setMode] = useState<Mode>("balanceado");
  const [destDir, setDestDir] = useState<string>("");
  const [destInitialized, setDestInitialized] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [pathInput, setPathInput] = useState("");
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  const [toast, setToast] = useState<{ kind: "ok" | "err"; msg: string } | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Initialize the destination to the real ~/Downloads path on mount
  useEffect(() => {
    if (!isTauri || destInitialized) return;
    (async () => {
      try {
        const { homeDir } = await import("@tauri-apps/api/path");
        const home = await homeDir();
        setDestDir(`${home}Downloads`);
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
    const t = setTimeout(() => setToast(null), 5000);
    return () => clearTimeout(t);
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

  // HTML file input (hidden)
  const onBrowse = useCallback(() => {
    fileInputRef.current?.click();
  }, []);
  const onFileChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const fl = Array.from(e.target.files ?? []);
    if (fl.length > 0) {
      const names = fl.map((f) => (f as any).path || f.name);
      acceptPaths(names);
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
    try {
      const lastResult: CompressResult = await tauriInvoke("compress_target_cmd", {
        req: {
          path: files[0],
          backend: m.backend,
          lzma_level: m.lzma,
          // Always send output_dir (the homeDir/Downloads default is
          // now a real path, so we never pass null).
          output_dir: destDir || null,
        },
      });
      const durationMs = Date.now() - startTime;
      const savings = lastResult.compressed_size / lastResult.original_size;
      const savingsPct = Math.round((1 - savings) * 100);
      setToast({
        kind: "ok",
        msg: `✓ ${lastResult.filename} → ${prettyBytes(lastResult.compressed_size)} (${savingsPct}% más pequeño) en ${(durationMs / 1000).toFixed(1)}s`,
      });
      onComplete({
        kind: "compress",
        filename: lastResult.filename,
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
  // Estimated savings for the preview (no real file size available
  // before compression, so this is a heuristic by mode).
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
          className={`fixed top-20 left-1/2 -translate-x-1/2 z-50 px-5 py-3 rounded-2xl shadow-2xl backdrop-blur-md border max-w-2xl animate-[slide-down_0.3s_ease-out] ${
            toast.kind === "ok"
              ? "bg-emerald-500/15 border-emerald-500/30 text-emerald-100"
              : "bg-red-500/15 border-red-500/30 text-red-100"
          }`}
        >
          <div className="text-[13.5px] font-medium">{toast.msg}</div>
        </div>
      )}

      <div className="max-w-4xl mx-auto px-8 pt-12 pb-20">
        {/* Header */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
            <button
              onClick={() => window.history.back()}
              className="hover:text-zinc-300 transition-colors"
            >
              ← Volver
            </button>
          </div>
          <h1 className="text-white text-[36px] font-semibold tracking-tight mb-3">
            Comprimir
          </h1>
          <p className="text-zinc-400 text-[14px] leading-relaxed max-w-2xl">
            Arrastra archivos aquí, elige el modo y el destino. La compresión
            ocurre en local — tus archivos no salen de tu Mac.
          </p>
        </div>

        {/* Big drop zone */}
        <div className="mb-10">
          {files.length === 0 ? (
            <div
              className={`relative rounded-3xl border-2 border-dashed transition-all p-16 text-center ${
                dragOver
                  ? "border-cyan-400 bg-cyan-500/[0.08]"
                  : "border-white/[0.08] bg-white/[0.02]"
              }`}
            >
              <div className="text-7xl mb-6 select-none">
                {dragOver ? "⤓" : "📦"}
              </div>
              <h3 className="text-white text-[20px] font-medium mb-2">
                {dragOver ? "Suelta para añadir" : "Arrastra tus archivos aquí"}
              </h3>
              <p className="text-zinc-500 text-[13px] mb-6">
                o usa el campo de abajo para escribir la ruta
              </p>
              <input
                ref={fileInputRef}
                type="file"
                multiple
                onChange={onFileChange}
                className="hidden"
              />
              <div className="flex items-center gap-2 max-w-xl mx-auto">
                <input
                  type="text"
                  value={pathInput}
                  onChange={(e) => setPathInput(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && onAddPath()}
                  placeholder="/Users/usuario/Desktop/archivo.mkv"
                  className="flex-1 bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-2.5 text-[13px] text-white placeholder-zinc-600 focus:outline-none focus:border-cyan-500/50"
                />
                <button
                  onClick={onBrowse}
                  className="px-4 py-2.5 text-[13px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-xl transition-colors"
                >
                  Explorar
                </button>
                <button
                  onClick={onAddPath}
                  disabled={!pathInput.trim()}
                  className="px-4 py-2.5 text-[13px] text-cyan-400 hover:text-cyan-300 border border-cyan-500/30 hover:border-cyan-500/50 rounded-xl transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                >
                  Añadir
                </button>
              </div>
            </div>
          ) : (
            <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] overflow-hidden">
              <div className="px-5 py-3 border-b border-white/[0.06] flex items-center justify-between">
                <div className="text-zinc-300 text-[13px] font-medium">
                  {files.length} archivo{files.length !== 1 ? "s" : ""}
                </div>
                <button
                  onClick={() => setFiles([])}
                  className="text-zinc-500 hover:text-red-400 text-[12px] transition-colors"
                  disabled={busy}
                >
                  Limpiar
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
            Modo
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
                      {m.title}
                    </span>
                  </div>
                  <div className="text-zinc-500 text-[11.5px] leading-relaxed mb-2.5 min-h-[2.6em]">
                    {m.description}
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
            Destino
          </div>
          <div className="flex items-center gap-3 px-5 py-4 rounded-2xl bg-white/[0.02] border border-white/[0.06]">
            <span className="text-zinc-500 text-[13px]">📁</span>
            <span
              className="text-white text-[14px] flex-1 truncate font-mono"
              title={destDir}
            >
              {destDir || "Detectando…"}
            </span>
            <button
              onClick={onBrowseDest}
              disabled={busy}
              className="px-3 py-1.5 text-[12px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-lg transition-colors disabled:opacity-50"
            >
              Cambiar
            </button>
          </div>
        </div>

        {/* Progress (only when compressing) */}
        {busy && progress && (
          <div className="mb-10 p-6 rounded-2xl bg-cyan-500/[0.06] border border-cyan-500/20">
            <div className="flex items-center justify-between mb-3">
              <div className="text-cyan-300 text-[11px] tracking-[0.2em] uppercase">
                {progress.phase === "reading"
                  ? "Leyendo"
                  : progress.phase === "compressing"
                  ? "Comprimiendo"
                  : progress.phase === "writing"
                  ? "Escribiendo"
                  : "Procesando"}
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
              Resultado estimado ({mode})
            </div>
            <div className="grid grid-cols-3 gap-6">
              <Stat label="Ahorro esperado" value={`${(estSavings * 100).toFixed(0)}%`} />
              <Stat label="Velocidad" value={mode === "rapido" ? "rápida" : mode === "balanceado" ? "media" : "lenta"} />
              <Stat
                label="Ideal para"
                value={mode === "rapido" ? "vídeos" : mode === "balanceado" ? "general" : "archivos"}
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
          {busy ? `Comprimiendo… ${progressPct.toFixed(0)}%` : "Comprimir"}
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