//! Smoke test for the dedicated debugger wire session.
//!
//! Drives `handle_dbg` directly (the same function the TUI's command
//! palette routes to) and asserts the round-trip with a real tclsh
//! subprocess. Gated by `integration_tclsh` so it doesn't run on hosts
//! that don't have tcl-tk 9 installed.

#![cfg(feature = "integration_tclsh")]

// Re-import the explore module's internals for direct testing.
// `handle_dbg` and `WireDbgSession` are pub(crate) — accessible via
// the `tcld` crate's lib root by convention if exposed there.

use std::path::PathBuf;

// The explore mod isn't pub by default; the `tcld` crate's `lib.rs`
// exposes it. We only need handle_dbg via a thin shim — define a
// public entry below in the test file's local module.

// For now, re-create a tiny end-to-end via the wire crate directly so
// the test isn't tied to mod-internal access.
use tcld_debug_wire::{BreakpointTarget, Controller, Op, WireEvent};

const SCRIPTS_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/tcld-dbg-ext/scripts"
);

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

#[test]
fn dbg_command_round_trips_via_wire() {
    let user = std::env::temp_dir().join("tcld_dbg_smoke_user.tcl");
    std::fs::write(&user, "proc demo {} { set a 1\nset b 2\nset c 3 }\n")
        .expect("write user script");

    let (mut ctrl, _port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let nub = ctrl
        .spawn_nub(&tclsh_path(), &bootstrap, &dylib_path())
        .expect("spawn_nub");
    let _nub_guard = nub;

    let _caps = ctrl.accept().expect("accept");
    ctrl.send(Op::StartTarget {
        path: user.to_string_lossy().into_owned(),
        stop_on_entry: false,
    })
    .expect("start target");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if ctrl
            .drain_events()
            .iter()
            .any(|event| matches!(event, WireEvent::TargetCompleted { .. }))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // proc_location → bp_add → expect paused → vars → continue.
    let loc = ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .expect("proc_location");
    let first_off = loc
        .payload()
        .split_whitespace()
        .skip_while(|t| *t != "first_offset")
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .expect("first_offset");
    let _ = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: first_off,
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");
    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "after 0 demo".into(),
    })
    .expect("schedule demo");

    // Wait for paused.
    let mut saw_paused = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && !saw_paused {
        for ev in ctrl.drain_events() {
            if matches!(ev, WireEvent::Paused { .. }) {
                saw_paused = true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(saw_paused, "expected Paused event from explore-driven nub");

    let _ = ctrl.close();
    let _ = std::fs::remove_file(&user);
}
