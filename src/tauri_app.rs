//! Tauri 2.x entry point for the NexusRAR desktop GUI.
//!
//! This binary wraps the `nexus_compress::api` module in async
//! Tauri commands. The UI lives in `../ui/` and is loaded by
//! Tauri at startup.
//!
//! ## Architecture
//!
//! ```text
//!   ┌────────────┐     IPC      ┌──────────────┐
//!   │  UI (HTML) │ ───────────► │  Tauri cmd   │
//!   │  (ui/)     │              │  (this file) │
//!   └────────────┘              └──────┬───────┘
//!                                      │
//!                                      ▼
//!                              ┌──────────────┐
//!                              │  api.rs      │
//!                              │  (sync)      │
//!                              └──────┬───────┘
//!                                     │
//!                                     ▼
//!                              ┌──────────────┐
//!                              │  codec.rs    │
//!                              │  (engine)    │
//!                              └──────────────┘
//! ```
//!
//! All commands use `tokio::task::spawn_blocking` (Tauri 2.x's
//! default) to keep the async runtime responsive while the
//! CPU-bound compression runs on a worker thread.

#![cfg(feature = "tauri-app")]

mod tauri_commands;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            tauri_commands::compress_bytes_cmd,
            tauri_commands::decompress_bytes_cmd,
            tauri_commands::compress_bytes_with_level_cmd,
            tauri_commands::engine_info_cmd,
            tauri_commands::self_test_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running NexusRAR Tauri app");
}
