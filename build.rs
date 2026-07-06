fn main() {
    // Only invoke `tauri_build::build()` for the `nexus-rar` binary.
    // For all other binaries (lib, CLI tools, examples, tests),
    // skip Tauri build steps — they're not needed and the
    // `tauri-build` proc macros complain when invoked without
    // a Tauri-aware context.
    if std::env::var("CARGO_BIN_NAME").as_deref() == Ok("nexus-rar") {
        tauri_build::build()
    }
}
