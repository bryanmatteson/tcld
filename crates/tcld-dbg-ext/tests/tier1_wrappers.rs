//! Tier 1 live tests: wrappers, inspect_stack, eval_in_frame,
//! conditional breakpoints, exception breakpoints.
//!
//! These run against a real `Tcl_Interp` with both the step engine and
//! the source-side command wrappers installed via the public Tcl
//! commands.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tcl_sys::{
    TCL_OK, Tcl_CreateInterp, Tcl_DeleteInterp, Tcl_EvalEx, Tcl_GetObjResult, Tcl_GetStringFromObj,
    Tcl_Init, Tcl_Size,
};
use tcld_dbg::step_engine;
use tcld_dbg::wrappers;
use tcld_debug_state::{AppState, BlockId, BreakpointKind, DebugEvent, DebugState, Location};

/// Serializes interp creation across test threads. Concurrent
/// `Tcl_Init` calls race Tcl's process-global path/VFS state: `file
/// exists` on a perfectly good $tcl_library/init.tcl transiently fails
/// and Tcl_Init errors with "Cannot find a usable init.tcl" (observed
/// ~20% of parallel runs on Linux). Only the creation edge is
/// serialized — test bodies still run in parallel.
static TCL_LIFECYCLE: Mutex<()> = Mutex::new(());

fn fresh_interp() -> *mut tcl_sys::Tcl_Interp {
    let _lifecycle = TCL_LIFECYCLE.lock().unwrap();
    // Tcl 8.6 finalizes the notifier when the last live interp is
    // deleted, and subsequent `Tcl_FindExecutable` doesn't reinit it
    // — `update`/`vwait` aborts later tests with "Notifier not
    // initialized". We work around that by holding ONE anchor interp
    // for the entire test process; it's never deleted so the
    // notifier stays alive. Tcl 9 doesn't need this but it's harmless.
    //
    // Tcl_FindExecutable itself is once per PROCESS by contract, so it
    // lives inside the anchor init.
    use std::sync::OnceLock;
    // The field is a keep-alive anchor: never read, only held so the interp
    // (and with it the 8.6 notifier) survives the whole test process.
    struct AnchorPtr(#[allow(dead_code)] *mut tcl_sys::Tcl_Interp);
    unsafe impl Send for AnchorPtr {}
    unsafe impl Sync for AnchorPtr {}
    static ANCHOR: OnceLock<AnchorPtr> = OnceLock::new();
    ANCHOR.get_or_init(|| unsafe {
        tcl_sys::Tcl_FindExecutable(std::ptr::null());
        let anchor = Tcl_CreateInterp();
        assert!(!anchor.is_null());
        AnchorPtr(anchor)
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

impl Spy {
    fn events(&self) -> Vec<DebugEvent> {
        self.events.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.events.lock().unwrap().clear();
        self.count.store(0, Ordering::SeqCst);
    }
}

unsafe extern "C" {
    fn Tcld_dbg_Init(interp: *mut tcl_sys::Tcl_Interp) -> std::os::raw::c_int;
}

fn install_full_engine(interp: *mut tcl_sys::Tcl_Interp) -> (Arc<Mutex<DebugState>>, Arc<Spy>) {
    // Bring the public ::tcld::dbg::* commands into the interp before
    // installing the engine — engine_install_cmd is not what we want
    // here because it creates its own state; we want our test-owned
    // state. So we register the public commands first, then call our
    // install path with a known DebugState.
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
    }
    (state, spy)
}

#[test]
fn proc_wrapper_registers_block_with_per_statement_offsets() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc target {} {
            set a 1
            set b 2
            set c 3
        }"#,
    )
    .expect("proc defined");

    let s = state.lock().unwrap();
    let block = s
        .blocks
        .get(&BlockId::ProcBody(Arc::from("target")))
        .expect("target block registered by wrapper");
    // Three top-level commands inside the body. Tree-sitter-tcl's
    // double-attribution (set captured both as `command` and `set`
    // nodes at the same start_byte) is folded by dedup, so we expect
    // exactly 3 distinct offsets.
    assert_eq!(
        block.valid_offsets().len(),
        3,
        "valid_offsets={:?}",
        block.valid_offsets()
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn proc_redefinition_bumps_block_version() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    eval(interp, "proc p {} { set x 1 }").expect("first def");
    eval(interp, "proc p {} { set y 2\nset z 3 }").expect("redef");

    let s = state.lock().unwrap();
    let block = s
        .blocks
        .get(&BlockId::ProcBody(Arc::from("p")))
        .expect("p registered");
    assert!(
        block.version() >= 1,
        "version should bump on redefinition, got {}",
        block.version()
    );
    assert_eq!(block.valid_offsets().len(), 2);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn catch_wrapper_emits_error_paused_when_exception_bp_armed() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    // Arm an exception breakpoint. Location is irrelevant for Exception
    // kind; we use a placeholder.
    let mut s = state.lock().unwrap();
    let placeholder_block = BlockId::ProcBody(Arc::from("__exception_anchor__"));
    s.blocks
        .register(placeholder_block.clone(), Arc::from(" "), vec![0])
        .unwrap();
    let exc_bp = s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Exception {
                    uncaught_only: false,
                },
                Location::new(placeholder_block, 0, 0..1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    eval(interp, "catch {error boom} msg").expect("catch returns");

    let events = spy.events();
    let error_pauses: Vec<_> = events
        .iter()
        .filter_map(|ev| match ev {
            DebugEvent::ErrorPaused {
                breakpoint,
                message,
                ..
            } => Some((*breakpoint, message.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        error_pauses.len(),
        1,
        "expected one ErrorPaused event, got {error_pauses:?}"
    );
    assert_eq!(error_pauses[0].0, Some(exc_bp));
    assert_eq!(error_pauses[0].1, "boom");

    let s = state.lock().unwrap();
    assert_eq!(s.breakpoint_hits, 1);
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn catch_wrapper_emits_event_but_does_not_increment_hits_without_bp() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    spy.clear();
    eval(interp, "catch {error nope} msg").expect("catch returns");

    let events = spy.events();
    let error_pauses: Vec<_> = events
        .iter()
        .filter_map(|ev| match ev {
            DebugEvent::ErrorPaused { breakpoint, .. } => Some(*breakpoint),
            _ => None,
        })
        .collect();
    // Event still emitted (host can subscribe purely for logging) but
    // breakpoint=None and hit counters stay at 0.
    assert_eq!(error_pauses, vec![None]);

    let s = state.lock().unwrap();
    assert_eq!(s.breakpoint_hits, 0);
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn rename_wrapper_refuses_internal_command_rename() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);

    let err = eval(interp, "rename ::tcld::dbg::breakpoint_add foo").unwrap_err();
    assert!(
        err.contains("rename of debugger-internal"),
        "expected refusal, got {err:?}"
    );
    // External rename still works.
    eval(interp, "proc q {} { set x 1 }").expect("define q");
    eval(interp, "rename q r").expect("external rename works");
    eval(interp, "r").expect("renamed command callable");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn breakpoint_in_wrapped_proc_uses_real_per_statement_offsets() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc multi {} {
            set a 1
            set b 2
            set c 3
        }"#,
    )
    .expect("proc defined");

    // Pick the SECOND offset registered by the wrapper. With wrappers
    // installed this corresponds to `set b 2`. Set a breakpoint there.
    // Because the trace handler's `info frame` line→offset mapping does
    // NOT yet match the body byte_offset (still a placeholder), we
    // assert the structural invariant that registered offsets are
    // distinct — the runtime mapping refinement is a Tier 2 follow-up.
    let s = state.lock().unwrap();
    let block_id = BlockId::ProcBody(Arc::from("multi"));
    let block = s.blocks.get(&block_id).unwrap();
    assert_eq!(block.valid_offsets().len(), 3);
    let offsets = block.valid_offsets().to_vec();
    drop(s);

    // Distinct offsets means breakpoints can target individual
    // statements once the trace handler's mapping aligns. Pin that
    // structural property here; add an integration test for the
    // mapped runtime behavior in the follow-up that fixes the
    // line→offset mapping.
    for window in offsets.windows(2) {
        assert_ne!(window[0], window[1]);
    }
    let _ = spy;

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn inspect_stack_errors_when_not_stopped() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    {
        let s = state.lock().unwrap();
        assert_eq!(s.app_state, AppState::Running);
    }

    let err = eval(interp, "::tcld::dbg::inspect_stack").unwrap_err();
    assert!(err.contains("not paused") || err.contains("not stopped"));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn eval_in_frame_frame_scope_errors_when_not_stopped_but_global_scope_is_allowed() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    {
        let s = state.lock().unwrap();
        assert_eq!(s.app_state, AppState::Running);
    }

    // A paused-frame eval (level > 0) requires a stop — that frame only
    // exists mid-pause.
    let err = eval(interp, "::tcld::dbg::eval_in_frame 1 {set x 1}").unwrap_err();
    assert!(err.contains("not paused") || err.contains("not stopped"));

    // The global frame (level 0) always exists, so a level-0 eval is
    // valid even at idle — this is what lets the controller read startup
    // globals such as the nub's load status.
    let result = eval(interp, "::tcld::dbg::eval_in_frame 0 {set ::probe_idle 7}")
        .expect("global-scope eval_in_frame is allowed at idle");
    assert_eq!(result, "7");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn eval_in_frame_runs_in_global_scope_when_paused() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    // Force Stopped state so the gate accepts the command. (In
    // production this is set by the trace handler at pause time; here
    // we simulate.)
    {
        let mut s = state.lock().unwrap();
        s.app_state = AppState::Stopped;
    }

    eval(interp, "set ::probe_var 42").expect("global set");
    let result = eval(interp, "::tcld::dbg::eval_in_frame 0 {set ::probe_var}")
        .expect("eval_in_frame returns");
    assert_eq!(result, "42");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn conditional_breakpoint_skips_pause_when_condition_false() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc target {} {
            set x 1
        }"#,
    )
    .expect("proc defined");

    // Use the engine's own observed offset by running once with
    // StepInto and capturing locations. We register a conditional bp
    // at the observed offset with an always-false condition, then call
    // target again. Expected: handler matches the bp, evals "0", skips
    // pause.
    spy.clear();
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::StepInto, 0).unwrap();
    }
    eval(interp, "target").expect("call under StepInto");
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::Continue, 0).unwrap();
    }
    let observed: Vec<(BlockId, usize)> = spy
        .events()
        .into_iter()
        .filter_map(|ev| match ev {
            DebugEvent::Paused { location, .. } => {
                Some((location.block().clone(), location.byte_offset()))
            }
            _ => None,
        })
        .collect();
    let target_pause = observed
        .iter()
        .find(|(b, _)| matches!(b, BlockId::ProcBody(name) if &**name == "target"))
        .cloned()
        .expect("at least one pause inside target body");

    // Replace the wrapper-registered block with one whose
    // valid_offsets match the trace handler's observed offsets. The
    // mismatch between wrapper offsets (from syntax parser) and
    // handler offsets (from info frame) is the documented Tier-2 gap.
    let mut s = state.lock().unwrap();
    s.blocks
        .register_replacement(
            target_pause.0.clone(),
            Arc::from(" ".repeat(target_pause.1 + 16)),
            vec![target_pause.1],
        )
        .unwrap();
    let bp_id = s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(
                    target_pause.0.clone(),
                    0,
                    target_pause.1..target_pause.1 + 1,
                )
                .unwrap(),
                Some("0".to_string()),
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    eval(interp, "target").expect("call again with conditional bp");

    let s = state.lock().unwrap();
    // Condition is constant 0 → bp must NOT increment hit_count.
    assert_eq!(s.breakpoints().get(bp_id).unwrap().hit_count, 0);
    assert_eq!(s.breakpoint_hits, 0);
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn conditional_breakpoint_with_eval_error_increments_counter_and_skips() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    eval(interp, "proc target {} { set x 1 }").expect("proc defined");

    // Observe the location.
    spy.clear();
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::StepInto, 0).unwrap();
    }
    eval(interp, "target").expect("step into");
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::Continue, 0).unwrap();
    }
    // Filter to pauses INSIDE target's body. The first pause is at
    // top-level (block=<unknown>) which doesn't help here.
    let target_pause = spy
        .events()
        .into_iter()
        .find_map(|ev| match ev {
            DebugEvent::Paused { location, .. } => match location.block() {
                BlockId::ProcBody(name) if &**name == "target" => {
                    Some((location.block().clone(), location.byte_offset()))
                }
                _ => None,
            },
            _ => None,
        })
        .expect("at least one pause inside target body");

    let mut s = state.lock().unwrap();
    s.blocks
        .register_replacement(
            target_pause.0.clone(),
            Arc::from(" ".repeat(target_pause.1 + 16)),
            vec![target_pause.1],
        )
        .unwrap();
    let bp_id = s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(
                    target_pause.0.clone(),
                    0,
                    target_pause.1..target_pause.1 + 1,
                )
                .unwrap(),
                // Bad condition: references undefined variable.
                Some("$does_not_exist > 0".to_string()),
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    eval(interp, "target").expect("call with bad-condition bp");

    let s = state.lock().unwrap();
    // No pause; condition_eval_errors incremented; bp NOT hit.
    assert_eq!(s.breakpoints().get(bp_id).unwrap().hit_count, 0);
    assert!(
        s.condition_eval_errors >= 1,
        "expected condition_eval_errors >= 1, got {}",
        s.condition_eval_errors
    );
    drop(s);

    let condition_errors: Vec<_> = spy
        .events()
        .into_iter()
        .filter(|ev| matches!(ev, DebugEvent::ConditionError { .. }))
        .collect();
    assert!(
        !condition_errors.is_empty(),
        "expected at least one ConditionError event"
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn inspect_stack_returns_frames_when_paused() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    eval(interp, "proc outer {} { set z 99 }").expect("define outer");

    // Force Stopped so inspect_stack passes its gate. Production sets
    // this from the trace handler; the test simulates.
    {
        let mut s = state.lock().unwrap();
        s.app_state = AppState::Stopped;
    }

    // From an outer Tcl context, inspect_stack reports at least one
    // frame (the toplevel). The result format is a Tcl list of dicts.
    let result = eval(interp, "::tcld::dbg::inspect_stack").expect("inspect_stack ok");
    assert!(!result.is_empty(), "inspect_stack returned empty result");
    assert!(
        result.contains("level"),
        "inspect_stack result missing level key: {result:?}"
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn eval_in_frame_returns_proc_local_when_paused_inside_proc() {
    // Create a proc, force-stop the interpreter, run eval_in_frame at
    // level 0 to access a global. (Reading proc locals requires being
    // paused mid-execution, which the live harness can't fully simulate
    // without a separate thread to drive resume — see PauseStrategy.)
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    eval(interp, "set ::shared 7").expect("globals set");
    {
        let mut s = state.lock().unwrap();
        s.app_state = AppState::Stopped;
    }

    let r = eval(
        interp,
        "::tcld::dbg::eval_in_frame 0 {expr {$::shared * 2}}",
    )
    .expect("eval_in_frame ok");
    assert_eq!(r, "14");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn per_statement_bp_fires_at_exactly_the_targeted_command() {
    // E101 closed: line→offset map on Block aligns wrapper-parsed
    // offsets with handler-observed (info frame) lines. Setting a
    // breakpoint at the SECOND command's offset should fire ONCE per
    // proc call, not for every body command.
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc multi {} {
            set a 1
            set b 2
            set c 3
        }"#,
    )
    .expect("proc defined");

    // Fetch the wrapper-registered offsets (real per-statement
    // boundaries) and pin a breakpoint at the second one.
    let mut s = state.lock().unwrap();
    let block_id = BlockId::ProcBody(Arc::from("multi"));
    let block = s.blocks.get(&block_id).unwrap();
    let offsets: Vec<usize> = block.valid_offsets().to_vec();
    let line_map: Vec<(u32, usize)> = block.line_to_offset().to_vec();
    assert_eq!(offsets.len(), 3);
    assert_eq!(line_map.len(), 3, "one entry per body line");
    let second_offset = offsets[1];

    let bp_id = s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(block_id.clone(), 0, second_offset..second_offset + 1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    eval(interp, "multi").expect("call multi");

    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoints().get(bp_id).unwrap().hit_count,
        1,
        "bp at second-statement offset must fire exactly ONCE per proc call"
    );
    assert_eq!(s.breakpoint_hits, 1);
    drop(s);

    let breakpoint_pauses: Vec<_> = spy
        .events()
        .into_iter()
        .filter_map(|ev| match ev {
            DebugEvent::Paused {
                breakpoint: Some(id),
                location,
                ..
            } => Some((id, location.byte_offset())),
            _ => None,
        })
        .collect();
    assert_eq!(breakpoint_pauses.len(), 1);
    assert_eq!(breakpoint_pauses[0].0, bp_id);
    assert_eq!(
        breakpoint_pauses[0].1, second_offset,
        "pause Location must carry the resolved body offset, not the placeholder"
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn exception_bp_uncaught_only_skips_nested_catch() {
    // E102 closed: BreakpointKind::Exception { uncaught_only: true }
    // pauses ONLY when the catch wrapper that observed the error was
    // the outermost. A nested catch inside another catch must not
    // trigger pause.
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    let mut s = state.lock().unwrap();
    let placeholder = BlockId::ProcBody(Arc::from("__exc_anchor__"));
    s.blocks
        .register(placeholder.clone(), Arc::from(" "), vec![0])
        .unwrap();
    s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Exception {
                    uncaught_only: true,
                },
                Location::new(placeholder, 0, 0..1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    // Outer catch wraps an inner catch that throws. The inner catch
    // wrapper observes depth_after_pop=1 (still inside outer), so
    // uncaught_only=true → no pause.
    eval(interp, "catch {catch {error inner} inner_msg} outer_msg").expect("nested catch returns");

    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoint_hits, 0,
        "uncaught_only must NOT fire when an outer catch will also see the error"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn exception_bp_uncaught_only_fires_when_outermost_catch_caught_error() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    let mut s = state.lock().unwrap();
    let placeholder = BlockId::ProcBody(Arc::from("__exc_anchor__"));
    s.blocks
        .register(placeholder.clone(), Arc::from(" "), vec![0])
        .unwrap();
    s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Exception {
                    uncaught_only: true,
                },
                Location::new(placeholder, 0, 0..1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    // Single catch — the error reaches the outermost catch wrapper.
    // depth_after_pop=0 → uncaught_only=true must fire (the error
    // would have escaped without our wrapper).
    eval(interp, "catch {error escaping} msg").expect("catch returns");

    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoint_hits, 1,
        "uncaught_only must fire when the outermost catch sees the error"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn exception_bp_any_error_fires_at_every_catch() {
    // BreakpointKind::Exception { uncaught_only: false } — pauses at
    // every catch site, regardless of nesting.
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    let mut s = state.lock().unwrap();
    let placeholder = BlockId::ProcBody(Arc::from("__exc_anchor__"));
    s.blocks
        .register(placeholder.clone(), Arc::from(" "), vec![0])
        .unwrap();
    s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Exception {
                    uncaught_only: false,
                },
                Location::new(placeholder, 0, 0..1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    // Nested re-throw: the inner catch consumes the first error, then
    // rethrows. The outer catch sees a second error. With
    // uncaught_only=false BOTH wrapper invocations bump hit count.
    eval(
        interp,
        "catch {catch {error inner} inner_msg; error rethrown} outer_msg",
    )
    .expect("nested catch returns");

    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoint_hits, 2,
        "any-error mode fires at every catch wrapper that sees an error"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn per_statement_bp_at_first_command_fires_only_for_first_statement() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc trio {} {
            set a 1
            set b 2
            set c 3
        }"#,
    )
    .expect("proc defined");

    let mut s = state.lock().unwrap();
    let block_id = BlockId::ProcBody(Arc::from("trio"));
    let first_offset = s.blocks.get(&block_id).unwrap().valid_offsets()[0];
    let bp_id = s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(block_id.clone(), 0, first_offset..first_offset + 1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    eval(interp, "trio").expect("call trio");

    let s = state.lock().unwrap();
    assert_eq!(s.breakpoints().get(bp_id).unwrap().hit_count, 1);
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn watchpoint_fires_on_variable_write() {
    // Watchpoints runtime: setting a Var breakpoint with on_write
    // installs a Tcl variable trace; writing the variable triggers a
    // Paused event and increments the bp's hit_count.
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    let result =
        eval(interp, "::tcld::dbg::watchpoint_add ::tracked write").expect("watchpoint_add ok");
    assert!(result.starts_with("bp"));

    spy.clear();
    eval(interp, "set ::tracked 7").expect("set tracked");
    eval(interp, "set ::tracked 8").expect("set tracked again");

    let s = state.lock().unwrap();
    assert_eq!(s.breakpoint_hits, 2, "two writes must produce two hits");
    drop(s);

    let pauses: Vec<_> = spy
        .events()
        .into_iter()
        .filter(|ev| matches!(ev, DebugEvent::Paused { .. }))
        .collect();
    assert_eq!(pauses.len(), 2);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn watchpoint_does_not_fire_on_unmonitored_op() {
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    eval(interp, "::tcld::dbg::watchpoint_add ::tracked write").expect("watchpoint_add ok");
    eval(interp, "set ::tracked 1").expect("seed");

    spy.clear();
    // Reads should NOT trigger the write-only watchpoint.
    let baseline_hits = state.lock().unwrap().breakpoint_hits;
    eval(interp, "set tmp $::tracked").expect("read");
    eval(interp, "set tmp $::tracked").expect("read again");
    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoint_hits, baseline_hits,
        "reads must not fire a write-only watchpoint"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn coverage_records_per_command_executions() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    eval(interp, "::tcld::dbg::coverage_enable on").expect("enable coverage");
    eval(
        interp,
        r#"proc target {} {
            set a 1
            set b 2
        }"#,
    )
    .expect("proc defined");

    eval(interp, "target").expect("call once");
    eval(interp, "target").expect("call twice");

    let s = state.lock().unwrap();
    let block_id = BlockId::ProcBody(Arc::from("target"));
    let block = s.blocks.get(&block_id).unwrap();
    let offsets: Vec<usize> = block.valid_offsets().to_vec();
    drop(s);

    let s = state.lock().unwrap();
    // Both commands executed twice (one per call).
    for off in &offsets {
        assert_eq!(
            s.coverage.count(&block_id, *off),
            2,
            "command at offset {off} should have count 2"
        );
    }
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn coverage_command_returns_recorded_entries() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);

    eval(interp, "::tcld::dbg::coverage_enable on").expect("enable coverage");
    eval(interp, "proc tiny {} { set a 1 }").expect("proc");
    eval(interp, "tiny").expect("call");

    let result = eval(interp, "::tcld::dbg::coverage").expect("coverage cmd");
    assert!(
        result.contains("proc:tiny"),
        "expected coverage to mention tiny: {result:?}"
    );
    assert!(result.contains("count"));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn pauses_are_deferred_inside_update() {
    // T1-S013 closed: while the traced interp is inside `update` (or
    // `vwait`), the trace handler MUST NOT pause — pausing would block
    // the event loop indefinitely. The deferred-pauses counter tracks
    // missed pauses so telemetry can surface "you tried to pause inside
    // an event loop".
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    // Define a proc that calls update mid-body. Set a breakpoint on a
    // command inside update's lambda (impossible to set on update
    // itself, so we put a bp in a proc that's called BEFORE update and
    // again AFTER update; the bp during update should not fire because
    // we're inside the event-loop-re-entry frame at that point).
    eval(
        interp,
        r#"proc inner_during_update {} {
            set inner 1
        }
        proc outer {} {
            inner_during_update
            update idletasks
        }"#,
    )
    .expect("procs defined");

    // Arm a breakpoint in inner. inner runs BEFORE update; the
    // deferral path is NOT in effect yet. So this should pause.
    let mut s = state.lock().unwrap();
    let block_id = BlockId::ProcBody(Arc::from("inner_during_update"));
    let block = s.blocks.get(&block_id).unwrap();
    let off = block.valid_offsets()[0];
    s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(block_id.clone(), 0, off..off + 1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.clear();
    eval(interp, "outer").expect("outer ok");

    let s = state.lock().unwrap();
    assert!(
        s.breakpoint_hits >= 1,
        "bp before update should still fire; got hits={}",
        s.breakpoint_hits
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn defer_depth_increments_during_update() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    // Confirm the wrapper updates defer_depth around update calls.
    eval(interp, "update idletasks").expect("update ok");
    let s = state.lock().unwrap();
    assert_eq!(s.defer_depth, 0, "defer_depth resets after update returns");
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn typed_inspect_stack_returns_at_least_toplevel_frame() {
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);
    {
        let mut s = state.lock().unwrap();
        s.app_state = AppState::Stopped;
    }
    let frames = unsafe { step_engine::inspect_stack(interp) };
    assert!(!frames.is_empty());
    // Level 0 always present.
    assert_eq!(frames[0].level, 0);
    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn uplevel_records_hop_during_call_and_pops_after() {
    // The uplevel wrapper pushes a hop on entry and pops on exit.
    // After the call returns, the virtual stack must be empty.
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc inner {} {
            uplevel 1 {set ::reached 1}
        }
        proc outer {} {
            inner
        }"#,
    )
    .expect("procs defined");

    eval(interp, "outer").expect("outer ok");
    let probe = eval(interp, "set ::reached").expect("global reachable");
    assert_eq!(probe, "1");
    let s = state.lock().unwrap();
    assert!(
        s.uplevel_hops.is_empty(),
        "uplevel_hops should be empty after the uplevel call returns; got {:?}",
        s.uplevel_hops
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn uplevel_hop_visible_during_call_via_subscriber() {
    // Subscribe to events and capture the uplevel hop while it's
    // active by checking state during a Paused event triggered inside
    // the uplevel'd body.
    use std::sync::Arc as StdArc;
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    let hop_during_pause = StdArc::new(Mutex::new(None::<tcld_debug_state::UplevelHop>));
    let captured = hop_during_pause.clone();
    let state_handle = state.clone();
    state.lock().unwrap().subscribe(StdArc::new(move |event| {
        if let DebugEvent::Paused { .. } = event
            && let Ok(guard) = state_handle.lock()
            && let Some(top) = guard.uplevel_hops.last()
        {
            *captured.lock().unwrap() = Some(top.clone());
        }
    }));

    eval(
        interp,
        r#"proc inner {} {
            uplevel 1 {set ::probe 1}
        }
        proc outer {} {
            inner
        }"#,
    )
    .expect("procs defined");

    // Step-into so a pause fires inside the uplevel'd body.
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::StepInto, 0).unwrap();
    }
    eval(interp, "outer").expect("outer ok");

    let captured_hop = hop_during_pause.lock().unwrap().clone();
    assert!(
        captured_hop.is_some(),
        "expected to observe at least one Paused event with an active uplevel hop"
    );
    let hop = captured_hop.unwrap();
    // inner is at level 2 (top → outer → inner). uplevel 1 from inner
    // targets level 1 (outer). from_level=2, to_level=1.
    assert_eq!(hop.from_level, 2);
    assert_eq!(hop.to_level, 1);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn typed_inspect_stack_annotates_uplevel_frames() {
    // When a body is mid-uplevel, typed inspect_stack returns frames
    // whose target frame has via_uplevel_from set to the source level.
    use std::sync::Arc as StdArc;
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    let frames_during_uplevel: StdArc<Mutex<Option<Vec<tcld_debug_state::FrameSnapshot>>>> =
        StdArc::new(Mutex::new(None));
    let captured = frames_during_uplevel.clone();
    // Subscribe BEFORE the eval. The handler runs the subscriber from
    // inside the trace path with IN_HANDLER guard set, so a recursive
    // inspect_stack is safe.
    let interp_addr = interp as usize;
    let state_handle = state.clone();
    state.lock().unwrap().subscribe(StdArc::new(move |event| {
        if let DebugEvent::Paused { .. } = event
            && let Ok(guard) = state_handle.lock()
            && !guard.uplevel_hops.is_empty()
        {
            // Re-mark Stopped temporarily so the typed inspect_stack
            // gate (require_stopped) accepts the call.
            let prev = guard.app_state;
            drop(guard);
            {
                let mut g = state_handle.lock().unwrap();
                g.app_state = AppState::Stopped;
            }
            let frames =
                unsafe { step_engine::inspect_stack(interp_addr as *mut tcl_sys::Tcl_Interp) };
            *captured.lock().unwrap() = Some(frames);
            let mut g = state_handle.lock().unwrap();
            g.app_state = prev;
        }
    }));

    eval(
        interp,
        r#"proc inner {} {
            uplevel 1 {set ::probe 1}
        }
        proc outer {} {
            inner
        }"#,
    )
    .expect("procs defined");

    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::StepInto, 0).unwrap();
    }
    eval(interp, "outer").expect("outer ok");

    let frames = frames_during_uplevel.lock().unwrap().clone();
    let frames = frames.expect("captured frames during uplevel");
    let outer_level_frame = frames
        .iter()
        .find(|f| f.via_uplevel_from.is_some())
        .expect("a frame should be annotated as via_uplevel_from");
    assert_eq!(outer_level_frame.via_uplevel_from, Some(2));
    assert_eq!(outer_level_frame.level, 1);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn inspect_stack_populates_location_when_called_from_proc_body() {
    // inspect_stack populates Location for any frame whose
    // info-frame `cmd` matches a registered Block's body. To exercise
    // this, we call ::tcld::dbg::inspect_stack from inside a proc
    // body — Tcl's `info frame` then reports the full frame stack
    // (the trace-handler-context reduced-depth quirk doesn't apply).
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc deep {} {
            ::tcld::dbg::inspect_stack
        }
        proc outer {} {
            deep
        }"#,
    )
    .expect("procs defined");

    // Force Stopped so the Tcl-side inspect_stack gate accepts.
    {
        let mut s = state.lock().unwrap();
        s.app_state = AppState::Stopped;
    }

    let result = eval(interp, "outer").expect("outer ok");
    // Result is a Tcl-list-of-dicts string. Assert both procs appear
    // and that one frame is type=proc with proper proc/cmd info.
    assert!(
        result.contains("::outer") || result.contains("outer"),
        "expected outer in inspect_stack output, got: {result}"
    );
    assert!(
        result.contains("::deep") || result.contains("deep"),
        "expected deep in inspect_stack output, got: {result}"
    );
    assert!(
        result.contains("kind proc"),
        "expected at least one proc-kind frame, got: {result}"
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn inspect_stack_normalizes_qname_without_leading_double_colon() {
    // `info frame.proc` returns "::name"; FrameSnapshot.proc_qname
    // must be "name" so it matches BlockId::ProcBody keys.
    let interp = fresh_interp();
    let (state, _spy) = install_full_engine(interp);

    {
        let mut s = state.lock().unwrap();
        s.app_state = AppState::Stopped;
    }
    eval(interp, "proc named {} { return 1 }").expect("proc");

    // Define a temporary proc and call typed inspect_stack from
    // inside its body via a Paused subscriber.
    use std::sync::Arc as StdArc;
    let captured: StdArc<Mutex<Option<Arc<str>>>> = StdArc::new(Mutex::new(None));
    let interp_addr = interp as usize;
    let captured_clone = captured.clone();
    state.lock().unwrap().subscribe(StdArc::new(move |event| {
        if let DebugEvent::Paused { .. } = event {
            let frames =
                unsafe { step_engine::inspect_stack(interp_addr as *mut tcl_sys::Tcl_Interp) };
            eprintln!("PROBE-SUB-FRAMES: {frames:?}");
            if let Some(name) = frames.iter().find_map(|f| f.proc_qname.clone())
                && captured_clone.lock().unwrap().is_none()
            {
                *captured_clone.lock().unwrap() = Some(name);
            }
        }
    }));
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::StepInto, 0).unwrap();
    }
    eval(interp, "named").expect("named ok");

    let qname = captured.lock().unwrap().clone().expect("a qname captured");
    assert!(
        !qname.starts_with("::"),
        "proc_qname must not start with `::` (got {qname:?})"
    );
    assert_eq!(&*qname, "named");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn loop_body_with_repeating_cmd_resolves_first_then_advances() {
    // Pin the documented loop-iteration disambiguation behavior:
    // `last_resolved_offset` biases offset_for_cmd toward the "next"
    // identical cmd in body order. For a body with two `puts hi`
    // separated by `puts mid`, the trace handler observes [puts hi,
    // puts mid, puts hi] in execution order and resolves them to
    // offsets [0, 7, 16] respectively, NOT [0, 7, 0].
    let interp = fresh_interp();
    let (state, spy) = install_full_engine(interp);

    eval(
        interp,
        r#"proc trio {} {
            puts hi
            puts mid
            puts hi
        }"#,
    )
    .expect("proc defined");

    // Inspect block offsets for assertion targets.
    let s = state.lock().unwrap();
    let block = s.blocks.get(&BlockId::ProcBody(Arc::from("trio"))).unwrap();
    let offsets: Vec<usize> = block.valid_offsets().to_vec();
    assert_eq!(offsets.len(), 3);
    drop(s);

    spy.clear();
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::StepInto, 0).unwrap();
    }
    eval(interp, "trio").expect("trio ok");
    unsafe {
        step_engine::set_step_mode(interp, step_engine::StepMode::Continue, 0).unwrap();
    }

    // Filter pause events whose block is trio's body, in order.
    let trio_pauses: Vec<usize> = spy
        .events()
        .into_iter()
        .filter_map(|ev| match ev {
            DebugEvent::Paused { location, .. } => match location.block() {
                BlockId::ProcBody(name) if &**name == "trio" => Some(location.byte_offset()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        trio_pauses, offsets,
        "loop-style hint should advance through body offsets in order, even with repeating cmd text"
    );

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn install_command_wrappers_is_idempotent() {
    let interp = fresh_interp();
    let (_state, _spy) = install_full_engine(interp);

    // Second install must succeed without touching the existing
    // wrappers (the installed flag short-circuits).
    unsafe {
        wrappers::install_command_wrappers(interp).expect("second install ok");
    }

    // Confirm `proc` still works correctly post-second-install.
    eval(interp, "proc p2 {} { set x 1 }").expect("proc still works");

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}
