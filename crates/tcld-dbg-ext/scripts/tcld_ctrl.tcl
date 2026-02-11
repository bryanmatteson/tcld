# tcld_ctrl: pure-Tcl controller library for tcld's wire protocol.
#
# Tier 4 Phase 4-C-Tcl. Speaks the same wire format as the Rust
# controller (`tcld-debug-wire::controller`) — both are first-class
# v2 deliverables of the parity tier.
#
# Public API: see `::tcld::ctrl::*` procs below.

package provide tcld_ctrl 0.1.0
namespace eval ::tcld::ctrl {}

# --- Internal state ----------------------------------------------------

namespace eval ::tcld::ctrl {
    variable state idle
    variable listen_sock ""
    variable bound_port 0
    variable peer_sock ""
    variable next_serial 1
    variable nub_capabilities {}
    variable pending_responses ;# array: serial -> {kind payload}
    variable pending_events {} ;# list of payloads (raw)
    variable event_callbacks ;# array: kind -> script
    variable nub_pid 0
    array set ::tcld::ctrl::pending_responses {}
    array set ::tcld::ctrl::event_callbacks {}
}

proc ::tcld::ctrl::status {} {
    variable state
    return $state
}

# Bind a TCP listener. 0 = OS picks a port.
proc ::tcld::ctrl::listen {port} {
    variable state
    variable listen_sock
    variable bound_port
    if {$state ne "idle"} {
        error "listener already $state"
    }
    set listen_sock [socket -server ::tcld::ctrl::__on_connect__ $port]
    set bound_port [lindex [chan configure $listen_sock -sockname] 2]
    set state listening
    return $bound_port
}

proc ::tcld::ctrl::__on_connect__ {sock host port} {
    variable state
    variable peer_sock
    if {$state ne "listening"} {
        # Already have a peer or wrong state; close incoming.
        catch {close $sock}
        return
    }
    set state accepting
    set peer_sock $sock
    chan configure $sock -translation binary -buffering none -blocking 1
    # Trigger the handshake. We do it inline (blocking) — accept() callers
    # must follow `listen` with `accept` to drive the handshake to
    # completion.
}

# Block until a nub connects and the handshake completes. Returns the
# nub's reported capabilities. handshake_timeout is in milliseconds.
proc ::tcld::ctrl::accept {{handshake_timeout 5000}} {
    variable state
    variable listen_sock
    variable peer_sock
    variable nub_capabilities
    if {$state ne "listening" && $state ne "accepting"} {
        error "controller not listening (state=$state)"
    }

    # Wait for __on_connect__ to populate peer_sock. We use vwait on
    # the state variable for this.
    set deadline [expr {[clock milliseconds] + $handshake_timeout}]
    while {$state eq "listening"} {
        set remaining [expr {$deadline - [clock milliseconds]}]
        if {$remaining <= 0} {
            error "handshake timeout waiting for nub connect"
        }
        # Process one event with a timeout via after.
        set ::tcld::ctrl::__tick__ 0
        set timer [after $remaining {set ::tcld::ctrl::__tick__ 1}]
        vwait ::tcld::ctrl::state
        after cancel $timer
    }

    if {$state ne "accepting" || $peer_sock eq ""} {
        error "no peer connected (state=$state)"
    }

    # Read hello.
    chan configure $peer_sock -blocking 1
    set state handshaking
    set hello_body [::tcld::ctrl::__read_frame__ $peer_sock $handshake_timeout]
    lassign $hello_body kind serial payload
    if {$kind ne "hello"} {
        ::tcld::ctrl::close
        error "handshake: expected hello, got $kind"
    }
    array set hello_dict $payload
    set proto_v [expr {[info exists hello_dict(protocol_version)] ? $hello_dict(protocol_version) : 0}]
    if {$proto_v != 2} {
        ::tcld::ctrl::close
        error "handshake: protocol version mismatch (got $proto_v, expected 2)"
    }
    if {[info exists hello_dict(capabilities)]} {
        set nub_capabilities $hello_dict(capabilities)
    }

    # Send hello_ack.
    set ack_payload "controller_version 0.1.0 protocol_version 2 capabilities {}"
    ::tcld::ctrl::__send_frame__ $peer_sock hello_ack 0 $ack_payload

    # Switch to event-driven (fileevent) for subsequent frames.
    chan configure $peer_sock -blocking 0
    fileevent $peer_sock readable [list ::tcld::ctrl::__on_readable__]
    set state open
    return $nub_capabilities
}

proc ::tcld::ctrl::__send_frame__ {sock kind serial payload} {
    set body [list $kind $serial $payload]
    set len [string length $body]
    if {[catch {
        puts -nonewline $sock "$len\n$body\n"
        flush $sock
    } err]} {
        ::tcld::ctrl::__handle_disconnect__ $err
    }
}

# Blocking read of one frame. Used during handshake.
proc ::tcld::ctrl::__read_frame__ {sock timeout_ms} {
    set deadline [expr {[clock milliseconds] + $timeout_ms}]
    set len_line ""
    while {1} {
        set chunk [read $sock 1]
        if {$chunk eq ""} {
            if {[clock milliseconds] > $deadline} {
                error "read timeout"
            }
            after 10
            continue
        }
        if {$chunk eq "\n"} break
        append len_line $chunk
    }
    if {![string is integer -strict $len_line]} {
        error "bad length prefix: $len_line"
    }
    set body ""
    while {[string length $body] < $len_line} {
        set need [expr {$len_line - [string length $body]}]
        set chunk [read $sock $need]
        if {$chunk eq ""} {
            if {[clock milliseconds] > $deadline} {
                error "read timeout"
            }
            after 10
            continue
        }
        append body $chunk
    }
    # Trailing newline.
    set trail [read $sock 1]
    if {$trail ne "\n"} {
        error "missing trailing newline"
    }
    return $body
}

proc ::tcld::ctrl::__on_readable__ {} {
    variable state
    variable peer_sock
    if {$state ne "open"} return
    if {[catch {::tcld::ctrl::__read_frame_nb__ $peer_sock} body]} {
        ::tcld::ctrl::__handle_disconnect__ $body
        return
    }
    if {$body eq ""} {
        # No complete frame yet; readable will fire again.
        return
    }
    lassign $body kind serial payload
    switch -- $kind {
        resp {
            set ::tcld::ctrl::pending_responses($serial) [list ok $payload]
        }
        err {
            set ::tcld::ctrl::pending_responses($serial) [list err $payload]
        }
        evt {
            ::tcld::ctrl::__handle_event__ $payload
        }
        bye {
            ::tcld::ctrl::__handle_disconnect__ "peer sent bye"
        }
        default {
            # protocol violation; close.
            ::tcld::ctrl::__handle_disconnect__ "unexpected kind: $kind"
        }
    }
}

# Non-blocking frame read. Returns the body or empty string if not yet
# available. Tracks partial reads in module-level state.
namespace eval ::tcld::ctrl {
    variable read_buf ""
    variable read_state length ;# length | body | trail
    variable read_len_str ""
    variable read_body_target 0
}

proc ::tcld::ctrl::__read_frame_nb__ {sock} {
    variable read_buf
    variable read_state
    variable read_len_str
    variable read_body_target
    while {1} {
        switch -- $read_state {
            length {
                set chunk [read $sock 1]
                if {$chunk eq ""} { return "" }
                if {$chunk eq "\n"} {
                    if {![string is integer -strict $read_len_str]} {
                        error "bad length prefix: $read_len_str"
                    }
                    set read_body_target $read_len_str
                    set read_buf ""
                    set read_state body
                } else {
                    append read_len_str $chunk
                }
            }
            body {
                set need [expr {$read_body_target - [string length $read_buf]}]
                if {$need == 0} {
                    set read_state trail
                } else {
                    set chunk [read $sock $need]
                    if {$chunk eq ""} { return "" }
                    append read_buf $chunk
                }
            }
            trail {
                set chunk [read $sock 1]
                if {$chunk eq ""} { return "" }
                if {$chunk ne "\n"} {
                    error "missing trailing newline"
                }
                set body $read_buf
                set read_buf ""
                set read_state length
                set read_len_str ""
                set read_body_target 0
                return $body
            }
        }
    }
}

proc ::tcld::ctrl::__handle_event__ {payload} {
    variable pending_events
    variable event_callbacks
    lappend pending_events $payload
    set kind [lindex $payload 0]
    if {[info exists event_callbacks($kind)]} {
        # Fire and forget; errors in callbacks shouldn't kill the
        # controller.
        catch {uplevel #0 $event_callbacks($kind) [list $payload]}
    }
    if {[info exists event_callbacks(all)]} {
        catch {uplevel #0 $event_callbacks(all) [list $payload]}
    }
}

proc ::tcld::ctrl::__handle_disconnect__ {reason} {
    variable state
    variable peer_sock
    set state closing
    if {$peer_sock ne ""} {
        catch {close $peer_sock}
        set peer_sock ""
    }
    set state idle
    # Wake any waiting `send` calls so they don't hang.
    set ::tcld::ctrl::__disconnected__ 1
}

# Send a request, block for response.
proc ::tcld::ctrl::send {op args} {
    variable state
    variable peer_sock
    variable next_serial
    if {$state ne "open"} {
        error "not_connected: controller state is $state"
    }
    set serial $next_serial
    incr next_serial
    set payload [linsert $args 0 $op]
    ::tcld::ctrl::__send_frame__ $peer_sock req $serial $payload

    # Wait for response.
    set timeout_ms 30000
    set deadline [expr {[clock milliseconds] + $timeout_ms}]
    while {![info exists ::tcld::ctrl::pending_responses($serial)]} {
        if {$state ne "open"} {
            error "not_connected: state changed to $state during send"
        }
        set remaining [expr {$deadline - [clock milliseconds]}]
        if {$remaining <= 0} {
            error "response timeout for op=$op serial=$serial"
        }
        # Tick on either a response arrival OR a tiny timeout.
        set timer [after [expr {min($remaining, 100)}] [list set ::tcld::ctrl::__tick__ 1]]
        vwait ::tcld::ctrl::__tick__
        after cancel $timer
    }
    set entry $::tcld::ctrl::pending_responses($serial)
    unset ::tcld::ctrl::pending_responses($serial)
    lassign $entry status payload
    if {$status eq "err"} {
        array set err_dict $payload
        set code [expr {[info exists err_dict(code)] ? $err_dict(code) : "unknown"}]
        set message [expr {[info exists err_dict(message)] ? $err_dict(message) : "no message"}]
        error "$code: $message" "" [list TCLD $code]
    }
    return $payload
}

proc ::tcld::ctrl::drain_events {} {
    variable pending_events
    set ev $pending_events
    set pending_events {}
    return $ev
}

# Register a callback for events of a given kind. `all` matches every
# event. Replaces any previous registration for that kind.
proc ::tcld::ctrl::on_event {kind script} {
    variable event_callbacks
    set event_callbacks($kind) $script
}

# Block until the next event of any kind arrives, or timeout (ms).
proc ::tcld::ctrl::next_event {{timeout_ms 30000}} {
    variable pending_events
    set deadline [expr {[clock milliseconds] + $timeout_ms}]
    while {[llength $pending_events] == 0} {
        set remaining [expr {$deadline - [clock milliseconds]}]
        if {$remaining <= 0} {
            error "next_event timeout"
        }
        set timer [after [expr {min($remaining, 100)}] [list set ::tcld::ctrl::__tick__ 1]]
        vwait ::tcld::ctrl::__tick__
        after cancel $timer
    }
    set ev [lindex $pending_events 0]
    set pending_events [lrange $pending_events 1 end]
    return $ev
}

proc ::tcld::ctrl::close {} {
    variable state
    variable peer_sock
    variable listen_sock
    if {$peer_sock ne ""} {
        catch {::tcld::ctrl::__send_frame__ $peer_sock bye 0 ""}
        catch {::close $peer_sock}
        set peer_sock ""
    }
    if {$listen_sock ne ""} {
        catch {::close $listen_sock}
        set listen_sock ""
    }
    set state idle
    return ok
}
