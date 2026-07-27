// Sprint 5.7.21-B-Abstract: settings drawer (slide-in
// panel from the right) that absorbs the previous 5-section
// accordion. The Compress page no longer shows a wall of
// detailed controls in the main flow — the user opens this
// drawer with the "Configure" button to fine-tune the
// compressor.
//
// Design language:
//   - Slide in from the right with a translucent backdrop
//   - Single accent color (cyan) for the active state
//   - 5 vertical sections separated by a hairline, each
//     a self-contained config group
//   - Sections are always open (no nested accordion) — the
//     drawer is for users who want to see everything at once
//   - Closes on Esc, click outside, or X button
//
// The active preset section at the top echoes the chip in
// the main view so the user always knows which preset is
// currently driving the configuration.

import { useEffect, useRef, useState } from "react";
import { useLocale } from "./LocaleProvider";
import {
  X,
  Zap,
  Sparkles,
  FolderTree,
  Sliders,
  Lock,
  Save,
  Trash2,
  Check,
} from "lucide-react";
import { MODES } from "../lib/modes";
import {
  type CompressionProfile,
  type AnyProfile,
  type CustomProfile,
  BUILTIN_PRESETS,
  profilesEqual,
} from "../lib/profiles";
import type { TranslationKey } from "@/lib/i18n";

interface SettingsDrawerProps {
  open: boolean;
  onClose: () => void;
  profile: CompressionProfile;
  updateProfile: (patch: Partial<CompressionProfile>) => void;
  customProfiles: CustomProfile[];
  applyPreset: (p: AnyProfile) => void;
  activePresetId: string | null;
  profileIsCustom: boolean;
  password: string;
  setPassword: (s: string) => void;
  showPwd: boolean;
  setShowPwd: React.Dispatch<React.SetStateAction<boolean>>;
  saveAsCustom: (name: string) => void;
  deleteCustom: (id: string) => void;
  busy: boolean;
}

const CODECS: { id: "auto" | "lzma" | "zstd"; icon: string }[] = [
  { id: "auto", icon: "🪄" },
  { id: "lzma", icon: "💎" },
  { id: "zstd", icon: "⚡" },
];

const FIDELITY: { id: "lossy" | "lossless"; icon: string }[] = [
  { id: "lossy", icon: "✨" },
  { id: "lossless", icon: "🔒" },
];

const CORPUS_MODES: {
  id: "everything" | "source" | "minimal";
  icon: string;
}[] = [
  { id: "everything", icon: "📦" },
  { id: "source", icon: "📝" },
  { id: "minimal", icon: "✨" },
];

function SectionHeader({
  icon,
  title,
  desc,
}: {
  icon: React.ReactNode;
  title: string;
  desc: string;
}) {
  return (
    <div className="mb-3">
      <div className="flex items-center gap-2 mb-1">
        <span className="text-zinc-500">{icon}</span>
        <span className="text-white text-[13px] font-medium tracking-tight">
          {title}
        </span>
      </div>
      <div className="text-zinc-500 text-[11.5px] leading-snug pl-6">
        {desc}
      </div>
    </div>
  );
}

function ChipButton<T extends string>({
  value,
  current,
  onSelect,
  disabled,
  icon,
  label,
  desc,
  accent = "cyan",
}: {
  value: T;
  current: T;
  onSelect: (v: T) => void;
  disabled?: boolean;
  icon: string;
  label: string;
  desc?: string;
  accent?: "cyan" | "violet" | "amber" | "emerald";
}) {
  const active = value === current;
  const accentRing = {
    cyan: "border-cyan-500/40 bg-cyan-500/[0.06]",
    violet: "border-violet-500/40 bg-violet-500/[0.06]",
    amber: "border-amber-500/40 bg-amber-500/[0.06]",
    emerald: "border-emerald-500/40 bg-emerald-500/[0.06]",
  }[accent];
  return (
    <button
      onClick={() => onSelect(value)}
      disabled={disabled}
      className={
        "text-left p-3 rounded-xl border transition-all disabled:opacity-50 " +
        (active
          ? accentRing
          : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]")
      }
    >
      <div className="flex items-center gap-2 mb-1">
        <span className="text-[15px]">{icon}</span>
        <span className="text-white text-[13px] font-medium">{label}</span>
        {active && (
          <Check size={11} className="ml-auto text-emerald-400" strokeWidth={3} />
        )}
      </div>
      {desc && (
        <div className="text-zinc-500 text-[10.5px] leading-snug pl-6">
          {desc}
        </div>
      )}
    </button>
  );
}

function getModeTitle(id: string, t: (k: TranslationKey) => string): string {
  if (id === "rapido") return t("mode.fast.title");
  if (id === "balanceado") return t("mode.balanced.title");
  return t("mode.ultra.title");
}

function getModeDesc(id: string, t: (k: TranslationKey) => string): string {
  if (id === "rapido") return t("mode.fast.desc");
  if (id === "balanceado") return t("mode.balanced.desc");
  return t("mode.ultra.desc");
}

export function SettingsDrawer({
  open,
  onClose,
  profile,
  updateProfile,
  customProfiles,
  applyPreset,
  activePresetId,
  profileIsCustom,
  password,
  setPassword,
  showPwd,
  setShowPwd,
  saveAsCustom,
  deleteCustom,
  busy,
}: SettingsDrawerProps) {
  const { t } = useLocale();
  const panelRef = useRef<HTMLDivElement>(null);
  const [saveAsOpen, setSaveAsOpen] = useState(false);
  const [saveAsName, setSaveAsName] = useState("");

  // Esc to close
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // Lock body scroll while open
  useEffect(() => {
    if (!open) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  }, [open]);

  if (!open) return null;

  // Determine which preset is currently driving the
  // configuration. Used by the "active preset" header in
  // the drawer to show the user where their config came
  // from.
  const activePreset: AnyProfile | undefined =
    BUILTIN_PRESETS.find((p) => p.id === activePresetId) ??
    customProfiles.find((p) => p.id === activePresetId);

  const isCustomActive =
    !!activePreset && !BUILTIN_PRESETS.some((p) => p.id === activePreset.id);

  return (
    <div
      className="fixed inset-0 z-50 flex"
      role="dialog"
      aria-modal="true"
      aria-label={t("compress.settings.title")}
    >
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/40 backdrop-blur-sm"
        onClick={onClose}
        aria-hidden
      />

      {/* Panel */}
      <div
        ref={panelRef}
        className="ml-auto relative h-full w-full max-w-md bg-[#0a0a0a] border-l border-white/[0.06] shadow-2xl overflow-y-auto"
        data-testid="settings-drawer"
      >
        {/* Header */}
        <div className="sticky top-0 z-10 bg-[#0a0a0a]/95 backdrop-blur-md border-b border-white/[0.06]">
          <div className="flex items-center justify-between px-6 py-5">
            <div>
              <div className="text-white text-[16px] font-semibold tracking-tight">
                {t("compress.settings.title")}
              </div>
              <div className="text-zinc-500 text-[12px] mt-0.5">
                {t("compress.settings.subtitle")}
              </div>
            </div>
            <button
              onClick={onClose}
              aria-label={t("compress.settings.close")}
              className="w-9 h-9 rounded-lg flex items-center justify-center text-zinc-500 hover:text-white hover:bg-white/[0.06] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
            >
              <X size={16} />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="px-6 py-6 space-y-10">
          {/* Active preset section */}
          <section>
            <SectionHeader
              icon={<Sparkles size={13} />}
              title={t("compress.settings.section.active")}
              desc={t("compress.profile.desc")}
            />
            <div className="pl-6 space-y-3">
              <div className="flex items-center gap-2">
                {activePreset ? (
                  <div
                    className="flex items-center gap-2 px-3 py-1.5 rounded-full border border-emerald-400/40 bg-emerald-400/[0.06] text-emerald-100"
                    data-testid="active-preset-chip"
                  >
                    <span className="text-[15px] leading-none">
                      {"icon" in activePreset &&
                      typeof (activePreset as { icon?: unknown }).icon === "string"
                        ? (activePreset as { icon: string }).icon
                        : "⭐"}
                    </span>
                    <span className="text-[12.5px] font-medium">
                      {isCustomActive
                        ? (activePreset as CustomProfile).name
                        : t(
                            `compress.preset.${(activePreset as { id: string }).id}` as TranslationKey,
                          )}
                    </span>
                  </div>
                ) : (
                  <div className="text-zinc-500 text-[12px] italic">
                    {t("compress.profile.desc")}
                  </div>
                )}
              </div>

              {/* Built-in presets as a compact list (the
                  chip in the main view is the primary entry
                  point; this is a fallback for users deep
                  in the drawer). */}
              <div className="grid grid-cols-2 gap-1.5">
                {BUILTIN_PRESETS.map((p) => {
                  const active = p.id === activePresetId;
                  return (
                    <button
                      key={p.id}
                      onClick={() => applyPreset(p)}
                      disabled={busy}
                      className={
                        "flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border text-[11.5px] transition-all disabled:opacity-50 " +
                        (active
                          ? "border-emerald-400/40 bg-emerald-400/[0.06] text-emerald-100"
                          : "border-white/[0.06] bg-white/[0.02] text-zinc-300 hover:bg-white/[0.04]")
                      }
                    >
                      <span className="text-[14px] leading-none">{p.icon}</span>
                      <span className="truncate">
                        {t(`compress.preset.${p.id}` as TranslationKey)}
                      </span>
                    </button>
                  );
                })}
              </div>

              {/* Custom profiles */}
              {customProfiles.length > 0 && (
                <div className="pt-2">
                  <div className="text-zinc-500 text-[10px] tracking-[0.15em] uppercase mb-2">
                    {t("compress.settings.preset.custom_section")}
                  </div>
                  <div className="space-y-1">
                    {customProfiles.map((cp) => (
                      <div
                        key={cp.id}
                        className="flex items-center gap-1.5 group"
                      >
                        <button
                          onClick={() => applyPreset(cp)}
                          disabled={busy}
                          className={
                            "flex-1 flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border text-[11.5px] transition-all disabled:opacity-50 text-left " +
                            (cp.id === activePresetId
                              ? "border-amber-400/40 bg-amber-400/[0.06] text-amber-100"
                              : "border-white/[0.06] bg-white/[0.02] text-zinc-300 hover:bg-white/[0.04]")
                          }
                        >
                          <span className="text-[14px] leading-none">⭐</span>
                          <span className="truncate flex-1">{cp.name}</span>
                        </button>
                        <button
                          onClick={() => deleteCustom(cp.id)}
                          disabled={busy}
                          aria-label={t("compress.settings.preset.delete")}
                          className="w-7 h-7 rounded-md text-zinc-500 hover:text-rose-400 hover:bg-rose-400/10 opacity-0 group-hover:opacity-100 transition-all flex items-center justify-center"
                        >
                          <Trash2 size={11} />
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Save current as custom */}
              {profileIsCustom && !saveAsOpen && (
                <button
                  onClick={() => {
                    setSaveAsName("");
                    setSaveAsOpen(true);
                  }}
                  disabled={busy}
                  className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border border-dashed border-amber-400/40 bg-amber-400/[0.04] text-amber-200 hover:bg-amber-400/[0.08] text-[11.5px] transition-all disabled:opacity-50"
                >
                  <Save size={11} />
                  {t("compress.settings.preset.save")}
                </button>
              )}
              {saveAsOpen && (
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
                    }}
                    className="px-2.5 py-1 rounded bg-amber-500/20 border border-amber-500/40 text-amber-100 text-[11px] font-medium hover:bg-amber-500/30"
                  >
                    {t("compress.profile.save")}
                  </button>
                  <button
                    onClick={() => setSaveAsOpen(false)}
                    className="px-2 py-1 rounded text-zinc-400 hover:text-zinc-200 text-[11px]"
                  >
                    ✕
                  </button>
                </div>
              )}
            </div>
          </section>

          {/* Speed section */}
          <section>
            <SectionHeader
              icon={<Zap size={13} />}
              title={t("compress.section.speed")}
              desc={t("compress.section.speed.desc")}
            />
            <div className="pl-6 space-y-4">
              <div>
                <div className="text-zinc-400 text-[10.5px] uppercase tracking-wider mb-2">
                  {t("compress.mode")}
                </div>
                <div className="grid grid-cols-3 gap-2">
                  {MODES.map((m) => {
                    const active = profile.mode === m.id;
                    return (
                      <button
                        key={m.id}
                        onClick={() => updateProfile({ mode: m.id })}
                        disabled={busy}
                        title={t(`compress.mode.${m.id}.tooltip` as TranslationKey)}
                        className={
                          "text-left p-3 rounded-xl border transition-all disabled:opacity-50 " +
                          (active
                            ? "border-cyan-500/40 bg-cyan-500/[0.06]"
                            : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]")
                        }
                      >
                        <div className="flex items-center gap-1.5 mb-1">
                          <span className="text-[15px]">{m.icon}</span>
                          <span className="text-white text-[12.5px] font-medium">
                            {getModeTitle(m.id, t)}
                          </span>
                        </div>
                        <div className="text-zinc-500 text-[10px] leading-snug">
                          {getModeDesc(m.id, t)}
                        </div>
                      </button>
                    );
                  })}
                </div>
              </div>
              <div>
                <div className="text-zinc-400 text-[10.5px] uppercase tracking-wider mb-2">
                  {t("compress.codec")}
                </div>
                <div className="grid grid-cols-3 gap-2">
                  {CODECS.map((c) => (
                    <ChipButton
                      key={c.id}
                      value={c.id}
                      current={profile.codec}
                      onSelect={(v) => updateProfile({ codec: v })}
                      disabled={busy}
                      icon={c.icon}
                      label={t(`compress.codec.${c.id}` as TranslationKey)}
                      desc={t(`compress.codec.${c.id}.desc` as TranslationKey)}
                      accent="violet"
                    />
                  ))}
                </div>
              </div>
            </div>
          </section>

          {/* Quality section */}
          <section>
            <SectionHeader
              icon={<Sparkles size={13} />}
              title={t("compress.section.quality")}
              desc={t("compress.section.quality.desc")}
            />
            <div className="pl-6">
              <div className="grid grid-cols-2 gap-2">
                {FIDELITY.map((f) => (
                  <ChipButton
                    key={f.id}
                    value={f.id}
                    current={profile.fidelity}
                    onSelect={(v) => updateProfile({ fidelity: v })}
                    disabled={busy}
                    icon={f.icon}
                    label={t(`compress.fidelity.${f.id}` as TranslationKey)}
                    desc={t(`compress.fidelity.${f.id}.desc` as TranslationKey)}
                    accent="amber"
                  />
                ))}
              </div>
            </div>
          </section>

          {/* Content section */}
          <section>
            <SectionHeader
              icon={<FolderTree size={13} />}
              title={t("compress.section.corpus")}
              desc={t("compress.section.corpus.desc")}
            />
            <div className="pl-6 space-y-2">
              {CORPUS_MODES.map((m) => {
                const active = profile.corpusMode === m.id;
                return (
                  <ChipButton
                    key={m.id}
                    value={m.id}
                    current={profile.corpusMode}
                    onSelect={(v) => updateProfile({ corpusMode: v })}
                    disabled={busy}
                    icon={m.icon}
                    label={t(`compress.corpus.${m.id}` as TranslationKey)}
                    desc={t(`compress.corpus.${m.id}.desc` as TranslationKey)}
                    accent="emerald"
                  />
                );
              })}
            </div>
          </section>

          {/* Advanced section */}
          <section>
            <SectionHeader
              icon={<Sliders size={13} />}
              title={t("compress.section.advanced")}
              desc={t("compress.section.advanced.desc")}
            />
            <div className="pl-6 space-y-3">
              <div>
                <label className="block text-zinc-400 text-[10.5px] uppercase tracking-wider mb-1.5">
                  {t("compress.advanced.raw.label")}
                </label>
                <input
                  type="text"
                  value={profile.rawExtensions}
                  onChange={(e) =>
                    updateProfile({ rawExtensions: e.target.value })
                  }
                  placeholder={t("compress.advanced.raw.placeholder")}
                  disabled={busy || profile.fidelity === "lossless"}
                  className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 disabled:opacity-40"
                />
              </div>
              <div>
                <label className="block text-zinc-400 text-[10.5px] uppercase tracking-wider mb-1.5">
                  {t("compress.advanced.minify.label")}
                </label>
                <input
                  type="text"
                  value={profile.minifyExtensions}
                  onChange={(e) =>
                    updateProfile({ minifyExtensions: e.target.value })
                  }
                  placeholder={t("compress.advanced.minify.placeholder")}
                  disabled={busy || profile.fidelity === "lossless"}
                  className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 disabled:opacity-40"
                />
              </div>
              {profile.fidelity === "lossless" && (
                <div className="text-zinc-500 text-[10.5px] italic">
                  {t("compress.advanced.disabled_lossless")}
                </div>
              )}
            </div>
          </section>

          {/* Security section */}
          <section>
            <SectionHeader
              icon={<Lock size={13} />}
              title={t("compress.section.security")}
              desc={t("compress.section.security.desc")}
            />
            <div className="pl-6 space-y-3">
              <label className="flex items-center justify-between p-3 rounded-xl border border-white/[0.06] bg-white/[0.02] cursor-pointer">
                <div>
                  <div className="text-white text-[12.5px] font-medium">
                    {t("compress.encrypt.label")}
                  </div>
                  <div className="text-zinc-500 text-[10.5px] leading-snug">
                    {t("compress.encrypt.desc")}
                  </div>
                </div>
                <input
                  type="checkbox"
                  checked={profile.encrypt}
                  onChange={(e) =>
                    updateProfile({ encrypt: e.target.checked })
                  }
                  disabled={busy}
                  className="w-4 h-4 accent-cyan-500 cursor-pointer disabled:opacity-50"
                />
              </label>
              {profile.encrypt && (
                <>
                  <div>
                    <label className="block text-zinc-400 text-[10.5px] uppercase tracking-wider mb-1.5">
                      {t("compress.password.label")}
                    </label>
                    <div className="relative">
                      <input
                        type={showPwd ? "text" : "password"}
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        placeholder={t("compress.password.placeholder")}
                        autoComplete="new-password"
                        spellCheck={false}
                        disabled={busy}
                        className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-2 pr-9 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 disabled:opacity-50"
                      />
                      <button
                        type="button"
                        onClick={() => setShowPwd((s) => !s)}
                        tabIndex={-1}
                        disabled={busy}
                        className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-200 text-[10px] uppercase tracking-wider px-1.5 py-0.5"
                      >
                        {showPwd
                          ? t("compress.password.hide")
                          : t("compress.password.show")}
                      </button>
                    </div>
                  </div>
                  <div>
                    <div className="text-zinc-400 text-[10.5px] uppercase tracking-wider mb-1.5">
                      {t("compress.recovery.label")}
                    </div>
                    <div className="grid grid-cols-3 gap-1.5">
                      {(
                        [
                          { v: "off", label: "off", desc: "0% overhead" },
                          { v: "low", label: "low", desc: "1 / 10 files" },
                          {
                            v: "high",
                            label: "high",
                            desc: "2-3 / 8 files",
                          },
                        ] as const
                      ).map((opt) => {
                        const active = profile.recoveryLevel === opt.v;
                        return (
                          <button
                            key={opt.v}
                            onClick={() =>
                              updateProfile({ recoveryLevel: opt.v })
                            }
                            disabled={busy}
                            className={
                              "p-2.5 rounded-lg border transition-all disabled:opacity-50 " +
                              (active
                                ? "border-cyan-500/40 bg-cyan-500/[0.06]"
                                : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]")
                            }
                          >
                            <div className="text-white text-[12px] font-medium">
                              {opt.label}
                            </div>
                            <div className="text-zinc-500 text-[10px]">
                              {opt.desc}
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </>
              )}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
