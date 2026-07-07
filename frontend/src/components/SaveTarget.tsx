"use client";

/**
 * SaveTarget — the "where do I write the output" panel.
 *
 * Two modes:
 *   - "next-to-input" (default): output goes to <parent>/<name>.{ext}.
 *     The user knows where it ends up because it's next to the file.
 *   - "custom-dir": output goes to a chosen directory (via the
 *     native save dialog). The user clicks "Choose…" to set it.
 *
 * This is what WinRAR / 7-zip expose as the "Destination folder"
 * field. Without it, the app auto-saves and the user has to
 * remember where to find the output.
 */

import type { Mode } from "@/components/ConfigPanel";

function basename(p: string): string {
  return p.split("/").pop() || p;
}

function extFor(mode: Mode): string {
  if (mode === "v4") return "nxs";
  if (mode === "v5-min") return "lz";
  return "nxs6"; // v6-solid
}

export function defaultOutputFilename(
  selectedPath: string | null,
  mode: Mode,
  isDirectory: boolean,
): string {
  const ext = extFor(mode);
  if (!selectedPath) return `archive.${ext}`;
  const base = basename(selectedPath);
  if (isDirectory) {
    // For directories we name it after the directory + extension.
    return `${base}.${ext}`;
  }
  // For files: input.txt → input.txt.{ext}
  return `${base}.${ext}`;
}

export function SaveTarget({
  selectedPath,
  isDirectory,
  mode,
  outputDir,
  onPickFolder,
  onReset,
}: {
  selectedPath: string | null;
  isDirectory: boolean;
  mode: Mode | null;
  outputDir: string | null;
  onPickFolder: () => void;
  onReset: () => void;
}) {
  const suggested = defaultOutputFilename(
    selectedPath,
    (mode ?? "v6-solid") as Mode,
    isDirectory,
  );

  return (
    <div className="panel p-3 flex flex-col gap-2">
      <div className="metric-label">output location</div>
      <div className="font-mono text-[10px] tracking-wider">
        {outputDir ? (
          <>
            <span className="text-matrix-500">▸</span>{" "}
            <span className="text-zinc-300" title={outputDir}>
              {outputDir.split("/").slice(-2).join("/")}/<span className="text-cyan-400">{suggested}</span>
            </span>
          </>
        ) : (
          <>
            <span className="text-amber-400">▸</span>{" "}
            <span className="text-zinc-400">
              next to input ·/<span className="text-cyan-400">{suggested}</span>
            </span>
          </>
        )}
      </div>
      <div className="flex gap-2">
        <button
          onClick={onPickFolder}
          disabled={!selectedPath}
          className="btn flex-1 disabled:opacity-30 disabled:cursor-not-allowed"
          title="Choose a folder to save the output"
        >
          ⌖ choose folder
        </button>
        <button
          onClick={onReset}
          disabled={!outputDir}
          className="btn disabled:opacity-30 disabled:cursor-not-allowed"
          title="Revert to saving next to the input file"
        >
          ↺ default
        </button>
      </div>
    </div>
  );
}
