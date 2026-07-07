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
    </div>
  );
}
