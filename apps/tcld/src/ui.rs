//! `tcld ui`: the native desktop workbench.
//!
//! Like the CLIs of the sibling workbenches, the window is hosted in this
//! process rather than spawned: with `--features native-ui` the `tcld-tauri`
//! app (frontend embedded at compile time) runs on the main thread until its
//! last window closes. Builds without the feature report `ui_unavailable`.

use crate::cli::UiArgs;
use crate::error::{Result, TcldError};
use std::path::{Path, PathBuf};

/// Stable, actionable failure for binaries built without `native-ui`.
pub const UI_UNAVAILABLE_MESSAGE: &str = "ui_unavailable: this tcld was built without the native desktop UI.\n\
Build the frontend, then rebuild tcld with the `native-ui` feature:\n  \
npm ci --prefix apps/tcld-gui && npm run build --prefix apps/tcld-gui\n  \
cargo build -p tcld --features native-ui";

pub fn run_ui(args: UiArgs) -> Result<()> {
    let open_path = args
        .input_path
        .as_deref()
        .map(resolve_open_path)
        .transpose()?;
    launch(open_path)
}

/// Validate FILE before any window opens and make it absolute, so the
/// desktop app (and its recent-files list) never depends on this process's
/// working directory.
fn resolve_open_path(input: &str) -> Result<PathBuf> {
    let path = Path::new(input);
    if !path.is_file() {
        return Err(TcldError::Other(format!(
            "ui: cannot open {input}: no such file"
        )));
    }
    Ok(std::path::absolute(path)?)
}

#[cfg(feature = "native-ui")]
fn launch(open_path: Option<PathBuf>) -> Result<()> {
    tcld_tauri::run(tcld_tauri::LaunchOptions { open_path }).map_err(TcldError::Other)
}

#[cfg(not(feature = "native-ui"))]
fn launch(_open_path: Option<PathBuf>) -> Result<()> {
    Err(TcldError::Other(UI_UNAVAILABLE_MESSAGE.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_rejected_before_launch() {
        let err = run_ui(UiArgs {
            input_path: Some("definitely/not/here.tbc".to_string()),
        })
        .expect_err("missing file must fail");
        assert!(err.to_string().contains("no such file"));
    }

    #[test]
    fn existing_file_resolves_to_an_absolute_path() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        let resolved = resolve_open_path(file.path().to_str().unwrap()).expect("resolves");
        assert!(resolved.is_absolute());
    }

    #[cfg(not(feature = "native-ui"))]
    #[test]
    fn builds_without_native_ui_report_an_actionable_unavailable_error() {
        let err = run_ui(UiArgs { input_path: None }).expect_err("ui is unavailable");
        let message = err.to_string();
        assert!(message.starts_with("ui_unavailable:"));
        assert!(message.contains("cargo build -p tcld --features native-ui"));
        assert!(message.contains("npm run build --prefix apps/tcld-gui"));
    }
}
