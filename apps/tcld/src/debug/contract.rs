//! Backend-neutral debugger contract used by CLI debugger frontends.
//!
//! Source locations deliberately reuse `tcld-debug-wire::SourceLocation`.
//! That wire type is the lossless serialization of the canonical
//! `tcld_debug_state::Location` plus its block/source provenance.  Hybrid
//! execution additionally needs a code-object-qualified PC, which the source
//! runtime model cannot represent.  `DebugLocation` is therefore a union: it
//! may carry the canonical source location, a code address, or both when a
//! verified source map relates them.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tcld_debug_wire::SourceLocation;

/// Stable identity for one loaded bytecode object within a debug session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DebugCodeId(pub u64);

/// A bytecode instruction address. PCs are qualified by code object so nested
/// procedures cannot alias one another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DebugCodeLocation {
    pub code_id: DebugCodeId,
    pub pc: u64,
}

/// One execution location. Source and code coordinates are independent and
/// optional; when both are present the backend has verified their mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugLocation {
    pub source: Option<SourceLocation>,
    pub code: Option<DebugCodeLocation>,
    /// A runtime-reported PC that cannot be qualified to a stable code
    /// object. The stock wire nub can report this diagnostic coordinate, but
    /// it must never be used as an instruction-breakpoint identity.
    pub observed_pc: Option<u64>,
    /// A useful but non-canonical companion-source projection. This is kept
    /// separate from `source` so stale or unrelated sibling files can never
    /// become debugger truth.
    pub source_hint: Option<CompanionSourceLocation>,
}

impl DebugLocation {
    pub fn source(location: SourceLocation) -> Self {
        Self {
            source: Some(location),
            code: None,
            observed_pc: None,
            source_hint: None,
        }
    }

    pub fn code(location: DebugCodeLocation) -> Self {
        Self {
            source: None,
            code: Some(location),
            observed_pc: None,
            source_hint: None,
        }
    }

    pub fn mapped(source: SourceLocation, code: DebugCodeLocation) -> Self {
        Self {
            source: Some(source),
            code: Some(code),
            observed_pc: None,
            source_hint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionSourceLocation {
    pub path: PathBuf,
    pub line: u32,
    pub qualified_proc: Option<String>,
    pub span: SourceCommandSpan,
    pub reason: String,
}

/// Command byte range within a canonical source block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceCommandSpan {
    pub offset_start: u64,
    pub offset_end: u64,
}

impl SourceCommandSpan {
    pub fn new(offset_start: u64, offset_end: u64) -> Result<Self, String> {
        if offset_end <= offset_start {
            return Err("source command span must be non-empty".into());
        }
        Ok(Self {
            offset_start,
            offset_end,
        })
    }

    pub fn byte_len(self) -> u64 {
        self.offset_end - self.offset_start
    }
}

/// Backend-neutral breakpoint target. Source variants retain the same block
/// identity/offset semantics as `tcld-debug-state`; instruction variants add
/// the code identity required by the hybrid VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugBreakpointSpec {
    SourceLine {
        path: PathBuf,
        line: u32,
        condition: Option<String>,
    },
    SourceCommand {
        block_id: String,
        source_path: Option<PathBuf>,
        qualified_proc: Option<String>,
        span: SourceCommandSpan,
        condition: Option<String>,
    },
    Instruction {
        location: DebugCodeLocation,
        condition: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DebugBreakpointId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugBreakpointResolution {
    Pending {
        reason: String,
    },
    Resolved {
        /// Runtime owner generation against which the requested logical
        /// source spec was resolved. Hybrid/code breakpoints may omit it.
        generation: Option<u64>,
        locations: Vec<DebugLocation>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugBreakpoint {
    pub id: DebugBreakpointId,
    pub spec: DebugBreakpointSpec,
    pub enabled: bool,
    pub resolution: DebugBreakpointResolution,
    pub hit_count: u64,
    pub ignore_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugLifecycleState {
    Disconnected,
    Starting,
    Ready,
    Loading,
    Running,
    Paused,
    Completed,
    Failed,
}

impl From<tcld_tui::debug::AppState> for DebugLifecycleState {
    fn from(value: tcld_tui::debug::AppState) -> Self {
        use tcld_tui::debug::AppState;
        match value {
            AppState::Dead => Self::Disconnected,
            AppState::Starting => Self::Starting,
            AppState::Ready => Self::Ready,
            AppState::Loading => Self::Loading,
            AppState::Running => Self::Running,
            AppState::Stopped => Self::Paused,
            AppState::Completed => Self::Completed,
            AppState::LoadFailed => Self::Failed,
        }
    }
}

impl From<DebugLifecycleState> for tcld_tui::debug::AppState {
    fn from(value: DebugLifecycleState) -> Self {
        use tcld_tui::debug::AppState;
        match value {
            DebugLifecycleState::Disconnected => AppState::Dead,
            DebugLifecycleState::Starting => AppState::Starting,
            DebugLifecycleState::Ready => AppState::Ready,
            DebugLifecycleState::Loading => AppState::Loading,
            DebugLifecycleState::Running => AppState::Running,
            DebugLifecycleState::Paused => AppState::Stopped,
            DebugLifecycleState::Completed => AppState::Completed,
            DebugLifecycleState::Failed => AppState::LoadFailed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugPauseReason {
    Entry,
    Step,
    StepOver,
    StepOut,
    Breakpoint,
    Watchpoint,
    Pause,
    InstructionStep,
    InstructionBreakpoint,
    RunToLocation,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendPause {
    pub location: DebugLocation,
    pub in_host: bool,
    pub reason: DebugPauseReason,
    pub proc_qname: Option<String>,
    pub frame_count: u32,
    pub breakpoint: Option<DebugBreakpointId>,
    pub locals: Vec<(String, String)>,
    pub error: Option<String>,
}

impl BackendPause {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugOutputStream {
    Stdout,
    Stderr,
    Console,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugSourceUnitIdentity {
    /// Canonical absolute path reported by the Tcl owner. Relative spelling
    /// is normalized in the nub before this identity crosses the wire.
    File(PathBuf),
    /// Qualified Tcl command name. Definition provenance is carried
    /// separately and is never folded into the executable identity.
    Procedure(String),
    /// Deterministic within one interpreter session (currently the owner's
    /// content-derived anonymous block token). It is intentionally not a
    /// restart key because eval provenance can change between executions.
    Anonymous { session_key: String },
}

impl DebugSourceUnitIdentity {
    pub fn display_label(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Procedure(qname) => format!("proc {qname}"),
            Self::Anonymous { session_key } => format!("eval {session_key}"),
        }
    }

    pub fn is_restart_stable(&self) -> bool {
        !matches!(self, Self::Anonymous { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSource {
    pub identity: DebugSourceUnitIdentity,
    /// Backend instance key used to qualify this concrete runtime block.
    /// This must not be persisted in place of `identity`.
    pub block_id: String,
    pub generation: u64,
    pub path: Option<PathBuf>,
    pub qualified_proc: Option<String>,
    /// Owner-derived file that defined a proc/eval unit. This is display
    /// provenance only, never executable identity.
    pub definition_path: Option<PathBuf>,
    /// Structured explanation shown when this unit cannot be safely rebound
    /// after a fresh interpreter restart.
    pub restart_limitation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredCode {
    pub id: DebugCodeId,
    pub name: Option<String>,
    pub source_block_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionSource {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugEvent {
    StateChanged(DebugLifecycleState),
    Paused(Box<BackendPause>),
    Output {
        stream: DebugOutputStream,
        text: String,
    },
    Diagnostic {
        severity: DebugDiagnosticSeverity,
        message: String,
    },
    TargetLoadFailed {
        path: PathBuf,
        status: String,
        message: String,
        error_code: String,
        error_info: String,
    },
    SourceRegistered(RegisteredSource),
    SourceRevalidated(RegisteredSource),
    SourceUnavailable {
        identity: DebugSourceUnitIdentity,
        block_id: String,
        reason: String,
    },
    CompanionSourceDiscovered(CompanionSource),
    CodeRegistered(RegisteredCode),
    Completed {
        result: Option<String>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugStepKind {
    Into,
    Over,
    Out,
    InstructionInto,
    InstructionOver,
    InstructionOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BackendOperation {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySupport {
    Supported,
    Unsupported { reason: String },
}

impl CapabilitySupport {
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

/// Structured capability map. A frontend can both gate an action and explain
/// exactly why the selected backend cannot perform it.
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    support: BTreeMap<BackendOperation, CapabilitySupport>,
}

impl BackendCapabilities {
    pub(crate) fn new(support: BTreeMap<BackendOperation, CapabilitySupport>) -> Self {
        Self { support }
    }

    pub fn support(&self, operation: BackendOperation) -> &CapabilitySupport {
        self.support
            .get(&operation)
            .expect("every backend operation must have an explicit capability entry")
    }

    #[cfg(feature = "hybrid")]
    pub(crate) fn set(&mut self, operation: BackendOperation, support: CapabilitySupport) {
        self.support.insert(operation, support);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugStackFrame {
    pub level: i64,
    pub kind: String,
    pub proc_name: Option<String>,
    pub location: DebugLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugProcedure {
    pub qualified_name: String,
    pub block_id: Option<String>,
    pub first_offset: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugEvaluation {
    pub completion_code: i32,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSnapshot {
    pub source: RegisteredSource,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionSourceSnapshot {
    pub source: CompanionSource,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisassemblySnapshot {
    pub code: RegisteredCode,
    pub instruction_set: Option<String>,
    pub lines: Vec<String>,
}
