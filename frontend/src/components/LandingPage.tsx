"use client";

/**
 * LandingPage — the "one screen, one decision" entry point
 * (Sprint 5.6 Neo Terminal).
 *
 * Three big action cards. Each opens its own dedicated view.
 * No competing CTAs. No "panel de administración" feel — just
 * three clear choices.
 */

import { View } from "./NeoTopBar";

const ACTIONS = [
  {
    id: "compress" as View,
    icon: "📦",
    title: "Comprimir",
    subtitle: "Crear archivo .nxs6",
    description: "Reduce el tamaño de tus archivos con compresión de alta densidad.",
    accent: "from-cyan-500/20 to-cyan-500/0",
    iconBg: "bg-cyan-500/10",
    iconColor: "text-cyan-400",
    badge: "Crea",
  },
  {
    id: "decompress" as View,
    icon: "📂",
    title: "Descomprimir",
    subtitle: "Extraer archivo .nxs6",
    description: "Recupera los archivos originales desde cualquier formato Nexus.",
    accent: "from-amber-500/20 to-amber-500/0",
    iconBg: "bg-amber-500/10",
    iconColor: "text-amber-400",
    badge: "Extrae",
  },
  {
    id: "share" as View,
    icon: "🚀",
    title: "Compartir",
    subtitle: "Enviar a otra persona",
    description: "Transfiere archivos pesados sin servidor intermedio, cifrado E2E.",
    accent: "from-emerald-500/20 to-emerald-500/0",
    iconBg: "bg-emerald-500/10",
    iconColor: "text-emerald-400",
    badge: "Envía",
  },
];

export function LandingPage({ onNavigate }: { onNavigate: (v: View) => void }) {
  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-5xl mx-auto px-8 pt-20 pb-16">
        {/* Hero */}
        <div className="mb-16">
          <div className="text-zinc-500 text-[13px] tracking-wide mb-3">
            ¿Qué quieres hacer hoy?
          </div>
          <h1 className="text-white text-[44px] font-semibold tracking-tight leading-[1.1] mb-4">
            Transfiere archivos
            <br />
            <span className="bg-gradient-to-r from-cyan-300 to-emerald-300 bg-clip-text text-transparent">
              sin complicaciones.
            </span>
          </h1>
          <p className="text-zinc-400 text-[15px] leading-relaxed max-w-2xl">
            Compresión de alta densidad y transferencia peer-to-peer cifrada
            punto a punto. Sin servidores intermedios, sin cuentas, sin
            rastreo.
          </p>
        </div>

        {/* Three big cards */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-5">
          {ACTIONS.map((action) => (
            <button
              key={action.id}
              onClick={() => onNavigate(action.id)}
              className="group relative text-left p-6 rounded-2xl bg-white/[0.03] hover:bg-white/[0.06] border border-white/[0.06] hover:border-white/[0.12] transition-all overflow-hidden"
            >
              {/* Gradient accent (top-right) */}
              <div
                className={`absolute -top-12 -right-12 w-40 h-40 rounded-full bg-gradient-to-br ${action.accent} blur-2xl opacity-60 group-hover:opacity-100 transition-opacity`}
              />

              <div className="relative">
                {/* Icon */}
                <div
                  className={`w-12 h-12 rounded-xl ${action.iconBg} ${action.iconColor} flex items-center justify-center text-2xl mb-5`}
                >
                  {action.icon}
                </div>

                {/* Title + subtitle */}
                <h3 className="text-white text-[20px] font-semibold tracking-tight mb-1">
                  {action.title}
                </h3>
                <div className="text-zinc-500 text-[12.5px] tracking-wide mb-4">
                  {action.subtitle}
                </div>

                {/* Description */}
                <p className="text-zinc-400 text-[13px] leading-relaxed">
                  {action.description}
                </p>

                {/* Footer arrow */}
                <div className="mt-6 flex items-center gap-2 text-zinc-500 group-hover:text-white transition-colors">
                  <span className="text-[12px] font-medium">Empezar</span>
                  <span className="transition-transform group-hover:translate-x-1">
                    →
                  </span>
                </div>
              </div>
            </button>
          ))}
        </div>

        {/* Tip strip */}
        <div className="mt-16 text-center text-zinc-600 text-[12px]">
          Arrastra archivos directamente sobre cualquier pantalla ·{" "}
          <span className="text-zinc-500">⌘</span>
          <span className="text-zinc-500">+</span>
          <span className="text-zinc-500">.</span> para mostrar/ocultar
        </div>
      </div>
    </div>
  );
}