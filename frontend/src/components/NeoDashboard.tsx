"use client";

/**
 * NeoDashboard — bottom panel showing the last completed
 * operation (Sprint 5.6 Neo Terminal).
 *
 * Single line, big numbers, soft colors. Designed for the
 * post-completion dopamine hit: "I just compressed 8.4 GB
 * to 4.2 GB in 8 seconds."
 */

export interface LastOp {
  filename: string;
  originalBytes: number;
  compressedBytes?: number; // for compress
  restoredBytes?: number; // for decompress
  durationMs: number;
  savings?: number; // ratio, e.g. 0.48
  kind: "compress" | "decompress" | "share";
  status: "ok" | "err";
}

export function NeoDashboard({ op }: { op: LastOp | null }) {
  if (!op) {
    return (
      <footer className="h-12 px-8 flex items-center justify-between border-t border-white/[0.04] backdrop-blur-md bg-black/20 text-[12px] text-zinc-600">
        <span>Aún no hay operaciones.</span>
        <span>Selecciona una acción para empezar.</span>
      </footer>
    );
  }

  const inputSize = op.originalBytes;
  const outputSize = op.compressedBytes ?? op.restoredBytes ?? 0;
  const saved = op.compressedBytes ? inputSize - outputSize : 0;
  const savings = saved > 0 ? saved / inputSize : 0;
  const durationSec = op.durationMs / 1000;

  return (
    <footer className="h-12 px-8 flex items-center justify-between border-t border-white/[0.04] backdrop-blur-md bg-black/20 text-[12px]">
      {/* Left: status dot + label */}
      <div className="flex items-center gap-3 text-zinc-400">
        <div
          className={`w-2 h-2 rounded-full ${
            op.status === "ok" ? "bg-emerald-400" : "bg-red-400"
          }`}
        />
        <span className="text-zinc-300 font-medium">{op.filename}</span>
        <span className="text-zinc-600">·</span>
        <span className="capitalize text-zinc-500">{op.kind === "share" ? "compartido" : op.kind === "compress" ? "comprimido" : "extraído"}</span>
      </div>

      {/* Right: stats */}
      <div className="flex items-center gap-5">
        <Stat label="tiempo" value={`${durationSec.toFixed(1)}s`} />
        {op.compressedBytes && (
          <Stat
            label="ahorro"
            value={`${(savings * 100).toFixed(0)}%`}
            highlight={savings > 0.3 ? "emerald" : savings > 0.1 ? "cyan" : "zinc"}
          />
        )}
        <Stat
          label={op.compressedBytes ? "original" : "tamaño"}
          value={prettyBytes(inputSize)}
        />
        {outputSize > 0 && (
          <>
            <span className="text-zinc-700">→</span>
            <Stat label="nuevo" value={prettyBytes(outputSize)} highlight="cyan" />
          </>
        )}
      </div>
    </footer>
  );
}

function Stat({
  label,
  value,
  highlight = "zinc",
}: {
  label: string;
  value: string;
  highlight?: "cyan" | "emerald" | "zinc";
}) {
  const colorClass =
    highlight === "cyan"
      ? "text-cyan-300"
      : highlight === "emerald"
      ? "text-emerald-300"
      : "text-zinc-200";
  return (
    <div className="flex items-center gap-1.5">
      <span className="text-zinc-600">{label}</span>
      <span className={`font-mono tabular-nums ${colorClass}`}>{value}</span>
    </div>
  );
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}