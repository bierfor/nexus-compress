"use client";

import { useLocale } from "@/components/LocaleProvider";
import { type View } from "@/components/NeoTopBar";

export interface LastOp {
  filename: string;
  originalBytes: number;
  compressedBytes?: number;
  restoredBytes?: number;
  durationMs: number;
  savings?: number;
  kind: "compress" | "decompress" | "share";
  status: "ok" | "err";
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function NeoDashboard({
  op,
  onNavigate,
}: {
  op: LastOp | null;
  onNavigate?: (v: View) => void;
}) {
  const { t } = useLocale();

  if (!op) {
    return (
      <footer className="h-11 px-6 flex items-center justify-between border-t border-white/[0.04] bg-black/20 backdrop-blur-md text-[11.5px] text-zinc-600 shrink-0">
        <span>{t("recent.empty")}</span>
        {onNavigate && (
          <button
            onClick={() => onNavigate("landing")}
            className="text-zinc-700 hover:text-zinc-400 transition-colors"
          >
            {t("landing.empty.cta")}
          </button>
        )}
      </footer>
    );
  }

  const inputSize = op.originalBytes;
  const outputSize = op.compressedBytes ?? op.restoredBytes ?? 0;
  const saved = op.compressedBytes ? inputSize - outputSize : 0;
  const savings = inputSize > 0 && saved > 0 ? saved / inputSize : 0;
  const durationSec = op.durationMs / 1000;

  const kindLabel =
    op.kind === "share"      ? t("recent.shared")     :
    op.kind === "compress"   ? t("recent.compressed")  :
                               t("recent.extracted");

  return (
    <footer className="h-11 px-6 flex items-center justify-between border-t border-white/[0.04] bg-black/20 backdrop-blur-md text-[11.5px] shrink-0">
      {/* Left — click to go to Recent */}
      <button
        onClick={() => onNavigate?.("recent")}
        className="flex items-center gap-2.5 text-zinc-400 hover:text-zinc-200 transition-colors min-w-0"
        title={t("nav.recent")}
      >
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${op.status === "ok" ? "bg-emerald-400" : "bg-red-400"}`} />
        <span className="text-zinc-200 font-medium truncate max-w-[220px]">{op.filename}</span>
        <span className="text-zinc-600">·</span>
        <span className="text-zinc-500 shrink-0">{kindLabel}</span>
      </button>

      {/* Right stats */}
      <div className="flex items-center gap-4 shrink-0">
        <Stat label="t" value={`${durationSec.toFixed(1)}s`} />
        {op.compressedBytes != null && savings > 0 && (
          <Stat
            label="−"
            value={`${(savings * 100).toFixed(0)}%`}
            color={savings > 0.3 ? "emerald" : savings > 0.1 ? "cyan" : "zinc"}
          />
        )}
        <Stat label="" value={prettyBytes(inputSize)} />
        {outputSize > 0 && outputSize !== inputSize && (
          <>
            <span className="text-zinc-700">→</span>
            <Stat label="" value={prettyBytes(outputSize)} color="cyan" />
          </>
        )}
      </div>
    </footer>
  );
}

function Stat({ label, value, color = "zinc" }: { label: string; value: string; color?: "cyan" | "emerald" | "zinc" }) {
  const cls = color === "cyan" ? "text-cyan-300" : color === "emerald" ? "text-emerald-300" : "text-zinc-200";
  return (
    <div className="flex items-center gap-1">
      {label && <span className="text-zinc-600">{label}</span>}
      <span className={`font-mono tabular-nums ${cls}`}>{value}</span>
    </div>
  );
}
