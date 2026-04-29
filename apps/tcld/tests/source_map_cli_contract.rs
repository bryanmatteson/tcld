use std::process::Command;

#[test]
fn source_map_command_prints_root_and_nested_bcproc_paths() {
    let binary = env!("CARGO_BIN_EXE_tcld");
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testing/fixtures/public/public_complex_fixture.tbc");
    let output = Command::new(binary)
        .arg("source-map")
        .arg(fixture)
        .output()
        .expect("source-map command runs");

    assert!(
        output.status.success(),
        "source-map failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("source-map stdout is utf8");
    assert!(
        stdout.lines().any(|line| line.contains("kind=root")
            && line.contains("path=c")
            && line.contains("fixture::startlog")),
        "{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line.contains("kind=nested")
            && line.contains("/n")
            && line.contains("parent=c")
            && line.contains("variable baseDir")),
        "{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line.contains("kind=nested")
            && line.contains("/n")
            && line.contains("parent=c")
            && line.contains("global fixture_startlog")),
        "{stdout}"
    );
}
