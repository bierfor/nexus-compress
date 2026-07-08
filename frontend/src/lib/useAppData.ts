/**
 * Sprint 5.7 — shared data hooks for db.rs-backed views.
 *
 * Both LandingPage and RecentView need the same fetch logic:
 *   - get_stats_cmd()            → AppStats (5 counters)
 *   - get_recent_events_cmd(N)   → top N ActivityEvent rows
 *
 * Centralizing it here means:
 *   1. One fallback path when Tauri isn't available
 *      (plain `next dev` shows a warning + uses the in-memory prop)
 *   2. One refetch trigger (window focus)
 *   3. One i18n locale passed through to the formatters
 *
 * The hooks return both data + loading/error state so callers can
 * render skeletons, warnings, or fall through to mock ops.
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLocale } from "@/components/LocaleProvider";
import type { RecentOp } from "@/components/RecentView";

// ── db.rs type mirrors ────────────────────────────────────────

export interface ActivityEventRaw {
  id: number;
  kind: string;
  filename: string;
  original_bytes: number;
  output_bytes: number;
  timestamp: number;
}

export interface AppStats {
  total_files_processed: number;
  total_bytes_saved: number;
  total_files_shared: number;
  average_compression_ratio: number;
  last_activity_timestamp: number;
}

export type { RecentOp };

// Map db rows → UI rows
function toOp(e: ActivityEventRaw): RecentOp {
  return {
    id: String(e.id),
    kind: (e.kind === "p2p_recv" ? "share" : e.kind) as RecentOp["kind"],
    filename: e.filename,
    originalBytes: e.original_bytes,
    compressedBytes: e.output_bytes,
    restoredBytes: e.output_bytes,
    durationMs: 0,
    timestamp: e.timestamp * 1000,
    status: "ok",
  };
}

// ── Hooks ──────────────────────────────────────────────────────

/**
 * Fetch the top N recent events from db.rs. Falls back to the
 * `fallback` prop (e.g. an in-memory list from compress/decompress
 * completions in this same session) when Tauri isn't available.
 *
 * Refreshes automatically on window focus.
 */
export function useRecentEvents(
  limit: number,
  fallback: RecentOp[] = [],
): {
  ops: RecentOp[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const [ops, setOps] = useState<RecentOp[]>(fallback);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setErr(null);
    try {
      const events = await invoke<ActivityEventRaw[]>("get_recent_events_cmd", {
        limit,
      });
      setOps(events.map(toOp));
    } catch (e) {
      setErr(String(e));
      setOps(fallback);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [limit]);

  return { ops, loading, error: err, refresh };
}

/**
 * Fetch the aggregate AppStats from db.rs. Returns null when
 * Tauri isn't available (e.g. `next dev`).
 *
 * Refreshes automatically on window focus.
 */
export function useAppStats(): {
  stats: AppStats | null;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const [stats, setStats] = useState<AppStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setErr(null);
    try {
      const s = await invoke<AppStats>("get_stats_cmd");
      setStats(s);
    } catch (e) {
      setErr(String(e));
      setStats(null);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  return { stats, loading, error: err, refresh };
}
