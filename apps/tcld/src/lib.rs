pub mod batch_color;
pub mod cli;
pub mod debug;
pub mod explore;
pub mod highlight;
pub mod run;

pub mod analysis {
    pub use tcld_core::analysis::*;
}

pub mod bytecode {
    pub use tcld_core::bytecode::*;
    pub use tcld_syntax::extract_tbc_payload_from_file;
}

pub mod error {
    pub use tcld_core::error::*;
}

pub mod inst_table {
    pub use tcld_core::inst_table::*;
}

pub mod scanner {
    pub use tcld_core::scanner::*;
}

pub mod source {
    pub use tcld_core::source::*;
}

pub mod util {
    pub use tcld_core::util::*;
}

pub use tcld_decompile::canonicalize_tcl;
pub use tcld_decompile::decompiler;
pub use tcld_decompile::{
    DecompileOptions, DisassembleOptions, DumpOptions, decompile_path, decompile_path_with_options,
    disassemble_path, disassemble_path_with_options, dump_path, dump_path_with_options,
    generate_coverage_matrix_by_version,
};
pub use tcld_syntax::{
    FoldingRange, FoldingRangeKind, FormatEdit, FormatOptions, SyntaxToken, folding_ranges,
    format_tcl, format_tcl_with_options, highlight_tcl, parse_tcl, plan_format_edits,
};
