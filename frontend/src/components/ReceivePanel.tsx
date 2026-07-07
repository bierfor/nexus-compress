"use client";

/**
 * ReceivePanel — UI for receiving a file from a friend.
 *
 * Two transport modes (auto-detected from the token prefix):
 *   - v1 token (nx:1:...) → Quick/Named Cloudflare Tunnel
 *   - v2 token (nx:2:...) → Direct LAN Mode (mDNS discovery,
 *     no Cloudflare). Sprint 5.5.2 Phase 1.
 *
 * Flow:
 *   1. User pastes the token.
 *   2. Picks where to save the decrypted file.
 *   3. Clicks "START RECEIVING" — backend does:
 *      - v1: HTTP/SPAKE2 handshake + AES-GCM stream
 *      - v2: mDNS browse + HTTP/SPAKE2 + AES-GCM stream
 *      Then decrypts, verifies SHA-256, writes to disk.
 *   4. We show progress and the final output path.
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

interface ReceiveResp {
  bytes_written: number;
  output_path: string;
}

/** Discriminated token version detected from the wire prefix. */
type TokenKind = "v1" | "v2" | "unknown";

function detectTokenKind(token: string): TokenKind {
  const t = token.trim();
  if (t.startsWith("nx:1:")) return "v1";
  if (t.startsWith("nx:2:")) return "v2";
  return "unknown";
}

export function ReceivePanel() {
  const [token, setToken] = useState("");
  const [outputPath, setOutputPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ReceiveResp | null>(null);
  const [progress, setProgress] = useState<{ bytes: number; total: number } | null>(null);
  const cancelRef = useRef(false);

  const kind = detectTokenKind(token);

  // Parse the v1 token's plaintext-size to show "downloading X MB"
  // UI. v2 tokens don't carry the size — it's fetched from the
  // sender's /meta endpoint AFTER mDNS resolves, so we can't
  // pre-fill it here.
  const tokenSize = useCallback((): number | null => {
    if (!token.startsWith("nx:1:")) return null;
    try {
      const b64 = token.slice(5);
      const json = JSON.parse(atob(b64.replace(/-/g, "+").replace(/_/g, "/")));
      return typeof json.size === "number" ? json.size : null;
    } catch {
      return null;
    }
  }, [token]);

  const expectedSize = tokenSize();

  const onPickOutput = useCallback(async () => {
    try {
      const suggested = expectedSize != null
        ? defaultFilenameFromToken(token)
        : "received.bin";
      const chosen = await tauriInvoke<string | null>(
        "pick_save_location_cmd",
        { req: { default_filename: suggested } }
      );
      if (chosen) {
        setOutputPath(chosen);
        setError(null);
      }
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, [expectedSize, token]);

  const onReceive = useCallback(async () => {
    if (!token || !outputPath) return;
    setBusy(true);
    setError(null);
    setResult(null);
    setProgress({ bytes: 0, total: expectedSize ?? 0 });
    cancelRef.current = false;
    try {
      // Dispatch on token version. v1 = Cloudflare (existing).
      // v2 = Direct LAN, with 5s mDNS browse timeout (the
      // user sees the "⟳ searching LAN…" state for up to 5s
      // before the backend returns a timeout error).
      let r: ReceiveResp;
      if (kind === "v2") {
        r = await tauriInvoke<ReceiveResp>("p2p_receive_direct_cmd", {
          req: {
            token: token.trim(),
            output_path: outputPath,
            timeout_secs: 5,
          },
        });
      } else {
        r = await tauriInvoke<ReceiveResp>("p2p_receive_cmd", {
          req: { token: token.trim(), output_path: outputPath },
        });
      }
      if (cancelRef.current) {
        setError("cancelled");
        return;
      }
      setResult(r);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }, [token, outputPath, expectedSize, kind]);

  const outputLabel = outputPath ? outputPath.split("/").pop() : null;

  return (
    <div className="panel p-3 flex flex-col gap-3 font-mono text-xs">
      <div className="flex items-center gap-2 text-magenta-400 text-[10px] tracking-[0.3em] uppercase">
        <span>⤵</span>
        <span>Receive from a friend</span>
        <span className="text-zinc-600">·</span>
        <span className="text-zinc-500">SPAKE2 + AES-GCM</span>
        {kind === "v2" && (
          <span className="ml-auto px-1.5 py-0.5 border border-matrix-500 text-matrix-400 text-[9px] tracking-widest">
            ▎ direct LAN (mDNS)
          </span>
        )}
      </div>

      {/* Token input */}
      <div className="border border-bg-border bg-bg-base p-3">
        <div className="text-[10px] tracking-widest text-zinc-500 uppercase mb-2">
          paste the token from the sender
        </div>
        <textarea
          value={token}
          onChange={(e) => setToken(e.target.value)}
          rows={3}
          placeholder={
            kind === "v2"
              ? "nx:2:direct:<hash>:<code>    ← Direct Mode (LAN)"
              : kind === "unknown" && token.length > 0
              ? "⚠ token must start with nx:1: (cloudflared) or nx:2: (direct)"
              : "nx:1:... or nx:2:..."
          }
          className={`w-full bg-bg-surface text-zinc-300 border p-2 text-[10px] font-mono break-all ${
            kind === "unknown" && token.length > 0
              ? "border-err"
              : "border-bg-border"
          }`}
        />
        {kind === "v1" && expectedSize != null && (
          <div className="text-[10px] text-matrix-500 mt-2">
            ✓ parsed · {prettySize(expectedSize)} incoming
          </div>
        )}
        {kind === "v2" && (
          <div className="text-[10px] text-zinc-500 mt-2">
            ▎ Direct Mode · sender discovered via mDNS on the LAN
            (5s timeout)
          </div>
        )}
        {kind === "unknown" && token.length > 0 && (
          <div className="text-[10px] text-err mt-2">
            ⚠ unknown token format — must be nx:1: or nx:2:
          </div>
        )}
      </div>

      {/* Output path */}
      <div className="border border-bg-border bg-bg-base p-3">
        <div className="text-[10px] tracking-widest text-zinc-500 uppercase mb-2">
          save as
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={onPickOutput}
            disabled={busy}
            className="px-2 py-1 border border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:border-zinc-700 disabled:text-zinc-600"
          >
            CHOOSE…
          </button>
          <div className="text-zinc-300 truncate flex-1" title={outputPath ?? ""}>
            {outputPath ? (
              <>
                <span className="text-cyan-400">▸</span> {outputLabel}
              </>
            ) : (
              <span className="text-zinc-600">(no output chosen)</span>
            )}
          </div>
        </div>
      </div>

      {/* Action button */}
      <button
        onClick={onReceive}
        disabled={!token || !outputPath || busy || kind === "unknown"}
        className="px-3 py-2 border-2 border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:border-zinc-700 disabled:text-zinc-600 text-[11px] tracking-[0.3em] uppercase"
      >
        {busy
          ? kind === "v2"
            ? "⟳ searching LAN…"
            : "⟳ downloading…"
          : kind === "v2"
          ? "⤵ start receiving (LAN)"
          : "⤵ start receiving"}
      </button>

      {/* Retry button — only shown on error after a v2 mDNS
          timeout, so the user can recover without re-typing the
          token. */}
      {error && !busy && kind === "v2" && (
        <button
          onClick={onReceive}
          className="px-3 py-1.5 border border-matrix-500 text-matrix-400 hover:bg-matrix-500/10 text-[10px] tracking-[0.3em] uppercase"
        >
          ⟲ retry mDNS browse
        </button>
      )}

      {/* Progress bar (linear, since we have byte counts) */}
      {progress && (
        <div className="border border-bg-border bg-bg-base p-2">
          <div className="text-[10px] text-zinc-500 uppercase tracking-widest mb-1">
            downloading
          </div>
          <div className="h-1 bg-bg-surface relative overflow-hidden">
            <div
              className="absolute inset-y-0 left-0 bg-cyan-500"
              style={{
                width:
                  progress.total > 0
                    ? `${Math.min(100, (progress.bytes / progress.total) * 100).toFixed(1)}%`
                    : "0%",
              }}
            />
          </div>
          <div className="text-[10px] text-zinc-400 mt-1 font-mono">
            {prettySize(progress.bytes)} / {prettySize(progress.total)}
          </div>
        </div>
      )}

      {/* Result */}
      {result && (
        <div className="border-2 border-matrix-500 bg-matrix-500/5 p-4 text-center">
          <div className="text-[9px] tracking-[0.4em] text-matrix-500 uppercase mb-1">
            received
          </div>
          <div className="text-xl text-matrix-400 font-bold">
            {prettySize(result.bytes_written)}
          </div>
          <div className="text-[10px] text-zinc-300 mt-2 break-all" title={result.output_path}>
            {result.output_path}
          </div>
          <div className="text-[9px] text-zinc-500 tracking-wider mt-2">
            SHA-256 verified · ready to open
          </div>
        </div>
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

function defaultFilenameFromToken(token: string): string {
  // Only v1 tokens embed a filename hint. v2 tokens (Direct
  // Mode) don't carry that field — we fall back to a generic
  // name and the user can rename before saving.
  if (!token.startsWith("nx:1:")) return "received.bin";
  try {
    const b64 = token.slice(5);
    const json = JSON.parse(atob(b64.replace(/-/g, "+").replace(/_/g, "/")));
    return json.suggested_name || "received.bin";
  } catch {
    return "received.bin";
  }
}
