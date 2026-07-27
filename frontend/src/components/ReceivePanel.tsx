"use client";

/**
 * ReceivePanel — UI for receiving a file from a friend.
 *
 * Sprint 5.5.5 "Zero Friction" redesign:
 *
 *   - Single smart input that auto-detects v1 (Cloudflare) /
 *     v2 (LAN mDNS) / v3 (cross-NAT UPnP) on paste.
 *   - Backend dispatches to the right transport.
 *   - Progress is shown as a 5-step human-language list:
 *     1. 🔍 detecting format
 *     2. 📡 finding the other computer on the network
 *     3. 🔐 establishing secure direct connection
 *     4. 📦 transferring
 *     5. ✓ verifying integrity
 *   - Each step gets a ✓/✗/… state in real-time.
 *   - On error: friendly text + retry button.
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
    // Dev-mode fallback: the browser preview doesn't have a
    // Tauri runtime, so the call is a no-op. We log to
    // devtools so engineers can see which commands the UI
    // would have issued.
    console.debug(`[dev] invoke ${cmd}`, args);
    return {} as T;
  }
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

// Sprint 5.5.5: bypass the Rust pick_save_location_cmd and
// use the JS API directly.
async function jsSaveFile(
  suggestedName: string,
  filters: { name: string; extensions: string[] }[] = [
    { name: "All files", extensions: ["*"] },
  ]
): Promise<string | null> {
  if (!isTauri) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  const result = await save({
    defaultPath: suggestedName,
    filters,
  });
  return result ?? null;
}

interface ReceiveResp {
  bytes_written: number;
  output_path: string;
}

type TokenKind = "v1" | "v2" | "v3" | "unknown";

function detectTokenKind(token: string): TokenKind {
  const t = token.trim();
  if (t.startsWith("nx:1:")) return "v1";
  if (t.startsWith("nx:2:")) return "v2";
  if (t.startsWith("nx:3:")) return "v3";
  return "unknown";
}

type StepStatus = "pending" | "active" | "done" | "error";

interface Step {
  icon: string;
  label: string;
  status: StepStatus;
}

// Human-language steps for the v2/v3 path. v1 uses a simplified version.
const STEPS_DIRECT: { icon: string; label: string }[] = [
  { icon: "🔍", label: "detecting format" },
  { icon: "📡", label: "finding the other computer on the network" },
  { icon: "🔐", label: "establishing secure direct connection" },
  { icon: "📦", label: "transferring" },
  { icon: "✓", label: "verifying integrity" },
];

const STEPS_V1: { icon: string; label: string }[] = [
  { icon: "🔍", label: "detecting format" },
  { icon: "☁️", label: "connecting to relay" },
  { icon: "🔐", label: "establishing secure connection" },
  { icon: "📦", label: "transferring" },
  { icon: "✓", label: "verifying integrity" },
];

function getSteps(kind: TokenKind): { icon: string; label: string }[] {
  if (kind === "v1") return STEPS_V1;
  return STEPS_DIRECT;
}

export function ReceivePanel() {
  const [token, setToken] = useState("");
  const [outputPath, setOutputPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ReceiveResp | null>(null);
  const [progress, setProgress] = useState<{ bytes: number; total: number } | null>(null);
  const [steps, setSteps] = useState<Step[]>([]);
  const cancelRef = useRef(false);

  const kind = detectTokenKind(token);

  // Initialize steps when transfer starts.
  const resetSteps = useCallback((k: TokenKind) => {
    const defs = getSteps(k);
    setSteps(defs.map((s) => ({ ...s, status: "pending" as StepStatus })));
  }, []);

  const updateStep = useCallback((index: number, status: StepStatus) => {
    setSteps((prev) => {
      const next = [...prev];
      if (next[index]) {
        next[index] = { ...next[index], status };
      }
      return next;
    });
  }, []);

  const expectedSize = useCallback((): number | null => {
    if (!token.startsWith("nx:1:")) return null;
    try {
      const b64 = token.slice(5);
      const json = JSON.parse(atob(b64.replace(/-/g, "+").replace(/_/g, "/")));
      return typeof json.size === "number" ? json.size : null;
    } catch {
      return null;
    }
  }, [token]);

  const total = expectedSize() ?? 0;

  const onPickOutput = useCallback(async () => {
    try {
      // v1: suggested_name is encoded directly in the token.
      // v2/v3: ask the sender via /meta what the file is called.
      // unknown: fall back to "received.bin".
      let suggested: string;
      if (kind === "v1" && total > 0) {
        suggested = defaultFilenameFromToken(token);
      } else if (kind === "v2" || kind === "v3") {
        suggested = "received.bin";
        try {
          const r = await tauriInvoke<{ filename: string | null }>(
            "p2p_peek_filename_cmd",
            { req: { token: token.trim(), timeout_secs: 3 } }
          );
          if (r?.filename) suggested = r.filename;
        } catch {
          // sender unreachable — keep "received.bin"
        }
      } else {
        suggested = "received.bin";
      }
      // Sprint 5.6.29 hotfix #15: skip the macOS save dialog.
      // The save dialog with `extensions: ["*"]` appends a literal
      // ".*" to the filename, which corrupts the receive. Instead,
      // we auto-save to ~/Downloads/<filename> with the original
      // name from the sender. The user can move the file from
      // Downloads to anywhere else if they want.
      //
      // For users who want a custom location, the "Custom..." button
      // opens the save dialog explicitly with a more specific filter
      // (see onPickOutputCustom below).
      const { homeDir, join } = await import("@tauri-apps/api/path");
      const home = await homeDir();
      const dest = await join(home, "Downloads", suggested);
      setOutputPath(dest);
      setError(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, [token, total, kind]);

  // Optional: open the save dialog explicitly with a filter
  // tailored to the sender's filename. Still subject to macOS
  // save-dialog quirks, so use with care.
  const onPickOutputCustom = useCallback(async () => {
    try {
      let suggested: string;
      if (kind === "v1" && total > 0) {
        suggested = defaultFilenameFromToken(token);
      } else {
        suggested = "received.bin";
      }
      // Derive the file's extension so the filter matches it.
      const dot = suggested.lastIndexOf(".");
      const ext = dot >= 0 ? suggested.slice(dot + 1) : "";
      const filters = ext
        ? [{ name: "File", extensions: [ext] }, { name: "All files", extensions: ["*"] }]
        : [{ name: "All files", extensions: ["*"] }];
      const chosen = await jsSaveFile(suggested, filters);
      if (chosen) {
        // Strip any literal ".*" suffix that the dialog may have
        // appended (macOS save-dialog quirk with `*` filters).
        const cleaned = chosen.replace(/\.\*$/, "");
        setOutputPath(cleaned);
        setError(null);
      }
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, [token, kind]);

  const onReceive = useCallback(async () => {
    if (!token || !outputPath) return;
    setBusy(true);
    setError(null);
    setResult(null);
    setProgress({ bytes: 0, total });
    cancelRef.current = false;
    resetSteps(kind);

    // Animate through the steps as work happens. Each step has
    // its own real-world timing (mDNS resolve, SPAKE2, etc.)
    // but here we just mark them active->done sequentially as
    // the user perceives progress.
    updateStep(0, "done");
    updateStep(1, "active");

    try {
      // The backend does steps 1-4 internally. We just mark
      // them done when the response arrives. The transfer step
      // is the only one we can show progress on.
      let r: ReceiveResp;
      if (kind === "v2" || kind === "v3") {
        r = await tauriInvoke<ReceiveResp>("p2p_receive_direct_cmd", {
          req: {
            token: token.trim(),
            output_path: outputPath,
            timeout_secs: 5,
          },
        });
      } else {
        // v1 — Cloudflare
        r = await tauriInvoke<ReceiveResp>("p2p_receive_cmd", {
          req: { token: token.trim(), output_path: outputPath },
        });
      }
      if (cancelRef.current) {
        setError("cancelled");
        return;
      }
      // Backend is done — mark everything done.
      updateStep(1, "done");
      updateStep(2, "done");
      updateStep(3, "done");
      updateStep(4, "done");
      setResult(r);
    } catch (e: any) {
      const msg = String(e?.message ?? e);
      setError(msg);
      // Mark the current active step as errored.
      setSteps((prev) => {
        const idx = prev.findIndex((s) => s.status === "active");
        if (idx >= 0) {
          const next = [...prev];
          next[idx] = { ...next[idx], status: "error" };
          return next;
        }
        return prev;
      });
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }, [token, outputPath, total, kind, resetSteps, updateStep]);

  const outputLabel = outputPath ? outputPath.split("/").pop() : null;

  return (
    <div className="panel p-3 flex flex-col gap-3 font-mono text-xs">
      <div className="flex items-center gap-2 text-magenta-400 text-[10px] tracking-[0.3em] uppercase">
        <span>⤵</span>
        <span>receive from a friend</span>
        <span className="text-zinc-600">·</span>
        <span className="text-zinc-500">end-to-end encrypted</span>
      </div>

      {/* Token input */}
      <div className="border border-bg-border bg-bg-base p-3">
        <div className="text-[10px] tracking-widest text-zinc-500 uppercase mb-2">
          paste the code or token
        </div>
        <textarea
          value={token}
          onChange={(e) => setToken(e.target.value)}
          rows={2}
          placeholder="nx:1:... or nx:2:... or nx:3:..."
          className={`w-full bg-bg-surface text-zinc-300 border p-2 text-[11px] font-mono break-all ${
            kind === "unknown" && token.length > 0
              ? "border-err"
              : "border-bg-border"
          }`}
        />
        {kind === "unknown" && token.length > 0 && (
          <div className="text-[10px] text-err mt-2">
            ⚠ unknown format — token must start with nx:1:, nx:2:, or nx:3:
          </div>
        )}
        {kind === "v2" && (
          <div className="text-[10px] text-zinc-500 mt-2">
            📡 LAN direct — same Wi-Fi required
          </div>
        )}
        {kind === "v3" && (
          <div className="text-[10px] text-zinc-500 mt-2">
            🌐 cross-NAT direct — works from any network
          </div>
        )}
        {kind === "v1" && total > 0 && (
          <div className="text-[10px] text-matrix-500 mt-2">
            ☁️ Cloudflare relay · {prettySize(total)} incoming
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
            className="px-3 py-1.5 border border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:opacity-40 text-[10px] tracking-widest uppercase"
          >
            💾 choose…
          </button>
          <div className="text-zinc-300 truncate flex-1 text-[11px]" title={outputPath ?? ""}>
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
        className="px-4 py-3 border-2 border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:border-zinc-700 disabled:text-zinc-600 text-[13px] tracking-[0.3em] uppercase font-bold"
      >
        {busy ? "⟳ working…" : "⤵ receive"}
      </button>

      {/* Progress steps (Sprint 5.5.5: human-language list) */}
      {(busy || steps.length > 0) && (
        <div className="border border-bg-border bg-bg-base p-3">
          <div className="text-[10px] text-zinc-500 uppercase tracking-widest mb-2">
            progress
          </div>
          <div className="space-y-1">
            {steps.map((s, i) => (
              <div
                key={i}
                className={`text-[11px] flex items-center gap-2 ${
                  s.status === "done"
                    ? "text-matrix-400"
                    : s.status === "active"
                    ? "text-cyan-300"
                    : s.status === "error"
                    ? "text-err"
                    : "text-zinc-600"
                }`}
              >
                <span className="w-4 text-center">
                  {s.status === "done" ? "✓" : s.status === "active" ? "⟳" : s.status === "error" ? "✗" : "·"}
                </span>
                <span className="opacity-70">{s.icon}</span>
                <span>{s.label}</span>
              </div>
            ))}
          </div>
          {progress && progress.total > 0 && (
            <div className="mt-3">
              <div className="h-1 bg-bg-surface relative overflow-hidden">
                <div
                  className="absolute inset-y-0 left-0 bg-cyan-500"
                  style={{
                    width: `${Math.min(100, (progress.bytes / progress.total) * 100).toFixed(1)}%`,
                  }}
                />
              </div>
              <div className="text-[10px] text-zinc-400 mt-1 font-mono">
                {prettySize(progress.bytes)} / {prettySize(progress.total)}
              </div>
            </div>
          )}
        </div>
      )}

      {/* Result */}
      {result && (
        <div className="border-2 border-matrix-500 bg-matrix-500/5 p-4 text-center">
          <div className="text-[10px] tracking-[0.4em] text-matrix-500 uppercase mb-1">
            ✓ received
          </div>
          <div className="text-2xl text-matrix-400 font-bold">
            {prettySize(result.bytes_written)}
          </div>
          <div className="text-[10px] text-zinc-300 mt-2 break-all" title={result.output_path}>
            ▸ {result.output_path}
          </div>
          <div className="text-[9px] text-zinc-500 tracking-wider mt-2">
            SHA-256 verified · integrity guaranteed
          </div>
        </div>
      )}

      {/* Retry button */}
      {error && !busy && (
        <div className="border border-err bg-err/5 p-3">
          <div className="text-[10px] text-err mb-2 truncate">
            ⚠ {friendlyError(error)}
          </div>
          <button
            onClick={onReceive}
            className="px-3 py-1.5 border border-matrix-500 text-matrix-400 hover:bg-matrix-500/10 text-[10px] tracking-widest uppercase"
          >
            ⟲ try again
          </button>
        </div>
      )}
    </div>
  );
}

// Convert technical error messages to user-friendly text.
function friendlyError(msg: string): string {
  if (msg.includes("mDNS") || msg.includes("no service matching")) {
    return "couldn't find the sender on the network — make sure both devices are on the same Wi-Fi";
  }
  if (msg.includes("timed out") || msg.includes("timeout")) {
    return "connection timed out — check that the sender is still active and on the same network";
  }
  if (msg.includes("authentication") || msg.includes("wrong code")) {
    return "wrong code — copy the full token, not just the 4-word code";
  }
  if (msg.includes("SHA-256") || msg.includes("corrupted")) {
    return "file corrupted during transfer — try sending again";
  }
  return msg;
}

function prettySize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MiB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}

function defaultFilenameFromToken(token: string): string {
  if (!token.startsWith("nx:1:")) return "received.bin";
  try {
    const b64 = token.slice(5);
    const json = JSON.parse(atob(b64.replace(/-/g, "+").replace(/_/g, "/")));
    return json.suggested_name || "received.bin";
  } catch {
    return "received.bin";
  }
}