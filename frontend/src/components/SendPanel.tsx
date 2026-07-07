"use client";

/**
 * SendPanel — UI for the "send a file to a friend over a
 * Quick Cloudflare Tunnel" feature (Sprint 5.0 demo).
 *
 * Flow:
 *   1. User picks a file (or drags one in).
 *   2. Clicks "GENERATE CODE" — backend spawns cloudflared +
 *      axum server, returns a token.
 *   3. We show the 4-word code in big letters and the full
 *      token in a copyable textbox. The user shares both with
 *      the receiver.
 *   4. When the user clicks ABORT or navigates away, we call
 *      `p2p_send_abort_cmd` to kill the tunnel.
 */

import { useEffect, useState, useCallback, useRef } from "react";
import { defaultOutputFilename } from "./SaveTarget";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(
  cmd: string,
  args: Record<string, unknown> = {}
): Promise<T> {
  if (!isTauri) {
    console.log(`[stub] invoke ${cmd}`, args);
    return {} as T;
  }
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

interface SendStartResp {
  token: string;
  code: string;
  filename: string;
  file_size: number;
}

export function SendPanel() {
  const [picked, setPicked] = useState<string | null>(null);
  const [resp, setResp] = useState<SendStartResp | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState<"token" | "code" | null>(null);
  const abortInFlight = useRef(false);

  const onPick = useCallback(async () => {
    try {
      const path = await tauriInvoke<string | null>("pick_file_cmd");
      if (path) {
        setPicked(path);
        setResp(null);
        setError(null);
      }
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, []);

  const onStart = useCallback(async () => {
    if (!picked) return;
    setBusy(true);
    setError(null);
    setResp(null);
    try {
      const r = await tauriInvoke<SendStartResp>("p2p_send_start_cmd", {
        req: { file_path: picked, code: null },
      });
      setResp(r);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
    }
  }, [picked]);

  const onAbort = useCallback(async () => {
    if (abortInFlight.current) return;
    abortInFlight.current = true;
    try {
      await tauriInvoke<void>("p2p_send_abort_cmd");
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setResp(null);
      abortInFlight.current = false;
    }
  }, []);

  const onCopy = useCallback(async (text: string, which: "token" | "code") => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(which);
      setTimeout(() => setCopied(null), 1500);
    } catch {
      // Fallback for non-secure contexts
      const el = document.createElement("textarea");
      el.value = text;
      document.body.appendChild(el);
      el.select();
      document.execCommand("copy");
      el.remove();
      setCopied(which);
      setTimeout(() => setCopied(null), 1500);
    }
  }, []);

  // Auto-abort on unmount.
  useEffect(() => {
    return () => {
      if (resp && !abortInFlight.current) {
        tauriInvoke<void>("p2p_send_abort_cmd").catch(() => {});
      }
    };
  }, [resp]);

  const filename = picked ? picked.split("/").pop() : null;
  const sizeLabel = resp ? prettySize(resp.file_size) : null;

  return (
    <div className="panel p-3 flex flex-col gap-3 font-mono text-xs">
      <div className="flex items-center gap-2 text-cyan-400 text-[10px] tracking-[0.3em] uppercase">
        <span>⤴</span>
        <span>Send to a friend</span>
        <span className="text-zinc-600">·</span>
        <span className="text-zinc-500">Quick Cloudflare + E2E encrypted</span>
      </div>

      {/* File picker */}
      <div className="border border-bg-border bg-bg-base p-3">
        <div className="text-[10px] tracking-widest text-zinc-500 uppercase mb-2">
          file to send
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={onPick}
            disabled={busy}
            className="px-2 py-1 border border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:border-zinc-700 disabled:text-zinc-600"
          >
            PICK FILE
          </button>
          <div className="text-zinc-300 truncate flex-1" title={picked ?? ""}>
            {picked ? (
              <>
                <span className="text-cyan-400">▸</span> {filename}
              </>
            ) : (
              <span className="text-zinc-600">(no file picked)</span>
            )}
          </div>
        </div>
      </div>

      {/* Action button */}
      {!resp ? (
        <button
          onClick={onStart}
          disabled={!picked || busy}
          className="px-3 py-2 border-2 border-magenta-500 text-magenta-400 hover:bg-magenta-500/10 disabled:border-zinc-700 disabled:text-zinc-600 text-[11px] tracking-[0.3em] uppercase"
        >
          {busy ? "⟳ starting tunnel…" : "⤴ generate code"}
        </button>
      ) : (
        <>
          {/* Code (the human-friendly part) */}
          <div className="border-2 border-matrix-500 bg-matrix-500/5 p-4 text-center">
            <div className="text-[9px] tracking-[0.4em] text-matrix-500 uppercase mb-1">
              share this code
            </div>
            <div className="text-2xl text-matrix-400 font-bold tracking-widest break-all">
              {resp.code}
            </div>
            <div className="text-[9px] text-zinc-500 tracking-wider mt-2">
              {resp.filename} · {sizeLabel}
            </div>
            <button
              onClick={() => onCopy(resp.code, "code")}
              className="mt-2 px-2 py-0.5 border border-matrix-500 text-matrix-500 hover:bg-matrix-500/10 text-[9px] tracking-widest uppercase"
            >
              {copied === "code" ? "✓ copied" : "copy code"}
            </button>
          </div>

          {/* Token (the machine-friendly part) */}
          <div className="border border-bg-border bg-bg-base p-3">
            <div className="flex items-center justify-between text-[10px] tracking-widest text-zinc-500 uppercase mb-2">
              <span>or send the full token</span>
              <button
                onClick={() => onCopy(resp.token, "token")}
                className="px-2 py-0.5 border border-cyan-500 text-cyan-400 hover:bg-cyan-500/10"
              >
                {copied === "token" ? "✓ copied" : "copy token"}
              </button>
            </div>
            <div className="text-[10px] text-zinc-400 break-all bg-bg-surface p-2 max-h-24 overflow-y-auto">
              {resp.token}
            </div>
            <div className="text-[9px] text-zinc-600 tracking-wider mt-2">
              The token includes the cloudflared URL, the code, the file
              hash, and the SHA-256. The receiver just needs this and the
              4-word code (which is also inside the token, so the code
              alone is enough for humans).
            </div>
          </div>

          <button
            onClick={onAbort}
            className="px-3 py-2 border border-err text-err hover:bg-err/10 text-[11px] tracking-[0.3em] uppercase"
          >
            ⨯ abort transfer
          </button>
        </>
      )}

      {error && (
        <div className="border-t border-err bg-err/10 px-3 py-1.5 text-[10px] text-err tracking-wider uppercase truncate">
          ⚠ {error}
        </div>
      )}
    </div>
  );
}

function prettySize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}
