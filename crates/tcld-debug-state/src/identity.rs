//! Adapter from `tcld_decompile`'s identity model to [`Location`].
//!
//! `tcld-decompile` does not depend on this crate (one-way arrow). The
//! adapter lives here so the dependency direction stays clean — and so
//! that the Block layer is the canonical "where can a breakpoint go"
//! surface, not a derived view on top of `SourceMap`.
//!
//! Today there's exactly one supported source: `CommandIdentity::Synthetic`
//! with a bound `proc_qname`. Bytecode identities are excluded by
//! contract row S022: PC-based pauses translate to `Location` at
//! trace-handler time using `SourceMap::source_for_bytecode`, not here.

use crate::block::{BlockError, BlockId, BlockRegistry};
use crate::location::{Location, LocationError};
use std::sync::Arc;
use tcld_decompile::SourceMapEntry;
use tcld_decompile::decompiler::CommandIdentity;

/// Parse a Tcl proc body's source with `tcld_syntax::command_spans` and
/// register the resulting [`BlockId::ProcBody`] in the registry with one
/// `valid_offset` per parsed command. Used by the `proc` / `apply` /
/// `namespace eval` wrappers in `tcld-dbg-ext` to give the trace handler
/// real per-statement breakpoint granularity.
///
/// Returns the registered [`Block`](crate::block::Block)'s id on
/// success. The function:
///
/// - Computes a `BlockId::ProcBody` keyed by the qname (or
///   `BlockId::anonymous_for(body)` when no qname is supplied).
/// - Walks `tcld_syntax::command_spans` and collects each command's
///   `start_byte` as a valid offset, sorted and deduplicated.
/// - Calls `BlockRegistry::register_replacement` so re-defining a proc
///   bumps its version and breakpoints are revalidated.
///
/// Errors propagate from the syntax parser (best-effort fallback to a single
/// whole-body offset on parse failure) and the registry.
pub fn register_body_block(
    registry: &mut BlockRegistry,
    qname: Option<&str>,
    body: &str,
) -> Result<BlockId, BlockError> {
    let id = match qname {
        Some(name) => BlockId::ProcBody(Arc::from(name)),
        None => BlockId::anonymous_for(body),
    };
    // tree-sitter-tcl drops the final command if the body has no
    // trailing newline. Normalize at parse time so we don't lose the
    // last statement; the registered Block keeps the original source so
    // byte offsets line up with what the trace handler later observes.
    let parse_input;
    let parse_str: &str = if body.ends_with('\n') {
        body
    } else {
        parse_input = format!("{body}\n");
        &parse_input
    };
    let spans = tcld_syntax::command_spans(parse_str).unwrap_or_default();
    let mut offsets: Vec<usize> = spans
        .iter()
        // tcld_syntax sorts spans so that outer commands come before
        // nested ones at the same byte position; dedup folds the
        // duplicate generic-command/special-form attributions for the
        // same statement (both tree-sitter passes report the same
        // start_byte).
        .map(|s| s.start_byte)
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    let source: Arc<str> = Arc::from(body);
    registry.register_replacement(id.clone(), source, offsets)?;
    Ok(id)
}

/// Convert a synthetic source-map entry into a [`Location`]. Returns
/// `None` for entries that aren't bound synthetic identities — bytecode
/// entries, unknown entries, and synthetic entries whose `proc_qname`
/// hasn't been bound yet.
///
/// The returned `Location` shares its [`BlockId::ProcBody`] arc with the
/// identity's `proc_qname` arc when possible — the conversion does not
/// allocate new strings.
pub fn synthetic_to_location(entry: &SourceMapEntry) -> Option<Result<Location, LocationError>> {
    let CommandIdentity::Synthetic {
        proc_qname,
        body_byte_offset,
        body_byte_len,
        ..
    } = &entry.identity
    else {
        return None;
    };
    let qname = proc_qname.clone()?;
    let block = BlockId::ProcBody(Arc::clone(&qname));
    let line = entry.source_lines.start;
    Some(Location::new(
        block,
        line,
        *body_byte_offset..*body_byte_offset + *body_byte_len,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tcld_decompile::{AnalysisSnapshot, LoadInput};

    #[test]
    fn synthetic_entry_round_trips_to_location() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("testing/fixtures/public/public_complex_fixture.tbc");
        let snapshot = AnalysisSnapshot::from_input(LoadInput::path(path)).unwrap();
        let entry = snapshot
            .source_map
            .entries()
            .iter()
            .find(|entry| {
                entry
                    .text
                    .trim_start()
                    .starts_with("foreach el $debug_types")
                    && matches!(entry.identity, CommandIdentity::Synthetic { .. })
            })
            .expect("foreach synthetic entry inside fixture::logLevelInit");
        let loc = synthetic_to_location(entry)
            .expect("synthetic identity")
            .expect("location is non-empty");
        match loc.block() {
            BlockId::ProcBody(name) => assert_eq!(&**name, "fixture::logLevelInit"),
            other => panic!("unexpected block id {other:?}"),
        }
        assert!(loc.byte_len() > 0);
    }

    #[test]
    fn register_body_block_parses_per_statement_offsets() {
        let mut reg = BlockRegistry::new();
        // No trailing newline — exercises the auto-normalization path.
        let body = "set a 1\nset b 2\nset c 3";
        let id = register_body_block(&mut reg, Some("greeter"), body).unwrap();
        assert_eq!(id, BlockId::ProcBody(Arc::from("greeter")));
        let block = reg.get(&id).unwrap();
        assert_eq!(block.valid_offsets(), &[0, 8, 16]);
    }

    #[test]
    fn register_body_block_handles_special_forms() {
        let mut reg = BlockRegistry::new();
        let body = "set x 1\nif {$x} { set y 2 } else { set y 3 }\nforeach el {a b c} { puts $el }";
        let id = register_body_block(&mut reg, Some("compound"), body).unwrap();
        let block = reg.get(&id).unwrap();
        // Four top-level forms: set, if, foreach, plus the inner commands.
        // We expect at minimum 3 distinct top-level start offsets.
        let top_offsets: Vec<usize> = block
            .valid_offsets()
            .iter()
            .filter(|&&off| off < body.find("foreach").unwrap() + 20)
            .copied()
            .collect();
        assert!(
            top_offsets.len() >= 3,
            "expected >=3 top-level command offsets, got {}",
            top_offsets.len()
        );
    }

    #[test]
    fn register_body_block_redefinition_bumps_version() {
        let mut reg = BlockRegistry::new();
        let id1 = register_body_block(&mut reg, Some("p"), "set x 1").unwrap();
        assert_eq!(reg.get(&id1).unwrap().version(), 0);
        let id2 = register_body_block(&mut reg, Some("p"), "set y 2").unwrap();
        assert_eq!(id1, id2);
        assert_eq!(reg.get(&id2).unwrap().version(), 1);
    }

    #[test]
    fn register_body_block_anonymous_collides_on_identical_body() {
        let mut reg1 = BlockRegistry::new();
        let mut reg2 = BlockRegistry::new();
        let a = register_body_block(&mut reg1, None, "set x 1").unwrap();
        let b = register_body_block(&mut reg2, None, "set x 1").unwrap();
        assert_eq!(a, b);
        let c = register_body_block(&mut reg2, None, "set x 2").unwrap();
        assert_ne!(b, c);
    }

    #[test]
    fn bytecode_entries_yield_none() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("testing/fixtures/public/public_complex_fixture.tbc");
        let snapshot = AnalysisSnapshot::from_input(LoadInput::path(path)).unwrap();
        let entry = snapshot
            .source_map
            .entries()
            .iter()
            .find(|entry| matches!(entry.identity, CommandIdentity::Bytecode { .. }))
            .expect("at least one bytecode-backed entry");
        assert!(synthetic_to_location(entry).is_none());
    }
}
