//! Live tests for the TclPro parity completion commands:
//! `get_level`, `quit`, `register`/`unregister`, `var_breakpoints`,
//! `line_breakpoints`, `pre_instrument_proc`, `pre_instrument_file`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tcl_sys::{
    TCL_OK, Tcl_CreateInterp, Tcl_DeleteInterp, Tcl_EvalEx, Tcl_GetObjResult, Tcl_GetStringFromObj,
    Tcl_Init, Tcl_Size,
};
use tcld_dbg::step_engine;
use tcld_dbg::wrappers;
use tcld_debug_state::{BlockId, DebugEvent, DebugState, PauseReason};

/// Serializes interp creation across test threads. Concurrent
/// `Tcl_Init` calls race Tcl's process-global path/VFS state: `file
/// exists` on a perfectly good $tcl_library/init.tcl transiently fails
/// and Tcl_Init errors with "Cannot find a usable init.tcl" (observed
/// ~20% of parallel runs on Linux). Only the creation edge is
/// serialized — test bodies still run in parallel.
static TCL_LIFECYCLE: Mutex<()> = Mutex::new(());

fn fresh_interp() -> *mut tcl_sys::Tcl_Interp {
    let _lifecycle = TCL_LIFECYCLE.lock().unwrap();
    // Tcl_FindExecutable's contract is once per PROCESS, before other
    // Tcl calls.
    static FIND_EXECUTABLE: std::sync::Once = std::sync::Once::new();
    // Tcl 9 returns the resolved executable name; only the side effect matters.
    FIND_EXECUTABLE.call_once(|| unsafe {
        tcl_sys::Tcl_FindExecutable(std::ptr::null());
    });
    unsafe {
        let interp = Tcl_CreateInterp();
        assert!(!interp.is_null());
        let ret = Tcl_Init(interp);
        if ret != TCL_OK {
            let result_obj = Tcl_GetObjResult(interp);
            let mut len: Tcl_Size = 0;
            let ptr = Tcl_GetStringFromObj(result_obj, &mut len);
            let bytes = std::slice::from_raw_parts(ptr as *const u8, len.max(0) as usize);
            panic!("Tcl_Init failed: {}", String::from_utf8_lossy(bytes));
        }
        interp
    }
}

fn eval(interp: *mut tcl_sys::Tcl_Interp, script: &str) -> Result<String, String> {
    let cstr = std::ffi::CString::new(script).map_err(|e| e.to_string())?;
    unsafe {
        let ret = Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0);
        let result_obj = Tcl_GetObjResult(interp);
        let mut len: Tcl_Size = 0;
        let ptr = Tcl_GetStringFromObj(result_obj, &mut len);
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len.max(0) as usize);
        let text = String::from_utf8_lossy(bytes).into_owned();
        if ret == TCL_OK { Ok(text) } else { Err(text) }
    }
}

#[derive(Default)]
struct Spy {
    events: Mutex<Vec<DebugEvent>>,
    count: AtomicUsize,
}

unsafe extern "C" {
    fn Tcld_dbg_Init(interp: *mut tcl_sys::Tcl_Interp) -> std::os::raw::c_int;
}

fn install_full_engine(interp: *mut tcl_sys::Tcl_Interp) -> (Arc<Mutex<DebugState>>, Arc<Spy>) {
    unsafe {
        let rc = Tcld_dbg_Init(interp);
        assert_eq!(rc, TCL_OK, "Tcld_dbg_Init failed");
    }
    let state = Arc::new(Mutex::new(DebugState::new()));
    let spy = Arc::new(Spy::default());
    {
        let mut guard = state.lock().unwrap();
        let s = spy.clone();
        guard.subscribe(Arc::new(move |event| {
            s.count.fetch_add(1, Ordering::SeqCst);
            s.events.lock().unwrap().push(event);
        }));
    }
    unsafe {
        step_engine::install(interp, state.clone()).expect("install engine");
        step_engine::set_pause_strategy(interp, step_engine::PauseStrategy::EmitOnly);
        wrappers::install_command_wrappers(interp).expect("install wrappers");
        let helper_install = r#"
namespace eval ::tcld::dbg {}
proc ::tcld::dbg::__t2_get_variables__ {level} {
    set vars [uplevel #$level {info vars}]
    set out {}
    foreach n $vars {
        if {[uplevel #$level [list array exists $n]]} {
            lappend out [list $n a]
        } else {
            lappend out [list $n s]
        }
    }
    return $out
}
proc ::tcld::dbg::__t2_get_var__ {level name maxlen} {
    if {[uplevel #$level [list array exists $name]]} {
        set raw [uplevel #$level [list array get $name]]
    } else {
        set raw [uplevel #$level [list set $name]]
    }
    if {$maxlen > 0 && [string length $raw] > $maxlen} {
        return "[string range $raw 0 [expr {$maxlen - 1}]]..."
    }
    return $raw
}
proc ::tcld::dbg::__t2_set_var__ {level name value} {
    return [uplevel #$level [list set $name $value]]
}
"#;
        let cstr = std::ffi::CString::new(helper_install).unwrap();
        let rc = Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0);
        assert_eq!(rc, TCL_OK, "tier2 helper install");
    }
    (state, spy)
}

fn first_offset(state: &Mutex<DebugState>, qname: &str) -> usize {
    let g = state.lock().unwrap();
    let block = g
        .blocks
        .get(&BlockId::ProcBody(Arc::from(qname)))
        .unwrap_or_else(|| panic!("block not registered: {qname}"));
    *block.valid_offsets().first().unwrap_or(&0)
}

fn unique_temp_script(name: &str, body: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("tcld-{name}-{}-{stamp}.tcl", std::process::id()));
    std::fs::write(&path, body).expect("write temp Tcl script");
    path
}

// ── get_level ───────────────────────────────────────────────────────

#[test]
fn get_level_returns_zero_at_top_level_when_stopped() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    {
        let mut g = state.lock().unwrap();
        g.app_state = tcld_debug_state::AppState::Stopped;
    }
    let r = eval(interp, "::tcld::dbg::get_level").expect("get_level");
    assert_eq!(r, "0");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn get_level_errors_when_not_stopped() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let r = eval(interp, "::tcld::dbg::get_level");
    assert!(
        r.is_err(),
        "get_level should reject when state != Stopped: {:?}",
        r
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── quit ────────────────────────────────────────────────────────────

#[test]
fn quit_uninstalls_engine_and_is_idempotent() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let r = eval(interp, "::tcld::dbg::quit").expect("quit");
    assert_eq!(r, "ok");
    // After quit, app_state should be reachable via app_state cmd → "dead"
    let r = eval(interp, "::tcld::dbg::app_state").expect("app_state");
    assert_eq!(r, "dead");
    // Calling quit again is a no-op (returns ok).
    let r = eval(interp, "::tcld::dbg::quit").expect("second quit");
    assert_eq!(r, "ok");

    unsafe {
        Tcl_DeleteInterp(interp);
    }
}

// ── register / unregister ───────────────────────────────────────────

#[test]
fn register_invokes_tcl_script_on_matching_event() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "set ::counter 0").unwrap();
    let id = eval(interp, r#"::tcld::dbg::register paused {incr ::counter}"#).expect("register");
    assert!(id.starts_with("sub"), "got id: {id}");

    // Fire a Paused event manually via the event system.
    {
        let g = state.lock().unwrap();
        let block_id = BlockId::ProcBody(Arc::from("__synth__"));
        // Need a registered block to make Location work; register via mut.
        drop(g);
        let mut mg = state.lock().unwrap();
        mg.blocks
            .register(block_id.clone(), Arc::from("x"), vec![0])
            .unwrap();
        let loc = tcld_debug_state::Location::new(block_id, 0, 0..1).unwrap();
        drop(mg);
        tcld_debug_state::fire_event_unlocked(
            &state,
            DebugEvent::Paused {
                location: loc,
                frame_count: 0,
                breakpoint: None,
                reason: tcld_debug_state::PauseReason::Step,
                pc: None,
            },
        );
    }

    let r = eval(interp, "set ::counter").unwrap();
    assert_eq!(r, "1");

    // Resumed event should NOT trigger this subscription (filter=paused).
    tcld_debug_state::fire_event_unlocked(&state, DebugEvent::Resumed);
    let r = eval(interp, "set ::counter").unwrap();
    assert_eq!(r, "1");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn register_with_all_filter_matches_every_event() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "set ::n 0").unwrap();
    eval(interp, r#"::tcld::dbg::register all {incr ::n}"#).expect("register");
    tcld_debug_state::fire_event_unlocked(&state, DebugEvent::Resumed);
    tcld_debug_state::fire_event_unlocked(&state, DebugEvent::Resumed);
    let r = eval(interp, "set ::n").unwrap();
    assert_eq!(r, "2");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn unregister_stops_subscription() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "set ::n 0").unwrap();
    let id = eval(interp, r#"::tcld::dbg::register all {incr ::n}"#).unwrap();
    eval(interp, &format!("::tcld::dbg::unregister {id}")).expect("unregister");
    tcld_debug_state::fire_event_unlocked(&state, DebugEvent::Resumed);
    let r = eval(interp, "set ::n").unwrap();
    assert_eq!(r, "0", "subscription should be removed");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn unregister_errors_for_unknown_subscription() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let r = eval(interp, "::tcld::dbg::unregister sub999");
    assert!(r.is_err(), "expected error: {:?}", r);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── var_breakpoints ────────────────────────────────────────────────

#[test]
fn var_breakpoints_lists_only_var_kind() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1 }").unwrap();
    let off = first_offset(&state, "demo");
    eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).unwrap();
    eval(interp, "::tcld::dbg::watchpoint_add foo write").unwrap();
    eval(interp, "::tcld::dbg::watchpoint_add bar {read write}").unwrap();
    let r = eval(interp, "::tcld::dbg::var_breakpoints").expect("list");
    assert!(r.contains("name {foo}"), "got {r}");
    assert!(r.contains("name {bar}"), "got {r}");
    // Filter by name.
    let r = eval(interp, "::tcld::dbg::var_breakpoints foo").expect("filter");
    assert!(r.contains("name {foo}"));
    assert!(!r.contains("name {bar}"), "filter leaked: {r}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── line_breakpoints ────────────────────────────────────────────────

#[test]
fn line_breakpoints_filters_by_qname() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc alpha {} { set a 1 }").unwrap();
    eval(interp, "proc beta {} { set b 2 }").unwrap();
    let oa = first_offset(&state, "alpha");
    let ob = first_offset(&state, "beta");
    eval(interp, &format!("::tcld::dbg::breakpoint_add alpha {oa} 1")).unwrap();
    eval(interp, &format!("::tcld::dbg::breakpoint_add beta {ob} 1")).unwrap();
    let all = eval(interp, "::tcld::dbg::line_breakpoints").expect("list");
    assert!(all.contains("proc:alpha"), "got {all}");
    assert!(all.contains("proc:beta"), "got {all}");
    let only_alpha = eval(interp, "::tcld::dbg::line_breakpoints alpha").expect("filter");
    assert!(only_alpha.contains("proc:alpha"));
    assert!(!only_alpha.contains("proc:beta"), "leaked: {only_alpha}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn line_breakpoints_includes_condition_when_set() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1 }").unwrap();
    let off = first_offset(&state, "demo");
    eval(
        interp,
        &format!("::tcld::dbg::breakpoint_add demo {off} 1 {{$a > 5}}"),
    )
    .unwrap();
    let r = eval(interp, "::tcld::dbg::line_breakpoints").expect("list");
    assert!(r.contains("condition {$a > 5}"), "got {r}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn patched_runtime_instruction_breakpoint_emits_pc_pause() {
    let interp = fresh_interp();
    let (_state, spy) = install_full_engine(interp);
    eval(interp, "::tcld::dbg::install").expect("adapter install");
    let supported = eval(interp, "::tcld::dbg::has instruction_step").expect("capability");
    if supported.trim() != "1" {
        unsafe {
            step_engine::uninstall(interp).expect("uninstall");
            Tcl_DeleteInterp(interp);
        }
        return;
    }

    eval(interp, "::tcld::dbg::breakpoint_add instruction 0").expect("instruction breakpoint");
    let before = spy.count.load(Ordering::SeqCst);
    eval(interp, "proc demo {} {set a 1\nset b 2}; demo").expect("run demo");

    let events = spy.events.lock().unwrap();
    assert!(
        events.iter().skip(before).any(|event| matches!(
            event,
            DebugEvent::Paused {
                reason: PauseReason::InstructionBreakpoint,
                pc: Some(0),
                ..
            }
        )),
        "expected instruction breakpoint pause at pc 0, saw {:?}",
        &events[before..]
    );

    drop(events);
    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn patched_runtime_opcode_pause_recovers_non_precompiled_source_location() {
    let interp = fresh_interp();
    let (_state, spy) = install_full_engine(interp);
    eval(interp, "::tcld::dbg::install").expect("adapter install");
    let supported = eval(interp, "::tcld::dbg::has instruction_step").expect("capability");
    if supported.trim() != "1" {
        unsafe {
            step_engine::uninstall(interp).expect("uninstall");
            Tcl_DeleteInterp(interp);
        }
        return;
    }

    let script_path = unique_temp_script(
        "opcode-source",
        "proc source_demo {} {\n    set a 1\n    set b 2\n}\nsource_demo\n",
    );
    let canonical = script_path.canonicalize().expect("canonical temp script");
    let tcl_path = canonical.to_string_lossy().replace('}', "\\}");

    eval(interp, "::tcld::dbg::breakpoint_add instruction 0").expect("instruction breakpoint");
    let before = spy.count.load(Ordering::SeqCst);
    eval(interp, &format!("source {{{tcl_path}}}")).expect("source temp script");

    let events = spy.events.lock().unwrap();
    let recovered = events.iter().skip(before).find_map(|event| match event {
        DebugEvent::Paused {
            reason: PauseReason::InstructionBreakpoint,
            location,
            pc,
            ..
        } => match location.block() {
            BlockId::File(path) if std::path::Path::new(path.as_ref()) == canonical.as_path() => {
                Some((*pc, location))
            }
            _ => None,
        },
        _ => None,
    });
    let (_pc, location) = recovered.unwrap_or_else(|| {
        panic!(
            "expected file-backed instruction breakpoint pause, saw {:?}",
            &events[before..]
        )
    });
    match location.block() {
        BlockId::File(path) => {
            assert_eq!(
                std::path::Path::new(path.as_ref()),
                canonical.as_path(),
                "opcode pause should recover the sourced file path"
            );
            assert!(location.line() >= 1, "expected a real source line");
        }
        other => panic!("expected file-backed opcode pause location, got {other:?}"),
    }

    drop(events);
    let _ = std::fs::remove_file(&script_path);
    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn patched_runtime_opcode_profile_counts_without_pausing() {
    let interp = fresh_interp();
    let (_state, spy) = install_full_engine(interp);
    eval(interp, "::tcld::dbg::install").expect("adapter install");
    let supported = eval(interp, "::tcld::dbg::has opcode_profiling").expect("capability");
    if supported.trim() != "1" {
        unsafe {
            step_engine::uninstall(interp).expect("uninstall");
            Tcl_DeleteInterp(interp);
        }
        return;
    }

    eval(interp, "::tcld::dbg::profile_start opcode_counts").expect("start profile");
    let before = spy.count.load(Ordering::SeqCst);
    eval(
        interp,
        "proc profile_demo {} {set a 1\nincr a\nset b $a}; profile_demo",
    )
    .expect("run profile demo");
    eval(interp, "::tcld::dbg::profile_stop").expect("stop profile");

    let total = eval(
        interp,
        r#"
set total 0
foreach {pc count} [dict get [::tcld::dbg::profile_data] data] {
    incr total $count
}
set total
"#,
    )
    .expect("profile data");
    let total: u64 = total.trim().parse().expect("numeric profile total");
    assert!(
        total > 0,
        "opcode profiler should record at least one PC hit"
    );

    let events = spy.events.lock().unwrap();
    assert!(
        !events[before..]
            .iter()
            .any(|event| matches!(event, DebugEvent::Paused { .. })),
        "opcode profiling must not pause execution, saw {:?}",
        &events[before..]
    );

    drop(events);
    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── pre_instrument_proc / pre_instrument_file ─────────────────────

#[test]
fn pre_instrument_proc_registers_block_before_definition() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    let token = eval(
        interp,
        r#"::tcld::dbg::pre_instrument_proc futureproc {set a 1
set b 2}"#,
    )
    .expect("pre_instrument");
    assert_eq!(token, "proc:futureproc");
    let g = state.lock().unwrap();
    let block = g
        .blocks
        .get(&BlockId::ProcBody(Arc::from("futureproc")))
        .expect("registered");
    // Two top-level commands.
    assert_eq!(
        block.valid_offsets().len(),
        2,
        "got {:?}",
        block.valid_offsets()
    );
    drop(g);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn pre_instrument_proc_validates_existing_breakpoints() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    // Register a pre-instrumented body, set a bp, then re-instrument
    // with a different body and verify validate-on-replace runs.
    eval(interp, "::tcld::dbg::pre_instrument_proc demo {set a 1}").unwrap();
    let off = {
        let g = state.lock().unwrap();
        let block = g.blocks.get(&BlockId::ProcBody(Arc::from("demo"))).unwrap();
        block.valid_offsets()[0]
    };
    eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).unwrap();
    // Re-instrument with a longer body. The bp should still be valid.
    eval(
        interp,
        "::tcld::dbg::pre_instrument_proc demo {set a 1\nset b 2}",
    )
    .unwrap();
    let g = state.lock().unwrap();
    let block = g.blocks.get(&BlockId::ProcBody(Arc::from("demo"))).unwrap();
    assert_eq!(block.version(), 1, "redefinition should bump version");
    assert_eq!(block.valid_offsets().len(), 2);
    drop(g);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn pre_instrument_file_registers_file_block() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    let token = eval(
        interp,
        r#"::tcld::dbg::pre_instrument_file /tmp/fake.tcl {puts hello
set z 42}"#,
    )
    .expect("pre_instrument_file");
    assert_eq!(token, "file:/tmp/fake.tcl");
    let g = state.lock().unwrap();
    let block = g
        .blocks
        .get(&BlockId::File(Arc::from("/tmp/fake.tcl")))
        .expect("file block registered");
    assert_eq!(block.valid_offsets().len(), 2);
    drop(g);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn pre_instrumented_block_pauses_when_proc_actually_runs() {
    // End-to-end: pre-instrument, set a bp, then define and run the
    // proc — bp should fire because the wrapper's later registration is
    // idempotent for identical content.
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);
    eval(interp, "::tcld::dbg::pre_instrument_proc demo {set a 1}").unwrap();
    let off = {
        let g = state.lock().unwrap();
        let block = g.blocks.get(&BlockId::ProcBody(Arc::from("demo"))).unwrap();
        block.valid_offsets()[0]
    };
    eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).unwrap();

    // Now actually define and invoke the proc. The proc wrapper will
    // re-register; identical content is idempotent so the existing bp
    // remains valid.
    eval(interp, "proc demo {} {set a 1}").unwrap();
    let pause_count_before = spy.count.load(Ordering::SeqCst);
    eval(interp, "demo").unwrap();
    let pause_count_after = spy.count.load(Ordering::SeqCst);
    assert!(
        pause_count_after > pause_count_before,
        "bp on pre-instrumented body should fire: before={pause_count_before} after={pause_count_after}"
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}
