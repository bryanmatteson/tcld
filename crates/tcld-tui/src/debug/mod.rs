//! Reusable view layer for an interactive Tcl source-level debugger.
//!
//! This module is intentionally driver-agnostic: it knows nothing
//! about the wire protocol, the legacy rdebug, or any specific
//! controller crate. A frontend (the `tcld debug` CLI, a future
//! debug-adapter bridge, the GUI shell, …) populates a
//! [`DebugSnapshot`] with whatever data it has on hand, threads the
//! per-frame [`DebugUiState`] for cursor / focus / list selections,
//! and calls [`render`] each frame.
//!
//! All colors come from [`crate::Theme`] so palette switches in the
//! explorer apply here verbatim.

pub mod bindings;
pub mod render;
pub mod state;
pub mod types;

pub use bindings::{
    BindingContext, DEBUG_BINDINGS, DebugAction, compact_hints, help_lines, resolve_binding,
    verify_bindings,
};
pub use render::{panel_block, render, render_in};
pub use state::{
    DebugUiState, HitTargets, Pane, PaneHit, PaneSelections, PromptInput, PromptStep, prompt_step,
};
pub use types::{
    AppState, BreakpointKindLabel, CoverageRow, DebugSnapshot, EventLine, EventSeverity,
    FocusedProc, LocalRow, PauseKind, PauseReasonLabel, PickerOverlay, ProcBreakpoint,
    SourceBlockKind, SourceCommand, SourceLine, SourceOrigin, SourceSearch, StackFrame, WatchEntry,
};
