//! Neutral data types passed into [`super::render`].
//!
//! Frontends translate their own controller types into these and
//! hand them to the renderer; the renderer never reaches back.

use ratatui::text::Line;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Dead,
    Starting,
    Ready,
    Loading,
    Running,
    Stopped,
    Completed,
    LoadFailed,
}

impl AppState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dead => "Dead",
            Self::Starting => "Starting debugger",
            Self::Ready => "Ready",
            Self::Loading => "Loading",
            Self::Running => "Running",
            Self::Stopped => "Paused",
            Self::Completed => "Completed",
            Self::LoadFailed => "Load Failed",
        }
    }
}

/// Why the nub is paused. The renderer colours the titlebar based on
/// this; the driver layer chooses which kind to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseKind {
    /// Step / breakpoint pause — normal flow.
    Normal,
    /// `error_paused` — Tcl error reached an armed exception bp.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceBlockKind {
    File,
    Proc,
    Anonymous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReasonLabel {
    Entry,
    Step,
    Breakpoint,
    Watchpoint,
    Pause,
    InstructionStep,
    InstructionBreakpoint,
    Error,
}

/// Where execution is currently paused.
#[derive(Debug, Clone)]
pub struct FocusedProc {
    /// `proc:foo`, `file:/path`, or `anon:HEX` — whatever the engine
    /// reported for the paused block. The driver should pass this
    /// through; the renderer surfaces it verbatim in the titlebar.
    pub block_id: String,
    pub block_kind: SourceBlockKind,
    pub source_path: Option<String>,
    /// Best-effort name to address breakpoints by (`bp_add`-friendly).
    pub qname: String,
    /// 1-based line within the block body (or anon source).
    pub line: u32,
    pub offset_start: u64,
    pub offset_end: u64,
    pub pause_reason: PauseReasonLabel,
    pub frame_count: u32,
    pub breakpoint: Option<String>,
    pub pause_kind: PauseKind,
    /// Optional error message (only meaningful when `pause_kind == Error`).
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub level: i64,
    pub kind: String,
    pub proc_name: Option<String>,
    pub block: Option<String>,
    pub block_kind: Option<SourceBlockKind>,
    pub source_path: Option<String>,
    pub line: Option<u32>,
    pub offset_start: Option<u64>,
    pub offset_end: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ProcBreakpoint {
    pub id: String,
    pub qname: String,
    pub source_path: Option<String>,
    pub line: Option<u32>,
    pub pending: bool,
    pub pending_reason: Option<String>,
    /// Byte range within the source block that the breakpoint is armed on.
    /// For a durable source breakpoint this is the requested command span;
    /// it never changes when a runtime generation is rebound.
    pub offset: u64,
    pub len: u64,
    /// Current generation-specific binding, kept separate from the durable
    /// requested coordinates above.
    pub resolved_block: Option<String>,
    pub resolved_line: Option<u32>,
    pub resolved_offset: Option<u64>,
    pub resolved_len: Option<u64>,
    pub resolution_generation: Option<u64>,
    pub enabled: bool,
    pub hits: u64,
    pub ignore_count: u64,
    pub condition: Option<String>,
    /// Var-write watchpoints carry a name instead of a line offset.
    pub kind: BreakpointKindLabel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointKindLabel {
    Line,
    VarWrite,
    Exception,
}

impl BreakpointKindLabel {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Line => "●",
            Self::VarWrite => "◆",
            Self::Exception => "!",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WatchEntry {
    pub id: String,
    pub name: String,
    pub expr: String,
    pub last_value: Option<String>,
    pub last_error: Option<String>,
    /// True when the value or error differs from the preceding refresh.
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSeverity {
    Info,
    Pause,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct EventLine {
    pub when: std::time::SystemTime,
    pub severity: EventSeverity,
    pub text: String,
}

impl EventLine {
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(EventSeverity::Info, text)
    }
    pub fn pause(text: impl Into<String>) -> Self {
        Self::new(EventSeverity::Pause, text)
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Self::new(EventSeverity::Warn, text)
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self::new(EventSeverity::Error, text)
    }
    pub fn new(severity: EventSeverity, text: impl Into<String>) -> Self {
        Self {
            when: std::time::SystemTime::now(),
            severity,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoverageRow {
    pub block: String,
    pub offset: u64,
    pub hits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommand {
    /// Byte offset in the text currently displayed.
    pub offset: u64,
    pub len: u64,
    /// Runtime proc block, when this command is nested inside a proc
    /// definition shown in the whole-file view.
    pub qname: Option<String>,
    /// Byte offset in the runtime block (`offset` for file/proc-body views).
    pub target_offset: u64,
    pub start_column: usize,
    pub end_column: usize,
    pub depth: usize,
    pub label: String,
}

/// One source line, optionally pre-styled by the driver. When
/// `spans` is empty the renderer falls back to a plain raw line.
#[derive(Debug, Clone, Default)]
pub struct SourceLine {
    pub raw: String,
    pub spans: Vec<ratatui::text::Span<'static>>,
    pub commands: Vec<SourceCommand>,
}

impl SourceLine {
    pub fn raw(text: impl Into<String>) -> Self {
        Self {
            raw: text.into(),
            spans: Vec::new(),
            commands: Vec::new(),
        }
    }
    pub fn highlighted(text: String, line: Line<'static>) -> Self {
        Self {
            raw: text,
            spans: line.spans,
            commands: Vec::new(),
        }
    }

    pub fn with_commands(mut self, commands: Vec<SourceCommand>) -> Self {
        self.commands = commands;
        self
    }
}

/// Provenance label rendered next to the source title — tells the
/// user whether the body in view came from the user-script file or
/// from a `proc_body` fetch on the paused block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOrigin {
    UserScript,
    CompiledSource,
    SourcedFile,
    ProcBody,
    Anonymous,
    Synthetic,
}

impl SourceOrigin {
    pub fn label(self) -> &'static str {
        match self {
            Self::UserScript => "user script",
            Self::CompiledSource => "compiled source",
            Self::SourcedFile => "sourced file",
            Self::ProcBody => "proc body",
            Self::Anonymous => "eval session-only",
            Self::Synthetic => "synthetic",
        }
    }
}

/// One row of the locals pane. The driver compares values against
/// the prior pause and flags the row so the renderer can highlight
/// what changed.
#[derive(Debug, Clone)]
pub struct LocalRow {
    pub name: String,
    pub value: String,
    pub changed: bool,
}

/// Live source-search state. The driver populates `query` /
/// `match_lines` while the user is searching; the renderer
/// underlines hits and badges the title with hit count + cursor.
#[derive(Debug, Clone, Default)]
pub struct SourceSearch<'a> {
    pub query: &'a str,
    pub match_lines: &'a [usize],
    pub current: Option<usize>,
}

/// Floating list popup for the proc-picker (and any future
/// fuzzy-list prompt). Driver fills `entries` from a live filter on
/// the prompt buffer; renderer pops up an overlay above the body.
#[derive(Debug, Clone, Default)]
pub struct PickerOverlay<'a> {
    pub title: &'a str,
    pub entries: &'a [String],
    pub selected: Option<usize>,
}

/// Everything the renderer needs to draw a single frame. References
/// borrowed from the frontend's owned state — no cloning required.
pub struct DebugSnapshot<'a> {
    pub title: &'a str,
    pub source_path: &'a str,
    pub source_origin: SourceOrigin,
    /// Proc body currently presented in the source pane. This can differ from
    /// the paused proc when the user browses another procedure.
    pub source_qname: Option<&'a str>,
    pub source_lines: &'a [SourceLine],
    /// Optional secondary code pane (hybrid uses source beside disassembly).
    pub secondary_source_lines: &'a [SourceLine],
    /// 1-based highlighted line in the secondary source pane.
    pub secondary_highlight_line: Option<u32>,
    pub focused_proc: Option<&'a FocusedProc>,
    pub app_state: AppState,
    pub stack: &'a [StackFrame],
    /// Currently-inspected stack-frame level (not necessarily the
    /// bottom). Drives which `level=N` locals appear.
    pub inspected_frame: i64,
    pub locals: &'a [LocalRow],
    pub breakpoints: &'a [ProcBreakpoint],
    pub watches: &'a [WatchEntry],
    pub events: &'a [EventLine],
    pub event_filter: &'a str,
    pub coverage: &'a [CoverageRow],
    pub coverage_enabled: bool,
    pub status: &'a str,
    /// When `Some`, an error banner is drawn above the body until the
    /// driver clears it (typically after the next step).
    pub error_banner: Option<&'a str>,
    pub source_search: SourceSearch<'a>,
    pub picker: Option<PickerOverlay<'a>>,
    pub theme_label: &'a str,
}
