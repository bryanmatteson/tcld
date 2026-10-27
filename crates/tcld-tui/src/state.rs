use crate::data::ViewId;
use ratatui::layout::Rect;
use std::collections::HashMap;

/// Identifies an application mode by an opaque static token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModeId(pub &'static str);

/// Generic layout families the shell knows how to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeLayoutProfile {
    Browse,
    Compare,
}

/// A generic mode registration provided by a `DataSource`.
#[derive(Debug, Clone, Copy)]
pub struct ModeSpec {
    pub id: ModeId,
    pub token: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub keybind: Option<char>,
    pub layout: ModeLayoutProfile,
    pub status_affordances: &'static [&'static str],
    pub unavailable_message: Option<&'static str>,
}

pub const MODE_BROWSE: ModeId = ModeId("browse");
pub const MODE_COMPARE: ModeId = ModeId("compare");

pub const DEFAULT_MODE_SPECS: &[ModeSpec] = &[
    ModeSpec {
        id: MODE_BROWSE,
        token: "browse",
        title: "Browse",
        icon: "B",
        keybind: Some('b'),
        layout: ModeLayoutProfile::Browse,
        status_affordances: &["Command", "Search", "Trace", "Export"],
        unavailable_message: None,
    },
    ModeSpec {
        id: MODE_COMPARE,
        token: "compare",
        title: "Compare",
        icon: "C",
        keybind: Some('C'),
        layout: ModeLayoutProfile::Compare,
        status_affordances: &["Diff", "Close", "Next", "Previous", "Status"],
        unavailable_message: Some("comparison requires an active diff"),
    },
];

/// Cursor position and scroll offsets per view.
#[derive(Debug, Default)]
pub struct NavigationState {
    /// Index into the available views list.
    pub view_index: usize,
    /// Per-view cursor position (for list views).
    pub selections: HashMap<ViewId, usize>,
    /// Per-view scroll offset (for list views).
    pub list_scrolls: HashMap<ViewId, usize>,
    /// Per-view scroll offset (for document views): the first visible line.
    pub doc_scrolls: HashMap<ViewId, usize>,
    /// Per-view cursor line (for document views). The inspector, the outline
    /// and links describe this line; the renderer scrolls to keep it visible.
    pub doc_cursors: HashMap<ViewId, usize>,
    /// Where to return to: positions left by following links and jumping.
    pub back_stack: Vec<NavPoint>,
    /// Scroll offset for the detail/context panel.
    pub detail_scroll: usize,
    /// Whether the detail panel is expanded.
    pub expanded: bool,
    /// Currently focused command group.
    pub group_focus: Option<usize>,
    /// Trace mode: whether bytecode execution trace is active.
    pub trace_mode: bool,
    /// Currently focused trace frame index.
    pub trace_index: Option<usize>,
}

impl NavigationState {
    /// Get the cursor position for the given view, defaulting to 0.
    pub fn selection(&self, view: ViewId) -> usize {
        self.selections.get(&view).copied().unwrap_or(0)
    }

    /// Get the list scroll offset for the given view, defaulting to 0.
    pub fn list_scroll(&self, view: ViewId) -> usize {
        self.list_scrolls.get(&view).copied().unwrap_or(0)
    }

    /// Get the document scroll offset for the given view, defaulting to 0.
    pub fn doc_scroll(&self, view: ViewId) -> usize {
        self.doc_scrolls.get(&view).copied().unwrap_or(0)
    }

    /// Get the document cursor line for the given view, defaulting to 0.
    pub fn doc_cursor(&self, view: ViewId) -> usize {
        self.doc_cursors.get(&view).copied().unwrap_or(0)
    }
}

/// A position to return to with Back: a view and the entry or line in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavPoint {
    pub view: ViewId,
    pub position: usize,
}

/// Command line state.
#[derive(Debug, Default)]
pub struct CommandState {
    /// Whether the command input line is active.
    pub active: bool,
    /// Current input text.
    pub input: String,
    /// Byte offset of the editing cursor in `input`.
    pub cursor: usize,
    /// Last command output message.
    pub output: Option<String>,
    /// Command history.
    pub history: Vec<String>,
    /// Current position in history (None = new input).
    pub history_index: Option<usize>,
}

/// Command palette (fuzzy search overlay) state.
#[derive(Debug, Default)]
pub struct PaletteState {
    /// Whether the palette is open.
    pub active: bool,
    /// Current search query.
    pub query: String,
    /// Selected index in the filtered results.
    pub index: usize,
}

/// Search state.
#[derive(Debug, Default)]
pub struct SearchState {
    /// Current search query.
    pub query: Option<String>,
    /// The view being searched.
    pub view: Option<ViewId>,
    /// Matching indices.
    pub hits: Vec<usize>,
    /// Current position in the hit list.
    pub cursor: usize,
}

/// Which pane has focus in multi-view layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    /// The navigation rail: the view list and, for documents, their outline.
    List,
    /// The canvas (center pane) showing the current view.
    Decompile,
    /// The inspector (right pane).
    Context,
}

/// Decompile rail rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailMode {
    /// Automatically choose between anchors and spans.
    Adaptive,
    /// Show group anchor lines only.
    Anchors,
    /// Show group span ranges.
    Spans,
}

/// Layout and focus state.
#[derive(Debug)]
pub struct LayoutState {
    /// Whether the Decompile document map is visible.
    pub decompile_map_visible: bool,
    /// Which pane has focus.
    pub focused_pane: PaneFocus,
    /// Decompile rail rendering mode.
    pub rail_mode: RailMode,
    /// Rail contrast level (0-3).
    pub rail_contrast: u8,
    /// Index into the DataSource's registered modes.
    pub mode_index: usize,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            decompile_map_visible: true,
            focused_pane: PaneFocus::Decompile,
            rail_mode: RailMode::Adaptive,
            rail_contrast: 1,
            mode_index: 0,
        }
    }
}

/// Modal and overlay state.
#[derive(Debug, Default)]
pub struct OverlayState {
    /// Show the help keybind reference.
    pub show_help: bool,
    /// Scroll offset in the help overlay.
    pub help_scroll: usize,
    /// Show the summary modal.
    pub show_summary: bool,
    /// Scroll offset in the summary modal.
    pub summary_scroll: usize,
    /// Show the quit confirmation prompt.
    pub quit_confirm: bool,
    /// Whether the app should exit.
    pub should_quit: bool,
}

/// Diff mode state.
#[derive(Debug, Default)]
pub struct DiffState {
    /// Whether diff mode is active.
    pub active: bool,
    /// Path to the diff target file.
    pub target_path: Option<String>,
    /// Cached diff report/document text for the right panel.
    pub diff_document: Option<String>,
    /// Domain-provided line offsets for navigable changes.
    pub change_lines: Vec<usize>,
    /// Scroll offset for the right-side diff panel.
    pub right_scroll: usize,
    /// Whether keyboard scrolling is routed to the right-side diff document.
    pub focus_right: bool,
}

/// Hit-testing areas populated during rendering, consumed during input handling.
#[derive(Debug)]
pub struct HitTestState {
    pub mode_switches: Vec<(ModeId, Rect)>,
    pub regions: HashMap<HitRegion, Rect>,
    pub list_area: Option<Rect>,
    pub list_view: Option<ViewId>,
    pub list_start: usize,
    pub list_targets: Vec<ListHitTarget>,
    pub decompile_text_area: Option<Rect>,
    pub decompile_rail_area: Option<Rect>,
    pub decompile_rail_groups: Vec<usize>,
    /// Rows of the navigation rail: views to open and outline lines to jump to.
    pub rail_targets: Vec<(Rect, RailTarget)>,
    /// The document text area, its view, and the line at its top row.
    pub document_target: Option<(Rect, ViewId, usize)>,
    pub last_frame_area: Rect,
}

impl Default for HitTestState {
    fn default() -> Self {
        Self {
            mode_switches: Vec::new(),
            regions: HashMap::new(),
            list_area: None,
            list_view: None,
            list_start: 0,
            list_targets: Vec::new(),
            decompile_text_area: None,
            decompile_rail_area: None,
            decompile_rail_groups: Vec::new(),
            rail_targets: Vec::new(),
            document_target: None,
            last_frame_area: Rect::new(0, 0, 0, 0),
        }
    }
}

/// What a click on a rail row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailTarget {
    View(ViewId),
    OutlineLine(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListHitTarget {
    pub area: Rect,
    pub view: ViewId,
    pub start: usize,
    pub end: usize,
}

/// Coarse hit regions populated by the active mode renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HitRegion {
    ModeSwitch,
    BrowseRail,
    Canvas,
    Minimap,
    Inspector,
    CompareLeft,
    CompareRight,
    FooterCommand,
    FooterStatus,
}
