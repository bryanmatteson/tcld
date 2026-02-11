//! Nub-side wire bridge.
//!
//! Tier 4 Phase 4-B. Adds three Tcl commands:
//!
//! - `::tcld::dbg::nub_connect <host> <port>` — opens a TCP socket
//!   to the controller, sends `hello`, waits for `hello_ack`, then
//!   installs a `fileevent` reader that dispatches incoming requests
//!   and an event subscriber that forwards `DebugEvent`s as wire
//!   `evt` frames.
//! - `::tcld::dbg::nub_status` — `disconnected | connecting | open | closing`
//! - `::tcld::dbg::nub_disconnect` — sends `bye` and tears down.
//!
//! The actual socket I/O is pure Tcl (uses `socket` / `fileevent` /
//! `gets` / `read` / `puts`) so it integrates with the user script's
//! `vwait` for free. Rust's only role here is:
//!   1. Installing the Tcl-side helper procs
//!   2. Registering an event subscriber that, on each `DebugEvent`,
//!      formats the wire payload and evals a Tcl proc to write it
//!   3. Providing a Tcl-side dispatch table that maps incoming op
//!      names to existing `::tcld::dbg::*` commands

use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::sync::Arc;

use tcl_sys::{TCL_OK, Tcl_EvalEx, Tcl_Interp, Tcl_Obj};
use tcld_debug_state::{DebugEvent, DebugState, SubscriptionId};
use tcld_debug_wire::{PROTOCOL_VERSION, WireEvent, handshake::Hello};

use crate::step_engine;

/// Tcl helper script. Defines `::tcld::dbg::__nub__::*` procs that
/// own the socket and fileevent. Idempotent — re-evaluating the
/// script is a no-op via the `installed` guard.
pub(crate) const NUB_INSTALL_SCRIPT: &str = r#"
namespace eval ::tcld::dbg::__nub__ {}
if {[info exists ::tcld::dbg::__nub__::installed]} { return }
set ::tcld::dbg::__nub__::installed 1
set ::tcld::dbg::__nub__::state disconnected

# Send a single framed wire message: <decimal-byte-length>\n<body>\n
# where body is `<kind> <serial> {<payload>}` rendered as a Tcl list.
proc ::tcld::dbg::__nub__::send_frame {kind serial payload} {
    if {$::tcld::dbg::__nub__::state ne "open" && $kind ne "hello"} { return }
    set body [list $kind $serial $payload]
    set len [string length $body]
    set sock $::tcld::dbg::__nub__::sock
    if {[::tcld::dbg::__wrap__::orig_catch {
        puts -nonewline $sock "$len\n$body\n"
        flush $sock
    } err]} {
        ::tcld::dbg::__nub__::handle_disconnect $err
    }
}

# Read one frame, blocking. Returns {kind serial payload} or raises on EOF.
proc ::tcld::dbg::__nub__::read_frame {} {
    set sock $::tcld::dbg::__nub__::sock
    set len_line [gets $sock]
    if {[eof $sock]} { error "EOF" }
    if {![string is integer -strict $len_line]} {
        error "bad length prefix: $len_line"
    }
    set body [read $sock $len_line]
    if {[string length $body] != $len_line} {
        error "short read: expected $len_line got [string length $body]"
    }
    set trail [read $sock 1]
    if {$trail ne "\n"} { error "missing trailing newline" }
    return $body
}

proc ::tcld::dbg::__nub__::handle_disconnect {reason} {
    set ::tcld::dbg::__nub__::state disconnected
    if {[info exists ::tcld::dbg::__nub__::sock]} {
        ::tcld::dbg::__wrap__::orig_catch {close $::tcld::dbg::__nub__::sock}
        unset ::tcld::dbg::__nub__::sock
    }
}

# Connect, handshake, install fileevent reader. Blocking.
proc ::tcld::dbg::__nub__::connect {host port hello_payload} {
    if {$::tcld::dbg::__nub__::state ne "disconnected"} {
        error "nub already $::tcld::dbg::__nub__::state"
    }
    set ::tcld::dbg::__nub__::state connecting
    set sock [socket $host $port]
    fconfigure $sock -translation binary -buffering none -blocking 1
    set ::tcld::dbg::__nub__::sock $sock
    set ::tcld::dbg::__nub__::state handshaking

    # Send hello; await hello_ack.
    ::tcld::dbg::__nub__::send_frame hello 0 $hello_payload
    set ack_body [::tcld::dbg::__nub__::read_frame]
    lassign $ack_body kind serial payload
    if {$kind ne "hello_ack"} {
        ::tcld::dbg::__nub__::handle_disconnect "expected hello_ack, got $kind"
        error "handshake failed: expected hello_ack, got $kind"
    }
    array set controller_dict $payload
    set controller_protocol [expr {
        [info exists controller_dict(protocol_version)] \
            ? $controller_dict(protocol_version) : 0
    }]
    if {$controller_protocol != 2} {
        ::tcld::dbg::__nub__::handle_disconnect \
            "protocol version mismatch: expected 2, got $controller_protocol"
        error "handshake protocol mismatch: expected v2, got v$controller_protocol"
    }
    set ::tcld::dbg::__nub__::controller_payload $payload
    set ::tcld::dbg::__nub__::state open
    fileevent $sock readable [list ::tcld::dbg::__nub__::on_readable]
    return ok
}

proc ::tcld::dbg::__nub__::on_readable {} {
    if {$::tcld::dbg::__nub__::state ne "open"} return
    if {[::tcld::dbg::__wrap__::orig_catch {::tcld::dbg::__nub__::read_frame} body]} {
        ::tcld::dbg::__nub__::handle_disconnect $body
        return
    }
    lassign $body kind serial payload
    switch -- $kind {
        req {
            ::tcld::dbg::__nub__::dispatch_req $serial $payload
        }
        bye {
            ::tcld::dbg::__nub__::handle_disconnect bye
        }
        default {
            ::tcld::dbg::__nub__::send_err $serial protocol_violation \
                "unexpected frame kind: $kind"
        }
    }
}

# Map a wire op name + args to a ::tcld::dbg::* command. Returns the
# response payload (may be empty).
proc ::tcld::dbg::__nub__::dispatch_req {serial payload} {
    set parts $payload
    set op [lindex $parts 0]
    set args [lrange $parts 1 end]
    if {[::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__nub__::call_op $op $args} result opts]} {
        # Map a Tcl error to a wire err frame.
        set code internal
        if {[string match {*not paused*} $result] || \
            [string match {*not Stopped*} $result]} {
            set code not_stopped
        } elseif {[string match {*not attached*} $result]} {
            set code not_attached
        } elseif {[string match {*not found*} $result]} {
            set code not_found
        } elseif {[string match {*wrong # args*} $result]} {
            set code bad_args
        }
        ::tcld::dbg::__nub__::send_err $serial $code $result
        return
    }
    ::tcld::dbg::__nub__::send_frame resp $serial $result
}

proc ::tcld::dbg::__nub__::send_err {serial code message} {
    set payload [list code $code message $message]
    ::tcld::dbg::__nub__::send_frame err $serial $payload
}

set ::tcld::dbg::__nub__::target_state ready
set ::tcld::dbg::__nub__::target_epoch 0
set ::tcld::dbg::__nub__::target_request {}
set ::tcld::dbg::__nub__::target_wakeup {}
set ::tcld::dbg::__nub__::pending_next 0
array set ::tcld::dbg::__nub__::pending_breakpoints {}

proc ::tcld::dbg::__nub__::send_event {kind fields} {
    ::tcld::dbg::__nub__::send_frame evt 0 [linsert $fields 0 $kind]
}

proc ::tcld::dbg::__nub__::bp_add {args_list} {
    if {[lindex $args_list 0] ne "file_line"} {
        return [::tcld::dbg::breakpoint_add {*}$args_list]
    }
    if {[llength $args_list] ni {3 4}} {
        error "wrong # args: file_line path line ?condition?"
    }
    set path [lindex $args_list 1]
    set line [lindex $args_list 2]
    if {![string is integer -strict $line] || $line < 1} {
        error "file line must be a positive integer"
    }
    set condition [lindex $args_list 3]
    set id pending[incr ::tcld::dbg::__nub__::pending_next]
    set ::tcld::dbg::__nub__::pending_breakpoints($id) [dict create \
        path $path line $line condition $condition enabled 1 ignore 0]
    if {[info exists ::tcld::dbg::__nub__::active_target] &&
        $::tcld::dbg::__nub__::active_target eq $path} {
        ::tcld::dbg::__nub__::resolve_pending_breakpoints $path
    }
    return $id
}

proc ::tcld::dbg::__nub__::bp_list {} {
    set rows [::tcld::dbg::breakpoint_list]
    foreach id [lsort [array names ::tcld::dbg::__nub__::pending_breakpoints]] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        set row [list id $id block "file:[dict get $record path]" \
            offset 0 len 1 line [dict get $record line] \
            enabled [dict get $record enabled] hits 0 \
            ignore [dict get $record ignore] pending 1]
        if {[dict get $record condition] ne ""} {
            lappend row condition [dict get $record condition]
        }
        lappend rows $row
    }
    return $rows
}

proc ::tcld::dbg::__nub__::bp_remove {id} {
    if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
        unset ::tcld::dbg::__nub__::pending_breakpoints($id)
        return ok
    }
    return [::tcld::dbg::bp_remove $id]
}

proc ::tcld::dbg::__nub__::bp_enable {id enabled} {
    if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
        dict set ::tcld::dbg::__nub__::pending_breakpoints($id) enabled $enabled
        return ok
    }
    if {$enabled} {
        return [::tcld::dbg::bp_enable $id]
    }
    return [::tcld::dbg::bp_disable $id]
}

proc ::tcld::dbg::__nub__::bp_set_ignore {id count} {
    if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
        dict set ::tcld::dbg::__nub__::pending_breakpoints($id) ignore $count
        return ok
    }
    return [::tcld::dbg::bp_set_ignore $id $count]
}

proc ::tcld::dbg::__nub__::bp_export {} {
    set rows [::tcld::dbg::bp_export]
    foreach id [lsort [array names ::tcld::dbg::__nub__::pending_breakpoints]] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        lappend rows [list kind file_line \
            path [dict get $record path] line [dict get $record line] \
            enabled [dict get $record enabled] ignore [dict get $record ignore] \
            condition [dict get $record condition]]
    }
    return $rows
}

proc ::tcld::dbg::__nub__::bp_import {serialized} {
    array unset ::tcld::dbg::__nub__::pending_breakpoints
    array set ::tcld::dbg::__nub__::pending_breakpoints {}
    set core_rows {}
    set imported {}
    foreach row $serialized {
        if {[dict get $row kind] ne "file_line"} {
            lappend core_rows $row
            continue
        }
        set id pending[incr ::tcld::dbg::__nub__::pending_next]
        set ::tcld::dbg::__nub__::pending_breakpoints($id) [dict create \
            path [dict get $row path] line [dict get $row line] \
            condition [dict get $row condition] enabled [dict get $row enabled] \
            ignore [dict get $row ignore]]
        lappend imported $id
    }
    set core_ids [::tcld::dbg::bp_import $core_rows]
    return [concat $core_ids $imported]
}

proc ::tcld::dbg::__nub__::resolve_pending_breakpoints {path} {
    set resolved {}
    foreach id [array names ::tcld::dbg::__nub__::pending_breakpoints] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        if {[dict get $record path] ne $path} continue
        set line [dict get $record line]
        if {[::tcld::dbg::__wrap__::orig_catch \
            {::tcld::dbg::file_line_location $path $line} location]} {
            continue
        }
        set offset [dict get $location offset]
        set len [dict get $location len]
        set args [list file $path $offset $len]
        if {[dict get $record condition] ne ""} {
            lappend args [dict get $record condition]
        }
        set real_id [::tcld::dbg::breakpoint_add {*}$args]
        if {![dict get $record enabled]} {
            ::tcld::dbg::bp_disable $real_id
        }
        if {[dict get $record ignore] > 0} {
            ::tcld::dbg::bp_set_ignore $real_id [dict get $record ignore]
        }
        unset ::tcld::dbg::__nub__::pending_breakpoints($id)
        lappend resolved [list pending $id resolved $real_id line $line offset $offset]
    }
    ::tcld::dbg::__nub__::send_event blocks_revalidated [list block "file:$path"]
    return $resolved
}

# Acknowledge scheduling before target execution. The zero-delay wakeup runs
# only after this socket callback has returned, so it cannot race between the
# bootstrap's queue check and its vwait.
proc ::tcld::dbg::__nub__::start_target {path stop_on_entry} {
    if {$::tcld::dbg::__nub__::target_state in {loading running paused}} {
        error "target already $::tcld::dbg::__nub__::target_state"
    }
    if {$stop_on_entry ni {0 1 false true off on no yes}} {
        error "stop_on_entry must be boolean"
    }
    set ::tcld::dbg::__nub__::target_state loading
    set ::tcld::dbg::__nub__::target_request \
        [list $path [expr {$stop_on_entry in {1 true on yes}}]]
    set ::tcld::dbg::__nub__::target_wakeup [after 0 {
        incr ::tcld::dbg::__nub__::target_epoch
    }]
    return [list status scheduled path $path stop_on_entry $stop_on_entry]
}

proc ::tcld::dbg::__nub__::run_target {path stop_on_entry} {
    set ::tcld::dbg::__nub__::active_target $path
    if {![file readable $path]} {
        set ::tcld::dbg::__nub__::target_state load_failed
        ::tcld::dbg::__nub__::send_event target_load_failed [list \
            path $path status load_failed message "target is not readable" \
            error_code {TCLD TARGET NOT_READABLE} error_info "target is not readable: $path"]
        return
    }

    # Register the complete file block before execution. This gives staged
    # file/line breakpoints and stop-on-entry an exact command boundary.
    set fd [open $path r]
    set body [read $fd]
    close $fd
    ::tcld::dbg::pre_instrument_file $path $body
    ::tcld::dbg::__nub__::resolve_pending_breakpoints $path
    ::tcld::dbg::activate_target_file $path 0

    set ::tcld::dbg::__nub__::target_state running
    ::tcld::dbg::__nub__::send_event target_started [list path $path status running]
    if {$stop_on_entry} {
        ::tcld::dbg::pause_at_target_entry $path
    }

    set rc [::tcld::dbg::__wrap__::orig_catch \
        [list uplevel #0 [list source $path]] result options]
    if {$rc == 0} {
        ::tcld::dbg::clear_active_target_file
        set ::tcld::dbg::__nub__::target_state completed
        ::tcld::dbg::__nub__::send_event target_completed [list \
            path $path status completed result $result]
    } else {
        ::tcld::dbg::clear_active_target_file
        set error_code {}
        set error_info $result
        if {[dict exists $options -errorcode]} {
            set error_code [dict get $options -errorcode]
        }
        if {[dict exists $options -errorinfo]} {
            set error_info [dict get $options -errorinfo]
        }
        set ::tcld::dbg::__nub__::target_state load_failed
        ::tcld::dbg::__nub__::send_event target_load_failed [list \
            path $path status load_failed message $result \
            error_code $error_code error_info $error_info]
    }
}

# Dispatch table. Each clause invokes the matching ::tcld::dbg::* cmd
# with the wire's positional args. `args` is a Tcl list at this point.
proc ::tcld::dbg::__nub__::call_op {op args_list} {
    switch -- $op {
        start_target     { return [::tcld::dbg::__nub__::start_target {*}$args_list] }
        step_into        { return [::tcld::dbg::step_into] }
        step_over        { return [::tcld::dbg::step_over] }
        step_out         { return [::tcld::dbg::step_out] }
        step_instruction { return [::tcld::dbg::step_instruction] }
        step_over_instruction { return [::tcld::dbg::step_over_instruction] }
        step_out_instruction { return [::tcld::dbg::step_out_instruction] }
        continue         { return [::tcld::dbg::continue] }
        run_to_pc        { return [::tcld::dbg::run_to_pc {*}$args_list] }
        interrupt        { return [::tcld::dbg::request_pause] }
        current_pc       { return [::tcld::dbg::current_pc] }
        app_state        { return [::tcld::dbg::app_state] }
        inspect_stack    { return [::tcld::dbg::inspect_stack] }
        eval_in_frame    { return [::tcld::dbg::eval_in_frame {*}$args_list] }
        get_level        { return [::tcld::dbg::get_level] }
        get_result       { return [::tcld::dbg::get_result {*}$args_list] }
        get_variables    { return [::tcld::dbg::get_variables {*}$args_list] }
        get_var          { return [::tcld::dbg::get_var {*}$args_list] }
        set_var          { return [::tcld::dbg::set_var {*}$args_list] }
        bp_add           { return [::tcld::dbg::__nub__::bp_add $args_list] }
        bp_remove        { return [::tcld::dbg::__nub__::bp_remove {*}$args_list] }
        bp_list          { return [::tcld::dbg::__nub__::bp_list] }
        bp_disable       { return [::tcld::dbg::__nub__::bp_enable [lindex $args_list 0] 0] }
        bp_enable        { return [::tcld::dbg::__nub__::bp_enable [lindex $args_list 0] 1] }
        bp_move          { return [::tcld::dbg::bp_move {*}$args_list] }
        bp_validate      { return [::tcld::dbg::bp_validate {*}$args_list] }
        bp_set_ignore    { return [::tcld::dbg::__nub__::bp_set_ignore {*}$args_list] }
        bp_export        { return [::tcld::dbg::__nub__::bp_export] }
        bp_import        { return [::tcld::dbg::__nub__::bp_import {*}$args_list] }
        wp_add           { return [::tcld::dbg::watchpoint_add {*}$args_list] }
        var_breakpoints  { return [::tcld::dbg::var_breakpoints {*}$args_list] }
        line_breakpoints { return [::tcld::dbg::line_breakpoints {*}$args_list] }
        instruction_breakpoints { return [::tcld::dbg::instruction_breakpoints {*}$args_list] }
        proc_list        { return [::tcld::dbg::list_procs] }
        proc_body        { return [::tcld::dbg::proc_body {*}$args_list] }
        proc_location    { return [::tcld::dbg::proc_location {*}$args_list] }
        block_disable    { return [::tcld::dbg::block_disable {*}$args_list] }
        block_enable     { return [::tcld::dbg::block_enable {*}$args_list] }
        coverage         { return [::tcld::dbg::coverage] }
        coverage_enable  { return [::tcld::dbg::coverage_enable {*}$args_list] }
        watch_add        { return [::tcld::dbg::watch_add {*}$args_list] }
        watch_remove     { return [::tcld::dbg::watch_remove {*}$args_list] }
        watch_list       { return [::tcld::dbg::watch_list] }
        watch_evaluate   { return [::tcld::dbg::watch_evaluate {*}$args_list] }
        pre_instrument_proc { return [::tcld::dbg::pre_instrument_proc {*}$args_list] }
        pre_instrument_file { return [::tcld::dbg::pre_instrument_file {*}$args_list] }
        slave_list       { return [::tcld::dbg::slave_list] }
        slave_install    { return [::tcld::dbg::slave_install {*}$args_list] }
        ignore_error     { return [::tcld::dbg::ignore_error] }
        quit             { return [::tcld::dbg::quit] }
        default          { error "unimplemented op: $op" }
    }
}

proc ::tcld::dbg::__nub__::disconnect {} {
    if {$::tcld::dbg::__nub__::state ne "open"} return
    set ::tcld::dbg::__nub__::state closing
    ::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::__nub__::send_frame bye 0 ""}
    ::tcld::dbg::__nub__::handle_disconnect "explicit disconnect"
}

# Synchronous pause loop. Called from the C trace handler instead of
# vwait when the wire is attached. Reads incoming requests synchronously
# (no fileevent) so the user's paused frame STAYS ON THE STACK while
# wire ops dispatch — that's the only way `uplevel #N`-based access to
# the user's locals works. Mirrors TclPro's `DbgNub_Wait`/`DbgNub_HandleEvent`.
#
# Exits when a step_into / step_over / step_out / step_instruction /
# step_over_instruction / step_out_instruction / continue / run_to /
# run_to_pc op
# arrives (those both dispatch and signal resume).
proc ::tcld::dbg::__nub__::pause_loop {} {
    if {$::tcld::dbg::__nub__::state ne "open"} return
    set sock $::tcld::dbg::__nub__::sock
    # Switch the socket to blocking for the synchronous reads, but
    # disable fileevent reader so the event-loop side doesn't race
    # with us.
    fileevent $sock readable {}
    fconfigure $sock -blocking 1
    set ::tcld::dbg::__nub__::resume_signal 0
    while {!$::tcld::dbg::__nub__::resume_signal} {
        if {$::tcld::dbg::__nub__::state ne "open"} { break }
        if {[::tcld::dbg::__wrap__::orig_catch \
            {::tcld::dbg::__nub__::read_frame} body]} {
            ::tcld::dbg::__nub__::handle_disconnect $body
            break
        }
        lassign $body kind serial payload
        switch -- $kind {
            req {
                set op [lindex $payload 0]
                set args_list [lrange $payload 1 end]
                # Step / continue ops resume execution. We dispatch
                # them so the engine state updates, send the response,
                # then signal exit.
                if {$op in {step_into step_over step_out step_instruction step_over_instruction step_out_instruction continue run_to run_to_pc}} {
                    if {[::tcld::dbg::__wrap__::orig_catch \
                        {::tcld::dbg::__nub__::call_op $op $args_list} result]} {
                        ::tcld::dbg::__nub__::send_err $serial internal $result
                    } else {
                        ::tcld::dbg::__nub__::send_frame resp $serial $result
                    }
                    set ::tcld::dbg::__nub__::resume_signal 1
                } else {
                    ::tcld::dbg::__nub__::dispatch_req $serial $payload
                }
            }
            bye {
                ::tcld::dbg::__nub__::handle_disconnect bye
                break
            }
            default {
                ::tcld::dbg::__nub__::send_err $serial protocol_violation \
                    "unexpected kind during pause: $kind"
            }
        }
    }
    # Restore fileevent for the running mode.
    if {$::tcld::dbg::__nub__::state eq "open"} {
        fconfigure $sock -blocking 0
        fileevent $sock readable [list ::tcld::dbg::__nub__::on_readable]
    }
}
"#;

pub(crate) unsafe fn install_nub_helpers(interp: *mut Tcl_Interp) -> Result<(), String> {
    let cstr =
        CString::new(NUB_INSTALL_SCRIPT).map_err(|_| "nub install script has NUL".to_string())?;
    let rc = unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
    if rc != TCL_OK {
        return Err("nub install failed".to_string());
    }
    Ok(())
}

/// Format a [`DebugEvent`] as a typed wire event. Proc-body locations retain
/// the active target path so controller renderers do not have to infer source
/// provenance from the block display id.
fn debug_event_to_wire(event: &DebugEvent, active_target_path: Option<&str>) -> Option<WireEvent> {
    use WireEvent as W;
    match event {
        DebugEvent::Paused {
            location,
            frame_count,
            breakpoint,
            reason,
            pc,
        } => Some(W::Paused {
            location: source_location(location, active_target_path),
            frame_count: *frame_count,
            breakpoint: breakpoint.map(|b| format!("bp{}", b.0)),
            reason: format_pause_reason(*reason),
            pc: *pc,
        }),
        DebugEvent::Resumed => Some(W::Resumed),
        DebugEvent::ErrorPaused {
            location,
            message,
            error_code,
            breakpoint,
        } => Some(W::ErrorPaused {
            location: location
                .as_ref()
                .map(|location| source_location(location, active_target_path)),
            message: message.clone(),
            error_code: error_code.clone(),
            breakpoint: breakpoint.map(|b| format!("bp{}", b.0)),
        }),
        DebugEvent::ConditionError { breakpoint, error } => Some(W::ConditionError {
            breakpoint: format!("bp{}", breakpoint.0),
            message: error.message.clone(),
        }),
        DebugEvent::BlocksRevalidated { block } => Some(W::BlocksRevalidated {
            block: block.as_string(),
        }),
    }
}

fn format_pause_reason(reason: tcld_debug_state::PauseReason) -> tcld_debug_wire::PauseReason {
    match reason {
        tcld_debug_state::PauseReason::Entry => tcld_debug_wire::PauseReason::Entry,
        tcld_debug_state::PauseReason::Step => tcld_debug_wire::PauseReason::Step,
        tcld_debug_state::PauseReason::Breakpoint => tcld_debug_wire::PauseReason::Breakpoint,
        tcld_debug_state::PauseReason::Watchpoint => tcld_debug_wire::PauseReason::Watchpoint,
        tcld_debug_state::PauseReason::Pause => tcld_debug_wire::PauseReason::Pause,
        tcld_debug_state::PauseReason::InstructionStep => {
            tcld_debug_wire::PauseReason::InstructionStep
        }
        tcld_debug_state::PauseReason::InstructionBreakpoint => {
            tcld_debug_wire::PauseReason::InstructionBreakpoint
        }
    }
}

fn source_location(
    loc: &tcld_debug_state::Location,
    active_target_path: Option<&str>,
) -> tcld_debug_wire::SourceLocation {
    use tcld_debug_state::BlockId;
    use tcld_debug_wire::SourceBlockKind;

    let (block_kind, source_path, qualified_proc) = match loc.block() {
        BlockId::File(path) => (SourceBlockKind::File, Some(path.to_string()), None),
        BlockId::ProcBody(qname) => (
            SourceBlockKind::Proc,
            active_target_path.map(str::to_string),
            Some(qname.to_string()),
        ),
        BlockId::Anonymous { .. } => (SourceBlockKind::Anonymous, None, None),
    };
    tcld_debug_wire::SourceLocation {
        block_id: loc.block().as_string(),
        block_kind,
        source_path,
        qualified_proc,
        line: u32::try_from(loc.line()).unwrap_or(u32::MAX),
        offset_start: u64::try_from(loc.byte_offset()).unwrap_or(u64::MAX),
        offset_end: u64::try_from(loc.byte_offset().saturating_add(loc.byte_len()))
            .unwrap_or(u64::MAX),
    }
}

/// Subscribe a [`DebugState`] subscriber that, for each event,
/// formats a wire payload and evals
/// `::tcld::dbg::__nub__::send_frame evt 0 <payload>`. Returns the
/// `SubscriptionId` so disconnect can unsubscribe.
fn subscribe_event_forwarder(
    state: &Arc<std::sync::Mutex<DebugState>>,
    interp: *mut Tcl_Interp,
) -> Option<SubscriptionId> {
    let interp_addr = interp as usize;
    let cb: tcld_debug_state::EventCallback = Arc::new(move |event| {
        let interp = interp_addr as *mut Tcl_Interp;
        let active_target_path = step_engine::active_target_path(interp);
        let Some(wire_event) = debug_event_to_wire(&event, active_target_path.as_deref()) else {
            return;
        };
        let payload = wire_event.to_payload();
        // Compose the Tcl invocation. Brace-quote payload so embedded
        // whitespace doesn't confuse the parser.
        let script = format!(
            "::tcld::dbg::__nub__::send_frame evt 0 {{{}}}",
            crate::tcl_escape_braces(&payload),
        );
        if let Ok(cstr) = CString::new(script) {
            unsafe {
                Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0);
            }
        }
    });
    state.lock().ok().map(|mut g| g.subscribe(cb))
}

pub(crate) unsafe extern "C" fn nub_connect_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 3 {
        return crate::set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::nub_connect host port\"",
        );
    }
    let host = match unsafe { crate::objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return crate::set_error(interp, "host missing"),
    };
    let port = match unsafe { crate::objv_to_string(objv, 2) } {
        Some(s) => s,
        None => return crate::set_error(interp, "port missing"),
    };
    let Some(state) = crate::step_engine_state(interp) else {
        return crate::set_error(interp, "step engine not installed");
    };

    if let Err(err) = unsafe { install_nub_helpers(interp) } {
        return crate::set_error(interp, &err);
    }

    // Build the hello payload.
    let tcl_version = crate::runtime_patchlevel(interp);
    let compiled_abi = if cfg!(feature = "tcl8_5") {
        "compiled_tcl8_5"
    } else if cfg!(feature = "tcl8_6") {
        "compiled_tcl8_6"
    } else {
        "compiled_tcl9"
    };
    let runtime_abi = if tcl_version.starts_with("8.5.") {
        "runtime_tcl8_5"
    } else if tcl_version.starts_with("8.6.") {
        "runtime_tcl8_6"
    } else if tcl_version.starts_with("9.") {
        "runtime_tcl9"
    } else {
        "runtime_tcl_unknown"
    };
    let hello = Hello {
        nub_version: env!("CARGO_PKG_VERSION").to_string(),
        tcl_version,
        protocol_version: PROTOCOL_VERSION,
        capabilities: vec![compiled_abi.to_string(), runtime_abi.to_string()],
    };
    let hello_payload = hello.to_payload();

    // Drive the Tcl-side connect proc as a Tcl list. Manually escaping the
    // payload inside braces flattens nested list values such as the handshake
    // capability list, causing Tcl controllers to parse an odd-sized dict.
    let connect_script = tcld_debug_wire::tcl_list_format([
        "::tcld::dbg::__nub__::connect",
        host.as_str(),
        port.as_str(),
        hello_payload.as_str(),
    ]);
    let cstr = match CString::new(connect_script) {
        Ok(c) => c,
        Err(_) => return crate::set_error(interp, "script contains NUL"),
    };
    let rc = unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
    if rc != TCL_OK {
        return rc;
    }

    // Now subscribe the event forwarder.
    if let Some(_sub_id) = subscribe_event_forwarder(&state, interp) {
        // Stash the SubscriptionId in a Tcl variable so disconnect can find it.
        // For now we keep it implicit; disconnect closes the socket and the
        // subscriber's writes will fail and be silently dropped.
    }

    // Mark the engine as wire-attached so the trace handler stops
    // deferring pauses while the user script is parked in `vwait`.
    step_engine::set_wire_attached(interp, true);

    crate::set_result(interp, "ok")
}

pub(crate) unsafe extern "C" fn nub_status_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    // Read the Tcl-side state variable.
    let script = b"if {[info exists ::tcld::dbg::__nub__::state]} { set ::tcld::dbg::__nub__::state } else { return disconnected }\0";
    unsafe {
        let _ = Tcl_EvalEx(
            interp,
            script.as_ptr() as *const std::os::raw::c_char,
            -1,
            0,
        );
    }
    TCL_OK
}

pub(crate) unsafe extern "C" fn nub_disconnect_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let script = b"::tcld::dbg::__nub__::disconnect\0";
    unsafe {
        let rc = Tcl_EvalEx(
            interp,
            script.as_ptr() as *const std::os::raw::c_char,
            -1,
            0,
        );
        if rc != TCL_OK {
            return rc;
        }
    }
    crate::set_result(interp, "ok")
}
