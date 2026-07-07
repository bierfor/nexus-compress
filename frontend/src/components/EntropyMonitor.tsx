"use client";

/**
 * The Entropy Monitor — the live telemetry panel.
 *
 * Reads the latest CompressResult (or null) and renders the three
 * key metrics: original size, compressed size, ratio + speed. The
 * ratio number is the hero — it glows cyan when above 1.0×, dim
 * gray when at 1.0× (random data).
 *
 * Future: this can subscribe to a Tauri event channel for per-block
 * progress. For now it's fed by the parent after each compress.
 */
export type Metrics = {
  originalSize?: number;
  compressedSize?: number;
  ratio?: number;
  compressMs?: number;
  decompressMs?: number;
  nFiles?: number;
  level?: "fast" | "premium";
} | null;

export function EntropyMonitor({ metrics }: { metrics: Metrics }) {
  if (!metrics) {
    return (
      <div className="panel p-4 h-full">
        <div className="metric-label">entropy monitor</div>
        <div className="text-zinc-600 text-xs font-mono mt-3">
          awaiting input…
        </div>
      </div>
    );
  }

  const ratio = metrics.ratio ?? 0;
  const glow = ratio > 1.05;

  return (
    <div className="panel p-4 h-full flex flex-col gap-3">
      <div className="metric-label">entropy monitor</div>

      <div className="flex flex-col gap-1">
        <div className="metric-label">ratio</div>
        <div
          className={[
            "metric-value text-4xl",
            glow ? "metric-value-glow" : "text-zinc-500",
          ].join(" ")}
        >
          {ratio > 0 ? `${ratio.toFixed(2)}×` : "—"}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3 mt-2">
        <div className="flex flex-col">
          <div className="metric-label">original</div>
          <div className="metric-value text-lg">
            {fmtBytes(metrics.originalSize)}
          </div>
        </div>
        <div className="flex flex-col">
          <div className="metric-label">compressed</div>
          <div className="metric-value text-lg">
            {fmtBytes(metrics.compressedSize)}
          </div>
        </div>
        <div className="flex flex-col">
          <div className="metric-label">compress</div>
          <div className="metric-value text-lg">
            {metrics.compressMs ? `${metrics.compressMs.toFixed(1)}ms` : "—"}
          </div>
        </div>
        <div className="flex flex-col">
          <div className="metric-label">decompress</div>
          <div className="metric-value text-lg">
            {metrics.decompressMs ? `${metrics.decompressMs.toFixed(1)}ms` : "—"}
          </div>
        </div>
      </div>

      {metrics.nFiles !== undefined && metrics.nFiles > 1 && (
        <div className="text-zinc-400 text-xs font-mono mt-1">
          {metrics.nFiles} files · {metrics.level} level
        </div>
      )}
    </div>
  );
}

function fmtBytes(n?: number): string {
  if (n === undefined) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(2)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
