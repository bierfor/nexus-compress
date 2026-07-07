"use client";

import { useEffect, useRef, useState } from "react";
import Hls from "hls.js";

export interface PlayerProps {
  src: string; // The HLS (.m3u8) or any direct URL to play
  title: string;
  poster?: string | null;
  onClose: () => void;
  referer?: string | null;
}

// Public CORS proxy services. We try multiple fallbacks because they often go down.
const CORS_PROXIES = [
  // corsproxy.io - free, no key required
  (url: string) => `https://corsproxy.io/?${encodeURIComponent(url)}`,
  // allorigins.win - free CORS proxy
  (url: string) => `https://api.allorigins.win/raw?url=${encodeURIComponent(url)}`,
  // codetabs.com CORS proxy
  (url: string) => `https://api.codetabs.com/v1/proxy/?quest=${encodeURIComponent(url)}`,
];

function buildProxiedUrls(url: string): string[] {
  return CORS_PROXIES.map((p) => p(url));
}

export default function Player({ src, title, poster, onClose, referer }: PlayerProps) {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const hlsRef = useRef<Hls | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [muted, setMuted] = useState(false);
  const [usingProxy, setUsingProxy] = useState(false);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !src) return;

    setError(null);
    setLoading(true);

    const cleanup = () => {
      if (hlsRef.current) {
        try {
          hlsRef.current.destroy();
        } catch {}
        hlsRef.current = null;
      }
      video.removeAttribute("src");
      video.load();
    };

    const tryDirect = () => {
      setUsingProxy(false);
      video.src = src;
      video.load();
    };

    const tryWithProxies = async (urls: string[], idx = 0) => {
      if (idx >= urls.length) {
        setError("No se pudo reproducir el canal. La fuente puede estar offline o bloqueada geográficamente.");
        setLoading(false);
        return;
      }
      const proxied = urls[idx];
      setUsingProxy(true);
      if (Hls.isSupported()) {
        const hls = new Hls({ enableWorker: true, lowLatencyMode: false });
        hlsRef.current = hls;
        hls.attachMedia(video);
        hls.on(Hls.Events.MEDIA_ATTACHED, () => {
          hls.loadSource(proxied);
        });
        hls.on(Hls.Events.MANIFEST_PARSED, () => {
          setLoading(false);
          video.play().catch(() => {});
        });
        hls.on(Hls.Events.ERROR, (_event, data) => {
          if (data.fatal) {
            try {
              hls.destroy();
            } catch {}
            hlsRef.current = null;
            tryWithProxies(urls, idx + 1);
          }
        });
      } else {
        video.src = proxied;
        video.addEventListener(
          "loadedmetadata",
          () => {
            setLoading(false);
            video.play().catch(() => {});
          },
          { once: true }
        );
        video.addEventListener(
          "error",
          () => {
            tryWithProxies(urls, idx + 1);
          },
          { once: true }
        );
      }
    };

    // Start with direct attempt, then fall back to proxies
    tryDirect();

    if (Hls.isSupported()) {
      const hls = new Hls({ enableWorker: true, lowLatencyMode: false });
      hlsRef.current = hls;
      hls.attachMedia(video);
      hls.on(Hls.Events.MEDIA_ATTACHED, () => {
        hls.loadSource(src);
      });
      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        setLoading(false);
        video.play().catch(() => {});
      });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (data.fatal) {
          try {
            hls.destroy();
          } catch {}
          hlsRef.current = null;
          // Try via proxies
          tryWithProxies(buildProxiedUrls(src));
        }
      });
    } else {
      // Safari / native HLS path
      video.addEventListener(
        "loadedmetadata",
        () => {
          setLoading(false);
          video.play().catch(() => {});
        },
        { once: true }
      );
      video.addEventListener(
        "error",
        () => {
          tryWithProxies(buildProxiedUrls(src));
        },
        { once: true }
      );
    }

    return cleanup;
  }, [src, referer]);

  return (
    <div className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4 fade-in">
      <div className="relative w-full max-w-5xl bg-slate-950 rounded-2xl overflow-hidden shadow-2xl border border-slate-800">
        <div className="flex items-center justify-between px-4 py-3 border-b border-slate-800">
          <div className="flex items-center gap-2 min-w-0">
            <h3 className="text-slate-100 font-semibold truncate">{title}</h3>
            {usingProxy && (
              <span className="text-[10px] px-2 py-0.5 rounded-full bg-indigo-500/20 text-indigo-300 border border-indigo-500/30">
                vía proxy CORS
              </span>
            )}
          </div>
          <button
            onClick={onClose}
            className="text-slate-400 hover:text-white transition-colors p-1.5 rounded-lg hover:bg-slate-800"
            aria-label="Cerrar reproductor"
          >
            <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 6L6 18M6 6l12 12" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </button>
        </div>

        <div className="relative aspect-video bg-black">
          <video
            ref={videoRef}
            controls
            playsInline
            autoPlay
            muted={muted}
            poster={poster ?? undefined}
            className="w-full h-full"
            crossOrigin="anonymous"
          />

          {loading && !error && (
            <div className="absolute inset-0 flex items-center justify-center bg-black/60 pointer-events-none">
              <div className="flex flex-col items-center gap-3">
                <svg className="animate-spin h-10 w-10 text-indigo-400" viewBox="0 0 24 24" fill="none">
                  <circle cx="12" cy="12" r="10" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
                  <path d="M22 12a10 10 0 0 1-10 10" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
                </svg>
                <p className="text-slate-300 text-sm">Cargando stream…</p>
              </div>
            </div>
          )}

          {error && (
            <div className="absolute inset-0 flex items-center justify-center bg-black/80 p-6">
              <div className="text-center max-w-md">
                <svg className="w-12 h-12 text-rose-400 mx-auto mb-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <path d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
                <p className="text-slate-200 font-semibold mb-2">Stream no disponible</p>
                <p className="text-slate-400 text-sm mb-4">{error}</p>
                <p className="text-slate-500 text-xs">
                  Algunos canales están geobloqueados u offline temporalmente. Intenta con otro canal.
                </p>
              </div>
            </div>
          )}
        </div>

        <div className="px-4 py-3 flex items-center justify-between text-xs text-slate-400 bg-slate-950">
          <span className="truncate max-w-[60%]">{src}</span>
          <div className="flex items-center gap-3">
            <a
              href={src}
              target="_blank"
              rel="noopener noreferrer"
              className="hover:text-indigo-300 transition-colors"
            >
              Abrir fuente
            </a>
            <button
              onClick={() => {
                setMuted((m) => !m);
                if (videoRef.current) videoRef.current.muted = !muted;
              }}
              className="hover:text-indigo-300 transition-colors"
            >
              {muted ? "Activar audio" : "Silenciar"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
