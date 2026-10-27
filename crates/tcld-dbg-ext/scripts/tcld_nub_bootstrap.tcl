# tcld_nub_bootstrap.tcl
#
# Loaded by spawned tclsh subprocesses to bring up the debug nub.
# Required env vars:
#   TCLD_DBG_DYLIB    — absolute path to libtcld_dbg.dylib
#   TCLD_NUB_HOST     — controller listener host
#   TCLD_NUB_PORT     — controller listener port

if {![info exists env(TCLD_DBG_DYLIB)]} {
    puts stderr "tcld_nub_bootstrap: TCLD_DBG_DYLIB not set"
    exit 2
}
if {![info exists env(TCLD_NUB_HOST)] || ![info exists env(TCLD_NUB_PORT)]} {
    puts stderr "tcld_nub_bootstrap: TCLD_NUB_HOST and TCLD_NUB_PORT must be set"
    exit 2
}

# Bring tcld_dbg into the interp. Tcl `load` capitalizes the first
# letter of the package name when looking up `<Pkgname>_Init`, but
# only when the supplied package name itself starts uppercase. We
# pass `Tcld_dbg` (capital T) so dlsym looks for `Tcld_dbg_Init`,
# which is what the cdylib actually exports.
load $env(TCLD_DBG_DYLIB) Tcld_dbg

# The controller reads the target's stdout through a pipe, where Tcl would
# otherwise buffer whole blocks; line buffering keeps output in step with
# the program, as it is in a terminal.
fconfigure stdout -buffering line

# Engine + wrappers + nub connect.
::tcld::dbg::engine_install
::tcld::dbg::install_command_wrappers
::tcld::dbg::nub_connect $env(TCLD_NUB_HOST) $env(TCLD_NUB_PORT)

# A disconnect (`bye`, or the controller going away) changes the nub's state
# without a target request. Wake the loop so tclsh exits on its own instead of
# waiting out the controller's kill grace period.
trace add variable ::tcld::dbg::__nub__::state write [list apply {{args} {
    incr ::tcld::dbg::__nub__::target_epoch
}}]

# StartTarget wakes this top-level loop. Running the target only after the
# socket fileevent callback has unwound keeps the controller channel
# serviceable from command-trace boundaries during synchronous `source`.
# The loop runs in its own frame so the target's global scope starts out
# holding only what tclsh itself defines.
apply {{bootstrap} {
    while {$::tcld::dbg::__nub__::state eq "open"} {
        if {$::tcld::dbg::__nub__::target_request eq {}} {
            vwait ::tcld::dbg::__nub__::target_epoch
        }
        if {$::tcld::dbg::__nub__::state ne "open"} break
        if {$::tcld::dbg::__nub__::target_wakeup ne {}} {
            after cancel $::tcld::dbg::__nub__::target_wakeup
            set ::tcld::dbg::__nub__::target_wakeup {}
        }
        set request $::tcld::dbg::__nub__::target_request
        set ::tcld::dbg::__nub__::target_request {}
        if {$request eq {}} continue
        lassign $request path stop_on_entry
        # tclsh named this bootstrap as the main script. A target run
        # directly by tclsh would see its own path, which the common
        # `[info script] eq $::argv0` main-script check relies on. A
        # controller that staged its own argv0 keeps it.
        if {$::argv0 eq $bootstrap} {
            set ::argv0 [file normalize $path]
        }
        ::tcld::dbg::__nub__::run_target $path $stop_on_entry
    }
}} [info script]
