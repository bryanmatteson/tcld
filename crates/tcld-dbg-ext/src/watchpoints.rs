//! Watchpoint runtime (Tier 2 #9, pulled forward).
//!
//! Wires `BreakpointKind::Var { name, on_read, on_write, on_unset }` to
//! Tcl's `trace add variable` so the trace fires on the configured ops.
//! The trace's body calls a Rust-registered helper command that records
//! the hit on the breakpoint store and emits a `DebugEvent::Paused` with
//! a synthesized location naming the variable.
//!
//! Why Tcl-side rather than `Tcl_TraceVar2`: the Tcl-side trace
//! infrastructure handles array-element ops uniformly and gets the
//! variable interpolation right (e.g. `arr(elem)` syntax). The Rust
//! handler stays simple and the FFI surface stays small.

use std::os::raw::{c_int, c_void};
use std::sync::Arc;

use tcl_sys::{TCL_OK, Tcl_EvalEx, Tcl_Interp, Tcl_Obj};
use tcld_debug_state::{BlockId, BreakpointId, BreakpointKind, DebugEvent, DebugState, Location};

use crate::step_engine;

/// Tcl script that installs the variable trace for a given watchpoint.
/// Variables are referenced by name; array elements are addressable as
/// `arr(elem)`. The dispatch helper is `::tcld::dbg::__on_var_trace`.
fn build_install_script(name: &str, ops: &str, bp_id: u64) -> String {
    // Disable interpolation by quoting carefully; bp_id is numeric so
    // it's safe to embed directly.
    format!(
        "trace add variable {{{name}}} {{{ops}}} \
         [list ::tcld::dbg::__on_var_trace {bp_id}]"
    )
}

fn build_remove_script(name: &str, ops: &str, bp_id: u64) -> String {
    format!(
        "trace remove variable {{{name}}} {{{ops}}} \
         [list ::tcld::dbg::__on_var_trace {bp_id}]"
    )
}

fn ops_string(on_read: bool, on_write: bool, on_unset: bool) -> String {
    let mut parts = Vec::with_capacity(3);
    if on_read {
        parts.push("read");
    }
    if on_write {
        parts.push("write");
    }
    if on_unset {
        parts.push("unset");
    }
    parts.join(" ")
}

/// Install the Tcl-side variable trace for a `BreakpointKind::Var`
/// breakpoint that's already registered in the BreakpointStore.
///
/// # Safety
///
/// `interp` must be a valid Tcl interpreter pointer with the helper
/// command `::tcld::dbg::__on_var_trace` registered (done by
/// [`install_var_trace_helper`]).
pub unsafe fn arm_watchpoint(
    interp: *mut Tcl_Interp,
    name: &str,
    on_read: bool,
    on_write: bool,
    on_unset: bool,
    bp_id: BreakpointId,
) -> Result<(), String> {
    let ops = ops_string(on_read, on_write, on_unset);
    if ops.is_empty() {
        return Err("watchpoint must have at least one of read/write/unset".to_string());
    }
    let script = build_install_script(name, &ops, bp_id.0);
    let cstr = std::ffi::CString::new(script).map_err(|_| "script NUL".to_string())?;
    let rc = unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
    if rc != TCL_OK {
        return Err("trace add variable failed".to_string());
    }
    Ok(())
}

/// Remove the trace for a `BreakpointKind::Var` breakpoint. Called by
/// the host before removing the breakpoint from the store; otherwise
/// the trace would still fire and the helper would no-op.
///
/// # Safety
///
/// `interp` must be the same pointer used in [`arm_watchpoint`].
pub unsafe fn disarm_watchpoint(
    interp: *mut Tcl_Interp,
    name: &str,
    on_read: bool,
    on_write: bool,
    on_unset: bool,
    bp_id: BreakpointId,
) -> Result<(), String> {
    let ops = ops_string(on_read, on_write, on_unset);
    if ops.is_empty() {
        return Ok(());
    }
    let script = build_remove_script(name, &ops, bp_id.0);
    let cstr = std::ffi::CString::new(script).map_err(|_| "script NUL".to_string())?;
    unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
    Ok(())
}

/// # Safety
///
/// `interp` must be a valid, live `Tcl_Interp` pointer owned by the calling
/// thread; registers `::tcld::dbg::__on_var_trace` on it.
pub unsafe fn install_var_trace_helper(interp: *mut Tcl_Interp) -> Result<(), String> {
    let cname = std::ffi::CString::new("::tcld::dbg::__on_var_trace")
        .map_err(|_| "bad name".to_string())?;
    unsafe {
        tcl_sys::Tcl_CreateObjCommand(
            interp,
            cname.as_ptr(),
            Some(on_var_trace_cmd),
            std::ptr::null_mut(),
            None,
        );
    }
    Ok(())
}

fn lookup_state(interp: *mut Tcl_Interp) -> Option<Arc<std::sync::Mutex<DebugState>>> {
    step_engine::lookup_ctx(interp).map(|ctx_ptr| unsafe { (*ctx_ptr).state.clone() })
}

unsafe extern "C" fn on_var_trace_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // trace add variable invokes the script as:
    //   {prefix args} name1 name2 op
    // where prefix args is what we registered (`__on_var_trace bp_id`).
    // So objv = [__on_var_trace, bp_id, name1, name2, op]. objc = 5.
    if objc < 5 {
        // Don't error — that would propagate as a Tcl error. Just
        // ignore mis-shaped invocations.
        return TCL_OK;
    }
    let bp_id = match unsafe { obj_to_string(*objv.add(1)) }.parse::<u64>() {
        Ok(n) => BreakpointId(n),
        Err(_) => return TCL_OK,
    };
    let name1 = unsafe { obj_to_string(*objv.add(2)) };
    let name2 = unsafe { obj_to_string(*objv.add(3)) };
    let op = unsafe { obj_to_string(*objv.add(4)) };

    let Some(state) = lookup_state(interp) else {
        return TCL_OK;
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return TCL_OK,
    };
    // Verify the bp still exists and is enabled.
    let still_active = guard
        .breakpoints()
        .get(bp_id)
        .map(|bp| bp.enabled && matches!(bp.kind, BreakpointKind::Var { .. }))
        .unwrap_or(false);
    if !still_active {
        return TCL_OK;
    }

    // Synthesize a Location naming the variable. The block is anonymous
    // because var traces aren't tied to a source location.
    let display_name = if name2.is_empty() {
        name1.clone()
    } else {
        format!("{name1}({name2})")
    };
    let block_id = BlockId::ProcBody(Arc::from(format!("__var_trace_{display_name}__")));
    if !guard.blocks.contains(&block_id) {
        let _ = guard
            .blocks
            .register(block_id.clone(), Arc::from("var"), vec![0]);
    }
    let location = Location::new(block_id, 0, 0..1).expect("non-empty");
    guard.breakpoint_hits = guard.breakpoint_hits.saturating_add(1);
    guard.with_breakpoints_mut(|store, _| store.record_hit(bp_id));
    drop(guard);
    tcld_debug_state::fire_event_unlocked(
        &state,
        DebugEvent::Paused {
            location,
            frame_count: 0,
            breakpoint: Some(bp_id),
            reason: tcld_debug_state::PauseReason::Watchpoint,
            pc: None,
        },
    );
    let _ = op;
    TCL_OK
}

unsafe fn obj_to_string(obj: *mut Tcl_Obj) -> String {
    if obj.is_null() {
        return String::new();
    }
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { tcl_sys::Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len <= 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}
