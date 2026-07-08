# Contributing

Thanks for your interest in NexusCompress! This guide covers how to set
up a development environment, build the engine + GUI, run the test
suite, and submit a pull request.

## Table of contents

- [Code of conduct](#code-of-conduct)
- [Project layout](#project-layout)
- [Prerequisites](#prerequisites)
  - [macOS](#macos)
  - [Linux (Debian / Ubuntu)](#linux-debian--ubuntu)
  - [Linux (Fedora / RHEL)](#linux-fedora--rhel)
  - [Windows](#windows)
- [Build from source](#build-from-source)
- [Run the test suite](#run-the-test-suite)
- [Open the GUI in dev mode](#open-the-gui-in-dev-mode)
- [Coding conventions](#coding-conventions)
- [Submitting changes](#submitting-changes)
- [Filing issues](#filing-issues)
- [File picker on macOS Sequoia](#file-picker-on-macos-sequoia)

---

## Code of conduct

This project follows the [Contributor Covenant](./CODE_OF_CONDUCT.md).
By participating you agree to its terms.

---

## Project layout

```
nexus-compress/
├── src/                      # pure-Rust compression engine (CLI + lib)
│   ├── main.rs               # CLI entrypoint (nexus c/d/bench)
│   ├── lz77_hash_chain.rs
│   ├── rANS_entropy.rs
│   ├── dict_codec.rs
│   ├── solid_archive.rs
│   └── chunked_io.rs
├── src-tauri/                # Tauri 2 GUI shell
│   ├── src/
│   │   ├── main.rs           # app entrypoint (nexus-rar binary)
│   │   ├── commands.rs       # 27 Tauri commands (invoke handlers)
│   │   ├── p2p_tunnel.rs     # P2P transport, SPAKE2, AES-GCM, UPnP
│   │   ├── p2p_auth.rs       # X-Nexus-Auth HMAC middleware
│   │   ├── p2p_config.rs     # token formats, TunnelConfig
│   │   ├── upnp_hole.rs      # UPnP port forwarding + cleanup
│   │   └── archive_inspect.rs # .tar / .nxs6 listing + extraction
│   ├── capabilities/
│   │   └── default.json      # dialog / fs permissions
│   ├── tauri.conf.json
│   ├── Cargo.toml
│   └── icons/
├── frontend/                 # Next.js + Tailwind UI
│   ├── src/
│   │   ├── app/              # Next.js app router
│   │   ├── components/       # NeoTerminal UI
│   │   └── lib/i18n.ts       # ES / EN / IT translations
│   ├── package.json
│   └── out/                  # build output (consumed by Tauri)
├── tests/                    # integration tests
├── corpus/                   # sample inputs for benchmarking
├── Cargo.toml                # engine crate
├── Cargo.lock
├── LICENSE                   # AGPL-3.0
├── COMMERCIAL-LICENSE.md     # commercial offering
├── README.md
├── ARCHITECTURE.md
├── CONTRIBUTING.md           # this file
├── SECURITY.md
└── CHANGELOG.md
```

---

## Prerequisites

You need:

- **Rust** 1.78 or newer (`rustup install stable`)
- **Node.js** 20.x or newer (use `nvm install 20` or download)
- **pnpm** or **npm** (the project ships with `package-lock.json` so
  `npm` works out of the box)
- Platform-specific C compiler + system libraries (see below)

### macOS

```bash
# Xcode Command Line Tools (clang, git, make, etc.)
xcode-select --install

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Node
brew install node@20   # or use nvm

# Optional but recommended
brew install cocoapods  # for some Tauri plugins
```

### Linux (Debian / Ubuntu)

```bash
# Build essentials + system libraries Tauri / WebKitGTK need
sudo apt install -y \
  build-essential curl wget file pkg-config libssl-dev \
  libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
  libjavascriptcoregtk-4.0-dev libwebkit2gtk-4.0-dev \
  libsoup2.4-dev libavahi-client3 libavahi-common3 \
  gnome-keyring libsecret-1-dev

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Node
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs
```

#### Fedora / RHEL

```bash
sudo dnf install -y \
  gcc gcc-c++ make pkgconfig openssl-devel \
  gtk3-devel libappindicator-gtk3-devel librsvg2-devel \
  webkit2gtk4.0-devel webkit2gtk3-devel \
  libsoup-devel avahi-devel libsecret-devel

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

sudo dnf install -y nodejs
```

### Windows

Install via [Microsoft's recommended path](https://learn.microsoft.com/en-us/windows/dev-environment/javascript/nodejs-on-windows):

```powershell
# Visual Studio Build Tools 2022 with:
#   - "Desktop development with C++" workload
#   - "Windows 10/11 SDK"
#   - "C++ CMake tools for Windows"
# Then:
winget install Rustlang.Rustup
winget install OpenJS.NodeJS.LTS
```

You'll also want:

- **WebView2** runtime (pre-installed on Windows 11; on 10, install
  Evergreen via
  https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
- **Bonjour Print Services** if you want mDNS to work on Windows 10
  (Windows 11 has mDNS built-in)

---

## Build from source

```bash
# 1. Clone
git clone https://github.com/bierfor/nexus-compress.git
cd nexus-compress

# 2. Engine only (CLI benchmarks, no GUI)
cargo build --release --bin nexus
./target/release/nexus bench corpus/text.txt  # sanity-check

# 3. GUI — frontend bundle, then Tauri shell
cd frontend
npm install           # ~30 seconds
npm run build         # outputs to out/
cd ..
cargo build --release --bin nexus-rar
./target/release/nexus-rar   # launches the GUI

# 4. Optional helper binaries
cargo build --release --bin p2p_smoke   # for unit + smoke tests
cargo build --release --bin e2e_v3      # for full e2e network tests
```

The frontend **must be built before** the Tauri shell on every change
or the GUI will use a stale Next.js bundle. CI rebuilds both.

### Build targets

| Binary | Purpose |
|--------|---------|
| `nexus` | Engine CLI: `nexus c <file> <out>`, `nexus d <in> <file>`, `nexus bench` |
| `nexus-rar` | Tauri 2 GUI (the desktop app) |
| `p2p_smoke` | Cargo test runner for the P2P transport |
| `e2e_v3` | Integration tests (sender ↔ receiver with UPnP) |

---

## Run the test suite

```bash
# Engine unit tests (no network, no GUI)
cargo test --release

# P2P transport + auth + archive tests
cargo build --release --bin p2p_smoke
cargo test --bin p2p_smoke --release
# Expected: 149 passed; 0 failed
```

The first command runs the engine in ~0.1 s. The second runs the P2P
test suite in ~1 s. `e2e_v3` requires `cargo run --bin e2e_v3 --release`
(no `cargo test` integration — it's a separate binary) and a real
network with UPnP on the sender.

---

## Open the GUI in dev mode

```bash
# Frontend dev server (Vite via Next.js) on http://localhost:1420
cd frontend && npm run tauri:dev
```

This is the fastest iteration loop: edit a React component, the page
hot-reloads via the dev server, and Tauri reloads the WebView.

For Rust changes, the dev server picks them up automatically (Tauri
re-runs `cargo build`).

---

## Coding conventions

### Rust

- `rustfmt` defaults — `cargo fmt` before commit
- `clippy --all-targets --all-features -- -D warnings` — no warnings allowed in CI
- Module structure: large features live in their own file under
  `src-tauri/src/`, anything < 200 lines inlines into the parent
- Hex / base64 / etc. via the `base64` crate's URL-safe-no-pad alphabet
  (no padding in transit — `=` is bad in tokens, filenames, query
  strings)
- All filesystem ops go through `tokio::fs` (async, non-blocking).
  Blocking `std::fs` only in `#[cfg(test)]` + the bin-side startup
  hooks

### TypeScript / React

- Functional components only. `useCallback` for every handler that
  becomes a dependency
- Tailwind utility classes, no CSS modules
- `useLocale()` for every translatable string
- All Tauri invokes go through a single `tauriInvoke<T>(cmd, args)`
  wrapper that shape-checks and surfaces errors cleanly
- No prop drilling past 2 levels — context or composition

### UX

- One screen, one primary action (see [ARCHITECTURE.md](./ARCHITECTURE.md#neo-terminal-ux))
- 8-pt spacing grid (`p-2 / p-4 / p-8 / p-12 / p-16`)
- Monospace only for technical numbers (sizes, hashes, paths)
- No emoji as a substitute for real text. Emoji in headings or as
  decorative icons is fine.

---

## Submitting changes

1. Fork the repo on GitHub
2. Create a branch: `git checkout -b fix/something-or-feature`
3. Make your change. If it's not trivial, open an issue first to discuss
4. Run tests: `cargo test --release && cargo test --bin p2p_smoke --release`
5. Run lints: `cargo fmt && cargo clippy --all-targets --all-features -- -D warnings`
6. If you touched the frontend: `cd frontend && npm run build`
7. Commit with a [Conventional Commits](https://www.conventionalcommits.org/)
   prefix: `fix:`, `feat:`, `chore:`, `docs:`, `refactor:`, `test:`
8. Push and open a PR. The CI will run:
   - Engine + P2P tests on Linux, macOS, Windows
   - `cargo fmt --check`
   - `cargo clippy -- -D warnings`
   - Frontend build + Next.js static export
9. Wait for a review from a maintainer. Be patient — we read every PR,
   but Sprint work is structured and we may need to coordinate around
   upcoming changes.

If you're shipping a fix for a security issue, please **don't** open a
public PR. Follow [SECURITY.md](./SECURITY.md) instead.

---

## Filing issues

Use [GitHub Issues](https://github.com/bierfor/nexus-compress/issues).
Please include:

- OS + version (e.g. "macOS 15.4 (Sequoia)", "Ubuntu 24.04 LTS",
  "Windows 11 23H2")
- App version (top-right of the landing page, e.g. "v0.1.0")
- What you expected vs what happened
- For crashes: full stderr from `cargo run --release --bin nexus-rar`
- For P2P / transfer issues: the format of the token (`nx:2:...` or
  `nx:3:...`) and whether you're on the same LAN, different LANs, or
  one endpoint behind symmetric NAT

---

## File picker on macOS Sequoia

`tauri-plugin-dialog` v2.7 has flaky behaviour on macOS Sequoia when
combined with `titleBarStyle: "Overlay"` frameless windows: the picker
opens but selection doesn't register.

We work around this in three places:

1. **Drag-and-drop** onto the panel (uses Tauri's internal
   `tauri://drag-drop` event — no OS picker involved)
2. **Paste-the-path**: in Finder, right-click → hold `Option` →
   "Copy ... as Pathname" (Cmd+Opt+C) → Cmd+V into the input
3. **Native picker** as a tertiary fallback (works on Linux, Windows,
   older macOS)

If you report a bug related to file picking on macOS Sequoia, please
try the workaround first and let us know which of the three paths you
used. It tells us whether the issue is in the picker code or somewhere
deeper.
