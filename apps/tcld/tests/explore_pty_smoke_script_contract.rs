use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn explore_pty_smoke_script_explains_usage_and_frame_capture() {
    let script = workspace_root().join("scripts/explore-pty-smoke.sh");

    let output = Command::new("bash")
        .arg(script)
        .arg("--help")
        .current_dir(workspace_root())
        .output()
        .expect("run explore pty smoke help");

    assert!(
        output.status.success(),
        "help should exit successfully\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("help is utf-8");
    assert!(stdout.contains("Usage: scripts/explore-pty-smoke.sh [INPUT]"));
    assert!(stdout.contains("cargo test -p tcld --test explore_pty_terminal_contract"));
    assert!(stdout.contains("TCLD_EXPLORE_PTY_FRAME"));
    assert!(stdout.contains("defaults to testing/fixtures/public/public_complex_fixture.tbc"));
}

fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("apps/tcld has workspace grandparent")
        .to_path_buf()
}
