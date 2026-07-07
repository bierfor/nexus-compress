"use client";

import { useCallback, useState } from "react";

/**
 * The Dropzone — the visual centerpiece of the app.
 *
 * Three ways for the user to feed input:
 *   1. Drag-and-drop a file or folder onto the dashed area
 *      (handled natively by Tauri on macOS via `tauri://drag-drop`)
 *   2. Click "📄 File" to pick a single file
 *   3. Click "📁 Folder" to pick a single folder
 *   4. Click "🗂️ Files" to pick multiple files at once
 *
 * Sprint 5.5.5: added multi-file picker + icons throughout.
 * The actual handling lives in the parent (page.tsx).
 */
export function Dropzone({
  onPickFile,
  onPickFolder,
  onPickFiles,
  pickDisabled,
}: {
  onPickFile: () => void;
  onPickFolder: () => void;
  onPickFiles?: () => void;
  pickDisabled?: boolean;
}) {
  const [over, setOver] = useState(false);

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
      <div className="text-cyan-500 text-7xl mb-4 font-mono">
        {over ? "⤓" : "▣"}
      </div>
      <h2 className="text-cyan-400 text-xl font-mono tracking-[0.2em] uppercase mb-2">
        {over ? "release to compress" : "drop file or folder"}
      </h2>
      <p className="text-zinc-500 text-xs font-mono tracking-wider max-w-sm mb-6">
        Drag any file or directory onto this area, or use the buttons
        below to pick via the native macOS dialog.
      </p>
      <div className="flex flex-wrap gap-3 justify-center">
        <button
          onClick={onPickFile}
          disabled={pickDisabled}
          className="btn border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          📄 file
        </button>
        <button
          onClick={onPickFolder}
          disabled={pickDisabled}
          className="btn border-amber-500 text-amber-400 hover:bg-amber-500/10 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          📁 folder
        </button>
        {onPickFiles && (
          <button
            onClick={onPickFiles}
            disabled={pickDisabled}
            className="btn border-matrix-500 text-matrix-400 hover:bg-matrix-500/10 disabled:opacity-40 disabled:cursor-not-allowed"
          >
            🗂️ multiple
          </button>
        )}
      </div>
      {pickDisabled && (
        <div className="text-zinc-600 text-[10px] font-mono mt-3 uppercase tracking-widest">
          ⚠ compress disabled — backend unavailable
        </div>
      )}
    </div>
  );
}