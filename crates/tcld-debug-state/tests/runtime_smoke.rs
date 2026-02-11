//! End-to-end format smoke test against `testing/fixtures/public/public_complex_fixture.tbc`.
//!
//! Exercises the full data-flow path: AnalysisSnapshot → Synthetic
//! CommandIdentity → Location → Block registration → Breakpoint add →
//! find_at_offset round-trip. The runtime-half (trace handler firing on
//! a live Tcl interp) is excluded from this smoke because it requires
//! Tcl runtime infrastructure outside this crate's dependency closure;
//! that path is exercised by `cargo test -p tcld-dbg-ext` (unit tests
//! against the trace machinery's pure logic).
//!
//! What this test asserts is the Tier-0 acceptance contract: a
//! breakpoint set against a synthetic identity in a bcproc body is
//! reachable via the runtime-key shape `(BlockId, body_byte_offset)` —
//! exactly what the trace handler would derive from `[info frame]`.

use std::path::PathBuf;
use std::sync::Arc;

use tcld_debug_state::{
    BlockId, BreakpointError, BreakpointKind, BreakpointStore, DebugState, Location,
    synthetic_to_location,
};
use tcld_decompile::decompiler::CommandIdentity;
use tcld_decompile::{AnalysisSnapshot, LoadInput};

fn public_fixture_snapshot() -> AnalysisSnapshot {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testing/fixtures/public/public_complex_fixture.tbc");
    AnalysisSnapshot::from_input(LoadInput::path(path)).expect("public_complex_fixture.tbc loads")
}

fn populate_debug_state(snapshot: &AnalysisSnapshot) -> DebugState {
    use std::collections::BTreeMap;
    let mut state = DebugState::new();
    let mut by_qname: BTreeMap<Arc<str>, Vec<&tcld_decompile::SourceMapEntry>> = BTreeMap::new();
    for entry in snapshot.source_map.entries() {
        if let CommandIdentity::Synthetic {
            proc_qname: Some(qname),
            ..
        } = &entry.identity
        {
            by_qname.entry(qname.clone()).or_default().push(entry);
        }
    }
    for (qname, entries) in by_qname {
        let mut offsets: Vec<usize> = entries
            .iter()
            .filter_map(|entry| match &entry.identity {
                CommandIdentity::Synthetic {
                    body_byte_offset, ..
                } => Some(*body_byte_offset),
                _ => None,
            })
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        let max_end = entries
            .iter()
            .map(|e| match &e.identity {
                CommandIdentity::Synthetic {
                    body_byte_offset,
                    body_byte_len,
                    ..
                } => body_byte_offset + body_byte_len,
                _ => 0,
            })
            .max()
            .unwrap_or(0);
        let placeholder: Arc<str> = Arc::from(" ".repeat(max_end));
        state
            .blocks
            .register(BlockId::ProcBody(qname), placeholder, offsets)
            .expect("block registers");
    }
    state
}

#[test]
fn synthetic_breakpoint_round_trips_through_runtime_key_shape() {
    let snapshot = public_fixture_snapshot();

    // Pick a representative synthetic span: the foreach inside
    // fixture::logLevelInit body.
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
        .expect("foreach synthetic entry");
    let location = synthetic_to_location(entry)
        .expect("synthetic identity")
        .expect("non-empty location");

    let mut state = populate_debug_state(&snapshot);
    let bp_id = state
        .with_breakpoints_mut(|store: &mut BreakpointStore, registry| {
            store.add(
                BreakpointKind::Line,
                location.clone(),
                None,
                false,
                registry,
            )
        })
        .expect("breakpoint registers against the populated block");

    // The trace handler's runtime path: derive (BlockId, body_byte_offset)
    // from [info frame] and call find_at_offset. Reproduce that here.
    let hit = state
        .breakpoints()
        .find_at_offset(location.block(), location.byte_offset())
        .expect("breakpoint is discoverable by runtime-key shape");
    assert_eq!(hit.id, bp_id);
    assert_eq!(hit.location.byte_range(), location.byte_range());
    match &hit.location.block() {
        BlockId::ProcBody(name) => assert_eq!(&**name, "fixture::logLevelInit"),
        other => panic!("unexpected block id {other:?}"),
    }
}

#[test]
fn breakpoint_add_rejects_offset_not_on_command_boundary() {
    let snapshot = public_fixture_snapshot();
    let mut state = populate_debug_state(&snapshot);
    // Use a known proc and an offset deliberately not on any boundary.
    let block_id = BlockId::ProcBody(Arc::from("fixture::logLevelInit"));
    let block = state.blocks.get(&block_id).expect("block populated");
    let bad_offset = block.valid_offsets().first().copied().unwrap_or(0) + 1;
    assert!(!block.is_valid_offset(bad_offset));

    let bad_loc = Location::new(block_id.clone(), 0, bad_offset..bad_offset + 1).unwrap();
    let err = state
        .with_breakpoints_mut(|store, registry| {
            store.add(BreakpointKind::Line, bad_loc, None, false, registry)
        })
        .unwrap_err();
    assert!(matches!(err, BreakpointError::InvalidOffset { .. }));
}

#[test]
fn breakpoint_add_rejects_unregistered_block() {
    let snapshot = public_fixture_snapshot();
    let mut state = populate_debug_state(&snapshot);
    let ghost = BlockId::ProcBody(Arc::from("does::not::exist"));
    let loc = Location::new(ghost, 0, 0..1).unwrap();
    let err = state
        .with_breakpoints_mut(|store, registry| {
            store.add(BreakpointKind::Line, loc, None, false, registry)
        })
        .unwrap_err();
    assert_eq!(err, BreakpointError::UnregisteredBlock);
}

#[test]
fn debug_state_population_covers_a_known_public_fixture_proc() {
    // Population covers every proc whose body has at least one bound
    // synthetic identity. fixture::logLevelInit is the canonical example
    // (its body has a foreach/if/set tree that the syntax pass walks).
    // Procs whose body the bytecode-walker emits as a single
    // `Bytecode`-identity span won't appear here — that's expected:
    // synthetic identity exists to fill gaps the bytecode walker
    // missed, not duplicate it.
    let snapshot = public_fixture_snapshot();
    let state = populate_debug_state(&snapshot);
    let id = BlockId::ProcBody(Arc::from("fixture::logLevelInit"));
    let block = state
        .blocks
        .get(&id)
        .expect("fixture::logLevelInit must populate (it exercises every command kind)");
    assert!(
        block.valid_offsets().len() >= 4,
        "fixture::logLevelInit body has at least 4 inner commands"
    );
    assert!(!state.blocks.is_empty());
}
