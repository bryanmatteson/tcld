//! Example wrappers showing how built-in passes could implement DecompilerPass.
//! These are NOT yet integrated into the pipeline — they serve as documentation
//! and a template for custom pass implementors.

use super::pass_trait::DecompilerPass;
use crate::bytecode::ByteCode;
use crate::code_graph::CodeGraph;
use crate::error::Result;
use std::ops::Range;

/// Example: a pass that could handle catch block recognition.
///
/// In a full implementation, this would call the logic from passes::catches::run()
/// to detect and transform catch exception handling patterns into readable code.
pub struct CatchesPass;

impl DecompilerPass for CatchesPass {
    fn name(&self) -> &str {
        "catches"
    }

    fn run(
        &self,
        _graph: &mut CodeGraph,
        _bytecode: &ByteCode,
        _range: Range<i32>,
    ) -> Result<bool> {
        // In a full implementation, this would call the logic from passes::catches::run()
        // For now, this serves as a template for custom pass implementors.
        Ok(false)
    }

    fn priority(&self) -> u32 {
        400
    }
}

/// Example: a fixpoint pass for loop recognition.
///
/// Loops are recognized through pattern matching on control flow graphs.
/// This pass needs to run repeatedly (fixpoint) because transforming one
/// loop pattern may create new loop patterns that need recognition.
pub struct LoopsPass;

impl DecompilerPass for LoopsPass {
    fn name(&self) -> &str {
        "loops"
    }

    fn run(
        &self,
        _graph: &mut CodeGraph,
        _bytecode: &ByteCode,
        _range: Range<i32>,
    ) -> Result<bool> {
        // In a full implementation, this would call the logic from passes::loops_fixpoint::run()
        Ok(false)
    }

    fn is_fixpoint(&self) -> bool {
        true
    }

    fn priority(&self) -> u32 {
        800
    }
}

/// Example: a single-shot pass for switch table optimization.
///
/// This pass recognizes jump table instructions and converts them into
/// readable switch statements.
pub struct SwitchTablesPass;

impl DecompilerPass for SwitchTablesPass {
    fn name(&self) -> &str {
        "switch_tables"
    }

    fn run(
        &self,
        _graph: &mut CodeGraph,
        _bytecode: &ByteCode,
        _range: Range<i32>,
    ) -> Result<bool> {
        // In a full implementation, this would call the logic from passes::switch_tables::run()
        Ok(false)
    }

    fn priority(&self) -> u32 {
        200
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{LoadInput, load_bytecode};

    fn fixture() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/tcld/tests/fixtures/expr.tbc")
    }

    #[test]
    fn documented_builtin_pass_adapters_expose_their_contracts() {
        let loaded = load_bytecode(LoadInput::path(fixture())).expect("fixture");
        let mut graph = CodeGraph::new(loaded.bytecode.get_instructions().expect("instructions"));

        let catches = CatchesPass;
        assert_eq!(catches.name(), "catches");
        assert_eq!(catches.priority(), 400);
        assert!(!catches.is_fixpoint());
        assert!(!catches.run(&mut graph, &loaded.bytecode, 0..1).unwrap());

        let loops = LoopsPass;
        assert_eq!(loops.name(), "loops");
        assert_eq!(loops.priority(), 800);
        assert!(loops.is_fixpoint());
        assert!(!loops.run(&mut graph, &loaded.bytecode, 0..1).unwrap());

        let switches = SwitchTablesPass;
        assert_eq!(switches.name(), "switch_tables");
        assert_eq!(switches.priority(), 200);
        assert!(!switches.is_fixpoint());
        assert!(!switches.run(&mut graph, &loaded.bytecode, 0..1).unwrap());
    }
}
