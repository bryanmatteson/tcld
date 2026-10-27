//! View-state container for the debugger TUI.
//!
//! Owns cursor positions, list selections, focus pane, command-line
//! prompt buffer, and the help-overlay flag. The frontend keeps
//! exactly one of these alive across frames; the renderer reads
//! everything from it (and updates the cached source-view top to
//! follow the source cursor).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Source,
    Stack,
    Locals,
    Breakpoints,
    Watches,
    Events,
    Coverage,
}

impl Pane {
    /// Panes the narrow layout folds into one tabbed inspector.
    pub fn is_inspector(self) -> bool {
        matches!(
            self,
            Self::Stack | Self::Locals | Self::Breakpoints | Self::Watches | Self::Coverage
        )
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Stack => "Stack",
            Self::Locals => "Locals",
            Self::Breakpoints => "Breakpoints",
            Self::Watches => "Watches",
            Self::Events => "Console",
            Self::Coverage => "Coverage",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Source => Self::Stack,
            Self::Stack => Self::Locals,
            Self::Locals => Self::Breakpoints,
            Self::Breakpoints => Self::Watches,
            Self::Watches => Self::Events,
            Self::Events => Self::Coverage,
            Self::Coverage => Self::Source,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Source => Self::Coverage,
            Self::Stack => Self::Source,
            Self::Locals => Self::Stack,
            Self::Breakpoints => Self::Locals,
            Self::Watches => Self::Breakpoints,
            Self::Events => Self::Watches,
            Self::Coverage => Self::Events,
        }
    }
}

#[derive(Debug, Default)]
pub struct PaneSelections {
    pub stack: ListState,
    pub locals: ListState,
    pub breakpoints: ListState,
    pub watches: ListState,
    pub events: ListState,
    pub coverage: ListState,
}

/// Generic single-line input field. Frontends layer their own kind
/// discriminator on top — the view only cares about the label, the
/// buffer, and a hint about what belongs in it.
#[derive(Debug, Default)]
pub struct PromptInput {
    pub label: String,
    pub buffer: String,
    /// Byte offset of the editing cursor; always on a char boundary.
    pub cursor: usize,
    /// What to type and how to finish, shown dimmed beside the input.
    pub placeholder: String,
}

impl PromptInput {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            buffer: String::new(),
            cursor: 0,
            placeholder: String::new(),
        }
    }

    pub fn with_initial(label: impl Into<String>, initial: impl Into<String>) -> Self {
        let buf = initial.into();
        let cursor = buf.len();
        Self {
            label: label.into(),
            buffer: buf,
            cursor,
            placeholder: String::new(),
        }
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Replace the buffer and park the cursor at its end.
    pub fn set(&mut self, text: impl Into<String>) {
        self.buffer = text.into();
        self.cursor = self.buffer.len();
    }

    /// Insert text at the cursor.
    pub fn insert(&mut self, text: &str) {
        let at = self.clamped_cursor();
        self.buffer.insert_str(at, text);
        self.cursor = at + text.len();
    }

    fn clamped_cursor(&self) -> usize {
        let mut at = self.cursor.min(self.buffer.len());
        while !self.buffer.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    fn previous_boundary(&self) -> usize {
        let at = self.clamped_cursor();
        self.buffer[..at]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self) -> usize {
        let at = self.clamped_cursor();
        self.buffer[at..]
            .chars()
            .next()
            .map_or(at, |character| at + character.len_utf8())
    }
}

/// One step of prompt input handling.
pub enum PromptStep {
    Pending,
    Submit(String),
    Cancel,
}

/// Apply a single key event to a prompt buffer. Returns whether the
/// caller should keep editing, treat the key as cancellation, or
/// consume the buffer as a submission.
///
/// Editing moves by characters, never bytes, so multi-byte input such as
/// `é` or `→` cannot split a character.
pub fn prompt_step(prompt: &mut PromptInput, key: KeyEvent) -> PromptStep {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => PromptStep::Cancel,
        KeyCode::Enter => {
            prompt.cursor = 0;
            PromptStep::Submit(std::mem::take(&mut prompt.buffer))
        }
        KeyCode::Backspace => {
            let end = prompt.clamped_cursor();
            let start = prompt.previous_boundary();
            prompt.buffer.drain(start..end);
            prompt.cursor = start;
            PromptStep::Pending
        }
        KeyCode::Delete => {
            let start = prompt.clamped_cursor();
            let end = prompt.next_boundary();
            prompt.buffer.drain(start..end);
            prompt.cursor = start;
            PromptStep::Pending
        }
        KeyCode::Left => {
            prompt.cursor = prompt.previous_boundary();
            PromptStep::Pending
        }
        KeyCode::Right => {
            prompt.cursor = prompt.next_boundary();
            PromptStep::Pending
        }
        KeyCode::Home => {
            prompt.cursor = 0;
            PromptStep::Pending
        }
        KeyCode::End => {
            prompt.cursor = prompt.buffer.len();
            PromptStep::Pending
        }
        KeyCode::Char('a') if control => {
            prompt.cursor = 0;
            PromptStep::Pending
        }
        KeyCode::Char('e') if control => {
            prompt.cursor = prompt.buffer.len();
            PromptStep::Pending
        }
        KeyCode::Char('u') if control => {
            let end = prompt.clamped_cursor();
            prompt.buffer.drain(..end);
            prompt.cursor = 0;
            PromptStep::Pending
        }
        KeyCode::Char('w') if control => {
            let end = prompt.clamped_cursor();
            let trimmed = prompt.buffer[..end].trim_end();
            let start = trimmed
                .char_indices()
                .rev()
                .find(|(_, character)| character.is_whitespace())
                .map_or(0, |(index, character)| index + character.len_utf8());
            prompt.buffer.drain(start..end);
            prompt.cursor = start;
            PromptStep::Pending
        }
        KeyCode::Char(c) if !control => {
            let mut encoded = [0; 4];
            prompt.insert(c.encode_utf8(&mut encoded));
            PromptStep::Pending
        }
        _ => PromptStep::Pending,
    }
}

/// Mouse click hit-test helper. The renderer publishes per-pane
/// rectangles each frame so the driver can resolve clicks back to
/// panes / list rows / source lines without re-running layout.
#[derive(Debug, Clone, Copy)]
pub struct PaneHit {
    pub pane: Pane,
    pub area: Rect,
}

#[derive(Debug, Default, Clone)]
pub struct HitTargets {
    pub panes: Vec<PaneHit>,
    /// Inner area of the source pane (no border). Used to translate
    /// y-clicks into source line indices.
    pub source_inner: Option<Rect>,
    pub source_view_top: usize,
    /// Cells of the source gutter (pause pointer, breakpoint marker, line
    /// number). A click here toggles a breakpoint instead of moving the
    /// cursor.
    pub source_gutter_width: u16,
    /// Clickable inspector tabs drawn by the narrow layout.
    pub inspector_tabs: Vec<PaneHit>,
    /// Inner areas of each list pane (no border) for row hit-testing.
    pub stack_inner: Option<Rect>,
    pub locals_inner: Option<Rect>,
    pub breakpoints_inner: Option<Rect>,
    pub watches_inner: Option<Rect>,
    pub events_inner: Option<Rect>,
    pub coverage_inner: Option<Rect>,
}

impl HitTargets {
    /// Whether a click lands in the source gutter.
    pub fn in_source_gutter(&self, x: u16) -> bool {
        self.source_inner
            .is_some_and(|inner| x >= inner.x && x < inner.x + self.source_gutter_width)
    }

    /// The inspector tab under a click, if any.
    pub fn inspector_tab_at(&self, x: u16, y: u16) -> Option<Pane> {
        self.inspector_tabs
            .iter()
            .find(|hit| {
                x >= hit.area.x
                    && x < hit.area.x + hit.area.width
                    && y >= hit.area.y
                    && y < hit.area.y + hit.area.height
            })
            .map(|hit| hit.pane)
    }

    pub fn pane_at(&self, x: u16, y: u16) -> Option<Pane> {
        self.panes
            .iter()
            .find(|hit| {
                x >= hit.area.x
                    && x < hit.area.x + hit.area.width
                    && y >= hit.area.y
                    && y < hit.area.y + hit.area.height
            })
            .map(|hit| hit.pane)
    }

    /// Translate a click within the source pane into a 0-based line
    /// index by combining the y offset with the cached view top.
    pub fn source_line_at(&self, x: u16, y: u16) -> Option<usize> {
        let inner = self.source_inner?;
        if x < inner.x || x >= inner.x + inner.width {
            return None;
        }
        if y < inner.y || y >= inner.y + inner.height {
            return None;
        }
        Some(self.source_view_top + (y - inner.y) as usize)
    }

    /// Translate a click within a list pane into the row index it hit.
    pub fn list_row_at(rect: Option<Rect>, y: u16) -> Option<usize> {
        let r = rect?;
        if y < r.y || y >= r.y + r.height {
            return None;
        }
        Some((y - r.y) as usize)
    }
}

/// A read-only overlay showing one value in full: a local, a watch, or the
/// last status message. Values are truncated in their panes; this is where
/// the rest of them can be read.
#[derive(Debug, Clone, Default)]
pub struct ValuePopup {
    pub title: String,
    pub body: String,
    pub scroll: u16,
}

#[derive(Debug)]
pub struct DebugUiState {
    pub focus: Pane,
    pub selections: PaneSelections,
    pub source_cursor: usize,
    pub source_command_cursor: usize,
    pub source_view_top: usize,
    /// Cells of source text scrolled off the left edge.
    pub source_hscroll: usize,
    pub center_source_cursor: bool,
    pub show_help: bool,
    pub help_scroll: u16,
    /// Waiting for the user to confirm ending a live debug session.
    pub confirm_quit: bool,
    /// Console lines scrolled back from the newest; 0 follows new output.
    pub console_scroll: usize,
    /// Side pane the narrow layout shows when focus is elsewhere.
    pub inspector_tab: Pane,
    pub popup: Option<ValuePopup>,
    /// Last-frame hit-test data. Renderer fills this; driver consults
    /// it on mouse events.
    pub hit: HitTargets,
}

impl Default for DebugUiState {
    fn default() -> Self {
        Self {
            focus: Pane::Source,
            selections: PaneSelections::default(),
            source_cursor: 0,
            source_command_cursor: 0,
            source_view_top: 0,
            source_hscroll: 0,
            center_source_cursor: false,
            show_help: false,
            help_scroll: 0,
            confirm_quit: false,
            console_scroll: 0,
            inspector_tab: Pane::Locals,
            popup: None,
            hit: HitTargets::default(),
        }
    }
}

impl DebugUiState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Focus a pane, remembering side panes for the narrow layout's tabs.
    pub fn focus_pane(&mut self, pane: Pane) {
        self.focus = pane;
        if pane.is_inspector() {
            self.inspector_tab = pane;
        }
    }

    pub fn move_list(state: &mut ListState, len: usize, delta: isize) {
        if len == 0 {
            state.select(None);
            return;
        }
        let cur = state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, (len - 1) as isize) as usize;
        state.select(Some(next));
    }

    pub fn set_list(state: &mut ListState, len: usize, target: usize) {
        if len == 0 {
            state.select(None);
            return;
        }
        state.select(Some(target.min(len - 1)));
    }
}
