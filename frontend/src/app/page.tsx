"use client";

/**
 * Home — Sprint 5.6 "Neo Terminal" redesign.
 *
 * Layout:
 *   - NeoTopBar: minimal nav (Inicio · Recientes · Compartir · Ajustes)
 *   - [VIEW]: the active screen (LandingPage | CompressView | DecompressView | ShareView)
 *   - NeoDashboard: bottom strip with last op stats
 *
 * The 3 main actions (Compress / Decompress / Share) each get
 * their own dedicated view. The LandingPage is the entry point
 * with 3 big cards.
 */

import { useState, useCallback } from "react";
import { NeoTopBar, type View } from "@/components/NeoTopBar";
import { LandingPage } from "@/components/LandingPage";
import { CompressView } from "@/components/CompressView";
import { DecompressView } from "@/components/DecompressView";
import { ShareView } from "@/components/ShareView";
import { NeoDashboard, type LastOp } from "@/components/NeoDashboard";

export default function Home() {
  const [view, setView] = useState<View>("landing");
  const [lastOp, setLastOp] = useState<LastOp | null>(null);

  const onNavigate = useCallback((v: View) => {
    setView(v);
  }, []);

  const onSettings = useCallback(() => {
    setView("settings");
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
      setLastOp({
        kind: op.kind,
        filename: op.filename,
        originalBytes: op.originalBytes,
        compressedBytes: op.compressedBytes,
        restoredBytes: op.restoredBytes,
        durationMs: op.durationMs,
        status: "ok",
      });
    },
    []
  );

  return (
    <main className="h-screen flex flex-col bg-[#0a0a0a] text-white">
      {/* Subtle ambient gradient background */}
      <div className="fixed inset-0 pointer-events-none">
        <div className="absolute top-0 left-1/4 w-[600px] h-[600px] rounded-full bg-cyan-500/[0.03] blur-[120px]" />
        <div className="absolute bottom-0 right-1/4 w-[600px] h-[600px] rounded-full bg-emerald-500/[0.02] blur-[120px]" />
      </div>

      <NeoTopBar view={view} onNavigate={onNavigate} onSettings={onSettings} />

      <div className="relative flex-1 flex flex-col min-h-0 overflow-hidden">
        {view === "landing" && <LandingPage onNavigate={onNavigate} />}
        {view === "compress" && <CompressView onComplete={onOpComplete} />}
        {view === "decompress" && <DecompressView onComplete={onOpComplete} />}
        {view === "share" && <ShareView onComplete={onOpComplete} />}
        {view === "settings" && <SettingsView onNavigate={onNavigate} />}
        {view === "recent" && <RecentView onNavigate={onNavigate} />}
      </div>

      <NeoDashboard op={lastOp} />
    </main>
  );
}

// ============================================================
//  Settings (placeholder — full settings come in next iteration)
// ============================================================

function SettingsView({ onNavigate }: { onNavigate: (v: View) => void }) {
  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-3xl mx-auto px-8 pt-12 pb-20">
        <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
          <button onClick={() => onNavigate("landing")} className="hover:text-zinc-300">
            ← Volver
          </button>
        </div>
        <h1 className="text-white text-[36px] font-semibold tracking-tight mb-8">
          Ajustes
        </h1>

        <Section title="Transporte P2P">
          <Row label="Modo" value="Directo (LAN + UPnP)" />
          <Row label="Apertura UPnP" value="automática" />
          <Row label="Fallback" value="mDNS local" />
        </Section>

        <Section title="Compresión">
          <Row label="Por defecto" value="Balanceado (v5)" />
          <Row label="Diccionario" value="5348 entradas" />
        </Section>

        <Section title="Interfaz">
          <Row label="Tema" value="Oscuro (Neo)" />
          <Row label="Idioma" value="Español" />
        </Section>

        <Section title="Acerca de">
          <Row label="Versión" value="NexusRAR 0.1.0" />
          <Row label="Motor" value="NexusCompress v6 Solid-AST" />
        </Section>
      </div>
    </div>
  );
}

// ============================================================
//  Recent (placeholder)
// ============================================================

function RecentView({ onNavigate }: { onNavigate: (v: View) => void }) {
  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-3xl mx-auto px-8 pt-12 pb-20">
        <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
          <button onClick={() => onNavigate("landing")} className="hover:text-zinc-300">
            ← Volver
          </button>
        </div>
        <h1 className="text-white text-[36px] font-semibold tracking-tight mb-8">
          Recientes
        </h1>
        <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] p-12 text-center">
          <div className="text-zinc-500 text-[13px]">
            Aún no hay operaciones recientes.
          </div>
          <button
            onClick={() => onNavigate("landing")}
            className="mt-4 text-cyan-400 hover:text-cyan-300 text-[13px]"
          >
            Empezar una operación →
          </button>
        </div>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
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