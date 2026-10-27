#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

fn main() {
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
