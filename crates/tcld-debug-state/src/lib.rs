//! Runtime data model for source-level Tcl debugging in tcld.
//!
//! This crate is the format-half of the debugger: stable identities for
//! executable code units (`Block`, `Location`), the breakpoint store, the
//! app-state FSM, and the event protocol. The runtime-half (the
//! `Tcl_CreateObjTrace` handler that consults this state and pauses
//! execution) lives in `tcld-dbg-ext`.
//!
//! ## Why this is a separate crate
//!
//! `tcld-decompile` produces source maps; `tcld-dbg-ext` is FFI-heavy and
//! ties to a specific Tcl runtime. The data model has to be visible to both
//! sides — and to tcld-explore, the GUI, and any future remote-debugger
//! frontend — without dragging either into the other. The dependency arrow
//! is one-way: this crate consumes `tcld-decompile` types but
//! `tcld-decompile` does not depend on this crate.
//!
//! ## Format vs. runtime
//!
//! Everything here shapes what a valid debug state *is*. The runtime half
//! (what gets done with that state when the program is running) is gated:
//! `BreakpointKind::Var` / `BreakpointKind::Exception` / instruction
//! breakpoints exist as schema
//! variants but their dispatch returns `Err(BreakpointError::*NotYetImplemented)`.

pub mod block;
pub mod breakpoint;
pub mod coverage;
pub mod event;
pub mod frame;
pub mod identity;
pub mod location;
pub mod state;

pub use block::{Block, BlockError, BlockId, BlockRegistry};
pub use breakpoint::{
    ArmedLocations, Breakpoint, BreakpointError, BreakpointId, BreakpointKind, BreakpointStore,
    ValidationOutcome,
};
pub use coverage::CoverageMap;
pub use event::{
    ConditionEvalError, DebugEvent, EventCallback, EventQueue, PauseReason, SubscriptionId,
};
pub use frame::{FrameKind, FrameSnapshot, UplevelHop};
pub use identity::{register_body_block, synthetic_to_location};
pub use location::{Location, LocationError};
pub use state::{
    AppState, DebugState, DebugStateError, WatchExpression, WatchId, WatchStore, WatchValue,
    fire_event_unlocked,
};
