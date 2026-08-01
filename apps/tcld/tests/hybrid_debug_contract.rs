//! Hybrid debug contracts (executable plan §9.1 / S10 / F12 / I1).

#[cfg(feature = "hybrid")]
use std::path::PathBuf;
use std::sync::atomic::Ordering;

#[cfg(feature = "hybrid")]
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[cfg(feature = "hybrid")]
fn tcl9_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testing/fixtures/bytecode/tcl9")
        .join(name)
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_debug_attach_never_decompiles() {
    use tcld::debug::{BackendOperation, DebugBackend, HybridInProcessBackend};
    use tcld_vm::RuntimeMode;

    let before = tcld::debug::DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst);
    let path = fixture("proc.tbc");
    let mut backend = HybridInProcessBackend::new(RuntimeMode::Hybrid).expect("hybrid backend");
    backend
        .start_target(&path, true)
        .expect("start_target .tbc without decompile");
    let after = tcld::debug::DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst);
    assert_eq!(
        after, before,
        "hybrid debug start_target must not call decompile_to_temp"
    );

    // Model B pause shape (F12).
    let events = backend.drain_events();
    let pauses: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            tcld::debug::DebugEvent::Paused(pause) => Some(pause.as_ref()),
            _ => None,
        })
        .collect();
    assert!(
        !pauses.is_empty(),
        "stop_on_entry should produce a pause event"
    );
    let p = &pauses[0];
    assert!(p.location.code.is_some(), "pc-primary");
    assert!(
        p.location.source_hint.is_some(),
        "top-level command source range should map into sibling proc.tcl"
    );
    assert!(p.location.code.is_some(), "PC must be code-qualified");
    assert!(
        p.location.source.is_none(),
        "a sibling file is not a canonical source map"
    );
    assert!(
        p.location.source_hint.is_some(),
        "sibling source remains available as an explicit unverified hint"
    );
    assert!(!p.in_host, "in_host=false on TEBC pause");

    backend
        .step(tcld::debug::DebugStepKind::InstructionInto)
        .expect("step_instruction advances");
    let events = backend.drain_events();
    if let Some(p) = events.iter().find_map(|event| match event {
        tcld::debug::DebugEvent::Paused(pause) => Some(pause.as_ref()),
        _ => None,
    }) {
        assert!(p.location.code.is_some());
        assert!(!p.in_host);
    }

    // capabilities
    let caps = backend.capabilities();
    assert!(
        caps.support(BackendOperation::StepInstruction)
            .is_supported()
    );
    assert!(
        !caps.support(BackendOperation::StepInto).is_supported(),
        "hybrid must not claim source-command stepping before that map is implemented"
    );
    assert!(
        caps.support(BackendOperation::StepOverInstruction)
            .is_supported()
    );
    assert!(
        caps.support(BackendOperation::StepOutInstruction)
            .is_supported()
    );
    assert!(caps.support(BackendOperation::RunToLocation).is_supported());
    assert!(
        caps.support(BackendOperation::InstructionBreakpoints)
            .is_supported()
    );
    assert!(
        caps.support(BackendOperation::DirectBytecode)
            .is_supported()
    );
    assert!(caps.support(BackendOperation::Stack).is_supported());
    assert!(caps.support(BackendOperation::Evaluate).is_supported());
    assert!(caps.support(BackendOperation::Watches).is_supported());
    assert!(
        caps.support(BackendOperation::DisassemblyAccess)
            .is_supported()
    );

    let result = backend
        .evaluate(0, "set hybrid_debug_probe 41")
        .expect("eval in suspended frame");
    assert_eq!((result.completion_code, result.result.as_str()), (0, "41"));
    let result = backend
        .evaluate(0, "expr {$hybrid_debug_probe + 1}")
        .expect("expr in suspended frame");
    assert_eq!((result.completion_code, result.result.as_str()), (0, "42"));

    Box::new(backend).close();
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_neutral_contract_exposes_typed_events_locations_and_breakpoints() {
    use tcld::debug::{
        BackendError, CapabilitySupport, DebugBackend, DebugBreakpointResolution,
        DebugBreakpointSpec, DebugEvent, DebugLifecycleState, DebugStepKind,
        HybridInProcessBackend,
    };
    use tcld_vm::RuntimeMode;

    let path = fixture("proc.tbc");
    let source_path = path.with_extension("tcl");
    let mut backend = HybridInProcessBackend::new(RuntimeMode::Hybrid).expect("hybrid backend");
    backend.start_target(&path, true).expect("start target");

    let events = backend.drain_events();
    assert!(events.iter().any(|event| {
        matches!(event, DebugEvent::CompanionSourceDiscovered(source) if source.path == source_path)
    }));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DebugEvent::SourceRegistered(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, DebugEvent::CodeRegistered(_)))
    );
    let pause = events
        .iter()
        .find_map(|event| match event {
            DebugEvent::Paused(pause) => Some(pause),
            _ => None,
        })
        .expect("typed pause event");
    assert!(pause.location.source.is_none());
    let source_hint = pause
        .location
        .source_hint
        .as_ref()
        .expect("companion-source hint");
    let code = pause.location.code.expect("qualified code coordinate");
    assert_eq!(source_hint.path, source_path);
    assert_eq!(pause.location.observed_pc, Some(code.pc));

    let breakpoint = backend
        .add_breakpoint(DebugBreakpointSpec::SourceLine {
            path: source_path,
            line: source_hint.line,
            condition: None,
        })
        .expect("add neutral source breakpoint");
    assert!(matches!(
        breakpoint.resolution,
        DebugBreakpointResolution::Pending { ref reason }
            if reason.contains("unverified companion source")
    ));
    assert_eq!(backend.breakpoints().expect("list breakpoints").len(), 1);
    backend
        .set_breakpoint_enabled(&breakpoint.id, false)
        .expect("disable breakpoint");
    assert!(!backend.breakpoints().unwrap()[0].enabled);
    backend
        .remove_breakpoint(&breakpoint.id)
        .expect("remove breakpoint");
    assert!(backend.breakpoints().unwrap().is_empty());

    let instruction_breakpoint = backend
        .add_breakpoint(DebugBreakpointSpec::Instruction {
            location: code,
            condition: None,
        })
        .expect("add code-qualified breakpoint");
    assert!(matches!(
        instruction_breakpoint.resolution,
        DebugBreakpointResolution::Resolved { .. }
    ));
    backend
        .restart_target(true)
        .expect("restart into a fresh hybrid engine");
    let restored = backend.breakpoints().expect("breakpoints after restart");
    assert_eq!(restored.len(), 1, "entry-code breakpoint intent survives");
    assert!(matches!(
        restored[0].spec,
        DebugBreakpointSpec::Instruction { location, .. } if location.pc == code.pc
    ));

    assert_eq!(backend.state().unwrap(), DebugLifecycleState::Paused);
    assert!(backend.sources().unwrap().is_empty());
    assert_eq!(backend.companion_sources().unwrap().len(), 1);
    assert_eq!(backend.disassembly().unwrap().len(), 1);
    assert!(matches!(
        backend
            .capabilities()
            .support(tcld::debug::BackendOperation::StepInto),
        CapabilitySupport::Unsupported { reason } if reason.contains("source-command")
    ));
    assert!(matches!(
        backend.step(DebugStepKind::Into),
        Err(BackendError::Unsupported { op: "source_step", reason })
            if reason.contains("instruction stepping")
    ));
}

#[test]
#[cfg(feature = "hybrid")]
fn arbitrary_sibling_source_never_becomes_a_canonical_hybrid_location() {
    use tcld::debug::{DebugBackend, DebugEvent, HybridInProcessBackend};
    use tcld_vm::RuntimeMode;

    let temp = tempfile::tempdir().expect("temp dir");
    let bytecode = temp.path().join("stale.tbc");
    let sibling = temp.path().join("stale.tcl");
    std::fs::copy(fixture("proc.tbc"), &bytecode).expect("copy bytecode");
    std::fs::write(
        &sibling,
        "# unrelated and stale\nproc definitely_not_the_bytecode {} { error stale }\n",
    )
    .expect("write arbitrary sibling");

    let mut backend = HybridInProcessBackend::new(RuntimeMode::Hybrid).expect("hybrid backend");
    backend
        .start_target(&bytecode, true)
        .expect("start copied bytecode");
    let events = backend.drain_events();
    let pause = events
        .iter()
        .find_map(|event| match event {
            DebugEvent::Paused(pause) => Some(pause),
            _ => None,
        })
        .expect("pause");
    assert!(pause.location.source.is_none());
    assert_eq!(
        pause
            .location
            .source_hint
            .as_ref()
            .map(|hint| hint.path.as_path()),
        Some(sibling.as_path())
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DebugEvent::SourceRegistered(_)))
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_debug_backend_source_has_no_decompile_path() {
    let backend_src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/backend.rs"));
    assert!(
        !backend_src.contains("decompile_to_temp"),
        "HybridInProcessBackend module must not mention decompile_to_temp"
    );
    let hybrid_debug = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/mod.rs"));
    // run_hybrid_debug must not call decompile_to_temp; host prepare_debug_target may.
    assert!(
        hybrid_debug.contains("run_hybrid_debug"),
        "expected run_hybrid_debug entry"
    );
    assert!(
        hybrid_debug.contains("DebugApp::with_backend"),
        "hybrid must enter the canonical debugger app"
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_disassembly_maps_pc_for_source_pane() {
    use tcld::debug::{BackendError, DebugBackend, HybridInProcessBackend};
    use tcld_vm::RuntimeMode;

    let path = fixture("proc.tbc");
    let mut backend = HybridInProcessBackend::new(RuntimeMode::VmStrict).expect("backend");
    backend.start_target(&path, true).expect("start");
    assert!(
        !backend.disassembly().unwrap()[0].lines.is_empty(),
        "disassembly source pane must load"
    );
    assert!(
        !backend.companion_sources().unwrap()[0].text.is_empty(),
        "sibling source pane must load when proc.tcl exists"
    );
    let caps = backend.capabilities();
    assert!(
        !caps
            .support(tcld::debug::BackendOperation::Evaluate)
            .is_supported()
    );
    assert!(
        !caps
            .support(tcld::debug::BackendOperation::Watches)
            .is_supported()
    );
    assert!(
        caps.support(tcld::debug::BackendOperation::Evaluate)
            .reason()
            .is_some_and(|reason| reason.contains("vm-strict"))
    );
    assert!(matches!(
        backend.evaluate(0, "set forbidden 1"),
        Err(BackendError::Unsupported { op: "evaluate", reason })
            if reason.contains("vm-strict")
    ));
    let events = backend.drain_events();
    let pc = events
        .iter()
        .find_map(|event| match event {
            tcld::debug::DebugEvent::Paused(pause) => {
                pause.location.code.map(|location| location.pc)
            }
            _ => None,
        })
        .expect("entry pc");
    assert!(
        backend.disassembly().unwrap()[0]
            .lines
            .iter()
            .any(|line| line.contains(&format!("({pc})"))),
        "pc {pc} should map to a disassembly line"
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_debug_disassembly_uses_the_tcl9_opcode_assignment() {
    use tcld::debug::{DebugBackend, HybridInProcessBackend};
    use tcld_vm::RuntimeMode;

    let path = tcl9_fixture("tc1.tbc");
    let mut backend = HybridInProcessBackend::new(RuntimeMode::VmStrict).expect("backend");
    backend.start_target(&path, true).expect("Tcl 9 target");
    let disassembly_snapshot = backend.disassembly().expect("disassembly").remove(0);
    let disassembly = disassembly_snapshot.lines.join("\n");

    assert_eq!(
        disassembly_snapshot.instruction_set.as_deref(),
        Some("tcl9.0")
    );
    assert!(
        disassembly.contains("(11) returnImm 0 1"),
        "Tcl 9 byte 92 must map to returnImm:\n{disassembly}"
    );
    assert!(
        !disassembly.contains("lappendArrayStk"),
        "Tcl 8.6 byte 92 meaning leaked into Tcl 9 disassembly:\n{disassembly}"
    );
}

#[test]
fn host_path_still_owns_decompile_for_tbc() {
    // Host path keeps decompile_to_temp for .tbc; hybrid never uses it.
    let debug_mod = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/mod.rs"));
    let planning = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/debug/planning.rs"
    ));
    assert!(
        planning.contains("decompile_to_temp"),
        "host prepare_debug_target must retain decompile_to_temp"
    );
    assert!(
        debug_mod.contains("plan_debug") && planning.contains("DebugBackendPolicy::Host"),
        "central debug planner must allow --backend=host"
    );
    // Counter is public so hybrid I1 tests can observe it.
    let before = tcld::debug::DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst);
    let _ = before;
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_debug_uses_the_canonical_disassembly_capable_app() {
    let app = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/app.rs"));
    let module = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/mod.rs"));
    let render = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/tcld-tui/src/debug/render.rs"
    ));
    assert!(
        app.contains("BackendOperation::DisassemblyAccess")
            && app.contains("unverified companion source")
            && render.contains("Source · companion (unverified)"),
        "canonical app must present disassembly primary and mark companion source unverified"
    );
    assert!(
        app.contains("DebugStepKind::InstructionInto")
            && app.contains("DebugStepKind::InstructionOver"),
        "canonical app must adapt coherent step actions to instruction stepping"
    );
    assert!(
        module.contains("DebugApp::with_backend") && !module.contains("mod hybrid_app"),
        "hybrid and host must share one terminal application entry"
    );
}
