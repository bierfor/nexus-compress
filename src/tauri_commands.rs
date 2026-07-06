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
