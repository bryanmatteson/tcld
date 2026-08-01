//! Frame-level renderer for the debugger TUI.
//!
//! All chrome (titlebar, panels, hint line, help overlay, error
//! banner) flows through [`crate::Theme`] so the debugger inherits
//! the explorer's palette switching for free. The renderer fills
//! [`HitTargets`] each frame so mouse events can be resolved without
//! re-running layout in the driver.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::Theme;
use crate::tokens;

use super::state::{DebugUiState, HitTargets, Pane, PaneHit, PromptInput};
use super::types::{AppState, DebugSnapshot, EventSeverity, PauseKind, SourceLine, SourceOrigin};

pub const MIN_DEBUG_WIDTH: u16 = 80;
pub const MIN_DEBUG_HEIGHT: u16 = 24;

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
    let banner_height = if snapshot.error_banner.is_some() {
        1
    } else {
        0
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // titlebar
            Constraint::Length(banner_height), // optional error banner
            Constraint::Min(8),                // body
            Constraint::Length(7),             // events / log
            Constraint::Length(1),             // hint / prompt line
        ])
        .split(area);

    render_titlebar(frame, chunks[0], snapshot, theme);
    if let Some(banner) = snapshot.error_banner {
        render_error_banner(frame, chunks[1], banner, theme);
    }
    render_body(frame, chunks[2], snapshot, ui, theme);
    render_events(frame, chunks[3], snapshot, ui, theme);
    render_hintline(frame, chunks[4], snapshot, prompt, theme);

    if let Some(picker) = snapshot.picker.as_ref() {
        render_picker_overlay(frame, chunks[2], picker, theme);
    }

    if ui.show_help {
        render_help_overlay(frame, area, theme, ui.help_scroll);
    }
}

fn render_titlebar(frame: &mut Frame<'_>, area: Rect, snapshot: &DebugSnapshot<'_>, theme: &Theme) {
    let pause_kind = snapshot.focused_proc.map(|p| p.pause_kind);
    let crashed = snapshot.app_state == AppState::Dead && pause_kind == Some(PauseKind::Error);
    let state_label = if crashed {
        "Crashed".to_string()
    } else {
        format!("[{}]", snapshot.app_state.label())
    };
    let state_style = match (snapshot.app_state, pause_kind) {
        (AppState::Stopped, Some(PauseKind::Error)) => theme.style(&tokens::DEBUG_STATE_ERROR),
        (AppState::Stopped, _) => theme.command_prompt(),
        (AppState::Ready | AppState::Starting | AppState::Loading, _) => theme.detail_value(),
        (AppState::Running, _) => theme.success(),
        (AppState::Completed, _) => theme.style(&tokens::DEBUG_STATE_COMPLETED),
        (AppState::Dead | AppState::LoadFailed, _) => theme.error(),
    };
    let mut spans = vec![
        Span::styled(
            format!(" {} ", snapshot.title),
            theme.style(&tokens::DEBUG_TITLE),
        ),
        Span::raw(" "),
        Span::styled(
            file_label(snapshot.source_path),
            theme.detail_value_accent(),
        ),
        Span::styled(
            format!(" [{}]", snapshot.source_origin.label()),
            theme.detail_label(),
        ),
        Span::styled("  ·  ", theme.detail_separator()),
        Span::styled(
            if crashed {
                state_label.clone()
            } else {
                state_label
            },
            state_style,
        ),
    ];
    if let Some(p) = snapshot.focused_proc {
        // Suppress the location chunk when there's nothing real to
        // show — without this an early-startup pause renders as
        // noisy `   : line 0`.
        if !p.qname.is_empty() || p.line > 0 {
            let location = if p.qname.is_empty() {
                format!("  line {}", p.line)
            } else if p.line > 0 {
                format!("  {} : line {}", p.qname, p.line)
            } else {
                format!("  {}", p.qname)
            };
            spans.push(Span::styled(location, theme.detail_value()));
        }
        if let Some(bp) = p.breakpoint.as_deref() {
            spans.push(Span::styled("  hit:", theme.detail_label()));
            spans.push(Span::styled(bp.to_string(), theme.error()));
        }
    }
    spans.push(Span::styled("  ·  ", theme.detail_separator()));
    spans.push(Span::styled(
        format!("{} bp", snapshot.breakpoints.len()),
        theme.detail_label(),
    ));
    spans.push(Span::styled(
        format!(" · {} watch", snapshot.watches.len()),
        theme.detail_label(),
    ));
    if snapshot.coverage_enabled {
        spans.push(Span::styled(" · cov", theme.success()));
    }
    if !snapshot.theme_label.is_empty() {
        spans.push(Span::styled("  ·  ", theme.detail_separator()));
        spans.push(Span::styled(
            format!("theme:{}", snapshot.theme_label),
            theme.detail_label(),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_error_banner(frame: &mut Frame<'_>, area: Rect, msg: &str, theme: &Theme) {
    let line = Line::from(vec![
        Span::styled(" error ", theme.style(&tokens::DEBUG_BANNER_ERROR)),
        Span::raw(" "),
        Span::styled(msg.to_string(), theme.error()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_body(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    // Reflow: when the terminal is narrow, drop the right column and
    // fold the inspector panes under the source.
    let narrow = area.width < 100;

    if narrow {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(6),
            ])
            .split(area);
        render_code_panes(frame, rows[0], snapshot, ui, theme);
        render_stack(frame, rows[1], snapshot, ui, theme);
        render_locals(frame, rows[2], snapshot, ui, theme);
        render_breakpoints(frame, rows[3], snapshot, ui, theme);
        render_watches(frame, rows[4], snapshot, ui, theme);
        render_coverage(frame, rows[5], snapshot, ui, theme);
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(8)])
        .split(columns[0]);

    let right_constraints = if snapshot.coverage_enabled {
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(25),
        ]
    } else {
        vec![
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ]
    };
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints(right_constraints)
        .split(columns[1]);

    render_code_panes(frame, left[0], snapshot, ui, theme);
    render_breakpoints(frame, left[1], snapshot, ui, theme);
    render_stack(frame, right[0], snapshot, ui, theme);
    render_locals(frame, right[1], snapshot, ui, theme);
    render_watches(frame, right[2], snapshot, ui, theme);
    if snapshot.coverage_enabled {
        render_coverage(frame, right[3], snapshot, ui, theme);
    }
}

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
    let lines: Vec<Line<'_>> = snapshot
        .secondary_source_lines
        .iter()
        .enumerate()
        .skip(top)
        .take(height)
        .map(|(index, line)| {
            let line_no = index + 1;
            let active = highlight == Some(line_no);
            Line::from(vec![
                Span::styled(
                    if active {
                        format!("▶ {line_no:5} ")
                    } else {
                        format!("  {line_no:5} ")
                    },
                    if active {
                        theme.gutter_focused()
                    } else {
                        theme.gutter_default()
                    },
                ),
                Span::styled(
                    line.raw.as_str(),
                    if active {
                        theme.focused_line_bg()
                    } else {
                        theme.style(&tokens::PLAIN)
                    },
                ),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        panes[1],
    );
}

fn render_source(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let focused = ui.focus == Pane::Source;
    let base_title = match snapshot.source_origin {
        SourceOrigin::UserScript => " Source ",
        SourceOrigin::CompiledSource => " Source · compiled source ",
        SourceOrigin::SourcedFile => " Source · sourced file ",
        SourceOrigin::ProcBody => " Source · proc body ",
        SourceOrigin::Anonymous => " Source · eval (session-only) ",
        SourceOrigin::Synthetic => " Source · synthetic ",
    };
    let selected_command = snapshot
        .source_lines
        .get(ui.source_cursor)
        .and_then(|line| {
            let index = ui
                .source_command_cursor
                .min(line.commands.len().saturating_sub(1));
            line.commands.get(index).map(|command| {
                (
                    index,
                    line.commands.len(),
                    command.label.as_str(),
                    command.offset,
                )
            })
        });
    let dynamic_title;
    let title: &str = if !snapshot.source_search.query.is_empty() {
        let total = snapshot.source_search.match_lines.len();
        let cur = snapshot.source_search.current.map(|c| c + 1).unwrap_or(0);
        dynamic_title = format!(
            "{} · /{}  {}/{}",
            base_title.trim_end(),
            snapshot.source_search.query,
            cur,
            total
        );
        &dynamic_title
    } else if let Some((index, count, label, offset)) = selected_command {
        dynamic_title = format!(
            "{} · command {}/{}: {} @{} ",
            base_title.trim_end(),
            index + 1,
            count,
            label,
            offset
        );
        &dynamic_title
    } else {
        base_title
    };
    let block = panel_block(title, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    ui.hit.panes.push(PaneHit {
        pane: Pane::Source,
        area,
    });
    ui.hit.source_inner = Some(inner);

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

    let paused_line = snapshot.focused_proc.map(|p| p.line as usize).unwrap_or(0);
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

        // Search hit highlighting: if the query is present and this
        // line matches, underline the row. The current match also
        // gets a yellow accent background.
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

        let mut spans = Vec::with_capacity(4 + snapshot.source_lines[idx].spans.len());
        spans.push(Span::styled(format!(" {pointer}"), gutter_style));
        spans.push(Span::styled(bp_marker, bp_render_style));
        spans.push(Span::styled(format!(" {line_no:>4} "), gutter_style));
        spans.push(Span::styled(" ", row_style));
        // Strip control bytes (NUL, BEL, ESC, …) from raw source text
        // before pushing to the renderer. Decompiled .tbc occasionally
        // contains stray control bytes from string-literal fragments
        // the decompiler couldn't reconstruct; ratatui happily writes
        // them as zero-width cells which then drift subsequent
        // characters left and produce phantom-letter artifacts.
        let selected = is_cursor.then(|| {
            let index = ui
                .source_command_cursor
                .min(source_line.commands.len().saturating_sub(1));
            source_line.commands.get(index)
        });
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
            spans.push(Span::styled(sanitize(&source_line.raw[..start]), row_style));
            spans.push(Span::styled(
                sanitize(&source_line.raw[start..end]),
                command_style,
            ));
            spans.push(Span::styled(sanitize(&source_line.raw[end..]), row_style));
        } else if source_line.spans.is_empty() {
            spans.push(Span::styled(sanitize(&source_line.raw), row_style));
        } else {
            for span in &source_line.spans {
                let merged = if row_style.bg.is_some() || is_search_match {
                    span.style.patch(row_style)
                } else {
                    span.style
                };
                spans.push(Span::styled(sanitize(&span.content), merged));
            }
        }
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

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_stack(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let focused = ui.focus == Pane::Stack;
    let block = panel_block(" Stack ", focused, theme);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Stack,
        area,
    });
    ui.hit.stack_inner = Some(block.inner(area));
    let items: Vec<ListItem> = if snapshot.stack.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (no frames — pause execution first)",
            theme.empty_state(),
        ))]
    } else {
        snapshot
            .stack
            .iter()
            .map(|f| {
                let proc = f.proc_name.clone().unwrap_or_else(|| f.kind.clone());
                let line = f.line.map(|l| format!(":{l}")).unwrap_or_default();
                let marker = if f.level == snapshot.inspected_frame {
                    "*"
                } else {
                    " "
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{marker}[{:>2}] ", f.level), theme.detail_label()),
                    Span::styled(proc, theme.detail_value_accent()),
                    Span::styled(line, theme.detail_value()),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.style(&tokens::SELECTION_STRONG))
        .highlight_symbol(if focused { "▶ " } else { "  " });
    frame.render_stateful_widget(list, area, &mut ui.selections.stack);
}

fn render_locals(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let focused = ui.focus == Pane::Locals;
    let title = format!(" Locals · level {} ", snapshot.inspected_frame);
    let block = panel_block(&title, focused, theme);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Locals,
        area,
    });
    ui.hit.locals_inner = Some(block.inner(area));
    let inner_width = block.inner(area).width as usize;
    let items: Vec<ListItem> = if snapshot.locals.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (no locals — pause inside a proc)",
            theme.empty_state(),
        ))]
    } else {
        snapshot
            .locals
            .iter()
            .map(|row| {
                let name_style = theme.variable();
                let (marker, value_style) = if row.changed {
                    (
                        Span::styled("Δ ", theme.style(&tokens::DEBUG_CHANGED)),
                        theme.style(&tokens::DEBUG_CHANGED),
                    )
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
                    Span::styled(truncate(&row.value, room, theme), value_style),
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
) {
    let focused = ui.focus == Pane::Breakpoints;
    let block = panel_block(" Breakpoints ", focused, theme);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Breakpoints,
        area,
    });
    ui.hit.breakpoints_inner = Some(block.inner(area));
    let items: Vec<ListItem> = if snapshot.breakpoints.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (none — press b on a source line, B to add by name)",
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
                let cond = bp
                    .condition
                    .as_ref()
                    .map(|c| format!("  if {c}"))
                    .unwrap_or_default();
                let location_name = bp
                    .source_path
                    .as_deref()
                    .and_then(|path| std::path::Path::new(path).file_name())
                    .and_then(|name| name.to_str())
                    .unwrap_or(&bp.qname);
                let location = format!(
                    "{location_name}:{}{}",
                    bp.line
                        .map(|line| line.to_string())
                        .unwrap_or_else(|| bp.offset.to_string()),
                    if bp.pending { " [pending]" } else { "" },
                );
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{glyph} "), glyph_style),
                    Span::styled(bp.id.clone(), theme.detail_value_accent()),
                    Span::styled(format!("  {location}"), theme.detail_value()),
                    Span::styled(format!("  hits={}", bp.hits), theme.detail_label()),
                    Span::styled(cond, theme.variable()),
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
) {
    let focused = ui.focus == Pane::Watches;
    let block = panel_block(" Watches ", focused, theme);
    let inner_width = block.inner(area).width as usize;
    ui.hit.panes.push(PaneHit {
        pane: Pane::Watches,
        area,
    });
    ui.hit.watches_inner = Some(block.inner(area));
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
                let (value_text, value_style) = if let Some(err) = &w.last_error {
                    (format!("→ ! {err}"), theme.error())
                } else if let Some(v) = &w.last_value {
                    let room = inner_width.saturating_sub(cells(&w.name) + 8).max(8);
                    (
                        format!("→ {}", truncate(v, room, theme)),
                        theme.detail_value(),
                    )
                } else {
                    ("→ (unevaluated)".to_string(), theme.empty_state())
                };
                let prefix_width = 2 + cells(&w.name) + 3;
                let total_width = prefix_width + cells(&w.expr) + 1 + cells(&value_text);
                let expression = truncate(
                    &w.expr,
                    inner_width.saturating_sub(prefix_width).max(1),
                    theme,
                );
                let name_line = || {
                    Line::from(vec![
                        Span::styled(changed_text, changed_style),
                        Span::styled(w.name.clone(), theme.variable()),
                        Span::styled(" = ", theme.detail_separator()),
                        Span::styled(expression.clone(), theme.detail_label()),
                    ])
                };
                if total_width <= inner_width {
                    ListItem::new(Line::from(vec![
                        Span::styled(changed_text, changed_style),
                        Span::styled(w.name.clone(), theme.variable()),
                        Span::styled(" = ", theme.detail_separator()),
                        Span::styled(w.expr.clone(), theme.detail_label()),
                        Span::raw(" "),
                        Span::styled(value_text, value_style),
                    ]))
                } else {
                    ListItem::new(vec![
                        name_line(),
                        Line::from(vec![Span::raw("  "), Span::styled(value_text, value_style)]),
                    ])
                }
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
) {
    let focused = ui.focus == Pane::Coverage;
    let block = panel_block(" Coverage ", focused, theme);
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

fn render_events(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    ui: &mut DebugUiState,
    theme: &Theme,
) {
    let focused = ui.focus == Pane::Events;
    let title = if snapshot.event_filter.is_empty() {
        " Events / Log ".to_string()
    } else {
        format!(" Events / Log · /{}", snapshot.event_filter)
    };
    let block = panel_block(&title, focused, theme);
    ui.hit.panes.push(PaneHit {
        pane: Pane::Events,
        area,
    });
    ui.hit.events_inner = Some(block.inner(area));
    let inner_width = block.inner(area).width as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    let take = inner_height.max(1) * 2;
    let filter_lc = snapshot.event_filter.to_lowercase();
    let items: Vec<ListItem> = snapshot
        .events
        .iter()
        .rev()
        .filter(|line| filter_lc.is_empty() || line.text.to_lowercase().contains(&filter_lc))
        .take(take)
        .map(|line| {
            let stamp = format_local_clock(line.when);
            let style = match line.severity {
                EventSeverity::Info => theme.detail_value(),
                EventSeverity::Pause => theme.command_prompt(),
                EventSeverity::Warn => theme.variable(),
                EventSeverity::Error => theme.error(),
            };
            let room = inner_width.saturating_sub(cells(&stamp) + 2).max(8);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{stamp}  "), theme.detail_label()),
                Span::styled(truncate(&line.text, room, theme), style),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selection());
    frame.render_stateful_widget(list, area, &mut ui.selections.events);
}

fn render_hintline(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DebugSnapshot<'_>,
    prompt: Option<&PromptInput>,
    theme: &Theme,
) {
    if let Some(prompt) = prompt {
        let line = Line::from(vec![
            Span::styled(format!(":{}> ", prompt.label), theme.command_prompt()),
            Span::styled(prompt.buffer.clone(), theme.command_input()),
            Span::styled("█", theme.command_prompt()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    // Hints keep their tersest form and the status takes what is left. A status
    // too narrow to read is dropped rather than shown as a stub, matching the
    // explorer.
    const MIN_STATUS_CELLS: usize = 12;
    let total_width = area.width as usize;
    let hints = crate::chrome::fit(
        &[
            crate::chrome::Variant::new().push(
                crate::tokens::STATUS_PILL_INACTIVE,
                super::bindings::compact_hints(),
            ),
            crate::chrome::Variant::new()
                .push(crate::tokens::STATUS_PILL_INACTIVE, " ?:help  q:quit "),
        ],
        total_width.saturating_sub(MIN_STATUS_CELLS),
        theme,
    );

    let mut spans = hints.spans;
    let room = total_width.saturating_sub(hints.width + 2);
    if room >= MIN_STATUS_CELLS
        && let Some(status) = crate::chrome::elide(snapshot.status, room, theme)
    {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(status, theme.detail_value_accent()));
    }
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_help_overlay(frame: &mut Frame<'_>, area: Rect, theme: &Theme, scroll: u16) {
    // Use nearly all of the product-owned viewport to keep the primary
    // execution bindings visible without an initial scroll.
    let popup = centered_rect(75, 95, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(Span::styled(
            " tcld debug — keybindings  (j/k to scroll · q/Esc to dismiss) ",
            theme.status_pill_active(),
        ))
        .borders(Borders::ALL)
        .border_style(theme.border_focused());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines: Vec<Line> = super::bindings::help_lines()
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.detail_value())))
        .collect();
    let max_scroll = lines.len().saturating_sub(inner.height as usize) as u16;
    let scroll = scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}

fn render_picker_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    picker: &super::types::PickerOverlay<'_>,
    theme: &Theme,
) {
    if picker.entries.is_empty() {
        return;
    }
    let height = (picker.entries.len() as u16 + 2).min(area.height.saturating_sub(2));
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
        .border_style(theme.border_focused());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

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

pub fn panel_block<'a>(title: &'a str, focused: bool, theme: &'a Theme) -> Block<'a> {
    let border_style = if focused {
        theme.border_focused()
    } else {
        theme.border_dim()
    };
    let title_style = if focused {
        theme.panel_title_focused()
    } else {
        theme.panel_title()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(title.to_string(), title_style))
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

/// Shorten `s` to `max` terminal cells.
///
/// Delegates to the shared chrome elision, so the debugger measures text the
/// same way the explorer does: in cells rather than characters, breaking at a
/// word boundary where the text allows one.
fn truncate(s: &str, max: usize, theme: &Theme) -> String {
    crate::chrome::elide(s, max, theme).unwrap_or_default()
}

/// Cells occupied by a run of text.
fn cells(value: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(value)
}

fn file_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

fn format_local_clock(when: std::time::SystemTime) -> String {
    // No external chrono dep; show seconds-of-day from the current
    // process to keep the log skim-able. Wallclock would need libc.
    let elapsed = when
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let h = (elapsed / 3600) % 24;
    let m = (elapsed / 60) % 60;
    let s = elapsed % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::debug::{
        AppState, BreakpointKindLabel, DebugSnapshot, DebugUiState, EventLine, FocusedProc,
        LocalRow, PauseKind, PauseReasonLabel, ProcBreakpoint, SourceBlockKind, SourceCommand,
        SourceLine, SourceOrigin, SourceSearch, StackFrame,
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
            source_path: path,
            source_origin: SourceOrigin::UserScript,
            source_qname: None,
            source_lines: lines,
            secondary_source_lines: &[],
            secondary_highlight_line: None,
            focused_proc: focused,
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
        }];
        let locals = vec![LocalRow {
            name: "a".to_string(),
            value: "1".to_string(),
            changed: true,
        }];
        let breakpoints = vec![line_bp("bp0", "demo", 2)];
        let events = vec![EventLine::pause("paused frame=1 bp=bp0")];
        let snapshot = DebugSnapshot {
            stack: &stack,
            locals: &locals,
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
        assert!(text.contains("user script"), "source-origin label missing");
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
        assert!(text.contains("command 2/2: expr @7"), "{text}");
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
        assert!(text.contains("F5 cont") || text.contains("?:help"));
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
        let mut prompt = PromptInput::new("dbg");
        prompt.buffer.push_str("vars 1");
        prompt.cursor = prompt.buffer.len();
        let mut ui = DebugUiState::new();
        let text = render_to_text(&snapshot, &mut ui, Some(&prompt), &theme);
        assert!(
            text.contains(":dbg> vars 1"),
            "prompt line missing the typed input — got:\n{text}"
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
    fn render_narrow_terminal_stacks_panes_vertically() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines: Vec<SourceLine> = vec![SourceLine::raw("puts x")];
        let snapshot = snap("tcld debug", "/tmp/x.tcl", &lines, &[], None, &[]);
        let mut ui = DebugUiState::new();
        let text = render_to_text_sized(&snapshot, &mut ui, None, &theme, 80, 40);
        // In narrow mode, every pane should appear stacked.
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
            EventLine::info("step into"),
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
            !text.contains("step into"),
            "unfiltered event should be hidden — got:\n{text}"
        );
    }
}
