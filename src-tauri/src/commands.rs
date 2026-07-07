//! Tauri command handlers — the IPC bridge between the Next.js UI
//! (in `../frontend/`) and the synchronous `nexus_compress::api`
//! module. Every command is `async` and wraps the CPU-bound work
//! in `spawn_blocking` so the Tauri runtime stays responsive.
//!
//! The mapping is one-to-one with the `api` module: each function
//! here is a thin `#[tauri::command]` that delegates to a
//! corresponding `api::*` function. The Tauri runtime serializes
//! the return values to the frontend via JSON.

use nexus_compress::api::{
    self, ApiResult, BackendInfo, CompressResult, CompressionBackend, CompressionLevel,
    CompressTargetResult, DecompressResult, DecompressTargetResult, EngineInfo, PeekResult,
    ProgressEvent, SelfTestResult,
};
use std::path::PathBuf;
use std::str::FromStr;

fn to_ipc<T>(r: ApiResult<T>) -> Result<T, String> {
    r.map_err(|e| format!("{}: {}", e.code, e.message))
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
    let level = CompressionLevel::from_str(&level)
        .map_err(|e| format!("invalid_level: {}", e))?;
    tauri::async_runtime::spawn_blocking(move || {
        api::compress_bytes_with_level(&input, level)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))
}

#[tauri::command]
pub async fn engine_info_cmd() -> Result<EngineInfo, String> {
    Ok(api::engine_info())
}

#[tauri::command]
pub async fn backend_info_cmd() -> Result<Vec<BackendInfo>, String> {
    Ok(api::backend_info())
}

#[tauri::command]
pub async fn compress_bytes_with_backend_cmd(
    input: Vec<u8>,
    file_name: String,
    backend: String,
    lzma_level: u32,
) -> Result<CompressResult, String> {
    let backend = CompressionBackend::from_str(&backend)
        .map_err(|e| format!("invalid_backend: {}", e))?;
    tauri::async_runtime::spawn_blocking(move || {
        api::compress_bytes_with_backend(&input, &file_name, backend, lzma_level)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))
}

#[tauri::command]
pub async fn compress_directory_with_backend_cmd(
    input_dir: String,
    backend: String,
    lzma_level: u32,
) -> Result<(api::DirectoryResult, Vec<u8>), String> {
    let backend = CompressionBackend::from_str(&backend)
        .map_err(|e| format!("invalid_backend: {}", e))?;
    let path = PathBuf::from(input_dir);
    tauri::async_runtime::spawn_blocking(move || {
        to_ipc(
            api::compress_directory_with_backend(&path, backend, lzma_level)
                .map(|(r, a)| (r, a.to_vec())),
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
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
    req: serde_json::Value,
) -> Result<CompressTargetResult, String> {
    let path = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path' in req".to_string())?
        .to_string();
    let backend_str = req
        .get("backend")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'backend' in req".to_string())?;
    let lzma_level = req
        .get("lzma_level")
        .and_then(|v| v.as_u64())
        .unwrap_or(6) as u32;
    let output_dir = req
        .get("output_dir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);
    let backend = CompressionBackend::from_str(backend_str)
        .map_err(|e| format!("invalid_backend: {}", e))?;
    let p = PathBuf::from(path);

    tauri::async_runtime::spawn_blocking(move || {
        let app_for_event = app.clone();
        let cb = |event: ProgressEvent| {
            use tauri::Emitter;
            let _ = app_for_event.emit("compress-progress", &event);
        };
        to_ipc(api::compress_target(
            &p,
            backend,
            lzma_level,
            output_dir.as_deref(),
            cb,
        ))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

/// Open the native save dialog for the user to pick a destination
/// file. Returns the chosen absolute path or `None` if cancelled.
///
/// Takes a single `{ req: { default_filename } }` JSON object to
/// side-step Tauri's arg-name conversion gotchas (the user's
/// `MISSING REQUIRED KEY DEFAULTFILENAME` error from earlier was
/// the same camelCase/snake_case trap that bit `lzma_level`).
#[tauri::command]
pub async fn pick_save_location_cmd(
    app: tauri::AppHandle,
    req: serde_json::Value,
) -> Result<Option<String>, String> {
    let default_filename = req
        .get("default_filename")
        .and_then(|v| v.as_str())
        .unwrap_or("archive.nxs")
        .to_string();
    use tauri::Manager;
    use tauri_plugin_dialog::{DialogExt, FilePath};
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
    let (tx, rx) = std::sync::mpsc::channel::<Option<FilePath>>();
    app.dialog()
        .file()
        .set_file_name(&default_filename)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked.and_then(|fp| fp.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

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
    let p = PathBuf::from(path);

    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Emitter;
        let app_for_event = app.clone();
        let cb = |event: ProgressEvent| {
            let _ = app_for_event.emit("compress-progress", &event);
        };
        to_ipc(api::decompress_target_with_progress(
            &p,
            output_dir.as_deref(),
            cb,
        ))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

/// Peek at an archive's contents WITHOUT decompressing. Returns
/// the file list, total uncompressed size, and archive kind.
/// The frontend uses this to render a WinRAR-style preview
/// before the user commits to extracting.
#[tauri::command]
pub async fn peek_archive_target_cmd(
    req: serde_json::Value,
) -> Result<PeekResult, String> {
    let path = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path' in req".to_string())?
        .to_string();
    let p = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || to_ipc(api::peek_archive_target(&p)))
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
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

#[tauri::command]
pub async fn self_test_cmd() -> Result<SelfTestResult, String> {
    to_ipc(api::self_test())
}

#[tauri::command]
pub async fn pick_file_cmd(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri::Manager;
    use tauri_plugin_dialog::{DialogExt, FilePath};
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
    let (tx, rx) = std::sync::mpsc::channel::<Option<FilePath>>();
    // Cover EVERY suffix the codebase can produce. Critically, the
    // "Nexus archive" filter is NOT made the default — the
    // default "All files" filter lets the user pick a single
    // source file (e.g. an uncompressed `.ts`) without having to
    // dig into a filter dropdown.
    app.dialog()
        .file()
        .add_filter(
            "Nexus archive (.nxs/.nxs6/.lz)",
            &["nxs", "nxs6", "lz", "nxar", "nxr"],
        )
        .add_filter("All files", &["*"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked.and_then(|fp| fp.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn pick_directory_cmd(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri::Manager;
    use tauri_plugin_dialog::{DialogExt, FilePath};
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
    let (tx, rx) = std::sync::mpsc::channel::<Option<FilePath>>();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked.and_then(|fp| fp.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn pick_folders_cmd(app: tauri::AppHandle) -> Result<Option<Vec<String>>, String> {
    use tauri::Manager;
    use tauri_plugin_dialog::{DialogExt, FilePath};
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
    let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<FilePath>>>();
    app.dialog().file().pick_folders(move |paths| {
        let _ = tx.send(paths);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("dialog join failed: {}", e))?;
    let out = match picked {
        None => None,
        Some(paths) => {
            let mut v = Vec::with_capacity(paths.len());
            for fp in paths {
                match fp.into_path() {
                    Ok(p) => v.push(p.to_string_lossy().into_owned()),
                    Err(e) => return Err(format!("FilePath -> PathBuf failed: {}", e)),
                }
            }
            Some(v)
        }
    };
    Ok(out)
}

#[tauri::command]
pub async fn compress_directory_cmd(
    input_dir: String,
    level: String,
) -> Result<(api::DirectoryResult, Vec<u8>), String> {
    let level = CompressionLevel::from_str(&level)
        .map_err(|e| format!("invalid_level: {}", e))?;
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
    let level = CompressionLevel::from_str(&level)
        .map_err(|e| format!("invalid_level: {}", e))?;
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
use crate::p2p_tunnel;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

/// Holds the currently-active send session (if any). There's
/// at most one at a time — the UI's intent switch resets the
/// other side.
pub struct P2pState {
    pub active: Mutex<Option<p2p_tunnel::StartedSend>>,
}

#[derive(Deserialize)]
pub struct P2pSendStartReq {
    /// Absolute path of the file to send.
    pub file_path: String,
    /// User-supplied code, or None to let the server generate one.
    pub code: Option<String>,
}

#[derive(Serialize)]
pub struct P2pSendStartResp {
    /// base64url-encoded token (the thing to share with the
    /// receiver). Includes URL, code, salt, file size, sha256.
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
}

#[tauri::command]
pub async fn p2p_send_start_cmd(
    req: serde_json::Value,
    state: State<'_, Arc<P2pState>>,
) -> Result<P2pSendStartResp, String> {
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
    let app_data_dir = std::env::temp_dir().join("nexus-rar-p2p");
    std::fs::create_dir_all(&app_data_dir)
        .map_err(|e| format!("mkdir app_data_dir: {}", e))?;
    let started = p2p_tunnel::start_sender(file_path.clone(), code.clone(), app_data_dir).await?;
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let file_size = started.token.size;
    let resp = P2pSendStartResp {
        token: started.token_compact.clone(),
        code: started.token.code.clone(),
        filename,
        file_size,
    };
    *state.active.lock().await = Some(started);
    Ok(resp)
}

#[tauri::command]
pub async fn p2p_send_abort_cmd(
    state: State<'_, Arc<P2pState>>,
) -> Result<(), String> {
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
}

#[tauri::command]
pub async fn p2p_receive_cmd(req: serde_json::Value) -> Result<P2pReceiveResp, String> {
    let req: P2pReceiveReq = serde_json::from_value(req)
        .map_err(|e| format!("invalid p2p_receive request: {}", e))?;
    let token = p2p_tunnel::P2pToken::from_compact(&req.token)
        .map_err(|e| format!("invalid token: {}", e))?;
    let output_path = PathBuf::from(&req.output_path);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir output parent: {}", e))?;
    }
    let result = p2p_tunnel::receive_send_file(token, output_path).await?;
    Ok(P2pReceiveResp {
        bytes_written: result.bytes_written,
        output_path: result.output_path.to_string_lossy().to_string(),
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
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir output parent: {}", e))?;
    }
    let result = p2p_tunnel::receive_direct_file(token_v2, output_path, timeout_secs).await?;
    Ok(P2pReceiveResp {
        bytes_written: result.bytes_written,
        output_path: result.output_path.to_string_lossy().to_string(),
    })
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
    use tauri::Manager; // brings .path() into scope on AppHandle
    use p2p_tunnel::p2p_config::{load_tunnel_config, TokenStore};
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
    use tauri::Manager; // brings .path() into scope on AppHandle
    use p2p_tunnel::p2p_config::{
        default_token_store, save_tunnel_config, validate_hostname, validate_token,
        TokenStore, TransportMode, TunnelConfig,
    };
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
    let hostname = req
        .hostname
        .as_deref()
        .map(validate_hostname)
        .transpose()?;
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
