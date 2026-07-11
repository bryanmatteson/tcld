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
    use tcld::debug::{DebugBackend, HybridInProcessBackend};
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
    let pauses = backend.drain_pauses();
    assert!(
        !pauses.is_empty(),
        "stop_on_entry should produce a pause event"
    );
    let p = &pauses[0];
    assert!(p.pc.is_some(), "pc-primary");
    assert!(
        p.source_line.is_some(),
        "top-level command source range should map into sibling proc.tcl"
    );
    assert!(p.location.is_none(), "Location=None");
    assert!(!p.in_host, "in_host=false on TEBC pause");

    let source_addresses = backend
        .toggle_source_breakpoint(p.source_line.expect("mapped source line"))
        .expect("source breakpoint resolution");
    assert!(
        !source_addresses.is_empty(),
        "mapped source line must resolve to qualified command addresses"
    );
    let current_code = backend.current_code_id().expect("current code");
    assert!(
        source_addresses
            .iter()
            .all(|address| address.code_id == current_code)
    );
    let command_address = backend
        .add_command_breakpoint(0)
        .expect("command breakpoint resolution");
    assert_eq!(command_address.code_id, current_code);
    assert!(backend.qualified_breakpoints().contains(&command_address));

    backend
        .step_instruction()
        .expect("step_instruction advances");
    let pauses = backend.drain_pauses();
    if let Some(p) = pauses.first() {
        assert!(p.location.is_none());
        assert!(!p.in_host);
    }

    // capabilities
    let caps = backend.capabilities();
    assert!(caps.instruction_step);
    assert!(caps.nested_step);
    assert!(caps.step_over);
    assert!(caps.step_out);
    assert!(caps.run_to_pc);
    assert!(caps.code_qualified_breakpoints);
    assert!(caps.bytecode_direct);
    assert!(caps.nested_frames);
    assert!(caps.eval_in_frame);
    assert!(caps.watches);
    assert!(caps.source_disassembly);

    let (code, value) = backend
        .eval_in_frame(0, "set hybrid_debug_probe 41")
        .expect("eval in suspended frame");
    assert_eq!((code, value.as_str()), (0, "41"));
    let (code, value) = backend
        .expr_in_frame(0, "$hybrid_debug_probe + 1")
        .expect("expr in suspended frame");
    assert_eq!((code, value.as_str()), (0, "42"));

    Box::new(backend).close();
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_debug_backend_source_has_no_decompile_path() {
    let backend_src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/backend.rs"));
    assert!(
        !backend_src.contains("decompile_to_temp"),
        "HybridInProcessBackend module must not mention decompile_to_temp"
    );
    let hybrid_app = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/debug/hybrid_app.rs"
    ));
    assert!(
        !hybrid_app.contains("decompile_to_temp"),
        "hybrid TUI must not mention decompile_to_temp"
    );
    let hybrid_debug = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/mod.rs"));
    // run_hybrid_debug must not call decompile_to_temp; host prepare_debug_target may.
    assert!(
        hybrid_debug.contains("run_hybrid_debug"),
        "expected run_hybrid_debug entry"
    );
    assert!(
        hybrid_debug.contains("run_hybrid_debug_tui") || hybrid_app.contains("HybridDebugApp"),
        "expected hybrid TUI dual path"
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_disassembly_maps_pc_for_source_pane() {
    use tcld::debug::{DebugBackend, HybridInProcessBackend};
    use tcld_vm::RuntimeMode;

    let path = fixture("proc.tbc");
    let mut backend = HybridInProcessBackend::new(RuntimeMode::VmStrict).expect("backend");
    backend.start_target(&path, true).expect("start");
    assert!(
        !backend.disassembly_lines().is_empty(),
        "disassembly source pane must load"
    );
    assert!(
        !backend.source_lines().is_empty(),
        "sibling source pane must load when proc.tcl exists"
    );
    let caps = backend.capabilities();
    assert!(!caps.eval_in_frame);
    assert!(!caps.watches);
    assert_eq!(
        caps.dynamic_eval_reason.as_deref(),
        Some("dynamic_eval denied by vm-strict")
    );
    let pauses = backend.drain_pauses();
    let pc = pauses.first().and_then(|p| p.pc).expect("entry pc");
    assert!(
        backend.line_for_pc(pc).is_some(),
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
    let disassembly = backend.disassembly_lines().join("\n");

    assert_eq!(backend.instruction_set_name(), Some("tcl9.0"));
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
    assert!(
        debug_mod.contains("decompile_to_temp"),
        "host prepare_debug_target must retain decompile_to_temp"
    );
    assert!(
        debug_mod.contains("force_host") || debug_mod.contains("RunBackend::Host"),
        "debug router must allow --backend=host"
    );
    // Counter is public so hybrid I1 tests can observe it.
    let before = tcld::debug::DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst);
    let _ = before;
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_debug_tui_entry_is_disassembly_primary() {
    let hybrid_app = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/debug/hybrid_app.rs"
    ));
    assert!(
        hybrid_app.contains("SourceOrigin::Synthetic") || hybrid_app.contains("disassembly"),
        "hybrid TUI must present disassembly-primary source"
    );
    assert!(
        hybrid_app.contains("step_instruction") || hybrid_app.contains("do_step"),
        "hybrid TUI must wire instruction step"
    );
    assert!(
        hybrid_app.contains("HybridDebugApp") && hybrid_app.contains("run_hybrid_debug_tui"),
        "hybrid TUI entry must exist"
    );
}
