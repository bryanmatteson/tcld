//! `tcld debug` — interactive source-level debugger TUI.
//!
//! Connects the dedicated debugger engine (`tcld-debug-state` +
//! `tcld-debug-wire` driving the `tcld-dbg-ext` nub inside `tclsh`)
//! to a focused ratatui frontend with source view, call stack,
//! locals, breakpoints, watches, and a rolling event log.

mod app;
mod backend;
mod config;
#[cfg(feature = "hybrid")]
mod hybrid_app;
mod session;

#[cfg(feature = "hybrid")]
pub use backend::HybridInProcessBackend;
pub use backend::{
    BackendCapabilities, BackendError, BackendPause, DebugBackend, WireSessionBackend,
};

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::cli::DebugArgs;
use crate::error::{Result, TcldError};

pub use app::{DebugApp, DebugTarget, SourceProvenance};
pub use config::{NubPaths, TclAbi, locate_paths, locate_paths_for_target};
pub use session::{ProcEntry, Session, format_event};

pub fn run_debug(args: DebugArgs) -> Result<()> {
    let raw_path = PathBuf::from(&args.input_path);
    config::ensure_user_script(&raw_path).map_err(TcldError::Other)?;

    let is_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !is_tty {
        return Err(TcldError::Other(
            "tcld debug requires an interactive terminal".to_string(),
        ));
    }

    let is_tbc = is_bytecode_path(&raw_path);
    let force_host = matches!(args.backend, Some(crate::cli::RunBackend::Host));
    let force_hybrid = matches!(
        args.backend,
        Some(crate::cli::RunBackend::Hybrid | crate::cli::RunBackend::VmStrict)
    );

    // Hybrid is the default for `.tbc` when the feature is linked (plan §2.1).
    #[cfg(feature = "hybrid")]
    if is_tbc && !force_host {
        return run_hybrid_debug(args, &raw_path);
    }
    #[cfg(not(feature = "hybrid"))]
    {
        let _ = force_host;
        if is_tbc && force_hybrid {
            return Err(TcldError::Other(
                "hybrid_unavailable: rebuild tcld with --features hybrid".into(),
            ));
        }
    }
    #[cfg(feature = "hybrid")]
    let _ = force_hybrid;

    // Host-shell path (may decompile `.tbc` to temp source).
    let target = prepare_debug_target(&raw_path).map_err(TcldError::Other)?;
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

/// Hybrid in-process debug for `.tbc` — never decompiles to a temp script (I1).
#[cfg(feature = "hybrid")]
fn run_hybrid_debug(args: DebugArgs, path: &Path) -> Result<()> {
    use tcld_vm::RuntimeMode;

    let mode = match args.backend {
        Some(crate::cli::RunBackend::VmStrict) => RuntimeMode::VmStrict,
        _ => RuntimeMode::Hybrid,
    };

    let theme = tcld_tui::Theme::for_terminal(args.no_colors, args.theme.as_deref())
        .map_err(TcldError::Other)?;

    hybrid_app::run_hybrid_debug_tui(path, mode, theme).map_err(|e| TcldError::Other(e.to_string()))
}

fn is_bytecode_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tbc"))
}

fn prepare_debug_target(path: &Path) -> std::result::Result<DebugTarget, String> {
    if !is_bytecode_path(path) {
        return Ok(DebugTarget::user_script(path.to_path_buf()));
    }
    decompile_to_temp(path)
}

/// Call counter for hybrid I1 verification (must stay 0 on hybrid run/debug).
pub static DECOMPILE_TO_TEMP_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn decompile_to_temp(path: &Path) -> std::result::Result<DebugTarget, String> {
    DECOMPILE_TO_TEMP_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let path_str = path
        .to_str()
        .ok_or_else(|| "input path is not valid UTF-8".to_string())?;
    let loaded = tcld_decompile::load_bytecode(tcld_decompile::LoadInput::path(path_str))
        .map_err(|error| format!("decode bytecode metadata: {error}"))?;
    let header = loaded
        .bytecode
        .header
        .as_ref()
        .ok_or_else(|| "decoded bytecode has no Tcl version header".to_string())?;
    let bytecode_tcl_version = format!("{}.{}", header.tcl_version.major, header.tcl_version.minor);
    let required_tcl_abi = if header.tcl_version.major >= 9 {
        TclAbi::Tcl9
    } else {
        TclAbi::Tcl86
    };
    // Host debugging executes the reconstruction in tclsh. Require an exact
    // result so the running program matches the input artifact.
    let source = tcld_decompile::decompile_loaded(&loaded)
        .map_err(|error| host_decompile_failure(path, &error.to_string()))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("bytecode");
    // `tempfile::TempDir` allocates a uniquely-named directory under
    // the platform temp root (with secure permissions on Unix) and
    // unlinks it on Drop. Using a dir lets us give the file a stable,
    // human-meaningful basename while still benefiting from automatic
    // cleanup.
    let dir = tempfile::Builder::new()
        .prefix("tcld-debug-")
        .tempdir()
        .map_err(|e| format!("create temp dir: {e}"))?;
    let target = dir.path().join(format!("{stem}.tcl"));
    std::fs::write(&target, source).map_err(|e| format!("write {}: {e}", target.display()))?;
    let provenance = SourceProvenance::decompiled_bytecode(
        target,
        path.to_path_buf(),
        bytecode_tcl_version,
        dir,
    );
    Ok(DebugTarget::new(provenance, Some(required_tcl_abi)))
}

/// Describe a host reconstruction failure and the available alternatives.
fn host_decompile_failure(path: &Path, error: &str) -> String {
    let display = path.display();
    let mut message = format!(
        "Host debugging requires an exact Tcl reconstruction of {display}.\n\n\
         Reconstruction failed: {error}\n\n\
         The host backend executes reconstructed source in tclsh. It does not \
         run an approximate reconstruction because that program may differ from \
         the input.\n\n\
         Available paths:"
    );
    if cfg!(feature = "hybrid") {
        message
            .push_str("\n  - tcld debug --backend hybrid <file>   debug the bytecode in process");
    } else {
        message.push_str(
            "\n  - build the in-process VM:\n      \
             cargo build -p tcld --features hybrid",
        );
    }
    message.push_str(&format!(
        "\n  - debug the original .tcl source\n  \
         - tcld explore {display}   inspect without executing"
    ));
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_decompile_failure_names_a_way_forward() {
        let message = host_decompile_failure(
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
        let target = prepare_debug_target(&fixture).expect("prepare bytecode debug target");
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
        let target = prepare_debug_target(&fixture).expect("prepare public fixture for host debug");
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
        let target = prepare_debug_target(&fixture).expect("prepare Tcl source target");
        assert_eq!(target.required_tcl_abi, None);
        assert_eq!(target.provenance.runtime_path, fixture);
        assert!(target.provenance.decompiled_from.is_none());
    }
}
