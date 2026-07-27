// Sprint 5.7.21-B-Remodel: preset card for the Compress
// flow. Replaces the old chip bar with a grid of cards
// that show the preset's icon, name, and a one-line
// description. Easier to compare than chips — the user
// can scan the whole row and pick the one that fits.

import { Check } from "lucide-react";
import { useLocale } from "./LocaleProvider";
import {
  type CompressionProfilePreset,
  type CustomProfile,
  type AnyProfile,
} from "@/lib/profiles";
import type { TranslationKey } from "@/lib/i18n";

interface PresetCardProps {
  preset: AnyProfile;
  active: boolean;
  onApply: (p: AnyProfile) => void;
  disabled?: boolean;
}

export function PresetCard({ preset, active, onApply, disabled }: PresetCardProps) {
  const { t } = useLocale();
  // The legacy `AnyProfile` union has `icon` only on the
  // built-in preset variant. Custom profiles use ⭐ as a
  // fallback. We narrow with a runtime check.
  const icon: string =
    "icon" in preset && typeof (preset as { icon: unknown }).icon === "string"
      ? ((preset as { icon: string }).icon)
      : "⭐";
  // Per-preset card description (i18n key derived from
  // preset.id). For custom profiles we don't have a
  // translation key (the user named it themselves), so we
  // use the preset's `name` field directly. The `t()` call
  // is still safe for the built-in path.
  const isCustom = !("icon" in preset);
  const cardDesc: string = isCustom
    ? "" // custom profiles: no description in the card
    : t(`compress.preset.${preset.id}.card` as TranslationKey);
  const displayName: string = isCustom
    ? (preset as CustomProfile).name
    : t(`compress.preset.${preset.id}` as TranslationKey);
  return (
    <button
      onClick={() => onApply(preset)}
      disabled={disabled}
      aria-pressed={active}
      aria-label={`${icon} ${displayName}${cardDesc ? " — " + cardDesc : ""}`}
      data-testid={`preset-${preset.id}`}
      data-active={active}
      className={
        "group relative flex flex-col items-start gap-2 p-4 rounded-2xl border text-left transition-all disabled:opacity-50 " +
        (active
          ? "border-cyan-400/50 bg-cyan-500/10 shadow-[0_0_0_1px_rgba(34,211,238,0.20),0_0_20px_-4px_rgba(34,211,238,0.30)]"
          : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.14] hover:bg-white/[0.04]")
      }
    >
      <div className="flex items-center gap-2 w-full">
        <span className="text-[20px] leading-none">{icon}</span>
        <span
          className={
            "text-[13.5px] font-semibold tracking-tight " +
            (active ? "text-cyan-100" : "text-white")
          }
        >
          {displayName}
        </span>
        {active && (
          <span className="ml-auto shrink-0 w-5 h-5 rounded-full bg-cyan-400 text-black flex items-center justify-center">
            <Check size={11} strokeWidth={3} />
          </span>
        )}
      </div>
      {cardDesc && (
        <div className="text-[11.5px] text-zinc-400 leading-snug">
          {cardDesc}
        </div>
      )}
    </button>
  );
}
