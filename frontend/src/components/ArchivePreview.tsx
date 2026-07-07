"use client";

/**
 * ArchivePreview — WinRAR-style file list shown after the user
 * picks/drops an archive but BEFORE clicking DECOMPRESS.
 *
 * Shows the file list (name + uncompressed size), the total
 * uncompressed size, the archive's own size on disk, the
 * archive kind, and the resulting ratio.
 */

export interface PreviewEntry {
  path: string;
  size: number;
  is_dir: boolean;
}

export interface Preview {
  archive_kind: string;
  n_files: number;
  total_uncompressed: number;
  compressed_size: number;
  files: PreviewEntry[];
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

const KIND_LABEL: Record<string, string> = {
  nxs6: "SOLID v6 (single LZMA stream)",
  nxar: "NXAR (lossless v4)",
  v4: "single-file v4",
  "v5-v6-single": "single-file v5/v6 (LZMA)",
};

export function ArchivePreview({ preview }: { preview: Preview | null }) {
  if (!preview) return null;

  const ratio =
    preview.compressed_size > 0
      ? preview.total_uncompressed / preview.compressed_size
      : 0;

  return (
    <div className="panel p-3 flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="metric-label">archive preview</span>
        <span className="text-[10px] font-mono text-zinc-500">
          {KIND_LABEL[preview.archive_kind] ?? preview.archive_kind}
        </span>
      </div>
      <div className="grid grid-cols-3 gap-3 font-mono text-[11px]">
        <div>
          <div className="metric-label">files</div>
          <div className="text-cyan-400">{preview.n_files}</div>
        </div>
        <div>
          <div className="metric-label">uncompressed</div>
          <div className="text-cyan-400">
            {fmtBytes(preview.total_uncompressed)}
          </div>
        </div>
        <div>
          <div className="metric-label">ratio</div>
          <div className="text-matrix-500">{ratio.toFixed(2)}×</div>
        </div>
      </div>

      {preview.files.length > 1 && (
        <div className="border border-zinc-800 max-h-48 overflow-y-auto font-mono text-[10px]">
          {preview.files.slice(0, 200).map((f, i) => (
            <div
              key={`${f.path}-${i}`}
              className="flex items-center justify-between px-2 py-1 border-b border-zinc-900 hover:bg-zinc-900/40"
            >
              <span className="text-zinc-300 truncate flex-1 mr-2" title={f.path}>
                <span className="text-cyan-500">▸</span> {f.path}
              </span>
              <span className="text-zinc-500 tabular-nums whitespace-nowrap">
                {fmtBytes(f.size)}
              </span>
            </div>
          ))}
          {preview.files.length > 200 && (
            <div className="px-2 py-1 text-zinc-600 italic">
              … and {preview.files.length - 200} more
            </div>
          )}
        </div>
      )}

      {preview.files.length === 1 && (
        <div className="font-mono text-[10px] text-zinc-400 border border-zinc-800 px-2 py-1.5 truncate">
          <span className="text-cyan-500">▸</span>{" "}
          <span className="text-zinc-300">{preview.files[0].path}</span>
          <span className="text-zinc-600 ml-2">
            ({fmtBytes(preview.files[0].size)})
          </span>
        </div>
      )}
    </div>
  );
}
