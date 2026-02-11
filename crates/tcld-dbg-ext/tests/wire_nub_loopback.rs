//! Phase 4-B verifier: Rust controller talks to a nub-bearing
//! `Tcl_Interp` over loopback TCP, all in one Rust process.
//!
//! Two interps in one process is exactly the threading model Tcl
//! supports — each interp owns its own thread; we never call into
//! the same interp from two threads.
//!
//! After `nub_connect` returns the user script (or, in our tests, the
//! nub-driver thread) MUST run the Tcl event loop so `fileevent`
//! readables can fire. We accomplish that with a loop calling
//! `update` until a shared exit flag is set.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tcl_sys::{
    TCL_OK, Tcl_CreateInterp, Tcl_DeleteInterp, Tcl_EvalEx, Tcl_GetObjResult, Tcl_GetStringFromObj,
    Tcl_Init, Tcl_Size,
};
use tcld_dbg::step_engine;
use tcld_dbg::wrappers;
use tcld_debug_state::{DebugEvent, DebugState};
use tcld_debug_wire::{BreakpointTarget, Controller, ControllerState, Op, WireEvent};

unsafe extern "C" {
    fn Tcld_dbg_Init(interp: *mut tcl_sys::Tcl_Interp) -> std::os::raw::c_int;
}

/// Serializes interp creation and thread teardown across test threads.
/// Concurrent `Tcl_Init` calls race Tcl's process-global path/VFS
/// state: `file exists` on a perfectly good $tcl_library/init.tcl
/// transiently fails and Tcl_Init errors with "Cannot find a usable
/// init.tcl" (observed ~20% of parallel runs on Linux). Only the
/// create/teardown edges are serialized — test bodies still run in
/// parallel.
static TCL_LIFECYCLE: Mutex<()> = Mutex::new(());

fn fresh_interp() -> *mut tcl_sys::Tcl_Interp {
    let _lifecycle = TCL_LIFECYCLE.lock().unwrap();
    // Tcl_FindExecutable's contract is once per PROCESS, before other
    // Tcl calls.
    static FIND_EXECUTABLE: std::sync::Once = std::sync::Once::new();
    FIND_EXECUTABLE.call_once(|| unsafe { tcl_sys::Tcl_FindExecutable(std::ptr::null()) });
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

fn install_full_engine(interp: *mut tcl_sys::Tcl_Interp) -> (Arc<Mutex<DebugState>>, Arc<Spy>) {
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
        let helper_install = r#"
namespace eval ::tcld::dbg {}
proc ::tcld::dbg::__t2_get_variables__ {level} {
    set vars [uplevel #$level {info vars}]
    set out {}
    foreach n $vars {
        if {[uplevel #$level [list array exists $n]]} {
            lappend out [list $n a]
        } else {
            lappend out [list $n s]
        }
    }
    return $out
}
proc ::tcld::dbg::__t2_get_var__ {level name maxlen} {
    if {[uplevel #$level [list array exists $name]]} {
        set raw [uplevel #$level [list array get $name]]
    } else {
        set raw [uplevel #$level [list set $name]]
    }
    if {$maxlen > 0 && [string length $raw] > $maxlen} {
        return "[string range $raw 0 [expr {$maxlen - 1}]]..."
    }
    return $raw
}
proc ::tcld::dbg::__t2_set_var__ {level name value} {
    return [uplevel #$level [list set $name $value]]
}
"#;
        let cstr = std::ffi::CString::new(helper_install).unwrap();
        let rc = Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0);
        assert_eq!(rc, TCL_OK, "tier2 helper install");
    }
    (state, spy)
}

/// Spawn a nub-driving thread. Returns a JoinHandle and an
/// `AtomicBool` the test can flip to ask the nub thread to stop.
/// The thread owns the interp end-to-end — creates, drives, and
/// deletes — because Tcl async handlers MUST be torn down on the
/// thread that created them.
fn spawn_nub(port: u16, pre_setup: &'static str) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let handle = thread::spawn(move || {
        let interp = fresh_interp();
        let _ = install_full_engine(interp);
        eval(
            interp,
            &format!("::tcld::dbg::nub_connect 127.0.0.1 {port}"),
        )
        .expect("nub_connect");
        if !pre_setup.is_empty() {
            eval(interp, pre_setup).expect("pre_setup");
        }
        // Service the Tcl event loop until told to stop.
        let deadline = Instant::now() + Duration::from_secs(15);
        while !stop_clone.load(Ordering::Acquire) {
            if Instant::now() > deadline {
                break;
            }
            let _ = eval(interp, "update");
            thread::sleep(Duration::from_millis(5));
        }
        unsafe {
            let _lifecycle = TCL_LIFECYCLE.lock().unwrap();
            // Tcl_AsyncDelete must run on the thread that created the
            // handler; this is the same thread.
            let _ = step_engine::uninstall(interp);
            Tcl_DeleteInterp(interp);
            // Interp-owning threads must finalize before exiting, or a
            // later thread reusing this pthread_t inherits a zombie
            // notifier record and aborts in epoll_ctl (see tcl-sys).
            tcl_sys::Tcl_FinalizeThread();
        }
    });
    (stop, handle)
}

#[test]
fn rust_controller_handshakes_with_nub() {
    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    assert_eq!(ctrl.status(), ControllerState::Listening);

    let (stop, handle) = spawn_nub(port, "");
    let caps = ctrl.accept().expect("accept");
    tcld_debug_wire::validate_nub_tcl_abi(&caps).expect("matching compiled/runtime Tcl ABI");
    assert_eq!(ctrl.status(), ControllerState::Open);

    // Round-trip a no-op op to confirm bidirectional flow.
    let resp = ctrl.send(Op::AppState).expect("app_state");
    assert!(
        ["running", "dead", "stopped"].contains(&resp.payload()),
        "unexpected app_state payload: {}",
        resp.payload()
    );

    ctrl.close().expect("close");
    stop.store(true, Ordering::Release);
    handle.join().expect("join");
}

#[test]
fn rust_controller_proc_list_round_trips() {
    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let (stop, handle) = spawn_nub(port, "proc demo {} { set a 1 }");
    ctrl.accept().expect("accept");
    let resp = ctrl.send(Op::ProcList).expect("proc_list");
    assert!(resp.payload().contains("demo"), "got {}", resp.payload());

    ctrl.close().expect("close");
    stop.store(true, Ordering::Release);
    handle.join().expect("join");
}

#[test]
fn rust_controller_sends_bp_add_and_receives_paused_event() {
    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    // Schedule `demo` to run 250ms after handshake — by then the
    // controller will have sent bp_add, so the bp will be armed when
    // the timer fires.
    let (stop, handle) = spawn_nub(
        port,
        "proc demo {} { set a 1\nset b 2\nset c 3 }; after 250 demo",
    );
    ctrl.accept().expect("accept");

    let loc = ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .expect("proc_location");
    let first_offset =
        parse_first_offset(loc.payload()).unwrap_or_else(|| panic!("{}", loc.payload()));
    let _bp = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: first_offset,
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");

    // Drain events; expect at least one Paused.
    let mut saw_paused = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !saw_paused {
        for ev in ctrl.drain_events() {
            if matches!(ev, WireEvent::Paused { .. }) {
                saw_paused = true;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(saw_paused, "expected at least one Paused event");

    ctrl.close().expect("close");
    stop.store(true, Ordering::Release);
    handle.join().expect("join");
}

fn parse_first_offset(payload: &str) -> Option<u64> {
    let parts: Vec<&str> = payload.split_whitespace().collect();
    for window in parts.windows(2) {
        if window[0] == "first_offset" {
            return window[1].parse().ok();
        }
    }
    None
}
