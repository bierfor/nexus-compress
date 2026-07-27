/**
 * RecentView — Sprint 5.6.29 + Sprint 5.7 hotfix #18.
 *
 * Reads from the persistent SQLite stats store (db.rs) so the
 * "Recientes" list survives app restarts. Falls back to the
 * in-memory `ops` prop if Tauri isn't available (e.g. running
 * under `next dev` outside the Tauri shell).
 *
 * Sprint 5.7 wiring:
 *   - get_recent_events_cmd(50) → list of ActivityEvent (top 50)
 *   - get_stats_cmd()           → AppStats for the sidebar cards
 *   - on focus / on mount → re-fetch (so completing a new op
 *     in the compress/decompress/share views shows up here).
 */

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLocale } from "@/components/LocaleProvider";
import type { View } from "@/components/NeoTopBar";
import { getFileKind, getKindBadge } from "@/lib/fileIcons";
import {
  Archive,
  FolderOpen,
  Send,
  FileText,
  Search,
  SlidersHorizontal,
  Layers,
  Clock,
  ChevronRight,
  ChevronDown,
  ArrowRight,
  HardDrive,
  Pencil,
  RotateCw,
  Check,
  Download,
  BarChart3,
  MoreHorizontal,
  RefreshCw,
  AlertCircle,
} from "lucide-react";

// ── Tauri-side types (mirror db.rs) ───────────────────────────
export interface RecentOp {
  id: string;
  /** "compress" | "decompress" | "share" | "p2p_recv" */
  kind: "compress" | "decompress" | "share";
  filename: string;
  originalBytes: number;
  compressedBytes?: number;
  restoredBytes?: number;
  durationMs: number;
  timestamp: number;
  status: "ok" | "err";
}

interface ActivityEventRaw {
  id: number;
  kind: string;
  filename: string;
  original_bytes: number;
  output_bytes: number;
  timestamp: number;
}

interface AppStatsRaw {
  total_files_processed: number;
  total_bytes_saved: number;
  total_files_shared: number;
  average_compression_ratio: number;
  last_activity_timestamp: number;
}

export interface RecentViewProps {
  /** Fallback when Tauri isn't available (e.g. plain `next dev`). */
  ops: RecentOp[];
  onNavigate: (v: View) => void;
  onClear: () => void;
}

const PAGE_SIZE = 8;

export function RecentView({ ops, onNavigate, onClear }: RecentViewProps) {
  const { t, locale } = useLocale();
  const [filter, setFilter] = useState<"all" | "compress" | "decompress" | "share">("all");
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);

  // Remote data
  const [remoteOps, setRemoteOps] = useState<RecentOp[] | null>(null);
  const [stats, setStats] = useState<AppStatsRaw | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  const fetchData = async () => {
    setLoading(true);
    setErr(null);
    try {
      // Sprint 5.7: read from the persistent SQLite store.
      const [events, appStats] = await Promise.all([
        invoke<ActivityEventRaw[]>("get_recent_events_cmd", { limit: 50 }),
        invoke<AppStatsRaw>("get_stats_cmd"),
      ]);
      setRemoteOps(
        events.map((e) => ({
          id: String(e.id),
          kind: e.kind === "p2p_recv" ? "share" : (e.kind as RecentOp["kind"]),
          filename: e.filename,
          originalBytes: e.original_bytes,
          compressedBytes: e.output_bytes,
          restoredBytes: e.output_bytes,
          durationMs: 0,
          timestamp: e.timestamp * 1000,
          status: "ok" as const,
        })),
      );
      setStats(appStats);
    } catch (e) {
      // Likely running under `next dev` outside the Tauri shell.
      setErr(String(e));
      setRemoteOps(null);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchData();
    // Refetch when the tab regains focus so a newly-completed
    // op from the compress/decompress/share view shows up here.
    const onFocus = () => fetchData();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Prefer remote data; fall back to in-memory if Tauri isn't available.
  const opsList = remoteOps ?? ops;

  const filtered = useMemo(() => {
    let list = opsList;
    if (filter !== "all") list = list.filter((op) => op.kind === filter);
    if (search.trim()) {
      const q = search.trim().toLowerCase();
      list = list.filter((op) => op.filename.toLowerCase().includes(q));
    }
    return list;
  }, [opsList, filter, search]);

  const visible = filtered.slice(0, page * PAGE_SIZE);
  const hasMore = filtered.length > visible.length;

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-7xl mx-auto px-8 pt-8 pb-16">
        {/* Header */}
        <div className="mb-6 flex items-start justify-between">
          <div>
            <h1 className="text-white text-[32px] font-semibold tracking-tight leading-tight">
              {t("recent.title")}
            </h1>
            <p className="text-zinc-500 text-[14px] mt-1">
              {t("recent.subtitle")}
            </p>
          </div>
          <button
            onClick={fetchData}
            className="btn btn-ghost shrink-0 mt-1"
            title="Refrescar"
          >
            <RefreshCw size={13} className={loading ? "animate-spin" : ""} />
            {t("recent.refresh")}
          </button>
        </div>

        <div className="grid grid-cols-3 gap-5">
          {/* ── Main column: filter tabs + table ──────────────── */}
          <div className="col-span-2">
            {/* Filter row */}
            <div className="flex items-center gap-2 mb-4 flex-wrap">
              <FilterTab
                active={filter === "all"}
                onClick={() => setFilter("all")}
                icon={Layers}
                label={t("recent.filter.all")}
              />
              <FilterTab
                active={filter === "compress"}
                onClick={() => setFilter("compress")}
                icon={Archive}
                label={t("recent.filter.compressed")}
                tone="cyan"
              />
              <FilterTab
                active={filter === "decompress"}
                onClick={() => setFilter("decompress")}
                icon={FolderOpen}
                label={t("recent.filter.extracted")}
                tone="amber"
              />
              <FilterTab
                active={filter === "share"}
                onClick={() => setFilter("share")}
                icon={Send}
                label={t("recent.filter.shared")}
                tone="emerald"
              />
              {/* Search */}
              <div className="flex-1 min-w-[200px] relative">
                <Search
                  size={14}
                  className="absolute left-3.5 top-1/2 -translate-y-1/2 text-zinc-500 pointer-events-none"
                />
                <input
                  type="text"
                  value={search}
                  onChange={(e) => {
                    setSearch(e.target.value);
                    setPage(1);
                  }}
                  placeholder={t("recent.search.placeholder")}
                  className="input pl-10 py-2 text-[12.5px]"
                />
              </div>
              {/* Filter dropdown (visual) */}
              <button className="btn btn-ghost shrink-0">
                <SlidersHorizontal size={14} />
                {t("recent.filter.btn")}
                <ChevronDown size={12} />
              </button>
            </div>

            {/* Error banner (Tauri not available) */}
            {err && (
              <div className="mb-4 rounded-2xl bg-amber-500/10 border border-amber-500/30 px-4 py-3 flex items-center gap-2.5 text-amber-300 text-[12.5px]">
                <AlertCircle size={14} />
                <span>
                  {t("recent.err.tauri_missing")}{" "}
                  <span className="text-amber-400/70 font-mono text-[11px]">
                    {err.slice(0, 60)}
                  </span>
                </span>
              </div>
            )}

            {/* Files table */}
            {opsList.length === 0 && !loading ? (
              <div className="rounded-2xl glass p-12 text-center">
                <Clock size={32} className="text-zinc-500 mx-auto mb-3" />
                <p className="text-zinc-400 text-[13.5px] mb-1 font-medium">
                  {t("recent.empty.title")}
                </p>
                <p className="text-zinc-600 text-[12px] mb-4">
                  {t("recent.empty.desc")}
                </p>
                <button
                  onClick={() => onNavigate("compress")}
                  className="btn btn-primary"
                >
                  {t("recent.empty.cta")}
                </button>
              </div>
            ) : filtered.length === 0 && !loading ? (
              <div className="rounded-2xl glass p-10 text-center">
                <Search size={28} className="text-zinc-500 mx-auto mb-3" />
                <p className="text-zinc-400 text-[13px]">
                  {t("recent.empty.filtered")}
                </p>
              </div>
            ) : (
              <>
                <div className="rounded-2xl glass overflow-hidden">
                  {/* Header row */}
                  <div className="grid grid-cols-[2.4fr_1fr_0.8fr_1fr_1.2fr_60px] gap-4 px-5 py-3 border-b border-white/[0.06] text-[10px] tracking-[0.2em] uppercase text-zinc-500 font-medium">
                    <span>{t("recent.col.name")}</span>
                    <span>{t("recent.col.type")}</span>
                    <span>{t("recent.col.size")}</span>
                    <span>{t("recent.col.action")}</span>
                    <span>{t("recent.col.date")}</span>
                    <span></span>
                  </div>
                  {/* Rows */}
                  <div className="divide-y divide-white/[0.04]">
                    {visible.map((op) => (
                      <RecentTableRow
                        key={op.id}
                        op={op}
                        onNavigate={onNavigate}
                        labels={kindLabels(t)}
                      />
                    ))}
                  </div>
                </div>
                {hasMore && (
                  <button
                    onClick={() => setPage((p) => p + 1)}
                    className="w-full mt-3 py-2.5 text-[12.5px] text-cyan-400 hover:text-cyan-300 font-medium inline-flex items-center justify-center gap-1.5 transition-colors"
                  >
                    <ChevronDown size={14} className="animate-pulse" />
                    {t("recent.loadmore")}
                  </button>
                )}
              </>
            )}

            {/* Footer */}
            <div className="mt-4 flex items-center justify-between text-[11px] text-zinc-500">
              <div className="flex items-center gap-2">
                <HardDrive size={12} className="text-zinc-600" />
                <span>{t("recent.footer.default")}</span>
                <span className="text-zinc-300 font-mono">~/NexusCompress</span>
                <button className="ml-1 text-zinc-500 hover:text-white transition-colors">
                  <Pencil size={11} />
                </button>
              </div>
              <div className="flex items-center gap-3">
                <div className="flex items-center gap-1.5">
                  <span className="relative flex w-2 h-2">
                    <span className="absolute inline-flex h-full w-full rounded-full opacity-75 animate-ping bg-emerald-400" />
                    <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-400" />
                  </span>
                  <span className="text-emerald-400">{t("recent.footer.synced")}</span>
                </div>
                <span className="text-zinc-600">
                  {t("recent.footer.lastsync")}: {t("recent.footer.now")}
                </span>
                <button
                  onClick={fetchData}
                  className="text-zinc-500 hover:text-white transition-colors"
                  title="Refrescar"
                >
                  <RotateCw size={12} className={loading ? "animate-spin" : ""} />
                </button>
                {opsList.length > 0 && (
                  <button
                    onClick={onClear}
                    className="text-zinc-500 hover:text-rose-400 transition-colors"
                    title="Limpiar (memoria local)"
                  >
                    <span className="text-[10.5px] underline">
                      {t("recent.clear")}
                    </span>
                  </button>
                )}
              </div>
            </div>
          </div>

          {/* ── Sidebar ──────────────────────────────────────── */}
          <div className="space-y-5">
            {/* Quick actions */}
            <div className="rounded-2xl glass p-5">
              <h3 className="text-zinc-300 text-[13px] font-semibold mb-3">
                {t("recent.sidebar.actions")}
              </h3>
              <div className="space-y-1.5">
                <QuickAction
                  icon={Archive}
                  label={t("recent.sidebar.compress")}
                  onClick={() => onNavigate("compress")}
                  tone="cyan"
                />
                <QuickAction
                  icon={FolderOpen}
                  label={t("recent.sidebar.extract")}
                  onClick={() => onNavigate("decompress")}
                  tone="amber"
                />
                <QuickAction
                  icon={Send}
                  label={t("recent.sidebar.share")}
                  onClick={() => onNavigate("share")}
                  tone="emerald"
                />
                <QuickAction
                  icon={FileText}
                  label={t("recent.sidebar.open")}
                  onClick={() => onNavigate("compress")}
                  tone="blue"
                />
              </div>
            </div>

            {/* Stats (este mes) — wired to db.rs */}
            <div className="rounded-2xl glass p-5">
              <div className="flex items-center justify-between mb-3">
                <h3 className="text-zinc-300 text-[13px] font-semibold">
                  {t("recent.sidebar.stats")}
                </h3>
                <button className="text-zinc-500 hover:text-cyan-400 text-[11px] flex items-center gap-1 transition-colors">
                  {t("recent.sidebar.thismonth")}
                  <ChevronDown size={11} />
                </button>
              </div>
              <div className="grid grid-cols-2 gap-2.5">
                <StatBlock
                  icon={FileText}
                  value={
                    stats
                      ? formatCount(stats.total_files_processed, locale)
                      : "—"
                  }
                  label={t("recent.sidebar.stats.processed")}
                  tone="blue"
                />
                <StatBlock
                  icon={Download}
                  value={
                    stats ? formatBytes(stats.total_bytes_saved, locale) : "—"
                  }
                  label={t("recent.sidebar.stats.saved")}
                  tone="cyan"
                />
                <StatBlock
                  icon={BarChart3}
                  value={
                    stats && stats.average_compression_ratio > 0
                      ? `${Math.round(
                          (1 - stats.average_compression_ratio) * 100,
                        )}%`
                      : "—"
                  }
                  label={t("recent.sidebar.stats.ratio")}
                  tone="emerald"
                />
                <StatBlock
                  icon={Clock}
                  value={
                    stats
                      ? formatDuration(
                          Date.now() / 1000 - stats.last_activity_timestamp,
                          t,
                        )
                      : "—"
                  }
                  label={t("recent.sidebar.stats.timesaved")}
                  tone="amber"
                />
              </div>
            </div>

            {/* Activity sidebar (top 3) */}
            <div className="rounded-2xl glass p-5">
              <h3 className="text-zinc-300 text-[13px] font-semibold mb-3">
                {t("recent.sidebar.activity")}
              </h3>
              <div className="space-y-2.5">
                {opsList.slice(0, 3).map((op) => (
                  <RecentSidebarItem
                    key={op.id}
                    op={op}
                    labels={kindLabels(t)}
                  />
                ))}
                {opsList.length === 0 && (
                  <p className="text-zinc-600 text-[11.5px]">
                    {t("recent.empty.title")}
                  </p>
                )}
              </div>
              <button
                onClick={() => onNavigate("recent")}
                className="mt-3 text-cyan-400 hover:text-cyan-300 text-[11.5px] font-medium flex items-center gap-1 transition-colors"
              >
                {t("recent.sidebar.activity.all")}
                <ArrowRight size={11} />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Helpers ────────────────────────────────────────────────────

type TFunc = (k: any) => string;

function kindLabels(t: TFunc) {
  return {
    share: {
      chip: t("recent.kind.share"),
      desc: t("recent.desc.share"),
      color: "emerald",
      icon: Send,
    },
    compress: {
      chip: t("recent.kind.compress"),
      desc: t("recent.desc.compress"),
      color: "cyan",
      icon: Check,
    },
    decompress: {
      chip: t("recent.kind.decompress"),
      desc: t("recent.desc.decompress"),
      color: "amber",
      icon: FolderOpen,
    },
  } as const;
}

function formatCount(n: number, locale: string): string {
  try {
    return new Intl.NumberFormat(locale === "es" ? "es-ES" : locale === "it" ? "it-IT" : "en-US").format(n);
  } catch {
    return String(n);
  }
}

function formatBytes(n: number, locale: string): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  try {
    return `${new Intl.NumberFormat(locale === "es" ? "es-ES" : locale === "it" ? "it-IT" : "en-US", { maximumFractionDigits: 1 }).format(v)} ${units[i]}`;
  } catch {
    return `${v.toFixed(1)} ${units[i]}`;
  }
}

function formatDuration(secondsAgo: number, t: TFunc): string {
  if (secondsAgo < 60) return t("recent.time.just_now");
  if (secondsAgo < 3600) return t("recent.time.min_ago").replace("{n}", String(Math.floor(secondsAgo / 60)));
  if (secondsAgo < 86400) return t("recent.time.h_ago").replace("{n}", String(Math.floor(secondsAgo / 3600)));
  return t("recent.time.d_ago").replace("{n}", String(Math.floor(secondsAgo / 86400)));
}

// ── Subcomponents ──────────────────────────────────────────────

function FilterTab({
  active,
  onClick,
  icon: Icon,
  label,
  tone = "default",
}: {
  active: boolean;
  onClick: () => void;
  icon: typeof Layers;
  label: string;
  tone?: "default" | "cyan" | "amber" | "emerald";
}) {
  const toneClass: Record<typeof tone, string> = {
    default: "text-zinc-400",
    cyan: "text-cyan-400",
    amber: "text-amber-400",
    emerald: "text-emerald-400",
  };
  return (
    <button
      onClick={onClick}
      className={
        "flex items-center gap-1.5 px-3.5 py-2 rounded-xl text-[12.5px] font-medium transition-all duration-150 " +
        (active
          ? "bg-white/[0.08] border border-white/[0.12] text-white shadow-[inset_0_0_0_1px_rgba(255,255,255,0.06)]"
          : "bg-white/[0.02] border border-white/[0.06] text-zinc-400 hover:text-zinc-200 hover:bg-white/[0.04]")
      }
    >
      <Icon size={13} className={active ? toneClass[tone] : ""} />
      {label}
    </button>
  );
}

function QuickAction({
  icon: Icon,
  label,
  onClick,
  tone,
}: {
  icon: typeof Archive;
  label: string;
  onClick: () => void;
  tone: "cyan" | "amber" | "emerald" | "blue";
}) {
  const toneClass: Record<typeof tone, string> = {
    cyan: "bg-cyan-500/10 border-cyan-500/30 text-cyan-400",
    amber: "bg-amber-500/10 border-amber-500/30 text-amber-400",
    emerald: "bg-emerald-500/10 border-emerald-500/30 text-emerald-400",
    blue: "bg-blue-500/10 border-blue-500/30 text-blue-400",
  };
  return (
    <button
      onClick={onClick}
      className="w-full flex items-center gap-3 px-3 py-2.5 rounded-xl hover:bg-white/[0.03] transition-colors text-left group"
    >
      <div className={`w-9 h-9 rounded-lg border flex items-center justify-center ${toneClass[tone]}`}>
        <Icon size={15} />
      </div>
      <span className="text-zinc-200 text-[12.5px] flex-1">{label}</span>
      <ChevronRight
        size={14}
        className="text-zinc-600 group-hover:text-cyan-400 transition-colors"
      />
    </button>
  );
}

function StatBlock({
  icon: Icon,
  value,
  label,
  tone,
}: {
  icon: typeof FileText;
  value: string;
  label: string;
  tone: "blue" | "cyan" | "emerald" | "amber";
}) {
  const toneClass: Record<typeof tone, string> = {
    blue: "bg-blue-500/10 text-blue-400",
    cyan: "bg-cyan-500/10 text-cyan-400",
    emerald: "bg-emerald-500/10 text-emerald-400",
    amber: "bg-amber-500/10 text-amber-400",
  };
  return (
    <div className="rounded-xl bg-white/[0.02] border border-white/[0.06] p-3 flex items-center gap-2.5">
      <div
        className={`w-9 h-9 rounded-lg flex items-center justify-center shrink-0 ${toneClass[tone]}`}
      >
        <Icon size={15} />
      </div>
      <div className="min-w-0">
        <p className="text-white text-[16px] font-semibold tabular-nums leading-tight">
          {value}
        </p>
        <p className="text-zinc-500 text-[10px] leading-tight truncate">
          {label}
        </p>
      </div>
    </div>
  );
}

function RecentTableRow({
  op,
  onNavigate,
  labels,
}: {
  op: RecentOp;
  onNavigate: (v: View) => void;
  labels: ReturnType<typeof kindLabels>;
}) {
  const meta = labels[op.kind] ?? labels.compress;
  const dest: View =
    op.kind === "share" ? "share" : op.kind === "compress" ? "compress" : "decompress";
  // Sprint 5.7.21-B-FileIcons-Shared: use the shared file-icon
  // helpers instead of the kind-only icon. The main icon now
  // reflects the file extension (FileCode / FileImage / etc.)
  // and the kind badge in the bottom-right corner shows the
  // op kind (compress / decompress / share).
  const fileKind = getFileKind(op.filename);
  const kindBadge = getKindBadge(op.kind);
  const FileIcon = fileKind.icon;
  const KindIcon = kindBadge.icon;
  return (
    <div
      onClick={() => onNavigate(dest)}
      className="grid grid-cols-[2.4fr_1fr_0.8fr_1fr_1.2fr_60px] gap-4 px-5 py-3 items-center hover:bg-white/[0.03] transition-colors cursor-pointer group"
    >
      {/* Nombre + path — with file-type icon + kind badge overlay */}
      <div className="flex items-center gap-3 min-w-0">
        <div className="relative shrink-0">
          <div
            className={
              "w-9 h-9 rounded-lg flex items-center justify-center border " +
              fileKind.bg +
              " " +
              fileKind.border
            }
          >
            <FileIcon size={16} className={fileKind.color} strokeWidth={1.8} />
          </div>
          <div
            className={
              "absolute -bottom-0.5 -right-0.5 w-4 h-4 rounded-full flex items-center justify-center border border-[#0a0a0a] " +
              kindBadge.bg
            }
          >
            <KindIcon size={8} className={kindBadge.color} strokeWidth={2.5} />
          </div>
        </div>
        <div className="min-w-0">
          <p className="text-zinc-100 text-[12.5px] truncate font-medium">
            {op.filename}
          </p>
          <p className="text-zinc-600 text-[10.5px] truncate font-mono mt-0.5">
            ~/{parentDir(op.filename)}/
          </p>
        </div>
      </div>
      {/* Tipo (chip) */}
      <span
        className="self-start mt-2 px-2.5 py-0.5 rounded-full text-[10.5px] font-medium border bg-white/[0.04] border-white/[0.08] text-zinc-300"
      >
        {fileKind.icon === FileIcon ? (op.filename.split(".").pop()?.toLowerCase() ?? "?") : meta.chip}
      </span>
      {/* Tamaño */}
      <span className="text-zinc-300 text-[12px] font-mono tabular-nums">
        {prettyBytes(op.originalBytes)}
      </span>
      {/* Acción (verb) */}
      <span className="text-zinc-300 text-[12px]">{meta.chip}</span>
      {/* Fecha */}
      <span className="text-zinc-500 text-[11.5px]">
        {relativeTime(op.timestamp)}
      </span>
      {/* Action button */}
      <button
        onClick={(e) => {
          e.stopPropagation();
        }}
        className="text-zinc-500 hover:text-white p-1.5 rounded-md hover:bg-white/[0.05] transition-colors justify-self-end"
      >
        <MoreHorizontal size={14} />
      </button>
    </div>
  );
}

function RecentSidebarItem({
  op,
  labels,
}: {
  op: RecentOp;
  labels: ReturnType<typeof kindLabels>;
}) {
  const meta = labels[op.kind] ?? labels.compress;
  const Icon = meta.icon;
  const toneRing =
    meta.color === "cyan"
      ? "bg-cyan-500/10 border-cyan-500/30 text-cyan-400"
      : meta.color === "emerald"
        ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-400"
        : "bg-amber-500/10 border-amber-500/30 text-amber-400";
  return (
    <div className="flex items-start gap-2.5">
      <div
        className={`mt-0.5 w-6 h-6 rounded-md border flex items-center justify-center shrink-0 ${toneRing}`}
      >
        <Icon size={11} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-zinc-200 text-[12px] truncate font-medium">
          {op.filename}
        </p>
        <p className="text-zinc-500 text-[10.5px] mt-0.5 truncate">
          {meta.desc}
        </p>
      </div>
      <span className="text-zinc-600 text-[10.5px] shrink-0">
        {relativeTime(op.timestamp)}
      </span>
    </div>
  );
}

function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "ahora";
  if (diff < 3_600_000) return `hace ${Math.floor(diff / 60_000)} min`;
  if (diff < 86_400_000) return `hace ${Math.floor(diff / 3_600_000)} h`;
  if (diff < 7 * 86_400_000) return `hace ${Math.floor(diff / 86_400_000)} d`;
  const d = new Date(ts);
  return d.toLocaleDateString("es-ES", { day: "numeric", month: "short" });
}

function parentDir(filename: string): string {
  const stem = filename.replace(/\.[^.]+$/, "");
  return stem.length > 18 ? stem.slice(0, 18) + "…" : stem;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
