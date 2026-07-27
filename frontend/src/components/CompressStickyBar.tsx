// Sprint 5.7.21-B-Remodel: sticky bottom action bar for the
// Compress flow. Replaces the in-flow "Comprimir" button
// at the bottom of the page so the user always knows where
// to click, regardless of how long the rest of the page
// scrolls.
//
// Layout (left to right, single row):
//   - Output destination picker (folder icon + path + edit button)
//   - Estimated output size (when files are picked)
//   - Encrypt toggle (optional, in the same row)
//   - Big "Comprimir" button (primary action, right-aligned)
//
// The bar is `position: sticky; bottom: 0` so it sits at
// the bottom of the viewport when the user scrolls down.
// We add a backdrop blur + border-top to separate it from
// the rest of the content visually.

import { useLocale } from "./LocaleProvider";
import { FolderOpen, Lock } from "lucide-react";

interface CompressStickyBarProps {
  /// Current destination directory (empty string = unset).
  destDir: string;
  /// Callback to open the destination picker.
  onPickDest: () => void;
  /// True when encryption is enabled.
  encrypt: boolean;
  /// Toggle encryption (only shown when enabled via the
  /// `showEncryptToggle` flag — disabled by default to keep
  /// the bar compact).
  onToggleEncrypt?: () => void;
  /// Show the encrypt toggle in the bar.
  showEncryptToggle?: boolean;
  /// Estimated output size as a human string (e.g. "1.2 MiB").
  /// Empty string = "unknown size" (we don't always have a
  /// good estimate, especially for dev-cache-heavy corpora).
  estimatedSize: string;
  /// Compress button state.
  canCompress: boolean;
  busy: boolean;
  /// Compress button label (different text while busy, done, etc.).
  buttonLabel: string;
  onCompress: () => void;
}

export function CompressStickyBar({
  destDir,
  onPickDest,
  encrypt,
  onToggleEncrypt,
  showEncryptToggle,
  estimatedSize,
  canCompress,
  busy,
  buttonLabel,
  onCompress,
}: CompressStickyBarProps) {
  const { t } = useLocale();
  return (
    <div className="sticky bottom-0 left-0 right-0 z-30 -mx-8 mt-12 px-8 py-4 bg-[#0a0a0a]/85 backdrop-blur-md border-t border-white/[0.08]">
      <div className="max-w-4xl mx-auto flex items-center gap-4">
        {/* Output destination picker */}
        <button
          onClick={onPickDest}
          aria-label={t("compress.bottom.click_to_pick")}
          className="flex items-center gap-2 px-3 py-2.5 rounded-lg bg-white/[0.04] hover:bg-white/[0.07] border border-white/[0.08] hover:border-white/[0.14] transition-all min-w-0 flex-shrink text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
        >
          <FolderOpen size={15} className="text-zinc-400 shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="text-[10px] text-zinc-500 uppercase tracking-wider leading-tight">
              {t("compress.dest.label")}
            </div>
            <div className="text-[12.5px] text-zinc-200 font-mono truncate max-w-[260px]">
              {destDir || t("compress.bottom.click_to_pick")}
            </div>
          </div>
        </button>

        {/* Estimated output */}
        {estimatedSize && (
          <div className="text-right shrink-0 hidden sm:block">
            <div className="text-[10px] text-zinc-500 uppercase tracking-wider leading-tight">
              {t("compress.bottom.estimated")}
            </div>
            <div className="text-[13px] text-cyan-300 font-mono font-semibold tabular-nums">
              {estimatedSize}
            </div>
          </div>
        )}

        {/* Encrypt toggle (optional) */}
        {showEncryptToggle && onToggleEncrypt && (
          <button
            onClick={onToggleEncrypt}
            aria-pressed={encrypt}
            className={
              "flex items-center gap-1.5 px-3 py-2.5 rounded-lg border text-[12px] font-medium transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50 " +
              (encrypt
                ? "border-amber-400/40 bg-amber-500/10 text-amber-200"
                : "border-white/[0.08] bg-white/[0.02] text-zinc-400 hover:bg-white/[0.04] hover:text-zinc-200")
            }
          >
            <Lock size={13} />
            {t("compress.bottom.encrypt_toggle")}
          </button>
        )}

        {/* Compress button */}
        <button
          onClick={onCompress}
          disabled={!canCompress}
          className="ml-auto px-7 py-3 rounded-xl bg-gradient-to-b from-cyan-500 to-cyan-600 hover:from-cyan-400 hover:to-cyan-500 disabled:from-zinc-800 disabled:to-zinc-800 disabled:text-zinc-600 text-white text-[14px] font-semibold tracking-tight transition-all shadow-lg shadow-cyan-500/20 disabled:shadow-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/60"
        >
          {busy ? (
            <span className="flex items-center gap-2">
              <span className="inline-block w-3 h-3 rounded-full border-2 border-cyan-200/40 border-t-cyan-100 animate-spin" />
              {buttonLabel}
            </span>
          ) : (
            buttonLabel
          )}
        </button>
      </div>
    </div>
  );
}
