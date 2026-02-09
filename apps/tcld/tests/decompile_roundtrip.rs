//! Round-trip guard: every fixture's decompiled output must be a
//! syntactically complete Tcl script.
//!
//! The audit-era golden tests assert decompiled text matches an expected
//! snapshot; they do NOT assert the text is *runnable*. public_complex_fixture.tbc exposed the
//! gap — the decompiler emitted string literals with unbalanced braces
//! (`{{}`, `"{$x\}"`) that are valid as compiled bytecode literals but do
//! not re-parse as Tcl source, so `tcld debug <file>.tbc` (decompile → nub
//! sources it) died with `missing close-brace`.
//!
//! `info complete` is exactly the brace/quote/bracket-balance check Tcl's
//! parser applies; a complete script is one the parser accepts (it says
//! nothing about whether commands *run*, which is right — fixtures
//! reference undefined vars). Gated behind `integration_tclsh` because it
//! needs a real `tclsh`; CI runs it in the tcl-compat matrix.
#![cfg(feature = "integration_tclsh")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tclsh_path() -> PathBuf {
    if let Ok(p) = std::env::var("TCLD_TCLSH") {
        return PathBuf::from(p);
    }
    PathBuf::from("tclsh")
}

/// True if `tclsh` considers `script` a complete (balanced) Tcl script.
fn is_complete(script: &str) -> bool {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("decompiled.tcl");
    std::fs::write(&target, script).expect("write decompiled");
    let driver = dir.path().join("driver.tcl");
    std::fs::write(
        &driver,
        "set f [open [lindex $argv 0] r]\n\
         set s [read $f]\n\
         close $f\n\
         exit [expr {[info complete $s] ? 0 : 3}]\n",
    )
    .expect("write driver");

    let status = Command::new(tclsh_path())
        .arg(&driver)
        .arg(&target)
        .status()
        .expect("run tclsh");
    match status.code() {
        Some(0) => true,
        Some(3) => false,
        other => panic!("tclsh driver exited unexpectedly: {other:?}"),
    }
}

#[test]
fn every_fixture_decompiles_to_a_complete_tcl_script() {
    let fixtures = fixtures_dir();
    let mut tbc_files: Vec<PathBuf> = std::fs::read_dir(&fixtures)
        .unwrap_or_else(|e| panic!("read fixtures dir {fixtures:?}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "tbc"))
        .collect();
    tbc_files.sort();
    assert!(!tbc_files.is_empty(), "no .tbc fixtures found");

    let mut incomplete = Vec::new();
    for tbc in &tbc_files {
        let source = tcld::decompile_path(tbc.to_str().unwrap())
            .unwrap_or_else(|e| panic!("decompile {}: {e}", tbc.display()));
        if !is_complete(&source) {
            incomplete.push(tbc.file_name().unwrap().to_string_lossy().into_owned());
        }
    }

    assert!(
        incomplete.is_empty(),
        "decompiled output is not a complete Tcl script (unbalanced \
         braces/quotes) for: {incomplete:?} — the decompiler must emit \
         re-parseable source"
    );
}
