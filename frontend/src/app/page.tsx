"use client";

import { useState, useCallback, useEffect } from "react";
import { Dropzone } from "@/components/Dropzone";
import { ActionToolbar, looksLikeArchive } from "@/components/ActionToolbar";
import { ProgressBar, type Progress } from "@/components/ProgressBar";
import { SaveTarget, defaultOutputFilename } from "@/components/SaveTarget";
import { EntropyMonitor, type Metrics } from "@/components/EntropyMonitor";
import { ConfigPanel, type Mode, type Strength } from "@/components/ConfigPanel";

// Tauri APIs are only available inside the Tauri Webview. In the
// browser (dev mode outside Tauri) we stub them so the UI can be
// developed independently.
const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

interface CompressTargetRow {
  is_directory: boolean;
  original_size: number;
  compressed_size: number;
  ratio: number;
  compress_time_ms: number;
  n_files: number;
  output_path: string;
  output_ext: string;
}

interface DecompressTargetRow {
  archive_kind: string;
  restored_size: number;
  n_files: number;
  is_directory: boolean;
  output_path: string;
  decompress_time_ms: number;
}

async function tauriInvoke<T>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  if (!isTauri) {
    console.log(`[stub] invoke ${cmd}`, args);
    return {} as T;
  }
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

function resolveBackend(
  mode: Mode,
  strength: Strength
): { backend: string; lzma: number } {
  if (mode === "v4") {
    return { backend: "v4", lzma: 0 };
  }
  const lzma = strength === "fast" ? 1 : strength === "balanced" ? 6 : 9;
  return { backend: mode, lzma };
}

function basename(path: string): string {
  return path.split("/").pop() || path;
}

export default function Home() {
  const [metrics, setMetrics] = useState<Metrics>(null);
  const [mode, setMode] = useState<Mode>("v4");
  const [strength, setStrength] = useState<Strength>("balanced");
  const [status, setStatus] = useState<string>("idle");
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [selectedIsDir, setSelectedIsDir] = useState<boolean>(false);
  const [lastOutputPath, setLastOutputPath] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  // `saveDir` overrides the "next to input" default when set.
  const [saveDir, setSaveDir] = useState<string | null>(null);

  // ---- Compress (explicit, on user click) ----
  const onCompress = useCallback(async () => {
    if (!selectedPath) return;
    const { backend, lzma } = resolveBackend(mode, strength);
    setStatus("working");
    setErrorMsg(null);
    setProgress({
      phase: "reading",
      current_file: basename(selectedPath),
      files_done: 0,
      files_total: 1,
      bytes_done: 0,
      bytes_total: 0,
    });
    try {
      const r = await tauriInvoke<CompressTargetRow>(
        "compress_target_cmd",
        {
          req: {
            path: selectedPath,
            backend,
            lzma_level: lzma,
            output_dir: saveDir,
          },
        }
      );
      setMetrics({
        originalSize: r.original_size,
        compressedSize: r.compressed_size,
        ratio: r.ratio,
        compressMs: r.compress_time_ms,
        nFiles: r.n_files,
        mode,
        strength,
      });
      setLastOutputPath(r.output_path || null);
      setStatus(r.output_path ? "ok" : "saved-elsewhere");
    } catch (e: any) {
      console.error(e);
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    } finally {
      // Keep the "done" progress visible for a beat so the user
      // sees the 100% flash, then clear it.
      setTimeout(() => setProgress(null), 1500);
    }
  }, [selectedPath, saveDir, mode, strength]);

  // ---- Decompress ----
  const onDecompress = useCallback(async () => {
    if (!selectedPath) return;
    setStatus("working");
    setErrorMsg(null);
    setProgress({
      phase: "reading",
      current_file: basename(selectedPath),
      files_done: 0,
      files_total: 1,
      bytes_done: 0,
      bytes_total: 0,
    });
    try {
      const r = await tauriInvoke<DecompressTargetRow>(
        "decompress_target_cmd",
        { path: selectedPath }
      );
      setMetrics({
        originalSize: r.restored_size,
        compressedSize: r.restored_size,
        ratio: 1.0,
        compressMs: r.decompress_time_ms,
        nFiles: r.n_files,
        mode,
        strength,
      });
      setLastOutputPath(r.output_path);
      setStatus("ok");
    } catch (e: any) {
      console.error(e);
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    } finally {
      setTimeout(() => setProgress(null), 1500);
    }
  }, [selectedPath, mode, strength]);

  // ---- Open saved output in Finder ----
  const onOpen = useCallback(async () => {
    if (!lastOutputPath) return;
    try {
      await tauriInvoke<void>("reveal_in_finder_cmd", { path: lastOutputPath });
    } catch (e: any) {
      setErrorMsg(String(e?.message ?? e));
    }
  }, [lastOutputPath]);

  // ---- Reset state (CLEAR) ----
  const onClear = useCallback(() => {
    setSelectedPath(null);
    setSelectedIsDir(false);
    setLastOutputPath(null);
    setMetrics(null);
    setErrorMsg(null);
    setProgress(null);
    setSaveDir(null);
    setStatus("idle");
  }, []);

  // ---- Pick handlers ----
  const onPickFile = useCallback(async () => {
    try {
      const path = await tauriInvoke<string | null>("pick_file_cmd");
      if (path) {
        setSelectedPath(path);
        setSelectedIsDir(false);
      }
    } catch (e: any) {
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    }
  }, []);

  const onPickFolder = useCallback(async () => {
    try {
      const path = await tauriInvoke<string | null>("pick_directory_cmd");
      if (path) {
        setSelectedPath(path);
        setSelectedIsDir(true);
      }
    } catch (e: any) {
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    }
  }, []);

  // ---- Choose a save destination folder (native dialog) ----
  const onPickSaveFolder = useCallback(async () => {
    try {
      const suggested =
        defaultOutputFilename(selectedPath, mode, selectedIsDir) ||
        "archive.nxs6";
      const chosen = await tauriInvoke<string | null>(
        "pick_save_location_cmd",
        { req: { default_filename: suggested } }
      );
      if (!chosen) return; // user cancelled
      // The user chose a specific FILE. We extract the dir part and
      // use that as the save target.
      const lastSlash = chosen.lastIndexOf("/");
      const dir = lastSlash > 0 ? chosen.slice(0, lastSlash) : chosen;
      setSaveDir(dir);
      setErrorMsg(null);
    } catch (e: any) {
      setErrorMsg(String(e?.message ?? e));
    }
  }, [selectedPath, mode, selectedIsDir]);

  // ---- Reset save target to "next to input" ----
  const onResetSave = useCallback(() => {
    setSaveDir(null);
    setErrorMsg(null);
  }, []);

  // ---- Self-test ----
  const onSelfTest = useCallback(async () => {
    setStatus("working");
    setErrorMsg(null);
    try {
      const r = await tauriInvoke<any>("self_test_cmd");
      setMetrics({
        originalSize: 8 * 1024,
        compressedSize: Math.round((8 * 1024) / (r.ratio || 1)),
        ratio: r.ratio,
        compressMs: r.compress_time_ms,
        decompressMs: r.decompress_time_ms,
        mode,
        strength,
      });
      setStatus("ok");
    } catch (e: any) {
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    }
  }, [mode, strength]);

  // ---- Native Tauri drag-drop listener ----
  useEffect(() => {
    if (!isTauri) return;
    let unlistenDrop: (() => void) | undefined;
    let unlistenEnter: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlistenDrop = await eventMod.listen(
          "tauri://drag-drop",
          (e: any) => {
            const paths: string[] = e?.payload?.paths ?? [];
            if (paths.length > 0) {
              setSelectedPath(paths[0]);
              // Heuristic: if path has no extension at the end, it
              // might be a directory. The Rust side re-detects
              // anyway via metadata when COMPRESS is clicked.
              setSelectedIsDir(true);
            }
          }
        );
      } catch (e) {
        console.error("failed to attach tauri drag-drop listener:", e);
      }
    })();
    return () => {
      if (unlistenDrop) unlistenDrop();
      if (unlistenEnter) unlistenEnter();
    };
  }, []);

  // ---- Progress event listener from Rust ----
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlisten = await eventMod.listen(
          "compress-progress",
          (e: any) => {
            const p = e?.payload;
            if (!p) return;
            setProgress({
              phase: p.phase,
              current_file: p.current_file ?? "",
              files_done: Number(p.files_done ?? 0),
              files_total: Number(p.files_total ?? 1),
              bytes_done: Number(p.bytes_done ?? 0),
              bytes_total: Number(p.bytes_total ?? 0),
            });
          }
        );
      } catch (e) {
        console.error("failed to attach progress listener:", e);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const working = status === "working";
  const selectedName = selectedPath ? basename(selectedPath) : null;
  const outputName = lastOutputPath ? basename(lastOutputPath) : null;

  return (
    <main className="h-screen flex flex-col bg-bg-base">
      <header
        data-tauri-drag-region
        className="no-select h-9 flex items-center justify-between px-4 border-b border-bg-border bg-bg-card shrink-0"
      >
        <div className="flex items-center gap-2 text-cyan-400 font-mono text-xs tracking-[0.3em] uppercase">
          <span>▣</span>
          <span>NexusRAR</span>
          <span className="text-zinc-600">·</span>
          <span className="text-zinc-500">v0.1.0</span>
        </div>
        <div className="flex items-center gap-3 text-[10px] font-mono tracking-widest uppercase">
          <span
            className={[
              "px-2 py-0.5 border",
              status === "ok" ? "border-matrix-500 text-matrix-500" : "",
              status === "working" ? "border-cyan-500 text-cyan-400" : "",
              status === "err" ? "border-err text-err" : "",
              status === "saved-elsewhere" ? "border-amber-500 text-amber-400" : "",
              status === "idle" ? "border-zinc-700 text-zinc-500" : "",
            ].join(" ")}
          >
            {status}
          </span>
        </div>
      </header>

      <div className="flex-1 grid grid-cols-3 gap-3 p-3 min-h-0 overflow-y-auto">
        <div className="col-span-2 min-h-0 flex flex-col gap-3">
          <Dropzone
            onPickFile={onPickFile}
            onPickFolder={onPickFolder}
          />

          {/* Selection / output strip */}
          <div className="panel p-3 font-mono text-[10px] tracking-wider">
            <div className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1">
              <div className="metric-label">selected</div>
              <div className="text-zinc-300 truncate">
                {selectedPath ? (
                  <>
                    <span className="text-cyan-400">▸</span>{" "}
                    <span title={selectedPath}>{selectedName}</span>
                    {selectedIsDir && (
                      <span className="ml-2 text-[9px] px-1 border border-cyan-500 text-cyan-400">
                        DIR
                      </span>
                    )}
                    {looksLikeArchive(selectedPath ?? "") && (
                      <span className="ml-2 text-[9px] px-1 border border-magenta-500 text-magenta-400">
                        ARCHIVE
                      </span>
                    )}
                  </>
                ) : (
                  <span className="text-zinc-600">
                    (nothing selected — pick or drop)
                  </span>
                )}
              </div>
              <div className="metric-label">output</div>
              <div className="text-zinc-300 truncate">
                {lastOutputPath ? (
                  <>
                    <span className="text-matrix-500">▸</span>{" "}
                    <span title={lastOutputPath}>{outputName}</span>
                    <span className="ml-2 text-amber-400">⌖ click OPEN to reveal</span>
                  </>
                ) : (
                  <span className="text-zinc-600">
                    (no output yet — click COMPRESS or DECOMPRESS)
                  </span>
                )}
              </div>
            </div>
          </div>

          {/* Progress bar — only visible while compressing */}
          {progress && <ProgressBar progress={progress} />}
        </div>

        <div className="col-span-1 min-h-0 flex flex-col gap-3">
          <SaveTarget
            selectedPath={selectedPath}
            isDirectory={selectedIsDir}
            mode={mode}
            outputDir={saveDir}
            onPickFolder={onPickSaveFolder}
            onReset={onResetSave}
          />
          <ActionToolbar
            selectedPath={selectedPath}
            lastOutputPath={lastOutputPath}
            working={working}
            onCompress={onCompress}
            onDecompress={onDecompress}
            onOpen={onOpen}
            onClear={onClear}
          />
          <div className="flex-1 min-h-0">
            <EntropyMonitor metrics={metrics} />
          </div>
          <ConfigPanel
            mode={mode}
            strength={strength}
            onModeChange={setMode}
            onStrengthChange={setStrength}
            onSelfTest={onSelfTest}
          />
        </div>
      </div>

      <footer className="no-select h-7 border-t border-bg-border bg-bg-card flex items-center justify-between px-4 text-[10px] font-mono tracking-widest uppercase shrink-0 gap-4">
        <div className="text-zinc-500 truncate min-w-0">
          {lastOutputPath
            ? `· saved → ${lastOutputPath}`
            : selectedName
            ? `· selected: ${selectedName}`
            : "· awaiting input"}
        </div>
        <div className="flex gap-3 text-zinc-600 shrink-0">
          <span>mode: {mode}</span>
          <span>str: {strength}</span>
          <span>dict: 5348</span>
        </div>
      </footer>

      {errorMsg && (
        <div className="border-t border-err bg-err/10 px-4 py-1.5 text-[10px] font-mono text-err tracking-wider uppercase truncate">
          ⚠ {errorMsg}
        </div>
      )}
    </main>
  );
}
