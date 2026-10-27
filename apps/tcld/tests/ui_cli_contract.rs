//! `tcld ui` process-level contract. The window itself is verified manually
//! from a `--features native-ui` build; these cases never open one.

use std::process::{Command, Output};

fn tcld(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tcld"))
        .args(args)
        .output()
        .expect("tcld runs")
}

#[test]
fn ui_is_listed_in_top_level_help() {
    let output = tcld(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| line.trim_start().starts_with("ui ")
                && line.contains("Open the native desktop workbench")),
        "{stdout}"
    );
}

#[test]
fn ui_rejects_a_missing_file_before_opening_a_window() {
    let output = tcld(&["ui", "definitely/not/here.tbc"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ui: cannot open definitely/not/here.tbc: no such file"),
        "{stderr}"
    );
}

#[cfg(not(feature = "native-ui"))]
#[test]
fn ui_without_native_ui_feature_exits_3_with_rebuild_instructions() {
    let output = tcld(&["ui"]);
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("ui_unavailable:"), "{stderr}");
    assert!(
        stderr.contains("npm run build --prefix apps/tcld-gui"),
        "{stderr}"
    );
    assert!(
        stderr.contains("cargo build -p tcld --features native-ui"),
        "{stderr}"
    );
}
