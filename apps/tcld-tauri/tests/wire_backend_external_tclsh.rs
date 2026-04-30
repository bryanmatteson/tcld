//! External (wire) debugger backend against a real `tclsh` nub.
//!
//! Gated behind `integration_tclsh` (same pattern as
//! `crates/tcld-dbg-ext`) because it needs:
//!   - a Tcl 9 `tclsh` (TCLD_TCLSH env, or discovery)
//!   - `libtcld_dbg.{dylib,so}` built (`cargo build -p tcld-dbg-ext`)
//!
//! Run with:
//! ```text
//! cargo build -p tcld-dbg-ext
//! TCLD_TCLSH=/opt/homebrew/bin/tclsh9.0 \
//!   cargo test -p tcld-tauri --features integration_tclsh --test wire_backend_external_tclsh
//! ```

#![cfg(all(feature = "debugger", feature = "integration_tclsh"))]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tcld_tauri::debugger::protocol::{AgentCommand, AgentEvent};
use tcld_tauri::debugger::remote::{
    STEP_AT_GATE_ERROR, WireBackend, WireLaunchConfig, locate_nub_paths,
};

const EVENT_WAIT: Duration = Duration::from_secs(10);

/// Fixture with a known line layout:
/// 1: comment, 2: proc header, 3-5: demo body, 6: close, 7: call, 8: top-level.
const SCRIPT: &str = "# wire backend integration fixture\n\
                      proc demo {} {\n    \
                          set a 1\n    \
                          set b 2\n    \
                          set c 3\n\
                      }\n\
                      demo\n\
                      set done 1\n";

fn write_fixture(name: &str) -> String {
    let path = std::env::temp_dir().join(format!("tcld-wire-it-{name}-{}.tcl", std::process::id()));
    std::fs::write(&path, SCRIPT).expect("write fixture script");
    // Canonicalize like the debug actor does before launching.
    path.canonicalize()
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn recv_event(rx: &mut tokio::sync::mpsc::Receiver<AgentEvent>, what: &str) -> AgentEvent {
    let deadline = Instant::now() + EVENT_WAIT;
    loop {
        match rx.try_recv() {
            Ok(event) => return event,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {what} within {EVENT_WAIT:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("event channel closed while waiting for {what}");
            }
        }
    }
}

/// Skip Output/other chatter and return the next event of interest.
fn recv_significant(rx: &mut tokio::sync::mpsc::Receiver<AgentEvent>, what: &str) -> AgentEvent {
    let deadline = Instant::now() + EVENT_WAIT;
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what} within {EVENT_WAIT:?}"
        );
        match recv_event(rx, what) {
            AgentEvent::Output { .. } => continue,
            other => return other,
        }
    }
}

fn expect_stopped(event: AgentEvent, expected_reason: &str, what: &str) -> (String, u32) {
    match event {
        AgentEvent::Stopped {
            reason, file, line, ..
        } => {
            assert_eq!(reason, expected_reason, "{what}: unexpected stop reason");
            (file, line)
        }
        other => panic!("{what}: expected Stopped, got {other:?}"),
    }
}

fn expect_resumed(event: AgentEvent, what: &str) {
    match event {
        AgentEvent::Resumed { .. } => {}
        other => panic!("{what}: expected Resumed, got {other:?}"),
    }
}

fn var_value(vars: &[serde_json::Value], name: &str) -> Option<String> {
    vars.iter()
        .find(|v| v.get("name").and_then(|n| n.as_str()) == Some(name))
        .and_then(|v| v.get("value").and_then(|x| x.as_str()))
        .map(str::to_string)
}

#[test]
fn external_backend_full_session_breakpoint_step_continue_exit() {
    let script_path = write_fixture("session");
    let paths = locate_nub_paths(None, None).expect(
        "nub paths (needs TCLD_TCLSH or a discoverable tclsh, and a built \
         libtcld_dbg — run `cargo build -p tcld-dbg-ext`)",
    );

    let mut backend = WireBackend::launch(WireLaunchConfig {
        script_path: script_path.clone(),
        args: Vec::new(),
        env: HashMap::new(),
        cwd: None,
        paths,
    })
    .expect("wire backend launch");
    let mut rx = backend.take_event_rx();

    // 1. Ready with a real Tcl version and pid.
    match recv_significant(&mut rx, "Ready") {
        AgentEvent::Ready {
            tcl_version, pid, ..
        } => {
            assert!(
                tcl_version.starts_with("9.")
                    || tcl_version.starts_with("8.6")
                    || tcl_version.starts_with("8.5"),
                "unexpected tcl version {tcl_version:?}"
            );
            assert!(pid > 0);
        }
        other => panic!("expected Ready first, got {other:?}"),
    }

    // 2. Synthetic entry stop at the script's first statement, while the
    // nub holds the launch gate.
    let (entry_file, entry_line) = expect_stopped(
        recv_significant(&mut rx, "entry stop"),
        "step",
        "entry stop",
    );
    assert_eq!(entry_file, script_path);
    assert_eq!(entry_line, 2, "first statement is the proc definition");

    // 3. Not-supported shapes at the gate: stepping, and breakpoints the
    // wire cannot address (top-level lines).
    let err = backend
        .send(&AgentCommand::StepInto)
        .expect_err("step at the gate is a defined error");
    assert_eq!(err, STEP_AT_GATE_ERROR);

    let err = backend
        .send(&AgentCommand::SetBreakpoint {
            file: script_path.clone(),
            line: 8,
            offset: None,
            len: None,
            condition: None,
        })
        .expect_err("top-level breakpoint is a defined error");
    assert!(
        err.contains("not supported by external backend"),
        "unexpected error shape: {err}"
    );
    assert!(err.contains("outside a proc body"), "unexpected: {err}");

    // 4. Breakpoint inside the proc body (line 3: `set a 1`), set before
    // the proc even exists in the nub — the placeholder block must
    // survive the real definition.
    backend
        .send(&AgentCommand::SetBreakpoint {
            file: script_path.clone(),
            line: 3,
            offset: None,
            len: None,
            condition: None,
        })
        .expect("breakpoint in proc body resolves");

    // 5. Continue from the gate → breakpoint pause at line 3.
    backend.send(&AgentCommand::Continue).expect("continue");
    expect_resumed(
        recv_significant(&mut rx, "resumed after continue"),
        "continue",
    );
    let (file, line) = expect_stopped(
        recv_significant(&mut rx, "breakpoint pause"),
        "breakpoint",
        "breakpoint pause",
    );
    assert_eq!(file, script_path);
    assert_eq!(line, 3);

    // 6. Variables at the breakpoint (before `set a 1` runs: no locals).
    backend
        .send(&AgentCommand::GetVariables {
            request_id: 1,
            level: 1,
        })
        .expect("get variables");
    let vars = match recv_significant(&mut rx, "variables") {
        AgentEvent::Variables { vars, .. } => vars,
        other => panic!("expected Variables, got {other:?}"),
    };
    assert!(
        var_value(&vars, "a").is_none(),
        "no locals expected before the first statement runs: {vars:?}"
    );

    // 7. step_into → next statement (line 4), `a` now visible.
    backend.send(&AgentCommand::StepInto).expect("step_into");
    expect_resumed(
        recv_significant(&mut rx, "resumed after step_into"),
        "step_into",
    );
    let (_, line) = expect_stopped(
        recv_significant(&mut rx, "step_into pause"),
        "step",
        "step_into pause",
    );
    assert_eq!(line, 4);

    backend
        .send(&AgentCommand::GetVariables {
            request_id: 2,
            level: 1,
        })
        .expect("get variables");
    let vars = match recv_significant(&mut rx, "variables after step") {
        AgentEvent::Variables { vars, .. } => vars,
        other => panic!("expected Variables, got {other:?}"),
    };
    assert_eq!(var_value(&vars, "a").as_deref(), Some("1"));

    // 8. step_over → line 5 (exercises the D9 step-anchor fix through
    // this backend: the follow-up pause must actually arrive).
    backend.send(&AgentCommand::StepOver).expect("step_over");
    expect_resumed(
        recv_significant(&mut rx, "resumed after step_over"),
        "step_over",
    );
    let (_, line) = expect_stopped(
        recv_significant(&mut rx, "step_over pause"),
        "step",
        "step_over pause",
    );
    assert_eq!(line, 5);

    // 9. Eval and set_variable in the paused frame.
    backend
        .send(&AgentCommand::Eval {
            request_id: 7,
            script: "expr {2 + 3}".into(),
            level: 0,
        })
        .expect("eval");
    match recv_significant(&mut rx, "eval result") {
        AgentEvent::EvalResult {
            request_id,
            result,
            error,
            ..
        } => {
            assert_eq!(request_id, 7);
            assert!(!error);
            assert_eq!(result, "5");
        }
        other => panic!("expected EvalResult, got {other:?}"),
    }

    backend
        .send(&AgentCommand::SetVariable {
            name: "b".into(),
            value: "42".into(),
            level: 1,
        })
        .expect("set variable");
    backend
        .send(&AgentCommand::GetVariables {
            request_id: 3,
            level: 1,
        })
        .expect("get variables");
    let vars = match recv_significant(&mut rx, "variables after set") {
        AgentEvent::Variables { vars, .. } => vars,
        other => panic!("expected Variables, got {other:?}"),
    };
    assert_eq!(var_value(&vars, "b").as_deref(), Some("42"));

    // 10. Continue to completion: demo returns, the top level finishes,
    // and the generated launcher exits the nub cleanly.
    backend
        .send(&AgentCommand::Continue)
        .expect("final continue");
    expect_resumed(recv_significant(&mut rx, "final resume"), "final resume");
    match recv_significant(&mut rx, "clean exit") {
        AgentEvent::Exited { code, .. } => assert_eq!(code, 0, "nub should exit cleanly"),
        other => panic!("expected Exited, got {other:?}"),
    }

    backend.stop();
    let _ = std::fs::remove_file(&script_path);
}

#[test]
fn external_backend_missing_dylib_reports_actionable_diagnostic() {
    let err = match locate_nub_paths(None, Some("/nonexistent/libtcld_dbg.dylib")) {
        Err(err) => err,
        Ok(_) => panic!("missing dylib override must fail"),
    };
    assert!(err.contains("/nonexistent/libtcld_dbg.dylib"), "{err}");
}

#[test]
fn external_backend_eval_reports_tcl_errors() {
    let script_path = write_fixture("eval-error");
    let paths = locate_nub_paths(None, None).expect("nub paths");
    let mut backend = WireBackend::launch(WireLaunchConfig {
        script_path: script_path.clone(),
        args: Vec::new(),
        env: HashMap::new(),
        cwd: None,
        paths,
    })
    .expect("wire backend launch");
    let mut rx = backend.take_event_rx();

    match recv_significant(&mut rx, "Ready") {
        AgentEvent::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }
    expect_stopped(
        recv_significant(&mut rx, "entry stop"),
        "step",
        "entry stop",
    );

    // Eval while paused at the gate: errors round-trip with error info.
    backend
        .send(&AgentCommand::Eval {
            request_id: 11,
            script: "error {boom from eval}".into(),
            level: 0,
        })
        .expect("eval send");
    match recv_significant(&mut rx, "eval error result") {
        AgentEvent::EvalResult {
            request_id,
            result,
            error,
            error_info,
            ..
        } => {
            assert_eq!(request_id, 11);
            assert!(error, "eval of `error` must report an error");
            assert_eq!(result, "boom from eval");
            assert!(
                error_info.as_deref().is_some_and(|i| i.contains("boom")),
                "errorInfo should carry the traceback: {error_info:?}"
            );
        }
        other => panic!("expected EvalResult, got {other:?}"),
    }

    backend.stop();
    let _ = std::fs::remove_file(&script_path);
}
