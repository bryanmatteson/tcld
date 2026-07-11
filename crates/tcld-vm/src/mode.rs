//! Runtime modes (executable plan §2.1).

/// TEBC execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// VM TEBC + embedded Tcl WorldHost.
    Hybrid,
    /// VM TEBC only; WorldHost denied (also used for `--sandbox`).
    VmStrict,
}

impl RuntimeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::VmStrict => "vm-strict",
        }
    }
}
