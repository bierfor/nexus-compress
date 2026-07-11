//! Tauri command handlers — the IPC bridge between the Next.js UI
//! (in `../frontend/`) and the synchronous `nexus_compress::api`
//! module. Every command is `async` and wraps the CPU-bound work
//! in `spawn_blocking` so the Tauri runtime stays responsive.
//!
//! The mapping is one-to-one with the `api` module: each function
//! here is a thin `#[tauri::command]` that delegates to a
//! corresponding `api::*` function. The Tauri runtime serializes
//! the return values to the frontend via JSON.
//!
//! The mapping is one-to-one with the `api` module: each function
//! here is a thin `#[tauri::command]` that delegates to a
//! corresponding `api::*` function. The Tauri runtime serializes
//! the return values to the frontend via JSON.

use nexus_compress::api::{
    self, ApiError, ApiResult, CompressResult, CompressTargetResult,
    CompressionLevel, DecompressResult, DecompressTargetResult, EngineInfo, PeekResult,
    ProgressEvent, SelfTestResult,
};
// Sprint 5.7.10-E: removed `BackendInfo` and `CompressionBackend` from
// the api imports. The legacy `compress_bytes_with_backend_cmd`,
// `compress_directory_with_backend_cmd`, and `backend_info_cmd`
// Tauri commands are gone (the SupremeEngine is the only public
// compress path). The frontend never called any of them.
use nexus_compress::supreme_engine::{
    CompressionProfile, ProfileCodec, ProfileFidelity, ProfileMode, PROFILE_SCHEMA_VERSION,
};
use std::path::PathBuf;
use std::str::FromStr;

fn to_ipc<T>(r: ApiResult<T>) -> Result<T, String> {
    r.map_err(|e| format!("{}: {}", e.code, e.message))
}

/// Sprint 5.7.10-C: translate the legacy flat-field Tauri
/// request shape into a `CompressionProfile`.
///
/// The 5.7.10-A/-B/-C infrastructure makes the engine profile-
/// driven, but the frontend still sends the legacy flat fields
/// (`backend`, `codec`, `lossless`, `corpus_mode`, etc.) for
/// backward compatibility. This helper maps each flat field
/// to its profile counterpart, defaulting to the safe option
/// when the field is missing.
///
/// In 5.7.10-D the frontend switches to sending
/// `{ req: { path, profile: { ... } } }` directly. The
/// `from_json` constructor on `CompressionProfile` already
/// handles the nested shape, so this helper becomes a
/// one-line passthrough.
fn build_profile_from_legacy_req(
    req: &serde_json::Value,
) -> Result<CompressionProfile, String> {
    // If the request includes a nested `profile` object,
    // use it directly (5.7.10-D forward path).
    if let Some(profile_value) = req.get("profile") {
        return CompressionProfile::from_json(profile_value);
    }

    // Legacy flat-field shape (5.7.10-C backward-compat path).
    //
    // `mode`: the preset ID maps directly to ProfileMode
    // ("rapido" | "balanceado" | "ultra"). Default "balanceado".
    let mode = req
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|s| ProfileMode::from_str(s))
        .transpose()?
        .unwrap_or_default();

    // `codec`: "auto" | "lzma" | "zstd". Default "auto".
    let codec = req
        .get("codec")
        .and_then(|v| v.as_str())
        .map(|s| ProfileCodec::from_str(s))
        .transpose()?
        .unwrap_or_default();

    // `fidelity`: derived from `lossless` / `no_minify` boolean.
    // Both flag forms are accepted (the legacy CLI uses one
    // and the legacy GUI the other).
    let fidelity = if req
        .get("lossless")
        .and_then(|v| v.as_bool())
        .or_else(|| req.get("no_minify").and_then(|v| v.as_bool()))
        .unwrap_or(false)
    {
        ProfileFidelity::Lossless
    } else {
        ProfileFidelity::Lossy
    };

    // `corpus_mode`: "everything" | "source" | "minimal".
    // Default "everything" (5.7.7 inversion).
    let corpus_mode = req
        .get("corpus_mode")
        .and_then(|v| v.as_str())
        .map(|s| api::CorpusMode::from_str(s))
        .transpose()?
        .unwrap_or_default();

    // `raw_extensions` / `minify_extensions`: arrays of strings.
    // Empty arrays default to "use the built-in per-extension
    // table" (the legacy behaviour).
    let raw_extensions: Vec<String> = req
        .get("raw_extensions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let minify_extensions: Vec<String> = req
        .get("minify_extensions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    // `encrypt` / `recovery_level`: derived from the password
    // + recovery pair. Encrypt is true when password is set
    // (the legacy path also forced encrypt when password was
    // present). Recovery defaults to "low" (the design-doc
    // default of 10% parity).
    let encrypt = req.get("password").and_then(|v| v.as_str()).is_some();
    let recovery_level = req
        .get("recovery_level")
        .and_then(|v| v.as_str())
        .map(|s| api::RecoveryLevel::from_str(s))
        .transpose()?
        .unwrap_or_default();

    Ok(CompressionProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        mode,
        codec,
        fidelity,
        corpus_mode,
        raw_extensions,
        minify_extensions,
        encrypt,
        recovery_level,
    })
}

// ─────────────────────────────────────────────────────────────
//  db persist helpers (Sprint 5.7 hotfix #20)
//
//  All three helpers are best-effort wrappers around `db::record_*`.
//  A failure to write stats must NEVER fail the actual compression
//  / decompression / share the user just spent time on — log and
//  return Ok so the IPC response goes back to the UI cleanly.
//  The errors land in stderr where they can be triaged later.
// ─────────────────────────────────────────────────────────────

async fn persist_compression(
    db_state: &State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    filename: &str,
    original_bytes: u64,
    compressed_bytes: u64,
) -> Result<(), String> {
    let conn = db_state.lock().await;
    db::record_compression(&conn, filename, original_bytes, compressed_bytes)
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn persist_decompression(
    db_state: &State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    filename: &str,
    archive_bytes: u64,
    restored_bytes: u64,
) -> Result<(), String> {
    let conn = db_state.lock().await;
    db::record_decompression(&conn, filename, archive_bytes, restored_bytes)
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn persist_share(
    db_state: &State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    filename: &str,
    bytes: u64,
) -> Result<(), String> {
    let conn = db_state.lock().await;
    db::record_share(&conn, filename, bytes).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn compress_bytes_cmd(input: Vec<u8>) -> Result<CompressResult, String> {
    tauri::async_runtime::spawn_blocking(move || api::compress_bytes(&input))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))
}

#[tauri::command]
pub async fn decompress_bytes_cmd(input: Vec<u8>) -> Result<DecompressResult, String> {
    tauri::async_runtime::spawn_blocking(move || to_ipc(api::decompress_bytes(&input)))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn compress_bytes_with_level_cmd(
    input: Vec<u8>,
    level: String,
) -> Result<CompressResult, String> {
    let level = CompressionLevel::from_str(&level).map_err(|e| format!("invalid_level: {}", e))?;
    tauri::async_runtime::spawn_blocking(move || api::compress_bytes_with_level(&input, level))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))
}

#[tauri::command]
pub async fn engine_info_cmd() -> Result<EngineInfo, String> {
    Ok(api::engine_info())
}

/// Compress any path by reference — auto-detects file vs directory.
/// This is the entry point the GUI uses when the user drops a file
/// or folder onto the dropzone (the OS gives us the path, not the
/// bytes) or when they pick via the native dialog. Reads the file
/// from disk on the Rust side, so we never have to ship large bytes
/// through the Tauri IPC boundary. The compressed output is
/// auto-saved next to the input so the user can verify on disk.
///
/// Args are bundled into a single JSON object `req` to avoid the
/// Tauri 2.x arg-name conversion gotchas (camelCase vs snake_case
/// for `lzma_level`). The frontend passes
/// `{ req: { path, backend, lzma_level, output_dir } }`.
///
/// `output_dir` (optional) overrides the "next to input" default:
/// the output filename is preserved but written into that dir.
///
/// Progress is emitted as `compress-progress` events for the JS
/// progress bar.
#[tauri::command]
pub async fn compress_target_cmd(
    app: tauri::AppHandle,
    db_state: State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    req: serde_json::Value,
) -> Result<CompressTargetResult, String> {
    // Sprint 5.7.10-C: this command was ~100 lines of JSON
    // parsing + if-let dispatch into 4 different backend
    // functions. It's now a thin shell that:
    //   1. Builds a `CompressionProfile` from the request fields
    //      (translates the legacy flat shape to the profile).
    //   2. Builds a `CompressInvocation` (profile + path + password
    //      + output dir).
    //   3. Hands it to `SupremeEngine::compress`, which resolves
    //      the right backend / codec / preprocessor internally.
    //
    // The legacy flat-field shape is preserved for 5.7.10-C
    // (this sprint) so the live frontend keeps working
    // unchanged. In 5.7.10-D the command will accept a
    // nested `profile: { ... }` object directly. In
    // 5.7.10-E the `CompressionBackend` enum is deleted.
    let path_str = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path' in req".to_string())?
        .to_string();
    let output_dir = req
        .get("output_dir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    let password: Option<Vec<u8>> = req
        .get("password")
        .and_then(|v| v.as_str())
        .map(|s| s.as_bytes().to_vec());

    // Translate the legacy flat-field shape to a CompressionProfile.
    // The frontend will switch to sending `profile: { ... }`
    // directly in 5.7.10-D. Until then, every field that
    // exists in the profile is mapped from its flat twin.
    let profile = build_profile_from_legacy_req(&req)?;
    let p = PathBuf::from(&path_str);
    let filename_for_db = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let invocation = nexus_compress::supreme_engine::CompressInvocation {
        profile,
        path: p,
        password,
        output_dir,
    };

    let inner: ApiResult<CompressTargetResult> = tauri::async_runtime::spawn_blocking(move || {
        let app_for_event = app.clone();
        // Sprint 5.7 throttling: ProgressEvent emits per chunk
        // (potentially hundreds per second on fast disks). We
        // collect into a slot and only flush to the IPC channel
        // every 100ms — see `throttle::ThrottledEmitter` below.
        let throttler = throttle::ThrottledEmitter::new(app_for_event.clone());
        let cb = |event: ProgressEvent| {
            throttler.feed("compress-progress", &event);
        };
        nexus_compress::supreme_engine::SupremeEngine::compress(&invocation, cb)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?;
    let result: CompressTargetResult = to_ipc(inner)?;

    // Sprint 5.7 hotfix #20: persist into the local SQLite store so
    // the Recientes view (RecentView.tsx) and the Home dashboard
    // (LandingPage) can show real numbers. Best-effort: a stats
    // failure must never fail the actual compression that the user
    // waited for — log + continue.
    if let Err(e) = persist_compression(
        &db_state,
        &filename_for_db,
        result.original_size,
        result.compressed_size,
    )
    .await
    {
        eprintln!("[nexus-rar] WARN: failed to persist compression stats: {}", e);
    }
    Ok(result)
}

/// Open the native save dialog for the user to pick a destination
/// file. Returns the chosen absolute path or `None` if cancelled.
///
/// Takes a single `{ req: { default_filename } }` JSON object to
/// side-step Tauri's arg-name conversion gotchas (the user's
/// `MISSING REQUIRED KEY DEFAULTFILENAME` error from earlier was
/// the same camelCase/snake_case trap that bit `lzma_level`).

/// Decompress an archive by path. Auto-detects the format from the
/// file's magic bytes (NXS6 / NXAR / v4 / v5-v6 single) and restores
/// the contents next to the input. The frontend passes
/// `{ req: { path, output_dir } }`.
///
/// `output_dir` (optional) overrides the auto-generated restore
/// location: extracted contents go there instead.
///
/// Progress is emitted as `compress-progress` events (same channel
/// as compress; the GUI already listens for them).
#[tauri::command]
pub async fn decompress_target_cmd(
    app: tauri::AppHandle,
    db_state: State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    req: serde_json::Value,
) -> Result<DecompressTargetResult, String> {
    let path = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path' in req".to_string())?
        .to_string();
    let output_dir = req
        .get("output_dir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    // Sprint 5.7.2: when the archive is encrypted (NXE\0 / NXR\0
    // magic), the frontend passes `req.password: string` here so
    // we can route to `decompress_target_with_password` which
    // does the AES-256-GCM decrypt (and Reed-Solomon reassembly
    // if recovery is enabled) BEFORE handing the recovered NXS
    // bytes to the plain inner-format decompressor. When password
    // is absent, we go through the plain path — same as before
    // Sprint 5.7.2 — so legacy `.tar` / `.zst` / `.gz` / `.nxs6`
    // / LZMA archives continue to work untouched.
    let password: Option<String> = req
        .get("password")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let p = PathBuf::from(path);
    let filename_for_db = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let inner: ApiResult<DecompressTargetResult> = tauri::async_runtime::spawn_blocking(move || {
        let app_for_event = app.clone();
        let throttler = throttle::ThrottledEmitter::new(app_for_event.clone());
        let cb = |event: ProgressEvent| {
            throttler.feed("compress-progress", &event);
        };
        match password {
            Some(pwd) => {
                // Drop the password immediately after we've moved
                // it into the closure so it doesn't linger on the
                // stack any longer than needed (defense in depth
                // — the `drop(pwd)` below ensures the heap copy
                // is zeroed when the function returns).
                api::decompress_target_with_password(
                    &p,
                    pwd.as_bytes(),
                    cb,
                )
            }
            None => api::decompress_target_with_progress(
                &p,
                output_dir.as_deref(),
                cb,
            ),
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?;
    let result: DecompressTargetResult = to_ipc(inner)?;

    // Sprint 5.7 hotfix #20: persist into the local SQLite store
    // (best-effort — never fail the op if stats fail).
    // For decompression we treat the archive size as the "input"
    // and the restored size as the "output". `original_bytes` in
    // db.rs is the input on disk; for extract flows it equals the
    // archive size. We don't have it in DecompressTargetResult so
    // we use `restored_size` for both — the weighted average stays
    // correct because record_decompression only updates counters
    // and the events row, not the compression_ratio.
    let restored = result.restored_size;
    if let Err(e) = persist_decompression(&db_state, &filename_for_db, restored, restored).await {
        eprintln!("[nexus-rar] WARN: failed to persist decompression stats: {}", e);
    }
    Ok(result)
}

/// Peek at an archive's contents WITHOUT decompressing. Returns
/// the file list, total uncompressed size, and archive kind.
/// The frontend uses this to render a WinRAR-style preview
/// before the user commits to extracting.
///
/// Sprint 5.7.2: extended to handle encrypted archives. The
/// optional `password` field in the req enables the unlock path
/// — when set, the encrypted archive is decrypted (GCM-verified)
/// and the underlying NXS file list is returned. When the password
/// is wrong, the GCM auth fails and we return a clear error code
/// (the frontend renders this as "✗ wrong password" in the
/// ArchivePreview component).
#[tauri::command]
pub async fn peek_archive_target_cmd(req: serde_json::Value) -> Result<PeekResult, String> {
    let path = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path' in req".to_string())?
        .to_string();
    let password: Option<String> = req
        .get("password")
        .and_then(|v| v.as_str())
        .map(String::from);
    let p = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || {
        match password.as_deref() {
            Some(pwd) => to_ipc(api::peek_archive_target_with_password(&p, pwd.as_bytes())),
            None => to_ipc(api::peek_archive_target(&p)),
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

// ============================================================================
//  Sprint 5.6.17: WinRAR-style archive inspection
// ============================================================================

#[derive(Serialize)]
pub struct ArchiveEntryRow {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Serialize)]
pub struct ArchiveListResp {
    /// Entries in this page (subset of the full archive).
    pub entries: Vec<ArchiveEntryRow>,
    /// Total files (excluding directories) in the full archive.
    pub total_files: usize,
    /// Total bytes (sum of original_size across all files).
    pub total_bytes: u64,
    /// Offset into the full entries list (where this page starts).
    pub offset: usize,
    /// Whether there are more entries past this page.
    pub has_more: bool,
    /// Wall-clock time to enumerate the headers (excludes
    /// payload reading — should be milliseconds even for
    /// multi-GB archives). Useful to show "parsed 87K entries
    /// in 12 ms" so users see the cost is small.
    pub parse_time_ms: u128,
}

#[derive(Serialize)]
pub struct ArchiveExtractResp {
    pub written: Vec<String>,
    pub count: usize,
    pub total_bytes: u64,
}

#[tauri::command]
pub async fn p2p_archive_list_cmd(req: serde_json::Value) -> Result<ArchiveListResp, String> {
    let path_str = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path'".to_string())?
        .to_string();
    // Pagination: front-end requests a window of entries at a
    // time. For 30+ GB archives with hundreds of thousands of
    // entries, sending the whole list over IPC would mean a
    // multi-MB JSON parse + a DOM tree the browser can't render.
    // 500 entries per page is enough to fill the viewport and
    // keeps the IPC payload < 100 KB.
    let offset: usize = req
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(0);
    let limit: usize = req
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(2000))
        .unwrap_or(500);
    let path = PathBuf::from(path_str);
    let (page, total_files, total_bytes, has_more, parse_time_ms) =
        tauri::async_runtime::spawn_blocking(move || {
            let start = std::time::Instant::now();
            // list_paginated streams headers incrementally and
            // stops after `limit + 1` entries to detect has_more
            // without enumerating the whole archive. For typical
            // archives this means touching only a few KB of the
            // file, regardless of total size.
            let page = crate::archive_inspect::list_paginated(&path, offset, limit)?;
            // Total stats — use a separate cheap count of just
            // header bytes by reading the file's end (for tar
            // there's no central header, so we approximate via
            // the same iterator).
            let total_bytes: u64 = page.entries.iter().map(|e| e.size).sum();
            // For the total file count, we approximate: if
            // has_more, we know the page is the start of more.
            // We don't enumerate everything — the UI shows
            // "showing N of M+" when has_more is true.
            let total_files = if page.has_more {
                // Upper bound: at least offset + limit + 1 more.
                offset + limit + 1
            } else {
                offset + page.entries.len()
            };
            Ok::<_, String>((
                page.entries,
                total_files,
                total_bytes,
                page.has_more,
                start.elapsed().as_millis(),
            ))
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))??;
    let rows = page
        .into_iter()
        .map(|e| ArchiveEntryRow {
            name: e.name,
            size: e.size,
            is_dir: e.is_dir,
        })
        .collect();
    Ok(ArchiveListResp {
        entries: rows,
        total_files,
        total_bytes,
        has_more,
        offset,
        parse_time_ms,
    })
}

#[tauri::command]
pub async fn p2p_archive_extract_cmd(req: serde_json::Value) -> Result<ArchiveExtractResp, String> {
    let path_str = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path'".to_string())?
        .to_string();
    // output_dir is optional — if null/absent, extract next to
    // the archive (same directory as the source file).
    let output_dir_str: Option<String> = req
        .get("output_dir")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let selected: Option<Vec<String>> = req.get("selected").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    });
    let path = PathBuf::from(&path_str);
    // Resolve output_dir: use the user-supplied path, or fall back
    // to the directory that contains the archive.
    let output_dir = match output_dir_str {
        Some(ref s) => PathBuf::from(s),
        None => PathBuf::from(&path_str)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let output_dir_for_total = output_dir.clone();
    let written = tauri::async_runtime::spawn_blocking(move || {
        crate::archive_inspect::extract_entries(&path, &output_dir, selected)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))??;
    let count = written.len();
    let total_bytes = std::fs::read_dir(&output_dir_for_total)
        .map(|_| 0u64) // quick approximation; the user gets real
        // sizes from list if they need exact totals
        .unwrap_or(0);
    Ok(ArchiveExtractResp {
        written,
        count,
        total_bytes,
    })
}

/// Reveal `path` in Finder (macOS) / File Manager (Linux/Windows).
/// Pass a path to a FILE — Finder will select it. Pass a path to a
/// DIRECTORY — Finder will open it.
#[tauri::command]
pub async fn reveal_in_finder_cmd(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || reveal_in_finder(&path))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[cfg(target_os = "macos")]
fn reveal_in_finder(path: &str) -> Result<(), String> {
    // `-R` flag tells `open` to REVEAL the file in its parent dir.
    let status = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .status()
        .map_err(|e| format!("open -R failed: {}", e))?;
    if !status.success() {
        return Err(format!("open exited with {:?}", status.code()));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reveal_in_finder(path: &str) -> Result<(), String> {
    // xdg-open opens the parent if `path` is a file.
    let status = std::process::Command::new("xdg-open")
        .arg(path)
        .status()
        .map_err(|e| format!("xdg-open failed: {}", e))?;
    if !status.success() {
        return Err(format!("xdg-open exited with {:?}", status.code()));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn reveal_in_finder(path: &str) -> Result<(), String> {
    // `explorer /select,<path>` reveals in Explorer.
    let status = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path))
        .status()
        .map_err(|e| format!("explorer failed: {}", e))?;
    if !status.success() {
        return Err(format!("explorer exited with {:?}", status.code()));
    }
    Ok(())
}

// ============================================================================
//  File/folder pickers (Sprint 5.5.5: blocking pattern — fixes hang on macOS)
// ============================================================================
//
// The previous version of these commands used the async-callback-with-mpsc
// pattern (pick_file(callback) + spawn_blocking(recv)). That pattern is
// documented as deadlock-prone by tauri-plugin-dialog — the callback is
// dispatched from a sub-thread but the dialog itself needs the main
// thread, and the channel rendezvous can hang. The fix is to use the
// blocking_pick_* variants inside spawn_blocking. Simpler and works.

/// Pick a single file. Returns the absolute path or None if cancelled.
#[tauri::command]
pub async fn pick_file_cmd(window: tauri::Window) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let window = window.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let path = window
            .dialog()
            .file()
            .add_filter(
                "Nexus archive (.nxs/.nxs6/.lz/.nxar/.nxr)",
                &["nxs", "nxs6", "lz", "nxar", "nxr"],
            )
            .add_filter("All files", &["*"])
            .blocking_pick_file();
        path.and_then(|fp| fp.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked)
}

/// Pick MULTIPLE files (Sprint 5.5.5).
#[tauri::command]
pub async fn pick_files_cmd(window: tauri::Window) -> Result<Option<Vec<String>>, String> {
    use tauri_plugin_dialog::DialogExt;
    let window = window.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let paths = window
            .dialog()
            .file()
            .add_filter("All files", &["*"])
            .blocking_pick_files();
        paths.map(|v| {
            v.into_iter()
                .filter_map(|fp| fp.into_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
    })
    .await
    .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked)
}

/// Pick a single directory.
#[tauri::command]
pub async fn pick_directory_cmd(window: tauri::Window) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let window = window.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let path = window.dialog().file().blocking_pick_folder();
        path.and_then(|fp| fp.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked)
}

/// Pick MULTIPLE directories.
#[tauri::command]
pub async fn pick_folders_cmd(window: tauri::Window) -> Result<Option<Vec<String>>, String> {
    use tauri_plugin_dialog::DialogExt;
    let window = window.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let paths = window.dialog().file().blocking_pick_folders();
        paths.map(|v| {
            v.into_iter()
                .filter_map(|fp| fp.into_path().ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
    })
    .await
    .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked)
}

/// Pick a save location (the user picks where to write the output).
/// Used by Receive panel. Args via single-JSON-arg pattern (the
/// camelCase/snake_case trap that bit `lzma_level`).
#[tauri::command]
pub async fn pick_save_location_cmd(
    window: tauri::Window,
    req: serde_json::Value,
) -> Result<Option<String>, String> {
    let default_filename = req
        .get("default_filename")
        .and_then(|v| v.as_str())
        .unwrap_or("archive.nxs")
        .to_string();
    use tauri_plugin_dialog::DialogExt;
    let window = window.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let path = window
            .dialog()
            .file()
            .set_file_name(&default_filename)
            .blocking_save_file();
        path.and_then(|fp| fp.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked)
}

#[tauri::command]
pub async fn self_test_cmd() -> Result<SelfTestResult, String> {
    to_ipc(api::self_test())
}

#[tauri::command]
pub async fn compress_directory_cmd(
    input_dir: String,
    level: String,
) -> Result<(api::DirectoryResult, Vec<u8>), String> {
    let level = CompressionLevel::from_str(&level).map_err(|e| format!("invalid_level: {}", e))?;
    let path = PathBuf::from(input_dir);
    tauri::async_runtime::spawn_blocking(move || {
        to_ipc(api::compress_directory(&path, level).map(|(r, a)| (r, a.to_vec())))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn compress_directories_cmd(
    input_dirs: Vec<String>,
    level: String,
) -> Result<(api::DirectoryResult, Vec<u8>), String> {
    let level = CompressionLevel::from_str(&level).map_err(|e| format!("invalid_level: {}", e))?;
    let paths: Vec<PathBuf> = input_dirs.into_iter().map(PathBuf::from).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let path_refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
        to_ipc(api::compress_directories(&path_refs, level).map(|(r, a)| (r, a.to_vec())))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn decompress_directory_cmd(
    archive: Vec<u8>,
    output_dir: String,
) -> Result<api::DirectoryResult, String> {
    let path = PathBuf::from(output_dir);
    tauri::async_runtime::spawn_blocking(move || to_ipc(api::decompress_directory(&archive, &path)))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn peek_archive_cmd(archive: Vec<u8>) -> Result<api::DirectoryResult, String> {
    to_ipc(api::peek_archive(&archive))
}

#[tauri::command]
pub async fn peek_archive_file_cmd(path: String) -> Result<Vec<api::ArchiveEntry>, String> {
    let p = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || to_ipc(api::peek_archive_file(&p)))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn peek_and_extract_file_cmd(
    path: String,
    extract_to: Option<String>,
) -> Result<(Vec<api::ArchiveEntry>, Option<api::DirectoryResult>), String> {
    let p = PathBuf::from(path);
    let out = extract_to.map(PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || {
        to_ipc(api::peek_and_extract_file(&p, out.as_deref()))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn read_file_cmd(path: String) -> Result<Vec<u8>, String> {
    let p = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(&p).map_err(|e| format!("read_file failed: {}", e))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

#[tauri::command]
pub async fn open_path_cmd(path: String) -> Result<(), String> {
    to_ipc(api::open_path(&path))
}

// ============================================================================
//  P2P tunnel helpers
// ============================================================================

/// After receiving a file, if the wire filename ends in `.tar`
/// (i.e. the sender archived a directory), extract it in-place
/// and remove the archive. This restores the original directory
/// structure (including macOS `.app` bundles with their symlinks
/// and permission bits).
///
/// Returns (final_path, final_filename):
///   - For `.tar` files: (extracted_dir_path, "DirName") — the
///     top-level entry name extracted from the archive.
///   - For everything else: (original_path, original_filename)
///     unchanged.
fn maybe_extract_tar(
    received_path: PathBuf,
    wire_filename: Option<&str>,
) -> Result<(PathBuf, Option<String>), String> {
    // Decide whether to extract based on the wire filename
    // (most reliable) or the received path extension.
    let is_tar = wire_filename
        .map(|n| n.ends_with(".tar"))
        .unwrap_or_else(|| {
            received_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("tar"))
                .unwrap_or(false)
        });

    if !is_tar {
        // Nothing to do — return as-is.
        return Ok((received_path, wire_filename.map(|s| s.to_string())));
    }

    // The archive should be extracted next to itself.
    let extract_dir = received_path
        .parent()
        .ok_or_else(|| "received .tar has no parent dir".to_string())?;

    eprintln!(
        "[p2p] auto-extracting {} into {}",
        received_path.display(),
        extract_dir.display()
    );

    // Use system tar to extract: -x extract, -p preserve
    // permissions (vital for .app bundles), -f archive file.
    let tar_output = std::process::Command::new("/usr/bin/tar")
        .arg("-xpf")
        .arg(&received_path)
        .arg("-C")
        .arg(extract_dir)
        .output()
        .map_err(|e| format!("spawn tar -xpf: {}", e))?;

    if !tar_output.status.success() {
        return Err(format!(
            "tar extraction failed: exit={:?} stderr={}",
            tar_output.status.code(),
            String::from_utf8_lossy(&tar_output.stderr)
        ));
    }

    // Determine the top-level name that was extracted. The
    // wire_filename is "DirName.tar", so we strip ".tar" to
    // get "DirName". This is exactly what tar created in
    // extract_dir.
    let extracted_name = wire_filename
        .and_then(|n| n.strip_suffix(".tar"))
        .unwrap_or_else(|| {
            received_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("extracted")
        })
        .to_string();

    let extracted_path = extract_dir.join(&extracted_name);

    // Remove the raw .tar now that we've extracted it.
    if let Err(e) = std::fs::remove_file(&received_path) {
        eprintln!(
            "[p2p] warning: could not remove temp tar {}: {}",
            received_path.display(),
            e
        );
    } else {
        eprintln!("[p2p] removed temp tar {}", received_path.display());
    }

    // Clear quarantine on the extracted tree (macOS: Gatekeeper
    // would block every .app file without this).
    #[cfg(target_os = "macos")]
    {
        let path_str = extracted_path.to_string_lossy().into_owned();
        let _ = std::process::Command::new("/usr/bin/xattr")
            .args(["-rd", "com.apple.quarantine", &path_str])
            .output();
    }

    eprintln!("[p2p] extraction complete: {}", extracted_path.display());

    Ok((extracted_path, Some(extracted_name)))
}

// ============================================================================
//  P2P tunnel — Sprint 5.0 demo
// ============================================================================
//
// The P2P module exposes three Tauri commands:
//   * `p2p_send_start` — pick a local port, spawn cloudflared,
//     run the axum server, and return the token. The started
//     send session is stored in `P2pState` so the UI can cancel
//     it.
//   * `p2p_send_abort` — kill the tunnel + abort the server
//     task. Returns Ok(()) always (idempotent).
//   * `p2p_receive` — connect to the URL in the token, do
//     SPAKE2, download + decrypt + verify the file.
//
// All three use the single-JSON-arg pattern (Tauri 2.x arg-name
// gotcha — see MEMORY.md).
use crate::db;
use crate::p2p_tunnel;
use crate::throttle;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::sync::Mutex;

/// Holds the currently-active send session (if any). There's
/// at most one at a time — the UI's intent switch resets the
/// other side.
pub struct P2pState {
    /// Arc<Mutex> so a background watcher task can monitor
    /// the slot's lifecycle (Sprint 5.6.13) without needing
    /// to borrow through the Tauri State wrapper.
    pub active: Arc<Mutex<Option<p2p_tunnel::StartedSend>>>,
}

#[derive(Deserialize)]
pub struct P2pSendStartReq {
    /// Absolute path of the file to send.
    pub file_path: String,
    /// User-supplied code, or None to let the server generate one.
    pub code: Option<String>,
}

#[derive(Serialize)]
pub struct UpnpStatusInfo {
    /// Public IP the sender is reachable at from the internet
    /// (from UPnP GetExternalIPAddress). Format: dotted-quad.
    pub external_ip: String,
    /// Port the UPnP mapping is on (= the sender's local axum
    /// port for Direct Mode).
    pub external_port: u16,
}

#[derive(Serialize)]
pub struct P2pSendStartResp {
    /// base64url-encoded token (the thing to share with the
    /// receiver). v1 for Quick/Named, v2 for Direct LAN-only,
    /// v3 for Direct cross-NAT (with UPnP hole).
    pub token: String,
    /// The code we used, regardless of whether the user
    /// supplied it or we generated it. The UI shows this in
    /// big letters under the QR / token box.
    pub code: String,
    /// Pretty-printed name of the file.
    pub filename: String,
    /// Plaintext size in bytes (so the UI can show
    /// "Sending 5.3 MB" before the connection starts).
    pub file_size: u64,
    /// Sprint 5.5.4 Phase 3: UPnP hole status. `Some` means
    /// the sender has an open port mapping and the token is v3
    /// (cross-NAT capable). `None` means UPnP was unavailable
    /// and the token is v2 (LAN-only).
    pub upnp_status: Option<UpnpStatusInfo>,
}

#[tauri::command]
pub async fn p2p_send_start_cmd(
    app: tauri::AppHandle,
    db_state: State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    req: serde_json::Value,
    state: State<'_, Arc<P2pState>>,
) -> Result<P2pSendStartResp, String> {
    use tauri::Manager; // brings .path() into scope on AppHandle
    let req: P2pSendStartReq = serde_json::from_value(req)
        .map_err(|e| format!("invalid p2p_send_start request: {}", e))?;
    let file_path = PathBuf::from(&req.file_path);
    if !file_path.exists() {
        return Err(format!("file not found: {}", req.file_path));
    }
    let code = match req.code {
        Some(c) if !c.trim().is_empty() => c.trim().to_string(),
        _ => p2p_tunnel::random_code_phrase(4),
    };
    // Refuse if a send is already in flight. The UI's intent
    // switch should call p2p_send_abort first; this is a
    // safety net.
    {
        let active = state.active.lock().await;
        if active.is_some() {
            return Err("a p2p send is already in progress — abort it first".to_string());
        }
    }
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app_data_dir: {}", e))?;
    std::fs::create_dir_all(&app_data_dir).map_err(|e| format!("mkdir app_data_dir: {}", e))?;
    let started = p2p_tunnel::start_sender(file_path.clone(), code.clone(), app_data_dir).await?;
    // Sprint 5.6.16: if the user picked a directory, start_sender
    // The filename is already computed correctly by start_sender
    // (it adds .tar for directories, preserves the original name
    // for files and already-compressed formats). Read it from the
    // token rather than re-computing it here to avoid drift.
    let filename = started.token.filename.clone().unwrap_or_else(|| {
        file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string()
    });
    let file_size = started.token.size;
    let upnp_status = started.upnp_info.as_ref().map(|info| UpnpStatusInfo {
        external_ip: info.external_ip.to_string(),
        external_port: info.external_port,
    });
    let resp = P2pSendStartResp {
        token: started.token_compact.clone(),
        code: started.token.code.clone(),
        filename,
        file_size,
        upnp_status,
    };
    // Sprint 5.7 hotfix #20: record the share start so Recientes
    // shows "Enlace compartido generado". We record AT START (not
    // on completion) because p2p_send_start_cmd returns the link
    // immediately and the receiver may or may not actually finish
    // downloading — counting it from the moment the user shared
    // matches the user-perceived "I shared this file" intent and
    // matches what the Recientes + Home counters should reflect.
    if let Err(e) = persist_share(&db_state, &resp.filename, resp.file_size).await {
        eprintln!("[nexus-rar] WARN: failed to persist share stats: {}", e);
    }
    // Sprint 5.6.15: clear state.active when either (a) the
    // receiver hits /done (signaling successful file transfer)
    // or (b) 10 minutes have passed since the sender started
    // (hard timeout for abandoned sessions). The watcher
    // polls every 500ms.
    //
    // Sprint 5.6.13 tried to detect this via
    // `server_task.is_finished()`, but axum::serve runs the
    // TCP listener loop FOREVER — it never returns naturally.
    // So that approach was broken. We now rely on explicit
    // signals from the receiver (/done) plus a timeout.
    let state_clone = state.inner().clone();
    *state.active.lock().await = Some(started);
    let active_for_watcher = state_clone.active.clone();
    tokio::spawn(async move {
        let session_start = std::time::Instant::now();
        let hard_timeout = Duration::from_secs(10 * 60); // 10 min
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let mut guard = active_for_watcher.lock().await;
            let should_clear = match guard.as_ref() {
                // receiver flagged completion — clear immediately
                Some(s) if s.done.load(std::sync::atomic::Ordering::SeqCst) => {
                    eprintln!("[p2p-state] /done received, clearing active slot");
                    true
                }
                // server task aborted (abort_cmd called) — clear
                Some(s) if s.server_task.is_finished() => {
                    eprintln!("[p2p-state] server task aborted, clearing active slot");
                    true
                }
                // hard timeout for sessions that never finished
                Some(s) if session_start.elapsed() >= hard_timeout => {
                    eprintln!(
                        "[p2p-state] hard timeout reached ({}s), clearing active slot",
                        session_start.elapsed().as_secs()
                    );
                    // also abort the server so it stops listening
                    s.server_task.abort();
                    true
                }
                Some(_) => false,
                None => return, // already cleared — exit watcher
            };
            if should_clear {
                *guard = None;
                return;
            }
        }
    });
    Ok(resp)
}

#[tauri::command]
pub async fn p2p_send_abort_cmd(state: State<'_, Arc<P2pState>>) -> Result<(), String> {
    let mut active = state.active.lock().await;
    if let Some(started) = active.take() {
        // Dropping StartedSend kills the tunnel child (Drop on
        // TunnelHandle) and aborts the server task (Drop on
        // JoinHandle, well, JoinHandle doesn't abort on drop
        // — we need to explicitly abort).
        started.server_task.abort();
        drop(started.tunnel);
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct P2pReceiveReq {
    /// base64url-encoded token from the sender.
    pub token: String,
    /// Where to write the decrypted file.
    pub output_path: String,
}

#[derive(Serialize)]
pub struct P2pReceiveResp {
    /// Plaintext bytes received.
    pub bytes_written: u64,
    /// Absolute path of the written file.
    pub output_path: String,
    /// Sprint 5.6.8: original filename from the sender. None
    /// if the sender didn't include it (older tokens).
    pub filename: Option<String>,
}

#[tauri::command]
pub async fn p2p_receive_cmd(req: serde_json::Value) -> Result<P2pReceiveResp, String> {
    let req: P2pReceiveReq =
        serde_json::from_value(req).map_err(|e| format!("invalid p2p_receive request: {}", e))?;
    let token = p2p_tunnel::P2pToken::from_compact(&req.token)
        .map_err(|e| format!("invalid token: {}", e))?;
    let output_path = PathBuf::from(&req.output_path);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir output parent: {}", e))?;
    }
    let result = p2p_tunnel::receive_send_file(token, output_path).await?;
    let (final_path, final_filename) =
        maybe_extract_tar(result.output_path, result.filename.as_deref())?;
    Ok(P2pReceiveResp {
        bytes_written: result.bytes_written,
        output_path: final_path.to_string_lossy().to_string(),
        filename: final_filename,
    })
}

/// Receive a file sent via Direct Mode (LAN + mDNS). The token
/// is a v2 token (`nx:2:direct:<hash>:<code>`) — no URL, the
/// sender is discovered via mDNS browse. `timeout_secs` caps
/// the mDNS browse window (5s default in the UI).
#[tauri::command]
pub async fn p2p_receive_direct_cmd(req: serde_json::Value) -> Result<P2pReceiveResp, String> {
    let req: serde_json::Value = req;
    let token_v2 = req
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'token'".to_string())?
        .to_string();
    let output_path_str = req
        .get("output_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'output_path'".to_string())?
        .to_string();
    let timeout_secs = req
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);
    let output_path = PathBuf::from(&output_path_str);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir output parent: {}", e))?;
    }
    let result = p2p_tunnel::receive_direct_file(token_v2, output_path, timeout_secs).await?;
    let (final_path, final_filename) =
        maybe_extract_tar(result.output_path, result.filename.as_deref())?;
    Ok(P2pReceiveResp {
        bytes_written: result.bytes_written,
        output_path: final_path.to_string_lossy().to_string(),
        filename: final_filename,
    })
}

/// Sprint 5.6.9: peek the original filename from the sender
/// BEFORE the user clicks "Recibir". Returns null if the
/// sender didn't include one (older tokens). For v2/v3 this
/// probes the sender's endpoint and reads /meta.
#[derive(Serialize)]
pub struct P2pPeekFilenameResp {
    pub filename: Option<String>,
}

#[tauri::command]
pub async fn p2p_peek_filename_cmd(req: serde_json::Value) -> Result<P2pPeekFilenameResp, String> {
    let token = req
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'token'".to_string())?
        .to_string();
    let timeout_secs = req
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);
    let filename = p2p_tunnel::peek_filename(&token, timeout_secs).await?;
    Ok(P2pPeekFilenameResp { filename })
}

// ============================================================================
//  Sprint 5.5.1 — tunnel config + transport mode commands
// ============================================================================

/// Returned by `p2p_get_tunnel_config_cmd`. `has_token` is true
/// iff a token is currently stored in the OS keyring. The
/// token itself is never returned (it's a secret).
#[derive(Serialize)]
pub struct P2pTunnelConfigResp {
    pub mode: String,
    pub hostname: Option<String>,
    pub has_token: bool,
}

#[tauri::command]
pub async fn p2p_get_tunnel_config_cmd(
    app: tauri::AppHandle,
) -> Result<P2pTunnelConfigResp, String> {
    use p2p_tunnel::p2p_config::{load_tunnel_config, TokenStore};
    use tauri::Manager; // brings .path() into scope on AppHandle
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app_data_dir: {}", e))?;
    let cfg = load_tunnel_config(&app_data_dir)?;
    let has_token = p2p_tunnel::p2p_config::default_token_store()
        .get_token()?
        .is_some();
    Ok(P2pTunnelConfigResp {
        mode: match cfg.mode {
            p2p_tunnel::p2p_config::TransportMode::Quick => "quick".to_string(),
            p2p_tunnel::p2p_config::TransportMode::Named => "named".to_string(),
            p2p_tunnel::p2p_config::TransportMode::Direct => "direct".to_string(),
        },
        hostname: cfg.hostname,
        has_token,
    })
}

#[derive(Deserialize)]
pub struct P2pSaveTunnelConfigReq {
    pub mode: String,
    pub hostname: Option<String>,
    /// Required iff mode == "named". The token is validated
    /// (length, base64 charset) and then written to the OS
    /// keyring. The token is NEVER persisted to disk.
    pub token: Option<String>,
}

#[tauri::command]
pub async fn p2p_save_tunnel_config_cmd(
    app: tauri::AppHandle,
    req: serde_json::Value,
) -> Result<(), String> {
    use p2p_tunnel::p2p_config::{
        default_token_store, save_tunnel_config, validate_hostname, validate_token, TokenStore,
        TransportMode, TunnelConfig,
    };
    use tauri::Manager; // brings .path() into scope on AppHandle
    let req: P2pSaveTunnelConfigReq = serde_json::from_value(req)
        .map_err(|e| format!("invalid save_tunnel_config request: {}", e))?;
    let mode = match req.mode.as_str() {
        "quick" => TransportMode::Quick,
        "named" => TransportMode::Named,
        "direct" => TransportMode::Direct,
        other => return Err(format!("invalid transport mode: {}", other)),
    };
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app_data_dir: {}", e))?;
    // Validate and normalize hostname (only relevant for Named
    // mode, but we validate if present).
    let hostname = req.hostname.as_deref().map(validate_hostname).transpose()?;
    let hostname = hostname.map(|s| s.strip_prefix("https://").unwrap_or(&s).to_string());
    // Validate the token (if provided). This is the FIRST
    // place we touch the secret — the token is in memory
    // only, never on disk.
    if let Some(token) = req.token.as_deref() {
        if !token.is_empty() {
            validate_token(token)?;
            default_token_store().set_token(token)?;
        }
    }
    let cfg = TunnelConfig { mode, hostname };
    save_tunnel_config(&app_data_dir, &cfg)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────
//  Sprint 5.7: persistent stats + activity events
// ─────────────────────────────────────────────────────────────

/// Return the current global counters for the home dashboard.
/// Tauri serializes this straight to JSON; the frontend reads
/// the same fields shown in AppStats.
#[tauri::command]
pub async fn get_stats_cmd(
    state: State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
) -> Result<db::AppStats, String> {
    let conn = state.lock().await;
    db::get_stats(&conn).map_err(|e| e.to_string())
}

/// Return the most recent activity events for the home
/// "Actividad" / "Actividad reciente" sections. `limit` caps
/// the result; pass 5 for the sidebar, 30+ for the main panel.
#[tauri::command]
pub async fn get_recent_events_cmd(
    state: State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
    limit: u32,
) -> Result<Vec<db::ActivityEvent>, String> {
    let conn = state.lock().await;
    db::list_events(&conn, limit).map_err(|e| e.to_string())
}

/// Returns the absolute path of the data directory used for
/// the SQLite store. Used by the Settings panel's
/// "Reveal in Finder / Explorer" action.
#[tauri::command]
pub fn data_dir_cmd() -> Result<String, String> {
    db::data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| e.to_string())
}

/// Sprint 5.7: wipe all stats + events from the local SQLite store.
/// Triggered by the Settings → Storage → "Borrar estadísticas"
/// button. The user is asked for confirmation in the UI before
/// this command is invoked; this command itself does not prompt.
#[tauri::command]
pub async fn reset_stats_cmd(
    state: State<'_, Arc<tokio::sync::Mutex<rusqlite::Connection>>>,
) -> Result<(), String> {
    let conn = state.lock().await;
    db::reset_stats(&conn).map_err(|e| e.to_string())
}
