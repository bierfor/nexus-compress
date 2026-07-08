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

import { useState, useCallback, useMemo } from "react";
import { NeoTopBar, type View } from "@/components/NeoTopBar";
import { LandingPage } from "@/components/LandingPage";
import { CompressView } from "@/components/CompressView";
import { DecompressView } from "@/components/DecompressView";
import { ShareView } from "@/components/ShareView";
import { NeoDashboard, type LastOp } from "@/components/NeoDashboard";
import { RecentView, type RecentOp } from "@/components/RecentView";

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
//  Settings — Sprint 5.7 rewrite
//
//  Sprint 5.7: extracted to its own component file with tabbed
//  sidebar navigation (Interfaz / Compresión / Transporte /
//  Almacenamiento / Acerca de). The page.tsx <Home> component
//  just imports the new view and drops it in.
// ============================================================

import { SettingsView } from "@/components/SettingsView";
