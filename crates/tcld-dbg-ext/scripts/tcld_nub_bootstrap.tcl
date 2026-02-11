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

# Engine + wrappers + nub connect.
::tcld::dbg::engine_install
::tcld::dbg::install_command_wrappers
::tcld::dbg::nub_connect $env(TCLD_NUB_HOST) $env(TCLD_NUB_PORT)

# StartTarget wakes this top-level loop. Running the target only after the
# socket fileevent callback has unwound keeps the controller channel
# serviceable from command-trace boundaries during synchronous `source`.
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
    ::tcld::dbg::__nub__::run_target $path $stop_on_entry
}
