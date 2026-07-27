// Sprint 5.7.21-B-Cleanup: success state component for the
// Compress flow. Replaces the inline success card with a
// takeover view that shows JUST the result — no other
// configuration UI competing for attention.
//
// Design philosophy:
//   - Single accent color (emerald) for the "done" state
//   - Big ratio number as the hero (e.g. "4.66x")
//   - One primary CTA (Reveal in Finder) + one secondary
//     (Compress another)
//   - No gradients, no shadows, no extra decoration —
//     the user already saw the configuration UI; the
//     result is the only thing that matters now
//   - Skipped bytes / corpus breakdown as quiet secondary
//     info (smaller, dimmer, only if the heuristic
//     applies)

import { FolderOpen, RefreshCw, Check, Lock } from "lucide-react";
import { useLocale } from "./LocaleProvider";
import { useState } from "react";

export interface CompressSuccessInfo {
  inputFilename: string;
  outputPath: string;
  originalSize: number;
  compressedSize: number;
  durationMs: number;
  encrypted: boolean;
  skippedBytes?: number;
  corpusBreakdown?: {
    sourceBytes: number;
    sourceFiles: number;
    buildArtifactBytes: number;
    buildArtifactFiles: number;
    otherBytes: number;
    otherFiles: number;
  };
}

interface CompressSuccessProps {
  info: CompressSuccessInfo;
  /// True when the app is in Tauri context (so we can
  /// invoke reveal_in_finder_cmd). When false, the reveal
  /// button is hidden (browser preview).
  isTauri: boolean;
  /// Fired when the user clicks "Compress another" — the
  /// parent clears `lastSuccess` and we're done.
  onAnother: () => void;
  /// Fired when the user clicks the "Reveal in Finder" button.
  /// Returns the error string if reveal fails, or null on
  /// success. The parent surfaces that as a toast.
  onReveal: (path: string) => Promise<string | null>;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function CompressSuccess({ info, isTauri, onAnother, onReveal }: CompressSuccessProps) {
  const { t } = useLocale();
  const [revealing, setRevealing] = useState(false);
  const ratio =
    info.originalSize > 0
      ? info.originalSize / info.compressedSize
      : 1;
  const savingsPct = Math.round((1 - info.compressedSize / info.originalSize) * 100);
  const durationSec = (info.durationMs / 1000).toFixed(1);

  const handleReveal = async () => {
    setRevealing(true);
    await onReveal(info.outputPath);
    setRevealing(false);
  };

  // The corpus breakdown is shown only when the corpus is
  // >50% build artifacts (the heuristic that flags "your
  // ratio is low because most of the corpus is already
  // compressed binaries").
  const showBreakdown = (() => {
    if (!info.corpusBreakdown) return null;
    const b = info.corpusBreakdown;
    const total = b.sourceBytes + b.buildArtifactBytes + b.otherBytes;
    if (total === 0) return null;
    const buildPct = (b.buildArtifactBytes / total) * 100;
    if (buildPct < 50) return null;
    return { pct: buildPct, bytes: b.buildArtifactBytes };
  })();

  return (
    <div className="flex flex-col items-center text-center max-w-2xl mx-auto py-16">
      {/* Hero: just a check + ratio number, no decoration */}
      <div className="mb-8">
        <div className="inline-flex items-center justify-center w-14 h-14 rounded-full bg-emerald-500/15 text-emerald-300 mb-6">
          <Check size={28} strokeWidth={2.4} />
        </div>
        <div className="text-zinc-500 text-[11.5px] uppercase tracking-[0.2em] mb-2">
          {t("compress.success.title")} {info.encrypted && <Lock size={11} className="inline ml-1 align-baseline" />}
        </div>
        <div className="text-white text-[64px] font-semibold tracking-tight tabular-nums leading-none mb-3">
          {ratio.toFixed(2)}x
        </div>
        <div className="text-zinc-400 text-[14px]">
          {prettyBytes(info.originalSize)} → {prettyBytes(info.compressedSize)}{" "}
          <span className="text-emerald-400">({savingsPct}% {t("compress.success.smaller")})</span>
          {" · "}{durationSec}s
        </div>
      </div>

      {/* Output path (monospace, dimmer) */}
      <div className="text-zinc-500 text-[11.5px] font-mono mb-10 max-w-md break-all">
        {info.outputPath}
      </div>

      {/* Skipped bytes — only when the dev-cache filter
          dropped something. Quiet presentation. */}
      {info.skippedBytes && info.skippedBytes > 0 && (
        <div className="text-amber-300/80 text-[12px] mb-3 max-w-md">
          <span className="font-semibold">{t("compress.success.skipped.title")}</span>{" "}
          {t("compress.success.skipped.body").replace(
            "{bytes}",
            prettyBytes(info.skippedBytes)
          )}
        </div>
      )}

      {/* Corpus breakdown warning — only when the corpus is
          >50% build artifacts. Same color as the skipped
          bytes note. */}
      {showBreakdown && (
        <div className="text-amber-300/80 text-[12px] mb-3 max-w-md">
          <span className="font-semibold">
            {t("compress.success.breakdown.title").replace(
              "{pct}",
              showBreakdown.pct.toFixed(0)
            )}
          </span>{" "}
          {t("compress.success.breakdown.bytes").replace(
            "{bytes}",
            prettyBytes(showBreakdown.bytes)
          )}{" "}
          {t("compress.success.breakdown.explainer")}{" "}
          {t("compress.success.breakdown.suggest")}
        </div>
      )}

      {/* Primary + secondary actions. The two-button row
          is the only chrome on this view. */}
      <div className="flex items-center gap-3 mt-4">
        {isTauri && (
          <button
            onClick={handleReveal}
            disabled={revealing}
            className="px-5 py-2.5 rounded-xl bg-cyan-500/15 hover:bg-cyan-500/25 border border-cyan-500/30 text-cyan-300 text-[13px] font-medium transition-all flex items-center gap-2 disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
          >
            <FolderOpen size={14} />
            {t("compress.success.reveal")}
          </button>
        )}
        <button
          onClick={onAnother}
          className="px-5 py-2.5 rounded-xl bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.08] text-zinc-300 hover:text-white text-[13px] font-medium transition-all flex items-center gap-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
        >
          <RefreshCw size={14} />
          {t("compress.success.another")}
        </button>
      </div>
    </div>
  );
}
