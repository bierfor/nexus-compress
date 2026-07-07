"use client";

import { useCallback, useState } from "react";

/**
 * The Dropzone — the visual centerpiece of the app.
 *
 * Two ways for the user to feed input:
 *   1. Drag-and-drop a file or folder onto the dashed area
 *      (handled natively by Tauri on macOS via `tauri://drag-drop`)
 *   2. Click "PICK FILE" / "PICK FOLDER" to use the native dialog
 *
 * The actual handling lives in the parent (page.tsx) — it needs
 * access to the Tauri commands and the (mode, strength) state.
 * The Dropzone is a presentational + drag-presentation layer.
 */
export function Dropzone({
  onPickFile,
  onPickFolder,
  pickDisabled,
}: {
  onPickFile: () => void;
  onPickFolder: () => void;
  pickDisabled?: boolean;
}) {
  const [over, setOver] = useState(false);

  // We keep the HTML5 handlers for the browser dev fallback. Inside
  // Tauri they will rarely fire (the OS-level drop is handled by
  // Tauri's tauri://drag-drop event in page.tsx), but they don't
  // hurt.
  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setOver(true);
  }, []);
  const onDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setOver(false);
  }, []);

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      className={[
        "dropzone h-full min-h-[420px] flex flex-col items-center justify-center",
        "px-8 py-12 text-center select-none",
        over ? "dragover" : "",
      ].join(" ")}
    >
      <div className="text-cyan-500 text-7xl mb-4 font-mono">▣</div>
      <h2 className="text-cyan-400 text-xl font-mono tracking-[0.2em] uppercase mb-2">
        {over ? "release to compress" : "drop file or folder"}
      </h2>
      <p className="text-zinc-500 text-xs font-mono tracking-wider max-w-sm mb-6">
        Drag any file or directory onto this area, or use the buttons
        below to pick via the native macOS dialog.
      </p>
      <div className="flex gap-3">
        <button
          onClick={onPickFile}
          disabled={pickDisabled}
          className="btn border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          Pick file
        </button>
        <button
          onClick={onPickFolder}
          disabled={pickDisabled}
          className="btn border-amber-500 text-amber-400 hover:bg-amber-500/10 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          Pick folder
        </button>
      </div>
      {pickDisabled && (
        <div className="text-zinc-600 text-[10px] font-mono mt-3 uppercase tracking-widest">
          compress disabled — backend unavailable
        </div>
      )}
    </div>
  );
}
