use crate::app::TuiApp;
use crate::data::DataSource;
use crate::state::ListHitTarget;
use crate::theme::{ACCENT_BAR, GROUP_MARKER, SELECTION_INDICATOR};
use crate::views;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowWindow {
    pub selection: usize,
    pub start: usize,
    pub end: usize,
    pub at_top: bool,
    pub at_bottom: bool,
}

pub fn follow_window(
    total: usize,
    selection: usize,
    previous_start: usize,
    capacity: usize,
) -> FollowWindow {
    if total == 0 || capacity == 0 {
        return FollowWindow {
            selection: 0,
            start: 0,
            end: 0,
            at_top: true,
            at_bottom: true,
        };
    }

    let selection = selection.min(total - 1);
    let max_start = total.saturating_sub(capacity);
    let previous_start = previous_start.min(max_start);
    let start = if selection < previous_start {
        selection
    } else if selection >= previous_start.saturating_add(capacity) {
        selection.saturating_add(1).saturating_sub(capacity)
    } else {
        previous_start
    }
    .min(max_start);
    let end = start.saturating_add(capacity).min(total);

    FollowWindow {
        selection,
        start,
        end,
        at_top: start == 0,
        at_bottom: end == total,
    }
}

/// Render a scrollable list panel with gutter, selection, and group markers.
pub fn render<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
    focused: bool,
) {
    let view = app.current_view();
    let title = views::title(app.specs(), view);
    let total = app.data.list_len(view);
    let selection = app.nav.selection(view).min(total.saturating_sub(1));

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
        .title(Span::styled(
            format!(" Canvas - {title} ({total}) "),
            title_style,
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if total == 0 {
        let empty = Paragraph::new(Span::styled(
            format!("  {}", app.data.empty_list_message(view)),
            app.theme.empty_state(),
        ));
        frame.render_widget(empty, inner);
        return;
    }

    let visible_height = inner.height as usize;
    if visible_height == 0 {
        return;
    }

    let window = follow_window(total, selection, app.nav.list_scroll(view), visible_height);
    app.nav.selections.insert(view, window.selection);
    app.nav.list_scrolls.insert(view, window.start);

    // Record hit area for mouse handling
    app.hit.list_area = Some(inner);
    app.hit.list_view = Some(view);
    app.hit.list_start = window.start;
    app.hit.list_targets.push(ListHitTarget {
        area: inner,
        view,
        start: window.start,
        end: window.end,
    });

    let mut items = Vec::with_capacity(visible_height);

    for i in window.start..window.end {
        let Some(entry) = app.data.list_entry(view, i) else {
            continue;
        };

        let is_selected = i == selection;
        let mut spans = Vec::new();

        // Accent bar
        if is_selected {
            spans.push(Span::styled(ACCENT_BAR, app.theme.accent_bar()));
        } else {
            spans.push(Span::raw(" "));
        }

        // Group marker
        if entry.is_group_start {
            spans.push(Span::styled(
                format!("{GROUP_MARKER} "),
                app.theme.group_start_marker(),
            ));
        } else {
            spans.push(Span::raw("  "));
        }

        // Content — delegate highlighting to the DataSource
        spans.extend(app.data.render_list_entry(view, &entry.text, &app.theme));

        // Selection indicator
        if is_selected {
            spans.push(Span::styled(
                format!(" {SELECTION_INDICATOR}"),
                app.theme.selection_indicator(),
            ));
        }

        let line = Line::from(spans);
        items.push(ListItem::new(line));
    }

    let list = List::new(items);
    frame.render_widget(list, inner);
}
