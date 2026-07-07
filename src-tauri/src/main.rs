//! Tauri 2.x entry point for NexusRAR.
//!
//! The Tauri runtime is loaded here. The window config is in
//! `tauri.conf.json`. The IPC commands are registered from
//! `commands.rs`. The Next.js frontend in `../frontend/` is served
//! from `frontendDist` (or `devUrl` during development).

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running NexusRAR Tauri app");
}
