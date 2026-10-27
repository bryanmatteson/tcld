//! Tier 3 live tests.
//!
//! Async pause, run-to-cursor, watch expressions, persisted breakpoint
//! store, and slave-interp registration. All run against a real
//! `Tcl_Interp` with the engine + wrappers + tier 2 helpers installed.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tcl_sys::{
    TCL_OK, Tcl_CreateInterp, Tcl_DeleteInterp, Tcl_EvalEx, Tcl_GetObjResult, Tcl_GetStringFromObj,
    Tcl_Init, Tcl_Size,
};
use tcld_dbg::step_engine;
use tcld_dbg::wrappers;
use tcld_debug_state::{BlockId, BreakpointKind, DebugEvent, DebugState, WatchValue};

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
        // Re-install tier 2 helpers (engine_install_cmd would have done it,
        // but we use step_engine::install directly to keep our state Arc).
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

// ── Async pause ─────────────────────────────────────────────────────

#[test]
fn request_pause_sets_state_flag_via_atomic() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "::tcld::dbg::request_pause").expect("request_pause");
    {
        let g = state.lock().unwrap();
        assert!(g.pause_requested.load(Ordering::Acquire));
    }

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn request_pause_forces_step_pause_on_next_traced_command() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1\nset b 2\nset c 3 }").unwrap();
    // Enable coverage so the trace handler isn't fully gated out (no
    // breakpoints, no step mode otherwise).
    eval(interp, "::tcld::dbg::coverage_enable on").unwrap();

    eval(interp, "::tcld::dbg::request_pause").unwrap();
    let pause_count_before = spy.count.load(Ordering::SeqCst);
    eval(interp, "demo").unwrap();
    let pause_count_after = spy.count.load(Ordering::SeqCst);
    assert!(
        pause_count_after > pause_count_before,
        "request_pause should have produced at least one pause event"
    );
    // The atomic flag should be cleared after consumption.
    let g = state.lock().unwrap();
    assert!(!g.pause_requested.load(Ordering::Acquire));
    drop(g);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── Run-to-cursor ───────────────────────────────────────────────────

#[test]
fn run_to_adds_temporary_breakpoint_that_disappears_on_hit() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1\nset b 2 }").unwrap();
    let off = first_offset(&state, "demo");
    let bp = eval(interp, &format!("::tcld::dbg::run_to demo {off} 1")).expect("run_to");
    // run_to returned a bp id and resumed. Run the proc to make the bp fire.
    eval(interp, "demo").unwrap();
    // Temporary bps auto-remove on first paused hit. Verify the store.
    let s = state.lock().unwrap();
    let bp_num: u64 = bp.strip_prefix("bp").unwrap().parse().unwrap();
    let inst = s.breakpoints().get(tcld_debug_state::BreakpointId(bp_num));
    assert!(
        inst.is_none(),
        "temporary breakpoint should auto-remove on first hit"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── Watch expressions ───────────────────────────────────────────────

#[test]
fn watch_add_and_list_round_trips() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let id1 = eval(interp, "::tcld::dbg::watch_add length {[llength $items]}").expect("watch_add");
    // Brace the expression so Tcl doesn't try to substitute `$x` at parse time.
    let id2 = eval(interp, "::tcld::dbg::watch_add x {$x}").expect("watch_add");
    assert_eq!(id1, "w0");
    assert_eq!(id2, "w1");
    let listed = eval(interp, "::tcld::dbg::watch_list").expect("watch_list");
    assert!(listed.contains("name {length}"), "got {listed}");
    assert!(listed.contains("name {x}"), "got {listed}");
    assert!(listed.contains("enabled 1"));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn watch_remove_drops_entry() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let id = eval(interp, "::tcld::dbg::watch_add x 42").expect("add");
    eval(interp, &format!("::tcld::dbg::watch_remove {id}")).expect("remove");
    let listed = eval(interp, "::tcld::dbg::watch_list").expect("watch_list");
    assert_eq!(listed, "");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn watch_evaluate_at_global_level_returns_values() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "set ::counter 5").unwrap();
    eval(interp, "set ::flag 1").unwrap();
    eval(interp, "::tcld::dbg::watch_add count {$::counter * 2}").unwrap();
    eval(interp, "::tcld::dbg::watch_add active {$::flag}").unwrap();

    let r = eval(interp, "::tcld::dbg::watch_evaluate 0").expect("evaluate");
    // Result formatting wraps values in braces: `value {10}`.
    assert!(r.contains("value {10}"), "got {r}");
    assert!(r.contains("value {1}"), "got {r}");

    // Results should also be persisted in state.watch_results.
    let g = state.lock().unwrap();
    assert_eq!(g.watch_results.len(), 2);
    assert!(matches!(
        g.watch_results.values().next().unwrap(),
        WatchValue::Ok(_)
    ));
    drop(g);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn watch_evaluate_captures_eval_errors() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    eval(interp, "::tcld::dbg::watch_add bad {[error fail]}").unwrap();
    let r = eval(interp, "::tcld::dbg::watch_evaluate 0").expect("evaluate");
    assert!(r.contains("error"), "expected error in: {r}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── Persisted breakpoint store ──────────────────────────────────────

#[test]
fn bp_export_then_import_round_trips_breakpoints() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1\nset b 2 }").unwrap();
    let off = first_offset(&state, "demo");
    let _bp1 = eval(
        interp,
        &format!("::tcld::dbg::breakpoint_add demo {off} 1 {{$a > 0}}"),
    )
    .expect("add");
    let bp2 = eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).expect("add");
    eval(interp, &format!("::tcld::dbg::bp_set_ignore {bp2} 3")).unwrap();
    eval(interp, &format!("::tcld::dbg::bp_disable {bp2}")).unwrap();

    let exported = eval(interp, "::tcld::dbg::bp_export").expect("export");
    assert!(exported.contains("kind line"));
    assert!(exported.contains("ignore 3"));
    assert!(exported.contains("enabled 0"));
    // Drop and re-import.
    eval(interp, "::tcld::dbg::bp_remove bp0").ok();
    eval(interp, &format!("::tcld::dbg::bp_remove {bp2}")).ok();
    {
        let s = state.lock().unwrap();
        assert_eq!(s.breakpoints().len(), 0);
    }
    let import_script = format!("::tcld::dbg::bp_import {{{exported}}}");
    let _ = eval(interp, &import_script).expect("import");
    let s = state.lock().unwrap();
    assert_eq!(s.breakpoints().len(), 2);
    let mut found_disabled_with_ignore = false;
    let mut found_with_condition = false;
    for bp in s.breakpoints().list() {
        if !bp.enabled && bp.ignore_count == 3 {
            found_disabled_with_ignore = true;
        }
        if bp.condition.is_some() {
            found_with_condition = true;
        }
    }
    assert!(
        found_disabled_with_ignore,
        "expected disabled bp with ignore=3"
    );
    assert!(
        found_with_condition,
        "expected at least one bp with condition"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn bp_import_replaces_existing_breakpoints() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1\nset b 2 }").unwrap();
    let off = first_offset(&state, "demo");
    eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).unwrap();
    {
        let s = state.lock().unwrap();
        assert_eq!(s.breakpoints().len(), 1);
    }
    // Empty import → all bps removed.
    eval(interp, "::tcld::dbg::bp_import {}").expect("import empty");
    let s = state.lock().unwrap();
    assert_eq!(s.breakpoints().len(), 0);
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn bp_export_serializes_exception_kind_with_uncaught_only() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    // Add an exception bp via the state crate (no Tcl wrapper for this).
    {
        let mut g = state.lock().unwrap();
        let id = BlockId::ProcBody(Arc::from("__exc__"));
        g.blocks
            .register(id.clone(), Arc::from("e"), vec![0])
            .unwrap();
        let loc = tcld_debug_state::Location::new(id, 0, 0..1).unwrap();
        g.with_breakpoints_mut(|store, registry| {
            store
                .add(
                    BreakpointKind::Exception {
                        uncaught_only: true,
                    },
                    loc,
                    None,
                    false,
                    registry,
                )
                .unwrap()
        });
    }
    let exported = eval(interp, "::tcld::dbg::bp_export").expect("export");
    assert!(exported.contains("kind exception"));
    assert!(exported.contains("uncaught_only 1"));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── Slave-interp ops ────────────────────────────────────────────────

#[test]
fn interp_create_records_slave_in_state() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "interp create child").expect("interp create");
    let g = state.lock().unwrap();
    assert!(g.slaves.contains("child"), "slaves={:?}", g.slaves);
    drop(g);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn slave_list_returns_known_slaves() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    eval(interp, "interp create alpha").unwrap();
    eval(interp, "interp create beta").unwrap();
    let r = eval(interp, "::tcld::dbg::slave_list").expect("slave_list");
    assert!(r.contains("alpha"), "got {r}");
    assert!(r.contains("beta"), "got {r}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn slave_install_attaches_engine_to_child_interp() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "interp create child").unwrap();
    eval(interp, "::tcld::dbg::slave_install child").expect("slave_install");
    // The parent and child now share state. Adding a proc on the child
    // should be visible in the same state's block registry.
    eval(interp, "interp eval child {proc inchild {} { set z 7 }}").expect("eval in child");
    let g = state.lock().unwrap();
    assert!(
        g.blocks.contains(&BlockId::ProcBody(Arc::from("inchild"))),
        "child's proc body should be registered in shared state"
    );
    drop(g);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn slave_install_errors_for_unknown_slave() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let r = eval(interp, "::tcld::dbg::slave_install ghost");
    assert!(r.is_err(), "expected error, got {:?}", r);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}
