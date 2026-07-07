"use client";

/**
 * ShareView — dedicated screen for sharing (Sprint 5.6).
 *
 * Tabs: Enviar | Recibir. Same view, different sub-action.
 * User picks a tab, fills in the form, clicks the big button.
 */

import { useEffect, useState, useCallback, useRef } from "react";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function tauriInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
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

export function ShareView({
  onComplete,
}: {
  onComplete: (op: {
    kind: "share";
    filename: string;
    originalBytes: number;
    durationMs: number;
  }) => void;
}) {
  const [tab, setTab] = useState<"send" | "receive">("send");

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="max-w-3xl mx-auto px-8 pt-12 pb-20">
        {/* Header */}
        <div className="mb-10">
          <div className="text-zinc-500 text-[12px] tracking-wide mb-2">
            ← Volver
          </div>
          <h1 className="text-white text-[36px] font-semibold tracking-tight mb-3">
            Compartir
          </h1>
          <p className="text-zinc-400 text-[14px] leading-relaxed max-w-2xl">
            Envía o recibe archivos pesados sin servidor intermedio. Cifrado
            punto a punto, directo entre dispositivos.
          </p>
        </div>

        {/* Tabs */}
        <div className="flex items-center gap-1 p-1 rounded-xl bg-white/[0.04] border border-white/[0.06] mb-8 w-fit">
          <TabButton
            active={tab === "send"}
            onClick={() => setTab("send")}
            icon="📤"
            label="Enviar"
          />
          <TabButton
            active={tab === "receive"}
            onClick={() => setTab("receive")}
            icon="📥"
            label="Recibir"
          />
        </div>

        {tab === "send" ? <SendTab onComplete={onComplete} /> : <ReceiveTab />}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: string;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-2 px-4 py-2 rounded-lg text-[13px] font-medium transition-all ${
        active
          ? "bg-white/[0.08] text-white"
          : "text-zinc-500 hover:text-zinc-300"
      }`}
    >
      <span>{icon}</span>
      <span>{label}</span>
    </button>
  );
}

// ============================================================
//  Send Tab
// ============================================================

function SendTab({
  onComplete,
}: {
  onComplete: (op: {
    kind: "share";
    filename: string;
    originalBytes: number;
    durationMs: number;
  }) => void;
}) {
  const [filePath, setFilePath] = useState<string | null>(null);
  const [resp, setResp] = useState<SendStartResp | null>(null);
  const [busy, setBusy] = useState(false);
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

  // Drag-drop
  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(true);
  }, []);
  const onDragLeave = useCallback(() => setDragOver(false), []);
  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const tauriPaths = (e as any).detail?.paths ?? null;
    if (tauriPaths?.length) acceptPath(tauriPaths[0]);
  }, [acceptPath]);

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
      } catch {}
    })();
    return () => unlisten?.();
  }, [acceptPath]);

  // Sprint 5.6.3: use the plugin dialog directly. The HTML5
  // file input on Tauri 2.x no longer exposes absolute paths
  // on the File object (`f.path` is undefined since v2.0.0 for
  // security reasons), so `f.path || f.name` would silently
  // fall back to just the basename. The plugin dialog returns
  // real paths via its JS API.
  const onBrowse = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "All files", extensions: ["*"] }],
      });
      if (typeof result === "string") acceptPath(result);
    } catch (e) {
      console.error("file picker:", e);
    }
  }, [acceptPath]);

  // Also support picking a folder (for sending whole directories).
  const onBrowseFolder = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: false,
        directory: true,
      });
      if (typeof result === "string") acceptPath(result);
    } catch (e) {
      console.error("folder picker:", e);
    }
  }, [acceptPath]);

  const onAddPath = useCallback(() => {
    const t = pathInput.trim();
    if (t) acceptPath(t);
  }, [pathInput, acceptPath]);

  const onSend = useCallback(async () => {
    if (!filePath) return;
    setBusy(true);
    setError(null);
    const startTime = Date.now();
    try {
      const r = await tauriInvoke<SendStartResp>("p2p_send_start_cmd", {
        req: { file_path: filePath, code: null },
      });
      setResp(r);
      onComplete({
        kind: "share",
        filename: r.filename,
        originalBytes: r.file_size,
        durationMs: Date.now() - startTime,
      });
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
    }
  }, [filePath, onComplete]);

  const onCopy = useCallback(async (text: string, which: "code" | "token") => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(which);
      setTimeout(() => setCopied(null), 1500);
    } catch {}
  }, []);

  const filename = filePath ? filePath.split("/").pop() : null;
  const crossNat = !!resp?.upnp_status;

  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`rounded-3xl transition-colors ${
        dragOver ? "bg-emerald-500/[0.04]" : ""
      }`}
    >
      {!resp ? (
        // File picker state
        <div className="rounded-3xl border-2 border-dashed border-white/[0.08] bg-white/[0.02] p-16 text-center">
          <div className="text-7xl mb-6 select-none">
            {dragOver ? "⤓" : "🚀"}
          </div>
          <h3 className="text-white text-[20px] font-medium mb-2">
            {dragOver ? "Suelta para enviar" : "Arrastra el archivo a enviar"}
          </h3>
          <p className="text-zinc-500 text-[13px] mb-6">
            o usa el campo de abajo para escribir la ruta
          </p>
          <div className="flex items-center gap-2 max-w-xl mx-auto mb-6">
            <input
              type="text"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && onAddPath()}
              placeholder="/Users/usuario/Desktop/pelicula.mkv o ~/Documents"
              className="flex-1 bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-2.5 text-[13px] text-white placeholder-zinc-600 focus:outline-none focus:border-emerald-500/50"
            />
            <button
              onClick={onBrowse}
              className="px-4 py-2.5 text-[13px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-xl transition-colors"
              title="Pick a single file"
            >
              📄 Archivo
            </button>
            <button
              onClick={onBrowseFolder}
              className="px-4 py-2.5 text-[13px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-xl transition-colors"
              title="Pick a whole folder"
            >
              📁 Carpeta
            </button>
          </div>

          {filePath && (
            <div className="mt-6 p-4 rounded-2xl bg-white/[0.03] border border-white/[0.06]">
              <div className="flex items-center gap-3 text-[13px] mb-4">
                <span className="text-emerald-400">📄</span>
                <span className="text-white flex-1 truncate text-left">{filename}</span>
                <button
                  onClick={() => setFilePath(null)}
                  className="text-zinc-500 hover:text-red-400 text-[12px]"
                >
                  ✕
                </button>
              </div>
              <button
                onClick={onSend}
                disabled={busy}
                className="w-full py-3.5 rounded-xl bg-gradient-to-b from-emerald-500 to-emerald-600 hover:from-emerald-400 hover:to-emerald-500 text-white text-[14px] font-semibold tracking-tight transition-all shadow-lg shadow-emerald-500/20 disabled:opacity-40"
              >
                {busy ? "Preparando…" : "Crear enlace"}
              </button>
            </div>
          )}
        </div>
      ) : (
        // Code display state
        <div className="rounded-3xl bg-gradient-to-br from-emerald-500/[0.08] to-cyan-500/[0.04] border border-white/[0.08] p-10">
          <div className="text-center mb-8">
            <div
              className={`inline-flex items-center gap-2 px-3 py-1 rounded-full text-[11px] tracking-widest uppercase mb-4 ${
                crossNat
                  ? "bg-emerald-500/20 text-emerald-300"
                  : "bg-amber-500/20 text-amber-300"
              }`}
            >
              <span className="w-1.5 h-1.5 rounded-full bg-current" />
              {crossNat ? "Listo para enviar a cualquier red" : "Listo en misma Wi-Fi"}
            </div>

            <div className="text-zinc-400 text-[11px] tracking-[0.3em] uppercase mb-3">
              Comparte este código
            </div>
            <div className="text-white text-[42px] font-mono font-bold tracking-[0.15em] mb-3 select-all">
              {resp.code}
            </div>
            <div className="text-zinc-500 text-[13px] mb-6">
              {resp.filename} · {prettyBytes(resp.file_size)}
            </div>

            <div className="flex items-center justify-center gap-2 mb-8">
              <button
                onClick={() => onCopy(resp.code, "code")}
                className="px-5 py-2.5 bg-white text-black text-[13px] font-semibold rounded-xl hover:bg-zinc-200 transition-colors"
              >
                {copied === "code" ? "✓ Copiado" : "Copiar código"}
              </button>
              <button
                onClick={() => onCopy(resp.token, "token")}
                className="px-5 py-2.5 bg-white/[0.06] text-white text-[13px] font-medium rounded-xl hover:bg-white/[0.1] transition-colors border border-white/[0.08]"
              >
                {copied === "token" ? "✓ Copiado" : "Copiar token"}
              </button>
            </div>

            {resp.upnp_status && (
              <div className="text-zinc-600 text-[11px] font-mono">
                conexión: {resp.upnp_status.external_ip}:{resp.upnp_status.external_port}
              </div>
            )}
          </div>
        </div>
      )}

      {error && (
        <div className="mt-4 p-4 rounded-xl bg-red-500/[0.08] border border-red-500/20 text-red-400 text-[13px]">
          {error}
        </div>
      )}
    </div>
  );
}

// ============================================================
//  Receive Tab
// ============================================================

function ReceiveTab() {
  const [token, setToken] = useState("");
  // Sprint 5.6.8: resolvedDownloads = actual filesystem path
  // for ~/Downloads. The display value is "~/Downloads" but
  // when we send to the backend we use resolvedDownloads +
  // suggestedName so the mkdir + write actually works.
  const [outputPath, setOutputPath] = useState<string>("~/Downloads");
  const [resolvedDownloads, setResolvedDownloads] = useState<string>("");
  const [resolvedHome, setResolvedHome] = useState<string>("");
  // Original filename from the sender. Used as the default
  // save name so the user doesn't see "received.bin".
  const [suggestedName, setSuggestedName] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ReceiveResp | null>(null);
  const [steps, setSteps] = useState<{ icon: string; label: string; status: "pending" | "active" | "done" | "error" }[]>([]);

  const kind = (() => {
    const t = token.trim();
    if (t.startsWith("nx:1:")) return "v1" as const;
    if (t.startsWith("nx:2:")) return "v2" as const;
    if (t.startsWith("nx:3:")) return "v3" as const;
    return "unknown" as const;
  })();

  // Sprint 5.6.9: resolve home + ~/Downloads to real paths on
  // mount. Rust doesn't expand ~, so we MUST do it here.
  useEffect(() => {
    if (!isTauri) return;
    let alive = true;
    (async () => {
      try {
        const { homeDir, join } = await import("@tauri-apps/api/path");
        const home = await homeDir();
        if (!alive) return;
        setResolvedHome(home);
        const dl = await join(home, "Downloads");
        if (!alive) return;
        setResolvedDownloads(dl);
      } catch (e) {
        console.error("homeDir resolve failed", e);
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  // Sprint 5.6.9: parse v1 client-side, peek v2/v3 from backend.
  // v1 has filename in the base64 JSON; v2/v3 require a round
  // trip to /meta on the sender's HTTP server.
  useEffect(() => {
    const t = token.trim();
    if (!t) {
      setSuggestedName("");
      return;
    }
    if (kind === "v1") {
      try {
        const b64 = t.slice("nx:1:".length);
        const json = JSON.parse(
          atob(b64.replace(/-/g, "+").replace(/_/g, "/"))
        );
        if (typeof json.filename === "string" && json.filename) {
          setSuggestedName(json.filename);
        }
      } catch {
        // ignore
      }
      return;
    }
    if (kind === "v2" || kind === "v3") {
      let cancelled = false;
      (async () => {
        try {
          const r = await tauriInvoke<{ filename: string | null }>(
            "p2p_peek_filename_cmd",
            { req: { token: t, timeout_secs: 5 } }
          );
          if (!cancelled && r?.filename) {
            setSuggestedName(r.filename);
          }
        } catch (e) {
          // sender not reachable yet — that's fine, user might
          // be typing the token. Just leave suggestedName empty.
        }
      })();
      return () => {
        cancelled = true;
      };
    }
  }, [token, kind]);

  const STEPS = kind === "v1"
    ? [
        { icon: "🔍", label: "Detectando formato" },
        { icon: "☁️", label: "Conectando al relay" },
        { icon: "🔐", label: "Estableciendo conexión segura" },
        { icon: "📦", label: "Transfiriendo" },
        { icon: "✓", label: "Verificando integridad" },
      ]
    : [
        { icon: "🔍", label: "Detectando formato" },
        { icon: "📡", label: "Buscando la otra computadora en la red" },
        { icon: "🔐", label: "Estableciendo conexión segura directa" },
        { icon: "📦", label: "Transfiriendo" },
        { icon: "✓", label: "Verificando integridad" },
      ];

  const onBrowseDest = useCallback(async () => {
    if (!isTauri) return;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      // Sprint 5.6.8: default the save dialog to the actual
      // ~/Downloads + suggested filename (from token / /meta),
      // not the hardcoded "archivo_recibido.bin".
      const { join } = await import("@tauri-apps/api/path");
      let defaultPath = "archivo_recibido.bin";
      if (resolvedDownloads && suggestedName) {
        defaultPath = await join(resolvedDownloads, suggestedName);
      } else if (resolvedDownloads) {
        defaultPath = await join(resolvedDownloads, defaultPath);
      }
      const r = await save({
        defaultPath,
        filters: [{ name: "All files", extensions: ["*"] }],
      });
      if (r) setOutputPath(r);
    } catch (e) {
      console.error(e);
    }
  }, [resolvedDownloads, suggestedName]);

  // Sprint 5.6.9: compute the actual output_path the backend
  // writes to. Three jobs:
//   1. Expand any leading "~/" (Rust does NOT expand ~).
//   2. If the user kept the default "~/Downloads" sentinel,
//      substitute resolvedDownloads + suggestedName.
//   3. If the user picked a custom path but the trailing
//      filename is still the placeholder ("archivo_recibido"
//      or anything ending in ".bin"), replace just the
//      basename with the real suggested filename.
  const computeActualOutputPath = useCallback(async (): Promise<string> => {
    let path = outputPath;
    const { join, dirname, basename } = await import("@tauri-apps/api/path");
    // 1. expand leading ~/
    if (path.startsWith("~/") && resolvedHome) {
      const homeName = path.slice(2);
      path = await join(resolvedHome, homeName);
    } else if (path === "~" && resolvedHome) {
      path = resolvedHome;
    } else if (path === "~/Downloads" && resolvedDownloads) {
      // sentinel → Downloads dir
      path = resolvedDownloads;
    }
    // 2. placeholder basename → swap to suggestedName
    const currentBase = await basename(path);
    const looksPlaceholder =
      currentBase === "archivo_recibido" ||
      currentBase === "archivo_recibido.bin" ||
      currentBase.endsWith(".bin");
    if (looksPlaceholder && suggestedName) {
      const dir = await dirname(path);
      path = await join(dir, suggestedName);
    }
    return path;
  }, [outputPath, resolvedDownloads, resolvedHome, suggestedName]);

  const onReceive = useCallback(async () => {
    if (!token) return;
    setBusy(true);
    setError(null);
    setResult(null);
    setSteps(STEPS.map((s) => ({ ...s, status: "pending" })));

    const update = (idx: number, status: "active" | "done" | "error") => {
      setSteps((prev) => {
        const next = [...prev];
        if (next[idx]) next[idx] = { ...next[idx], status };
        return next;
      });
    };

    update(0, "done");
    update(1, "active");

    let actualPath: string;
    try {
      actualPath = await computeActualOutputPath();
    } catch (e: any) {
      setError(String(e?.message ?? e));
      setBusy(false);
      return;
    }

    try {
      let r: ReceiveResp;
      if (kind === "v2" || kind === "v3") {
        r = await tauriInvoke<ReceiveResp>("p2p_receive_direct_cmd", {
          req: {
            token: token.trim(),
            output_path: actualPath,
            timeout_secs: 5,
          },
        });
      } else {
        r = await tauriInvoke<ReceiveResp>("p2p_receive_cmd", {
          req: { token: token.trim(), output_path: actualPath },
        });
      }
      // After the receive, prefer the filename reported by the
      // sender (it knows the truth) over what we guessed.
      if (r.filename) setSuggestedName(r.filename);
      update(1, "done");
      update(2, "done");
      update(3, "done");
      update(4, "done");
      setResult(r);
    } catch (e: any) {
      const msg = String(e?.message ?? e);
      setError(msg);
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
    }
  }, [token, outputPath, kind, STEPS, computeActualOutputPath]);

  return (
    <div>
      {/* Token input */}
      <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] p-6 mb-6">
        <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
          Código o enlace
        </div>
        <textarea
          value={token}
          onChange={(e) => setToken(e.target.value)}
          rows={3}
          placeholder="Pega el código de 4 palabras o el token completo"
          className="w-full bg-white/[0.04] border border-white/[0.08] rounded-xl px-4 py-3 text-[15px] text-white font-mono placeholder-zinc-600 focus:outline-none focus:border-cyan-500/50 resize-none"
        />
        {kind === "v3" && (
          <div className="mt-2 text-emerald-400 text-[12px]">
            ✓ Detectado: enlace directo cross-red
          </div>
        )}
        {kind === "v2" && (
          <div className="mt-2 text-amber-400 text-[12px]">
            ✓ Detectado: enlace LAN (misma Wi-Fi)
          </div>
        )}
        {kind === "unknown" && token.length > 0 && (
          <div className="mt-2 text-red-400 text-[12px]">
            ✗ Formato no reconocido
          </div>
        )}
      </div>

      {/* Destination */}
      <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] p-6 mb-6">
        <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-3">
          Guardar como
        </div>
        <div className="flex items-center gap-3">
          <span className="text-zinc-500 text-[14px]">📁</span>
          <span className="text-white text-[14px] flex-1 truncate font-mono">
            {outputPath}
          </span>
          <button
            onClick={onBrowseDest}
            className="px-3 py-1.5 text-[12px] text-zinc-400 hover:text-white border border-white/[0.08] hover:border-white/[0.16] rounded-lg transition-colors"
          >
            Cambiar
          </button>
        </div>
        {/* Sprint 5.6.9: show the suggested filename whenever we
            know it. */}
        {suggestedName && (
          <div className="text-zinc-500 text-[12px] mt-3 flex items-center gap-2">
            <span className="text-cyan-400">↻</span>
            Se guardará como
            <span className="text-zinc-200 font-medium">{suggestedName}</span>
          </div>
        )}
      </div>

      {/* Receive button */}
      <button
        onClick={onReceive}
        disabled={!token || !outputPath || busy || kind === "unknown"}
        className="w-full py-4 rounded-2xl bg-gradient-to-b from-cyan-500 to-cyan-600 hover:from-cyan-400 hover:to-cyan-500 disabled:from-zinc-800 disabled:to-zinc-800 disabled:text-zinc-600 text-white text-[15px] font-semibold tracking-tight transition-all shadow-lg shadow-cyan-500/20 disabled:shadow-none mb-6"
      >
        {busy ? "Recibiendo…" : "Recibir"}
      </button>

      {/* Progress steps */}
      {(busy || steps.length > 0) && steps.some((s) => s.status !== "pending") && (
        <div className="rounded-2xl bg-white/[0.03] border border-white/[0.08] p-6 mb-6">
          <div className="text-zinc-500 text-[11px] tracking-[0.2em] uppercase mb-4">
            Progreso
          </div>
          <div className="space-y-2.5">
            {steps.map((s, i) => (
              <div
                key={i}
                className={`text-[13.5px] flex items-center gap-3 ${
                  s.status === "done"
                    ? "text-emerald-300"
                    : s.status === "active"
                    ? "text-cyan-300"
                    : s.status === "error"
                    ? "text-red-400"
                    : "text-zinc-700"
                }`}
              >
                <span className="w-5 text-center text-[14px]">
                  {s.status === "done" ? "✓" : s.status === "active" ? "⟳" : s.status === "error" ? "✗" : "·"}
                </span>
                <span className="opacity-70 text-[15px]">{s.icon}</span>
                <span>{s.label}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Result */}
      {result && (
        <div className="rounded-2xl bg-gradient-to-br from-emerald-500/[0.08] to-cyan-500/[0.04] border border-emerald-500/20 p-6">
          <div className="text-emerald-300 text-[11px] tracking-[0.2em] uppercase mb-2">
            ✓ Recibido
          </div>
          <div className="text-white text-[28px] font-semibold tabular-nums mb-2">
            {prettyBytes(result.bytes_written)}
          </div>
          <div className="text-zinc-200 text-[15px] mb-2 font-medium">
            {result.filename || "archivo"}
          </div>
          <div className="text-zinc-400 text-[12px] font-mono truncate">
            {result.output_path}
          </div>
        </div>
      )}

      {error && (
        <div className="mt-4 p-4 rounded-xl bg-red-500/[0.08] border border-red-500/20 text-red-400 text-[13px]">
          {error}
        </div>
      )}
    </div>
  );
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(2)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}