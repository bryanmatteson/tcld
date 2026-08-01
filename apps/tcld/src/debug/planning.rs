//! Backend-neutral planning for `tcld debug`.
//!
//! The product contract is deliberately explicit:
//!
//! - Tcl source (`.tcl`, `.tk`, `.tm`, or any existing non-`.tbc` file) is a
//!   first-class host-debug target. `auto` and `host` execute that source
//!   directly through the wire debugger.
//! - TclPro bytecode is identified once, case-insensitively, from `.tbc`.
//!   `auto` selects the in-process hybrid debugger when it is linked and the
//!   exact-reconstruction host debugger otherwise.
//! - Explicit backend selection is never silently weakened. In particular,
//!   `hybrid` and `vm-strict` compile admitted source into one deterministic
//!   debug image before terminal setup; failures never fall back to host.
//! - Hybrid bytecode debugging preserves the original artifact and never
//!   decompiles it. Host bytecode debugging owns an exact reconstructed Tcl
//!   file for precisely the lifetime of the prepared target.

use std::path::{Path, PathBuf};

use super::config::TclAbi;

/// The kind of user-owned target, independent of the requested backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugTargetKind {
    TclSource,
    TclProBytecode,
}

impl DebugTargetKind {
    pub fn identify(path: &Path) -> Self {
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tbc"))
        {
            Self::TclProBytecode
        } else {
            Self::TclSource
        }
    }
}

/// What the user requested, before target/backend compatibility is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugBackendPolicy {
    Auto,
    Host,
    Hybrid,
    VmStrict,
}

impl From<Option<crate::cli::RunBackend>> for DebugBackendPolicy {
    fn from(backend: Option<crate::cli::RunBackend>) -> Self {
        match backend.unwrap_or(crate::cli::RunBackend::Auto) {
            crate::cli::RunBackend::Auto => Self::Auto,
            crate::cli::RunBackend::Host => Self::Host,
            crate::cli::RunBackend::Hybrid => Self::Hybrid,
            crate::cli::RunBackend::VmStrict => Self::VmStrict,
        }
    }
}

/// The backend selected by the planner. This remains separate from
/// [`DebugTargetKind`] because source VM routes require compiler-owned image
/// preparation while bytecode VM routes execute the original artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugExecutionBackend {
    Host,
    Hybrid,
    VmStrict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugRequest {
    pub original_path: PathBuf,
    pub backend_policy: DebugBackendPolicy,
}

impl DebugRequest {
    pub fn new(path: impl Into<PathBuf>, backend_policy: DebugBackendPolicy) -> Self {
        Self {
            original_path: path.into(),
            backend_policy,
        }
    }
}

/// A validated routing decision. Planning is pure and performs no decompile or
/// runtime discovery; materialization happens later in [`prepare_debug_target`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugPlan {
    pub original_path: PathBuf,
    pub target_kind: DebugTargetKind,
    pub backend_policy: DebugBackendPolicy,
    pub backend: DebugExecutionBackend,
}

pub fn plan_debug(request: DebugRequest, hybrid_available: bool) -> Result<DebugPlan, String> {
    let target_kind = DebugTargetKind::identify(&request.original_path);
    let backend = match (target_kind, request.backend_policy) {
        (DebugTargetKind::TclSource, DebugBackendPolicy::Auto | DebugBackendPolicy::Host) => {
            DebugExecutionBackend::Host
        }
        (DebugTargetKind::TclSource, DebugBackendPolicy::Hybrid) if hybrid_available => {
            DebugExecutionBackend::Hybrid
        }
        (DebugTargetKind::TclSource, DebugBackendPolicy::VmStrict) if hybrid_available => {
            DebugExecutionBackend::VmStrict
        }
        (DebugTargetKind::TclSource, DebugBackendPolicy::Hybrid | DebugBackendPolicy::VmStrict) => {
            return Err(
                "hybrid_unavailable: rebuild tcld with --features hybrid to compile and debug Tcl source in process"
                    .into(),
            );
        }
        (DebugTargetKind::TclProBytecode, DebugBackendPolicy::Host) => DebugExecutionBackend::Host,
        (DebugTargetKind::TclProBytecode, DebugBackendPolicy::Auto) if hybrid_available => {
            DebugExecutionBackend::Hybrid
        }
        (DebugTargetKind::TclProBytecode, DebugBackendPolicy::Auto) => DebugExecutionBackend::Host,
        (DebugTargetKind::TclProBytecode, DebugBackendPolicy::Hybrid) if hybrid_available => {
            DebugExecutionBackend::Hybrid
        }
        (DebugTargetKind::TclProBytecode, DebugBackendPolicy::VmStrict) if hybrid_available => {
            DebugExecutionBackend::VmStrict
        }
        (
            DebugTargetKind::TclProBytecode,
            DebugBackendPolicy::Hybrid | DebugBackendPolicy::VmStrict,
        ) => {
            return Err(
                "hybrid_unavailable: rebuild tcld with --features hybrid to debug TclPro bytecode in process"
                    .into(),
            );
        }
    };

    Ok(DebugPlan {
        original_path: request.original_path,
        target_kind,
        backend_policy: request.backend_policy,
        backend,
    })
}

/// Original/display/runtime identity and ownership for a prepared target.
///
/// `display_path` always identifies the user's input. `runtime_path` is either
/// that same user-owned path or the exact host reconstruction. The optional
/// temp-dir guard ensures generated source exists for the entire debug session
/// and is removed when the prepared target/application is dropped.
#[derive(Debug)]
pub struct TargetProvenance {
    pub original_path: PathBuf,
    pub runtime_path: PathBuf,
    pub display_path: PathBuf,
    pub decompiled_from: Option<PathBuf>,
    pub bytecode_tcl_version: Option<String>,
    pub(super) temp_dir: Option<tempfile::TempDir>,
}

impl TargetProvenance {
    pub fn user_script(path: PathBuf) -> Self {
        Self {
            original_path: path.clone(),
            runtime_path: path.clone(),
            display_path: path,
            decompiled_from: None,
            bytecode_tcl_version: None,
            temp_dir: None,
        }
    }

    pub fn direct_bytecode(path: PathBuf, bytecode_tcl_version: String) -> Self {
        Self {
            original_path: path.clone(),
            runtime_path: path.clone(),
            display_path: path,
            decompiled_from: None,
            bytecode_tcl_version: Some(bytecode_tcl_version),
            temp_dir: None,
        }
    }

    pub fn decompiled(
        runtime_path: PathBuf,
        display_path: PathBuf,
        temp_dir: tempfile::TempDir,
    ) -> Self {
        let original_path = display_path.clone();
        let decompiled_from = Some(display_path.clone());
        Self {
            original_path,
            runtime_path,
            display_path,
            decompiled_from,
            bytecode_tcl_version: None,
            temp_dir: Some(temp_dir),
        }
    }

    pub fn decompiled_bytecode(
        runtime_path: PathBuf,
        display_path: PathBuf,
        bytecode_tcl_version: String,
        temp_dir: tempfile::TempDir,
    ) -> Self {
        Self {
            original_path: display_path.clone(),
            runtime_path,
            display_path: display_path.clone(),
            decompiled_from: Some(display_path),
            bytecode_tcl_version: Some(bytecode_tcl_version),
            temp_dir: Some(temp_dir),
        }
    }
}

/// A target ready for the selected backend, including any generated artifact
/// lifetime and bytecode ABI constraint.
#[derive(Debug)]
pub struct PreparedDebugTarget {
    pub plan: DebugPlan,
    pub runtime_path: PathBuf,
    pub display_path: PathBuf,
    pub provenance: TargetProvenance,
    pub required_tcl_abi: Option<TclAbi>,
    pub compiled_image: Option<std::rc::Rc<tcld_compile::DebugImage>>,
}

impl PreparedDebugTarget {
    pub fn from_plan(
        plan: DebugPlan,
        provenance: TargetProvenance,
        required_tcl_abi: Option<TclAbi>,
    ) -> Self {
        Self {
            runtime_path: provenance.runtime_path.clone(),
            display_path: provenance.display_path.clone(),
            plan,
            provenance,
            required_tcl_abi,
            compiled_image: None,
        }
    }

    /// Host-target constructor retained for callers that already own source
    /// provenance (primarily focused UI tests).
    pub fn new(provenance: TargetProvenance, required_tcl_abi: Option<TclAbi>) -> Self {
        let policy = DebugBackendPolicy::Host;
        let plan = plan_debug(
            DebugRequest::new(provenance.original_path.clone(), policy),
            cfg!(feature = "hybrid"),
        )
        .expect("host target plan is always valid");
        Self::from_plan(plan, provenance, required_tcl_abi)
    }

    /// Convenience constructor retained for the host debugger's focused tests.
    pub fn user_script(path: PathBuf) -> Self {
        let plan = plan_debug(
            DebugRequest::new(path.clone(), DebugBackendPolicy::Host),
            cfg!(feature = "hybrid"),
        )
        .expect("host source plan is always valid");
        Self::from_plan(plan, TargetProvenance::user_script(path), None)
    }
}

/// Call counter for the invariant that source and hybrid plans never enter the
/// host bytecode reconstruction path.
pub static DECOMPILE_TO_TEMP_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub fn prepare_debug_target(plan: DebugPlan) -> Result<PreparedDebugTarget, String> {
    match (plan.target_kind, plan.backend) {
        (DebugTargetKind::TclSource, DebugExecutionBackend::Host) => {
            let path = plan.original_path.clone();
            Ok(PreparedDebugTarget::from_plan(
                plan,
                TargetProvenance::user_script(path),
                None,
            ))
        }
        (DebugTargetKind::TclProBytecode, DebugExecutionBackend::Host) => decompile_to_temp(plan),
        (
            DebugTargetKind::TclProBytecode,
            DebugExecutionBackend::Hybrid | DebugExecutionBackend::VmStrict,
        ) => prepare_direct_bytecode(plan),
        (
            DebugTargetKind::TclSource,
            DebugExecutionBackend::Hybrid | DebugExecutionBackend::VmStrict,
        ) => prepare_compiled_source(plan),
    }
}

#[cfg(feature = "hybrid")]
fn prepare_compiled_source(plan: DebugPlan) -> Result<PreparedDebugTarget, String> {
    let mode = match plan.backend {
        DebugExecutionBackend::Hybrid => tcld_compile::CompileMode::Hybrid,
        DebugExecutionBackend::VmStrict => tcld_compile::CompileMode::VmStrict,
        DebugExecutionBackend::Host => unreachable!("host source is not compiled"),
    };
    let image = std::rc::Rc::new(
        tcld_compile::compile_path(&plan.original_path, mode).map_err(|error| error.to_string())?,
    );
    let path = image.source.path.clone();
    let mut prepared =
        PreparedDebugTarget::from_plan(plan, TargetProvenance::user_script(path), None);
    prepared.compiled_image = Some(image);
    Ok(prepared)
}

#[cfg(not(feature = "hybrid"))]
fn prepare_compiled_source(_plan: DebugPlan) -> Result<PreparedDebugTarget, String> {
    unreachable!("planner rejects source VM backends when hybrid is not linked")
}

fn load_bytecode_metadata(
    path: &Path,
) -> Result<(tcld_decompile::LoadedBytecode, String, TclAbi), String> {
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
    Ok((loaded, bytecode_tcl_version, required_tcl_abi))
}

fn prepare_direct_bytecode(plan: DebugPlan) -> Result<PreparedDebugTarget, String> {
    let path = plan.original_path.clone();
    let (_, bytecode_tcl_version, required_tcl_abi) = load_bytecode_metadata(&path)?;
    Ok(PreparedDebugTarget::from_plan(
        plan,
        TargetProvenance::direct_bytecode(path, bytecode_tcl_version),
        Some(required_tcl_abi),
    ))
}

fn decompile_to_temp(plan: DebugPlan) -> Result<PreparedDebugTarget, String> {
    DECOMPILE_TO_TEMP_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let path = plan.original_path.clone();
    let (loaded, bytecode_tcl_version, required_tcl_abi) = load_bytecode_metadata(&path)?;
    let source = tcld_decompile::decompile_loaded(&loaded)
        .map_err(|error| host_decompile_failure(&path, &error.to_string()))?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("bytecode");
    let dir = tempfile::Builder::new()
        .prefix("tcld-debug-")
        .tempdir()
        .map_err(|error| format!("create temp dir: {error}"))?;
    let runtime_path = dir.path().join(format!("{stem}.tcl"));
    std::fs::write(&runtime_path, source)
        .map_err(|error| format!("write {}: {error}", runtime_path.display()))?;
    let provenance =
        TargetProvenance::decompiled_bytecode(runtime_path, path, bytecode_tcl_version, dir);
    Ok(PreparedDebugTarget::from_plan(
        plan,
        provenance,
        Some(required_tcl_abi),
    ))
}

/// Describe a host reconstruction failure and the available alternatives.
pub(super) fn host_decompile_failure(path: &Path, error: &str) -> String {
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

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
            .canonicalize()
            .unwrap()
    }

    fn plan(path: &str, policy: DebugBackendPolicy, hybrid: bool) -> Result<DebugPlan, String> {
        plan_debug(DebugRequest::new(path, policy), hybrid)
    }

    #[test]
    fn source_extensions_are_first_class_host_targets() {
        for path in ["script.tcl", "widget.tk", "package.tm", "extensionless"] {
            assert_eq!(
                DebugTargetKind::identify(Path::new(path)),
                DebugTargetKind::TclSource
            );
            for policy in [DebugBackendPolicy::Auto, DebugBackendPolicy::Host] {
                let selected = plan(path, policy, true).expect("source host plan");
                assert_eq!(selected.target_kind, DebugTargetKind::TclSource);
                assert_eq!(selected.backend, DebugExecutionBackend::Host);
            }
        }
    }

    #[test]
    fn bytecode_identification_is_case_insensitive() {
        for path in ["program.tbc", "program.TBC", "program.TbC"] {
            assert_eq!(
                DebugTargetKind::identify(Path::new(path)),
                DebugTargetKind::TclProBytecode
            );
        }
    }

    #[test]
    fn routing_matrix_keeps_target_and_backend_independent() {
        assert_eq!(
            plan("program.tcl", DebugBackendPolicy::Hybrid, true)
                .unwrap()
                .backend,
            DebugExecutionBackend::Hybrid
        );
        assert_eq!(
            plan("program.tcl", DebugBackendPolicy::VmStrict, true)
                .unwrap()
                .backend,
            DebugExecutionBackend::VmStrict
        );
        for policy in [DebugBackendPolicy::Hybrid, DebugBackendPolicy::VmStrict] {
            let error = plan("program.tcl", policy, false).unwrap_err();
            assert!(error.starts_with("hybrid_unavailable:"), "{error}");
        }

        assert_eq!(
            plan("program.tbc", DebugBackendPolicy::Auto, true)
                .unwrap()
                .backend,
            DebugExecutionBackend::Hybrid
        );
        assert_eq!(
            plan("program.tbc", DebugBackendPolicy::Auto, false)
                .unwrap()
                .backend,
            DebugExecutionBackend::Host
        );
        assert_eq!(
            plan("program.tbc", DebugBackendPolicy::Host, true)
                .unwrap()
                .backend,
            DebugExecutionBackend::Host
        );
        assert_eq!(
            plan("program.tbc", DebugBackendPolicy::Hybrid, true)
                .unwrap()
                .backend,
            DebugExecutionBackend::Hybrid
        );
        assert_eq!(
            plan("program.tbc", DebugBackendPolicy::VmStrict, true)
                .unwrap()
                .backend,
            DebugExecutionBackend::VmStrict
        );
        for policy in [DebugBackendPolicy::Hybrid, DebugBackendPolicy::VmStrict] {
            let error = plan("program.tbc", policy, false).unwrap_err();
            assert!(error.starts_with("hybrid_unavailable:"), "{error}");
            assert!(error.contains("--features hybrid"), "{error}");
        }
    }

    #[test]
    fn source_and_hybrid_preparation_never_decompile() {
        let source = fixture("hello.tcl");
        let source_target = prepare_debug_target(
            plan_debug(DebugRequest::new(&source, DebugBackendPolicy::Auto), true).unwrap(),
        )
        .unwrap();
        assert_eq!(source_target.runtime_path, source);
        assert_eq!(source_target.provenance.original_path, source);
        assert_eq!(source_target.provenance.display_path, source);
        assert!(source_target.provenance.decompiled_from.is_none());
        assert_eq!(source_target.required_tcl_abi, None);

        let bytecode = fixture("expr.tbc");
        let hybrid_target = prepare_debug_target(
            plan_debug(
                DebugRequest::new(&bytecode, DebugBackendPolicy::Hybrid),
                true,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(hybrid_target.runtime_path, bytecode);
        assert_eq!(hybrid_target.provenance.original_path, bytecode);
        assert_eq!(hybrid_target.provenance.display_path, bytecode);
        assert!(hybrid_target.provenance.decompiled_from.is_none());
        assert_eq!(hybrid_target.required_tcl_abi, Some(TclAbi::Tcl86));
    }

    #[test]
    fn host_bytecode_preparation_owns_exact_reconstruction_lifetime() {
        let bytecode = fixture("expr.tbc");
        let target = prepare_debug_target(
            plan_debug(DebugRequest::new(&bytecode, DebugBackendPolicy::Host), true).unwrap(),
        )
        .unwrap();

        assert_eq!(target.provenance.original_path, bytecode);
        assert_eq!(target.provenance.display_path, bytecode);
        assert_eq!(target.provenance.decompiled_from.as_ref(), Some(&bytecode));
        assert_eq!(target.required_tcl_abi, Some(TclAbi::Tcl86));
        let runtime_path = target.runtime_path.clone();
        let runtime_dir = runtime_path.parent().unwrap().to_path_buf();
        assert!(runtime_path.is_file());
        assert_eq!(
            runtime_path.extension().and_then(|value| value.to_str()),
            Some("tcl")
        );

        drop(target);
        assert!(!runtime_path.exists(), "generated source removed on drop");
        assert!(
            !runtime_dir.exists(),
            "generated temp directory removed on drop"
        );
        assert!(bytecode.is_file(), "user-owned bytecode is never removed");
    }
}
