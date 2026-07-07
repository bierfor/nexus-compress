"use client";

/**
 * NeoTopBar — minimal top bar (Sprint 5.6 "Neo Terminal").
 *
 * - Brand mark on the left
 * - Centered nav: Inicio · Recientes · Ajustes
 * - Right side: a single settings button (no clutter)
 *
 * Active nav item is highlighted with a subtle accent dot.
 */

export type View = "landing" | "compress" | "decompress" | "share" | "settings" | "recent";

export function NeoTopBar({
  view,
  onNavigate,
  onSettings,
}: {
  view: View;
  onNavigate: (v: View) => void;
  onSettings: () => void;
}) {
  const navItems: { id: View; label: string }[] = [
    { id: "landing", label: "Inicio" },
    { id: "recent", label: "Recientes" },
    { id: "share", label: "Compartir" },
    { id: "settings", label: "Ajustes" },
  ];

  return (
    <header
      data-tauri-drag-region
      className="h-14 flex items-center justify-between px-8 shrink-0 select-none border-b border-white/[0.04] backdrop-blur-md bg-black/20"
    >
      {/* Brand */}
      <div className="flex items-center gap-2.5">
        <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-cyan-400 to-blue-500 flex items-center justify-center text-black font-bold text-[14px]">
          ▣
        </div>
        <span className="text-white font-semibold text-[14px] tracking-tight">
          NexusRAR
        </span>
      </div>

      {/* Nav */}
      <nav className="flex items-center gap-1">
        {navItems.map((item) => {
          const active = view === item.id;
          return (
            <button
              key={item.id}
              onClick={() => onNavigate(item.id)}
              className={`px-3.5 py-1.5 text-[12.5px] font-medium rounded-lg transition-all ${
                active
                  ? "bg-white/[0.08] text-white"
                  : "text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.03]"
              }`}
            >
              {item.label}
            </button>
          );
        })}
      </nav>

      {/* Right spacer for symmetry */}
      <div className="w-[120px]" />
    </header>
  );
}