//! Reference Rust frontend for the tcld wire protocol.
//!
//! Mirrors `crates/tcld-dbg-ext/scripts/tcld_ctrl_demo.tcl`: spawns a
//! tclsh subprocess running the tcld nub bootstrap, sets a breakpoint
//! on the first command of `demo`, runs to the pause, reads variables,
//! steps once, then continues to completion — printing each step and
//! every event as it arrives.
//!
//! Run from a workspace checkout (the dylib and bootstrap script are
//! located relative to it):
//!
//! ```text
//! cargo build -p tcld-dbg-ext          # builds libtcld_dbg.{dylib,so}
//! cargo run -p tcld-debug-wire --example wire_demo -- [user-script.tcl]
//! ```
//!
//! Environment variables (all optional; the demo exits with a clear
//! message if discovery fails):
//!
//! - `TCLD_TCLSH`     — path to a Tcl 9 `tclsh`. Default: well-known
//!   Homebrew locations, then `tclsh9.0` / `tclsh` on `PATH`.
//! - `TCLD_DBG_DYLIB` — path to `libtcld_dbg.{dylib,so}`. Default:
//!   `$CARGO_TARGET_DIR` (or `<workspace>/target`) `/{debug,release}/`.
//!
//! The user script defaults to `fixtures/demo.tcl`, which defines a
//! three-statement proc `demo` and schedules it via `after`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tcld_debug_wire::{BreakpointTarget, Controller, ControllerError, Op, WireEvent};

const PAUSE_WAIT: Duration = Duration::from_secs(5);

fn find_tclsh() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("TCLD_TCLSH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!(
            "TCLD_TCLSH is set to {} but no file exists there",
            p.display()
        ));
    }
    for candidate in [
        "/opt/homebrew/bin/tclsh9.0",
        "/opt/homebrew/Cellar/tcl-tk/9.0.3/bin/tclsh9.0",
        "/usr/local/bin/tclsh9.0",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Ok(p);
        }
    }
    // Fall back to PATH lookup.
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in ["tclsh9.0", "tclsh"] {
                let p = dir.join(name);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    Err("no tclsh found; set TCLD_TCLSH to a Tcl 9 tclsh binary".to_string())
}

fn find_dylib() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("TCLD_DBG_DYLIB") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!(
            "TCLD_DBG_DYLIB is set to {} but no file exists there",
            p.display()
        ));
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../target"));
    for profile in ["debug", "release"] {
        for ext in ["dylib", "so"] {
            let p = target_dir.join(profile).join(format!("libtcld_dbg.{ext}"));
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "could not locate libtcld_dbg.{{dylib,so}} under {} — \
         run `cargo build -p tcld-dbg-ext` first, or set TCLD_DBG_DYLIB",
        target_dir.display()
    ))
}

fn find_bootstrap() -> Result<PathBuf, String> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl");
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!(
            "bootstrap script not found at {} — this demo must run \
             from a workspace checkout",
            p.display()
        ))
    }
}

/// Print every event as it arrives; return the first `paused` event,
/// or `None` on timeout / disconnect.
fn wait_for_paused(ctrl: &mut Controller, timeout: Duration) -> Option<WireEvent> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match ctrl.next_event(Duration::from_millis(200)) {
            Ok(ev) => {
                println!("      event: {}", ev.to_payload());
                if matches!(ev, WireEvent::Paused { .. }) {
                    return Some(ev);
                }
            }
            Err(ControllerError::ResponseTimeout) => continue,
            Err(_) => return None, // disconnected or protocol violation
        }
    }
    None
}

fn print_pending_events(ctrl: &mut Controller) {
    for ev in ctrl.drain_events() {
        println!("      event: {}", ev.to_payload());
    }
}

fn wait_for_target_completed(ctrl: &mut Controller, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match ctrl.next_event(Duration::from_millis(200)) {
            Ok(event) => {
                println!("      event: {}", event.to_payload());
                if matches!(event, WireEvent::TargetCompleted { .. }) {
                    return true;
                }
                if matches!(event, WireEvent::TargetLoadFailed { .. }) {
                    return false;
                }
            }
            Err(ControllerError::ResponseTimeout) => continue,
            Err(_) => return false,
        }
    }
    false
}

fn parse_first_offset(payload: &str) -> Option<u64> {
    payload
        .split_whitespace()
        .skip_while(|t| *t != "first_offset")
        .nth(1)?
        .parse()
        .ok()
}

fn fail(msg: &str) -> ! {
    eprintln!("wire_demo: {msg}");
    std::process::exit(2);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tclsh = find_tclsh().unwrap_or_else(|e| fail(&e));
    let dylib = find_dylib().unwrap_or_else(|e| fail(&e));
    let bootstrap = find_bootstrap().unwrap_or_else(|e| fail(&e));
    let user_script = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/demo.tcl"));
    if !user_script.is_file() {
        fail(&format!("user script not found: {}", user_script.display()));
    }

    println!("[1/9] tclsh:       {}", tclsh.display());
    println!("      dylib:       {}", dylib.display());
    println!("      bootstrap:   {}", bootstrap.display());
    println!("      user script: {}", user_script.display());

    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse()?)?;
    println!("[2/9] listening on 127.0.0.1:{port}");

    let mut nub = ctrl.spawn_nub(&tclsh, &bootstrap, &dylib)?;
    println!("[3/9] spawned nub (pid {:?})", nub.pid());

    // Tear down deterministically on every path: close the wire first
    // (sends `bye`), then reap or kill the child so no tclsh is ever
    // orphaned, even when the session fails partway.
    let result = run_session(&mut ctrl, &mut nub, &user_script);
    let _ = ctrl.close();
    let _ = nub.kill();
    match result {
        Ok(()) => {
            println!("done.");
            Ok(())
        }
        Err(e) => {
            eprintln!("wire_demo: {e}");
            std::process::exit(1);
        }
    }
}

fn run_session(
    ctrl: &mut Controller,
    nub: &mut tcld_debug_wire::NubChild,
    user_script: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let caps = ctrl.accept()?;
    println!("[4/9] handshake ok; nub capabilities: {caps:?}");
    ctrl.send(Op::StartTarget {
        path: user_script.to_string_lossy().into_owned(),
        stop_on_entry: false,
    })?;
    if !wait_for_target_completed(ctrl, PAUSE_WAIT) {
        return Err("target did not load successfully".into());
    }

    // Find the first statement of `demo` and set a breakpoint there.
    let proc_loc = ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })?
        .into_payload();
    let first_offset = parse_first_offset(&proc_loc)
        .ok_or_else(|| format!("no first_offset in proc_location: {proc_loc}"))?;
    let bp = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: first_offset,
                len: 1,
            },
            condition: None,
        })?
        .into_payload();
    println!("[5/9] proc demo: {proc_loc}");
    println!("      breakpoint {bp} set at offset {first_offset}");
    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "after 0 demo".into(),
    })?;

    // Run to the pause (the fixture schedules `demo` via `after`).
    println!("[6/9] waiting for paused event…");
    let paused = wait_for_paused(ctrl, PAUSE_WAIT)
        .ok_or_else(|| format!("no paused event within {PAUSE_WAIT:?}"))?;
    if let WireEvent::Paused {
        location,
        frame_count,
        breakpoint,
        ..
    } = &paused
    {
        println!(
            "      paused at {} (frames: {frame_count}, bp: {breakpoint:?})",
            location.to_payload()
        );
    }
    let vars = ctrl.send(Op::GetVariables { level: 1 })?.into_payload();
    println!("      variables at level 1: {vars}");

    // Step past the first statement, then read the variable it set.
    println!("[7/9] step_into…");
    ctrl.send(Op::StepInto)?;
    wait_for_paused(ctrl, PAUSE_WAIT).ok_or("no paused event after step_into")?;
    let a = ctrl
        .send(Op::GetVar {
            level: 1,
            name: "a".into(),
            maxlen: None,
        })?
        .into_payload();
    println!("      after one step, a = {a}");

    // While still paused, schedule a clean nub exit for after the
    // script completes, then continue to completion.
    println!("[8/9] continue to completion…");
    if let Err(e) = ctrl.send(Op::EvalInFrame {
        level: 1,
        script: "after 400 {exit 0}".into(),
    }) {
        println!("      warn: could not schedule nub exit ({e}); will kill instead");
    }
    ctrl.send(Op::Continue)?;
    print_pending_events(ctrl);

    // The demo proc finishes, the scheduled `exit 0` fires, and the
    // nub process terminates on its own.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        print_pending_events(ctrl);
        if let Some(status) = nub.try_wait()? {
            println!("[9/9] nub exited: {status}");
            return Ok(());
        }
        if Instant::now() >= deadline {
            println!("[9/9] nub still running; killing it");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
