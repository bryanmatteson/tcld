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
set ::tcld::dbg::__nub__::breakpoint_next -1
# These are logical breakpoint records, despite the legacy array name. They
# live for the full controller session, retain requested source identity, and
# optionally point at a generation-specific core breakpoint via `real_id`.
array set ::tcld::dbg::__nub__::pending_breakpoints {}
# Controller ids assigned to core-only breakpoint kinds. This keeps the raw
# core bpN namespace private even when its counters diverge from logical ids.
array set ::tcld::dbg::__nub__::passthrough_breakpoints {}

proc ::tcld::dbg::__nub__::send_event {kind fields} {
    ::tcld::dbg::__nub__::send_frame evt 0 [linsert $fields 0 $kind]
}

proc ::tcld::dbg::__nub__::allocate_breakpoint_id {} {
    while 1 {
        set id bp[incr ::tcld::dbg::__nub__::breakpoint_next]
        if {![info exists ::tcld::dbg::__nub__::pending_breakpoints($id)] && \
            ![info exists ::tcld::dbg::__nub__::passthrough_breakpoints($id)]} {
            return $id
        }
    }
}

proc ::tcld::dbg::__nub__::external_id_for_core {real_id} {
    set logical [::tcld::dbg::__nub__::logical_id_for_core $real_id]
    if {$logical ne $real_id} { return $logical }
    foreach id [array names ::tcld::dbg::__nub__::passthrough_breakpoints] {
        if {$::tcld::dbg::__nub__::passthrough_breakpoints($id) eq $real_id} { return $id }
    }
    set id [::tcld::dbg::__nub__::allocate_breakpoint_id]
    set ::tcld::dbg::__nub__::passthrough_breakpoints($id) $real_id
    ::tcld::dbg::__bind_logical_breakpoint $id $real_id
    return $id
}

proc ::tcld::dbg::__nub__::logical_id_for_core {real_id} {
    foreach id [array names ::tcld::dbg::__nub__::pending_breakpoints] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        if {[dict exists $record real_id] && [dict get $record real_id] eq $real_id} {
            return $id
        }
    }
    foreach id [array names ::tcld::dbg::__nub__::passthrough_breakpoints] {
        if {$::tcld::dbg::__nub__::passthrough_breakpoints($id) eq $real_id} { return $id }
    }
    return $real_id
}

proc ::tcld::dbg::__nub__::core_breakpoint_row {real_id} {
    foreach row [::tcld::dbg::breakpoint_list] {
        if {[dict get $row id] eq $real_id} { return $row }
    }
    return {}
}

proc ::tcld::dbg::__nub__::logical_hit_count {record} {
    set hits [dict get $record hits]
    if {[dict exists $record real_id]} {
        set row [::tcld::dbg::__nub__::core_breakpoint_row [dict get $record real_id]]
        if {$row ne ""} { incr hits [dict get $row hits] }
    }
    return $hits
}

proc ::tcld::dbg::__nub__::detach_logical_binding {id} {
    if {![info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} { return }
    set record $::tcld::dbg::__nub__::pending_breakpoints($id)
    if {[dict exists $record real_id]} {
        set real_id [dict get $record real_id]
        set row [::tcld::dbg::__nub__::core_breakpoint_row $real_id]
        if {$row ne ""} { dict incr record hits [dict get $row hits] }
        ::tcld::dbg::__unbind_logical_breakpoint $real_id
        ::tcld::dbg::__wrap__::orig_catch {::tcld::dbg::bp_remove $real_id}
        foreach key {real_id resolved_block resolved_offset resolved_len resolved_line \
                     resolution_generation} {
            if {[dict exists $record $key]} { dict unset record $key }
        }
    }
    set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
}

proc ::tcld::dbg::__nub__::resolve_logical_breakpoint {id generation change} {
    if {![info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} { return 0 }
    ::tcld::dbg::__nub__::detach_logical_binding $id
    set record $::tcld::dbg::__nub__::pending_breakpoints($id)
    set kind [dict get $record kind]
    if {$kind eq "file_line"} {
        set block "file:[dict get $record path]"
        set line [dict get $record line]
        if {[::tcld::dbg::__wrap__::orig_catch \
            {::tcld::dbg::file_line_location [dict get $record path] $line} location]} {
            dict set record pending_reason \
                "requested line $line has no executable command in $block generation $generation after $change"
            set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
            return 0
        }
        set offset [dict get $location offset]
        set len [dict get $location len]
        set resolved_line $line
        set args [list file [dict get $record path] $offset $len]
    } elseif {$kind eq "file"} {
        set block "file:[dict get $record path]"
        set offset [dict get $record offset]
        set len [dict get $record len]
        set resolved_line {}
        set args [list file [dict get $record path] $offset $len]
    } else {
        set block "proc:[dict get $record qname]"
        set offset [dict get $record offset]
        set len [dict get $record len]
        set resolved_line {}
        set args [list source [dict get $record qname] $offset $len]
    }
    if {$kind ne "file_line"} {
        set requested_len [dict get $record len]
        if {[::tcld::dbg::__wrap__::orig_catch \
            {::tcld::dbg::block_command_span $block $offset $requested_len} command]} {
            dict set record pending_reason \
                "requested command coordinate $offset is not executable in $block generation $generation after $change"
            set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
            return 0
        }
        set command_text [dict get $command text]
        set actual_len [dict get $command len]
        if {[dict exists $record anchor_text] && \
            ([dict get $record anchor_text] ne $command_text || \
             [dict get $record anchor_len] != $actual_len)} {
            dict set record pending_reason \
                "requested command anchor changed in $block generation $generation after $change"
            set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
            return 0
        }
        if {![dict exists $record anchor_text]} {
            dict set record anchor_text $command_text
            dict set record anchor_len $actual_len
        }
        set len $actual_len
        if {$kind eq "file"} {
            set args [list file [dict get $record path] $offset $len]
        } else {
            set args [list source [dict get $record qname] $offset $len]
        }
    }
    if {[dict get $record condition] ne ""} { lappend args [dict get $record condition] }
    if {[::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::breakpoint_add {*}$args} real_id]} {
        dict set record pending_reason \
            "requested command span $offset+$len is not executable in $block generation $generation after $change"
        set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
        return 0
    }
    if {![dict get $record enabled]} { ::tcld::dbg::bp_disable $real_id }
    set remaining_ignore [expr {max(0, [dict get $record ignore] - [dict get $record hits])}]
    if {$remaining_ignore > 0} { ::tcld::dbg::bp_set_ignore $real_id $remaining_ignore }
    set core_row [::tcld::dbg::__nub__::core_breakpoint_row $real_id]
    if {$core_row ne "" && [dict exists $core_row line]} {
        set resolved_line [dict get $core_row line]
    }
    dict set record real_id $real_id
    ::tcld::dbg::__bind_logical_breakpoint $id $real_id
    dict set record resolved_block $block
    dict set record resolved_offset $offset
    dict set record resolved_len $len
    if {$resolved_line ne ""} { dict set record resolved_line $resolved_line }
    dict set record resolution_generation $generation
    dict set record last_bound_generation $generation
    if {[dict exists $record pending_reason]} { dict unset record pending_reason }
    set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
    return 1
}

proc ::tcld::dbg::__nub__::bp_add {args_list} {
    set kind [lindex $args_list 0]
    if {$kind ni {file_line source file}} {
        set real_id [::tcld::dbg::breakpoint_add {*}$args_list]
        return [::tcld::dbg::__nub__::external_id_for_core $real_id]
    }
    set condition [lindex $args_list end]
    set id [::tcld::dbg::__nub__::allocate_breakpoint_id]
    if {$kind eq "file_line"} {
        if {[llength $args_list] ni {3 4}} {
            error "wrong # args: file_line path line ?condition?"
        }
        set path [file normalize [lindex $args_list 1]]
        set line [lindex $args_list 2]
        if {![string is integer -strict $line] || $line < 1} {
            error "file line must be a positive integer"
        }
        if {[llength $args_list] == 3} { set condition {} }
        set record [dict create kind file_line path $path line $line \
            condition $condition enabled 1 ignore 0]
    } elseif {$kind eq "file"} {
        if {[llength $args_list] ni {4 5}} { error "wrong # args: file path offset len ?condition?" }
        if {[llength $args_list] == 4} { set condition {} }
        set offset [lindex $args_list 2]
        set len [lindex $args_list 3]
        if {![string is integer -strict $offset] || $offset < 0 || \
            ![string is integer -strict $len] || $len < 1} {
            error "file command offset must be non-negative and len must be positive"
        }
        set record [dict create kind file path [file normalize [lindex $args_list 1]] \
            offset $offset len $len condition $condition enabled 1 ignore 0 hits 0]
    } else {
        if {[llength $args_list] ni {4 5}} { error "wrong # args: source qname offset len ?condition?" }
        if {[llength $args_list] == 4} { set condition {} }
        set offset [lindex $args_list 2]
        set len [lindex $args_list 3]
        if {![string is integer -strict $offset] || $offset < 0 || \
            ![string is integer -strict $len] || $len < 1} {
            error "proc command offset must be non-negative and len must be positive"
        }
        set record [dict create kind source qname [string trimleft [lindex $args_list 1] :] \
            offset $offset len $len condition $condition enabled 1 ignore 0 hits 0]
    }
    if {$kind eq "file_line"} { dict set record hits 0 }
    set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
    if {$kind eq "source"} {
        set block "proc:[dict get $record qname]"
    } else {
        set block "file:[dict get $record path]"
    }
    if {![::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::block_generation $block} generation]} {
        ::tcld::dbg::__nub__::resolve_logical_breakpoint $id $generation registration
    } else {
        dict set ::tcld::dbg::__nub__::pending_breakpoints($id) pending_reason \
            "source owner $block is not registered yet"
    }
    return $id
}

proc ::tcld::dbg::__nub__::bp_list {} {
    set internal_ids {}
    foreach id [array names ::tcld::dbg::__nub__::pending_breakpoints] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        if {[dict exists $record real_id]} { lappend internal_ids [dict get $record real_id] }
    }
    set rows {}
    foreach row [::tcld::dbg::breakpoint_list] {
        if {[dict get $row id] ni $internal_ids} {
            dict set row id [::tcld::dbg::__nub__::external_id_for_core [dict get $row id]]
            lappend rows $row
        }
    }
    foreach id [lsort [array names ::tcld::dbg::__nub__::pending_breakpoints]] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        set kind [dict get $record kind]
        if {$kind eq "source"} {
            set block "proc:[dict get $record qname]"
        } else {
            set block "file:[dict get $record path]"
        }
        set offset [expr {$kind eq "file_line" ? 0 : [dict get $record offset]}]
        set len [expr {$kind eq "file_line" ? 1 : [dict get $record len]}]
        set pending [expr {![dict exists $record real_id]}]
        set row [list id $id kind line block $block offset $offset len $len \
            enabled [dict get $record enabled] \
            hits [::tcld::dbg::__nub__::logical_hit_count $record] \
            ignore [dict get $record ignore] pending $pending]
        if {$kind eq "file_line"} { lappend row line [dict get $record line] }
        if {[dict exists $record real_id]} {
            foreach key {resolved_block resolved_offset resolved_len resolved_line \
                         resolution_generation} {
                if {[dict exists $record $key]} { lappend row $key [dict get $record $key] }
            }
        } elseif {[dict exists $record pending_reason]} {
            lappend row pending_reason [dict get $record pending_reason]
        }
        if {[dict get $record condition] ne ""} {
            lappend row condition [dict get $record condition]
        }
        lappend rows $row
    }
    return $rows
}

proc ::tcld::dbg::__nub__::bp_remove {id} {
    if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
        ::tcld::dbg::__nub__::detach_logical_binding $id
        unset ::tcld::dbg::__nub__::pending_breakpoints($id)
        return ok
    }
    if {[info exists ::tcld::dbg::__nub__::passthrough_breakpoints($id)]} {
        set real_id $::tcld::dbg::__nub__::passthrough_breakpoints($id)
        set result [::tcld::dbg::bp_remove $real_id]
        ::tcld::dbg::__unbind_logical_breakpoint $real_id
        unset ::tcld::dbg::__nub__::passthrough_breakpoints($id)
        return $result
    }
    error "breakpoint not found: $id"
}

proc ::tcld::dbg::__nub__::bp_enable {id enabled} {
    if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
        dict set ::tcld::dbg::__nub__::pending_breakpoints($id) enabled $enabled
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        if {[dict exists $record real_id]} {
            if {$enabled} {
                ::tcld::dbg::bp_enable [dict get $record real_id]
            } else {
                ::tcld::dbg::bp_disable [dict get $record real_id]
            }
        }
        return ok
    }
    if {[info exists ::tcld::dbg::__nub__::passthrough_breakpoints($id)]} {
        set id $::tcld::dbg::__nub__::passthrough_breakpoints($id)
    } else {
        error "breakpoint not found: $id"
    }
    if {$enabled} {
        return [::tcld::dbg::bp_enable $id]
    }
    return [::tcld::dbg::bp_disable $id]
}

proc ::tcld::dbg::__nub__::bp_set_ignore {id count} {
    if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
        dict set ::tcld::dbg::__nub__::pending_breakpoints($id) ignore $count
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        if {[dict exists $record real_id]} {
            set remaining [expr {max(0, $count - [dict get $record hits])}]
            ::tcld::dbg::bp_set_ignore [dict get $record real_id] $remaining
        }
        return ok
    }
    if {[info exists ::tcld::dbg::__nub__::passthrough_breakpoints($id)]} {
        set id $::tcld::dbg::__nub__::passthrough_breakpoints($id)
    } else {
        error "breakpoint not found: $id"
    }
    return [::tcld::dbg::bp_set_ignore $id $count]
}

proc ::tcld::dbg::__nub__::bp_move {args_list} {
    set id [lindex $args_list 0]
    set target_kind [lindex $args_list 1]
    if {$target_kind in {source file}} {
        if {[llength $args_list] != 5} {
            error "wrong # args: bp_move id source|file identity offset len"
        }
        set identity [lindex $args_list 2]
        set offset [lindex $args_list 3]
        set len [lindex $args_list 4]
        if {![string is integer -strict $offset] || $offset < 0 || \
            ![string is integer -strict $len] || $len < 1} {
            error "breakpoint move offset must be non-negative and len must be positive"
        }
    } elseif {$target_kind eq "file_line"} {
        if {[llength $args_list] != 4} {
            error "wrong # args: bp_move id file_line path line"
        }
        set identity [lindex $args_list 2]
        set line [lindex $args_list 3]
        if {![string is integer -strict $line] || $line < 1} {
            error "breakpoint move line must be positive"
        }
    } elseif {$target_kind eq "instruction"} {
        # The core relocate API cannot change breakpoint kind or instruction
        # PC, so replace the core binding while retaining the external id.
        if {[llength $args_list] != 3} {
            error "wrong # args: bp_move id instruction pc"
        }
        set pc [lindex $args_list 2]
        if {![string is integer -strict $pc] || $pc < 0} {
            error "instruction pc must be non-negative"
        }
    } else {
        error "unsupported breakpoint move target: $target_kind"
    }

    if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
        ::tcld::dbg::__nub__::detach_logical_binding $id
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
    } elseif {[info exists ::tcld::dbg::__nub__::passthrough_breakpoints($id)]} {
        set real_id $::tcld::dbg::__nub__::passthrough_breakpoints($id)
        set record [::tcld::dbg::__nub__::core_breakpoint_row $real_id]
        if {$record eq ""} { error "breakpoint not found: $id" }
        if {[dict get $record kind] in {var exception}} {
            error "[dict get $record kind] breakpoint cannot move to $target_kind"
        }
        ::tcld::dbg::bp_remove $real_id
        ::tcld::dbg::__unbind_logical_breakpoint $real_id
        unset ::tcld::dbg::__nub__::passthrough_breakpoints($id)
    } else {
        error "breakpoint not found: $id"
    }

    if {![dict exists $record condition]} { dict set record condition {} }
    if {![dict exists $record enabled]} { dict set record enabled 1 }
    if {![dict exists $record ignore]} { dict set record ignore 0 }
    if {![dict exists $record hits]} { dict set record hits 0 }

    if {$target_kind eq "instruction"} {
        if {[info exists ::tcld::dbg::__nub__::pending_breakpoints($id)]} {
            unset ::tcld::dbg::__nub__::pending_breakpoints($id)
        }
        set add_args [list instruction $pc]
        if {[dict get $record condition] ne ""} {
            lappend add_args [dict get $record condition]
        }
        set real_id [::tcld::dbg::breakpoint_add {*}$add_args]
        if {![dict get $record enabled]} { ::tcld::dbg::bp_disable $real_id }
        if {[dict get $record ignore] > 0} {
            ::tcld::dbg::bp_set_ignore $real_id [dict get $record ignore]
        }
        set ::tcld::dbg::__nub__::passthrough_breakpoints($id) $real_id
        ::tcld::dbg::__bind_logical_breakpoint $id $real_id
        return ok
    }

    foreach key {path line qname offset len anchor_text anchor_len pending_reason \
                 last_bound_generation real_id resolved_block resolved_offset \
                 resolved_len resolved_line resolution_generation kind} {
        if {[dict exists $record $key]} { dict unset record $key }
    }
    if {$target_kind eq "source"} {
        set identity [string trimleft $identity :]
        dict set record kind source
        dict set record qname $identity
        dict set record offset $offset
        dict set record len $len
        set block "proc:$identity"
    } elseif {$target_kind eq "file"} {
        set identity [file normalize $identity]
        dict set record kind file
        dict set record path $identity
        dict set record offset $offset
        dict set record len $len
        set block "file:$identity"
    } else {
        set identity [file normalize $identity]
        dict set record kind file_line
        dict set record path $identity
        dict set record line $line
        set block "file:$identity"
    }
    set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
    if {[::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::block_generation $block} generation]} {
        dict set ::tcld::dbg::__nub__::pending_breakpoints($id) pending_reason \
            "source owner $block is not registered yet"
    } else {
        ::tcld::dbg::__nub__::resolve_logical_breakpoint $id $generation relocation
    }
    return ok
}

# Rewrite rows returned by core-only filtered queries while the binding map is
# still authoritative. This is the only path that may allocate passthrough ids.
proc ::tcld::dbg::__nub__::externalize_breakpoint_rows {rows} {
    set external {}
    foreach row $rows {
        if {[dict exists $row id]} {
            dict set row id [::tcld::dbg::__nub__::external_id_for_core [dict get $row id]]
        }
        lappend external $row
    }
    return $external
}

proc ::tcld::dbg::__nub__::bp_validate {qname} {
    set rows [::tcld::dbg::__nub__::externalize_breakpoint_rows \
        [::tcld::dbg::bp_validate $qname]]
    # Core validation may relocate a generation-specific binding. Re-resolve
    # logical intent immediately so its owner-derived anchor remains authoritative.
    ::tcld::dbg::__nub__::resolve_pending_proc_breakpoints $qname
    return $rows
}

proc ::tcld::dbg::__nub__::line_breakpoints {args_list} {
    if {[llength $args_list] > 2} {
        error "wrong # args: should be \"line_breakpoints ?qname? ?offset?\""
    }
    set qname_filter [expr {[llength $args_list] >= 1 \
        ? [string trimleft [lindex $args_list 0] :] : {}}]
    set has_offset [expr {[llength $args_list] == 2}]
    if {$has_offset} {
        set offset_filter [lindex $args_list 1]
        if {![string is integer -strict $offset_filter] || $offset_filter < 0} {
            error "offset must be non-negative integer"
        }
    }
    set rows {}
    foreach row [::tcld::dbg::__nub__::bp_list] {
        if {![dict exists $row kind] || [dict get $row kind] ne "line" || \
            ![dict exists $row block] || ![dict exists $row offset]} continue
        set block [dict get $row block]
        if {$qname_filter ne ""} {
            if {[string match {proc:*} $block]} {
                set owner [string range $block 5 end]
            } elseif {[string match {file:*} $block]} {
                set owner [string range $block 5 end]
            } else {
                continue
            }
            if {$owner ne $qname_filter} continue
        }
        set current_offset [expr {[dict exists $row resolved_offset] \
            ? [dict get $row resolved_offset] : [dict get $row offset]}]
        if {$has_offset && $current_offset != $offset_filter} continue
        lappend rows $row
    }
    return $rows
}

proc ::tcld::dbg::__nub__::bp_export {} {
    set rows {}
    foreach row [::tcld::dbg::bp_export] {
        # Source line rows backing logical intent are reconstructed below.
        # Non-source kinds retain the core persistence contract.
        if {[dict get $row kind] ne "line"} { lappend rows $row }
    }
    foreach id [lsort [array names ::tcld::dbg::__nub__::pending_breakpoints]] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        set kind [dict get $record kind]
        if {$kind eq "file_line"} {
            lappend rows [list kind file_line path [dict get $record path] \
                line [dict get $record line] enabled [dict get $record enabled] \
                ignore [dict get $record ignore] condition [dict get $record condition]]
        } elseif {$kind eq "source"} {
            set row [list kind proc_command qname [dict get $record qname] \
                offset [dict get $record offset] len [dict get $record len] \
                enabled [dict get $record enabled] ignore [dict get $record ignore] \
                condition [dict get $record condition]]
            if {[dict exists $record anchor_text]} {
                lappend row anchor_text [dict get $record anchor_text] \
                    anchor_len [dict get $record anchor_len]
            }
            lappend rows $row
        } else {
            set row [list kind file_command path [dict get $record path] \
                offset [dict get $record offset] len [dict get $record len] \
                enabled [dict get $record enabled] ignore [dict get $record ignore] \
                condition [dict get $record condition]]
            if {[dict exists $record anchor_text]} {
                lappend row anchor_text [dict get $record anchor_text] \
                    anchor_len [dict get $record anchor_len]
            }
            lappend rows $row
        }
    }
    return $rows
}

proc ::tcld::dbg::__nub__::bp_import {serialized} {
    foreach id [array names ::tcld::dbg::__nub__::pending_breakpoints] {
        ::tcld::dbg::__nub__::detach_logical_binding $id
    }
    foreach id [array names ::tcld::dbg::__nub__::passthrough_breakpoints] {
        ::tcld::dbg::__unbind_logical_breakpoint \
            $::tcld::dbg::__nub__::passthrough_breakpoints($id)
    }
    array unset ::tcld::dbg::__nub__::pending_breakpoints
    array set ::tcld::dbg::__nub__::pending_breakpoints {}
    array unset ::tcld::dbg::__nub__::passthrough_breakpoints
    array set ::tcld::dbg::__nub__::passthrough_breakpoints {}
    set core_rows {}
    set imported {}
    foreach row $serialized {
        set kind [dict get $row kind]
        if {$kind ni {file_line file_command proc_command line}} {
            lappend core_rows $row
            continue
        }
        set id [::tcld::dbg::__nub__::allocate_breakpoint_id]
        if {$kind eq "file_line"} {
            set record [dict create kind file_line path [file normalize [dict get $row path]] \
                line [dict get $row line] condition [dict get $row condition] \
                enabled [dict get $row enabled] ignore [dict get $row ignore] hits 0 \
                pending_reason "source owner is not registered in this interpreter yet"]
        } elseif {$kind eq "file_command"} {
            set record [dict create kind file path [file normalize [dict get $row path]] \
                offset [dict get $row offset] len [dict get $row len] \
                condition [dict get $row condition] enabled [dict get $row enabled] \
                ignore [dict get $row ignore] hits 0 \
                pending_reason "source owner is not registered in this interpreter yet"]
        } elseif {$kind eq "proc_command"} {
            set record [dict create kind source qname [string trimleft [dict get $row qname] :] \
                offset [dict get $row offset] len [dict get $row len] \
                condition [dict get $row condition] enabled [dict get $row enabled] \
                ignore [dict get $row ignore] hits 0 \
                pending_reason "procedure owner is not registered in this interpreter yet"]
        } else {
            set block [dict get $row block]
            if {[string match {proc:*} $block]} {
                set record [dict create kind source qname [string range $block 5 end] \
                    offset [dict get $row offset] len [dict get $row len] \
                    condition [dict get $row condition] enabled [dict get $row enabled] \
                    ignore [dict get $row ignore] hits 0 \
                    pending_reason "procedure owner is not registered in this interpreter yet"]
            } elseif {[string match {file:*} $block]} {
                set record [dict create kind file path [file normalize [string range $block 5 end]] \
                    offset [dict get $row offset] len [dict get $row len] \
                    condition [dict get $row condition] enabled [dict get $row enabled] \
                    ignore [dict get $row ignore] hits 0 \
                    pending_reason "source owner is not registered in this interpreter yet"]
            } else {
                # Anonymous identities are deliberately not restart-durable.
                continue
            }
        }
        if {[dict exists $row anchor_text]} {
            dict set record anchor_text [dict get $row anchor_text]
            dict set record anchor_len [dict get $row anchor_len]
        }
        set ::tcld::dbg::__nub__::pending_breakpoints($id) $record
        lappend imported $id
    }
    set core_ids [::tcld::dbg::bp_import $core_rows]
    set external_core_ids {}
    foreach real_id $core_ids {
        lappend external_core_ids [::tcld::dbg::__nub__::external_id_for_core $real_id]
    }
    return [concat $external_core_ids $imported]
}

proc ::tcld::dbg::__nub__::resolve_pending_breakpoints {path} {
    set path [file normalize $path]
    if {[::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::block_generation "file:$path"} generation]} { return {} }
    set resolved {}
    foreach id [array names ::tcld::dbg::__nub__::pending_breakpoints] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        set kind [dict get $record kind]
        if {$kind eq "source"} continue
        if {[dict get $record path] ne $path} continue
        set transition [expr {[dict exists $record real_id] || \
            [dict exists $record last_bound_generation] ? "revalidation" : "registration"}]
        if {[::tcld::dbg::__nub__::resolve_logical_breakpoint \
            $id $generation $transition]} { lappend resolved $id }
    }
    return $resolved
}

proc ::tcld::dbg::__nub__::resolve_pending_proc_breakpoints {qname} {
    set qname [string trimleft $qname :]
    if {[::tcld::dbg::__wrap__::orig_catch \
        {::tcld::dbg::block_generation "proc:$qname"} generation]} { return {} }
    set resolved {}
    foreach id [array names ::tcld::dbg::__nub__::pending_breakpoints] {
        set record $::tcld::dbg::__nub__::pending_breakpoints($id)
        if {[dict get $record kind] ne "source" || [dict get $record qname] ne $qname} continue
        set transition [expr {[dict exists $record real_id] || \
            [dict exists $record last_bound_generation] ? "revalidation" : "registration"}]
        if {[::tcld::dbg::__nub__::resolve_logical_breakpoint \
            $id $generation $transition]} { lappend resolved $id }
    }
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
    set path [file normalize $path]
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
        bp_move          { return [::tcld::dbg::__nub__::bp_move $args_list] }
        bp_validate      { return [::tcld::dbg::__nub__::bp_validate {*}$args_list] }
        bp_set_ignore    { return [::tcld::dbg::__nub__::bp_set_ignore {*}$args_list] }
        bp_export        { return [::tcld::dbg::__nub__::bp_export] }
        bp_import        { return [::tcld::dbg::__nub__::bp_import {*}$args_list] }
        wp_add           {
            set real_id [::tcld::dbg::watchpoint_add {*}$args_list]
            return [::tcld::dbg::__nub__::external_id_for_core $real_id]
        }
        var_breakpoints  {
            return [::tcld::dbg::__nub__::externalize_breakpoint_rows \
                [::tcld::dbg::var_breakpoints {*}$args_list]]
        }
        line_breakpoints { return [::tcld::dbg::__nub__::line_breakpoints $args_list] }
        instruction_breakpoints {
            return [::tcld::dbg::__nub__::externalize_breakpoint_rows \
                [::tcld::dbg::instruction_breakpoints {*}$args_list]]
        }
        proc_list        { return [::tcld::dbg::list_procs] }
        proc_body        { return [::tcld::dbg::proc_body {*}$args_list] }
        block_source     { return [::tcld::dbg::block_source {*}$args_list] }
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
        DebugEvent::BlocksRevalidated {
            block,
            generation,
            source_path,
            change,
        } => Some(W::BlocksRevalidated {
            block: block.as_string(),
            generation: *generation,
            source_path: source_path.as_ref().map(ToString::to_string),
            change: match change {
                tcld_debug_state::BlockChange::Registered => {
                    tcld_debug_wire::SourceUnitChange::Registered
                }
                tcld_debug_state::BlockChange::Revalidated => {
                    tcld_debug_wire::SourceUnitChange::Revalidated
                }
            },
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
    _active_target_path: Option<&str>,
) -> tcld_debug_wire::SourceLocation {
    use tcld_debug_state::BlockId;
    use tcld_debug_wire::SourceBlockKind;

    let (block_kind, source_path, qualified_proc) = match loc.block() {
        BlockId::File(path) => (SourceBlockKind::File, Some(path.to_string()), None),
        // A proc's defining file is not necessarily the active entry target.
        // Registration events carry owner-derived definition provenance; do
        // not invent an entry-file association here.
        BlockId::ProcBody(qname) => (SourceBlockKind::Proc, None, Some(qname.to_string())),
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

fn logical_breakpoint_id(interp: *mut Tcl_Interp, core_id: &str) -> String {
    step_engine::logical_breakpoint_id(interp, core_id).unwrap_or_else(|| core_id.to_owned())
}

fn rewrite_wire_breakpoint_id(interp: *mut Tcl_Interp, event: &mut WireEvent) {
    let breakpoint = match event {
        WireEvent::Paused { breakpoint, .. } | WireEvent::ErrorPaused { breakpoint, .. } => {
            breakpoint
        }
        WireEvent::ConditionError { breakpoint, .. } => {
            *breakpoint = logical_breakpoint_id(interp, breakpoint);
            return;
        }
        _ => return,
    };
    if let Some(core_id) = breakpoint.as_mut() {
        *core_id = logical_breakpoint_id(interp, core_id);
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
        let Some(mut wire_event) = debug_event_to_wire(&event, active_target_path.as_deref())
        else {
            return;
        };
        rewrite_wire_breakpoint_id(interp, &mut wire_event);
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
