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

use std::collections::BTreeMap;
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

fn breakpoint_rows(payload: &str) -> Vec<BTreeMap<String, String>> {
    tcld_debug_wire::tcl_list_parse(payload)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| tcld_debug_wire::tcl_list_parse(&row).ok())
        .map(|tokens| {
            tokens
                .chunks_exact(2)
                .map(|pair| (pair[0].clone(), pair[1].clone()))
                .collect()
        })
        .collect()
}

fn breakpoint_row(payload: &str, id: &str) -> BTreeMap<String, String> {
    breakpoint_rows(payload)
        .into_iter()
        .find(|row| row.get("id").is_some_and(|candidate| candidate == id))
        .unwrap_or_else(|| panic!("breakpoint {id} missing from {payload}"))
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
fn spawn_nub(port: u16, pre_setup: String) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
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
            eval(interp, &pre_setup).expect("pre_setup");
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

    let (stop, handle) = spawn_nub(port, String::new());
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
    let (stop, handle) = spawn_nub(port, "proc demo {} { set a 1 }".into());
    ctrl.accept().expect("accept");
    let resp = ctrl.send(Op::ProcList).expect("proc_list");
    assert!(resp.payload().contains("demo"), "got {}", resp.payload());

    ctrl.close().expect("close");
    stop.store(true, Ordering::Release);
    handle.join().expect("join");
}

#[test]
fn logical_and_core_only_breakpoints_share_one_collision_free_external_id_space() {
    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let (stop, handle) = spawn_nub(port, String::new());
    ctrl.accept().expect("accept");

    let logical = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::FileLine {
                path: "/tmp/not-registered-yet.tcl".into(),
                line: 2,
            },
            condition: None,
        })
        .expect("pending logical breakpoint")
        .payload()
        .to_string();
    assert_eq!(logical, "bp0");
    let core_only = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Instruction { pc: 12 },
            condition: None,
        })
        .expect("core-only instruction breakpoint")
        .payload()
        .to_string();
    assert_eq!(core_only, "bp1");

    let listed = ctrl.send(Op::BpList).expect("collision-free list");
    let rows = breakpoint_rows(listed.payload());
    assert_eq!(
        rows.iter()
            .filter(|row| row.get("id") == Some(&logical))
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.get("id") == Some(&core_only))
            .count(),
        1
    );
    ctrl.send(Op::BpRemove {
        id: core_only.clone(),
    })
    .expect("remove core-only by external id");
    let after_remove = ctrl.send(Op::BpList).expect("list after remove");
    assert!(
        breakpoint_rows(after_remove.payload())
            .iter()
            .all(|row| row.get("id") != Some(&core_only))
    );

    ctrl.close().expect("close");
    stop.store(true, Ordering::Release);
    handle.join().expect("join");
}

#[test]
fn controller_breakpoint_id_surface_uses_only_ledger_ids() {
    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let (stop, handle) = spawn_nub(
        port,
        "proc demo {} {set a 1\nset b 2}; set ::monitored 0".into(),
    );
    ctrl.accept().expect("accept");

    // Reserve external bp0 with unresolved source intent. The first core-only
    // breakpoint is also raw bp0 internally, but must be exposed as bp1.
    let pending = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::FileLine {
                path: "/tmp/ledger-id-pending.tcl".into(),
                line: 2,
            },
            condition: None,
        })
        .expect("pending source add")
        .payload()
        .to_string();
    assert_eq!(pending, "bp0");
    let instruction = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Instruction { pc: 12 },
            condition: None,
        })
        .expect("instruction add")
        .payload()
        .to_string();
    assert_eq!(instruction, "bp1");
    let raw_instruction = ctrl
        .send(Op::EvalInFrame {
            level: 0,
            script: format!("set ::tcld::dbg::__nub__::passthrough_breakpoints({instruction})"),
        })
        .expect("inspect private core id");
    assert_eq!(raw_instruction.payload(), "bp0");

    let instruction_rows = ctrl
        .send(Op::InstructionBreakpoints { pc: Some(12) })
        .expect("filtered instruction list");
    assert_eq!(
        breakpoint_rows(instruction_rows.payload())[0].get("id"),
        Some(&instruction)
    );
    ctrl.send(Op::BpDisable {
        id: pending.clone(),
    })
    .expect("disable external bp0 logical intent");
    let instruction_row = breakpoint_row(
        ctrl.send(Op::BpList)
            .expect("list collision state")
            .payload(),
        &instruction,
    );
    assert_eq!(
        instruction_row.get("enabled").map(String::as_str),
        Some("1"),
        "raw core bp0 must not be reachable through external bp0"
    );
    assert!(
        ctrl.send(Op::BpRemove { id: "bp99".into() }).is_err(),
        "unknown external ids must not fall through to the core namespace"
    );

    // Instruction relocation replaces its core binding but retains bp1.
    ctrl.send(Op::BpMove {
        id: instruction.clone(),
        target: BreakpointTarget::Instruction { pc: 24 },
    })
    .expect("move instruction through external id");
    let moved_instruction = ctrl
        .send(Op::InstructionBreakpoints { pc: Some(24) })
        .expect("filter moved instruction");
    assert_eq!(
        breakpoint_rows(moved_instruction.payload())[0].get("id"),
        Some(&instruction)
    );

    // Watchpoint creation, filtered lookup, mutations, and pause events all
    // use its allocated passthrough ledger id.
    let watchpoint = ctrl
        .send(Op::WpAdd {
            name: "::monitored".into(),
            ops: "write".into(),
        })
        .expect("watchpoint add")
        .payload()
        .to_string();
    assert_eq!(watchpoint, "bp2");
    let var_rows = ctrl
        .send(Op::VarBreakpoints {
            name: Some("::monitored".into()),
        })
        .expect("filtered watchpoints");
    assert_eq!(
        breakpoint_rows(var_rows.payload())[0].get("id"),
        Some(&watchpoint)
    );
    ctrl.send(Op::BpDisable {
        id: watchpoint.clone(),
    })
    .expect("disable watchpoint");
    ctrl.send(Op::BpEnable {
        id: watchpoint.clone(),
    })
    .expect("enable watchpoint");
    ctrl.send(Op::BpSetIgnore {
        id: watchpoint.clone(),
        count: 2,
    })
    .expect("ignore watchpoint");
    let mutated_var = breakpoint_row(
        ctrl.send(Op::VarBreakpoints {
            name: Some("::monitored".into()),
        })
        .expect("watchpoint mutation query")
        .payload(),
        &watchpoint,
    );
    assert_eq!(mutated_var.get("enabled").map(String::as_str), Some("1"));
    assert_eq!(mutated_var.get("ignore").map(String::as_str), Some("2"));
    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "set ::monitored 1".into(),
    })
    .expect("trigger watchpoint");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_watchpoint = false;
    let mut observed_events = Vec::new();
    while Instant::now() < deadline && !saw_watchpoint {
        for event in ctrl.drain_events() {
            observed_events.push(format!("{event:?}"));
            if let WireEvent::Paused {
                breakpoint,
                reason: tcld_debug_wire::PauseReason::Watchpoint,
                ..
            } = event
            {
                assert_eq!(breakpoint.as_deref(), Some(watchpoint.as_str()));
                saw_watchpoint = true;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        saw_watchpoint,
        "watchpoint pause must carry its ledger id; observed {observed_events:?}"
    );

    // Source-filter queries, validation, and move retain the logical id while
    // requested intent and generation-specific binding change underneath.
    let source = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: 0,
                len: 1,
            },
            condition: None,
        })
        .expect("source add")
        .payload()
        .to_string();
    assert_eq!(source, "bp3");
    let line_rows = ctrl
        .send(Op::LineBreakpoints {
            qname: Some("demo".into()),
            offset: Some(0),
        })
        .expect("filtered source list");
    assert_eq!(
        breakpoint_rows(line_rows.payload())[0].get("id"),
        Some(&source)
    );
    ctrl.send(Op::BpMove {
        id: source.clone(),
        target: BreakpointTarget::Source {
            qname: "demo".into(),
            offset: 8,
            len: 1,
        },
    })
    .expect("move logical source breakpoint");
    let moved_source = breakpoint_row(
        ctrl.send(Op::LineBreakpoints {
            qname: Some("demo".into()),
            offset: Some(8),
        })
        .expect("filter moved source")
        .payload(),
        &source,
    );
    assert_eq!(moved_source.get("offset").map(String::as_str), Some("8"));
    let validation = ctrl
        .send(Op::BpValidate {
            qname: "demo".into(),
        })
        .expect("validate source bindings");
    assert_eq!(
        breakpoint_rows(validation.payload())[0].get("id"),
        Some(&source)
    );
    ctrl.send(Op::BpSetIgnore {
        id: source.clone(),
        count: 3,
    })
    .expect("set source ignore");

    // Condition replacement is the controller's documented remove+add path;
    // the replacement receives a fresh ledger id and every query agrees on it.
    ctrl.send(Op::BpRemove { id: source })
        .expect("remove before condition replacement");
    let conditioned = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: 8,
                len: 1,
            },
            condition: Some("1".into()),
        })
        .expect("condition replacement")
        .payload()
        .to_string();
    assert_eq!(conditioned, "bp4");
    let conditioned_row = breakpoint_row(
        ctrl.send(Op::LineBreakpoints {
            qname: Some("demo".into()),
            offset: Some(8),
        })
        .expect("conditioned source query")
        .payload(),
        &conditioned,
    );
    assert_eq!(
        conditioned_row.get("condition").map(String::as_str),
        Some("1")
    );

    // Persistence rows deliberately carry durable specs, not runtime ids.
    // Import allocates a new collision-free ledger namespace, and every
    // returned id must be present and removable through that namespace.
    let exported = ctrl.send(Op::BpExport).expect("export identity matrix");
    assert!(
        breakpoint_rows(exported.payload())
            .iter()
            .all(|row| !row.contains_key("id"))
    );
    let imported = ctrl
        .send(Op::BpImport {
            serialized: exported.payload().to_string(),
        })
        .expect("import identity matrix");
    let imported_ids = tcld_debug_wire::tcl_list_parse(imported.payload()).unwrap();
    assert_eq!(
        imported_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        imported_ids.len(),
        "imported controller ids must be unique"
    );
    let imported_list = ctrl.send(Op::BpList).expect("list imported ids");
    for id in &imported_ids {
        breakpoint_row(imported_list.payload(), id);
    }
    for id in imported_ids {
        ctrl.send(Op::BpRemove { id })
            .expect("remove imported external id");
    }
    assert!(breakpoint_rows(ctrl.send(Op::BpList).expect("empty final list").payload()).is_empty());

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
        "proc demo {} { set a 1\nset b 2\nset c 3 }; after 250 demo".into(),
    );
    ctrl.accept().expect("accept");

    let loc = ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .expect("proc_location");
    let first_offset =
        parse_first_offset(loc.payload()).unwrap_or_else(|| panic!("{}", loc.payload()));
    let bp = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: first_offset,
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");
    let logical_id = bp.payload().to_string();
    assert_eq!(logical_id, "bp0");
    let mapping = ctrl
        .send(Op::EvalInFrame {
            level: 0,
            script: format!(
                "set record $::tcld::dbg::__nub__::pending_breakpoints({logical_id}); \
                 ::tcld::dbg::__nub__::logical_id_for_core [dict get $record real_id]"
            ),
        })
        .expect("inspect logical mapping");
    assert_eq!(mapping.payload(), logical_id);
    // Drain events; expect at least one Paused.
    let mut saw_paused = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !saw_paused {
        for ev in ctrl.drain_events() {
            if let WireEvent::Paused { breakpoint, .. } = ev {
                assert_eq!(breakpoint.as_deref(), Some(logical_id.as_str()));
                saw_paused = true;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(saw_paused, "expected at least one Paused event");

    let first_list = ctrl.send(Op::BpList).expect("first bp_list");
    let first_row = breakpoint_row(first_list.payload(), &logical_id);
    assert_eq!(first_row.get("pending").map(String::as_str), Some("0"));
    assert_eq!(first_row.get("hits").map(String::as_str), Some("1"));
    let first_generation: u64 = first_row["resolution_generation"].parse().unwrap();

    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "proc demo {} { set a 1\nset b 2\nset c 3 }; demo".into(),
    })
    .expect("redefine and invoke proc");
    let second_list = ctrl.send(Op::BpList).expect("second bp_list");
    let second_row = breakpoint_row(second_list.payload(), &logical_id);
    assert_eq!(second_row.get("pending").map(String::as_str), Some("0"));
    assert_eq!(second_row.get("hits").map(String::as_str), Some("2"));
    assert!(second_row["resolution_generation"].parse::<u64>().unwrap() > first_generation);

    ctrl.send(Op::BpRemove {
        id: logical_id.clone(),
    })
    .expect("remove logical breakpoint");
    let conditioned = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: first_offset,
                len: 1,
            },
            condition: Some("$does_not_exist > 0".into()),
        })
        .expect("conditional logical breakpoint")
        .payload()
        .to_string();
    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "demo".into(),
    })
    .expect("invoke condition error");
    let condition_deadline = Instant::now() + Duration::from_secs(2);
    let mut mapped_condition_error = false;
    while Instant::now() < condition_deadline && !mapped_condition_error {
        for event in ctrl.drain_events() {
            if let WireEvent::ConditionError { breakpoint, .. } = event {
                assert_eq!(breakpoint, conditioned);
                mapped_condition_error = true;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        mapped_condition_error,
        "logical condition error id was not forwarded"
    );

    ctrl.close().expect("close");
    stop.store(true, Ordering::Release);
    handle.join().expect("join");
}

#[test]
fn file_line_intent_rebinds_by_line_across_changed_source_generation() {
    let root = std::env::temp_dir().join(format!(
        "tcld-logical-file-breakpoint-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("reload.tcl");
    std::fs::write(&path, "set first 1\nset target 2\n").unwrap();
    let path = path.canonicalize().unwrap();

    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let (stop, handle) = spawn_nub(port, format!("source {{{}}}", path.display()));
    ctrl.accept().expect("accept");
    let logical_id = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::FileLine {
                path: path.display().to_string(),
                line: 2,
            },
            condition: Some("1".into()),
        })
        .expect("file-line add")
        .payload()
        .to_string();
    let initial_list = ctrl.send(Op::BpList).expect("initial list");
    let initial = breakpoint_row(initial_list.payload(), &logical_id);
    assert_eq!(initial.get("line").map(String::as_str), Some("2"));
    assert_eq!(initial.get("pending").map(String::as_str), Some("0"));
    let initial_offset = initial["resolved_offset"].parse::<u64>().unwrap();
    let initial_generation = initial["resolution_generation"].parse::<u64>().unwrap();

    ctrl.send(Op::BpDisable {
        id: logical_id.clone(),
    })
    .expect("disable logical intent");
    ctrl.send(Op::BpSetIgnore {
        id: logical_id.clone(),
        count: 3,
    })
    .expect("set logical ignore");
    std::fs::write(
        &path,
        "set first_with_a_much_longer_name 123456789\nset target 3\n",
    )
    .unwrap();
    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: format!("source {{{}}}", path.display()),
    })
    .expect("reload changed source");

    let rebound_list = ctrl.send(Op::BpList).expect("rebound list");
    let rebound = breakpoint_row(rebound_list.payload(), &logical_id);
    assert_eq!(rebound.get("line").map(String::as_str), Some("2"));
    assert_eq!(rebound.get("pending").map(String::as_str), Some("0"));
    assert_eq!(rebound.get("enabled").map(String::as_str), Some("0"));
    assert_eq!(rebound.get("ignore").map(String::as_str), Some("3"));
    assert_eq!(rebound.get("condition").map(String::as_str), Some("1"));
    assert_ne!(
        rebound["resolved_offset"].parse::<u64>().unwrap(),
        initial_offset,
        "line intent must resolve against the changed body, not reuse a stale offset"
    );
    assert!(rebound["resolution_generation"].parse::<u64>().unwrap() > initial_generation);

    let exported = ctrl.send(Op::BpExport).expect("export logical intent");
    let exported_rows = breakpoint_rows(exported.payload());
    let durable = exported_rows
        .iter()
        .find(|row| row.get("kind").is_some_and(|kind| kind == "file_line"))
        .expect("durable file-line export");
    assert_eq!(durable.get("path"), Some(&path.display().to_string()));
    assert_eq!(durable.get("line").map(String::as_str), Some("2"));
    assert!(!durable.contains_key("offset"));

    ctrl.send(Op::BpRemove {
        id: logical_id.clone(),
    })
    .expect("remove logical intent");
    let after_remove = ctrl.send(Op::BpList).expect("list after remove");
    assert!(
        breakpoint_rows(after_remove.payload())
            .iter()
            .all(|row| row.get("id") != Some(&logical_id))
    );

    ctrl.close().expect("close");
    stop.store(true, Ordering::Release);
    handle.join().expect("join");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_proc_command_anchor_becomes_pending_without_stale_offset_reuse() {
    let (mut ctrl, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
    let (stop, handle) = spawn_nub(port, "proc unstable {} {set a 1\nset b 2}".into());
    ctrl.accept().expect("accept");
    let logical_id = ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "unstable".into(),
                offset: 8,
                len: 1,
            },
            condition: None,
        })
        .expect("proc command add")
        .payload()
        .to_string();
    let initial_list = ctrl.send(Op::BpList).expect("initial list");
    let initial = breakpoint_row(initial_list.payload(), &logical_id);
    assert_eq!(initial.get("pending").map(String::as_str), Some("0"));
    let initial_generation = initial["resolution_generation"].parse::<u64>().unwrap();

    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "proc unstable {} {set a 1\nset changed 9}".into(),
    })
    .expect("redefine proc with different command at requested offset");
    let changed_list = ctrl.send(Op::BpList).expect("changed anchor list");
    let changed = breakpoint_row(changed_list.payload(), &logical_id);
    assert_eq!(changed.get("offset").map(String::as_str), Some("8"));
    assert_eq!(changed.get("pending").map(String::as_str), Some("1"));
    let reason = changed
        .get("pending_reason")
        .expect("changed-anchor generation reason");
    assert!(reason.contains("anchor changed"), "{reason}");
    assert!(reason.contains("generation"), "{reason}");
    assert!(reason.contains("revalidation"), "{reason}");
    assert!(!changed.contains_key("resolved_offset"));

    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "proc unstable {} {set only 1}".into(),
    })
    .expect("redefine proc without requested span");
    let missing_list = ctrl.send(Op::BpList).expect("missing span list");
    let missing = breakpoint_row(missing_list.payload(), &logical_id);
    assert_eq!(missing.get("offset").map(String::as_str), Some("8"));
    assert_eq!(missing.get("pending").map(String::as_str), Some("1"));
    let reason = missing
        .get("pending_reason")
        .expect("generation mismatch reason");
    assert!(reason.contains("generation"), "{reason}");
    assert!(reason.contains("revalidation"), "{reason}");
    assert!(!missing.contains_key("resolved_offset"));

    ctrl.send(Op::EvalInFrame {
        level: 0,
        script: "proc unstable {} {set a 1\nset b 2}".into(),
    })
    .expect("restore matching proc shape");
    let restored_list = ctrl.send(Op::BpList).expect("restored list");
    let restored = breakpoint_row(restored_list.payload(), &logical_id);
    assert_eq!(restored.get("offset").map(String::as_str), Some("8"));
    assert_eq!(
        restored.get("resolved_offset").map(String::as_str),
        Some("8")
    );
    assert_eq!(restored.get("pending").map(String::as_str), Some("0"));
    assert!(restored["resolution_generation"].parse::<u64>().unwrap() > initial_generation);

    let exported = ctrl.send(Op::BpExport).expect("export proc intent");
    let durable = breakpoint_rows(exported.payload())
        .into_iter()
        .find(|row| row.get("kind").is_some_and(|kind| kind == "proc_command"))
        .expect("durable proc-command export");
    assert_eq!(durable.get("qname").map(String::as_str), Some("unstable"));
    assert_eq!(durable.get("offset").map(String::as_str), Some("8"));
    assert_eq!(
        durable.get("anchor_text").map(String::as_str),
        Some("set b 2")
    );

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
