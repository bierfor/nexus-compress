"use client";

import { useEffect, useState, useCallback } from "react";
import { type View } from "@/components/NeoTopBar";
import { PageHeader } from "@/components/PageHeader";
import { useLocale } from "@/components/LocaleProvider";

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
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
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
  const { t } = useLocale();
  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-4xl mx-auto px-8 pt-10 pb-16">
        <PageHeader title={t("share.title")} onBack={() => onNavigate("landing")}>
          {t("share.desc")}
        </PageHeader>
        <div className="grid grid-cols-2 gap-5">
          <SendPanel onComplete={onComplete} />
          <ReceivePanel />
        </div>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────
//  Send panel
// ─────────────────────────────────────────────────────────────

function SendPanel({
  onComplete,
}: {
  onComplete: (op: {
    kind: "share";
    filename: string;
    originalBytes: number;
    durationMs: number;
  }) => void;
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
      className="flex flex-col gap-4"
    >
      {/* Column label */}
      <div className="flex items-center gap-2">
        <span className="text-emerald-400 text-[18px]">📤</span>
        <h2 className="text-white text-[16px] font-semibold">{t("share.send.title")}</h2>
      </div>

      {!resp ? (
        <>
          {/* Drop zone */}
          <div
            className={`rounded-2xl border-2 border-dashed p-8 text-center transition-colors ${
              dragOver
                ? "border-emerald-400 bg-emerald-500/[0.06]"
                : "border-white/[0.08] bg-white/[0.02]"
            }`}
          >
            <div className="text-4xl mb-3 select-none">{dragOver ? "⤓" : "🚀"}</div>
            <p className="text-zinc-400 text-[13px] mb-4">
              {dragOver ? t("share.send.drop.active") : t("share.send.drop")}
            </p>
            <div className="flex justify-center gap-2 flex-wrap">
              <button
                onClick={onBrowseFile}
                className="px-3 py-1.5 text-[12px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-lg transition-colors"
              >
                {"📄 " + t("share.send.file")}
              </button>
              <button
                onClick={onBrowseFolder}
                className="px-3 py-1.5 text-[12px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-lg transition-colors"
              >
                {"📁 " + t("share.send.folder")}
              </button>
            </div>
          </div>

          {/* Path input */}
          <div className="flex gap-2">
            <input
              type="text"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && pathInput.trim() && acceptPath(pathInput.trim())}
              placeholder={t("share.send.path")}
              className="flex-1 bg-white/[0.04] border border-white/[0.08] rounded-xl px-3 py-2 text-[12px] text-white placeholder-zinc-600 focus:outline-none focus:border-emerald-500/40 font-mono"
            />
            <button
              onClick={() => pathInput.trim() && acceptPath(pathInput.trim())}
              disabled={!pathInput.trim()}
              className="px-3 py-2 text-[12px] text-emerald-400 border border-emerald-500/30 rounded-xl hover:bg-emerald-500/10 disabled:opacity-30 transition-colors"
            >
              {t("share.send.use")}
            </button>
          </div>

          {/* Hint below input */}
          <p className="text-zinc-600 text-[11px] -mt-2">{t("share.send.drop.hint")}</p>

          {/* Selected file */}
          {filePath && (
            <div className="rounded-xl bg-white/[0.03] border border-white/[0.06] px-4 py-3 flex items-center gap-3">
              <span className="text-emerald-400 text-[15px]">📄</span>
              <span className="flex-1 text-white text-[13px] truncate font-medium">{filename}</span>
              <button onClick={() => setFilePath(null)} className="text-zinc-600 hover:text-red-400 transition-colors text-[12px]">✕</button>
            </div>
          )}

          {/* Action */}
          <div className="flex gap-2">
            <button
              onClick={onSend}
              disabled={!filePath || busy}
              className="flex-1 py-3 rounded-xl bg-emerald-500 hover:bg-emerald-400 disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-[14px] font-semibold transition-colors"
            >
              {busy ? t("share.send.btn.busy") : t("share.send.btn")}
            </button>
            {busy && (
              <button
                onClick={onCancel}
                disabled={cancelling}
                className="px-4 py-3 rounded-xl border border-white/[0.08] text-zinc-400 hover:text-red-400 hover:border-red-500/30 text-[13px] transition-colors disabled:opacity-40"
              >
                {cancelling ? "…" : t("share.send.cancel")}
              </button>
            )}
          </div>
        </>
      ) : (
        /* Code display */
        <div className="rounded-2xl bg-emerald-500/[0.06] border border-emerald-500/20 p-6 flex flex-col gap-5">
          {/* Status pill */}
          <div className={`self-start flex items-center gap-2 px-3 py-1 rounded-full text-[11px] font-medium ${
            resp.upnp_status ? "bg-emerald-500/20 text-emerald-300" : "bg-amber-500/20 text-amber-300"
          }`}>
            <span className="w-1.5 h-1.5 rounded-full bg-current" />
            {resp.upnp_status ? t("share.send.status.anynet") : t("share.send.status.samewifi")}
          </div>

          {/* Code */}
          <div className="text-center">
            <p className="text-zinc-500 text-[10px] tracking-[0.3em] uppercase mb-2">
              {t("share.send.code.label")}
            </p>
            <p className="text-white text-[32px] font-mono font-bold tracking-[0.1em] select-all leading-none">
              {resp.code}
            </p>
            <p className="text-zinc-600 text-[12px] mt-2">{resp.filename} · {prettyBytes(resp.file_size)}</p>
          </div>

          {/* Copy buttons */}
          <div className="flex flex-col gap-2">
            <button
              onClick={() => onCopy(resp.code, "code")}
              className="w-full py-2.5 rounded-xl bg-white text-black text-[13px] font-semibold hover:bg-zinc-100 transition-colors"
            >
              {copied === "code" ? t("share.send.copied") : t("share.send.copy.code")}
            </button>
            <div className="flex gap-2">
              <button
                onClick={() => onCopy(resp.token, "token")}
                className="flex-1 py-2.5 rounded-xl bg-white/[0.06] border border-white/[0.08] text-zinc-300 text-[12px] hover:bg-white/[0.1] transition-colors"
              >
                {copied === "token" ? t("share.send.token.copied") : t("share.send.copy.token")}
              </button>
              <button
                onClick={onReset}
                className="px-4 py-2.5 rounded-xl bg-white/[0.04] border border-white/[0.06] text-zinc-500 text-[12px] hover:text-zinc-300 hover:bg-white/[0.08] transition-colors"
              >
                {t("share.send.new")}
              </button>
            </div>
          </div>

          {resp.upnp_status && (
            <p className="text-zinc-700 text-[10px] font-mono text-center">
              {resp.upnp_status.external_ip}:{resp.upnp_status.external_port}
            </p>
          )}
        </div>
      )}

      {error && (
        <div className="rounded-xl bg-red-500/[0.08] border border-red-500/20 px-4 py-3 text-red-400 text-[13px]">
          {error}
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
    <div className="flex flex-col gap-4">
      {/* Column label */}
      <div className="flex items-center gap-2">
        <span className="text-cyan-400 text-[18px]">📥</span>
        <h2 className="text-white text-[16px] font-semibold">{t("share.receive.title")}</h2>
      </div>

      {/* Token */}
      <div className="flex flex-col gap-1.5">
        <label className="text-zinc-500 text-[11px] tracking-[0.15em] uppercase">
          {t("share.receive.token.label")}
        </label>
        <textarea
          value={token}
          onChange={(e) => { setToken(e.target.value); setResult(null); setError(null); }}
          rows={3}
          placeholder={t("share.receive.token.placeholder")}
          className="w-full bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-3 text-[13px] text-white font-mono placeholder-zinc-600 focus:outline-none focus:border-cyan-500/40 resize-none"
        />
        {kind !== "unknown" && token.length > 0 && (
          <p className={`text-[11px] ${kind === "v3" ? "text-emerald-400" : "text-amber-400"}`}>
            {kind === "v3"
              ? t("share.receive.token.v3")
              : kind === "v2"
              ? t("share.receive.token.v2")
              : t("share.receive.token.v1")}
          </p>
        )}
        {kind === "unknown" && token.length > 4 && (
          <p className="text-red-400 text-[11px]">{t("share.receive.token.unknown")}</p>
        )}
      </div>

      {/* Destination */}
      <div className="rounded-xl bg-white/[0.03] border border-white/[0.06] px-4 py-3">
        <div className="text-zinc-500 text-[10px] tracking-[0.15em] uppercase mb-1.5">
          {t("share.receive.dest.label")}
        </div>
        <div className="flex items-center gap-2">
          <span className="text-zinc-500">📁</span>
          <span className="flex-1 text-zinc-300 text-[12px] font-mono truncate">
            {outputPath || (
              <span className="text-zinc-600 italic">
                {probing ? t("share.receive.dest.probing") : t("share.receive.dest.empty")}
              </span>
            )}
          </span>
          {/* Sprint 5.6.26: subtle spinner while probing, to
              signal that the system is fetching the real
              filename and the field will auto-fill shortly. */}
          {probing && (
            <span className="inline-block w-3 h-3 border border-cyan-500/40 border-t-cyan-400 rounded-full animate-spin shrink-0" />
          )}
          <button
            onClick={onChangeDest}
            disabled={probing}
            title={probing ? t("share.receive.dest.change.waiting") : undefined}
            className="text-zinc-500 hover:text-zinc-300 disabled:text-zinc-700 disabled:cursor-not-allowed text-[11px] transition-colors shrink-0"
          >
            {t("share.receive.dest.change")}
          </button>
        </div>
        {suggestedName && (
          <p className="text-zinc-600 text-[11px] mt-2 truncate">
            <span className="text-cyan-500">↳</span> {suggestedName}
          </p>
        )}
      </div>

      {/* Receive button */}
      <button
        onClick={onReceive}
        disabled={!token || busy || kind === "unknown"}
        className="w-full py-3 rounded-xl bg-cyan-500 hover:bg-cyan-400 disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-[14px] font-semibold transition-colors"
      >
        {busy ? t("share.receive.btn.busy") : t("share.receive.btn")}
      </button>

      {/* Progress */}
      {busy && stepIdx >= 0 && (
        <div className="rounded-xl bg-white/[0.03] border border-white/[0.06] px-4 py-3 space-y-2">
          {STEPS.map((label, i) => {
            const done = i < stepIdx;
            const active = i === stepIdx && !stepErr;
            const err = i === stepIdx && stepErr;
            return (
              <div key={i} className={`flex items-center gap-2 text-[12px] ${
                done ? "text-emerald-400" : active ? "text-cyan-300" : err ? "text-red-400" : "text-zinc-700"
              }`}>
                <span className="w-4 text-center">
                  {done ? "✓" : active ? "⟳" : err ? "✗" : "·"}
                </span>
                <span>{label}</span>
              </div>
            );
          })}
        </div>
      )}

      {/* Result */}
      {result && !busy && (
        <div className="rounded-xl bg-emerald-500/[0.08] border border-emerald-500/20 px-4 py-4">
          <p className="text-emerald-300 text-[11px] tracking-[0.2em] uppercase mb-1">
            {t("share.receive.result.title")}
          </p>
          <p className="text-white text-[22px] font-semibold tabular-nums">{prettyBytes(result.bytes_written)}</p>
          <p className="text-zinc-300 text-[13px] font-medium mt-1">{result.filename || "archivo"}</p>
          <p className="text-zinc-600 text-[11px] font-mono mt-1 truncate">{result.output_path}</p>
        </div>
      )}

      {/* Error */}
      {error && (
        <div className="rounded-xl bg-red-500/[0.08] border border-red-500/20 px-4 py-3 text-red-400 text-[12px]">
          {error}
          <button onClick={onReceive} className="block mt-2 text-zinc-400 hover:text-white text-[11px] transition-colors">
            {t("share.receive.retry")}
          </button>
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
