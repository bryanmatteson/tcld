#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() {
    // Probe the installed engines without opening a webview.
    if std::env::args().nth(1).as_deref() == Some("--distribution-smoke-test") {
        if let Err(error) = tcld_tauri::distribution_smoke_test() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    // `tcld-tauri [FILE]`: the optional file mirrors `tcld ui [FILE]`. Flags
    // are skipped so launcher-injected arguments (e.g. macOS `-psn_*`) are
    // never mistaken for a path.
    let open_path = std::env::args_os()
        .skip(1)
        .find(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(PathBuf::from)
        .map(|path| std::path::absolute(&path).unwrap_or(path));
    if let Err(error) = tcld_tauri::run(tcld_tauri::LaunchOptions { open_path }) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
