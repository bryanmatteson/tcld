use crate::app::TuiApp;
use crate::data::DataSource;
use crate::views;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::Path;

/// Render the file header row: `tcld | filename | counts`.
pub fn render_file_header<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &TuiApp<D>,
) {
    let file_name = Path::new(app.data.path())
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(app.data.path());

    let mut spans = Vec::new();
    spans.push(Span::styled(" tcld ", app.theme.accent()));
    spans.push(Span::styled("│ ", app.theme.fg_dim()));
    spans.push(Span::styled(file_name.to_string(), app.theme.fg()));

    if let Some(info) = app.data.header_info() {
        spans.push(Span::styled(" │ ", app.theme.fg_dim()));
        spans.push(Span::styled(info, app.theme.fg_muted()));
    }
    spans.push(Span::styled(" │ ", app.theme.fg_dim()));
    spans.push(Span::styled(
        format!("theme: {}", app.theme.name()),
        app.theme.accent(),
    ));
    let chips = app.data.capability_chips();
    if !chips.is_empty() {
        spans.push(Span::styled(" │ Capabilities: ", app.theme.fg_dim()));
        spans.push(Span::styled(chips.join(" "), app.theme.accent()));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(app.theme.header_bg());
    frame.render_widget(paragraph, area);
}

/// Render the tab bar (2 rows: labels + active underline).
pub fn render_tab_bar<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &TuiApp<D>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let available = app.data.available_views();
    let tabs = views::tab_specs(app.specs(), available);
    let current = app.current_view();

    // Row 1: Tab labels
    let mut spans = Vec::new();
    spans.push(Span::styled(" Tabs: ", app.theme.fg_dim()));

    // Track positions for the underline
    let mut tab_positions: Vec<(usize, usize, bool)> = Vec::new(); // (start_col, width, active)
    let mut col = 1usize; // start after the leading space

    for (i, spec) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", app.theme.fg_dim()));
            col += 2;
        }

        let icon = spec.icon;
        let is_active = spec.view == current;

        let style = if is_active {
            app.theme.tab_active()
        } else {
            app.theme.tab_inactive()
        };

        let label = format!("{icon} {}", spec.title);
        let label_width = label.chars().count();
        spans.push(Span::styled(label, style));

        let mut total_width = label_width;
        if let Some(key) = spec.keybind {
            let hint = format!(" [{key}]");
            total_width += hint.chars().count();
            spans.push(Span::styled(hint, app.theme.fg_dim()));
        }

        tab_positions.push((col, total_width, is_active));
        col += total_width;
    }

    let labels_line = Line::from(spans);
    frame.render_widget(Paragraph::new(labels_line), rows[0]);

    // Row 2: Underline beneath active tab
    let width = rows[1].width as usize;
    let mut underline_chars = vec![' '; width];
    for (start, tab_width, is_active) in &tab_positions {
        if *is_active {
            for ch in underline_chars
                .iter_mut()
                .take((*start + *tab_width).min(width))
                .skip(*start)
            {
                *ch = '━';
            }
        }
    }
    let underline_str: String = underline_chars.into_iter().collect();
    let underline_line = Line::from(Span::styled(underline_str, app.theme.tab_active()));
    frame.render_widget(Paragraph::new(underline_line), rows[1]);
}
