//! Process-wide host capability policy.

use crate::mode::RuntimeMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecPolicy {
    pub allow_host_invoke: bool,
    pub allow_host_load: bool,
    pub allow_host_exec: bool,
    pub allow_host_opcode: bool,
    pub allow_dynamic_eval: bool,
}

impl ExecPolicy {
    pub fn for_mode(mode: RuntimeMode) -> Self {
        match mode {
            RuntimeMode::Hybrid => Self {
                allow_host_invoke: true,
                allow_host_load: true,
                allow_host_exec: true,
                allow_host_opcode: true,
                allow_dynamic_eval: true,
            },
            RuntimeMode::VmStrict => Self {
                allow_host_invoke: false,
                allow_host_load: false,
                allow_host_exec: false,
                allow_host_opcode: false,
                allow_dynamic_eval: false,
            },
        }
    }
}
