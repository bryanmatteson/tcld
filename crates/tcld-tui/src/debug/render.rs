//! Frame-level renderer for the debugger TUI.
//!
//! All chrome (titlebar, panels, hint line, help overlay, error
//! banner) flows through [`crate::Theme`] so the debugger inherits
//! the explorer's palette switching for free. The renderer fills
//! [`HitTargets`] each frame so mouse events can be resolved without
//! re-running layout in the driver.
//!
//! The source pane is the point of the screen, so it gets the width and the
//! height; the inspector panes on the right are sized to what they hold, and
//! the console along the bottom carries program output and debugger messages
//! in the order they happened.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::Theme;
use crate::chrome;
use crate::tokens;

use super::state::{DebugUiState, HitTargets, Pane, PaneHit, PromptInput, ValuePopup};
use super::types::{
    AppState, DebugSnapshot, EventLine, EventSeverity, PauseKind, PauseReasonLabel, SourceLine,
    SourceOrigin, StackFrame,
};

pub const MIN_DEBUG_WIDTH: u16 = 80;
pub const MIN_DEBUG_HEIGHT: u16 = 24;

/// Below this width the inspector panes fold into one tabbed pane under the
/// source instead of a column beside it.
const NARROW_WIDTH: u16 = 100;

/// Cells taken by the source gutter: ` ▶●` pointer and breakpoint marker,
/// a right-aligned line number, and a separating space.
const SOURCE_GUTTER_CELLS: u16 = 10;

/// Render one frame of the debugger TUI. Pass an active prompt iff
/// the user is editing a command line; pass `None` for the normal
/// hint-line view.
pub fn render(
    frame: &mut Frame<'_>,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    prompt: Option<&PromptInput>,
    theme: &Theme,
) {
    render_in(frame, frame.area(), snapshot, ui, prompt, theme);
}

/// Render the debugger inside a host-provided surface rectangle.
pub fn render_in(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    prompt: Option<&PromptInput>,
    theme: &Theme,
) {
    ui.hit = HitTargets::default();
    // Single up-front clear: every pane below renders into a chunk of
    // this area, but the chunks change shape (banner appears /
    // disappears, terminal resizes, narrow-mode reflow) and individual
    // widgets only paint the cells they explicitly cover. Without
    // this wipe the previous frame's cells leak through wherever the
    // new content is shorter — exactly what produced the
    // `lpackage` / `tfixture.tbc` / `nub_bootstrap.tcl" line 33` artifacts
    // when a long line was replaced by a short one.
    frame.render_widget(Clear, area);
    let terminal_area = frame.area();
    if terminal_area.width < MIN_DEBUG_WIDTH || terminal_area.height < MIN_DEBUG_HEIGHT {
        let message = format!(
            "tcld debug\n\nTerminal too small: {}x{}\nRequired: {}x{}\n\nCtrl-C exit | q quit",
            terminal_area.width, terminal_area.height, MIN_DEBUG_WIDTH, MIN_DEBUG_HEIGHT
        );
        frame.render_widget(
            Paragraph::new(message)
                .style(theme.detail_value())
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let banner_height = u16::from(snapshot.error_banner.is_some());
    let console_height = if area.height >= 30 { 7 } else { 5 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),              // titlebar
            Constraint::Length(banner_height),  // optional error banner
            Constraint::Min(8),                 // body
            Constraint::Length(console_height), // console
            Constraint::Length(1),              // hint / prompt line
        ])
        .split(area);

    render_titlebar(frame, chunks[0], snapshot, theme);
    if let Some(banner) = snapshot.error_banner {
        render_error_banner(frame, chunks[1], banner, theme);
    }
    render_body(frame, chunks[2], snapshot, ui, theme);
    render_console(frame, chunks[3], snapshot, ui, theme);
    render_hintline(frame, chunks[4], snapshot, ui, prompt, theme);

    if let Some(picker) = snapshot.picker.as_ref() {
        render_picker_overlay(frame, chunks[2], picker, theme);
    }
    if let Some(popup) = ui.popup.as_mut() {
        render_value_popup(frame, area, popup, theme);
    }
    if ui.show_help {
        render_help_overlay(frame, area, theme, &mut ui.help_scroll);
    }
}

// ── Title bar ───────────────────────────────────────────────────────────

fn render_titlebar(frame: &mut Frame<'_>, area: Rect, snapshot: &DebugSnapshot<'_>, theme: &Theme) {
    let pause_kind = snapshot.focused_proc.map(|p| p.pause_kind);
    let crashed = snapshot.app_state == AppState::Dead && pause_kind == Some(PauseKind::Error);
    let state_label = if crashed {
        " Crashed ".to_string()
    } else {
        format!("[{}]", snapshot.app_state.label())
    };
    let state_token = match (snapshot.app_state, pause_kind) {
        (_, _) if crashed => tokens::DEBUG_STATE_ERROR,
        (AppState::Stopped, Some(PauseKind::Error)) => tokens::DEBUG_STATE_ERROR,
        (AppState::Stopped, _) => tokens::COMMAND_PROMPT,
        (AppState::Ready | AppState::Starting | AppState::Loading, _) => tokens::DETAIL_VALUE,
        (AppState::Running, _) => tokens::SUCCESS,
        (AppState::Completed, _) => tokens::DEBUG_STATE_COMPLETED,
        (AppState::Dead | AppState::LoadFailed, _) => tokens::ERROR,
    };
    let target = if snapshot.target_path.is_empty() {
        snapshot.source_path
    } else {
        snapshot.target_path
    };
    let origin = matches!(
        snapshot.source_origin,
        SourceOrigin::CompiledSource | SourceOrigin::Synthetic
    )
    .then(|| format!(" [{}]", snapshot.source_origin.label()));
    let paused = snapshot.app_state == AppState::Stopped || crashed;
    let location = snapshot
        .location_label
        .filter(|label| paused && !label.is_empty())
        .map(|label| format!("  at {label}"));
    let reason = snapshot.focused_proc.filter(|_| paused).map(|focus| {
        match (focus.pause_reason, focus.breakpoint.as_deref()) {
            (_, Some(id)) => format!("breakpoint {id}"),
            (PauseReasonLabel::Entry, _) => "entry".to_string(),
            (PauseReasonLabel::Step, _) => "step".to_string(),
            (PauseReasonLabel::Breakpoint, None) => "breakpoint".to_string(),
            (PauseReasonLabel::Watchpoint, _) => "variable written".to_string(),
            (PauseReasonLabel::Pause, _) => "paused by you".to_string(),
            (PauseReasonLabel::InstructionStep, _) => "instruction step".to_string(),
            (PauseReasonLabel::InstructionBreakpoint, _) => "instruction breakpoint".to_string(),
            (PauseReasonLabel::Error, _) => "error".to_string(),
        }
    });

    let base = || {
        let mut variant = chrome::Variant::new()
            .push(tokens::DEBUG_TITLE, format!(" {} ", snapshot.title))
            .push(tokens::FG, " ")
            .push(tokens::DETAIL_VALUE_ACCENT, file_label(target));
        if let Some(origin) = &origin {
            variant = variant.push(tokens::DETAIL_LABEL, origin.clone());
        }
        variant
            .push(tokens::DETAIL_SEPARATOR, "  ")
            .push(state_token.clone(), state_label.clone())
    };
    let with_location = |variant: chrome::Variant| match &location {
        Some(location) => variant.push(tokens::DETAIL_VALUE, location.clone()),
        None => variant,
    };
    let with_reason = |variant: chrome::Variant| match &reason {
        Some(reason) => variant.push(tokens::DETAIL_SEPARATOR, "  ·  ").push(
            if reason.starts_with("breakpoint") || reason == "error" {
                tokens::ERROR
            } else {
                tokens::DETAIL_LABEL
            },
            reason.clone(),
        ),
        None => variant,
    };
    let with_opcodes = |variant: chrome::Variant| match snapshot.instruction_set {
        Some(set) => variant
            .push(tokens::DETAIL_SEPARATOR, "  ·  ")
            .push(tokens::DETAIL_LABEL, format!("{set} opcodes")),
        None => variant,
    };
    let row = chrome::fit(
        &[
            with_opcodes(with_reason(with_location(base()))),
            with_reason(with_location(base())),
            with_location(base()),
            base(),
        ],
        area.width as usize,
        theme,
    );
    frame.render_widget(Paragraph::new(Line::from(row.spans)), area);
}

fn render_error_banner(frame: &mut Frame<'_>, area: Rect, msg: &str, theme: &Theme) {
    let room = (area.width as usize).saturating_sub(8);
    let line = Line::from(vec![
        Span::styled(" error ", theme.style(&tokens::DEBUG_BANNER_ERROR)),
        Span::raw(" "),
        Span::styled(truncate_cells(&single_line(msg), room), theme.error()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ── Body layout ─────────────────────────────────────────────────────────

fn render_body(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    if area.width < NARROW_WIDTH {
        render_narrow_body(frame, area, snapshot, ui, theme);
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);
    render_code_panes(frame, columns[0], snapshot, ui, theme);
    render_side_column(frame, columns[1], snapshot, ui, theme);
}

/// Stack, Locals, Breakpoints, Watches (and Coverage) stacked beside the
/// source. Panes holding a few rows shrink to them; Locals takes the rest.
fn render_side_column(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let mut fixed = vec![
        snapshot.stack.len(),
        snapshot.breakpoints.len(),
        watch_rows(snapshot),
    ];
    if snapshot.coverage_enabled {
        fixed.push(snapshot.coverage.len());
    }
    let (heights, locals_height) = side_heights(area.height, &fixed);
    let mut constraints = vec![
        Constraint::Length(heights[0]),
        Constraint::Length(locals_height),
        Constraint::Length(heights[1]),
        Constraint::Length(heights[2]),
    ];
    if snapshot.coverage_enabled {
        constraints.push(Constraint::Length(heights[3]));
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_stack(frame, rows[0], snapshot, ui, theme, None);
    render_locals(frame, rows[1], snapshot, ui, theme, None);
    render_breakpoints(frame, rows[2], snapshot, ui, theme, None);
    render_watches(frame, rows[3], snapshot, ui, theme, None);
    if snapshot.coverage_enabled {
        render_coverage(frame, rows[4], snapshot, ui, theme, None);
    }
}

/// Heights for content-sized panes plus what remains for the flexible one.
///
/// Each content-sized pane wants its rows plus a border, is never shorter
/// than an empty pane, and never takes more than a quarter of the column.
/// When even that leaves the flexible pane too little room, the tallest
/// panes give rows back first.
fn side_heights(total: u16, rows: &[usize]) -> (Vec<u16>, u16) {
    const EMPTY_PANE: u16 = 3;
    const MIN_FLEX: u16 = 5;
    let cap = (total / 4).max(EMPTY_PANE);
    let mut heights: Vec<u16> = rows
        .iter()
        .map(|rows| (u16::try_from(*rows).unwrap_or(u16::MAX).max(1) + 2).min(cap))
        .collect();
    while heights.iter().sum::<u16>() + MIN_FLEX > total {
        let Some(tallest) = heights
            .iter_mut()
            .filter(|height| **height > EMPTY_PANE)
            .max()
        else {
            break;
        };
        *tallest -= 1;
    }
    let flex = total.saturating_sub(heights.iter().sum());
    (heights, flex)
}

fn watch_rows(snapshot: &DebugSnapshot<'_>) -> usize {
    snapshot.watches.len()
}

/// Narrow terminals fold the inspector panes into one tabbed pane below the
/// source, so the source keeps its full width.
fn render_narrow_body(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let inspector_height = (area.height * 2 / 5)
        .clamp(6, 14)
        .min(area.height.saturating_sub(6));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(inspector_height)])
        .split(area);
    render_code_panes(frame, rows[0], snapshot, ui, theme);

    let mut tabs = vec![Pane::Stack, Pane::Locals, Pane::Breakpoints, Pane::Watches];
    if snapshot.coverage_enabled {
        tabs.push(Pane::Coverage);
    }
    let shown = if ui.focus.is_inspector() {
        ui.focus
    } else {
        ui.inspector_tab
    };
    let shown = if tabs.contains(&shown) {
        shown
    } else {
        Pane::Locals
    };
    let tabs = Some(tabs.as_slice());
    match shown {
        Pane::Stack => render_stack(frame, rows[1], snapshot, ui, theme, tabs),
        Pane::Breakpoints => render_breakpoints(frame, rows[1], snapshot, ui, theme, tabs),
        Pane::Watches => render_watches(frame, rows[1], snapshot, ui, theme, tabs),
        Pane::Coverage => render_coverage(frame, rows[1], snapshot, ui, theme, tabs),
        _ => render_locals(frame, rows[1], snapshot, ui, theme, tabs),
    }
}

/// The block for an inspector pane. With `tabs`, the title is the narrow
/// layout's tab strip and each label is registered as a click target.
fn side_block<'a>(
    pane: Pane,
    title: String,
    ui: &mut DebugUiState,
    area: Rect,
    theme: &'a Theme,
    tabs: Option<&[Pane]>,
) -> Block<'a> {
    let focused = ui.focus == pane;
    let Some(tabs) = tabs else {
        return panel_block(&title, focused, theme);
    };
    let mut spans = Vec::new();
    let mut column = area.x + 1;
    for (index, tab) in tabs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("│", theme.border_dim()));
            column += 1;
        }
        let label = if *tab == pane {
            format!(" {title} ")
        } else {
            format!(" {} ", tab.title())
        };
        let width = cells(&label) as u16;
        ui.hit.inspector_tabs.push(PaneHit {
            pane: *tab,
            area: Rect::new(column, area.y, width, 1),
        });
        column += width;
        spans.push(Span::styled(
            label,
            if *tab == pane {
                if focused {
                    theme.panel_title_focused()
                } else {
                    theme.tab_active()
                }
            } else {
                theme.tab_inactive()
            },
        ));
    }
    base_block(focused, theme).title(Line::from(spans))
}

// ── Source ──────────────────────────────────────────────────────────────

fn render_code_panes(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    if snapshot.secondary_source_lines.is_empty() {
        render_source(frame, area, snapshot, ui, theme);
        return;
    }
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
    render_source(frame, panes[0], snapshot, ui, theme);
    let title = if matches!(
        snapshot.source_origin,
        SourceOrigin::UserScript | SourceOrigin::CompiledSource
    ) {
        " Disassembly · executable "
    } else {
        " Source · companion (unverified) "
    };
    let block = panel_block(title, false, theme);
    let inner = block.inner(panes[1]);
    let highlight = snapshot
        .secondary_highlight_line
        .and_then(|line| usize::try_from(line).ok())
        .filter(|&line| line > 0);
    let height = inner.height.max(1) as usize;
    let top = highlight
        .map(|line| line.saturating_sub(1).saturating_sub(height / 2))
        .unwrap_or(0)
        .min(snapshot.secondary_source_lines.len().saturating_sub(height));
    let width = inner.width as usize;
    let lines: Vec<Line<'_>> = snapshot
        .secondary_source_lines
        .iter()
        .enumerate()
        .skip(top)
        .take(height)
        .map(|(index, line)| {
            let line_no = index + 1;
            let active = highlight == Some(line_no);
            let gutter = if active {
                format!("▶ {line_no:5} ")
            } else {
                format!("  {line_no:5} ")
            };
            let text_style = if active {
                theme.focused_line_bg()
            } else {
                theme.style(&tokens::PLAIN)
            };
            let room = width.saturating_sub(cells(&gutter));
            Line::from(vec![
                Span::styled(
                    gutter,
                    if active {
                        theme.gutter_focused()
                    } else {
                        theme.gutter_default()
                    },
                ),
                Span::styled(truncate_cells(&sanitize(&line.raw), room), text_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), panes[1]);
}

fn source_title(snapshot: &DebugSnapshot<'_>, ui: &DebugUiState) -> String {
    let label = if snapshot.source_label.is_empty() {
        file_label(snapshot.source_path)
    } else {
        snapshot.source_label.to_string()
    };
    let qualifier = match snapshot.source_origin {
        SourceOrigin::UserScript => None,
        SourceOrigin::CompiledSource => Some("compiled source"),
        SourceOrigin::SourcedFile => Some("sourced file"),
        SourceOrigin::ProcBody => Some("proc body"),
        SourceOrigin::Anonymous => Some("eval, session-only"),
        SourceOrigin::Synthetic => Some("synthetic"),
    };
    let mut title = match qualifier {
        Some(qualifier) => format!(" Source · {label} · {qualifier}"),
        None => format!(" Source · {label}"),
    };
    if !snapshot.source_search.query.is_empty() {
        let total = snapshot.source_search.match_lines.len();
        let current = snapshot.source_search.current.map_or(0, |c| c + 1);
        title.push_str(&format!(
            " · /{}  {current}/{total}",
            snapshot.source_search.query
        ));
    } else if let Some(line) = snapshot.source_lines.get(ui.source_cursor)
        && line.commands.len() > 1
    {
        let index = ui
            .source_command_cursor
            .min(line.commands.len().saturating_sub(1));
        title.push_str(&format!(
            " · command {}/{}: {}",
            index + 1,
            line.commands.len(),
            line.commands[index].label
        ));
    }
    title.push(' ');
    title
}

fn render_source(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let focused = ui.focus == Pane::Source;
    let title = source_title(snapshot, ui);
    let block = panel_block(&title, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    ui.hit.panes.push(PaneHit {
        pane: Pane::Source,
        area,
    });
    ui.hit.source_inner = Some(inner);
    ui.hit.source_gutter_width = SOURCE_GUTTER_CELLS;

    let height = inner.height.max(1) as usize;
    let half = height / 2;
    if ui.center_source_cursor {
        let max_top = snapshot.source_lines.len().saturating_sub(height);
        ui.source_view_top = ui.source_cursor.saturating_sub(half).min(max_top);
        ui.center_source_cursor = false;
    } else if ui.source_cursor < ui.source_view_top {
        ui.source_view_top = ui.source_cursor;
    } else if ui.source_cursor >= ui.source_view_top + height {
        ui.source_view_top = ui.source_cursor + 1 - height;
    } else if ui.source_cursor.saturating_sub(half) > ui.source_view_top {
        ui.source_view_top = ui.source_cursor.saturating_sub(half);
    }
    ui.hit.source_view_top = ui.source_view_top;

    let paused_line = snapshot.paused_line.map_or(0, |line| line as usize);
    let active_qname = snapshot
        .source_qname
        .or_else(|| snapshot.focused_proc.map(|proc| proc.qname.as_str()))
        .unwrap_or("");
    let active_breakpoints: Vec<&super::types::ProcBreakpoint> = snapshot
        .breakpoints
        .iter()
        .filter(|bp| bp.kind == super::types::BreakpointKindLabel::Line)
        .collect();
    let bp_lines: std::collections::HashSet<u32> = active_breakpoints
        .iter()
        .filter(|bp| match snapshot.source_origin {
            SourceOrigin::ProcBody => bp.qname == active_qname,
            SourceOrigin::UserScript | SourceOrigin::CompiledSource | SourceOrigin::SourcedFile => {
                bp.qname.is_empty() && bp.source_path.is_some()
            }
            SourceOrigin::Synthetic => false,
            SourceOrigin::Anonymous => false,
        })
        .filter_map(|bp| bp.line)
        .collect();
    let command_has_breakpoint = |command: &super::types::SourceCommand| {
        active_breakpoints.iter().any(|bp| {
            bp.offset == command.target_offset
                && if let Some(qname) = command.qname.as_deref() {
                    bp.qname == qname
                } else {
                    match snapshot.source_origin {
                        SourceOrigin::ProcBody => bp.qname == active_qname,
                        SourceOrigin::UserScript
                        | SourceOrigin::CompiledSource
                        | SourceOrigin::SourcedFile => {
                            bp.qname.is_empty() && bp.source_path.is_some()
                        }
                        SourceOrigin::Synthetic => false,
                        SourceOrigin::Anonymous => false,
                    }
                }
        })
    };

    let gutter_default = theme.gutter_default();
    let gutter_focused = theme.gutter_focused();
    let bp_style = theme.style(&tokens::DEBUG_BREAKPOINT);
    let cursor_bg = theme.selection();
    let paused_bg = theme.focused_line_bg();
    let code_width = (inner.width as usize).saturating_sub(SOURCE_GUTTER_CELLS as usize);

    let mut lines = Vec::with_capacity(height);
    for row in 0..height {
        let idx = ui.source_view_top + row;
        if idx >= snapshot.source_lines.len() {
            break;
        }
        let line_no = idx + 1;
        let is_paused = paused_line != 0 && line_no == paused_line;
        let is_cursor = idx == ui.source_cursor;
        let source_line: &SourceLine = &snapshot.source_lines[idx];
        let command_bp_count = source_line
            .commands
            .iter()
            .filter(|command| command_has_breakpoint(command))
            .count();
        let has_bp = command_bp_count > 0 || bp_lines.contains(&(line_no as u32));

        let pointer = if is_paused { "▶" } else { " " };
        let bp_marker = match command_bp_count {
            0 if has_bp => "●".to_string(),
            0 => " ".to_string(),
            1 => "●".to_string(),
            count => count.min(9).to_string(),
        };
        let gutter_style = if is_paused {
            gutter_focused
        } else if is_cursor && focused {
            theme.gutter_annotated()
        } else {
            gutter_default
        };
        let row_bg = if is_paused {
            paused_bg
        } else if is_cursor && focused {
            cursor_bg
        } else {
            theme.style(&tokens::PLAIN)
        };
        let bp_render_style = if has_bp {
            bp_style
        } else {
            theme.style(&tokens::PLAIN)
        };

        // Search hits are underlined; the current one also takes the
        // gutter's accent.
        let is_search_match = !snapshot.source_search.query.is_empty()
            && snapshot.source_search.match_lines.contains(&idx);
        let is_current_match = is_search_match
            && snapshot
                .source_search
                .current
                .and_then(|c| snapshot.source_search.match_lines.get(c).copied())
                == Some(idx);

        let mut row_style = row_bg;
        if is_current_match {
            row_style = row_style.patch(theme.gutter_focused());
        } else if is_search_match {
            row_style = row_style.patch(theme.style(&tokens::SEARCH_MATCH));
        }

        let mut spans = Vec::with_capacity(4 + source_line.spans.len());
        spans.push(Span::styled(format!(" {pointer}"), gutter_style));
        spans.push(Span::styled(bp_marker, bp_render_style));
        spans.push(Span::styled(format!(" {line_no:>4} "), gutter_style));
        spans.push(Span::styled(" ", row_style));

        // Control bytes (NUL, BEL, ESC, …) are replaced before rendering:
        // ratatui writes them as zero-width cells, which drifts the rest of
        // the row left and leaves phantom letters behind.
        let selected = is_cursor.then(|| {
            let index = ui
                .source_command_cursor
                .min(source_line.commands.len().saturating_sub(1));
            source_line.commands.get(index)
        });
        let mut code: Vec<Span<'_>> = Vec::new();
        if let Some(Some(command)) = selected {
            let start = char_boundary_at_or_before(&source_line.raw, command.start_column);
            let end = char_boundary_at_or_before(
                &source_line.raw,
                command.end_column.max(command.start_column),
            )
            .max(start);
            let command_style = if command_has_breakpoint(command) {
                theme
                    .style(&tokens::DEBUG_COMMAND_BREAKPOINT)
                    .patch(row_style)
            } else {
                theme
                    .style(&tokens::DEBUG_COMMAND_SELECTED)
                    .patch(row_style)
            };
            code.push(Span::styled(sanitize(&source_line.raw[..start]), row_style));
            code.push(Span::styled(
                sanitize(&source_line.raw[start..end]),
                command_style,
            ));
            code.push(Span::styled(sanitize(&source_line.raw[end..]), row_style));
        } else if source_line.spans.is_empty() {
            code.push(Span::styled(sanitize(&source_line.raw), row_style));
        } else {
            for span in &source_line.spans {
                let merged = if row_style.bg.is_some() || is_search_match {
                    span.style.patch(row_style)
                } else {
                    span.style
                };
                code.push(Span::styled(sanitize(&span.content), merged));
            }
        }
        // Rows never wrap: a wrapped row would shift every line below it
        // away from the line numbers and break click-to-line. Long lines are
        // clipped, scrolled with ←/→, and marked where text continues.
        let clipped = clip_spans(
            code,
            ui.source_hscroll,
            code_width.saturating_sub(1),
            theme.fg_muted().patch(row_style),
        );
        spans.extend(clipped);
        // Pad the row out to inner.width with row-styled spaces so the
        // *cells* themselves get overwritten — defends against any
        // ratatui buffer leak that survives the up-front Clear.
        let used: usize = spans.iter().map(|s| cells(s.content.as_ref())).sum();
        let inner_w = inner.width as usize;
        if used < inner_w {
            spans.push(Span::styled(" ".repeat(inner_w - used), row_style));
        }
        lines.push(Line::from(spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no source available — pass --bootstrap or check the path)",
            theme.empty_state(),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Drop `skip` cells from the left of a styled row and keep at most `width`
/// cells, marking each clipped edge with `…`.
fn clip_spans<'a>(
    spans: Vec<Span<'a>>,
    skip: usize,
    width: usize,
    marker_style: Style,
) -> Vec<Span<'a>> {
    let total: usize = spans.iter().map(|span| cells(span.content.as_ref())).sum();
    if skip == 0 && total <= width {
        return spans;
    }
    let mut out = Vec::new();
    let mut position = 0usize;
    let mut used = 0usize;
    let left_clipped = skip > 0 && total > 0;
    if left_clipped {
        out.push(Span::styled("…", marker_style));
        used += 1;
    }
    let start = if left_clipped { skip + 1 } else { skip };
    let right_clipped = total > skip + width;
    let budget = if right_clipped {
        width.saturating_sub(1)
    } else {
        width
    };
    for span in spans {
        let mut kept = String::new();
        for character in span.content.chars() {
            let wide = char_cells(character);
            if position >= start && used + wide <= budget {
                kept.push(character);
                used += wide;
            }
            position += wide;
        }
        if !kept.is_empty() {
            out.push(Span::styled(kept, span.style));
        }
    }
    if right_clipped {
        out.push(Span::styled("…", marker_style));
    }
    out
}

// ── Inspector panes ─────────────────────────────────────────────────────

/// What a stack frame is called in the Stack pane.
fn frame_name(frame: &StackFrame) -> String {
    if let Some(name) = frame.proc_name.as_deref().filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    if frame.level == 0 || frame.kind == "toplevel" {
        return "(top level)".to_string();
    }
    frame
        .source_path
        .as_deref()
        .map(file_label)
        .unwrap_or_else(|| frame.kind.clone())
}

/// Where a stack frame is, for the Stack pane.
fn frame_location(frame: &StackFrame) -> String {
    if let Some(label) = frame.location_label.as_deref() {
        return label.to_string();
    }
    match (frame.source_path.as_deref(), frame.line) {
        (Some(path), Some(line)) if frame.proc_name.is_none() => {
            format!("{}:{line}", file_label(path))
        }
        (_, Some(line)) => format!("line {line}"),
        _ => String::new(),
    }
}

fn render_stack(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
    tabs: Option<&[Pane]>,
) {
    let block = side_block(Pane::Stack, "Stack".to_string(), ui, area, theme, tabs);
    let inner = block.inner(area);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Stack,
        area,
    });
    ui.hit.stack_inner = Some(inner);
    let width = inner.width as usize;
    let items: Vec<ListItem> = if snapshot.stack.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (no frames — pause execution first)",
            theme.empty_state(),
        ))]
    } else {
        let name_width = snapshot
            .stack
            .iter()
            .map(|frame| cells(&frame_name(frame)))
            .max()
            .unwrap_or(0)
            .min(width / 2);
        snapshot
            .stack
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let marker = if index == 0 { "▶ " } else { "  " };
                let name = truncate_cells(&frame_name(frame), name_width);
                let padding = " ".repeat(name_width.saturating_sub(cells(&name)));
                let location = frame_location(frame);
                let room = width.saturating_sub(2 + name_width + 2);
                ListItem::new(Line::from(vec![
                    Span::styled(marker, theme.gutter_focused()),
                    Span::styled(name, theme.detail_value_accent()),
                    Span::raw(padding),
                    Span::raw("  "),
                    Span::styled(truncate_cells(&location, room), theme.detail_label()),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selection());
    frame.render_stateful_widget(list, area, &mut ui.selections.stack);
}

fn render_locals(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
    tabs: Option<&[Pane]>,
) {
    let scope = snapshot
        .stack
        .iter()
        .find(|frame| frame.level == snapshot.inspected_frame)
        .map(frame_name)
        .unwrap_or_else(|| format!("level {}", snapshot.inspected_frame));
    let title = if snapshot.app_state == AppState::Stopped {
        format!("Locals · {scope}")
    } else {
        "Locals".to_string()
    };
    let block = side_block(Pane::Locals, title, ui, area, theme, tabs);
    let inner = block.inner(area);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Locals,
        area,
    });
    ui.hit.locals_inner = Some(inner);
    let inner_width = inner.width as usize;
    let items: Vec<ListItem> = if snapshot.locals.is_empty() {
        let message = if snapshot.app_state == AppState::Stopped {
            "  (no variables in this frame)"
        } else {
            "  (no locals — pause execution first)"
        };
        vec![ListItem::new(Span::styled(message, theme.empty_state()))]
    } else {
        snapshot
            .locals
            .iter()
            .map(|row| {
                let name_style = if row.builtin {
                    theme.fg_muted()
                } else {
                    theme.variable()
                };
                let (marker, value_style) = if row.changed {
                    (
                        Span::styled("Δ ", theme.style(&tokens::DEBUG_CHANGED)),
                        theme.style(&tokens::DEBUG_CHANGED),
                    )
                } else if row.builtin {
                    (Span::raw("  "), theme.fg_muted())
                } else {
                    (Span::raw("  "), theme.detail_value())
                };
                // "Δ " + name + " = " is the fixed part of the row; the value
                // gets whatever the pane has left rather than a constant.
                let room = inner_width.saturating_sub(cells(&row.name) + 5).max(8);
                ListItem::new(Line::from(vec![
                    marker,
                    Span::styled(row.name.clone(), name_style),
                    Span::styled(" = ", theme.detail_separator()),
                    Span::styled(truncate_cells(&single_line(&row.value), room), value_style),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selection());
    frame.render_stateful_widget(list, area, &mut ui.selections.locals);
}

fn render_breakpoints(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
    tabs: Option<&[Pane]>,
) {
    let block = side_block(
        Pane::Breakpoints,
        "Breakpoints".to_string(),
        ui,
        area,
        theme,
        tabs,
    );
    let inner = block.inner(area);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Breakpoints,
        area,
    });
    ui.hit.breakpoints_inner = Some(inner);
    let width = inner.width as usize;
    let items: Vec<ListItem> = if snapshot.breakpoints.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (none — press b on a source line)",
            theme.empty_state(),
        ))]
    } else {
        snapshot
            .breakpoints
            .iter()
            .map(|bp| {
                let glyph = if bp.enabled { bp.kind.glyph() } else { "○" };
                let glyph_style = if bp.enabled {
                    theme.style(&tokens::DEBUG_BREAKPOINT)
                } else {
                    theme.fg_muted()
                };
                let location = bp.location_label.clone().unwrap_or_else(|| {
                    let name = bp
                        .source_path
                        .as_deref()
                        .map(file_label)
                        .unwrap_or_else(|| bp.qname.clone());
                    format!(
                        "{name}:{}",
                        bp.line
                            .map(|line| line.to_string())
                            .unwrap_or_else(|| bp.offset.to_string())
                    )
                });
                let mut details = Vec::new();
                if bp.pending {
                    details.push("[pending]".to_string());
                }
                if !bp.enabled {
                    details.push("disabled".to_string());
                }
                if let Some(condition) = bp.condition.as_deref() {
                    details.push(format!("if {condition}"));
                }
                if bp.ignore_count > 0 {
                    details.push(format!("skip {}", bp.ignore_count));
                }
                if bp.hits > 0 {
                    details.push(format!("hits {}", bp.hits));
                }
                let details = if details.is_empty() {
                    String::new()
                } else {
                    format!(" {}", details.join(" · "))
                };
                let used = 2 + cells(&bp.id) + 1 + cells(&location);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{glyph} "), glyph_style),
                    Span::styled(bp.id.clone(), theme.fg_muted()),
                    Span::raw(" "),
                    Span::styled(
                        location,
                        if bp.enabled {
                            theme.detail_value()
                        } else {
                            theme.fg_muted()
                        },
                    ),
                    Span::styled(
                        truncate_cells(&details, width.saturating_sub(used)),
                        if bp.condition.is_some() {
                            theme.variable()
                        } else {
                            theme.detail_label()
                        },
                    ),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selection());
    frame.render_stateful_widget(list, area, &mut ui.selections.breakpoints);
}

fn render_watches(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
    tabs: Option<&[Pane]>,
) {
    let block = side_block(Pane::Watches, "Watches".to_string(), ui, area, theme, tabs);
    let inner = block.inner(area);
    let inner_width = inner.width as usize;
    ui.hit.panes.push(PaneHit {
        pane: Pane::Watches,
        area,
    });
    ui.hit.watches_inner = Some(inner);
    let items: Vec<ListItem> = if snapshot.watches.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (none — press w to add)",
            theme.empty_state(),
        ))]
    } else {
        snapshot
            .watches
            .iter()
            .map(|w| {
                let changed_text = if w.changed { "Δ " } else { "  " };
                let changed_style = if w.changed {
                    theme.style(&tokens::DEBUG_CHANGED)
                } else {
                    theme.style(&tokens::PLAIN)
                };
                // The value is the point of a watch; the name and expression
                // give way first. A watch named after its own expression
                // would read `x = $x → 1`, so the expression is said once.
                let name = truncate_cells(&w.name, inner_width / 3);
                let value_room = inner_width
                    .saturating_sub(cells(changed_text) + cells(&name) + 3)
                    .clamp(8, (inner_width * 2 / 3).max(8));
                let (value_text, value_style) = if let Some(err) = &w.last_error {
                    (
                        format!("→ ! {}", truncate_cells(&single_line(err), value_room)),
                        theme.error(),
                    )
                } else if let Some(v) = &w.last_value {
                    (
                        format!("→ {}", truncate_cells(&single_line(v), value_room)),
                        theme.detail_value(),
                    )
                } else {
                    ("→ (not evaluated yet)".to_string(), theme.empty_state())
                };
                let expression_room = inner_width.saturating_sub(
                    cells(changed_text) + cells(&name) + 1 + cells(&value_text) + 3,
                );
                let expression =
                    (w.expr != w.name && w.expr != format!("${}", w.name) && expression_room >= 4)
                        .then(|| format!(" = {}", truncate_cells(&w.expr, expression_room)));
                let mut spans = vec![
                    Span::styled(changed_text, changed_style),
                    Span::styled(name, theme.variable()),
                ];
                if let Some(expression) = expression {
                    spans.push(Span::styled(expression, theme.detail_label()));
                }
                spans.push(Span::raw(" "));
                spans.push(Span::styled(value_text, value_style));
                ListItem::new(Line::from(spans))
            })
            .collect()
    };
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selection());
    frame.render_stateful_widget(list, area, &mut ui.selections.watches);
}

fn render_coverage(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
    tabs: Option<&[Pane]>,
) {
    let block = side_block(
        Pane::Coverage,
        "Coverage".to_string(),
        ui,
        area,
        theme,
        tabs,
    );
    ui.hit.panes.push(PaneHit {
        pane: Pane::Coverage,
        area,
    });
    ui.hit.coverage_inner = Some(block.inner(area));
    let items: Vec<ListItem> = if snapshot.coverage.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (no coverage — press C to enable, then run code)",
            theme.empty_state(),
        ))]
    } else {
        snapshot
            .coverage
            .iter()
            .take(area.height as usize * 4)
            .map(|row| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("hits={:>4}  ", row.hits), theme.success()),
                    Span::styled(
                        format!("{}@{}", row.block, row.offset),
                        theme.detail_value(),
                    ),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selection());
    frame.render_stateful_widget(list, area, &mut ui.selections.coverage);
}

// ── Console ─────────────────────────────────────────────────────────────

/// Console lines worth showing: everything while filtering, otherwise all
/// but debugger bookkeeping.
pub fn visible_console_lines<'a>(events: &'a [EventLine], filter: &str) -> Vec<&'a EventLine> {
    let filter = filter.to_lowercase();
    events
        .iter()
        .filter(|line| {
            if filter.is_empty() {
                line.severity != EventSeverity::Trace
            } else {
                line.text.to_lowercase().contains(&filter)
            }
        })
        .collect()
}

fn render_console(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let lines = visible_console_lines(snapshot.events, snapshot.event_filter);
    let height = area.height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(height);
    ui.console_scroll = ui.console_scroll.min(max_scroll);
    let mut title = if snapshot.event_filter.is_empty() {
        " Console ".to_string()
    } else {
        format!(" Console · /{} ", snapshot.event_filter)
    };
    if ui.console_scroll > 0 {
        title.push_str(&format!("· {} newer below ", ui.console_scroll));
    }
    let focused = ui.focus == Pane::Events;
    let block = panel_block(&title, focused, theme);
    let inner = block.inner(area);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Events,
        area,
    });
    ui.hit.events_inner = Some(inner);
    let width = inner.width as usize;
    let end = lines.len().saturating_sub(ui.console_scroll);
    let start = end.saturating_sub(height);
    let rows: Vec<Line> = if lines.is_empty() {
        vec![Line::from(Span::styled(
            if snapshot.event_filter.is_empty() {
                "  (program output and debugger messages appear here)"
            } else {
                "  (no console lines match the filter — / to change it)"
            },
            theme.empty_state(),
        ))]
    } else {
        // No timestamps: the order is the story, and program output reads
        // like it would in a terminal. Transcripts keep the times.
        lines[start..end]
            .iter()
            .map(|line| {
                let style = match line.severity {
                    EventSeverity::Info => theme.detail_value(),
                    EventSeverity::Pause => theme.command_prompt(),
                    EventSeverity::Warn => theme.variable(),
                    EventSeverity::Error | EventSeverity::ErrorOutput => theme.error(),
                    EventSeverity::Output => theme.fg(),
                    EventSeverity::Trace => theme.fg_muted(),
                };
                let room = width.saturating_sub(1);
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(truncate_cells(&sanitize(&line.text), room), style),
                ])
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(rows).block(block), area);
}

// ── Hint line ───────────────────────────────────────────────────────────

/// One key hint: the key, an alternative key, and a long and a short label.
#[derive(Clone, Copy)]
struct Hint {
    key: &'static str,
    alt: Option<&'static str>,
    label: &'static str,
    short: &'static str,
}

const fn hint(key: &'static str, label: &'static str, short: &'static str) -> Hint {
    Hint {
        key,
        alt: None,
        label,
        short,
    }
}

const fn hint_alt(
    key: &'static str,
    alt: &'static str,
    label: &'static str,
    short: &'static str,
) -> Hint {
    Hint {
        key,
        alt: Some(alt),
        label,
        short,
    }
}

/// The keys worth advertising right now, most important first.
fn hints_for(snapshot: &DebugSnapshot<'_>, focus: Pane) -> Vec<Hint> {
    let mut hints = Vec::new();
    if !snapshot.source_search.query.is_empty() {
        hints.push(hint("n/N", "next/prev match", "match"));
    }
    match focus {
        Pane::Breakpoints if !snapshot.breakpoints.is_empty() => hints.extend([
            hint("Space", "enable/disable", "on/off"),
            hint("c", "condition", "cond"),
            hint("d", "delete", "del"),
        ]),
        Pane::Watches if !snapshot.watches.is_empty() => hints.extend([
            hint("Enter", "show value", "view"),
            hint("d", "delete", "del"),
        ]),
        Pane::Locals if !snapshot.locals.is_empty() => {
            hints.push(hint("Enter", "show value", "view"));
        }
        Pane::Stack if snapshot.stack.len() > 1 => {
            hints.push(hint("j/k", "choose frame", "frame"));
        }
        Pane::Events => hints.push(hint("/", "filter", "filter")),
        _ => {}
    }
    match snapshot.app_state {
        AppState::Stopped => hints.extend([
            hint_alt("F5", "c", "continue", "cont"),
            hint_alt("F10", "n", "step over", "over"),
            hint_alt("F11", "s", "step into", "into"),
            hint_alt("S-F11", "S", "step out", "out"),
            hint("b", "breakpoint", "bp"),
            hint("e", "eval", "eval"),
            hint("w", "watch", "watch"),
        ]),
        AppState::Running => hints.push(hint("p", "pause", "pause")),
        AppState::Completed | AppState::Dead | AppState::LoadFailed => hints.extend([
            hint("r", "restart", "restart"),
            hint("b", "breakpoint", "bp"),
        ]),
        AppState::Starting | AppState::Ready | AppState::Loading => {}
    }
    hints.push(hint("?", "help", "help"));
    if snapshot.app_state != AppState::Stopped {
        hints.push(hint("q", "quit", "quit"));
    }
    hints
}

#[derive(Clone, Copy)]
enum HintDensity {
    WithAlternatives,
    Labels,
    Short,
    KeysOnly,
}

fn hint_variant(hints: &[Hint], density: HintDensity) -> chrome::Variant {
    let mut variant = chrome::Variant::new().push(tokens::FG, " ");
    for (index, hint) in hints.iter().enumerate() {
        if index > 0 {
            variant = variant.push(tokens::FG_MUTED, "  ");
        }
        let key = match (density, hint.alt) {
            (HintDensity::WithAlternatives, Some(alt)) => format!("{}/{alt}", hint.key),
            _ => hint.key.to_string(),
        };
        variant = variant.push(tokens::HELP_KEY, key);
        let label = match density {
            HintDensity::WithAlternatives | HintDensity::Labels => Some(hint.label),
            HintDensity::Short => Some(hint.short),
            // Help keeps its word even at the tightest density: it is the
            // way to everything that was dropped.
            HintDensity::KeysOnly => (hint.key == "?").then_some(hint.label),
        };
        if let Some(label) = label {
            variant = variant.push(tokens::FG_DIM, format!(" {label}"));
        }
    }
    variant
}

fn render_hintline(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &DebugUiState,
    prompt: Option<&PromptInput>,
    theme: &Theme,
) {
    if ui.confirm_quit {
        let line = Line::from(vec![
            Span::styled(
                " End the debug session and quit? ",
                theme.style(&tokens::DEBUG_STATE_ERROR),
            ),
            Span::styled("  y", theme.help_key()),
            Span::styled(" quit   ", theme.fg_dim()),
            Span::styled("any other key", theme.help_key()),
            Span::styled(" keep debugging", theme.fg_dim()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    if let Some(prompt) = prompt {
        render_prompt(frame, area, prompt, theme);
        return;
    }
    if ui.popup.is_some() {
        let line = Line::from(vec![
            Span::styled(" Esc", theme.help_key()),
            Span::styled(" close  ", theme.fg_dim()),
            Span::styled("j/k", theme.help_key()),
            Span::styled(" scroll", theme.fg_dim()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    // The status is the answer to whatever the user just did; it keeps up
    // to a third of the row and is shortened past that. Hints take the rest
    // at the densest phrasing that fits.
    let total_width = area.width as usize;
    let status = single_line(snapshot.status);
    let hints = hints_for(snapshot, ui.focus);
    // Labelled hints beat a status that repeats the title bar: when the
    // status would push the hints down to bare keys, the status yields.
    let short_hints = chrome::fit(
        &[hint_variant(&hints, HintDensity::Short)],
        usize::from(u16::MAX),
        theme,
    )
    .width;
    let mut status_reserve = if status.is_empty() {
        0
    } else {
        cells(&status).clamp(12, total_width / 3) + 2
    };
    if short_hints + status_reserve > total_width && total_width >= short_hints + 2 {
        status_reserve = total_width - short_hints - 2;
    }
    let ladder = [
        HintDensity::WithAlternatives,
        HintDensity::Labels,
        HintDensity::Short,
        HintDensity::KeysOnly,
    ]
    .map(|density| hint_variant(&hints, density));
    let left = chrome::fit(&ladder, total_width.saturating_sub(status_reserve), theme);
    let room = total_width.saturating_sub(left.width + 2);
    let right = match chrome::elide(&status, room, theme) {
        Some(status) if room >= 8 => chrome::Row {
            width: cells(&status) + 1,
            spans: vec![
                Span::styled(status, theme.detail_value_accent()),
                Span::raw(" "),
            ],
        },
        _ => chrome::Row::empty(),
    };
    frame.render_widget(
        Paragraph::new(chrome::justify(left, right, total_width)),
        area,
    );
}

fn render_prompt(frame: &mut Frame<'_>, area: Rect, prompt: &PromptInput, theme: &Theme) {
    let width = area.width as usize;
    let label = format!(" {} › ", prompt.label);
    let mut cursor = prompt.cursor.min(prompt.buffer.len());
    while !prompt.buffer.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let (before, after) = prompt.buffer.split_at(cursor);
    let room = width.saturating_sub(cells(&label) + 1);
    // Keep the cursor in view: show the end of what precedes it when the
    // buffer is wider than the row.
    let before = tail_cells(before, room.saturating_sub(room / 4).max(1));
    let after = truncate_cells(after, room.saturating_sub(cells(&before)));
    let used = cells(&label) + cells(&before) + 1 + cells(&after);
    let mut spans = vec![
        Span::styled(label, theme.command_prompt()),
        Span::styled(before, theme.command_input()),
        Span::styled("█", theme.command_prompt()),
        Span::styled(after, theme.command_input()),
    ];
    let placeholder_room = width.saturating_sub(used + 3);
    if !prompt.placeholder.is_empty() && placeholder_room >= 12 {
        let placeholder = truncate_cells(&prompt.placeholder, placeholder_room);
        let gap = width.saturating_sub(used + cells(&placeholder) + 1).max(2);
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(placeholder, theme.fg_muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Overlays ────────────────────────────────────────────────────────────

fn render_help_overlay(frame: &mut Frame<'_>, area: Rect, theme: &Theme, scroll: &mut u16) {
    let popup = centered_rect(86, 92, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(Span::styled(
            " tcld debug — keybindings ",
            theme.status_pill_active(),
        ))
        .title_bottom(Line::from(Span::styled(
            " j/k scroll · Esc or ? closes ",
            theme.fg_muted(),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(theme.border_focused());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let sections = super::bindings::help_sections();
    let keys_width = sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .map(|(keys, _)| cells(keys))
        .max()
        .unwrap_or(8)
        .min(18);
    // Two columns when there is room, so the common keys fit without
    // scrolling; sections never split across columns.
    let two_columns = inner.width >= 96;
    let column_width = if two_columns {
        (inner.width / 2) as usize
    } else {
        inner.width as usize
    };
    let section_lines = |section: &super::bindings::HelpSection| {
        let mut lines = vec![Line::from(Span::styled(
            section.title,
            theme.help_section_title(),
        ))];
        for (keys, description) in &section.rows {
            let padding = " ".repeat(keys_width.saturating_sub(cells(keys)));
            let room = column_width.saturating_sub(keys_width + 5);
            lines.push(Line::from(vec![
                Span::styled(format!("  {keys}"), theme.help_key()),
                Span::raw(padding),
                Span::styled(
                    format!("  {}", truncate_cells(description, room)),
                    theme.help_desc(),
                ),
            ]));
        }
        lines.push(Line::from(""));
        lines
    };
    let mut columns: Vec<Vec<Line>> = vec![Vec::new()];
    if two_columns {
        let total: usize = sections.iter().map(|s| s.rows.len() + 2).sum();
        let mut left = Vec::new();
        let mut right = Vec::new();
        let mut used = 0usize;
        for section in &sections {
            let lines = section_lines(section);
            if used < total / 2 {
                used += lines.len();
                left.extend(lines);
            } else {
                right.extend(lines);
            }
        }
        columns = vec![left, right];
    } else {
        for section in &sections {
            columns[0].extend(section_lines(section));
        }
    }
    let content_height = columns.iter().map(Vec::len).max().unwrap_or(0);
    let max_scroll = content_height.saturating_sub(inner.height as usize) as u16;
    *scroll = (*scroll).min(max_scroll);
    let areas = if two_columns {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner)
            .to_vec()
    } else {
        vec![inner]
    };
    for (column, area) in columns.into_iter().zip(areas) {
        frame.render_widget(Paragraph::new(column).scroll((*scroll, 0)), area);
    }
}

fn render_value_popup(frame: &mut Frame<'_>, area: Rect, popup: &mut ValuePopup, theme: &Theme) {
    let rect = centered_rect(70, 60, area);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", popup.title),
            theme.panel_title_focused(),
        ))
        .title_bottom(Line::from(Span::styled(
            " j/k scroll · Esc closes ",
            theme.fg_muted(),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(theme.border_focused());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let body = sanitize(&popup.body);
    let max_scroll = wrapped_rows(&body, inner.width as usize).saturating_sub(inner.height);
    popup.scroll = popup.scroll.min(max_scroll);
    let paragraph = Paragraph::new(body)
        .style(theme.detail_value())
        .wrap(Wrap { trim: false })
        .scroll((popup.scroll, 0));
    frame.render_widget(paragraph, inner);
}

fn render_picker_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &super::types::PickerOverlay<'_>,
    theme: &Theme,
) {
    let height = (picker.entries.len().max(1) as u16 + 2).min(area.height.saturating_sub(2));
    let width = area.width.saturating_sub(8);
    let popup = Rect {
        x: area.x + 4,
        y: area.y + 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", picker.title),
            theme.panel_title_focused(),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(theme.border_focused());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if picker.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  no procs match — keep typing or press Esc",
                theme.empty_state(),
            )),
            inner,
        );
        return;
    }

    let visible = inner.height as usize;
    let cursor = picker.selected.unwrap_or(0);
    let view_top = if cursor >= visible {
        cursor + 1 - visible
    } else {
        0
    };
    let mut lines = Vec::with_capacity(visible);
    for (i, entry) in picker
        .entries
        .iter()
        .enumerate()
        .skip(view_top)
        .take(visible)
    {
        let style = if Some(i) == picker.selected {
            theme.style(&tokens::SELECTION_STRONG)
        } else {
            theme.detail_value()
        };
        let prefix = if Some(i) == picker.selected {
            "▶ "
        } else {
            "  "
        };
        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), theme.detail_value_accent()),
            Span::styled(entry.clone(), style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Shared pieces ───────────────────────────────────────────────────────

fn base_block(focused: bool, theme: &Theme) -> Block<'_> {
    // Focus reads from the border's weight as well as its color, so it
    // survives monochrome terminals and `--no-colors`.
    Block::default()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(if focused {
            theme.border_focused()
        } else {
            theme.border_dim()
        })
}

pub fn panel_block<'a>(title: &str, focused: bool, theme: &'a Theme) -> Block<'a> {
    let title = if title.starts_with(' ') {
        title.to_string()
    } else {
        format!(" {title} ")
    };
    base_block(focused, theme).title(Span::styled(
        title,
        if focused {
            theme.panel_title_focused()
        } else {
            theme.panel_title()
        },
    ))
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let pop_y = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(pop_y[1])[1]
}

/// Replace ASCII control bytes (everything below 0x20 except common
/// whitespace) and `DEL` with `·` so they never reach the terminal.
/// Decompiled `.tbc` payloads sometimes carry stray control bytes
/// from string-literal fragments; left untouched they trash the
/// cursor position and produce phantom-letter artifacts.
fn sanitize(s: &str) -> String {
    if s.chars().all(|c| !is_unsafe_control(c)) {
        return s.to_string();
    }
    s.chars()
        .map(|c| if is_unsafe_control(c) { '·' } else { c })
        .collect()
}

fn char_boundary_at_or_before(value: &str, requested: usize) -> usize {
    let mut boundary = requested.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn is_unsafe_control(c: char) -> bool {
    matches!(c, '\0'..='\u{08}' | '\u{0b}'..='\u{1f}' | '\u{7f}')
}

/// Collapse a multi-line value onto one row, marking where lines joined.
fn single_line(value: &str) -> String {
    if !value.contains('\n') {
        return value.to_string();
    }
    value
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join(" ⏎ ")
}

/// Keep the first `max` cells of `value`, ending with `…` when cut.
///
/// Unlike [`chrome::elide`], this cuts anywhere, which is right for paths,
/// values and log lines: breaking at the last space would throw away the
/// part of `paused at file:/very/long/path.tcl:31` that matters.
fn truncate_cells(value: &str, max: usize) -> String {
    if cells(value) <= max {
        return value.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for character in value.chars() {
        let wide = char_cells(character);
        if used + wide > max - 1 {
            break;
        }
        out.push(character);
        used += wide;
    }
    out.push('…');
    out
}

/// Keep the last `max` cells of `value`, starting with `…` when cut.
fn tail_cells(value: &str, max: usize) -> String {
    if cells(value) <= max {
        return value.to_string();
    }
    let mut kept = std::collections::VecDeque::new();
    let mut used = 1;
    for character in value.chars().rev() {
        let wide = char_cells(character);
        if used + wide > max {
            break;
        }
        kept.push_front(character);
        used += wide;
    }
    std::iter::once('…').chain(kept).collect()
}

/// Rows `text` occupies when wrapped at `width` cells; close enough to bound
/// scrolling without re-running ratatui's word wrapper.
fn wrapped_rows(text: &str, width: usize) -> u16 {
    let width = width.max(1);
    let rows: usize = text
        .lines()
        .map(|line| cells(line).div_ceil(width).max(1))
        .sum();
    u16::try_from(rows).unwrap_or(u16::MAX)
}

/// Cells occupied by a run of text.
fn cells(value: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(value)
}

fn char_cells(character: char) -> usize {
    unicode_width::UnicodeWidthChar::width(character).unwrap_or(0)
}

fn file_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::debug::{
        AppState, BreakpointKindLabel, DebugSnapshot, DebugUiState, EventLine, EventSeverity,
        FocusedProc, LocalRow, PauseKind, PauseReasonLabel, ProcBreakpoint, SourceBlockKind,
        SourceCommand, SourceLine, SourceOrigin, SourceSearch, StackFrame,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_to_text(
        snapshot: &DebugSnapshot<'_>,
        ui: &mut DebugUiState,
        prompt: Option<&PromptInput>,
        theme: &Theme,
    ) -> String {
        render_to_text_sized(snapshot, ui, prompt, theme, 140, 32)
    }

    fn render_to_text_sized(
        snapshot: &DebugSnapshot<'_>,
        ui: &mut DebugUiState,
        prompt: Option<&PromptInput>,
        theme: &Theme,
        width: u16,
        height: u16,
    ) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, snapshot, ui, prompt, theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        let w = width as usize;
        for row in 0..height as usize {
            let start = row * w;
            let end = start + w;
            let line: String = buffer.content()[start..end]
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string();
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    #[test]
    fn minimum_size_frame_renders_only_remediation_and_exit_keys() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines = vec![SourceLine::raw("puts hi")];
        let snapshot = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &[]);
        for (width, height) in [
            (MIN_DEBUG_WIDTH - 1, MIN_DEBUG_HEIGHT),
            (MIN_DEBUG_WIDTH, MIN_DEBUG_HEIGHT - 1),
        ] {
            let mut ui = DebugUiState::new();
            let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, width, height);
            assert!(text.contains("Terminal too small"), "{text}");
            assert!(text.contains("Ctrl-C exit | q quit"), "{text}");
            assert!(!text.contains("Source "), "workspace panes leaked: {text}");
            assert!(ui.hit.panes.is_empty());
        }
    }

    fn line_bp(id: &str, qname: &str, offset: u64) -> ProcBreakpoint {
        ProcBreakpoint {
            id: id.into(),
            qname: qname.into(),
            source_path: None,
            line: None,
            pending: false,
            pending_reason: None,
            offset,
            len: 1,
            resolved_block: Some(format!("proc:{qname}")),
            resolved_line: None,
            resolved_offset: Some(offset),
            resolved_len: Some(1),
            resolution_generation: None,
            enabled: true,
            hits: 0,
            ignore_count: 0,
            condition: None,
            kind: BreakpointKindLabel::Line,
            location_label: None,
        }
    }

    fn paused(qname: &str, line: u32, kind: PauseKind) -> FocusedProc {
        FocusedProc {
            block_id: format!("proc:{qname}"),
            block_kind: SourceBlockKind::Proc,
            source_path: Some("/tmp/demo.tcl".into()),
            qname: qname.into(),
            line,
            offset_start: 0,
            offset_end: 1,
            pause_reason: PauseReasonLabel::Step,
            frame_count: 1,
            breakpoint: None,
            pause_kind: kind,
            error_message: None,
        }
    }

    fn snap<'a>(
        title: &'a str,
        path: &'a str,
        lines: &'a [SourceLine],
        bps: &'a [ProcBreakpoint],
        focused: Option<&'a FocusedProc>,
        events: &'a [EventLine],
    ) -> DebugSnapshot<'a> {
        DebugSnapshot {
            title,
            target_path: path,
            source_path: path,
            source_label: "",
            source_origin: SourceOrigin::UserScript,
            source_qname: None,
            source_lines: lines,
            secondary_source_lines: &[],
            secondary_highlight_line: None,
            focused_proc: focused,
            paused_line: focused.map(|focus| focus.line),
            location_label: None,
            app_state: AppState::Stopped,
            stack: &[],
            inspected_frame: 0,
            locals: &[],
            breakpoints: bps,
            watches: &[],
            events,
            event_filter: "",
            coverage: &[],
            coverage_enabled: false,
            status: "ready",
            error_banner: None,
            source_search: SourceSearch::default(),
            picker: None,
            theme_label: "obsidian",
            instruction_set: None,
        }
    }

    #[test]
    fn render_paused_state_shows_source_breakpoint_stack_locals() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = ["proc demo {} {", "    set a 1", "    set b 2", "}"]
            .into_iter()
            .map(SourceLine::raw)
            .collect();
        let focused = paused("demo", 2, PauseKind::Normal);
        let stack = vec![StackFrame {
            level: 1,
            kind: "proc".into(),
            proc_name: Some("demo".into()),
            block: Some("proc:demo".into()),
            block_kind: Some(SourceBlockKind::Proc),
            source_path: Some("/tmp/demo.tcl".into()),
            line: Some(2),
            offset_start: Some(0),
            offset_end: Some(1),
            location_label: Some("demo.tcl:2".into()),
        }];
        let locals = vec![LocalRow {
            name: "a".to_string(),
            value: "1".to_string(),
            changed: true,
            builtin: false,
        }];
        let breakpoints = vec![line_bp("bp0", "demo", 2)];
        let events = vec![EventLine::pause("paused frame=1 bp=bp0")];
        let snapshot = DebugSnapshot {
            stack: &stack,
            locals: &locals,
            inspected_frame: 1,
            location_label: Some("demo · demo.tcl:2"),
            ..snap(
                "tcld debug",
                "/tmp/demo.tcl",
                &lines,
                &breakpoints,
                Some(&focused),
                &events,
            )
        };
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(text.contains("tcld debug"), "title pill missing");
        assert!(text.contains("[Paused]"), "app-state badge missing");
        assert!(text.contains("demo"), "proc qname missing from titlebar");
        assert!(text.contains("set a 1"), "source line missing");
        assert!(text.contains("bp0"), "breakpoint id missing");
        assert!(text.contains("a = 1"), "locals row missing");
        assert!(text.contains("paused frame=1"), "event log missing");
        assert!(
            text.contains("Source · demo.tcl"),
            "source pane label missing"
        );
        assert!(
            text.contains("at demo · demo.tcl:2"),
            "pause location missing"
        );
        assert!(text.contains("Locals · demo"), "locals scope missing");
        assert!(
            text.contains("F5/c continue") || text.contains("F5 continue"),
            "paused hints advertise stepping: {text}"
        );
    }

    #[test]
    fn source_title_and_gutter_expose_nested_command_selection() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines = vec![SourceLine::raw("set x [expr {1 + 1}]").with_commands(vec![
            SourceCommand {
                offset: 0,
                len: 20,
                qname: None,
                target_offset: 0,
                start_column: 0,
                end_column: 20,
                depth: 0,
                label: "set".into(),
            },
            SourceCommand {
                offset: 7,
                len: 12,
                qname: None,
                target_offset: 7,
                start_column: 7,
                end_column: 19,
                depth: 1,
                label: "expr".into(),
            },
        ])];
        let focused = paused("demo", 1, PauseKind::Normal);
        let breakpoints = vec![line_bp("bp0", "demo", 0), line_bp("bp1", "demo", 7)];
        let mut snapshot = snap(
            "tcld debug",
            "/tmp/demo.tcl",
            &lines,
            &breakpoints,
            Some(&focused),
            &[],
        );
        snapshot.source_origin = SourceOrigin::ProcBody;
        let mut ui = DebugUiState::new();
        ui.source_command_cursor = 1;
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(text.contains("command 2/2: expr"), "{text}");
        assert!(
            !text.contains("@7"),
            "byte offsets stay out of the title: {text}"
        );
        assert!(
            text.lines().any(|line| line.contains("2    1")),
            "two breakpoints on the line should be visible in the gutter: {text}"
        );
    }

    #[test]
    fn render_dual_code_panes_centers_and_marks_original_source() {
        let theme = Theme::with_palette(false, "obsidian").unwrap();
        let disassembly: Vec<SourceLine> = ["(0) push1 0", "(2) done"]
            .into_iter()
            .map(SourceLine::raw)
            .collect();
        let original: Vec<SourceLine> = ["set a 1", "set b 2", "set c 3"]
            .into_iter()
            .map(SourceLine::raw)
            .collect();
        let snapshot = DebugSnapshot {
            secondary_source_lines: &original,
            secondary_highlight_line: Some(2),
            source_origin: SourceOrigin::Synthetic,
            ..snap("hybrid", "/tmp/demo.tbc", &disassembly, &[], None, &[])
        };
        let mut ui = DebugUiState::new();
        let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, 140, 32);
        assert!(text.contains("Source · companion (unverified)"), "{text}");
        assert!(text.contains("▶     2 set b 2"), "{text}");
    }

    #[test]
    fn render_dead_state_shows_empty_panes_and_hint_line() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = vec![SourceLine::raw("puts hello")];
        let snapshot = DebugSnapshot {
            app_state: AppState::Dead,
            status: "press r to start",
            ..snap("tcld debug", "/tmp/empty.tcl", &lines, &[], None, &[])
        };
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(text.contains("[Dead]"));
        assert!(text.contains("r restart"), "{text}");
        assert!(
            !text.contains("F10"),
            "stepping is not offered while dead: {text}"
        );
        assert!(text.contains("press r to start"));
        assert!(text.contains("(none — press b"));
    }

    #[test]
    fn render_with_prompt_shows_command_line() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = Vec::new();
        let snapshot = DebugSnapshot {
            app_state: AppState::Dead,
            status: "",
            ..snap("tcld debug", "<scratch>", &lines, &[], None, &[])
        };
        let mut prompt = PromptInput::new("dbg").with_placeholder("Tab completes");
        prompt.buffer.push_str("vars 1");
        prompt.cursor = prompt.buffer.len();
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, Some(&prompt), &theme);
        assert!(
            text.contains(" dbg › vars 1█"),
            "prompt line missing the typed input — got:\n{text}"
        );
        assert!(
            text.contains("Tab completes"),
            "placeholder missing:\n{text}"
        );

        prompt.cursor = 4;
        let text = render_to_text(&snapshot, &mut ui, Some(&prompt), &theme);
        assert!(
            text.contains(" dbg › vars█ 1"),
            "cursor must render where it is, not at the end — got:\n{text}"
        );
    }

    #[test]
    fn render_error_pause_shows_banner_and_red_state() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = vec![SourceLine::raw("expr 1/0")];
        let focused = FocusedProc {
            block_id: "proc:demo".into(),
            block_kind: SourceBlockKind::Proc,
            source_path: Some("/tmp/x.tcl".into()),
            qname: "demo".into(),
            line: 1,
            offset_start: 0,
            offset_end: 1,
            pause_reason: PauseReasonLabel::Error,
            frame_count: 1,
            breakpoint: None,
            pause_kind: PauseKind::Error,
            error_message: Some("divide by zero".into()),
        };
        let banner = "divide by zero";
        let snapshot = DebugSnapshot {
            error_banner: Some(banner),
            ..snap("tcld debug", "/tmp/x.tcl", &lines, &[], Some(&focused), &[])
        };
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(text.contains(" error "), "error banner pill missing");
        assert!(text.contains("divide by zero"), "error message missing");
    }

    #[test]
    fn render_narrow_terminal_folds_inspector_panes_into_tabs() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = vec![SourceLine::raw("puts x")];
        let snapshot = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &[]);
        let mut ui = DebugUiState::new();
        let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, 80, 40);
        // The source keeps full width; every inspector pane is one tab away.
        for needle in [
            " Source ",
            " Stack ",
            " Locals ",
            " Breakpoints ",
            " Watches ",
        ] {
            assert!(
                text.contains(needle),
                "expected pane {needle} in narrow render — got:\n{text}"
            );
        }
        assert_eq!(ui.hit.inspector_tabs.len(), 4, "tabs are clickable");
        assert!(
            text.contains("(no variables"),
            "Locals is the default tab: {text}"
        );

        ui.focus_pane(Pane::Watches);
        let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, 80, 40);
        assert!(text.contains("(none — press w to add)"), "{text}");
        ui.focus_pane(Pane::Source);
        let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, 80, 40);
        assert!(
            text.contains("(none — press w to add)"),
            "the last inspector tab stays shown while the source has focus: {text}"
        );
    }

    #[test]
    fn long_source_lines_are_clipped_not_wrapped() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let long = format!("set value {}", "x".repeat(200));
        let lines = vec![SourceLine::raw(long), SourceLine::raw("set next 2")];
        let snapshot = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &[]);
        let mut ui = DebugUiState::new();
        let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, 120, 32);
        let rows: Vec<&str> = text.lines().collect();
        let first = rows
            .iter()
            .position(|row| row.contains("set value"))
            .expect("first line");
        assert!(rows[first].contains('…'), "clipped edge is marked: {text}");
        assert!(
            rows[first + 1].contains("2  set next 2"),
            "the next source line is the next row: {text}"
        );

        ui.source_hscroll = 20;
        let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, 120, 32);
        assert!(!text.contains("set value"), "scrolled past: {text}");
    }

    #[test]
    fn console_is_chronological_and_hides_bookkeeping() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = vec![SourceLine::raw("puts hi")];
        let events = vec![
            EventLine::trace("source unit registered: file demo.tcl"),
            EventLine::new(EventSeverity::Output, "hello from the program"),
            EventLine::pause("Paused at demo.tcl:1"),
        ];
        let snapshot = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &events);
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(!text.contains("source unit registered"), "{text}");
        let output = text.find("hello from the program").expect("output");
        let pause = text.find("Paused at demo.tcl:1").expect("pause");
        assert!(output < pause, "newest line is at the bottom: {text}");

        let filtered = DebugSnapshot {
            event_filter: "registered",
            ..snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &events)
        };
        let text = render_to_text(&filtered, &mut ui, None, &theme);
        assert!(
            text.contains("source unit registered"),
            "filtering reaches bookkeeping lines too: {text}"
        );
    }

    #[test]
    fn quit_confirmation_replaces_the_hint_line() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = vec![SourceLine::raw("puts hi")];
        let snapshot = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &[]);
        let mut ui = DebugUiState::new();
        ui.confirm_quit = true;
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(text.contains("End the debug session and quit?"), "{text}");
    }

    #[test]
    fn side_heights_fit_content_and_leave_locals_the_rest() {
        let (heights, flex) = side_heights(30, &[2, 0, 1]);
        assert_eq!(heights, vec![4, 3, 3]);
        assert_eq!(flex, 20);

        let (heights, flex) = side_heights(16, &[40, 40, 40]);
        assert!(
            flex >= 5,
            "locals keeps a usable minimum: {heights:?} {flex}"
        );
        assert!(heights.iter().all(|height| *height >= 3));
        assert_eq!(heights.iter().sum::<u16>() + flex, 16);
    }

    #[test]
    fn render_records_hit_targets_for_panes_and_source() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = (1..=12)
            .map(|n| SourceLine::raw(format!("line {n}")))
            .collect();
        let snapshot = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &[]);
        let mut ui = DebugUiState::new();
        let _ = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(ui.hit.source_inner.is_some());
        assert!(ui.hit.panes.iter().any(|h| h.pane == Pane::Source));
        // A click in the source pane's interior should resolve to a line.
        let inner = ui.hit.source_inner.unwrap();
        let click_x = inner.x + 1;
        let click_y = inner.y + 2;
        assert_eq!(
            ui.hit.source_line_at(click_x, click_y),
            Some(ui.hit.source_view_top + 2)
        );
    }

    #[test]
    fn titlebar_suppresses_location_when_qname_empty_and_line_zero() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = vec![SourceLine::raw("puts hi")];
        let focused = FocusedProc {
            block_id: String::new(),
            block_kind: SourceBlockKind::Anonymous,
            source_path: None,
            qname: String::new(),
            line: 0,
            offset_start: 0,
            offset_end: 1,
            pause_reason: PauseReasonLabel::Error,
            frame_count: 0,
            breakpoint: None,
            pause_kind: PauseKind::Error,
            error_message: Some("can't find package tbcload 1.6".into()),
        };
        let banner = "can't find package tbcload 1.6";
        let snapshot = DebugSnapshot {
            app_state: AppState::Dead,
            error_banner: Some(banner),
            ..snap(
                "tcld debug",
                "/tmp/public_complex_fixture.tbc",
                &lines,
                &[],
                Some(&focused),
                &[],
            )
        };
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(
            !text.contains(": line 0"),
            "stale `: line 0` chunk should be suppressed — got:\n{text}"
        );
        assert!(
            text.contains("Crashed"),
            "Dead + error pause should show Crashed badge — got:\n{text}"
        );
        assert!(text.contains("can't find package tbcload 1.6"));
    }

    #[test]
    fn control_bytes_in_decompiled_source_are_sanitised() {
        // Decompiled .tbc strings sometimes carry stray ASCII control
        // bytes (NUL, BEL, ESC) that ratatui will happily emit as
        // zero-width cells, drifting subsequent characters left and
        // producing phantom-letter artifacts. Sanitising replaces
        // them with `·` so the rendered grid stays aligned.
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines = vec![SourceLine::raw(
            "package provide fixture \u{0007}\u{001b}[31mflag\u{0000}",
        )];
        let snap = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &[]);
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snap, &mut ui, None, &theme);
        for ch in ['\u{0007}', '\u{001b}', '\u{0000}'] {
            assert!(
                !text.contains(ch),
                "raw control byte {ch:?} leaked into rendered text"
            );
        }
        assert!(text.contains("package provide fixture"));
        assert!(
            text.contains("·"),
            "expected `·` placeholder for control bytes"
        );
    }

    #[test]
    fn shorter_source_does_not_leak_prior_frames_cells() {
        // Reproduces the `tfixture.tbc` / `lpackage` artifact from a real
        // session: render once with a long line of source, then again
        // with a shorter line, and confirm no stale chars remain past
        // the new content.
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let backend = TestBackend::new(140, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        let long: Vec<SourceLine> = vec![SourceLine::raw(
            "package require tbcload 1.6 ; # this line is intentionally long to seed the buffer with characters",
        )];
        let short: Vec<SourceLine> = vec![SourceLine::raw("set a 1")];

        // Frame 1: long content fills cells.
        let snap1 = snap("tcld debug", "/tmp/x.tcl", &long, &[], None, &[]);
        let mut ui = DebugUiState::new();
        terminal
            .draw(|frame| render(frame, &snap1, &mut ui, None, &theme))
            .unwrap();

        // Frame 2: short content. With the bug, leftover chars from the
        // long line bleed past `set a 1`; with the up-front Clear they
        // are wiped.
        let snap2 = snap("tcld debug", "/tmp/x.tcl", &short, &[], None, &[]);
        terminal
            .draw(|frame| render(frame, &snap2, &mut ui, None, &theme))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let width = 140usize;
        // The source pane occupies the body area; line 1 of source
        // sits a few rows down inside the Source block. Walk the
        // whole buffer and check that no row contains the leaked
        // suffix `tbcload`.
        let mut text = String::new();
        for row in 0..32usize {
            let start = row * width;
            let end = start + width;
            let line: String = buffer.content()[start..end]
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            text.push_str(&line);
            text.push('\n');
        }
        assert!(
            !text.contains("tbcload"),
            "stale `tbcload` chars from the long first frame leaked into the second frame:\n{text}"
        );
        assert!(
            text.contains("set a 1"),
            "new content should still be present:\n{text}"
        );
    }

    #[test]
    fn render_event_log_filter_hides_non_matching_lines() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = Vec::new();
        let events = vec![
            EventLine::info("resumed execution"),
            EventLine::pause("paused frame=1"),
            EventLine::error("error_paused divide by zero"),
        ];
        let snapshot = DebugSnapshot {
            event_filter: "error",
            ..snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &events)
        };
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, None, &theme);
        assert!(
            text.contains("error_paused"),
            "filtered event missing — got:\n{text}"
        );
        assert!(
            !text.contains("resumed execution"),
            "unfiltered event should be hidden — got:\n{text}"
        );
    }
}
