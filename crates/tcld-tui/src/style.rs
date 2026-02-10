use crate::data::DocumentAnchor;
use crate::theme::Theme;
use ratatui::text::{Line, Span, Text};

/// Highlight a bytecode instruction line: `(pc) opcode operands  # comment`
pub fn highlight_instruction_text(text: &str, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let (before_comment, comment) = split_comment(text);

    if let Some(paren_idx) = before_comment.find(')') {
        let opcode_start = before_comment[paren_idx + 1..]
            .char_indices()
            .find_map(|(i, ch)| {
                if ch.is_whitespace() {
                    None
                } else {
                    Some(paren_idx + 1 + i)
                }
            });

        if let Some(start) = opcode_start {
            let opcode_end = before_comment[start..]
                .char_indices()
                .find_map(|(i, ch)| {
                    if ch.is_whitespace() {
                        Some(start + i)
                    } else {
                        None
                    }
                })
                .unwrap_or(before_comment.len());

            spans.push(Span::styled(
                before_comment[..start].to_string(),
                theme.inst_pc(),
            ));
            spans.push(Span::styled(
                before_comment[start..opcode_end].to_string(),
                theme.inst_opcode(),
            ));
            let rest = &before_comment[opcode_end..];
            if !rest.is_empty() {
                let trimmed = rest.trim_start();
                let leading_space = rest.len() - trimmed.len();
                if leading_space > 0 {
                    spans.push(Span::raw(rest[..leading_space].to_string()));
                }
                if let Some(num_end) = trimmed.find(|c: char| !c.is_ascii_digit() && c != '-') {
                    if num_end > 0 {
                        spans.push(Span::styled(trimmed[..num_end].to_string(), theme.number()));
                        spans.push(Span::styled(trimmed[num_end..].to_string(), theme.fg()));
                    } else {
                        spans.push(Span::styled(trimmed.to_string(), theme.fg()));
                    }
                } else if !trimmed.is_empty() {
                    spans.push(Span::styled(trimmed.to_string(), theme.number()));
                }
            }
        } else {
            spans.push(Span::styled(before_comment.to_string(), theme.fg()));
        }
    } else {
        spans.push(Span::styled(before_comment.to_string(), theme.fg()));
    }

    if let Some(comment_text) = comment {
        spans.push(Span::styled(comment_text.to_string(), theme.comment()));
    }

    spans
}

/// Highlight a source list line like `  42: set x [expr {$a + 1}]`
pub fn highlight_source_list_text(text: &str, theme: &Theme) -> Line<'static> {
    let Some((prefix, body)) = text.split_once(':') else {
        return Line::from(text.to_string());
    };
    let mut spans = Vec::new();
    spans.push(Span::styled(format!("{}:", prefix), theme.fg_dim()));
    spans.push(Span::raw(" ".to_string()));
    spans.extend(highlight_tcl_line(body.trim_start(), theme).spans);
    Line::from(spans)
}

/// Render plain text lines with the default foreground color.
pub fn plain_lines(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| Line::from(vec![Span::styled(line.to_string(), theme.fg())]))
        .collect()
}

/// Highlight detail text (key: value pairs with section headers).
pub fn highlight_detail_text(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_value_block = false;

    for raw_line in text.lines() {
        if raw_line.trim().is_empty() {
            in_value_block = false;
            lines.push(Line::from(String::new()));
            continue;
        }

        if raw_line.trim().ends_with(':') && !raw_line.trim().contains(' ') {
            in_value_block = true;
            lines.push(Line::from(vec![Span::styled(
                raw_line.to_string(),
                theme.detail_section(),
            )]));
            continue;
        }

        if in_value_block {
            lines.push(highlight_tcl_line(raw_line, theme));
            continue;
        }

        if let Some(colon_pos) = raw_line.find(':') {
            let key = &raw_line[..colon_pos];
            let rest = &raw_line[colon_pos + 1..];
            let value = rest.trim();

            let mut spans = Vec::new();
            spans.push(Span::styled(format!("{key}:"), theme.detail_label()));

            if value.is_empty() {
                lines.push(Line::from(spans));
                continue;
            }

            spans.push(Span::raw(" "));

            if value.starts_with("c")
                && value.len() >= 4
                && value[1..].chars().all(|c| c.is_ascii_digit())
            {
                spans.push(Span::styled(value.to_string(), theme.detail_group_ref()));
            } else if value.contains("..") {
                let parts: Vec<&str> = value.splitn(2, "..").collect();
                if parts.len() == 2 {
                    spans.push(Span::styled(parts[0].to_string(), theme.number()));
                    spans.push(Span::styled("..".to_string(), theme.detail_label()));
                    spans.push(Span::styled(parts[1].to_string(), theme.number()));
                } else {
                    spans.push(Span::styled(value.to_string(), theme.detail_value()));
                }
            } else if value == "true" || value == "false" || value == "n/a" {
                let style = match value {
                    "true" => theme.success(),
                    "false" => theme.detail_label(),
                    _ => theme.fg_dim(),
                };
                spans.push(Span::styled(value.to_string(), style));
            } else if value.contains('=') && !value.contains(' ') {
                if let Some(eq_pos) = value.find('=') {
                    spans.push(Span::styled(
                        value[..eq_pos + 1].to_string(),
                        theme.detail_label(),
                    ));
                    spans.push(Span::styled(
                        value[eq_pos + 1..].to_string(),
                        theme.detail_value(),
                    ));
                }
            } else if value.contains('=') {
                let mut first = true;
                for token in value.split_whitespace() {
                    if !first {
                        spans.push(Span::raw(" "));
                    }
                    first = false;
                    if let Some(eq_pos) = token.find('=') {
                        spans.push(Span::styled(
                            token[..eq_pos + 1].to_string(),
                            theme.detail_label(),
                        ));
                        let val = &token[eq_pos + 1..];
                        let style = match val {
                            "true" => theme.success(),
                            "false" => theme.detail_label(),
                            _ => theme.detail_value(),
                        };
                        spans.push(Span::styled(val.to_string(), style));
                    } else {
                        spans.push(Span::styled(token.to_string(), theme.detail_value()));
                    }
                }
            } else if value.starts_with('[') || value.starts_with("Some(") {
                spans.push(Span::styled(value.to_string(), theme.fg_muted()));
            } else if value.chars().all(|c| c.is_ascii_digit() || c == '-') && !value.is_empty() {
                spans.push(Span::styled(value.to_string(), theme.number()));
            } else {
                spans.push(Span::styled(value.to_string(), theme.detail_value_accent()));
            }

            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(vec![Span::styled(
                raw_line.to_string(),
                theme.fg(),
            )]));
        }
    }
    lines
}

/// Highlight a single Tcl line lexically.
///
/// Whole documents are highlighted by the host's Tree-sitter parse, which sees
/// the surrounding context. This remains for the two places that have no
/// document to parse: a list row showing one line out of context, and an
/// inspector fragment. Both emit the same semantic tokens as the parser, so the
/// two paths cannot disagree about what a string or a comment looks like.
pub fn highlight_tcl_line(line: &str, theme: &Theme) -> Line<'static> {
    if line.trim_start().starts_with('#') {
        return Line::from(vec![Span::styled(line.to_string(), theme.tcl_comment())]);
    }

    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut idx = 0usize;
    let mut command_colored = false;

    while idx < bytes.len() {
        let ch = bytes[idx] as char;

        if ch.is_whitespace() {
            let start = idx;
            idx += 1;
            while idx < bytes.len() && (bytes[idx] as char).is_whitespace() {
                idx += 1;
            }
            spans.push(Span::raw(line[start..idx].to_string()));
            continue;
        }

        if ch == '#' {
            spans.push(Span::styled(line[idx..].to_string(), theme.tcl_comment()));
            break;
        }

        if ch == '$' {
            let start = idx;
            idx += 1;
            while idx < bytes.len() {
                let c = bytes[idx] as char;
                if c.is_alphanumeric() || matches!(c, '_' | ':' | '(' | ')') {
                    idx += 1;
                } else {
                    break;
                }
            }
            spans.push(Span::styled(
                line[start..idx].to_string(),
                theme.tcl_variable(),
            ));
            continue;
        }

        if ch == '"' {
            let start = idx;
            idx += 1;
            while idx < bytes.len() {
                let c = bytes[idx] as char;
                idx += 1;
                if c == '\\' {
                    idx = (idx + 1).min(bytes.len());
                    continue;
                }
                if c == '"' {
                    break;
                }
            }
            spans.push(Span::styled(
                line[start..idx].to_string(),
                theme.tcl_string(),
            ));
            continue;
        }

        if matches!(ch, '{' | '}' | '[' | ']' | '(' | ')') {
            spans.push(Span::styled(ch.to_string(), theme.tcl_brace()));
            idx += 1;
            continue;
        }

        let start = idx;
        idx += 1;
        while idx < bytes.len() {
            let c = bytes[idx] as char;
            if c.is_whitespace() || matches!(c, '#' | '$' | '"' | '{' | '}' | '[' | ']' | '(' | ')')
            {
                break;
            }
            idx += 1;
        }
        let token = &line[start..idx];

        let style = if !command_colored {
            command_colored = true;
            theme.tcl_command()
        } else if token
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
        {
            theme.tcl_number()
        } else {
            theme.fg()
        };

        spans.push(Span::styled(token.to_string(), style));
    }

    Line::from(spans)
}

/// Build a window of decompiled text with a group gutter.
// Rendering helper fed directly from view state; its callers already hold
// these values individually, so a params struct adds indirection without reuse.
#[allow(clippy::too_many_arguments)]
/// Build the decompile view: a group gutter beside highlighted content.
///
/// The gutter is generic and lives here; how a line's content is highlighted is
/// the host's business, so it is supplied as `highlight`. A host with a real
/// parser passes one that consults it, rather than this module guessing at Tcl
/// a second time and disagreeing with the parse elsewhere in the interface.
pub fn build_decompile_text_window(
    lines: &[String],
    anchors: &[usize],
    group_spans: &[Option<(usize, usize)>],
    document_anchors: &[DocumentAnchor],
    focused_group: Option<usize>,
    theme: &Theme,
    start_line: usize,
    end_line: usize,
    highlight: &dyn Fn(usize) -> Line<'static>,
) -> Text<'static> {
    if lines.is_empty() {
        return Text::from(vec![Line::from(String::new())]);
    }

    let start = start_line.min(lines.len().saturating_sub(1));
    let end = end_line.max(start + 1).min(lines.len());
    let mut rendered = Vec::with_capacity(end.saturating_sub(start));
    let mut anchor_labels = vec![None; lines.len()];
    for (group, anchor) in anchors.iter().copied().enumerate() {
        if anchor == usize::MAX || anchor >= lines.len() {
            continue;
        }
        anchor_labels[anchor] = Some(group);
    }
    let mut document_anchor_labels = vec![None::<String>; lines.len()];
    for anchor in document_anchors {
        let line = anchor.line_span.0;
        if line >= lines.len() {
            continue;
        }
        document_anchor_labels[line] = Some(anchor.display_path.clone());
    }
    // A ragged gutter starts the code at a different column on every row. Size it
    // once for the whole window so the code itself stays in a straight line.
    let gutter_width = (start..end)
        .map(
            |idx| match (document_anchor_labels[idx].as_deref(), anchor_labels[idx]) {
                (Some(path), _) => path.chars().count(),
                (None, Some(_)) => 4,
                (None, None) => 0,
            },
        )
        .max()
        .unwrap_or(4)
        .max(4);

    for idx in start..end {
        let line_group = group_for_line(idx, anchors, group_spans);
        let anchor_group = anchor_labels[idx];
        let document_anchor = document_anchor_labels[idx].as_deref();

        let mut spans = Vec::new();
        let label = match (document_anchor, anchor_group) {
            (Some(path), _) => path.to_string(),
            (None, Some(group)) => format!("c{group:03}"),
            (None, None) => String::new(),
        };
        let prefix = format!("{label:>gutter_width$}│ ");

        let is_focused_label = anchor_group.is_some() && anchor_group == focused_group;
        let is_in_focused_group = line_group.is_some() && line_group == focused_group;

        // Addresses annotate the code; they must not compete with it. Only the
        // group you are actually in is emphasized — every other label recedes.
        let prefix_style = if is_focused_label {
            theme.gutter_focused()
        } else if is_in_focused_group {
            theme.gutter_annotated()
        } else {
            theme.gutter_default()
        };
        spans.push(Span::styled(prefix, prefix_style));

        let mut content = highlight(idx).spans;
        if is_in_focused_group && !is_focused_label {
            let bg_style = theme.focused_line_bg();
            for span in &mut content {
                if let Some(bg) = bg_style.bg {
                    span.style = span.style.bg(bg);
                }
            }
        }

        spans.extend(content);
        rendered.push(Line::from(spans));
    }

    Text::from(rendered)
}

/// Determine which command group a decompile line belongs to.
pub fn group_for_line(
    line_idx: usize,
    anchors: &[usize],
    group_spans: &[Option<(usize, usize)>],
) -> Option<usize> {
    let mut best_span_group = None;
    let mut best_span_start = 0usize;
    for (group, span) in group_spans.iter().enumerate() {
        let Some((start, end)) = span else {
            continue;
        };
        if *start <= line_idx
            && line_idx <= *end
            && (best_span_group.is_none() || *start >= best_span_start)
        {
            best_span_group = Some(group);
            best_span_start = *start;
        }
    }
    if best_span_group.is_some() {
        return best_span_group;
    }

    if anchors.is_empty() {
        return None;
    }

    let mut current = None;
    for (group, anchor) in anchors.iter().enumerate() {
        if *anchor <= line_idx {
            current = Some(group);
        } else {
            break;
        }
    }
    current
}

/// Build a plain-text decompile window with no group gutter.
/// Used as the default `DataSource::render_decompile_window` implementation.
pub fn build_plain_decompile_window(
    lines: &[String],
    start_line: usize,
    end_line: usize,
    theme: &Theme,
) -> Text<'static> {
    if lines.is_empty() {
        return Text::from(vec![Line::from(String::new())]);
    }
    let start = start_line.min(lines.len().saturating_sub(1));
    let end = end_line.max(start + 1).min(lines.len());
    let rendered: Vec<Line> = lines[start..end]
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), theme.fg())))
        .collect();
    Text::from(rendered)
}

fn split_comment(line: &str) -> (&str, Option<&str>) {
    if let Some(idx) = line.find(" #") {
        (&line[..idx], Some(&line[idx..]))
    } else {
        (line, None)
    }
}
