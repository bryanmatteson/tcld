//! Stable error surface (executable plan §7).

use std::path::PathBuf;

use thiserror::Error;

/// Exhaustive public error variants for the hybrid system.
#[derive(Debug, Error)]
pub enum VmError {
    #[error("vm: unsupported opcode {opcode} at pc {pc}")]
    UnimplementedOpcode { opcode: String, pc: u64 },

    #[error("vm: varName form unsupported: {cmd} ({form})")]
    VarNameUnsupported { cmd: String, form: String },

    #[error("vm: stack underflow at pc {pc} ({op})")]
    StackUnderflow { pc: u64, op: String },

    #[error("vm: invalid catch state")]
    InvalidCatchState,

    #[error("vm: invalid auxiliary data for {opcode}: {detail}")]
    InvalidAuxData { opcode: String, detail: String },

    #[error("vm: invalid operand for {opcode}: {detail}")]
    InvalidOpcodeOperand { opcode: String, detail: String },

    #[error("vm: invalid jump target {pc} in code {code_id}")]
    InvalidJumpTarget { code_id: u64, pc: u32 },

    #[error("vm: invalid expansion state")]
    InvalidExpansionState,

    #[error("vm: cross-runtime upvar")]
    CrossRuntimeUpvar,

    #[error("vm: return level crosses an opaque host frame")]
    CrossRuntimeReturnLevel,

    #[error("vm: uplevel crosses an opaque host frame")]
    CrossRuntimeUplevel,

    #[error("vm: re-entrancy limit exceeded")]
    ReentrancyLimit,

    #[error("vm: cannot marshal ProcBody to host")]
    BridgeMarshalProcBody,

    #[error("vm: host frame unavailable: {0}")]
    HostFrameUnavailable(String),

    #[error("vm: cannot mirror value into host frame: {0}")]
    HostFrameMarshalUnsupported(String),

    #[error("vm: host frame suspension is unsupported")]
    HostFrameSuspensionUnsupported,

    #[error("vm: registered command not found: {0}")]
    RegisteredCommandNotFound(String),

    #[error("vm: corpus contract violation: {0}")]
    CorpusContractViolation(String),

    #[error("vm: call depth exceeded")]
    CallDepth,

    #[error("vm: policy denied {action}: {detail}")]
    PolicyDenied {
        action: &'static str,
        detail: String,
    },

    #[error("vm: break outside loop")]
    BreakOutsideLoop,

    #[error("vm: continue outside loop")]
    ContinueOutsideLoop,

    #[error("vm: bcproc body extract failed: {0}")]
    ExtractBadBody(String),

    #[error("vm: bcproc body is not bytecode")]
    BcprocBodyNotBytecode,

    #[error("vm: hybrid backend unavailable: {0}")]
    HybridUnavailable(String),

    #[error("vm: I/O {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("vm: decode: {0}")]
    Decode(String),

    #[error("vm: internal: {0}")]
    Internal(String),

    /// Uncaught Tcl/script error after unwind (maps to process exit 1).
    #[error("{message}")]
    TclError {
        message: String,
        error_info: Option<String>,
        error_code: Option<String>,
    },
}

impl VmError {
    /// Process exit code for `tcld run` (executable plan §2.1 / §7).
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::TclError { .. } => 1,
            Self::Io { .. } | Self::HybridUnavailable(_) => 3,
            Self::Decode(_)
            | Self::UnimplementedOpcode { .. }
            | Self::VarNameUnsupported { .. }
            | Self::StackUnderflow { .. }
            | Self::InvalidCatchState
            | Self::InvalidAuxData { .. }
            | Self::InvalidOpcodeOperand { .. }
            | Self::InvalidJumpTarget { .. }
            | Self::InvalidExpansionState
            | Self::CrossRuntimeUpvar
            | Self::CrossRuntimeReturnLevel
            | Self::CrossRuntimeUplevel
            | Self::ReentrancyLimit
            | Self::BridgeMarshalProcBody
            | Self::HostFrameUnavailable(_)
            | Self::HostFrameMarshalUnsupported(_)
            | Self::HostFrameSuspensionUnsupported
            | Self::RegisteredCommandNotFound(_)
            | Self::CorpusContractViolation(_)
            | Self::CallDepth
            | Self::PolicyDenied { .. }
            | Self::BreakOutsideLoop
            | Self::ContinueOutsideLoop
            | Self::ExtractBadBody(_)
            | Self::BcprocBodyNotBytecode
            | Self::Internal(_) => 2,
        }
    }
}
