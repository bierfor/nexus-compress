/**
 * SettingsView — Sprint 5.7 rewrite.
 *
 * Replaces the previous inline implementation in page.tsx (which
 * was a PageHeader + a duplicate language switcher + the full
 * ConfigPanel, all stacked vertically in a single column).
 *
 * New layout: 12-col grid with a sticky **sidebar** of section
 * tabs on the left (Interfaz / Compresión / Transporte / Almacenamiento /
 * Acerca de) and the **active section** on the right. The sidebar
 * makes every section one click away regardless of vertical scroll
 * position, which solves the "scrolled past the thing I want to
 * change" pain the old layout had.
 *
 * Sprint 5.7 wiring:
 *   - data_dir_cmd()       → real absolute path of the SQLite store
 *   - get_stats_cmd()      → real counter totals in the Storage card
 *   - reset_stats_cmd()    → "Borrar estadísticas" wipes events + counters
 *   - reveal_in_finder_cmd → opens the platform file manager
 *   - localStorage (theme) → already wired in ThemeProvider.tsx
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLocale } from "@/components/LocaleProvider";
import { useTheme } from "@/components/ThemeProvider";
import {
  Settings as SettingsIcon,
  Languages,
  Zap,
  Network,
  Database,
  Info,
  ArrowRight,
  ChevronRight,
  FolderOpen,
  CheckCircle2,
  AlertTriangle,
  Trash2,
  Github,
  Globe,
  Shield,
  HardDrive,
  Sun,
  Moon,
  
  Sparkles,
} from "lucide-react";

import type { Locale } from "@/lib/i18n";
import { CompressionPanel, TunnelPanel } from "@/components/ConfigPanel";

type SectionId =
  | "interface"
  | "compression"
  | "transport"
  | "storage"
  | "about";

interface SectionDef {
  id: SectionId;
  icon: typeof SettingsIcon;
  titleKey: string;
  descKey: string;
}

const SECTIONS: SectionDef[] = [
  {
    id: "interface",
    icon: Languages,
    titleKey: "settings.nav.interface",
    descKey: "settings.interface.desc",
  },
  {
    id: "compression",
    icon: Zap,
    titleKey: "settings.nav.compression",
    descKey: "settings.compression.desc",
  },
  {
    id: "transport",
    icon: Network,
    titleKey: "settings.nav.transport",
    descKey: "settings.transport.desc",
  },
  {
    id: "storage",
    icon: Database,
    titleKey: "settings.nav.storage",
    descKey: "settings.storage.desc",
  },
  {
    id: "about",
    icon: Info,
    titleKey: "settings.nav.about",
    descKey: "settings.about.desc",
  },
];

// ─────────────────────────────────────────────────────────────
//  Root
// ─────────────────────────────────────────────────────────────

export function SettingsView() {
  const { t } = useLocale();
  const [active, setActive] = useState<SectionId>("interface");

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-6xl mx-auto px-8 pt-10 pb-20">
        {/* Page header */}
        <div className="mb-10 flex items-start justify-between gap-6">
          <div>
            <h1 className="text-white text-[32px] font-semibold tracking-tight leading-tight">
              {t("settings.title")}
            </h1>
            <p className="text-zinc-500 text-[14px] mt-1.5">
              {t("settings.desc")}
            </p>
          </div>
        </div>

        {/* 12-col grid: sidebar (3) + active panel (9) */}
        <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
          <SectionNav active={active} onSelect={setActive} />
          <div className="lg:col-span-9">
            {active === "interface" && <InterfaceSection />}
            {active === "compression" && <CompressionSection />}
            {active === "transport" && <TransportSection />}
            {active === "storage" && <StorageSection />}
            {active === "about" && <AboutSection />}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Sidebar nav
// ─────────────────────────────────────────────────────────────

function SectionNav({
  active,
  onSelect,
}: {
  active: SectionId;
  onSelect: (s: SectionId) => void;
}) {
  const { t } = useLocale();
  return (
    <nav className="lg:col-span-3 lg:sticky lg:top-6 self-start space-y-1.5">
      {SECTIONS.map((s) => {
        const isActive = s.id === active;
        const Icon = s.icon;
        return (
          <button
            key={s.id}
            onClick={() => onSelect(s.id)}
            className={
              "w-full text-left rounded-2xl border transition-all px-4 py-3 flex items-start gap-3 group " +
              (isActive
                ? "bg-cyan-500/[0.08] border-cyan-500/30 shadow-[inset_0_0_0_1px_rgba(34,211,238,0.08)]"
                : "bg-white/[0.02] border-white/[0.06] hover:bg-white/[0.04] hover:border-white/[0.10]")
            }
          >
            <div
              className={
                "w-9 h-9 rounded-lg flex items-center justify-center shrink-0 border " +
                (isActive
                  ? "bg-cyan-500/15 border-cyan-500/40 text-cyan-300"
                  : "bg-white/[0.03] border-white/[0.06] text-zinc-500 group-hover:text-zinc-300")
              }
            >
              <Icon size={15} />
            </div>
            <div className="flex-1 min-w-0">
              <p
                className={
                  "text-[13px] font-semibold leading-tight " +
                  (isActive ? "text-white" : "text-zinc-200 group-hover:text-white")
                }
              >
                {t(s.titleKey as any)}
              </p>
              <p className="text-zinc-500 text-[11px] leading-snug mt-1">
                {t(s.descKey as any)}
              </p>
            </div>
            <ChevronRight
              size={14}
              className={
                "shrink-0 mt-2 " +
                (isActive
                  ? "text-cyan-400"
                  : "text-zinc-700 group-hover:text-zinc-500")
              }
            />
          </button>
        );
      })}
    </nav>
  );
}

// ─────────────────────────────────────────────────────────────
//  Section: Interfaz
// ─────────────────────────────────────────────────────────────

function InterfaceSection() {
  const { t, locale, setLocale } = useLocale();
  const { theme, setTheme } = useTheme();

  const LANGS: { id: Locale; flag: string; label: string }[] = [
    { id: "es", flag: "🇪🇸", label: t("settings.language.es") },
    { id: "en", flag: "🇬🇧", label: t("settings.language.en") },
    { id: "it", flag: "🇮🇹", label: t("settings.language.it") },
  ];

  const THEMES: { id: typeof theme; icon: typeof Sun; label: string }[] = [
    { id: "dark", icon: Moon, label: t("settings.appearance.theme.dark") },
    { id: "light", icon: Sun, label: t("settings.appearance.theme.light") },
  ];

  return (
    <div className="space-y-5">
      <SectionCard
        icon={Languages}
        title={t("settings.section.language")}
        desc={t("settings.interface.desc")}
      >
        <div className="p-5 flex flex-wrap gap-3">
          {LANGS.map((l) => {
            const active = locale === l.id;
            return (
              <button
                key={l.id}
                onClick={() => setLocale(l.id)}
                className={
                  "min-w-[140px] px-5 py-3 rounded-2xl border text-left transition-all " +
                  (active
                    ? "bg-cyan-500/15 border-cyan-500/40 text-white shadow-[inset_0_0_0_1px_rgba(34,211,238,0.08)]"
                    : "bg-white/[0.02] border-white/[0.06] text-zinc-400 hover:text-white hover:border-white/[0.14] hover:bg-white/[0.04]")
                }
              >
                <div className="text-xl mb-1">{l.flag}</div>
                <p className="font-semibold text-[13px]">{l.label}</p>
              </button>
            );
          })}
        </div>
      </SectionCard>

      <SectionCard
        icon={Sparkles}
        title={t("settings.section.appearance")}
        desc={t("settings.appearance.theme")}
      >
        <div className="p-5 grid grid-cols-3 gap-3">
          {THEMES.map((th) => {
            const active = theme === th.id;
            const Icon = th.icon;
            return (
              <button
                key={th.id}
                onClick={() => setTheme(th.id)}
                className={
                  "px-5 py-4 rounded-2xl border transition-all flex flex-col items-center gap-2 " +
                  (active
                    ? "bg-cyan-500/10 border-cyan-500/40 text-cyan-300"
                    : "bg-white/[0.02] border-white/[0.06] text-zinc-400 hover:text-white hover:border-white/[0.14]")
                }
              >
                <Icon size={18} />
                <p className="text-[12.5px] font-medium">{th.label}</p>
              </button>
            );
          })}
        </div>
      </SectionCard>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Section: Compresión
// ─────────────────────────────────────────────────────────────

function CompressionSection() {
  const { t } = useLocale();
  return (
    <SectionCard
      icon={Zap}
      title={t("settings.section.compression")}
      desc={t("settings.compression.desc")}
      wide
    >
      <div className="p-5">
        {/* Just the compression piece — mode cards + strength + self-test.
            No language picker, no tunnel config, no about — those have
            their own dedicated sidebar tabs. */}
        <CompressionPanel />
      </div>
    </SectionCard>
  );
}

// ─────────────────────────────────────────────────────────────
//  Section: Transporte P2P — informational now (real config
//  still lives inside ConfigPanel.TunnelPanel). The sidebar entry
//  exists so the user can find the config; in v0.1.2 we'll split
//  TunnelPanel out so each section has its own panel.
// ─────────────────────────────────────────────────────────────

function TransportSection() {
  const { t } = useLocale();
  return (
    <SectionCard
      icon={Network}
      title={t("settings.section.tunnel")}
      desc={t("settings.transport.desc")}
      wide
    >
      {/* Mount the real TunnelPanel here — mode selector +
          named-mode hostname + token + Save. Same state as the
          legacy ConfigPanel-composed version, but reachable from
          its own sidebar tab instead of being buried in the
          composite. */}
      <TunnelPanel />
    </SectionCard>
  );
}

// ─────────────────────────────────────────────────────────────
//  Section: Almacenamiento
// ─────────────────────────────────────────────────────────────

function StorageSection() {
  const { t, locale } = useLocale();
  const [path, setPath] = useState<string>("");
  const [pathErr, setPathErr] = useState(false);
  const [revealing, setRevealing] = useState(false);
  const [resetPending, setResetPending] = useState(false);
  const [resetMsg, setResetMsg] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    invoke<string>("data_dir_cmd")
      .then((p) => {
        setPath(p);
        setPathErr(false);
      })
      .catch(() => {
        setPathErr(true);
      });
  }, []);

  const onOpenFolder = async () => {
    if (!path || pathErr) return;
    setRevealing(true);
    try {
      await invoke("reveal_in_finder_cmd", { path });
    } catch {
      /* fallback: nothing to do */
    }
    setTimeout(() => setRevealing(false), 600);
  };

  const onReset = async () => {
    setResetMsg(null);
    try {
      await invoke("reset_stats_cmd");
      setResetMsg({ ok: true, text: t("settings.storage.actions.reset.ok") });
    } catch (e: any) {
      setResetMsg({ ok: false, text: String(e?.message ?? e) });
    } finally {
      setResetPending(false);
      // Auto-clear the success banner after 3s so it doesn't linger.
      setTimeout(() => setResetMsg(null), 3000);
    }
  };

  return (
    <div className="space-y-5">
      {/* Card: data path */}
      <SectionCard
        icon={HardDrive}
        title={t("settings.storage.path")}
        desc={
          pathErr
            ? t("settings.storage.path.unknown")
            : "SQLite store (.db) + tunnel config (TOML)"
        }
      >
        <div className="p-5">
          <div className="flex items-center gap-3 rounded-xl bg-white/[0.02] border border-white/[0.06] px-4 py-3">
            <FolderOpen size={16} className="text-zinc-500 shrink-0" />
            <code className="flex-1 text-zinc-300 text-[12.5px] font-mono truncate">
              {pathErr ? "—" : path || "…"}
            </code>
            <button
              onClick={onOpenFolder}
              disabled={!path || pathErr || revealing}
              className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg text-[12px] font-semibold bg-cyan-500 hover:bg-cyan-400 disabled:bg-zinc-800 disabled:text-zinc-600 text-white transition-all duration-150 disabled:cursor-not-allowed"
            >
              <FolderOpen size={12} />
              {t("settings.storage.open")}
            </button>
          </div>
        </div>
      </SectionCard>

      {/* Card: stats counters (read-only summary, real values from db.rs) */}
      <SectionCard
        icon={Database}
        title={t("settings.storage.stats")}
        desc={t("settings.storage.stats.desc")}
      >
        <StorageStats />
      </SectionCard>

      {/* Card: actions (reset) */}
      <SectionCard
        icon={Trash2}
        title={t("settings.storage.actions")}
        desc=""
      >
        <div className="p-5">
          {!resetPending ? (
            <button
              onClick={() => setResetPending(true)}
              className="w-full px-4 py-3 rounded-xl bg-rose-500/[0.06] border border-rose-500/30 text-rose-300 hover:bg-rose-500/[0.12] text-[13px] font-medium transition-all flex items-center justify-center gap-2"
            >
              <Trash2 size={14} />
              {t("settings.storage.actions.reset")}
            </button>
          ) : (
            <div className="rounded-xl border border-rose-500/30 bg-rose-500/[0.06] p-4 space-y-3">
              <div className="flex items-start gap-2.5">
                <AlertTriangle
                  size={16}
                  className="text-rose-300 shrink-0 mt-0.5"
                />
                <p className="text-rose-200 text-[13px]">
                  {t("settings.storage.actions.reset.confirm")}
                </p>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={onReset}
                  className="flex-1 py-2 rounded-lg bg-rose-500 hover:bg-rose-400 text-white text-[12.5px] font-semibold transition-all"
                >
                  {t("settings.storage.actions.reset")}
                </button>
                <button
                  onClick={() => {
                    setResetPending(false);
                    setResetMsg(null);
                  }}
                  className="flex-1 py-2 rounded-lg bg-white/[0.04] border border-white/[0.08] text-zinc-300 hover:text-white text-[12.5px] font-medium transition-all"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
          {resetMsg && (
            <div
              className={
                "mt-3 text-[12px] flex items-center gap-1.5 " +
                (resetMsg.ok ? "text-emerald-400" : "text-rose-400")
              }
            >
              {resetMsg.ok ? (
                <CheckCircle2 size={12} />
              ) : (
                <AlertTriangle size={12} />
              )}
              {resetMsg.text}
            </div>
          )}
        </div>
      </SectionCard>
    </div>
  );
}

function StorageStats() {
  const { t, locale } = useLocale();
  const { stats } = useAppStatsLite();
  if (!stats) {
    return (
      <div className="p-5 text-zinc-500 text-[12.5px]">—</div>
    );
  }
  const fmtCount = (n: number) =>
    new Intl.NumberFormat(
      locale === "es" ? "es-ES" : locale === "it" ? "it-IT" : "en-US",
    ).format(n);
  const fmtBytes = (n: number) => {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let v = n;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(1)} ${units[i]}`;
  };
  const ratio =
    stats.average_compression_ratio > 0
      ? `${Math.round((1 - stats.average_compression_ratio) * 100)}%`
      : "—";
  return (
    <div className="p-5 grid grid-cols-2 md:grid-cols-4 gap-3">
      <StatPill label="Archivos" value={fmtCount(stats.total_files_processed)} />
      <StatPill label="Ahorrado" value={fmtBytes(stats.total_bytes_saved)} />
      <StatPill label="Compartido" value={fmtCount(stats.total_files_shared)} />
      <StatPill label="Ratio" value={ratio} />
    </div>
  );
}

function StatPill({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl bg-white/[0.02] border border-white/[0.06] px-3 py-3">
      <p className="text-zinc-500 text-[10px] tracking-[0.15em] uppercase">
        {label}
      </p>
      <p className="text-white text-[18px] font-semibold tabular-nums leading-tight mt-1">
        {value}
      </p>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Section: Acerca de
// ─────────────────────────────────────────────────────────────

function AboutSection() {
  const { t, locale } = useLocale();
  return (
    <div className="space-y-5">
      <SectionCard
        icon={Info}
        title="NexusCompress"
        desc={t("settings.about.desc")}
        wide
      >
        <div className="divide-y divide-white/[0.04]">
          <AboutRow
            icon={Sparkles}
            label={t("settings.about.version")}
            value="0.1.1"
            mono
          />
          <AboutRow
            icon={Zap}
            label="Engine"
            value="NexusCompress v6 Solid-AST"
            mono
          />
          <AboutRow
            icon={Shield}
            label={t("settings.about.license")}
            value={t("settings.about.license.value")}
            mono
          />
          <AboutRow
            icon={Globe}
            label={t("settings.about.privacy")}
            value={t("settings.about.privacy.value")}
          />
          <AboutRow
            icon={Github}
            label={t("settings.about.repo")}
            value="github.com/bierfor/nexus-compress"
            mono
            href="https://github.com/bierfor/nexus-compress"
          />
        </div>
      </SectionCard>
    </div>
  );
}

function AboutRow({
  icon: Icon,
  label,
  value,
  mono,
  href,
}: {
  icon: typeof Info;
  label: string;
  value: string;
  mono?: boolean;
  href?: string;
}) {
  const content = (
    <div className="flex items-center justify-between gap-4">
      <div className="flex items-center gap-3 min-w-0">
        <Icon size={14} className="text-zinc-500 shrink-0" />
        <span className="text-zinc-400 text-[12.5px]">{label}</span>
      </div>
      <span
        className={
          "text-white text-[12.5px] truncate " +
          (mono ? "font-mono " : "") +
          (href ? "group-hover:text-cyan-300 transition-colors" : "")
        }
      >
        {value}
      </span>
    </div>
  );
  return (
    <div className={href ? "group" : ""}>
      {href ? (
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="block px-5 py-3.5 hover:bg-white/[0.03] transition-colors"
        >
          {content}
        </a>
      ) : (
        <div className="px-5 py-3.5">{content}</div>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Shared section wrapper
// ─────────────────────────────────────────────────────────────

function SectionCard({
  icon: Icon,
  title,
  desc,
  children,
  wide,
}: {
  icon: typeof Info;
  title: string;
  desc?: string;
  children: React.ReactNode;
  wide?: boolean;
}) {
  return (
    <div className="rounded-3xl bg-white/[0.02] border border-white/[0.06] overflow-hidden">
      <header className="px-5 py-4 border-b border-white/[0.04] flex items-start gap-3">
        <div className="w-9 h-9 rounded-xl bg-cyan-500/10 border border-cyan-500/30 flex items-center justify-center text-cyan-300 shrink-0">
          <Icon size={15} />
        </div>
        <div className="flex-1 min-w-0">
          <h2 className="text-white text-[14.5px] font-semibold leading-tight">
            {title}
          </h2>
          {desc && (
            <p className="text-zinc-500 text-[11.5px] leading-snug mt-1">
              {desc}
            </p>
          )}
        </div>
      </header>
      <div className={wide ? "" : ""}>{children}</div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Mini-hook for live stats in the Storage card.
//  Avoids importing the full useAppData so we don't pull in
//  the Tauri invoke + window-focus machinery for a single read.
// ─────────────────────────────────────────────────────────────

interface AppStatsLite {
  total_files_processed: number;
  total_bytes_saved: number;
  total_files_shared: number;
  average_compression_ratio: number;
  last_activity_timestamp: number;
}

function useAppStatsLite() {
  const [stats, setStats] = useState<AppStatsLite | null>(null);
  useEffect(() => {
    let alive = true;
    invoke<AppStatsLite>("get_stats_cmd")
      .then((s) => alive && setStats(s))
      .catch(() => alive && setStats(null));
    return () => {
      alive = false;
    };
  }, []);
  return { stats };
}

