// Sprint 5.7.21-B-FileIcons-Shared: shared file-icon helpers.
//
// Extracted from LandingPage.tsx in the Sprint 5.7.21-B-Home-Icons
// refactor so every view that renders file rows can use the same
// mapping. The pattern: each row gets a file-type icon (FileCode /
// FileImage / FileVideo / etc.) based on the filename extension,
// plus a small "kind" badge in the corner (Archive for compress,
// FolderOpen for decompress, Send for share).
//
// Use `getFileKind(filename)` to get the icon + colors, and
// `getKindBadge(kind)` to get the corner badge. Compose them
// in your row component.
//
// Special handling for compound extensions: .tar.gz, .tar.bz2,
// .tar.xz resolve to the archive icon.

import {
  Archive,
  FileText,
  FileCode,
  FileImage,
  FileVideo,
  FileAudio,
  FileArchive,
  FileSpreadsheet,
  FileJson,
  FolderOpen,
  Send,
  type LucideIcon,
} from "lucide-react";

export type FileKind = {
  icon: LucideIcon;
  color: string;
  bg: string;
  border: string;
};

const FILE_ICON_MAP: Record<string, FileKind> = {
  // Code — cyan (matches the "compress" code source use case)
  js: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  jsx: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  ts: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  tsx: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  py: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  rs: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  go: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  java: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  rb: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  php: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  swift: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  kt: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  sh: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  html: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  css: { icon: FileCode, color: "text-cyan-300", bg: "bg-cyan-500/10", border: "border-cyan-500/20" },
  // Images — violet
  jpg: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  jpeg: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  png: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  gif: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  svg: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  webp: { icon: FileImage, color: "text-violet-300", bg: "bg-violet-500/10", border: "border-violet-500/20" },
  // Video — rose
  mp4: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  mov: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  avi: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  mkv: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  webm: { icon: FileVideo, color: "text-rose-300", bg: "bg-rose-500/10", border: "border-rose-500/20" },
  // Audio — amber
  mp3: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  wav: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  flac: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  aac: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  ogg: { icon: FileAudio, color: "text-amber-300", bg: "bg-amber-500/10", border: "border-amber-500/20" },
  // Archives — emerald (matches the share/archive theme)
  zip: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  tar: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  gz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  tgz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  bz2: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  xz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  "7z": { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  rar: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxs: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxs6: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxe: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  nxr: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  lz: { icon: FileArchive, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  // Spreadsheets — emerald
  xls: { icon: FileSpreadsheet, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  xlsx: { icon: FileSpreadsheet, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  csv: { icon: FileSpreadsheet, color: "text-emerald-300", bg: "bg-emerald-500/10", border: "border-emerald-500/20" },
  // JSON — yellow
  json: { icon: FileJson, color: "text-yellow-300", bg: "bg-yellow-500/10", border: "border-yellow-500/20" },
  // Documents — zinc (neutral)
  pdf: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  doc: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  docx: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  txt: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
  md: { icon: FileText, color: "text-zinc-300", bg: "bg-white/[0.04]", border: "border-white/[0.08]" },
};

export const DEFAULT_FILE_KIND: FileKind = {
  icon: FileText,
  color: "text-zinc-400",
  bg: "bg-white/[0.04]",
  border: "border-white/[0.08]",
};

/**
 * Get the file-type icon (and accent color) for a filename.
 * The mapping is intentionally broad (every common file type
 * covered) and falls back to a generic FileText for unknown
 * extensions. Compound extensions like .tar.gz, .tar.bz2, .tar.xz
 * resolve to the archive icon.
 */
export function getFileKind(filename: string): FileKind {
  if (!filename) return DEFAULT_FILE_KIND;
  const base = filename.split("/").pop() ?? filename;
  const lower = base.toLowerCase();
  // Compound extensions first (otherwise the .gz / .bz2 / .xz
  // suffix would be picked and we'd get a generic archive
  // icon instead of the tar-archive icon).
  for (const compound of ["tar.gz", "tar.bz2", "tar.xz"]) {
    if (lower.endsWith(`.${compound}`)) {
      return FILE_ICON_MAP[compound] ?? DEFAULT_FILE_KIND;
    }
  }
  const ext = base.split(".").pop()?.toLowerCase() ?? "";
  return FILE_ICON_MAP[ext] ?? DEFAULT_FILE_KIND;
}

export type KindBadge = {
  icon: LucideIcon;
  color: string;
  bg: string;
};

/**
 * Get the "kind" badge for a recent activity row. The badge
 * is rendered as a small 16x16 dot in the bottom-right corner
 * of the file icon, with a 1px border matching the page
 * background so it reads as a separate 'status' element.
 */
export function getKindBadge(
  kind: "compress" | "decompress" | "share"
): KindBadge {
  if (kind === "compress") {
    return { icon: Archive, color: "text-cyan-300", bg: "bg-cyan-500/20" };
  }
  if (kind === "decompress") {
    return { icon: FolderOpen, color: "text-amber-300", bg: "bg-amber-500/20" };
  }
  return { icon: Send, color: "text-emerald-300", bg: "bg-emerald-500/20" };
}
