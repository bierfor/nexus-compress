"use client";

import { useEffect, useState, useCallback } from "react";
import { type View } from "@/components/NeoTopBar";
import { PageHeader } from "@/components/PageHeader";
import { useLocale } from "@/components/LocaleProvider";
import {
  Send,
  FileText,
  FolderOpen,
  ArrowRight,
  Copy,
  Check,
  RefreshCw,
  File,
  X,
  Wifi,
  Globe,
  AlertCircle,
  Sparkles,
  Link as LinkIcon,
  QrCode,
  Lock,
  Shield,
  Download,
  Clock,
  Infinity as InfinityIcon,
  ChevronDown,
  Settings2,
  Server,
  HardDrive,
  KeyRound,
  Eye,
  EyeOff,
} from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { useAppStats, useRecentEvents } from "@/lib/useAppData";
import { formatBytes, formatTimestampMs } from "@/lib/format";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  if (!isTauri) return {} as T;
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

interface SendStartResp {
  token: string;
  code: string;
  filename: string;
  file_size: number;
  upnp_status: { external_ip: string; external_port: number } | null;
}

interface ReceiveResp {
  bytes_written: number;
  output_path: string;
  filename?: string;
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 / 1024)).toFixed(2)} GB`;
}

// ─────────────────────────────────────────────────────────────
//  Root
// ─────────────────────────────────────────────────────────────

export function ShareView({
  onComplete,
  onNavigate,
}: {
  onComplete: (op: {
    kind: "share";
    filename: string;
    originalBytes: number;
    durationMs: number;
  }) => void;
  onNavigate: (v: View) => void;
}) {
  const { t, locale } = useLocale();

  // Sprint 5.6.29: lift the share state to ShareView so both the
  // SendPanel (left) and LinkPanel (right) can render from the same
  // state without prop-drilling or context.
  const [shareResp, setShareResp] = useState<SendStartResp | null>(null);

  // Sprint 5.7.21-B-Abstract: tabs at the top so the user
  // picks ONE flow (Send or Receive) instead of seeing both
  // flows + a "selling points" header competing for attention.
  // Default to "send" because that's the more common path
  // (the user has a file they want to share).
  const [tab, setTab] = useState<"send" | "receive">("send");

  // Sprint 5.7: real share stats + recent share events so the
  // LinkPanel Actividad section shows actual past transfers,
  // not hardcoded placeholder text.
  const { stats } = useAppStats();
  const { ops: recentShares } = useRecentEvents(50, []);
  const recentShareEvents = recentShares.filter((o) => o.kind === "share").slice(0, 3);
  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-3xl mx-auto px-8 pt-8 pb-16">
        {/* Sprint 5.7.21-B-Abstract: the big page header
            (h1 + 3 selling-point chips) is gone. The TopBar
            already provides global nav; the per-page title
            and the "Unlimited / No storage / E2E" marketing
            chips were visual noise. The tabs below + the
            panel content is the only thing the user needs. */}

        {/* Tabs: Send | Receive */}
        <div className="mb-8 inline-flex items-center gap-1 p-1 rounded-2xl border border-white/[0.06] bg-white/[0.02]">
          <button
            onClick={() => setTab("send")}
            data-testid="share-tab-send"
            data-active={tab === "send"}
            className={
              "px-5 py-2 rounded-xl text-[13px] font-medium transition-all " +
              (tab === "send"
                ? "bg-cyan-500/15 text-cyan-200 border border-cyan-500/30"
                : "text-zinc-400 hover:text-zinc-200 border border-transparent")
            }
          >
            {t("share.tab.send")}
          </button>
          <button
            onClick={() => setTab("receive")}
            data-testid="share-tab-receive"
            data-active={tab === "receive"}
            className={
              "px-5 py-2 rounded-xl text-[13px] font-medium transition-all " +
              (tab === "receive"
                ? "bg-cyan-500/15 text-cyan-200 border border-cyan-500/30"
                : "text-zinc-400 hover:text-zinc-200 border border-transparent")
            }
          >
            {t("share.tab.receive")}
          </button>
        </div>

        {/* Active panel. The 2-col layout (Send | Link) is gone —
            the LinkPanel renders as a sub-section below the
            SendPanel once a share is active. The ReceivePanel
            renders alone when the receive tab is active. */}
        {tab === "send" ? (
          <div className="space-y-5">
            <SendPanel onComplete={onComplete} onRespChange={setShareResp} />
            {shareResp && (
              <LinkPanel
                resp={shareResp}
                stats={stats}
                recentShareEvents={recentShareEvents}
                locale={locale}
                t={t as (k: string, vars?: Record<string, string | number>) => string}
                onNavigate={onNavigate}
              />
            )}
          </div>
        ) : (
          <ReceivePanel />
        )}
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Send panel
// ─────────────────────────────────────────────────────────────

function SendPanel({
  onComplete,
  onRespChange,
}: {
  onComplete: (op: {
    kind: "share";
    filename: string;
    originalBytes: number;
    durationMs: number;
  }) => void;
  onRespChange?: (resp: SendStartResp | null) => void;
}) {
  const { t } = useLocale();
  const [filePath, setFilePath] = useState<string | null>(null);
  const [resp, setResp] = useState<SendStartResp | null>(null);
  const [busy, setBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pathInput, setPathInput] = useState("");
  const [dragOver, setDragOver] = useState(false);
  const [copied, setCopied] = useState<"code" | "token" | null>(null);

  // Sprint 5.6.29: bubble `resp` up to ShareView so LinkPanel can render.
  useEffect(() => {
    onRespChange?.(resp);
  }, [resp, onRespChange]);

  const acceptPath = useCallback((p: string | null) => {
    if (p) {
      setFilePath(p);
      setError(null);
      setResp(null);
      setPathInput("");
    }
  }, []);

  // drag-drop
  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(true);
  }, []);
  const onDragLeave = useCallback(() => setDragOver(false), []);
  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const paths: string[] = (e as any).detail?.paths ?? [];
      if (paths.length) acceptPath(paths[0]);
    },
    [acceptPath]
  );

  // Tauri native drag-drop
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const ev = (window as any).__TAURI__?.event;
        if (!ev?.listen) return;
        unlisten = await ev.listen("tauri://drag-drop", (e: any) => {
          const paths: string[] = e?.payload?.paths ?? [];
          if (paths.length) acceptPath(paths[0]);
        });
      } catch {}
    })();
    return () => unlisten?.();
  }, [acceptPath]);

  const onBrowseFile = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const r = await open({ multiple: false, directory: false, filters: [{ name: "All files", extensions: ["*"] }] });
      if (typeof r === "string") acceptPath(r);
    } catch {}
  }, [acceptPath]);

  const onBrowseFolder = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const r = await open({ multiple: false, directory: true });
      if (typeof r === "string") acceptPath(r);
    } catch {}
  }, [acceptPath]);

  const onSend = useCallback(async () => {
    if (!filePath) return;
    setBusy(true);
    setError(null);
    const t0 = Date.now();
    try {
      const r = await tauriInvoke<SendStartResp>("p2p_send_start_cmd", {
        req: { file_path: filePath, code: null },
      });
      setResp(r);
      onComplete({ kind: "share", filename: r.filename, originalBytes: r.file_size, durationMs: Date.now() - t0 });
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
    }
  }, [filePath, onComplete]);

  const onCancel = useCallback(async () => {
    setCancelling(true);
    try { await tauriInvoke("p2p_send_abort_cmd", {}); } catch {}
    setCancelling(false);
    setBusy(false);
    setResp(null);
  }, []);

  const onReset = useCallback(() => {
    setFilePath(null);
    setResp(null);
    setError(null);
    setPathInput("");
    setCopied(null);
  }, []);

  const onCopy = useCallback(async (text: string, which: "code" | "token") => {
    try { await navigator.clipboard.writeText(text); } catch {}
    setCopied(which);
    setTimeout(() => setCopied(null), 1800);
  }, []);

  const filename = filePath ? filePath.split("/").pop() : null;

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className="glass rounded-3xl p-6 flex flex-col gap-5"
    >
      {/* Step header */}
      <div className="flex items-center gap-2.5">
        <div className="w-8 h-8 rounded-lg bg-cyan-500/15 border border-cyan-500/30 flex items-center justify-center text-cyan-300 text-[13px] font-bold">
          1
        </div>
        <div>
          <h2 className="text-white text-[15px] font-semibold leading-tight">
            {t("share.send.step1.title")}
          </h2>
          <p className="text-zinc-500 text-[11.5px] mt-0.5">
            {t("share.send.step1.desc")}
          </p>
        </div>
      </div>

      {!resp ? (
        <>
          {/* Dropzone with file icon + CTA button */}
          <div
            className={
              "relative rounded-2xl p-10 text-center transition-all duration-200 cursor-pointer " +
              (dragOver
                ? "border-2 border-solid border-cyan-400 bg-cyan-500/[0.10] shadow-[0_0_40px_-8px_rgba(34,211,238,0.5)] scale-[1.01]"
                : "border-2 border-dashed border-white/[0.10] bg-white/[0.02] hover:border-white/[0.20] hover:bg-white/[0.03]")
            }
          >
            {dragOver && (
              <div className="absolute inset-0 rounded-2xl pointer-events-none animate-pulse-glow" />
            )}
            <div
              className={
                "mx-auto mb-4 w-16 h-16 rounded-2xl flex items-center justify-center transition-all duration-300 text-blue-500 " +
                (dragOver
                  ? "bg-blue-500/20 scale-110 rotate-[-6deg]"
                  : "bg-blue-500/10 group-hover:scale-105")
              }
            >
              {dragOver ? (
                <ArrowRight size={28} strokeWidth={2.2} />
              ) : (
                <FileText size={28} strokeWidth={1.6} />
              )}
            </div>
            <p className="text-white text-[14px] font-medium mb-1">
              {dragOver ? t("share.send.drop.active") : t("share.send.drop")}
            </p>
            <p className="text-zinc-500 text-[12px] mb-5">
              {t("share.send.drop.hint")}
            </p>
            {/* Solid cyan button (matches mockup — no gradient) */}
            <button
              onClick={onBrowseFile}
              className="inline-flex items-center gap-2 px-6 py-2.5 rounded-xl bg-blue-500 hover:bg-blue-400 text-white text-[13px] font-semibold transition-all duration-150 hover:-translate-y-px shadow-[0_4px_20px_-6px_rgba(59,130,246,0.55)]"
            >
              <File size={14} />
              {t("share.send.choose")}
            </button>
          </div>

          {/* Selected file preview — matches mockup: gray icon, simple card */}
          {filePath && (
            <div className="rounded-xl bg-white/[0.02] border border-white/[0.06] p-3 flex items-center gap-3 animate-scale-in">
              <div className="w-10 h-10 rounded-lg bg-white/[0.05] flex items-center justify-center text-zinc-400 shrink-0">
                <FileText size={18} />
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-white text-[13px] truncate font-medium">
                  {filename}
                </p>
                <p className="text-zinc-500 text-[10.5px] mt-0.5">
                  Listo para compartir
                </p>
              </div>
              <button
                onClick={() => setFilePath(null)}
                className="text-zinc-500 hover:text-white transition-colors p-1.5 rounded-md hover:bg-white/[0.05]"
                aria-label={t("share.send.remove")}
              >
                <X size={16} />
              </button>
            </div>
          )}

          {/* Advanced options accordion */}
          {filePath && (
            <details className="rounded-xl border border-white/[0.06] bg-white/[0.02] open:bg-white/[0.03] group">
              <summary className="flex items-center gap-2 px-4 py-3 cursor-pointer list-none select-none text-zinc-400 hover:text-white transition-colors">
                <Settings2 size={14} />
                <span className="text-[12px] font-medium flex-1">
                  {t("share.send.advanced")}
                </span>
                <ChevronDown
                  size={14}
                  className="transition-transform duration-200 group-open:rotate-180"
                />
              </summary>
              <div className="px-4 pb-4 flex flex-col gap-3 text-[12.5px]">
                {/* Custom slug */}
                <div className="flex items-center gap-3">
                  <LinkIcon size={14} className="text-zinc-500 shrink-0" />
                  <span className="text-zinc-300 shrink-0">
                    {t("share.send.customslug")}
                  </span>
                  <span className="text-zinc-600 text-[10.5px]">
                    ({t("share.send.optional")})
                  </span>
                  <input
                    type="text"
                    placeholder={t("share.send.customslug.placeholder")}
                    className="input flex-1 py-1.5 text-[12px] font-mono"
                  />
                </div>
                {/* Expiration */}
                <div className="flex items-center gap-3">
                  <Clock size={14} className="text-zinc-500 shrink-0" />
                  <span className="text-zinc-300 shrink-0">
                    {t("share.send.expiration")}
                  </span>
                  <select
                    defaultValue="7d"
                    className="ml-auto bg-transparent border border-white/[0.08] rounded-lg px-3 py-1.5 text-[12px] text-zinc-200 focus:outline-none focus:border-cyan-500/50 cursor-pointer hover:border-white/[0.16] transition-colors"
                  >
                    <option value="1d">1 {t("share.send.day")}</option>
                    <option value="7d">7 {t("share.send.days")}</option>
                    <option value="30d">30 {t("share.send.days")}</option>
                    <option value="never">{t("share.send.never")}</option>
                  </select>
                </div>
                {/* Download limit */}
                <div className="flex items-center gap-3">
                  <Download size={14} className="text-zinc-500 shrink-0" />
                  <span className="text-zinc-300 shrink-0">
                    {t("share.send.downloadlimit")}
                  </span>
                  <select
                    defaultValue="none"
                    className="ml-auto bg-transparent border border-white/[0.08] rounded-lg px-3 py-1.5 text-[12px] text-zinc-200 focus:outline-none focus:border-cyan-500/50 cursor-pointer hover:border-white/[0.16] transition-colors"
                  >
                    <option value="none">{t("share.send.unlimited")}</option>
                    <option value="5">5</option>
                    <option value="10">10</option>
                    <option value="50">50</option>
                    <option value="100">100</option>
                  </select>
                </div>
              </div>
            </details>
          )}

          {/* Big gradient CTA */}
          <button
            onClick={onSend}
            disabled={!filePath || busy}
            className="w-full py-4 rounded-2xl font-semibold text-[14.5px] text-white relative overflow-hidden disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-200 hover:scale-[1.01] active:scale-[0.99] disabled:hover:scale-100"
            style={{
              background: busy
                ? "linear-gradient(135deg, #34d399 0%, #22d3ee 100%)"
                : "linear-gradient(135deg, #34d399 0%, #10b981 35%, #22d3ee 100%)",
              boxShadow: !busy && filePath
                ? "0 8px 32px -8px rgba(34,211,238,0.55), 0 0 0 1px rgba(255,255,255,0.08) inset"
                : "none",
            }}
          >
            <span className="relative z-10 flex items-center justify-center gap-2">
              {busy ? (
                <>
                  <RefreshCw size={16} className="animate-spin-slow" />
                  {t("share.send.btn.busy")}
                </>
              ) : (
                <>
                  <LinkIcon size={16} />
                  {t("share.send.cta")}
                </>
              )}
            </span>
          </button>
          <p className="text-zinc-500 text-[11.5px] text-center -mt-2">
            {t("share.send.cta.sub")}
          </p>
        </>
      ) : (
        /* Active link — show summary in left card too */
        <div className="rounded-2xl bg-emerald-500/[0.06] border border-emerald-500/20 p-5 flex flex-col gap-4 animate-scale-in">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-rose-500/15 border border-rose-500/30 flex items-center justify-center text-rose-300 shrink-0">
              <FileText size={18} />
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-white text-[13.5px] truncate font-semibold">
                {resp.filename}
              </p>
              <p className="text-zinc-500 text-[11px] font-mono mt-0.5">
                {prettyBytes(resp.file_size)}
              </p>
            </div>
            <button
              onClick={onReset}
              className="btn btn-ghost"
            >
              <RefreshCw size={12} />
              {t("share.send.new")}
            </button>
          </div>
        </div>
      )}

      {/* Error toast */}
      {error && (
        <div className="rounded-xl bg-rose-500/[0.08] border border-rose-500/30 px-4 py-3 text-rose-300 text-[13px] flex items-start gap-2.5 animate-scale-in">
          <AlertCircle size={15} className="shrink-0 mt-0.5" />
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Receive panel
// ─────────────────────────────────────────────────────────────

function ReceivePanel() {
  const { t } = useLocale();
  const [token, setToken] = useState("");
  const [outputPath, setOutputPath] = useState("");
  const [resolvedDownloads, setResolvedDownloads] = useState("");
  const [resolvedHome, setResolvedHome] = useState("");
  const [suggestedName, setSuggestedName] = useState("");
  // Sprint 5.6.25: tracks whether the user has manually picked
  // a destination via the save dialog. While false, the field
  // auto-syncs to `<resolvedDownloads>/<suggestedName>` whenever
  // the filename becomes known. While true, the auto-sync is
  // disabled so we don't clobber the user's chosen destination.
  // Reset to false whenever the user changes the token (handled
  // by a dedicated useEffect that watches [token] only).
  const [userEditedPath, setUserEditedPath] = useState(false);

  // Sprint 5.6.26: true while the v2/v3 filename probe is in
  // flight. While true, the "Cambiar" button is disabled so the
  // user can't open the save dialog with a fallback default and
  // pick a path before the real filename is known. Prevents the
  // race: "user clicks Cambiar → probe resolves → user has a
  // path with the wrong filename and we can't auto-fix it".
  const [probing, setProbing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ReceiveResp | null>(null);
  const [stepIdx, setStepIdx] = useState(-1);
  const [stepErr, setStepErr] = useState(false);

  const kind = (() => {
    const tk = token.trim();
    if (tk.startsWith("nx:1:")) return "v1" as const;
    if (tk.startsWith("nx:2:")) return "v2" as const;
    if (tk.startsWith("nx:3:")) return "v3" as const;
    if (/^[a-z0-9]+-[a-z0-9]+-[a-z0-9]+-[a-z0-9]+$/.test(tk)) return "v2" as const;
    return "unknown" as const;
  })();

  // Resolve home dir
  useEffect(() => {
    if (!isTauri) return;
    (async () => {
      try {
        const { homeDir, join } = await import("@tauri-apps/api/path");
        const home = await homeDir();
        setResolvedHome(home);
        setResolvedDownloads(await join(home, "Downloads"));
      } catch {}
    })();
  }, []);

// Sprint 5.6.26: separate effect for the manual-edit reset.
// Triggered ONLY by the user's actual onChange action (typing,
// pasting, clearing the token field). Decoupled from the
// probe logic so the behaviour is predictable regardless of
// probe timing — clearing the token resets the flag, pasting a
// new token resets the flag, no probe race can ever leak an
// unwanted reset.
  useEffect(() => {
    setUserEditedPath(false);
  }, [token]);

// Peek filename from v1 token (parsed locally) and from v2/v3
  // (probes the sender's /meta endpoint via p2p_peek_filename_cmd).
  // Sprint 5.6.25: works for ALL token versions now, so the path
  // field can auto-fill the moment the user pastes a token — no
  // waiting for the receive round-trip to complete.
  useEffect(() => {
    const tk = token.trim();
    if (!tk) { setSuggestedName(""); setProbing(false); return; }
    if (kind === "v1") {
      try {
        const b64 = tk.slice("nx:1:".length);
        const json = JSON.parse(atob(b64.replace(/-/g, "+").replace(/_/g, "/")));
        if (json.filename) setSuggestedName(json.filename);
      } catch {}
      return;
    }
    if (kind === "v2" || kind === "v3") {
      // Probe the sender's /meta to read the original filename.
      // We don't wait — fire-and-forget. The path field will
      // auto-update if the user hasn't manually edited it yet.
      //
      // Sprint 5.6.26: while probing, the "Cambiar" button is
      // disabled and a subtle spinner shows next to the path.
      // This kills the race "user clicks Cambiar → opens dialog
      // with fallback → picks dir → probe resolves → user has
      // a path with the wrong filename".
      setProbing(true);
      let cancelled = false;
      (async () => {
        try {
          const r = await tauriInvoke<{ filename: string | null }>(
            "p2p_peek_filename_cmd",
            { req: { token: tk, timeout_secs: 3 } }
          );
          if (!cancelled) {
            if (r?.filename) setSuggestedName(r.filename);
            setProbing(false);
          }
        } catch {
          // Sender not reachable / token stale — silently skip.
          // The path field will still default to ~/Downloads
          // with the suggestedName being empty (so just the dir).
          if (!cancelled) setProbing(false);
        }
      })();
      return () => { cancelled = true; };
    }
  }, [token, kind]);

  // Sprint 5.6.25: auto-sync outputPath with the resolved final
  // destination. The path field is ALWAYS the real final path
  // the backend will write to — no sentinels, no placeholder
  // detection, no exception lists. When suggestedName arrives
  // AND the user hasn't manually overridden the destination,
  // we populate the field with `<resolvedDownloads>/<filename>`.
  //
  // This is the architectural fix: instead of trying to detect
  // "did the user type a placeholder?" (which requires a
  // forever-growing list of patterns), we ensure the field
  // shows the correct path from the start. The user only
  // needs to type if they want a different destination.
  useEffect(() => {
    if (userEditedPath) return;       // respect manual override
    if (!resolvedDownloads) return;   // home not resolved yet
    if (!suggestedName) return;       // no filename known yet
    let cancelled = false;
    (async () => {
      try {
        const { join } = await import("@tauri-apps/api/path");
        const full = await join(resolvedDownloads, suggestedName);
        if (!cancelled && outputPath !== full) setOutputPath(full);
      } catch {}
    })();
    return () => { cancelled = true; };
  }, [resolvedDownloads, suggestedName, userEditedPath]);

  const STEPS = kind === "v1"
    ? [
        t("share.receive.step.detecting"),
        t("share.receive.step.connecting"),
        t("share.receive.step.handshake"),
        t("share.receive.step.transferring"),
        t("share.receive.step.verifying"),
      ]
    : [
        t("share.receive.step.detecting"),
        t("share.receive.step.finding"),
        t("share.receive.step.handshake"),
        t("share.receive.step.transferring"),
        t("share.receive.step.verifying"),
      ];

  // Sprint 5.6.25: resolve the final output path. Since the
  // `outputPath` field is ALWAYS the real final path (kept in
  // sync via the useEffect above), this function only needs to
  // expand a leading `~/` if present. No placeholder detection,
  // no exception lists, no heuristics.
  const buildOutputPath = useCallback(async (): Promise<string> => {
    const { join } = await import("@tauri-apps/api/path");
    let p = outputPath;
    if (p.startsWith("~/") && resolvedHome) {
      p = await join(resolvedHome, p.slice(2));
    } else if (p === "~" && resolvedHome) {
      p = resolvedHome;
    }
    return p;
  }, [outputPath, resolvedHome]);

  const onChangeDest = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const { join, homeDir } = await import("@tauri-apps/api/path");
      // Sprint 5.6.25: never open the save dialog with a
      // placeholder default like "recibido" or "archivo_recibido.bin".
      // If we don't have resolvedDownloads yet (the mount-time
      // useEffect hasn't fired), resolve it NOW before opening
      // the dialog — otherwise the user could type "Untitled.*"
      // and we'd save with that literal filename.
      //
      // Pre-fill strategy:
      //   * outputPath already populated (auto-sync fired) → use it
      //   * suggestedName known but outputPath empty (race) → dir + name
      //   * neither → just the dir (no default filename)
      let dir = resolvedDownloads;
      if (!dir) {
        try {
          const home = await homeDir();
          dir = await join(home, "Downloads");
        } catch {}
      }
      const defPath = outputPath || (dir && suggestedName ? await join(dir, suggestedName) : dir) || undefined;
      const r = await save({
        defaultPath: defPath,
        filters: [{ name: "All files", extensions: ["*"] }],
      });
      if (r) {
        setOutputPath(r);
        setUserEditedPath(true);  // dialog pick = manual choice
      }
    } catch {}
  }, [outputPath, resolvedDownloads, suggestedName]);

  const onReceive = useCallback(async () => {
    if (!token || kind === "unknown") return;
    setBusy(true);
    setError(null);
    setResult(null);
    setStepIdx(0);
    setStepErr(false);

    let actualPath: string;
    try { actualPath = await buildOutputPath(); }
    catch (e: any) { setError(String(e?.message ?? e)); setBusy(false); return; }

    setStepIdx(1);
    try {
      let r: ReceiveResp;
      if (kind === "v2" || kind === "v3") {
        r = await tauriInvoke<ReceiveResp>("p2p_receive_direct_cmd", {
          req: { token: token.trim(), output_path: actualPath, timeout_secs: 5 },
        });
      } else {
        r = await tauriInvoke<ReceiveResp>("p2p_receive_cmd", {
          req: { token: token.trim(), output_path: actualPath },
        });
      }
      if (r.filename) setSuggestedName(r.filename);
      setStepIdx(4);
      setResult(r);
      // Sprint 5.6.24: sync the input field with the actual
      // path the backend wrote to. Previously the user's typed
      // "Untitled.*" stayed in the field, confusing them after
      // the success card appeared (which shows the new name).
      if (r.output_path) setOutputPath(r.output_path);
    } catch (e: any) {
      setError(friendlyError(String(e?.message ?? e)));
      setStepErr(true);
    } finally {
      setBusy(false);
    }
  }, [token, kind, buildOutputPath]);

  return (
    <div className="glass rounded-3xl p-6 flex flex-col gap-5">
      {/* Step header */}
      <div className="flex items-center gap-2.5">
        <div className="w-8 h-8 rounded-lg bg-cyan-500/15 border border-cyan-500/30 flex items-center justify-center text-cyan-300">
          <Download size={16} strokeWidth={2.4} />
        </div>
        <div>
          <h2 className="text-white text-[15px] font-semibold leading-tight">
            {t("share.receive.title")}
          </h2>
          <p className="text-zinc-500 text-[11.5px] mt-0.5">
            {t("share.receive.desc")}
          </p>
        </div>
      </div>

      {/* Token input */}
      <div>
        <label className="text-zinc-500 text-[10px] tracking-[0.2em] uppercase font-medium pl-1 block mb-2">
          {t("share.receive.token.label")}
        </label>
        <textarea
          value={token}
          onChange={(e) => {
            setToken(e.target.value);
            setResult(null);
            setError(null);
          }}
          rows={3}
          placeholder={t("share.receive.token.placeholder")}
          className="input w-full font-mono text-[12.5px] resize-none"
        />
        {kind !== "unknown" && token.length > 0 && (
          <p
            className={
              "text-[11px] mt-1.5 pl-1 " +
              (kind === "v3"
                ? "text-emerald-400"
                : "text-amber-400")
            }
          >
            <span className="inline-flex items-center gap-1.5">
              <span
                className={
                  "w-1.5 h-1.5 rounded-full " +
                  (kind === "v3" ? "bg-emerald-400" : "bg-amber-400")
                }
              />
              {kind === "v3"
                ? t("share.receive.token.v3")
                : kind === "v2"
                ? t("share.receive.token.v2")
                : t("share.receive.token.v1")}
            </span>
          </p>
        )}
        {kind === "unknown" && token.length > 4 && (
          <p className="text-rose-400 text-[11px] mt-1.5 pl-1">
            {t("share.receive.token.unknown")}
          </p>
        )}
      </div>

      {/* Destination card */}
      <div className="rounded-xl bg-white/[0.02] border border-white/[0.06] p-3">
        <div className="text-zinc-500 text-[10px] tracking-[0.2em] uppercase font-medium mb-2 pl-1">
          {t("share.receive.dest.label")}
        </div>
        <div className="flex items-center gap-2.5">
          <div className="w-9 h-9 rounded-lg bg-white/[0.05] flex items-center justify-center text-zinc-400 shrink-0">
            <FolderOpen size={16} />
          </div>
          <span className="flex-1 text-zinc-300 text-[12px] font-mono truncate">
            {outputPath || (
              <span className="text-zinc-600 italic">
                {probing
                  ? t("share.receive.dest.probing")
                  : t("share.receive.dest.empty")}
              </span>
            )}
          </span>
          {probing && (
            <span className="inline-block w-3 h-3 border border-cyan-500/40 border-t-cyan-400 rounded-full animate-spin shrink-0" />
          )}
          <button
            onClick={onChangeDest}
            disabled={probing}
            title={probing ? t("share.receive.dest.change.waiting") : undefined}
            className="text-zinc-500 hover:text-white disabled:text-zinc-700 disabled:cursor-not-allowed text-[11px] font-medium transition-colors shrink-0 px-2 py-1 rounded-md hover:bg-white/[0.05]"
          >
            {t("share.receive.dest.change")}
          </button>
        </div>
        {suggestedName && (
          <p className="text-zinc-500 text-[11px] mt-2 pl-1 flex items-center gap-1.5">
            <FileText size={11} className="text-cyan-400 shrink-0" />
            <span className="truncate">{suggestedName}</span>
          </p>
        )}
      </div>

      {/* Big CTA — solid blue matching the Send "Elegir archivo" */}
      <button
        onClick={onReceive}
        disabled={!token || busy || kind === "unknown"}
        className="w-full py-3.5 rounded-2xl bg-blue-500 hover:bg-blue-400 disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-[14px] font-semibold transition-all duration-150 hover:-translate-y-px disabled:hover:translate-y-0 inline-flex items-center justify-center gap-2 shadow-[0_4px_20px_-6px_rgba(59,130,246,0.55)] disabled:shadow-none"
      >
        {busy ? (
          <>
            <RefreshCw size={16} className="animate-spin-slow" />
            {t("share.receive.btn.busy")}
          </>
        ) : (
          <>
            <Download size={16} />
            {t("share.receive.btn")}
          </>
        )}
      </button>

      {/* Progress steps */}
      {busy && stepIdx >= 0 && (
        <div className="rounded-xl bg-white/[0.02] border border-white/[0.06] px-4 py-3 space-y-2.5 animate-fade-in">
          {STEPS.map((label, i) => {
            const done = i < stepIdx;
            const active = i === stepIdx && !stepErr;
            const err = i === stepIdx && stepErr;
            return (
              <div
                key={i}
                className={
                  "flex items-center gap-2.5 text-[12px] " +
                  (done
                    ? "text-emerald-400"
                    : active
                    ? "text-cyan-300"
                    : err
                    ? "text-rose-400"
                    : "text-zinc-700")
                }
              >
                <span className="w-4 text-center shrink-0">
                  {done ? (
                    <Check size={14} strokeWidth={3} />
                  ) : active ? (
                    <RefreshCw size={12} className="animate-spin-slow inline-block" />
                  ) : err ? (
                    <X size={14} />
                  ) : (
                    <span className="opacity-50">·</span>
                  )}
                </span>
                <span className={active ? "font-medium" : ""}>{label}</span>
              </div>
            );
          })}
        </div>
      )}

      {/* Success result */}
      {result && !busy && (
        <div className="rounded-2xl bg-emerald-500/[0.06] border border-emerald-500/20 p-5 animate-scale-in">
          <div className="flex items-center gap-2 mb-2">
            <div className="w-7 h-7 rounded-lg bg-emerald-500/20 flex items-center justify-center text-emerald-300">
              <Check size={14} strokeWidth={3} />
            </div>
            <p className="text-emerald-300 text-[10px] tracking-[0.2em] uppercase font-medium">
              {t("share.receive.result.title")}
            </p>
          </div>
          <p className="text-white text-[24px] font-semibold tabular-nums mb-1">
            {prettyBytes(result.bytes_written)}
          </p>
          <p className="text-zinc-200 text-[13px] font-medium truncate mb-1">
            {result.filename || "archivo"}
          </p>
          <p
            className="text-zinc-500 text-[11px] font-mono truncate"
            title={result.output_path}
          >
            ▸ {result.output_path}
          </p>
        </div>
      )}

      {/* Error toast */}
      {error && !busy && (
        <div className="rounded-xl bg-rose-500/[0.08] border border-rose-500/30 px-4 py-3 text-rose-300 text-[13px] flex items-start gap-2.5 animate-scale-in">
          <AlertCircle size={15} className="shrink-0 mt-0.5" />
          <div className="flex-1">
            <span>{error}</span>
            <button
              onClick={onReceive}
              className="block mt-2 text-zinc-400 hover:text-white text-[11px] font-medium transition-colors"
            >
              {t("share.receive.retry")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Link panel — right column with URL, QR, feature pills,
//  Vista previa, Actividad
// ─────────────────────────────────────────────────────────────

function LinkPanel({
  resp,
  stats,
  recentShareEvents,
  locale,
  t,
  onNavigate,
}: {
  resp: SendStartResp | null;
  stats: import("@/lib/useAppData").AppStats | null;
  recentShareEvents: import("@/lib/useAppData").RecentOp[];
  locale: string;
  t: (k: string, vars?: Record<string, string | number>) => string;
  onNavigate?: (v: View) => void;
}) {
  const [copied, setCopied] = useState<"link" | "token" | null>(null);
  const [showQR, setShowQR] = useState(false);
  const [showToken, setShowToken] = useState(false);

  // Build a pretty URL for display. The token from the backend
  // already encodes the slug; we wrap it in a friendly URL.
  const linkUrl = resp
    ? `https://nexuscompress.dev/d/${resp.code.toLowerCase()}`
    : null;

  const onCopy = useCallback(async (text: string, which: "link" | "token") => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {}
    setCopied(which);
    setTimeout(() => setCopied(null), 1800);
  }, []);

  return (
    <div className="glass rounded-3xl p-6 flex flex-col gap-5">
      {/* Step header with status pill */}
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded-lg bg-cyan-500/15 border border-cyan-500/30 flex items-center justify-center text-cyan-300 text-[13px] font-bold">
            2
          </div>
          <div>
            <h2 className="text-white text-[15px] font-semibold leading-tight">
              {t("share.link.title")}
            </h2>
            <p className="text-zinc-500 text-[11.5px] mt-0.5">
              {t("share.link.desc")}
            </p>
          </div>
        </div>
        {resp && (
          <span className="chip chip-emerald shrink-0">
            <span className="relative flex w-2 h-2">
              <span className="absolute inline-flex h-full w-full rounded-full opacity-75 animate-ping bg-emerald-400" />
              <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-400" />
            </span>
            {t("share.link.active")}
          </span>
        )}
      </div>

      {resp && linkUrl ? (
        <>
          {/* Link display card — friendly URL + QR + copy */}
          <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] p-4 flex items-center gap-3 animate-scale-in">
            <div className="w-9 h-9 rounded-lg bg-white/[0.05] flex items-center justify-center text-zinc-400 shrink-0">
              <LinkIcon size={16} />
            </div>
            <p className="flex-1 text-white text-[13px] font-mono truncate">
              {linkUrl}
            </p>
            <button
              onClick={() => onCopy(linkUrl, "link")}
              className={
                "inline-flex items-center gap-1.5 px-3.5 py-2 rounded-xl text-[13px] font-semibold transition-all duration-150 " +
                (copied === "link"
                  ? "bg-emerald-500/20 text-emerald-300"
                  : "bg-blue-500 hover:bg-blue-400 text-white shadow-[0_4px_16px_-4px_rgba(59,130,246,0.5)]")
              }
            >
              {copied === "link" ? (
                <>
                  <Check size={14} />
                  {t("share.link.copied")}
                </>
              ) : (
                <>
                  <Copy size={14} />
                  {t("share.link.copy")}
                </>
              )}
            </button>
            <button
              onClick={() => setShowQR((v) => !v)}
              className={
                "btn btn-ghost px-3 " +
                (showQR ? "bg-white/[0.08] text-white" : "")
              }
              title={t("share.link.qr")}
            >
              <QrCode size={14} />
            </button>
          </div>

          {/* Raw token display — what the receiver actually pastes */}
          <div className="rounded-2xl bg-cyan-500/[0.04] border border-cyan-500/20 p-3 animate-scale-in">
            <div className="flex items-center gap-2 mb-2">
              <KeyRound size={12} className="text-cyan-400 shrink-0" />
              <span className="text-cyan-300 text-[10px] tracking-[0.2em] uppercase font-medium">
                {t("share.link.token.label")}
              </span>
              <button
                onClick={() => setShowToken((v) => !v)}
                className="ml-auto text-zinc-500 hover:text-white text-[10px] flex items-center gap-1 transition-colors"
              >
                {showToken ? <EyeOff size={10} /> : <Eye size={10} />}
                {showToken ? t("share.link.token.hide") : t("share.link.token.show")}
              </button>
            </div>
            <div className="flex items-center gap-2">
              <p className="flex-1 text-white text-[11.5px] font-mono truncate">
                {showToken
                  ? resp.token
                  : "•".repeat(Math.min(resp.token.length, 28))}
              </p>
              <button
                onClick={() => onCopy(resp.token, "token")}
                className={
                  "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-semibold transition-all duration-150 shrink-0 " +
                  (copied === "token"
                    ? "bg-emerald-500/20 text-emerald-300"
                    : "bg-cyan-500 hover:bg-cyan-400 text-white")
                }
              >
                {copied === "token" ? (
                  <>
                    <Check size={12} />
                    {t("share.link.copied")}
                  </>
                ) : (
                  <>
                    <Copy size={12} />
                    {t("share.link.copy.token")}
                  </>
                )}
              </button>
            </div>
          </div>

          {/* QR code (collapsible) */}
          {showQR && (
            <div className="rounded-2xl bg-white p-4 flex flex-col items-center gap-3 animate-scale-in">
              <div className="rounded-xl bg-white p-2">
                <QRCodeSVG
                  value={linkUrl}
                  size={180}
                  level="M"
                  bgColor="#ffffff"
                  fgColor="#0a0a0c"
                />
              </div>
              <p className="text-zinc-600 text-[11.5px] font-mono">
                {t("share.link.qr.scan")}
              </p>
            </div>
          )}

          {/* Feature pills row removed in 5.7.21-B-Abstract —
              the 3 pills (Expiración / Ilimitado / E2E) just
              repeated the selling points that used to live in
              the page header. The user already knows the share
              is encrypted + direct; no need to remind them. */}

          {/* Vista previa — kept: the user wants to confirm the
              file is the one they sent (especially for transfers
              that take >1 minute to connect). */}
          <div>
            <h3 className="text-zinc-400 text-[10px] tracking-[0.2em] uppercase font-medium mb-3">
              {t("share.link.preview")}
            </h3>
            <div className="rounded-xl bg-white/[0.02] border border-white/[0.06] p-3 flex items-center gap-3">
              <div className="w-10 h-10 rounded-lg bg-rose-500/15 border border-rose-500/30 flex items-center justify-center text-rose-300 shrink-0">
                <FileText size={18} />
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-white text-[13px] truncate font-medium">
                  {resp.filename}
                </p>
                <p className="text-zinc-500 text-[10.5px] font-mono mt-0.5">
                  {prettyBytes(resp.file_size)} · {t("share.link.preview.doc")}
                </p>
              </div>
            </div>
          </div>

          {/* Actividad — Sprint 5.7: real past transfers from db.rs.
              Kept compact (single column, no card per row) so it
              doesn't compete with the link/token/QR above for
              visual attention. */}
          <div>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-zinc-400 text-[10px] tracking-[0.2em] uppercase font-medium">
                {t("share.link.activity")}
              </h3>
              {recentShareEvents.length > 0 && (
                <button
                  onClick={() => onNavigate && onNavigate("recent")}
                  className="text-zinc-500 hover:text-cyan-400 text-[10.5px] transition-colors flex items-center gap-1"
                >
                  {t("share.link.activity.seeall")}
                  <ArrowRight size={10} />
                </button>
              )}
            </div>
            {recentShareEvents.length === 0 ? (
              <p className="text-zinc-600 text-[12px] italic">
                {t("share.link.activity.empty")}
              </p>
            ) : (
              <div className="rounded-xl border border-white/[0.06] bg-white/[0.02] divide-y divide-white/[0.04]">
                {recentShareEvents.map((ev) => (
                  <div
                    key={ev.id}
                    className="px-3 py-2 flex items-center gap-3"
                  >
                    <div className="flex-1 min-w-0">
                      <p className="text-zinc-200 text-[12.5px] truncate">
                        {ev.filename}
                      </p>
                      <p className="text-zinc-600 text-[10.5px] mt-0.5 font-mono">
                        {formatBytes(ev.originalBytes, locale)}
                      </p>
                    </div>
                    <span className="text-zinc-500 text-[10.5px] shrink-0 tabular-nums">
                      {formatTimestampMs(ev.timestamp, t)}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </>
      ) : (
        /* Empty state when no link is active */
        <div className="rounded-2xl border-2 border-dashed border-white/[0.08] p-10 flex flex-col items-center text-center gap-3 min-h-[280px] justify-center">
          <div className="w-14 h-14 rounded-2xl bg-white/[0.04] flex items-center justify-center text-zinc-500">
            <LinkIcon size={26} strokeWidth={1.5} />
          </div>
          <div>
            <p className="text-zinc-400 text-[13.5px] font-medium mb-1">
              {t("share.link.empty.title")}
            </p>
            <p className="text-zinc-600 text-[12px] max-w-xs mx-auto leading-relaxed">
              {t("share.link.empty.desc")}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}


function friendlyError(msg: string): string {
  if (msg.includes("mDNS") || msg.includes("no service")) return "No se encontró el dispositivo — asegúrate de estar en la misma Wi-Fi.";
  if (msg.includes("timeout") || msg.includes("timed out")) return "Tiempo de espera agotado — verifica que el otro dispositivo siga activo.";
  if (msg.includes("authentication") || msg.includes("wrong code")) return "Código incorrecto — copia el token completo del remitente.";
  if (msg.includes("SHA-256") || msg.includes("corrupted")) return "Archivo corrupto durante la transferencia — intenta de nuevo.";
  return msg;
}
