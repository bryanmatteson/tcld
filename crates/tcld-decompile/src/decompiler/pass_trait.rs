use crate::bytecode::ByteCode;
use crate::code_graph::CodeGraph;
use crate::error::Result;
use std::ops::Range;

/// Trait for implementing custom decompilation passes.
///
/// A `DecompilerPass` transforms the intermediate representation (CodeGraph)
/// during one stage of the decompilation pipeline. Built-in passes handle
/// pattern matching for control flow constructs (loops, conditionals, etc.),
/// while custom passes can add support for application-specific patterns.
///
/// # Lifecycle
///
/// Passes are executed in order by the pipeline. Each pass receives a mutable
/// reference to the CodeGraph and a byte-offset range to operate on. Passes
/// that need to run until no more changes are made should return `true` from
/// `is_fixpoint()`.
///
/// # Example
///
/// ```ignore
/// struct MyCustomPass;
///
/// impl DecompilerPass for MyCustomPass {
///     fn name(&self) -> &str { "my_custom_pass" }
///
///     fn run(&self, graph: &mut CodeGraph, bytecode: &ByteCode, range: Range<i32>) -> Result<bool> {
///         // Transform the graph and return true if changes were made
///         Ok(false)
///     }
/// }
/// ```
pub trait DecompilerPass: Send + Sync {
    /// A short human-readable name for this pass (e.g. "catches", "loops").
    fn name(&self) -> &str;

    /// Run the pass over the given range of the code graph.
    ///
    /// Returns `true` if any transformation was applied (used by fixpoint passes).
    fn run(&self, graph: &mut CodeGraph, bytecode: &ByteCode, range: Range<i32>) -> Result<bool>;

    /// Whether this pass should be re-run until it returns `false`.
    /// Default is `false` (single-shot pass).
    fn is_fixpoint(&self) -> bool {
        false
    }

    /// Priority for ordering. Lower values run first. Default is 1000.
    /// Built-in passes typically use values like 100-900.
    fn priority(&self) -> u32 {
        1000
    }
}
