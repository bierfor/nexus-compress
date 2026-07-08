"use client";

/**
 * ProgressBar — the WinRAR-style horizontal bar with a percentage.
 *
 * `progress` is the most recent ProgressEvent emitted by Rust.
 * `null` means "idle" and hides the bar entirely.
 */
export interface Progress {
  phase: "reading" | "preprocessing" | "compressing" | "done";
  current_file: string;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
}

export function ProgressBar({ progress }: { progress: Progress | null }) {
  if (!progress) return null;

  // Prefer bytes-based percentage (more accurate for mixed file
  // sizes); fall back to file count if both bytes are 0.
  const useBytes = progress.bytes_total > 0;
  const pct = useBytes
    ? Math.min(
        100,
        Math.round((progress.bytes_done / progress.bytes_total) * 100)
      )
    : progress.files_total > 0
    ? Math.min(
        100,
        Math.round((progress.files_done / progress.files_total) * 100)
      )
    : 0;

  // Color hint based on phase.
  const barColor =
    progress.phase === "done"
      ? "border-matrix-500 bg-matrix-500/20"
      : progress.phase === "reading"
      ? "border-amber-500 bg-amber-500/20"
      : "border-cyan-500 bg-cyan-500/20";
  const labelColor =
    progress.phase === "done"
      ? "text-matrix-500"
      : progress.phase === "reading"
      ? "text-amber-400"
      : "text-cyan-400";

  const phaseLabel: Record<Progress["phase"], string> = {
    reading: "reading",
    preprocessing: "preprocessing",
    compressing: "compressing",
    done: "done",
  };

  return (
    <div className="panel p-4 flex flex-col gap-2.5 animate-scale-in">
      <div className="flex items-center justify-between">
        <span className={`metric-label ${labelColor}`}>
          {phaseLabel[progress.phase]}
          {progress.files_total > 1
            ? ` · ${progress.files_done}/${progress.files_total} files`
            : ""}
          {progress.current_file
            ? ` · ${progress.current_file.length > 32
                ? "…" + progress.current_file.slice(-32)
                : progress.current_file}`
            : ""}
        </span>
        <span className={`font-mono text-sm tabular-nums font-semibold ${labelColor}`}>{pct}%</span>
      </div>
      <div className="relative h-2 rounded-full bg-white/[0.04] border border-white/[0.06] overflow-hidden">
        <div
          className={[
            "absolute inset-y-0 left-0 transition-all duration-200 ease-out",
            progress.phase === "done"
              ? "bg-emerald-500"
              : progress.phase === "reading"
              ? "bg-amber-500"
              : "bg-cyan-500",
          ].join(" ")}
          style={{
            width: `${pct}%`,
            // Sprint 5.6.29: subtle moving-stripe overlay while
            // compression is active. Static fill when done.
            backgroundImage:
              progress.phase === "done" || pct === 100
                ? "none"
                : "linear-gradient(45deg, rgba(255,255,255,0.18) 25%, transparent 25%, transparent 50%, rgba(255,255,255,0.18) 50%, rgba(255,255,255,0.18) 75%, transparent 75%)",
            backgroundSize: "20px 20px",
            animation:
              progress.phase === "done" || pct === 100
                ? "none"
                : "progress-stripes 0.8s linear infinite",
          }}
        />
        {/* Glow at the leading edge for cyberpunk feel */}
        {pct > 0 && pct < 100 && (
          <div
            className="absolute inset-y-0 w-4 pointer-events-none"
            style={{
              left: `calc(${pct}% - 16px)`,
              background: `linear-gradient(90deg, transparent, ${
                progress.phase === "reading" ? "rgba(251,191,36,0.5)" : "rgba(34,211,238,0.5)"
              }, transparent)`,
            }}
          />
        )}
      </div>
      <div className="flex items-center justify-between text-[10px] font-mono text-zinc-500 tracking-wider tabular-nums">
        <span>
          {fmtBytes(progress.bytes_done)} / {fmtBytes(progress.bytes_total)}
        </span>
        {progress.files_total > 1 && (
          <span>
            file {Math.min(progress.files_done + 1, progress.files_total)}/
            {progress.files_total}
          </span>
        )}
      </div>
    </div>
  );
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
