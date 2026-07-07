"use client";

import { useState, useCallback, useEffect } from "react";
import { Dropzone } from "@/components/Dropzone";
import { EntropyMonitor, type Metrics } from "@/components/EntropyMonitor";
import { ConfigPanel, type Mode, type Strength } from "@/components/ConfigPanel";

// Tauri APIs are only available inside the Tauri Webview. In the
// browser (dev mode outside Tauri) we stub them so the UI can be
// developed independently.
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

interface CompressTargetRow {
  is_directory: boolean;
  original_size: number;
  compressed_size: number;
  ratio: number;
  compress_time_ms: number;
  n_files: number;
  output_path: string;
  output_ext: string;
  // compressed_bytes: not surfaced here — the bytes are auto-saved
  // on the Rust side. We only need the stats.
}

async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri) {
    console.log(`[stub] invoke ${cmd}`, args);
    return {} as T;
  }
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

/// Map (mode, strength) to a Tauri-callable backend string + LZMA level.
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

export default function Home() {
  const [metrics, setMetrics] = useState<Metrics>(null);
  const [mode, setMode] = useState<Mode>("v4");
  const [strength, setStrength] = useState<Strength>("balanced");
  const [status, setStatus] = useState<string>("idle");
  const [fileName, setFileName] = useState<string | null>(null);
  const [outputPath, setOutputPath] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // ---- Compress a path (file OR directory) ----
  // Auto-detects file vs dir on the Rust side. The compressed bytes
  // are auto-saved next to the input so the user can verify.
  const onPath = useCallback(
    async (path: string) => {
      const { backend, lzma } = resolveBackend(mode, strength);
      const displayName = path.split("/").pop() || path;
      setStatus("working");
      setFileName(displayName);
      setOutputPath(null);
      setErrorMsg(null);
      try {
        const r = await tauriInvoke<CompressTargetRow>(
          "compress_target_cmd",
          { req: { path, backend, lzma_level: lzma } }
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
        setOutputPath(r.output_path || null);
        setStatus(r.output_path ? "ok" : "saved-elsewhere");
      } catch (e: any) {
        console.error(e);
        setErrorMsg(String(e?.message ?? e));
        setStatus("err");
      }
    },
    [mode, strength]
  );

  // ---- Pick handlers (Tauri native dialogs) ----
  const onPickFile = useCallback(async () => {
    try {
      const path = await tauriInvoke<string | null>("pick_file_cmd");
      if (path) await onPath(path);
    } catch (e: any) {
      console.error(e);
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    }
  }, [onPath]);

  const onPickFolder = useCallback(async () => {
    try {
      const path = await tauriInvoke<string | null>("pick_directory_cmd");
      if (path) await onPath(path);
    } catch (e: any) {
      console.error(e);
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    }
  }, [onPath]);

  // ---- Self-test (synthetic benchmark, no I/O) ----
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
      console.error(e);
      setErrorMsg(String(e?.message ?? e));
      setStatus("err");
    }
  }, [mode, strength]);

  // ---- Native Tauri drag-drop listener ----
  // The OS hands Tauri's webview a `tauri://drag-drop` event with
  // the array of file paths the user dropped. We pick the first
  // one and route it through onPath (which auto-detects file vs
  // folder).
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        // Tauri 2.x: with withGlobalTauri=true, events are at
        // window.__TAURI__.event.listen.
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) {
          console.warn("Tauri event API not available; drag-drop will only work via the pick buttons.");
          return;
        }
        unlisten = await eventMod.listen("tauri://drag-drop", (e: any) => {
          const paths: string[] = e?.payload?.paths ?? [];
          if (paths.length > 0) onPath(paths[0]);
        });
        // Optional: highlight the dropzone on drag-over by listening
        // to tauri://drag-enter / drag-leave.
        await eventMod.listen("tauri://drag-enter", () => {});
      } catch (e) {
        console.error("failed to attach tauri drag-drop listener:", e);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [onPath]);

  return (
    <main className="h-screen flex flex-col bg-bg-base">
      {/* Title bar — frameless window. The whole row is a drag region
          except for the buttons. The "buttons" (close, min, max) are
          emitted by Tauri as window controls via the titleBarStyle. */}
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

      {/* Main 3-section grid: dropzone (left, big) | entropy monitor + config
          (right column). */}
      <div className="flex-1 grid grid-cols-3 gap-3 p-3 min-h-0">
        <div className="col-span-2 min-h-0">
          <Dropzone
            onPickFile={onPickFile}
            onPickFolder={onPickFolder}
          />
        </div>
        <div className="col-span-1 min-h-0 flex flex-col gap-3 overflow-y-auto">
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

      {/* Status bar at the bottom — selected file name + output path
          + mode + strength + dict size. */}
      <footer className="no-select h-7 border-t border-bg-border bg-bg-card flex items-center justify-between px-4 text-[10px] font-mono tracking-widest uppercase shrink-0 gap-4">
        <div className="text-zinc-500 truncate min-w-0">
          {outputPath
            ? `· saved → ${outputPath}`
            : fileName
            ? `· ${fileName}`
            : "· awaiting input"}
        </div>
        <div className="flex gap-3 text-zinc-600 shrink-0">
          <span>mode: {mode}</span>
          <span>str: {strength}</span>
          <span>dict: 5348</span>
        </div>
      </footer>

      {/* Error toast at the very bottom (only when status=err). */}
      {errorMsg && (
        <div className="border-t border-err bg-err/10 px-4 py-1.5 text-[10px] font-mono text-err tracking-wider uppercase truncate">
          ⚠ {errorMsg}
        </div>
      )}
    </main>
  );
}
