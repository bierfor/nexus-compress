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
  const [outputPath, setOutputPath] = useState("~/Downloads");
  const [resolvedDownloads, setResolvedDownloads] = useState("");
  const [resolvedHome, setResolvedHome] = useState("");
  const [suggestedName, setSuggestedName] = useState("");
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

  // Peek filename from v1 token
  useEffect(() => {
    const tk = token.trim();
    if (!tk) { setSuggestedName(""); return; }
    if (kind === "v1") {
      try {
        const b64 = tk.slice("nx:1:".length);
        const json = JSON.parse(atob(b64.replace(/-/g, "+").replace(/_/g, "/")));
        if (json.filename) setSuggestedName(json.filename);
      } catch {}
    }
  }, [token, kind]);

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

  const buildOutputPath = useCallback(async (): Promise<string> => {
    const { join, dirname, basename } = await import("@tauri-apps/api/path");
    let p = outputPath;
    // 1. expand leading ~/
    if (p.startsWith("~/") && resolvedHome) {
      p = await join(resolvedHome, p.slice(2));
    } else if (p === "~" && resolvedHome) {
      p = resolvedHome;
    }
    // 2. defaults / sentinel → use the destination dir + the
    //    real filename (no `.bin` placeholders ever).
    if (p === "~/Downloads" || (resolvedDownloads && p === resolvedDownloads)) {
      if (suggestedName) return await join(resolvedDownloads!, suggestedName);
      return resolvedDownloads!;
    }
    // 3. user typed a path with a placeholder basename — replace
    //    just the basename with the real filename. We match a
    //    broad set: `archivo_recibido*`, `*.bin`, `*.bin.*`, `*.*`.
    //    We strip shell-glob trailing `.*` too.
    const base = await basename(p);
    const cleanedBase = base.replace(/\.\*+$/, ""); // strip trailing ".*"
    const isPlaceholder =
      base === "archivo_recibido" ||
      base === "archivo_recibido.bin" ||
      base === "received.bin" ||
      base === "received" ||
      cleanedBase === "" ||
      cleanedBase === "recibido" ||
      base.endsWith(".bin") ||
      base.endsWith(".bin.*") ||
      /^\*\.[a-z0-9]+$/i.test(cleanedBase) ||
      /^\.[a-z0-9]+$/i.test(cleanedBase);
    if (isPlaceholder && suggestedName) {
      return await join(await dirname(p), suggestedName);
    }
    return p;
  }, [outputPath, resolvedHome, resolvedDownloads, suggestedName]);

  const onChangeDest = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const { join } = await import("@tauri-apps/api/path");
      // Sprint 5.6.23: never open the dialog with a generic
      // "archivo_recibido.bin" placeholder. If we know the
      // filename, pre-fill it. Otherwise default to the dir
      // only and let the OS file dialog pick a name.
      let defPath: string;
      if (resolvedDownloads && suggestedName) {
        defPath = await join(resolvedDownloads, suggestedName);
      } else if (resolvedDownloads) {
        defPath = resolvedDownloads;
      } else {
        defPath = "recibido";
      }
      const r = await save({
        defaultPath: defPath,
        filters: [{ name: "All files", extensions: ["*"] }],
      });
      if (r) setOutputPath(r);
    } catch {}
  }, [resolvedDownloads, suggestedName]);

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
          <span className="flex-1 text-zinc-300 text-[12px] font-mono truncate">{outputPath}</span>
          <button onClick={onChangeDest} className="text-zinc-500 hover:text-zinc-300 text-[11px] transition-colors shrink-0">
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
