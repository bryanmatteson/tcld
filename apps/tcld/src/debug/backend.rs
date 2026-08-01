//! `DebugBackend` trait + hybrid / wire adapters (executable plan S10).
//!
//! Hybrid path never decompiles bytecode to a temp script target (I1).
//! Wire path remains the existing nub Session adapter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::contract::{
    BackendCapabilities, BackendOperation, BackendPause, CapabilitySupport,
    CompanionSourceSnapshot, DebugBreakpoint, DebugBreakpointId, DebugBreakpointResolution,
    DebugBreakpointSpec, DebugDiagnosticSeverity, DebugEvaluation, DebugEvent, DebugLifecycleState,
    DebugLocation, DebugPauseReason, DebugProcedure, DebugSourceUnitIdentity, DebugStackFrame,
    DebugStepKind, DebugVariable, DisassemblySnapshot, RegisteredSource, SourceCommandSpan,
    SourceSnapshot,
};
#[cfg(feature = "hybrid")]
use super::contract::{
    CompanionSource, CompanionSourceLocation, DebugCodeId, DebugCodeLocation, RegisteredCode,
};

impl BackendCapabilities {
    #[cfg(feature = "hybrid")]
    pub fn hybrid(mode: tcld_vm::RuntimeMode) -> Self {
        let dynamic_eval = mode == tcld_vm::RuntimeMode::Hybrid;
        let mut support = all_unsupported("operation is not implemented by the hybrid backend");
        for operation in [
            BackendOperation::Start,
            BackendOperation::Restart,
            BackendOperation::Continue,
            BackendOperation::StepOut,
            BackendOperation::StepInstruction,
            BackendOperation::StepOverInstruction,
            BackendOperation::StepOutInstruction,
            BackendOperation::RunToLocation,
            BackendOperation::Stack,
            BackendOperation::Variables,
            BackendOperation::InstructionBreakpoints,
            BackendOperation::CompanionSourceAccess,
            BackendOperation::DisassemblyAccess,
            BackendOperation::Coverage,
            BackendOperation::DirectBytecode,
        ] {
            support.insert(operation, CapabilitySupport::Supported);
        }
        support.insert(
            BackendOperation::SourceBreakpoints,
            CapabilitySupport::Unsupported {
                reason: "hybrid has no canonical source map; companion-source hints cannot arm source breakpoints"
                    .into(),
            },
        );
        support.insert(
            BackendOperation::SourceAccess,
            CapabilitySupport::Unsupported {
                reason: "hybrid bytecode has no verified source identity; use companion-source hints when present"
                    .into(),
            },
        );
        support.insert(
            BackendOperation::Pause,
            CapabilitySupport::Unsupported {
                reason: "the in-process VM executes synchronously between debugger stops".into(),
            },
        );
        support.insert(
            BackendOperation::StepInto,
            CapabilitySupport::Unsupported {
                reason: "hybrid source-command stepping requires a verified source command map; use instruction step".into(),
            },
        );
        support.insert(
            BackendOperation::StepOver,
            CapabilitySupport::Unsupported {
                reason: "hybrid source-command stepping requires a verified source command map; use instruction step-over".into(),
            },
        );
        support.insert(
            BackendOperation::Evaluate,
            if dynamic_eval {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported {
                    reason: "dynamic evaluation is denied by vm-strict mode".into(),
                }
            },
        );
        support.insert(
            BackendOperation::Watches,
            if dynamic_eval {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported {
                    reason: "watch evaluation requires dynamic evaluation, which vm-strict denies"
                        .into(),
                }
            },
        );
        support.insert(
            BackendOperation::ProcedureList,
            CapabilitySupport::Unsupported {
                reason: "the hybrid VM does not yet expose a session-wide procedure catalog".into(),
            },
        );
        Self::new(support)
    }

    #[cfg(feature = "hybrid")]
    pub fn compiled_source_hybrid(mode: tcld_vm::RuntimeMode) -> Self {
        let mut capabilities = Self::hybrid(mode);
        for operation in [
            BackendOperation::StepInto,
            BackendOperation::StepOver,
            BackendOperation::SourceBreakpoints,
            BackendOperation::SourceAccess,
            BackendOperation::ProcedureList,
        ] {
            capabilities.set(operation, CapabilitySupport::Supported);
        }
        capabilities.set(
            BackendOperation::CompanionSourceAccess,
            CapabilitySupport::Unsupported {
                reason: "compiled-source debugging uses the canonical original source, not a companion projection".into(),
            },
        );
        capabilities.set(
            BackendOperation::DirectBytecode,
            CapabilitySupport::Unsupported {
                reason: "the target is compiler-owned Tcl source with a verified executable image"
                    .into(),
            },
        );
        capabilities
    }

    pub fn wire_host() -> Self {
        let mut support = all_unsupported("operation is not available on the host wire backend");
        for operation in [
            BackendOperation::Start,
            BackendOperation::Restart,
            BackendOperation::Continue,
            BackendOperation::Pause,
            BackendOperation::StepInto,
            BackendOperation::StepOver,
            BackendOperation::StepOut,
            BackendOperation::Stack,
            BackendOperation::Variables,
            BackendOperation::Evaluate,
            BackendOperation::Watches,
            BackendOperation::SourceBreakpoints,
            BackendOperation::SourceAccess,
            BackendOperation::Coverage,
            BackendOperation::ProcedureList,
        ] {
            support.insert(operation, CapabilitySupport::Supported);
        }
        let instruction_reason = CapabilitySupport::Unsupported {
            reason:
                "stock Tcl shells do not expose stable code-object-qualified instruction control"
                    .into(),
        };
        for operation in [
            BackendOperation::StepInstruction,
            BackendOperation::StepOverInstruction,
            BackendOperation::StepOutInstruction,
            BackendOperation::RunToLocation,
            BackendOperation::InstructionBreakpoints,
            BackendOperation::DisassemblyAccess,
            BackendOperation::DirectBytecode,
        ] {
            support.insert(operation, instruction_reason.clone());
        }
        Self::new(support)
    }
}

fn all_unsupported(reason: &str) -> BTreeMap<BackendOperation, CapabilitySupport> {
    use BackendOperation::*;
    [
        Start,
        Restart,
        Continue,
        Pause,
        StepInto,
        StepOver,
        StepOut,
        StepInstruction,
        StepOverInstruction,
        StepOutInstruction,
        RunToLocation,
        Stack,
        Variables,
        Evaluate,
        Watches,
        SourceBreakpoints,
        InstructionBreakpoints,
        SourceAccess,
        CompanionSourceAccess,
        DisassemblyAccess,
        Coverage,
        ProcedureList,
        DirectBytecode,
    ]
    .into_iter()
    .map(|operation| {
        (
            operation,
            CapabilitySupport::Unsupported {
                reason: reason.to_owned(),
            },
        )
    })
    .collect()
}

#[derive(Debug)]
pub enum BackendError {
    Unsupported { op: &'static str, reason: String },
    Message(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { op, reason } => write!(f, "unsupported {op}: {reason}"),
            Self::Message(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<String> for BackendError {
    fn from(s: String) -> Self {
        Self::Message(s)
    }
}

impl BackendError {
    fn unsupported(op: &'static str, reason: impl Into<String>) -> Self {
        Self::Unsupported {
            op,
            reason: reason.into(),
        }
    }
}

/// Complete backend-neutral session contract.
///
/// Source and bytecode backends implement the same semantic surface, while
/// [`BackendCapabilities`] and typed [`BackendError::Unsupported`] values keep
/// backend-specific affordances honest.
pub trait DebugBackend {
    fn capabilities(&self) -> BackendCapabilities;
    fn start_target(&mut self, path: &Path, stop_on_entry: bool) -> Result<(), BackendError>;
    /// Restart in a fresh execution context; re-sourcing into a dirty
    /// interpreter is forbidden. Source breakpoint intent accepted by a host
    /// backend is restored before the target starts. UI watch expressions are
    /// frontend-owned and may be reapplied after return (request
    /// `stop_on_entry` when that ordering matters). A VM restart assigns new
    /// code identities, so the hybrid adapter only reapplies saved PC intent
    /// when the suspended code has the same stable procedure identity; other
    /// instruction breakpoints are reported as unresolved instead of being
    /// silently armed on the wrong code object.
    fn restart_target(&mut self, stop_on_entry: bool) -> Result<(), BackendError>;
    fn continue_execution(&mut self) -> Result<(), BackendError>;
    fn pause_execution(&mut self) -> Result<(), BackendError>;
    fn step(&mut self, kind: DebugStepKind) -> Result<(), BackendError>;
    fn run_to_location(&mut self, location: DebugLocation) -> Result<(), BackendError>;
    fn stack_frames(&mut self) -> Result<Vec<DebugStackFrame>, BackendError>;
    fn variables_for_frame(&mut self, level: i64) -> Result<Vec<DebugVariable>, BackendError>;
    fn evaluate(&mut self, level: i64, script: &str) -> Result<DebugEvaluation, BackendError>;
    fn add_breakpoint(
        &mut self,
        spec: DebugBreakpointSpec,
    ) -> Result<DebugBreakpoint, BackendError>;
    fn remove_breakpoint(&mut self, id: &DebugBreakpointId) -> Result<(), BackendError>;
    fn breakpoints(&mut self) -> Result<Vec<DebugBreakpoint>, BackendError>;
    fn set_breakpoint_enabled(
        &mut self,
        id: &DebugBreakpointId,
        enabled: bool,
    ) -> Result<(), BackendError>;
    fn drain_events(&mut self) -> Vec<DebugEvent>;
    fn state(&mut self) -> Result<DebugLifecycleState, BackendError>;
    fn sources(&mut self) -> Result<Vec<SourceSnapshot>, BackendError>;
    fn companion_sources(&mut self) -> Result<Vec<CompanionSourceSnapshot>, BackendError>;
    fn disassembly(&mut self) -> Result<Vec<DisassemblySnapshot>, BackendError>;

    fn procedures(&mut self) -> Result<Vec<DebugProcedure>, BackendError> {
        Err(self.unsupported(BackendOperation::ProcedureList, "procedure list"))
    }
    fn procedure_location(&mut self, _qualified_name: &str) -> Result<Option<u64>, BackendError> {
        Err(self.unsupported(BackendOperation::ProcedureList, "procedure location"))
    }
    fn procedure_source(&mut self, qualified_name: &str) -> Result<SourceSnapshot, BackendError> {
        self.sources()?
            .into_iter()
            .find(|snapshot| snapshot.source.qualified_proc.as_deref() == Some(qualified_name))
            .ok_or_else(|| {
                BackendError::Message(format!("procedure source not found: {qualified_name}"))
            })
    }
    fn watch_entries(&mut self) -> Result<Vec<tcld_tui::debug::WatchEntry>, BackendError> {
        Err(self.unsupported(BackendOperation::Watches, "watch list"))
    }
    fn evaluate_watch_entries(
        &mut self,
        _level: i64,
    ) -> Result<Vec<tcld_tui::debug::WatchEntry>, BackendError> {
        Err(self.unsupported(BackendOperation::Watches, "watch evaluation"))
    }
    fn add_watch(&mut self, _name: &str, _expression: &str) -> Result<(), BackendError> {
        Err(self.unsupported(BackendOperation::Watches, "add watch"))
    }
    fn remove_watch(&mut self, _id: &str) -> Result<(), BackendError> {
        Err(self.unsupported(BackendOperation::Watches, "remove watch"))
    }
    fn add_watchpoint(&mut self, _name: &str, _operations: &str) -> Result<(), BackendError> {
        Err(self.unsupported(BackendOperation::Watches, "variable watchpoint"))
    }
    fn coverage_snapshot(&mut self) -> Result<Vec<tcld_tui::debug::CoverageRow>, BackendError> {
        Err(self.unsupported(BackendOperation::Coverage, "coverage snapshot"))
    }
    fn set_coverage_enabled(&mut self, _enabled: bool) -> Result<(), BackendError> {
        Err(self.unsupported(BackendOperation::Coverage, "coverage control"))
    }
    fn ignore_current_error(&mut self) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            "ignore_error",
            "the selected backend has no exception-pause suppression operation",
        ))
    }
    fn slave_interpreters(&mut self) -> Result<Vec<String>, BackendError> {
        Err(BackendError::unsupported(
            "slave_interpreters",
            "the selected backend does not expose slave interpreters",
        ))
    }
    fn preinstrument_procedure(
        &mut self,
        _qualified_name: &str,
        _body: &str,
    ) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            "preinstrument",
            "the selected backend does not instrument Tcl procedure source",
        ))
    }
    fn export_breakpoint_state(&mut self) -> Result<String, BackendError> {
        Err(BackendError::unsupported(
            "export_breakpoints",
            "the selected backend does not export persistent breakpoint state",
        ))
    }
    fn import_breakpoint_state(&mut self, _serialized: &str) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            "import_breakpoints",
            "the selected backend does not import persistent breakpoint state",
        ))
    }
    fn set_breakpoint_ignore_count(
        &mut self,
        _id: &DebugBreakpointId,
        _count: u64,
    ) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            "breakpoint_ignore",
            "the selected backend does not support breakpoint ignore counts",
        ))
    }
    fn set_breakpoint_condition(
        &mut self,
        _id: &DebugBreakpointId,
        _condition: Option<&str>,
    ) -> Result<DebugBreakpointId, BackendError> {
        Err(BackendError::unsupported(
            "breakpoint_condition",
            "the selected backend does not support breakpoint conditions",
        ))
    }
    fn get_variable(&mut self, level: i64, name: &str) -> Result<String, BackendError> {
        self.variables_for_frame(level)?
            .into_iter()
            .find(|variable| variable.name == name)
            .map(|variable| variable.value)
            .ok_or_else(|| BackendError::Message(format!("variable not found: {name}")))
    }
    fn set_variable(&mut self, _level: i64, _name: &str, _value: &str) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            "set_variable",
            "the selected backend cannot mutate frame variables",
        ))
    }

    fn close(self: Box<Self>);

    fn unsupported(&self, operation: BackendOperation, label: &'static str) -> BackendError {
        BackendError::unsupported(
            label,
            self.capabilities()
                .support(operation)
                .reason()
                .unwrap_or("operation is unavailable"),
        )
    }
}

// ── Hybrid in-process backend ────────────────────────────────────────

/// In-process hybrid TEBC debugger for raw `.tbc` and compiler-owned source
/// images (I1: no decompile-to-temp execution target).
#[cfg(feature = "hybrid")]
pub struct HybridInProcessBackend {
    mode: tcld_vm::RuntimeMode,
    engine: tcld_vm::HybridEngine,
    target: Option<PathBuf>,
    events: Vec<DebugEvent>,
    finished: bool,
    last_result: Option<String>,
    /// Disassembly text for the current target (source pane).
    disassembly: Vec<String>,
    source_text: Option<String>,
    source_path: Option<PathBuf>,
    current_code_id: Option<tcld_vm::load::CodeId>,
    current_bytecode: Option<tcld_core::ByteCode>,
    current_proc_qname: Option<String>,
    /// Code-qualified breakpoints currently armed (for UI).
    pc_breakpoints: std::collections::BTreeSet<tcld_vm::CodeAddress>,
    breakpoints: BTreeMap<DebugBreakpointId, DebugBreakpoint>,
    next_breakpoint_id: u64,
    image: Option<std::rc::Rc<tcld_compile::DebugImage>>,
    image_bindings: BTreeMap<tcld_vm::load::CodeId, String>,
    captured_stdout_len: usize,
    captured_stderr_len: usize,
    runtime_error_paused: bool,
}

#[cfg(feature = "hybrid")]
impl HybridInProcessBackend {
    pub fn new(mode: tcld_vm::RuntimeMode) -> Result<Self, BackendError> {
        let mut engine =
            tcld_vm::HybridEngine::new(mode).map_err(|e| BackendError::Message(e.to_string()))?;
        engine
            .bootstrap(std::env::args().next().as_deref())
            .map_err(|e| BackendError::Message(e.to_string()))?;
        if mode == tcld_vm::RuntimeMode::Hybrid {
            engine
                .begin_output_capture()
                .map_err(|e| BackendError::Message(e.to_string()))?;
        }
        Ok(Self {
            mode,
            engine,
            target: None,
            events: Vec::new(),
            finished: false,
            last_result: None,
            disassembly: Vec::new(),
            source_text: None,
            source_path: None,
            current_code_id: None,
            current_bytecode: None,
            current_proc_qname: None,
            pc_breakpoints: std::collections::BTreeSet::new(),
            breakpoints: BTreeMap::new(),
            next_breakpoint_id: 1,
            image: None,
            image_bindings: BTreeMap::new(),
            captured_stdout_len: 0,
            captured_stderr_len: 0,
            runtime_error_paused: false,
        })
    }

    pub fn new_with_image(
        mode: tcld_vm::RuntimeMode,
        image: std::rc::Rc<tcld_compile::DebugImage>,
    ) -> Result<Self, BackendError> {
        let mut backend = Self::new(mode)?;
        if image.mode
            != match mode {
                tcld_vm::RuntimeMode::Hybrid => tcld_compile::CompileMode::Hybrid,
                tcld_vm::RuntimeMode::VmStrict => tcld_compile::CompileMode::VmStrict,
            }
        {
            return Err(BackendError::Message(
                "compiled image policy does not match the selected runtime mode".into(),
            ));
        }
        backend.image = Some(image);
        Ok(backend)
    }

    fn image_unit_for_code(
        &self,
        code_id: tcld_vm::load::CodeId,
    ) -> Option<&tcld_compile::DebugCodeUnit> {
        let logical = self.image_bindings.get(&code_id)?;
        self.image.as_ref()?.unit(logical)
    }

    fn image_block_id(&self, unit: &tcld_compile::DebugCodeUnit) -> Option<String> {
        Some(format!(
            "image:{}:{}",
            self.image.as_ref()?.image_digest,
            unit.logical_id
        ))
    }

    fn mapped_source_location(
        &self,
        address: tcld_vm::CodeAddress,
    ) -> Option<tcld_debug_wire::SourceLocation> {
        self.mapped_source_command(address)
            .map(|(location, _)| location)
    }

    fn mapped_source_command(
        &self,
        address: tcld_vm::CodeAddress,
    ) -> Option<(tcld_debug_wire::SourceLocation, u32)> {
        use tcld_compile::DebugCodeUnitKind;
        use tcld_debug_wire::SourceBlockKind;
        let image = self.image.as_ref()?;
        let unit = self.image_unit_for_code(address.code_id)?;
        let map = unit
            .commands
            .iter()
            .filter(|map| map.pc_range.contains(&address.pc))
            .max_by_key(|map| map.nesting_depth)?;
        let (block_kind, qualified_proc) = match &unit.kind {
            DebugCodeUnitKind::TopLevel => (SourceBlockKind::File, None),
            DebugCodeUnitKind::Procedure { qname, .. } => {
                (SourceBlockKind::Proc, Some(qname.clone()))
            }
        };
        Some((
            tcld_debug_wire::SourceLocation {
                block_id: self.image_block_id(unit)?,
                block_kind,
                source_path: Some(image.source.path.display().to_string()),
                qualified_proc,
                line: map.start_line,
                offset_start: map.source_span.start as u64,
                offset_end: map.source_span.end as u64,
            },
            map.nesting_depth,
        ))
    }

    fn push_runtime_error(&mut self, error: impl ToString) {
        self.push_captured_output();
        self.runtime_error_paused = true;
        let address = self
            .engine
            .observations()
            .last()
            .map(|observation| tcld_vm::CodeAddress {
                code_id: observation.code_id,
                pc: observation.pc,
            });
        let source = address.and_then(|address| self.mapped_source_location(address));
        let code = address.map(|address| DebugCodeLocation {
            code_id: DebugCodeId(address.code_id.0),
            pc: u64::from(address.pc),
        });
        self.events.push(DebugEvent::Paused(Box::new(BackendPause {
            location: DebugLocation {
                source,
                code,
                observed_pc: address.map(|address| u64::from(address.pc)),
                source_hint: None,
            },
            in_host: self.mode == tcld_vm::RuntimeMode::Hybrid,
            reason: DebugPauseReason::Error,
            proc_qname: self.current_proc_qname.clone(),
            frame_count: u32::try_from(self.engine.ui_stack_frames().len()).unwrap_or(u32::MAX),
            breakpoint: None,
            locals: self
                .engine
                .pause_snapshot()
                .map_or_else(Vec::new, |pause| pause.locals),
            error: Some(error.to_string()),
        })));
        self.events
            .push(DebugEvent::StateChanged(DebugLifecycleState::Paused));
    }

    fn register_image_events(&mut self) {
        use tcld_compile::DebugCodeUnitKind;
        let Some(image) = self.image.clone() else {
            return;
        };
        for (runtime_id, logical_id) in self.image_bindings.clone() {
            let Some(unit) = image.unit(&logical_id) else {
                continue;
            };
            let (identity, qname) = match &unit.kind {
                DebugCodeUnitKind::TopLevel => (
                    DebugSourceUnitIdentity::File(image.source.path.clone()),
                    None,
                ),
                DebugCodeUnitKind::Procedure { qname, .. } => (
                    DebugSourceUnitIdentity::Procedure(qname.clone()),
                    Some(qname.clone()),
                ),
            };
            let block_id = self.image_block_id(unit).expect("image block id");
            self.events
                .push(DebugEvent::SourceRegistered(RegisteredSource {
                    identity,
                    block_id: block_id.clone(),
                    generation: 1,
                    path: Some(image.source.path.clone()),
                    qualified_proc: qname.clone(),
                    definition_path: Some(image.source.path.clone()),
                    restart_limitation: None,
                }));
            self.events.push(DebugEvent::CodeRegistered(RegisteredCode {
                id: DebugCodeId(runtime_id.0),
                name: qname.or(Some(logical_id)),
                source_block_id: Some(block_id),
            }));
        }
    }

    fn source_step(&mut self, over: bool) -> Result<tcld_vm::DebugOutcome, BackendError> {
        let mut initially_active = self
            .engine
            .ui_stack_frames()
            .into_iter()
            .filter_map(|frame| frame.address)
            .filter_map(|address| self.mapped_source_command(address))
            .map(|(location, _)| {
                (
                    location.block_id,
                    location.offset_start,
                    location.offset_end,
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        let initial = self
            .engine
            .pause_snapshot()
            .and_then(|snapshot| snapshot.address)
            .and_then(|address| self.mapped_source_command(address))
            .map(|(location, depth)| {
                (
                    location.block_id,
                    location.offset_start,
                    location.offset_end,
                    depth,
                )
            });
        if let Some((block, start, end, _)) = &initial {
            initially_active.insert((block.clone(), *start, *end));
        }
        for _ in 0..100_000usize {
            let outcome = if over {
                self.engine.step_over()
            } else {
                self.engine.step_instruction()
            }
            .map_err(|error| BackendError::Message(error.to_string()))?;
            match &outcome {
                tcld_vm::DebugOutcome::Finished { .. } => return Ok(outcome),
                tcld_vm::DebugOutcome::Paused(snapshot) => {
                    let current = snapshot
                        .address
                        .and_then(|address| self.mapped_source_command(address))
                        .map(|(location, depth)| {
                            (
                                location.block_id,
                                location.offset_start,
                                location.offset_end,
                                depth,
                            )
                        });
                    let reached = current.as_ref().is_some_and(|current| {
                        initial.as_ref().is_some_and(|initial| {
                            !initially_active.contains(&(current.0.clone(), current.1, current.2))
                                && current != initial
                                && (!over || current.3 <= initial.3)
                        })
                    });
                    if reached {
                        return Ok(outcome);
                    }
                }
            }
        }
        Err(BackendError::Message(
            "source step exceeded 100000 instructions without reaching another verified command boundary".into(),
        ))
    }

    fn resolve_image_source_breakpoint(
        &self,
        spec: &DebugBreakpointSpec,
    ) -> Result<Vec<(tcld_vm::CodeAddress, tcld_debug_wire::SourceLocation)>, BackendError> {
        let Some(image) = self.image.as_ref() else {
            return Ok(Vec::new());
        };
        let requested_path = match spec {
            DebugBreakpointSpec::SourceLine { path, .. } => {
                Some(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            }
            DebugBreakpointSpec::SourceCommand { source_path, .. } => source_path
                .as_ref()
                .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone())),
            DebugBreakpointSpec::Instruction { .. } => None,
        };
        if requested_path
            .as_ref()
            .is_some_and(|path| path != &image.source.path)
        {
            return Ok(Vec::new());
        }
        let mut resolved = Vec::new();
        for (code_id, logical_id) in &self.image_bindings {
            let Some(unit) = image.unit(logical_id) else {
                continue;
            };
            let block_id = self.image_block_id(unit).expect("image block");
            for map in &unit.commands {
                let matches = match spec {
                    DebugBreakpointSpec::SourceLine { line, .. } => {
                        map.start_line <= *line && *line <= map.end_line
                    }
                    DebugBreakpointSpec::SourceCommand {
                        block_id: requested,
                        span,
                        ..
                    } => {
                        requested == &block_id
                            && map.source_span.start as u64 == span.offset_start
                            && map.source_span.end as u64 == span.offset_end
                    }
                    DebugBreakpointSpec::Instruction { .. } => false,
                };
                if matches {
                    let address = tcld_vm::CodeAddress {
                        code_id: *code_id,
                        pc: map.anchor_pc,
                    };
                    if let Some(source) = self.mapped_source_location(address) {
                        resolved.push((address, source));
                    }
                }
            }
        }
        Ok(resolved)
    }

    fn instruction_set_name(&self) -> Option<&'static str> {
        self.current_bytecode
            .as_ref()?
            .instruction_set()
            .ok()
            .map(|set| set.name)
    }

    fn coverage_snapshot(&self) -> tcld_vm::CoverageSnapshot {
        self.engine.coverage_snapshot()
    }

    /// Restart target from scratch (new engine + bootstrap).
    fn restart(&mut self, stop_on_entry: bool) -> Result<(), BackendError> {
        let path = self
            .target
            .clone()
            .ok_or_else(|| BackendError::Message("no target".into()))?;
        let mode = self.mode;
        let previous_code_id = self.current_code_id;
        let previous_proc = self.current_proc_qname.clone();
        let instruction_breakpoints: Vec<(DebugCodeLocation, bool)> = self
            .breakpoints
            .values()
            .filter_map(|breakpoint| match breakpoint.spec {
                DebugBreakpointSpec::Instruction { location, .. } => {
                    Some((location, breakpoint.enabled))
                }
                _ => None,
            })
            .collect();
        let source_breakpoints: Vec<(DebugBreakpointSpec, bool)> = self
            .breakpoints
            .values()
            .filter(|breakpoint| {
                !matches!(breakpoint.spec, DebugBreakpointSpec::Instruction { .. })
            })
            .map(|breakpoint| (breakpoint.spec.clone(), breakpoint.enabled))
            .collect();
        let image = self.image.clone();
        *self = match image {
            Some(image) => Self::new_with_image(mode, image)?,
            None => Self::new(mode)?,
        };
        self.start_target(&path, stop_on_entry)?;
        for (spec, enabled) in source_breakpoints {
            let restored = self.add_breakpoint(spec)?;
            if !enabled {
                self.set_breakpoint_enabled(&restored.id, false)?;
            }
        }
        let Some(code_id) = self.current_code_id else {
            return Ok(());
        };
        let can_requalify_current_code =
            previous_code_id.is_some() && previous_proc == self.current_proc_qname;
        let mut stale_count = 0usize;
        for (previous_location, enabled) in instruction_breakpoints {
            if !can_requalify_current_code
                || previous_code_id != Some(tcld_vm::load::CodeId(previous_location.code_id.0))
            {
                stale_count += 1;
                continue;
            }
            let breakpoint = self.add_breakpoint(DebugBreakpointSpec::Instruction {
                location: DebugCodeLocation {
                    code_id: DebugCodeId(code_id.0),
                    pc: previous_location.pc,
                },
                condition: None,
            })?;
            if !enabled {
                self.set_breakpoint_enabled(&breakpoint.id, false)?;
            }
        }
        if stale_count > 0 {
            self.events.push(DebugEvent::Diagnostic {
                severity: DebugDiagnosticSeverity::Warning,
                message: format!(
                    "restart left {stale_count} instruction breakpoint(s) unresolved because their prior code identity is not the new entry code object"
                ),
            });
        }
        Ok(())
    }

    fn load_disassembly(&mut self, path: &Path) -> Result<(), BackendError> {
        let path_str = path
            .to_str()
            .ok_or_else(|| BackendError::Message("path not UTF-8".into()))?;
        let text = tcld_decompile::disassemble_path(path_str)
            .map_err(|e| BackendError::Message(format!("disassemble: {e}")))?;
        self.disassembly.clear();
        self.source_text = None;
        self.source_path = None;
        self.disassembly.extend(text.lines().map(str::to_owned));
        let source_path = path.with_extension("tcl");
        if let Ok(source) = std::fs::read_to_string(&source_path) {
            self.source_text = Some(source);
            self.source_path = Some(source_path.clone());
            self.events
                .push(DebugEvent::CompanionSourceDiscovered(CompanionSource {
                    path: source_path,
                    reason: "sibling source is an unverified display companion; bytecode carries no source digest"
                        .into(),
                }));
        }
        Ok(())
    }

    fn refresh_current_disassembly(&mut self) {
        let Some((code_id, bytecode)) = self.engine.current_code() else {
            return;
        };
        if self.current_code_id == Some(code_id) {
            return;
        }
        let Ok(text) = tcld_decompile::disassemble_bytecode(
            &bytecode,
            tcld_decompile::DisassembleOptions::default(),
        ) else {
            return;
        };
        self.current_code_id = Some(code_id);
        self.current_bytecode = Some(bytecode.clone());
        self.disassembly.clear();
        self.disassembly.extend(text.lines().map(str::to_owned));
        self.events.push(DebugEvent::CodeRegistered(RegisteredCode {
            id: DebugCodeId(code_id.0),
            name: self.current_proc_qname.clone(),
            source_block_id: None,
        }));
    }

    fn source_base_for_proc(&self, qname: Option<&str>) -> Option<usize> {
        let source = self.source_text.as_deref()?;
        let Some(qname) = qname else {
            return Some(0);
        };
        let leaf = qname.rsplit("::").next().unwrap_or(qname);
        let declaration = source.lines().enumerate().find_map(|(index, line)| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("proc ")?.trim_start();
            let name = rest.split_whitespace().next()?;
            (name == qname || name.trim_start_matches("::") == leaf).then_some(index + 1)
        })?;
        Some(
            source
                .lines()
                .take(declaration)
                .map(|line| line.len() + 1)
                .sum(),
        )
    }

    fn companion_location_for_frame(
        &self,
        frame: &tcld_vm::UiStackFrame,
    ) -> Option<CompanionSourceLocation> {
        let offset = frame.source_offset?;
        let source = self.source_text.as_deref()?;
        let source_path = self.source_path.as_ref()?;
        let base = self.source_base_for_proc(frame.proc_qname.as_deref())?;
        let absolute = base.saturating_add(offset).min(source.len());
        let line = source.as_bytes()[..absolute]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32
            + 1;
        let source_range = self
            .current_bytecode
            .as_ref()?
            .cmd_source_ranges
            .as_ref()?
            .iter()
            .find(|range| usize::try_from(range.start).ok() == Some(offset))?;
        let offset_start = u64::try_from(source_range.start).ok()?;
        let offset_end = u64::try_from(source_range.end).ok()?;
        if offset_end <= offset_start {
            return None;
        }
        Some(CompanionSourceLocation {
            path: source_path.clone(),
            line,
            qualified_proc: frame.proc_qname.clone(),
            span: SourceCommandSpan::new(offset_start, offset_end).ok()?,
            reason: "derived from an unverified sibling file and bytecode command offsets".into(),
        })
    }

    fn push_captured_output(&mut self) {
        if self.mode == tcld_vm::RuntimeMode::Hybrid
            && let Ok((stdout, stderr)) = self.engine.captured_output()
        {
            if stdout.len() > self.captured_stdout_len {
                self.events.push(DebugEvent::Output {
                    stream: super::contract::DebugOutputStream::Stdout,
                    text: String::from_utf8_lossy(&stdout[self.captured_stdout_len..]).into_owned(),
                });
                self.captured_stdout_len = stdout.len();
            }
            if stderr.len() > self.captured_stderr_len {
                self.events.push(DebugEvent::Output {
                    stream: super::contract::DebugOutputStream::Stderr,
                    text: String::from_utf8_lossy(&stderr[self.captured_stderr_len..]).into_owned(),
                });
                self.captured_stderr_len = stderr.len();
            }
        }
    }

    fn push_outcome(&mut self, outcome: tcld_vm::DebugOutcome) {
        self.push_captured_output();
        match outcome {
            tcld_vm::DebugOutcome::Paused(s) => {
                self.current_proc_qname = s.proc_qname.clone();
                self.refresh_current_disassembly();
                let code = s.address.map(|address| DebugCodeLocation {
                    code_id: DebugCodeId(address.code_id.0),
                    pc: u64::from(address.pc),
                });
                let source_hint = if self.image.is_some() {
                    None
                } else {
                    s.frames
                        .first()
                        .and_then(|frame| self.companion_location_for_frame(frame))
                };
                let source = s
                    .address
                    .and_then(|address| self.mapped_source_location(address));
                let location = DebugLocation {
                    source,
                    code,
                    observed_pc: s.pc.map(u64::from),
                    source_hint,
                };
                let reason = match s.reason {
                    tcld_vm::PauseReason::Entry => DebugPauseReason::Entry,
                    tcld_vm::PauseReason::InstructionStep => DebugPauseReason::InstructionStep,
                    tcld_vm::PauseReason::StepOver => DebugPauseReason::StepOver,
                    tcld_vm::PauseReason::StepOut => DebugPauseReason::StepOut,
                    tcld_vm::PauseReason::Breakpoint => DebugPauseReason::InstructionBreakpoint,
                    tcld_vm::PauseReason::RunToPc => DebugPauseReason::RunToLocation,
                    tcld_vm::PauseReason::Completed => return,
                };
                let hit_breakpoint = if s.reason == tcld_vm::PauseReason::Breakpoint {
                    let address = s.address.map(|address| DebugCodeLocation {
                        code_id: DebugCodeId(address.code_id.0),
                        pc: u64::from(address.pc),
                    });
                    let id = self.breakpoints.iter().find_map(|(id, breakpoint)| {
                        let hit = address.is_some_and(|address| match &breakpoint.resolution {
                            DebugBreakpointResolution::Resolved { locations, .. } => locations
                                .iter()
                                .any(|location| location.code == Some(address)),
                            DebugBreakpointResolution::Pending { .. } => false,
                        });
                        hit.then(|| id.clone())
                    });
                    if let Some(id) = id.as_ref()
                        && let Some(breakpoint) = self.breakpoints.get_mut(id)
                    {
                        breakpoint.hit_count = breakpoint.hit_count.saturating_add(1);
                    }
                    id
                } else {
                    None
                };
                let reason = if hit_breakpoint
                    .as_ref()
                    .and_then(|id| self.breakpoints.get(id))
                    .is_some_and(|breakpoint| {
                        !matches!(breakpoint.spec, DebugBreakpointSpec::Instruction { .. })
                    }) {
                    DebugPauseReason::Breakpoint
                } else {
                    reason
                };
                self.events.push(DebugEvent::Paused(Box::new(BackendPause {
                    location,
                    in_host: s.in_host,
                    reason,
                    proc_qname: s.proc_qname,
                    frame_count: u32::try_from(s.frames.len()).unwrap_or(u32::MAX),
                    breakpoint: hit_breakpoint,
                    locals: s.locals,
                    error: None,
                })));
            }
            tcld_vm::DebugOutcome::Finished { result } => {
                self.finished = true;
                self.last_result = result.as_string().ok();
                self.events.push(DebugEvent::Completed {
                    result: self.last_result.clone(),
                });
                self.events
                    .push(DebugEvent::StateChanged(DebugLifecycleState::Completed));
            }
        }
    }
}

#[cfg(feature = "hybrid")]
impl DebugBackend for HybridInProcessBackend {
    fn capabilities(&self) -> BackendCapabilities {
        if self.image.is_some() {
            BackendCapabilities::compiled_source_hybrid(self.mode)
        } else {
            BackendCapabilities::hybrid(self.mode)
        }
    }

    fn start_target(&mut self, path: &Path, stop_on_entry: bool) -> Result<(), BackendError> {
        if self.image.is_none()
            && !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("tbc"))
        {
            return Err(BackendError::Message(
                "raw hybrid debug requires a .tbc target; Tcl source must be supplied as a verified DebugImage"
                    .into(),
            ));
        }
        self.target = Some(path.to_path_buf());
        self.finished = false;
        self.runtime_error_paused = false;
        self.events.clear();
        self.events
            .push(DebugEvent::StateChanged(DebugLifecycleState::Loading));
        self.pc_breakpoints.clear();
        self.breakpoints.clear();
        self.next_breakpoint_id = 1;
        self.current_code_id = None;
        self.current_bytecode = None;
        self.current_proc_qname = None;
        self.image_bindings.clear();
        let outcome = if let Some(image) = self.image.clone() {
            let requested = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if requested != image.source.path {
                return Err(BackendError::Message(format!(
                    "prepared image belongs to {}, not {}",
                    image.source.path.display(),
                    requested.display()
                )));
            }
            self.source_text = Some(image.source.content.to_string());
            self.source_path = Some(image.source.path.clone());
            let started = self
                .engine
                .debug_start_image(&image, stop_on_entry)
                .map_err(|error| BackendError::Message(error.to_string()))?;
            self.image_bindings.extend(
                started
                    .bindings
                    .into_iter()
                    .map(|binding| (binding.code_id, binding.logical_unit_id)),
            );
            self.register_image_events();
            started.outcome
        } else {
            self.load_disassembly(path)?;
            self.engine
                .debug_start_tbc_path(path, stop_on_entry)
                .map_err(|e| BackendError::Message(e.to_string()))?
        };
        self.push_outcome(outcome);
        Ok(())
    }

    fn restart_target(&mut self, stop_on_entry: bool) -> Result<(), BackendError> {
        HybridInProcessBackend::restart(self, stop_on_entry)
    }

    fn continue_execution(&mut self) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        let outcome = match self.engine.continue_run() {
            Ok(outcome) => outcome,
            Err(error) if self.image.is_some() => {
                self.push_runtime_error(error);
                return Ok(());
            }
            Err(error) => return Err(BackendError::Message(error.to_string())),
        };
        self.events
            .push(DebugEvent::StateChanged(DebugLifecycleState::Running));
        self.push_outcome(outcome);
        Ok(())
    }

    fn pause_execution(&mut self) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            "pause",
            self.capabilities()
                .support(BackendOperation::Pause)
                .reason()
                .unwrap_or("pause is unavailable"),
        ))
    }

    fn step(&mut self, kind: DebugStepKind) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        if self.image.is_none() && matches!(kind, DebugStepKind::Into | DebugStepKind::Over) {
            return Err(BackendError::unsupported(
                "source_step",
                "hybrid source-command stepping requires a verified command map; use instruction stepping",
            ));
        }
        let outcome: Result<_, BackendError> = match kind {
            DebugStepKind::Into => self.source_step(false),
            DebugStepKind::Over => self.source_step(true),
            DebugStepKind::InstructionInto => self
                .engine
                .step_instruction()
                .map_err(|error| BackendError::Message(error.to_string())),
            DebugStepKind::InstructionOver => self
                .engine
                .step_over()
                .map_err(|error| BackendError::Message(error.to_string())),
            DebugStepKind::InstructionOut | DebugStepKind::Out => self
                .engine
                .step_out()
                .map_err(|error| BackendError::Message(error.to_string())),
        };
        let outcome = outcome?;
        self.events
            .push(DebugEvent::StateChanged(DebugLifecycleState::Running));
        self.push_outcome(outcome);
        Ok(())
    }

    fn run_to_location(&mut self, location: DebugLocation) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        let code = if let Some(code) = location.code {
            code
        } else if let Some(source) = location.source {
            let spec = DebugBreakpointSpec::SourceCommand {
                block_id: source.block_id,
                source_path: source.source_path.map(PathBuf::from),
                qualified_proc: source.qualified_proc,
                span: SourceCommandSpan::new(source.offset_start, source.offset_end)
                    .map_err(BackendError::Message)?,
                condition: None,
            };
            let resolved = self.resolve_image_source_breakpoint(&spec)?;
            let (address, _) = resolved.into_iter().next().ok_or_else(|| {
                BackendError::Message(
                    "selected source command has no executable image address".into(),
                )
            })?;
            DebugCodeLocation {
                code_id: DebugCodeId(address.code_id.0),
                pc: u64::from(address.pc),
            }
        } else {
            return Err(BackendError::unsupported(
                "run_to_location",
                "hybrid run-to requires a verified source command or code-object-qualified PC",
            ));
        };
        if Some(tcld_vm::load::CodeId(code.code_id.0)) != self.current_code_id {
            return Err(BackendError::Message(format!(
                "code object {} is not the currently suspended code object",
                code.code_id.0
            )));
        }
        let outcome = self
            .engine
            .run_to_pc(u32::try_from(code.pc).map_err(|_| {
                BackendError::Message(format!("PC {} exceeds the VM address range", code.pc))
            })?)
            .map_err(|e| BackendError::Message(e.to_string()))?;
        self.events
            .push(DebugEvent::StateChanged(DebugLifecycleState::Running));
        self.push_outcome(outcome);
        Ok(())
    }

    fn stack_frames(&mut self) -> Result<Vec<DebugStackFrame>, BackendError> {
        let frames = self.engine.ui_stack_frames();
        Ok(frames
            .into_iter()
            .enumerate()
            .map(|(i, frame)| {
                let source_hint = if self.image.is_some() {
                    None
                } else {
                    self.companion_location_for_frame(&frame)
                };
                let code = frame.address.map(|address| DebugCodeLocation {
                    code_id: DebugCodeId(address.code_id.0),
                    pc: u64::from(address.pc),
                });
                let source = frame
                    .address
                    .and_then(|address| self.mapped_source_location(address));
                DebugStackFrame {
                    level: i as i64,
                    kind: frame.kind_label,
                    proc_name: frame.proc_qname,
                    location: DebugLocation {
                        source,
                        code,
                        observed_pc: frame.pc.map(u64::from),
                        source_hint,
                    },
                }
            })
            .collect())
    }

    fn variables_for_frame(&mut self, level: i64) -> Result<Vec<DebugVariable>, BackendError> {
        Ok(self
            .engine
            .ui_frame_locals(level.max(0) as usize)
            .into_iter()
            .map(|(name, value)| DebugVariable { name, value })
            .collect())
    }

    fn evaluate(&mut self, level: i64, script: &str) -> Result<DebugEvaluation, BackendError> {
        if !self
            .capabilities()
            .support(BackendOperation::Evaluate)
            .is_supported()
        {
            return Err(BackendError::unsupported(
                "evaluate",
                self.capabilities()
                    .support(BackendOperation::Evaluate)
                    .reason()
                    .unwrap_or("evaluation is unavailable"),
            ));
        }
        let completion = self
            .engine
            .eval_in_frame(level.max(0) as usize, tcld_vm::VmObj::from_str(script))
            .map_err(|error| BackendError::Message(error.to_string()))?;
        Ok(DebugEvaluation {
            completion_code: completion.code,
            result: completion
                .result
                .as_string()
                .unwrap_or_else(|_| format!("{:?}", completion.result)),
        })
    }

    fn add_breakpoint(
        &mut self,
        spec: DebugBreakpointSpec,
    ) -> Result<DebugBreakpoint, BackendError> {
        let id = DebugBreakpointId(format!("hybrid-bp{}", self.next_breakpoint_id));
        self.next_breakpoint_id += 1;
        let resolution = match &spec {
            DebugBreakpointSpec::Instruction {
                location,
                condition,
            } => {
                if condition.is_some() {
                    return Err(BackendError::unsupported(
                        "conditional_instruction_breakpoint",
                        "the hybrid VM does not yet evaluate breakpoint conditions",
                    ));
                }
                if Some(tcld_vm::load::CodeId(location.code_id.0)) != self.current_code_id {
                    return Err(BackendError::Message(format!(
                        "code object {} is not the currently suspended code object",
                        location.code_id.0
                    )));
                }
                let address = tcld_vm::CodeAddress {
                    code_id: tcld_vm::load::CodeId(location.code_id.0),
                    pc: u32::try_from(location.pc).map_err(|_| {
                        BackendError::Message(format!(
                            "PC {} exceeds the VM address range",
                            location.pc
                        ))
                    })?,
                };
                self.engine.add_breakpoint(address);
                self.pc_breakpoints.insert(address);
                DebugBreakpointResolution::Resolved {
                    generation: None,
                    locations: vec![DebugLocation::code(*location)],
                }
            }
            DebugBreakpointSpec::SourceLine { condition, .. } => {
                if condition.is_some() {
                    return Err(BackendError::unsupported(
                        "conditional_source_breakpoint",
                        "the hybrid VM does not yet evaluate breakpoint conditions",
                    ));
                }
                let resolved = self.resolve_image_source_breakpoint(&spec)?;
                if resolved.is_empty() {
                    DebugBreakpointResolution::Pending {
                        reason: if self.image.is_some() {
                            "no admitted command begins on the requested source line".into()
                        } else {
                            "hybrid has only an unverified companion source; a canonical source map is required before arming this breakpoint".into()
                        },
                    }
                } else {
                    let mut locations = Vec::new();
                    for (address, source) in resolved {
                        self.engine.add_breakpoint(address);
                        self.pc_breakpoints.insert(address);
                        locations.push(DebugLocation::mapped(
                            source,
                            DebugCodeLocation {
                                code_id: DebugCodeId(address.code_id.0),
                                pc: u64::from(address.pc),
                            },
                        ));
                    }
                    DebugBreakpointResolution::Resolved {
                        generation: Some(1),
                        locations,
                    }
                }
            }
            DebugBreakpointSpec::SourceCommand { condition, .. } => {
                if condition.is_some() {
                    return Err(BackendError::unsupported(
                        "conditional_source_breakpoint",
                        "the hybrid VM does not yet evaluate breakpoint conditions",
                    ));
                }
                let resolved = self.resolve_image_source_breakpoint(&spec)?;
                if resolved.is_empty() {
                    DebugBreakpointResolution::Pending {
                        reason: if self.image.is_some() {
                            "the source command does not belong to this compiled image generation"
                                .into()
                        } else {
                            "hybrid has only an unverified companion source; a canonical source map is required before arming this breakpoint".into()
                        },
                    }
                } else {
                    let mut locations = Vec::new();
                    for (address, source) in resolved {
                        self.engine.add_breakpoint(address);
                        self.pc_breakpoints.insert(address);
                        locations.push(DebugLocation::mapped(
                            source,
                            DebugCodeLocation {
                                code_id: DebugCodeId(address.code_id.0),
                                pc: u64::from(address.pc),
                            },
                        ));
                    }
                    DebugBreakpointResolution::Resolved {
                        generation: Some(1),
                        locations,
                    }
                }
            }
        };
        let breakpoint = DebugBreakpoint {
            id: id.clone(),
            spec,
            enabled: true,
            resolution,
            hit_count: 0,
            ignore_count: 0,
        };
        self.breakpoints.insert(id, breakpoint.clone());
        Ok(breakpoint)
    }

    fn remove_breakpoint(&mut self, id: &DebugBreakpointId) -> Result<(), BackendError> {
        let breakpoint = self
            .breakpoints
            .remove(id)
            .ok_or_else(|| BackendError::Message(format!("breakpoint {} not found", id.0)))?;
        if let DebugBreakpointResolution::Resolved { locations, .. } = breakpoint.resolution {
            for code in locations.into_iter().filter_map(|location| location.code) {
                let address = tcld_vm::CodeAddress {
                    code_id: tcld_vm::load::CodeId(code.code_id.0),
                    pc: u32::try_from(code.pc).map_err(|_| {
                        BackendError::Message(format!("PC {} exceeds VM address range", code.pc))
                    })?,
                };
                self.engine.remove_breakpoint(address);
                self.pc_breakpoints.remove(&address);
            }
        }
        Ok(())
    }

    fn breakpoints(&mut self) -> Result<Vec<DebugBreakpoint>, BackendError> {
        Ok(self.breakpoints.values().cloned().collect())
    }

    fn set_breakpoint_enabled(
        &mut self,
        id: &DebugBreakpointId,
        enabled: bool,
    ) -> Result<(), BackendError> {
        let breakpoint = self
            .breakpoints
            .get_mut(id)
            .ok_or_else(|| BackendError::Message(format!("breakpoint {} not found", id.0)))?;
        if breakpoint.enabled == enabled {
            return Ok(());
        }
        if let DebugBreakpointResolution::Resolved { locations, .. } = &breakpoint.resolution {
            for code in locations.iter().filter_map(|location| location.code) {
                let address = tcld_vm::CodeAddress {
                    code_id: tcld_vm::load::CodeId(code.code_id.0),
                    pc: u32::try_from(code.pc).map_err(|_| {
                        BackendError::Message(format!("PC {} exceeds VM address range", code.pc))
                    })?,
                };
                if enabled {
                    self.engine.add_breakpoint(address);
                    self.pc_breakpoints.insert(address);
                } else {
                    self.engine.remove_breakpoint(address);
                    self.pc_breakpoints.remove(&address);
                }
            }
        }
        breakpoint.enabled = enabled;
        Ok(())
    }

    fn drain_events(&mut self) -> Vec<DebugEvent> {
        std::mem::take(&mut self.events)
    }

    fn state(&mut self) -> Result<DebugLifecycleState, BackendError> {
        if self.finished {
            Ok(DebugLifecycleState::Completed)
        } else if self.runtime_error_paused || self.engine.pause_snapshot().is_some() {
            Ok(DebugLifecycleState::Paused)
        } else {
            Ok(DebugLifecycleState::Running)
        }
    }

    fn sources(&mut self) -> Result<Vec<SourceSnapshot>, BackendError> {
        let Some(image) = self.image.clone() else {
            return Ok(Vec::new());
        };
        Ok(image
            .units
            .iter()
            .map(|unit| {
                let (identity, qname) = match &unit.kind {
                    tcld_compile::DebugCodeUnitKind::TopLevel => (
                        DebugSourceUnitIdentity::File(image.source.path.clone()),
                        None,
                    ),
                    tcld_compile::DebugCodeUnitKind::Procedure { qname, .. } => (
                        DebugSourceUnitIdentity::Procedure(qname.clone()),
                        Some(qname.clone()),
                    ),
                };
                SourceSnapshot {
                    source: RegisteredSource {
                        identity,
                        block_id: self.image_block_id(unit).expect("image block"),
                        generation: 1,
                        path: Some(image.source.path.clone()),
                        qualified_proc: qname,
                        definition_path: Some(image.source.path.clone()),
                        restart_limitation: None,
                    },
                    text: image.source.content.to_string(),
                }
            })
            .collect())
    }

    fn companion_sources(&mut self) -> Result<Vec<CompanionSourceSnapshot>, BackendError> {
        if self.image.is_some() {
            return Ok(Vec::new());
        }
        match (&self.source_path, &self.source_text) {
            (Some(path), Some(text)) => Ok(vec![CompanionSourceSnapshot {
                source: CompanionSource {
                    path: path.clone(),
                    reason:
                        "sibling source is unverified because bytecode carries no source digest"
                            .into(),
                },
                text: text.clone(),
            }]),
            _ => Ok(Vec::new()),
        }
    }

    fn disassembly(&mut self) -> Result<Vec<DisassemblySnapshot>, BackendError> {
        let Some(code_id) = self.current_code_id else {
            return Ok(Vec::new());
        };
        Ok(vec![DisassemblySnapshot {
            code: RegisteredCode {
                id: DebugCodeId(code_id.0),
                name: self.current_proc_qname.clone(),
                source_block_id: self
                    .image_unit_for_code(code_id)
                    .and_then(|unit| self.image_block_id(unit)),
            },
            instruction_set: self.instruction_set_name().map(str::to_owned),
            lines: self.disassembly.clone(),
        }])
    }

    fn coverage_snapshot(&mut self) -> Result<Vec<tcld_tui::debug::CoverageRow>, BackendError> {
        Ok(HybridInProcessBackend::coverage_snapshot(self)
            .executed_by_opcode
            .into_iter()
            .map(|(opcode, hits)| tcld_tui::debug::CoverageRow {
                block: opcode,
                offset: 0,
                hits,
            })
            .collect())
    }

    fn set_coverage_enabled(&mut self, _enabled: bool) -> Result<(), BackendError> {
        Ok(())
    }

    fn procedures(&mut self) -> Result<Vec<DebugProcedure>, BackendError> {
        let Some(image) = self.image.as_ref() else {
            return Err(self.unsupported(BackendOperation::ProcedureList, "procedure list"));
        };
        let active = self
            .engine
            .vm_proc_names()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        Ok(image
            .units
            .iter()
            .filter_map(|unit| {
                let tcld_compile::DebugCodeUnitKind::Procedure { qname, .. } = &unit.kind else {
                    return None;
                };
                if !active.contains(qname) {
                    return None;
                }
                Some(DebugProcedure {
                    qualified_name: qname.clone(),
                    block_id: self.image_block_id(unit),
                    first_offset: unit
                        .commands
                        .first()
                        .map(|map| map.source_span.start as u64),
                })
            })
            .collect())
    }

    fn close(self: Box<Self>) {}
}

// ── Wire session adapter ─────────────────────────────────────────────

/// Adapter around the existing wire `Session` for host-shell debug.
pub struct WireSessionBackend {
    session: super::session::Session,
    target: Option<PathBuf>,
    source_registry: BTreeMap<String, RegisteredSource>,
    pending_events: Vec<DebugEvent>,
}

impl WireSessionBackend {
    pub fn connect(paths: super::config::NubPaths) -> Result<Self, BackendError> {
        let session = super::session::Session::connect(paths).map_err(BackendError::Message)?;
        Ok(Self {
            session,
            target: None,
            source_registry: BTreeMap::new(),
            pending_events: Vec::new(),
        })
    }

    fn register_source(&mut self, source: RegisteredSource) {
        self.source_registry.insert(source.block_id.clone(), source);
    }

    fn enrich_location(&self, location: &mut DebugLocation) {
        let Some(source) = location.source.as_mut() else {
            return;
        };
        let Some(registered) = self.source_registry.get(&source.block_id) else {
            return;
        };
        // Only owner-derived definition provenance may fill this display
        // field. It does not change the canonical block identity.
        if source.source_path.is_none() {
            source.source_path = registered
                .definition_path
                .as_ref()
                .map(|path| path.display().to_string());
        }
    }
}

impl DebugBackend for WireSessionBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::wire_host()
    }

    fn start_target(&mut self, path: &Path, stop_on_entry: bool) -> Result<(), BackendError> {
        let path = std::fs::canonicalize(path).map_err(|error| {
            BackendError::Message(format!(
                "normalize debug target {}: {error}",
                path.display()
            ))
        })?;
        self.target = Some(path.clone());
        self.session
            .start_target(&path, stop_on_entry)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn restart_target(&mut self, stop_on_entry: bool) -> Result<(), BackendError> {
        let path = self
            .target
            .clone()
            .ok_or_else(|| BackendError::Message("no target has been started".into()))?;
        // The wire backend owns every breakpoint accepted through this
        // adapter. Export before replacing the interpreter, then import into
        // the fresh nub before any target code runs. UI-only watch expressions
        // are intentionally not part of this backend contract.
        let breakpoints = self
            .session
            .export_breakpoints()
            .map_err(BackendError::Message)?;
        let unavailable: Vec<_> = self
            .source_registry
            .values()
            .filter(|source| !source.identity.is_restart_stable())
            .cloned()
            .collect();
        self.session.reconnect().map_err(BackendError::Message)?;
        self.source_registry.clear();
        self.pending_events
            .extend(
                unavailable
                    .into_iter()
                    .map(|source| DebugEvent::SourceUnavailable {
                        identity: source.identity,
                        block_id: source.block_id,
                        reason: source.restart_limitation.unwrap_or_else(|| {
                            "dynamic source unit belongs to the previous interpreter session".into()
                        }),
                    }),
            );
        self.session
            .import_breakpoints(&breakpoints)
            .map_err(BackendError::Message)?;
        self.start_target(&path, stop_on_entry)
    }

    fn continue_execution(&mut self) -> Result<(), BackendError> {
        use tcld_debug_wire::Op;
        self.session
            .send(Op::Continue)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn pause_execution(&mut self) -> Result<(), BackendError> {
        self.session
            .send(tcld_debug_wire::Op::Interrupt)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn step(&mut self, kind: DebugStepKind) -> Result<(), BackendError> {
        use tcld_debug_wire::Op;
        let op = match kind {
            DebugStepKind::Into => Op::StepInto,
            DebugStepKind::Over => Op::StepOver,
            DebugStepKind::Out => Op::StepOut,
            DebugStepKind::InstructionInto
            | DebugStepKind::InstructionOver
            | DebugStepKind::InstructionOut => {
                return Err(BackendError::unsupported(
                    "instruction_step",
                    self.capabilities()
                        .support(BackendOperation::StepInstruction)
                        .reason()
                        .unwrap_or("instruction stepping is unavailable"),
                ));
            }
        };
        self.session.send(op).map_err(BackendError::Message)?;
        Ok(())
    }

    fn run_to_location(&mut self, _location: DebugLocation) -> Result<(), BackendError> {
        Err(BackendError::unsupported(
            "run_to_location",
            self.capabilities()
                .support(BackendOperation::RunToLocation)
                .reason()
                .unwrap_or("run-to is unavailable"),
        ))
    }

    fn stack_frames(&mut self) -> Result<Vec<DebugStackFrame>, BackendError> {
        let mut frames: Vec<DebugStackFrame> = self
            .session
            .stack()
            .map(|frames| frames.into_iter().map(tui_stack_frame_to_backend).collect())
            .map_err(BackendError::Message)?;
        for frame in &mut frames {
            self.enrich_location(&mut frame.location);
        }
        Ok(frames)
    }

    fn variables_for_frame(&mut self, level: i64) -> Result<Vec<DebugVariable>, BackendError> {
        self.session
            .variables(level)
            .map(|variables| {
                variables
                    .into_iter()
                    .map(|(name, value)| DebugVariable { name, value })
                    .collect()
            })
            .map_err(BackendError::Message)
    }

    fn evaluate(&mut self, level: i64, script: &str) -> Result<DebugEvaluation, BackendError> {
        use tcld_debug_wire::Op;
        let result = self
            .session
            .send(Op::EvalInFrame {
                level,
                script: script.to_string(),
            })
            .map_err(BackendError::Message)?;
        Ok(DebugEvaluation {
            completion_code: 0,
            result,
        })
    }

    fn add_breakpoint(
        &mut self,
        spec: DebugBreakpointSpec,
    ) -> Result<DebugBreakpoint, BackendError> {
        let id = match &spec {
            DebugBreakpointSpec::SourceLine {
                path,
                line,
                condition,
            } => {
                let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                self.session
                    .add_file_breakpoint(&canonical, *line, condition.as_deref())
            }
            DebugBreakpointSpec::SourceCommand {
                source_path: Some(path),
                span,
                condition,
                ..
            } => self.session.add_file_command_breakpoint(
                path,
                span.offset_start,
                span.byte_len(),
                condition.as_deref(),
            ),
            DebugBreakpointSpec::SourceCommand {
                qualified_proc: Some(qname),
                span,
                condition,
                ..
            } => self.session.add_command_breakpoint(
                qname,
                span.offset_start,
                span.byte_len(),
                condition.as_deref(),
            ),
            DebugBreakpointSpec::SourceCommand { .. } => {
                return Err(BackendError::Message(
                    "source command breakpoint requires a file path or qualified procedure".into(),
                ));
            }
            DebugBreakpointSpec::Instruction { .. } => {
                return Err(BackendError::unsupported(
                    "instruction_breakpoint",
                    self.capabilities()
                        .support(BackendOperation::InstructionBreakpoints)
                        .reason()
                        .unwrap_or("instruction breakpoints are unavailable"),
                ));
            }
        }
        .map_err(BackendError::Message)?;
        let id = DebugBreakpointId(id.trim().to_owned());
        if let Ok(confirmed) = self.session.breakpoints()
            && let Some(confirmed) = confirmed
                .into_iter()
                .find(|breakpoint| breakpoint.id == id.0)
        {
            return Ok(wire_breakpoint_to_backend(confirmed));
        }
        Ok(DebugBreakpoint {
            id,
            spec,
            enabled: true,
            resolution: DebugBreakpointResolution::Pending {
                reason: "awaiting backend breakpoint-list confirmation".into(),
            },
            hit_count: 0,
            ignore_count: 0,
        })
    }

    fn remove_breakpoint(&mut self, id: &DebugBreakpointId) -> Result<(), BackendError> {
        self.session
            .remove_breakpoint(&id.0)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn breakpoints(&mut self) -> Result<Vec<DebugBreakpoint>, BackendError> {
        self.session
            .breakpoints()
            .map(|breakpoints| {
                breakpoints
                    .into_iter()
                    .map(wire_breakpoint_to_backend)
                    .collect()
            })
            .map_err(BackendError::Message)
    }

    fn set_breakpoint_enabled(
        &mut self,
        id: &DebugBreakpointId,
        enabled: bool,
    ) -> Result<(), BackendError> {
        self.session
            .enable_breakpoint(&id.0, enabled)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn drain_events(&mut self) -> Vec<DebugEvent> {
        let mut events = std::mem::take(&mut self.pending_events);
        let translated: Vec<_> = self
            .session
            .drain_events()
            .into_iter()
            .flat_map(wire_event_to_backend)
            .collect();
        for mut event in translated {
            match &mut event {
                DebugEvent::SourceRegistered(source) | DebugEvent::SourceRevalidated(source) => {
                    self.register_source(source.clone());
                }
                DebugEvent::Paused(pause) => {
                    if let Some(source) = pause.location.source.as_ref()
                        && !self.source_registry.contains_key(&source.block_id)
                    {
                        let registration = source_registration_from_block(
                            source.block_id.clone(),
                            0,
                            source.source_path.clone().map(PathBuf::from),
                        );
                        self.register_source(registration.clone());
                        events.push(DebugEvent::SourceRegistered(registration));
                    }
                    self.enrich_location(&mut pause.location);
                }
                _ => {}
            }
            events.push(event);
        }
        events
    }

    fn state(&mut self) -> Result<DebugLifecycleState, BackendError> {
        self.session
            .app_state()
            .map(Into::into)
            .map_err(BackendError::Message)
    }

    fn sources(&mut self) -> Result<Vec<SourceSnapshot>, BackendError> {
        let registrations: Vec<_> = self.source_registry.values().cloned().collect();
        Ok(registrations
            .into_iter()
            .filter_map(|source| {
                self.session
                    .block_source(&source.block_id)
                    .ok()
                    .map(|text| SourceSnapshot { source, text })
            })
            .collect())
    }

    fn companion_sources(&mut self) -> Result<Vec<CompanionSourceSnapshot>, BackendError> {
        Ok(Vec::new())
    }

    fn disassembly(&mut self) -> Result<Vec<DisassemblySnapshot>, BackendError> {
        Err(BackendError::unsupported(
            "disassembly",
            self.capabilities()
                .support(BackendOperation::DisassemblyAccess)
                .reason()
                .unwrap_or("disassembly is unavailable"),
        ))
    }

    fn procedures(&mut self) -> Result<Vec<DebugProcedure>, BackendError> {
        self.session
            .proc_list()
            .map(|procedures| {
                procedures
                    .into_iter()
                    .map(|procedure| DebugProcedure {
                        qualified_name: procedure.qname,
                        block_id: procedure.block,
                        first_offset: procedure.first_offset,
                    })
                    .collect()
            })
            .map_err(BackendError::Message)
    }

    fn procedure_location(&mut self, qualified_name: &str) -> Result<Option<u64>, BackendError> {
        self.session
            .proc_location(qualified_name)
            .map_err(BackendError::Message)
    }

    fn procedure_source(&mut self, qualified_name: &str) -> Result<SourceSnapshot, BackendError> {
        let bare = qualified_name.trim_start_matches("::");
        let source = self
            .source_registry
            .values()
            .find(|source| source.qualified_proc.as_deref() == Some(bare))
            .cloned()
            .ok_or_else(|| {
                BackendError::Message(format!("procedure source not registered: {qualified_name}"))
            })?;
        let text = self
            .session
            .block_source(&source.block_id)
            .map_err(BackendError::Message)?;
        Ok(SourceSnapshot { source, text })
    }

    fn watch_entries(&mut self) -> Result<Vec<tcld_tui::debug::WatchEntry>, BackendError> {
        self.session.watches().map_err(BackendError::Message)
    }

    fn evaluate_watch_entries(
        &mut self,
        level: i64,
    ) -> Result<Vec<tcld_tui::debug::WatchEntry>, BackendError> {
        self.session
            .evaluate_watches(level)
            .map_err(BackendError::Message)
    }

    fn add_watch(&mut self, name: &str, expression: &str) -> Result<(), BackendError> {
        self.session
            .watch_add(name, expression)
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn remove_watch(&mut self, id: &str) -> Result<(), BackendError> {
        self.session
            .watch_remove(id)
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn add_watchpoint(&mut self, name: &str, operations: &str) -> Result<(), BackendError> {
        self.session
            .watchpoint_add(name, operations)
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn coverage_snapshot(&mut self) -> Result<Vec<tcld_tui::debug::CoverageRow>, BackendError> {
        self.session.coverage().map_err(BackendError::Message)
    }

    fn set_coverage_enabled(&mut self, enabled: bool) -> Result<(), BackendError> {
        self.session
            .coverage_enable(enabled)
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn ignore_current_error(&mut self) -> Result<(), BackendError> {
        self.session
            .ignore_error()
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn slave_interpreters(&mut self) -> Result<Vec<String>, BackendError> {
        self.session.slave_list().map_err(BackendError::Message)
    }

    fn preinstrument_procedure(
        &mut self,
        qualified_name: &str,
        body: &str,
    ) -> Result<(), BackendError> {
        self.session
            .pre_instrument_proc(qualified_name, body)
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn export_breakpoint_state(&mut self) -> Result<String, BackendError> {
        self.session
            .export_breakpoints()
            .map_err(BackendError::Message)
    }

    fn import_breakpoint_state(&mut self, serialized: &str) -> Result<(), BackendError> {
        self.session
            .import_breakpoints(serialized)
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn set_breakpoint_ignore_count(
        &mut self,
        id: &DebugBreakpointId,
        count: u64,
    ) -> Result<(), BackendError> {
        self.session
            .set_ignore(&id.0, count)
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn set_breakpoint_condition(
        &mut self,
        id: &DebugBreakpointId,
        condition: Option<&str>,
    ) -> Result<DebugBreakpointId, BackendError> {
        let row = self
            .session
            .breakpoints()
            .map_err(BackendError::Message)?
            .into_iter()
            .find(|row| row.id == id.0)
            .ok_or_else(|| BackendError::Message(format!("breakpoint {} not found", id.0)))?;
        self.session
            .update_condition(&row, condition)
            .map(DebugBreakpointId)
            .map_err(BackendError::Message)
    }

    fn set_variable(&mut self, level: i64, name: &str, value: &str) -> Result<(), BackendError> {
        self.session
            .send(tcld_debug_wire::Op::SetVar {
                level,
                name: name.to_owned(),
                value: value.to_owned(),
            })
            .map(|_| ())
            .map_err(BackendError::Message)
    }

    fn close(self: Box<Self>) {
        self.session.close();
    }
}

fn tui_stack_frame_to_backend(frame: tcld_tui::debug::StackFrame) -> DebugStackFrame {
    let source = match (
        frame.block.clone(),
        frame.block_kind,
        frame.line,
        frame.offset_start,
        frame.offset_end,
    ) {
        (Some(block_id), Some(block_kind), Some(line), Some(offset_start), Some(offset_end))
            if offset_end > offset_start =>
        {
            Some(tcld_debug_wire::SourceLocation {
                block_id,
                block_kind: match block_kind {
                    tcld_tui::debug::SourceBlockKind::File => {
                        tcld_debug_wire::SourceBlockKind::File
                    }
                    tcld_tui::debug::SourceBlockKind::Proc => {
                        tcld_debug_wire::SourceBlockKind::Proc
                    }
                    tcld_tui::debug::SourceBlockKind::Anonymous => {
                        tcld_debug_wire::SourceBlockKind::Anonymous
                    }
                },
                source_path: frame.source_path.clone(),
                qualified_proc: frame.proc_name.clone(),
                line,
                offset_start,
                offset_end,
            })
        }
        _ => None,
    };
    DebugStackFrame {
        level: frame.level,
        kind: frame.kind,
        proc_name: frame.proc_name,
        location: DebugLocation {
            source,
            code: None,
            observed_pc: None,
            source_hint: None,
        },
    }
}

fn wire_breakpoint_to_backend(row: tcld_tui::debug::ProcBreakpoint) -> DebugBreakpoint {
    let requested_span =
        SourceCommandSpan::new(row.offset, row.offset.saturating_add(row.len.max(1)))
            .expect("the normalized wire breakpoint span is non-empty");
    let spec = match (&row.source_path, row.line) {
        (Some(path), Some(line)) => DebugBreakpointSpec::SourceLine {
            path: PathBuf::from(path),
            line,
            condition: row.condition.clone(),
        },
        (source_path, _) => DebugBreakpointSpec::SourceCommand {
            block_id: if !row.qname.is_empty() {
                format!("proc:{}", row.qname)
            } else if let Some(path) = source_path {
                format!("file:{path}")
            } else {
                "anonymous".into()
            },
            source_path: source_path.as_ref().map(PathBuf::from),
            qualified_proc: (!row.qname.is_empty()).then(|| row.qname.clone()),
            span: requested_span,
            condition: row.condition.clone(),
        },
    };
    let resolution = if row.pending {
        DebugBreakpointResolution::Pending {
            reason: row.pending_reason.clone().unwrap_or_else(|| {
                "source unit or requested command location is not registered yet".into()
            }),
        }
    } else {
        let resolved_offset = row.resolved_offset.unwrap_or(row.offset);
        let resolved_len = row.resolved_len.unwrap_or(row.len).max(1);
        let resolved_span = SourceCommandSpan::new(
            resolved_offset,
            resolved_offset.saturating_add(resolved_len),
        )
        .expect("the normalized resolved wire breakpoint span is non-empty");
        let location = breakpoint_source_location(&row, resolved_span);
        DebugBreakpointResolution::Resolved {
            generation: row.resolution_generation,
            locations: location.map(DebugLocation::source).into_iter().collect(),
        }
    };
    DebugBreakpoint {
        id: DebugBreakpointId(row.id),
        spec,
        enabled: row.enabled,
        resolution,
        hit_count: row.hits,
        ignore_count: row.ignore_count,
    }
}

fn breakpoint_source_location(
    row: &tcld_tui::debug::ProcBreakpoint,
    span: SourceCommandSpan,
) -> Option<tcld_debug_wire::SourceLocation> {
    let line = row.resolved_line.or(row.line)?;
    let (default_block_id, block_kind) = if !row.qname.is_empty() {
        (
            format!("proc:{}", row.qname),
            tcld_debug_wire::SourceBlockKind::Proc,
        )
    } else if let Some(path) = &row.source_path {
        (
            format!("file:{path}"),
            tcld_debug_wire::SourceBlockKind::File,
        )
    } else {
        return None;
    };
    let block_id = row.resolved_block.clone().unwrap_or(default_block_id);
    Some(tcld_debug_wire::SourceLocation {
        block_id,
        block_kind,
        source_path: row.source_path.clone(),
        qualified_proc: (!row.qname.is_empty()).then(|| row.qname.clone()),
        line,
        offset_start: span.offset_start,
        offset_end: span.offset_end,
    })
}

fn wire_event_to_backend(event: tcld_debug_wire::WireEvent) -> Vec<DebugEvent> {
    use tcld_debug_wire::WireEvent;
    match event {
        WireEvent::TargetStarted { path, .. } => vec![
            DebugEvent::SourceRegistered(source_registration_from_block(
                format!("file:{path}"),
                0,
                Some(PathBuf::from(path)),
            )),
            DebugEvent::StateChanged(DebugLifecycleState::Running),
        ],
        WireEvent::TargetCompleted { result, .. } => vec![
            DebugEvent::Completed {
                result: Some(result),
            },
            DebugEvent::StateChanged(DebugLifecycleState::Completed),
        ],
        WireEvent::TargetLoadFailed {
            path,
            status,
            message,
            error_code,
            error_info,
        } => vec![
            DebugEvent::TargetLoadFailed {
                path: PathBuf::from(path),
                status,
                message,
                error_code,
                error_info,
            },
            DebugEvent::StateChanged(DebugLifecycleState::Failed),
        ],
        WireEvent::Paused {
            location,
            frame_count,
            breakpoint,
            reason,
            pc,
        } => vec![DebugEvent::Paused(Box::new(BackendPause {
            proc_qname: location.qualified_proc.clone(),
            location: DebugLocation {
                source: Some(location),
                code: None,
                observed_pc: pc,
                source_hint: None,
            },
            in_host: true,
            reason: wire_pause_reason(reason),
            frame_count,
            breakpoint: breakpoint.map(DebugBreakpointId),
            locals: Vec::new(),
            error: None,
        }))],
        WireEvent::Resumed => vec![DebugEvent::StateChanged(DebugLifecycleState::Running)],
        WireEvent::ErrorPaused {
            location,
            message,
            breakpoint,
            ..
        } => vec![DebugEvent::Paused(Box::new(BackendPause {
            proc_qname: location
                .as_ref()
                .and_then(|location| location.qualified_proc.clone()),
            location: DebugLocation {
                source: location,
                code: None,
                observed_pc: None,
                source_hint: None,
            },
            in_host: true,
            reason: DebugPauseReason::Error,
            frame_count: 0,
            breakpoint: breakpoint.map(DebugBreakpointId),
            locals: Vec::new(),
            error: Some(message),
        }))],
        WireEvent::ConditionError {
            breakpoint,
            message,
        } => vec![DebugEvent::Diagnostic {
            severity: DebugDiagnosticSeverity::Error,
            message: format!("breakpoint {breakpoint} condition failed: {message}"),
        }],
        WireEvent::BlocksRevalidated {
            block,
            generation,
            source_path,
            change,
        } => {
            let source =
                source_registration_from_block(block, generation, source_path.map(PathBuf::from));
            vec![match change {
                tcld_debug_wire::SourceUnitChange::Registered => {
                    DebugEvent::SourceRegistered(source)
                }
                tcld_debug_wire::SourceUnitChange::Revalidated => {
                    DebugEvent::SourceRevalidated(source)
                }
            }]
        }
        WireEvent::NubStarted {
            version,
            tcl_version,
            ..
        } => vec![
            DebugEvent::Diagnostic {
                severity: DebugDiagnosticSeverity::Info,
                message: format!("debugger nub {version} connected to Tcl {tcl_version}"),
            },
            DebugEvent::StateChanged(DebugLifecycleState::Ready),
        ],
        WireEvent::NubStopped { reason } => vec![
            DebugEvent::Diagnostic {
                severity: DebugDiagnosticSeverity::Warning,
                message: format!("debugger nub stopped: {reason}"),
            },
            DebugEvent::StateChanged(DebugLifecycleState::Disconnected),
        ],
    }
}

fn wire_pause_reason(reason: tcld_debug_wire::PauseReason) -> DebugPauseReason {
    match reason {
        tcld_debug_wire::PauseReason::Entry => DebugPauseReason::Entry,
        tcld_debug_wire::PauseReason::Step => DebugPauseReason::Step,
        tcld_debug_wire::PauseReason::Breakpoint => DebugPauseReason::Breakpoint,
        tcld_debug_wire::PauseReason::Watchpoint => DebugPauseReason::Watchpoint,
        tcld_debug_wire::PauseReason::Pause => DebugPauseReason::Pause,
        tcld_debug_wire::PauseReason::InstructionStep => DebugPauseReason::InstructionStep,
        tcld_debug_wire::PauseReason::InstructionBreakpoint => {
            DebugPauseReason::InstructionBreakpoint
        }
    }
}

fn source_registration_from_block(
    block_id: String,
    generation: u64,
    definition_path: Option<PathBuf>,
) -> RegisteredSource {
    if let Some(path) = block_id.strip_prefix("file:").map(PathBuf::from) {
        RegisteredSource {
            identity: DebugSourceUnitIdentity::File(path.clone()),
            block_id,
            generation,
            path: Some(path.clone()),
            qualified_proc: None,
            definition_path: Some(path),
            restart_limitation: None,
        }
    } else if let Some(qname) = block_id.strip_prefix("proc:").map(str::to_owned) {
        RegisteredSource {
            identity: DebugSourceUnitIdentity::Procedure(qname.clone()),
            block_id,
            generation,
            path: None,
            qualified_proc: Some(qname),
            definition_path,
            restart_limitation: None,
        }
    } else {
        RegisteredSource {
            identity: DebugSourceUnitIdentity::Anonymous {
                session_key: block_id.clone(),
            },
            block_id,
            generation,
            path: None,
            qualified_proc: None,
            definition_path,
            restart_limitation: Some(
                "eval/anonymous units are session-only and cannot be rebound safely after a fresh interpreter restart"
                    .into(),
            ),
        }
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    fn wire_source_location() -> tcld_debug_wire::SourceLocation {
        tcld_debug_wire::SourceLocation {
            block_id: "proc:::demo::work".into(),
            block_kind: tcld_debug_wire::SourceBlockKind::Proc,
            source_path: Some("/tmp/a script.tcl".into()),
            qualified_proc: Some("::demo::work".into()),
            line: 17,
            offset_start: 31,
            offset_end: 42,
        }
    }

    #[test]
    fn wire_pause_translation_is_lossless_and_does_not_qualify_observed_pc() {
        let source = wire_source_location();
        let events = wire_event_to_backend(tcld_debug_wire::WireEvent::Paused {
            location: source.clone(),
            frame_count: 3,
            breakpoint: Some("bp12".into()),
            reason: tcld_debug_wire::PauseReason::Breakpoint,
            pc: Some(99),
        });
        let [DebugEvent::Paused(pause)] = events.as_slice() else {
            panic!("expected exactly one neutral pause event");
        };
        assert_eq!(pause.location.source.as_ref(), Some(&source));
        assert_eq!(pause.location.observed_pc, Some(99));
        assert_eq!(pause.location.code, None, "wire PC has no stable code id");
        assert_eq!(pause.frame_count, 3);
        assert_eq!(pause.breakpoint, Some(DebugBreakpointId("bp12".into())));
        assert_eq!(pause.reason, DebugPauseReason::Breakpoint);
    }

    #[test]
    fn wire_error_pause_keeps_optional_source_and_error_text() {
        let source = wire_source_location();
        let events = wire_event_to_backend(tcld_debug_wire::WireEvent::ErrorPaused {
            location: Some(source.clone()),
            message: "boom".into(),
            error_code: Some("TCL ERROR".into()),
            breakpoint: Some("bp9".into()),
        });
        let [DebugEvent::Paused(pause)] = events.as_slice() else {
            panic!("expected neutral error pause");
        };
        assert_eq!(pause.location.source.as_ref(), Some(&source));
        assert_eq!(pause.reason, DebugPauseReason::Error);
        assert_eq!(pause.error.as_deref(), Some("boom"));
    }

    #[test]
    fn wire_lifecycle_events_translate_to_typed_states() {
        let events = wire_event_to_backend(tcld_debug_wire::WireEvent::TargetCompleted {
            path: "/tmp/demo.tcl".into(),
            status: "ok".into(),
            result: "42".into(),
        });
        assert_eq!(
            events,
            vec![
                DebugEvent::Completed {
                    result: Some("42".into())
                },
                DebugEvent::StateChanged(DebugLifecycleState::Completed)
            ]
        );
    }

    #[test]
    fn wire_load_failure_preserves_typed_diagnostic_context() {
        let events = wire_event_to_backend(tcld_debug_wire::WireEvent::TargetLoadFailed {
            path: "/tmp/broken.tcl".into(),
            status: "error".into(),
            message: "missing close-brace".into(),
            error_code: "TCL PARSE".into(),
            error_info: "while parsing proc body".into(),
        });
        assert_eq!(
            events,
            vec![
                DebugEvent::TargetLoadFailed {
                    path: PathBuf::from("/tmp/broken.tcl"),
                    status: "error".into(),
                    message: "missing close-brace".into(),
                    error_code: "TCL PARSE".into(),
                    error_info: "while parsing proc body".into(),
                },
                DebugEvent::StateChanged(DebugLifecycleState::Failed),
            ]
        );
    }

    #[test]
    fn wire_revalidation_separates_durable_identity_runtime_generation_and_provenance() {
        let events = wire_event_to_backend(tcld_debug_wire::WireEvent::BlocksRevalidated {
            block: "proc:child::work".into(),
            generation: 7,
            source_path: Some("/tmp/child.tcl".into()),
            change: tcld_debug_wire::SourceUnitChange::Revalidated,
        });
        let [DebugEvent::SourceRevalidated(source)] = events.as_slice() else {
            panic!("expected explicit neutral revalidation");
        };
        assert_eq!(
            source.identity,
            DebugSourceUnitIdentity::Procedure("child::work".into())
        );
        assert_eq!(source.block_id, "proc:child::work");
        assert_eq!(source.generation, 7);
        assert_eq!(
            source.definition_path,
            Some(PathBuf::from("/tmp/child.tcl"))
        );
        assert!(source.restart_limitation.is_none());
    }

    #[test]
    fn anonymous_registration_is_session_only_and_never_claims_a_file_identity() {
        let events = wire_event_to_backend(tcld_debug_wire::WireEvent::BlocksRevalidated {
            block: "anon:0123456789abcdef".into(),
            generation: 0,
            source_path: Some("/tmp/entry.tcl".into()),
            change: tcld_debug_wire::SourceUnitChange::Registered,
        });
        let [DebugEvent::SourceRegistered(source)] = events.as_slice() else {
            panic!("expected explicit neutral registration");
        };
        assert_eq!(
            source.identity,
            DebugSourceUnitIdentity::Anonymous {
                session_key: "anon:0123456789abcdef".into()
            }
        );
        assert!(!source.identity.is_restart_stable());
        assert_eq!(source.path, None);
        assert_eq!(
            source.definition_path,
            Some(PathBuf::from("/tmp/entry.tcl"))
        );
        assert!(
            source
                .restart_limitation
                .as_deref()
                .is_some_and(|reason| reason.contains("session-only"))
        );
    }

    #[test]
    fn wire_breakpoint_rows_preserve_source_identity_span_and_condition() {
        let row = tcld_tui::debug::ProcBreakpoint {
            id: "bp3".into(),
            qname: "::demo::work".into(),
            source_path: Some("/tmp/a script.tcl".into()),
            line: None,
            pending: false,
            pending_reason: None,
            offset: 31,
            len: 11,
            resolved_block: Some("proc:::demo::work".into()),
            resolved_line: Some(17),
            resolved_offset: Some(31),
            resolved_len: Some(11),
            resolution_generation: Some(4),
            enabled: false,
            hits: 4,
            ignore_count: 6,
            condition: Some("$x > 2".into()),
            kind: tcld_tui::debug::BreakpointKindLabel::Line,
        };
        let breakpoint = wire_breakpoint_to_backend(row);
        assert_eq!(breakpoint.id, DebugBreakpointId("bp3".into()));
        assert!(!breakpoint.enabled);
        assert_eq!(breakpoint.hit_count, 4);
        assert_eq!(breakpoint.ignore_count, 6);
        assert!(matches!(
            &breakpoint.resolution,
            DebugBreakpointResolution::Resolved {
                generation: Some(4),
                locations,
            } if locations.first().and_then(|location| location.source.as_ref())
                .is_some_and(|source| source.line == 17 && source.offset_start == 31)
        ));
        let DebugBreakpointSpec::SourceCommand {
            block_id,
            source_path,
            qualified_proc,
            span,
            condition,
        } = breakpoint.spec
        else {
            panic!("resolved wire row must remain a source command breakpoint");
        };
        assert_eq!(block_id, "proc:::demo::work");
        assert_eq!(source_path, Some(PathBuf::from("/tmp/a script.tcl")));
        assert_eq!(qualified_proc.as_deref(), Some("::demo::work"));
        assert_eq!(span.offset_start, 31);
        assert_eq!(span.offset_end, 42);
        assert_eq!(condition.as_deref(), Some("$x > 2"));
    }

    #[test]
    fn resolved_file_line_keeps_requested_line_separate_from_runtime_offset() {
        let breakpoint = wire_breakpoint_to_backend(tcld_tui::debug::ProcBreakpoint {
            id: "pending7".into(),
            qname: String::new(),
            source_path: Some("/tmp/child.tcl".into()),
            line: Some(2),
            pending: false,
            pending_reason: None,
            offset: 0,
            len: 1,
            resolved_block: Some("file:/tmp/child.tcl".into()),
            resolved_line: Some(2),
            resolved_offset: Some(42),
            resolved_len: Some(7),
            resolution_generation: Some(9),
            enabled: true,
            hits: 3,
            ignore_count: 0,
            condition: None,
            kind: tcld_tui::debug::BreakpointKindLabel::Line,
        });
        assert!(matches!(
            breakpoint.spec,
            DebugBreakpointSpec::SourceLine { ref path, line: 2, .. }
                if path == std::path::Path::new("/tmp/child.tcl")
        ));
        assert!(matches!(
            breakpoint.resolution,
            DebugBreakpointResolution::Resolved {
                generation: Some(9),
                ref locations,
            } if locations.first().and_then(|location| location.source.as_ref())
                .is_some_and(|source| source.line == 2
                    && source.offset_start == 42
                    && source.offset_end == 49)
        ));
    }

    #[test]
    fn host_capabilities_explain_instruction_limits_without_demoting_source_debug() {
        let capabilities = BackendCapabilities::wire_host();
        assert!(
            capabilities
                .support(BackendOperation::StepInto)
                .is_supported()
        );
        assert!(
            capabilities
                .support(BackendOperation::SourceBreakpoints)
                .is_supported()
        );
        let instruction = capabilities.support(BackendOperation::StepInstruction);
        assert!(!instruction.is_supported());
        assert!(
            instruction
                .reason()
                .is_some_and(|reason| reason.contains("code-object-qualified"))
        );
    }

    #[cfg(feature = "hybrid")]
    #[test]
    fn hybrid_capabilities_do_not_claim_source_command_step() {
        let capabilities = BackendCapabilities::hybrid(tcld_vm::RuntimeMode::Hybrid);
        assert!(
            !capabilities
                .support(BackendOperation::StepInto)
                .is_supported()
        );
        assert!(
            capabilities
                .support(BackendOperation::StepInstruction)
                .is_supported()
        );
        assert!(
            capabilities
                .support(BackendOperation::InstructionBreakpoints)
                .is_supported()
        );
        assert!(
            !capabilities
                .support(BackendOperation::SourceBreakpoints)
                .is_supported()
        );
        assert!(
            capabilities
                .support(BackendOperation::CompanionSourceAccess)
                .is_supported()
        );
    }

    #[cfg(feature = "hybrid")]
    #[test]
    fn compiled_source_backend_registers_canonical_maps_steps_and_restores_breakpoints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compiled.tcl");
        std::fs::write(
            &path,
            "proc inc {x} {\n  return [expr {$x + 1}]\n}\nset n 1\nset n [inc $n]\nset done $n\n",
        )
        .unwrap();
        let path = path.canonicalize().unwrap();
        let image = std::rc::Rc::new(
            tcld_compile::compile_path(&path, tcld_compile::CompileMode::VmStrict).unwrap(),
        );
        let mut backend =
            HybridInProcessBackend::new_with_image(tcld_vm::RuntimeMode::VmStrict, image.clone())
                .unwrap();
        backend.start_target(&path, true).unwrap();
        let events = backend.drain_events();
        let pause = events
            .iter()
            .find_map(|event| match event {
                DebugEvent::Paused(pause) => Some(pause),
                _ => None,
            })
            .expect("entry pause");
        let source = pause
            .location
            .source
            .as_ref()
            .expect("canonical source map");
        assert_eq!(
            source.source_path.as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
        assert!(pause.location.source_hint.is_none());
        assert!(
            events
                .iter()
                .any(|event| matches!(event, DebugEvent::SourceRegistered(_)))
        );
        assert!(backend.companion_sources().unwrap().is_empty());
        assert_eq!(
            backend.sources().unwrap()[0].text.as_str(),
            image.source.content.as_ref()
        );
        assert!(
            backend.procedures().unwrap().is_empty(),
            "the line-1 declaration has not executed at entry"
        );

        let declaration_breakpoint = backend
            .add_breakpoint(DebugBreakpointSpec::SourceLine {
                path: path.clone(),
                line: 1,
                condition: None,
            })
            .unwrap();
        assert!(matches!(
            declaration_breakpoint.resolution,
            DebugBreakpointResolution::Resolved { .. }
        ));

        backend.step(DebugStepKind::Into).unwrap();
        let stepped = backend
            .drain_events()
            .into_iter()
            .find_map(|event| match event {
                DebugEvent::Paused(pause) => Some(pause),
                _ => None,
            })
            .expect("source step pause");
        assert!(stepped.location.source.is_some());
        assert_eq!(backend.procedures().unwrap()[0].qualified_name, "::inc");

        let breakpoint = backend
            .add_breakpoint(DebugBreakpointSpec::SourceLine {
                path: path.clone(),
                line: 6,
                condition: None,
            })
            .unwrap();
        assert!(matches!(
            breakpoint.resolution,
            DebugBreakpointResolution::Resolved { .. }
        ));
        backend.restart_target(true).unwrap();
        let restored = backend.breakpoints().unwrap();
        assert!(restored.iter().any(|breakpoint| matches!(
            breakpoint.spec,
            DebugBreakpointSpec::SourceLine { line: 6, .. }
        ) && matches!(
            breakpoint.resolution,
            DebugBreakpointResolution::Resolved { .. }
        )));
    }

    #[cfg(feature = "hybrid")]
    #[test]
    fn compiled_source_error_after_nested_call_maps_back_to_the_outer_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested-error.tcl");
        let source = "error [puts hello]\n";
        std::fs::write(&path, source).unwrap();
        let path = path.canonicalize().unwrap();
        let image = std::rc::Rc::new(
            tcld_compile::compile_path(&path, tcld_compile::CompileMode::Hybrid).unwrap(),
        );
        let outer = image
            .top_level()
            .commands
            .iter()
            .find(|map| map.nesting_depth == 0)
            .unwrap()
            .clone();
        let nested = image
            .top_level()
            .commands
            .iter()
            .find(|map| map.nesting_depth == 1)
            .unwrap();
        assert!(outer.pc_range.start < nested.pc_range.start);
        assert!(outer.pc_range.end > nested.pc_range.end);

        let mut backend =
            HybridInProcessBackend::new_with_image(tcld_vm::RuntimeMode::Hybrid, image).unwrap();
        backend.start_target(&path, true).unwrap();
        backend.drain_events();
        backend.continue_execution().unwrap();
        let events = backend.drain_events();
        assert!(events.iter().any(|event| matches!(
            event,
            DebugEvent::Output { text, .. } if text.contains("hello")
        )));
        let error_pause = events
            .iter()
            .find_map(|event| match event {
                DebugEvent::Paused(pause) if pause.reason == DebugPauseReason::Error => Some(pause),
                _ => None,
            })
            .expect("mapped error pause");
        let mapped = error_pause.location.source.as_ref().expect("source map");
        assert_eq!(mapped.offset_start, outer.source_span.start as u64);
        assert_eq!(mapped.offset_end, outer.source_span.end as u64);
        assert_eq!(mapped.line, 1);
    }

    #[cfg(feature = "hybrid")]
    #[test]
    fn compiled_instruction_steps_keep_deepest_source_continuity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested-step.tcl");
        std::fs::write(&path, "set n [expr {1 + 1}]\n").unwrap();
        let path = path.canonicalize().unwrap();
        let image = std::rc::Rc::new(
            tcld_compile::compile_path(&path, tcld_compile::CompileMode::VmStrict).unwrap(),
        );
        let outer = image
            .top_level()
            .commands
            .iter()
            .find(|map| map.nesting_depth == 0)
            .unwrap()
            .source_span
            .clone();
        let nested = image
            .top_level()
            .commands
            .iter()
            .find(|map| map.nesting_depth == 1)
            .unwrap()
            .source_span
            .clone();
        let mut backend =
            HybridInProcessBackend::new_with_image(tcld_vm::RuntimeMode::VmStrict, image).unwrap();
        backend.start_target(&path, true).unwrap();
        let mut spans = Vec::new();
        for event in backend.drain_events() {
            if let DebugEvent::Paused(pause) = event
                && let Some(source) = pause.location.source
            {
                spans.push(source.offset_start as usize..source.offset_end as usize);
            }
        }
        for _ in 0..32 {
            backend.step(DebugStepKind::InstructionInto).unwrap();
            let events = backend.drain_events();
            for event in events {
                if let DebugEvent::Paused(pause) = event
                    && let Some(source) = pause.location.source
                {
                    spans.push(source.offset_start as usize..source.offset_end as usize);
                }
            }
            if backend.finished {
                break;
            }
        }
        assert!(spans.iter().all(|span| span == &outer || span == &nested));
        let nested_start = spans.iter().position(|span| span == &nested).unwrap();
        assert!(spans[..nested_start].iter().all(|span| span == &outer));
        assert!(
            spans[nested_start + 1..].iter().any(|span| span == &outer),
            "source ownership must return to the outer command after nested execution: {spans:?}"
        );
    }
}
