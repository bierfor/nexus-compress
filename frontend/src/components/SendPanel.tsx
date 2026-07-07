"use client";

/**
 * SendPanel — UI for the "send a file to a friend" feature.
 *
 * Sprint 5.5.2: reads the current TransportMode from the app
 * config and shows a small badge so the user knows whether the
 * generated token will be a v1 (Cloudflare) or v2 (Direct LAN)
 * token. The actual dispatch happens server-side in
 * `start_sender`, which reads the same config.
 *
 * Flow:
 *   1. User picks a file.
 *   2. Clicks "GENERATE CODE" — backend dispatches on the
 *      current transport mode (Quick / Named / Direct) and
 *      returns a v1 or v2 token accordingly.
 *   3. We show the 4-word code in big letters and the full
 *      token (v1 or v2) in a copyable textbox.
 *   4. When the user clicks ABORT or navigates away, we call
 *      `p2p_send_abort_cmd` to kill the tunnel / unregister
 *      the mDNS service.
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
  upnp_status: {
    external_ip: string;
    external_port: number;
  } | null;
}

interface TunnelConfigInfo {
  mode: "quick" | "named" | "direct";
  hostname: string | null;
  has_token: boolean;
}

export function SendPanel() {
  const [picked, setPicked] = useState<string | null>(null);
  const [resp, setResp] = useState<SendStartResp | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState<"token" | "code" | null>(null);
  const [cfg, setCfg] = useState<TunnelConfigInfo | null>(null);
  // Sprint 5.5.4: track whether the token has been "consumed"
  // (user clicked copy OR 5s elapsed since the sender started).
  // After that, the token text on screen is replaced with
  // a "copied to clipboard" hint — the plaintext token still
  // lives in `resp.token` but isn't rendered anymore. This
  // mitigates the privacy risk of the public IP being
  // shoulder-surfed from a screen at a coffee shop.
  const [tokenHidden, setTokenHidden] = useState(false);
  const abortInFlight = useRef(false);

  // Read the current transport mode on mount so the user sees
  // which kind of token they'll generate.
  useEffect(() => {
    (async () => {
      try {
        const c = await tauriInvoke<TunnelConfigInfo>(
          "p2p_get_tunnel_config_cmd"
        );
        setCfg(c);
      } catch {
        // Fall back silently — the backend always returns the
        // current config (defaults to Quick if file missing).
      }
    })();
  }, []);

  const tokenKind = resp
    ? resp.token.startsWith("nx:3:")
      ? "v3"
      : resp.token.startsWith("nx:2:")
      ? "v2"
      : "v1"
    : null;

  // Privacy mitigation (Sprint 5.5.4): after the user copies
  // the token OR after 5s of display, hide the token text from
  // screen. The token still exists in `resp.token` (for any
  // re-copy via clipboard) but is not rendered anymore.
  useEffect(() => {
    if (!resp) {
      setTokenHidden(false);
      return;
    }
    const timer = setTimeout(() => setTokenHidden(true), 5000);
    return () => clearTimeout(timer);
  }, [resp]);

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
        <span className="text-zinc-500">
          {cfg?.mode === "direct"
            ? "Direct LAN (mDNS) + E2E encrypted"
            : cfg?.mode === "named"
            ? "Named Tunnel + E2E encrypted"
            : "Quick Cloudflare + E2E encrypted"}
        </span>
        {cfg?.mode === "direct" && (
          <span className="ml-auto px-1.5 py-0.5 border border-matrix-500 text-matrix-400 text-[9px] tracking-widest">
            ▎ direct LAN
          </span>
        )}
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
              onClick={() => {
                onCopy(resp.code, "code");
                setTokenHidden(true); // privacy: hide token once user starts sharing
              }}
              className="mt-2 px-2 py-0.5 border border-matrix-500 text-matrix-500 hover:bg-matrix-500/10 text-[9px] tracking-widest uppercase"
            >
              {copied === "code" ? "✓ copied" : "copy code"}
            </button>
          </div>

          {/* UPnP status (Sprint 5.5.4 Phase 3) — visible feedback
              before the user even looks at the token. Tells them
              whether the transfer will work cross-NAT or not. */}
          <div
            className={`border p-2 text-[10px] tracking-wider ${
              resp.upnp_status
                ? "border-matrix-500 bg-matrix-500/5 text-matrix-400"
                : "border-amber-500 bg-amber-500/5 text-amber-400"
            }`}
          >
            {resp.upnp_status ? (
              <>
                ✓ UPnP hole open — external {resp.upnp_status.external_ip}:
                {resp.upnp_status.external_port}
                <br />
                <span className="text-[9px] text-zinc-500">
                  Cross-NAT ready. The receiver can be on any network.
                </span>
              </>
            ) : (
              <>
                ⚠ UPnP unavailable — falling back to LAN-only
                <br />
                <span className="text-[9px] text-zinc-500">
                  Receiver must be on the same Wi-Fi as you.
                </span>
              </>
            )}
          </div>

          {/* Token (the machine-friendly part) */}
          <div className="border border-bg-border bg-bg-base p-3">
            <div className="flex items-center justify-between text-[10px] tracking-widest text-zinc-500 uppercase mb-2">
              <span>or send the full token</span>
              <button
                onClick={() => {
                  onCopy(resp.token, "token");
                  setTokenHidden(true); // privacy: hide after copy
                }}
                className="px-2 py-0.5 border border-cyan-500 text-cyan-400 hover:bg-cyan-500/10"
              >
                {copied === "token" ? "✓ copied" : "copy token"}
              </button>
            </div>
            {tokenHidden ? (
              <div className="text-[10px] text-zinc-500 italic bg-bg-surface p-2 border border-zinc-700">
                ▎ token hidden for privacy (public IP exposure).
                Click <em>copy token</em> again to paste from clipboard
                history, or use the 4-word code above (works the same).
              </div>
            ) : (
              <div className="text-[10px] text-zinc-400 break-all bg-bg-surface p-2 max-h-24 overflow-y-auto">
                {resp.token}
              </div>
            )}
            <div className="text-[9px] text-zinc-600 tracking-wider mt-2">
              {tokenKind === "v3"
                ? "v3 cross-NAT token: includes your public IP + UPnP port. The receiver tries this first; falls back to LAN mDNS if their router blocks NAT loopback. Auto-hidden after 5s."
                : tokenKind === "v2"
                ? "Direct Mode v2 token: service hash + code. The receiver's mDNS browse finds your machine on the LAN — no URL, no Cloudflare. Requires same Wi-Fi."
                : "v1 token includes the cloudflared URL, the code, the file hash, and SHA-256. The receiver just needs this and the 4-word code."}
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
