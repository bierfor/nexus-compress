"use client";

import { useLocale } from "@/components/LocaleProvider";
import { useTheme } from "@/components/ThemeProvider";
import {
  Home,
  History,
  Share2,
  Settings as SettingsIcon,
  Hexagon,
  Sun,
  Moon,
} from "lucide-react";

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
  const { theme, toggle } = useTheme();

  const navItems: { id: View; label: string; Icon: typeof Home }[] = [
    { id: "landing",  label: t("nav.home"),     Icon: Home },
    { id: "recent",   label: t("nav.recent"),   Icon: History },
    { id: "share",    label: t("nav.share"),    Icon: Share2 },
    { id: "settings", label: t("nav.settings"), Icon: SettingsIcon },
  ];

  return (
    <header
      data-tauri-drag-region
      className="h-14 flex items-center justify-between px-8 shrink-0 select-none border-b border-white/[0.06] glass-strong"
    >
      {/* Brand */}
      <div className="flex items-center gap-2.5 w-[120px]">
        <div className="relative w-8 h-8 rounded-xl bg-gradient-to-br from-cyan-400 to-blue-500 flex items-center justify-center text-black shadow-[0_0_24px_-4px_rgba(34,211,238,0.5)] transition-transform duration-300 hover:scale-105 hover:rotate-6">
          <Hexagon size={18} strokeWidth={2.5} fill="currentColor" />
        </div>
        <span className="text-white font-semibold text-[14px] tracking-tight">
          NexusRAR
        </span>
      </div>

      {/* Nav */}
      <nav className="flex items-center gap-1">
        {navItems.map((item) => {
          const active = view === item.id;
          const Icon = item.Icon;
          return (
            <button
              key={item.id}
              onClick={() => onNavigate(item.id)}
              aria-label={item.label}
              aria-current={active ? "page" : undefined}
              className={
                "group relative flex items-center gap-1.5 px-3.5 py-1.5 text-[12.5px] font-medium rounded-lg transition-all duration-200 " +
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50 " +
                (active
                  ? "text-white bg-white/[0.08] shadow-[inset_0_0_0_1px_rgba(255,255,255,0.08)]"
                  : "text-zinc-500 hover:text-zinc-200 hover:bg-white/[0.04] hover:-translate-y-px")
              }
            >
              <Icon
                size={14}
                strokeWidth={active ? 2.4 : 2}
                className={"transition-transform duration-200 " + (active ? "" : "group-hover:scale-110")}
              />
              {item.label}
              {/* Active underline indicator — animates in/out */}
              {active && (
                <span className="absolute -bottom-px left-2 right-2 h-px bg-gradient-to-r from-transparent via-cyan-400 to-transparent animate-fade-in" />
              )}
            </button>
          );
        })}
      </nav>

      {/* Version + theme toggle */}
      <div className="w-[120px] flex justify-end items-center gap-2">
        <span className="text-zinc-500 text-[11px] font-mono tracking-wide tabular-nums px-2 py-0.5 rounded-md border border-white/[0.06]">
          v0.3.0
        </span>
        <button
          onClick={toggle}
          className="w-8 h-8 rounded-lg flex items-center justify-center text-zinc-400 hover:text-amber-400 hover:bg-white/[0.05] border border-white/[0.06] hover:border-amber-500/30 transition-all duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
          aria-label={theme === "dark" ? t("theme.toggle.toLight") : t("theme.toggle.toDark")}
        >
          {theme === "dark" ? <Sun size={14} /> : <Moon size={14} />}
        </button>
      </div>
    </header>
  );
}
