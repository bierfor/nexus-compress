"use client";

/**
 * Home — Sprint 5.6.1 "Neo Terminal" + recientes persistence.
 *
 * Sprint 5.6.1 fixes:
 *   - `recentOps` state accumulates every completed op (not just last)
 *   - `RecentView` reads from it and renders the full history
 *   - Each view (Compress / Decompress / Share) still pushes to
 *     `lastOp` for the dashboard + to `recentOps` for the history
 */

import { useState, useCallback } from "react";
import { NeoTopBar, type View } from "@/components/NeoTopBar";
import { LandingPage } from "@/components/LandingPage";
import { CompressView } from "@/components/CompressView";
import { DecompressView } from "@/components/DecompressView";
import { ShareView } from "@/components/ShareView";
import { NeoDashboard, type LastOp } from "@/components/NeoDashboard";

export interface RecentOp extends LastOp {
  id: string;
  timestamp: number;
}

export default function Home() {
  const [view, setView] = useState<View>("landing");
  const [lastOp, setLastOp] = useState<LastOp | null>(null);
  const [recentOps, setRecentOps] = useState<RecentOp[]>([]);

  const onNavigate = useCallback((v: View) => {
    setView(v);
  }, []);


  const onOpComplete = useCallback(
    (op: {
      kind: "compress" | "decompress" | "share";
      filename: string;
      originalBytes: number;
      compressedBytes?: number;
      restoredBytes?: number;
      durationMs: number;
    }) => {
      const fullOp: RecentOp = {
        ...op,
        id:
          typeof crypto !== "undefined" && "randomUUID" in crypto
            ? crypto.randomUUID()
            : `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        timestamp: Date.now(),
        status: "ok",
      };
      setLastOp(fullOp);
      setRecentOps((prev) => [fullOp, ...prev].slice(0, 30));
    },
    []
  );

  const onClearRecent = useCallback(() => {
    setRecentOps([]);
  }, []);

  return (
    <main className="h-screen flex flex-col bg-[#0a0a0a] text-white">
      {/* Ambient gradient background */}
      <div className="fixed inset-0 pointer-events-none">
        <div className="absolute top-0 left-1/4 w-[600px] h-[600px] rounded-full bg-cyan-500/[0.03] blur-[120px]" />
        <div className="absolute bottom-0 right-1/4 w-[600px] h-[600px] rounded-full bg-emerald-500/[0.02] blur-[120px]" />
      </div>

      <NeoTopBar view={view} onNavigate={onNavigate} />

      {/* Sprint 5.6.29: keyed wrapper re-mounts on view change so the
          `animate-fade-in` keyframe plays on every navigation,
          giving a smooth slide-up + fade transition. */}
      <div
        key={view}
        className="relative flex-1 flex flex-col min-h-0 overflow-hidden animate-fade-in"
      >
        {view === "landing" && <LandingPage onNavigate={onNavigate} ops={recentOps} />}
        {view === "compress" && (
          <CompressView
            onComplete={onOpComplete}
            onNavigate={onNavigate}
          />
        )}
        {view === "decompress" && (
          <DecompressView
            onComplete={onOpComplete}
            onNavigate={onNavigate}
          />
        )}
        {view === "share" && (
          <ShareView
            onComplete={onOpComplete}
            onNavigate={onNavigate}
          />
        )}
        {view === "settings" && <SettingsView />}
        {view === "recent" && (
          <RecentView ops={recentOps} onNavigate={onNavigate} onClear={onClearRecent} />
        )}
      </div>

      <NeoDashboard op={lastOp} />
    </main>
  );
}

// ============================================================
//  Settings — Sprint 5.6.29 hotfix #15 (rev)
//
//  Uses the real <ConfigPanel /> for actual configuration
//  (compression mode, transport, etc.) wrapped in a localized
//  page header + language switcher. Replaces the inline stub
//  that was hardcoded Spanish.
// ============================================================

import { useLocale } from "@/components/LocaleProvider";
import type { Locale } from "@/lib/i18n";
import { ConfigPanel } from "@/components/ConfigPanel";
import { PageHeader } from "@/components/PageHeader";
import { Languages } from "lucide-react";

function SettingsView() {
  const { t, locale, setLocale } = useLocale();

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-3xl mx-auto px-8 pt-12 pb-20">
        <PageHeader title={t("settings.title")} />

        {/* Language switcher — lives at the top of settings */}
        <div className="mb-10 rounded-2xl bg-white/[0.03] border border-white/[0.08] overflow-hidden">
          <div className="px-5 py-3 border-b border-white/[0.04] flex items-center gap-2 text-zinc-500 text-[11px] tracking-[0.2em] uppercase">
            <Languages size={12} />
            {t("settings.section.language")}
          </div>
          <div className="px-5 py-4 flex items-center justify-between">
            <span className="text-zinc-400 text-[13.5px]">{t("settings.interface.language")}</span>
            <div className="flex items-center gap-1.5">
              {(["es", "en", "it"] as Locale[]).map((l) => (
                <button
                  key={l}
                  onClick={() => setLocale(l)}
                  className={
                    "px-3 py-1.5 text-[12px] rounded-lg font-mono uppercase tracking-wider transition-colors " +
                    (locale === l
                      ? "bg-cyan-500/20 border border-cyan-500/40 text-cyan-300"
                      : "bg-white/[0.04] border border-white/[0.08] text-zinc-400 hover:text-white hover:border-white/[0.16]")
                  }
                >
                  {l}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Real config panel */}
        <ConfigPanel />
      </div>
    </div>
  );
}

// ============================================================
//  Recent — Sprint 5.6.1: real ops list from parent state
// ============================================================

function RecentView({
  ops,
  onNavigate,
  onClear,
}: {
  ops: RecentOp[];
  onNavigate: (v: View) => void;
  onClear: () => void;
}) {
  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-3xl mx-auto px-8 pt-12 pb-20">
        <div className="flex items-center justify-between mb-8">
          <div>
            <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
              <button
                onClick={() => onNavigate("landing")}
                className="hover:text-zinc-300"
              >
                ← Volver
              </button>
            </div>
            <h1 className="text-white text-[36px] font-semibold tracking-tight">
              Recientes
            </h1>
          </div>
          {ops.length > 0 && (
            <button
              onClick={onClear}
              className="px-3 py-1.5 text-[12px] text-zinc-500 hover:text-red-400 border border-white/[0.08] hover:border-red-500/30 rounded-lg transition-colors"
            >
              Limpiar todo
            </button>
          )}
        </div>

        {ops.length === 0 ? (
          <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] p-12 text-center">
            <div className="text-zinc-500 text-[13px] mb-4">
              Aún no hay operaciones recientes.
            </div>
            <button
              onClick={() => onNavigate("landing")}
              className="text-cyan-400 hover:text-cyan-300 text-[13px]"
            >
              Empezar una operación →
            </button>
          </div>
        ) : (
          <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] overflow-hidden divide-y divide-white/[0.04]">
            {ops.map((op) => (
              <RecentRow key={op.id} op={op} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function RecentRow({ op }: { op: RecentOp }) {
  const inputSize = op.originalBytes;
  const outputSize = op.compressedBytes ?? op.restoredBytes ?? 0;
  const saved = op.compressedBytes ? inputSize - outputSize : 0;
  const savings = saved > 0 ? saved / inputSize : 0;
  const kindLabel =
    op.kind === "share"
      ? { icon: "🚀", label: "Compartido", color: "emerald" }
      : op.kind === "compress"
      ? { icon: "📦", label: "Comprimido", color: "cyan" }
      : { icon: "📂", label: "Extraído", color: "amber" };

  const colorClass =
    kindLabel.color === "emerald"
      ? "text-emerald-400"
      : kindLabel.color === "cyan"
      ? "text-cyan-400"
      : "text-amber-400";

  return (
    <div className="px-5 py-4 flex items-center gap-4 text-[13px]">
      <span className="text-xl">{kindLabel.icon}</span>
      <div className="flex-1 min-w-0">
        <div className="text-white truncate" title={op.filename}>
          {op.filename}
        </div>
        <div className="text-zinc-600 text-[11.5px] mt-0.5">
          {relativeTime(op.timestamp)} · {kindLabel.label} ·{" "}
          <span className="font-mono tabular-nums">
            {(op.durationMs / 1000).toFixed(1)}s
          </span>
        </div>
      </div>
      <div className="text-right text-[12px] tabular-nums">
        {op.compressedBytes && (
          <div className={`${colorClass}`}>
            −{(savings * 100).toFixed(0)}%
          </div>
        )}
        <div className="text-zinc-600 text-[10.5px] font-mono">
          {prettyBytes(inputSize)}
          {outputSize > 0 && outputSize !== inputSize && (
            <> → {prettyBytes(outputSize)}</>
          )}
        </div>
      </div>
    </div>
  );
}

function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "ahora";
  if (diff < 3_600_000) return `hace ${Math.floor(diff / 60_000)} min`;
  if (diff < 86_400_000) return `hace ${Math.floor(diff / 3_600_000)} h`;
  return `hace ${Math.floor(diff / 86_400_000)} d`;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-8">
      <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
        {title}
      </div>
      <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] overflow-hidden divide-y divide-white/[0.04]">
        {children}
      </div>
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="px-5 py-3 flex items-center justify-between text-[13.5px]">
      <span className="text-zinc-400">{label}</span>
      <span className="text-white font-mono text-[12px]">{value}</span>
    </div>
  );
}