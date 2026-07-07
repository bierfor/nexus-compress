// Client-side IPTV module.
// Fetches M3U playlists directly from raw.githubusercontent.com (which allows CORS).
// Replaces the server-side /api routes when deployed as a static export.

import { parseM3U, M3UChannel } from "./m3u";
import { COUNTRIES, CountryInfo } from "./countries";

const IPTV_BASE = "https://raw.githubusercontent.com/iptv-org/iptv/master/streams";

interface CacheEntry {
  channels: M3UChannel[];
  raw: string;
  fetchedAt: number;
}
const cache = new Map<string, CacheEntry>();
const TTL_MS = 6 * 60 * 60 * 1000;

export interface CountryWithAvailability extends CountryInfo {
  available: boolean;
}

export function getCountries(): CountryWithAvailability[] {
  return Object.values(COUNTRIES)
    .map((c) => ({ ...c, available: true }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function getCountry(iso2: string): CountryWithAvailability | undefined {
  const c = COUNTRIES[iso2.toLowerCase()];
  if (!c) return undefined;
  return { ...c, available: true };
}

export async function fetchCountryChannels(iso2: string): Promise<M3UChannel[]> {
  const code = iso2.toLowerCase();
  const cached = cache.get(code);
  if (cached && Date.now() - cached.fetchedAt < TTL_MS) return cached.channels;

  const url = `${IPTV_BASE}/${code}.m3u`;
  const res = await fetch(url, { cache: "no-store" });
  if (!res.ok) {
    throw new Error(`Failed to fetch ${code}.m3u: ${res.status}`);
  }
  const raw = await res.text();
  const channels = parseM3U(raw);
  cache.set(code, { channels, raw, fetchedAt: Date.now() });
  return channels;
}

export async function fetchCountryRaw(iso2: string): Promise<string | null> {
  const code = iso2.toLowerCase();
  const cached = cache.get(code);
  if (cached && Date.now() - cached.fetchedAt < TTL_MS) return cached.raw;
  const url = `${IPTV_BASE}/${code}.m3u`;
  const res = await fetch(url, { cache: "no-store" });
  if (!res.ok) return null;
  const raw = await res.text();
  const channels = parseM3U(raw);
  cache.set(code, { channels, raw, fetchedAt: Date.now() });
  return raw;
}
