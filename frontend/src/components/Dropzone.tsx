"use client";

import { useCallback, useState } from "react";

/**
 * The Dropzone — the visual centerpiece of the app.
 *
 * Huge dashed-bordered area where the user drops files/folders.
 * Cyan pulse when something is dragged over. Industrial feel.
 *
 * The actual Tauri-native drag-drop handling is wired in the page
 * component (it needs access to the path-based compress flow).
 * This component is the visual layer + the HTML5 fallback.
 */
export function Dropzone({ onFile }: { onFile: (file: File) => void }) {
  const [over, setOver] = useState(false);

  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setOver(true);
  }, []);
  const onDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setOver(false);
  }, []);
  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setOver(false);
      const f = e.dataTransfer?.files?.[0];
      if (f) onFile(f);
    },
    [onFile]
  );

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={[
        "dropzone h-full min-h-[420px] flex flex-col items-center justify-center",
        "px-8 py-12 text-center select-none cursor-pointer",
        over ? "dragover" : "",
        over ? "animate-border-pulse" : "",
      ].join(" ")}
    >
      <div className="text-cyan-500 text-6xl mb-4 font-mono">▣</div>
      <h2 className="text-cyan-400 text-xl font-mono tracking-[0.2em] uppercase mb-2">
        {over ? "release to compress" : "drop file or folder"}
      </h2>
      <p className="text-zinc-500 text-xs font-mono tracking-wider max-w-sm">
        LZ77 lazy + multi-stream rANS + dict codec.
        <br />
        Or use the buttons above to pick via the native dialog.
      </p>
    </div>
  );
}
