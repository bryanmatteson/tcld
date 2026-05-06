//! D3 guard: when the *target* script fails to load in the nub, the
//! debugger surfaces it distinctly instead of connecting to a corpse.
//!
//! The nub bootstrap sources the user script; a parse or load-time error
//! there used to kill the tclsh process right after a successful
//! handshake, so the CLI debugger reported "session started" and then
//! silently did nothing. The bootstrap now catches the error, records a
//! load status, and stays alive; `Session::start` reads that status and
//! returns a `LOAD_FAILURE_PREFIX` error.
//!
//! Gated behind `integration_tclsh`: needs a real tclsh and the built
//! `libtcld_dbg` dylib (CI's tcl-compat matrix).
#![cfg(feature = "integration_tclsh")]

use std::io::Write;

use std::time::{Duration, Instant};

use tcld::debug::{Session, locate_paths};
use tcld_debug_wire::WireEvent;

fn write_temp(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tcld_d3_{}_{}", std::process::id(), name));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.tcl"));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    path
}

/// A parse error (unbalanced braces) is a load failure under any Tcl
/// version — the exact class the decompiler used to emit.
#[test]
fn unparseable_target_reports_load_failure_not_a_dead_session() {
    let script = write_temp("unparseable", "proc p {} {\n    lappend x \"{a\\}\"\n}\n");
    let paths = locate_paths(None, None, None).expect("locate nub paths");

    let mut session = Session::connect(paths).expect("connect Ready session");
    session
        .start_target(&script, false)
        .expect("schedule target");
    let deadline = Instant::now() + Duration::from_secs(5);
    let failure = loop {
        assert!(
            Instant::now() < deadline,
            "target-load-failure event timed out"
        );
        if let Some(event) = session
            .drain_events()
            .into_iter()
            .find(|event| matches!(event, WireEvent::TargetLoadFailed { .. }))
        {
            break event;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let WireEvent::TargetLoadFailed {
        message,
        error_info,
        ..
    } = failure
    else {
        unreachable!()
    };
    assert!(!message.is_empty());
    assert!(error_info.contains("unparseable.tcl"));
    session.close();
}

/// A well-formed script that defines a proc must still start cleanly —
/// the load-status check must not produce false positives.
#[test]
fn well_formed_target_starts_cleanly() {
    let script = write_temp("ok", "proc greet {} { return hi }\n");
    let paths = locate_paths(None, None, None).expect("locate nub paths");

    let mut session = Session::connect(paths).expect("connect Ready session");
    session
        .start_target(&script, false)
        .expect("schedule target");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if session
            .drain_events()
            .iter()
            .any(|event| matches!(event, WireEvent::TargetCompleted { .. }))
        {
            session.close();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("well-formed target did not complete");
}

#[test]
fn corrupt_raw_nub_override_reports_runtime_and_loader_context() {
    let discovered = locate_paths(None, None, None).expect("discover working debugger runtime");
    let tclsh = discovered.tclsh.to_string_lossy().into_owned();
    let patchlevel = discovered.tcl_patchlevel.clone();
    let abi = discovered.tcl_abi.expect("probed Tcl ABI").to_string();
    let extension = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let dir = tempfile::tempdir().expect("create corrupt-nub fixture directory");
    let corrupt = dir.path().join(format!("libtcld_dbg.{extension}"));
    std::fs::write(&corrupt, b"not a native debugger library").unwrap();
    let corrupt_text = corrupt.to_string_lossy().into_owned();
    let paths = locate_paths(Some(&tclsh), Some(&corrupt_text), None)
        .expect("raw override is validated by the real loader");

    let started = Instant::now();
    let error = match Session::connect(paths) {
        Ok(session) => {
            session.close();
            panic!("corrupt raw nub unexpectedly connected")
        }
        Err(error) => error,
    };
    assert!(started.elapsed() < Duration::from_secs(8), "{error}");
    for expected in [&corrupt_text, &tclsh, &patchlevel, &abi, "loader output"] {
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }
}
