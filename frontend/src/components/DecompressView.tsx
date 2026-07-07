"use client";

/**
 * DecompressView — dedicated screen for decompression (Sprint 5.6).
 *
 * User drops an archive → app detects format, shows what's inside,
 * user picks a destination, presses the big EXTRACT button.
 */

import { useEffect, useState, useCallback, useRef } from "react";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri) return {} as T;
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

interface ArchiveInfo {
  archive_kind: string;
  n_files: number;
  total_uncompressed: number;
  compressed_size: number;
  files: { path: string; size: number; is_dir: boolean }[];
}

interface DecompressResult {
  archive_kind: string;
  restored_size: number;
  n_files: number;
  output_path: string;
  decompress_time_ms: number;
}

export function DecompressView({
  onComplete,
}: {
  onComplete: (op: {
    kind: "decompress";
    filename: string;
    originalBytes: number;
    restoredBytes: number;
    durationMs: number;
  }) => void;
}) {
  const [archivePath, setArchivePath] = useState<string | null>(null);
  const [info, setInfo] = useState<ArchiveInfo | null>(null);
  const [destDir, setDestDir] = useState<string>("~/Downloads");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [pathInput, setPathInput] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Peek archive on path change
  useEffect(() => {
    if (!archivePath) {
      setInfo(null);
      return;
    }
    tauriInvoke<ArchiveInfo>("peek_archive_target_cmd", { path: archivePath })
      .then(setInfo)
      .catch((e) => {
        setError(String(e?.message ?? e));
        setInfo(null);
      });
  }, [archivePath]);

  const acceptPath = useCallback((path: string | null) => {
    if (path) {
      setArchivePath(path);
      setError(null);
      setPathInput("");
    }
  }, []);

  // Drag-drop
  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(true);
  }, []);
  const onDragLeave = useCallback(() => setDragOver(false), []);
  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const tauriPaths = (e as any).detail?.paths ?? null;
    if (tauriPaths?.length) acceptPath(tauriPaths[0]);
  }, [acceptPath]);

  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlisten = await eventMod.listen("tauri://drag-drop", (e: any) => {
          const paths: string[] = e?.payload?.paths ?? [];
          if (paths.length > 0) acceptPath(paths[0]);
        });
      } catch (e) {
        console.error(e);
      }
    })();
    return () => unlisten?.();
  }, [acceptPath]);

  const onBrowse = useCallback(() => fileInputRef.current?.click(), []);
  const onFileChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const f = (e.target.files ?? [])[0] as any;
    if (f) acceptPath(f.path || f.name);
  }, [acceptPath]);
  const onAddPath = useCallback(() => {
    const trimmed = pathInput.trim();
    if (trimmed) {
      acceptPath(trimmed);
    }
  }, [pathInput, acceptPath]);

  const onBrowseDest = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({ multiple: false, directory: true });
      if (typeof result === "string") setDestDir(result);
    } catch (e) {
      console.error(e);
    }
  }, []);

  const onExtract = useCallback(async () => {
    if (!archivePath) return;
    setBusy(true);
    setError(null);
    const startTime = Date.now();
    try {
      const r = await tauriInvoke<DecompressResult>("decompress_target_cmd", {
        req: {
          path: archivePath,
          output_dir: destDir === "~/Downloads" ? null : destDir,
        },
      });
      onComplete({
        kind: "decompress",
        filename: archivePath.split("/").pop() || "archive",
        originalBytes: info?.total_uncompressed ?? r.restored_size,
        restoredBytes: r.restored_size,
        durationMs: Date.now() - startTime,
      });
      setArchivePath(null);
      setInfo(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
    }
  }, [archivePath, destDir, onComplete]);

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`flex-1 overflow-y-auto transition-colors ${
        dragOver ? "bg-amber-500/[0.04]" : ""
      }`}
    >
      <div className="max-w-4xl mx-auto px-8 pt-12 pb-20">
        {/* Header */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
            ← Volver
          </div>
          <h1 className="text-white text-[36px] font-semibold tracking-tight mb-3">
            Descomprimir
          </h1>
          <p className="text-zinc-400 text-[14px] leading-relaxed max-w-2xl">
            Arrastra un archivo .nxs6 y la app te mostrará qué hay dentro
            antes de extraer nada.
          </p>
        </div>

        {/* Drop zone or archive info */}
        {!archivePath ? (
          <div
            className={`relative rounded-3xl border-2 border-dashed transition-all p-16 text-center mb-10 ${
              dragOver
                ? "border-amber-400 bg-amber-500/[0.08]"
                : "border-white/[0.08] bg-white/[0.02]"
            }`}
          >
            <div className="text-7xl mb-6 select-none">
              {dragOver ? "⤓" : "📂"}
            </div>
            <h3 className="text-white text-[20px] font-medium mb-2">
              {dragOver ? "Suelta el archivo" : "Arrastra un archivo .nxs6"}
            </h3>
            <p className="text-zinc-500 text-[13px] mb-6">
              o escribe la ruta absoluta
            </p>
            <input
              ref={fileInputRef}
              type="file"
              onChange={onFileChange}
              className="hidden"
            />
            <div className="flex items-center gap-2 max-w-xl mx-auto">
              <input
                type="text"
                value={pathInput}
                onChange={(e) => setPathInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && onAddPath()}
                placeholder="/Users/usuario/Downloads/archivo.nxs6"
                className="flex-1 bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-2.5 text-[13px] text-white placeholder-zinc-600 focus:outline-none focus:border-amber-500/50"
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
                className="px-4 py-2.5 text-[13px] text-amber-400 hover:text-amber-300 border border-amber-500/30 hover:border-amber-500/50 rounded-xl transition-colors disabled:opacity-30"
              >
                Cargar
              </button>
            </div>
          </div>
        ) : (
          <div className="mb-10 rounded-2xl bg-white/[0.03] border border-amber-500/20 overflow-hidden">
            <div className="px-6 py-5">
              <div className="text-amber-400 text-[11px] tracking-[0.2em] uppercase mb-3">
                Archivo detectado
              </div>
              <div className="text-white text-[20px] font-medium mb-1">
                {archivePath.split("/").pop()}
              </div>
              <div className="text-zinc-500 text-[12px] font-mono truncate mb-5">
                {archivePath}
              </div>

              {info ? (
                <div className="grid grid-cols-3 gap-5 pt-4 border-t border-white/[0.06]">
                  <DetailStat label="Formato" value={info.archive_kind} />
                  <DetailStat
                    label="Archivos"
                    value={`${info.n_files}`}
                  />
                  <DetailStat
                    label="Tamaño original"
                    value={prettyBytes(info.total_uncompressed)}
                  />
                </div>
              ) : (
                <div className="text-zinc-500 text-[13px] py-4">
                  Detectando contenido…
                </div>
              )}

              {info && info.files.length > 0 && (
                <details className="mt-5 pt-4 border-t border-white/[0.06]">
                  <summary className="text-zinc-400 text-[12px] cursor-pointer hover:text-white transition-colors">
                    Ver contenido ({info.files.length} archivos)
                  </summary>
                  <div className="mt-3 max-h-48 overflow-y-auto space-y-1">
                    {info.files.slice(0, 50).map((f, i) => (
                      <div
                        key={i}
                        className="text-zinc-500 text-[12px] flex items-center gap-2 font-mono"
                      >
                        <span className="text-zinc-700 w-6 text-right">
                          {f.is_dir ? "📁" : "📄"}
                        </span>
                        <span className="flex-1 truncate">{f.path}</span>
                        <span className="text-zinc-700 text-[10.5px]">
                          {prettyBytes(f.size)}
                        </span>
                      </div>
                    ))}
                    {info.files.length > 50 && (
                      <div className="text-zinc-600 text-[11px] pt-2">
                        … y {info.files.length - 50} más
                      </div>
                    )}
                  </div>
                </details>
              )}

              <button
                onClick={() => {
                  setArchivePath(null);
                  setInfo(null);
                }}
                className="mt-4 text-zinc-500 hover:text-red-400 text-[12px] transition-colors"
              >
                Cambiar archivo
              </button>
            </div>
          </div>
        )}

        {/* Destination */}
        {archivePath && (
          <div className="mb-10">
            <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
              Destino
            </div>
            <div className="flex items-center gap-3 px-5 py-4 rounded-2xl bg-white/[0.02] border border-white/[0.06]">
              <span className="text-zinc-500 text-[13px]">📁</span>
              <span className="text-white text-[14px] flex-1 truncate font-mono">
                {destDir}
              </span>
              <button
                onClick={onBrowseDest}
                className="px-3 py-1.5 text-[12px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-lg transition-colors"
              >
                Cambiar
              </button>
            </div>
          </div>
        )}

        {/* Extract button */}
        <button
          onClick={onExtract}
          disabled={!archivePath || busy}
          className="w-full py-4 rounded-2xl bg-gradient-to-b from-amber-500 to-amber-600 hover:from-amber-400 hover:to-amber-500 disabled:from-zinc-800 disabled:to-zinc-800 disabled:text-zinc-600 text-white text-[15px] font-semibold tracking-tight transition-all shadow-lg shadow-amber-500/20 disabled:shadow-none"
        >
          {busy ? "Extrayendo…" : "Descomprimir"}
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

function DetailStat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-zinc-500 text-[10.5px] uppercase tracking-wider mb-1">
        {label}
      </div>
      <div className="text-white text-[18px] font-medium tabular-nums">{value}</div>
    </div>
  );
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}