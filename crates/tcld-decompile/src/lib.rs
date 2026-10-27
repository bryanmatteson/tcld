pub mod bytecode {
    pub use tcld_core::bytecode::*;
}

pub mod error {
    pub use tcld_core::error::*;
}

pub mod inst_table {
    pub use tcld_core::inst_table::*;
}

pub mod analysis {
    pub use tcld_core::analysis::*;
}

pub mod source {
    pub use tcld_core::source::*;
}

pub mod util {
    pub use tcld_core::util::*;
}

pub mod code {
    pub use tcld_ir::code::*;
}

pub mod code_graph {
    pub use tcld_ir::code_graph::CodeGraph;
}

pub mod coverage;
pub mod decompiler;
pub mod diagnostics;
pub mod features;
pub mod fixture_harness;
mod init;
pub mod loader;
pub mod snapshot;
pub mod wasm;

pub use coverage::{
    CoverageClassification, CoverageEvidence, CoverageMatrix, CoverageMatrixConfig, CoverageRow,
    TclVersionCategory, generate_coverage_matrix, generate_coverage_matrix_by_version,
};
pub use diagnostics::{
    Approximation, ApproximationKind, DecompileWithDiagnostics, DiagnosticsReport, PassDecision,
    ReconstructionStatus, StrictDecompileError,
};
pub use features::canonicalize::canonicalize_tcl;
pub use features::decompile::{
    DecompileOptions, decompile_input_approximate_with_diagnostics,
    decompile_input_strict_with_diagnostics, decompile_loaded, decompile_path,
    decompile_path_with_options,
};
pub use features::disassemble::{
    DisassembleOptions, disassemble_bytecode, disassemble_loaded, disassemble_path,
    disassemble_path_with_options,
};
pub use features::dump::{
    DumpOptions, dump_bytecode, dump_loaded, dump_path, dump_path_with_options,
};
pub use fixture_harness::{
    FixtureHarnessConfig, FixtureHarnessReport, FixtureResult, RegenerationStatus,
    run_golden_fixture_harness,
};
pub use loader::{LoadInput, LoadKind, LoadProvenance, LoadedBytecode, load_bytecode};
pub use snapshot::{AnalysisSnapshot, build_instruction_group_map};

// Plugin architecture exports
pub use decompiler::pass_registry::{PassRegistry, global_registry, register_pass};
pub use decompiler::pass_trait::DecompilerPass;
pub use decompiler::{Decompiler, InlineTraceCapture};

// Source mapping exports for interactive diffing
pub use decompiler::source_map::{CommandPath, OriginalSourceStatus, SourceMap, SourceMapEntry};
