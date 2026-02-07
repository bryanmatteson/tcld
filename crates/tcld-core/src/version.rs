//! Bytecode version detection and compatibility layer.
//!
//! Tcl bytecode format has evolved across Tcl versions. This module provides
//! automatic version detection and version-specific parsing strategies.

use std::ops::RangeInclusive;

/// Known Tcl bytecode format versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BytecodeVersion {
    /// Tcl 8.0 — initial bytecode format.
    Tcl80,
    /// Tcl 8.4 — extended instruction set.
    Tcl84,
    /// Tcl 8.5 — added dict, lassign, etc.
    Tcl85,
    /// Tcl 8.6 — coroutines, tailcall, try/throw.
    Tcl86,
    /// Tcl 8.7 development line.
    Tcl87,
    /// Tcl 9.0.
    Tcl90,
    /// Unknown or unsupported version.
    Unknown,
}

impl BytecodeVersion {
    /// Detect version from bytecode header fields.
    ///
    /// Uses the compiler version major.minor and bytecode format number
    /// to determine which Tcl version produced the bytecode.
    pub fn detect(compiler_major: i32, compiler_minor: i32, _format: i32) -> Self {
        match (compiler_major, compiler_minor) {
            (8, 0..=3) => BytecodeVersion::Tcl80,
            (8, 4) => BytecodeVersion::Tcl84,
            (8, 5) => BytecodeVersion::Tcl85,
            (8, 6) => BytecodeVersion::Tcl86,
            (8, 7..) => BytecodeVersion::Tcl87,
            (9, 0) => BytecodeVersion::Tcl90,
            _ => BytecodeVersion::Unknown,
        }
    }

    /// Human-readable version string.
    pub fn label(self) -> &'static str {
        match self {
            BytecodeVersion::Tcl80 => "Tcl 8.0",
            BytecodeVersion::Tcl84 => "Tcl 8.4",
            BytecodeVersion::Tcl85 => "Tcl 8.5",
            BytecodeVersion::Tcl86 => "Tcl 8.6",
            BytecodeVersion::Tcl87 => "Tcl 8.7",
            BytecodeVersion::Tcl90 => "Tcl 9.0",
            BytecodeVersion::Unknown => "Unknown",
        }
    }

    /// Range of supported opcode numbers for this version.
    pub fn opcode_range(self) -> Option<RangeInclusive<u8>> {
        match self {
            BytecodeVersion::Tcl80 => Some(0..=72),
            BytecodeVersion::Tcl84 => Some(0..=97),
            BytecodeVersion::Tcl85 => Some(0..=131),
            BytecodeVersion::Tcl86 => Some(0..=189),
            BytecodeVersion::Tcl90 => Some(0..=190),
            BytecodeVersion::Tcl87 | BytecodeVersion::Unknown => None,
        }
    }

    /// Whether this version supports a specific feature.
    pub fn supports(self, feature: BytecodeFeature) -> bool {
        if self == BytecodeVersion::Unknown {
            return false;
        }
        match feature {
            BytecodeFeature::Coroutines => self >= BytecodeVersion::Tcl86,
            BytecodeFeature::DictOperations => self >= BytecodeVersion::Tcl85,
            BytecodeFeature::TailCall => self >= BytecodeVersion::Tcl86,
            BytecodeFeature::TryThrow => self >= BytecodeVersion::Tcl86,
            BytecodeFeature::Lmap => self >= BytecodeVersion::Tcl86,
            BytecodeFeature::StringClass => self >= BytecodeVersion::Tcl86,
            BytecodeFeature::ClockRead => self >= BytecodeVersion::Tcl86,
        }
    }
}

impl std::fmt::Display for BytecodeVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Features that vary across bytecode versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BytecodeFeature {
    Coroutines,
    DictOperations,
    TailCall,
    TryThrow,
    Lmap,
    StringClass,
    ClockRead,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_tcl86() {
        let v = BytecodeVersion::detect(8, 6, 2);
        assert_eq!(v, BytecodeVersion::Tcl86);
        assert_eq!(v.label(), "Tcl 8.6");
    }

    #[test]
    fn detect_tcl85() {
        let v = BytecodeVersion::detect(8, 5, 2);
        assert_eq!(v, BytecodeVersion::Tcl85);
        assert_eq!(BytecodeVersion::Tcl80.opcode_range(), Some(0..=72));
        assert_eq!(BytecodeVersion::Tcl84.opcode_range(), Some(0..=97));
        assert_eq!(v.opcode_range(), Some(0..=131));
    }

    #[test]
    fn detect_tcl90_without_conflating_it_with_tcl87() {
        let v = BytecodeVersion::detect(9, 0, 3);
        assert_eq!(v, BytecodeVersion::Tcl90);
        assert_eq!(v.label(), "Tcl 9.0");
        assert_eq!(v.opcode_range(), Some(0..=190));
        assert_eq!(BytecodeVersion::detect(9, 1, 3), BytecodeVersion::Unknown);
    }

    #[test]
    fn detect_unknown_old_version() {
        let v = BytecodeVersion::detect(7, 0, 0);
        assert_eq!(v, BytecodeVersion::Unknown);
        assert_eq!(v.opcode_range(), None);
        assert!(!v.supports(BytecodeFeature::Coroutines));
    }

    #[test]
    fn feature_support() {
        assert!(BytecodeVersion::Tcl86.supports(BytecodeFeature::Coroutines));
        assert!(!BytecodeVersion::Tcl85.supports(BytecodeFeature::Coroutines));
        assert!(BytecodeVersion::Tcl85.supports(BytecodeFeature::DictOperations));
        assert!(!BytecodeVersion::Tcl84.supports(BytecodeFeature::DictOperations));
        assert!(BytecodeVersion::Tcl86.supports(BytecodeFeature::StringClass));
        assert!(BytecodeVersion::Tcl86.supports(BytecodeFeature::ClockRead));
    }

    #[test]
    fn version_ordering() {
        assert!(BytecodeVersion::Tcl90 > BytecodeVersion::Tcl87);
        assert!(BytecodeVersion::Tcl87 > BytecodeVersion::Tcl86);
        assert!(BytecodeVersion::Tcl86 > BytecodeVersion::Tcl85);
    }
}
