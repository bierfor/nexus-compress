"use client";

import { useLocale } from "@/components/LocaleProvider";
import { type View } from "./NeoTopBar";
import { type RecentOp } from "@/app/page";

const ACTIONS: {
  id: View;
  titleKey: "action.compress.title" | "action.decompress.title" | "action.share.title";
  subKey:   "action.compress.sub"   | "action.decompress.sub"   | "action.share.sub";
  icon: string;
  accent: string;
  dot: string;
  iconColor: string;
}[] = [
  {
    id: "compress",
    icon: "📦",
    titleKey: "action.compress.title",
    subKey:   "action.compress.sub",
    accent: "group-hover:bg-cyan-500/10 group-hover:border-cyan-500/30",
    dot: "bg-cyan-500",
    iconColor: "text-cyan-400",
  },
  {
    id: "decompress",
    icon: "📂",
    titleKey: "action.decompress.title",
    subKey:   "action.decompress.sub",
    accent: "group-hover:bg-amber-500/10 group-hover:border-amber-500/30",
    dot: "bg-amber-500",
    iconColor: "text-amber-400",
  },
  {
    id: "share",
    icon: "🚀",
    titleKey: "action.share.title",
    subKey:   "action.share.sub",
    accent: "group-hover:bg-emerald-500/10 group-hover:border-emerald-500/30",
    dot: "bg-emerald-500",
    iconColor: "text-emerald-400",
  },
];

const KIND_COLORS = {
  compress:   "text-cyan-400",
  decompress: "text-amber-400",
  share:      "text-emerald-400",
} as const;

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function LandingPage({
  onNavigate,
  recentOps = [],
}: {
  onNavigate: (v: View) => void;
  recentOps?: RecentOp[];
}) {
  const { t } = useLocale();
  const recent3 = recentOps.slice(0, 3);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-4xl mx-auto px-8 pt-10 pb-12">

        {/* Hero */}
        <div className="mb-10">
          <p className="text-zinc-500 text-[12px] tracking-[0.15em] uppercase mb-3">
            {t("landing.question")}
          </p>
          <h1 className="text-white text-[38px] font-semibold tracking-tight leading-[1.1] mb-3">
            {t("landing.headline1")}
            <br />
            <span className="bg-gradient-to-r from-cyan-300 to-emerald-300 bg-clip-text text-transparent">
              {t("landing.headline2")}
            </span>
          </h1>
          <p className="text-zinc-500 text-[14px] leading-relaxed max-w-xl">
            {t("landing.sub")}
          </p>
        </div>

        {/* 3 action cards */}
        <div className="grid grid-cols-3 gap-3 mb-10">
          {ACTIONS.map((a) => (
            <button
              key={a.id}
              onClick={() => onNavigate(a.id)}
              className={`group relative text-left p-5 rounded-2xl bg-white/[0.03] border border-white/[0.06] transition-all ${a.accent}`}
            >
              <div className={`text-2xl mb-3 ${a.iconColor}`}>{a.icon}</div>
              <h3 className="text-white text-[16px] font-semibold mb-1">
                {t(a.titleKey)}
              </h3>
              <p className="text-zinc-500 text-[12px] leading-snug">{t(a.subKey)}</p>
              <div className="mt-4 flex items-center gap-1.5 text-zinc-600 group-hover:text-zinc-300 transition-colors text-[12px]">
                <span>{t("landing.open")}</span>
                <span className="transition-transform group-hover:translate-x-0.5">→</span>
              </div>
              <div className={`absolute top-4 right-4 w-1.5 h-1.5 rounded-full ${a.dot} opacity-0 group-hover:opacity-100 transition-opacity`} />
            </button>
          ))}
        </div>

        {/* Recent */}
        {recent3.length > 0 && (
          <div>
            <div className="flex items-center justify-between mb-3">
              <p className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase">
                {t("landing.recent.title")}
              </p>
              <button
                onClick={() => onNavigate("recent")}
                className="text-zinc-600 hover:text-zinc-400 text-[11px] transition-colors"
              >
                {t("landing.recent.all")}
              </button>
            </div>

            <div className="rounded-2xl bg-white/[0.02] border border-white/[0.05] overflow-hidden divide-y divide-white/[0.04]">
              {recent3.map((op) => {
                const dest: View =
                  op.kind === "share" ? "share" :
                  op.kind === "compress" ? "compress" : "decompress";
                const inputSize = op.originalBytes;
                const outputSize = op.compressedBytes ?? op.restoredBytes ?? 0;
                const saved = op.compressedBytes ? inputSize - outputSize : 0;
                const savings = inputSize > 0 && saved > 0 ? saved / inputSize : 0;
                const kindLabel =
                  op.kind === "compress"   ? t("recent.compressed") :
                  op.kind === "decompress" ? t("recent.extracted")  :
                                             t("recent.shared");
                const relTime = relativeTime(op.timestamp, t);

                return (
                  <button
                    key={op.id}
                    onClick={() => onNavigate(dest)}
                    className="w-full px-5 py-3.5 flex items-center gap-4 hover:bg-white/[0.02] transition-colors text-left"
                  >
                    <span className="text-lg w-6 text-center shrink-0">
                      {op.kind === "compress" ? "📦" : op.kind === "decompress" ? "📂" : "🚀"}
                    </span>
                    <div className="flex-1 min-w-0">
                      <p className="text-zinc-200 text-[13px] font-medium truncate">{op.filename}</p>
                      <p className="text-zinc-600 text-[11px] mt-0.5">{relTime} · {kindLabel}</p>
                    </div>
                    <div className="text-right shrink-0">
                      {savings > 0 && (
                        <p className={`${KIND_COLORS[op.kind]} text-[12px] font-mono`}>
                          −{(savings * 100).toFixed(0)}%
                        </p>
                      )}
                      <p className="text-zinc-600 text-[11px] font-mono">{prettyBytes(inputSize)}</p>
                    </div>
                  </button>
                );
              })}
            </div>
          </div>
        )}

      </div>
    </div>
  );
}

function relativeTime(ts: number, t: (k: any, v?: any) => string): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return t("now");
  if (diff < 3_600_000) return t("mins_ago", { n: Math.floor(diff / 60_000) });
  if (diff < 86_400_000) return t("hours_ago", { n: Math.floor(diff / 3_600_000) });
  return t("days_ago", { n: Math.floor(diff / 86_400_000) });
}
