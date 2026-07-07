"use client";

import { useMemo } from "react";

export interface CountryLite {
  iso2: string;
  iso3: string;
  name: string;
  spanish: string;
  region: string;
  flag: string;
  available: boolean;
}

export interface RegionFilterProps {
  countries: CountryLite[];
  selected: string | null;
  onSelect: (iso2: string | null) => void;
  region: string;
  onRegionChange: (region: string) => void;
  search: string;
  onSearchChange: (s: string) => void;
  totalChannels: number;
}

const REGIONS = ["All", "Africa", "Americas", "Asia", "Europe", "Oceania"];

export default function RegionFilter({
  countries,
  selected,
  onSelect,
  region,
  onRegionChange,
  search,
  onSearchChange,
  totalChannels,
}: RegionFilterProps) {
  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    return countries.filter((c) => {
      if (region !== "All" && c.region !== region) return false;
      if (!q) return true;
      return (
        c.name.toLowerCase().includes(q) ||
        c.spanish.toLowerCase().includes(q) ||
        c.iso2.includes(q) ||
        c.iso3.toLowerCase().includes(q)
      );
    });
  }, [countries, region, search]);

  return (
    <aside className="fixed top-0 left-0 z-30 h-full w-72 bg-slate-950/90 backdrop-blur-xl border-r border-slate-800 flex flex-col shadow-2xl">
      <div className="px-5 py-5 border-b border-slate-800">
        <div className="flex items-center gap-2.5 mb-3">
          <div className="w-9 h-9 rounded-lg bg-gradient-to-br from-indigo-500 via-violet-500 to-pink-500 flex items-center justify-center shadow-lg">
            <svg className="w-5 h-5 text-white" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="9" />
              <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
            </svg>
          </div>
          <div>
            <h1 className="text-base font-bold gradient-text">TV Mundo 3D</h1>
            <p className="text-[10px] text-slate-500 -mt-0.5">Explorador IPTV global</p>
          </div>
        </div>

        <div className="relative">
          <svg
            className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-slate-500"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
          >
            <circle cx="11" cy="11" r="7" />
            <path d="M21 21l-4.35-4.35" strokeLinecap="round" />
          </svg>
          <input
            type="text"
            placeholder="Buscar país..."
            value={search}
            onChange={(e) => onSearchChange(e.target.value)}
            className="w-full bg-slate-900 border border-slate-800 rounded-lg pl-9 pr-3 py-2 text-sm text-slate-100 placeholder-slate-500 focus:border-indigo-500 focus:outline-none transition-colors"
          />
        </div>

        <div className="mt-3 flex items-center gap-2 text-[10px] text-slate-500">
          <span className="inline-flex items-center gap-1">
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
            {totalChannels} canales
          </span>
          <span>·</span>
          <span>{filtered.length} países</span>
        </div>
      </div>

      <div className="px-3 py-2 border-b border-slate-800">
        <div className="flex flex-wrap gap-1.5">
          {REGIONS.map((r) => (
            <button
              key={r}
              onClick={() => onRegionChange(r)}
              className={`text-[11px] px-2.5 py-1 rounded-full border transition-colors ${
                region === r
                  ? "bg-indigo-500/20 border-indigo-400 text-indigo-200"
                  : "bg-slate-900 border-slate-800 text-slate-400 hover:border-slate-700 hover:text-slate-200"
              }`}
            >
              {r}
            </button>
          ))}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="p-6 text-center text-slate-500 text-xs">
            Sin resultados.
          </div>
        ) : (
          <ul className="divide-y divide-slate-800/60">
            {filtered.map((c) => (
              <li key={c.iso2}>
                <button
                  onClick={() => onSelect(selected === c.iso2 ? null : c.iso2)}
                  className={`w-full text-left px-4 py-2.5 flex items-center gap-3 transition-colors ${
                    selected === c.iso2
                      ? "bg-indigo-500/15 border-l-2 border-indigo-400"
                      : "hover:bg-slate-900/50 border-l-2 border-transparent"
                  }`}
                >
                  <span className="text-xl leading-none">{c.flag}</span>
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-slate-100 truncate">
                      {c.spanish || c.name}
                    </p>
                    <p className="text-[10px] text-slate-500 uppercase tracking-wide">
                      {c.region}
                    </p>
                  </div>
                  {c.available ? (
                    <span className="w-1.5 h-1.5 rounded-full bg-emerald-400/80 shrink-0" />
                  ) : (
                    <span className="w-1.5 h-1.5 rounded-full bg-slate-700 shrink-0" />
                  )}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="px-4 py-3 border-t border-slate-800 text-[10px] text-slate-500">
        Datos de{" "}
        <a
          href="https://github.com/iptv-org/iptv"
          target="_blank"
          rel="noopener noreferrer"
          className="text-indigo-400 hover:text-indigo-300"
        >
          iptv-org/iptv
        </a>
        {" · "}
        Mapa:{" "}
        <a
          href="https://github.com/topojson/world-atlas"
          target="_blank"
          rel="noopener noreferrer"
          className="text-indigo-400 hover:text-indigo-300"
        >
          world-atlas
        </a>
      </div>
    </aside>
  );
}
