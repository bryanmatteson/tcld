//! Phase 4-E verifier: representative Tier 0–3 operations exercised
//! over the wire and asserted to behave identically to in-process.
//!
//! This is the load-bearing claim of Tier 4: every engine surface
//! that works in-process MUST work the same way over the wire. The
//! list of cases isn't exhaustive (running the entire suite in two
//! modes would be expensive); we cherry-pick one representative test
//! per Tier 0–3 surface area and assert identical observable behavior.

#![cfg(feature = "integration_tclsh")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tcld_debug_wire::{BreakpointTarget, Controller, Op, WireEvent};

const SCRIPTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts");

fn tclsh_path() -> PathBuf {
    if let Ok(p) = std::env::var("TCLD_TCLSH") {
        return PathBuf::from(p);
    }
    let candidate = PathBuf::from("/opt/homebrew/Cellar/tcl-tk/9.0.3/bin/tclsh9.0");
    if candidate.exists() {
        return candidate;
    }
    PathBuf::from("tclsh")
}

fn dylib_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.join("../..");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            }
        })
        .unwrap_or_else(|_| workspace.join("target"));
    // The integration workflow builds the cdylib before starting this test.
    // Prefer deps in case Cargo leaves an older uplifted copy in <profile>/.
    for profile in ["debug", "release"] {
        for dir in [
            target_dir.join(profile).join("deps"),
            target_dir.join(profile),
        ] {
            for ext in ["dylib", "so"] {
                let p = dir.join(format!("libtcld_dbg.{ext}"));
                if p.exists() {
                    return p;
                }
            }
        }
    }
    panic!("could not locate libtcld_dbg.dylib (searched debug|release, incl. deps/)");
}

/// A collision-free suffix for the per-test temp scripts.
///
/// These tests run in parallel (C4 removed `--test-threads=1`), all as
/// threads of one process. `std::process::id()` is therefore identical
/// across them, so a `pid ^ nanos` name is unique only if the clock
/// actually advances between two `Harness::new` calls — and under
/// parallel load on a coarse-granularity clock it doesn't: two tests
/// land on the same nanos, produce the same filename, and write
/// different bodies (`proc demo …` vs `proc rec …`) to the same path.
/// Each test's nub then sources whichever write won the race, so a
/// controller ends up driving a nub that defined someone else's procs
/// ("proc body not registered: demo"). A monotonic per-process counter
/// makes the suffix unique regardless of clock resolution; the pid keeps
/// it distinct across concurrently-running test binaries.
fn unique_tag() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    format!("{}_{}_{:x}", std::process::id(), seq, nanos)
}

struct Harness {
    ctrl: Controller,
    _nub: tcld_debug_wire::NubChild,
    _user_script: PathBuf,
    entry_invocation: String,
}

impl Harness {
    /// Spawn a Ready v2 nub and load a target that only defines `body`.
    /// The invocation is scheduled later by [`Harness::go`], after each
    /// test has staged its breakpoints, so no wall-clock delay can race setup.
    fn new(body: &str, entry_invocation: &str) -> Self {
        let (mut ctrl, _port) = Controller::listen("127.0.0.1:0".parse().unwrap()).expect("listen");
        let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
        let dylib = dylib_path();
        let user = std::env::temp_dir().join(format!("tcld_parity_{}.tcl", unique_tag()));
        let script = format!("{body}\n");
        std::fs::write(&user, script).expect("write user script");
        let nub = ctrl
            .spawn_nub(&tclsh_path(), &bootstrap, &dylib)
            .expect("spawn_nub");
        ctrl.accept().expect("accept");
        ctrl.send(Op::StartTarget {
            path: user.to_string_lossy().into_owned(),
            stop_on_entry: false,
        })
        .expect("start target");
        let mut h = Harness {
            ctrl,
            _nub: nub,
            _user_script: user,
            entry_invocation: entry_invocation.to_string(),
        };
        h.wait_for_completed(Duration::from_secs(30))
            .expect("target definitions complete");
        h
    }

    /// Schedule the deterministic entry point after breakpoints are armed.
    fn go(&mut self) {
        self.ctrl
            .send(Op::EvalInFrame {
                level: 0,
                script: format!("after 0 {{{}}}", self.entry_invocation),
            })
            .expect("schedule entry invocation");
    }

    fn wait_for_completed(&mut self, deadline: Duration) -> Option<WireEvent> {
        let stop = Instant::now() + deadline;
        while Instant::now() < stop {
            for event in self.ctrl.drain_events() {
                if matches!(event, WireEvent::TargetCompleted { .. }) {
                    return Some(event);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    fn wait_for_paused(&mut self, deadline: Duration) -> Option<WireEvent> {
        let stop = Instant::now() + deadline;
        while Instant::now() < stop {
            for ev in self.ctrl.drain_events() {
                if matches!(ev, WireEvent::Paused { .. }) {
                    return Some(ev);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.ctrl.close();
        let _ = std::fs::remove_file(&self._user_script);
    }
}

// ── Tier 0: line breakpoint at proc body offset ──────────────────────

#[test]
fn parity_line_bp_pauses_proc_via_wire() {
    let mut h = Harness::new("proc demo {} { set a 1\nset b 2 }", "demo");
    let loc = h
        .ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .expect("proc_location");
    let off = parse_first_offset(loc.payload()).unwrap();
    h.ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: off,
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");
    h.go();
    let evt = h.wait_for_paused(Duration::from_secs(30));
    assert!(evt.is_some(), "expected Paused event");
}

// ── Tier 1: conditional breakpoint over the wire ─────────────────────

#[test]
fn parity_conditional_bp_via_wire() {
    // Bp on `set b ...` with condition `$a == 1` — should fire (a was
    // set to 1 by the previous command). Uses straight-line proc so
    // each command is wrapper-instrumented.
    let mut h = Harness::new("proc demo {} { set a 1\nset b 2\nset c 3 }", "demo");
    let loc = h
        .ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .unwrap();
    let off = parse_first_offset(loc.payload()).unwrap();
    h.ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: off + 8, // second command (`set b 2`)
                len: 1,
            },
            condition: Some("$a == 1".into()),
        })
        .expect("bp_add with condition");
    h.go();
    let evt = h.wait_for_paused(Duration::from_secs(30));
    assert!(evt.is_some(), "expected Paused event from conditional bp");
}

// ── Tier 2: get_variables + get_var ──────────────────────────────────

#[test]
fn parity_get_variables_via_wire_after_pause() {
    let mut h = Harness::new("proc demo {} { set a 1\nset b two\nset c {three} }", "demo");
    let loc = h
        .ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .unwrap();
    let off = parse_first_offset(loc.payload()).unwrap();
    h.ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: off + 8, // pause on the second command (set b ...)
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");
    h.go();
    let _ = h
        .wait_for_paused(Duration::from_secs(30))
        .expect("paused before get_variables");
    // Inside the paused frame, `a` is set; `b`/`c` may not be yet.
    // Frame depth at pause: 1 (inside demo's body).
    let vars = h
        .ctrl
        .send(Op::GetVariables { level: 1 })
        .expect("get_variables");
    assert!(
        vars.payload().contains("a s"),
        "expected `a s` in vars: {}",
        vars.payload()
    );
    let a_val = h
        .ctrl
        .send(Op::GetVar {
            level: 1,
            name: "a".into(),
            maxlen: None,
        })
        .expect("get_var");
    assert_eq!(a_val.payload(), "1");
}

// ── Tier 2: bp_export round-trips over the wire ──────────────────────

#[test]
fn parity_bp_export_via_wire() {
    let mut h = Harness::new("proc demo {} { set a 1 }", "");
    let loc = h
        .ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .unwrap();
    let off = parse_first_offset(loc.payload()).unwrap();
    h.ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: off,
                len: 1,
            },
            condition: Some("$a > 0".into()),
        })
        .expect("bp_add");
    let exp = h.ctrl.send(Op::BpExport).expect("bp_export");
    let parse_record = |payload: &str| {
        let rows = tcld_debug_wire::tcl_list::tcl_list_parse(payload).expect("breakpoint list");
        assert_eq!(rows.len(), 1, "expected one breakpoint: {payload}");
        let fields =
            tcld_debug_wire::tcl_list::tcl_list_parse(&rows[0]).expect("breakpoint record");
        assert_eq!(fields.len() % 2, 0, "expected a dictionary: {payload}");
        fields
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let exported = parse_record(exp.payload());
    // Wire persistence retains the logical procedure target rather than its
    // transient core line-breakpoint binding.
    assert_eq!(exported["kind"], "proc_command");
    assert_eq!(exported["qname"], "demo");
    assert_eq!(exported["offset"], off.to_string());
    assert_eq!(exported["len"], "1");
    assert_eq!(exported["condition"], "$a > 0");
    h.ctrl
        .send(Op::BpImport {
            serialized: exp.payload().into(),
        })
        .expect("bp_import");
    let round_trip = h.ctrl.send(Op::BpExport).expect("bp_export after import");
    assert_eq!(parse_record(round_trip.payload()), exported);
}

// ── Tier 3: watch expressions evaluated at pause ─────────────────────

#[test]
fn parity_watch_evaluate_via_wire() {
    let mut h = Harness::new("set ::counter 7\nproc demo {} { set a 1 }", "demo");
    h.ctrl
        .send(Op::WatchAdd {
            name: "doubled".into(),
            expr: "$::counter * 2".into(),
        })
        .expect("watch_add");
    let loc = h
        .ctrl
        .send(Op::ProcLocation {
            qname: "demo".into(),
        })
        .unwrap();
    let off = parse_first_offset(loc.payload()).unwrap();
    h.ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: off,
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");
    h.go();
    let _ = h.wait_for_paused(Duration::from_secs(30)).expect("paused");
    let result = h
        .ctrl
        .send(Op::WatchEvaluate { level: 0 })
        .expect("watch_evaluate");
    assert!(
        result.payload().contains("value {14}"),
        "expected doubled=14 in: {}",
        result.payload()
    );
}

// ── Tier 3: ignore_count gates pauses ────────────────────────────────

#[test]
fn parity_ignore_count_via_wire_skips_first_n_hits() {
    // bp on a recursive proc so it's hit multiple times. The wrapper
    // instruments each call's body. With ignore_count=2, the third
    // hit pauses.
    let mut h = Harness::new(
        "proc rec {n} { if {$n > 0} { rec [expr {$n - 1}] } }",
        "rec 5",
    );
    let loc = h
        .ctrl
        .send(Op::ProcLocation {
            qname: "rec".into(),
        })
        .unwrap();
    let off = parse_first_offset(loc.payload()).unwrap();
    let bp = h
        .ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "rec".into(),
                offset: off,
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");
    h.ctrl
        .send(Op::BpSetIgnore {
            id: bp.payload().to_string(),
            count: 2,
        })
        .expect("set_ignore");
    h.go();
    let _ = h
        .wait_for_paused(Duration::from_secs(30))
        .expect("paused after ignore");
    let listed = h.ctrl.send(Op::BpList).expect("bp_list");
    assert!(listed.payload().contains("ignore 2"));
}

// ── Tier 4 / C2: step_over and step_out reach a second pause ─────────
//
// The same scenario is asserted in three modes so all of them verify
// identical behavior:
//
//   1. in-process  — steps issued via the `::tcld::dbg::step_*` Tcl
//      commands from a Paused-event subscriber (EmitOnly strategy)
//   2. remote-rust — the Rust `Controller` drives a spawned tclsh nub
//      over the wire
//   3. remote-tcl  — the pure-Tcl controller (`tcld_ctrl.tcl`) drives
//      an in-process nub that suspends in its synchronous pause loop
//      (Vwait strategy + wire attached — the production nub shape)
//
// Scenario: breakpoint on the first statement of `inner`, which is
// called by `outer`. `step_over` must produce a second Paused at the
// next statement of `inner` (same frame level); `step_out` must
// produce a second Paused after `inner` returns, at the next
// statement of `outer` (caller level).

const STEP_FIXTURE_PROCS: &str =
    "proc inner {} { set x 1\nset y 2\nset z 3 }\nproc outer {} { inner\nset mid 1\nset done 2 }\n";

fn assert_paused_in(evt: &WireEvent, expected_block: &str) {
    let WireEvent::Paused { location, .. } = evt else {
        panic!("expected Paused event, got: {}", evt.to_payload());
    };
    assert!(
        location.block_id.contains(expected_block),
        "expected pause in {expected_block}, got location: {location:?}"
    );
}

fn step_scenario_remote_rust(step: Op, expected_block: &str) {
    let mut h = Harness::new(STEP_FIXTURE_PROCS, "outer");
    let loc = h
        .ctrl
        .send(Op::ProcLocation {
            qname: "inner".into(),
        })
        .expect("proc_location");
    let off = parse_first_offset(loc.payload()).unwrap();
    h.ctrl
        .send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "inner".into(),
                offset: off,
                len: 1,
            },
            condition: None,
        })
        .expect("bp_add");
    h.go();
    let first = h
        .wait_for_paused(Duration::from_secs(30))
        .expect("first pause at breakpoint");
    assert_paused_in(&first, "proc:inner");
    h.ctrl.send(step).expect("step op");
    let second = h
        .wait_for_paused(Duration::from_secs(30))
        .expect("second pause after step");
    assert_paused_in(&second, expected_block);
    // The nub must be genuinely stopped at the follow-up pause, not
    // merely have emitted an event while running on.
    let st = h.ctrl.send(Op::AppState).expect("app_state");
    assert_eq!(st.payload(), "stopped");
    let _ = h.ctrl.send(Op::Continue);
}

#[test]
fn parity_step_over_second_pause_remote_rust() {
    step_scenario_remote_rust(Op::StepOver, "proc:inner");
}

#[test]
fn parity_step_out_second_pause_remote_rust() {
    step_scenario_remote_rust(Op::StepOut, "proc:outer");
}

// ── in-process mode ──────────────────────────────────────────────────

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
        let interp = tcl_sys::Tcl_CreateInterp();
        assert!(!interp.is_null());
        assert_eq!(tcl_sys::Tcl_Init(interp), tcl_sys::TCL_OK, "Tcl_Init");
        interp
    }
}

fn eval(interp: *mut tcl_sys::Tcl_Interp, script: &str) -> Result<String, String> {
    let cstr = std::ffi::CString::new(script).map_err(|e| e.to_string())?;
    unsafe {
        let ret = tcl_sys::Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0);
        let result_obj = tcl_sys::Tcl_GetObjResult(interp);
        let mut len: tcl_sys::Tcl_Size = 0;
        let ptr = tcl_sys::Tcl_GetStringFromObj(result_obj, &mut len);
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len.max(0) as usize);
        let text = String::from_utf8_lossy(bytes).into_owned();
        if ret == tcl_sys::TCL_OK {
            Ok(text)
        } else {
            Err(text)
        }
    }
}

fn step_scenario_in_process(step_cmd: &'static str, expected_block: &str) {
    use tcld_debug_state::{BlockId, DebugEvent, DebugState};

    let interp = fresh_interp();
    unsafe {
        assert_eq!(Tcld_dbg_Init(interp), tcl_sys::TCL_OK, "Tcld_dbg_Init");
    }
    let state = Arc::new(Mutex::new(DebugState::new()));
    unsafe {
        tcld_dbg::step_engine::install(interp, state.clone()).expect("install");
        tcld_dbg::step_engine::set_pause_strategy(
            interp,
            tcld_dbg::step_engine::PauseStrategy::EmitOnly,
        );
        tcld_dbg::wrappers::install_command_wrappers(interp).expect("wrappers");
    }

    // Subscriber: on the first pause issue the step command, on any
    // later pause issue continue. Runs on the interp thread during
    // the trace handler's event dispatch — the same shape a poll-based
    // in-process host uses.
    let pauses: Arc<Mutex<Vec<DebugEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let interp_addr = interp as usize;
    {
        let pauses = pauses.clone();
        state
            .lock()
            .unwrap()
            .subscribe(Arc::new(move |event: DebugEvent| {
                if !matches!(event, DebugEvent::Paused { .. }) {
                    return;
                }
                let count = {
                    let mut v = pauses.lock().unwrap();
                    v.push(event);
                    v.len()
                };
                let script = if count == 1 {
                    format!("::tcld::dbg::{step_cmd}")
                } else {
                    "::tcld::dbg::continue".to_string()
                };
                let interp = interp_addr as *mut tcl_sys::Tcl_Interp;
                eval(interp, &script).expect("step command from subscriber");
            }));
    }

    eval(interp, STEP_FIXTURE_PROCS).expect("define fixture procs");
    let loc = eval(interp, "::tcld::dbg::proc_location inner").expect("proc_location");
    let off = parse_first_offset(&loc).expect("first_offset");
    eval(
        interp,
        &format!("::tcld::dbg::breakpoint_add inner {off} 1"),
    )
    .expect("breakpoint_add");
    eval(interp, "outer").expect("run outer");

    let pauses = pauses.lock().unwrap();
    assert!(
        pauses.len() >= 2,
        "expected a second pause after {step_cmd}; got {} pause(s)",
        pauses.len()
    );
    let DebugEvent::Paused { location, .. } = &pauses[1] else {
        unreachable!("subscriber only records Paused events");
    };
    assert_eq!(
        location.block(),
        &BlockId::ProcBody(Arc::from(expected_block)),
        "expected second pause in {expected_block}"
    );
    unsafe {
        let _lifecycle = TCL_LIFECYCLE.lock().unwrap();
        let _ = tcld_dbg::step_engine::uninstall(interp);
        tcl_sys::Tcl_DeleteInterp(interp);
        // This interp lives on the test-harness thread, which exits
        // when the test ends. Interp-owning threads must finalize
        // before exiting, or a later thread reusing this pthread_t
        // inherits a zombie notifier record and aborts in epoll_ctl
        // (see tcl-sys).
        tcl_sys::Tcl_FinalizeThread();
    }
}

#[test]
fn parity_step_over_second_pause_in_process() {
    step_scenario_in_process("step_over", "inner");
}

#[test]
fn parity_step_out_second_pause_in_process() {
    step_scenario_in_process("step_out", "outer");
}

// ── remote-tcl mode ──────────────────────────────────────────────────
//
// Both endpoints are real tclsh subprocesses: the controller sources
// the pure-Tcl `tcld_ctrl.tcl` package and drives the scenario; the
// nub runs the production `tcld_nub_bootstrap.tcl` (default Vwait
// strategy — pauses suspend in the nub's synchronous pause loop).
// Subprocesses rather than in-binary interp threads: repeated
// create/delete of event-loop-driving Tcl interps across joined
// threads in one process trips Tcl's per-thread notifier
// finalization ("Tcl_WaitForEvent: Notifier not initialized").

const TCLD_CTRL_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/tcld_ctrl.tcl");

fn step_scenario_remote_tcl(step_op: &'static str, expected_block: &'static str) {
    let tag = unique_tag();
    let ctrl_script_path = std::env::temp_dir().join(format!("tcld_parity_ctrl_{tag}.tcl"));
    let user_script_path = std::env::temp_dir().join(format!("tcld_parity_user_{tag}.tcl"));
    std::fs::write(&user_script_path, STEP_FIXTURE_PROCS).expect("write user script");
    std::fs::write(
        &ctrl_script_path,
        format!(
            r#"source {{{TCLD_CTRL_PATH}}}
proc wait_paused {{timeout_ms}} {{
    set deadline [expr {{[clock milliseconds] + $timeout_ms}}]
    while 1 {{
        set remaining [expr {{$deadline - [clock milliseconds]}}]
        if {{$remaining <= 0}} {{ error "no paused event" }}
        set ev [::tcld::ctrl::next_event $remaining]
        if {{[lindex $ev 0] eq "paused"}} {{ return $ev }}
    }}
}}
proc wait_completed {{timeout_ms}} {{
    set deadline [expr {{[clock milliseconds] + $timeout_ms}}]
    while 1 {{
        set remaining [expr {{$deadline - [clock milliseconds]}}]
        if {{$remaining <= 0}} {{ error "no target_completed event" }}
        set ev [::tcld::ctrl::next_event $remaining]
        if {{[lindex $ev 0] eq "target_completed"}} {{ return $ev }}
    }}
}}
proc fail {{msg}} {{ puts stderr $msg; exit 1 }}
puts [::tcld::ctrl::listen 0]
flush stdout
::tcld::ctrl::accept
::tcld::ctrl::send start_target {{{}}} 1
set gate [wait_paused 30000]
if {{[string first "paused" $gate] < 0}} {{ fail "no entry-gate pause: $gate" }}
::tcld::ctrl::send continue
wait_completed 30000
set loc [::tcld::ctrl::send proc_location inner]
set idx [lsearch $loc first_offset]
if {{$idx < 0}} {{ fail "no first_offset in: $loc" }}
set off [lindex $loc [expr {{$idx + 1}}]]
::tcld::ctrl::send bp_add inner $off 1
::tcld::ctrl::send eval_in_frame 0 {{after 0 outer}}
set first [wait_paused 30000]
if {{[string first "proc:inner" $first] < 0}} {{ fail "first pause not in inner: $first" }}
set r [::tcld::ctrl::send {step_op}]
if {{$r ne "ok"}} {{ fail "{step_op} responded: $r" }}
set second [wait_paused 30000]
if {{[string first "{expected_block}" $second] < 0}} {{
    fail "second pause not in {expected_block}: $second"
}}
set st [::tcld::ctrl::send app_state]
if {{$st ne "stopped"}} {{ fail "expected stopped at second pause, got: $st" }}
::tcld::ctrl::send continue
::tcld::ctrl::close
exit 0
"#,
            user_script_path.display()
        ),
    )
    .expect("write controller script");

    let tclsh = tclsh_path();
    let mut ctrl_child = std::process::Command::new(&tclsh)
        .arg(&ctrl_script_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tcl controller");

    // First stdout line is the listener port.
    let port = {
        use std::io::{BufRead, BufReader};
        let stdout = ctrl_child.stdout.take().expect("ctrl stdout");
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut line = String::new();
            let mut reader = BufReader::new(stdout);
            if reader.read_line(&mut line).is_ok() {
                let _ = tx.send(line.trim().to_string());
            }
        });
        rx.recv_timeout(Duration::from_secs(10))
            .expect("controller printed its port")
    };

    let bootstrap = PathBuf::from(SCRIPTS_DIR).join("tcld_nub_bootstrap.tcl");
    let mut nub_child = std::process::Command::new(&tclsh)
        .arg(&bootstrap)
        .env("TCLD_DBG_DYLIB", dylib_path())
        .env("TCLD_NUB_HOST", "127.0.0.1")
        .env("TCLD_NUB_PORT", &port)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn tcl nub");

    // The controller script exits 0 only when the whole scenario —
    // bp pause, step op ack, second pause in the expected block,
    // app_state stopped — checked out.
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = ctrl_child.try_wait().expect("try_wait ctrl") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = nub_child.kill();
    let _ = nub_child.wait();
    let _ = std::fs::remove_file(&ctrl_script_path);
    let _ = std::fs::remove_file(&user_script_path);
    let Some(status) = status else {
        let _ = ctrl_child.kill();
        let _ = ctrl_child.wait();
        panic!("tcl controller did not finish the {step_op} scenario in time");
    };
    if !status.success() {
        use std::io::Read;
        let mut err = String::new();
        if let Some(mut stderr) = ctrl_child.stderr.take() {
            let _ = stderr.read_to_string(&mut err);
        }
        panic!("tcl controller {step_op} scenario failed: {}", err.trim());
    }
}

#[test]
fn parity_step_over_second_pause_remote_tcl() {
    step_scenario_remote_tcl("step_over", "proc:inner");
}

#[test]
fn parity_step_out_second_pause_remote_tcl() {
    step_scenario_remote_tcl("step_out", "proc:outer");
}

fn parse_first_offset(payload: &str) -> Option<u64> {
    let parts: Vec<&str> = payload.split_whitespace().collect();
    for w in parts.windows(2) {
        if w[0] == "first_offset" {
            return w[1].parse().ok();
        }
    }
    None
}
