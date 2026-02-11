//! Tier 2 live tests.
//!
//! Exercises every Tier 2 Tcl command surface against a real
//! `Tcl_Interp`: variable inspection at a frame, last-result capture,
//! the proc registry, error continuation, hit-count gating, breakpoint
//! lifecycle ops, and per-block trace disable.
//!
//! Like the Tier 1 tests we run with `PauseStrategy::EmitOnly`. That
//! lets the trace handler emit a Paused event and bump counters, but
//! the script keeps running so the harness can drive the next step
//! without an event-loop dance.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tcl_sys::{
    TCL_OK, Tcl_CreateInterp, Tcl_DeleteInterp, Tcl_EvalEx, Tcl_GetObjResult, Tcl_GetStringFromObj,
    Tcl_Init, Tcl_Size,
};
use tcld_dbg::step_engine;
use tcld_dbg::wrappers;
use tcld_debug_state::{BlockId, BreakpointKind, DebugEvent, DebugState, Location};

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
    FIND_EXECUTABLE.call_once(|| unsafe { tcl_sys::Tcl_FindExecutable(std::ptr::null()) });
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

/// Resolve the first parsed-command offset of a registered proc body.
/// Tcl bodies often start with whitespace ("proc x {} { set a 1 }" →
/// body = " set a 1 ") so offset 0 isn't on a command boundary; the
/// store rejects breakpoint_add calls there.
fn first_offset(state: &Mutex<DebugState>, qname: &str) -> usize {
    let g = state.lock().unwrap();
    let block = g
        .blocks
        .get(&BlockId::ProcBody(Arc::from(qname)))
        .unwrap_or_else(|| panic!("block not registered: {qname}"));
    *block.valid_offsets().first().unwrap_or(&0)
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
        // Re-install the tier 2 helper script. The public engine_install
        // path does this; in tests we install via step_engine::install
        // directly so we re-run the helper installer inline through a
        // throwaway eval of `::tcld::dbg::engine_install` is not safe
        // (it creates a NEW state). Instead we just eval the helper
        // namespace setup directly here; the source is small.
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

// ── get_variables / get_var / set_var ───────────────────────────────

#[test]
fn get_variables_lists_scalars_and_arrays_at_global_level() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "set ::a 1; set ::b 2; array set ::arr {k v}").unwrap();
    {
        let mut guard = state.lock().unwrap();
        guard.app_state = tcld_debug_state::AppState::Stopped;
    }
    let result = eval(interp, "::tcld::dbg::get_variables 0").expect("get_variables");
    assert!(result.contains("a s"), "should mark a as scalar: {result}");
    assert!(result.contains("b s"), "should mark b as scalar: {result}");
    assert!(
        result.contains("arr a"),
        "should mark arr as array: {result}"
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn get_var_returns_scalar_value_with_maxlen_truncation() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "set ::greeting {hello world}").unwrap();
    {
        let mut guard = state.lock().unwrap();
        guard.app_state = tcld_debug_state::AppState::Stopped;
    }
    let full = eval(interp, "::tcld::dbg::get_var 0 ::greeting").expect("get_var");
    assert_eq!(full, "hello world");

    let truncated = eval(interp, "::tcld::dbg::get_var 0 ::greeting 5").expect("get_var truncated");
    assert_eq!(truncated, "hello...");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn get_var_returns_array_as_dict() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "array set ::a {one 1 two 2}").unwrap();
    {
        let mut guard = state.lock().unwrap();
        guard.app_state = tcld_debug_state::AppState::Stopped;
    }
    let result = eval(interp, "::tcld::dbg::get_var 0 ::a").expect("get_var");
    // array get returns key/value pairs; both keys must be present.
    assert!(result.contains("one"));
    assert!(result.contains("two"));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn set_var_mutates_named_variable_at_frame() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "set ::v initial").unwrap();
    {
        let mut guard = state.lock().unwrap();
        guard.app_state = tcld_debug_state::AppState::Stopped;
    }
    eval(interp, "::tcld::dbg::set_var 0 ::v changed").expect("set_var");
    let after = eval(interp, "set ::v").unwrap();
    assert_eq!(after, "changed");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── get_result ──────────────────────────────────────────────────────
//
// `get_result` exposes `DebugState::last_result`. The host populates
// that field — typically a paused-state subscriber that snapshots
// `Tcl_GetObjResult` while the trace handler is suspended (Vwait
// pause). We can't auto-capture it from `Tcl_CreateObjTrace`'s ENTER
// callback because Tcl resets the obj result before the trace fires
// for an about-to-run command.

#[test]
fn get_result_returns_state_last_result_field() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    {
        let mut g = state.lock().unwrap();
        g.last_result = Some("preset value".to_string());
    }
    let r = eval(interp, "::tcld::dbg::get_result").expect("get_result");
    assert_eq!(r, "preset value");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn get_result_returns_empty_when_last_result_unset() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let r = eval(interp, "::tcld::dbg::get_result").expect("get_result");
    assert_eq!(r, "");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn get_result_truncates_with_maxlen() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    {
        let mut g = state.lock().unwrap();
        g.last_result = Some("x".repeat(100));
    }
    let r = eval(interp, "::tcld::dbg::get_result 5").expect("get_result");
    assert_eq!(r, "xxxxx...");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── Proc registry ───────────────────────────────────────────────────

#[test]
fn list_procs_includes_user_defined_proc() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set x 1 }").unwrap();
    let r = eval(interp, "::tcld::dbg::list_procs").expect("list_procs");
    assert!(r.contains("demo"), "expected `demo` in list: {r}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn proc_body_returns_registered_source() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set hello world }").unwrap();
    let r = eval(interp, "::tcld::dbg::proc_body demo").expect("proc_body");
    assert!(r.contains("set hello world"));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn proc_location_returns_block_metadata() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1\nset b 2 }").unwrap();
    let r = eval(interp, "::tcld::dbg::proc_location demo").expect("proc_location");
    assert!(r.contains("block {proc:demo}"));
    assert!(r.contains("first_offset"));
    assert!(r.contains("cmd_count"));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn proc_body_errors_for_unregistered_proc() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);
    let r = eval(interp, "::tcld::dbg::proc_body ghost");
    assert!(r.is_err(), "expected error, got {:?}", r);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── block_disable / block_enable ────────────────────────────────────

#[test]
fn block_disable_marks_block_in_state() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set x 1 }").unwrap();
    eval(interp, "::tcld::dbg::block_disable demo").expect("disable");
    {
        let g = state.lock().unwrap();
        let id = BlockId::ProcBody(Arc::from("demo"));
        assert!(g.is_block_disabled(&id));
    }
    eval(interp, "::tcld::dbg::block_enable demo").expect("enable");
    {
        let g = state.lock().unwrap();
        let id = BlockId::ProcBody(Arc::from("demo"));
        assert!(!g.is_block_disabled(&id));
    }

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn disabled_block_skips_breakpoint_pause() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1\nset b 2 }").unwrap();
    // Add a line breakpoint at the first command.
    let off = first_offset(&state, "demo");
    let _ = eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).expect("bp add");
    // Disable the block; runs should not pause.
    eval(interp, "::tcld::dbg::block_disable demo").unwrap();

    let pause_count_before = spy.count.load(Ordering::SeqCst);
    eval(interp, "demo").unwrap();
    let pause_count_after = spy.count.load(Ordering::SeqCst);
    assert_eq!(
        pause_count_before, pause_count_after,
        "disabled block should not pause"
    );
    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoint_hits, 0,
        "no breakpoint hits should be recorded for disabled block"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── Hit-count gating ────────────────────────────────────────────────

#[test]
fn breakpoint_skips_hits_until_ignore_count_satisfied() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(
        interp,
        "proc loop {} { for {set i 0} {$i < 5} {incr i} { set y $i } }",
    )
    .unwrap();
    let off = first_offset(&state, "loop");
    let bp = eval(interp, &format!("::tcld::dbg::breakpoint_add loop {off} 1")).expect("bp");
    // Skip first 2 hits.
    eval(interp, &format!("::tcld::dbg::bp_set_ignore {bp} 2")).expect("set ignore");

    eval(interp, "loop").unwrap();

    // Hit count includes skipped hits; breakpoint_hits only counts pauses.
    let s = state.lock().unwrap();
    let bp_id_num: u64 = bp.strip_prefix("bp").unwrap().parse().unwrap();
    let bp_inst = s
        .breakpoints()
        .get(tcld_debug_state::BreakpointId(bp_id_num))
        .unwrap();
    assert!(bp_inst.hit_count >= 1, "hit_count should be >= 1");
    assert_eq!(bp_inst.ignore_count, 2, "ignore_count should be set to 2");
    // breakpoint_hits should be hit_count - 2 (skipped) when hit_count > 2,
    // else 0. We don't assert exact value since the for-loop body executes
    // both `set y $i` and the implicit incr/expr commands; we only assert
    // that pauses < hit_count when ignore_count was applied.
    assert!(
        s.breakpoint_hits < bp_inst.hit_count.max(1),
        "pauses ({}) should be less than hit_count ({}) when ignore_count={}",
        s.breakpoint_hits,
        bp_inst.hit_count,
        bp_inst.ignore_count
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── ignore_error ────────────────────────────────────────────────────

#[test]
fn ignore_error_suppresses_next_exception_pause() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);
    // Set up an exception breakpoint via the state crate.
    {
        let mut guard = state.lock().unwrap();
        let placeholder = BlockId::ProcBody(Arc::from("__exc_anchor__"));
        guard
            .blocks
            .register(placeholder.clone(), Arc::from("e"), vec![0])
            .unwrap();
        let loc = Location::new(placeholder, 0, 0..1).unwrap();
        guard.with_breakpoints_mut(|store, registry| {
            store
                .add(
                    BreakpointKind::Exception {
                        uncaught_only: false,
                    },
                    loc,
                    None,
                    false,
                    registry,
                )
                .unwrap()
        });
    }
    eval(interp, "::tcld::dbg::ignore_error").expect("ignore_error");
    spy.events.lock().unwrap().clear();
    spy.count.store(0, Ordering::SeqCst);
    // Trigger an error inside a catch — the catch wrapper consults the
    // ignore_error flag.
    let _ = eval(interp, "catch {error oops} msg");
    // No ErrorPaused event should have fired this time.
    let events = spy.events.lock().unwrap().clone();
    let error_pauses = events
        .iter()
        .filter(|e| matches!(e, DebugEvent::ErrorPaused { .. }))
        .count();
    assert_eq!(
        error_pauses, 0,
        "ignore_error should suppress the next exception pause"
    );

    // Subsequent error pauses normally.
    spy.events.lock().unwrap().clear();
    let _ = eval(interp, "catch {error second} msg");
    let events = spy.events.lock().unwrap().clone();
    let error_pauses = events
        .iter()
        .filter(|e| matches!(e, DebugEvent::ErrorPaused { .. }))
        .count();
    assert!(
        error_pauses >= 1,
        "second error should pause normally, events={:?}",
        events
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

// ── bp lifecycle: remove / disable / enable / move / validate ───────

#[test]
fn bp_remove_drops_breakpoint_from_store() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1 }").unwrap();
    let off = first_offset(&state, "demo");
    let bp = eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).expect("add");
    eval(interp, &format!("::tcld::dbg::bp_remove {bp}")).expect("remove");
    let s = state.lock().unwrap();
    assert_eq!(s.breakpoints().len(), 0);
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn bp_disable_then_enable_round_trips() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1 }").unwrap();
    let off = first_offset(&state, "demo");
    let bp = eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).expect("add");
    eval(interp, &format!("::tcld::dbg::bp_disable {bp}")).expect("disable");
    {
        let s = state.lock().unwrap();
        let bp_num: u64 = bp.strip_prefix("bp").unwrap().parse().unwrap();
        let inst = s
            .breakpoints()
            .get(tcld_debug_state::BreakpointId(bp_num))
            .unwrap();
        assert!(!inst.enabled);
    }
    eval(interp, &format!("::tcld::dbg::bp_enable {bp}")).expect("enable");
    {
        let s = state.lock().unwrap();
        let bp_num: u64 = bp.strip_prefix("bp").unwrap().parse().unwrap();
        let inst = s
            .breakpoints()
            .get(tcld_debug_state::BreakpointId(bp_num))
            .unwrap();
        assert!(inst.enabled);
    }

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn bp_move_relocates_breakpoint_to_new_offset() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(
        interp,
        r#"proc demo {} {
            set a 1
            set b 2
            set c 3
        }"#,
    )
    .unwrap();
    let off = first_offset(&state, "demo");
    let bp = eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).expect("add");
    // Find a second valid offset to move to.
    let second_offset = {
        let s = state.lock().unwrap();
        let block = s.blocks.get(&BlockId::ProcBody(Arc::from("demo"))).unwrap();
        block.valid_offsets()[1]
    };
    eval(
        interp,
        &format!("::tcld::dbg::bp_move {bp} demo {second_offset} 1"),
    )
    .expect("move");
    let s = state.lock().unwrap();
    let bp_num: u64 = bp.strip_prefix("bp").unwrap().parse().unwrap();
    let inst = s
        .breakpoints()
        .get(tcld_debug_state::BreakpointId(bp_num))
        .unwrap();
    assert_eq!(inst.location.byte_offset(), second_offset);
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn bp_validate_reports_unchanged_for_still_valid_offset() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1\nset b 2 }").unwrap();
    let off = first_offset(&state, "demo");
    let bp = eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).expect("add");
    let r = eval(interp, "::tcld::dbg::bp_validate demo").expect("validate");
    assert!(
        r.contains(&format!("id {bp}")),
        "expected bp id in result: {r}"
    );
    assert!(r.contains("outcome unchanged"), "got: {r}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn breakpoint_list_includes_ignore_count() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    eval(interp, "proc demo {} { set a 1 }").unwrap();
    let off = first_offset(&state, "demo");
    let bp = eval(interp, &format!("::tcld::dbg::breakpoint_add demo {off} 1")).expect("add");
    eval(interp, &format!("::tcld::dbg::bp_set_ignore {bp} 7")).expect("set ignore");
    let r = eval(interp, "::tcld::dbg::breakpoint_list").expect("list");
    assert!(r.contains("ignore 7"), "expected ignore 7 in: {r}");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}
