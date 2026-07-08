# Security

> Safety claims, threat model, and how to report a vulnerability to
> the NexusCompress team.

We take security seriously. **SPAKE2 + Argon2id + AES-256-GCM** is a
real cryptographic protocol stack, not a marketing checklist, and we
want it audited by the community.

## TL;DR

- **End-to-end encryption.** AES-256-GCM, session keys derived via
  SPAKE2 + Argon2id + HKDF, never persisted to disk.
- **No plaintext telemetry.** The optional Cloudflare relay sees only
  ciphertext.
- **Authenticated requests.** Every privileged endpoint is HMAC-signed
  per-request, with rate-limiting BEFORE the HMAC check.
- **Sanitised filenames.** The filename carried in transit is filtered
  through `safe_basename()` at the trust boundary — path traversal
  (`../../etc/passwd`) and Windows reserved names (`CON`, `NUL`,
  `COM1`, ...) can't reach the filesystem.

## Safety claims (what we're willing to put in writing)

These are the cryptographic properties this release provides:

1. **Confidentiality against passive eavesdroppers.** A network observer
   who records the entire TCP stream (including the SPAKE2 exchange,
   the HMAC headers, and the file chunks) sees only ciphertext. They
   cannot recover the file without the 4-word code.

2. **Confidentiality against active attackers on the SPAKE2 exchange,
   in Direct Mode (LAN / cross-NAT with pre-shared key).** No MITM
   can complete the SPAKE2 exchange without knowing the code. (In
   Cloudflare mode the relay sees the obfuscated pre-auth HMAC
   header — see below.)

3. **Integrity and authentication of every chunk.** Any tamper —
   flipping a bit, replaying a chunk, reordering chunks — fails the
   AES-GCM tag check on the receiver side and terminates the
   transfer. The SHA-256 hash at the end verifies the entire
   plaintext matches.

4. **Forward secrecy within a session.** Compromising the Argon2id
   output after a transfer completes does not let you decrypt that
   transfer — the session keys were already deleted.

We do **NOT** claim:

- **Post-compromise secrecy.** If an attacker compromises an endpoint
  during a transfer, they can read the current transfer. Future
  transfers are still safe.
- **Deniability.** The cryptographic record (HMAC headers, etc.) lets
  a network observer prove that a transfer happened between two
  parties. Don't use this for whistleblowing.
- **Anonymity.** The optional v3 token includes the sender's public
  IP. The CLI auto-hides it in the UI after 5s (Sprint 5.4.3) but
  the user can capture it from the network traffic.

## Threat model

### What an attacker CAN do (assumed)

- Read any ciphertext that traverses the relay (Cloudflare's network)
- Send packets to either endpoint (TCP / mDNS / UPnP)
- Add, drop, or replay packets in transit
- Delay packets (jitter)
- Brute-force low-entropy inputs (e.g. an obvious passphrase)

### What an attacker CANNOT do (goals)

- Recover the plaintext without knowing the 4-word code
- Forge a valid HMAC header without the code
- Impersonate either endpoint during the SPAKE2 exchange
- Survive the SHA-256 final hash check
- Make the receiver accept a chunk from a different transfer
  (counter + nonce + key binding prevents this)

### What we TRUST (sometimes called "out of scope")

- The user's own machine. If their machine is rooted, an attacker
  has full access regardless of crypto.
- The 4-word code. We assume 32 bits of entropy in the code, which
  Argon2id amplifies to ~256 effective bits. A non-obvious code is
  a prerequisite for confidentiality against offline brute-force.

## Cryptographic stack (in detail)

| Layer | Algorithm | Source |
|-------|-----------|--------|
| Passphrase stretching | Argon2id, m=64MiB, t=3, p=1 | `argon2 = "0.5"` |
| Password-authenticated key exchange | SPAKE2 over Ed25519 group | `spake2 = "0.8"` |
| Key derivation from shared secret | HKDF-SHA-256 | `hkdf = "0.12"` |
| Symmetric encryption | AES-256-GCM, per-chunk counter | `aes-gcm = "0.10"` |
| Request MAC | HMAC-SHA-256, key derived from KEK | `hmac = "0.12"` |
| Hash verification (E2E) | SHA-256 | `sha2 = "0.10"` |

### Pre-auth HMAC (`X-Nexus-Auth`)

Every privileged endpoint requires `X-Nexus-Auth: <ts>|<hmac>` where:

```
key = HKDF(pre_key, salt=fhash, info="nexus-auth-v1")
mac = HMAC-SHA-256(key, "<ts>|<method>|<path>")
```

`pre_key` comes from Argon2id(passphrase, salt). Middleware:

1. **Rate-limit** by source IP — 5 attempts / 60 s window per IP.
   Reject with uniform 401 if exceeded.
2. **Verify HMAC** — uniform 401 if signature mismatch OR timestamp
   drift > 30 s. Never distinguishes between failure modes to an
   attacker (anti-fingerprinting).

### Session HMAC (`X-P2P-Auth`)

After SPAKE2 completes, both sides have a `session_mac_key`. The
`/file` handler verifies request-level integrity with this key (in
addition to the AES-GCM tag on every chunk — defense in depth).

### Path-traversal sanitisation

`safe_basename()` in `src-tauri/src/p2p_tunnel.rs` is the boundary:

- Rejects any path separator (`/`, `\`)
- Rejects `..` and `.` as complete components
- Rejects null bytes and other control characters
- Rejects Windows reserved names (`CON`, `PRN`, `AUX`, `NUL`,
  `COM1`..`COM9`, `LPT1`..`LPT9`)
- Caps length at 255 bytes

The unit tests in `p2p_tunnel::tests::safe_basename_*` cover all
branches (8 tests, all passing).

## Things users should know (not crypto, but actual safety)

- **Don't paste your token into a public chat or stream.** The token
  is the only credential — anyone with it can read the in-flight
  transfer (until it completes).
- **Pick non-obvious codes.** A 4-word phrase like "alpha-bear-cosmic-delta"
  is hard to brute-force. "test-test-test-test" is not.
- **macOS Gatekeeper quarantine is stripped** automatically
  (Sprint 5.6.14, hardened 5.6.23). The file opens without the
  "downloaded from the internet" warning. This is by design — the
  file came from a known peer, not the open internet.

## Reporting a vulnerability

**Please don't open a public GitHub issue for security problems.**

Email **security@nexuscompress.dev** (PGP key on request) or use
[GitHub's Security tab → "Report a vulnerability"](https://github.com/bierfor/nexus-compress/security/advisories/new).

We commit to:

- Acknowledge within **72 hours**
- Provide a status update within **7 days**
- Patch critical vulnerabilities within **30 days** of disclosure
- Coordinate disclosure timing so users have time to update before
  the public advisory

We follow [coordinated disclosure](https://en.wikipedia.org/wiki/Coordinated_vulnerability_disclosure).
Please give us a reasonable window (typically 90 days, but we're
flexible) before publishing details.

## Audit status

**No formal third-party audit has been performed as of this writing.**
We're a small project; an audit is on the roadmap but not in the
budget yet. (Interested in funding one? Email
licensing@nexuscompress.dev.)

In the meantime, we welcome community review. The transport code is
in `src-tauri/src/p2p_tunnel.rs` (119 unit tests covering the
crypto), `src-tauri/src/p2p_auth.rs` (8 HMAC tests). PRs that tighten
the cryptographic posture are prioritised.

## Security-related changelog

- **Sprint 5.6.26** — `safe_basename()` rejects path-traversal
  filenames from remote peers at the trust boundary
- **Sprint 5.6.7** — `X-Nexus-Auth` pre-auth HMAC required on
  `/spake` (was previously optional, was leaked in pre-auth state)
- **Sprint 5.6.3** — fixed path-basename leak in file picker (picker
  used to return the basename instead of an absolute path; backend
  errored with "file not found")
- **Sprint 5.5.0** — uniform 401 for any auth failure (was leaking
  rate-limit vs bad-sig distinction)
- **Sprint 5.5.0** — rate-limit BEFORE HMAC check (was burning CPU
  on attackers)
- **Sprint 5.4.0** — privacy auto-hide: v3 tokens contain the
  sender's public IP, auto-hidden in the UI 5 s after generation
