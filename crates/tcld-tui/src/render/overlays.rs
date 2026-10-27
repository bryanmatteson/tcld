use crate::app::TuiApp;
use crate::bindings::{self, BindingAction};
use crate::command;
use crate::data::DataSource;
use crate::helpers::centered_rect;
use crate::views;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use std::collections::HashSet;

/// One titled group of the help overlay: `(keys, description)` rows.
struct HelpSection {
    title: &'static str,
    rows: Vec<(String, String)>,
}

/// Render the help overlay showing keybindings.
///
/// Every key shown is read back from the live binding registry, so help
/// cannot advertise a key that does something else.
pub fn render_help<D: DataSource>(frame: &mut ratatui::Frame<'_>, viewport: Rect, app: &TuiApp<D>) {
    let area = centered_rect(100, 84, viewport);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Thick)
        .border_style(app.theme.border_focused())
        .title(Span::styled(
            " Quick Reference ",
            app.theme.panel_title_focused(),
        ))
        .title_bottom(Line::from(Span::styled(
            " j/k scroll · Esc or ? closes ",
            app.theme.fg_muted(),
        )));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = help_sections(app);
    let two_columns = inner.width >= 100;
    let column_width = if two_columns {
        (inner.width / 2) as usize
    } else {
        inner.width as usize
    };
    let section_lines = |section: &HelpSection| {
        // Each section aligns its own descriptions: one long `:command`
        // should not squeeze every single-key row.
        let keys_width = section
            .rows
            .iter()
            .map(|(keys, _)| keys.chars().count())
            .max()
            .unwrap_or(1)
            .clamp(6, 26);
        let mut lines = vec![Line::from(Span::styled(
            section.title,
            app.theme.help_section_title(),
        ))];
        for (keys, description) in &section.rows {
            let padding = " ".repeat(keys_width.saturating_sub(keys.chars().count()));
            let room = column_width.saturating_sub(keys_width + 5);
            lines.push(Line::from(vec![
                Span::styled(format!("  {keys}"), app.theme.help_key()),
                Span::raw(padding),
                Span::styled(
                    format!("  {}", fit_cells(description, room)),
                    app.theme.help_desc(),
                ),
            ]));
        }
        lines.push(Line::from(""));
        lines
    };

    // Sections never split across columns; the left column takes sections
    // until it holds about half the rows.
    let mut columns: Vec<Vec<Line>> = vec![Vec::new()];
    if two_columns {
        let total: usize = sections.iter().map(|s| s.rows.len() + 2).sum();
        let mut left = Vec::new();
        let mut right = Vec::new();
        for section in &sections {
            let lines = section_lines(section);
            if left.len() < total / 2 {
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
    let max_scroll = content_height.saturating_sub(inner.height as usize);
    let scroll = app
        .overlay
        .help_scroll
        .min(max_scroll)
        .min(u16::MAX as usize) as u16;
    let areas = if two_columns {
        ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                ratatui::layout::Constraint::Percentage(50),
                ratatui::layout::Constraint::Percentage(50),
            ])
            .split(inner)
            .to_vec()
    } else {
        vec![inner]
    };
    for (column, area) in columns.into_iter().zip(areas) {
        frame.render_widget(Paragraph::new(column).scroll((scroll, 0)), area);
    }
}

fn help_sections<D: DataSource>(app: &TuiApp<D>) -> Vec<HelpSection> {
    let key = |action: &BindingAction, fallback: &str| binding_key(app, action, fallback);
    let row = |keys: String, description: &str| (keys, description.to_string());
    let has_groups = app.data.command_group_count() > 0;
    let mut sections = Vec::new();

    let chips = super::capability_chip_labels(&app.data.capability_chips()).join(" · ");
    sections.push(HelpSection {
        title: "This file",
        rows: vec![
            row(
                "Source".to_string(),
                &app.data
                    .header_info()
                    .unwrap_or_else(|| "source: unknown".to_string()),
            ),
            row(
                "Supports".to_string(),
                if chips.is_empty() {
                    "basic navigation"
                } else {
                    &chips
                },
            ),
        ],
    });

    let mut moving = vec![
        row(
            format!(
                "{} / {}",
                key(&BindingAction::Navigate(1), "j"),
                key(&BindingAction::Navigate(-1), "k")
            ),
            "move down / up",
        ),
        row(
            format!(
                "{} / {}",
                key(&BindingAction::First, "g"),
                key(&BindingAction::Last, "G")
            ),
            "first / last",
        ),
        row(
            format!(
                "{} / {}",
                key(&BindingAction::Page(-1), "PgUp"),
                key(&BindingAction::Page(1), "PgDn")
            ),
            "page up / down",
        ),
        row(
            key(&BindingAction::FocusNext, "Tab"),
            "next pane (outline · canvas · inspector)",
        ),
    ];
    if has_groups {
        moving.push(row(
            format!(
                "{} / {}",
                key(&BindingAction::NavigateGroup(-1), "["),
                key(&BindingAction::NavigateGroup(1), "]")
            ),
            "previous / next command group",
        ));
    }
    sections.push(HelpSection {
        title: "Moving around",
        rows: moving,
    });

    let mut links = vec![
        row(
            key(&BindingAction::Follow, "Enter"),
            "jump to what the selection refers to",
        ),
        row(
            format!(
                "Bksp / {}",
                bindings::chord_for_action(app, &BindingAction::Back)
                    .filter(|chord| chord.code != crossterm::event::KeyCode::Backspace)
                    .map(|chord| chord.display())
                    .unwrap_or_else(|| "Ctrl-o".to_string())
            ),
            "go back to where you were",
        ),
    ];
    if has_groups {
        links.push(row(
            key(&BindingAction::RevealDecompile, "v"),
            "show the selection in Decompile",
        ));
    }
    sections.push(HelpSection {
        title: "Following links",
        rows: links,
    });

    let available = app.data.available_views();
    let views = views::help_specs(app.specs(), available)
        .into_iter()
        .filter_map(|spec| {
            bindings::chord_for_action(app, &BindingAction::View(spec.view))
                .map(|chord| row(chord.display(), spec.title))
        })
        .collect::<Vec<_>>();
    if !views.is_empty() {
        sections.push(HelpSection {
            title: "Views",
            rows: views,
        });
    }

    sections.push(HelpSection {
        title: "Searching",
        rows: vec![
            row(key(&BindingAction::Search, "/"), "search this view"),
            row(
                format!(
                    "{} / {}",
                    key(&BindingAction::SearchHit(1), "n"),
                    key(&BindingAction::SearchHit(-1), "N")
                ),
                "next / previous match",
            ),
            row(key(&BindingAction::Escape, "Esc"), "clear the search"),
        ],
    });

    sections.push(HelpSection {
        title: "Inspector",
        rows: vec![
            row(
                key(&BindingAction::ToggleInspector, "z"),
                "zoom the inspector to full size",
            ),
            row(
                format!(
                    "{} / {}",
                    key(&BindingAction::ScrollDetail(1), "Ctrl-j"),
                    key(&BindingAction::ScrollDetail(-1), "Ctrl-k")
                ),
                "scroll the inspector",
            ),
        ],
    });

    if app.data.decompile_view().is_some() {
        sections.push(HelpSection {
            title: "Decompile map",
            rows: vec![
                row(key(&BindingAction::ToggleMap, "m"), "show or hide the map"),
                row(
                    key(&BindingAction::CycleMap, "M"),
                    "anchors, spans, or adaptive",
                ),
                row(
                    format!(
                        "{} / {}",
                        key(&BindingAction::AdjustMapContrast(1), "="),
                        key(&BindingAction::AdjustMapContrast(-1), "-")
                    ),
                    "more / less contrast",
                ),
            ],
        });
    }

    if app.data.trace_len() > 0 {
        sections.push(HelpSection {
            title: "Trace",
            rows: vec![row(
                key(&BindingAction::ToggleTrace, "t"),
                "show the stack effect of each instruction",
            )],
        });
    }

    let modes = app
        .mode_specs()
        .iter()
        .map(|mode| {
            let keys = bindings::chord_for_action(app, &BindingAction::Mode(mode.id))
                .map(|chord| chord.display())
                .unwrap_or_else(|| ":mode".to_string());
            let description = match mode.unavailable_message {
                Some(_) if !app.diff.active => format!("{} (needs :diff <path>)", mode.title),
                _ => mode.title.to_string(),
            };
            (keys, description)
        })
        .collect();
    sections.push(HelpSection {
        title: "Modes",
        rows: modes,
    });

    sections.push(HelpSection {
        title: "General",
        rows: vec![
            row(key(&BindingAction::Palette, "Ctrl-p"), "command palette"),
            row(key(&BindingAction::Command, ":"), "command line"),
            row(key(&BindingAction::Help, "?"), "show or hide this help"),
            row("q / Ctrl-c".to_string(), "quit (your place is saved)"),
        ],
    });

    let commands = prioritized_help_lines(app)
        .into_iter()
        .map(|line| match line.split_once("  ") {
            Some((command, description)) => (
                format!(":{}", command.trim()),
                description.trim().to_string(),
            ),
            None => (format!(":{}", line.trim()), String::new()),
        })
        .collect();
    sections.push(HelpSection {
        title: "Commands",
        rows: commands,
    });
    sections
}

/// Keep at most `max` cells of `value`, ending with `…` when cut.
fn fit_cells(value: &str, max: usize) -> String {
    if unicode_width::UnicodeWidthStr::width(value) <= max {
        return value.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for character in value.chars() {
        let wide = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if used + wide + 1 > max {
            break;
        }
        out.push(character);
        used += wide;
    }
    out.push('…');
    out
}

fn binding_key<D: DataSource>(app: &TuiApp<D>, action: &BindingAction, fallback: &str) -> String {
    bindings::chord_for_action(app, action)
        .map(|chord| chord.display())
        .unwrap_or_else(|| fallback.to_string())
}

fn prioritized_help_lines<D: DataSource>(app: &TuiApp<D>) -> Vec<&'static str> {
    let mut seen = HashSet::new();
    let mut lines = app
        .extra_modules
        .iter()
        .chain(command::command_modules().iter())
        .flat_map(|module| module.help_lines.iter().copied())
        .filter(|line| seen.insert(*line))
        .enumerate()
        .collect::<Vec<_>>();
    lines.sort_by_key(|(index, line)| (command::help_line_priority(line), *index));
    lines.into_iter().map(|(_, line)| line).collect()
}

/// Render the command palette (fuzzy search overlay).
pub fn render_palette<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    viewport: Rect,
    app: &TuiApp<D>,
) {
    let area = centered_rect(50, 40, viewport);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border_focused())
        .title(Span::styled(
            " Command Palette ",
            app.theme.panel_title_focused(),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let entries = app.palette_entries();

    if inner.height < 2 {
        return;
    }

    // Input line
    let query_label = if app.palette.query.is_empty() {
        Span::styled("type to search actions", app.theme.palette_hint())
    } else {
        Span::styled(app.palette.query.clone(), app.theme.command_input())
    };
    let count_label = if app.palette.query.is_empty() {
        format!("{} available", entries.len())
    } else {
        format!("{} matches", entries.len())
    };
    let input_line = Line::from(vec![
        Span::styled("Filter: ", app.theme.command_prompt()),
        query_label,
        Span::styled(" | ", app.theme.palette_hint()),
        Span::styled(count_label, app.theme.palette_hint()),
    ]);
    let input_area = Rect { height: 1, ..inner };
    frame.render_widget(Paragraph::new(input_line), input_area);

    // Results
    let results_area = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(2),
        ..inner
    };
    let selected_index = app.palette.index.min(entries.len().saturating_sub(1));
    let result_capacity = results_area.height as usize;
    let first_index = if result_capacity == 0 || entries.is_empty() {
        0
    } else {
        let max_start = entries.len().saturating_sub(result_capacity);
        selected_index
            .saturating_add(1)
            .saturating_sub(result_capacity)
            .min(max_start)
    };
    let last_index = first_index
        .saturating_add(result_capacity)
        .min(entries.len());

    let items: Vec<ListItem> = if entries.is_empty() {
        vec![ListItem::new(Line::from(vec![
            Span::styled("  No matching actions", app.theme.fg_muted()),
            Span::styled(" - edit the filter or press Esc", app.theme.palette_hint()),
        ]))]
    } else {
        entries
            .iter()
            .enumerate()
            .skip(first_index)
            .take(result_capacity)
            .map(|(i, entry)| {
                let selected = i == selected_index;
                let style = if selected {
                    app.theme.palette_selected()
                } else {
                    app.theme.fg()
                };
                let marker = if selected { ">" } else { " " };
                let mut spans = vec![Span::styled(format!("{marker} {}", entry.label), style)];
                if !entry.hint.is_empty() {
                    spans.push(Span::styled(
                        format!("  {}", entry.hint),
                        app.theme.palette_hint(),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let list = List::new(items);
    frame.render_widget(list, results_area);

    let showing = if entries.is_empty() {
        "Showing 0 of 0".to_string()
    } else {
        format!(
            "Showing {}-{} of {}",
            first_index + 1,
            last_index,
            entries.len()
        )
    };
    let footer = Line::from(Span::styled(
        palette_footer_text(!entries.is_empty(), &showing, inner.width as usize),
        app.theme.palette_hint(),
    ));
    let footer_area = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    frame.render_widget(Paragraph::new(footer), footer_area);
}

fn palette_footer_text(has_entries: bool, showing: &str, width: usize) -> String {
    let candidates = if has_entries {
        vec![
            format!("↑/↓: move | Enter: run | Esc: close | {showing}"),
            format!("Enter: run | Esc: close | {showing}"),
            format!("Enter | Esc | {showing}"),
        ]
    } else {
        vec![
            format!("Esc: close | {showing}"),
            format!("Esc | {showing}"),
        ]
    };

    candidates
        .iter()
        .find(|candidate| candidate.chars().count() <= width)
        .cloned()
        .unwrap_or_else(|| {
            truncate_ascii_footer(candidates.last().expect("footer candidate"), width)
        })
}

fn truncate_ascii_footer(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 3 {
        return text.chars().take(width).collect();
    }

    let mut truncated = text.chars().take(width - 3).collect::<String>();
    truncated.push_str("...");
    truncated
}

/// Render the summary modal.
pub fn render_summary_modal<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    viewport: Rect,
    app: &TuiApp<D>,
) {
    let area = centered_rect(70, 60, viewport);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border_focused())
        .title(Span::styled(" Summary ", app.theme.panel_title_focused()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = app.data.summary_text();
    let lines = crate::style::plain_lines(&text, &app.theme);
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((app.overlay.summary_scroll as u16, 0));
    frame.render_widget(paragraph, inner);
}
