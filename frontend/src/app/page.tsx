"use client";

import { useState, useCallback } from "react";
import { Dropzone } from "@/components/Dropzone";
import { EntropyMonitor, type Metrics } from "@/components/EntropyMonitor";
import { ConfigPanel } from "@/components/ConfigPanel";

// Tauri APIs are only available inside the Tauri Webview. In the
// browser (dev mode outside Tauri) we stub them with console logs
// so the UI can be developed independently.
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri) {
    console.log(`[stub] invoke ${cmd}`, args);
    // Return shape-matched stubs so the UI doesn't crash during
    // browser-only development. Real Tauri calls happen in the
    // production build.
    if (cmd === "compress_bytes_cmd") {
      return {
        compressed: new Uint8Array([0x4e, 0x58, 0x53, 0x00]),
        original_size: 0,
        compressed_size: 0,
        ratio: 1.0,
        compress_time_ms: 0,
      } as T;
    }
    throw new Error(`Tauri not available; cmd=${cmd} not stubbed`);
  }
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

export default function Home() {
  const [metrics, setMetrics] = useState<Metrics>(null);
  const [level, setLevel] = useState<"fast" | "premium">("fast");
  const [status, setStatus] = useState<string>("idle");
  const [fileName, setFileName] = useState<string | null>(null);

  const onFile = useCallback(
    async (file: File) => {
      setStatus("working");
      setFileName(file.name);
      const bytes = new Uint8Array(await file.arrayBuffer());
      try {
        const t0 = performance.now();
        const result = await tauriInvoke<any>("compress_bytes_cmd", {
          input: Array.from(bytes),
        });
        const t1 = performance.now();
        setMetrics({
          originalSize: result.original_size ?? bytes.length,
          compressedSize: result.compressed_size,
          ratio: result.ratio,
          compressMs: result.compress_time_ms ?? t1 - t0,
          level,
        });
        setStatus("ok");
      } catch (e) {
        console.error(e);
        setStatus("err");
      }
    },
    [level]
  );

  const onSelfTest = useCallback(async () => {
    setStatus("working");
    try {
      const r = await tauriInvoke<any>("self_test_cmd");
      setMetrics({
        originalSize: 8 * 1024,
        compressedSize: Math.round((8 * 1024) / (r.ratio || 1)),
        ratio: r.ratio,
        compressMs: r.compress_time_ms,
        decompressMs: r.decompress_time_ms,
        level,
      });
      setStatus("ok");
    } catch (e) {
      console.error(e);
      setStatus("err");
    }
  }, [level]);

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
              status === "idle" ? "border-zinc-700 text-zinc-500" : "",
            ].join(" ")}
          >
            {status}
          </span>
        </div>
      </header>

      {/* Main 3-section grid: dropzone (center, big) | entropy monitor
          (right, telemetry) — and config panel below. */}
      <div className="flex-1 grid grid-cols-3 gap-3 p-3 min-h-0">
        <div className="col-span-2 min-h-0">
          <Dropzone onFile={onFile} />
        </div>
        <div className="col-span-1 min-h-0 flex flex-col gap-3">
          <div className="flex-1 min-h-0">
            <EntropyMonitor metrics={metrics} />
          </div>
          <ConfigPanel
            level={level}
            onLevelChange={setLevel}
            onSelfTest={onSelfTest}
          />
        </div>
      </div>

      {/* Status bar at the bottom — selected file name + ratio + level. */}
      <footer className="no-select h-7 border-t border-bg-border bg-bg-card flex items-center justify-between px-4 text-[10px] font-mono tracking-widest uppercase shrink-0">
        <div className="text-zinc-500">
          {fileName ? `· ${fileName}` : "· awaiting input"}
        </div>
        <div className="flex gap-3 text-zinc-600">
          <span>level: {level}</span>
          <span>format: v4</span>
          <span>dict: 5348</span>
        </div>
      </footer>
    </main>
  );
}
