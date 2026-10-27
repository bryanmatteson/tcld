//! Live integration tests for the step engine.
//!
//! These tests construct a real Tcl_Interp via tcl-sys, install the
//! step engine, evaluate a Tcl script, and assert the trace handler
//! observed the expected sequence of pauses. The pause strategy is
//! `PauseStrategy::EmitOnly` — pause events are emitted and counters
//! advance, but the handler doesn't enter Tcl's event loop. EmitOnly
//! is a real production-shape strategy (used by poll-based hosts and
//! headless inspection tools), not a test-only path.
//!
//! Tests live here, not in `crates/tcld-debug-state/tests/`, because
//! they require a linked Tcl runtime and the `Tcl_CmdObjTraceProc` FFI
//! machinery from this crate.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tcl_sys::{
    TCL_OK, Tcl_CreateInterp, Tcl_DeleteInterp, Tcl_EvalEx, Tcl_GetObjResult, Tcl_GetStringFromObj,
    Tcl_Init, Tcl_Size,
};
use tcld_dbg::step_engine::{self, StepMode};
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

/// Spy callback that records every Paused event for later assertion.
#[derive(Default)]
struct PauseSpy {
    paused_at: Mutex<Vec<DebugEvent>>,
    pause_count: AtomicUsize,
}

impl PauseSpy {
    fn pause_count(&self) -> usize {
        self.pause_count.load(Ordering::SeqCst)
    }

    fn paused(&self) -> Vec<DebugEvent> {
        self.paused_at.lock().unwrap().clone()
    }
}

fn install_with_spy(interp: *mut tcl_sys::Tcl_Interp) -> (Arc<Mutex<DebugState>>, Arc<PauseSpy>) {
    let state = Arc::new(Mutex::new(DebugState::new()));
    let spy = Arc::new(PauseSpy::default());
    {
        let mut guard = state.lock().unwrap();
        let s = spy.clone();
        guard.subscribe(Arc::new(move |event| {
            if matches!(event, DebugEvent::Paused { .. }) {
                s.pause_count.fetch_add(1, Ordering::SeqCst);
                s.paused_at.lock().unwrap().push(event);
            }
        }));
    }
    unsafe {
        step_engine::install(interp, state.clone()).expect("install");
        step_engine::set_pause_strategy(interp, step_engine::PauseStrategy::EmitOnly);
    }
    (state, spy)
}

#[test]
fn engine_install_is_idempotent() {
    let interp = fresh_interp();
    let state = Arc::new(Mutex::new(DebugState::new()));
    unsafe {
        step_engine::install(interp, state.clone()).expect("first install");
        // Second install must be a no-op (returns Ok, does not panic, does
        // not register a second trace).
        step_engine::install(interp, Arc::new(Mutex::new(DebugState::new())))
            .expect("second install");
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn install_transitions_app_state_dead_to_running() {
    let interp = fresh_interp();
    let (state, _spy) = install_with_spy(interp);
    let s = state.lock().unwrap();
    assert_eq!(s.app_state, AppState::Running);
    drop(s);
    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn step_into_pauses_on_every_command() {
    let interp = fresh_interp();
    let (state, spy) = install_with_spy(interp);

    // Put the engine in StepInto mode. From the perspective of the test,
    // we never reached a real Stopped state via a breakpoint pause —
    // we're driving the mode directly. The handler still consults mode
    // on each command.
    unsafe {
        step_engine::set_step_mode(interp, StepMode::StepInto, 0).expect("set step into");
    }

    // Evaluate a 3-command script. Expect at least 3 pauses (one per
    // top-level command).
    eval(interp, "set x 1\nset y 2\nset z 3").expect("script ok");

    // step_pauses counter should reflect the same.
    let s = state.lock().unwrap();
    assert!(
        s.step_pauses >= 3,
        "expected step_pauses >= 3, got {}",
        s.step_pauses
    );
    assert!(
        spy.pause_count() >= 3,
        "spy saw {} pauses, expected >= 3",
        spy.pause_count()
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

/// Observe the (block, byte_offset) pairs the trace handler emits for a
/// known script under StepInto, then return them. Lets tests bind
/// breakpoints to the same offsets the handler will see at run time —
/// robust against the Tier-0 line→offset placeholder mapping which the
/// future wrapper-commands work will refine.
fn observe_locations(
    interp: *mut tcl_sys::Tcl_Interp,
    spy: &PauseSpy,
    script: &str,
) -> Vec<(BlockId, usize)> {
    spy.paused_at.lock().unwrap().clear();
    spy.pause_count.store(0, Ordering::SeqCst);
    unsafe {
        step_engine::set_step_mode(interp, StepMode::StepInto, 0).expect("step mode");
    }
    eval(interp, script).expect("script ok");
    unsafe {
        step_engine::set_step_mode(interp, StepMode::Continue, 0).expect("step mode");
    }
    spy.paused()
        .into_iter()
        .filter_map(|ev| match ev {
            DebugEvent::Paused { location, .. } => {
                Some((location.block().clone(), location.byte_offset()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn breakpoint_in_proc_body_fires_under_armed_offset() {
    // Tier-0 line→offset mapping limitation: Tcl's `info frame` for
    // eval-loaded proc bodies reports a single `line` for the whole
    // body, not per-statement. Until the wrapper-commands work
    // (Tier 1 #5) registers proc bodies with real per-statement offsets
    // derived from the source, breakpoints are body-granular: one
    // armed offset on a proc body fires for every command in that body.
    //
    // This test pins that observed behavior. When the wrapper-commands
    // refinement lands, this test should be tightened to per-statement
    // expectations. Until then, "engine fires when an offset is armed
    // and the proc is entered" is the load-bearing acceptance.
    let interp = fresh_interp();
    let (state, spy) = install_with_spy(interp);

    eval(
        interp,
        r#"proc target {} {
            set a 1
            set b 2
            set c 3
        }"#,
    )
    .expect("proc defined");

    let observed = observe_locations(interp, &spy, "target");
    let target_pauses: Vec<&(BlockId, usize)> = observed
        .iter()
        .filter(|(b, _)| matches!(b, BlockId::ProcBody(name) if &**name == "target"))
        .collect();
    assert!(
        !target_pauses.is_empty(),
        "expected >= 1 pause inside target body"
    );
    let body_pause_count = target_pauses.len();
    let (target_block, target_offset) = target_pauses[0].clone();

    let mut s = state.lock().unwrap();
    let mut offsets: Vec<usize> = target_pauses.iter().map(|(_, o)| *o).collect();
    offsets.sort_unstable();
    offsets.dedup();
    let max_off = *offsets.last().unwrap();
    s.blocks
        .register(
            target_block.clone(),
            Arc::from(" ".repeat(max_off + 16)),
            offsets,
        )
        .expect("block registers");
    let bp_id = s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(target_block.clone(), 0, target_offset..target_offset + 1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.paused_at.lock().unwrap().clear();
    spy.pause_count.store(0, Ordering::SeqCst);
    eval(interp, "target").expect("call target");

    let s = state.lock().unwrap();
    // body-granular: every body command at the armed offset fires the bp.
    assert_eq!(
        s.breakpoint_hits as usize, body_pause_count,
        "armed offset fires for every body command at that offset (Tier-0 granularity)"
    );
    assert_eq!(
        s.breakpoints().get(bp_id).unwrap().hit_count as usize,
        body_pause_count
    );
    drop(s);

    let breakpoint_pauses: Vec<_> = spy
        .paused()
        .into_iter()
        .filter_map(|ev| match ev {
            DebugEvent::Paused {
                breakpoint: Some(id),
                ..
            } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(breakpoint_pauses.len(), body_pause_count);
    assert!(breakpoint_pauses.iter().all(|id| *id == bp_id));

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn breakpoint_at_unarmed_offset_does_not_fire() {
    // Negative case: register the block under a DIFFERENT proc qname
    // than the one being executed. The trace handler looks up the
    // running proc's Block; no match → no resolved offset → no
    // breakpoint fires. This pins the "armed offset must align with
    // the executing proc's block" invariant.
    let interp = fresh_interp();
    let (state, spy) = install_with_spy(interp);

    eval(
        interp,
        r#"proc target {} {
            set a 1
            set b 2
        }"#,
    )
    .expect("proc defined");

    // Register a Block for an UNRELATED proc qname (not "target") with
    // a single valid offset, then arm a breakpoint there. Calling
    // `target` must NOT fire it because the cmd's BlockId is
    // ProcBody("target") and the bp is on ProcBody("ghost").
    let ghost_block = BlockId::ProcBody(Arc::from("ghost"));
    let mut s = state.lock().unwrap();
    s.blocks
        .register(ghost_block.clone(), Arc::from("set unused 1"), vec![0])
        .unwrap();
    s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(ghost_block.clone(), 0, 0..1).unwrap(),
                None,
                false,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.paused_at.lock().unwrap().clear();
    spy.pause_count.store(0, Ordering::SeqCst);
    eval(interp, "target").expect("ok");

    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoint_hits, 0,
        "bp on a different block must not fire when an unrelated proc runs"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn lock_free_hot_path_no_pause_when_no_breakpoints_or_step() {
    // When step mode is Continue and no breakpoints are armed, the trace
    // handler must NOT lock the state mutex on its no-pause path. We
    // verify this indirectly: install the engine, evaluate many
    // commands with Continue mode and zero breakpoints, and assert the
    // counters are zero AND that the breakpoint store mutex was never
    // contended (we hold it exclusively across the eval).
    let interp = fresh_interp();
    let (state, spy) = install_with_spy(interp);

    // Hold the mutex across eval. If the handler tried to lock state,
    // we'd deadlock. The lock-free hot path means this is safe.
    let guard = state.lock().expect("lock state");
    drop(guard); // we just demonstrated lockability before eval

    // Re-acquire and hold during eval.
    let held = state.lock().expect("hold state across eval");
    eval(interp, "set i 0\nincr i\nincr i\nincr i\nincr i").expect("eval ok");
    // Counters: still zero (no pauses).
    assert_eq!(held.step_pauses, 0);
    assert_eq!(held.breakpoint_hits, 0);
    drop(held);

    assert_eq!(spy.pause_count(), 0);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}

#[test]
fn temporary_breakpoint_fires_once_then_removes_itself() {
    let interp = fresh_interp();
    let (state, spy) = install_with_spy(interp);

    eval(
        interp,
        r#"proc once {} {
            set a 1
            set b 2
        }"#,
    )
    .expect("proc defined");

    // Observe locations.
    let observed = observe_locations(interp, &spy, "once");
    let target_pauses: Vec<&(BlockId, usize)> = observed
        .iter()
        .filter(|(b, _)| matches!(b, BlockId::ProcBody(name) if &**name == "once"))
        .collect();
    assert!(
        !target_pauses.is_empty(),
        "expected at least one pause inside once"
    );
    let (block, offset) = target_pauses[0].clone();

    let mut s = state.lock().unwrap();
    let mut offsets: Vec<usize> = target_pauses.iter().map(|(_, o)| *o).collect();
    offsets.sort_unstable();
    offsets.dedup();
    let max_off = *offsets.last().unwrap();
    s.blocks
        .register(block.clone(), Arc::from(" ".repeat(max_off + 16)), offsets)
        .unwrap();
    let bp_id = s.with_breakpoints_mut(|store, registry| {
        store
            .add(
                BreakpointKind::Line,
                Location::new(block.clone(), 0, offset..offset + 1).unwrap(),
                None,
                /* temporary */ true,
                registry,
            )
            .unwrap()
    });
    drop(s);

    spy.paused_at.lock().unwrap().clear();
    spy.pause_count.store(0, Ordering::SeqCst);
    eval(interp, "once").expect("call once");

    // Temporary bp removes itself after the first hit; subsequent body
    // commands during the same call do NOT re-fire (the bp is gone from
    // the armed set after the first record_hit).
    let s = state.lock().unwrap();
    assert_eq!(
        s.breakpoint_hits, 1,
        "temporary bp fires exactly once (self-removal after first hit)"
    );
    assert!(
        s.breakpoints().get(bp_id).is_none(),
        "temporary bp removed after hit"
    );
    drop(s);

    unsafe {
        step_engine::uninstall(interp).expect("uninstall");
        Tcl_DeleteInterp(interp);
    }
}
