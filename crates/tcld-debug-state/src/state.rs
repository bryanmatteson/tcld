//! Top-level container.
//!
//! The host (`tcld-dbg-ext`) holds an `Arc<Mutex<DebugState>>` and shares
//! it with whoever else needs read access (today: nobody else; tomorrow:
//! the GUI command handler). Step commands MUST gate on
//! [`AppState::Stopped`] — invoking [`AppState::Running`] step is an
//! error, not a no-op.
//!
//! ## Hot-path discipline
//!
//! The trace handler is on the per-command hot path. For Tier 0 we do NOT
//! optimize the lock-contention shape (the handler simply locks the mutex
//! once per command). The contract reserves the right to introduce a
//! lock-free armed-locations vec later — call sites that mutate
//! breakpoints already update through a single mutator method
//! ([`DebugState::with_breakpoints_mut`]) so the future hot-path can be
//! introduced without touching call sites.

use crate::block::{BlockId, BlockRegistry};
use crate::breakpoint::{ArmedLocations, BreakpointStore};
use crate::coverage::CoverageMap;
use crate::event::{EventCallback, EventQueue, SubscriptionId};
use crate::frame::UplevelHop;
use arc_swap::ArcSwap;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// Three-state FSM mirroring TclPro's `appState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppState {
    /// No interpreter attached. Step commands return
    /// [`DebugStateError::NotAttached`].
    #[default]
    Dead,
    /// Interpreter running. Step / continue commands return
    /// [`DebugStateError::NotStopped`]. Only the trace handler can
    /// transition to `Stopped`.
    Running,
    /// Interpreter paused. Step / continue commands are valid; they
    /// transition state back to `Running`.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugStateError {
    NotAttached,
    NotStopped,
    AlreadyAttached,
}

impl std::fmt::Display for DebugStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAttached => write!(f, "no interpreter attached (state == Dead)"),
            Self::NotStopped => write!(f, "interpreter is not paused (state != Stopped)"),
            Self::AlreadyAttached => write!(
                f,
                "interpreter already attached; uninstall_step_engine before re-attaching"
            ),
        }
    }
}

impl std::error::Error for DebugStateError {}

#[derive(Default)]
pub struct DebugState {
    pub blocks: BlockRegistry,
    breakpoints: BreakpointStore,
    /// Lock-free hot-path snapshot of armed line breakpoints. The trace
    /// handler reads via [`armed_locations`](DebugState::armed_locations)
    /// without acquiring any mutex; mutators (breakpoint add/remove,
    /// validate_against) call [`rebuild_armed`](DebugState::rebuild_armed)
    /// to atomically swap in a fresh snapshot. `arc-swap` guarantees a
    /// single-instruction atomic load on the hot path.
    armed: Arc<ArcSwap<ArmedLocations>>,
    pub app_state: AppState,
    /// Registered event subscribers, keyed by [`SubscriptionId`]. Use
    /// [`Self::subscribe`] / [`Self::unsubscribe`] to mutate; the trace
    /// handler iterates `values()` from a clone of this map.
    pub event_subscribers: BTreeMap<SubscriptionId, EventCallback>,
    /// Monotonic counter for the next subscription id.
    next_subscription_id: u64,
    /// Polling drain alongside callbacks. The trace handler pushes to
    /// both. Useful for GUIs that prefer a per-tick `drain()` over
    /// per-event callbacks.
    pub event_queue: Arc<EventQueue>,
    /// Bumped each time a conditional breakpoint's condition raised a Tcl
    /// error. Part of the contract — tests assert this counter advances.
    pub condition_eval_errors: u64,
    /// Bumped each time the trace handler paused due to step-mode (not
    /// breakpoint).
    pub step_pauses: u64,
    /// Bumped each time the trace handler paused due to a breakpoint hit.
    pub breakpoint_hits: u64,
    /// Per-(block, offset) execution counts. Distinct from
    /// `Breakpoint.hit_count` (per-bp); coverage persists even when no
    /// breakpoint is set on a location and across breakpoint
    /// add/remove cycles.
    pub coverage: CoverageMap,
    /// Event-loop re-entry depth, set by the vwait/update wrappers.
    /// When > 0 the trace handler defers pauses (would otherwise
    /// deadlock the event loop). Bumped each time a Paused-but-deferred
    /// event would have fired so callers can see it from telemetry.
    pub defer_depth: u32,
    /// Count of pause events that were skipped because
    /// `defer_depth > 0`. Lets the host surface "you tried to pause
    /// inside vwait, here's how often it would have happened".
    pub deferred_pauses: u64,
    /// Stack of in-progress `uplevel` transitions. Pushed by the
    /// uplevel wrapper on entry, popped on exit. Consulted by
    /// `inspect_stack` to annotate frames whose body is currently
    /// running in someone else's scope.
    pub uplevel_hops: Vec<UplevelHop>,
    /// Blocks the host has marked "do not pause / do not coverage". The
    /// trace handler short-circuits for any command whose resolved block
    /// is in this set. Mirrors TclPro's `dbg::uninstrumentProc`.
    pub disabled_blocks: HashSet<BlockId>,
    /// One-shot flag set by `ignore_error`. When the catch wrapper sees
    /// an error and this flag is set, it consumes the flag and skips the
    /// pause for THAT error. Subsequent errors pause normally. Mirrors
    /// TclPro's `dbg::ignoreError` semantics.
    pub error_ignored_pending: bool,
    /// Result of the most-recently-completed command, captured by the
    /// trace handler before each step. Populated on the entry-trace
    /// of every command — when paused, this holds the result of the
    /// previous command. `None` means "the engine has not seen a
    /// result yet" (initial state, or just after install). Mirrors
    /// TclPro's `dbg::getResult`.
    pub last_result: Option<String>,
    /// Async pause request (Tier 3). When set to true, the trace
    /// handler treats the next traced command as a pause point and
    /// resets the flag — effectively "interrupt the running script".
    /// Stored as `Arc<AtomicBool>` because external threads (a UI
    /// pump, a signal handler, an RPC server) need to set this without
    /// holding the state mutex. Read on the hot path with `Acquire`;
    /// reset with `Acquire`+`Relaxed` swap.
    pub pause_requested: Arc<std::sync::atomic::AtomicBool>,
    /// Watch expressions evaluated automatically at every pause. The
    /// host registers `(name, expression)` pairs; on each pause the
    /// trace handler evaluates each in the paused frame's scope and
    /// can deliver the snapshot to subscribers.
    pub watches: WatchStore,
    /// Most recent watch evaluation results, keyed by watch id. Updated
    /// after every pause (or on demand via `evaluate_all_watches`).
    pub watch_results: std::collections::BTreeMap<WatchId, WatchValue>,
    /// Slave interps the host has explicitly attached to. Each entry is
    /// a Tcl-visible name (e.g. `"child"`). Engine state is per-interp;
    /// this is just the registry of which slaves the user has asked us
    /// to track. Populated by the `interp create` wrapper.
    pub slaves: std::collections::BTreeSet<String>,
}

/// Stable id for a watch expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WatchId(pub u64);

/// One auto-evaluated watch entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchExpression {
    pub id: WatchId,
    /// Display name. Often the expression itself, but the host may
    /// prefer a shorter human label.
    pub name: String,
    /// Tcl expression evaluated via `expr` in the paused frame's
    /// scope. Use raw script form for non-expression watches —
    /// the runtime decides which evaluator to call.
    pub expr: String,
    pub enabled: bool,
}

/// Result of one watch evaluation. Either the value (truncated by
/// the host as needed) or the Tcl error message that the eval raised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchValue {
    Ok(String),
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct WatchStore {
    next_id: u64,
    entries: std::collections::BTreeMap<WatchId, WatchExpression>,
}

impl WatchStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: String, expr: String) -> WatchId {
        let id = WatchId(self.next_id);
        self.next_id += 1;
        self.entries.insert(
            id,
            WatchExpression {
                id,
                name,
                expr,
                enabled: true,
            },
        );
        id
    }

    pub fn remove(&mut self, id: WatchId) -> Option<WatchExpression> {
        self.entries.remove(&id)
    }

    pub fn get(&self, id: WatchId) -> Option<&WatchExpression> {
        self.entries.get(&id)
    }

    pub fn list(&self) -> impl Iterator<Item = &WatchExpression> {
        self.entries.values()
    }

    pub fn enable(&mut self, id: WatchId, enabled: bool) -> bool {
        match self.entries.get_mut(&id) {
            Some(w) => {
                w.enabled = enabled;
                true
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl std::fmt::Debug for DebugState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugState")
            .field("blocks_len", &self.blocks.len())
            .field("breakpoints_len", &self.breakpoints.len())
            .field("app_state", &self.app_state)
            .field("event_subscribers_len", &self.event_subscribers.len())
            .field("condition_eval_errors", &self.condition_eval_errors)
            .field("step_pauses", &self.step_pauses)
            .field("breakpoint_hits", &self.breakpoint_hits)
            .finish()
    }
}

impl DebugState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Single mutator for breakpoint changes. Call sites that touch
    /// breakpoints route through here so the lock-free hot-path snapshot
    /// is always rebuilt after a mutation.
    pub fn with_breakpoints_mut<R>(
        &mut self,
        f: impl FnOnce(&mut BreakpointStore, &BlockRegistry) -> R,
    ) -> R {
        let result = f(&mut self.breakpoints, &self.blocks);
        self.rebuild_armed();
        result
    }

    /// Hot-path handle. The trace handler holds an `Arc<ArcSwap<...>>`
    /// to this and calls `.load_full()` once per command — no mutex,
    /// no allocation, single atomic load.
    pub fn armed_locations(&self) -> Arc<ArcSwap<ArmedLocations>> {
        self.armed.clone()
    }

    /// Rebuild the armed-locations snapshot from the breakpoint store.
    /// Called automatically by [`with_breakpoints_mut`]; expose as `pub`
    /// for the rare case where a caller mutates state through a
    /// non-standard path (e.g. validate_against on a registry update).
    pub fn rebuild_armed(&self) {
        let snapshot = self.breakpoints.build_armed();
        self.armed.store(Arc::new(snapshot));
    }

    /// Read-only access to the breakpoint store. The trace handler uses
    /// this from the hot path.
    pub fn breakpoints(&self) -> &BreakpointStore {
        &self.breakpoints
    }

    /// Record one execution at `(block, offset)`. Called by the trace
    /// handler on every traced command, before pause-decision logic, so
    /// coverage reflects every command the runtime actually executed
    /// regardless of breakpoint behavior.
    pub fn record_coverage(&mut self, block: &BlockId, body_byte_offset: usize) {
        self.coverage.record(block, body_byte_offset);
    }

    /// Subscribe to debug events. Returns a stable [`SubscriptionId`]
    /// that can be used to remove the subscription via
    /// [`unsubscribe`](Self::unsubscribe).
    pub fn subscribe(&mut self, callback: EventCallback) -> SubscriptionId {
        let id = SubscriptionId(self.next_subscription_id);
        self.next_subscription_id += 1;
        self.event_subscribers.insert(id, callback);
        id
    }

    /// Remove a previously registered subscription. Returns true iff a
    /// subscription with that id existed.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        self.event_subscribers.remove(&id).is_some()
    }

    /// Fan out one event to all subscribers AND push it onto the
    /// poll-drain queue. Consumers can use whichever style suits them;
    /// both reflect the same event stream.
    ///
    /// Subscribers must NOT re-lock the same `Arc<Mutex<DebugState>>`
    /// they were registered against — `emit` is typically invoked
    /// while the caller holds that lock (the trace handler updates
    /// state then emits). Use [`fire_event_unlocked`] from contexts
    /// that need subscribers to safely re-lock state.
    pub fn emit(&self, event: crate::event::DebugEvent) {
        for cb in self.event_subscribers.values() {
            cb(event.clone());
        }
        self.event_queue.push(event);
    }

    /// Snapshot subscribers + queue handle and return them so the
    /// caller can release its `DebugState` lock before invoking the
    /// subscribers. The trace handler uses this pattern to avoid
    /// deadlocking subscribers that re-lock state.
    pub fn drain_emit_into(&self) -> (Vec<EventCallback>, Arc<EventQueue>) {
        (
            self.event_subscribers.values().cloned().collect(),
            self.event_queue.clone(),
        )
    }

    /// Mark a block disabled for trace and breakpoint dispatch. Coverage
    /// recording is also skipped on disabled blocks. Idempotent.
    pub fn disable_block(&mut self, id: BlockId) {
        self.disabled_blocks.insert(id);
    }

    /// Re-enable a previously disabled block. Idempotent.
    pub fn enable_block(&mut self, id: &BlockId) {
        self.disabled_blocks.remove(id);
    }

    pub fn is_block_disabled(&self, id: &BlockId) -> bool {
        self.disabled_blocks.contains(id)
    }

    /// Step commands gate on this. Returns `Err(NotStopped)` from
    /// `Running`, `Err(NotAttached)` from `Dead`.
    pub fn require_stopped(&self) -> Result<(), DebugStateError> {
        match self.app_state {
            AppState::Stopped => Ok(()),
            AppState::Running => Err(DebugStateError::NotStopped),
            AppState::Dead => Err(DebugStateError::NotAttached),
        }
    }
}

/// Free-function helper: snapshot subscribers and queue handle from a
/// shared `DebugState`, drop the lock, then dispatch the event. Use
/// this whenever the caller holds the state lock and the subscribers
/// might re-acquire it. Direct `guard.emit(...)` is fine only when
/// subscribers are guaranteed not to touch state.
pub fn fire_event_unlocked(
    state: &Arc<std::sync::Mutex<DebugState>>,
    event: crate::event::DebugEvent,
) {
    let snapshot = state.lock().ok().map(|g| {
        (
            g.event_subscribers.values().cloned().collect::<Vec<_>>(),
            g.event_queue.clone(),
        )
    });
    if let Some((subs, queue)) = snapshot {
        for cb in &subs {
            cb(event.clone());
        }
        queue.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_stopped_only_from_stopped() {
        let mut state = DebugState::new();
        state.app_state = AppState::Dead;
        assert_eq!(state.require_stopped(), Err(DebugStateError::NotAttached));
        state.app_state = AppState::Running;
        assert_eq!(state.require_stopped(), Err(DebugStateError::NotStopped));
        state.app_state = AppState::Stopped;
        assert!(state.require_stopped().is_ok());
    }

    #[test]
    fn armed_locations_reflects_breakpoint_mutations() {
        use crate::block::BlockId;
        use crate::breakpoint::BreakpointKind;
        use crate::location::Location;
        use std::sync::Arc;

        let mut state = DebugState::new();
        let id = BlockId::ProcBody(Arc::from("p"));
        state
            .blocks
            .register(id.clone(), Arc::from("set x 1\nset y 2"), vec![0, 8])
            .unwrap();

        // Initially empty.
        let armed = state.armed_locations().load_full();
        assert!(armed.is_empty());

        let bp = state.with_breakpoints_mut(|store, registry| {
            store
                .add(
                    BreakpointKind::Line,
                    Location::new(id.clone(), 0, 8..15).unwrap(),
                    None,
                    false,
                    registry,
                )
                .unwrap()
        });

        // Snapshot is rebuilt after the mutation.
        let armed = state.armed_locations().load_full();
        assert_eq!(armed.len(), 1);
        assert_eq!(armed.find(&id, 8), Some(bp));
        assert_eq!(armed.find(&id, 0), None);

        // Disabling drops it from the armed set.
        state.with_breakpoints_mut(|store, _| store.disable(bp).unwrap());
        let armed = state.armed_locations().load_full();
        assert!(armed.is_empty());
    }

    #[test]
    fn armed_locations_handle_outlives_state_mutex_path() {
        // The handle returned by armed_locations() is meant to be cloned
        // out and held by the trace handler across many invocations.
        // Mutations to DebugState must remain visible without re-fetching
        // the handle.
        use crate::block::{Block, BlockId, BlockRegistry};
        use crate::breakpoint::BreakpointKind;
        use crate::location::Location;
        use std::sync::Arc;

        let mut state = DebugState::new();
        let id = BlockId::ProcBody(Arc::from("p"));
        state
            .blocks
            .register(id.clone(), Arc::from("set x 1\nset y 2"), vec![0, 8])
            .unwrap();
        let handle = state.armed_locations(); // cached

        // Mutate after the handle was cloned out.
        let bp = state.with_breakpoints_mut(|store, registry| {
            store
                .add(
                    BreakpointKind::Line,
                    Location::new(id.clone(), 0, 0..7).unwrap(),
                    None,
                    false,
                    registry,
                )
                .unwrap()
        });

        // The cached handle sees the new snapshot.
        assert_eq!(handle.load_full().find(&id, 0), Some(bp));

        // Confirm Block ref is unused warning silencer.
        let _ = std::any::type_name::<Block>();
        let _ = std::any::type_name::<BlockRegistry>();
    }

    #[test]
    fn emit_pushes_to_event_queue_and_callbacks() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut state = DebugState::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        state.subscribe(Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        // Snapshot the queue handle BEFORE emitting; ensures the
        // handle sees the events.
        let queue = state.event_queue.clone();
        for _ in 0..3 {
            state.emit(crate::event::DebugEvent::Resumed);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        let drained = queue.drain();
        assert_eq!(drained.len(), 3);
        assert!(queue.is_empty());
    }

    #[test]
    fn event_queue_overflow_drops_oldest() {
        use crate::event::EventQueue;
        let q = EventQueue::with_capacity(2);
        q.push(crate::event::DebugEvent::Resumed);
        q.push(crate::event::DebugEvent::Resumed);
        q.push(crate::event::DebugEvent::Resumed);
        assert_eq!(q.len(), 2);
        assert_eq!(q.overflow_count(), 1);
        q.reset_overflow_count();
        assert_eq!(q.overflow_count(), 0);
    }

    #[test]
    fn disable_block_is_idempotent_and_round_trips() {
        use crate::block::BlockId;
        use std::sync::Arc;
        let mut state = DebugState::new();
        let id = BlockId::ProcBody(Arc::from("foo"));
        assert!(!state.is_block_disabled(&id));
        state.disable_block(id.clone());
        state.disable_block(id.clone()); // idempotent
        assert!(state.is_block_disabled(&id));
        state.enable_block(&id);
        assert!(!state.is_block_disabled(&id));
    }

    #[test]
    fn error_ignored_pending_defaults_false() {
        let state = DebugState::new();
        assert!(!state.error_ignored_pending);
    }

    #[test]
    fn pause_requested_round_trips_via_atomic() {
        use std::sync::atomic::Ordering;
        let state = DebugState::new();
        assert!(!state.pause_requested.load(Ordering::Acquire));
        state.pause_requested.store(true, Ordering::Release);
        // Swap-and-test pattern the trace handler uses.
        let prev = state.pause_requested.swap(false, Ordering::AcqRel);
        assert!(prev);
        assert!(!state.pause_requested.load(Ordering::Acquire));
    }

    #[test]
    fn watch_store_assigns_unique_ids_and_lists_them() {
        let mut store = WatchStore::new();
        let a = store.add("size".into(), "[llength $items]".into());
        let b = store.add("first".into(), "[lindex $items 0]".into());
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
        assert_eq!(store.list().count(), 2);
        assert_eq!(store.get(a).unwrap().name, "size");
    }

    #[test]
    fn watch_store_remove_drops_entry() {
        let mut store = WatchStore::new();
        let id = store.add("x".into(), "$x".into());
        assert!(store.remove(id).is_some());
        assert!(store.get(id).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn watch_store_enable_toggle() {
        let mut store = WatchStore::new();
        let id = store.add("x".into(), "$x".into());
        assert!(store.get(id).unwrap().enabled);
        assert!(store.enable(id, false));
        assert!(!store.get(id).unwrap().enabled);
        assert!(!store.enable(WatchId(999), true)); // unknown id
    }

    #[test]
    fn slaves_default_empty_and_round_trip() {
        let mut state = DebugState::new();
        assert!(state.slaves.is_empty());
        state.slaves.insert("child1".into());
        state.slaves.insert("child2".into());
        assert_eq!(state.slaves.len(), 2);
        assert!(state.slaves.contains("child1"));
    }

    #[test]
    fn emit_fans_out_to_all_subscribers() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut state = DebugState::new();
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let c1 = counter.clone();
        state.subscribe(std::sync::Arc::new(move |_event| {
            c1.fetch_add(1, Ordering::SeqCst);
        }));
        let c2 = counter.clone();
        state.subscribe(std::sync::Arc::new(move |_event| {
            c2.fetch_add(10, Ordering::SeqCst);
        }));
        state.emit(crate::event::DebugEvent::Resumed);
        assert_eq!(counter.load(Ordering::SeqCst), 11);
    }
}
