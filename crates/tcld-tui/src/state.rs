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

pub(crate) fn format_affordances(affordances: &[&str]) -> String {
    let mut formatted = Vec::new();
    let mut index = 0;
    while index < affordances.len() {
        if affordances[index] == "Command"
            && affordances
                .get(index + 1)
                .is_some_and(|next| *next == "Palette")
        {
            formatted.push("Command Palette".to_string());
            index += 2;
        } else {
            formatted.push(display_affordance_label(affordances[index]).to_string());
            index += 1;
        }
    }
    formatted.join(" | ")
}

fn display_affordance_label(affordance: &str) -> &str {
    match affordance {
        "Diff" => "Open",
        "Diff Close" | "Close" => "Close",
        "Next" => "Next change",
        "Previous" => "Previous change",
        other => other,
    }
}

/// Cursor position and scroll offsets per view.
#[derive(Debug, Default)]
pub struct NavigationState {
    /// Index into the available views list.
    pub view_index: usize,
    /// Per-view cursor position (for list views).
    pub selections: HashMap<ViewId, usize>,
    /// Per-view scroll offset (for list views).
    pub list_scrolls: HashMap<ViewId, usize>,
    /// Per-view scroll offset (for document views).
    pub doc_scrolls: HashMap<ViewId, usize>,
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
    /// The list panel (instructions, literals, etc.).
    List,
    /// The decompile panel (center pane).
    Decompile,
    /// The detail/context panel (right pane).
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
            focused_pane: PaneFocus::List,
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
            last_frame_area: Rect::new(0, 0, 0, 0),
        }
    }
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
