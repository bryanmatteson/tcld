use crate::app::TuiApp;
use crate::data::{DataSource, DecompileRenderContext, ViewId};
use crate::state::{PaneFocus, RailMode};
use crate::views;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::collections::HashSet;

/// Render a document panel for the current view.
pub fn render<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &mut TuiApp<D>) {
    let view = app.current_view();
    render_view(frame, area, app, view);
}

/// Render a document panel for a specific view (used by multi-pane layout).
pub fn render_view<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
    view: ViewId,
) {
    let is_decompile = app.data.decompile_view() == Some(view);
    let show_rail = is_decompile && app.layout.decompile_map_visible && area.width >= 30;

    let (text_area, rail_area) = if show_rail {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    render_text_panel(frame, text_area, app, view, is_decompile);

    if let Some(rail) = rail_area {
        render_rail(frame, rail, app);
    }
}

fn render_text_panel<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
    view: ViewId,
    is_decompile: bool,
) {
    let title = views::title(app.specs(), view);
    let focused = app.layout.focused_pane == PaneFocus::Decompile;

    let border_style = if focused {
        app.theme.border_focused()
    } else {
        app.theme.border()
    };
    let title_style = if focused {
        app.theme.panel_title_focused()
    } else {
        app.theme.panel_title()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(format!(" Canvas - {title} "), title_style));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let document = match app.cached_document(view) {
        Ok(document) => document,
        Err(e) => {
            let err = Paragraph::new(Span::styled(format!("  error: {e}"), app.theme.error()));
            frame.render_widget(err, inner);
            return;
        }
    };

    if document.text.is_empty() {
        let empty = Paragraph::new(Span::styled(
            format!("  {}", app.data.empty_document_message(view)),
            app.theme.empty_state(),
        ));
        frame.render_widget(empty, inner);
        return;
    }

    let scroll = app.nav.doc_scroll(view);

    let rendered = if is_decompile {
        let visible_height = inner.height as usize;
        let rendered = app.data.render_decompile_window(DecompileRenderContext {
            lines: &document.lines,
            anchors: &document.decompile_anchors,
            group_spans: &document.decompile_group_spans,
            document_anchors: &document.anchors,
            focused_group: app.nav.group_focus,
            theme: &app.theme,
            start_line: scroll,
            end_line: scroll + visible_height,
        });
        Text::from(
            rendered
                .lines
                .into_iter()
                .map(|line| fit_line_ellipsis(line, inner.width as usize))
                .collect::<Vec<_>>(),
        )
    } else {
        let start = scroll.min(document.lines.len());
        let end = start
            .saturating_add(inner.height as usize)
            .min(document.lines.len());
        let visible =
            app.data
                .render_document_window(view, &document.lines, &app.theme, start, end);
        Text::from(
            visible
                .lines
                .into_iter()
                .map(|line| fit_line_ellipsis(line, inner.width as usize))
                .collect::<Vec<_>>(),
        )
    };

    let paragraph = Paragraph::new(rendered);
    frame.render_widget(paragraph, inner);
}

fn fit_line_ellipsis<'a>(line: Line<'a>, max_width: usize) -> Line<'a> {
    let total_width = line
        .spans
        .iter()
        .map(|span| cell_width(span.content.as_ref()))
        .sum::<usize>();
    if total_width <= max_width {
        return line;
    }
    if max_width == 0 {
        return Line::from("");
    }
    if max_width == 1 {
        return Line::from("…");
    }

    let limit = max_width - 1;
    let mut cells = Vec::new();
    let mut width = 0;
    for span in line.spans {
        let style = span.style;
        for ch in span.content.chars() {
            let next_width = char_width(ch);
            if width + next_width > limit {
                break;
            }
            cells.push((ch, style));
            width += next_width;
        }
        if width >= limit {
            break;
        }
    }

    while cells.last().is_some_and(|(ch, _)| ch.is_whitespace()) {
        cells.pop();
    }
    let ellipsis_style = cells
        .last()
        .map(|(_, style)| *style)
        .unwrap_or_else(Style::default);
    let mut spans = spans_from_cells(cells);
    spans.push(Span::styled("…", ellipsis_style));
    Line::from(spans)
}

fn cell_width(value: &str) -> usize {
    value.chars().map(char_width).sum()
}

fn char_width(ch: char) -> usize {
    if ch.is_ascii() { 1 } else { 2 }
}

fn spans_from_cells<'a>(cells: Vec<(char, Style)>) -> Vec<Span<'a>> {
    let mut spans: Vec<(String, Style)> = Vec::new();
    for (ch, style) in cells {
        if let Some((text, current_style)) = spans.last_mut()
            && *current_style == style
        {
            text.push(ch);
            continue;
        }
        spans.push((ch.to_string(), style));
    }
    spans
        .into_iter()
        .map(|(text, style)| Span::styled(text, style))
        .collect()
}

// ── Decompile Rail (minimap) ─────────────────────────────────────

fn render_rail<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &mut TuiApp<D>) {
    let inner = area;

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let anchors = app.data.decompile_anchors().unwrap_or_default();
    let group_spans = app.data.decompile_group_spans().unwrap_or_default();

    let decompile_view = app.data.decompile_view().unwrap_or(ViewId(""));
    let line_count = app
        .cached_document(decompile_view)
        .map(|document| document.lines.len())
        .unwrap_or(0);

    if line_count == 0 {
        return;
    }

    let rows = inner.height as usize;
    let scroll = app.nav.doc_scroll(decompile_view);
    let viewport_lines = inner.height as usize;

    let row_map = build_rail_row_map(
        &group_spans,
        &anchors,
        line_count,
        rows,
        app.layout.rail_mode,
    );

    // Record hit test data
    app.hit.decompile_rail_area = Some(inner);
    app.hit.decompile_rail_groups = row_map.iter().map(|g| g.unwrap_or(usize::MAX)).collect();

    // Viewport position
    let denom = line_count.max(1);
    let view_start_row = scroll.saturating_mul(rows) / denom;
    let view_end_row = scroll
        .saturating_add(viewport_lines)
        .min(line_count)
        .saturating_mul(rows)
        / denom;
    let view_end_row = view_end_row.max(view_start_row + 1).min(rows);

    // Below roughly one anchor per row the ticks stop discriminating: every row
    // becomes an anchor and the map renders as a solid bar. Drop them and let the
    // group shading and viewport carry the signal instead.
    let anchor_count = anchors.iter().filter(|&&a| a != usize::MAX).count();
    let dense = anchor_count > rows;
    let anchor_rows: HashSet<usize> = if dense {
        HashSet::new()
    } else {
        anchors
            .iter()
            .copied()
            .filter(|&a| a != usize::MAX)
            .map(|line| line.saturating_mul(rows) / denom)
            .map(|row| row.min(rows.saturating_sub(1)))
            .collect()
    };

    let focus_group = app.nav.group_focus;
    let rail_contrast = app.layout.rail_contrast;

    let mut lines = Vec::with_capacity(rows);
    let mut prev_group = None;
    for (row_idx, group) in row_map.into_iter().enumerate() {
        let in_viewport = view_start_row <= row_idx && row_idx < view_end_row;
        let is_anchor = anchor_rows.contains(&row_idx);
        let is_boundary = !dense && group.is_some() && prev_group.is_some() && group != prev_group;

        // Group column (2 chars)
        let (glyph, style) = if group.is_some() && group == focus_group {
            if is_anchor || is_boundary {
                (
                    if rail_contrast == 0 {
                        "▊▊"
                    } else {
                        "▉▉"
                    },
                    app.theme.accent(),
                )
            } else {
                (
                    if rail_contrast == 0 {
                        "▓▓"
                    } else {
                        "██"
                    },
                    app.theme.accent(),
                )
            }
        } else if let Some(group_idx) = group {
            let (a, b) = match rail_contrast {
                0 => ("░░", "▒▒"),
                1 => ("▒▒", "▓▓"),
                _ => ("▓▓", "██"),
            };
            let pattern = if (row_idx + group_idx) % 2 == 0 { a } else { b };
            if is_anchor {
                ("▌▌", app.theme.gutter_annotated())
            } else if is_boundary {
                ("━━", app.theme.gutter_annotated())
            } else {
                (pattern, app.theme.gutter_annotated())
            }
        } else {
            ("  ", app.theme.gutter_default())
        };

        // Viewport column (1 char)
        let viewport_glyph = if in_viewport {
            if is_anchor {
                "▊"
            } else if rail_contrast == 0 {
                "▐"
            } else {
                "█"
            }
        } else if is_anchor {
            "┃"
        } else if rail_contrast == 2 {
            "▏"
        } else {
            "│"
        };
        let viewport_style = if in_viewport {
            app.theme.accent()
        } else if is_anchor {
            app.theme.gutter_annotated()
        } else {
            app.theme.fg_dim()
        };

        lines.push(Line::from(vec![
            Span::styled(glyph.to_string(), style),
            Span::styled(viewport_glyph.to_string(), viewport_style),
        ]));
        prev_group = group;
    }

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn build_rail_row_map(
    group_spans: &[Option<(usize, usize)>],
    anchors: &[usize],
    line_count: usize,
    rows: usize,
    rail_mode: RailMode,
) -> Vec<Option<usize>> {
    if rows == 0 {
        return Vec::new();
    }

    let mut map = vec![None; rows];
    let denom = line_count.max(1);
    let has_spans = group_spans.iter().any(|s| s.is_some());

    if matches!(rail_mode, RailMode::Spans | RailMode::Adaptive) && has_spans {
        let mut span_rows = 0usize;
        for (group, span) in group_spans.iter().enumerate() {
            let Some((start, end)) = span else {
                continue;
            };
            let start_row = start.saturating_mul(rows) / denom;
            let end_row = (end.saturating_add(1)).saturating_mul(rows) / denom;
            let end_row = end_row.max(start_row + 1).min(rows);
            for slot in map
                .iter_mut()
                .take(end_row)
                .skip(start_row.min(rows.saturating_sub(1)))
            {
                if slot.is_none() {
                    span_rows += 1;
                }
                *slot = Some(group);
            }
        }

        if matches!(rail_mode, RailMode::Adaptive) && span_rows * 3 < rows {
            map.fill(None);
            fill_rail_from_anchors(&mut map, anchors, line_count, rows);
        }
    } else if !anchors.is_empty() {
        fill_rail_from_anchors(&mut map, anchors, line_count, rows);
    }

    // Always project anchor ticks
    for (group, anchor) in anchors.iter().copied().enumerate() {
        if anchor == usize::MAX {
            continue;
        }
        let row = anchor.saturating_mul(rows) / denom;
        let row = row.min(rows.saturating_sub(1));
        map[row] = Some(group);
    }

    map
}

fn fill_rail_from_anchors(
    map: &mut [Option<usize>],
    anchors: &[usize],
    line_count: usize,
    rows: usize,
) {
    if anchors.is_empty() || rows == 0 {
        return;
    }
    let denom = line_count.max(1);
    for (group, start) in anchors.iter().copied().enumerate() {
        if start == usize::MAX {
            continue;
        }
        let end = anchors
            .get(group + 1)
            .copied()
            .filter(|&a| a != usize::MAX)
            .unwrap_or(line_count)
            .saturating_sub(1);
        let start_row = start.saturating_mul(rows) / denom;
        let end_row = (end.saturating_add(1)).saturating_mul(rows) / denom;
        let end_row = end_row.max(start_row + 1).min(rows);
        for slot in map
            .iter_mut()
            .take(end_row)
            .skip(start_row.min(rows.saturating_sub(1)))
        {
            *slot = Some(group);
        }
    }
}
