//! Source-side command wrappers (Tier 1 #5).
//!
//! Installs Tcl-side proc shims that intercept `proc`, `source`, `catch`,
//! `apply`, `namespace eval`, `eval`, and `rename` so the debugger can:
//!
//! - **proc / apply / namespace eval / eval**: register a [`Block`] for
//!   the body source so the trace handler resolves real per-statement
//!   breakpoint offsets. Closes the Tier-0 "body-granular only"
//!   limitation.
//! - **source**: register a `BlockId::File` for every sourced file.
//! - **catch / error**: when an error is detected, decide whether to
//!   pause based on `BreakpointKind::Exception`. Closes S020b.
//! - **rename**: refuse to rename `::tcld::dbg::*` and the wrapper
//!   helpers themselves so the debugger can't be partially uninstalled.
//!
//! Strategy: each wrapper calls a Rust-registered helper command (e.g.
//! `::tcld::dbg::__on_proc_define name body`) before delegating to the
//! original built-in via `rename`. The Rust helpers update the shared
//! `DebugState` directly. This is more reliable than C-level command
//! replacement because it preserves Tcl's argument parsing semantics.
//!
//! See TclPro's `nub.tcl` for the original design (DbgNub_WrapCommands).

use crate::step_engine;
use std::os::raw::{c_int, c_void};
use std::sync::Arc;

use tcl_sys::{
    TCL_ERROR, TCL_OK, Tcl_EvalEx, Tcl_GetObjResult, Tcl_GetStringFromObj, Tcl_Interp, Tcl_Obj,
    Tcl_SetObjResult,
};
use tcld_debug_state::{BlockChange, BlockId, DebugEvent, DebugState, register_body_block};

const WRAPPER_INSTALL_SCRIPT: &str = r#"
namespace eval ::tcld::dbg::__wrap__ {}

# Install once. Re-running is a no-op.
if {[info exists ::tcld::dbg::__wrap__::installed]} {
    return
}
set ::tcld::dbg::__wrap__::installed 1

# --- proc -----------------------------------------------------------
rename ::proc ::tcld::dbg::__wrap__::orig_proc
::tcld::dbg::__wrap__::orig_proc ::proc {name args body} {
    set qname [uplevel 1 [list ::tcld::dbg::__wrap__::resolve_qname $name]]
    set owner [info script]
    if {$owner ne ""} { set owner [file normalize $owner] }
    ::tcld::dbg::__on_proc_define $qname $body $owner
    if {[llength [info commands ::tcld::dbg::__nub__::resolve_pending_proc_breakpoints]]} {
        ::tcld::dbg::__nub__::resolve_pending_proc_breakpoints $qname
    }
    uplevel 1 [list ::tcld::dbg::__wrap__::orig_proc $name $args $body]
}

# Resolve a proc name to its fully-qualified form within the caller's
# namespace. Used by the proc wrapper so BlockId::ProcBody keys match
# what `info frame` reports at run time.
::tcld::dbg::__wrap__::orig_proc ::tcld::dbg::__wrap__::resolve_qname {name} {
    if {[string match "::*" $name]} { return $name }
    set ns [uplevel 1 namespace current]
    if {$ns eq "::"} { return "::$name" }
    return "${ns}::$name"
}

# --- source ---------------------------------------------------------
rename ::source ::tcld::dbg::__wrap__::orig_source
::tcld::dbg::__wrap__::orig_proc ::source {args} {
    # Last argument is the path; preceding args are options. Mirror the
    # built-in's interface without enumerating all of -encoding etc.
    set path [file normalize [lindex $args end]]
    set tracked 0
    if {[file readable $path]} {
        set fd [::tcld::dbg::__wrap__::orig_source_open $path]
        set body [read $fd]
        close $fd
        ::tcld::dbg::__on_source_load $path $body
        set tracked 1
        # File/line breakpoint intent is durable and may be staged before
        # this sourced unit exists. Resolve it only after the owner registry
        # has accepted the canonical file block, before any command runs.
        if {[llength [info commands ::tcld::dbg::__nub__::resolve_pending_breakpoints]]} {
            ::tcld::dbg::__nub__::resolve_pending_breakpoints $path
        }
    }
    set normalized_args [lreplace $args end end $path]
    set rc [::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__wrap__::orig_uplevel 1 \
            [list ::tcld::dbg::__wrap__::orig_source {*}$normalized_args]} result options]
    if {$tracked} { ::tcld::dbg::__on_source_exit }
    return -options $options $result
}
::tcld::dbg::__wrap__::orig_proc ::tcld::dbg::__wrap__::orig_source_open {path} {
    open $path r
}

# --- catch ----------------------------------------------------------
# Tracks catch nesting depth in ::tcld::dbg::__wrap__::catch_depth so the
# Rust handler can distinguish "error caught by an outer catch" from
# "error escaping all catches" (E102 semantics).
set ::tcld::dbg::__wrap__::catch_depth 0
rename ::catch ::tcld::dbg::__wrap__::orig_catch
::tcld::dbg::__wrap__::orig_proc ::catch {script args} {
    incr ::tcld::dbg::__wrap__::catch_depth
    set rc [uplevel 1 [list ::tcld::dbg::__wrap__::orig_catch $script {*}$args]]
    incr ::tcld::dbg::__wrap__::catch_depth -1
    if {$rc != 0} {
        set msg ""
        if {[llength $args] >= 1} {
            set msg [uplevel 1 [list set [lindex $args 0]]]
        }
        # depth_after_pop is the catch nesting level the error WOULD
        # see next. 0 means this catch was the outermost — the error
        # would have escaped without our wrapping.
        set depth_after_pop $::tcld::dbg::__wrap__::catch_depth
        ::tcld::dbg::__on_catch_error $rc $msg $depth_after_pop
    }
    return $rc
}

# --- apply ----------------------------------------------------------
# `apply {args body ?ns?} ?arg ...?` — body is the second element of the
# lambda list. We re-register the anonymous body as a Block on every
# call; anonymous IDs collide on identical content so this is cheap.
rename ::apply ::tcld::dbg::__wrap__::orig_apply
::tcld::dbg::__wrap__::orig_proc ::apply {lambda args} {
    set tracked 0
    if {[llength $lambda] >= 2} {
        set owner [info script]
        if {$owner ne ""} { set owner [file normalize $owner] }
        ::tcld::dbg::__on_anonymous_body [lindex $lambda 1] $owner
        set tracked 1
    }
    set rc [::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__wrap__::orig_uplevel 1 \
            [list ::tcld::dbg::__wrap__::orig_apply $lambda {*}$args]} result options]
    if {$tracked} { ::tcld::dbg::__on_anonymous_exit }
    return -options $options $result
}

# --- namespace eval -------------------------------------------------
# We intercept `namespace` and dispatch on the first arg. Other
# subcommands pass through unchanged.
rename ::namespace ::tcld::dbg::__wrap__::orig_namespace
::tcld::dbg::__wrap__::orig_proc ::namespace {sub args} {
    if {$sub eq "eval" && [llength $args] >= 2} {
        set ns [lindex $args 0]
        set body [join [lrange $args 1 end] \n]
        set owner [info script]
        if {$owner ne ""} { set owner [file normalize $owner] }
        ::tcld::dbg::__on_namespace_eval $ns $body $owner
    }
    uplevel 1 [list ::tcld::dbg::__wrap__::orig_namespace $sub {*}$args]
}

# --- eval -----------------------------------------------------------
rename ::eval ::tcld::dbg::__wrap__::orig_eval
::tcld::dbg::__wrap__::orig_proc ::eval {args} {
    set tracked 0
    if {[llength $args] >= 1} {
        set body [join $args \n]
        set owner [info script]
        if {$owner ne ""} { set owner [file normalize $owner] }
        ::tcld::dbg::__on_anonymous_body $body $owner
        set tracked 1
    }
    set rc [::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__wrap__::orig_uplevel 1 \
            [list ::tcld::dbg::__wrap__::orig_eval {*}$args]} result options]
    if {$tracked} { ::tcld::dbg::__on_anonymous_exit }
    return -options $options $result
}

# --- vwait / update -------------------------------------------------
# Track event-loop-re-entry depth so the trace handler can decline to
# pause inside vwait/update — pausing there would block the event
# loop indefinitely with no way for an external resume command to
# reach us. Counter is read by the handler via __defer_depth.
set ::tcld::dbg::__wrap__::defer_depth 0
rename ::vwait ::tcld::dbg::__wrap__::orig_vwait
::tcld::dbg::__wrap__::orig_proc ::vwait {args} {
    incr ::tcld::dbg::__wrap__::defer_depth
    ::tcld::dbg::__defer_depth_changed $::tcld::dbg::__wrap__::defer_depth
    # Use orig_catch + orig_uplevel here too: avoid the cycle with our
    # own wrappers.
    set rc [::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__wrap__::orig_uplevel 1 \
            [list ::tcld::dbg::__wrap__::orig_vwait {*}$args]} result]
    incr ::tcld::dbg::__wrap__::defer_depth -1
    ::tcld::dbg::__defer_depth_changed $::tcld::dbg::__wrap__::defer_depth
    if {$rc != 0} { return -code $rc $result }
    return $result
}
rename ::update ::tcld::dbg::__wrap__::orig_update
::tcld::dbg::__wrap__::orig_proc ::update {args} {
    incr ::tcld::dbg::__wrap__::defer_depth
    ::tcld::dbg::__defer_depth_changed $::tcld::dbg::__wrap__::defer_depth
    set rc [::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__wrap__::orig_uplevel 1 \
            [list ::tcld::dbg::__wrap__::orig_update {*}$args]} result]
    incr ::tcld::dbg::__wrap__::defer_depth -1
    ::tcld::dbg::__defer_depth_changed $::tcld::dbg::__wrap__::defer_depth
    if {$rc != 0} { return -code $rc $result }
    return $result
}

# --- uplevel --------------------------------------------------------
# Records the (from_level, to_level) transition so inspect_stack can
# annotate frames that are running an uplevel'd body in someone else's
# scope. The wrapper resolves the destination level using Tcl's same
# semantics as the original (default 1, `#N` for absolute, otherwise
# relative).
rename ::uplevel ::tcld::dbg::__wrap__::orig_uplevel
::tcld::dbg::__wrap__::orig_proc ::uplevel {args} {
    # `info level` returns OUR level (the wrapper). The user's caller
    # is one level up. `uplevel 1` from the user's perspective means
    # "the user's caller's caller", which is OUR level minus 2.
    set our_level [info level]
    set user_level [expr {$our_level - 1}]
    # Parse the destination. Mirrors Tcl's parser: first arg may be
    # #N (absolute), a number (relative), or omitted (default 1, with
    # args[0] being the script).
    set first [lindex $args 0]
    set rel 1
    set rest_idx 0
    if {[regexp {^#[0-9]+$} $first]} {
        set rel [expr {$user_level - [string range $first 1 end]}]
        set rest_idx 1
    } elseif {[regexp {^[0-9]+$} $first]} {
        set rel $first
        set rest_idx 1
    }
    set target_level [expr {$user_level - $rel}]
    if {$target_level < 0} { set target_level 0 }
    ::tcld::dbg::__on_uplevel_enter $user_level $target_level
    # From OUR frame, the user's relative level becomes rel+1 because
    # we have to skip our own wrapper frame. Build the call directly
    # against the original built-ins (not the wrapped names) to avoid
    # the wrapper-cycle (uplevel ↔ catch).
    set rest [lrange $args $rest_idx end]
    set adjusted [expr {$rel + 1}]
    set rc [::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__wrap__::orig_uplevel $adjusted {*}$rest} result]
    ::tcld::dbg::__on_uplevel_exit
    if {$rc != 0} { return -code $rc $result }
    return $result
}

# --- interp ---------------------------------------------------------
# Tier 3: track child interps. We don't auto-install the engine on
# slaves (that's the host's call via `slave_install`), but we record
# every `interp create` so the host can enumerate them and decide
# which to attach to. Must come BEFORE the rename wrapper, because the
# rename wrapper refuses any rename whose target is in ::tcld::dbg::*.
rename ::interp ::tcld::dbg::__wrap__::orig_interp
::tcld::dbg::__wrap__::orig_proc ::interp {sub args} {
    set rc [::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__wrap__::orig_uplevel 1 \
            [list ::tcld::dbg::__wrap__::orig_interp $sub {*}$args]} result]
    if {$sub eq "create" && $rc == 0} {
        # The created interp's name is whatever `interp create` returned —
        # `result` here is that name. (`interp create -safe foo` returns
        # `foo`; `interp create` with no name returns an auto-generated id.)
        ::tcld::dbg::__on_interp_create $result
    }
    if {$rc != 0} { return -code $rc $result }
    return $result
}

# --- rename ---------------------------------------------------------
# Refuse to rename anything in ::tcld::dbg::*. Otherwise the user could
# break the wrappers above. Must come LAST among renames, since this
# wrapper refuses subsequent renames into our namespace.
rename ::rename ::tcld::dbg::__wrap__::orig_rename
::tcld::dbg::__wrap__::orig_proc ::rename {src dst} {
    if {[string match "::tcld::dbg::*" $src] || [string match "::tcld::dbg::*" $dst]} {
        error "rename of debugger-internal command refused: $src"
    }
    uplevel 1 [list ::tcld::dbg::__wrap__::orig_rename $src $dst]
}
"#;

/// Install the wrapper Tcl commands. Idempotent — the script's `if
/// installed` guard makes a second call a no-op.
///
/// # Safety
///
/// `interp` must be a valid Tcl interpreter pointer with the step
/// engine already installed (see [`step_engine::install`]).
pub unsafe fn install_command_wrappers(interp: *mut Tcl_Interp) -> Result<(), String> {
    // Helper commands first — the wrappers below call them.
    create_helper_cmd(interp, "::tcld::dbg::__on_proc_define", on_proc_define_cmd)?;
    create_helper_cmd(interp, "::tcld::dbg::__on_source_load", on_source_load_cmd)?;
    create_helper_cmd(interp, "::tcld::dbg::__on_source_exit", on_source_exit_cmd)?;
    create_helper_cmd(
        interp,
        "::tcld::dbg::__on_anonymous_body",
        on_anonymous_body_cmd,
    )?;
    create_helper_cmd(
        interp,
        "::tcld::dbg::__on_anonymous_exit",
        on_anonymous_exit_cmd,
    )?;
    create_helper_cmd(
        interp,
        "::tcld::dbg::__on_namespace_eval",
        on_namespace_eval_cmd,
    )?;
    create_helper_cmd(interp, "::tcld::dbg::__on_catch_error", on_catch_error_cmd)?;
    create_helper_cmd(
        interp,
        "::tcld::dbg::__defer_depth_changed",
        on_defer_depth_changed_cmd,
    )?;
    create_helper_cmd(
        interp,
        "::tcld::dbg::__on_uplevel_enter",
        on_uplevel_enter_cmd,
    )?;
    create_helper_cmd(
        interp,
        "::tcld::dbg::__on_uplevel_exit",
        on_uplevel_exit_cmd,
    )?;
    create_helper_cmd(
        interp,
        "::tcld::dbg::__on_interp_create",
        on_interp_create_cmd,
    )?;
    // Variable-trace helper used by watchpoints. Wrappers and watchpoints
    // are runtime-paired; installing one without the other leaves the
    // engine in an incoherent state.
    unsafe { crate::watchpoints::install_var_trace_helper(interp)? };

    // Then the rename-and-redefine script.
    let script = std::ffi::CString::new(WRAPPER_INSTALL_SCRIPT)
        .map_err(|_| "wrapper script contains NUL byte".to_string())?;
    let rc = unsafe { Tcl_EvalEx(interp, script.as_ptr(), -1, 0) };
    if rc != TCL_OK {
        let err = unsafe {
            let obj = Tcl_GetObjResult(interp);
            obj_to_string(obj)
        };
        return Err(format!("wrapper install failed: {err}"));
    }
    Ok(())
}

fn create_helper_cmd(
    interp: *mut Tcl_Interp,
    name: &str,
    proc_fn: unsafe extern "C" fn(
        *mut c_void,
        *mut Tcl_Interp,
        c_int,
        *const *mut Tcl_Obj,
    ) -> c_int,
) -> Result<(), String> {
    let cname = std::ffi::CString::new(name).map_err(|_| "bad cstring".to_string())?;
    unsafe {
        tcl_sys::Tcl_CreateObjCommand(
            interp,
            cname.as_ptr(),
            Some(proc_fn),
            std::ptr::null_mut(),
            None,
        );
    }
    Ok(())
}

unsafe fn obj_to_string(obj: *mut Tcl_Obj) -> String {
    if obj.is_null() {
        return String::new();
    }
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len <= 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

unsafe fn objv_string(objv: *const *mut Tcl_Obj, index: usize) -> Option<String> {
    let obj = unsafe { *objv.add(index) };
    if obj.is_null() {
        None
    } else {
        Some(unsafe { obj_to_string(obj) })
    }
}

fn lookup_state(interp: *mut Tcl_Interp) -> Option<Arc<std::sync::Mutex<DebugState>>> {
    let ctx_ptr = step_engine::lookup_ctx(interp)?;
    Some(unsafe { (*ctx_ptr).state.clone() })
}

fn ok_result(interp: *mut Tcl_Interp) -> c_int {
    unsafe {
        Tcl_SetObjResult(interp, tcl_sys::Tcl_NewStringObj(c"".as_ptr(), 0));
    }
    TCL_OK
}

fn err_result(interp: *mut Tcl_Interp, msg: &str) -> c_int {
    let bytes = msg.as_bytes();
    unsafe {
        Tcl_SetObjResult(
            interp,
            tcl_sys::Tcl_NewStringObj(
                bytes.as_ptr() as *const std::os::raw::c_char,
                bytes.len() as tcl_sys::Tcl_Size,
            ),
        );
    }
    TCL_ERROR
}

unsafe extern "C" fn on_proc_define_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 4 {
        return err_result(
            interp,
            "wrong # args: __on_proc_define qname body source_path",
        );
    }
    let qname = match unsafe { objv_string(objv, 1) } {
        Some(s) => s,
        None => return err_result(interp, "qname missing"),
    };
    let body = unsafe { objv_string(objv, 2) }.unwrap_or_default();
    let source_path = unsafe { objv_string(objv, 3) }.filter(|path| !path.is_empty());
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    if let Ok(mut guard) = state.lock() {
        // Strip leading `::` so BlockId matches what `info frame`
        // reports for proc names (info frame returns "::name", but the
        // rest of our wiring uses unprefixed qnames).
        let bare = qname.trim_start_matches("::");
        let id = BlockId::ProcBody(Arc::from(bare));
        let existed = guard.blocks.get(&id).is_some();
        let registered = register_body_block(&mut guard.blocks, Some(bare), &body);
        if let Ok(id) = registered {
            let block = guard.blocks.get(&id).cloned();
            if let Some(block) = block {
                let generation = block.version();
                let _ = guard.with_breakpoints_mut(|store, _| store.validate_against(&block));
                drop(guard);
                tcld_debug_state::fire_event_unlocked(
                    &state,
                    DebugEvent::BlocksRevalidated {
                        block: id.clone(),
                        generation,
                        source_path: source_path.map(Arc::from),
                        change: if existed {
                            BlockChange::Revalidated
                        } else {
                            BlockChange::Registered
                        },
                    },
                );
            }
        }
    }
    ok_result(interp)
}

unsafe extern "C" fn on_source_load_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 3 {
        return err_result(interp, "wrong # args: __on_source_load path body");
    }
    let path = match unsafe { objv_string(objv, 1) } {
        Some(s) => s,
        None => return err_result(interp, "path missing"),
    };
    let body = unsafe { objv_string(objv, 2) }.unwrap_or_default();
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    let event = if let Ok(mut guard) = state.lock() {
        let parse_input = if body.ends_with('\n') {
            body.clone()
        } else {
            format!("{body}\n")
        };
        let spans = tcld_syntax::command_spans(&parse_input).unwrap_or_default();
        let mut offsets: Vec<usize> = spans.iter().map(|s| s.start_byte).collect();
        offsets.sort_unstable();
        offsets.dedup();
        let id = BlockId::File(Arc::from(path.as_str()));
        let existed = guard.blocks.get(&id).is_some();
        let source_unchanged = guard
            .blocks
            .get(&id)
            .is_some_and(|block| block.source() == body);
        if !source_unchanged {
            let _ =
                guard
                    .blocks
                    .register_replacement(id.clone(), Arc::from(body.as_str()), offsets);
        }
        guard.blocks.get(&id).map(|block| {
            (
                id,
                block.version(),
                if existed {
                    BlockChange::Revalidated
                } else {
                    BlockChange::Registered
                },
            )
        })
    } else {
        None
    };
    if let Some((id, generation, change)) = event {
        if let Some(ctx_ptr) = step_engine::lookup_ctx(interp) {
            unsafe {
                (*ctx_ptr).active_source_files.push(id.clone());
            }
        }
        tcld_debug_state::fire_event_unlocked(
            &state,
            DebugEvent::BlocksRevalidated {
                block: id,
                generation,
                source_path: Some(Arc::from(path)),
                change,
            },
        );
    }
    ok_result(interp)
}

unsafe extern "C" fn on_source_exit_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return err_result(interp, "wrong # args: __on_source_exit");
    }
    if let Some(ctx_ptr) = step_engine::lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr).active_source_files.pop();
        }
    }
    ok_result(interp)
}

unsafe extern "C" fn on_anonymous_body_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 3 {
        return err_result(interp, "wrong # args: __on_anonymous_body body source_path");
    }
    let body = unsafe { objv_string(objv, 1) }.unwrap_or_default();
    let source_path = unsafe { objv_string(objv, 2) }.filter(|path| !path.is_empty());
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    let event = if let Ok(mut guard) = state.lock() {
        let id = BlockId::anonymous_for(&body);
        let existed = guard.blocks.get(&id).is_some();
        let _ = register_body_block(&mut guard.blocks, None, &body);
        guard.blocks.get(&id).map(|block| {
            (
                id,
                block.version(),
                if existed {
                    BlockChange::Revalidated
                } else {
                    BlockChange::Registered
                },
            )
        })
    } else {
        None
    };
    if let Some((block, generation, change)) = event {
        if let Some(ctx_ptr) = step_engine::lookup_ctx(interp) {
            unsafe {
                (*ctx_ptr).active_dynamic_blocks.push(block.clone());
            }
        }
        tcld_debug_state::fire_event_unlocked(
            &state,
            DebugEvent::BlocksRevalidated {
                block,
                generation,
                source_path: source_path.map(Arc::from),
                change,
            },
        );
    }
    ok_result(interp)
}

unsafe extern "C" fn on_anonymous_exit_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return err_result(interp, "wrong # args: __on_anonymous_exit");
    }
    if let Some(ctx_ptr) = step_engine::lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr).active_dynamic_blocks.pop();
        }
    }
    ok_result(interp)
}

unsafe extern "C" fn on_namespace_eval_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 4 {
        return err_result(
            interp,
            "wrong # args: __on_namespace_eval ns body source_path",
        );
    }
    let _ns = unsafe { objv_string(objv, 1) }.unwrap_or_default();
    let body = unsafe { objv_string(objv, 2) }.unwrap_or_default();
    let source_path = unsafe { objv_string(objv, 3) }.filter(|path| !path.is_empty());
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    let event = if let Ok(mut guard) = state.lock() {
        // Anonymous body — namespace eval bodies don't have a stable
        // qname. The host can attach them to the namespace via a
        // separate API once we expose it.
        let id = BlockId::anonymous_for(&body);
        let existed = guard.blocks.get(&id).is_some();
        let _ = register_body_block(&mut guard.blocks, None, &body);
        guard.blocks.get(&id).map(|block| {
            (
                id,
                block.version(),
                if existed {
                    BlockChange::Revalidated
                } else {
                    BlockChange::Registered
                },
            )
        })
    } else {
        None
    };
    if let Some((block, generation, change)) = event {
        tcld_debug_state::fire_event_unlocked(
            &state,
            DebugEvent::BlocksRevalidated {
                block,
                generation,
                source_path: source_path.map(Arc::from),
                change,
            },
        );
    }
    ok_result(interp)
}

unsafe extern "C" fn on_uplevel_enter_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 3 {
        return err_result(interp, "wrong # args: __on_uplevel_enter from to");
    }
    let from: u32 = unsafe { objv_string(objv, 1) }
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let to: u32 = unsafe { objv_string(objv, 2) }
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    if let Ok(mut guard) = state.lock() {
        guard.uplevel_hops.push(tcld_debug_state::UplevelHop {
            from_level: from,
            to_level: to,
        });
    }
    ok_result(interp)
}

unsafe extern "C" fn on_uplevel_exit_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    if let Ok(mut guard) = state.lock() {
        guard.uplevel_hops.pop();
    }
    ok_result(interp)
}

unsafe extern "C" fn on_interp_create_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return err_result(interp, "wrong # args: __on_interp_create slave_name");
    }
    let name = unsafe { objv_string(objv, 1) }.unwrap_or_default();
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    if let Ok(mut guard) = state.lock() {
        guard.slaves.insert(name);
    }
    ok_result(interp)
}

unsafe extern "C" fn on_defer_depth_changed_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return err_result(interp, "wrong # args: __defer_depth_changed depth");
    }
    let depth: u32 = unsafe { objv_string(objv, 1) }
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    if let Ok(mut guard) = state.lock() {
        guard.defer_depth = depth;
    }
    ok_result(interp)
}

unsafe extern "C" fn on_catch_error_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Args: rc message depth_after_pop
    if objc != 4 {
        return err_result(
            interp,
            "wrong # args: __on_catch_error rc message depth_after_pop",
        );
    }
    let _rc = unsafe { objv_string(objv, 1) }.unwrap_or_default();
    let message = unsafe { objv_string(objv, 2) }.unwrap_or_default();
    let depth_after_pop: i64 = unsafe { objv_string(objv, 3) }
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // depth_after_pop == 0 means the catch we just popped was the
    // outermost. That's the "uncaught" case — the error would have
    // escaped the program. >= 1 means there's an outer catch that will
    // also see this error.
    let was_outermost_catch = depth_after_pop == 0;

    let Some(state) = lookup_state(interp) else {
        return ok_result(interp);
    };
    // ignore_error: one-shot suppression. If set, consume it and skip
    // the pause for this error; subsequent errors are unaffected.
    {
        if let Ok(mut guard) = state.lock()
            && guard.error_ignored_pending
        {
            guard.error_ignored_pending = false;
            return ok_result(interp);
        }
    }
    // Find the first enabled exception breakpoint whose semantics match.
    let bp_id = {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return ok_result(interp),
        };
        guard.breakpoints().list().find_map(|bp| {
            if !bp.enabled {
                return None;
            }
            match bp.kind {
                tcld_debug_state::BreakpointKind::Exception { uncaught_only } => {
                    if uncaught_only && !was_outermost_catch {
                        // Inner catch will see it; user wants only
                        // truly uncaught errors → skip.
                        None
                    } else {
                        Some(bp.id)
                    }
                }
                _ => None,
            }
        })
    };

    if let Ok(mut guard) = state.lock()
        && let Some(id) = bp_id
    {
        guard.breakpoint_hits = guard.breakpoint_hits.saturating_add(1);
        guard.with_breakpoints_mut(|store, _| store.record_hit(id));
    }
    tcld_debug_state::fire_event_unlocked(
        &state,
        DebugEvent::ErrorPaused {
            location: None,
            message,
            error_code: None,
            breakpoint: bp_id,
        },
    );
    ok_result(interp)
}
