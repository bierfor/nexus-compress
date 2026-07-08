"use client";

/**
 * ArchivePreview — WinRAR-style file list shown after the user
 * picks/drops an archive but BEFORE clicking DECOMPRESS.
 *
 * Shows the file list (name + uncompressed size), the total
 * uncompressed size, the archive's own size on disk, the
 * archive kind, and the resulting ratio.
 *
 * Sprint 5.7.2: also handles the encrypted case (NXE\0 / NXR\0
 * magic). For encrypted archives the file list is INSIDE the
 * ciphertext, so without the password we can only show the
 * outer envelope stats + a clear "password required" banner +
 * a password input that re-issues the peek with the key.
 */

import { useState } from "react";

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
  "nxe-encrypted": "NXE — encrypted (no recovery)",
  "nxr-encrypted": "NXR — encrypted + Reed-Solomon recovery",
};

interface ArchivePreviewProps {
  preview: Preview | null;
  /** Optional callback to re-peek with a password (encrypted archives). */
  onUnlock?: (password: string) => void;
  /** Whether the unlock attempt is in flight. */
  unlocking?: boolean;
  /** Last unlock error to display. */
  unlockError?: string | null;
}

export function ArchivePreview({
  preview,
  onUnlock,
  unlocking,
  unlockError,
}: ArchivePreviewProps) {
  if (!preview) return null;

  const ratio =
    preview.compressed_size > 0
      ? preview.total_uncompressed / preview.compressed_size
      : 0;

  // Encrypted archives: lock banner + password input. The file
  // list area is replaced with a "locked" placeholder.
  const isEncrypted =
    preview.archive_kind === "nxe-encrypted" ||
    preview.archive_kind === "nxr-encrypted";

  return (
    <div className="panel p-3 flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="metric-label">archive preview</span>
        <span
          className={`text-[10px] font-mono ${
            isEncrypted ? "text-amber-400" : "text-zinc-500"
          }`}
        >
          {isEncrypted && <span className="mr-1">🔒</span>}
          {KIND_LABEL[preview.archive_kind] ?? preview.archive_kind}
        </span>
      </div>

      <div className="grid grid-cols-3 gap-3 font-mono text-[11px]">
        <div>
          <div className="metric-label">files</div>
          <div className="text-cyan-400">
            {isEncrypted ? "🔒 locked" : preview.n_files}
          </div>
        </div>
        <div>
          <div className="metric-label">on disk</div>
          <div className="text-cyan-400">
            {fmtBytes(preview.compressed_size)}
          </div>
        </div>
        <div>
          <div className="metric-label">format</div>
          <div className="text-amber-400 text-[10px]">
            {isEncrypted
              ? preview.archive_kind === "nxr-encrypted"
                ? "AES + RS"
                : "AES only"
              : "plain"}
          </div>
        </div>
      </div>

      {isEncrypted ? (
        <EncryptedPreview
          preview={preview}
          onUnlock={onUnlock}
          unlocking={unlocking}
          unlockError={unlockError}
        />
      ) : (
        <PlainPreview preview={preview} />
      )}
    </div>
  );
}

function PlainPreview({ preview }: { preview: Preview }) {
  if (preview.files.length > 1) {
    return (
      <div className="border border-zinc-800 max-h-48 overflow-y-auto font-mono text-[10px]">
        {preview.files.slice(0, 200).map((f, i) => (
          <div
            key={`${f.path}-${i}`}
            className="flex items-center justify-between px-2 py-1 border-b border-zinc-900 hover:bg-zinc-900/40"
          >
            <span
              className="text-zinc-300 truncate flex-1 mr-2"
              title={f.path}
            >
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
    );
  }

  if (preview.files.length === 1) {
    return (
      <div className="font-mono text-[10px] text-zinc-400 border border-zinc-800 px-2 py-1.5 truncate">
        <span className="text-cyan-500">▸</span>{" "}
        <span className="text-zinc-300">{preview.files[0].path}</span>
        <span className="text-zinc-600 ml-2">
          ({fmtBytes(preview.files[0].size)})
        </span>
      </div>
    );
  }

  return null;
}

/**
 * EncryptedPreview — the password-prompt panel for NXE\0 / NXR\0
 * archives. Shows the lock banner, a password input with
 * visibility toggle, and the last unlock error. Calls
 * `onUnlock(password)` when the user submits.
 *
 * Sits inline inside the main ArchivePreview panel rather than
 * a modal — the user already has the file picked, and the only
 * missing piece of information to commit the extraction is the
 * password. Inline is the right ergonomic.
 */
function EncryptedPreview({
  preview,
  onUnlock,
  unlocking,
  unlockError,
}: {
  preview: Preview;
  onUnlock?: (password: string) => void;
  unlocking?: boolean;
  unlockError?: string | null;
}) {
  const [password, setPassword] = useState("");
  const [showPwd, setShowPwd] = useState(false);

  const submit = (e?: React.FormEvent) => {
    e?.preventDefault();
    if (!password || unlocking) return;
    onUnlock?.(password);
  };

  return (
    <div className="flex flex-col gap-2">
      {/* Lock banner — explains why we can't list files yet. */}
      <div className="border border-amber-500/30 bg-amber-500/[0.04] rounded px-3 py-2 text-[11px]">
        <div className="flex items-center gap-1.5 text-amber-300 mb-1">
          <span>🔒</span>
          <span className="font-medium">file list is encrypted</span>
        </div>
        <div className="text-zinc-400 text-[10.5px] leading-relaxed">
          This archive is {fmtBytes(preview.compressed_size)} on disk and
          uses AES-256-GCM with Argon2id key derivation. Enter the
          password below to list the contained files before
          extracting.
          {preview.archive_kind === "nxr-encrypted" && (
            <>
              {" "}
              <span className="text-cyan-400">
                Reed-Solomon recovery is enabled
              </span>{" "}
              — corrupted blocks can be reconstructed automatically.
            </>
          )}
        </div>
      </div>

      {/* Password input + unlock button. */}
      <form
        onSubmit={submit}
        className="flex items-center gap-2"
      >
        <div className="relative flex-1">
          <input
            type={showPwd ? "text" : "password"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="archive password"
            autoComplete="off"
            spellCheck={false}
            className="w-full bg-zinc-900/60 border border-zinc-700 rounded px-3 py-1.5 pr-9 text-[12px] font-mono text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-500 focus:bg-zinc-900"
          />
          <button
            type="button"
            onClick={() => setShowPwd((s) => !s)}
            tabIndex={-1}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-cyan-400 text-[14px] leading-none"
            title={showPwd ? "hide password" : "show password"}
          >
            {showPwd ? "🙈" : "👁"}
          </button>
        </div>
        <button
          type="submit"
          disabled={!password || unlocking}
          className="px-3 py-1.5 bg-cyan-600 hover:bg-cyan-500 disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-[11px] font-medium rounded transition-colors"
        >
          {unlocking ? "unlocking…" : "unlock list"}
        </button>
      </form>

      {unlockError && (
        <div className="text-rose-400 text-[10.5px] font-mono">
          ✗ {unlockError}
        </div>
      )}

      {/* Tiny format-on-disk stat row (compact metadata, always visible). */}
      <div className="font-mono text-[10px] text-zinc-500 flex items-center justify-between">
        <span>on disk</span>
        <span className="text-zinc-300">
          {fmtBytes(preview.compressed_size)}
        </span>
      </div>
    </div>
  );
}
