//! `tcld debug` — interactive source-level debugger TUI.
//!
//! Connects the dedicated debugger engine (`tcld-debug-state` +
//! `tcld-debug-wire` driving the `tcld-dbg-ext` nub inside `tclsh`)
//! to a focused ratatui frontend with source view, call stack,
//! locals, breakpoints, watches, and a rolling event log.

mod app;
mod backend;
mod config;
mod contract;
mod planning;
mod session;

#[cfg(feature = "hybrid")]
pub use backend::HybridInProcessBackend;
pub use backend::{BackendError, DebugBackend, WireSessionBackend};
pub use contract::{
    BackendCapabilities, BackendOperation, BackendPause, CapabilitySupport, CompanionSource,
    CompanionSourceLocation, CompanionSourceSnapshot, DebugBreakpoint, DebugBreakpointId,
    DebugBreakpointResolution, DebugBreakpointSpec, DebugCodeId, DebugCodeLocation,
    DebugDiagnosticSeverity, DebugEvaluation, DebugEvent, DebugLifecycleState, DebugLocation,
    DebugOutputStream, DebugPauseReason, DebugProcedure, DebugSourceUnitIdentity, DebugStackFrame,
    DebugStepKind, DebugVariable, DisassemblySnapshot, RegisteredCode, RegisteredSource,
    SourceCommandSpan, SourceSnapshot,
};

use std::io::IsTerminal;
use std::path::PathBuf;

use crate::cli::DebugArgs;
use crate::error::{Result, TcldError};

pub use app::DebugApp;
pub use config::{NubPaths, TclAbi, locate_paths, locate_paths_for_target};
pub use planning::{
    DECOMPILE_TO_TEMP_CALLS, DebugBackendPolicy, DebugExecutionBackend, DebugPlan, DebugRequest,
    DebugTargetKind, PreparedDebugTarget, TargetProvenance, plan_debug, prepare_debug_target,
};
pub use session::{ProcEntry, Session, format_event};

pub fn run_debug(args: DebugArgs) -> Result<()> {
    let raw_path = PathBuf::from(&args.input_path);
    config::ensure_user_script(&raw_path).map_err(TcldError::Other)?;

    // Plan before entering terminal/runtime setup. This makes explicit backend
    // conflicts observable and ensures they can never be silently replaced by
    // whichever backend happens to be linked into this binary.
    let request = DebugRequest::new(raw_path.clone(), DebugBackendPolicy::from(args.backend));
    let plan = plan_debug(request, cfg!(feature = "hybrid")).map_err(TcldError::Other)?;

    // Materialize (including source compiler admission) before terminal entry,
    // so syntax/unsupported diagnostics are useful in scripts and CI too.
    let target = prepare_debug_target(plan).map_err(TcldError::Other)?;

    let is_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !is_tty {
        return Err(TcldError::Other(
            "tcld debug requires an interactive terminal".to_string(),
        ));
    }

    #[cfg(feature = "hybrid")]
    if matches!(
        target.plan.backend,
        DebugExecutionBackend::Hybrid | DebugExecutionBackend::VmStrict
    ) {
        return run_hybrid_debug(args, target);
    }

    // Host-shell path: source is used directly; bytecode is an exact owned
    // reconstruction selected by the planner.
    let paths = locate_paths_for_target(
        args.tclsh.as_deref(),
        args.dylib.as_deref(),
        args.bootstrap.as_deref(),
        target.required_tcl_abi,
    )
    .map_err(TcldError::Other)?;
    if let Some(warning) = &paths.selection_warning {
        eprintln!("{warning}");
    }

    let theme = tcld_tui::Theme::for_terminal(args.no_colors, args.theme.as_deref())
        .map_err(TcldError::Other)?;

    let app = DebugApp::new(target, paths, theme).map_err(|e| TcldError::Other(e.to_string()))?;
    app.run().map_err(|e| TcldError::Other(e.to_string()))?;
    Ok(())
}

/// Hybrid in-process debug for raw `.tbc` or a compiler-owned Tcl source
/// image; neither route executes decompiled temporary source (I1).
#[cfg(feature = "hybrid")]
fn run_hybrid_debug(args: DebugArgs, target: PreparedDebugTarget) -> Result<()> {
    use tcld_vm::RuntimeMode;

    let mode = match target.plan.backend {
        DebugExecutionBackend::VmStrict => RuntimeMode::VmStrict,
        DebugExecutionBackend::Hybrid => RuntimeMode::Hybrid,
        DebugExecutionBackend::Host => unreachable!("host plan cannot enter hybrid runner"),
    };

    let theme = tcld_tui::Theme::for_terminal(args.no_colors, args.theme.as_deref())
        .map_err(TcldError::Other)?;

    let backend = match target.compiled_image.clone() {
        Some(image) => HybridInProcessBackend::new_with_image(mode, image),
        None => HybridInProcessBackend::new(mode),
    }
    .map_err(|error| TcldError::Other(error.to_string()))?;
    let label = if target.compiled_image.is_some() {
        format!("compiled Tcl source ({mode:?})")
    } else {
        format!("hybrid TEBC ({mode:?})")
    };
    let app = DebugApp::with_backend(target, Box::new(backend), label, theme)
        .map_err(|error| TcldError::Other(error.to_string()))?;
    app.run()
        .map_err(|error| TcldError::Other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn host_plan(path: &Path) -> DebugPlan {
        plan_debug(
            DebugRequest::new(path, DebugBackendPolicy::Host),
            cfg!(feature = "hybrid"),
        )
        .expect("host plan")
    }

    #[test]
    fn a_host_decompile_failure_names_a_way_forward() {
        let message = planning::host_decompile_failure(
            Path::new("testing/fixtures/public/public_complex_fixture.tbc"),
            "strict decompilation failed: nested proc body could not be decompiled",
        );

        assert!(
            message.contains("testing/fixtures/public/public_complex_fixture.tbc"),
            "names the artifact: {message}"
        );
        assert!(
            message.contains("nested proc body"),
            "keeps the underlying reason: {message}"
        );
        assert!(
            message.contains("tcld explore"),
            "offers inspection without execution: {message}"
        );
        assert!(
            message.contains("hybrid"),
            "points at the backend that does not decompile: {message}"
        );
        assert!(
            !message.contains("placeholder bodies are fine"),
            "never suggests an approximation fallback: {message}"
        );
    }

    #[test]
    fn bytecode_target_carries_tcl_abi_and_decompile_provenance() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/expr.tbc")
            .canonicalize()
            .unwrap();
        let target =
            prepare_debug_target(host_plan(&fixture)).expect("prepare bytecode debug target");
        assert_eq!(target.required_tcl_abi, Some(TclAbi::Tcl86));
        assert_eq!(target.provenance.display_path, fixture);
        assert_eq!(target.provenance.decompiled_from.as_ref(), Some(&fixture));
        assert_eq!(
            target.provenance.bytecode_tcl_version.as_deref(),
            Some("8.0")
        );
        assert!(target.provenance.runtime_path.is_file());
        assert_eq!(
            target
                .provenance
                .runtime_path
                .extension()
                .and_then(|value| value.to_str()),
            Some("tcl")
        );
    }

    #[test]
    fn public_fixture_prepares_for_host_debug_without_approximated_proc_bodies() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/public/public_complex_fixture.tbc")
            .canonicalize()
            .unwrap();
        let target = prepare_debug_target(host_plan(&fixture))
            .expect("prepare public fixture for host debug");
        let source = std::fs::read_to_string(&target.provenance.runtime_path)
            .expect("read decompiled host source");

        assert_eq!(target.required_tcl_abi, Some(TclAbi::Tcl86));
        assert_eq!(target.provenance.decompiled_from.as_ref(), Some(&fixture));
        assert!(source.contains("proc fixture::startlog"));
        assert!(source.contains("proc fixture::logLevelInit"));
        assert!(!source.contains("tcld: unavailable nested bytecode"));
    }

    #[test]
    fn source_target_has_no_bytecode_abi_requirement() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hello.tcl")
            .canonicalize()
            .unwrap();
        let target = prepare_debug_target(host_plan(&fixture)).expect("prepare Tcl source target");
        assert_eq!(target.required_tcl_abi, None);
        assert_eq!(target.provenance.runtime_path, fixture);
        assert!(target.provenance.decompiled_from.is_none());
    }
}
