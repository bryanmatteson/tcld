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
/// discriminator on top — the view only cares about the label and
/// the buffer.
#[derive(Debug, Default)]
pub struct PromptInput {
    pub label: String,
    pub buffer: String,
    pub cursor: usize,
}

impl PromptInput {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            buffer: String::new(),
            cursor: 0,
        }
    }

    pub fn with_initial(label: impl Into<String>, initial: impl Into<String>) -> Self {
        let buf = initial.into();
        let cursor = buf.len();
        Self {
            label: label.into(),
            buffer: buf,
            cursor,
        }
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
pub fn prompt_step(prompt: &mut PromptInput, key: KeyEvent) -> PromptStep {
    match key.code {
        KeyCode::Esc => PromptStep::Cancel,
        KeyCode::Enter => PromptStep::Submit(std::mem::take(&mut prompt.buffer)),
        KeyCode::Backspace => {
            if prompt.cursor > 0 {
                prompt.buffer.remove(prompt.cursor - 1);
                prompt.cursor -= 1;
            }
            PromptStep::Pending
        }
        KeyCode::Delete => {
            if prompt.cursor < prompt.buffer.len() {
                prompt.buffer.remove(prompt.cursor);
            }
            PromptStep::Pending
        }
        KeyCode::Left => {
            prompt.cursor = prompt.cursor.saturating_sub(1);
            PromptStep::Pending
        }
        KeyCode::Right => {
            prompt.cursor = (prompt.cursor + 1).min(prompt.buffer.len());
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
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            prompt.buffer.insert(prompt.cursor, c);
            prompt.cursor += 1;
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
    /// Inner areas of each list pane (no border) for row hit-testing.
    pub stack_inner: Option<Rect>,
    pub locals_inner: Option<Rect>,
    pub breakpoints_inner: Option<Rect>,
    pub watches_inner: Option<Rect>,
    pub events_inner: Option<Rect>,
    pub coverage_inner: Option<Rect>,
}

impl HitTargets {
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

#[derive(Debug)]
pub struct DebugUiState {
    pub focus: Pane,
    pub selections: PaneSelections,
    pub source_cursor: usize,
    pub source_command_cursor: usize,
    pub source_view_top: usize,
    pub center_source_cursor: bool,
    pub show_help: bool,
    pub help_scroll: u16,
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
            center_source_cursor: false,
            show_help: false,
            help_scroll: 0,
            hit: HitTargets::default(),
        }
    }
}

impl DebugUiState {
    pub fn new() -> Self {
        Self::default()
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
