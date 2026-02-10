use crate::command::{
    self, CommandModule, CustomCommandHandler, ExploreCommand, PaletteActionKind, ThemeCommand,
    TraceCommand,
};
use crate::data::{DataSource, DocumentAnchor, OperationAvailability, ViewId};
use crate::helpers::fuzzy_score;
use crate::render;
use crate::session;
use crate::state::{
    CommandState, DiffState, HitTestState, LayoutState, ModeId, ModeLayoutProfile, ModeSpec,
    NavigationState, OverlayState, PaletteState, PaneFocus, RailMode, SearchState,
};
use crate::terminal::{Event, Frame, Rect, RuntimeAction, RuntimeEvent, TerminalApplication};
use crate::theme::Theme;
use crate::views::{self, ViewOpenBehavior, ViewSpec};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct CachedDocument {
    pub text: String,
    pub lines: Vec<String>,
    pub anchors: Vec<DocumentAnchor>,
    pub decompile_anchors: Vec<usize>,
    pub decompile_group_spans: Vec<Option<(usize, usize)>>,
}

/// The main TUI application, parameterized over a `DataSource`.
pub struct TuiApp<D: DataSource> {
    pub data: D,
    pub theme: Theme,
    pub nav: NavigationState,
    pub cmd: CommandState,
    pub palette: PaletteState,
    pub search: SearchState,
    pub layout: LayoutState,
    pub overlay: OverlayState,
    pub diff: DiffState,
    pub hit: HitTestState,
    pub status: String,
    pub extra_modules: Vec<CommandModule>,
    document_cache: HashMap<(ViewId, u64), Arc<CachedDocument>>,
    /// Callback for handling `Custom` commands the TUI doesn't understand.
    /// Receives `(command_name, args)`, returns an optional status message.
    pub custom_handler: Option<CustomCommandHandler>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaletteEntry {
    pub score: usize,
    pub kind_rank: usize,
    pub action_rank: usize,
    pub order: usize,
    pub label: String,
    pub hint: String,
    pub action: PaletteAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaletteAction {
    Execute(String),
    Prefill(String),
}

impl<D: DataSource> TuiApp<D> {
    pub fn new(data: D, start_in_command_mode: bool) -> Self {
        Self::new_with_theme(data, start_in_command_mode, None)
    }

    pub fn new_with_theme(data: D, start_in_command_mode: bool, theme_name: Option<&str>) -> Self {
        let theme = theme_name
            .and_then(|name| Theme::with_palette(data.no_colors(), name).ok())
            .unwrap_or_else(|| Theme::new(data.no_colors()));
        Self::with_theme(data, start_in_command_mode, theme_name, theme)
    }

    pub fn new_with_terminal_theme(
        data: D,
        start_in_command_mode: bool,
        theme_name: Option<&str>,
    ) -> Result<Self, String> {
        let theme = Theme::for_terminal(data.no_colors(), theme_name)?;
        Ok(Self::with_theme(
            data,
            start_in_command_mode,
            theme_name,
            theme,
        ))
    }

    fn with_theme(
        data: D,
        start_in_command_mode: bool,
        theme_name: Option<&str>,
        theme: Theme,
    ) -> Self {
        let available = data.available_views();
        assert!(
            !available.is_empty(),
            "DataSource::available_views() must return at least one view"
        );
        let default_view = data.default_view();
        let view_index = available
            .iter()
            .position(|view| *view == default_view)
            .unwrap_or(0);

        let mut app = Self {
            data,
            theme,
            nav: NavigationState {
                view_index,
                ..Default::default()
            },
            cmd: CommandState {
                active: start_in_command_mode,
                ..Default::default()
            },
            palette: PaletteState::default(),
            search: SearchState::default(),
            layout: LayoutState::default(),
            overlay: OverlayState::default(),
            diff: DiffState::default(),
            hit: HitTestState::default(),
            status: "ready".to_string(),
            extra_modules: Vec::new(),
            document_cache: HashMap::new(),
            custom_handler: None,
        };
        app.restore_session(theme_name.is_some());
        app
    }

    // ── View access ──────────────────────────────────────────────

    /// The view specs provided by the DataSource.
    pub fn specs(&self) -> &[ViewSpec] {
        self.data.view_specs()
    }

    /// The currently active view.
    pub fn current_view(&self) -> ViewId {
        let available = self.data.available_views();
        available
            .get(self.nav.view_index)
            .copied()
            .unwrap_or(available[0])
    }

    /// The current view's spec.
    pub fn current_spec(&self) -> Option<&ViewSpec> {
        views::spec(self.specs(), self.current_view())
    }

    /// Whether the current view is a list view.
    pub fn is_current_list_view(&self) -> bool {
        views::is_list_view(self.specs(), self.current_view())
    }

    pub(crate) fn cached_document(&mut self, view: ViewId) -> Result<Arc<CachedDocument>, String> {
        let revision = self.data.document_revision(view);
        if let Some(document) = self.document_cache.get(&(view, revision)) {
            return Ok(Arc::clone(document));
        }

        let text = self.data.document_text(view)?;
        let lines = text.lines().map(str::to_string).collect();
        let anchors = self.data.document_anchors(view).unwrap_or_default();
        let is_decompile = self.data.decompile_view() == Some(view);
        let decompile_anchors = if is_decompile {
            self.data.decompile_anchors().unwrap_or_default()
        } else {
            Vec::new()
        };
        let decompile_group_spans = if is_decompile {
            self.data.decompile_group_spans().unwrap_or_default()
        } else {
            Vec::new()
        };
        let document = Arc::new(CachedDocument {
            text,
            lines,
            anchors,
            decompile_anchors,
            decompile_group_spans,
        });
        self.document_cache
            .retain(|(cached_view, _), _| *cached_view != view);
        self.document_cache
            .insert((view, revision), Arc::clone(&document));
        Ok(document)
    }

    /// Switch to a view by its ViewId. Returns true if the switch happened.
    pub fn switch_to_view(&mut self, target: ViewId) -> bool {
        let available = self.data.available_views();
        if let Some(idx) = available.iter().position(|v| *v == target) {
            self.nav.view_index = idx;

            if let Some(spec) = views::spec(self.specs(), target) {
                match spec.open_behavior {
                    ViewOpenBehavior::SummaryModal => {
                        self.overlay.show_summary = true;
                    }
                    ViewOpenBehavior::ReplaceMain => {}
                }
            }
            true
        } else {
            false
        }
    }

    // ── Command execution ────────────────────────────────────────

    pub fn execute_command(&mut self, line: &str) {
        let cmd = command::parse_command_with(line, &self.extra_modules, self.data.view_specs());
        self.handle_parsed_command(cmd);
    }

    pub(crate) fn palette_entries(&self) -> Vec<PaletteEntry> {
        let query = self.palette.query.to_ascii_lowercase();
        let available = self.data.available_views();
        let mut entries: Vec<PaletteEntry> = Vec::new();
        let mut order = 0usize;

        for spec in self.mode_specs() {
            let label = format!("Open {} mode", spec.title);
            if let Some(score) = fuzzy_score(&label, &query) {
                let hint = spec
                    .keybind
                    .map(|key| format!("[{key}]"))
                    .unwrap_or_default();
                entries.push(PaletteEntry {
                    score,
                    kind_rank: 0,
                    action_rank: 0,
                    order,
                    label,
                    hint,
                    action: PaletteAction::Execute(format!("mode {}", spec.token)),
                });
                order += 1;
            }
        }

        for spec in views::palette_specs(self.specs(), available) {
            let label = format!("Open {} view", spec.title);
            if let Some(score) = fuzzy_score(&label, &query) {
                let hint = spec
                    .keybind
                    .map(|key| format!("[{key}]"))
                    .unwrap_or_default();
                entries.push(PaletteEntry {
                    score,
                    kind_rank: 1,
                    action_rank: 0,
                    order,
                    label,
                    hint,
                    action: PaletteAction::Execute(format!("view {}", spec.token)),
                });
                order += 1;
            }
        }

        for module in self
            .extra_modules
            .iter()
            .chain(command::command_modules().iter())
        {
            for action in module.palette_actions {
                if !self.palette_action_is_available(action) {
                    continue;
                }
                if let Some(score) = fuzzy_score(action.label, &query) {
                    entries.push(PaletteEntry {
                        score,
                        kind_rank: 2,
                        action_rank: order,
                        order,
                        label: action.label.to_string(),
                        hint: String::new(),
                        action: match action.action {
                            PaletteActionKind::Execute(command) => {
                                PaletteAction::Execute(command.to_string())
                            }
                            PaletteActionKind::Prefill(prefix) => {
                                PaletteAction::Prefill(prefix.to_string())
                            }
                        },
                    });
                    order += 1;
                }
            }
        }

        entries.sort_by_key(|entry| (entry.score, entry.kind_rank, entry.action_rank, entry.order));
        entries
    }

    fn palette_action_is_available(&self, action: &command::PaletteActionSpec) -> bool {
        let command = match action.action {
            PaletteActionKind::Execute(command) | PaletteActionKind::Prefill(command) => command,
        };
        if command.starts_with("export ") {
            return matches!(
                self.data.export_availability(),
                OperationAvailability::Available
            );
        }
        if command.starts_with("trace") {
            return self.data.trace_len() > 0;
        }
        if command.starts_with("showdecomp") {
            return self.data.command_group_count() > 0;
        }
        true
    }

    pub(crate) fn selected_palette_action(&self) -> Option<PaletteAction> {
        let entries = self.palette_entries();
        if entries.is_empty() {
            return None;
        }
        let index = self.palette.index.min(entries.len().saturating_sub(1));
        Some(entries[index].action.clone())
    }

    fn handle_parsed_command(&mut self, cmd: ExploreCommand) {
        match cmd {
            ExploreCommand::Help => {
                self.overlay.show_help = !self.overlay.show_help;
                self.overlay.help_scroll = 0;
            }
            ExploreCommand::Summary => {
                self.overlay.show_summary = !self.overlay.show_summary;
                self.overlay.summary_scroll = 0;
            }
            ExploreCommand::Views => {
                let available = self.data.available_views();
                let specs = self.specs();
                let names: Vec<_> = available.iter().map(|v| views::title(specs, *v)).collect();
                self.cmd.output = Some(format!("views: {}", names.join(", ")));
            }
            ExploreCommand::View(token) => {
                let specs = self.specs();
                if let Some(view_id) =
                    views::parse_token(specs, &token).or_else(|| views::parse_alias(specs, &token))
                {
                    if !self.switch_to_view(view_id) {
                        self.cmd.output =
                            Some(format!("view '{token}' not available for this file"));
                    }
                } else {
                    self.cmd.output = Some(format!("unknown view: {token}"));
                }
            }
            ExploreCommand::Mode(token) => {
                self.switch_mode_token(&token);
            }
            ExploreCommand::Next { count } => {
                self.move_current_view(count.unwrap_or(1), true);
            }
            ExploreCommand::Prev { count } => {
                self.move_current_view(count.unwrap_or(1), false);
            }
            ExploreCommand::Goto { value } => {
                let view = self.current_view();
                if let Some(idx) = self.data.jump_index(view, value) {
                    if views::is_list_view(self.specs(), view) {
                        self.nav.selections.insert(view, idx);
                        self.update_group_focus();
                    } else {
                        self.nav.doc_scrolls.insert(view, idx);
                    }
                } else {
                    self.cmd.output = Some(format!("goto: value {value} not found"));
                }
            }
            ExploreCommand::Show { index } => {
                let view = self.current_view();
                if let Some(row) = index {
                    if row == 0 {
                        self.cmd.output = Some("show expects a one-based row".to_string());
                        return;
                    }
                    let target = row - 1;
                    let len = if views::is_list_view(self.specs(), view) {
                        self.data.list_len(view)
                    } else {
                        match self.cached_document(view) {
                            Ok(document) => document.lines.len(),
                            Err(err) => {
                                self.cmd.output = Some(format!("show error: {err}"));
                                return;
                            }
                        }
                    };
                    if target >= len {
                        self.cmd.output = Some(format!("show: row {row} is outside 1..={len}"));
                        return;
                    }
                    if views::is_list_view(self.specs(), view) {
                        self.nav.selections.insert(view, target);
                        self.update_group_focus();
                    } else {
                        self.nav.doc_scrolls.insert(view, target);
                    }
                }
                self.nav.expanded = true;
                self.nav.detail_scroll = 0;
                self.status = "inspector opened".to_string();
            }
            ExploreCommand::Search { text } => {
                let view = self.current_view();
                let is_list = views::is_list_view(self.specs(), view);
                let hits_result = if is_list {
                    Ok(self.data.search_result(view, &text))
                } else {
                    self.data.document_search_result(view, &text)
                };
                let result = match hits_result {
                    Ok(result) => result,
                    Err(err) => {
                        self.cmd.output = Some(format!("search error: {err}"));
                        self.search = SearchState::default();
                        return;
                    }
                };
                if result.hit_indices.is_empty() {
                    self.cmd.output = Some(format!("no matches for '{text}'"));
                    self.search = SearchState::default();
                } else {
                    let first = result.hit_indices[0];
                    self.search.hits = result.hit_indices;
                    self.search.cursor = 0;
                    self.search.query = Some(text.clone());
                    self.search.view = Some(view);
                    if is_list {
                        self.nav.selections.insert(view, first);
                        self.update_group_focus();
                    } else {
                        self.nav.doc_scrolls.insert(view, first);
                    }
                    self.cmd.output = Some(if result.truncated {
                        format!(
                            "{} match(es) for '{text}'; navigating the first {}",
                            result.total_matches,
                            self.search.hits.len()
                        )
                    } else {
                        format!("{} match(es) for '{text}'", result.total_matches)
                    });
                }
            }
            ExploreCommand::Map { enabled } => {
                if self.data.decompile_view() == Some(self.current_view()) {
                    let new_val = enabled.unwrap_or(!self.layout.decompile_map_visible);
                    self.layout.decompile_map_visible = new_val;
                    self.status = if new_val {
                        "decompile map visible".to_string()
                    } else {
                        "decompile map hidden".to_string()
                    };
                } else {
                    self.status = "map is available only in Decompile".to_string();
                }
            }
            ExploreCommand::ShowDecompile => {
                self.show_decompile_for_current();
            }
            ExploreCommand::Trace(action) => {
                self.command_trace(action);
            }
            ExploreCommand::Diff { path } => {
                self.command_diff(&path);
            }
            ExploreCommand::DiffClose => {
                self.diff.active = false;
                self.diff.target_path = None;
                self.diff.diff_document = None;
                self.diff.change_lines.clear();
                self.diff.right_scroll = 0;
                self.diff.focus_right = false;
            }
            ExploreCommand::Export { path } => match self.data.export_generated_source(&path) {
                Ok(message) => {
                    self.cmd.output = Some(message);
                }
                Err(err) => {
                    self.cmd.output = Some(format!("export error: {err}"));
                }
            },
            ExploreCommand::Theme(action) => {
                self.command_theme(action);
            }
            ExploreCommand::Quit => {
                if self.overlay.quit_confirm {
                    self.overlay.should_quit = true;
                } else {
                    self.overlay.quit_confirm = true;
                }
            }
            ExploreCommand::Custom(cmd_name, args) => {
                if let Some(handler) = &mut self.custom_handler {
                    if let Some(msg) = handler(&cmd_name, &args) {
                        self.cmd.output = Some(msg);
                    }
                } else {
                    self.cmd.output = Some(format!("unknown command: {cmd_name}"));
                }
            }
            ExploreCommand::Unknown(msg) => {
                self.cmd.output = Some(msg);
            }
            ExploreCommand::Noop => {}
        }
    }

    fn update_group_focus(&mut self) {
        let view = self.current_view();
        let sel = self.nav.selection(view);
        self.nav.group_focus = self.data.group_for_index(sel);
        self.nav.trace_index = Some(sel);
    }

    fn move_current_view(&mut self, step: usize, forward: bool) {
        let view = self.current_view();
        if views::is_list_view(self.specs(), view) {
            let max = self.data.list_len(view).saturating_sub(1);
            let selection = self.nav.selection(view);
            let next = if forward {
                selection.saturating_add(step).min(max)
            } else {
                selection.saturating_sub(step)
            };
            self.nav.selections.insert(view, next);
            self.update_group_focus();
            return;
        }

        let max = match self.cached_document(view) {
            Ok(document) => document.lines.len().saturating_sub(1),
            Err(err) => {
                self.cmd.output = Some(format!("navigation error: {err}"));
                return;
            }
        };
        let scroll = self.nav.doc_scroll(view);
        let next = if forward {
            scroll.saturating_add(step).min(max)
        } else {
            scroll.saturating_sub(step)
        };
        self.nav.doc_scrolls.insert(view, next);
    }

    pub fn advance_search(&mut self, direction: i32) {
        let Some(view) = self.search.view else {
            return;
        };
        if self.search.query.is_none() || self.search.hits.is_empty() {
            return;
        }
        if direction >= 0 {
            self.search.cursor = (self.search.cursor + 1) % self.search.hits.len();
        } else {
            self.search.cursor = if self.search.cursor == 0 {
                self.search.hits.len() - 1
            } else {
                self.search.cursor - 1
            };
        }
        let target = self.search.hits[self.search.cursor];
        if views::is_list_view(self.specs(), view) {
            self.nav.selections.insert(view, target);
            self.update_group_focus();
        } else {
            self.nav.doc_scrolls.insert(view, target);
        }
        self.status = format!(
            "match {}/{}",
            self.search.cursor + 1,
            self.search.hits.len()
        );
    }

    // ── Trace ────────────────────────────────────────────────────

    pub fn command_trace(&mut self, action: TraceCommand) {
        let trace_len = self.data.trace_len();
        if trace_len == 0 && !matches!(action, TraceCommand::Off) {
            self.status = "trace unavailable for this file".to_string();
            return;
        }

        match action {
            TraceCommand::Toggle => {
                self.nav.trace_mode = !self.nav.trace_mode;
                self.status = if self.nav.trace_mode {
                    "trace mode enabled".to_string()
                } else {
                    "trace mode disabled".to_string()
                };
            }
            TraceCommand::On => {
                self.nav.trace_mode = true;
                self.status = "trace mode enabled".to_string();
            }
            TraceCommand::Off => {
                self.nav.trace_mode = false;
                self.status = "trace mode disabled".to_string();
            }
            TraceCommand::Next => {
                let view = self.current_view();
                let cur = self
                    .nav
                    .trace_index
                    .unwrap_or_else(|| self.nav.selection(view));
                let next = (cur + 1).min(trace_len.saturating_sub(1));
                self.nav.trace_mode = true;
                self.nav.trace_index = Some(next);
                self.nav.selections.insert(view, next);
                self.update_group_focus();
                self.status = format!("trace frame {next}/{trace_len}");
            }
            TraceCommand::Prev => {
                let view = self.current_view();
                let cur = self
                    .nav
                    .trace_index
                    .unwrap_or_else(|| self.nav.selection(view));
                let next = cur.saturating_sub(1);
                self.nav.trace_mode = true;
                self.nav.trace_index = Some(next);
                self.nav.selections.insert(view, next);
                self.update_group_focus();
                self.status = format!("trace frame {next}/{trace_len}");
            }
            TraceCommand::Goto(idx) => {
                let view = self.current_view();
                let clamped = idx.min(trace_len.saturating_sub(1));
                self.nav.trace_mode = true;
                self.nav.trace_index = Some(clamped);
                self.nav.selections.insert(view, clamped);
                self.update_group_focus();
                self.status = format!("trace frame {clamped}/{trace_len}");
            }
        }
    }

    // ── Group navigation ─────────────────────────────────────────

    pub fn navigate_group(&mut self, direction: i32) {
        let view = self.current_view();
        let sel = self.nav.selection(view);
        let base = self
            .nav
            .group_focus
            .or_else(|| self.data.group_for_index(sel));
        let Some(base_group) = base else {
            self.status = "no command groups available".to_string();
            return;
        };

        let count = self.data.command_group_count();
        if count == 0 {
            return;
        }

        let next_group = if direction > 0 {
            if base_group + 1 < count {
                base_group + 1
            } else {
                self.status = "already at last command group".to_string();
                return;
            }
        } else if base_group > 0 {
            base_group - 1
        } else {
            self.status = "already at first command group".to_string();
            return;
        };

        self.nav.group_focus = Some(next_group);

        // Update instruction selection
        if let Some(inst_idx) = self.data.first_index_for_group(next_group) {
            let available = self.data.available_views();
            for &v in available {
                if views::is_list_view(self.specs(), v) {
                    self.nav.selections.insert(v, inst_idx);
                    break;
                }
            }
            self.nav.trace_index = Some(inst_idx);
        }

        // Scroll decompile to group anchor
        if let Some(decompile_view) = self.data.decompile_view()
            && let Ok(anchors) = self.data.decompile_anchors()
            && let Some(&anchor) = anchors.get(next_group)
            && anchor != usize::MAX
        {
            self.nav
                .doc_scrolls
                .insert(decompile_view, anchor.saturating_sub(2));
        }

        self.status = format!("focused command group c{next_group:03}");
    }

    // ── Reveal decompile ─────────────────────────────────────────

    pub fn show_decompile_for_current(&mut self) {
        let view = self.current_view();
        let sel = self.nav.selection(view);
        if self.data.command_group_count() == 0 {
            self.status = "source map unavailable for this artifact".to_string();
            return;
        }

        let current_list_selection = views::is_list_view(self.specs(), view)
            .then_some(sel)
            .filter(|idx| self.data.group_for_index(*idx).is_some());
        let context_index = current_list_selection
            .or_else(|| {
                self.nav
                    .group_focus
                    .and_then(|group| self.data.first_index_for_group(group))
            })
            .or(self.nav.trace_index)
            .unwrap_or(sel);

        let anchor = match self.data.decompile_anchor_for_index(context_index) {
            Ok(a) => a,
            Err(e) => {
                self.status = format!("reveal error: {e}");
                return;
            }
        };

        let group = self.data.group_for_index(context_index);
        if let Some(g) = group {
            self.nav.group_focus = Some(g);
        }
        self.nav.trace_index = Some(context_index);

        if let (Some(decompile_view), Some(line_idx)) = (self.data.decompile_view(), anchor) {
            self.switch_to_view(decompile_view);
            self.nav
                .doc_scrolls
                .insert(decompile_view, line_idx.saturating_sub(2));
            self.status = format!(
                "revealed decompile for #{context_index}{}",
                group.map(|g| format!(" (c{g:03})")).unwrap_or_default()
            );
        } else if self.data.decompile_view().is_some() {
            self.status = format!("no decompile mapping for #{context_index}");
        } else {
            self.status = "no decompile view available".to_string();
        }
    }

    // ── Diff ─────────────────────────────────────────────────────

    fn command_diff(&mut self, path: &str) {
        match self.data.semantic_diff(path) {
            Ok(report) => {
                self.diff.active = true;
                self.diff.target_path = Some(path.to_string());
                self.diff.diff_document = Some(report.text);
                self.diff.change_lines = report.change_lines;
                self.diff.right_scroll = 0;
                self.diff.focus_right = false;
                self.status = "ready".to_string();
                self.cmd.output = Some(format!("diff opened: {path}"));
            }
            Err(e) => {
                self.cmd.output = Some(format!("diff error: {e}"));
            }
        }
    }

    pub fn navigate_diff_change(&mut self, direction: i32) {
        if self.diff.diff_document.is_none() {
            self.status = "open a semantic diff before navigating changes".to_string();
            return;
        }
        let change_lines = &self.diff.change_lines;
        if change_lines.is_empty() {
            self.status = "diff contains no changes".to_string();
            return;
        }

        let current = self.diff.right_scroll;
        let target = if direction >= 0 {
            change_lines.iter().copied().find(|line| *line > current)
        } else {
            change_lines
                .iter()
                .copied()
                .rev()
                .find(|line| *line < current)
        };
        let Some(target) = target else {
            self.status = if direction >= 0 {
                "already at last diff change".to_string()
            } else {
                "already at first diff change".to_string()
            };
            return;
        };
        self.diff.right_scroll = target;
        self.diff.focus_right = true;
        let position = change_lines
            .iter()
            .position(|line| *line == target)
            .unwrap_or(0);
        self.status = format!("diff change {}/{}", position + 1, change_lines.len());
    }

    fn command_theme(&mut self, action: ThemeCommand) {
        match action {
            ThemeCommand::List => {
                self.cmd.output = Some(format!("themes: {}", Theme::palette_names().join(", ")));
            }
            ThemeCommand::Next => {
                self.theme.next_palette();
                self.status = format!("theme: {}", self.theme.name());
            }
            ThemeCommand::Set(name) => match self.theme.set_palette(&name) {
                Ok(()) => {
                    self.status = format!("theme: {}", self.theme.name());
                }
                Err(err) => {
                    self.cmd.output = Some(err);
                }
            },
        }
    }

    // ── Rail mode ────────────────────────────────────────────────

    pub fn cycle_rail_mode(&mut self) {
        if self.data.decompile_view() != Some(self.current_view()) {
            self.status = "map presentation is available only in Decompile".to_string();
            return;
        }
        self.layout.rail_mode = match self.layout.rail_mode {
            RailMode::Adaptive => RailMode::Anchors,
            RailMode::Anchors => RailMode::Spans,
            RailMode::Spans => RailMode::Adaptive,
        };
        let label = match self.layout.rail_mode {
            RailMode::Adaptive => "adaptive",
            RailMode::Anchors => "anchors",
            RailMode::Spans => "spans",
        };
        self.status = format!("rail mode: {label}");
    }

    pub fn adjust_rail_contrast(&mut self, delta: i8) {
        let new_val = (self.layout.rail_contrast as i8 + delta).clamp(0, 2) as u8;
        self.layout.rail_contrast = new_val;
        self.status = format!("rail contrast: {new_val}");
    }

    // ── Modes ───────────────────────────────────────────────────

    pub fn mode_specs(&self) -> &[ModeSpec] {
        self.data.mode_specs()
    }

    pub fn current_mode_id(&self) -> ModeId {
        self.current_mode_spec().id
    }

    pub fn current_mode_spec(&self) -> ModeSpec {
        let modes = self.data.mode_specs();
        modes
            .get(self.layout.mode_index)
            .copied()
            .unwrap_or(crate::state::DEFAULT_MODE_SPECS[0])
    }

    pub fn current_layout_profile(&self) -> ModeLayoutProfile {
        self.current_mode_spec().layout
    }

    pub fn mode_status_affordances(&self) -> &'static [&'static str] {
        self.current_mode_spec().status_affordances
    }

    pub fn switch_mode(&mut self, target: ModeId) -> bool {
        let Some(index) = self
            .data
            .mode_specs()
            .iter()
            .position(|mode| mode.id == target)
        else {
            self.cmd.output = Some(format!("unknown mode: {}", target.0));
            return false;
        };
        self.layout.mode_index = index;
        self.layout.focused_pane = match self.current_layout_profile() {
            ModeLayoutProfile::Browse => PaneFocus::List,
            ModeLayoutProfile::Compare => PaneFocus::Decompile,
        };
        let spec = self.current_mode_spec();
        self.cmd.output = Some(format!("mode: {}", spec.title));
        true
    }

    pub fn switch_mode_token(&mut self, token: &str) -> bool {
        let normalized = token.trim().to_ascii_lowercase();
        let Some(mode) = self
            .data
            .mode_specs()
            .iter()
            .find(|mode| mode.token == normalized || mode.title.eq_ignore_ascii_case(&normalized))
            .copied()
        else {
            self.cmd.output = Some(format!("unknown mode: {token}"));
            return false;
        };
        self.switch_mode(mode.id)
    }

    // ── Session persistence ──────────────────────────────────────

    fn restore_session(&mut self, preserve_explicit_theme: bool) {
        let available = self.data.available_views().to_vec();
        let specs = self.data.view_specs().to_vec();
        let Some(snapshot) = session::restore(self.data.path(), &specs, &available) else {
            return;
        };

        if let Some(view_id) = snapshot.view_id
            && let Some(idx) = available.iter().position(|v| *v == view_id)
        {
            self.nav.view_index = idx;
        }
        self.layout.decompile_map_visible = snapshot.decompile_map_visible;
        self.nav.trace_mode = snapshot.trace_mode;
        self.layout.focused_pane = snapshot.focused_pane;
        self.layout.rail_mode = snapshot.rail_mode;
        self.layout.rail_contrast = snapshot.rail_contrast;
        self.nav.group_focus = snapshot
            .group_focus
            .filter(|group| *group < self.data.command_group_count());
        self.nav.trace_index = snapshot
            .trace_index
            .filter(|index| *index < self.data.trace_len());

        for view in available {
            if views::is_list_view(&specs, view) {
                let len = self.data.list_len(view);
                let max = len.saturating_sub(1);
                let selection = snapshot
                    .selections
                    .get(&view)
                    .copied()
                    .unwrap_or(0)
                    .min(max);
                let scroll = snapshot
                    .list_scrolls
                    .get(&view)
                    .copied()
                    .unwrap_or(0)
                    .min(max);
                self.nav.selections.insert(view, selection);
                self.nav.list_scrolls.insert(view, scroll);
            } else {
                let max = self
                    .cached_document(view)
                    .map(|document| document.lines.len().saturating_sub(1))
                    .unwrap_or(0);
                let scroll = snapshot
                    .doc_scrolls
                    .get(&view)
                    .copied()
                    .unwrap_or(0)
                    .min(max);
                self.nav.doc_scrolls.insert(view, scroll);
            }
        }

        let current = self.current_view();
        let detail_max = self
            .data
            .list_detail(current, self.nav.selection(current))
            .map(|detail| detail.lines().count().saturating_sub(1))
            .unwrap_or(0);
        self.nav.detail_scroll = snapshot.detail_scroll.min(detail_max);
        if !preserve_explicit_theme && let Some(theme) = snapshot.theme {
            let _ = self.theme.set_palette(&theme);
        }
        if let Some(target) = snapshot.diff_target {
            // Regenerate the diff report from the persisted target path.
            match self.data.semantic_diff(&target) {
                Ok(report) => {
                    self.diff.active = true;
                    self.diff.target_path = Some(target);
                    self.diff.diff_document = Some(report.text);
                    self.diff.change_lines = report.change_lines;
                }
                Err(_) => {
                    // Target file no longer accessible; skip diff restore.
                }
            }
        }
    }

    fn persist_session(&self) {
        let _ = session::persist(
            &self.data,
            &self.nav,
            &self.layout,
            &self.diff,
            self.current_view(),
            self.specs(),
            self.theme.name(),
        );
    }
}

struct ExplorerSurface<D: DataSource> {
    app: TuiApp<D>,
    persisted: bool,
}

impl<D: DataSource> ExplorerSurface<D> {
    fn persist(&mut self) {
        if !self.persisted {
            self.app.persist_session();
            self.persisted = true;
        }
    }
}

impl<D: DataSource> Drop for ExplorerSurface<D> {
    fn drop(&mut self) {
        self.persist();
    }
}

impl<D: DataSource> TerminalApplication for ExplorerSurface<D> {
    type Exit = ();

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        render::draw_in(frame, area, &mut self.app);
    }

    fn handle(&mut self, event: RuntimeEvent) -> RuntimeAction<Self::Exit> {
        let changed = match event {
            RuntimeEvent::Terminal(Event::Key(key)) => {
                crate::input::handle_key(&mut self.app, key);
                true
            }
            RuntimeEvent::Terminal(Event::Mouse(mouse)) => {
                crate::input::handle_mouse(&mut self.app, mouse);
                true
            }
            RuntimeEvent::Terminal(Event::Paste(text)) if self.app.cmd.active => {
                crate::input::insert_command_text(&mut self.app, &text);
                true
            }
            RuntimeEvent::Terminal(Event::Paste(text)) if self.app.palette.active => {
                self.app.palette.query.push_str(&text);
                self.app.palette.index = 0;
                true
            }
            RuntimeEvent::Terminal(Event::Resize(..)) => true,
            _ => false,
        };

        if self.app.overlay.should_quit {
            self.persist();
            RuntimeAction::Exit(())
        } else if changed {
            RuntimeAction::Redraw
        } else {
            RuntimeAction::Ignored
        }
    }
}

/// Run the Explorer through TurtleTap's product-owned terminal runtime.
pub fn run_loop<D: DataSource>(app: TuiApp<D>) -> Result<(), Box<dyn std::error::Error>> {
    crate::terminal::run(
        "explore",
        ExplorerSurface {
            app,
            persisted: false,
        },
    )
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::data::ListEntry;
    use crate::state::{ModeId, ModeLayoutProfile, ModeSpec};
    use crate::views::{ViewOpenBehavior, ViewSpec};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const LIST: ViewId = ViewId("list");
    const DOC: ViewId = ViewId("doc");
    const EXPORT: ViewId = ViewId("export");

    const SPECS: &[ViewSpec] = &[
        ViewSpec {
            view: LIST,
            token: "list",
            title: "List",
            aliases: &["list"],
            keybind: Some('l'),
            icon: "*",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: true,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: DOC,
            token: "doc",
            title: "Doc",
            aliases: &["doc"],
            keybind: Some('d'),
            icon: "#",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: EXPORT,
            token: "export",
            title: "Export",
            aliases: &["export"],
            keybind: None,
            icon: "!",
            show_in_tab_bar: false,
            show_in_palette: false,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
    ];

    #[derive(Clone)]
    struct FakeData {
        exported: Rc<RefCell<Vec<String>>>,
        groups: bool,
        document_revision: Rc<Cell<u64>>,
        document_fetches: Rc<Cell<usize>>,
        rendered_windows: Rc<RefCell<Vec<(usize, usize)>>>,
    }

    impl FakeData {
        fn new(exported: Rc<RefCell<Vec<String>>>, groups: bool) -> Self {
            Self {
                exported,
                groups,
                document_revision: Rc::new(Cell::new(0)),
                document_fetches: Rc::new(Cell::new(0)),
                rendered_windows: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl DataSource for FakeData {
        fn path(&self) -> &str {
            "fake.tcl"
        }

        fn available_views(&self) -> &[ViewId] {
            &[LIST, DOC]
        }

        fn default_view(&self) -> ViewId {
            LIST
        }

        fn no_colors(&self) -> bool {
            false
        }

        fn page_size(&self) -> usize {
            5
        }

        fn summary_text(&self) -> String {
            "summary".to_string()
        }

        fn view_specs(&self) -> &[ViewSpec] {
            SPECS
        }

        fn mode_specs(&self) -> &[ModeSpec] {
            &[
                ModeSpec {
                    id: ModeId("browse"),
                    token: "browse",
                    title: "Browse",
                    icon: "B",
                    keybind: Some('b'),
                    layout: ModeLayoutProfile::Browse,
                    status_affordances: &["Command", "Search", "Trace", "Export"],
                    unavailable_message: None,
                },
                ModeSpec {
                    id: ModeId("compare"),
                    token: "compare",
                    title: "Compare",
                    icon: "C",
                    keybind: Some('C'),
                    layout: ModeLayoutProfile::Compare,
                    status_affordances: &["Diff", "Close", "Next", "Previous"],
                    unavailable_message: Some("comparison requires an active semantic diff"),
                },
            ]
        }

        fn list_len(&self, view: ViewId) -> usize {
            if view == LIST { 3 } else { 0 }
        }

        fn list_entry(&self, view: ViewId, index: usize) -> Option<ListEntry> {
            (view == LIST).then(|| ListEntry {
                text: format!("entry {index} alpha"),
                group: None,
                is_group_start: false,
            })
        }

        fn list_detail(&self, _view: ViewId, index: usize) -> Option<String> {
            Some(format!("detail {index}"))
        }

        fn search_result(&self, view: ViewId, needle: &str) -> crate::data::SearchResult {
            let hit_indices = if view == LIST && needle == "alpha" {
                vec![0, 2]
            } else if view == LIST && needle == "many" {
                vec![0, 1]
            } else {
                Vec::new()
            };
            crate::data::SearchResult {
                total_matches: if needle == "many" {
                    2_000
                } else {
                    hit_indices.len()
                },
                truncated: needle == "many",
                hit_indices,
            }
        }

        fn jump_index(&self, view: ViewId, value: i32) -> Option<usize> {
            (view == LIST && value == 2).then_some(1)
        }

        fn document_revision(&self, _view: ViewId) -> u64 {
            self.document_revision.get()
        }

        fn document_text(&mut self, _view: ViewId) -> Result<String, String> {
            self.document_fetches
                .set(self.document_fetches.get().saturating_add(1));
            Ok("doc alpha\nsecond\nthird alpha\n".to_string())
        }

        fn render_document_window(
            &self,
            _view: ViewId,
            lines: &[String],
            _theme: &Theme,
            start_line: usize,
            end_line: usize,
        ) -> ratatui::text::Text<'static> {
            self.rendered_windows
                .borrow_mut()
                .push((start_line, end_line));
            ratatui::text::Text::from(
                lines[start_line.min(lines.len())..end_line.min(lines.len())].join("\n"),
            )
        }

        fn document_search_result(
            &mut self,
            view: ViewId,
            needle: &str,
        ) -> Result<crate::data::SearchResult, String> {
            let hit_indices = if view == DOC && needle == "alpha" {
                vec![0, 2]
            } else {
                Vec::new()
            };
            Ok(crate::data::SearchResult {
                total_matches: hit_indices.len(),
                truncated: false,
                hit_indices,
            })
        }

        fn command_group_count(&self) -> usize {
            if self.groups { 3 } else { 0 }
        }

        fn group_for_index(&self, index: usize) -> Option<usize> {
            self.groups.then_some(index.min(2))
        }

        fn first_index_for_group(&self, group: usize) -> Option<usize> {
            self.groups.then_some(group)
        }

        fn trace_len(&self) -> usize {
            if self.groups { 3 } else { 0 }
        }

        fn decompile_anchors(&mut self) -> Result<Vec<usize>, String> {
            Ok(vec![0, 8, 16])
        }

        fn decompile_anchor_for_index(&mut self, index: usize) -> Result<Option<usize>, String> {
            Ok(self.groups.then_some(index.saturating_mul(8)))
        }

        fn semantic_diff_report(&mut self, path: &str) -> Result<String, String> {
            Ok(format!("semantic diff\nright: {path}\n"))
        }

        fn semantic_diff(&mut self, path: &str) -> Result<crate::data::SemanticDiff, String> {
            Ok(crate::data::SemanticDiff {
                text: format!("semantic diff\nright: {path}\nchange at arbitrary wording\n"),
                change_lines: vec![2],
            })
        }

        fn decompile_view(&self) -> Option<ViewId> {
            self.groups.then_some(DOC)
        }

        fn export_generated_source(&self, path: &str) -> Result<String, String> {
            self.exported.borrow_mut().push(path.to_string());
            Ok(format!("exported generated source to {path}"))
        }
    }

    #[test]
    fn unified_tui_command_handling_covers_export_search_goto_and_theme() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(Rc::clone(&exported), true), false);
        app.custom_handler = Some(Box::new(|cmd, args| {
            (cmd == "runtime").then(|| format!("runtime output: {args}"))
        }));
        app.extra_modules = vec![CommandModule {
            id: "runtime-test",
            description: "runtime debug test commands",
            help_lines: &["runtime state            show runtime output"],
            palette_actions: &[],
            parse: |command, rest| {
                (command == "runtime")
                    .then(|| ExploreCommand::Custom("runtime".to_string(), rest.to_string()))
            },
        }];

        app.execute_command("search alpha");
        assert_eq!(app.nav.selection(LIST), 0);
        assert_eq!(app.search.hits, vec![0, 2]);
        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
        );
        assert_eq!(app.nav.selection(LIST), 2);

        app.execute_command("goto 2");
        assert_eq!(app.nav.selection(LIST), 1);
        app.execute_command("goto 99");
        assert!(app.cmd.output.as_ref().unwrap().contains("not found"));

        app.execute_command("doc");
        assert_eq!(app.current_view(), DOC);
        app.execute_command("search alpha");
        assert_eq!(app.nav.doc_scroll(DOC), 0);
        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()),
        );
        assert_eq!(app.nav.doc_scroll(DOC), 2);
        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT),
        );
        assert_eq!(app.nav.doc_scroll(DOC), 0);

        app.execute_command("view export");
        assert!(app.cmd.output.as_ref().unwrap().contains("not available"));

        app.execute_command("export /tmp/generated.tcl");
        assert_eq!(exported.borrow().as_slice(), &["/tmp/generated.tcl"]);
        assert!(
            app.cmd
                .output
                .as_ref()
                .unwrap()
                .contains("exported generated source")
        );

        app.nav.group_focus = Some(1);
        app.show_decompile_for_current();
        assert_eq!(app.current_view(), DOC);
        assert_eq!(app.nav.doc_scroll(DOC), 6);
        assert!(app.status.contains("c001"));

        app.execute_command("diff other.tcl");
        assert!(app.diff.active);
        assert!(
            app.diff
                .diff_document
                .as_ref()
                .unwrap()
                .contains("other.tcl")
        );
        app.execute_command("diff close");
        assert!(!app.diff.active);

        app.execute_command("trace on");
        assert!(app.nav.trace_mode);
        app.execute_command("trace next");
        assert_eq!(app.nav.trace_index, Some(2));

        app.switch_to_view(DOC);
        app.execute_command("map off");
        assert!(!app.layout.decompile_map_visible);
        app.execute_command("map on");
        assert!(app.layout.decompile_map_visible);

        app.execute_command("runtime state");
        assert_eq!(app.cmd.output.as_deref(), Some("runtime output: state"));

        app.execute_command("theme sunset");
        assert_eq!(app.theme.name(), "sunset");
        app.execute_command("theme next");
        assert_ne!(app.theme.name(), "sunset");
        app.execute_command("theme list");
        assert!(app.cmd.output.as_ref().unwrap().contains("obsidian"));
    }

    #[test]
    fn source_mode_reveal_preserves_view_with_unavailable_status() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(exported, false), false);
        app.execute_command("doc");
        assert_eq!(app.current_view(), DOC);
        app.show_decompile_for_current();
        assert_eq!(app.current_view(), DOC);
        assert_eq!(app.status, "source map unavailable for this artifact");
    }

    #[test]
    fn document_cache_tracks_revision_and_styles_only_the_visible_window() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(exported, false), false);
        let revision = Rc::clone(&app.data.document_revision);
        let fetches = Rc::clone(&app.data.document_fetches);
        let rendered_windows = Rc::clone(&app.data.rendered_windows);
        app.document_cache.clear();
        fetches.set(0);

        let first = app.cached_document(DOC).expect("first document");
        let second = app.cached_document(DOC).expect("cached document");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(fetches.get(), 1);

        revision.set(1);
        let revised = app.cached_document(DOC).expect("revised document");
        assert!(!Arc::ptr_eq(&first, &revised));
        assert_eq!(fetches.get(), 2);

        app.switch_to_view(DOC);
        app.nav.doc_scrolls.insert(DOC, 1);
        crate::render_frame_to_text(&mut app, 100, 28).expect("document frame");
        assert!(
            rendered_windows.borrow().contains(&(1, 3)),
            "rendered windows: {:?}",
            rendered_windows.borrow().as_slice()
        );
        assert_eq!(fetches.get(), 2, "render should reuse the raw document");
    }

    #[test]
    fn domain_workspace_command_flow() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(Rc::clone(&exported), true), false);

        app.execute_command("search alpha");
        assert_eq!(app.search.query.as_deref(), Some("alpha"));
        assert_eq!(app.search.hits, vec![0, 2]);
        assert_eq!(app.nav.selection(LIST), 0);
        assert_eq!(app.nav.group_focus, Some(0));
        app.advance_search(1);
        assert_eq!(app.nav.selection(LIST), 2);
        assert_eq!(app.nav.group_focus, Some(2));

        app.navigate_group(-1);
        assert_eq!(app.nav.group_focus, Some(1));
        assert_eq!(app.nav.selection(LIST), 1);
        assert_eq!(app.nav.trace_index, Some(1));
        assert!(app.status.contains("c001"));

        app.show_decompile_for_current();
        assert_eq!(app.current_view(), DOC);
        assert_eq!(app.nav.doc_scroll(DOC), 6);
        assert!(app.status.contains("revealed"));

        app.execute_command("search alpha");
        assert_eq!(app.search.hits, vec![0, 2]);
        app.advance_search(1);
        assert_eq!(app.nav.doc_scroll(DOC), 2);

        app.execute_command("view list");
        app.execute_command("goto 2");
        assert_eq!(app.nav.selection(LIST), 1);
        app.execute_command("goto 99");
        assert!(
            app.cmd
                .output
                .as_deref()
                .unwrap_or("")
                .contains("not found")
        );

        app.execute_command("diff other.tcl");
        assert!(app.diff.active);
        app.execute_command("export /tmp/generated-domain-flow.tcl");
        assert_eq!(
            exported.borrow().as_slice(),
            &["/tmp/generated-domain-flow.tcl"]
        );
        assert!(
            app.cmd
                .output
                .as_deref()
                .unwrap_or("")
                .contains("exported generated source")
        );

        app.execute_command("trace on");
        assert!(app.nav.trace_mode);
        app.execute_command("trace prev");
        assert_eq!(app.nav.trace_index, Some(0));

        app.execute_command("view export");
        assert!(
            app.cmd
                .output
                .as_deref()
                .unwrap_or("")
                .contains("not available")
        );
    }

    #[test]
    fn modern_app_interaction_contract() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(exported, true), false);

        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::empty()),
        );
        assert!(app.overlay.show_help);
        crate::input::handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
        assert!(!app.overlay.show_help);

        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        );
        assert!(app.palette.active);
        for ch in "doc".chars() {
            crate::input::handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()),
            );
        }
        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert_eq!(app.current_view(), DOC);
        assert!(!app.palette.active);

        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        );
        for ch in "compare".chars() {
            crate::input::handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()),
            );
        }
        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert_eq!(app.current_mode_id(), ModeId("compare"));
        assert_eq!(app.cmd.output.as_deref(), Some("mode: Compare"));
        assert!(!app.palette.active);

        app.execute_command("theme sunset");
        assert_eq!(app.theme.name(), "sunset");

        app.switch_mode(ModeId("browse"));
        crate::input::handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
        assert_eq!(app.layout.focused_pane, PaneFocus::Decompile);
        let list_selection = app.nav.selection(LIST);
        let group_focus = app.nav.group_focus;
        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::empty()),
        );
        assert!(app.nav.doc_scroll(DOC) > 0);
        assert_eq!(app.nav.selection(LIST), list_selection);
        assert_eq!(app.nav.group_focus, group_focus);

        crate::input::handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
        assert_eq!(app.layout.focused_pane, PaneFocus::Context);
        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::empty()),
        );
        assert_eq!(app.nav.detail_scroll, 1);
        assert_eq!(app.nav.selection(LIST), list_selection);
        assert_eq!(app.nav.group_focus, group_focus);

        app.switch_to_view(LIST);
        app.hit.list_area = Some(ratatui::layout::Rect::new(1, 1, 20, 5));
        app.hit.list_view = Some(LIST);
        app.hit.list_start = 0;
        crate::input::handle_mouse(
            &mut app,
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 2,
                row: 3,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert_eq!(app.nav.selection(LIST), 2);
        assert_eq!(app.layout.focused_pane, PaneFocus::List);

        crate::input::handle_mouse(
            &mut app,
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::ScrollUp,
                column: 2,
                row: 3,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert_eq!(app.nav.selection(LIST), 0);

        app.execute_command("unknown-control");
        assert!(
            app.cmd
                .output
                .as_deref()
                .unwrap_or("")
                .contains("unknown command")
        );
    }

    #[test]
    fn mode_registry_and_switching_contract() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(exported, true), false);

        assert_eq!(app.current_mode_id(), ModeId("browse"));
        assert_eq!(app.current_layout_profile(), ModeLayoutProfile::Browse);
        assert!(app.mode_status_affordances().contains(&"Search"));

        app.execute_command("mode compare");
        assert_eq!(app.current_mode_id(), ModeId("compare"));
        assert_eq!(app.current_layout_profile(), ModeLayoutProfile::Compare);
        assert_eq!(app.cmd.output.as_deref(), Some("mode: Compare"));

        assert!(!app.switch_mode(ModeId("unknown")));
        assert_eq!(app.current_mode_id(), ModeId("compare"));
        assert!(
            app.cmd
                .output
                .as_deref()
                .unwrap_or("")
                .contains("unknown mode")
        );
    }

    #[test]
    fn compare_change_keys_navigate_report_changes() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(exported, true), false);
        app.diff.active = true;
        app.diff.diff_document = Some(
            "Semantic diff\nSummary text can change freely\nfirst arbitrary finding\n  detail\nsecond arbitrary finding\n"
                .to_string(),
        );
        app.diff.change_lines = vec![2, 4];
        app.switch_mode(ModeId("compare"));

        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(']'), KeyModifiers::empty()),
        );
        assert_eq!(app.diff.right_scroll, 2);
        assert!(app.diff.focus_right);
        assert_eq!(app.status, "diff change 1/2");

        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(']'), KeyModifiers::empty()),
        );
        assert_eq!(app.diff.right_scroll, 4);
        assert_eq!(app.status, "diff change 2/2");

        crate::input::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('['), KeyModifiers::empty()),
        );
        assert_eq!(app.diff.right_scroll, 2);
        assert_eq!(app.status, "diff change 1/2");
    }

    #[test]
    fn next_prev_and_show_update_document_state() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(exported, true), false);
        app.switch_to_view(DOC);

        app.execute_command("next 2");
        assert_eq!(app.nav.doc_scroll(DOC), 2);
        app.execute_command("prev");
        assert_eq!(app.nav.doc_scroll(DOC), 1);
        app.execute_command("show 3");
        assert_eq!(app.nav.doc_scroll(DOC), 2);
        assert!(app.nav.expanded);

        app.execute_command("show 99");
        assert!(
            app.cmd
                .output
                .as_deref()
                .unwrap_or("")
                .contains("outside 1..=3")
        );
    }

    #[test]
    fn bounded_search_feedback_reports_total_and_retained_navigation_count() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut app = TuiApp::new(FakeData::new(exported, true), false);

        app.execute_command("search many");

        assert_eq!(app.search.hits, vec![0, 1]);
        assert_eq!(
            app.cmd.output.as_deref(),
            Some("2000 match(es) for 'many'; navigating the first 2")
        );
    }

    #[test]
    fn palette_ranks_exact_actions_and_hides_unavailable_actions() {
        let exported = Rc::new(RefCell::new(Vec::new()));
        let mut source_app = TuiApp::new(FakeData::new(Rc::clone(&exported), false), false);
        source_app.palette.query = "export".to_string();
        assert!(
            source_app
                .palette_entries()
                .iter()
                .all(|entry| entry.label != "Export generated source")
        );
        source_app.palette.query = "trace".to_string();
        assert!(
            source_app
                .palette_entries()
                .iter()
                .all(|entry| !entry.label.to_ascii_lowercase().contains("trace"))
        );

        let mut bytecode_app = TuiApp::new(FakeData::new(exported, true), false);
        bytecode_app.palette.query = "trace".to_string();
        let entries = bytecode_app.palette_entries();
        assert_eq!(
            entries.first().map(|entry| entry.label.as_str()),
            Some("Toggle trace")
        );
    }
}
