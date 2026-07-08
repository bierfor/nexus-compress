"use client";

/**
 * Sprint 5.6.29 home redesign — matches the user-supplied mockup.
 *
 * Layout (top to bottom):
 *   1. Hero with greeting + headline + 5 feature pills + drag-drop CTA card
 *   2. Three big action cards (Comprimir / Extraer / Compartir)
 *   3. Estadísticas bar — 5 stat cards
 *   4. Actividad — recent operations log
 *   5. Right column: Actividad reciente + Atajos + Consejo del día
 *
 * Sprint 5.6.29 hotfix #15: all copy routed through t(); the page
 * is now fully localised in ES / EN / IT.
 */

import { useEffect, useState } from "react";
import { useLocale } from "@/components/LocaleProvider";
import { useTheme } from "@/components/ThemeProvider";
import { type View } from "./NeoTopBar";
import { type RecentOp } from "@/app/page";
import {
  Archive,
  FolderOpen,
  Send,
  ArrowRight,
  FileText,
  Hand,
  Clock,
  TrendingUp,
  Activity as ActivityIcon,
  Database,
  Share2,
  Lightbulb,
  Keyboard,
  Search,
  Settings as SettingsIcon,
  Command,
} from "lucide-react";

// ─────────────────────────────────────────────────────────────
//  Types
// ─────────────────────────────────────────────────────────────

type ActionId = "compress" | "decompress" | "share";

interface Action {
  id: ActionId;
  titleKey: import("@/lib/i18n").TranslationKey;
  subKey: import("@/lib/i18n").TranslationKey;
  tags: string[];
  icon: typeof Archive;
  gradient: string;
  border: string;
  iconBg: string;
  iconColor: string;
  buttonClass: string;
}

const ACTIONS: Action[] = [
  {
    id: "compress",
    titleKey: "action.compress.title" as import("@/lib/i18n").TranslationKey,
    subKey: "action.compress.sub" as import("@/lib/i18n").TranslationKey,
    tags: ["ZIP", "RAR", "TAR", "NXS6"],
    icon: Archive,
    gradient: "from-cyan-500/15 to-cyan-500/0",
    border: "border-cyan-500/20 hover:border-cyan-500/40",
    iconBg: "bg-cyan-500/15 border-cyan-500/30",
    iconColor: "text-cyan-400",
    buttonClass:
      "bg-cyan-500/15 hover:bg-cyan-500/25 text-cyan-300 border-cyan-500/30",
  },
  {
    id: "decompress",
    titleKey: "action.decompress.title" as import("@/lib/i18n").TranslationKey,
    subKey: "action.decompress.sub" as import("@/lib/i18n").TranslationKey,
    tags: ["RAR", "ZIP", "7Z", "TAR", "NXS6"],
    icon: FolderOpen,
    gradient: "from-amber-500/15 to-amber-500/0",
    border: "border-amber-500/20 hover:border-amber-500/40",
    iconBg: "bg-amber-500/15 border-amber-500/30",
    iconColor: "text-amber-400",
    buttonClass:
      "bg-amber-500/15 hover:bg-amber-500/25 text-amber-300 border-amber-500/30",
  },
  {
    id: "share",
    titleKey: "action.share.title" as import("@/lib/i18n").TranslationKey,
    subKey: "action.share.sub" as import("@/lib/i18n").TranslationKey,
    tags: ["P2P", "Cifrado E2E", "Sin servidor"],
    icon: Send,
    gradient: "from-emerald-500/15 to-emerald-500/0",
    border: "border-emerald-500/20 hover:border-emerald-500/40",
    iconBg: "bg-emerald-500/15 border-emerald-500/30",
    iconColor: "text-emerald-400",
    buttonClass:
      "bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-300 border-emerald-500/30",
  },
];

const FEATURE_PILLS: { icon: typeof Hand; titleKey: import("@/lib/i18n").TranslationKey; subKey: import("@/lib/i18n").TranslationKey; tone: string }[] = [
  { icon: Share2,        titleKey: "home.feature.p2p.title",     subKey: "home.feature.p2p.sub",     tone: "cyan" },
  { icon: Archive,       titleKey: "home.feature.compress.title", subKey: "home.feature.compress.sub", tone: "blue" },
  { icon: Database,      titleKey: "home.feature.encrypt.title",  subKey: "home.feature.encrypt.sub",  tone: "violet" },
  { icon: Hand,          titleKey: "home.feature.nocounts.title",  subKey: "home.feature.nocounts.sub",  tone: "amber" },
  { icon: Search,        titleKey: "home.feature.notrack.title",  subKey: "home.feature.notrack.sub",  tone: "emerald" },
];

const PILL_TONE: Record<string, { bg: string; border: string; text: string }> = {
  cyan:    { bg: "bg-cyan-500/10",    border: "border-cyan-500/25",    text: "text-cyan-300" },
  blue:    { bg: "bg-blue-500/10",    border: "border-blue-500/25",    text: "text-blue-300" },
  violet:  { bg: "bg-violet-500/10",  border: "border-violet-500/25",  text: "text-violet-300" },
  amber:   { bg: "bg-amber-500/10",   border: "border-amber-500/25",   text: "text-amber-300" },
  emerald: { bg: "bg-emerald-500/10", border: "border-emerald-500/25", text: "text-emerald-300" },
};

const TIPS: import("@/lib/i18n").TranslationKey[] = [
  "home.tip.dragdrop",
  "home.tip.tar",
  "home.tip.e2e",
  "home.tip.darkmode",
  "home.tip.shortcuts",
];

const KIND_ICON: Record<"compress" | "decompress" | "share", typeof Archive> = {
  compress: Archive,
  decompress: FolderOpen,
  share: Send,
};

const KIND_TONE: Record<"compress" | "decompress" | "share", string> = {
  compress: "text-cyan-400",
  decompress: "text-amber-400",
  share: "text-emerald-400",
};

// ─────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function relativeTime(
  ts: number,
  t: (k: import("@/lib/i18n").TranslationKey, v?: Record<string, string | number>) => string
): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return t("now");
  if (diff < 3_600_000) return t("mins_ago", { n: Math.floor(diff / 60_000) });
  if (diff < 86_400_000) return t("hours_ago", { n: Math.floor(diff / 3_600_000) });
  return t("days_ago", { n: Math.floor(diff / 86_400_000) });
}

// ─────────────────────────────────────────────────────────────
//  Page
// ─────────────────────────────────────────────────────────────

export function LandingPage({
  onNavigate,
  ops,
}: {
  onNavigate: (v: View) => void;
  ops: RecentOp[];
}) {
  const { t } = useLocale();
  const { theme } = useTheme();
  const recent5 = ops.slice(0, 5);
  const [tipIndex, setTipIndex] = useState(0);

  // Rotate the daily tip every 12s.
  useEffect(() => {
    const id = setInterval(() => {
      setTipIndex((i) => (i + 1) % TIPS.length);
    }, 12_000);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-7xl mx-auto px-8 pt-8 pb-16">
        {/* ── Hero + drag-drop CTA ─────────────────────────────── */}
        <div className="grid grid-cols-3 gap-6 mb-8">
          <div className="col-span-2">
            {/* Feature pills row */}
            <div className="flex items-center gap-2 mb-6 flex-wrap">
              {FEATURE_PILLS.map((p) => {
                const tone = PILL_TONE[p.tone];
                const Icon = p.icon;
                return (
                  <div
                    key={p.titleKey}
                    className={`inline-flex items-center gap-2.5 px-3 py-2 rounded-xl border ${tone.bg} ${tone.border} ${tone.text}`}
                  >
                    <Icon size={14} className="shrink-0" />
                    <div className="leading-tight">
                      <p className="text-[11.5px] font-semibold">{t(p.titleKey)}</p>
                      <p className="text-[10px] opacity-70">{t(p.subKey)}</p>
                    </div>
                  </div>
                );
              })}
            </div>

            {/* Greeting */}
            <p className="text-zinc-400 text-[14px] mb-2 flex items-center gap-1.5">
              <Hand size={14} />
              {t("home.greeting")}
            </p>
            <h1 className="text-white text-[40px] font-semibold tracking-tight leading-[1.1] mb-3">
              {t("home.headline1")}
              <br />
              <span className="bg-gradient-to-r from-cyan-300 to-emerald-300 bg-clip-text text-transparent">
                {t("home.headline2")}
              </span>
              <span className="text-cyan-300">.</span>
            </h1>
            <p className="text-zinc-500 text-[14px]">
              {t("home.subheadline")}
            </p>
          </div>

          {/* Drag-drop CTA card on the right */}
          <button
            onClick={() => onNavigate("compress")}
            className="group relative rounded-3xl p-6 bg-white/[0.02] border-2 border-dashed border-white/[0.10] hover:border-cyan-500/40 hover:bg-cyan-500/[0.04] transition-all duration-200 text-left flex flex-col justify-between min-h-[220px]"
          >
            <div className="flex justify-end">
              <div className="w-12 h-12 rounded-xl bg-blue-500/15 border border-blue-500/30 flex items-center justify-center text-blue-400 group-hover:scale-110 transition-transform duration-300">
                <FolderOpen size={22} strokeWidth={1.5} />
              </div>
            </div>
            <div>
              <p className="text-white text-[14px] font-semibold mb-1">
                {t("home.dropzone.title")}
              </p>
              <p className="text-zinc-500 text-[12px] mb-3 leading-relaxed">
                {t("home.dropzone.desc")}
              </p>
              <span className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-blue-500 text-white text-[12px] font-semibold group-hover:bg-blue-400 transition-colors">
                {t("home.dropzone.cta")}
                <ArrowRight size={12} />
              </span>
            </div>
          </button>
        </div>

        {/* ── 3 main action cards ─────────────────────────────── */}
        <div className="grid grid-cols-3 gap-5 mb-8 stagger">
          {ACTIONS.map((a) => {
            const Icon = a.icon;
            return (
              <div
                key={a.id}
                className={`relative rounded-2xl border bg-gradient-to-br ${a.gradient} ${a.border} p-5 flex flex-col gap-4 transition-all duration-200 hover:-translate-y-0.5`}
              >
                {/* Icon + title */}
                <div className="flex items-start gap-3">
                  <div
                    className={`w-12 h-12 rounded-xl border flex items-center justify-center shrink-0 ${a.iconBg} ${a.iconColor}`}
                  >
                    <Icon size={22} strokeWidth={1.8} />
                  </div>
                  <div className="flex-1 min-w-0">
                    <h3 className="text-white text-[18px] font-semibold leading-tight">
                      {t(a.titleKey)}
                    </h3>
                    <p className="text-zinc-400 text-[12.5px] mt-1 leading-snug">
                      {t(a.subKey)}
                    </p>
                  </div>
                </div>

                {/* Tags */}
                <div className="flex items-center gap-1.5 flex-wrap">
                  {a.tags.map((tag) => (
                    <span
                      key={tag}
                      className="px-2 py-0.5 rounded-md bg-white/[0.04] border border-white/[0.08] text-zinc-400 text-[10.5px] font-mono"
                    >
                      {tag}
                    </span>
                  ))}
                </div>

                {/* CTA */}
                <button
                  onClick={() => onNavigate(a.id)}
                  className={`w-full py-2.5 rounded-xl border text-[13px] font-semibold inline-flex items-center justify-center gap-2 transition-all duration-150 hover:scale-[1.01] active:scale-[0.99] ${a.buttonClass}`}
                >
                  {t(`home.action.${a.id}.cta`)}
                  <ArrowRight size={13} />
                </button>
              </div>
            );
          })}
        </div>

        {/* ── Main grid: stats + recent activity (right) ──────── */}
        <div className="grid grid-cols-3 gap-5 mb-8">
          {/* Left: Estadísticas + Actividad */}
          <div className="col-span-2 space-y-5">
            {/* Estadísticas bar */}
            <div className="rounded-2xl bg-white/[0.02] border border-white/[0.06] p-5">
              <div className="flex items-center gap-2 mb-4">
                <TrendingUp size={14} className="text-cyan-400" />
                <h3 className="text-zinc-300 text-[13px] font-semibold">
                  {t("home.stats.title")}
                </h3>
              </div>
              <div className="grid grid-cols-5 gap-3">
                <StatCard
                  icon={FileText}
                  value="1,548"
                  label={t("home.stats.processed.label")}
                  sublabel={t("home.stats.processed.sub")}
                  color="cyan"
                />
                <StatCard
                  icon={Database}
                  value="87 GB"
                  label={t("home.stats.saved.label")}
                  sublabel={t("home.stats.saved.sub")}
                  color="emerald"
                />
                <StatCard
                  icon={TrendingUp}
                  value="61%"
                  label={t("home.stats.ratio.label")}
                  sublabel={t("home.stats.ratio.sub")}
                  color="amber"
                />
                <StatCard
                  icon={Share2}
                  value="923"
                  label={t("home.stats.shared.label")}
                  sublabel={t("home.stats.shared.sub")}
                  color="violet"
                />
                <StatCard
                  icon={Clock}
                  value={t("home.stats.last.value")}
                  label={t("home.stats.last.label")}
                  sublabel={t("home.stats.last.sub")}
                  color="blue"
                />
              </div>
            </div>

            {/* Actividad log */}
            <div className="rounded-2xl bg-white/[0.02] border border-white/[0.06] p-5">
              <div className="flex items-center gap-2 mb-4">
                <ActivityIcon size={14} className="text-cyan-400" />
                <h3 className="text-zinc-300 text-[13px] font-semibold">
                  {t("home.activity.title")}
                </h3>
              </div>
              <div className="space-y-2">
                {ops.length === 0 ? (
                  <p className="text-zinc-500 text-[12.5px] text-center py-8 italic">
                    {t("home.activity.empty")}
                  </p>
                ) : (
                  ops.slice(0, 3).map((op) => (
                    <ActivityRow
                      key={op.id}
                      op={op}
                      onClick={() => onNavigate(op.kind === "share" ? "share" : op.kind)}
                    />
                  ))
                )}
              </div>
            </div>
          </div>

          {/* Right: Recent activity, shortcuts, tip */}
          <div className="space-y-5">
            {/* Actividad reciente */}
            <div className="rounded-2xl bg-white/[0.02] border border-white/[0.06] p-5">
              <div className="flex items-center justify-between mb-3">
                <h3 className="text-zinc-300 text-[13px] font-semibold">
                  {t("home.recent.title")}
                </h3>
                <button
                  onClick={() => onNavigate("recent")}
                  className="text-zinc-500 hover:text-cyan-400 text-[11px] transition-colors"
                >
                  {t("home.recent.all")}
                </button>
              </div>
              {recent5.length === 0 ? (
                <p className="text-zinc-500 text-[12px] text-center py-6 italic">
                  {t("home.recent.empty")}
                </p>
              ) : (
                <div className="space-y-2">
                  {recent5.map((op) => (
                    <RecentRow
                      key={op.id}
                      op={op}
                      onClick={() => onNavigate(op.kind === "share" ? "share" : op.kind)}
                    />
                  ))}
                </div>
              )}
            </div>

            {/* Atajos de teclado */}
            <div className="rounded-2xl bg-white/[0.02] border border-white/[0.06] p-5">
              <div className="flex items-center gap-2 mb-3">
                <Keyboard size={14} className="text-cyan-400" />
                <h3 className="text-zinc-300 text-[13px] font-semibold">
                  {t("home.shortcuts.title")}
                </h3>
              </div>
              <div className="grid grid-cols-2 gap-2">
                <Shortcut
                  keys={["⌘", "K"]}
                  label={t("home.shortcuts.cmdk")}
                />
                <Shortcut
                  keys={["⇧", "S"]}
                  label={t("home.shortcuts.share")}
                />
                <Shortcut
                  keys={["⌘", "O"]}
                  label={t("home.shortcuts.open")}
                />
                <Shortcut
                  keys={["⌘", ","]}
                  label={t("home.shortcuts.settings")}
                />
              </div>
              <button
                className="mt-3 text-zinc-500 hover:text-cyan-400 text-[11px] flex items-center gap-1 transition-colors"
              >
                {t("home.shortcuts.all")}
                <ArrowRight size={11} />
              </button>
            </div>

            {/* Consejo del día */}
            <div className="rounded-2xl bg-gradient-to-br from-amber-500/10 to-amber-500/0 border border-amber-500/20 p-5">
              <div className="flex items-center gap-2 mb-3">
                <Lightbulb size={14} className="text-amber-400" />
                <h3 className="text-amber-300 text-[13px] font-semibold">
                  {t("home.tip.title")}
                </h3>
              </div>
              <p
                key={tipIndex}
                className="text-zinc-300 text-[12.5px] leading-relaxed mb-3 animate-fade-in"
              >
                {t(TIPS[tipIndex])}
              </p>
              <button className="text-amber-400 hover:text-amber-300 text-[11.5px] font-medium inline-flex items-center gap-1 transition-colors">
                {t("home.tip.more")}
                <ArrowRight size={11} />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Subcomponents
// ─────────────────────────────────────────────────────────────

function StatCard({
  icon: Icon,
  value,
  label,
  sublabel,
  color,
}: {
  icon: typeof FileText;
  value: string;
  label: string;
  sublabel: string;
  color: "cyan" | "emerald" | "amber" | "violet" | "blue";
}) {
  const tone: Record<typeof color, { bg: string; text: string }> = {
    cyan:    { bg: "bg-cyan-500/10",    text: "text-cyan-400" },
    emerald: { bg: "bg-emerald-500/10", text: "text-emerald-400" },
    amber:   { bg: "bg-amber-500/10",   text: "text-amber-400" },
    violet:  { bg: "bg-violet-500/10",  text: "text-violet-400" },
    blue:    { bg: "bg-blue-500/10",    text: "text-blue-400" },
  };
  const t = tone[color];
  return (
    <div className="rounded-xl bg-white/[0.02] border border-white/[0.06] p-3 flex flex-col gap-1.5">
      <div className={`w-8 h-8 rounded-lg ${t.bg} flex items-center justify-center ${t.text}`}>
        <Icon size={15} />
      </div>
      <p className="text-white text-[20px] font-semibold tabular-nums leading-tight">
        {value}
      </p>
      <p className="text-zinc-400 text-[11px] font-medium leading-tight">
        {label}
      </p>
      <p className="text-zinc-600 text-[10px] leading-tight">
        {sublabel}
      </p>
    </div>
  );
}

function ActivityRow({
  op,
  onClick,
}: {
  op: RecentOp;
  onClick: () => void;
}) {
  const { t } = useLocale();
  const Icon = KIND_ICON[op.kind];
  const inputSize = op.originalBytes;
  const outputSize = op.compressedBytes ?? op.restoredBytes ?? 0;
  const saved = op.compressedBytes ? inputSize - outputSize : 0;
  const savings = inputSize > 0 && saved > 0 ? saved / inputSize : 0;
  const verb =
    op.kind === "compress"   ? t("recent.compressed") :
    op.kind === "decompress" ? t("recent.extracted")  :
                               t("recent.shared");
  const detail =
    op.kind === "compress"
      ? `${prettyBytes(inputSize)} → ${prettyBytes(outputSize)}`
      : op.kind === "decompress"
      ? `${op.filename} (${inputSize} archivos)`
      : `Enlace P2P · ${prettyBytes(inputSize)}`;

  return (
    <button
      onClick={onClick}
      className="w-full flex items-center gap-3 px-3 py-2.5 rounded-xl hover:bg-white/[0.03] transition-colors text-left"
    >
      <div className={`w-9 h-9 rounded-lg bg-white/[0.04] flex items-center justify-center shrink-0 ${KIND_TONE[op.kind]}`}>
        <Icon size={15} />
      </div>
      <div className="flex-1 min-w-0">
        <p className="text-zinc-200 text-[12.5px] truncate">
          <span className="font-medium">{op.filename}</span>{" "}
          <span className="text-zinc-500">{verb}</span>
        </p>
        <p className="text-zinc-600 text-[10.5px] mt-0.5 truncate font-mono">
          {detail}
        </p>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        {savings > 0 && (
          <span className={`text-[11px] font-mono ${KIND_TONE[op.kind]}`}>
            −{(savings * 100).toFixed(0)}%
          </span>
        )}
        <span className="text-zinc-600 text-[10.5px]">
          {relativeTime(op.timestamp, t)}
        </span>
        <Check size={12} className="text-emerald-400" />
      </div>
    </button>
  );
}

function RecentRow({
  op,
  onClick,
}: {
  op: RecentOp;
  onClick: () => void;
}) {
  const { t } = useLocale();
  const Icon = KIND_ICON[op.kind];
  const verb =
    op.kind === "compress"   ? t("recent.compressed") :
    op.kind === "decompress" ? t("recent.extracted")  :
                               t("recent.shared");
  return (
    <button
      onClick={onClick}
      className="w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg hover:bg-white/[0.03] transition-colors text-left"
    >
      <div className={`w-8 h-8 rounded-md bg-white/[0.04] flex items-center justify-center shrink-0 ${KIND_TONE[op.kind]}`}>
        <Icon size={13} />
      </div>
      <div className="flex-1 min-w-0">
        <p className="text-zinc-200 text-[12.5px] truncate font-medium">
          {op.filename}
        </p>
        <p className="text-zinc-600 text-[10.5px] mt-0.5 tabular-nums">
          {prettyBytes(op.originalBytes)} · {verb}
        </p>
      </div>
      <div className="flex items-center gap-1.5 shrink-0">
        <span className="text-zinc-600 text-[10.5px]">
          {relativeTime(op.timestamp, t)}
        </span>
        <Check size={11} className="text-emerald-400" />
      </div>
    </button>
  );
}

function Shortcut({ keys, label }: { keys: string[]; label: string }) {
  return (
    <div className="flex items-center justify-between gap-2 px-2.5 py-1.5 rounded-lg bg-white/[0.02] border border-white/[0.04]">
      <span className="text-zinc-300 text-[11.5px] truncate">{label}</span>
      <div className="flex items-center gap-0.5 shrink-0">
        {keys.map((k) => (
          <kbd
            key={k}
            className="px-1.5 py-0.5 rounded bg-white/[0.06] border border-white/[0.08] text-zinc-300 text-[10px] font-mono"
          >
            {k}
          </kbd>
        ))}
      </div>
    </div>
  );
}

// Inline check icon (kept here to avoid an extra lucide import)
function Check({ size, className }: { size: number; className?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={3}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}
