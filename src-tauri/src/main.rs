//! Tauri 2.x entry point for NexusRAR.
//!
//! The Tauri runtime is loaded here. The window config is in
//! `tauri.conf.json`. The IPC commands are registered from
//! `commands.rs`. The Next.js frontend in `../frontend/` is served
//! from `frontendDist` (or `devUrl` during development).

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod archive_inspect;
mod commands;
mod db;
mod p2p_config;
mod p2p_tunnel;
mod upnp_hole;

use std::sync::Arc;

fn main() {
    // Sprint 5.5.3 Phase 2: panic recovery for UPnP port
    // mappings. If the sender process panics while a UPnP
    // hole is open, this hook drains the global registry
    // and removes every mapping it can. Best-effort —
    // SIGKILL bypasses everything, but panic-unwind hits
    // this hook.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[nexus-rar] PANIC: {}", info);
        upnp_hole::emergency_cleanup_all();
        eprintln!("[nexus-rar] emergency UPnP cleanup done");
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(commands::P2pState {
            active: Arc::new(tokio::sync::Mutex::new(None)),
        }))
        .manage(Arc::new(tokio::sync::Mutex::new(
            db::open().expect("failed to open local stats db"),
        )))
        .invoke_handler(tauri::generate_handler![
            commands::compress_bytes_cmd,
            commands::decompress_bytes_cmd,
            commands::compress_bytes_with_level_cmd,
            commands::compress_bytes_with_backend_cmd,
            commands::compress_directory_with_backend_cmd,
            commands::compress_target_cmd,
            commands::decompress_target_cmd,
            commands::peek_archive_target_cmd,
            commands::reveal_in_finder_cmd,
            commands::pick_save_location_cmd,
            commands::backend_info_cmd,
            commands::engine_info_cmd,
            commands::self_test_cmd,
            commands::pick_file_cmd,
            commands::pick_files_cmd,
            commands::pick_directory_cmd,
            commands::pick_folders_cmd,
            commands::compress_directory_cmd,
            commands::compress_directories_cmd,
            commands::decompress_directory_cmd,
            commands::peek_archive_cmd,
            commands::peek_archive_file_cmd,
            commands::peek_and_extract_file_cmd,
            commands::read_file_cmd,
            commands::open_path_cmd,
            commands::p2p_send_start_cmd,
            commands::p2p_send_abort_cmd,
            commands::p2p_receive_cmd,
            commands::p2p_receive_direct_cmd,
            commands::p2p_peek_filename_cmd,
            commands::p2p_get_tunnel_config_cmd,
            commands::p2p_save_tunnel_config_cmd,
            commands::p2p_archive_list_cmd,
            commands::p2p_archive_extract_cmd,
            commands::get_stats_cmd,
            commands::get_recent_events_cmd,
            commands::data_dir_cmd,
            commands::reset_stats_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running NexusRAR Tauri app");
}
