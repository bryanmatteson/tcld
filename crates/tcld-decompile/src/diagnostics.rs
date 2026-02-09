use crate::error::TcldError;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconstructionStatus {
    Strict,
    Approximation,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproximationKind {
    NestedProcedure,
    UnresolvedInstruction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassDecision {
    pub name: String,
    pub enabled: bool,
    pub iteration: usize,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approximation {
    pub kind: ApproximationKind,
    pub bytecode_range: Option<Range<i32>>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsReport {
    pub recognized_constructs: Vec<String>,
    pub unreduced_opcodes: Vec<String>,
    pub approximations: Vec<Approximation>,
    pub suppressed_fast_path_ranges: Vec<Range<i32>>,
    pub pass_decisions: Vec<PassDecision>,
    pub unsupported_instructions: Vec<String>,
    pub reconstruction: ReconstructionStatus,
    pub suggested_next_work: Vec<String>,
}

impl Default for DiagnosticsReport {
    fn default() -> Self {
        Self {
            recognized_constructs: Vec::new(),
            unreduced_opcodes: Vec::new(),
            approximations: Vec::new(),
            suppressed_fast_path_ranges: Vec::new(),
            pass_decisions: Vec::new(),
            unsupported_instructions: Vec::new(),
            reconstruction: ReconstructionStatus::Strict,
            suggested_next_work: Vec::new(),
        }
    }
}

impl DiagnosticsReport {
    pub fn summary_messages(&self) -> Vec<String> {
        let mut out = vec![format!("reconstruction: {:?}", self.reconstruction)];
        if !self.unreduced_opcodes.is_empty() {
            out.push(format!(
                "unreduced opcodes: {}",
                self.unreduced_opcodes.join(", ")
            ));
        }
        if !self.approximations.is_empty() {
            out.push(format!("approximations: {}", self.approximations.len()));
        }
        out
    }
}

#[derive(Debug)]
pub struct StrictDecompileError {
    pub source: TcldError,
    pub diagnostics: DiagnosticsReport,
}

impl std::fmt::Display for StrictDecompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl std::error::Error for StrictDecompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug, Clone)]
pub struct DecompileWithDiagnostics {
    pub output: crate::decompiler::DecompiledOutput,
    pub diagnostics: DiagnosticsReport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reports_only_present_diagnostic_categories() {
        let default = DiagnosticsReport::default();
        assert_eq!(default.summary_messages(), vec!["reconstruction: Strict"]);

        let report = DiagnosticsReport {
            reconstruction: ReconstructionStatus::Approximation,
            unreduced_opcodes: vec!["mystery".to_string(), "unknown".to_string()],
            approximations: vec![Approximation {
                kind: ApproximationKind::UnresolvedInstruction,
                bytecode_range: Some(3..7),
                reason: "unreduced".to_string(),
            }],
            ..Default::default()
        };
        assert_eq!(
            report.summary_messages(),
            vec![
                "reconstruction: Approximation",
                "unreduced opcodes: mystery, unknown",
                "approximations: 1",
            ]
        );
    }

    #[test]
    fn strict_error_displays_and_exposes_its_source() {
        let error = StrictDecompileError {
            source: TcldError::Other("strict failure".to_string()),
            diagnostics: DiagnosticsReport::default(),
        };
        assert_eq!(error.to_string(), "strict failure");
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some("strict failure".to_string())
        );
    }
}
