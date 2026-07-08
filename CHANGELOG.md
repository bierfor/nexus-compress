# Changelog

All notable changes to NexusCompress are documented here. Versions
follow [Semantic Versioning](https://semver.org/).

> Format adapted from [Keep a Changelog](https://keepachangelog.com/).
> Each Sprint commit is identified in the Git log by a 7-char short
> hash.

---

## [Unreleased]

### Added
- **Dual licensing** — AGPL-3.0 for OSS, Commercial License for closed-
  source embedding. See [LICENSE](./LICENSE) and
  [COMMERCIAL-LICENSE.md](./COMMERCIAL-LICENSE.md).
- **Cross-platform release pipeline** — GitHub Actions builds
  `nexus-rar` on macOS, Linux, and Windows; uploads artifacts.
- **CI test matrix** — engine + P2P tests run on every PR across all
  three platforms.
- Public documentation set: `README.md` (EN + ES + IT), `ARCHITECTURE.md`,
  `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`.

### Fixed
- **macOS Sequoia file picker** was flaky with frameless windows
  (Sprint 5.6.27). All three input panels now have a visible
  drag-and-drop zone + a primary text-path input.

---

## Sprint 5.6.27

`ba7e39e` — File picking reliable on macOS Sequoia.

- Drag-and-drop zone + text-path input promoted to primary
  actions in CompressView and ShareView.
- Picker buttons demoted to tertiary fallback.
- i18n keys added for the workaround hints (ES/EN/IT).

## Sprint 5.6.26

`47e3ed1` — Three hardening fixes.

- **Path traversal via malicious filename**: new `safe_basename()`
  helper applied to both v2/v3 `/meta` and v1 token parsing.
  8 unit tests added; rejects path separators, `..`, null bytes,
  Windows reserved names.
- **Race condition**: `probing: boolean` state disables the
  "Cambiar" button while the filename probe is in flight. Subtle
  spinner shown.
- **Reset trigger cleanup**: `userEditedPath` reset moved to its
  own `useEffect([token])` so it's purely tied to user actions.

## Sprint 5.6.25

`171b799` — Eliminate the placeholder concept entirely.

- `outputPath` is now ALWAYS the resolved final path. No sentinel,
  no placeholder detection.
- Auto-sync `<resolvedDownloads>/<suggestedName>` whenever
  suggestedName becomes known (v1 parse + v2/v3 probe).
- `userEditedPath` flag prevents auto-sync from clobbering the
  user's chosen destination.

## Sprint 5.6.24

`d8c0646` — Catch-all `.bin` placeholder detection + UI sync.

## Sprint 5.6.23

`a9d5d2a` — Never substitute a `.bin` extension. Path input
sanitisation, xattr strip verbose logging.

## Sprint 5.6.22

`80392a9` — Paginated central directory for archive browsing. 500
entries per page, infinite scroll on the frontend, WinRAR-speed
listing even on multi-GB archives.

## Sprint 5.6.21

`7dc6678` — Italian translation (152 keys). Settings UX redesign
with tunnel mode cards + About section.

## Sprint 5.6.20

`d7587a9` — `.nxs6` solid archives also use the TOC-at-front
listing pattern, with extract falling back to a single LZMA pass
+ slice-by-offset.

## Sprint 5.6.19 — 5.6.17

Archive inspection module: `tar::Archive::entries()` for tar
streaming, `nexus_compress::solid_archive::parse_toc` for .nxs6.
WinRAR-style UI with checkboxes + bulk Todos/Ninguno.

## Sprint 5.6.16 — 5.6.5

Folder sharing via system `/usr/bin/tar` (universal, Finder-
extractable). Three mDNS bugs fixed + localhost fallback +
quarantine strip. `X-Nexus-Auth` pre-auth HMAC. `/done` endpoint +
hard-timeout watcher. Multi-chunk ChunkCipher counter increment.

---

For the full history prior to Sprint 5.5.0 (where semantic versioning
started in earnest), see `git log --oneline`.
