"use client";

import { useLocale } from "@/components/LocaleProvider";

export type View =
  | "landing"
  | "compress"
  | "decompress"
  | "share"
  | "settings"
  | "recent";

export function NeoTopBar({
  view,
  onNavigate,
}: {
  view: View;
  onNavigate: (v: View) => void;
}) {
  const { t } = useLocale();

  const navItems: { id: View; label: string }[] = [
    { id: "landing",  label: t("nav.home")     },
    { id: "recent",   label: t("nav.recent")   },
    { id: "share",    label: t("nav.share")    },
    { id: "settings", label: t("nav.settings") },
  ];

  return (
    <header
      data-tauri-drag-region
      className="h-14 flex items-center justify-between px-8 shrink-0 select-none border-b border-white/[0.04] backdrop-blur-md bg-black/20"
    >
      {/* Brand */}
      <div className="flex items-center gap-2.5 w-[120px]">
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

      {/* Version */}
      <div className="w-[120px] flex justify-end">
        <span className="text-zinc-600 text-[11px] font-mono tracking-wide">
          v0.1.0
        </span>
      </div>
    </header>
  );
}
