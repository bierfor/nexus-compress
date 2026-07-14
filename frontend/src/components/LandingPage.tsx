"use client";

/**
 * Sprint 5.7.21-B-Abstract: home view redesign.
 *
 * The previous version was a "marketing" landing with 5 feature pills,
 * a gradient headline, a drag-drop CTA card, 3 colorful action cards,
 * a 5-stat bar with different colors per stat, two duplicated
 * "recent activity" sections, a rotating "tip of the day", and a
 * keyboard shortcuts panel. Total: 674 lines of visual noise.
 *
 * The abstract version keeps the essential (greeting + 3 actions +
 * recent activity + shortcuts) and removes everything else. The
 * design follows the same principles as the Compress/Decompress
 * page refactor:
 *   - One accent color (cyan) for primary action
 *   - Neutral surfaces for everything else
 *   - No gradients, no shadows except primary CTA
 *   - Typography hierarchy (size/weight) does the work
 *   - The user reads the headline, picks an action, moves on
 *
 * Layout (top to bottom):
 *   1. Greeting (1 line)
 *   2. Headline (1 line, plain text)
 *   3. Three action buttons (compact, single accent)
 *   4. Recent activity (single column, 5 items)
 *   5. Keyboard shortcuts (1 row of kbd chips)
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useLocale } from "@/components/LocaleProvider";
import { type View } from "./NeoTopBar";
import { type RecentOp } from "@/components/RecentView";
import { useRecentEvents } from "@/lib/useAppData";
import {
  Archive,
  FolderOpen,
  Send,
  ArrowRight,
  FileText,
  FileCode,
  FileImage,
  FileVideo,
  FileAudio,
  FileArchive,
  FileSpreadsheet,
  FileJson,
  type LucideIcon,
} from "lucide-react";

type ActionId = "compress" | "decompress" | "share";

interface Action {
  id: ActionId;
  titleKey: import("@/lib/i18n").TranslationKey;
  descKey: import("@/lib/i18n").TranslationKey;
  icon: typeof Archive;
}

const ACTIONS: Action[] = [
  {
    id: "compress",
    titleKey: "action.compress.title" as import("@/lib/i18n").TranslationKey,
    descKey: "action.compress.sub" as import("@/lib/i18n").TranslationKey,
    icon: Archive,
  },
  {
    id: "decompress",
    titleKey: "action.decompress.title" as import("@/lib/i18n").TranslationKey,
    descKey: "action.decompress.sub" as import("@/lib/i18n").TranslationKey,
    icon: FolderOpen,
  },
  {
    id: "share",
    titleKey: "action.share.title" as import("@/lib/i18n").TranslationKey,
    descKey: "action.share.sub" as import("@/lib/i18n").TranslationKey,
    icon: Send,
  },
];

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

// Sprint 5.7.21-B-Home-Icons: file-type icon picker. Each
// extension family maps to a Lucide icon + accent color so the
// user can scan the recent activity and tell at a glance what
// kind of file was worked with. The mapping is intentionally
// broad (every common file type covered) and falls back to a
// generic FileText for unknown extensions.
type FileKind = {
  icon: LucideIcon;
  color: string;
  bg: string;
  border: string;
};

const FILE_ICON_MAP: Record<string, FileKind> = {
  // Code — cyan (matches the "compress" code source use case)
  js: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  jsx: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  ts: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  tsx: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  py: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  rs: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  go: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  java: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  rb: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  php: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  swift: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  kt: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  sh: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  html: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  css: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  // Images — violet
  jpg: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  jpeg: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  png: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  gif: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  svg: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  webp: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  // Video — rose
  mp4: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  mov: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  avi: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  mkv: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  webm: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  // Audio — amber
  mp3: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  wav: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  flac: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  aac: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  ogg: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  // Archives — emerald (matches the share/archive theme)
  zip: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  tar: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  gz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  tgz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  bz2: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  xz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  "7z": { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  rar: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxs: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxs6: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxe: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxr: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  lz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  // Spreadsheets — emerald
  xls: { icon: FileSpreadsheet, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  xlsx: { icon: FileSpreadsheet, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  csv: { icon: FileSpreadsheet, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  // JSON — yellow
  json: { icon: FileJson, color: "text-yellow-300", bg: "bg-yellow-500/10", border: "border-yellow-500/20" },
  // Documents — zinc (neutral)
  pdf: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  doc: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  docx: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  txt: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  md: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
};

const DEFAULT_FILE_KIND: FileKind = {
  icon: FileText,
  color: "text-zinc-400",
  bg: "bg-white/[0.04]",
  border: "border-white/[0.08]",
};

function getFileKind(filename: string): FileKind {
  // Strip path, then extension. Handles tar.gz, .tar.bz2, etc.
  // by taking the last extension only (most common case).
  const base = filename.split("/").pop() ?? filename;
  // Special: compound extensions like .tar.gz, .tar.bz2, .tar.xz
  const lower = base.toLowerCase();
  for (const compound of ["tar.gz", "tar.bz2", "tar.xz"]) {
    if (lower.endsWith(`.${compound}`)) {
      return FILE_ICON_MAP[compound] ?? DEFAULT_FILE_KIND;
    }
  }
  const ext = base.split(".").pop()?.toLowerCase() ?? "";
  return FILE_ICON_MAP[ext] ?? DEFAULT_FILE_KIND;
}

// Kind badge: small icon overlay on the file icon. The
// user can see at a glance whether the row is a compress,
// decompress, or share op.
function getKindBadge(kind: "compress" | "decompress" | "share"): {
  icon: LucideIcon;
  color: string;
  bg: string;
} {
  if (kind === "compress") {
    return { icon: Archive, color: "text-cyan-300", bg: "bg-cyan-500/20" };
  }
  if (kind === "decompress") {
    return { icon: FolderOpen, color: "text-amber-300", bg: "bg-amber-500/20" };
  }
  return { icon: Send, color: "text-emerald-300", bg: "bg-emerald-500/20" };
}

export function LandingPage({
  onNavigate,
  ops,
}: {
  onNavigate: (v: View) => void;
  ops: RecentOp[];
}) {
  const { t } = useLocale();
  // Sprint 5.7.21-B-Abstract fix: previous version of the
  // abstract home only read `ops` from props (in-memory list
  // for the current session). The old LandingPage used
  // useRecentEvents to fetch persisted events from db.rs
  // (SQLite), which surfaces activity across app restarts.
  // The home now merges both: persistent events from db.rs
  // + in-memory ops from the current session (fallback when
  // the Tauri runtime isn't available, e.g. `next dev`).
  const { ops: persistedOps, refresh } = useRecentEvents(10, ops);
  // Manual refresh trigger: the hook refreshes on window focus
  // but navigating to the home after a compress doesn't always
  // fire a focus event. We force a refresh on mount + whenever
  // a new op lands in the in-memory list (current session).
  const [refreshKey, setRefreshKey] = useState(0);
  const [lastSeenCount, setLastSeenCount] = useState(ops.length);
  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey, lastSeenCount]);
  useEffect(() => {
    if (ops.length !== lastSeenCount) {
      setLastSeenCount(ops.length);
    }
  }, [ops.length, lastSeenCount]);
  const recent5 = useMemo(
    () => persistedOps.slice(0, 5),
    [persistedOps],
  );
  const hasMore = persistedOps.length > 5;

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-4xl mx-auto px-8 pt-12 pb-16">
        {/* Greeting + headline */}
        <div className="mb-12">
          <p className="text-zinc-500 text-[13px] mb-3">
            {t("home.greeting")}
          </p>
          <h1 className="text-white text-[32px] font-semibold tracking-tight leading-[1.15]">
            {t("home.subheadline")}
          </h1>
        </div>

        {/* Action cards: 3 columns, compact, single accent */}
        <div className="grid grid-cols-3 gap-3 mb-12">
          {ACTIONS.map((a) => {
            const Icon = a.icon;
            return (
              <button
                key={a.id}
                onClick={() => onNavigate(a.id)}
                className="group text-left p-5 rounded-2xl border border-white/[0.06] bg-white/[0.02] hover:border-cyan-500/30 hover:bg-cyan-500/[0.04] transition-all duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
                data-testid={`home-action-${a.id}`}
              >
                <div className="flex items-center justify-between mb-4">
                  <Icon size={20} className="text-cyan-400" strokeWidth={1.5} />
                  <ArrowRight
                    size={14}
                    className="text-zinc-600 group-hover:text-cyan-400 group-hover:translate-x-0.5 transition-all"
                  />
                </div>
                <h3 className="text-white text-[15px] font-semibold tracking-tight mb-1">
                  {t(a.titleKey)}
                </h3>
                <p className="text-zinc-500 text-[12px] leading-snug">
                  {t(a.descKey)}
                </p>
              </button>
            );
          })}
        </div>

        {/* Recent activity */}
        <div className="mb-12">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-zinc-400 text-[11px] tracking-[0.2em] uppercase">
              {t("home.recent.title")}
            </h2>
            {persistedOps.length > 5 && (
              <button
                onClick={() => onNavigate("recent")}
                className="text-zinc-500 hover:text-cyan-400 text-[11.5px] transition-colors"
              >
                {t("home.recent.all")}
              </button>
            )}
          </div>
          {recent5.length === 0 ? (
            <p className="text-zinc-600 text-[12.5px] text-center py-12 italic">
              {t("home.recent.empty")}
            </p>
          ) : (
            <div className="rounded-2xl border border-white/[0.06] bg-white/[0.02] divide-y divide-white/[0.04]">
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

        {/* Shortcuts */}
        <div>
          <h2 className="text-zinc-400 text-[11px] tracking-[0.2em] uppercase mb-3">
            {t("home.shortcuts.title")}
          </h2>
          <div className="flex items-center gap-4 flex-wrap">
            <Shortcut keys={["⌘", "K"]} label={t("home.shortcuts.cmdk")} />
            <Shortcut keys={["⇧", "S"]} label={t("home.shortcuts.share")} />
            <Shortcut keys={["⌘", "O"]} label={t("home.shortcuts.open")} />
            <Shortcut keys={["⌘", ","]} label={t("home.shortcuts.settings")} />
          </div>
        </div>
      </div>
    </div>
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
  const verb =
    op.kind === "compress"
      ? t("recent.compressed")
      : op.kind === "decompress"
        ? t("recent.extracted")
        : t("recent.shared");
  const inputSize = op.originalBytes;
  const outputSize = op.compressedBytes ?? op.restoredBytes ?? 0;
  const saved =
    op.compressedBytes && op.compressedBytes > 0
      ? Math.round((1 - op.compressedBytes / op.originalBytes) * 100)
      : null;

  // Sprint 5.7.21-B-Home-Icons: file-type icon + kind badge
  // overlay. The main icon reflects the file extension
  // (FileCode / FileImage / FileVideo / etc.) and the corner
  // badge reflects the op kind (compress / decompress / share).
  const fileKind = getFileKind(op.filename);
  const kindBadge = getKindBadge(op.kind);
  const FileIcon = fileKind.icon;
  const KindIcon = kindBadge.icon;

  return (
    <button
      onClick={onClick}
      data-testid="home-recent-row"
      className="w-full flex items-center gap-3 px-4 py-3 hover:bg-white/[0.02] transition-colors text-left group"
    >
      {/* File-type icon with kind badge overlay */}
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
        {/* Kind badge in the bottom-right corner — small
            colored dot with the kind icon. Tells the user
            at a glance what operation this row represents. */}
        <div
          className={
            "absolute -bottom-0.5 -right-0.5 w-4 h-4 rounded-full flex items-center justify-center border border-[#0a0a0a] " +
            kindBadge.bg
          }
        >
          <KindIcon size={8} className={kindBadge.color} strokeWidth={2.5} />
        </div>
      </div>
      <div className="flex-1 min-w-0">
        <p className="text-zinc-200 text-[13px] truncate font-medium group-hover:text-white transition-colors">
          {op.filename}
        </p>
        <p className="text-zinc-600 text-[11px] mt-0.5 font-mono tabular-nums flex items-center gap-1.5">
          <span>{prettyBytes(inputSize)} → {prettyBytes(outputSize)}</span>
          <span className="text-zinc-700">·</span>
          <span className="text-zinc-500">{verb}</span>
        </p>
      </div>
      {saved !== null && saved > 0 && (
        <span className="text-cyan-400 text-[12px] font-mono tabular-nums shrink-0">
          −{saved}%
        </span>
      )}
      <span className="text-zinc-600 text-[11px] tabular-nums shrink-0">
        {relativeTime(op.timestamp, t)}
      </span>
    </button>
  );
}

function Shortcut({ keys, label }: { keys: string[]; label: string }) {
  return (
    <div className="flex items-center gap-2">
      <div className="flex items-center gap-0.5">
        {keys.map((k) => (
          <kbd
            key={k}
            className="px-1.5 py-0.5 rounded bg-white/[0.04] border border-white/[0.08] text-zinc-300 text-[10.5px] font-mono"
          >
            {k}
          </kbd>
        ))}
      </div>
      <span className="text-zinc-500 text-[12px]">{label}</span>
    </div>
  );
}
