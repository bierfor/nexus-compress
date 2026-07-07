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
    CompressTargetResult, DecompressResult, DecompressTargetResult, EngineInfo, SelfTestResult,
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
/// `{ req: { path, backend, lzma_level } }`.
#[tauri::command]
pub async fn compress_target_cmd(req: serde_json::Value) -> Result<CompressTargetResult, String> {
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
    let backend = CompressionBackend::from_str(backend_str)
        .map_err(|e| format!("invalid_backend: {}", e))?;
    let p = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || {
        to_ipc(api::compress_target(&p, backend, lzma_level))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {}", e))?
}

/// Decompress an archive by path. Auto-detects the format from the
/// file's magic bytes (NXS6 / NXAR / v4 / v5-v6 single) and restores
/// the contents next to the input. The frontend passes
/// `{ path }`.
#[tauri::command]
pub async fn decompress_target_cmd(
    req: serde_json::Value,
) -> Result<DecompressTargetResult, String> {
    let path = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing 'path' in req".to_string())?
        .to_string();
    let p = PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || to_ipc(api::decompress_target(&p)))
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
    app.dialog()
        .file()
        .add_filter("NexusRAR archive", &["nxar", "nxr"])
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
