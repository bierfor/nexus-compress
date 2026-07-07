"use client";

/**
 * DecompressView — Sprint 5.6.17 WinRAR-style browsing.
 *
 * Flow:
 *   1. User drops/picks an archive (.tar, .nxs6, .nxs, .lz, .nxar)
 *   2. App detects the format and reads the central directory
 *      (without extracting payload bytes).
 *   3. UI lists every entry with a checkbox. Default: all checked.
 *   4. User picks a destination folder.
 *   5. Click "Extract all" or "Extract N selected" → backend
 *      streams only the chosen entries from the archive.
 *
 * Two backend paths:
 *   - p2p_archive_list_cmd / p2p_archive_extract_cmd for .tar /
 *     .nxs6 (uses src-tauri/src/archive_inspect.rs).
 *   - peek_archive_target_cmd / decompress_target_cmd for
 *     legacy .lz / .nxar / .nxr (single-file, no central dir).
 */

import { useEffect, useState, useCallback, useRef } from "react";
import { type View } from "@/components/NeoTopBar";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri) return {} as T;
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

interface ArchiveEntry {
  name: string;
  size: number;
  is_dir: boolean;
}

interface ArchiveListResp {
  entries: ArchiveEntry[];
  total_files: number;
  total_bytes: number;
}

interface ArchiveInspectInfo {
  archive_kind: string; // "TAR" or legacy single-stream format name
  n_files: number;
  total_bytes: number;
  entries: ArchiveEntry[];
}

interface LegacyArchiveInfo {
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

interface ArchiveExtractResp {
  written: string[];
  count: number;
  total_bytes: number;
}

function isInspectable(name: string | null): "tar" | null {
  if (!name) return null;
  const lower = name.toLowerCase();
  // Only .tar has a central directory we can browse without
  // extracting. .nxs6/.nxs are V6Solid single-stream archives —
  // no TOC, so they fall through to the legacy decompress flow.
  if (lower.endsWith(".tar")) return "tar";
  return null;
}

export function DecompressView({
  onComplete,
  onNavigate,
}: {
  onComplete: (op: {
    kind: "decompress";
    filename: string;
    originalBytes: number;
    restoredBytes: number;
    durationMs: number;
  }) => void;
  onNavigate: (v: View) => void;
}) {
  const [archivePath, setArchivePath] = useState<string | null>(null);
  const [info, setInfo] = useState<ArchiveInspectInfo | null>(null);
  const [legacyInfo, setLegacyInfo] = useState<LegacyArchiveInfo | null>(null);
  const [destDir, setDestDir] = useState<string>("");
  const [destInitialized, setDestInitialized] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [pathInput, setPathInput] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const fileInputRef = useRef<HTMLInputElement>(null);

  const inspectable = isInspectable(archivePath);

  // Resolve homeDir on mount
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

  // Peek archive on path change
  useEffect(() => {
    if (!archivePath) {
      setInfo(null);
      setLegacyInfo(null);
      setSelected(new Set());
      return;
    }
    setError(null);
    setInfo(null);
    setLegacyInfo(null);
    setSelected(new Set());
    if (inspectable) {
      // New path: read central directory, no extraction.
      tauriInvoke<ArchiveListResp>("p2p_archive_list_cmd", {
        req: { path: archivePath },
      })
        .then((r) => {
          const inspect: ArchiveInspectInfo = {
            archive_kind: inspectable ? "TAR" : "Legacy single-stream",
            n_files: r.total_files,
            total_bytes: r.total_bytes,
            entries: r.entries,
          };
          setInfo(inspect);
          // Default: all files checked.
          const initial = new Set<string>();
          for (const e of r.entries) {
            if (!e.is_dir) initial.add(e.name);
          }
          setSelected(initial);
        })
        .catch((e) => {
          setError(String(e?.message ?? e));
        });
    } else {
      // Legacy path: peek for legacy formats (.lz, .nxar, .nxr).
      tauriInvoke<LegacyArchiveInfo>("peek_archive_target_cmd", {
        req: { path: archivePath },
      })
        .then(setLegacyInfo)
        .catch((e) => {
          setError(String(e?.message ?? e));
        });
    }
  }, [archivePath, inspectable]);

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
  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const tauriPaths = (e as any).detail?.paths ?? null;
      if (tauriPaths?.length) acceptPath(tauriPaths[0]);
    },
    [acceptPath],
  );

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

  // File picker — show all supported archive extensions.
  const onBrowse = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: false,
        directory: false,
        filters: [
          { name: "Archive (WinRAR-style browse)", extensions: ["tar"] },
          { name: "Nexus legacy", extensions: ["nxs", "nxs6", "lz", "nxar", "nxr"] },
          { name: "All files", extensions: ["*"] },
        ],
      });
      if (typeof result === "string") acceptPath(result);
    } catch (e) {
      console.error("file picker:", e);
    }
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

  const toggleEntry = useCallback((name: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }, []);

  const selectAll = useCallback(() => {
    if (!info) return;
    setSelected(new Set(info.entries.filter((e) => !e.is_dir).map((e) => e.name)));
  }, [info]);

  const selectNone = useCallback(() => setSelected(new Set()), []);

  // Extract the chosen entries (or all, for legacy formats).
  const onExtract = useCallback(async () => {
    if (!archivePath) return;
    setBusy(true);
    setError(null);
    const startTime = Date.now();
    try {
      if (inspectable && info) {
        // New selective path.
        const selectedArr = Array.from(selected);
        const r = await tauriInvoke<ArchiveExtractResp>(
          "p2p_archive_extract_cmd",
          {
            req: {
              path: archivePath,
              output_dir: destDir || null,
              selected: selectedArr.length === info.entries.filter((e) => !e.is_dir).length
                ? null
                : selectedArr,
            },
          },
        );
        onComplete({
          kind: "decompress",
          filename: archivePath.split("/").pop() || "archive",
          originalBytes: r.total_bytes,
          restoredBytes: r.total_bytes,
          durationMs: Date.now() - startTime,
        });
        setArchivePath(null);
        setInfo(null);
      } else {
        // Legacy single-file extraction.
        const r = await tauriInvoke<DecompressResult>("decompress_target_cmd", {
          req: {
            path: archivePath,
            output_dir: destDir || null,
          },
        });
        onComplete({
          kind: "decompress",
          filename: archivePath.split("/").pop() || "archive",
          originalBytes: legacyInfo?.total_uncompressed ?? r.restored_size,
          restoredBytes: r.restored_size,
          durationMs: Date.now() - startTime,
        });
        setArchivePath(null);
        setLegacyInfo(null);
      }
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
    }
  }, [archivePath, destDir, inspectable, info, selected, legacyInfo, onComplete]);

  const fileCount = info?.entries.filter((e) => !e.is_dir).length ?? 0;
  const selectedCount = selected.size;
  const allSelected = fileCount > 0 && selectedCount === fileCount;

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
            <button
              onClick={() => onNavigate("landing")}
              className="hover:text-zinc-300 transition-colors"
            >
              ← Volver
            </button>
          </div>
          <h1 className="text-white text-[36px] font-semibold tracking-tight mb-3">
            Descomprimir
          </h1>
          <p className="text-zinc-400 text-[14px] leading-relaxed max-w-2xl">
            Arrastra un archivo <code className="text-amber-300">.tar</code> — la app lee el
            directorio central sin descomprimir nada y te deja elegir qué
            archivos querés sacar (estilo WinRAR).
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
              {dragOver ? "Suelta el archivo" : "Arrastra un archivo .tar"}
            </h3>
            <p className="text-zinc-500 text-[13px] mb-6">
              o escribe la ruta absoluta
            </p>
            <div className="flex items-center gap-2 max-w-xl mx-auto">
              <input
                type="text"
                value={pathInput}
                onChange={(e) => setPathInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && onAddPath()}
                placeholder="/Users/usuario/Downloads/Counter-Strike 2.tar"
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
              <div className="flex items-start justify-between mb-3">
                <div>
                  <div className="text-amber-400 text-[11px] tracking-[0.2em] uppercase mb-2">
                    {inspectable ? `Directorio central · ${inspectable.toUpperCase()}` : "Archivo detectado"}
                  </div>
                  <div className="text-white text-[20px] font-medium mb-1">
                    {archivePath.split("/").pop()}
                  </div>
                  <div className="text-zinc-500 text-[12px] font-mono truncate">
                    {archivePath}
                  </div>
                </div>
                <button
                  onClick={() => {
                    setArchivePath(null);
                    setInfo(null);
                    setLegacyInfo(null);
                    setSelected(new Set());
                  }}
                  className="text-zinc-500 hover:text-red-400 text-[12px] transition-colors px-2 py-1"
                >
                  Cambiar
                </button>
              </div>

              {/* Stats */}
              {info ? (
                <div className="grid grid-cols-3 gap-5 pt-4 border-t border-white/[0.06]">
                  <DetailStat label="Formato" value={info.archive_kind} />
                  <DetailStat
                    label="Archivos"
                    value={`${info.n_files}`}
                  />
                  <DetailStat
                    label="Tamaño total"
                    value={prettyBytes(info.total_bytes)}
                  />
                </div>
              ) : legacyInfo ? (
                <div className="grid grid-cols-3 gap-5 pt-4 border-t border-white/[0.06]">
                  <DetailStat label="Formato" value={legacyInfo.archive_kind} />
                  <DetailStat
                    label="Archivos"
                    value={`${legacyInfo.n_files}`}
                  />
                  <DetailStat
                    label="Tamaño original"
                    value={prettyBytes(legacyInfo.total_uncompressed)}
                  />
                </div>
              ) : (
                <div className="text-zinc-500 text-[13px] py-4">
                  Leyendo directorio central…
                </div>
              )}

              {/* WinRAR-style entry list with checkboxes */}
              {info && info.entries.length > 0 && (
                <div className="mt-5 pt-4 border-t border-white/[0.06]">
                  <div className="flex items-center justify-between mb-3">
                    <div className="text-zinc-400 text-[12px]">
                      {selectedCount} de {fileCount} archivos seleccionados
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        onClick={selectAll}
                        disabled={allSelected}
                        className="text-[11px] px-2 py-1 text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-md transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                      >
                        Todos
                      </button>
                      <button
                        onClick={selectNone}
                        disabled={selectedCount === 0}
                        className="text-[11px] px-2 py-1 text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-md transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                      >
                        Ninguno
                      </button>
                    </div>
                  </div>
                  <div className="max-h-80 overflow-y-auto rounded-xl bg-black/30 border border-white/[0.04] divide-y divide-white/[0.04]">
                    {info.entries.map((entry, i) => {
                      if (entry.is_dir) {
                        return (
                          <div
                            key={i}
                            className="text-zinc-500 text-[12px] flex items-center gap-3 px-4 py-2 font-mono"
                          >
                            <span className="w-4">📁</span>
                            <span className="flex-1 truncate">{entry.name}</span>
                            <span className="text-zinc-700 text-[10.5px]">dir</span>
                          </div>
                        );
                      }
                      const checked = selected.has(entry.name);
                      return (
                        <label
                          key={i}
                          className="text-zinc-300 text-[12px] flex items-center gap-3 px-4 py-1.5 hover:bg-white/[0.02] cursor-pointer font-mono"
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() => toggleEntry(entry.name)}
                            className="accent-amber-500 w-4 h-4 flex-shrink-0"
                          />
                          <span className="w-4 flex-shrink-0">📄</span>
                          <span className="flex-1 truncate">{entry.name}</span>
                          <span className="text-zinc-500 text-[10.5px] flex-shrink-0 tabular-nums">
                            {prettyBytes(entry.size)}
                          </span>
                        </label>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Legacy: just list, no selection */}
              {legacyInfo && legacyInfo.files.length > 0 && (
                <details className="mt-5 pt-4 border-t border-white/[0.06]">
                  <summary className="text-zinc-400 text-[12px] cursor-pointer hover:text-white transition-colors">
                    Ver contenido ({legacyInfo.files.length} archivos)
                  </summary>
                  <div className="mt-3 max-h-48 overflow-y-auto space-y-1">
                    {legacyInfo.files.slice(0, 50).map((f, i) => (
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
                    {legacyInfo.files.length > 50 && (
                      <div className="text-zinc-600 text-[11px] pt-2">
                        … y {legacyInfo.files.length - 50} más
                      </div>
                    )}
                  </div>
                </details>
              )}
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
                {destDir || "Detectando…"}
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
          disabled={
            archivePath == null ||
            busy ||
            (!!inspectable && selectedCount === 0)
          }
          className="w-full py-4 rounded-2xl bg-gradient-to-b from-amber-500 to-amber-600 hover:from-amber-400 hover:to-amber-500 disabled:from-zinc-800 disabled:to-zinc-800 disabled:text-zinc-600 text-white text-[15px] font-semibold tracking-tight transition-all shadow-lg shadow-amber-500/20 disabled:shadow-none"
        >
          {busy
            ? "Extrayendo…"
            : inspectable && info
              ? allSelected
                ? "Extraer todo"
                : `Extraer ${selectedCount} de ${fileCount}`
              : "Descomprimir"}
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