"use client";

/**
 * The Config Panel — the user-facing controls.
 *
 * Three compression modes are exposed as radio cards:
 *   - v4         : lossless, multi-stream LZ77 + rANS + trained dict
 *   - v5 Text-Min: LZMA + conservative text minify (strips comments)
 *   - v6 Solid-AST: swc AST minify + LZMA, with a single-stream
 *     SOLID variant for directories (dictionary spans whole corpus).
 *     For a single file the same backend gives AST + LZMA.
 *
 * Each card shows a one-line tagline, the lossy/lossless contract,
 * and a "ratio hint" — the approximate ratio as % of 7z from
 * `bench_v6` / `bench_solid` (corpus_real/).
 *
 * The strength selector maps to the right knob per mode:
 *   - v4        : LZ77 strategy (Fast / Premium)
 *   - v5 / v6   : LZMA preset (1 / 6 / 9)
 */

export type Mode = "v4" | "v5-min" | "v6-solid";
export type Strength = "fast" | "balanced" | "max";

interface ModeMeta {
  id: Mode;
  name: string;
  subtitle: string;
  contract: string;
  pctOf7z: number; // rough ratio hint, -1 if not measured
  lossy: boolean;
  accent: "cyan" | "matrix" | "magenta" | "amber";
}

const MODES: ModeMeta[] = [
  {
    id: "v4",
    name: "v4",
    subtitle: "LOSSLESS",
    contract: "multi-stream LZ77 + rANS + 5348-entry dict. 100% byte-identical roundtrip.",
    pctOf7z: 137,
    lossy: false,
    accent: "matrix",
  },
  {
    id: "v5-min",
    name: "v5 Text-Min",
    subtitle: "TEXT MINIFY",
    contract: "Strips comments, collapses whitespace, then LZMA. Best on minified bundles (kills sourcemap comments).",
    pctOf7z: 162,
    lossy: true,
    accent: "amber",
  },
  {
    id: "v6-solid",
    name: "v6 Solid-AST",
    subtitle: "AST + SOLID LZMA",
    contract: "swc AST minify on .js/.ts/tsx (drops types, comments, formatting) + LZMA. Directory mode = single stream over the whole corpus.",
    pctOf7z: 117,
    lossy: true,
    accent: "magenta",
  },
];

const ACCENT_TEXT: Record<ModeMeta["accent"], string> = {
  cyan: "text-cyan-400",
  matrix: "text-matrix-500",
  magenta: "text-magenta-400",
  amber: "text-amber-400",
};
const ACCENT_BORDER: Record<ModeMeta["accent"], string> = {
  cyan: "border-cyan-500",
  matrix: "border-matrix-500",
  magenta: "border-magenta-500",
  amber: "border-amber-500",
};
const ACCENT_BG: Record<ModeMeta["accent"], string> = {
  cyan: "bg-cyan-500/10",
  matrix: "bg-matrix-500/10",
  magenta: "bg-magenta-500/10",
  amber: "bg-amber-500/10",
};

export function ConfigPanel({
  mode,
  strength,
  onModeChange,
  onStrengthChange,
  onSelfTest,
}: {
  mode: Mode;
  strength: Strength;
  onModeChange: (mode: Mode) => void;
  onStrengthChange: (s: Strength) => void;
  onSelfTest: () => void;
}) {
  const selected = MODES.find((m) => m.id === mode)!;
  return (
    <div className="panel p-3 flex flex-col gap-3">
      <div className="metric-label">compression mode</div>

      <div className="grid grid-cols-2 gap-2">
        {MODES.map((m) => {
          const active = m.id === mode;
          return (
            <button
              key={m.id}
              onClick={() => onModeChange(m.id)}
              className={[
                "relative flex flex-col items-start gap-1 p-2 border text-left",
                "transition-all duration-150 font-mono",
                active
                  ? `${ACCENT_BORDER[m.accent]} ${ACCENT_BG[m.accent]} ${ACCENT_TEXT[m.accent]}`
                  : "border-zinc-800 text-zinc-400 hover:border-zinc-600 hover:text-zinc-200",
              ].join(" ")}
              data-tauri-drag-region={false}
            >
              <div className="flex items-center gap-2 w-full">
                <span
                  className={[
                    "inline-block w-1.5 h-1.5",
                    active ? ACCENT_TEXT[m.accent] : "text-zinc-700",
                  ].join(" ")}
                >
                  {active ? "●" : "○"}
                </span>
                <span className="text-xs font-bold tracking-wider">
                  {m.name}
                </span>
                <span
                  className={[
                    "ml-auto text-[8px] tracking-widest px-1 border",
                    active
                      ? `${ACCENT_BORDER[m.accent]} ${ACCENT_TEXT[m.accent]}`
                      : "border-zinc-700 text-zinc-600",
                  ].join(" ")}
                >
                  {m.lossy ? "LOSSY" : "LOSSLESS"}
                </span>
              </div>
              <div className="text-[9px] tracking-[0.2em] uppercase text-zinc-500">
                {m.subtitle}
              </div>
              <div
                className={[
                  "text-[9px] font-mono mt-0.5",
                  active ? ACCENT_TEXT[m.accent] : "text-zinc-600",
                ].join(" ")}
              >
                {m.pctOf7z > 0 ? `${m.pctOf7z}% of 7z` : "—"}
              </div>
            </button>
          );
        })}
      </div>

      <div className="text-[10px] font-mono text-zinc-400 leading-relaxed border-l-2 border-zinc-800 pl-2">
        {selected.contract}
      </div>

      <div className="border-t border-bg-border pt-2">
        <div className="metric-label mb-1">
          {mode === "v4" ? "lz77 strategy" : "lzma level"}
        </div>
        <div className="grid grid-cols-3 gap-1">
          {(["fast", "balanced", "max"] as Strength[]).map((s) => {
            const active = s === strength;
            const label =
              mode === "v4"
                ? s === "fast"
                  ? "FAST"
                  : s === "balanced"
                  ? "FAST"
                  : "PREMIUM"
                : s === "fast"
                ? "-1"
                : s === "balanced"
                ? "-6"
                : "-9";
            return (
              <button
                key={s}
                onClick={() => onStrengthChange(s)}
                className={[
                  "px-2 py-1 border text-[10px] font-mono tracking-wider uppercase",
                  "transition-all duration-150",
                  active
                    ? `border-cyan-500 text-cyan-400 bg-cyan-500/10`
                    : "border-zinc-800 text-zinc-500 hover:border-zinc-600 hover:text-zinc-300",
                ].join(" ")}
              >
                {label}
              </button>
            );
          })}
        </div>
        <div className="text-[9px] font-mono text-zinc-600 mt-1">
          {mode === "v4"
            ? strength === "max"
              ? "Optimal DP — ~8× slower, ~0% gain. Kept for experimentation."
              : "Lazy LZ77 + entropy gatekeeper. Recommended."
            : strength === "fast"
            ? "LZMA preset 1 — fast, lower ratio"
            : strength === "balanced"
            ? "LZMA preset 6 — default, balanced"
            : "LZMA preset 9 — max ratio (apples-to-apples with 7z -mx=9)"}
        </div>
      </div>

      <button
        onClick={onSelfTest}
        className="btn flex-1 mt-1"
      >
        self-test
      </button>

      <TunnelSettingsPanel />
    </div>
  );
}

// ============================================================================
//  TunnelSettingsPanel — Sprint 5.5.1
// ============================================================================
//
// Configures the P2P tunnel transport mode and the Cloudflare
// tunnel credentials. The token is stored in the OS keyring
// (Keychain on macOS, Credential Manager on Windows, Secret
// Service on Linux) — never on disk. The hostname is a public
// field saved in `nexus_config.json` inside the app data dir.

import { useEffect, useState } from "react";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  if (!isTauri) {
    console.log(`[stub] invoke ${cmd}`, args);
    return {} as T;
  }
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

function TunnelSettingsPanel() {
  const [mode, setMode] = useState<"quick" | "named" | "direct">("quick");
  const [hostname, setHostname] = useState<string>("");
  const [token, setToken] = useState<string>("");
  const [hasToken, setHasToken] = useState<boolean>(false);
  const [status, setStatus] = useState<string>("");
  const [busy, setBusy] = useState<boolean>(false);

  // Load current config on mount.
  useEffect(() => {
    tauriInvoke<{ mode: string; hostname: string | null; has_token: boolean }>(
      "p2p_get_tunnel_config_cmd"
    )
      .then((r) => {
        setMode(r.mode as "quick" | "named" | "direct");
        setHostname(r.hostname ?? "");
        setHasToken(r.has_token);
      })
      .catch((e) => setStatus(`load error: ${e}`));
  }, []);

  const onSave = async () => {
    setBusy(true);
    setStatus("");
    try {
      await tauriInvoke("p2p_save_tunnel_config_cmd", {
        req: {
          mode,
          hostname: mode === "named" ? hostname : null,
          // Only send the token if the user actually typed one.
          // Empty string = "no change". Omit = "no change" too.
          token: token || null,
        },
      });
      // Refresh to see updated has_token.
      const r = await tauriInvoke<{ has_token: boolean }>(
        "p2p_get_tunnel_config_cmd"
      );
      setHasToken(r.has_token);
      setToken(""); // clear sensitive input
      setStatus("✓ saved");
    } catch (e: any) {
      setStatus(`✗ ${e?.message ?? e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mt-3 pt-3 border-t border-bg-border space-y-2">
      <div className="text-[10px] tracking-[0.3em] uppercase text-cyan-400">
        ⚙ tunnel transport (Sprint 5.5.1)
      </div>

      {/* Mode selector */}
      <div className="flex flex-col gap-1">
        <label className="text-[9px] tracking-widest uppercase text-zinc-500">
          mode
        </label>
        <select
          value={mode}
          onChange={(e) => setMode(e.target.value as any)}
          disabled={busy}
          className="bg-bg-base border border-bg-border text-zinc-200 text-[11px] px-2 py-1 font-mono"
        >
          <option value="quick">quick — anonymous (rate-limited by Cloudflare)</option>
          <option value="named">named — per-account tunnel (no rate limit)</option>
          <option value="direct" disabled>
            direct — LAN/P2P (Sprint 5.5.2)
          </option>
        </select>
      </div>

      {/* Named-mode fields */}
      {mode === "named" && (
        <>
          <div className="flex flex-col gap-1">
            <label className="text-[9px] tracking-widest uppercase text-zinc-500">
              cloudflare tunnel hostname
            </label>
            <input
              type="text"
              value={hostname}
              onChange={(e) => setHostname(e.target.value)}
              placeholder="p2p.example.com"
              disabled={busy}
              className="bg-bg-base border border-bg-border text-zinc-200 text-[11px] px-2 py-1 font-mono"
            />
          </div>
          <div className="flex flex-col gap-1">
            <label className="text-[9px] tracking-widest uppercase text-zinc-500">
              tunnel token (stored in OS keyring — never on disk)
              {hasToken && (
                <span className="ml-2 text-matrix-500">[token saved]</span>
              )}
            </label>
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder={
                hasToken
                  ? "paste new token to replace, or leave blank to keep current"
                  : "paste the token from Cloudflare dashboard"
              }
              disabled={busy}
              autoComplete="off"
              className="bg-bg-base border border-bg-border text-zinc-200 text-[11px] px-2 py-1 font-mono"
            />
          </div>
        </>
      )}

      {/* Save button */}
      <button
        onClick={onSave}
        disabled={busy || (mode === "named" && !hostname)}
        className="btn w-full"
      >
        {busy ? "saving…" : "save tunnel settings"}
      </button>

      {status && (
        <div
          className={
            "text-[10px] font-mono " +
            (status.startsWith("✓")
              ? "text-matrix-500"
              : status.startsWith("✗")
              ? "text-err"
              : "text-zinc-500")
          }
        >
          {status}
        </div>
      )}

      <div className="text-[9px] text-zinc-600 tracking-wider leading-relaxed">
        Quick mode is the default — works out of the box, but
        Cloudflare throttles after a few connections. Named mode
        requires a free Cloudflare account; see README for the
        5-minute setup.
      </div>
    </div>
  );
}
