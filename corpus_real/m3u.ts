// M3U/M3U8 parser
// Parses extended M3U playlists from IPTV-org into structured channel objects.

export interface M3UChannel {
  name: string;
  logo: string | null;
  group: string | null;
  tvgId: string | null;
  tvgName: string | null;
  country: string | null;
  language: string | null;
  url: string;
  quality: string | null;
  not247: boolean;
  extras: Record<string, string>;
}

interface RawExtInf {
  attrs: Record<string, string>;
  name: string;
}

function parseExtInf(line: string): RawExtInf {
  // #EXTINF:-1 tvg-id="..." tvg-name="..." tvg-logo="..." group-title="..." ,Channel Name
  const result: RawExtInf = { attrs: {}, name: "" };
  const commaIdx = line.indexOf(",");
  if (commaIdx === -1) return result;

  const before = line.substring(0, commaIdx);
  result.name = line.substring(commaIdx + 1).trim();

  // Match key="value" or key=value
  const regex = /([a-zA-Z0-9-]+)="([^"]*)"|([a-zA-Z0-9-]+)=([^\s"]+)/g;
  let match;
  while ((match = regex.exec(before)) !== null) {
    const key = (match[1] || match[3]).toLowerCase();
    const val = match[2] !== undefined ? match[2] : match[4];
    result.attrs[key] = val;
  }

  return result;
}

function extractQuality(name: string): string | null {
  const m = name.match(/\((\d+p)\)/i);
  if (m) return m[1].toUpperCase();
  return null;
}

export function parseM3U(content: string): M3UChannel[] {
  const lines = content.split(/\r?\n/);
  const channels: M3UChannel[] = [];
  let lastExtInf: RawExtInf | null = null;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;
    if (line.startsWith("#EXTM3U")) continue;
    if (line.startsWith("#EXTINF")) {
      lastExtInf = parseExtInf(line);
    } else if (line.startsWith("#EXTVLCOPT")) {
      // ignore VLC-specific options for now
    } else if (line.startsWith("#")) {
      // unknown directive - ignore
    } else {
      // This is a URL line
      const url = line;
      if (!/^https?:\/\//i.test(url)) {
        lastExtInf = null;
        continue;
      }
      const name = lastExtInf?.name || "Unknown";
      const attrs = lastExtInf?.attrs || {};
      const channel: M3UChannel = {
        name,
        logo: attrs["tvg-logo"] || null,
        group: attrs["group-title"] || null,
        tvgId: attrs["tvg-id"] || null,
        tvgName: attrs["tvg-name"] || null,
        country: attrs["tvg-country"] || null,
        language: attrs["tvg-language"] || null,
        url,
        quality: extractQuality(name),
        not247: /\[Not 24\/7\]/i.test(name),
        extras: {},
      };
      channels.push(channel);
      lastExtInf = null;
    }
  }
  return channels;
}
