//! Hybrid TEBC verification matrix (executable plan §9).

use std::path::{Path, PathBuf};

use tcld_core::bytecode::{
    AuxData, AuxDataType, AuxDataValue, ByteCode, ByteCodeInfo, CompiledLocal, DictUpdateInfo,
    ExceptionRange, ExceptionType, ForeachInfo, Header, JumptableInfo, LiteralObject,
    LiteralObjectType, NewForeachInfo, Version,
};
use tcld_core::inst_table::{DEFAULT_INSTRUCTION_SET, Opcode, TCL_9_INSTRUCTION_SET};
use tcld_core::source::SourceRef;
use tcld_vm::{HybridEngine, RuntimeMode, VmError, VmObj};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/tcld/tests/fixtures")
        .join(name)
}

fn hybrid() -> HybridEngine {
    let mut eng = HybridEngine::new(RuntimeMode::Hybrid).expect("engine");
    eng.bootstrap(Some("tcld-vm-test")).expect("bootstrap");
    eng
}

fn strict() -> HybridEngine {
    HybridEngine::new(RuntimeMode::VmStrict).expect("engine")
}

fn result_str(v: &VmObj) -> String {
    v.as_string().unwrap_or_else(|_| format!("{v:?}"))
}

#[test]
fn f1_hand_built_arith_jump() {
    // push 10; push 0; jumpFalse → skip; push 7; done
    // Actually: push 2; push 3; add; done → 5
    let mut eng = strict();
    let r = eng.run_tbc(arith_bc()).expect("f1");
    assert_eq!(r.as_i64().unwrap(), 5);
}

#[test]
fn f2_host_puts() {
    let mut eng = hybrid();
    // push "puts"; push "hello-from-hybrid"; invokeStk1 2; push ""; done
    // After puts, result is empty; we need something on stack for done.
    // Sequence: puts hello; pop; push ok; done
    let bc = puts_bc("hello-from-hybrid-f2");
    let r = eng.run_tbc(bc).expect("f2 puts");
    assert_eq!(result_str(&r), "ok");
}

#[test]
fn f3_local_set_append() {
    let mut eng = strict();
    // set result {}; append result a; append result b; load result; done
    // Using invoke of set/append + loadScalarStk
    let bc = set_append_bc();
    let r = eng.run_tbc(bc).expect("f3");
    assert_eq!(result_str(&r), "ab");
}

#[test]
fn f4_proc_tbc() {
    let path = fixture("proc.tbc");
    assert!(
        path.exists(),
        "required Pass fixture missing: {}",
        path.display()
    );
    let mut eng = hybrid();
    let r = eng.run_tbc_path(&path).expect("f4 proc.tbc");
    // list [a TEST] [b TEST] => {TEST : TEST} {TEST > TEST}
    let s = result_str(&r);
    assert!(s.contains("TEST : TEST"), "got {s}");
    assert!(s.contains("TEST > TEST"), "got {s}");
    assert_eq!(eng.vm_proc_count(), 2);
}

#[test]
fn f5_catch1_tbc() {
    let path = fixture("catch1.tbc");
    assert!(
        path.exists(),
        "required Pass fixture missing: {}",
        path.display()
    );
    let mut eng = hybrid();
    let r = eng.run_tbc_path(&path).expect("f5 catch1");
    assert_eq!(result_str(&r), "PASS");
}

#[test]
fn mirrored_catch_writes_vm_result_variable() {
    let result = hybrid()
        .run_tbc_path(&fixture("catch.tbc"))
        .expect("catch.tbc");
    assert_eq!(result_str(&result), "PASS");
}

#[test]
fn eval_stack_continue_routes_through_enclosing_vm_loop() {
    let result = hybrid()
        .run_tbc_path(&fixture("cont.tbc"))
        .expect("cont.tbc");
    assert_eq!(
        result_str(&result),
        "2.4..6...8....10.....12......14.......16........DONE"
    );
}

#[test]
fn catch_captures_return_break_and_continue_codes() {
    for expected in [2, 3, 4] {
        let mut eng = strict();
        let result = eng
            .run_tbc(catch_completion_bc(expected))
            .unwrap_or_else(|error| panic!("catch code {expected}: {error:?}"));
        assert_eq!(result.as_i64().unwrap(), expected as i64);
    }
    let result = hybrid()
        .run_tbc(catch_host_frame_custom_code_bc())
        .expect("catch custom HostFrame completion");
    assert_eq!(result.as_i64().unwrap(), 7);
}

#[test]
fn f6_break_tbc() {
    let path = fixture("break.tbc");
    assert!(
        path.exists(),
        "required Pass fixture missing: {}",
        path.display()
    );
    let mut eng = hybrid();
    let r = eng.run_tbc_path(&path).expect("f6 break");
    // Nested loops: outer 1..8, inner dots = outerCounter
    // result like 1.2..3...4....5.....6......7.......8........DONE
    let s = result_str(&r);
    assert!(s.ends_with("DONE"), "got {s}");
    assert!(s.starts_with('1'), "got {s}");
    assert!(s.contains("8"), "got {s}");
}

#[test]
fn f7_cont2_happy_path() {
    let path = fixture("cont2.tbc");
    assert!(
        path.exists(),
        "required Pass fixture missing: {}",
        path.display()
    );
    let mut eng = hybrid();
    let r = eng.run_tbc_path(&path).expect("f7 cont2");
    let s = result_str(&r);
    // Even outer counters 2,4,...,16 with odd inner dots
    assert!(s.ends_with("DONE"), "got {s}");
    assert!(s.contains('2'), "got {s}");
}

#[test]
fn f9_hostvar_synthetic() {
    let mut eng = hybrid();
    // Host seed
    #[cfg(feature = "host")]
    {
        // Variable opcode needs LVT. Build: Variable on local0 with name from stack,
        // then store/load via LVT.
        // code:
        //   push "::f9x" ; variable LVT0 ; pop effect already
        //   push 42; storeScalar1 0; pop
        //   loadScalar1 0; done
        let bc = hostvar_bc();
        // Pre-set host var empty — Variable installs link; store writes host
        let r = eng.run_tbc(bc).expect("f9");
        assert_eq!(result_str(&r), "42");
    }
}

#[test]
fn nsupvar_links_namespace_variable_and_is_denied_before_dispatch_in_strict() {
    let mut code = vec![
        tcl86_opcode(Opcode::Push1),
        0, // namespace
        tcl86_opcode(Opcode::Push1),
        1, // eval
        tcl86_opcode(Opcode::Push1),
        2, // ::demo
        tcl86_opcode(Opcode::Push1),
        3, // set linked 3
        tcl86_opcode(Opcode::InvokeStk1),
        4,
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::Push1),
        2, // namespace object retained by Nsupvar
        tcl86_opcode(Opcode::Push1),
        4, // linked
        tcl86_opcode(Opcode::Nsupvar),
    ];
    code.extend(0u32.to_be_bytes());
    code.extend([
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::Push1),
        5, // 9
        tcl86_opcode(Opcode::StoreScalar1),
        0,
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::LoadScalar1),
        0,
        tcl86_opcode(Opcode::Done),
    ]);
    let mut bytecode = minimal_bc(
        code,
        vec![
            lit_str("namespace"),
            lit_str("eval"),
            lit_str("::demo"),
            lit_str("set linked 3"),
            lit_str("linked"),
            lit_str("9"),
        ],
    );
    bytecode.proc_num_locals = 1;
    bytecode.locals = local_slots(&["local_link"]);
    assert_eq!(hybrid().run_tbc(bytecode).unwrap().as_i64().unwrap(), 9);

    let mut host_array_code = vec![
        tcl86_opcode(Opcode::Push1),
        0, // namespace
        tcl86_opcode(Opcode::Push1),
        1, // eval
        tcl86_opcode(Opcode::Push1),
        2, // ::demo
        tcl86_opcode(Opcode::Push1),
        3, // initialize host array
        tcl86_opcode(Opcode::InvokeStk1),
        4,
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::Push1),
        2, // retained namespace
        tcl86_opcode(Opcode::Push1),
        4, // linked_array
        tcl86_opcode(Opcode::Nsupvar),
    ];
    host_array_code.extend(0u32.to_be_bytes());
    host_array_code.extend([
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::Push1),
        5, // HostFrame uses existing array and mutates it
        tcl86_opcode(Opcode::EvalStk),
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::Push1),
        6, // set
        tcl86_opcode(Opcode::Push1),
        7, // fully-qualified array element
        tcl86_opcode(Opcode::InvokeStk1),
        2,
        tcl86_opcode(Opcode::Done),
    ]);
    let mut host_array = minimal_bc(
        host_array_code,
        vec![
            lit_str("namespace"),
            lit_str("eval"),
            lit_str("::demo"),
            lit_str("array set linked_array {k 2 other 3}"),
            lit_str("linked_array"),
            lit_str("set prior $local_link(other); set local_link(k) 21"),
            lit_str("set"),
            lit_str("::demo::linked_array(k)"),
        ],
    );
    host_array.proc_num_locals = 1;
    host_array.locals = local_slots(&["local_link"]);
    assert_eq!(hybrid().run_tbc(host_array).unwrap().as_i64().unwrap(), 21);

    let mut denied = minimal_bc(
        {
            let mut bytes = vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::Nsupvar),
            ];
            bytes.extend(0u32.to_be_bytes());
            bytes.push(tcl86_opcode(Opcode::Done));
            bytes
        },
        vec![lit_str("::demo"), lit_str("linked")],
    );
    denied.proc_num_locals = 1;
    denied.locals = local_slots(&["local_link"]);
    assert!(matches!(
        strict().run_tbc(denied),
        Err(VmError::PolicyDenied {
            action: "host_opcode",
            ..
        })
    ));
}

#[test]
fn f9b_upvar_vm_to_vm() {
    // A relative level beyond the VM stack crosses the opaque host boundary.
    let mut eng = strict();
    let bc = upvar_toplevel_bc();
    let err = eng.run_tbc(bc).unwrap_err();
    assert!(
        matches!(err, VmError::CrossRuntimeUpvar),
        "expected CrossRuntimeUpvar, got {err:?}"
    );

    // Happy path: nested call
    let mut eng = strict();
    let r = eng.run_tbc(upvar_nested_bc()).expect("f9b nested");
    assert_eq!(result_str(&r), "7");

    let mut eng = strict();
    let r = eng
        .run_tbc(upvar_nested_bc_with_level("#0"))
        .expect("f9b absolute top-level");
    assert_eq!(result_str(&r), "7");

    let err = strict()
        .run_tbc(upvar_nested_bc_with_level("bogus"))
        .expect_err("malformed upvar level must fail");
    assert!(matches!(
        err,
        VmError::TclError { ref message, .. } if message == "bad level \"bogus\""
    ));
}

#[test]
fn f10_trap_suite() {
    let mut eng = strict();
    for opcode in [
        Opcode::Yield,
        Opcode::TclooSelf,
        Opcode::InfoLevelNum,
        Opcode::OriginCommand,
    ] {
        let bc = minimal_bc(vec![tcl86_opcode(opcode)], vec![]);
        let err = eng.run_tbc(bc).unwrap_err();
        assert!(
            matches!(err, VmError::UnimplementedOpcode { .. }),
            "{opcode:?}: {err:?}"
        );
    }
}

/// Option B: host-side call into a registered VmProc via stub re-entry (S05).
#[test]
fn option_b_host_eval_reenters_vm_proc() {
    let path = fixture("proc.tbc");
    assert!(path.exists(), "proc.tbc fixture required");
    let mut eng = hybrid();
    // Register procs `a` / `b` (and run nested calls once via TEBC).
    eng.run_tbc_path(&path).expect("register via proc.tbc");

    // Host `eval {a Z}` is WorldHost; host invokes stub `a` → InVmFromHostStub → VmProc.
    // Body of `a` is `return "$x : $x"` → result "Z : Z".
    let bc = host_eval_bc("a Z");
    let r = eng.run_tbc(bc).expect("option B re-entry");
    assert_eq!(result_str(&r), "Z : Z");

    let custom_error = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::ReturnStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("-code error -level 0 -errorcode {A B}"),
            lit_str("boom"),
        ],
    );
    eng.run_tbc(register_named_bodies(&[("custom_error", custom_error)]))
        .expect("register custom-error body");
    let options = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str(
            "catch {custom_error} result options; dict get $options -errorcode",
        )],
    );
    assert_eq!(result_str(&eng.run_tbc(options).unwrap()), "A B");
}

/// Host `return -level 2` crosses an opaque host boundary and must fail closed.
#[test]
fn host_return_level_gt1_unsupported() {
    let mut eng = hybrid();
    // Direct WorldHost of `return -level 2 x` (not wrapped in eval, which
    // converts unexpected codes to ERROR).
    let bc = minimal_bc(
        vec![
            1, 0, // return
            1, 1, // -level
            1, 2, // 2
            1, 3, // x
            6, 4, // invokeStk1 4
            0, // done
        ],
        vec![
            lit_str("return"),
            lit_str("-level"),
            lit_str("2"),
            lit_str("x"),
        ],
    );
    let err = eng.run_tbc(bc).expect_err("level>1 must fail");
    assert!(
        matches!(err, VmError::CrossRuntimeReturnLevel),
        "expected CrossRuntimeReturnLevel, got {err:?}"
    );
    assert_eq!(err.exit_code(), 2);
}

#[cfg(feature = "debug")]
#[test]
fn debug_resume_clears_frames_on_error() {
    use tcld_vm::DebugOutcome;

    // Trap opcode so step eventually errors.
    let expr_stk = tcl86_opcode(Opcode::Yield);
    let mut eng = HybridEngine::new(RuntimeMode::VmStrict).expect("engine");
    let bc = minimal_bc(vec![expr_stk], vec![]);
    let _ = eng.debug_start_tbc(bc, true).expect("start");
    // Resume past entry → executes ExprStk → UnimplementedOpcode.
    let err = eng.step_instruction().expect_err("trap should error");
    assert!(
        matches!(err, VmError::UnimplementedOpcode { .. }),
        "{err:?}"
    );
    // Further resume must not use corrupt frames.
    let err2 = eng.step_instruction().expect_err("no suspended frames");
    assert!(
        err2.to_string().contains("no suspended frames") || matches!(err2, VmError::Internal(_)),
        "{err2:?}"
    );
    // Restart-style: start clean again still works.
    let mut eng2 = HybridEngine::new(RuntimeMode::VmStrict).expect("engine2");
    let r = eng2
        .debug_start_tbc(arith_bc(), false)
        .expect("fresh start");
    // Without stop_on_entry, may finish immediately.
    match r {
        DebugOutcome::Finished { result } => {
            assert_eq!(result.as_i64().unwrap(), 5);
        }
        DebugOutcome::Paused(_) => {
            let _ = eng2.continue_run().expect("continue");
        }
    }
}

fn host_eval_bc(script: &str) -> ByteCode {
    // push "eval"; push script; invokeStk1 2; done
    minimal_bc(
        vec![
            1, 0, // push1 lit0 = eval
            1, 1, // push1 lit1 = script
            6, 2, // invokeStk1 2
            0, // done
        ],
        vec![lit_str("eval"), lit_str(script)],
    )
}

#[test]
fn f11_vm_strict_world_host_denied() {
    let mut eng = strict();
    let bc = command_bc("exec", "should-deny");
    let err = eng.run_tbc(bc).unwrap_err();
    assert!(
        matches!(err, VmError::PolicyDenied { .. }),
        "expected PolicyDenied, got {err:?}"
    );
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn f11_vm_strict_puts_writes_its_own_stdout() {
    let mut eng = strict();
    eng.begin_output_capture().unwrap();
    let result = eng.run_tbc(puts_bc("from-vm-strict")).unwrap();
    assert_eq!(result.as_string().unwrap(), "ok");
    let (stdout, stderr) = eng.captured_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&stdout), "from-vm-strict\n");
    assert!(stderr.is_empty());
}

#[cfg(feature = "debug")]
#[test]
fn f12_debug_hybrid_pause_shape() {
    use tcld_vm::{DebugOutcome, PauseReason};

    // Step a simple body (same arithmetic as F1) under Model B.
    let mut eng = HybridEngine::new(RuntimeMode::VmStrict).expect("engine");
    let outcome = eng.debug_start_tbc(arith_bc(), true).expect("debug start");
    let snap = match outcome {
        DebugOutcome::Paused(s) => s,
        DebugOutcome::Finished { .. } => panic!("expected pause on entry"),
    };
    assert!(snap.pc.is_some(), "pc-primary pause");
    assert!(snap.location.is_none(), "Location must be None (Model B)");
    assert!(!snap.in_host, "TEBC pause is not in_host");
    assert!(
        matches!(
            snap.reason,
            PauseReason::Entry | PauseReason::InstructionStep
        ),
        "reason={:?}",
        snap.reason
    );
    let pc0 = snap.pc.unwrap();

    let outcome2 = eng.step_instruction().expect("step");
    match outcome2 {
        DebugOutcome::Paused(s) => {
            assert!(s.pc.is_some());
            assert!(s.location.is_none());
            assert!(!s.in_host);
            // PC should advance (or stay only if same instr — expect move).
            assert_ne!(s.pc.unwrap(), pc0, "step should advance pc");
        }
        DebugOutcome::Finished { result } => {
            // Tiny body may finish in few steps; still assert last shape via snapshot.
            let s = eng.pause_snapshot().expect("completed snapshot");
            assert!(s.location.is_none());
            assert!(!s.in_host);
            let _ = result;
        }
    }

    // Also exercise F4 registration stream under hybrid then step its top-level.
    let path = fixture("proc.tbc");
    assert!(
        path.exists(),
        "required Pass fixture missing: {}",
        path.display()
    );
    let mut eng = hybrid();
    eng.enable_debug(true);
    let out = eng
        .debug_start_tbc_path(&path, true)
        .expect("f4 debug start");
    let finished_immediately = matches!(out, DebugOutcome::Finished { .. });
    match out {
        DebugOutcome::Paused(s) => {
            assert!(s.pc.is_some());
            assert!(s.location.is_none());
            assert!(!s.in_host);
        }
        DebugOutcome::Finished { .. } => {}
    }
    // One instruction step preserves Model B shape.
    assert!(
        eng.pause_snapshot().is_some() || finished_immediately,
        "expected pause or finished after debug start"
    );
    if eng.pause_snapshot().is_some() {
        match eng.step_instruction() {
            Ok(DebugOutcome::Paused(s)) => {
                assert!(s.pc.is_some());
                assert!(s.location.is_none());
                assert!(!s.in_host);
            }
            Ok(DebugOutcome::Finished { .. }) => {}
            Err(e) => panic!("step after f4: {e}"),
        }
    }
}

#[test]
fn f8_public_fixture_registration_count() {
    use sha2::{Digest, Sha256};

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testing/fixtures/public/public_complex_fixture.tbc");
    assert!(
        path.exists(),
        "required Pass fixture missing: {} (F8 registration oracle)",
        path.display()
    );
    let bytes = std::fs::read(&path).expect("read public_complex_fixture.tbc");
    let hash = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        hash, "335059a4ec1bb8610162f829a0c52e08c938e44b8d8dda216595e5beab950d2d",
        "input fixture drift — update pin only intentionally"
    );

    let mut eng = hybrid();
    let _ = eng
        .run_tbc_path(&path)
        .expect("public_complex_fixture.tbc registration stream");
    assert_eq!(eng.vm_proc_count(), 8, "expected 8 fixture procedures");
}

#[test]
fn public_fixture_bodies_execute_through_public_api() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testing/fixtures/public/public_complex_fixture.tbc");
    let mut engine = hybrid();
    engine
        .run_tbc_path(&path)
        .expect("load public fixture registrations");
    assert_eq!(engine.vm_proc_count(), 8);

    for (name, args, expected) in [
        ("::fixture::isValidContent", vec![VmObj::from_str("")], "0"),
        (
            "::fixture::isValidContent",
            vec![VmObj::from_str("abc")],
            "1",
        ),
        (
            "::fixture::calculateInterval",
            vec![VmObj::from_str("2h"), VmObj::from_str("m")],
            "120",
        ),
        (
            "::fixture::getProperTimeShow",
            vec![VmObj::from_str("61")],
            " 1 minute(s) 1 second(s)",
        ),
    ] {
        let completion = engine
            .invoke_registered(name.as_bytes(), &args)
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(completion.code, 0, "{name}: {completion:?}");
        assert_eq!(result_str(&completion.result), expected, "{name}");
    }

    let coverage = engine.coverage_snapshot();
    assert_eq!(coverage.opcode_total, 190);
    assert_eq!(coverage.trap_count, 14);
    assert!(coverage.executed_instruction_total > 0);
}

#[test]
fn nested_vm_proc_is_a_pausable_scheduler_frame() {
    use tcld_vm::DebugOutcome;

    let path = fixture("proc.tbc");
    let mut eng = hybrid();
    let mut outcome = eng
        .debug_start_tbc_path(&path, true)
        .expect("debug proc.tbc");
    let nested = loop {
        match outcome {
            DebugOutcome::Paused(snapshot) if snapshot.frame_kind == "proc" => break snapshot,
            DebugOutcome::Paused(_) => {
                outcome = eng.step_instruction().expect("step toward nested proc")
            }
            DebugOutcome::Finished { .. } => panic!("proc.tbc finished without entering a VM proc"),
        }
    };

    assert!(nested.call_depth >= 2);
    assert_eq!(nested.frames.len(), nested.call_depth);
    assert!(nested.frame_id.is_some());
    assert!(nested.address.is_some());
    assert_ne!(
        nested.frames[1].address.expect("caller address").code_id,
        nested.address.expect("callee address").code_id
    );

    match eng.step_out().expect("step out of nested proc") {
        DebugOutcome::Paused(snapshot) => {
            assert_eq!(snapshot.frame_kind, "toplevel");
            assert_eq!(snapshot.call_depth, 1);
        }
        DebugOutcome::Finished { .. } => {}
    }
    let code_ids = eng
        .observations()
        .iter()
        .map(|observation| observation.code_id)
        .collect::<std::collections::BTreeSet<_>>();
    let frame_ids = eng
        .observations()
        .iter()
        .map(|observation| observation.frame_id)
        .collect::<std::collections::HashSet<_>>();
    assert!(
        code_ids.len() >= 2,
        "ordered trace must retain nested CodeIds"
    );
    assert!(
        frame_ids.len() >= 2,
        "ordered trace must retain nested FrameIds"
    );
}

#[test]
fn suspended_nested_frames_support_selected_eval_and_refuse_vm_reentry() {
    use tcld_vm::DebugOutcome;

    let path = fixture("proc.tbc");
    let mut engine = hybrid();
    let mut outcome = engine
        .debug_start_tbc_path(&path, true)
        .expect("debug proc.tbc");
    loop {
        match outcome {
            DebugOutcome::Paused(ref snapshot) if snapshot.frame_kind == "proc" => break,
            DebugOutcome::Paused(_) => {
                outcome = engine.step_instruction().expect("step toward nested proc")
            }
            DebugOutcome::Finished { .. } => panic!("expected nested procedure pause"),
        }
    }

    let completion = engine
        .eval_in_frame(0, VmObj::from_str("set x CHANGED"))
        .expect("eval selected callee");
    assert_eq!(completion.code, 0, "{completion:?}");
    assert_eq!(result_str(&completion.result), "CHANGED");
    assert!(
        engine
            .ui_frame_locals(0)
            .iter()
            .any(|(name, value)| name == "x" && value == "CHANGED")
    );

    let parent = engine
        .eval_in_frame(1, VmObj::from_str("set parent_probe 17"))
        .expect("eval selected caller");
    assert_eq!(parent.code, 0, "{parent:?}");
    assert_eq!(result_str(&parent.result), "17");

    let reentry = engine
        .eval_in_frame(0, VmObj::from_str("a AGAIN"))
        .expect("VM reentry is returned as a Tcl completion");
    assert_eq!(reentry.code, 1, "{reentry:?}");
    assert!(
        result_str(&reentry.result).contains("host frame suspension is unsupported"),
        "{reentry:?}"
    );

    let error = engine
        .eval_in_frame(99, VmObj::from_str("set x"))
        .expect_err("invalid frame level must fail");
    assert!(matches!(error, tcld_vm::VmError::HostFrameUnavailable(_)));
}

#[test]
fn return_level_two_unwinds_two_vm_proc_frames() {
    let mut child_bytes = vec![1, 0, 1, 1, 98]; // result, options, returnImm
    child_bytes.extend(0i32.to_be_bytes()); // -code ok
    child_bytes.extend(2u32.to_be_bytes()); // -level 2
    let child = minimal_bc(child_bytes, vec![lit_str("LEVEL2"), lit_str("")]);
    let parent = minimal_bc(
        vec![1, 0, 6, 1, 0], // child; invoke 1; done
        vec![lit_str("child")],
    );

    let top = minimal_bc(
        vec![
            1, 0, 1, 1, 1, 2, 1, 3, 6, 4, 3, // register child
            1, 0, 1, 4, 1, 2, 1, 5, 6, 4, 3, // register parent
            1, 4, 6, 1, 0, // invoke parent; done
        ],
        vec![
            lit_str("tbcload::bcproc"),
            lit_str("child"),
            lit_str(""),
            proc_body_lit(child),
            lit_str("parent"),
            proc_body_lit(parent),
        ],
    );

    let result = strict().run_tbc(top).expect("return -level 2 in VM");
    assert_eq!(result_str(&result), "LEVEL2");
}

#[test]
fn frame_namespace_resolves_sibling_vm_proc_relative_host_and_ns_current() {
    let child = minimal_bc(
        vec![tcl86_opcode(Opcode::Push1), 0, tcl86_opcode(Opcode::Done)],
        vec![lit_str("child-ok")],
    );
    let parent = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::InvokeStk1),
            1,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("child")],
    );
    let host_relative = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::InvokeStk1),
            3,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("string"), lit_str("length"), lit_str("abcd")],
    );
    let current = minimal_bc(
        vec![tcl86_opcode(Opcode::NsCurrent), tcl86_opcode(Opcode::Done)],
        vec![],
    );
    let eval_current = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("namespace current")],
    );
    let registration = register_named_bodies(&[
        ("::demo::child", child),
        ("::demo::parent", parent),
        ("::demo::host_relative", host_relative),
        ("::demo::current", current),
        ("::demo::eval_current", eval_current),
    ]);
    let mut engine = hybrid();
    engine
        .run_tbc(registration)
        .expect("register namespace bodies");
    let sibling = engine
        .invoke_registered(b"::demo::parent", &[])
        .expect("relative sibling");
    assert_eq!(result_str(&sibling.result), "child-ok");
    let host = engine
        .invoke_registered(b"::demo::host_relative", &[])
        .expect("relative host command");
    assert_eq!(host.result.as_i64().unwrap(), 4);
    let namespace = engine
        .invoke_registered(b"::demo::current", &[])
        .expect("NsCurrent");
    assert_eq!(result_str(&namespace.result), "::demo");
    let eval_namespace = engine
        .invoke_registered(b"::demo::eval_current", &[])
        .expect("HostFrame namespace current");
    assert_eq!(result_str(&eval_namespace.result), "::demo");
}

#[test]
fn foreach_legacy_assigns_groups_and_fills_missing_values() {
    let mut bytecode = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::StoreScalar1),
            0,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::ForeachStart4),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::ForeachStep4),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::LoadScalar1),
            2,
            tcl86_opcode(Opcode::LoadScalar1),
            3,
            tcl86_opcode(Opcode::List),
            0,
            0,
            0,
            2,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("alpha")],
    );
    bytecode.proc_num_locals = 4;
    bytecode.locals = local_slots(&["values", "counter", "left", "right"]);
    bytecode.auxiliaries.push(AuxData {
        kind: AuxDataType::Foreach,
        value: AuxDataValue::Foreach(ForeachInfo {
            first_value: 0,
            loop_counter: 1,
            lists: vec![vec![2, 3]],
        }),
    });
    bytecode.info.num_aux_data_items = 1;

    let result = strict().run_tbc(bytecode).expect("legacy foreach");
    assert_eq!(
        result.as_list_vec().unwrap(),
        vec![VmObj::from_str("alpha"), VmObj::empty()]
    );
}

#[test]
fn foreach_new_uses_frame_owned_state_and_cleans_stack() {
    let mut bytecode = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::ForeachStart),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Nop),
            tcl86_opcode(Opcode::ForeachStep),
            tcl86_opcode(Opcode::ForeachEnd),
            tcl86_opcode(Opcode::LoadScalar1),
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("alpha beta")],
    );
    bytecode.proc_num_locals = 1;
    bytecode.locals = local_slots(&["item"]);
    bytecode.auxiliaries.push(AuxData {
        kind: AuxDataType::NewForeach,
        value: AuxDataValue::NewForeach(NewForeachInfo {
            loop_counter: -1,
            lists: vec![vec![0]],
        }),
    });
    bytecode.info.num_aux_data_items = 1;

    let result = strict().run_tbc(bytecode).expect("new foreach");
    assert_eq!(result_str(&result), "beta");

    let mut multiple = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::ForeachStart),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Nop),
            tcl86_opcode(Opcode::ForeachStep),
            tcl86_opcode(Opcode::ForeachEnd),
            tcl86_opcode(Opcode::LoadScalar1),
            0,
            tcl86_opcode(Opcode::LoadScalar1),
            1,
            tcl86_opcode(Opcode::LoadScalar1),
            2,
            tcl86_opcode(Opcode::List),
            0,
            0,
            0,
            3,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("a b c"), lit_str("1 2")],
    );
    multiple.proc_num_locals = 3;
    multiple.locals = local_slots(&["left", "right", "number"]);
    multiple.auxiliaries.push(AuxData {
        kind: AuxDataType::NewForeach,
        value: AuxDataValue::NewForeach(NewForeachInfo {
            loop_counter: -1,
            lists: vec![vec![0, 1], vec![2]],
        }),
    });
    multiple.info.num_aux_data_items = 1;
    assert_eq!(
        strict().run_tbc(multiple).unwrap().as_list_vec().unwrap(),
        vec![VmObj::from_str("c"), VmObj::empty(), VmObj::from_str("2")]
    );
}

#[test]
fn lmap_collect_uses_foreach_state_and_preserves_the_result_list() {
    for (input, expected) in [("alpha beta", "alpha beta"), ("", "")] {
        let mut bytecode = minimal_bc(
            vec![
                tcl86_opcode(Opcode::List),
                0,
                0,
                0,
                0,
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::ForeachStart),
                0,
                0,
                0,
                0,
                tcl86_opcode(Opcode::LoadScalar1),
                0,
                tcl86_opcode(Opcode::LmapCollect),
                tcl86_opcode(Opcode::ForeachStep),
                tcl86_opcode(Opcode::ForeachEnd),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str(input)],
        );
        bytecode.proc_num_locals = 1;
        bytecode.locals = local_slots(&["item"]);
        bytecode.auxiliaries.push(AuxData {
            kind: AuxDataType::NewForeach,
            value: AuxDataValue::NewForeach(NewForeachInfo {
                loop_counter: -3,
                lists: vec![vec![0]],
            }),
        });
        bytecode.info.num_aux_data_items = 1;
        assert_eq!(result_str(&strict().run_tbc(bytecode).unwrap()), expected);
    }
    let invalid = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::LmapCollect),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("result")],
    );
    assert!(matches!(
        strict().run_tbc(invalid),
        Err(VmError::InvalidAuxData { .. })
    ));
}

#[test]
fn jump_tables_consume_keys() {
    for (key, expected) in [("alpha", "hit"), ("other", "miss")] {
        let mut bytecode = minimal_bc(
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::JumpTable),
                0,
                0,
                0,
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::Done),
                tcl86_opcode(Opcode::Push1),
                2,
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str(key), lit_str("miss"), lit_str("hit")],
        );
        bytecode.auxiliaries.push(AuxData {
            kind: AuxDataType::JumpTable,
            value: AuxDataValue::JumpTable(JumptableInfo {
                dict: [("alpha".to_string(), 8)].into_iter().collect(),
            }),
        });
        bytecode.info.num_aux_data_items = 1;
        let result = strict().run_tbc(bytecode).expect("jump table");
        assert_eq!(result_str(&result), expected, "key={key}");
    }
}

#[test]
fn foreach_and_jump_tables_reject_invalid_aux_state_and_targets() {
    let missing_foreach = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ForeachStart4),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![],
    );
    assert!(matches!(
        strict().run_tbc(missing_foreach),
        Err(VmError::InvalidAuxData { .. })
    ));

    let step_without_start = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ForeachStep),
            tcl86_opcode(Opcode::Done),
        ],
        vec![],
    );
    assert!(matches!(
        strict().run_tbc(step_without_start),
        Err(VmError::InvalidAuxData { .. })
    ));

    let end_without_start = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::ForeachEnd),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("99")],
    );
    assert!(matches!(
        strict().run_tbc(end_without_start),
        Err(VmError::InvalidAuxData { .. })
    ));

    let mut invalid_lvt = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ForeachStart4),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![],
    );
    invalid_lvt.auxiliaries.push(AuxData {
        kind: AuxDataType::Foreach,
        value: AuxDataValue::Foreach(ForeachInfo {
            first_value: 0,
            loop_counter: 1,
            lists: vec![vec![2]],
        }),
    });
    invalid_lvt.info.num_aux_data_items = 1;
    assert!(matches!(
        strict().run_tbc(invalid_lvt),
        Err(VmError::InvalidAuxData { .. })
    ));

    let mut bad_target = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::JumpTable),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("hit")],
    );
    bad_target.auxiliaries.push(AuxData {
        kind: AuxDataType::JumpTable,
        value: AuxDataValue::JumpTable(JumptableInfo {
            dict: [("hit".to_string(), 1)].into_iter().collect(),
        }),
    });
    bad_target.info.num_aux_data_items = 1;
    assert!(matches!(
        strict().run_tbc(bad_target),
        Err(VmError::InvalidJumpTarget { .. })
    ));
}

#[test]
fn host_setup_is_hybrid_only_while_math_and_dicts_run_natively() {
    let mut setup_engine = hybrid();
    assert_eq!(
        setup_engine
            .host_setup_eval("set ::corpus_setup 17")
            .expect("typed host setup")
            .code,
        0
    );
    let setup_read = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::InvokeStk1),
            2,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("set"), lit_str("::corpus_setup")],
    );
    assert_eq!(
        setup_engine.run_tbc(setup_read).unwrap().as_i64().unwrap(),
        17
    );
    assert!(matches!(
        strict().host_setup_eval("set ::forbidden 1"),
        Err(VmError::PolicyDenied {
            action: "host_setup",
            ..
        })
    ));

    let mut capture_engine = hybrid();
    capture_engine.begin_output_capture().unwrap();
    let output = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::InvokeStk1),
            2,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::Push1),
            3,
            tcl86_opcode(Opcode::InvokeStk1),
            3,
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("puts"),
            lit_str("captured-out"),
            lit_str("stderr"),
            lit_str("captured-err"),
        ],
    );
    capture_engine.run_tbc(output).unwrap();
    let (stdout, stderr) = capture_engine.captured_output().unwrap();
    assert_eq!(stdout, b"captured-out\n");
    assert_eq!(stderr, b"captured-err\n");

    let math = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::CallBuiltinFunc1),
            19, // abs
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("-5")],
    );
    assert_eq!(hybrid().run_tbc(math.clone()).unwrap().as_i64().unwrap(), 5);
    assert_eq!(strict().run_tbc(math).unwrap().as_i64().unwrap(), 5);
    let dynamic_math = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::CallFunc1),
            2,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("abs"), lit_str("-6")],
    );
    assert_eq!(
        hybrid()
            .run_tbc(dynamic_math.clone())
            .unwrap()
            .as_i64()
            .unwrap(),
        6
    );
    assert_eq!(strict().run_tbc(dynamic_math).unwrap().as_i64().unwrap(), 6);

    let dict_get = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::DictGet),
            0,
            0,
            0,
            1,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("alpha 7"), lit_str("alpha")],
    );
    assert_eq!(
        result_str(&hybrid().run_tbc(dict_get.clone()).unwrap()),
        "7"
    );
    assert_eq!(result_str(&strict().run_tbc(dict_get).unwrap()), "7");

    let invalid_dict = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::DictGet),
            0,
            0,
            0,
            1,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("{"), lit_str("alpha")],
    );
    for mut engine in [hybrid(), strict()] {
        assert!(matches!(
            engine.run_tbc(invalid_dict.clone()),
            Err(VmError::TclError { message, error_code: Some(code), .. })
                if message == "unmatched open brace in dict" && code == "TCL VALUE DICTIONARY BRACE"
        ));
    }
}

#[test]
fn tcl9_tail_opcodes_execute_or_trap_at_the_declared_boundary() {
    for (opcode, left, right, expected) in [
        (Opcode::StrLt, "alpha", "beta", 1),
        (Opcode::StrGt, "alpha", "beta", 0),
        (Opcode::StrLe, "same", "same", 1),
        (Opcode::StrGe, "same", "same", 1),
    ] {
        let bytecode = tcl9_bc(
            vec![
                tcl9_opcode(Opcode::Push1),
                0,
                tcl9_opcode(Opcode::Push1),
                1,
                tcl9_opcode(opcode),
                tcl9_opcode(Opcode::Done),
            ],
            vec![lit_str(left), lit_str(right)],
        );
        assert_eq!(
            strict().run_tbc(bytecode).unwrap().as_i64().unwrap(),
            expected,
            "{opcode:?}"
        );
    }

    let mut dict_get_default = vec![
        tcl9_opcode(Opcode::Push1),
        0,
        tcl9_opcode(Opcode::Push1),
        1,
        tcl9_opcode(Opcode::Push1),
        2,
        tcl9_opcode(Opcode::DictGetDef),
    ];
    dict_get_default.extend(1i32.to_be_bytes());
    dict_get_default.push(tcl9_opcode(Opcode::Done));
    let dict_get_default = tcl9_bc(
        dict_get_default,
        vec![
            lit_str("present value"),
            lit_str("missing"),
            lit_str("fallback"),
        ],
    );
    assert_eq!(
        result_str(&hybrid().run_tbc(dict_get_default.clone()).unwrap()),
        "fallback"
    );
    assert_eq!(
        result_str(&strict().run_tbc(dict_get_default).unwrap()),
        "fallback"
    );

    for (flags, args, expected) in [
        (2, vec!["a b", "1", "x"], "a x b"),
        (1, vec!["a b c", "0", "1", "x"], "x c"),
    ] {
        let mut code = Vec::new();
        let mut literals = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            code.extend([tcl9_opcode(Opcode::Push1), index as u8]);
            literals.push(lit_str(arg));
        }
        code.push(tcl9_opcode(Opcode::Lreplace4));
        code.extend((args.len() as i32).to_be_bytes());
        code.push(flags);
        code.push(tcl9_opcode(Opcode::Done));
        let bytecode = tcl9_bc(code, literals);
        assert_eq!(
            result_str(&hybrid().run_tbc(bytecode.clone()).unwrap()),
            expected
        );
        assert_eq!(result_str(&strict().run_tbc(bytecode).unwrap()), expected);
    }
    let mut invalid_lreplace = vec![tcl9_opcode(Opcode::Lreplace4)];
    invalid_lreplace.extend(2i32.to_be_bytes());
    invalid_lreplace.push(0);
    assert!(matches!(
        hybrid().run_tbc(tcl9_bc(invalid_lreplace, vec![])),
        Err(VmError::InvalidOpcodeOperand { opcode, .. }) if opcode == "Lreplace4"
    ));

    let mut invalid_dict_get_default = vec![tcl9_opcode(Opcode::DictGetDef)];
    invalid_dict_get_default.extend(0i32.to_be_bytes());
    assert!(matches!(
        hybrid().run_tbc(tcl9_bc(invalid_dict_get_default, vec![])),
        Err(VmError::InvalidOpcodeOperand { opcode, .. }) if opcode == "DictGetDef"
    ));

    let mut const_imm = vec![tcl9_opcode(Opcode::Push1), 0, tcl9_opcode(Opcode::ConstImm)];
    const_imm.extend(0i32.to_be_bytes());
    let mut const_imm_bc = tcl9_bc(const_imm, vec![lit_str("value")]);
    const_imm_bc.proc_num_locals = 1;
    const_imm_bc.locals = vec![CompiledLocal {
        name: "answer".into(),
        default_value: None,
        frame_index: 0,
        flag_mask: 0,
    }];
    let const_stk = tcl9_bc(
        vec![
            tcl9_opcode(Opcode::Push1),
            0,
            tcl9_opcode(Opcode::Push1),
            1,
            tcl9_opcode(Opcode::ConstStk),
        ],
        vec![lit_str("answer"), lit_str("value")],
    );
    for (opcode, pc, bytecode) in [
        (Opcode::ConstImm, 2, const_imm_bc),
        (Opcode::ConstStk, 4, const_stk),
    ] {
        let error = strict()
            .run_tbc(bytecode)
            .expect_err("Tcl 9 const opcodes remain deliberate traps");
        assert!(
            matches!(
                &error,
                VmError::UnimplementedOpcode {
                    opcode: trapped,
                    pc: trapped_pc
                } if trapped == &format!("{opcode:?}") && *trapped_pc == pc
            ),
            "{opcode:?}: {error:?}"
        );
    }
}

#[test]
fn host_value_string_and_clock_protocols_match_tcl() {
    let cases = [
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::Push1),
                2,
                tcl86_opcode(Opcode::StrMap),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("a"), lit_str("X"), lit_str("aab")],
            "XXb",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::StrFind),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("bc"), lit_str("abcabc")],
            "1",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::StrFindLast),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("bc"), lit_str("abcabc")],
            "4",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::Push1),
                2,
                tcl86_opcode(Opcode::StrRange),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("abcdef"), lit_str("1"), lit_str("3")],
            "bcd",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::StrTrim),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("xxhellox"), lit_str("x")],
            "hello",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::StrTrimLeft),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("xxhellox"), lit_str("x")],
            "hellox",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::StrTrimRight),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("xxhellox"), lit_str("x")],
            "xxhello",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::StrUpper),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("MiXeD")],
            "MIXED",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::StrLower),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("MiXeD")],
            "mixed",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::StrTitle),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("mixed")],
            "Mixed",
        ),
        (
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::Push1),
                1,
                tcl86_opcode(Opcode::Push1),
                2,
                tcl86_opcode(Opcode::Push1),
                3,
                tcl86_opcode(Opcode::StrReplace),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str("abcdef"), lit_str("1"), lit_str("3"), lit_str("X")],
            "aXef",
        ),
    ];
    for (code, literals, expected) in cases {
        assert_eq!(
            result_str(&hybrid().run_tbc(minimal_bc(code, literals)).unwrap()),
            expected
        );
    }

    let string_class = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::StrClass),
            4, // digit
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("123")],
    );
    assert_eq!(hybrid().run_tbc(string_class).unwrap().as_i64().unwrap(), 1);

    for selector in 0..=4 {
        let clock = minimal_bc(
            vec![
                tcl86_opcode(Opcode::ClockRead),
                selector,
                tcl86_opcode(Opcode::Done),
            ],
            vec![],
        );
        let value = hybrid().run_tbc(clock).expect("clock selector");
        assert!(
            value.as_i64().is_ok(),
            "clock selector {selector}: {value:?}"
        );
    }
    assert!(matches!(
        hybrid().run_tbc(minimal_bc(
            vec![
                tcl86_opcode(Opcode::ClockRead),
                5,
                tcl86_opcode(Opcode::Done)
            ],
            vec![]
        )),
        Err(VmError::InvalidOpcodeOperand { .. })
    ));
}

#[test]
fn dict_expand_recombine_and_iteration_follow_compiled_protocols() {
    let mut update_code = vec![
        tcl86_opcode(Opcode::Push1),
        0, // d
        tcl86_opcode(Opcode::Push1),
        1, // a 1 b 2
        tcl86_opcode(Opcode::StoreScalarStk),
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::Push1),
        2, // key list remains through Start/End
        tcl86_opcode(Opcode::DictUpdateStart),
    ];
    update_code.extend(0u32.to_be_bytes());
    update_code.extend(0u32.to_be_bytes());
    update_code.extend([
        tcl86_opcode(Opcode::Push1),
        3, // 9
        tcl86_opcode(Opcode::StoreScalar1),
        1,
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::UnsetScalar),
        1, // leave error
    ]);
    update_code.extend(2u32.to_be_bytes());
    update_code.push(tcl86_opcode(Opcode::DictUpdateEnd));
    update_code.extend(0u32.to_be_bytes());
    update_code.extend(0u32.to_be_bytes());
    update_code.extend([
        tcl86_opcode(Opcode::Push1),
        4, // dict
        tcl86_opcode(Opcode::Push1),
        5, // get
        tcl86_opcode(Opcode::LoadScalar1),
        0,
        tcl86_opcode(Opcode::Push1),
        6, // a
        tcl86_opcode(Opcode::InvokeStk1),
        4,
        tcl86_opcode(Opcode::Push1),
        4, // dict
        tcl86_opcode(Opcode::Push1),
        7, // exists
        tcl86_opcode(Opcode::LoadScalar1),
        0,
        tcl86_opcode(Opcode::Push1),
        8, // b
        tcl86_opcode(Opcode::InvokeStk1),
        4,
        tcl86_opcode(Opcode::List),
    ]);
    update_code.extend(2u32.to_be_bytes());
    update_code.push(tcl86_opcode(Opcode::Done));
    let mut update = minimal_bc(
        update_code,
        vec![
            lit_str("d"),
            lit_str("a 1 b 2"),
            lit_str("a b"),
            lit_str("9"),
            lit_str("dict"),
            lit_str("get"),
            lit_str("a"),
            lit_str("exists"),
            lit_str("b"),
        ],
    );
    update.proc_num_locals = 3;
    update.locals = local_slots(&["d", "a", "b"]);
    update.auxiliaries.push(AuxData {
        kind: AuxDataType::DictUpdate,
        value: AuxDataValue::DictUpdate(DictUpdateInfo {
            indices: vec![1, 2],
        }),
    });
    update.info.num_aux_data_items = 1;
    assert_eq!(
        hybrid().run_tbc(update).unwrap().as_list_vec().unwrap(),
        vec![VmObj::from_str("9"), VmObj::from_str("0")]
    );

    let mut invalid_update = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::DictUpdateStart),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("a")],
    );
    invalid_update.proc_num_locals = 1;
    invalid_update.locals = local_slots(&["d"]);
    invalid_update.auxiliaries.push(AuxData {
        kind: AuxDataType::DictUpdate,
        value: AuxDataValue::DictUpdate(DictUpdateInfo { indices: vec![5] }),
    });
    invalid_update.info.num_aux_data_items = 1;
    assert!(matches!(
        hybrid().run_tbc(invalid_update),
        Err(VmError::InvalidAuxData { .. })
    ));

    let recombine = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0, // d
            tcl86_opcode(Opcode::Push1),
            1, // dict
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            1, // dict
            tcl86_opcode(Opcode::Push1),
            2, // empty path
            tcl86_opcode(Opcode::DictExpand),
            tcl86_opcode(Opcode::Pop), // mapped key list
            tcl86_opcode(Opcode::Push1),
            3, // a
            tcl86_opcode(Opcode::Push1),
            4, // 9
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            5, // b
            tcl86_opcode(Opcode::UnsetStk),
            1,
            tcl86_opcode(Opcode::Push1),
            0, // target d
            tcl86_opcode(Opcode::Push1),
            2, // path
            tcl86_opcode(Opcode::Push1),
            6, // original keys
            tcl86_opcode(Opcode::DictRecombineStk),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::LoadScalarStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("d"),
            lit_str("a 1 b 2"),
            lit_str(""),
            lit_str("a"),
            lit_str("9"),
            lit_str("b"),
            lit_str("a b"),
        ],
    );
    assert_eq!(result_str(&hybrid().run_tbc(recombine).unwrap()), "a 9");

    let mut iteration = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::DictFirst),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::DictNext),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::DictDone),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::List),
            0,
            0,
            0,
            3,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("a 1 b 2")],
    );
    iteration.proc_num_locals = 1;
    iteration.locals = vec![CompiledLocal {
        name: "iterator".into(),
        default_value: None,
        frame_index: 0,
        flag_mask: 0,
    }];
    assert_eq!(
        hybrid()
            .run_tbc(iteration)
            .unwrap()
            .as_list_vec()
            .unwrap()
            .iter()
            .map(result_str)
            .collect::<Vec<_>>(),
        ["2", "b", "0"]
    );

    let invalid = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::DictVerify),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("odd")],
    );
    assert!(matches!(
        hybrid().run_tbc(invalid),
        Err(VmError::TclError { .. })
    ));
}

#[test]
fn dict_lvt_mutations_set_unset_incr_append_and_lappend() {
    let mut code = vec![
        tcl86_opcode(Opcode::Push1),
        0,
        tcl86_opcode(Opcode::StoreScalar1),
        0,
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::Push1),
        1,
        tcl86_opcode(Opcode::Push1),
        2,
        tcl86_opcode(Opcode::DictSet),
    ];
    code.extend(1u32.to_be_bytes());
    code.extend(0u32.to_be_bytes());
    code.push(tcl86_opcode(Opcode::Pop));
    code.extend([
        tcl86_opcode(Opcode::Push1),
        3,
        tcl86_opcode(Opcode::DictIncrImm),
    ]);
    code.extend(3i32.to_be_bytes());
    code.extend(0u32.to_be_bytes());
    code.push(tcl86_opcode(Opcode::Pop));
    code.extend([
        tcl86_opcode(Opcode::Push1),
        1,
        tcl86_opcode(Opcode::Push1),
        4,
        tcl86_opcode(Opcode::DictAppend),
    ]);
    code.extend(0u32.to_be_bytes());
    code.push(tcl86_opcode(Opcode::Pop));
    code.extend([
        tcl86_opcode(Opcode::Push1),
        5,
        tcl86_opcode(Opcode::Push1),
        6,
        tcl86_opcode(Opcode::DictLappend),
    ]);
    code.extend(0u32.to_be_bytes());
    code.push(tcl86_opcode(Opcode::Pop));
    code.extend([
        tcl86_opcode(Opcode::Push1),
        3,
        tcl86_opcode(Opcode::DictUnset),
    ]);
    code.extend(1u32.to_be_bytes());
    code.extend(0u32.to_be_bytes());
    code.extend([
        tcl86_opcode(Opcode::Pop),
        tcl86_opcode(Opcode::LoadScalar1),
        0,
        tcl86_opcode(Opcode::Done),
    ]);

    let mut bytecode = minimal_bc(
        code,
        vec![
            lit_str("a 1"),
            lit_str("b"),
            lit_str("2"),
            lit_str("a"),
            lit_str("X"),
            lit_str("c"),
            lit_str("Z"),
        ],
    );
    bytecode.proc_num_locals = 1;
    bytecode.locals = vec![CompiledLocal {
        name: "d".into(),
        default_value: None,
        frame_index: 0,
        flag_mask: 0,
    }];
    assert_eq!(result_str(&hybrid().run_tbc(bytecode).unwrap()), "b 2X c Z");
}

#[test]
fn host_frame_expr_and_eval_see_and_write_vm_locals() {
    let expr = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0, // x
            tcl86_opcode(Opcode::Push1),
            1, // 2
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            2, // $x + 3
            tcl86_opcode(Opcode::ExprStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("x"), lit_str("2"), lit_str("$x + 3")],
    );
    assert_eq!(hybrid().run_tbc(expr).unwrap().as_i64().unwrap(), 5);

    let eval = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::LoadScalarStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("x"), lit_str("2"), lit_str("set x 9; set x")],
    );
    assert_eq!(hybrid().run_tbc(eval).unwrap().as_i64().unwrap(), 9);

    let visible_locals = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("x"), lit_str("2"), lit_str("lsort [info locals]")],
    );
    assert_eq!(result_str(&hybrid().run_tbc(visible_locals).unwrap()), "x");
}

#[test]
fn host_frame_refuses_proc_body_before_creating_transport_state() {
    let body = minimal_bc(
        vec![tcl86_opcode(Opcode::Push1), 0, tcl86_opcode(Opcode::Done)],
        vec![lit_str("body")],
    );
    let mut refuses = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::StoreScalar1),
            0,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![proc_body_lit(body), lit_str("set x")],
    );
    refuses.proc_num_locals = 1;
    refuses.locals = local_slots(&["x"]);
    let mut engine = hybrid();
    assert!(matches!(
        engine.run_tbc(refuses),
        Err(VmError::BridgeMarshalProcBody)
    ));

    let no_ephemeral_command = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::InvokeStk1),
            3,
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("info"),
            lit_str("commands"),
            lit_str("::__tcld_frame_*"),
        ],
    );
    assert_eq!(
        result_str(&engine.run_tbc(no_ephemeral_command).unwrap()),
        ""
    );
}

#[test]
fn expanded_invoke_handles_lists_empty_marks_and_parse_errors() {
    let expanded = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ExpandStart),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::ExpandStktop),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::InvokeExpanded),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("list"), lit_str("alpha beta")],
    );
    assert_eq!(
        hybrid().run_tbc(expanded).unwrap().as_list_vec().unwrap(),
        vec![VmObj::from_str("alpha"), VmObj::from_str("beta")]
    );

    let empty = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ExpandStart),
            tcl86_opcode(Opcode::InvokeExpanded),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![],
    );
    assert!(
        hybrid()
            .run_tbc(empty)
            .unwrap()
            .ensure_bytes()
            .unwrap()
            .is_empty()
    );

    let malformed = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ExpandStart),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::ExpandStktop),
            0,
            0,
            0,
            0,
        ],
        vec![lit_str("{")],
    );
    assert!(matches!(
        hybrid().run_tbc(malformed),
        Err(VmError::TclError { .. })
    ));
}

#[test]
fn nested_expansion_and_invoke_replace_preserve_stack_protocols() {
    let nested = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ExpandStart),
            tcl86_opcode(Opcode::Push1),
            0, // outer list command
            tcl86_opcode(Opcode::ExpandStart),
            tcl86_opcode(Opcode::Push1),
            0, // inner list command
            tcl86_opcode(Opcode::Push1),
            1, // alpha beta
            tcl86_opcode(Opcode::ExpandStktop),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::InvokeExpanded),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::ExpandStktop),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::InvokeExpanded),
            0,
            0,
            0,
            0,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("list"), lit_str("alpha beta")],
    );
    assert_eq!(
        hybrid()
            .run_tbc(nested)
            .unwrap()
            .as_list_vec()
            .unwrap()
            .iter()
            .map(result_str)
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );

    let mut replace_code = vec![
        tcl86_opcode(Opcode::Push1),
        0, // discarded ensemble word
        tcl86_opcode(Opcode::Push1),
        1, // discarded subcommand word
        tcl86_opcode(Opcode::Push1),
        2, // retained argument
        tcl86_opcode(Opcode::Push1),
        3, // implementation command
        tcl86_opcode(Opcode::InvokeReplace),
    ];
    replace_code.extend(3u32.to_be_bytes());
    replace_code.push(2);
    replace_code.push(tcl86_opcode(Opcode::Done));
    assert_eq!(
        result_str(
            &hybrid()
                .run_tbc(minimal_bc(
                    replace_code,
                    vec![
                        lit_str("ignored"),
                        lit_str("subcommand"),
                        lit_str("argument"),
                        lit_str("list"),
                    ],
                ))
                .unwrap()
        ),
        "argument"
    );
}

#[test]
fn promoted_integer_string_regexp_syntax_and_variable_edges_match_tcl() {
    let bigint_shift = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Lshift),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("1"), lit_str("64")],
    );
    assert_eq!(
        result_str(&strict().run_tbc(bigint_shift).unwrap()),
        "18446744073709551616"
    );

    let bigint_or = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Bitor),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("18446744073709551616"), lit_str("1")],
    );
    assert_eq!(
        result_str(&strict().run_tbc(bigint_or).unwrap()),
        "18446744073709551617"
    );

    let negative_shift = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Rshift),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("1"), lit_str("-1")],
    );
    assert!(matches!(
        strict().run_tbc(negative_shift),
        Err(VmError::TclError { message, .. }) if message == "negative shift argument"
    ));

    let mut range_code = vec![
        tcl86_opcode(Opcode::Push1),
        0,
        tcl86_opcode(Opcode::StrRangeImm),
    ];
    range_code.extend((-3i32).to_be_bytes()); // end-1
    range_code.extend((-2i32).to_be_bytes()); // end
    range_code.push(tcl86_opcode(Opcode::Done));
    assert_eq!(
        result_str(
            &hybrid()
                .run_tbc(minimal_bc(range_code, vec![lit_str("abcdef")]))
                .unwrap()
        ),
        "ef"
    );

    let regexp = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Regexp),
            0o33, // advanced + nocase + nosub
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("abc"), lit_str("ABC")],
    );
    assert_eq!(
        HybridEngine::new(RuntimeMode::VmStrict)
            .unwrap()
            .run_tbc(regexp.clone())
            .unwrap()
            .as_i64()
            .unwrap(),
        1
    );
    assert_eq!(hybrid().run_tbc(regexp).unwrap().as_i64().unwrap(), 1);

    let unicode_length = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::StrLen),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("aé中")],
    );
    assert_eq!(
        strict().run_tbc(unicode_length).unwrap().as_i64().unwrap(),
        3
    );

    let noerr_unset = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::UnsetStk),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("missing"), lit_str("ok")],
    );
    assert_eq!(result_str(&strict().run_tbc(noerr_unset).unwrap()), "ok");

    let error_unset = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::UnsetStk),
            1,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("missing")],
    );
    assert!(matches!(
        strict().run_tbc(error_unset),
        Err(VmError::TclError { message, .. }) if message.contains("no such variable")
    ));

    let scalar_to_array = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::ArrayMakeStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("x"), lit_str("scalar")],
    );
    assert!(matches!(
        strict().run_tbc(scalar_to_array),
        Err(VmError::TclError { message, .. }) if message.contains("variable isn't array")
    ));

    let bigint_incr = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::IncrStkImm),
            1,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("n"), lit_str("9223372036854775808")],
    );
    assert_eq!(
        result_str(&strict().run_tbc(bigint_incr).unwrap()),
        "9223372036854775809"
    );

    let invalid_lappend = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::InvokeStk1),
            3,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            3,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            4,
            tcl86_opcode(Opcode::InvokeStk1),
            3,
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("set"),
            lit_str("x"),
            lit_str("{"),
            lit_str("lappend"),
            lit_str("value"),
        ],
    );
    assert!(matches!(
        strict().run_tbc(invalid_lappend),
        Err(VmError::TclError { message, .. }) if message == "unmatched open brace in list"
    ));

    let invalid_incr = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::InvokeStk1),
            3,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            3,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::InvokeStk1),
            2,
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("set"),
            lit_str("x"),
            lit_str("not-an-integer"),
            lit_str("incr"),
        ],
    );
    assert!(matches!(
        strict().run_tbc(invalid_incr),
        Err(VmError::TclError { message, .. }) if message.contains("expected integer")
    ));

    let append_array_to_scalar = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::Push1),
            3,
            tcl86_opcode(Opcode::AppendArrayStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("x"),
            lit_str("scalar"),
            lit_str("k"),
            lit_str("value"),
        ],
    );
    assert!(matches!(
        strict().run_tbc(append_array_to_scalar),
        Err(VmError::TclError { message, .. }) if message.contains("variable isn't array")
    ));

    let mut syntax_code = vec![
        tcl86_opcode(Opcode::Push1),
        0,
        tcl86_opcode(Opcode::Push1),
        1,
        tcl86_opcode(Opcode::Syntax),
    ];
    syntax_code.extend(1i32.to_be_bytes());
    syntax_code.extend(0u32.to_be_bytes());
    syntax_code.push(tcl86_opcode(Opcode::Done));
    assert!(matches!(
        strict().run_tbc(minimal_bc(
            syntax_code,
            vec![lit_str("syntax failure"), lit_str("")]
        )),
        Err(VmError::TclError { message, .. }) if message == "syntax failure"
    ));
}

#[test]
fn stable_semantic_failure_oracles_are_exact() {
    assert!(matches!(
        strict().run_tbc(minimal_bc(
            vec![
                tcl86_opcode(Opcode::BeginCatch4),
                0,
                0,
                0,
                0,
                tcl86_opcode(Opcode::Done)
            ],
            vec![]
        )),
        Err(VmError::InvalidCatchState)
    ));

    let malformed_list = minimal_bc(
        vec![
            tcl86_opcode(Opcode::ExpandStart),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::ExpandStktop),
            0,
            0,
            0,
            0,
        ],
        vec![lit_str("{")],
    );
    assert!(matches!(
        hybrid().run_tbc(malformed_list),
        Err(VmError::TclError { message, error_code: Some(code), .. })
            if message == "unmatched open brace in list" && code == "TCL VALUE LIST BRACE"
    ));

    let invalid_dict = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::DictVerify),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("odd")],
    );
    assert!(matches!(
        hybrid().run_tbc(invalid_dict),
        Err(VmError::TclError { message, .. }) if message == "missing value to go with key"
    ));

    let negative_shift = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Lshift),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("1"), lit_str("-1")],
    );
    // Tcl 9 sets no errorCode for this error, so scripts see NONE.
    assert!(matches!(
        strict().run_tbc(negative_shift),
        Err(VmError::TclError { message, error_code: None, .. })
            if message == "negative shift argument"
    ));

    let missing_unset = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::UnsetStk),
            1,
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("missing")],
    );
    assert!(matches!(
        strict().run_tbc(missing_unset),
        Err(VmError::TclError { message, error_code: Some(code), .. })
            if message == "can't unset \"missing\": no such variable"
                && code == "TCL LOOKUP VARNAME"
    ));

    assert!(matches!(
        strict().run_tbc(upvar_toplevel_bc()),
        Err(VmError::CrossRuntimeUpvar)
    ));

    let host_level = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::Push1),
            3,
            tcl86_opcode(Opcode::InvokeStk1),
            4,
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("return"),
            lit_str("-level"),
            lit_str("2"),
            lit_str("x"),
        ],
    );
    assert!(matches!(
        hybrid().run_tbc(host_level),
        Err(VmError::CrossRuntimeReturnLevel)
    ));

    let mut vm_return = vec![
        tcl86_opcode(Opcode::Push1),
        0,
        tcl86_opcode(Opcode::Push1),
        1,
        tcl86_opcode(Opcode::ReturnImm),
    ];
    vm_return.extend(0i32.to_be_bytes());
    vm_return.extend(2u32.to_be_bytes());
    vm_return.push(tcl86_opcode(Opcode::Done));
    assert!(matches!(
        strict().run_tbc(minimal_bc(vm_return, vec![lit_str("value"), lit_str("")])),
        Err(VmError::CrossRuntimeReturnLevel)
    ));

    assert!(matches!(
        strict().run_tbc(minimal_bc(vec![tcl86_opcode(Opcode::Yield)], vec![])),
        Err(VmError::UnimplementedOpcode { opcode, pc: 0 }) if opcode == "Yield"
    ));

    for (script, expected) in [
        ("upvar 1 x alias", "upvar"),
        ("uplevel 1 {set x 1}", "uplevel"),
        ("yield value", "suspension"),
        ("::yield value", "suspension"),
        ("::yieldto value", "suspension"),
    ] {
        let bytecode = minimal_bc(
            vec![
                tcl86_opcode(Opcode::Push1),
                0,
                tcl86_opcode(Opcode::EvalStk),
                tcl86_opcode(Opcode::Done),
            ],
            vec![lit_str(script)],
        );
        let error = hybrid()
            .run_tbc(bytecode)
            .expect_err("HostFrame boundary operation must fail structurally");
        assert!(
            matches!(
                (expected, error),
                ("upvar", VmError::CrossRuntimeUpvar)
                    | ("uplevel", VmError::CrossRuntimeUplevel)
                    | ("suspension", VmError::HostFrameSuspensionUnsupported)
            ),
            "unexpected HostFrame boundary error for {script}"
        );
    }
}

#[test]
fn promoted_list_index_range_membership_concat_and_lset_protocols() {
    let mut flat_code = vec![
        tcl86_opcode(Opcode::Push1),
        0, // index
        tcl86_opcode(Opcode::Push1),
        1, // replacement
        tcl86_opcode(Opcode::Push1),
        2, // old list is TOS
        tcl86_opcode(Opcode::LsetFlat),
    ];
    flat_code.extend(3u32.to_be_bytes());
    flat_code.push(tcl86_opcode(Opcode::Done));
    assert_eq!(
        result_str(
            &strict()
                .run_tbc(minimal_bc(
                    flat_code,
                    vec![lit_str("1"), lit_str("X"), lit_str("a b c")],
                ))
                .unwrap()
        ),
        "a X c"
    );

    let list_form = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0, // index list
            tcl86_opcode(Opcode::Push1),
            1, // replacement
            tcl86_opcode(Opcode::Push1),
            2, // old list
            tcl86_opcode(Opcode::LsetList),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("1 0"), lit_str("X"), lit_str("a {b c}")],
    );
    let nested = strict().run_tbc(list_form).unwrap().as_list_vec().unwrap();
    assert_eq!(nested[0], VmObj::from_str("a"));
    assert_eq!(
        nested[1]
            .as_list_vec()
            .unwrap()
            .iter()
            .map(result_str)
            .collect::<Vec<_>>(),
        ["X", "c"]
    );

    let mut range_code = vec![
        tcl86_opcode(Opcode::Push1),
        0,
        tcl86_opcode(Opcode::ListRangeImm),
    ];
    range_code.extend((-3i32).to_be_bytes()); // end-1
    range_code.extend((-2i32).to_be_bytes()); // end
    range_code.push(tcl86_opcode(Opcode::Done));
    assert_eq!(
        result_str(
            &strict()
                .run_tbc(minimal_bc(range_code, vec![lit_str("a b c")]))
                .unwrap()
        ),
        "b c"
    );

    let membership = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::ListIn),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("b"), lit_str("a b c")],
    );
    assert_eq!(strict().run_tbc(membership).unwrap().as_i64().unwrap(), 1);
}

#[test]
fn host_frame_mirror_round_trips_arrays_new_locals_unsets_and_vm_links() {
    let mirror = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0, // x
            tcl86_opcode(Opcode::Push1),
            1, // 1
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            2, // arr
            tcl86_opcode(Opcode::Push1),
            3, // k
            tcl86_opcode(Opcode::Push1),
            4, // 2
            tcl86_opcode(Opcode::StoreArrayStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            5, // eval body
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::ExistStk),
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::Push1),
            3,
            tcl86_opcode(Opcode::LoadArrayStk),
            tcl86_opcode(Opcode::Push1),
            6, // fresh
            tcl86_opcode(Opcode::LoadScalarStk),
            tcl86_opcode(Opcode::List),
            0,
            0,
            0,
            3,
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("x"),
            lit_str("1"),
            lit_str("arr"),
            lit_str("k"),
            lit_str("2"),
            lit_str("set arr(k) 9; unset x; set fresh 7; set arr(k)"),
            lit_str("fresh"),
        ],
    );
    let values = hybrid().run_tbc(mirror).unwrap().as_list_vec().unwrap();
    assert_eq!(
        values.iter().map(result_str).collect::<Vec<_>>(),
        ["0", "9", "7"]
    );

    let linked = hybrid()
        .run_tbc(upvar_host_frame_writeback_bc())
        .expect("HostFrame writeback follows Vm upvar link");
    assert_eq!(result_str(&linked), "8");

    let cross_upvar = hybrid()
        .run_tbc(host_frame_cross_level_bc("upvar 1 x alias; set alias 11"))
        .expect("HostFrame upvar resolves the VM caller, not a helper frame");
    assert_eq!(result_str(&cross_upvar), "11");

    let cross_uplevel = hybrid()
        .run_tbc(host_frame_cross_level_bc("uplevel 1 {set x 13}"))
        .expect("HostFrame uplevel evaluates in the VM caller");
    assert_eq!(result_str(&cross_uplevel), "13");

    let live_alias = hybrid()
        .run_tbc(host_frame_cross_level_bc(
            "upvar 1 x alias; uplevel 1 {set x 19}; set alias",
        ))
        .expect("HostFrame upvar alias refreshes after cross-frame uplevel");
    assert_eq!(result_str(&live_alias), "19");

    let cross_array = hybrid()
        .run_tbc(host_frame_cross_array_bc())
        .expect("HostFrame upvar preserves linked array ownership");
    assert_eq!(result_str(&cross_array), "15");
}

// ─── hand-built bytecode helpers ─────────────────────────────────────

fn minimal_bc(code_bytes: Vec<u8>, literals: Vec<LiteralObject>) -> ByteCode {
    ByteCode {
        header: None,
        info: ByteCodeInfo {
            num_commands: 0,
            num_src_bytes: 0,
            num_code_bytes: code_bytes.len() as i32,
            num_lit_objects: literals.len() as i32,
            num_except_ranges: 0,
            num_aux_data_items: 0,
            num_cmd_loc_bytes: 0,
            max_except_depth: 0,
            max_stack_depth: 16,
            code_delta_size: 0,
            code_length_size: 0,
            src_delta_size: 0,
            src_length_size: 0,
        },
        code_bytes,
        cmd_code_ranges: vec![],
        cmd_source_ranges: None,
        literals,
        exceptions: vec![],
        auxiliaries: vec![],
        proc_num_args: 0,
        proc_num_locals: 0,
        locals: vec![],
    }
}

fn tcl86_opcode(opcode: Opcode) -> u8 {
    DEFAULT_INSTRUCTION_SET
        .opcode_table
        .iter()
        .position(|candidate| *candidate == opcode)
        .and_then(|index| u8::try_from(index).ok())
        .unwrap_or_else(|| panic!("{opcode:?} is not in the Tcl 8.6 opcode table"))
}

fn tcl9_opcode(opcode: Opcode) -> u8 {
    TCL_9_INSTRUCTION_SET
        .opcode_table
        .iter()
        .position(|candidate| *candidate == opcode)
        .and_then(|index| u8::try_from(index).ok())
        .unwrap_or_else(|| panic!("{opcode:?} is not in the Tcl 9 opcode table"))
}

fn tcl9_bc(code_bytes: Vec<u8>, literals: Vec<LiteralObject>) -> ByteCode {
    let mut bytecode = minimal_bc(code_bytes, literals);
    bytecode.header = Some(Header {
        format: 3,
        build: 0,
        compiler_version: Version { major: 2, minor: 0 },
        tcl_version: Version { major: 9, minor: 0 },
    });
    bytecode
}

fn catch_completion_bc(code: i32) -> ByteCode {
    let mut bytes = vec![69, 0, 0, 0, 0]; // beginCatch4 range 0
    let protected_start = bytes.len() as i32;
    let literals = if code == 2 {
        bytes.extend([1, 0, 1, 1]); // result, empty return options
        bytes.push(98); // returnImm code=ok level=1
        bytes.extend(0i32.to_be_bytes());
        bytes.extend(1u32.to_be_bytes());
        vec![lit_str("returned"), lit_str("")]
    } else {
        bytes.push(if code == 3 { 65 } else { 66 });
        vec![]
    };
    let catch_offset = bytes.len() as i32;
    bytes.push(72); // pushReturnCode
    bytes.push(70); // endCatch
    bytes.push(0); // done

    let mut bytecode = minimal_bc(bytes, literals);
    bytecode.info.num_except_ranges = 1;
    bytecode.info.max_except_depth = 1;
    bytecode.exceptions.push(ExceptionRange {
        kind: ExceptionType::Catch,
        nesting_level: 0,
        code_offset: protected_start,
        num_code_bytes: catch_offset - protected_start,
        break_offset: -1,
        continue_offset: -1,
        catch_offset,
        code_range: protected_start..catch_offset,
    });
    bytecode
}

fn catch_host_frame_custom_code_bc() -> ByteCode {
    let mut bytes = vec![69, 0, 0, 0, 0]; // beginCatch4 range 0
    let protected_start = bytes.len() as i32;
    bytes.extend([
        tcl86_opcode(Opcode::Push1),
        0,
        tcl86_opcode(Opcode::EvalStk),
    ]);
    let catch_offset = bytes.len() as i32;
    bytes.push(tcl86_opcode(Opcode::PushReturnCode));
    bytes.push(tcl86_opcode(Opcode::EndCatch));
    bytes.push(tcl86_opcode(Opcode::Done));
    let mut bytecode = minimal_bc(bytes, vec![lit_str("return -level 0 -code 7 custom")]);
    bytecode.info.num_except_ranges = 1;
    bytecode.info.max_except_depth = 1;
    bytecode.exceptions.push(ExceptionRange {
        kind: ExceptionType::Catch,
        nesting_level: 0,
        code_offset: protected_start,
        num_code_bytes: catch_offset - protected_start,
        break_offset: -1,
        continue_offset: -1,
        catch_offset,
        code_range: protected_start..catch_offset,
    });
    bytecode
}

fn lit_str(s: &str) -> LiteralObject {
    LiteralObject {
        kind: LiteralObjectType::XString,
        value: SourceRef::Shared(s.into()),
        proc_body: None,
    }
}

fn lit_int(i: i32) -> LiteralObject {
    LiteralObject {
        kind: LiteralObjectType::Int,
        value: SourceRef::Int(i),
        proc_body: None,
    }
}

fn proc_body_lit(bytecode: ByteCode) -> LiteralObject {
    LiteralObject {
        kind: LiteralObjectType::ProcBody,
        value: SourceRef::Static("procbody"),
        proc_body: Some(tcld_core::bytecode::ProcBody { bytecode }),
    }
}

fn register_named_bodies(bodies: &[(&str, ByteCode)]) -> ByteCode {
    let mut literals = vec![lit_str("tbcload::bcproc"), lit_str("")];
    let mut code = Vec::new();
    for (name, body) in bodies {
        let name_index = literals.len() as u8;
        literals.push(lit_str(name));
        let body_index = literals.len() as u8;
        literals.push(proc_body_lit(body.clone()));
        code.extend([
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            name_index,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            body_index,
            tcl86_opcode(Opcode::InvokeStk1),
            4,
            tcl86_opcode(Opcode::Pop),
        ]);
    }
    code.extend([tcl86_opcode(Opcode::Push1), 1, tcl86_opcode(Opcode::Done)]);
    minimal_bc(code, literals)
}

fn local_slots(names: &[&str]) -> Vec<CompiledLocal> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| CompiledLocal {
            name: (*name).into(),
            default_value: None,
            frame_index: index as i32,
            flag_mask: 0,
        })
        .collect()
}

fn arith_bc() -> ByteCode {
    minimal_bc(
        vec![
            1, 0, // push1 0
            1, 1,  // push1 1
            53, // add
            0,  // done
        ],
        vec![lit_int(2), lit_int(3)],
    )
}

fn puts_bc(msg: &str) -> ByteCode {
    command_bc("puts", msg)
}

fn command_bc(command: &str, arg: &str) -> ByteCode {
    // push command; push arg; invoke 2; pop; push ok; done
    minimal_bc(
        vec![
            1, 0, // command
            1, 1, // arg
            6, 2, // invokeStk1 2
            3, // pop
            1, 2, // ok
            0, // done
        ],
        vec![lit_str(command), lit_str(arg), lit_str("ok")],
    )
}

fn set_append_bc() -> ByteCode {
    // set result {}
    // append result a
    // append result b
    // push result; loadScalarStk; done
    // lits: set, result, {}, append, a, b
    minimal_bc(
        vec![
            1, 0, // set
            1, 1, // result
            1, 2, // {}
            6, 3, // invoke set
            3, // pop
            1, 3, // append
            1, 1, // result
            1, 4, // a
            6, 3, // invoke append
            3, // pop
            1, 3, // append
            1, 1, // result
            1, 5, // b
            6, 3, // invoke append
            3, // pop
            1, 1,  // result
            12, // loadScalarStk
            0,  // done
        ],
        vec![
            lit_str("set"),
            lit_str("result"),
            lit_str(""),
            lit_str("append"),
            lit_str("a"),
            lit_str("b"),
        ],
    )
}

fn hostvar_bc() -> ByteCode {
    // Variable opcode 124 with Lvt4, then store/load
    // push name; variable lvt0; push 42; storeScalar1 0; loadScalar1 0; done
    // variable: opcode 124, 4-byte BE lvt index 0
    // storeScalar1: 17, lvt1
    // loadScalar1: 10, lvt1
    let code = vec![
        1, 0, // push "::f9host"
        124, 0, 0, 0, 0, // variable LVT0
        1, 1, // push 42
        17, 0, // storeScalar1 0
        10, 0, // loadScalar1 0
        0, // done
    ];
    let mut bc = minimal_bc(code, vec![lit_str("::f9host"), lit_str("42")]);
    bc.proc_num_locals = 1;
    bc.locals = vec![CompiledLocal {
        name: "v".into(),
        default_value: None,
        frame_index: 0,
        flag_mask: 0,
    }];
    bc
}

fn upvar_toplevel_bc() -> ByteCode {
    // push level 1; push "x"; upvar LVT0 — only one frame → CrossRuntimeUpvar
    let code = vec![
        1, 0, // push level
        1, 1, // push x
        122, 0, 0, 0, 0, // upvar LVT0 (opcode 122)
        0,
    ];
    let mut bc = minimal_bc(code, vec![lit_str("1"), lit_str("x")]);
    bc.proc_num_locals = 1;
    bc.locals = vec![CompiledLocal {
        name: "y".into(),
        default_value: None,
        frame_index: 0,
        flag_mask: 0,
    }];
    bc
}

fn upvar_host_frame_writeback_bc() -> ByteCode {
    let mut child = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0, // level 1
            tcl86_opcode(Opcode::Push1),
            1, // x
            tcl86_opcode(Opcode::Upvar),
            0,
            0,
            0,
            0, // destination LVT0 = y
            tcl86_opcode(Opcode::Push1),
            2, // set y 8
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("1"), lit_str("x"), lit_str("set y 8")],
    );
    child.proc_num_locals = 1;
    child.locals = vec![CompiledLocal {
        name: "y".into(),
        default_value: None,
        frame_index: 0,
        flag_mask: 0,
    }];

    minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0, // bcproc
            tcl86_opcode(Opcode::Push1),
            1, // child
            tcl86_opcode(Opcode::Push1),
            2, // args
            tcl86_opcode(Opcode::Push1),
            3, // body
            tcl86_opcode(Opcode::InvokeStk1),
            4,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            4, // x
            tcl86_opcode(Opcode::Push1),
            5, // 7
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            1, // child
            tcl86_opcode(Opcode::InvokeStk1),
            1,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            4, // x
            tcl86_opcode(Opcode::LoadScalarStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("tbcload::bcproc"),
            lit_str("child"),
            lit_str(""),
            proc_body_lit(child),
            lit_str("x"),
            lit_str("7"),
        ],
    )
}

fn host_frame_cross_level_bc(script: &str) -> ByteCode {
    let child = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str(script)],
    );
    minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0, // bcproc
            tcl86_opcode(Opcode::Push1),
            1, // child
            tcl86_opcode(Opcode::Push1),
            2, // args
            tcl86_opcode(Opcode::Push1),
            3, // body
            tcl86_opcode(Opcode::InvokeStk1),
            4,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            4, // x
            tcl86_opcode(Opcode::Push1),
            5, // 7
            tcl86_opcode(Opcode::StoreScalarStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            1, // child
            tcl86_opcode(Opcode::InvokeStk1),
            1,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            4, // x
            tcl86_opcode(Opcode::LoadScalarStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("tbcload::bcproc"),
            lit_str("child"),
            lit_str(""),
            proc_body_lit(child),
            lit_str("x"),
            lit_str("7"),
        ],
    )
}

fn host_frame_cross_array_bc() -> ByteCode {
    let child = minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::EvalStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![lit_str("upvar 1 arr alias; set alias(k) 15")],
    );
    minimal_bc(
        vec![
            tcl86_opcode(Opcode::Push1),
            0,
            tcl86_opcode(Opcode::Push1),
            1,
            tcl86_opcode(Opcode::Push1),
            2,
            tcl86_opcode(Opcode::Push1),
            3,
            tcl86_opcode(Opcode::InvokeStk1),
            4,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            4, // arr
            tcl86_opcode(Opcode::Push1),
            5, // k
            tcl86_opcode(Opcode::Push1),
            6, // 2
            tcl86_opcode(Opcode::StoreArrayStk),
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            1, // child
            tcl86_opcode(Opcode::InvokeStk1),
            1,
            tcl86_opcode(Opcode::Pop),
            tcl86_opcode(Opcode::Push1),
            4,
            tcl86_opcode(Opcode::Push1),
            5,
            tcl86_opcode(Opcode::LoadArrayStk),
            tcl86_opcode(Opcode::Done),
        ],
        vec![
            lit_str("tbcload::bcproc"),
            lit_str("child"),
            lit_str(""),
            proc_body_lit(child),
            lit_str("arr"),
            lit_str("k"),
            lit_str("2"),
        ],
    )
}

fn upvar_nested_bc() -> ByteCode {
    upvar_nested_bc_with_level("1")
}

fn upvar_nested_bc_with_level(level: &str) -> ByteCode {
    // Register a real nested ProcBody whose Upvar retains the level word and
    // links local `y` to top-level `x`.

    let child_code = ByteCode {
        header: None,
        info: ByteCodeInfo {
            num_commands: 0,
            num_src_bytes: 0,
            num_code_bytes: 12,
            num_lit_objects: 2,
            num_except_ranges: 0,
            num_aux_data_items: 0,
            num_cmd_loc_bytes: 0,
            max_except_depth: 0,
            max_stack_depth: 4,
            code_delta_size: 0,
            code_length_size: 0,
            src_delta_size: 0,
            src_length_size: 0,
        },
        // push1 0 (level 1); push1 1 (x); upvar lvt0; loadScalar1 0; done
        code_bytes: vec![1, 0, 1, 1, 122, 0, 0, 0, 0, 10, 0, 0],
        cmd_code_ranges: vec![],
        cmd_source_ranges: None,
        literals: vec![lit_str(level), lit_str("x")],
        exceptions: vec![],
        auxiliaries: vec![],
        proc_num_args: 0,
        proc_num_locals: 1,
        locals: vec![CompiledLocal {
            name: "y".into(),
            default_value: None,
            frame_index: 0,
            flag_mask: 0,
        }],
    };

    // Parent uses bcproc to register then call.
    // lits: tbcload::bcproc, child, args "", body(ProcBody), x, 7, childname
    let pb = tcld_core::bytecode::ProcBody {
        bytecode: child_code,
    };
    let body_lit = LiteralObject {
        kind: LiteralObjectType::ProcBody,
        value: SourceRef::Static("procbody"),
        proc_body: Some(pb),
    };

    // push bcproc; push child; push ""; push body; invoke 4; pop;
    // push x; push 7; storeScalarStk; pop;
    // push child; invoke 1; done
    let code = vec![
        1, 0, // tbcload::bcproc
        1, 1, // child
        1, 2, // args
        1, 3, // body (lit index 3)
        6, 4, // invoke 4
        3, // pop
        1, 4, // x
        1, 5,  // 7
        19, // storeScalarStk
        3,  // pop
        1, 1, // child
        6, 1, // invoke 1
        0, // done
    ];
    // Wait - push body uses Push1 with lit index; lit 3 is ProcBody — load resolves via as_proc_body. Good.
    // But Push1 operand is u8 index 3 - OK.
    // issue: lit indices: 0=bcproc, 1=child, 2=args, 3=body, 4=x, 5=7
    minimal_bc(
        code,
        vec![
            lit_str("tbcload::bcproc"),
            lit_str("child"),
            lit_str(""),
            body_lit,
            lit_str("x"),
            lit_str("7"),
        ],
    )
}
