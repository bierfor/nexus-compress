# NexusCompress v0.1.2 — Destructive Testing Protocol

> **Audience:** technical beta testers, security researchers, sysadmins.
> **Goal:** validate the AES-256-GCM encryption + Reed-Solomon recovery stack
> introduced in Sprint 5.7.2, end-to-end against the v0.1.2 release.
> **Risk:** zero. The protocol is non-destructive on real files — every step
> operates on a copy in `/tmp`.

---

## 0. Why this protocol exists

The v0.1.1 release shipped the GUI + CLI baseline (compression, decompression,
inspection) on all 6 platforms. v0.1.2 adds the **crypto + recovery** stack
that the design doc §2 commits to:

- **AES-256-GCM** per shard with `block_id || archive_nonce || uncompressed_size`
  triple-binding AAD (defeats block-shuffling attacks).
- **Argon2id** KDF with auto-downgrade when the system has <19 MiB free RAM.
- **Reed-Solomon** over GF(2^8) with Cauchy matrix + in-place Gauss-Jordan
  (no GPL-3.0 `reed-solomon-erasure` dependency).
- **Lazy parity decrypt** — when no data shard is corrupt, the parity shards
  are never decrypted (saves ≈10% / 25% wall time on Low / High recovery).
- **Phase-level progress events** — the UI bar updates at every meaningful
  boundary, with a special amber "🚨 Corrupción detectada — Reparando bloques
  con Reed-Solomon" event when RS recovery kicks in.

The only way to **prove** all of that works is to break it on purpose. This
protocol walks through three test scenarios, from easy to gnarly, that you can
run in 10–15 minutes total.

---

## 1. Prerequisites

Download the v0.1.2 build for your platform from the GitHub release page. The
release draft will be at:

```
https://github.com/bierfor/nexus-compress/releases/tag/v0.1.2
```

Install the GUI app, then make sure the **standalone CLI** is also on your
PATH. On macOS:

```bash
# Standalone engine zip from the workflow artifacts section:
#   nexus-engine-x86_64-apple-darwin   (Intel)
#   nexus-engine-aarch64-apple-darwin  (Apple Silicon)
unzip nexus-engine-*.zip -d ~/.local/bin
export PATH="$HOME/.local/bin/nexus-engine-*:$PATH"
nexus --version
# → nexus 0.1.2
```

Linux / Windows are analogous; the CLI binary is `nexus.exe` on Windows.

You also need a **hex editor**. Recommended:

- **macOS**: [Hex Fiend](https://hexfiend.com) (free, App Store)
- **Linux**: `hexedit` (terminal) or `ghex` (GUI)
- **Windows**: [HxD](https://mh-nexus.de/en/hxd/) (free)
- **CLI-only**: `xxd` (everywhere — used in §4)

Pick a **test file**: a 1–10 MiB file that you don't mind corrupting copies of.
Source code, a small image, a compressed archive, whatever. Avoid files >100
MiB for the first run — the recovery math is fast but the round-trip can take
10–20 seconds on slow disks.

---

## 2. Test A — The Wrong Password (negative test)

**What this proves:** the wrong-password failure mode surfaces a clean UI
error, doesn't silently decrypt garbage, doesn't crash, and doesn't leak
bytes.

**Estimated time:** 2 minutes.

### Step-by-step

1. Open the NexusRAR GUI.
2. Drag a 1 MiB test file into the **Comprimir** pane.
3. Tick **🔐 Cifrar archivo** (Encrypt).
4. Type the password `correct-horse-battery-staple`.
5. Set **Recuperación ante daños** to `Baja (10%)` (Low).
6. Click **Comprimir**. Wait for the green "Listo" event.
7. Note the output path (something like `~/Desktop/test.nxr`).

8. Switch to the **Descomprimir** pane.
9. Drag the `.nxr` you just made into the drop zone.
10. The preview should show **🔒 Archivo cifrado** with a password input.
11. Type `wrong-password-on-purpose`.
12. Click **🔓 Desbloquear lista**.

**Expected result:** the preview shows a clean **"Contraseña incorrecta"**
error. The file list stays empty. The UI does not freeze. No extraction
starts.

**Bonus (paranoid check):** open the corrupted `.nxr` in a hex editor. You
should see the `NXR\0` magic at offset 0 and a fully-populated header. The
encrypted shards are unintelligible — you literally cannot tell whether the
password was right or wrong by inspecting the ciphertext alone. That's the
point of AES-GCM.

**What this test catches:**

- ❌ Wrong password → file opens, shows garbage data → bug (GCM broken)
- ❌ Wrong password → app crashes → bug (missing error handling)
- ❌ Wrong password → app freezes for >5 s → bug (no Argon2id downgrade)
- ✅ Wrong password → clean error, exit code 0, no data leak

### CLI equivalent

```bash
# Setup: encrypt a test file
head -c 1048576 /dev/urandom > /tmp/test.bin
nexus c --password 'correct-horse-battery-staple' --recovery low \
    /tmp/test.bin /tmp/test.nxr

# Negative test: decrypt with the wrong password
nexus d --password 'wrong-password' /tmp/test.nxr /tmp/wrong.bin
# Expected exit: non-zero
# Expected stderr: "encrypted.wrong_password: wrong password"
```

---

## 3. Test B — The Hex Fiend Trauma (the marquee test)

**What this proves:** the Reed-Solomon recovery path correctly reconstructs
shards when the underlying disk has lost bytes. This is the feature that
**no commercial archiver** ships by default (7z/WinRAR need explicit setup;
RAR adds it but charges). NexusRAR v0.1.2 does it out of the box.

**Estimated time:** 5–10 minutes.

### Step-by-step

1. Compress a 5 MiB test file with **High recovery (25%)**:
   - In the GUI: tick encryption, set recovery to `Alta (25%)`, password anything.
   - The output will be `.nxr` (encryption + recovery).
   - For a 5 MiB input → roughly **5.2 MiB** output (extra 25% parity shards).

2. **Make a backup copy** — the next step is destructive:
   ```bash
   cp ~/Desktop/test.nxr ~/Desktop/test.nxr.backup
   ```

3. Open the `.nxr` file in your hex editor.

4. Locate a data-shard region. The first 64 bytes are the **V3 header**
   (you can identify it by the `NXR\0` magic at offset 0). The actual data
   shards start at byte 64 and are contiguous from there. Each shard is
   **64 KiB + 16 bytes GCM tag** = **65,552 bytes** (`0x10018`).

5. Pick a shard to "lose" — say **shard #5** (the 6th shard, zero-indexed).
   Its byte offset in the file is:

   ```
   64 (header) + 5 × 65552 = 328,324 → 0x50064 + 0x40 = 0x500A4
   ```

6. In the hex editor, select **4096 bytes** starting at that offset and
   overwrite them with **all zeros** (`00 00 00 00 ...`). This simulates
   4 KiB of physical disk corruption inside shard #5.

7. Save the file.

8. Switch to the **Descomprimir** pane in NexusRAR.
9. Drag the **corrupted** `.nxr` (NOT the backup) into the drop zone.
10. Type the password you used in step 1.
11. Click **🔓 Desbloquear lista** — the file list should appear normally
    (the peek path validates the password; corruption is detected later).
12. Click **Decomprimir**.

**Expected result:** the progress bar transitions:

```
"Descifrando..." (cyan)
   ↓
"🚨 Corrupción detectada — Reparando bloques con Reed-Solomon" (amber, bold)
   ↓
"Reensamblando..." (cyan)
   ↓
"Listo" (terminal)
```

The restored file is **byte-for-byte identical** to the original. Confirm with:

```bash
cmp ~/Desktop/test.bin ~/Desktop/test_restored/test.bin
# Exit 0 → exact match
# Exit 1 → recovery failed (report a bug!)
```

**What this test catches:**

- ❌ Corrupted shard → "UnrecoverableBlock" error → bug (RS broken)
- ❌ Corrupted shard → silently restored file with wrong data → bug (GCM bypass)
- ❌ Corrupted shard → app crashes → bug (panic in decode_recover)
- ❌ UI doesn't show "🚨 Corrupción detectada" event → bug (progress channel broken)
- ✅ Corrupted shard → amber event, exact restore, exit 0

### CLI equivalent

```bash
# 1. Compress with high recovery
head -c 5242880 /dev/urandom > /tmp/big.bin
nexus c --password 'demo' --recovery high /tmp/big.bin /tmp/big.nxr

# 2. Corrupt 4 KiB inside shard #5 (offset = 64 + 5*65552 = 328324)
printf '\x00%.0s' $(seq 1 4096) | dd of=/tmp/big.nxr \
    bs=1 seek=328324 count=4096 conv=notrunc

# 3. Decrypt + recover
nexus d --password 'demo' /tmp/big.nxr /tmp/big_restored.bin

# 4. Verify exact match
cmp /tmp/big.bin /tmp/big_restored.bin && echo "✓ Recovery OK"
```

---

## 4. Test C — The Mass Corruption (stress test)

**What this proves:** the recovery budget is honored exactly. With
`Low` recovery (10% parity), up to 10% of shards can be lost and the file
is still recoverable. Above 10%, the math breaks and we return a clean
error instead of garbage.

**Estimated time:** 5 minutes.

### Step-by-step

1. Compress a 5 MiB test file with **Low recovery (10%)**:
   - Output ≈ 5 MiB + 10% parity shards ≈ 5.2 MiB.
   - For 5 MiB compressed: roughly **78 shards** at 64 KiB each.
   - **Low recovery budget:** floor(78 × 0.10) = 7 missing shards OK.

2. In your hex editor, **zero out 4096 bytes in 7 different shards**:
   - Shards 1, 12, 23, 34, 45, 56, 67 (every ~11th shard).
   - Use offset = `64 + shard_index × 65552` for each.

3. Save the file.

4. Open in the **Descomprimir** pane, type the password, click **Descomprimir**.

**Expected result:** amber recovery event, exact restore. The bar advances
through all 6 phases and the restored file matches the original byte-for-byte.

5. Now corrupt **8 shards** (one over the budget):
   - Add shard 70 to your zero-out list.

6. Save, re-attempt decompression.

**Expected result:** clean error in the UI, exit code non-zero:

```
Corruption exceeds recovery budget: 8 missing, 7 parity shards available.
```

**What this test catches:**

- ❌ Over-budget corruption → silent garbage file → CRITICAL bug (RS broken)
- ❌ Over-budget corruption → panic → bug
- ✅ Over-budget corruption → clean error, no partial file written

### CLI equivalent

```bash
# Stress: corrupt exactly at the budget (should succeed)
# ... (encrypt + zero 7 shards as above) ...
nexus d --password 'demo' /tmp/big_corrupted.nxr /tmp/big_recovered.bin
cmp /tmp/big.bin /tmp/big_recovered.bin && echo "✓ Within-budget recovery OK"

# Stress: corrupt over the budget (should fail clean)
# ... (zero 8 shards instead of 7) ...
nexus d --password 'demo' /tmp/big_corrupted.nxr /tmp/big_recovered.bin
# Expected: non-zero exit, stderr "TooManyMissing { missing: 8, parity: 7 }"
```

---

## 5. Reporting findings

If a test fails, please open a GitHub issue at
`https://github.com/bierfor/nexus-compress/issues/new` with:

- **Title:** `[v0.1.2 test] <which test, what's wrong>` (e.g. `[v0.1.2 test B] RS recovery returns wrong data`)
- **Body:**
  1. Your OS + arch (e.g. `macOS 14.5 arm64`, `Ubuntu 24.04 x86_64`)
  2. NexusRAR version: `nexus --version`
  3. The exact command(s) you ran
  4. The exact output (stdout + stderr)
  5. The SHA-256 of the input file and the (corrupted) archive, so we can
     reproduce byte-for-byte:
     ```bash
     shasum -a 256 /tmp/test.bin /tmp/test.nxr
     ```

**No need to share the input file contents** — the SHA-256 + your repro
commands are enough for us to bisect.

---

## 6. Reference: format spec at a glance

For the curious. The `.nxr` file is laid out exactly as:

```
Offset  Size  Field
0       4     Magic ("NXR\0" = encrypted + recovery, or "NXE\0" = encrypted, no recovery)
4       1     Version (always 3)
5       1     Flags (bitfield — see docs/sprint-5.7.2-design.md §3.2)
6       2     Reserved (zero)
8       16    KDF salt (Argon2id)
24      4     KDF params (m_lo, m_hi, t_cost, p_cost)
28      8     Archive nonce (per-archive prefix; per-block id is in the GCM nonce)
36      4     Block count (data_shards + parity_shards, u32 LE)
40      2     Parity count (u16 LE)
42      2     Data shard count (u16 LE)
44      8     Original uncompressed size (u64 LE)
52      12    Padding (must be zero)
─── 64 bytes total ───

64+N×4  ...   Frames: u32 ciphertext_len (LE) + ciphertext + 16-byte GCM tag
                 (one frame per shard, in order shard 0..N-1)
```

AES-GCM nonce per block is `[block_id (u32 LE) || archive_nonce (8 bytes)]`
= 12 bytes total. AAD per block is `[block_id || archive_nonce || uncompressed_size (u32 LE)]`.

So if you want to corrupt a specific shard `i`, the offset of its ciphertext
in the file is:

```
frame_offset(i) = 64
                + Σ_{k=0}^{i-1} (4 + len(ciphertext_k))
```

For shard 5 in a file with uniform shard sizes (the common case), that's
`64 + 5 × (4 + shard_size + 16)`. With `shard_size = 65536`, that's
`64 + 5 × 65556 = 327,844` — close enough to the round `328,324` in §3 that
you can use either.

---

## 7. What we WON'T fix in v0.1.2

These are known limitations, deferred to v0.2.0:

- **No progress ticks inside the rayon `par_iter` itself.** The bar updates
  at phase boundaries (`reading → encrypting → done` for compress;
  `decrypting → recovering → reassembling → done` for decompress). Live
  per-shard ticks would require a `crossbeam_channel` + dedicated drainer
  thread, and AES-GCM at ~25 GB/s aggregate makes the encryption step
  sub-second anyway. The "🚨 recovering" event fires before the actual RS
  math runs, so the user gets a sub-50ms signal that corruption was
  detected.
- **No `unstable` flag.** v0.1.2 is stable. The next breaking format change
  will be v3.0 (probably for true streaming encrypt).
- **No streaming compression for archives >1 GiB.** The full compressed
  buffer has to fit in RAM. This is an intentional trade-off — the
  alternative is a windowed LZ77 that loses ~10–15% ratio. Tracked for
  v0.3.0.

---

*Happy testing. If you find a bug, you'll be credited in the v0.1.3
changelog (and probably sent a sticker).* 🛠️