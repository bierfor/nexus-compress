"use client";

/**
 * ActionToolbar — the explicit action buttons.
 *
 * The app's primary verbs:
 *   - COMPRESS : run the chosen backend on `selectedPath`
 *   - DECOMPRESS: restore from the archive at `selectedPath`
 *                 (only enabled when the extension looks like an
 *                 archive we know: .nxs / .nxs6 / .lz)
 *   - OPEN     : reveal the last saved output in Finder/Explorer
 *                 (only enabled when `lastOutputPath` is set)
 *   - CLEAR    : reset all state (selectedPath, metrics, outputPath)
 */

const ARCHIVE_EXTS = ["nxs", "nxs6", "lz"];

export function looksLikeArchive(path: string): boolean {
  const ext = (path.split(".").pop() ?? "").toLowerCase();
  return ARCHIVE_EXTS.includes(ext);
}

export function ActionToolbar({
  selectedPath,
  lastOutputPath,
  working,
  onCompress,
  onDecompress,
  onOpen,
  onClear,
}: {
  selectedPath: string | null;
  lastOutputPath: string | null;
  working: boolean;
  onCompress: () => void;
  onDecompress: () => void;
  onOpen: () => void;
  onClear: () => void;
}) {
  const canCompress = !!selectedPath && !working;
  const canDecompress = !!selectedPath && looksLikeArchive(selectedPath) && !working;
  const canOpen = !!lastOutputPath && !working;

  return (
    <div className="panel p-3 flex flex-col gap-2">
      <div className="metric-label">actions</div>
      <div className="grid grid-cols-2 gap-2">
        <button
          onClick={onCompress}
          disabled={!canCompress}
          className="btn btn-primary disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent"
          title={canCompress ? "Compress selected path" : "Pick or drop a file/folder first"}
        >
          ▣ compress
        </button>
        <button
          onClick={onDecompress}
          disabled={!canDecompress}
          className="btn btn-magenta disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent"
          title={
            !selectedPath
              ? "Pick an archive first"
              : !looksLikeArchive(selectedPath)
              ? "Selected path doesn't look like a Nexus archive (.nxs/.nxs6/.lz)"
              : "Decompress archive"
          }
        >
          ▥ decompress
        </button>
        <button
          onClick={onOpen}
          disabled={!canOpen}
          className="btn disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent"
          title={canOpen ? "Reveal output in Finder" : "Run compress first"}
        >
          ⌖ open output
        </button>
        <button
          onClick={onClear}
          disabled={working}
          className="btn disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent"
          title="Reset state"
        >
          ✕ clear
        </button>
      </div>
    </div>
  );
}
