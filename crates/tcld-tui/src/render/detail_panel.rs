use crate::app::TuiApp;
use crate::data::DataSource;
use crate::state::PaneFocus;
use crate::views;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Paragraph, Wrap};

const COMPACT_DETAIL_ALIAS_WIDTH: usize = 18;

/// Render the detail/context panel (right side in multi-pane layout).
pub fn render<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &TuiApp<D>) {
    let focused = app.layout.focused_pane == PaneFocus::Context;

    // Trace mode: show trace frame info instead of normal detail
    let is_trace = app.nav.trace_mode && app.data.trace_len() > 0;
    let title = match (app.nav.expanded, is_trace) {
        (true, true) => " Inspector · trace · zoomed (z restores) ",
        (true, false) => " Inspector · zoomed (z restores) ",
        (false, true) => " Inspector · trace ",
        (false, false) => " Inspector ",
    };

    let block = super::panel_block(title.to_string(), focused, &app.theme);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if is_trace {
        let view = app.current_view();
        let idx = app
            .nav
            .trace_index
            .unwrap_or_else(|| app.nav.selection(view));
        if let Some(text) = app.data.trace_text(idx) {
            let text = compact_detail_text_for_width(&text, inner.width as usize);
            let lines = app.data.render_detail(&text, &app.theme);
            let paragraph = Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.nav.detail_scroll as u16, 0));
            frame.render_widget(paragraph, inner);
        } else {
            let empty = Paragraph::new(Span::styled(
                "  no trace data for this instruction",
                app.theme.empty_state(),
            ));
            frame.render_widget(empty, inner);
        }
        return;
    }

    let view = app.current_view();
    let detail = if views::is_list_view(app.specs(), view) {
        app.data.list_detail(view, app.nav.selection(view))
    } else {
        app.data
            .document_detail(view, app.nav.doc_cursor(view), app.search.query.as_deref())
    };

    match detail {
        Some(text) if !text.is_empty() => {
            let text = compact_detail_text_for_width(&text, inner.width as usize);
            let lines = app.data.render_detail(&text, &app.theme);
            let paragraph = Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.nav.detail_scroll as u16, 0));
            frame.render_widget(paragraph, inner);
        }
        _ => {
            let empty = Paragraph::new(Span::styled(
                "  select an item to see details",
                app.theme.empty_state(),
            ));
            frame.render_widget(empty, inner);
        }
    }
}

fn compact_detail_text_for_width(text: &str, width: usize) -> String {
    let mut lines = Vec::new();
    let mut source_lines = text.lines().peekable();

    while let Some(line) = source_lines.next() {
        if let Some(compacted) =
            compact_detail_value_block_for_width(line, source_lines.peek().copied(), width)
        {
            lines.push(compacted);
            source_lines.next();
            continue;
        }

        lines.push(compact_detail_line_for_width(line, width));
    }

    lines.join("\n")
}

fn compact_detail_line_for_width(line: &str, width: usize) -> String {
    if width == 0 {
        return line.to_string();
    }

    let Some((key, value)) = line.split_once(':') else {
        return line.to_string();
    };
    let value = value.trim_start();
    if value.is_empty() {
        return line.to_string();
    }

    let Some(short_key) = compact_detail_key(key) else {
        return line.to_string();
    };
    let candidate = format!("{short_key}: {value}");
    let line_width = cell_width(line);
    let should_compact = matches!(
        key,
        "provenance" | "source line" | "stack delta" | "stack depth" | "popped" | "pushed"
    ) || line_width > width
        || width <= COMPACT_DETAIL_ALIAS_WIDTH;
    if should_compact && cell_width(&candidate) <= width {
        candidate
    } else {
        line.to_string()
    }
}

fn compact_detail_value_block_for_width(
    header: &str,
    next_line: Option<&str>,
    width: usize,
) -> Option<String> {
    if width == 0 {
        return None;
    }

    let key = header.strip_suffix(':')?;
    if key.trim() != key || key.contains(' ') {
        return None;
    }
    if key != "content" && width > COMPACT_DETAIL_ALIAS_WIDTH {
        return None;
    }

    let value = next_line?.trim_end();
    if value.trim().is_empty() {
        return None;
    }

    let short_key = compact_detail_key(key)?;
    let prefix = format!("{short_key}: ");
    let prefix_width = cell_width(&prefix);
    if prefix_width >= width {
        return None;
    }

    let value = fit_detail_value(value, width - prefix_width);
    Some(format!("{prefix}{value}"))
}

fn compact_detail_key(key: &str) -> Option<&'static str> {
    match key {
        "content" => Some("text"),
        "provenance" => Some("origin"),
        "source line" => Some("line"),
        "stack delta" => Some("delta"),
        "stack depth" => Some("stack"),
        "popped" => Some("pop"),
        "pushed" => Some("push"),
        _ => None,
    }
}

fn fit_detail_value(value: &str, width: usize) -> String {
    if cell_width(value) <= width {
        return value.to_string();
    }

    if width <= 1 {
        return "…".to_string();
    }

    let mut out = String::new();
    let mut used = 0;
    for ch in value.chars() {
        let ch_width = if ch.is_ascii() { 1 } else { 2 };
        if used + ch_width >= width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }

    let trimmed_len = out.trim_end().len();
    out.truncate(trimmed_len);
    out.push('…');
    out
}

fn cell_width(value: &str) -> usize {
    value
        .chars()
        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
        .sum()
}
