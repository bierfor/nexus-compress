// Sprint 5.7.21-B-Abstract: single-chip preset picker for
// the Compress page. Replaces the 3-column PresetCard grid
// with one compact element in the main view. Click → small
// popover with all built-in presets + custom profiles +
// save-as action.
//
// The popover is anchored to the chip and closes on
// click-outside, Esc, or selecting a preset.

import { useEffect, useRef, useState } from "react";
import { useLocale } from "./LocaleProvider";
import {
  ChevronDown,
  Check,
  Save,
  Trash2,
  Star,
  Settings,
} from "lucide-react";
import {
  type AnyProfile,
  type CustomProfile,
  BUILTIN_PRESETS,
} from "../lib/profiles";
import type { TranslationKey } from "@/lib/i18n";

interface PresetPickerProps {
  /// The currently active preset (built-in or custom).
  /// null = "no preset selected" (the user has edited the
  /// profile manually).
  activePreset: AnyProfile | undefined;
  /// List of custom profiles to render below the built-ins.
  customProfiles: CustomProfile[];
  /// Apply a preset (built-in or custom).
  applyPreset: (p: AnyProfile) => void;
  /// True when the current profile doesn't match any
  /// preset — used to surface the "Save as custom" action.
  profileIsCustom: boolean;
  /// Save the current profile as a custom preset.
  saveAsCustom: (name: string) => void;
  /// Delete a custom profile.
  deleteCustom: (id: string) => void;
  /// Open the full settings drawer (for advanced tuning).
  onOpenSettings: () => void;
  busy: boolean;
}

export function PresetPicker({
  activePreset,
  customProfiles,
  applyPreset,
  profileIsCustom,
  saveAsCustom,
  deleteCustom,
  onOpenSettings,
  busy,
}: PresetPickerProps) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [saveAsOpen, setSaveAsOpen] = useState(false);
  const [saveAsName, setSaveAsName] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on click outside
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const isCustomActive =
    !!activePreset &&
    !BUILTIN_PRESETS.some((p) => p.id === activePreset.id);

  const icon: string =
    activePreset && "icon" in activePreset
      ? ((activePreset as { icon: string }).icon)
      : "⭐";

  const displayName: string = activePreset
    ? isCustomActive
      ? (activePreset as CustomProfile).name
      : t(
          `compress.preset.${(activePreset as { id: string }).id}` as TranslationKey,
        )
    : t("compress.profile.desc");

  return (
    <div className="relative inline-block" ref={containerRef}>
      <button
        onClick={() => setOpen((s) => !s)}
        disabled={busy}
        aria-haspopup="listbox"
        aria-expanded={open}
        data-testid="preset-picker-chip"
        data-active-preset={activePreset?.id ?? "none"}
        className={
          "group flex items-center gap-2 pl-2.5 pr-2.5 py-1.5 rounded-full border transition-all disabled:opacity-50 " +
          (open
            ? "border-cyan-400/40 bg-cyan-500/[0.06]"
            : activePreset
              ? "border-white/[0.10] bg-white/[0.03] hover:border-white/[0.16] hover:bg-white/[0.05]"
              : "border-dashed border-amber-400/40 bg-amber-400/[0.04] text-amber-200 hover:bg-amber-400/[0.08]")
        }
      >
        <span className="text-[14px] leading-none">{icon}</span>
        <span
          className={
            "text-[12.5px] font-medium tracking-tight " +
            (activePreset ? "text-white" : "text-amber-200")
          }
        >
          {displayName}
        </span>
        <ChevronDown
          size={12}
          className={
            "transition-transform " +
            (open ? "rotate-180 text-cyan-300" : "text-zinc-500")
          }
        />
      </button>

      {open && (
        <div
          role="listbox"
          data-testid="preset-picker-popover"
          className="absolute top-full left-0 mt-2 w-72 max-h-[70vh] overflow-y-auto rounded-2xl border border-white/[0.08] bg-[#0a0a0a]/95 backdrop-blur-md shadow-2xl z-40"
        >
          {/* Built-in presets */}
          <div className="px-2 py-2">
            <div className="text-zinc-500 text-[10px] tracking-[0.15em] uppercase px-2 py-1.5">
              {t("compress.profile.title")}
            </div>
            {BUILTIN_PRESETS.map((p) => {
              const active = p.id === activePreset?.id;
              return (
                <button
                  key={p.id}
                  role="option"
                  aria-selected={active}
                  onClick={() => {
                    applyPreset(p);
                    setOpen(false);
                  }}
                  disabled={busy}
                  className={
                    "w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-left transition-colors disabled:opacity-50 " +
                    (active
                      ? "bg-cyan-500/[0.08] text-cyan-100"
                      : "text-zinc-200 hover:bg-white/[0.04]")
                  }
                >
                  <span className="text-[15px] leading-none">{p.icon}</span>
                  <div className="flex-1 min-w-0">
                    <div className="text-[12.5px] font-medium">
                      {t(`compress.preset.${p.id}` as TranslationKey)}
                    </div>
                    <div className="text-zinc-500 text-[10.5px] leading-snug truncate">
                      {t(`compress.preset.${p.id}.card` as TranslationKey)}
                    </div>
                  </div>
                  {active && (
                    <Check size={12} className="text-cyan-400 shrink-0" />
                  )}
                </button>
              );
            })}
          </div>

          {/* Custom profiles */}
          {customProfiles.length > 0 && (
            <div className="px-2 py-2 border-t border-white/[0.06]">
              <div className="text-zinc-500 text-[10px] tracking-[0.15em] uppercase px-2 py-1.5 flex items-center gap-1.5">
                <Star size={10} />
                {t("compress.settings.preset.custom_section")}
              </div>
              {customProfiles.map((cp) => {
                const active = cp.id === activePreset?.id;
                return (
                  <div
                    key={cp.id}
                    className="group flex items-center gap-1 hover:bg-white/[0.04] rounded-lg"
                  >
                    <button
                      onClick={() => {
                        applyPreset(cp);
                        setOpen(false);
                      }}
                      disabled={busy}
                      className="flex-1 flex items-center gap-2 px-2.5 py-2 text-left"
                    >
                      <span className="text-[15px] leading-none">⭐</span>
                      <span className="text-[12.5px] font-medium text-zinc-200 truncate flex-1">
                        {cp.name}
                      </span>
                      {active && (
                        <Check size={12} className="text-amber-400 shrink-0" />
                      )}
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        deleteCustom(cp.id);
                      }}
                      disabled={busy}
                      aria-label={t("compress.settings.preset.delete")}
                      className="w-7 h-7 mr-1 rounded-md text-zinc-500 hover:text-rose-400 hover:bg-rose-400/10 opacity-0 group-hover:opacity-100 transition-all flex items-center justify-center"
                    >
                      <Trash2 size={11} />
                    </button>
                  </div>
                );
              })}
            </div>
          )}

          {/* Save as custom */}
          {profileIsCustom && (
            <div className="px-2 py-2 border-t border-white/[0.06]">
              {!saveAsOpen ? (
                <button
                  onClick={() => {
                    setSaveAsName("");
                    setSaveAsOpen(true);
                  }}
                  disabled={busy}
                  className="w-full flex items-center gap-2 px-2.5 py-2 rounded-lg border border-dashed border-amber-400/40 bg-amber-400/[0.04] text-amber-200 hover:bg-amber-400/[0.08] text-[12px] transition-all disabled:opacity-50"
                >
                  <Save size={12} />
                  {t("compress.settings.preset.save")}
                </button>
              ) : (
                <div className="flex items-center gap-1.5 p-2 rounded-lg border border-amber-400/30 bg-amber-400/[0.04]">
                  <input
                    type="text"
                    value={saveAsName}
                    onChange={(e) => setSaveAsName(e.target.value)}
                    placeholder={t("compress.profile.save_as_placeholder")}
                    autoFocus
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        saveAsCustom(saveAsName);
                        setSaveAsOpen(false);
                        setOpen(false);
                      } else if (e.key === "Escape") {
                        setSaveAsOpen(false);
                      }
                    }}
                    className="flex-1 bg-zinc-900/60 border border-zinc-700 rounded px-2 py-1 text-[11.5px] text-zinc-200 placeholder-zinc-500 focus:outline-none focus:border-amber-400"
                  />
                  <button
                    onClick={() => {
                      saveAsCustom(saveAsName);
                      setSaveAsOpen(false);
                      setOpen(false);
                    }}
                    className="px-2.5 py-1 rounded bg-amber-500/20 border border-amber-500/40 text-amber-100 text-[11px] font-medium hover:bg-amber-500/30"
                  >
                    {t("compress.profile.save")}
                  </button>
                </div>
              )}
            </div>
          )}

          {/* Open settings drawer */}
          <div className="px-2 py-2 border-t border-white/[0.06]">
            <button
              onClick={() => {
                setOpen(false);
                onOpenSettings();
              }}
              className="w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-zinc-300 hover:bg-white/[0.04] text-[12px] transition-colors"
            >
              <Settings size={12} className="text-zinc-500" />
              {t("compress.configure")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
