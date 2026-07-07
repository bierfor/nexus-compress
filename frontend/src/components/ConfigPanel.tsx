"use client";

import { useEffect, useState } from "react";
import { useLocale } from "@/components/LocaleProvider";

// ─── Types (still exported so CompressView can import them) ───

export type Mode = "v4" | "v5-min" | "v6-solid";
export type Strength = "fast" | "balanced" | "max";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri) return {} as T;
  return (window as any).__TAURI_INTERNALS__.invoke(cmd, args);
}

// ─── Mode metadata ─────────────────────────────────────────────

interface ModeMeta {
  id: Mode;
  icon: string;
  titleKey: "mode.fast.title" | "mode.balanced.title" | "mode.ultra.title";
  descKey: "mode.fast.desc" | "mode.balanced.desc" | "mode.ultra.desc";
  tag: "LOSSLESS" | "TEXT-MIN" | "AST+LZMA";
  tagColor: string;
  activeRing: string;
  activeBg: string;
  activeDot: string;
}

const MODES: ModeMeta[] = [
  {
    id: "v4",
    icon: "⚡",
    titleKey: "mode.fast.title",
    descKey: "mode.fast.desc",
    tag: "LOSSLESS",
    tagColor: "text-cyan-400 bg-cyan-500/10 border-cyan-500/30",
    activeRing: "border-cyan-500/50",
    activeBg: "bg-cyan-500/[0.07]",
    activeDot: "bg-cyan-400",
  },
  {
    id: "v5-min",
    icon: "✂️",
    titleKey: "mode.balanced.title",
    descKey: "mode.balanced.desc",
    tag: "TEXT-MIN",
    tagColor: "text-amber-400 bg-amber-500/10 border-amber-500/30",
    activeRing: "border-amber-500/50",
    activeBg: "bg-amber-500/[0.07]",
    activeDot: "bg-amber-400",
  },
  {
    id: "v6-solid",
    icon: "💎",
    titleKey: "mode.ultra.title",
    descKey: "mode.ultra.desc",
    tag: "AST+LZMA",
    tagColor: "text-violet-400 bg-violet-500/10 border-violet-500/30",
    activeRing: "border-violet-500/50",
    activeBg: "bg-violet-500/[0.07]",
    activeDot: "bg-violet-400",
  },
];

// ─── Tunnel mode descriptions (per locale via i18n keys) ─────

interface TunnelModeMeta {
  id: "quick" | "named" | "direct";
  titleKey: string;
  descKey: string;
  badgeKey: string;
  icon: string;
  badgeColor: string;
}

const TUNNEL_MODES: TunnelModeMeta[] = [
  {
    id: "quick",
    titleKey: "settings.tunnel.mode.quick",
    descKey: "settings.tunnel.mode.quick.desc",
    badgeKey: "settings.tunnel.mode.quick.badge",
    icon: "☁️",
    badgeColor: "text-amber-300 bg-amber-500/10 border-amber-500/30",
  },
  {
    id: "named",
    titleKey: "settings.tunnel.mode.named",
    descKey: "settings.tunnel.mode.named.desc",
    badgeKey: "settings.tunnel.mode.named.badge",
    icon: "🔒",
    badgeColor: "text-cyan-300 bg-cyan-500/10 border-cyan-500/30",
  },
  {
    id: "direct",
    titleKey: "settings.tunnel.mode.direct",
    descKey: "settings.tunnel.mode.direct.desc",
    badgeKey: "settings.tunnel.mode.direct.badge",
    icon: "📡",
    badgeColor: "text-emerald-300 bg-emerald-500/10 border-emerald-500/30",
  },
];

// ─── Main component ─────────────────────────────────────────────

export function ConfigPanel({
  mode: modeProp,
  strength: strengthProp,
  onModeChange,
  onStrengthChange,
  onSelfTest,
}: {
  mode?: Mode;
  strength?: Strength;
  onModeChange?: (m: Mode) => void;
  onStrengthChange?: (s: Strength) => void;
  onSelfTest?: () => void;
} = {}) {
  const { locale, setLocale, t } = useLocale();

  const [modeInternal, setModeInternal] = useState<Mode>("v5-min");
  const [strengthInternal, setStrengthInternal] = useState<Strength>("balanced");
  const [selfTestStatus, setSelfTestStatus] = useState<"idle" | "busy" | "ok" | "err">("idle");
  const [selfTestMsg, setSelfTestMsg] = useState("");

  const mode = modeProp ?? modeInternal;
  const strength = strengthProp ?? strengthInternal;

  const setMode = (m: Mode) => {
    setModeInternal(m);
    onModeChange?.(m);
  };
  const setStrength = (s: Strength) => {
    setStrengthInternal(s);
    onStrengthChange?.(s);
  };

  const handleSelfTest = async () => {
    setSelfTestStatus("busy");
    setSelfTestMsg("");
    try {
      if (onSelfTest) await onSelfTest();
      else await tauriInvoke("self_test_cmd", {});
      setSelfTestStatus("ok");
      setSelfTestMsg(t("settings.compression.selftest.ok"));
    } catch (e: any) {
      setSelfTestStatus("err");
      setSelfTestMsg(`✗ ${e?.message ?? e}`);
    }
  };

  const strengthLabel = (s: Strength) => {
    if (mode === "v4") return s === "max" ? "Premium" : "Fast";
    return s === "fast" ? "−1" : s === "balanced" ? "−6" : "−9";
  };

  return (
    <div className="space-y-7">
      {/* ── Language ──────────────────────────────────────────── */}
      <Section
        title={t("settings.section.language")}
        hint="Cambia el idioma de toda la interfaz."
      >
        <div className="px-5 py-4 flex items-center gap-3 flex-wrap">
          {(["es", "en", "it"] as const).map((lang) => {
            const active = locale === lang;
            const flag = lang === "es" ? "🇪🇸" : lang === "en" ? "🇬🇧" : "🇮🇹";
            const langLabel = lang === "es"
              ? t("settings.language.es")
              : lang === "en"
                ? t("settings.language.en")
                : t("settings.language.it");
            return (
              <button
                key={lang}
                onClick={() => setLocale(lang)}
                className={`px-6 py-2.5 rounded-xl text-[13px] font-medium transition-all ${
                  active
                    ? "bg-cyan-500/20 border border-cyan-500/40 text-cyan-300 shadow-sm shadow-cyan-500/10"
                    : "bg-white/[0.04] border border-white/[0.08] text-zinc-400 hover:text-white hover:border-white/[0.18]"
                }`}
              >
                {flag}  {langLabel}
              </button>
            );
          })}
        </div>
      </Section>

      {/* ── Compression mode ──────────────────────────────────── */}
      <Section
        title={t("settings.section.compression")}
        hint="Modo por defecto al comprimir. Se puede cambiar en cada compresión."
      >
        <div className="p-4 grid grid-cols-3 gap-3">
          {MODES.map((m) => {
            const active = mode === m.id;
            return (
              <button
                key={m.id}
                onClick={() => setMode(m.id)}
                className={`relative text-left p-4 rounded-2xl border transition-all ${
                  active
                    ? `${m.activeBg} ${m.activeRing}`
                    : "bg-white/[0.02] border-white/[0.06] hover:border-white/[0.14] hover:bg-white/[0.04]"
                }`}
              >
                {active && (
                  <span className={`absolute top-3 right-3 w-2 h-2 rounded-full ${m.activeDot}`} />
                )}
                <div className="text-2xl mb-3">{m.icon}</div>
                <p className="text-white text-[14px] font-semibold mb-1 leading-tight">
                  {t(m.titleKey)}
                </p>
                <p className="text-zinc-500 text-[11.5px] leading-relaxed mb-3 min-h-[2.5em]">
                  {t(m.descKey)}
                </p>
                <span
                  className={`inline-flex items-center px-2 py-0.5 rounded-md border text-[10px] font-mono tracking-wider ${m.tagColor}`}
                >
                  {m.tag}
                </span>
              </button>
            );
          })}
        </div>

        {/* Strength */}
        <div className="px-5 pb-4 border-t border-white/[0.04] pt-4">
          <p className="text-zinc-500 text-[11px] tracking-[0.15em] uppercase mb-3">
            {t("settings.compression.strength")}
          </p>
          <div className="flex gap-2">
            {(["fast", "balanced", "max"] as Strength[]).map((s) => {
              const active = strength === s;
              return (
                <button
                  key={s}
                  onClick={() => setStrength(s)}
                  className={`flex-1 py-2.5 rounded-xl text-[13px] font-mono font-medium transition-all ${
                    active
                      ? "bg-cyan-500/20 border border-cyan-500/40 text-cyan-300"
                      : "bg-white/[0.03] border border-white/[0.08] text-zinc-500 hover:text-zinc-200 hover:border-white/[0.18]"
                  }`}
                >
                  {strengthLabel(s)}
                </button>
              );
            })}
          </div>
        </div>

        {/* Self-test */}
        <div className="px-5 pb-5 pt-2">
          <button
            onClick={handleSelfTest}
            disabled={selfTestStatus === "busy"}
            className={`w-full py-2.5 rounded-xl text-[13px] font-medium transition-all border ${
              selfTestStatus === "ok"
                ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300"
                : selfTestStatus === "err"
                ? "bg-red-500/10 border-red-500/30 text-red-400"
                : "bg-white/[0.04] border-white/[0.08] text-zinc-300 hover:text-white hover:border-white/[0.18]"
            } disabled:opacity-50`}
          >
            {selfTestStatus === "busy"
              ? t("settings.compression.selftest.busy")
              : selfTestMsg || t("settings.compression.selftest")}
          </button>
        </div>
      </Section>

      {/* ── P2P Tunnel ────────────────────────────────────────── */}
      <Section
        title={t("settings.section.tunnel")}
        hint="Cómo se establece la conexión entre dispositivos al enviar un archivo."
      >
        <TunnelPanel />
      </Section>

      {/* ── About ─────────────────────────────────────────────── */}
      <Section
        title={t("settings.section.about")}
        hint="Información técnica y enlaces."
      >
        <InfoRow label={t("settings.about.version")} value="0.6.20" mono />
        <InfoRow label={t("settings.about.engine")} value="NexusCompress v6 Solid-AST" mono />
        <InfoRow
          label="GitHub"
          value="github.com/bierfor/nexus-compress"
          mono
          href="https://github.com/bierfor/nexus-compress"
        />
        <InfoRow label="Localización" value={locale.toUpperCase()} mono />
      </Section>
    </div>
  );
}

// ─── Section wrapper ────────────────────────────────────────────

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="flex items-baseline justify-between mb-3 px-1 gap-4">
        <p className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase">
          {title}
        </p>
        {hint && (
          <p className="text-zinc-600 text-[11px] leading-snug text-right flex-1 max-w-md">
            {hint}
          </p>
        )}
      </div>
      <div className="rounded-2xl bg-white/[0.03] border border-white/[0.06] overflow-hidden divide-y divide-white/[0.04]">
        {children}
      </div>
    </div>
  );
}

// ─── Info row ───────────────────────────────────────────────────

function InfoRow({
  label,
  value,
  mono,
  href,
}: {
  label: string;
  value: string;
  mono?: boolean;
  href?: string;
}) {
  const content = (
    <>
      <span className="text-zinc-400 text-[13px]">{label}</span>
      <span
        className={`text-white text-[13px] ${mono ? "font-mono" : "font-medium"} ${
          href ? "hover:text-cyan-300 transition-colors" : ""
        }`}
      >
        {value}
      </span>
    </>
  );
  return (
    <div className="px-5 py-3.5 flex items-center justify-between">
      {href ? (
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="flex items-center justify-between w-full"
        >
          {content}
        </a>
      ) : (
        <div className="flex items-center justify-between w-full">{content}</div>
      )}
    </div>
  );
}

// ─── Tunnel panel ───────────────────────────────────────────────

function TunnelPanel() {
  const { t, locale } = useLocale();
  const [mode, setMode] = useState<"quick" | "named" | "direct">("quick");
  const [hostname, setHostname] = useState("");
  const [token, setToken] = useState("");
  const [hasToken, setHasToken] = useState(false);
  const [status, setStatus] = useState<"" | "ok" | "err">("");
  const [statusMsg, setStatusMsg] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    tauriInvoke<{ mode: string; hostname: string | null; has_token: boolean }>(
      "p2p_get_tunnel_config_cmd"
    )
      .then((r) => {
        setMode(r.mode as "quick" | "named" | "direct");
        setHostname(r.hostname ?? "");
        setHasToken(r.has_token);
      })
      .catch(() => {});
  }, []);

  const onSave = async () => {
    setBusy(true);
    setStatus("");
    setStatusMsg("");
    try {
      await tauriInvoke("p2p_save_tunnel_config_cmd", {
        req: { mode, hostname: mode === "named" ? hostname : null, token: token || null },
      });
      const r = await tauriInvoke<{ has_token: boolean }>("p2p_get_tunnel_config_cmd");
      setHasToken(r.has_token);
      setToken("");
      setStatus("ok");
      setStatusMsg("✓ " + t("settings.tunnel.save"));
      // Auto-clear after 3 seconds
      setTimeout(() => {
        setStatus("");
        setStatusMsg("");
      }, 3000);
    } catch (e: any) {
      setStatus("err");
      setStatusMsg(`✗ ${e?.message ?? e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      {/* Mode selector */}
      <div className="px-5 py-4">
        <label className="text-zinc-500 text-[11px] tracking-[0.15em] uppercase block mb-3">
          {t("settings.tunnel.mode")}
        </label>
        <div className="flex flex-col gap-2">
          {TUNNEL_MODES.map((m) => {
            const active = mode === m.id;
            const disabled = m.id === "direct";
            const title = t(m.titleKey as any);
            // Inline descriptions in the user's current language.
            // Future: move these into the i18n dictionary under
            // settings.tunnel.mode.{quick,named,direct}.desc.
            const desc =
              locale === "en"
                ? m.id === "quick"
                  ? "Anonymous. Works without configuration. Ideal for trying it out. Limited by Cloudflare rate-limits (~3-5 connections before errors)."
                  : m.id === "named"
                    ? "Requires a free Cloudflare account + your own tunnel. No rate-limits, fixed domain (e.g. p2p.your-domain.com)."
                    : "Direct LAN/Wi-Fi connection. No intermediary server. Only works on the same network."
                : locale === "it"
                ? m.id === "quick"
                  ? "Anonimo. Funziona senza configurazione. Ideale per provare. Limitato dai rate-limit di Cloudflare (~3-5 connessioni prima di errori)."
                  : m.id === "named"
                    ? "Richiede account Cloudflare gratuito + tunnel proprio. Nessun rate-limit, dominio fisso (es. p2p.tuo-dominio.com)."
                    : "Connessione diretta LAN/Wi-Fi. Nessun server intermedio. Funziona solo sulla stessa rete."
                : m.id === "quick"
                ? "Anónimo. Funciona sin configurar nada. Ideal para probar. Limitado por rate-limits de Cloudflare (~3-5 conexiones antes de errores)."
                : m.id === "named"
                ? "Requiere cuenta Cloudflare gratuita + tunnel propio. Sin rate-limits, dominio fijo (ej. p2p.tu-dominio.com)."
                : "Conexión directa LAN/Wi-Fi. Sin servidor intermedio. Solo funciona en la misma red.";
            const badge =
              locale === "en"
                ? m.id === "quick"
                  ? "No setup"
                  : m.id === "named"
                    ? "Cloudflare"
                    : "Default"
                : locale === "it"
                ? m.id === "quick"
                  ? "No config"
                  : m.id === "named"
                    ? "Cloudflare"
                    : "Default"
                : m.id === "quick"
                ? "Sin config"
                : m.id === "named"
                ? "Cloudflare"
                : "Default";
            return (
              <button
                key={m.id}
                onClick={() => !disabled && setMode(m.id)}
                disabled={disabled}
                className={`flex items-start gap-3 px-4 py-3 rounded-xl border text-left text-[13px] transition-all ${
                  active && !disabled
                    ? "bg-cyan-500/10 border-cyan-500/40 text-white"
                    : disabled
                    ? "bg-white/[0.02] border-white/[0.04] text-zinc-700 cursor-not-allowed"
                    : "bg-white/[0.03] border-white/[0.06] text-zinc-400 hover:text-white hover:border-white/[0.14]"
                }`}
              >
                <span className="text-2xl shrink-0 mt-0.5">{m.icon}</span>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className="font-semibold">{title}</span>
                    <span
                      className={`px-1.5 py-0.5 rounded text-[9.5px] font-mono tracking-wider border ${m.badgeColor}`}
                    >
                      {badge}
                    </span>
                  </div>
                  <p className="text-zinc-500 text-[11.5px] leading-relaxed">{desc}</p>
                </div>
                {active && !disabled && (
                  <span className="w-2 h-2 rounded-full shrink-0 mt-1.5 bg-cyan-400" />
                )}
              </button>
            );
          })}
        </div>
      </div>

      {/* Named-mode fields */}
      {mode === "named" && (
        <>
          <div className="px-5 py-4">
            <label className="text-zinc-500 text-[11px] tracking-[0.15em] uppercase block mb-2">
              {t("settings.tunnel.hostname")}
            </label>
            <input
              type="text"
              value={hostname}
              onChange={(e) => setHostname(e.target.value)}
              placeholder="p2p.example.com"
              disabled={busy}
              className="w-full bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-2.5 text-[13px] text-white placeholder-zinc-600 focus:outline-none focus:border-cyan-500/40 font-mono"
            />
          </div>
          <div className="px-5 py-4">
            <label className="text-zinc-500 text-[11px] tracking-[0.15em] uppercase block mb-2 flex items-center gap-2">
              {t("settings.tunnel.token")}
              {hasToken && (
                <span className="ml-2 inline-flex items-center gap-1 text-emerald-400 text-[10px] normal-case tracking-normal font-medium">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
                  {t("settings.tunnel.token.saved")}
                </span>
              )}
            </label>
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder={
                hasToken ? t("settings.tunnel.token.replace") : t("settings.tunnel.token.new")
              }
              disabled={busy}
              autoComplete="off"
              className="w-full bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-2.5 text-[13px] text-white placeholder-zinc-600 focus:outline-none focus:border-cyan-500/40 font-mono"
            />
          </div>
        </>
      )}

      {/* Save */}
      <div className="px-5 py-4 flex items-center gap-3">
        <button
          onClick={onSave}
          disabled={busy || (mode === "named" && !hostname)}
          className={`flex-1 py-2.5 rounded-xl text-[13px] font-medium transition-all border ${
            status === "ok"
              ? "bg-emerald-500/20 border-emerald-500/40 text-emerald-300"
              : "bg-cyan-500/20 border border-cyan-500/30 text-cyan-300 hover:bg-cyan-500/30 hover:border-cyan-500/50"
          } disabled:opacity-40 disabled:cursor-not-allowed`}
        >
          {busy
            ? t("settings.tunnel.save.busy")
            : status === "ok"
              ? statusMsg
              : t("settings.tunnel.save")}
        </button>
        {status === "err" && statusMsg && (
          <span className="text-[12px] font-mono shrink-0 text-red-400">{statusMsg}</span>
        )}
      </div>

      {/* Note */}
      <div className="px-5 py-4 bg-white/[0.01]">
        <p className="text-zinc-600 text-[12px] leading-relaxed">
          {t("settings.tunnel.note")}
        </p>
      </div>
    </>
  );
}