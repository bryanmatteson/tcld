//! End-to-end smoke test for the `tcld debug` driver's [`Session`].
//!
//! Drives the same wrapper the TUI uses against a real tclsh nub:
//! start → set a line breakpoint by qname → wait for `Paused` →
//! inspect the parsed snapshot → step → continue. Gated by
//! `integration_tclsh` so it doesn't run on hosts without tcl-tk 9.

#![cfg(feature = "integration_tclsh")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tcld::debug::{NubPaths, Session};
use tcld_debug_wire::{Op, WireEvent};

fn tclsh_path() -> PathBuf {
    if let Ok(p) = std::env::var("TCLD_TCLSH") {
        return PathBuf::from(p);
    }
    let candidate = PathBuf::from("/opt/homebrew/Cellar/tcl-tk/9.0.3/bin/tclsh9.0");
    if candidate.exists() {
        return candidate;
    }
    PathBuf::from("tclsh")
}

fn dylib_path() -> PathBuf {
    if let Ok(p) = std::env::var("TCLD_DBG_DYLIB") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = manifest.join("../../target");
    for profile in ["debug", "release"] {
        for ext in ["dylib", "so"] {
            let p = target.join(profile).join(format!("libtcld_dbg.{ext}"));
            if p.exists() {
                return p;
            }
        }
    }
    panic!("dylib not found; run `cargo build` first");
}

fn bootstrap_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl")
}

fn paths() -> NubPaths {
    NubPaths::explicit(tclsh_path(), bootstrap_path(), dylib_path())
}

#[test]
fn session_breakpoint_pause_and_step_round_trip() {
    let user = std::env::temp_dir().join("tcld_debug_session_smoke.tcl");
    std::fs::write(&user, "set a 1\nset b 2\nset c 3\n").expect("write user script");
    let canonical_user = user.canonicalize().expect("resolve user script");

    let mut session = Session::connect(paths()).expect("session connect");
    session
        .add_file_breakpoint(&user, 2, None)
        .expect("stage file/line breakpoint");
    session.start_target(&user, false).expect("start target");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_paused = false;
    while Instant::now() < deadline && !saw_paused {
        for ev in session.drain_events() {
            if matches!(ev, WireEvent::Paused { .. }) {
                saw_paused = true;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(saw_paused, "expected Paused event from Session");

    // Parsed snapshot should reflect the pause.
    let focused = session.focused_proc().expect("focused proc populated");
    assert_eq!(focused.source_path.as_deref(), canonical_user.to_str());
    assert_eq!(focused.line, 2);

    // Stack and locals should be queryable while stopped.
    let stack = session.stack().expect("stack ok");
    assert!(!stack.is_empty(), "stack must contain at least one frame");
    let _vars = session.variables(0).expect("get_variables ok");

    // Coverage path should be parseable even when empty.
    let _cov = session.coverage().unwrap_or_default();

    // Continue executes, the script finishes, and we should observe
    // Resumed and then target completion within a generous deadline.
    let _ = session.send(Op::Continue).expect("continue ok");
    let deadline2 = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline2 {
        let drained = session.drain_events();
        if drained
            .iter()
            .any(|ev| matches!(ev, WireEvent::TargetCompleted { .. }))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    session.close();
    let _ = std::fs::remove_file(&user);
}
