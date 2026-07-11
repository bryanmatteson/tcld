//! `tcld run` — hybrid TEBC / host script runner.
//!
use std::path::Path;

use crate::cli::{RunArgs, RunBackend};
use crate::error::{Result, TcldError};

pub fn run_script(args: RunArgs) -> Result<()> {
    let path = Path::new(&args.input_path);
    if !path.exists() {
        return Err(TcldError::Other(format!(
            "file not found: {}",
            path.display()
        )));
    }

    let is_tbc = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tbc"));

    let backend = if args.sandbox {
        RunBackend::VmStrict
    } else {
        args.backend
    };

    match backend {
        RunBackend::Host => run_host(path),
        RunBackend::VmStrict | RunBackend::Hybrid | RunBackend::Auto => {
            if is_tbc {
                run_hybrid(path, backend)
            } else if matches!(backend, RunBackend::Auto | RunBackend::Host) {
                run_host(path)
            } else {
                Err(TcldError::Other(
                    "vm-strict/hybrid backends require a .tbc input".into(),
                ))
            }
        }
    }
}

#[cfg(feature = "hybrid")]
fn run_hybrid(path: &Path, backend: RunBackend) -> Result<()> {
    use tcld_vm::{HybridEngine, RuntimeMode};

    let mode = match backend {
        RunBackend::VmStrict => RuntimeMode::VmStrict,
        RunBackend::Hybrid | RunBackend::Auto => RuntimeMode::Hybrid,
        RunBackend::Host => unreachable!(),
    };
    let mut eng = HybridEngine::new(mode).map_err(|e| TcldError::Other(e.to_string()))?;
    eng.bootstrap(std::env::args().next().as_deref())
        .map_err(|e| TcldError::Other(e.to_string()))?;
    match eng.run_tbc_path(path) {
        Ok(v) => {
            if let Ok(s) = v.as_string()
                && !s.is_empty()
            {
                println!("{s}");
            }
            Ok(())
        }
        Err(e) => {
            let code = e.exit_code();
            eprintln!("{e}");
            std::process::exit(code);
        }
    }
}

#[cfg(not(feature = "hybrid"))]
fn run_hybrid(path: &Path, backend: RunBackend) -> Result<()> {
    // Forced hybrid / vm-strict / sandbox must fail closed — host is not
    // a substitute for the TEBC VM (plan §2.1 / review bug 4).
    if matches!(backend, RunBackend::Hybrid | RunBackend::VmStrict) {
        eprintln!("hybrid_unavailable: rebuild tcld with --features hybrid");
        std::process::exit(3);
    }
    // Auto only: fall back to host with logged reason (`.tcl` scripts).
    // `.tbc` has no host-run path here → clean exit 3.
    eprintln!(
        "hybrid_unavailable: falling back to host for {}",
        path.display()
    );
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tbc"))
    {
        eprintln!("hybrid_unavailable: no host runner for .tbc without hybrid feature");
        std::process::exit(3);
    }
    run_host(path)
}

fn run_host(path: &Path) -> Result<()> {
    // Minimal host runner: require tclsh on PATH for .tcl; for .tbc decompile+temp is host debug path.
    use std::process::Command;

    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tbc"))
    {
        return Err(TcldError::Other(
            "host backend for .tbc is not wired in `tcld run` yet; use --features hybrid or `tcld debug --backend=host`"
                .into(),
        ));
    }

    let tclsh = std::env::var("TCLD_TCLSH").unwrap_or_else(|_| "tclsh".into());
    let status = Command::new(&tclsh)
        .arg(path)
        .status()
        .map_err(|e| TcldError::Other(format!("failed to spawn {tclsh}: {e}")))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
