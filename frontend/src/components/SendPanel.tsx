"use client";

/**
 * SendPanel — UI for sending a file to a friend (Sprint 5.5.5).
 *
 * Three ways to provide a file (in order of reliability):
 *   1. **Drag-and-drop** onto the panel itself (most reliable
 *      on macOS — no native dialog involved)
 *   2. **Type the path** in the text input (Cmd+V from Finder
 *      "Copy path" works perfectly)
 *   3. **Click "browse"** to open the native dialog (sometimes
 *      flaky on macOS Sequoia — see memory entry on picker bugs)
 *
 * After the file is set, click "SHARE" — backend auto-decides
 * the transport (Direct via UPnP + mDNS, fallback to LAN-only
 * if UPnP fails). No manual mode selection.
 */

import { useEffect, useState, useCallback, useRef } from "react";

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
    // would have issued (useful for testing layout in
    // isolation).
    console.debug(`[dev] invoke ${cmd}`, args);
    return {} as T;
  }
  const invoke = (window as any).__TAURI_INTERNALS__.invoke;
  return await invoke(cmd, args);
}

// Sprint 5.5.5: try the JS plugin API directly. If that fails
// (macOS Sequoia has flaky native dialog behavior), the user
// can fall back to drag-and-drop or path text input.
async function jsPickFile(): Promise<string | null> {
  if (!isTauri) return null;
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "All files", extensions: ["*"] }],
    });
    return typeof result === "string" ? result : null;
  } catch (e) {
    console.error("jsPickFile failed:", e);
    return null;
  }
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

export function SendPanel() {
  const [picked, setPicked] = useState<string | null>(null);
  const [resp, setResp] = useState<SendStartResp | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState<"token" | "code" | null>(null);
  const [tokenHidden, setTokenHidden] = useState(false);
  const [tokenHover, setTokenHover] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [pathInput, setPathInput] = useState("");
  const abortInFlight = useRef(false);

  // Auto-hide token after 5s.
  useEffect(() => {
    if (!resp) {
      setTokenHidden(false);
      return;
    }
    const timer = setTimeout(() => setTokenHidden(true), 5000);
    return () => clearTimeout(timer);
  }, [resp]);

  const acceptPath = useCallback((path: string | null | undefined) => {
    if (path && path.length > 0) {
      setPicked(path);
      setResp(null);
      setError(null);
      setPathInput("");
    }
  }, []);

  // --- Drag and drop ---
  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(true);
  }, []);
  const onDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
  }, []);
  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
    const files = Array.from(e.dataTransfer.files || []);
    if (files.length > 0) {
      // Browser File API doesn't give us the absolute path,
      // but Tauri intercepts drag-drop and emits the real
      // path via `tauri://drag-drop`. We listen below as the
      // primary path.
      const tauriPaths = (e as any).detail?.paths ?? null;
      if (tauriPaths && tauriPaths.length > 0) {
        acceptPath(tauriPaths[0]);
      } else {
        // Browser fallback: use the file's name to guess a
        // path. We can't get the absolute path from the
        // browser File API, but we can show the name.
        acceptPath((files[0] as any).path ?? null);
      }
    }
  }, [acceptPath]);

  // Tauri-native drag-drop listener (gives absolute paths).
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const eventMod = (window as any).__TAURI__?.event;
        if (!eventMod?.listen) return;
        unlisten = await eventMod.listen(
          "tauri://drag-drop",
          (e: any) => {
            const paths: string[] = e?.payload?.paths ?? [];
            if (paths.length > 0) {
              acceptPath(paths[0]);
            }
          }
        );
      } catch (e) {
        console.error("failed to attach tauri drag-drop:", e);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, [acceptPath]);

  // --- Pick via dialog (may fail on macOS Sequoia) ---
  const onBrowse = useCallback(async () => {
    const path = await jsPickFile();
    if (path) acceptPath(path);
  }, [acceptPath]);

  // --- Type the path manually ---
  const onSubmitPath = useCallback(() => {
    const trimmed = pathInput.trim();
    if (trimmed) acceptPath(trimmed);
  }, [pathInput, acceptPath]);

  const onStart = useCallback(async () => {
    if (!picked) return;
    setBusy(true);
    setError(null);
    setResp(null);
    setTokenHidden(false);
    try {
      const r = await tauriInvoke<SendStartResp>("p2p_send_start_cmd", {
        req: { file_path: picked, code: null },
      });
      setResp(r);
    } catch (e: any) {
      setError(`⚠ ${e?.message ?? e}`);
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
      setTokenHidden(true);
      setTimeout(() => setCopied(null), 1500);
    } catch {
      const el = document.createElement("textarea");
      el.value = text;
      document.body.appendChild(el);
      el.select();
      document.execCommand("copy");
      el.remove();
      setCopied(which);
      setTokenHidden(true);
      setTimeout(() => setCopied(null), 1500);
    }
  }, []);

  useEffect(() => {
    return () => {
      if (resp && !abortInFlight.current) {
        tauriInvoke<void>("p2p_send_abort_cmd").catch(() => {});
      }
    };
  }, [resp]);

  const filename = picked ? picked.split("/").pop() : null;
  const sizeLabel = resp ? prettySize(resp.file_size) : null;
  const crossNat = !!resp?.upnp_status;

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`panel p-3 flex flex-col gap-3 font-mono text-xs transition-colors ${
        dragOver ? "border-cyan-400 bg-cyan-500/5" : ""
      }`}
    >
      <div className="flex items-center gap-2 text-cyan-400 text-[10px] tracking-[0.3em] uppercase">
        <span>⤴</span>
        <span>share with a friend</span>
        <span className="text-zinc-600">·</span>
        <span className="text-zinc-500">end-to-end encrypted · no server</span>
      </div>

      {/* File picker — three methods */}
      <div className="border border-bg-border bg-bg-base p-3 space-y-2">
        {dragOver ? (
          <div className="text-center py-4 text-cyan-300 text-[14px] tracking-widest">
            ⤓ release to share
          </div>
        ) : (
          <>
            <div className="flex items-center gap-2">
              <button
                onClick={onBrowse}
                disabled={busy}
                className="px-3 py-1.5 border border-cyan-500 text-cyan-400 hover:bg-cyan-500/10 disabled:opacity-40 text-[10px] tracking-widest uppercase"
              >
                📄 browse…
              </button>
              <span className="text-zinc-600 text-[9px] tracking-wider">
                or drag &amp; drop here
              </span>
            </div>

            <div className="flex items-center gap-2">
              <input
                type="text"
                value={pathInput}
                onChange={(e) => setPathInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") onSubmitPath();
                }}
                placeholder="…or paste the absolute path"
                className="flex-1 bg-bg-surface text-zinc-300 border border-bg-border p-1.5 text-[10px] font-mono"
              />
              <button
                onClick={onSubmitPath}
                disabled={!pathInput.trim()}
                className="px-2 py-1.5 border border-amber-500 text-amber-400 hover:bg-amber-500/10 disabled:opacity-40 text-[10px] tracking-widest uppercase"
              >
                use path
              </button>
            </div>

            {picked && (
              <div className="text-[11px] text-zinc-300 truncate flex items-center gap-2 pt-1 border-t border-bg-border">
                <span className="text-cyan-400">▸</span>
                <span className="flex-1 truncate" title={picked}>
                  {filename ?? picked}
                </span>
                <button
                  onClick={() => setPicked(null)}
                  className="text-zinc-600 hover:text-err text-[10px]"
                  title="clear"
                >
                  ✕
                </button>
              </div>
            )}
          </>
        )}
      </div>

      {/* Action button */}
      {!resp ? (
        <button
          onClick={onStart}
          disabled={!picked || busy}
          className="px-4 py-3 border-2 border-magenta-500 text-magenta-400 hover:bg-magenta-500/10 disabled:border-zinc-700 disabled:text-zinc-600 text-[13px] tracking-[0.3em] uppercase font-bold"
        >
          {busy ? "⟳ preparing…" : "⤴ share"}
        </button>
      ) : (
        <>
          {/* Status badge */}
          <div
            className={`border-2 p-3 ${
              crossNat
                ? "border-matrix-500 bg-matrix-500/5"
                : "border-amber-500 bg-amber-500/5"
            }`}
          >
            <div
              className={`text-[11px] tracking-widest uppercase mb-1 ${
                crossNat ? "text-matrix-400" : "text-amber-400"
              }`}
            >
              {crossNat ? "🟢 ready to share" : "🟡 ready to share"}
            </div>
            <div
              className={`text-[12px] ${
                crossNat ? "text-matrix-300" : "text-amber-300"
              }`}
            >
              {crossNat ? (
                <>
                  any network — direct peer-to-peer
                  {resp.upnp_status && (
                    <span className="text-zinc-500">
                      {" "}
                      ({resp.upnp_status.external_ip}:
                      {resp.upnp_status.external_port})
                    </span>
                  )}
                </>
              ) : (
                <>same Wi-Fi only — local high-speed</>
              )}
            </div>
          </div>

          {/* Code */}
          <div className="border-2 border-matrix-500 bg-matrix-500/5 p-4 text-center">
            <div className="text-[9px] tracking-[0.4em] text-matrix-500 uppercase mb-2">
              share this code
            </div>
            <div className="text-3xl text-matrix-400 font-bold tracking-widest break-all leading-tight">
              {resp.code}
            </div>
            <div className="text-[9px] text-zinc-500 tracking-wider mt-2">
              {resp.filename} · {sizeLabel}
            </div>
            <button
              onClick={() => {
                onCopy(resp.code, "code");
                setTokenHidden(true);
              }}
              className="mt-3 px-3 py-1 border border-matrix-500 text-matrix-500 hover:bg-matrix-500/10 text-[10px] tracking-widest uppercase"
            >
              {copied === "code" ? "✓ copied" : "📋 copy code"}
            </button>
          </div>

          {/* Token */}
          <div className="border border-bg-border bg-bg-base p-3">
            <div className="flex items-center justify-between text-[10px] tracking-widest text-zinc-500 uppercase mb-2">
              <span>or share the full token</span>
              <button
                onClick={() => onCopy(resp.token, "token")}
                className="px-2 py-0.5 border border-cyan-500 text-cyan-400 hover:bg-cyan-500/10"
              >
                {copied === "token" ? "✓ copied" : "📋 copy"}
              </button>
            </div>
            <div
              onMouseEnter={() => setTokenHover(true)}
              onMouseLeave={() => setTokenHover(false)}
              className={`text-[10px] text-zinc-400 break-all bg-bg-surface p-2 max-h-24 overflow-y-auto transition-all duration-500 ${
                tokenHidden && !tokenHover
                  ? "blur-sm select-none opacity-50"
                  : ""
              }`}
              title={
                tokenHidden && !tokenHover
                  ? "hover to reveal · auto-hidden after 5s for privacy"
                  : ""
              }
            >
              {resp.token}
            </div>
            <div className="text-[9px] text-zinc-600 tracking-wider mt-2">
              {crossNat
                ? "🔒 includes the sender's public IP. Auto-blurred after 5s — hover to reveal. Copy clears the blur."
                : "🔒 local-network token. Same Wi-Fi required to receive."}
            </div>
          </div>

          <button
            onClick={onAbort}
            className="px-3 py-2 border border-err text-err hover:bg-err/10 text-[11px] tracking-[0.3em] uppercase"
          >
            ⨯ cancel
          </button>
        </>
      )}

      {error && (
        <div className="border-t border-err bg-err/10 px-3 py-1.5 text-[10px] text-err tracking-wider uppercase truncate">
          {error}
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