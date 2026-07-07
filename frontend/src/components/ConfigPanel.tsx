"use client";

/**
 * The Config Panel — the user-facing controls.
 *
 * The headline control is the Fast/Premium slider. In Fast mode
 * (left, value 0) the codec uses lazy LZ77 matching, the entropy
 * gatekeeper, and the local-sub-dict path. In Premium mode (right,
 * value 1) it would use the optimal DP — which we found to be 8.8×
 * slower with no measurable ratio gain, so Premium currently
 * delegates to Fast. The slider is preserved as a UX placeholder
 * for future improvements.
 *
 * Below the slider: self-test, reset, and (in Premium) the warning
 * text that explains Premium's tradeoff.
 */
export function ConfigPanel({
  level,
  onLevelChange,
  onSelfTest,
}: {
  level: "fast" | "premium";
  onLevelChange: (level: "fast" | "premium") => void;
  onSelfTest: () => void;
}) {
  return (
    <div className="panel p-4 flex flex-col gap-3">
      <div className="metric-label">config</div>

      <div className="flex flex-col gap-2">
        <div className="flex justify-between text-[10px] font-mono tracking-widest uppercase">
          <span className={level === "fast" ? "text-cyan-400" : "text-zinc-600"}>
            ◀ fast
          </span>
          <span className="text-zinc-500">level</span>
          <span
            className={level === "premium" ? "text-magenta-400" : "text-zinc-600"}
          >
            premium ▶
          </span>
        </div>
        <input
          type="range"
          min={0}
          max={1}
          step={1}
          value={level === "fast" ? 0 : 1}
          onChange={(e) =>
            onLevelChange(e.target.value === "0" ? "fast" : "premium")
          }
          className="slider"
        />
        <div className="text-[10px] font-mono text-zinc-500 leading-relaxed">
          {level === "fast"
            ? "Lazy LZ77 · entropy gatekeeper on · local sub-dict selected per block. Recommended."
            : "Optimal DP (8.8× slower, ~0% ratio gain on natural text). Kept for experimentation."}
        </div>
      </div>

      <div className="flex gap-2 mt-2">
        <button onClick={onSelfTest} className="btn flex-1">
          self-test
        </button>
      </div>
    </div>
  );
}
