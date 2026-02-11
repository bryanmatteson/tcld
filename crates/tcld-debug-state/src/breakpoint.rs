//! Breakpoint store.
//!
//! Single owner of breakpoint lifecycle. The `Tcl_CreateObjTrace` handler
//! in `tcld-dbg-ext` consults this; nothing else holds breakpoint truth.
//!
//! ## Schema vs. runtime
//!
//! [`BreakpointKind`] carries `Line`, `Instruction`, `Var`, and
//! `Exception` variants now. Only source and opcode execution have wired
//! runtime paths in this contract; `Var` and `Exception` dispatch returns
//! [`BreakpointError::VarDispatchNotYetImplemented`] /
//! [`BreakpointError::ExceptionDispatchNotYetImplemented`]. Adding the
//! variants now means the future wrapper-commands work and watchpoint
//! work are additive — no enum migration required.

use crate::block::{Block, BlockId, BlockRegistry};
use crate::location::Location;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Stable id assigned by the store. Opaque to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BreakpointId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BreakpointKind {
    /// Pause when execution reaches this command.
    Line,
    /// Pause when the bytecode program counter reaches this offset.
    Instruction { pc: usize },
    /// Pause on read/write/unset of a named variable. Tcl-side trace
    /// dispatch is wired in `tcld-dbg-ext::watchpoints`.
    Var {
        name: String,
        /// Trace flags. `true` matches that side of the variable trace.
        on_read: bool,
        on_write: bool,
        on_unset: bool,
    },
    /// Pause on Tcl error. The catch wrapper records the error and
    /// consults this kind:
    ///
    /// - `uncaught_only: true` — pause only when the error is escaping
    ///   the outermost `catch` frame. Equivalent to "stop on uncaught
    ///   exception" in conventional debuggers; matches TclPro's
    ///   errorAction semantics.
    /// - `uncaught_only: false` — pause on any error reaching a catch
    ///   wrapper, even nested. Useful for diagnosing "why does this
    ///   inner error happen at all" without disabling outer catches.
    Exception { uncaught_only: bool },
}

#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub id: BreakpointId,
    pub kind: BreakpointKind,
    pub location: Location,
    /// Tcl expression evaluated in the paused frame's scope. The
    /// breakpoint pauses iff this evaluates to non-zero. Eval errors do
    /// NOT pause; they bump `condition_eval_errors` on
    /// [`crate::DebugState`].
    pub condition: Option<String>,
    pub enabled: bool,
    /// Auto-removed on first hit. Powers run-to-location.
    pub temporary: bool,
    pub hit_count: u64,
    /// Skip the first `ignore_count` hits before pausing. Hit (N+1)
    /// pauses; thereafter every hit pauses. `hit_count` increments
    /// regardless. Mirrors GDB's "ignore" + TclPro's per-bp skip-count.
    pub ignore_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakpointError {
    UnregisteredBlock,
    InvalidOffset {
        offset: usize,
        nearest_valid: Option<usize>,
    },
    /// Breakpoint id not found.
    NotFound(BreakpointId),
    /// `BreakpointKind::Var` was added but trace dispatch is not in this
    /// release; schema-only. Returned by the trace handler when it
    /// declines to wire the trace.
    VarDispatchNotYetImplemented,
    /// `BreakpointKind::Exception` was added but error trapping is not in
    /// this release; depends on Tier 1 #5 wrapper commands.
    ExceptionDispatchNotYetImplemented,
}

impl std::fmt::Display for BreakpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnregisteredBlock => write!(f, "block not registered"),
            Self::InvalidOffset {
                offset,
                nearest_valid,
            } => write!(
                f,
                "byte offset {offset} not on a command boundary; nearest valid: {nearest_valid:?}"
            ),
            Self::NotFound(id) => write!(f, "breakpoint id {} not found", id.0),
            Self::VarDispatchNotYetImplemented => write!(
                f,
                "variable breakpoint dispatch not yet implemented (schema variant only)"
            ),
            Self::ExceptionDispatchNotYetImplemented => write!(
                f,
                "exception breakpoint dispatch not yet implemented (schema variant only)"
            ),
        }
    }
}

impl std::error::Error for BreakpointError {}

/// What [`BreakpointStore::validate_against`] decided for one breakpoint
/// when its block's source changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// Offset still landed on a valid command boundary; no change.
    Unchanged,
    /// Snapped from `was` to `now`. The breakpoint stays enabled.
    Snapped { was: usize, now: usize },
    /// No valid offset >= the requested offset; breakpoint disabled.
    Disabled { was: usize },
}

#[derive(Debug, Default, Clone)]
pub struct BreakpointStore {
    next_id: u64,
    breakpoints: BTreeMap<BreakpointId, Breakpoint>,
}

impl BreakpointStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a breakpoint. Validates that:
    /// 1. The location's block is registered.
    /// 2. The location's byte offset is on a command boundary in that
    ///    block.
    ///
    /// Returns `Err(BreakpointError::UnregisteredBlock)` or
    /// `Err(BreakpointError::InvalidOffset)` rather than silently snapping
    /// — snapping is what `validate_against` does AFTER a re-source. At
    /// add-time the caller has authoritative source and should not ask for
    /// a fuzzy offset.
    pub fn add(
        &mut self,
        kind: BreakpointKind,
        location: Location,
        condition: Option<String>,
        temporary: bool,
        registry: &BlockRegistry,
    ) -> Result<BreakpointId, BreakpointError> {
        let block = registry
            .get(location.block())
            .ok_or(BreakpointError::UnregisteredBlock)?;
        let offset = location.byte_offset();
        if !matches!(
            kind,
            BreakpointKind::Var { .. }
                | BreakpointKind::Exception { .. }
                | BreakpointKind::Instruction { .. }
        ) && !block.is_valid_offset(offset)
        {
            return Err(BreakpointError::InvalidOffset {
                offset,
                nearest_valid: block.snap_to_nearest(offset),
            });
        }
        let id = BreakpointId(self.next_id);
        self.next_id += 1;
        self.breakpoints.insert(
            id,
            Breakpoint {
                id,
                kind,
                location,
                condition,
                enabled: true,
                temporary,
                hit_count: 0,
                ignore_count: 0,
            },
        );
        Ok(id)
    }

    /// Set the ignore count on an existing breakpoint. Hit `(ignore_count + 1)`
    /// pauses; earlier hits increment `hit_count` but skip the pause.
    pub fn set_ignore_count(
        &mut self,
        id: BreakpointId,
        ignore_count: u64,
    ) -> Result<(), BreakpointError> {
        self.breakpoints
            .get_mut(&id)
            .ok_or(BreakpointError::NotFound(id))?
            .ignore_count = ignore_count;
        Ok(())
    }

    pub fn remove(&mut self, id: BreakpointId) -> Result<Breakpoint, BreakpointError> {
        self.breakpoints
            .remove(&id)
            .ok_or(BreakpointError::NotFound(id))
    }

    pub fn get(&self, id: BreakpointId) -> Option<&Breakpoint> {
        self.breakpoints.get(&id)
    }

    pub fn get_mut(&mut self, id: BreakpointId) -> Option<&mut Breakpoint> {
        self.breakpoints.get_mut(&id)
    }

    pub fn list(&self) -> impl Iterator<Item = &Breakpoint> {
        self.breakpoints.values()
    }

    pub fn enable(&mut self, id: BreakpointId) -> Result<(), BreakpointError> {
        self.breakpoints
            .get_mut(&id)
            .ok_or(BreakpointError::NotFound(id))?
            .enabled = true;
        Ok(())
    }

    pub fn disable(&mut self, id: BreakpointId) -> Result<(), BreakpointError> {
        self.breakpoints
            .get_mut(&id)
            .ok_or(BreakpointError::NotFound(id))?
            .enabled = false;
        Ok(())
    }

    /// Record a hit. Used by the trace handler. Increments `hit_count`;
    /// removes the breakpoint if `temporary == true`.
    pub fn record_hit(&mut self, id: BreakpointId) {
        if let Some(bp) = self.breakpoints.get_mut(&id) {
            bp.hit_count += 1;
            let temporary = bp.temporary;
            if temporary {
                self.breakpoints.remove(&id);
            }
        }
    }

    /// Increment `hit_count` and decide whether the runtime should pause
    /// for this hit. Returns `true` when `hit_count > ignore_count`.
    /// Removes the breakpoint when `temporary == true` AND the caller is
    /// actually pausing (skipped hits do not consume the temporary).
    /// Returns `false` for non-existent ids.
    pub fn record_hit_and_check_pause(&mut self, id: BreakpointId) -> bool {
        let (should_pause, temporary) = match self.breakpoints.get_mut(&id) {
            Some(bp) => {
                bp.hit_count = bp.hit_count.saturating_add(1);
                let pause = bp.hit_count > bp.ignore_count;
                (pause, bp.temporary)
            }
            None => return false,
        };
        if should_pause && temporary {
            self.breakpoints.remove(&id);
        }
        should_pause
    }

    /// Move a breakpoint's location. Used when the GUI drags a breakpoint
    /// gutter marker; not by the runtime. The new location must satisfy
    /// the same validity rules as `add`.
    pub fn relocate(
        &mut self,
        id: BreakpointId,
        new_location: Location,
        registry: &BlockRegistry,
    ) -> Result<(), BreakpointError> {
        let kind = self
            .breakpoints
            .get(&id)
            .ok_or(BreakpointError::NotFound(id))?
            .kind
            .clone();
        let block = registry
            .get(new_location.block())
            .ok_or(BreakpointError::UnregisteredBlock)?;
        let offset = new_location.byte_offset();
        if !matches!(
            kind,
            BreakpointKind::Var { .. }
                | BreakpointKind::Exception { .. }
                | BreakpointKind::Instruction { .. }
        ) && !block.is_valid_offset(offset)
        {
            return Err(BreakpointError::InvalidOffset {
                offset,
                nearest_valid: block.snap_to_nearest(offset),
            });
        }
        self.breakpoints
            .get_mut(&id)
            .ok_or(BreakpointError::NotFound(id))?
            .location = new_location;
        Ok(())
    }

    /// Find a breakpoint at exactly `(block_id, body_byte_offset)`. The
    /// trace handler's hot path uses this lookup form to decide whether
    /// the current command should pause.
    pub fn find_at_offset(
        &self,
        block_id: &crate::block::BlockId,
        body_byte_offset: usize,
    ) -> Option<&Breakpoint> {
        self.breakpoints.values().find(|bp| {
            bp.location.block() == block_id && bp.location.byte_offset() == body_byte_offset
        })
    }

    pub fn find_instruction_at_pc(&self, pc: usize) -> Option<&Breakpoint> {
        self.breakpoints.values().find(|bp| {
            bp.enabled
                && matches!(bp.kind, BreakpointKind::Instruction { pc: bp_pc } if bp_pc == pc)
        })
    }

    /// Reconcile each breakpoint in `block` against the block's current
    /// `valid_offsets`. Called when a block's version increments (e.g.
    /// proc redefined). Mirrors TclPro's `dbg::validateBreakpoints`.
    ///
    /// Returns the per-breakpoint outcome so the host can surface it.
    pub fn validate_against(&mut self, block: &Block) -> Vec<(BreakpointId, ValidationOutcome)> {
        let mut outcomes = Vec::new();
        for bp in self.breakpoints.values_mut() {
            if !matches!(bp.kind, BreakpointKind::Line) {
                continue;
            }
            if bp.location.block() != block.id() {
                continue;
            }
            let offset = bp.location.byte_offset();
            if block.is_valid_offset(offset) {
                outcomes.push((bp.id, ValidationOutcome::Unchanged));
                continue;
            }
            match block.snap_to_nearest(offset) {
                Some(snapped) => {
                    let line = bp.location.line();
                    let new_loc = Location::new(
                        block.id().clone(),
                        line,
                        snapped..snapped + bp.location.byte_len(),
                    )
                    .or_else(|_| Location::new(block.id().clone(), line, snapped..snapped + 1))
                    .expect("snapped location is non-empty");
                    bp.location = new_loc;
                    outcomes.push((
                        bp.id,
                        ValidationOutcome::Snapped {
                            was: offset,
                            now: snapped,
                        },
                    ));
                }
                None => {
                    bp.enabled = false;
                    outcomes.push((bp.id, ValidationOutcome::Disabled { was: offset }));
                }
            }
        }
        outcomes
    }

    pub fn len(&self) -> usize {
        self.breakpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.breakpoints.is_empty()
    }

    /// Find the first enabled `BreakpointKind::Exception` breakpoint, if
    /// any. Used by the catch wrapper to decide whether to pause on an
    /// uncaught Tcl error. There is conventionally at most one — the
    /// debugger UI exposes "pause on error" as a toggle, not a
    /// per-call-site breakpoint — but we don't enforce uniqueness in
    /// the store.
    pub fn first_enabled_exception(&self) -> Option<&Breakpoint> {
        self.breakpoints
            .values()
            .find(|bp| bp.enabled && matches!(bp.kind, BreakpointKind::Exception { .. }))
    }

    /// Build an [`ArmedLocations`] snapshot for the trace handler's
    /// lock-free hot path. Called by [`crate::DebugState`] every time
    /// breakpoints are mutated.
    pub fn build_armed(&self) -> ArmedLocations {
        let mut entries: Vec<ArmedEntry> = self
            .breakpoints
            .values()
            .filter(|bp| bp.enabled && matches!(bp.kind, BreakpointKind::Line))
            .map(|bp| ArmedEntry {
                block: bp.location.block().clone(),
                offset: bp.location.byte_offset(),
                bp: bp.id,
            })
            .collect();
        // Sort by (block, offset) so the hot path can binary-search.
        entries.sort_by(|a, b| a.block.cmp(&b.block).then(a.offset.cmp(&b.offset)));
        ArmedLocations { entries }
    }
}

/// Snapshot of currently-armed line breakpoints for the trace handler's
/// hot path. Wrapped in `Arc<ArcSwap<ArmedLocations>>` on
/// [`crate::DebugState`]; the handler reads it without acquiring any
/// mutex on the no-pause path. Rebuilt by [`BreakpointStore::build_armed`]
/// every time a mutation goes through [`crate::DebugState::with_breakpoints_mut`].
#[derive(Debug, Clone, Default)]
pub struct ArmedLocations {
    entries: Vec<ArmedEntry>,
}

#[derive(Debug, Clone)]
struct ArmedEntry {
    block: BlockId,
    offset: usize,
    bp: BreakpointId,
}

impl ArmedLocations {
    pub fn empty() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hot-path lookup. Binary-search over a sorted vec; O(log n) and
    /// allocation-free. The trace handler calls this directly without
    /// taking any lock.
    pub fn find(&self, block: &BlockId, offset: usize) -> Option<BreakpointId> {
        self.entries
            .binary_search_by(|entry| entry.block.cmp(block).then(entry.offset.cmp(&offset)))
            .ok()
            .map(|idx| self.entries[idx].bp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{BlockId, BlockRegistry};
    use std::sync::Arc;

    fn arc(s: &str) -> Arc<str> {
        Arc::from(s)
    }

    fn registry_with(id: BlockId, source: &str, offsets: Vec<usize>) -> BlockRegistry {
        let mut reg = BlockRegistry::new();
        reg.register(id, arc(source), offsets).unwrap();
        reg
    }

    fn line_loc(id: BlockId, offset: usize, len: usize) -> Location {
        Location::new(id, 0, offset..offset + len).unwrap()
    }

    #[test]
    fn add_rejects_unregistered_block() {
        let registry = BlockRegistry::new();
        let mut store = BreakpointStore::new();
        let loc = line_loc(BlockId::ProcBody(arc("ghost")), 0, 4);
        let err = store
            .add(BreakpointKind::Line, loc, None, false, &registry)
            .unwrap_err();
        assert_eq!(err, BreakpointError::UnregisteredBlock);
    }

    #[test]
    fn add_rejects_offset_not_on_command_boundary() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1\nset y 2", vec![0, 8]);
        let mut store = BreakpointStore::new();
        let bad = line_loc(id, 4, 4);
        let err = store
            .add(BreakpointKind::Line, bad, None, false, &registry)
            .unwrap_err();
        assert!(matches!(
            err,
            BreakpointError::InvalidOffset {
                offset: 4,
                nearest_valid: Some(8)
            }
        ));
    }

    #[test]
    fn add_accepts_var_and_exception_at_arbitrary_offset() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "irrelevant", vec![0]);
        let mut store = BreakpointStore::new();
        // Var/Exception breakpoints don't bind to a command boundary.
        store
            .add(
                BreakpointKind::Var {
                    name: "x".into(),
                    on_read: true,
                    on_write: true,
                    on_unset: true,
                },
                line_loc(id.clone(), 5, 1),
                None,
                false,
                &registry,
            )
            .unwrap();
        store
            .add(
                BreakpointKind::Exception {
                    uncaught_only: true,
                },
                line_loc(id, 99, 1),
                None,
                false,
                &registry,
            )
            .unwrap();
    }

    #[test]
    fn add_accepts_instruction_breakpoint_at_pc_without_source_boundary() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1", vec![0]);
        let mut store = BreakpointStore::new();
        let bp = store
            .add(
                BreakpointKind::Instruction { pc: 12 },
                line_loc(id.clone(), 99, 1),
                None,
                false,
                &registry,
            )
            .unwrap();
        assert_eq!(store.find_instruction_at_pc(12).map(|bp| bp.id), Some(bp));
        assert!(store.find_instruction_at_pc(13).is_none());

        let mut registry2 = BlockRegistry::new();
        registry2
            .register_replacement(id, arc("set y 2"), vec![0])
            .unwrap();
        let outcomes = store.validate_against(registry2.get(&BlockId::ProcBody(arc("p"))).unwrap());
        assert!(outcomes.is_empty());
    }

    #[test]
    fn add_assigns_unique_ids_and_lists_them() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1", vec![0]);
        let mut store = BreakpointStore::new();
        let a = store
            .add(
                BreakpointKind::Line,
                line_loc(id.clone(), 0, 1),
                None,
                false,
                &registry,
            )
            .unwrap();
        let b = store
            .add(
                BreakpointKind::Line,
                line_loc(id, 0, 1),
                None,
                false,
                &registry,
            )
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn temporary_breakpoint_removes_itself_on_first_hit() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1", vec![0]);
        let mut store = BreakpointStore::new();
        let bp = store
            .add(
                BreakpointKind::Line,
                line_loc(id, 0, 1),
                None,
                true,
                &registry,
            )
            .unwrap();
        store.record_hit(bp);
        assert!(store.get(bp).is_none());
    }

    #[test]
    fn permanent_breakpoint_increments_hit_count() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1", vec![0]);
        let mut store = BreakpointStore::new();
        let bp = store
            .add(
                BreakpointKind::Line,
                line_loc(id, 0, 1),
                None,
                false,
                &registry,
            )
            .unwrap();
        store.record_hit(bp);
        store.record_hit(bp);
        assert_eq!(store.get(bp).unwrap().hit_count, 2);
    }

    #[test]
    fn ignore_count_skips_first_n_hits_then_pauses() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1", vec![0]);
        let mut store = BreakpointStore::new();
        let bp = store
            .add(
                BreakpointKind::Line,
                line_loc(id, 0, 1),
                None,
                false,
                &registry,
            )
            .unwrap();
        store.set_ignore_count(bp, 2).unwrap();
        // Hits 1, 2 should not pause; hit 3 onwards should.
        assert!(!store.record_hit_and_check_pause(bp));
        assert!(!store.record_hit_and_check_pause(bp));
        assert!(store.record_hit_and_check_pause(bp));
        assert!(store.record_hit_and_check_pause(bp));
        assert_eq!(store.get(bp).unwrap().hit_count, 4);
    }

    #[test]
    fn ignore_count_respects_temporary_only_on_pausing_hit() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1", vec![0]);
        let mut store = BreakpointStore::new();
        let bp = store
            .add(
                BreakpointKind::Line,
                line_loc(id, 0, 1),
                None,
                true,
                &registry,
            )
            .unwrap();
        store.set_ignore_count(bp, 1).unwrap();
        // First hit: skipped, not removed.
        assert!(!store.record_hit_and_check_pause(bp));
        assert!(store.get(bp).is_some());
        // Second hit: pauses, removed.
        assert!(store.record_hit_and_check_pause(bp));
        assert!(store.get(bp).is_none());
    }

    #[test]
    fn validate_against_snaps_when_offset_no_longer_valid() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1\nset y 2", vec![0, 8]);
        let mut store = BreakpointStore::new();
        let bp = store
            .add(
                BreakpointKind::Line,
                line_loc(id.clone(), 8, 7),
                None,
                false,
                &registry,
            )
            .unwrap();
        // Replacement block whose only valid offset is 16.
        let mut registry2 = BlockRegistry::new();
        registry2
            .register_replacement(id.clone(), arc("# header\nset y 2\nset z 3"), vec![9, 17])
            .unwrap();
        let block = registry2.get(&id).unwrap();
        let outcomes = store.validate_against(block);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].0, bp);
        assert!(matches!(
            outcomes[0].1,
            ValidationOutcome::Snapped { was: 8, now: 9 }
        ));
        assert!(store.get(bp).unwrap().enabled);
    }

    #[test]
    fn validate_against_disables_when_no_offset_above_remains() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1", vec![0]);
        let mut store = BreakpointStore::new();
        let bp = store
            .add(
                BreakpointKind::Line,
                line_loc(id.clone(), 0, 7),
                None,
                false,
                &registry,
            )
            .unwrap();
        let mut registry2 = BlockRegistry::new();
        registry2
            .register_replacement(id.clone(), arc(""), vec![])
            .unwrap();
        let block = registry2.get(&id).unwrap();
        let outcomes = store.validate_against(block);
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0].1,
            ValidationOutcome::Disabled { was: 0 }
        ));
        assert!(!store.get(bp).unwrap().enabled);
    }

    #[test]
    fn find_at_offset_returns_matching_breakpoint() {
        let id = BlockId::ProcBody(arc("p"));
        let registry = registry_with(id.clone(), "set x 1\nset y 2", vec![0, 8]);
        let mut store = BreakpointStore::new();
        store
            .add(
                BreakpointKind::Line,
                line_loc(id.clone(), 0, 7),
                None,
                false,
                &registry,
            )
            .unwrap();
        let target = store
            .add(
                BreakpointKind::Line,
                line_loc(id.clone(), 8, 7),
                None,
                false,
                &registry,
            )
            .unwrap();
        let hit = store.find_at_offset(&id, 8).unwrap();
        assert_eq!(hit.id, target);
    }
}
