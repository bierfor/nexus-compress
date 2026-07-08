/**
 * Sprint 5.7 — shared formatting helpers used by LandingPage,
 * RecentView, and any other component that surfaces db.rs data.
 *
 * All number formatters respect the user's locale (es / en / it)
 * via Intl.NumberFormat. All byte formatters pick the right unit
 * (B / KB / MB / GB / TB) automatically.
 */

const LOCALE_MAP: Record<string, string> = {
  es: "es-ES",
  en: "en-US",
  it: "it-IT",
};

/** Format a counter with locale-aware thousands separators.
 *  e.g. 1548 → "1.548" (es) | "1,548" (en) | "1.548" (it). */
export function formatCount(n: number, locale: string): string {
  try {
    return new Intl.NumberFormat(LOCALE_MAP[locale] ?? "en-US").format(n);
  } catch {
    return String(n);
  }
}

/** Format a byte count as a short human string with the right
 *  binary prefix (KiB-style). e.g. 24_500_000_000 → "24.5 GB". */
export function formatBytes(n: number, locale: string): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  try {
    return `${new Intl.NumberFormat(LOCALE_MAP[locale] ?? "en-US", {
      maximumFractionDigits: 1,
    }).format(v)} ${units[i]}`;
  } catch {
    return `${v.toFixed(1)} ${units[i]}`;
  }
}

/** Format a Unix-epoch-seconds timestamp as a relative "X min ago"
 *  string in the user's language (uses the dict if provided, else
 *  falls back to a Spanish default).
 *
 *  Pass the `t` translation function so the result is fully localized. */
export function formatRelativeSeconds(
  secondsAgo: number,
  t: (k: string, vars?: Record<string, string | number>) => string,
): string {
  if (secondsAgo < 0) secondsAgo = 0;
  if (secondsAgo < 60) return t("recent.time.just_now");
  if (secondsAgo < 3600)
    return t("recent.time.min_ago").replace(
      "{n}",
      String(Math.floor(secondsAgo / 60)),
    );
  if (secondsAgo < 86400)
    return t("recent.time.h_ago").replace(
      "{n}",
      String(Math.floor(secondsAgo / 3600)),
    );
  return t("recent.time.d_ago").replace(
    "{n}",
    String(Math.floor(secondsAgo / 86400)),
  );
}

/** Convert Unix-epoch-seconds to a "hace X" relative string using
 *  formatRelativeSeconds. Convenience wrapper. */
export function formatTimestamp(
  unixSeconds: number,
  t: (k: string, vars?: Record<string, string | number>) => string,
): string {
  if (!unixSeconds) return "—";
  return formatRelativeSeconds(Date.now() / 1000 - unixSeconds, t);
}

/** Convert Unix-epoch-ms to the same "hace X" string. */
export function formatTimestampMs(
  unixMs: number,
  t: (k: string, vars?: Record<string, string | number>) => string,
): string {
  return formatTimestamp(unixMs / 1000, t);
}
