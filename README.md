# NexusCompress

> **Hybrid file compressor + encrypted P2P transfer, all in one desktop app.**
> Native Tauri 2 shell (Rust) with a Next.js UI on macOS, Linux and Windows.

[![License: AGPL-3.0 + Commercial](https://img.shields.io/badge/license-AGPL--3.0%20%2B%20Commercial-blue.svg)](./LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#installation)
[![CI](https://github.com/bierfor/nexus-compress/actions/workflows/ci.yml/badge.svg)](https://github.com/bierfor/nexus-compress/actions)
[![Tests](https://img.shields.io/badge/tests-149%20passing-brightgreen.svg)](#development)

[English](#english) · [Español](#español) · [Italiano](#italiano)

---

<a id="english"></a>

## 🇬🇧 English

### What is NexusCompress?

NexusCompress is two things in one native desktop app:

1. **A hybrid file compressor.** `LZ77` hash-chain matching + lazy parsing,
   5-stream `rANS` entropy coding, content-defined chunking and block-level
   dictionary hints. Self-extracting `.nxs6` solid archives for the cases
   where ratio matters more than speed; streaming `.tar` wrapping otherwise.
   Detailed codec notes in [`ARCHITECTURE.md`](./ARCHITECTURE.md).

2. **An encrypted, peer-to-peer file transfer tool.** `SPAKE2` password-authenticated
   key exchange stretched by `Argon2id` (32-bit codes → 256-bit effective entropy),
   `AES-256-GCM` chunked AEAD, HMAC-bound request signing, three transport modes:
   - **Direct LAN** (mDNS discovery + optional UPnP for cross-NAT)
   - **Direct cross-NAT** (UPnP hole-punching + localhost fallback for the
     same-network case where the public IP isn't hairpin-reachable)
   - **Cloudflare Quick / Named Tunnel** (fallback for symmetric NATs)

No accounts. No telemetry. No tracking. The relay Cloudflare provides
is optional — both endpoints need to know the 4-word code.

### Why?

Existing tools solve one half of the problem:

- File compressors (zstd, 7z) don't transfer over the network.
- File-transfer tools (Syncthing, Magic Wormhole) don't compress content.

NexusCompress does both, with **end-to-end encryption** and **no
intermediary servers** by default.

### When to use it

- Sending a large build artifact to a colleague on another continent
- Streaming a 4K video file from a recording setup to an editor in real time
- Quickly inspecting a `.tar` or `.nxs6` archive without unpacking it to disk
  (WinRAR-style central-directory view)
- Compressing large media repositories locally with a tuned hybrid codec

### When NOT to use it

- You need ≥ zstd-19 ratios on arbitrary data — use `zstd` or `7z` instead
- You need > 2 GB/s throughput on a single thread — use `lz4`
- You want SaaS-mode sharing (Google Drive / Dropbox) — this is direct P2P

### License

Dual-licensed. See [`LICENSE`](./LICENSE) and [`COMMERCIAL-LICENSE.md`](./COMMERCIAL-LICENSE.md).

- **Open source** under AGPL-3.0 — for OSS projects, personal use, and
  AGPL-compatible commercial deployments
- **Commercial license** — for organisations that want to embed NexusCompress
  in proprietary products without AGPL's source-disclosure obligations

Pricing is company-size based (not per-seat). Indie / startup tier is free.

### Installation

Download a release for your platform from the
[Releases page](https://github.com/bierfor/nexus-compress/releases):

- macOS (universal: Apple Silicon + Intel, signed + notarised)
- Linux (`.deb`, `.rpm`, `.AppImage`)
- Windows (`x86_64` MSI installer + portable ZIP)

Or build from source (see [CONTRIBUTING.md](./CONTRIBUTING.md#build-from-source)).

### Quick tour

After launching `nexus-rar`:

- **Comprimir** — drop a file (or folder) → choose a mode (FAST / BALANCED / ULTRA) → enter.
- **Descomprimir** — drop an archive → tick the entries you want → extract.
- **Compartir** —
  - **Enviar** → drop a file → "Crear enlace" → share the 4-word code
    + token. Receiver pastes them into **Recibir**.
  - **Recibir** → paste the token → "Recibir archivo".
- **Ajustes** — language (ES / EN / IT), compression defaults, tunnel mode.

#### File picking on macOS Sequoia

The native file picker in `tauri-plugin-dialog` has known issues on
macOS Sequoia (dialog opens but selection doesn't register with
`titleBarStyle: "Overlay"` frameless windows). All three input panels
support **drag-and-drop**, **paste-the-path** (`Cmd+Opt+C` in Finder
copies pathname → `Cmd+V` into the input), and the picker as a tertiary
fallback. See the in-app hint or [TAURI-PITFALLS](https://github.com/bierfor/nexus-compress/blob/master/CONTRIBUTING.md#file-picker-on-macos-sequoia).

### Development

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for:

- Building from source on macOS, Linux, Windows
- Running the 149-test suite
- Adding a new transport mode, codec, or UI screen
- The coding conventions we follow (Neo Terminal UX)

### Security

NexusCompress prioritises:

- **End-to-end encryption** — AES-256-GCM with per-transfer session keys
- **No plaintext telemetry** — the relay Cloudflare tunnel sees ciphertext only
- **Authenticated key exchange** — SPAKE2 + Argon2id for low-entropy codes
- **HMAC request signing** — `X-Nexus-Auth` on every privileged endpoint
- **Path-traversal sanitisation** at the trust boundary (Sprint 5.6.26)

Report vulnerabilities per [`SECURITY.md`](./SECURITY.md).

---

<a id="español"></a>

## 🇪🇸 Español

### ¿Qué es NexusCompress?

NexusCompress son dos cosas en una app desktop nativa:

1. **Un compresor de archivos híbrido.** Matching LZ77 con hash chains + lazy
   parsing, codificación entrópica `rANS` de 5 streams, chunking definido por
   contenido y diccionario a nivel de bloque. Archivos sólidos `.nxs6` para
   cuando la ratio importa más que la velocidad; wrapping `.tar` en streaming
   cuando no. Detalles del codec en [`ARCHITECTURE.md`](./ARCHITECTURE.md).

2. **Transferencia de archivos cifrada punto a punto.** Intercambio de claves
   autenticado por contraseña `SPAKE2` reforzado con `Argon2id` (códigos de 32 bits
   → entropía efectiva de 256 bits), AEAD chunked `AES-256-GCM`, firma de
   requests con HMAC, tres modos de transporte:
   - **LAN directa** (descubrimiento mDNS + UPnP opcional para cruzar NAT)
   - **Directa cross-NAT** (UPnP hole-punching + fallback a localhost cuando
     la red bloquea hairpin al IP público)
   - **Túnel Cloudflare Quick / Named** (fallback para NATs simétricos)

Sin cuentas. Sin telemetría. El relay de Cloudflare es opcional — los dos
extremos necesitan saber el código de 4 palabras.

### Licencia

Doble licencia. Ver [`LICENSE`](./LICENSE) y [`COMMERCIAL-LICENSE.md`](./COMMERCIAL-LICENSE.md).

- **Open source** bajo AGPL-3.0 — para proyectos OSS, uso personal y
  despliegues comerciales AGPL-compatibles
- **Licencia comercial** — para empresas que quieren embeber NexusCompress
  en productos privativos sin las obligaciones de source-disclosure de AGPL

Precios según tamaño de empresa (no por asiento). El tier Indie/startup es gratuito.

### Instalación

Descargá un release para tu plataforma desde la
[página de Releases](https://github.com/bierfor/nexus-compress/releases):

- macOS (universal: Apple Silicon + Intel, signed + notarised)
- Linux (`.deb`, `.rpm`, `.AppImage`)
- Windows (instalador MSI `x86_64` + ZIP portable)

O compilá desde el código fuente (ver [CONTRIBUTING.md](./CONTRIBUTING.md#build-from-source)).

### Recorrido rápido

- **Comprimir** — soltá un archivo (o carpeta) → elegí un modo (FAST / BALANCED / ULTRA) → enter.
- **Descomprimir** — soltá un archivo → tildá las entries que querés → extraer.
- **Compartir** —
  - **Enviar** → soltá un archivo → "Crear enlace" → compartí el código de 4
    palabras + token. El receptor lo pega en **Recibir**.
  - **Recibir** → pegá el token → "Recibir archivo".
- **Ajustes** — idioma (ES / EN / IT), compresión por defecto, modo de túnel.

---

<a id="italiano"></a>

## 🇮🇹 Italiano

### Cos'è NexusCompress?

NexusCompress è due cose in un'unica app desktop nativa:

1. **Un compressore di file ibrido.** Matching LZ77 con hash chain + lazy
   parsing, codifica entropica `rANS` a 5 stream, chunking definito dal
   contenuto e dizionario a livello di blocco. Archivi solidi `.nxs6` per
   quando il rapporto di compressione conta più della velocità; wrapping
   `.tar` in streaming altrimenti. Dettagli del codec in
   [`ARCHITECTURE.md`](./ARCHITECTURE.md).

2. **Trasferimento di file cifrato peer-to-peer.** Scambio di chiavi
   autenticato da password `SPAKE2` rafforzato con `Argon2id` (codici da 32 bit
   → entropia effettiva di 256 bit), AEAD chunked `AES-256-GCM`, firma delle
   richieste con HMAC, tre modalità di trasporto:
   - **LAN diretta** (discovery mDNS + UPnP opzionale per attraversare il NAT)
   - **Diretta cross-NAT** (UPnP hole-punching + fallback localhost quando la
     rete blocca l'hairpin all'IP pubblico)
   - **Tunnel Cloudflare Quick / Named** (fallback per NAT simmetrici)

Senza account. Senza telemetria. Il relay Cloudflare è opzionale — entrambi
gli endpoint devono conoscere il codice di 4 parole.

### Licenza

Doppia licenza. Vedi [`LICENSE`](./LICENSE) e [`COMMERCIAL-LICENSE.md`](./COMMERCIAL-LICENSE.md).

- **Open source** sotto AGPL-3.0 — per progetti OSS, uso personale e
  distribuzioni commerciali AGPL-compatibili
- **Licenza commerciale** — per organizzazioni che vogliono incorporare
  NexusCompress in prodotti proprietari senza gli obblighi di source-disclosure
  dell'AGPL

I prezzi sono basati sulla dimensione dell'azienda (non per posto). Il tier
Indie/startup è gratuito.

### Installazione

Scarica una release per la tua piattaforma dalla
[pagina Releases](https://github.com/bierfor/nexus-compress/releases):

- macOS (universale: Apple Silicon + Intel, firmato e notarizzato)
- Linux (`.deb`, `.rpm`, `.AppImage`)
- Windows (installer MSI `x86_64` + ZIP portatile)

Oppure compila dal codice sorgente (vedi
[CONTRIBUTING.md](./CONTRIBUTING.md#build-from-source)).

### Tour rapido

- **Comprimi** — trascina un file (o cartella) → scegli una modalità (FAST / BALANCED / ULTRA) → invio.
- **Decomprimi** — trascina un archivio → spunta le voci che vuoi → estrai.
- **Condividi** —
  - **Invia** → trascina un file → "Crea collegamento" → condividi il codice
    di 4 parole + token. Il destinatario lo incolla in **Ricevi**.
  - **Ricevi** → incolla il token → "Ricevi file".
- **Impostazioni** — lingua (ES / EN / IT), compressione predefinita, modalità tunnel.

---

## Acknowledgements

Built on:

- [`lspack`](https://crates.io/crates/lspack) / [`rans`](https://crates.io/crates/rans)
  — rANS entropy coding
- [`xz2`](https://crates.io/crates/xz2) — LZMA via liblzma
- [`aes-gcm`](https://crates.io/crates/aes-gcm) — authenticated encryption
- [`spake2`](https://crates.io/crates/spake2) — password-authenticated key exchange
- [`argon2`](https://crates.io/crates/argon2) — key-stretching
- [`mdns-sd`](https://crates.io/crates/mdns-sd) — LAN discovery
- [`igd`](https://crates.io/crates/igd) — UPnP port forwarding
- [`keyring`](https://crates.io/crates/keyring) — OS credential store
- [`axum`](https://crates.io/crates/axum) — local HTTP server
- [`tauri`](https://tauri.app) — native shell
- [Next.js](https://nextjs.org) / [React](https://react.dev) — UI framework
- [Tailwind CSS](https://tailwindcss.com) — styling

Thanks to the maintainers of all dependencies, and to everyone who files
issues, sends PRs, or spreads the word.

---

**Made with Rust + Next.js + a lot of ☕ in Rome.**
