//! Tauri command handlers — the IPC bridge between the UI
//! (in `ui/`) and the synchronous `nexus_compress::api` module.
//!
//! Every command is `async` and uses `spawn_blocking` to keep
//! the Tauri runtime responsive while the CPU-bound compression
//! runs on a Tokio worker thread.
//!
//! ## Why spawn_blocking?
//!
//! Tauri 2.x runs commands on an async runtime. A 100 KB
//! compress call takes ~5-25 ms. Without `spawn_blocking` that
//! would block the runtime for that duration, freezing all
//! other IPC. With `spawn_blocking`, the call moves to a
//! dedicated thread pool and the runtime keeps serving events
//! (UI repaints, other commands, etc.) at 60 FPS.
//!
//! ## Error handling
//!
//! All commands return `Result<T, String>` (Tauri's default
//! error type). The Tauri runtime serializes the error string
//! and ships it to the UI. For a production app we'd use a
//! structured `AppError` type with serde, but for v1 a string
//! is enough — the UI shows it in the telemetry console.

#![cfg(feature = "tauri-app")]

use nexus_compress::api::{
    self, ApiResult, CompressResult, CompressionLevel, DecompressResult, EngineInfo,
    SelfTestResult,
};
use std::str::FromStr;

/// Wrap an `ApiResult<T>` into a `Result<T, String>` for Tauri's
/// IPC. The string error is `format!("{}: {}", code, message)`
/// so the UI can show both the machine code and the message.
fn to_ipc<T>(r: ApiResult<T>) -> Result<T, String> {
    r.map_err(|e| format!("{}: {}", e.code, e.message))
}

/// Compress a byte buffer (Fast / default level).
///
/// # Arguments (from JS)
///
/// ```ts
/// invoke<CompressResult>('compress_bytes_cmd', { input: number[] })
/// ```
///
/// `input` is a `number[]` (u8 values) because Tauri doesn't
/// have a direct `Uint8Array` codec — the UI converts the
/// `File.arrayBuffer()` to a plain array before invoking.
#[tauri::command]
pub async fn compress_bytes_cmd(input: Vec<u8>) -> Result<CompressResult, String> {
    tauri::async_runtime::spawn_blocking(move || api::compress_bytes(&input))
        .await
        .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))
}

/// Decompress a NexusCompress stream.
#[tauri::command]
pub async fn decompress_bytes_cmd(input: Vec<u8>) -> Result<DecompressResult, String> {
    tauri::async_runtime::spawn_blocking(move || to_ipc(api::decompress_bytes(&input)))
        .await
        .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))?
}

/// Compress with explicit level selection.
///
/// `level` is a string ("fast" or "premium") parsed via
/// `CompressionLevel::from_str` (the `FromStr` impl is auto-
/// generated from the serde lowercase rename).
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
    .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))
}

/// Engine metadata for the UI's About / settings panel.
#[tauri::command]
pub async fn engine_info_cmd() -> Result<EngineInfo, String> {
    // Cheap, no need to spawn_blocking.
    Ok(api::engine_info())
}

/// Run the canned self-test (8KB of repeating English text).
/// The UI calls this on startup to show a green/red "engine OK"
/// indicator.
#[tauri::command]
pub async fn self_test_cmd() -> Result<SelfTestResult, String> {
    // Also cheap (8KB), no need to spawn_blocking.
    to_ipc(api::self_test())
}

// -----------------------------------------------------------------------
// Directory commands (NXAR archive)
// -----------------------------------------------------------------------

/// Open the OS file picker. Returns the absolute path the user
/// chose, or `None` if they cancelled. The dialog is filtered
/// to .nxar archives (the only archive type we extract).
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
        .pick_file(move |path: Option<FilePath>| {
            let _ = tx.send(path);
        });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("dialog join failed: {}", e))?;
    Ok(picked.and_then(|fp| fp.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

/// Open the OS folder picker. Returns the absolute path the user
/// chose, or `None` if they cancelled.
///
/// The dialog is opened synchronously on the main thread — Tauri
/// 2.x dialog plugin blocks the main thread for the native
/// picker but releases immediately when the user picks or
/// cancels. On macOS the dialog can open behind other windows
/// if the app isn't focused, so we explicitly `set_focus` on the
/// main window first.
#[tauri::command]
pub async fn pick_directory_cmd(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri::Manager;
    use tauri_plugin_dialog::{DialogExt, FilePath};

    // Bring the main window to the foreground so the NSOpenPanel
    // appears on top. Without this on macOS the dialog can open
    // invisibly behind another app and the user thinks the
    // button does nothing.
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }

    let (tx, rx) = std::sync::mpsc::channel::<Option<FilePath>>();
    app.dialog()
        .file()
        .pick_folder(move |path: Option<FilePath>| {
            let _ = tx.send(path);
        });

    // Block the worker thread (NOT the main thread) until the
    // user picks or cancels. The dialog itself runs on the main
    // thread, so this is safe.
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("dialog join failed: {}", e))?;

    // Convert FilePath -> PathBuf. On macOS desktop this is the
    // Path variant; on mobile it would be Url. We only target
    // desktop, but report the conversion failure clearly if it
    // happens.
    let path_str = match picked {
        None => None,
        Some(fp) => match fp.into_path() {
            Ok(p) => Some(p.to_string_lossy().into_owned()),
            Err(e) => return Err(format!("FilePath -> PathBuf failed: {}", e)),
        },
    };
    Ok(path_str)
}

/// Compress a directory into an NXAR archive. The Rust side walks
/// the directory recursively, compresses each file with the v4
/// engine, and returns aggregate stats + the archive bytes (as a
/// `Vec<u8>` — Tauri serializes this as a plain JS array).
#[tauri::command]
pub async fn compress_directory_cmd(
    input_dir: String,
    level: String,
) -> Result<(api::DirectoryResult, Vec<u8>), String> {
    let level = CompressionLevel::from_str(&level)
        .map_err(|e| format!("invalid_level: {}", e))?;
    let path = std::path::PathBuf::from(input_dir);
    tauri::async_runtime::spawn_blocking(move || {
        to_ipc(api::compress_directory(&path, level).map(|(r, a)| (r, a.to_vec())))
    })
    .await
    .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))?
}

/// Compress multiple directories into a single NXAR archive.
/// Each root's files are stored under a top-level folder named
/// after the root's leaf (e.g. `/a/x.txt` and `/b/y.txt` become
/// `a/x.txt` and `b/y.txt` in the archive). Returns aggregate
/// stats + the archive bytes.
#[tauri::command]
pub async fn compress_directories_cmd(
    input_dirs: Vec<String>,
    level: String,
) -> Result<(api::DirectoryResult, Vec<u8>), String> {
    let level = CompressionLevel::from_str(&level)
        .map_err(|e| format!("invalid_level: {}", e))?;
    let paths: Vec<std::path::PathBuf> = input_dirs.into_iter()
        .map(std::path::PathBuf::from)
        .collect();
    tauri::async_runtime::spawn_blocking(move || {
        let path_refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
        to_ipc(api::compress_directories(&path_refs, level).map(|(r, a)| (r, a.to_vec())))
    })
    .await
    .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))?
}

/// Read a file's bytes from disk. Used by the drag-drop file
/// flow to load a single dropped file into Rust memory (not JS)
/// so we don't blow up the JS heap with a 318 MB Uint8Array.
/// Returns the raw bytes (NOT base64 — Tauri serializes Vec<u8>
/// as a plain JS array, which is fast).
#[tauri::command]
pub async fn read_file_cmd(path: String) -> Result<Vec<u8>, String> {
    let p = std::path::PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(&p).map_err(|e| format!("read_file failed for {}: {}", p.display(), e))
    })
    .await
    .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))?
}

/// Open the OS folder picker in multi-select mode. Returns the
/// absolute paths the user chose (one per picked folder), or
/// `None` if they cancelled. On macOS the NSOpenPanel is
/// configured to allow multiple folder selection.
#[tauri::command]
pub async fn pick_folders_cmd(app: tauri::AppHandle) -> Result<Option<Vec<String>>, String> {
    use tauri::Manager;
    use tauri_plugin_dialog::{DialogExt, FilePath};

    // Bring the main window to the foreground so the NSOpenPanel
    // appears on top. Without this on macOS the dialog can open
    // invisibly behind another app and the user thinks the
    // button does nothing.
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }

    let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<FilePath>>>();
    app.dialog()
        .file()
        .pick_folders(move |paths: Option<Vec<FilePath>>| {
            let _ = tx.send(paths);
        });

    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("dialog join failed: {}", e))?;

    let path_strs = match picked {
        None => None,
        Some(paths) => {
            let mut out = Vec::with_capacity(paths.len());
            for fp in paths {
                match fp.into_path() {
                    Ok(p) => out.push(p.to_string_lossy().into_owned()),
                    Err(e) => return Err(format!("FilePath -> PathBuf failed: {}", e)),
                }
            }
            Some(out)
        }
    };
    Ok(path_strs)
}

/// Decompress an NXAR archive into `output_dir`.
#[tauri::command]
pub async fn decompress_directory_cmd(
    archive: Vec<u8>,
    output_dir: String,
) -> Result<api::DirectoryResult, String> {
    let path = std::path::PathBuf::from(output_dir);
    tauri::async_runtime::spawn_blocking(move || {
        to_ipc(api::decompress_directory(&archive, &path))
    })
    .await
    .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))?
}

/// Peek at an NXAR archive by reading from a FILE PATH (not via IPC).
///
/// This is the fast path: the file is read from disk in Rust, so
/// the entire archive bytes never cross the IPC boundary. For a
/// 318 MB archive, the IPC-based peek would JSON-serialize 318 MB
/// of bytes (~30 seconds); this version is just a 318 MB read
/// from disk (~0.3 seconds on SSD).
#[tauri::command]
pub async fn peek_archive_file_cmd(path: String) -> Result<Vec<api::ArchiveEntry>, String> {
    let p = std::path::PathBuf::from(path);
    tauri::async_runtime::spawn_blocking(move || to_ipc(api::peek_archive_file(&p)))
        .await
        .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))?
}

/// Peek + extract in one call. Reads the archive from `path`,
/// peeks the manifest, and (if `extract_to` is Some) extracts
/// to that directory. Returns the manifest + the extract
/// statistics.
#[tauri::command]
pub async fn peek_and_extract_file_cmd(
    path: String,
    extract_to: Option<String>,
) -> Result<(Vec<api::ArchiveEntry>, Option<api::DirectoryResult>), String> {
    let p = std::path::PathBuf::from(path);
    let out = extract_to.map(std::path::PathBuf::from);
    tauri::async_runtime::spawn_blocking(move || {
        to_ipc(api::peek_and_extract_file(&p, out.as_deref()))
    })
    .await
    .map_err(|e| format!("internal: spawn_blocking join failed: {}", e))?
}

/// Peek at an NXAR archive's manifest. Returns the file list
/// (paths + sizes) without loading or extracting any payload.
/// Used by the UI to populate the archive contents preview.
#[tauri::command]
pub async fn peek_archive_cmd(archive: Vec<u8>) -> Result<api::DirectoryResult, String> {
    // No spawn_blocking needed — peek is O(n) over the header,
    // which is small (a few KB even for 10k entries).
    to_ipc(api::peek_archive(&archive))
}

/// Open a path in the OS file manager (Finder / Explorer /
/// xdg-open). Used by the UI's "open extracted folder" button.
#[tauri::command]
pub async fn open_path_cmd(path: String) -> Result<(), String> {
    to_ipc(api::open_path(&path))
}
