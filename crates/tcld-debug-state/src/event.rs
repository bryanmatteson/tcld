//! Async pause-protocol shape.
//!
//! At the timescale our trace handler runs (per-command callbacks during
//! `Tcl_Eval`) there is no synchronous return point — `step_into` /
//! `continue` issue a resume and the next pause arrives whenever it
//! arrives. Sync-return would require blocking the embedding host's
//! eventloop, which breaks Tcl's `vwait` / `update`.
//!
//! [`DebugEvent`] is therefore the canonical pause-protocol shape, and
//! [`EventCallback`] is registered at `install_step_engine` time. The
//! same enum is what the future GUI's IPC layer will serialize.

use crate::block::BlockId;
use crate::breakpoint::BreakpointId;
use crate::location::Location;
use std::sync::Arc;

/// Whether an owner-reported executable source unit is entering the registry
/// for the first time or replacing an earlier runtime generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockChange {
    Registered,
    Revalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Entry,
    Step,
    Breakpoint,
    Watchpoint,
    Pause,
    InstructionStep,
    InstructionBreakpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugEvent {
    /// The traced interpreter paused. The host should now interactively
    /// query state and may issue a step / continue command.
    Paused {
        location: Location,
        /// Tcl frame depth (`info level`) at the pause point. 0 == top
        /// level, 1+ == inside a proc.
        frame_count: u32,
        /// Set when this pause was caused by a registered breakpoint, not
        /// a step-mode match.
        breakpoint: Option<BreakpointId>,
        /// Closed reason vocabulary for user-facing stop classification.
        reason: PauseReason,
        /// Bytecode program-counter offset when the runtime can report it.
        pc: Option<u64>,
    },
    /// The interpreter is no longer paused. Emitted at most once between
    /// any two `Paused` events. Lets the GUI dim its "stopped at line"
    /// indicator.
    Resumed,
    /// Uncaught Tcl error reached the catch-wrapper boundary. Emitted by
    /// the catch wrapper when error propagation would have terminated
    /// the script. Pauses iff at least one `BreakpointKind::Exception`
    /// is enabled.
    ErrorPaused {
        location: Option<Location>,
        message: String,
        error_code: Option<String>,
        breakpoint: Option<BreakpointId>,
    },
    /// A conditional breakpoint's condition raised a Tcl error. Pause did
    /// NOT happen — the breakpoint is treated as condition=false. Emitted
    /// so the GUI can surface the bad condition.
    ConditionError {
        breakpoint: BreakpointId,
        error: ConditionEvalError,
    },
    /// Block source changed; breakpoints may have been snapped or
    /// disabled.
    BlocksRevalidated {
        block: BlockId,
        /// Runtime-owned generation of this block. It is monotonic only
        /// within the current interpreter and must not be persisted as a
        /// restart identity.
        generation: u64,
        /// File that caused this unit to be registered, when Tcl can report
        /// one. For a proc this is definition provenance, not its identity.
        source_path: Option<Arc<str>>,
        change: BlockChange,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionEvalError {
    pub message: String,
}

/// Callback type registered at `install_step_engine` time.
///
/// Wrapped in `Arc` because the trace handler may need to invoke it from
/// any thread (the FFI layer is responsible for thread discipline).
pub type EventCallback = Arc<dyn Fn(DebugEvent) + Send + Sync + 'static>;

/// Stable id for one event subscription. Used by `unsubscribe` to
/// remove a registered callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubscriptionId(pub u64);

/// Bounded ring buffer for events that consumers want to poll instead
/// of receive via callback. The trace handler appends to this queue
/// every time it emits an event; consumers (e.g. a GUI poll loop) call
/// [`drain`](EventQueue::drain) to read all pending events at once.
///
/// Bounded so a runaway program doesn't grow the queue unboundedly when
/// no one is draining it. Default capacity is 4096; older events drop
/// silently when the buffer overflows. The dropped count is reported by
/// [`overflow_count`](EventQueue::overflow_count) so the consumer can
/// detect data loss.
#[derive(Debug)]
pub struct EventQueue {
    buf: std::sync::Mutex<EventQueueInner>,
}

#[derive(Debug)]
struct EventQueueInner {
    items: std::collections::VecDeque<DebugEvent>,
    capacity: usize,
    overflow: u64,
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::with_capacity(4096)
    }
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: std::sync::Mutex::new(EventQueueInner {
                items: std::collections::VecDeque::with_capacity(capacity.min(64)),
                capacity,
                overflow: 0,
            }),
        }
    }

    /// Push an event. Drops the oldest if the queue is full.
    pub fn push(&self, event: DebugEvent) {
        if let Ok(mut inner) = self.buf.lock() {
            if inner.items.len() >= inner.capacity {
                inner.items.pop_front();
                inner.overflow = inner.overflow.saturating_add(1);
            }
            inner.items.push_back(event);
        }
    }

    /// Take all currently-queued events. Caller gets a snapshot; the
    /// queue is emptied. Used by polling consumers (e.g. a UI tick).
    pub fn drain(&self) -> Vec<DebugEvent> {
        self.buf
            .lock()
            .map(|mut inner| inner.items.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.buf.lock().map(|inner| inner.items.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of events that were dropped due to overflow. Resets to 0
    /// after [`reset_overflow_count`].
    pub fn overflow_count(&self) -> u64 {
        self.buf.lock().map(|inner| inner.overflow).unwrap_or(0)
    }

    pub fn reset_overflow_count(&self) {
        if let Ok(mut inner) = self.buf.lock() {
            inner.overflow = 0;
        }
    }
}
