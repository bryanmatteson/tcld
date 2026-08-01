#![cfg_attr(not(feature = "integration_tclsh"), allow(dead_code, unused_imports))]

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::json;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const ROWS: u16 = 32;
const COLS: u16 = 120;
const SMALL_ROWS: u16 = 20;
const SMALL_COLS: u16 = 70;

#[test]
#[cfg(feature = "integration_tclsh")]
fn debug_tui_drives_target_lifecycle_under_real_pty() {
    let fixture = fixture("debug_immediate_top_level.tcl");
    let session_dir = tempfile::tempdir().expect("create debug PTY session directory");
    let target = session_dir.path().join("target.tcl");
    fs::copy(&fixture, &target).expect("copy target fixture");
    let target = target.canonicalize().expect("canonical target path");
    let entry_sentinel = session_dir.path().join("entry-ran");
    let breakpoint_sentinel = session_dir.path().join("breakpoint-ran");

    let persisted = json!({
        "version": 1,
        "breakpoints": format!(
            "{{kind file_line path {{{}}} line 2 enabled 1 ignore 0 condition {{}}}}",
            target.display()
        ),
        "watches": []
    });
    fs::write(
        session_dir.path().join(".tcld-debug.tcl"),
        serde_json::to_string_pretty(&persisted).unwrap(),
    )
    .expect("write persisted breakpoint");

    let frame_path = env::var_os("TCLD_DEBUG_PTY_FRAME").map(PathBuf::from);
    let mut pty = DebugPty::spawn_debug(
        &target,
        &entry_sentinel,
        &breakpoint_sentinel,
        frame_path.as_deref(),
    );

    pty.capture("entry_pause", Duration::from_secs(8), |text| {
        require(text, "tcld debug")?;
        if text.contains("screen 1/1") {
            return Err("TurtleTap Shell chrome leaked into the debugger viewport".to_string());
        }
        require(text, "[Paused]")?;
        require(text, "▶     1")?;
        require(text, "exec touch $::env(TCLD_DEBUG_PTY_ENTRY_SENTINEL)")?;
        require(text, "bp0")?;
        require(text, "Source")?;
        require(text, "Stack")?;
        require(text, "Locals")?;
        require(text, "Breakpoints")
    });
    assert_lifecycle_order(pty.lifecycle_states());
    assert!(
        !entry_sentinel.exists(),
        "entry command ran before entry pause"
    );
    assert!(
        !breakpoint_sentinel.exists(),
        "breakpoint command ran before entry pause"
    );

    pty.send(b"?");
    pty.capture("help", Duration::from_secs(2), |text| {
        require(text, "tcld debug — keybindings")?;
        require(text, "Execution")?;
        require(text, "Ctrl-C")?;
        require(text, "continue")?;
        require(text, "step over")
    });
    pty.send(b"\x1b");

    pty.send(b"wflag 40+2\r");
    pty.send(b"R");
    pty.capture("watch_evaluated", Duration::from_secs(3), |text| {
        require(text, "flag = 40+2")?;
        require(text, "42")
    });
    pty.send(b"\x13");
    pty.capture("state_exported", Duration::from_secs(2), |text| {
        require(text, "breakpoints exported")
    });

    pty.send(b"c");
    pty.capture(
        "persisted_breakpoint_pause",
        Duration::from_secs(5),
        |text| {
            require(text, "[Paused]")?;
            require(text, "▶●    2")?;
            require(text, "bp0")
        },
    );
    assert!(
        entry_sentinel.exists(),
        "first command did not run after continue"
    );
    assert!(
        !breakpoint_sentinel.exists(),
        "line-2 command ran before its breakpoint pause"
    );

    pty.send(b"n");
    pty.capture("single_key_step", Duration::from_secs(5), |text| {
        require(text, "[Paused]")?;
        require(text, "▶     3")
    });
    assert!(
        breakpoint_sentinel.exists(),
        "step-over did not execute the paused line"
    );

    pty.send(b"c");
    pty.capture("target_completed", Duration::from_secs(5), |text| {
        require(text, "[Completed]")?;
        require(text, "no frames — pause execution first")
    });
    let error_pauses_before = pty.raw.matches("error_paused").count();
    pty.send(b"n");
    pty.capture(
        "completed_step_rejected_once",
        Duration::from_secs(2),
        |text| {
            require(text, "step over unavailable: debugger is Completed")?;
            require(text, "[Completed]")
        },
    );
    pty.pump(Duration::from_millis(750));
    assert_eq!(
        pty.raw.matches("error_paused").count(),
        error_pauses_before,
        "a rejected debugger command recursively emitted user exception pauses"
    );

    fs::remove_file(&entry_sentinel).expect("clear entry sentinel before restart");
    fs::remove_file(&breakpoint_sentinel).expect("clear breakpoint sentinel before restart");
    pty.send(b"r");
    pty.capture("restart_entry_pause", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "▶     1")?;
        require(text, "flag = 40+2")?;
        require(text, "42")
    });
    assert!(!entry_sentinel.exists(), "restart ran past the entry gate");
    assert!(
        !breakpoint_sentinel.exists(),
        "restart ran past the persisted breakpoint"
    );

    pty.resize(SMALL_ROWS, SMALL_COLS);
    pty.capture("minimum_size_guard", Duration::from_secs(3), |text| {
        require(text, "Terminal too small: 70x20")?;
        require(text, "Required: 80x24")?;
        require(text, "Ctrl-C exit | q quit")
    });
    pty.resize(ROWS, COLS);
    pty.capture("restored_size", Duration::from_secs(3), |text| {
        require(text, "[Paused]")?;
        require(text, "Source")
    });

    pty.send(b"e");
    pty.send(b"1+1");
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "integration_tclsh")]
fn debug_tui_tracks_sourced_proc_and_eval_units_with_durable_pending_intent() {
    let session_dir = tempfile::tempdir().expect("create multi-unit PTY directory");
    let child = session_dir.path().join("child.tcl");
    fs::write(
        &child,
        "namespace eval ::child {}\nset ::child::file_marker child_file\nproc ::child::work {} {\n    set proc_marker child_proc\n    return $proc_marker\n}\n",
    )
    .expect("write sourced child fixture");
    let child = child.canonicalize().expect("canonical child fixture");
    let target = session_dir.path().join("entry.tcl");
    fs::write(
        &target,
        "set here [file dirname [info script]]\nsource [file join $here child.tcl]\n::child::work\neval {set dynamic_marker eval_unit\nset dynamic_result [string length $dynamic_marker]}\nset completed 1\n",
    )
    .expect("write multi-unit entry fixture");
    let target = target.canonicalize().expect("canonical multi-unit entry");

    let persisted = json!({
        "version": 1,
        "breakpoints": format!(
            "{{kind file_line path {{{}}} line 2 enabled 1 ignore 0 condition {{}}}} {{kind line block {{proc:child::work}} offset 5 len 1 enabled 1 ignore 0 temporary 0 condition {{}}}}",
            child.display()
        ),
        "watches": []
    });
    fs::write(
        session_dir.path().join(".tcld-debug.tcl"),
        serde_json::to_string_pretty(&persisted).unwrap(),
    )
    .expect("persist pre-load file and proc breakpoint intent");

    let mut pty = DebugPty::spawn_debug(
        &target,
        Path::new("/tmp/tcld-debug-unused-entry"),
        Path::new("/tmp/tcld-debug-unused-breakpoint"),
        None,
    );
    pty.capture("multi_unit_entry", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "entry.tcl [user script]")?;
        require(text, "bp0")?;
        require(text, "bp1")?;
        require(text, "child.tcl:2 [pending]")?;
        require(text, "child::work:5 [pending]")
    });

    pty.send(b"c");
    pty.capture(
        "sourced_file_pending_resolved",
        Duration::from_secs(8),
        |text| {
            require(text, "[Paused]")?;
            require(text, "child.tcl [sourced file]")?;
            require(text, "bp0")?;
            require(text, "set ::child::file_marker child_file")?;
            require(text, "▶●    2")
        },
    );

    pty.send(b"c");
    pty.capture(
        "dynamic_proc_pending_resolved",
        Duration::from_secs(8),
        |text| {
            require(text, "[Paused]")?;
            require(text, "proc child::work [proc body]")?;
            require(text, "bp1")?;
            require(text, "set proc_marker child_proc")?;
            require(text, "▶●    2")
        },
    );

    pty.send(b"\x1b[23;2~");
    pty.capture("return_to_eval_callsite", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "entry.tcl [user script]")?;
        require(text, "eval {set dynamic_marker eval_unit")
    });
    pty.send(b"\x1b[23~");
    pty.capture("anonymous_eval_identity", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "eval anon:")?;
        require(text, "[eval session-only]")?;
        require(text, "Source · eval (session-only)")?;
        require(text, "set dynamic_marker eval_unit")
    });

    pty.send(b"\x1b[23;2~");
    pty.capture(
        "eval_returns_to_entry_owner",
        Duration::from_secs(8),
        |text| {
            require(text, "[Paused]")?;
            require(text, "entry.tcl [user script]")?;
            require(text, "bp0")?;
            require(text, "bp1")?;
            require(text, "set completed 1")
        },
    );

    pty.send(b"r");
    pty.capture(
        "multi_unit_fresh_restart",
        Duration::from_secs(10),
        |text| {
            require(text, "[Paused]")?;
            require(text, "entry.tcl [user script]")?;
            require(text, "child.tcl:2 [pending]")?;
            require(text, "child::work:5 [pending]")?;
            require(text, "session-only")
        },
    );
    pty.send(b"c");
    pty.capture(
        "fresh_restart_file_line_intent_rebound",
        Duration::from_secs(8),
        |text| {
            require(text, "[Paused]")?;
            require(text, "child.tcl [sourced file]")?;
            require(text, "bp0")?;
            require(text, "set ::child::file_marker child_file")?;
            require(text, "▶●    2")
        },
    );
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "integration_tclsh")]
fn public_bytecode_starts_in_the_real_host_debugger_under_pty() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testing/fixtures/public/public_complex_fixture.tbc")
        .canonicalize()
        .expect("canonical public_complex_fixture.tbc");
    let mut pty = DebugPty::spawn_debug(
        &target,
        Path::new("/tmp/tcld-debug-unused-fixture-entry"),
        Path::new("/tmp/tcld-debug-unused-fixture-breakpoint"),
        None,
    );

    pty.capture("soc_host_entry_pause", Duration::from_secs(15), |text| {
        require(text, "tcld debug")?;
        require(text, "[Paused]")?;
        require(text, "▶     1")?;
        require(text, "package provide tcld_fixture")?;
        if text.contains("strict decompilation failed")
            || text.contains("tcld: unavailable nested bytecode")
        {
            return Err(format!(
                "host debugger admitted approximated public_complex_fixture.tbc source\n{text}"
            ));
        }
        Ok(())
    });

    pty.send(b"\x1b[21~");
    pty.capture(
        "soc_step_over_first_command",
        Duration::from_secs(8),
        |text| {
            require(text, "[Paused]")?;
            require(text, "▶     2")?;
            require(text, "namespace eval fixture")
        },
    );

    pty.send(b"\x1b[23~");
    pty.capture(
        "soc_step_into_namespace_body",
        Duration::from_secs(8),
        |text| {
            require(text, "[Paused]")?;
            require(text, "▶     3")?;
            require(text, "variable baseDir")
        },
    );

    pty.send(b"\x1b[21~");
    pty.capture("soc_step_over_nested_set", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "▶     4")?;
        require(text, "variable debug_types")
    });
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "integration_tclsh")]
fn debug_tui_nested_controls_update_visible_state() {
    let session_dir = tempfile::tempdir().expect("create nested PTY session directory");
    let target = session_dir.path().join("nested.tcl");
    fs::copy(fixture("debug_nested_steps.tcl"), &target).expect("copy nested fixture");
    let target = target.canonicalize().expect("canonical nested fixture");
    let mut pty = DebugPty::spawn_debug(
        &target,
        Path::new("/tmp/tcld-debug-unused-entry"),
        Path::new("/tmp/tcld-debug-unused-breakpoint"),
        None,
    );

    pty.capture("nested_entry_pause", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "▶     1")?;
        require(text, "proc inner")
    });

    pty.send(b"wproc_count [llength [info procs inner]]\r");
    pty.send(b"R");
    pty.capture("nested_watch_before_load", Duration::from_secs(3), |text| {
        require(text, "proc_count")?;
        require(text, "→ 0")
    });

    pty.send(&[b'j'; 7]);
    pty.send(b"b");
    pty.capture(
        "nested_command_breakpoint",
        Duration::from_secs(3),
        |text| {
            require(text, "●")?;
            require(text, "breakpoint @")?;
            require(text, "line 8, inner")?;
            require(text, "bp0")?;
            require(text, "inner")
        },
    );

    pty.send(b"e1+1");
    pty.send(b"\x1b");
    pty.capture("nested_prompt_cancelled", Duration::from_secs(2), |text| {
        require(text, "[Paused]")?;
        if text.contains("eval >") {
            return Err("Esc did not cancel eval prompt".to_string());
        }
        Ok(())
    });

    pty.send(b"\x1b[15~");
    pty.capture("nested_f5_breakpoint", Duration::from_secs(6), |text| {
        require(text, "[Paused]")?;
        require(text, "outer : line 3")?;
        require(text, "outer_value = ready")?;
        require(text, "Δ proc_count")?;
        require(text, "→ 1")?;
        require(text, "paused")
    });

    pty.send(b"4c1\r");
    pty.capture("nested_condition_updated", Duration::from_secs(3), |text| {
        require(text, "if 1")?;
        require(text, "condition updated")
    });

    pty.send(b"1eformat {first\\nsecond}\r");
    pty.capture("nested_multiline_eval", Duration::from_secs(3), |text| {
        require(text, "eval@")?;
        require(text, "first")?;
        require(text, "second")
    });

    pty.send(b"\x1b[23~");
    pty.capture("nested_f11_step_into", Duration::from_secs(6), |text| {
        require(text, "[Paused]")?;
        require(text, "inner : line 2")?;
        require(text, "set inner_value 41")
    });

    pty.send(b"2k");
    pty.capture("nested_parent_frame", Duration::from_secs(3), |text| {
        require(text, "outer_value = ready")?;
        require(text, "inner")
    });
    pty.send(b"j");
    pty.capture(
        "nested_top_frame_restored",
        Duration::from_secs(3),
        |text| {
            require(text, "set inner_value 41")?;
            if text.contains("outer_value = ready") {
                return Err("returning to the top frame kept parent locals".to_string());
            }
            Ok(())
        },
    );

    pty.send(b"\x1b[21~");
    pty.capture("nested_f10_step_over", Duration::from_secs(6), |text| {
        require(text, "[Paused]")?;
        require(text, "inner : line 3")?;
        require(text, "inner_value = 41")
    });

    pty.send(b"\x1b[23;2~");
    pty.capture(
        "nested_shift_f11_step_out",
        Duration::from_secs(6),
        |text| {
            require(text, "[Paused]")?;
            require(text, "outer : line 4")?;
            require(text, "set result 42")
        },
    );
    pty.send(b"c");
    pty.capture("nested_first_completion", Duration::from_secs(5), |text| {
        require(text, "[Completed]")?;
        require(text, "no frames — pause execution first")
    });

    pty.send(b"r");
    pty.capture("nested_restart_entry", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "▶     1")?;
        require(text, "if 1")
    });
    pty.send(b"c");
    pty.capture("nested_c_breakpoint", Duration::from_secs(6), |text| {
        require(text, "[Paused]")?;
        require(text, "outer : line 3")
    });
    pty.send(b"c");
    pty.capture("nested_second_completion", Duration::from_secs(5), |text| {
        require(text, "[Completed]")
    });
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "integration_tclsh")]
fn debug_tui_breaks_on_a_nested_command_sharing_its_source_line() {
    let session_dir = tempfile::tempdir().expect("create command breakpoint PTY directory");
    let target = session_dir.path().join("same-line.tcl");
    fs::write(
        &target,
        "proc demo {} { set value [expr {1 + 1}]; return $value }\ndemo\n",
    )
    .expect("write nested same-line fixture");
    let target = target.canonicalize().expect("canonical same-line fixture");
    let mut pty = DebugPty::spawn_debug(
        &target,
        Path::new("/tmp/tcld-debug-unused-entry"),
        Path::new("/tmp/tcld-debug-unused-breakpoint"),
        None,
    );

    pty.capture("same_line_entry", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "command 1/4: proc")
    });
    pty.send(b"]]");
    pty.capture("same_line_expr_selected", Duration::from_secs(2), |text| {
        require(text, "command 3/4")?;
        require(text, "expr")
    });
    pty.send(b"b");
    pty.capture(
        "same_line_expr_breakpoint",
        Duration::from_secs(3),
        |text| {
            require(text, "breakpoint @ line 1, expr")?;
            require(text, "bp0")
        },
    );
    pty.send(b"c");
    pty.capture("same_line_expr_pause", Duration::from_secs(6), |text| {
        require(text, "[Paused]")?;
        require(text, "demo : line 1")?;
        require(text, "hits=1")
    });
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "integration_tclsh")]
fn debug_load_failure_is_visible_under_real_pty() {
    let mut pty = DebugPty::spawn_debug(
        &fixture("debug_invalid.tcl"),
        Path::new("/tmp/tcld-debug-unused-entry"),
        Path::new("/tmp/tcld-debug-unused-breakpoint"),
        None,
    );
    pty.capture("invalid_entry_pause", Duration::from_secs(8), |text| {
        require(text, "[Paused]")?;
        require(text, "proc broken")
    });
    pty.send(b"c");
    pty.capture("load_failed", Duration::from_secs(8), |text| {
        require(text, "[Load Failed]")?;
        require(text, "missing close-brace")?;
        require(text, "target_load_failed")
    });
    pty.send(b"/error_\r");
    pty.capture("load_failure_details", Duration::from_secs(3), |text| {
        require(text, "error_code=")?;
        require(text, "error_info=")
    });
    pty.send(b"?");
    pty.capture("load_failure_help", Duration::from_secs(2), |text| {
        require(text, "tcld debug — keybindings")?;
        require(text, "Execution")
    });
    pty.send(b"\x1b");
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "integration_tclsh")]
fn debug_panic_is_printed_after_terminal_restore() {
    let mut pty = DebugPty::spawn_shell();
    let (tclsh_wrapper, wrapper_guard) = probe_aware_tclsh();
    let command = format!(
        "{} debug {} --tclsh {} --dylib {} --bootstrap {}\r",
        shell_quote(Path::new(env!("CARGO_BIN_EXE_tcld"))),
        shell_quote(&fixture("debug_nested_steps.tcl")),
        shell_quote(&tclsh_wrapper),
        shell_quote(&dylib_path()),
        shell_quote(&bootstrap_path()),
    );
    pty._tclsh_wrapper = Some(wrapper_guard);
    pty.send(command.as_bytes());
    pty.capture("panic_after_restore", Duration::from_secs(5), |text| {
        require(text, "panic at ")?;
        require(text, "intentional debug terminal panic")
    });

    let probe = "printf 'TCLD_DEBUG_PTY_RESTORED\\n'";
    pty.send(probe.as_bytes());
    pty.capture("shell_echo_restored", Duration::from_secs(2), |text| {
        require(text, probe)
    });
    pty.send(b"\r");
    pty.capture(
        "shell_command_after_panic",
        Duration::from_secs(2),
        |text| require(text, "TCLD_DEBUG_PTY_RESTORED"),
    );
    pty.send(b"exit\r");
    pty.wait_for_exit(Duration::from_secs(2));
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_debug_tui_exposes_dual_code_frames_eval_watch_and_nested_controls() {
    let mut pty = DebugPty::spawn_hybrid_debug(&fixture("proc.tbc"));
    pty.capture("hybrid_entry", Duration::from_secs(8), |text| {
        require(text, "tcld debug")?;
        if text.contains("screen 1/1") {
            return Err("TurtleTap Shell chrome leaked into the hybrid viewport".to_string());
        }
        require(text, "[Paused]")?;
        require(text, "Source · synthetic")?;
        require(text, "Source · companion (unverified)")?;
        require(text, "tbcload::bcproc")
    });

    pty.send(b"b");
    pty.capture(
        "hybrid_instruction_breakpoint",
        Duration::from_secs(3),
        |text| {
            require(text, "set instruction breakpoint")?;
            require(text, "code")?;
            require(text, "pc")
        },
    );

    pty.send(b"e");
    pty.send(b"set hybrid_probe 41\r");
    pty.capture("hybrid_eval", Duration::from_secs(3), |text| {
        require(text, "eval(level=0) code=0")?;
        require(text, "41")
    });

    pty.send(b"w");
    pty.send(b"answer 40+2\r");
    pty.capture("hybrid_watch", Duration::from_secs(3), |text| {
        require(text, "answer = 40+2")?;
        require(text, "42")
    });

    pty.send(b"r");
    pty.capture(
        "hybrid_restart_restores_ui_state",
        Duration::from_secs(4),
        |text| {
            require(text, "[Paused]")?;
            require(text, "answer = 40+2")?;
            require(text, "code:")
        },
    );

    pty.send(b"\x1b[B\x1b[B");
    pty.send(b"\x07");
    pty.capture("hybrid_run_to_cursor", Duration::from_secs(4), |text| {
        require(text, "[Paused]")?;
        require(text, "RunToLocation")
    });

    pty.send(b"\x1b[21~");
    pty.capture("hybrid_f10_step_over", Duration::from_secs(4), |text| {
        require(text, "[Paused]")?;
        require(text, "step over")
    });

    for _ in 0..15 {
        pty.send(b"\x1b[23~");
    }
    pty.capture("hybrid_f11_nested_step", Duration::from_secs(6), |text| {
        require(text, "[Paused]")?;
        require(text, "a")?;
        require(text, "x")?;
        require(text, "TEST")
    });

    pty.send(b"\x1b[23;2~");
    pty.capture(
        "hybrid_shift_f11_step_out",
        Duration::from_secs(6),
        |text| {
            require(text, "[Paused]")?;
            require(text, "step out")
        },
    );
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "hybrid")]
fn compiled_source_hybrid_pty_keeps_source_and_pc_truth_in_one_debugger() {
    let dir = tempfile::tempdir().expect("compiled source PTY dir");
    let target = dir.path().join("compiled-source.tcl");
    fs::write(
        &target,
        "proc inc {x} {\n    return [expr {$x + 1}]\n}\nset value 1\nset value [inc $value]\nset done $value\n",
    ).unwrap();
    let target = target.canonicalize().unwrap();
    let mut pty = DebugPty::spawn_vm_strict_debug(&target);
    pty.capture("compiled_source_entry", Duration::from_secs(8), |text| {
        require(text, "compiled-source.tcl [compiled source]")?;
        require(text, "Source · compiled source")?;
        require(text, "Disassembly · executable")?;
        require(text, "set value 1")?;
        require(text, "(0) nop")?;
        require(text, "[Paused]")
    });

    for _ in 0..4 {
        pty.send(b"\x1b[B");
    }
    pty.send(b"b");
    pty.capture(
        "compiled_source_line_breakpoint",
        Duration::from_secs(3),
        |text| {
            require(text, "breakpoint @ line 5")?;
            require(text, "line 5")
        },
    );
    pty.send(b"c");
    pty.capture(
        "compiled_source_breakpoint_hit",
        Duration::from_secs(5),
        |text| {
            require(text, "[Paused]")?;
            require(text, "▶●    5")?;
            require(text, "set value [inc $value]")
        },
    );

    pty.send(b"\x1b[23~");
    pty.capture(
        "compiled_source_f11_nested_command",
        Duration::from_secs(5),
        |text| {
            require(text, "[Paused]")?;
            require(text, "line 5")?;
            require(text, "inc")
        },
    );
    pty.send(b"\x1b[23~");
    pty.capture("compiled_source_f11_proc", Duration::from_secs(5), |text| {
        require(text, "[Paused]")?;
        require(text, "inc : line 2")?;
        require(text, "x = 1")
    });

    pty.send(b"\x1b[21~");
    pty.capture(
        "compiled_source_f10_over_return",
        Duration::from_secs(5),
        |text| {
            require(text, "[Paused]")?;
            require(text, "▶     6")?;
            require(text, "set done $value")
        },
    );

    pty.send(b"e");
    pty.capture(
        "compiled_source_vm_strict_eval_reason",
        Duration::from_secs(3),
        |text| {
            require(text, "vm-strict")?;
            require(text, "dynamic evaluation")
        },
    );

    pty.send(b"r");
    pty.capture(
        "compiled_source_restart_entry",
        Duration::from_secs(6),
        |text| {
            require(text, "[Paused]")?;
            require(text, "▶     1")?;
            require(text, "proc inc {x}")?;
            require(text, "compiled source")
        },
    );
    pty.send(b"c");
    pty.capture(
        "compiled_source_restart_breakpoint",
        Duration::from_secs(6),
        |text| {
            require(text, "[Paused]")?;
            require(text, "▶●    5")
        },
    );
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

#[test]
#[cfg(feature = "hybrid")]
fn compiled_source_hybrid_worldhost_output_and_error_keep_canonical_location() {
    let dir = tempfile::tempdir().expect("compiled hybrid PTY dir");
    let target = dir.path().join("compiled-hybrid.tcl");
    fs::write(&target, "puts hello\nerror boom\n").unwrap();
    let target = target.canonicalize().unwrap();
    let mut pty = DebugPty::spawn_hybrid_debug(&target);
    pty.capture("compiled_hybrid_entry", Duration::from_secs(8), |text| {
        require(text, "compiled-hybrid.tcl [compiled source]")?;
        require(text, "▶     1")?;
        require(text, "puts hello")?;
        require(text, "Disassembly · executable")
    });
    pty.send(b"\x1b[23~");
    pty.capture(
        "compiled_hybrid_worldhost_output",
        Duration::from_secs(5),
        |text| {
            require(text, "▶     2")?;
            require(text, "error boom")?;
            require(text, "hello")
        },
    );
    pty.send(b"c");
    pty.capture(
        "compiled_hybrid_mapped_error",
        Duration::from_secs(5),
        |text| {
            require(text, "[Paused]")?;
            require(text, "line 2")?;
            require(text, "boom")
        },
    );
    pty.send(b"\x03");
    pty.wait_for_exit(Duration::from_secs(5));
}

struct DebugPty {
    parser: vt100::Parser,
    rows: u16,
    cols: u16,
    raw: String,
    lifecycle_states: Vec<String>,
    _home: tempfile::TempDir,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    captures: Vec<(&'static str, String)>,
    frame_path: Option<PathBuf>,
    _tclsh_wrapper: Option<tempfile::TempDir>,
}

impl DebugPty {
    #[cfg(feature = "hybrid")]
    fn spawn_hybrid_debug(target: &Path) -> Self {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tcld"));
        command.arg("debug");
        command.arg(target);
        command.arg("--backend");
        command.arg("hybrid");
        command.env("TERM", "xterm-256color");
        command.cwd(env!("CARGO_MANIFEST_DIR"));
        Self::spawn_command(command, None)
    }

    #[cfg(feature = "hybrid")]
    fn spawn_vm_strict_debug(target: &Path) -> Self {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tcld"));
        command.arg("debug");
        command.arg(target);
        command.arg("--backend");
        command.arg("vm-strict");
        command.env("TERM", "xterm-256color");
        command.cwd(env!("CARGO_MANIFEST_DIR"));
        Self::spawn_command(command, None)
    }

    fn spawn_debug(
        target: &Path,
        entry_sentinel: &Path,
        breakpoint_sentinel: &Path,
        frame_path: Option<&Path>,
    ) -> Self {
        let (tclsh_wrapper, wrapper_guard) = probe_aware_tclsh();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_tcld"));
        command.arg("debug");
        command.arg(target);
        command.arg("--tclsh");
        command.arg(tclsh_wrapper);
        command.arg("--dylib");
        command.arg(dylib_path());
        command.arg("--bootstrap");
        command.arg(bootstrap_path());
        command.env("TERM", "xterm-256color");
        command.env("TCLD_DEBUG_PTY_ENTRY_SENTINEL", entry_sentinel);
        command.env("TCLD_DEBUG_PTY_BREAKPOINT_SENTINEL", breakpoint_sentinel);
        command.cwd(env!("CARGO_MANIFEST_DIR"));
        let mut pty = Self::spawn_command(command, frame_path);
        pty._tclsh_wrapper = Some(wrapper_guard);
        pty
    }

    fn spawn_shell() -> Self {
        let mut command = CommandBuilder::new("/bin/sh");
        command.env("TERM", "xterm-256color");
        command.env("PS1", "tcld-debug-test$ ");
        command.env("RUST_BACKTRACE", "0");
        command.env("TCLD_TEST_PANIC_AFTER_TERMINAL_ENTER", "debug");
        command.cwd(env!("CARGO_MANIFEST_DIR"));
        Self::spawn_command(command, None)
    }

    fn spawn_command(mut command: CommandBuilder, frame_path: Option<&Path>) -> Self {
        let home = tempfile::tempdir().expect("create isolated PTY home");
        command.env("HOME", home.path());
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open PTY");
        let child = pair
            .slave
            .spawn_command(command)
            .expect("spawn PTY command");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
        let writer = pair.master.take_writer().expect("take PTY writer");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if tx.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });
        Self {
            parser: vt100::Parser::new(ROWS, COLS, 0),
            rows: ROWS,
            cols: COLS,
            raw: String::new(),
            lifecycle_states: Vec::new(),
            _home: home,
            master: pair.master,
            child,
            writer,
            rx,
            captures: Vec::new(),
            frame_path: frame_path.map(Path::to_path_buf),
            _tclsh_wrapper: None,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("write PTY input");
        self.writer.flush().expect("flush PTY input");
        self.pump(Duration::from_millis(120));
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize PTY");
        self.parser.set_size(rows, cols);
        self.rows = rows;
        self.cols = cols;
        self.pump(Duration::from_millis(150));
    }

    fn capture<F>(&mut self, label: &'static str, timeout: Duration, check: F) -> String
    where
        F: Fn(&str) -> CheckResult,
    {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump(Duration::from_millis(50));
            let text = self.screen_text();
            match check(&text) {
                Ok(()) => {
                    self.captures.push((label, text.clone()));
                    self.write_captures();
                    return text;
                }
                Err(error) if Instant::now() >= deadline => {
                    panic!("timed out waiting for {label}: {error}\n{text}");
                }
                Err(_) => {}
            }
        }
    }

    fn lifecycle_states(&self) -> &[String] {
        &self.lifecycle_states
    }

    fn wait_for_exit(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump(Duration::from_millis(50));
            if self.child.try_wait().expect("poll PTY child").is_some() {
                return;
            }
        }
        panic!("tcld debug did not exit after Ctrl-C");
    }

    fn pump(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self
                .rx
                .recv_timeout(remaining.min(Duration::from_millis(20)))
            {
                Ok(bytes) => {
                    self.raw.push_str(&String::from_utf8_lossy(&bytes));
                    let mut start = 0;
                    for (index, byte) in bytes.iter().enumerate() {
                        if *byte == b']' {
                            self.parser.process(&bytes[start..=index]);
                            self.record_lifecycle_state();
                            start = index + 1;
                        }
                    }
                    if start < bytes.len() {
                        self.parser.process(&bytes[start..]);
                    }
                    self.record_lifecycle_state();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn screen_text(&self) -> String {
        let mut rows = self
            .parser
            .screen()
            .rows(0, self.cols)
            .map(|row| row.trim_end().to_string())
            .collect::<Vec<_>>();
        while rows.last().is_some_and(|row| row.is_empty()) {
            rows.pop();
        }
        let mut text = rows.join("\n");
        text.push('\n');
        text
    }

    fn record_lifecycle_state(&mut self) {
        let frame = self
            .parser
            .screen()
            .rows(0, self.cols)
            .collect::<Vec<_>>()
            .join("\n");
        for state in ["Starting debugger", "Ready", "Loading", "Paused"] {
            if frame.contains(&format!("[{state}]"))
                && self.lifecycle_states.last().map(String::as_str) != Some(state)
            {
                self.lifecycle_states.push(state.to_string());
            }
        }
    }

    fn write_captures(&self) {
        if let Some(path) = &self.frame_path {
            let mut output = String::new();
            for (label, text) in &self.captures {
                output.push_str("===== ");
                output.push_str(label);
                output.push_str(" =====\n");
                output.push_str(text);
                output.push('\n');
            }
            fs::write(path, output).expect("write debug PTY captures");
        }
    }
}

impl Drop for DebugPty {
    fn drop(&mut self) {
        self.write_captures();
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

type CheckResult = Result<(), String>;

fn require(text: &str, needle: &str) -> CheckResult {
    if text.contains(needle) {
        Ok(())
    } else {
        Err(format!("missing {needle:?}"))
    }
}

fn assert_lifecycle_order(states: &[String]) {
    assert_eq!(
        states,
        ["Starting debugger", "Ready", "Loading", "Paused"],
        "unexpected visible debugger lifecycle"
    );
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|error| panic!("resolve fixture {name}: {error}"))
}

fn tclsh_path() -> PathBuf {
    env::var_os("TCLD_DEBUG_PTY_TCLSH")
        .map(PathBuf::from)
        .expect("TCLD_DEBUG_PTY_TCLSH must name a supported Tcl shell")
}

fn probe_aware_tclsh() -> (PathBuf, tempfile::TempDir) {
    let real = tclsh_path();
    let mut probe = Command::new(&real)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start Tcl shell patchlevel probe");
    probe
        .stdin
        .as_mut()
        .expect("Tcl probe stdin")
        .write_all(b"puts -nonewline [info patchlevel]\n")
        .expect("write Tcl patchlevel probe");
    drop(probe.stdin.take());
    let output = probe
        .wait_with_output()
        .expect("wait for Tcl patchlevel probe");
    assert!(output.status.success(), "Tcl patchlevel probe failed");
    let patchlevel = String::from_utf8(output.stdout)
        .expect("Tcl patchlevel is UTF-8")
        .trim()
        .to_string();
    assert!(!patchlevel.is_empty(), "Tcl patchlevel probe was empty");
    let directory = tempfile::tempdir().expect("create Tcl shell wrapper directory");
    let wrapper = directory.path().join("tclsh-wrapper");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$#\" -eq 0 ]; then printf '%s' {}; exit 0; fi\nexec {} \"$@\"\n",
            shell_quote(Path::new(&patchlevel)),
            shell_quote(&real),
        ),
    )
    .expect("write Tcl shell wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("make Tcl shell wrapper executable");
    (wrapper, directory)
}

fn dylib_path() -> PathBuf {
    env::var_os("TCLD_DEBUG_PTY_DYLIB")
        .map(PathBuf::from)
        .expect("TCLD_DEBUG_PTY_DYLIB must name the matching loadable debugger nub")
        .canonicalize()
        .expect("canonical debugger nub path")
}

fn bootstrap_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl")
        .canonicalize()
        .expect("canonical debugger bootstrap path")
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}
