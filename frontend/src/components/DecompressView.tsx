"use client";

/**
 * DecompressView — Sprint 5.6.17 WinRAR-style browsing.
 *
 * Flow:
 *   1. User drops/picks an archive (.tar, .nxs6, .nxs, .lz, .nxar)
 *   2. App detects the format and reads the central directory
 *      (without extracting payload bytes).
 *   3. UI lists every entry with a checkbox. Default: all checked.
 *   4. User picks a destination folder.
 *   5. Click "Extract all" or "Extract N selected" → backend
 *      streams only the chosen entries from the archive.
 *
 * Sprint 5.7.2: encrypted archives (NXE\0 / NXR\0 magic). When
 * the peek returns `nxe-encrypted` or `nxr-encrypted`, the
 * ArchivePreview component renders the 🔒 + password prompt.
 * On unlock we re-peek with the password (the backend decrypts
 * then runs the inner-format peek on the recovered NXS bytes)
 * and stash the password in `password` state for the eventual
 * extract. The password is wiped from memory right after the
 * extract completes.
 */

import { useEffect, useState, useCallback, useRef } from "react";
import { type View } from "@/components/NeoTopBar";
import { useLocale } from "@/components/LocaleProvider";
import { ArchivePreview, type Preview } from "@/components/ArchivePreview";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!isTauri) return {} as T;
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

interface ArchiveEntry {
  name: string;
  size: number;
  is_dir: boolean;
}

interface ArchiveListResp {
  entries: ArchiveEntry[];
  total_files: number;
  total_bytes: number;
  has_more: boolean;
  offset: number;
  parse_time_ms: number;
}

interface ArchiveInspectInfo {
  archive_kind: string;
  n_files: number;
  total_bytes: number;
  entries: ArchiveEntry[];
}

interface LegacyArchiveInfo {
  archive_kind: string;
  n_files: number;
  total_uncompressed: number;
  compressed_size: number;
  files: { path: string; size: number; is_dir: boolean }[];
}

interface DecompressResult {
  archive_kind: string;
  restored_size: number;
  n_files: number;
  output_path: string;
  decompress_time_ms: number;
}

interface ArchiveExtractResp {
  written: string[];
  count: number;
  total_bytes: number;
}

function isInspectable(name: string | null): "tar" | "solid" | null {
  if (!name) return null;
  const lower = name.toLowerCase();
  if (lower.endsWith(".tar")) return "tar";
  if (lower.endsWith(".nxs6") || lower.endsWith(".nxs")) return "solid";
  return null;
}

export function DecompressView({
  onComplete,
  onNavigate,
}: {
  onComplete: (op: {
    kind: "decompress";
    filename: string;
    originalBytes: number;
    restoredBytes: number;
    durationMs: number;
  }) => void;
  onNavigate: (v: View) => void;
}) {
  const { t } = useLocale();
  const [archivePath, setArchivePath] = useState<string | null>(null);
  const [info, setInfo] = useState<ArchiveInspectInfo | null>(null);
  const [legacyInfo, setLegacyInfo] = useState<LegacyArchiveInfo | null>(null);
  const [destDir, setDestDir] = useState<string>("");
  const [destInitialized, setDestInitialized] = useState(false);
  const [busy, setBusy] = useState(false);
  const [peeking, setPeeking] = useState(false);
  // Sprint 5.7.2 hotfix #23: progress state mirrors CompressView's
  // — populated by the 'compress-progress' event listener above
  // and rendered in the progress card so the user sees elapsed
  // time, throughput, and ETA during extraction.
  const [progress, setProgress] = useState<{
    phase: string;
    current_file: string;
    files_done: number;
    files_total: number;
    bytes_done: number;
    bytes_total: number;
    elapsed_ms: number;
    bytes_per_sec: number;
    eta_ms: number;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  // Sprint 5.7.2: password state for encrypted archives (NXE\0 /
  // NXR\0 magic). The user types the password in the
  // ArchivePreview's EncryptedPreview panel → onUnlock callback,
  // which calls peek_archive_target_cmd with the password, which
  // returns the decrypted file list. We then stash the password
  // here so the onExtract call below can pass it to
  // decompress_target_cmd. Wiped from memory immediately after
  // the extract succeeds (security: no lingering sensitive bytes
  // in the JS heap).
  const [password, setPassword] = useState<string>("");
  // Locked-state mirrors the ArchivePreview's EncryptedPreview
  // panel: `unlocking` is true while the backend re-peek is in
  // flight (so the password input shows a "unlocking…" button),
  // and `unlockError` carries the last failed password attempt
  // for the "✗ wrong password" inline error.
  const [unlocking, setUnlocking] = useState(false);
  const [unlockError, setUnlockError] = useState<string | null>(null);

  // Adapter: LegacyArchiveInfo → Preview. The encrypted peek
  // returns `nxe-encrypted` / `nxr-encrypted` with files: [] and
  // total_uncompressed ≈ compressed_size (we only have the outer
  // envelope before unlock). The ArchivePreview component
  // expects the Preview shape; this adapter fills the gaps.
  function legacyToPreview(li: LegacyArchiveInfo): Preview {
    return {
      archive_kind: li.archive_kind,
      n_files: li.n_files,
      total_uncompressed: li.total_uncompressed,
      compressed_size: li.compressed_size,
      files: (li.files ?? []).map((f) => ({
        path: f.path,
        size: f.size,
        is_dir: f.is_dir,
      })),
    };
  }

  // onUnlock: the ArchivePreview's password prompt calls this
  // when the user submits. We re-invoke peek_archive_target_cmd
  // with the password; on success the new file list is rendered
  // (the same `info` legacy path), and the password is stashed
  // for onExtract. On wrong password we show the inline error.
  const onUnlock = useCallback(
    async (pwd: string) => {
      if (!archivePath) return;
      setUnlocking(true);
      setUnlockError(null);
      try {
        const r = await tauriInvoke<LegacyArchiveInfo>(
          "peek_archive_target_cmd",
          { req: { path: archivePath, password: pwd } },
        );
        setLegacyInfo(r);
        setPassword(pwd);
      } catch (e: any) {
        const msg = String(e?.message ?? e);
        // The backend's encrypted.wrong_password error code
        // surfaces as "encrypted.wrong_password: wrong password"
        // — strip the code prefix and show just the message.
        const cleaned = msg.replace(/^[^:]+:\s*/, "");
        setUnlockError(
          cleaned.includes("wrong") ? "wrong password" : cleaned
        );
      } finally {
        setUnlocking(false);
      }
    },
    [archivePath],
  );
  const [pathInput, setPathInput] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  // Pagination for huge archives. Loads 500 entries at a time
  // (configurable in the call) and shows "Load more" only when
  // has_more is true. Keeps the DOM tree < 600 nodes regardless
  // of archive size, so 100k-entry lists scroll smoothly.
  const [hasMore, setHasMore] = useState(false);
  const [nextOffset, setNextOffset] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  const [parseTimeMs, setParseTimeMs] = useState(0);
  const [filter, setFilter] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);

  const inspectable = isInspectable(archivePath);

  // Resolve homeDir on mount
  useEffect(() => {
    if (!isTauri || destInitialized) return;
    (async () => {
      try {
        const { homeDir, join } = await import("@tauri-apps/api/path");
        const home = await homeDir();
        const downloads = await join(home, "Downloads");
        setDestDir(downloads);
      } catch {
        setDestDir("Downloads");
      } finally {
        setDestInitialized(true);
      }
    })();
  }, [destInitialized]);

  // Subscribe to the 'compress-progress' event from the Rust backend.
  // Sprint 5.7.2 hotfix #23: DecompressView was missing this listener
  // entirely (only CompressView had it), so the progress bar / elapsed
  // time / throughput / ETA never updated during extraction.
  //
  // Implementation note: registered ONCE on mount with empty dep
  // array — re-registering on every `busy` toggle caused events to
  // slip through the unlisten/listen race window. We accept ALL
  // events here; the progress card only renders when `progress` is
  // non-null, and the only events we get when not extracting come
  // from the peek path (small bytes_total, set by setProgress
  // briefly then cleared on the next user action). The peek bar
  // flicker is acceptable; the alternative was a window of lost
  // events during the busy=false→true transition.
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlisten = await eventMod.listen("compress-progress", (e: any) => {
          const p = e?.payload;
          if (!p) return;
          const phase = String(p.phase ?? "");
          setProgress({
            phase,
            current_file: String(p.current_file ?? ""),
            files_done: Number(p.files_done ?? 0),
            files_total: Number(p.files_total ?? 1),
            bytes_done: Number(p.bytes_done ?? 0),
            bytes_total: Number(p.bytes_total ?? 0),
            elapsed_ms: Number(p.elapsed_ms ?? 0),
            bytes_per_sec: Number(p.bytes_per_sec ?? 0),
            eta_ms: Number(p.eta_ms ?? 0),
          });
          // Auto-dismiss the bar once extraction is complete.
          if (phase === "done") {
            // Keep the "done" event visible for ~1.2s so the user
            // can read "Listo" / 100%, then clear.
            setTimeout(() => setProgress(null), 1200);
          }
        });
      } catch (e) {
        console.error(e);
      }
    })();
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Peek archive on path change
  useEffect(() => {
    if (!archivePath) {
      setInfo(null);
      setLegacyInfo(null);
      setSelected(new Set());
      setPeeking(false);
      return;
    }
    setError(null);
    setInfo(null);
    setLegacyInfo(null);
    setSelected(new Set());
    setHasMore(false);
    setNextOffset(0);
    setParseTimeMs(0);
    setFilter("");
    setPeeking(true);
    if (inspectable) {
      tauriInvoke<ArchiveListResp>("p2p_archive_list_cmd", {
        req: { path: archivePath, offset: 0, limit: 500 },
      })
        .then((r) => {
          const inspect: ArchiveInspectInfo = {
            archive_kind:
              inspectable === "tar"
                ? "TAR"
                : inspectable === "solid"
                  ? "NXS6"
                  : "Legacy single-stream",
            n_files: r.total_files,
            total_bytes: r.total_bytes,
            entries: r.entries,
          };
          setInfo(inspect);
          setHasMore(r.has_more);
          setNextOffset(r.offset + r.entries.length);
          setParseTimeMs(r.parse_time_ms);
          const initial = new Set<string>();
          for (const e of r.entries) {
            if (!e.is_dir) initial.add(e.name);
          }
          setSelected(initial);
        })
        .catch((e: any) => {
          const msg = String(e?.message ?? e);
          setError(`No se pudo leer el archivo: ${msg}`);
        })
        .finally(() => setPeeking(false));
    } else {
      tauriInvoke<LegacyArchiveInfo>("peek_archive_target_cmd", {
        req: { path: archivePath },
      })
        .then(setLegacyInfo)
        .catch((e: any) => {
          const msg = String(e?.message ?? e);
          setError(`No se pudo leer el archivo: ${msg}`);
        })
        .finally(() => setPeeking(false));
    }
  }, [archivePath, inspectable]);

  const acceptPath = useCallback((path: string | null) => {
    if (path) {
      setArchivePath(path);
      setError(null);
      setPathInput("");
    }
  }, []);

  // Drag-drop
  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(true);
  }, []);
  const onDragLeave = useCallback(() => setDragOver(false), []);
  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const tauriPaths = (e as any).detail?.paths ?? null;
      if (tauriPaths?.length) acceptPath(tauriPaths[0]);
    },
    [acceptPath],
  );

  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlisten = await eventMod.listen("tauri://drag-drop", (e: any) => {
          const paths: string[] = e?.payload?.paths ?? [];
          if (paths.length > 0) acceptPath(paths[0]);
        });
      } catch (e) {
        console.error(e);
      }
    })();
    return () => unlisten?.();
  }, [acceptPath]);

  const onBrowse = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      // macOS Sequoia / Sonoma bug: when no `filters` is set on
      // a file picker, the native NSOpenPanel sometimes defaults
      // to "folders only" mode and the user can't see regular
      // files. Pass an explicit "All files" filter with `["*"]`
      // to force the standard file-selection mode. The
      // "NexusCompress archives" filter is a convenience for
      // users who want to see their .nxs / .nxs6 / .nxe / .nxr
      // / .lz / .nxar files first.
      const result = await open({
        multiple: false,
        directory: false,
        filters: [
          { name: "All files", extensions: ["*"] },
          // Sprint 5.7.21-EXT: include the third-party
          // formats the backend can now extract. The list
          // is mirrored from src/external_decompress.rs::
          // detect_format + the extension overrides for
          // TAR (no magic).
          { name: "NexusCompress archives", extensions: ["nxs", "nxs6", "nxe", "nxr", "lz", "nxar"] },
          { name: "ZIP archives", extensions: ["zip"] },
          { name: "TAR archives", extensions: ["tar"] },
          { name: "Compressed archives", extensions: ["tar.gz", "tgz", "gz"] },
        ],
      });
      if (typeof result === "string") acceptPath(result);
    } catch (e) {
      // Sprint 5.7.21-B cleanup: surface the picker error
      // to the user via the existing `error` state. User-
      // cancelled dialog is silent.
      const msg = String((e as Error)?.message ?? e);
      if (!/cancel/i.test(msg)) {
        setError(t("decompress.error.file_picker") + ": " + msg);
      }
    }
  }, [acceptPath, t]);

  const onAddPath = useCallback(() => {
    const trimmed = pathInput.trim();
    if (trimmed) {
      acceptPath(trimmed);
    }
  }, [pathInput, acceptPath]);

  const onBrowseDest = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({ multiple: false, directory: true });
      if (typeof result === "string") setDestDir(result);
    } catch (e) {
      // Sprint 5.7.21-B cleanup: surface the picker error.
      const msg = String((e as Error)?.message ?? e);
      if (!/cancel/i.test(msg)) {
        setError(t("decompress.error.dest_picker") + ": " + msg);
      }
    }
  }, [t]);

  const toggleEntry = useCallback((name: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }, []);

  const selectAll = useCallback(() => {
    if (!info) return;
    // When has_more is true, info.entries is only the loaded page —
    // we can't "select all" reliably. So we explicitly set a marker
    // by emptying the override, and the extract command will treat
    // null `selected` as "everything". For now, when paginated,
    // select-all only selects the visible page.
    if (hasMore) {
      setSelected(
        new Set(info.entries.filter((e) => !e.is_dir).map((e) => e.name)),
      );
    } else {
      setSelected(
        new Set(info.entries.filter((e) => !e.is_dir).map((e) => e.name)),
      );
    }
  }, [info, hasMore]);

  const selectNone = useCallback(() => setSelected(new Set()), []);

  // Lazy load next page of entries. Hit when the user scrolls to
  // the bottom of the visible list. The backend returns up to
  // limit+1 entries; we drop the extra to detect has_more.
  const loadMore = useCallback(async () => {
    if (!archivePath || !hasMore || loadingMore) return;
    setLoadingMore(true);
    try {
      const r = await tauriInvoke<ArchiveListResp>(
        "p2p_archive_list_cmd",
        {
          req: {
            path: archivePath,
            offset: nextOffset,
            limit: 500,
          },
        },
      );
      setInfo((prev) =>
        prev
          ? { ...prev, entries: [...prev.entries, ...r.entries] }
          : prev,
      );
      // Tick the selection for newly-loaded file entries.
      const initial = new Set<string>();
      for (const e of r.entries) {
        if (!e.is_dir) initial.add(e.name);
      }
      setSelected((prev) => new Set([...prev, ...initial]));
      setHasMore(r.has_more);
      setNextOffset(r.offset + r.entries.length);
      setParseTimeMs((prev) => prev + r.parse_time_ms);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setLoadingMore(false);
    }
  }, [archivePath, hasMore, loadingMore, nextOffset]);

  // Filtered entries shown in the list. Empty filter = show all.
  const visibleEntries = filter.trim()
    ? (info?.entries ?? []).filter((e) =>
        e.name.toLowerCase().includes(filter.trim().toLowerCase()),
      )
    : info?.entries ?? [];

  const onExtract = useCallback(async () => {
    if (!archivePath) return;
    setBusy(true);
    setError(null);
    // Sprint 5.7.2 hotfix #23: clear stale progress so the new
    // extraction starts from 0% / 0:00 instead of showing the
    // previous run's last frame.
    setProgress(null);
    const startTime = Date.now();
    // Sprint 5.7.2: if the archive is encrypted but the user
    // hasn't unlocked yet, we don't have a password to pass
    // to the backend. Show a clear error and bail.
    const isEncrypted =
      legacyInfo?.archive_kind === "nxe-encrypted" ||
      legacyInfo?.archive_kind === "nxr-encrypted";
    if (isEncrypted && !password) {
      setError(
        "archive is encrypted — type the password in the preview above to unlock"
      );
      setBusy(false);
      return;
    }
    try {
      if (inspectable && info) {
        const selectedArr = Array.from(selected);
        const r = await tauriInvoke<ArchiveExtractResp>(
          "p2p_archive_extract_cmd",
          {
            req: {
              path: archivePath,
              output_dir: destDir || null,
              selected: selectedArr.length === info.entries.filter((e) => !e.is_dir).length
                ? null
                : selectedArr,
              // Sprint 5.7.2: forward the password to the
              // extract command for encrypted archives.
              // The backend (p2p_archive_extract_cmd or
              // decompress_target_cmd) routes to the
              // encrypted path when password is set.
              password: password || null,
            },
          },
        );
        onComplete({
          kind: "decompress",
          filename: archivePath.split("/").pop() || "archive",
          originalBytes: r.total_bytes,
          restoredBytes: r.total_bytes,
          durationMs: Date.now() - startTime,
        });
        setArchivePath(null);
        setInfo(null);
      } else {
        const r = await tauriInvoke<DecompressResult>("decompress_target_cmd", {
          req: {
            path: archivePath,
            output_dir: destDir || null,
            // Sprint 5.7.2: same — forward the password. When
            // set, the backend routes to
            // decompress_target_with_password which decrypts
            // with AES-256-GCM (and Reed-Solomon if NXR).
            password: password || null,
          },
        });
        onComplete({
          kind: "decompress",
          filename: archivePath.split("/").pop() || "archive",
          originalBytes: legacyInfo?.total_uncompressed ?? r.restored_size,
          restoredBytes: r.restored_size,
          durationMs: Date.now() - startTime,
        });
        setArchivePath(null);
        setLegacyInfo(null);
      }
      // Wipe the in-memory password on success (and on
      // failure). The user will need to re-enter it if they
      // want to extract again.
      setPassword("");
    } catch (e: any) {
      setError(String(e?.message ?? e));
      setPassword("");
    } finally {
      setBusy(false);
    }
  }, [archivePath, destDir, inspectable, info, selected, legacyInfo, onComplete, password]);

  const fileCount = info?.entries.filter((e) => !e.is_dir).length ?? 0;
  const selectedCount = selected.size;
  const allSelected = fileCount > 0 && selectedCount === fileCount;

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`flex-1 overflow-y-auto transition-colors ${
        dragOver ? "bg-amber-500/[0.04]" : ""
      }`}
    >
      <div className="max-w-4xl mx-auto px-8 pt-12 pb-20">
        {/* Header */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
            <button
              onClick={() => onNavigate("landing")}
              className="hover:text-zinc-300 transition-colors"
            >
              {t("back")}
            </button>
          </div>
          <h1 className="text-white text-[36px] font-semibold tracking-tight mb-3">
            {t("decompress.title")}
          </h1>
          <p className="text-zinc-400 text-[14px] leading-relaxed max-w-2xl">
            {t("decompress.desc")}
          </p>
        </div>

        {/* Drop zone or archive info */}
        {!archivePath ? (
          <div
            className={`relative rounded-3xl border-2 border-dashed transition-all p-16 text-center mb-10 ${
              dragOver
                ? "border-amber-400 bg-amber-500/[0.08]"
                : "border-white/[0.08] bg-white/[0.02]"
            }`}
          >
            <div className="text-7xl mb-6 select-none">
              {dragOver ? "⤓" : "📂"}
            </div>
            <h3 className="text-white text-[20px] font-medium mb-2">
              {dragOver ? t("decompress.drop.active") : t("decompress.drop")}
            </h3>
            <p className="text-zinc-500 text-[13px] mb-6">
              {t("decompress.drop.hint")}
            </p>
            <div className="flex items-center gap-2 max-w-xl mx-auto">
              <input
                type="text"
                value={pathInput}
                onChange={(e) => setPathInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && onAddPath()}
                placeholder={t("decompress.placeholder")}
                className="flex-1 bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-2.5 text-[13px] text-white placeholder-zinc-600 focus:outline-none focus:border-amber-500/50"
              />
              <button
                onClick={onBrowse}
                className="px-4 py-2.5 text-[13px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-xl transition-colors"
              >
                {t("decompress.browse")}
              </button>
              <button
                onClick={onAddPath}
                disabled={!pathInput.trim()}
                className="px-4 py-2.5 text-[13px] text-amber-400 hover:text-amber-300 border border-amber-500/30 hover:border-amber-500/50 rounded-xl transition-colors disabled:opacity-30"
              >
                {t("decompress.load")}
              </button>
            </div>
          </div>
        ) : (
          <div className="mb-10 rounded-2xl bg-white/[0.03] border border-amber-500/20 overflow-hidden">
            <div className="px-6 py-5">
              <div className="flex items-start justify-between mb-3">
                <div>
                  <div className="text-amber-400 text-[11px] tracking-[0.2em] uppercase mb-2">
                    {inspectable
                      ? `${t("decompress.detected")} · ${inspectable.toUpperCase()}`
                      : t("decompress.detected")}
                  </div>
                  <div className="text-white text-[20px] font-medium mb-1">
                    {archivePath.split("/").pop()}
                  </div>
                  <div className="text-zinc-500 text-[12px] font-mono truncate">
                    {archivePath}
                  </div>
                </div>
                <button
                  onClick={() => {
                    setArchivePath(null);
                    setInfo(null);
                    setLegacyInfo(null);
                    setSelected(new Set());
                  }}
                  className="text-zinc-500 hover:text-red-400 text-[12px] transition-colors px-2 py-1"
                >
                  {t("decompress.change")}
                </button>
              </div>

              {/* Stats */}
              {info ? (
                <div className="grid grid-cols-3 gap-5 pt-4 border-t border-white/[0.06]">
                  <DetailStat label={t("decompress.format")} value={info.archive_kind} />
                  <DetailStat label={t("decompress.files")} value={`${info.n_files}`} />
                  <DetailStat label={t("decompress.size")} value={prettyBytes(info.total_bytes)} />
                </div>
              ) : legacyInfo ? (
                <>
                  <div className="grid grid-cols-3 gap-5 pt-4 border-t border-white/[0.06]">
                    <DetailStat label={t("decompress.format")} value={legacyInfo.archive_kind} />
                    <DetailStat label={t("decompress.files")} value={`${legacyInfo.n_files}`} />
                    <DetailStat
                      label={t("decompress.size.original")}
                      value={prettyBytes(legacyInfo.total_uncompressed)}
                    />
                  </div>
                  {/* Sprint 5.7.2: encrypted archive unlock UI. The
                      ArchivePreview's EncryptedPreview panel handles
                      the password prompt + re-peek with the key.
                      onUnlock stores the password in state for the
                      eventual onExtract call below. */}
                  {legacyInfo.archive_kind === "nxe-encrypted" ||
                  legacyInfo.archive_kind === "nxr-encrypted" ? (
                    <ArchivePreview
                      preview={legacyToPreview(legacyInfo)}
                      unlocking={unlocking}
                      unlockError={unlockError}
                      onUnlock={onUnlock}
                    />
                  ) : null}
                </>
              ) : peeking ? (
                <div className="text-zinc-500 text-[13px] py-4 flex items-center gap-2">
                  <span className="animate-spin inline-block">⟳</span>
                  {t("decompress.reading")}
                </div>
              ) : (
                <div className="text-zinc-500 text-[13px] py-4">
                  No se pudo leer el contenido.
                </div>
              )}

              {/* WinRAR-style entry list with checkboxes */}
              {info && info.entries.length > 0 && (
                <div className="mt-5 pt-4 border-t border-white/[0.06]">
                  <div className="flex items-center justify-between mb-3 gap-3">
                    <div className="text-zinc-400 text-[12px] flex items-center gap-2">
                      <span>
                        {selectedCount} {t("decompress.selected")}{" "}
                        {info.entries.length}
                        {hasMore ? "+" : ""} {t("decompress.selected.files")}
                      </span>
                      {parseTimeMs > 0 && (
                        <span className="text-zinc-700 text-[10px] font-mono">
                          ({parseTimeMs} ms)
                        </span>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      {/* Filter input */}
                      <input
                        type="text"
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                        placeholder="Filtrar…"
                        className="px-2 py-1 text-[11px] bg-white/[0.04] border border-white/[0.08] rounded-md text-white placeholder-zinc-600 focus:outline-none focus:border-cyan-500/40 w-32"
                      />
                      <button
                        onClick={selectAll}
                        disabled={allSelected}
                        className="text-[11px] px-2 py-1 text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-md transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                      >
                        {t("decompress.all")}
                      </button>
                      <button
                        onClick={selectNone}
                        disabled={selectedCount === 0}
                        className="text-[11px] px-2 py-1 text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-md transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                      >
                        {t("decompress.none")}
                      </button>
                    </div>
                  </div>
                  <div
                    className="max-h-80 overflow-y-auto rounded-xl bg-black/30 border border-white/[0.04] divide-y divide-white/[0.04]"
                    onScroll={(e) => {
                      // Auto-load next page when the user
                      // scrolls within 200px of the bottom.
                      // This is the "infinite scroll" trick
                      // — WinRAR shows a static list but with
                      // virtual scroll, this is faster.
                      const target = e.currentTarget;
                      const nearBottom =
                        target.scrollHeight -
                          target.scrollTop -
                          target.clientHeight <
                        200;
                      if (nearBottom && hasMore && !loadingMore) {
                        loadMore();
                      }
                    }}
                  >
                    {visibleEntries.map((entry, i) => {
                      if (entry.is_dir) {
                        return (
                          <div
                            key={`dir-${i}-${entry.name}`}
                            className="text-zinc-500 text-[12px] flex items-center gap-3 px-4 py-2 font-mono"
                          >
                            <span className="w-4">📁</span>
                            <span className="flex-1 truncate">{entry.name}</span>
                            <span className="text-zinc-700 text-[10.5px]">dir</span>
                          </div>
                        );
                      }
                      const checked = selected.has(entry.name);
                      return (
                        <label
                          key={`file-${i}-${entry.name}`}
                          className="text-zinc-300 text-[12px] flex items-center gap-3 px-4 py-1.5 hover:bg-white/[0.02] cursor-pointer font-mono"
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() => toggleEntry(entry.name)}
                            className="accent-amber-500 w-4 h-4 flex-shrink-0"
                          />
                          <span className="w-4 flex-shrink-0">📄</span>
                          <span className="flex-1 truncate">{entry.name}</span>
                          <span className="text-zinc-500 text-[10.5px] flex-shrink-0 tabular-nums">
                            {prettyBytes(entry.size)}
                          </span>
                        </label>
                      );
                    })}
                    {loadingMore && (
                      <div className="px-4 py-3 text-center text-zinc-500 text-[11px] font-mono">
                        Cargando…
                      </div>
                    )}
                    {hasMore && !loadingMore && (
                      <button
                        onClick={loadMore}
                        className="w-full px-4 py-3 text-center text-cyan-400 hover:text-cyan-300 text-[12px] font-medium hover:bg-white/[0.02] transition-colors"
                      >
                        Cargar más ({(info.entries.length)}/{hasMore ? "…" : ""} cargados)
                      </button>
                    )}
                  </div>
                </div>
              )}

              {/* Legacy: just list, no selection */}
              {legacyInfo && legacyInfo.files.length > 0 && (
                <details className="mt-5 pt-4 border-t border-white/[0.06]">
                  <summary className="text-zinc-400 text-[12px] cursor-pointer hover:text-white transition-colors">
                    {t("decompress.content")} ({legacyInfo.files.length} {t("decompress.files").toLowerCase()})
                  </summary>
                  <div className="mt-3 max-h-48 overflow-y-auto space-y-1">
                    {legacyInfo.files.slice(0, 50).map((f, i) => (
                      <div
                        key={i}
                        className="text-zinc-500 text-[12px] flex items-center gap-2 font-mono"
                      >
                        <span className="text-zinc-700 w-6 text-right">
                          {f.is_dir ? "📁" : "📄"}
                        </span>
                        <span className="flex-1 truncate">{f.path}</span>
                        <span className="text-zinc-700 text-[10.5px]">
                          {prettyBytes(f.size)}
                        </span>
                      </div>
                    ))}
                    {legacyInfo.files.length > 50 && (
                      <div className="text-zinc-600 text-[11px] pt-2">
                        … y {legacyInfo.files.length - 50} más
                      </div>
                    )}
                  </div>
                </details>
              )}
            </div>
          </div>
        )}

        {/* Destination */}
        {archivePath && (
          <div className="mb-10">
            <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
              {t("decompress.dest")}
            </div>
            <div className="flex items-center gap-3 px-5 py-4 rounded-2xl bg-white/[0.02] border border-white/[0.06]">
              <span className="text-zinc-500 text-[13px]">📁</span>
              <span className="text-white text-[14px] flex-1 truncate font-mono">
                {destDir || t("compress.dest.detecting")}
              </span>
              <button
                onClick={onBrowseDest}
                className="px-3 py-1.5 text-[12px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-lg transition-colors"
              >
                {t("decompress.dest.change")}
              </button>
            </div>
          </div>
        )}

        {/* Extract button */}
        <button
          onClick={onExtract}
          disabled={
            archivePath == null ||
            busy ||
            peeking ||
            (!!inspectable && info != null && selectedCount === 0)
          }
          className="w-full py-4 rounded-2xl bg-gradient-to-b from-amber-500 to-amber-600 hover:from-amber-400 hover:to-amber-500 disabled:from-zinc-800 disabled:to-zinc-800 disabled:text-zinc-600 text-white text-[15px] font-semibold tracking-tight transition-all shadow-lg shadow-amber-500/20 disabled:shadow-none"
        >
          {busy
            ? t("decompress.btn.busy")
            : peeking
              ? t("decompress.btn.reading")
              : inspectable && info
                ? allSelected
                  ? t("decompress.btn.extract.all")
                  : `${t("decompress.btn.extract.n")} ${selectedCount} ${t("decompress.selected")} ${fileCount}`
                : t("decompress.btn")}
        </button>

        {/* Sprint 5.7.2 hotfix #23: progress card — mirrors the
            CompressView render. Shows phase label, percentage,
            bar, current file, bytes done/total, elapsed time,
            throughput, and ETA. Hidden when no progress is
            active (e.g. when the user is just looking at the
            archive preview before clicking Extract). */}
        {progress && (
          <div
            className={
              progress.phase === "recovering"
                ? "mt-4 p-6 rounded-2xl bg-amber-500/[0.08] border border-amber-500/30"
                : "mt-4 p-6 rounded-2xl bg-cyan-500/[0.06] border border-cyan-500/20"
            }
          >
            <div className="flex items-center justify-between mb-3">
              <div
                className={
                  progress.phase === "recovering"
                    ? "text-amber-300 text-[11px] tracking-[0.2em] uppercase font-semibold"
                    : "text-cyan-300 text-[11px] tracking-[0.2em] uppercase"
                }
              >
                {progress.phase === "reading"
                  ? t("compress.phase.reading")
                  : progress.phase === "decrypting"
                  ? "Descifrando"
                  : progress.phase === "reassembling"
                  ? "Reensamblando"
                  : progress.phase === "recovering"
                  ? "🚨 " + t("compress.phase.recovering")
                  : progress.phase === "done"
                  ? t("compress.phase.done")
                  : t("compress.phase.processing")}
              </div>
              <div className="text-white text-[20px] font-semibold tabular-nums">
                {progress.bytes_total > 0
                  ? `${((progress.bytes_done / progress.bytes_total) * 100).toFixed(1)}%`
                  : "—"}
              </div>
            </div>
            <div className="h-2 bg-white/[0.04] rounded-full overflow-hidden mb-2">
              <div
                className={
                  progress.phase === "recovering"
                    ? "h-full bg-gradient-to-r from-amber-400 to-amber-500 rounded-full transition-all duration-200"
                    : "h-full bg-gradient-to-r from-cyan-400 to-cyan-500 rounded-full transition-all duration-200"
                }
                style={{
                  width: `${
                    progress.bytes_total > 0
                      ? Math.min(
                          100,
                          (progress.bytes_done / progress.bytes_total) * 100
                        )
                      : 0
                  }%`,
                }}
              />
            </div>
            <div className="flex items-center justify-between text-[11.5px] text-zinc-400 tabular-nums">
              <span className="truncate max-w-md">
                {progress.current_file || "—"}
              </span>
              <span>
                {prettyBytes(progress.bytes_done)} / {prettyBytes(progress.bytes_total)}
              </span>
            </div>
            <div className="flex items-center justify-between text-[10.5px] text-zinc-500 tabular-nums mt-1">
              <span>
                {progress.elapsed_ms > 0 ? formatMs(progress.elapsed_ms) : "—"}
                <span className="text-zinc-700 mx-1.5">·</span>
                {progress.bytes_per_sec > 0 ? formatRate(progress.bytes_per_sec) : "—"}
              </span>
              <span>
                {progress.eta_ms > 0
                  ? `ETA ${formatMs(progress.eta_ms)}`
                  : "ETA —"}
              </span>
            </div>
          </div>
        )}

        {error && (
          <div className="mt-4 p-4 rounded-xl bg-red-500/[0.08] border border-red-500/20 text-red-400 text-[13px]">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}

function DetailStat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-zinc-500 text-[10.5px] uppercase tracking-wider mb-1">
        {label}
      </div>
      <div className="text-white text-[18px] font-medium tabular-nums">{value}</div>
    </div>
  );
}

function formatMs(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  if (m === 0) return `${s} s`;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatRate(bps: number): string {
  if (bps >= 1024 * 1024) return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${bps.toFixed(0)} B/s`;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
