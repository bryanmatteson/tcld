use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn debug_pty_smoke_script_documents_runtime_and_capture_contract() {
    let script = workspace_root().join("scripts/debug-pty-smoke.sh");
    let output = Command::new("bash")
        .arg(script)
        .arg("--help")
        .current_dir(workspace_root())
        .output()
        .expect("run debug PTY smoke help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("Usage: scripts/debug-pty-smoke.sh"));
    assert!(stdout.contains("--features integration_tclsh"));
    assert!(stdout.contains("debug_pty_terminal_contract"));
    assert!(stdout.contains("TCLD_DEBUG_PTY_TCLSH"));
    assert!(stdout.contains("TCLD_DEBUG_PTY_FRAME"));
    assert!(stdout.contains("Tcl 8.5, 8.6, or 9"));
    assert!(stdout.contains("patchlevel"));
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/tcld has workspace grandparent")
        .to_path_buf()
}
