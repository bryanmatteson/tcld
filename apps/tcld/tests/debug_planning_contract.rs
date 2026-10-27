//! CLI-level contracts for first-class Tcl source routing.
//!
//! These commands intentionally run without a PTY. Planning occurs before the
//! terminal gate, so an explicit backend conflict must be reported directly;
//! a compatible route reaches the ordinary interactive-terminal diagnostic.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::Ordering;

use tcld::debug::{
    DECOMPILE_TO_TEMP_CALLS, DebugBackendPolicy, DebugRequest, plan_debug, prepare_debug_target,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run_debug(path: &Path, backend: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tcld"))
        .args(["debug", "--backend", backend])
        .arg(path)
        .output()
        .expect("run tcld debug")
}

#[test]
#[cfg(not(feature = "hybrid"))]
fn explicit_source_vm_backends_report_unavailable_when_not_linked() {
    let source = fixture("hello.tcl");
    for backend in ["hybrid", "vm-strict"] {
        let output = run_debug(&source, backend);
        assert!(!output.status.success(), "backend={backend}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.starts_with("hybrid_unavailable:"),
            "backend={backend}: {stderr}"
        );
        assert_eq!(output.status.code(), Some(3), "backend={backend}: {stderr}");
        assert!(
            stderr.contains("--features hybrid"),
            "backend={backend}: {stderr}"
        );
        assert!(
            !stderr.contains("requires an interactive terminal"),
            "backend conflict must not be masked by the TTY gate: {stderr}"
        );
    }
}

#[test]
#[cfg(feature = "hybrid")]
fn explicit_source_vm_backends_compile_before_the_terminal_gate() {
    let source = fixture("hello.tcl");
    let hybrid = run_debug(&source, "hybrid");
    assert!(String::from_utf8_lossy(&hybrid.stderr).contains("requires an interactive terminal"));
    let dir = tempfile::tempdir().unwrap();
    let unsupported = dir.path().join("unsupported.tcl");
    std::fs::write(&unsupported, "exec ls\n").unwrap();
    let strict = run_debug(&unsupported, "vm-strict");
    let stderr = String::from_utf8_lossy(&strict.stderr);
    assert!(stderr.starts_with("source_compile_failed:"), "{stderr}");
    assert!(stderr.contains("WorldHost"), "{stderr}");
    assert!(stderr.contains("--backend=host"), "{stderr}");
    assert!(
        !stderr.contains("interactive terminal"),
        "compile admission precedes TTY: {stderr}"
    );

    // A static `namespace eval` body compiles into the image.
    let namespace = dir.path().join("namespace-eval.tcl");
    std::fs::write(&namespace, "namespace eval hidden {set value 1}\n").unwrap();
    let stderr = String::from_utf8_lossy(&run_debug(&namespace, "hybrid").stderr).into_owned();
    assert!(
        stderr.contains("requires an interactive terminal"),
        "{stderr}"
    );

    for (filename, source, expected) in [
        (
            "package-require.tcl",
            "package require hidden\n",
            "package require",
        ),
        (
            "oo-class-new.tcl",
            "oo::class new {method x {} {}}\n",
            "oo::class new",
        ),
    ] {
        let evaluator = dir.path().join(filename);
        std::fs::write(&evaluator, source).unwrap();
        let hybrid = run_debug(&evaluator, "hybrid");
        let stderr = String::from_utf8_lossy(&hybrid.stderr);
        assert!(stderr.starts_with("source_compile_failed:"), "{stderr}");
        assert!(stderr.contains(expected), "{expected}: {stderr}");
        assert!(stderr.contains("outside the debug image"), "{stderr}");
        assert!(stderr.contains("--backend=host"), "{stderr}");
        assert!(!stderr.contains("interactive terminal"), "{stderr}");
    }
}

#[test]
fn source_auto_and_host_reach_the_real_host_debug_journey() {
    let source = fixture("hello.tcl");
    for backend in ["auto", "host"] {
        let output = run_debug(&source, backend);
        assert!(!output.status.success(), "non-PTY invocation should stop");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("tcld debug requires an interactive terminal"),
            "compatible source route should reach the debugger terminal gate: {stderr}"
        );
        assert!(!stderr.contains("source_hybrid_unimplemented"), "{stderr}");
        assert!(!stderr.contains("hybrid_unavailable"), "{stderr}");
    }
}

#[test]
fn source_paths_with_spaces_are_one_first_class_debug_argument() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source target with spaces.tcl");
    std::fs::write(&source, "set value 1\n").unwrap();

    let output = run_debug(&source, "auto");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("tcld debug requires an interactive terminal"),
        "a structured path argument must reach the normal host journey: {stderr}"
    );
    assert!(!stderr.contains("file not found"), "{stderr}");
}

#[test]
#[cfg(not(feature = "hybrid"))]
fn requested_bytecode_vm_backend_reports_hybrid_unavailable() {
    let bytecode = fixture("expr.tbc");
    for backend in ["hybrid", "vm-strict"] {
        let output = run_debug(&bytecode, backend);
        assert!(!output.status.success(), "backend={backend}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with("hybrid_unavailable:"), "{stderr}");
        assert_eq!(output.status.code(), Some(3), "backend={backend}: {stderr}");
        assert!(stderr.contains("--features hybrid"), "{stderr}");
        assert!(
            !stderr.contains("requires an interactive terminal"),
            "unavailable explicit backend must not be masked: {stderr}"
        );
    }
}

#[test]
#[cfg(not(feature = "hybrid"))]
fn bytecode_auto_remains_the_supported_exact_host_route_without_hybrid() {
    let output = run_debug(&fixture("expr.tbc"), "auto");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("tcld debug requires an interactive terminal"),
        "bytecode auto should plan host rather than fail hybrid availability: {stderr}"
    );
    assert!(!stderr.contains("hybrid_unavailable"), "{stderr}");
}

#[test]
fn source_and_hybrid_plans_do_not_call_host_reconstruction() {
    let before = DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst);
    let source = fixture("hello.tcl");
    prepare_debug_target(
        plan_debug(
            DebugRequest::new(source, DebugBackendPolicy::Auto),
            cfg!(feature = "hybrid"),
        )
        .expect("source auto plan"),
    )
    .expect("prepare source");

    let bytecode = fixture("expr.tbc");
    prepare_debug_target(
        plan_debug(
            DebugRequest::new(bytecode, DebugBackendPolicy::Hybrid),
            true,
        )
        .expect("hybrid bytecode plan"),
    )
    .expect("prepare direct bytecode");

    assert_eq!(
        DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst),
        before,
        "source and hybrid preparation must never reconstruct Tcl"
    );
}
