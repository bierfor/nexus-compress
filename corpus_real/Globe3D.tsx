"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import dynamic from "next/dynamic";
import * as topojson from "topojson-client";
import type { Topology } from "topojson-specification";
import type { FeatureCollection, MultiPolygon, Polygon } from "geojson";

// react-globe.gl must run on the client only
const Globe = dynamic(() => import("react-globe.gl"), { ssr: false }) as any;

interface CountryFeature {
  type: "Feature";
  properties: { name: string; iso2: string; available: boolean };
  geometry: Polygon | MultiPolygon;
}

interface Globe3DProps {
  onCountryClick: (iso2: string, name: string) => void;
  selectedIso2?: string | null;
  availableCountries?: Set<string>;
}

function nameToIso2(_name: string, numericId: string): string {
  // numericId is ISO 3166-1 numeric as string (e.g. "604" for Peru)
  const map: Record<string, string> = {
    "004": "af", "008": "al", "010": "aq", "012": "dz", "016": "as", "020": "ad",
    "024": "ao", "028": "ag", "031": "az", "032": "ar", "036": "au", "040": "at",
    "044": "bs", "048": "bh", "050": "bd", "051": "am", "052": "bb", "056": "be",
    "060": "bm", "064": "bt", "068": "bo", "070": "ba", "072": "bw", "074": "bv",
    "076": "br", "084": "bz", "090": "sb", "092": "vg", "096": "bn", "100": "bg",
    "104": "mm", "108": "bi", "112": "by", "116": "kh", "120": "cm", "124": "ca",
    "132": "cv", "136": "ky", "140": "cf", "144": "lk", "148": "td", "152": "cl",
    "156": "cn", "158": "tw", "162": "cx", "166": "cc", "170": "co", "174": "km",
    "175": "yt", "178": "cg", "180": "cd", "184": "ck", "188": "cr", "191": "hr",
    "192": "cu", "196": "cy", "203": "cz", "204": "bj", "208": "dk", "212": "dm",
    "214": "do", "218": "ec", "222": "sv", "226": "gq", "231": "et", "232": "er",
    "233": "ee", "234": "fo", "238": "fk", "239": "gs", "242": "fj", "246": "fi",
    "248": "ax", "250": "fr", "254": "gf", "258": "pf", "260": "tf", "262": "dj",
    "266": "ga", "268": "ge", "270": "gm", "275": "ps", "276": "de", "288": "gh",
    "292": "gi", "296": "ki", "300": "gr", "304": "gl", "308": "gd", "312": "gp",
    "316": "gu", "320": "gt", "324": "gn", "328": "gy", "332": "ht", "336": "va",
    "340": "hn", "344": "hk", "348": "hu", "352": "is", "356": "in", "360": "id",
    "364": "ir", "368": "iq", "372": "ie", "376": "il", "380": "it", "384": "ci",
    "388": "jm", "392": "jp", "398": "kz", "400": "jo", "404": "ke", "408": "kp",
    "410": "kr", "414": "kw", "417": "kg", "418": "la", "422": "lb", "426": "ls",
    "428": "lv", "430": "lr", "434": "ly", "438": "li", "440": "lt", "442": "lu",
    "446": "mo", "450": "mg", "454": "mw", "458": "my", "462": "mv", "466": "ml",
    "470": "mt", "474": "mq", "478": "mr", "480": "mu", "484": "mx", "492": "mc",
    "496": "mn", "498": "md", "499": "me", "500": "ms", "504": "ma", "508": "mz",
    "512": "om", "516": "na", "520": "nr", "524": "np", "528": "nl", "531": "cw",
    "533": "aw", "534": "sx", "535": "bq", "540": "nc", "548": "vu", "554": "nz",
    "558": "ni", "562": "ne", "566": "ng", "570": "nu", "574": "nf", "578": "no",
    "580": "mp", "581": "um", "583": "fm", "584": "mh", "585": "pw", "586": "pk",
    "591": "pa", "598": "pg", "600": "py", "604": "pe", "608": "ph", "616": "pl",
    "620": "pt", "624": "gw", "626": "tl", "630": "pr", "634": "qa", "638": "re",
    "642": "ro", "643": "ru", "646": "rw", "652": "bl", "654": "sh", "659": "kn",
    "660": "ai", "662": "lc", "663": "mf", "666": "pm", "670": "vc", "674": "sm",
    "678": "st", "682": "sa", "686": "sn", "688": "rs", "690": "sc", "694": "sl",
    "702": "sg", "703": "sk", "704": "vn", "705": "si", "706": "so", "710": "za",
    "716": "zw", "724": "es", "728": "ss", "729": "sd", "732": "eh", "740": "sr",
    "744": "sj", "748": "sz", "752": "se", "756": "ch", "760": "sy", "762": "tj",
    "764": "th", "768": "tg", "772": "tk", "776": "to", "780": "tt", "784": "ae",
    "788": "tn", "792": "tr", "795": "tm", "796": "tc", "798": "tv", "800": "ug",
    "804": "ua", "807": "mk", "818": "eg", "826": "gb", "831": "gg", "832": "je",
    "833": "im", "834": "tz", "840": "us", "850": "vi", "854": "bf", "858": "uy",
    "860": "uz", "862": "ve", "876": "wf", "882": "ws", "887": "ye", "894": "zm",
    "983": "xk",
  };
  return map[numericId] ?? "";
}

export default function Globe3D({
  onCountryClick,
  selectedIso2,
  availableCountries,
}: Globe3DProps) {
  const globeEl = useRef<any>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 800, height: 600 });
  const [hoverIso2, setHoverIso2] = useState<string | null>(null);
  const [countries, setCountries] = useState<CountryFeature[]>([]);
  const [loading, setLoading] = useState(true);

  // Resize observer
  useEffect(() => {
    if (!containerRef.current) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        setSize({ width, height });
      }
    });
    ro.observe(containerRef.current);
    return () => ro.disconnect();
  }, []);

  // Load TopoJSON and build features
  useEffect(() => {
    let mounted = true;
    fetch("/data/countries-110m.json")
      .then((r) => r.json())
      .then((topo: Topology) => {
        if (!mounted) return;
        const obj = topo.objects["countries"] as any;
        const fc = topojson.feature(topo, obj) as unknown as FeatureCollection<Polygon | MultiPolygon>;
        const features: CountryFeature[] = (fc.features as any[])
          .map((f: any) => {
            const numericId: string = String(f.id ?? "");
            const iso2 = nameToIso2(f.properties?.name ?? "", numericId);
            if (!iso2) return null;
            return {
              type: "Feature",
              properties: {
                name: f.properties?.name ?? iso2,
                iso2,
                available: true,
              },
              geometry: f.geometry,
            };
          })
          .filter(Boolean) as CountryFeature[];
        setCountries(features);
        setLoading(false);
      })
      .catch(() => setLoading(false));
    return () => {
      mounted = false;
    };
  }, []);

  // Initial camera + auto-rotate
  useEffect(() => {
    if (!globeEl.current) return;
    const g = globeEl.current;
    g.controls().autoRotate = true;
    g.controls().autoRotateSpeed = 0.35;
    g.controls().enableDamping = true;
    g.pointOfView({ lat: 15, lng: -30, altitude: 2.6 }, 0);
  }, [loading]);

  // Stop auto-rotation when user interacts
  useEffect(() => {
    if (!globeEl.current) return;
    const g = globeEl.current;
    const onStart = () => {
      if (g.controls().autoRotate) g.controls().autoRotate = false;
    };
    g.controls()?.addEventListener?.("start", onStart);
    return () => g.controls()?.removeEventListener?.("start", onStart);
  }, [loading]);

  // Animate camera when a country is selected
  useEffect(() => {
    if (!globeEl.current || !selectedIso2) return;
    const g = globeEl.current;
    g.controls().autoRotate = false;
    const feature = countries.find((c) => c.properties.iso2 === selectedIso2);
    if (!feature) return;
    // Compute centroid (rough) from first polygon
    let lat = 0;
    let lng = 0;
    let count = 0;
    const visit = (ring: number[][]) => {
      for (const pt of ring) {
        lng += pt[0];
        lat += pt[1];
        count++;
      }
    };
    const geom: any = feature.geometry;
    if (geom.type === "Polygon") {
      geom.coordinates.forEach((ring: number[][]) => visit(ring));
    } else if (geom.type === "MultiPolygon") {
      geom.coordinates.forEach((poly: number[][][]) => poly.forEach((r) => visit(r)));
    }
    if (count === 0) return;
    lat /= count;
    lng /= count;
    g.pointOfView({ lat, lng, altitude: 1.4 }, 900);
  }, [selectedIso2, countries]);

  const polygonsData = useMemo(() => countries, [countries]);

  const polygonCapColor = (d: any) => {
    const iso2 = d?.properties?.iso2;
    const hasAvail = !availableCountries || availableCountries.has(iso2);
    if (iso2 && iso2 === selectedIso2) return "rgba(99, 102, 241, 0.95)";
    if (iso2 && iso2 === hoverIso2)
      return hasAvail ? "rgba(96, 165, 250, 0.85)" : "rgba(148, 163, 184, 0.75)";
    return hasAvail ? "rgba(30, 64, 175, 0.55)" : "rgba(30, 41, 59, 0.45)";
  };

  const polygonSideColor = () => "rgba(2, 6, 23, 0.6)";
  const polygonStrokeColor = () => "rgba(148, 163, 184, 0.35)";

  return (
    <div ref={containerRef} className="relative w-full h-full">
      {loading && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none z-10">
          <div className="flex flex-col items-center gap-3 text-slate-400">
            <svg
              className="animate-spin h-8 w-8 text-indigo-400"
              viewBox="0 0 24 24"
              fill="none"
            >
              <circle cx="12" cy="12" r="10" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
              <path
                d="M22 12a10 10 0 0 1-10 10"
                stroke="currentColor"
                strokeWidth="3"
                strokeLinecap="round"
              />
            </svg>
            <p className="text-sm">Cargando globo terráqueo…</p>
          </div>
        </div>
      )}

      <Globe
        ref={globeEl}
        width={size.width}
        height={size.height}
        backgroundColor="rgba(2,6,23,0)"
        globeImageUrl={null}
        showAtmosphere={true}
        atmosphereColor="#60a5fa"
        atmosphereAltitude={0.18}
        polygonsData={polygonsData}
        polygonAltitude={(d: any) =>
          d?.properties?.iso2 === selectedIso2 ? 0.04 : 0.012
        }
        polygonCapColor={polygonCapColor}
        polygonSideColor={polygonSideColor}
        polygonStrokeColor={polygonStrokeColor}
        onPolygonHover={(d: any) => {
          if (d) {
            setHoverIso2(d.properties.iso2);
            document.body.style.cursor = "pointer";
          } else {
            setHoverIso2(null);
            document.body.style.cursor = "default";
          }
        }}
        onPolygonClick={(d: any) => {
          if (!d) return;
          const iso2 = d.properties.iso2;
          if (availableCountries && !availableCountries.has(iso2)) {
            // Country has no IPTV playlist - still allow but show empty state
          }
          onCountryClick(iso2, d.properties.name);
        }}
        polygonsTransitionDuration={300}
      />

      {/* Hover tooltip */}
      {hoverIso2 && (() => {
        const c = countries.find((x) => x.properties.iso2 === hoverIso2);
        if (!c) return null;
        const hasAvail = !availableCountries || availableCountries.has(hoverIso2);
        return (
          <div className="pointer-events-none absolute bottom-6 left-1/2 -translate-x-1/2 z-20 bg-slate-900/90 border border-slate-700 backdrop-blur-md px-4 py-2 rounded-lg shadow-2xl">
            <div className="flex items-center gap-2">
              <span className="text-2xl leading-none">
                {iso2ToFlag(hoverIso2)}
              </span>
              <div className="flex flex-col">
                <span className="text-sm font-semibold text-slate-100">
                  {prettyCountryName(hoverIso2, c.properties.name)}
                </span>
                <span className="text-xs text-slate-400">
                  {hasAvail ? "Click para ver canales" : "Sin playlist disponible"}
                </span>
              </div>
            </div>
          </div>
        );
      })()}
    </div>
  );
}

function iso2ToFlag(iso2: string): string {
  if (!iso2 || iso2.length !== 2) return "🏳️";
  const codePoints = iso2
    .toUpperCase()
    .split("")
    .map((c) => 127397 + c.charCodeAt(0));
  return String.fromCodePoint(...codePoints);
}

function prettyCountryName(iso2: string, fallback: string): string {
  const map: Record<string, string> = {
    pe: "Perú", us: "Estados Unidos", gb: "Reino Unido", uk: "Reino Unido",
    cz: "Chequia", de: "Alemania", es: "España", fr: "Francia", it: "Italia",
    pt: "Portugal", br: "Brasil", mx: "México", ar: "Argentina", cl: "Chile",
    co: "Colombia", ve: "Venezuela", ec: "Ecuador", bo: "Bolivia", uy: "Uruguay",
    py: "Paraguay", cr: "Costa Rica", pa: "Panamá", cu: "Cuba", do: "República Dominicana",
    jp: "Japón", cn: "China", kr: "Corea del Sur", in: "India", ru: "Rusia",
    au: "Australia", nz: "Nueva Zelanda", ca: "Canadá", za: "Sudáfrica", eg: "Egipto",
    ng: "Nigeria", ke: "Kenia", ma: "Marruecos", tr: "Turquía", sa: "Arabia Saudita",
    ae: "Emiratos Árabes Unidos", il: "Israel", se: "Suecia", no: "Noruega", fi: "Finlandia",
    dk: "Dinamarca", pl: "Polonia", nl: "Países Bajos", be: "Bélgica", ch: "Suiza",
    at: "Austria", gr: "Grecia", ie: "Irlanda", ua: "Ucrania", ro: "Rumania",
  };
  return map[iso2] ?? capitalize(fallback);
}

function capitalize(s: string): string {
  if (!s) return s;
  return s
    .toLowerCase()
    .split(" ")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}
