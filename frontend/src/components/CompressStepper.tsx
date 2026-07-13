// Sprint 5.7.21-B-Remodel: horizontal stepper for the
// Compress flow. Shows the user WHERE they are in the
// 4-step flow (Files → Profile → Output → Compress) and
// which steps are done.
//
// Visual states per step:
//   - pending  (default): greyed out, dimmed
//   - active   (current step): cyan ring, full opacity
//   - done     (previous step): cyan filled, checkmark
//
// The stepper is read-only — it doesn't accept clicks. The
// user navigates between steps by interacting with the
// corresponding section (drop zone, preset cards, output
// picker). The stepper is just a visual aid.

import { Check } from "lucide-react";
import { useLocale } from "./LocaleProvider";
import type { TranslationKey } from "@/lib/i18n";

export type StepState = "pending" | "active" | "done";

export interface Step {
  /// Stable id used for the React key + test ids.
  id: string;
  /// i18n key for the short label (e.g. "compress.step.files").
  /// Typed as TranslationKey so the t() call below is
  /// type-checked (the i18n file is the source of truth).
  labelKey: TranslationKey;
  /// i18n key for the action-verb hint (e.g. "compress.step.files.hint").
  hintKey: TranslationKey;
}

interface CompressStepperProps {
  steps: Step[];
  /// Index of the active step (0-based). The previous
  /// steps are "done", the active one is "active", the rest
  /// are "pending".
  activeIndex: number;
}

export function CompressStepper({ steps, activeIndex }: CompressStepperProps) {
  const { t } = useLocale();
  return (
    <nav
      aria-label="Compression flow"
      className="mb-10 flex items-center gap-1.5 select-none"
    >
      {steps.map((step, i) => {
        const state: StepState =
          i < activeIndex ? "done" : i === activeIndex ? "active" : "pending";
        const isLast = i === steps.length - 1;
        return (
          <div key={step.id} className="flex items-center gap-1.5">
            <div
              className={
                "flex items-center gap-2 px-3 py-2 rounded-lg transition-all " +
                (state === "active"
                  ? "bg-cyan-500/10 ring-1 ring-cyan-400/40"
                  : state === "done"
                    ? "bg-emerald-500/[0.06]"
                    : "bg-white/[0.02]")
              }
              data-testid={`step-${step.id}`}
              data-step-state={state}
            >
              <div
                className={
                  "shrink-0 w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-mono font-semibold transition-all " +
                  (state === "active"
                    ? "bg-cyan-400 text-black shadow-[0_0_12px_-2px_rgba(34,211,238,0.6)]"
                    : state === "done"
                      ? "bg-emerald-400 text-black"
                      : "bg-white/[0.08] text-zinc-500")
                }
                aria-current={state === "active" ? "step" : undefined}
              >
                {state === "done" ? (
                  <Check size={11} strokeWidth={3} />
                ) : (
                  i + 1
                )}
              </div>
              <div className="flex flex-col leading-tight">
                <div
                  className={
                    "text-[12.5px] font-medium tracking-tight " +
                    (state === "active"
                      ? "text-white"
                      : state === "done"
                        ? "text-emerald-200"
                        : "text-zinc-500")
                  }
                >
                  {t(step.labelKey)}
                </div>
                <div
                  className={
                    "text-[10.5px] " +
                    (state === "pending" ? "text-zinc-600" : "text-zinc-400")
                  }
                >
                  {t(step.hintKey)}
                </div>
              </div>
            </div>
            {!isLast && (
              <div
                className={
                  "h-px w-6 transition-colors " +
                  (state === "done" || state === "active"
                    ? "bg-cyan-400/30"
                    : "bg-white/[0.06]")
                }
                aria-hidden
              />
            )}
          </div>
        );
      })}
    </nav>
  );
}
