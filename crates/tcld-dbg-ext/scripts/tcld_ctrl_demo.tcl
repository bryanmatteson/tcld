#!/usr/bin/env tclsh9.0
#
# Reference Tcl frontend.
#
# Spawns a tclsh subprocess running the tcld nub bootstrap, sets a
# breakpoint on the first command of `demo`, and prints events as
# they arrive. Mirrors the Rust example's behaviour byte-for-byte.
#
# Usage:
#   tclsh9.0 tcld_ctrl_demo.tcl <user-script.tcl>

package require Tcl 9
set scripts_dir [file normalize [file dirname [info script]]]
set workspace_root [file normalize [file join $scripts_dir .. .. ..]]
source [file join $scripts_dir tcld_ctrl.tcl]
package require tcld_ctrl

set user_script [lindex $argv 0]
if {$user_script eq ""} {
    set user_script [file join $workspace_root crates tcld-debug-wire fixtures demo.tcl]
}
puts "user script: $user_script"

# Locate the dylib.
proc locate_dylib {workspace_root} {
    foreach profile {release debug} {
        foreach ext {dylib so} {
            set p [file join $workspace_root target $profile "libtcld_dbg.$ext"]
            if {[file exists $p]} { return $p }
        }
    }
    error "could not locate libtcld_dbg.dylib (run `cargo build` first)"
}
set dylib [locate_dylib $workspace_root]

# Locate tclsh — prefer Tcl 9 from Homebrew on macOS, fall back to PATH.
proc locate_tclsh {} {
    if {[info exists ::env(TCLD_TCLSH)]} { return $::env(TCLD_TCLSH) }
    set candidate /opt/homebrew/Cellar/tcl-tk/9.0.3/bin/tclsh9.0
    if {[file executable $candidate]} { return $candidate }
    return tclsh
}
set tclsh_bin [locate_tclsh]

set port [::tcld::ctrl::listen 0]
puts "listening on 127.0.0.1:$port"

set bootstrap [file join [file dirname [info script]] tcld_nub_bootstrap.tcl]

# Tcl's exec inherits the parent environment.
set ::env(TCLD_DBG_DYLIB) $dylib
set ::env(TCLD_NUB_HOST) 127.0.0.1
set ::env(TCLD_NUB_PORT) $port
set nub_pid [exec $tclsh_bin $bootstrap &]
puts "spawned nub (pid $nub_pid)"

# Accept and handshake.
set caps [::tcld::ctrl::accept]
puts "nub connected; capabilities: $caps"

::tcld::ctrl::send start_target $user_script 0
set loaded 0
set deadline [expr {[clock milliseconds] + 5000}]
while {[clock milliseconds] < $deadline} {
    if {[catch {::tcld::ctrl::next_event 500} ev]} continue
    puts "event: $ev"
    if {[lindex $ev 0] eq "target_completed"} {
        set loaded 1
        break
    }
}
if {!$loaded} { error "target did not complete loading" }

set proc_loc [::tcld::ctrl::send proc_location demo]
puts "proc demo: $proc_loc"

# Extract first_offset.
set first_offset 0
set parts $proc_loc
foreach {k v} $parts {
    if {$k eq "first_offset"} { set first_offset $v }
}
puts "first_offset: $first_offset"

set bp [::tcld::ctrl::send bp_add demo $first_offset 1]
puts "breakpoint added: $bp"
::tcld::ctrl::send eval_in_frame 0 {after 0 demo}

puts "waiting for paused event..."
set got_paused 0
set deadline [expr {[clock milliseconds] + 5000}]
while {[clock milliseconds] < $deadline} {
    if {[catch {::tcld::ctrl::next_event 500} ev]} {
        # timeout, retry
        continue
    }
    puts "event: $ev"
    set ev_kind [lindex $ev 0]
    if {$ev_kind eq "paused"} {
        set got_paused 1
        break
    }
}

if {$got_paused} {
    puts "PARITY OK: nub paused on bp"
    set vars [::tcld::ctrl::send get_variables 1]
    puts "vars at frame 1: $vars"
} else {
    puts stderr "WARN: no paused event within 5s"
}

::tcld::ctrl::close
catch {exec kill $nub_pid}
exit 0
