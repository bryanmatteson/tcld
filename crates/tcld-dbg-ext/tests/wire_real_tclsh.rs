//! Phase 4-D verifier: Rust controller drives a real `tclsh`
//! subprocess via the wire protocol.
//!
//! Gated behind `integration_tclsh` because it requires:
//!   - A Tcl 9 `tclsh` on PATH (or via `TCLD_TCLSH` env)
//!   - The `libtcld_dbg.dylib` (or `.so`) built first with `cargo build`.
//!
//! The test:
//!   1. Listens on loopback
//!   2. Writes a tiny user-script to a temp file
//!   3. Spawns `tclsh tcld_nub_bootstrap.tcl` with env vars wired up
//!   4. Accepts the connection, sends `bp_add`, watches for `paused`
//!   5. Sends `quit` and asserts the process exits cleanly

#![cfg(feature = "integration_tclsh")]

use std::io::{BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tcld_debug_wire::{
    BreakpointTarget, Controller, FrameKind, Hello, HelloAck, Op, WireEvent, WireFrame,
    decode_frame, encode_frame,
};

const SCRIPTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts");

fn tclsh_path() -> PathBuf {
    if let Ok(p) = std::env::var("TCLD_TCLSH") {
        return PathBuf::from(p);
    }
    // Common Tcl 9 install on macOS via Homebrew.
    let candidate = PathBuf::from("/opt/homebrew/Cellar/tcl-tk/9.0.3/bin/tclsh9.0");
    if candidate.exists() {
        return candidate;
    }
    PathBuf::from("tclsh")
}

fn dylib_path() -> PathBuf {
    // Locate the dylib relative to CARGO_MANIFEST_DIR / ../../target/debug/.
    // CI builds it explicitly before starting the integration test binary so
    // Cargo cannot race the test process against the cdylib linker.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.join("../..");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            }
        })
        .unwrap_or_else(|_| workspace.join("target"));
    // The integration workflow builds the cdylib before starting this test.
    // Prefer deps in case Cargo leaves an older uplifted copy in <profile>/.
    for profile in ["debug", "release"] {
        for dir in [
            target_dir.join(profile).join("deps"),
            target_dir.join(profile),
        ] {
            for ext in ["dylib", "so"] {
                let p = dir.join(format!("libtcld_dbg.{ext}"));
                if p.exists() {
                    return p;
                }
            }
        }
    }
    panic!(
        "could not locate libtcld_dbg.dylib in {:?} (debug|release, incl. deps/)",
        target_dir
    );
}

#[test]
fn real_tclsh_handshake_and_bp_pause() {
    let (mut ctrl, _port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");

    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let dylib = dylib_path();

    // Write a user script that defines a proc. The controller schedules the
    // invocation only after the target-completed event and breakpoint staging.
    let user_script = std::env::temp_dir().join("tcld_wire_real_user.tcl");
    std::fs::write(&user_script, "proc demo {} { set a 1\nset b 2\nset c 3 }\n")
        .expect("write user script");

    let nub = ctrl
        .spawn_nub(&tclsh_path(), &bootstrap, &dylib)
        .expect("spawn_nub");
    // Hold the child until end of test for cleanup.
    let _nub_guard = nub;

    let _caps = ctrl.accept().expect("accept (handshake)");
    ctrl.send(Op::StartTarget {
        path: user_script.to_string_lossy().into_owned(),
        stop_on_entry: false,
    })
    .expect("start_target");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ctrl
            .drain_events()
            .iter()
            .any(|event| matches!(event, WireEvent::TargetCompleted { .. }))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // Discover proc location.
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
    let _bp = ctrl
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
    let mut all_events: Vec<WireEvent> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !saw_paused {
        for ev in ctrl.drain_events() {
            if matches!(ev, WireEvent::Paused { .. }) {
                saw_paused = true;
            }
            all_events.push(ev);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        saw_paused,
        "real tclsh nub never sent Paused. all events seen: {:?}",
        all_events
    );

    // Cleanup runs via Drop.
    let _ = std::fs::remove_file(&user_script);
}

#[test]
fn empty_target_completes_without_inventing_a_pause_location() {
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(format!("tcld-empty-{tag}"));
    std::fs::create_dir_all(&dir).expect("create empty-target fixture directory");
    let dir = dir.canonicalize().expect("canonical fixture directory");
    let target = dir.join("empty.tcl");
    let target_text = target.to_string_lossy().into_owned();
    std::fs::write(&target, "# no executable commands\n\n").expect("write empty target");
    let (mut ctrl, _) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let mut nub = ctrl
        .spawn_nub(&tclsh_path(), &bootstrap, &dylib_path())
        .expect("spawn nub");
    ctrl.accept().expect("v2 handshake");
    ctrl.send(Op::StartTarget {
        path: target_text.clone(),
        stop_on_entry: true,
    })
    .expect("schedule empty target");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = Vec::new();
    while Instant::now() < deadline
        && !observed
            .iter()
            .any(|event| matches!(event, WireEvent::TargetCompleted { .. }))
    {
        observed.extend(ctrl.drain_events());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        observed.iter().any(
            |event| matches!(event, WireEvent::TargetStarted { path, .. } if path == &target_text)
        ),
        "missing target_started: {observed:?}"
    );
    assert!(
        observed.iter().any(
            |event| matches!(event, WireEvent::TargetCompleted { path, .. } if path == &target_text)
        ),
        "missing target_completed: {observed:?}"
    );
    assert!(
        !observed
            .iter()
            .any(|event| matches!(event, WireEvent::Paused { .. })),
        "empty target invented a pause location: {observed:?}"
    );

    let _ = ctrl.close();
    let _ = nub.kill();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn start_target_pauses_before_first_user_side_effect() {
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(format!("tcld-entry-{tag}"));
    std::fs::create_dir_all(&dir).expect("create fixture directory");
    let dir = dir.canonicalize().expect("canonical fixture directory");
    let target = dir.join("target.tcl");
    let sentinel = dir.join("first-command-ran");
    std::fs::write(
        &target,
        format!(
            "set fd [open {{{}}} w]\nputs $fd reached\nclose $fd\n",
            sentinel.display()
        ),
    )
    .expect("write target");

    let (mut ctrl, _port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let mut nub = ctrl
        .spawn_nub(&tclsh_path(), &bootstrap, &dylib_path())
        .expect("spawn nub");
    ctrl.accept().expect("v2 handshake");
    ctrl.send(Op::StartTarget {
        path: target.to_string_lossy().into_owned(),
        stop_on_entry: true,
    })
    .expect("schedule target");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = Vec::new();
    let paused = 'entry: loop {
        assert!(
            Instant::now() < deadline,
            "entry pause timed out; sentinel={} events={observed:?}",
            sentinel.exists()
        );
        for event in ctrl.drain_events() {
            if matches!(event, WireEvent::Paused { .. }) {
                break 'entry event;
            }
            observed.push(event);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let WireEvent::Paused {
        location, reason, ..
    } = paused
    else {
        unreachable!()
    };
    assert_eq!(reason, tcld_debug_wire::PauseReason::Entry);
    assert_eq!(location.source_path.as_deref(), target.to_str());
    assert_eq!(location.block_kind, tcld_debug_wire::SourceBlockKind::File);
    assert!(
        !sentinel.exists(),
        "first target command ran before stop-on-entry pause"
    );

    ctrl.send(Op::Continue).expect("continue target");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut completed = false;
    while Instant::now() < deadline && !completed {
        completed = ctrl
            .drain_events()
            .iter()
            .any(|event| matches!(event, WireEvent::TargetCompleted { .. }));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(completed, "target did not complete after Continue");
    assert!(
        sentinel.exists(),
        "target side effect missing after Continue"
    );

    let _ = ctrl.close();
    let _ = nub.kill();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pending_file_line_breakpoint_resolves_before_target_reaches_it() {
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(format!("tcld-pending-{tag}"));
    std::fs::create_dir_all(&dir).expect("create fixture directory");
    let dir = dir.canonicalize().expect("canonical fixture directory");
    let target = dir.join("target.tcl");
    let sentinel = dir.join("breakpoint-command-ran");
    std::fs::write(
        &target,
        format!(
            "set before 1\nexec touch {{{}}}\nset after 1\n",
            sentinel.display()
        ),
    )
    .expect("write target");

    let (mut ctrl, _port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let mut nub = ctrl
        .spawn_nub(&tclsh_path(), &bootstrap, &dylib_path())
        .expect("spawn nub");
    ctrl.accept().expect("v2 handshake");
    let pending = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::FileLine {
                path: target.to_string_lossy().into_owned(),
                line: 2,
            },
            condition: None,
        })
        .expect("stage file line breakpoint")
        .into_payload();
    assert!(!pending.is_empty(), "missing breakpoint id");
    let listed = ctrl.send(Op::BpList).expect("list pending").into_payload();
    assert!(
        listed.contains(&format!("id {pending} ")) && listed.contains("pending 1"),
        "pending row missing: {listed}"
    );
    let persisted = ctrl
        .send(Op::BpExport)
        .expect("export pending breakpoint")
        .into_payload();
    assert!(
        persisted.contains("kind file_line") && persisted.contains("line 2"),
        "pending file/line identity was not exported: {persisted}"
    );
    ctrl.send(Op::BpRemove { id: pending })
        .expect("remove staged breakpoint before import");
    ctrl.send(Op::BpImport {
        serialized: persisted,
    })
    .expect("re-import pending breakpoint");
    let listed = ctrl
        .send(Op::BpList)
        .expect("list re-imported pending breakpoint")
        .into_payload();
    assert!(listed.contains("pending 1") && listed.contains("line 2"));

    ctrl.send(Op::StartTarget {
        path: target.to_string_lossy().into_owned(),
        stop_on_entry: false,
    })
    .expect("schedule target");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = Vec::new();
    let paused = 'pause: loop {
        if Instant::now() >= deadline {
            let state = ctrl
                .send(Op::EvalInFrame {
                    level: 0,
                    script: "list target_state $::tcld::dbg::__nub__::target_state target_request $::tcld::dbg::__nub__::target_request target_epoch $::tcld::dbg::__nub__::target_epoch".into(),
                })
                .map(|response| response.into_payload());
            panic!("pending breakpoint pause timed out; events={observed:?} state={state:?}");
        }
        for event in ctrl.drain_events() {
            if matches!(event, WireEvent::Paused { .. }) {
                break 'pause event;
            }
            observed.push(event);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let WireEvent::Paused {
        location,
        reason,
        breakpoint,
        ..
    } = paused
    else {
        unreachable!()
    };
    assert_eq!(reason, tcld_debug_wire::PauseReason::Breakpoint);
    assert!(breakpoint.is_some());
    assert_eq!(location.source_path.as_deref(), target.to_str());
    assert!(location.line == 2, "wrong pause location: {location:?}");
    assert!(
        !sentinel.exists(),
        "breakpoint command ran before pending breakpoint pause"
    );
    let stack = ctrl
        .send(Op::InspectStack)
        .expect("inspect exact paused stack")
        .into_payload();
    assert!(
        stack.contains("source_path") && stack.contains("offset_start"),
        "stack omitted structured source provenance: {stack}"
    );

    ctrl.send(Op::Continue).expect("continue target");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut completed = false;
    while Instant::now() < deadline && !completed {
        completed = ctrl
            .drain_events()
            .iter()
            .any(|event| matches!(event, WireEvent::TargetCompleted { .. }));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(completed);
    assert!(sentinel.exists());

    let _ = ctrl.close();
    let _ = nub.kill();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn interrupt_pauses_a_running_target_before_completion() {
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(format!("tcld-interrupt-{tag}"));
    std::fs::create_dir_all(&dir).expect("create fixture directory");
    let target = dir.join("target.tcl");
    let started = dir.join("started");
    let completed = dir.join("completed");
    std::fs::write(
        &target,
        format!(
            "exec touch {{{}}}\n\
             for {{set i 0}} {{$i < 10000}} {{incr i}} {{after 1}}\n\
             exec touch {{{}}}\n",
            started.display(),
            completed.display()
        ),
    )
    .expect("write target");

    let (mut ctrl, _port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let mut nub = ctrl
        .spawn_nub(&tclsh_path(), &bootstrap, &dylib_path())
        .expect("spawn nub");
    ctrl.accept().expect("v2 handshake");
    ctrl.send(Op::StartTarget {
        path: target.to_string_lossy().into_owned(),
        stop_on_entry: false,
    })
    .expect("schedule target");

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !started.exists() {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(started.exists(), "target did not begin running");
    assert!(!completed.exists(), "target completed before interrupt");

    ctrl.send(Op::Interrupt)
        .expect("interrupt should be acknowledged while target runs");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut observed = Vec::new();
    let paused = 'pause: loop {
        assert!(
            Instant::now() < deadline,
            "interrupt pause timed out; completed={} events={observed:?}",
            completed.exists()
        );
        for event in ctrl.drain_events() {
            if matches!(event, WireEvent::Paused { .. }) {
                break 'pause event;
            }
            observed.push(event);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let WireEvent::Paused { reason, .. } = paused else {
        unreachable!()
    };
    assert_eq!(reason, tcld_debug_wire::PauseReason::Pause);
    assert!(
        !completed.exists(),
        "target completed before the interrupt pause"
    );

    let _ = nub.kill();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn v2_nub_rejects_v1_controller_ack_before_target_execution() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let port = listener.local_addr().unwrap().port();
    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let mut child = Command::new(tclsh_path())
        .arg(bootstrap)
        .env("TCLD_DBG_DYLIB", dylib_path())
        .env("TCLD_NUB_HOST", "127.0.0.1")
        .env("TCLD_NUB_PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn v2 nub");
    let (stream, _) = listener.accept().expect("accept nub");
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let hello_frame = decode_frame(&mut reader).expect("read v2 hello");
    assert_eq!(hello_frame.kind, FrameKind::Hello);
    let hello = Hello::from_payload(&hello_frame.payload).expect("parse hello");
    assert_eq!(hello.protocol_version, 2);

    let mut writer = stream;
    let ack = HelloAck {
        controller_version: "0.1.0".into(),
        protocol_version: 1,
        capabilities: Vec::new(),
    };
    encode_frame(
        &mut writer,
        &WireFrame::new(FrameKind::HelloAck, 0, ack.to_payload()),
    )
    .expect("send v1 ack");
    writer.flush().unwrap();
    drop(writer);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("poll nub") {
            assert!(!status.success(), "nub accepted a v1 controller ack");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "nub did not reject v1 controller"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
