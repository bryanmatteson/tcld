use crate::app::TuiApp;
use crate::bindings::{self, BindingAction};
use crate::command;
use crate::data::{DataSource, OperationAvailability};
use crate::helpers::centered_rect;
use crate::state::format_affordances;
use crate::views;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use std::collections::HashSet;

/// Render the help overlay showing keybindings.
pub fn render_help<D: DataSource>(frame: &mut ratatui::Frame<'_>, viewport: Rect, app: &TuiApp<D>) {
    let area = centered_rect(100, 84, viewport);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border_focused())
        .title(Span::styled(
            " Quick Reference ",
            app.theme.panel_title_focused(),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Capabilities and provenance are fixed for a given artifact, so they belong
    // here rather than occupying a permanent header row.
    let mut lines = vec![Line::from(Span::styled(
        "Artifact",
        app.theme.help_section_title(),
    ))];
    lines.push(Line::from(vec![
        Span::styled("  Source   ", app.theme.help_key()),
        Span::styled(
            app.data
                .header_info()
                .unwrap_or_else(|| "source: unknown".to_string()),
            app.theme.help_desc(),
        ),
    ]));
    let chips = super::capability_chip_labels(&app.data.capability_chips()).join(" · ");
    lines.push(Line::from(vec![
        Span::styled("  Supports ", app.theme.help_key()),
        Span::styled(
            if chips.is_empty() {
                "basic navigation".to_string()
            } else {
                chips
            },
            app.theme.help_desc(),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Modes",
        app.theme.help_section_title(),
    )));
    for mode in app.mode_specs() {
        let key = bindings::chord_for_action(app, &BindingAction::Mode(mode.id))
            .map(|chord| chord.display())
            .unwrap_or_else(|| ":mode".to_string());
        let affordances = available_help_affordances(app, mode.status_affordances);
        lines.push(Line::from(vec![
            Span::styled(format!("  {key:<7} "), app.theme.help_key()),
            Span::styled(
                format!("{} - {}", mode.title, format_affordances(&affordances)),
                app.theme.help_desc(),
            ),
        ]));
    }
    let move_keys = format!(
        "{}/{}",
        binding_key(app, &BindingAction::Navigate(1), "j"),
        binding_key(app, &BindingAction::Navigate(-1), "k")
    );
    let edge_keys = format!(
        "{}/{}",
        binding_key(app, &BindingAction::First, "g"),
        binding_key(app, &BindingAction::Last, "G")
    );
    let page_keys = format!(
        "{}/{}",
        binding_key(app, &BindingAction::Page(-1), "PgUp"),
        binding_key(app, &BindingAction::Page(1), "PgDn")
    );
    let focus_key = binding_key(app, &BindingAction::FocusNext, "Tab");
    let detail_key = binding_key(app, &BindingAction::ToggleInspector, "Enter");
    let search_key = binding_key(app, &BindingAction::Search, "/");
    lines.extend([
        Line::from(""),
        Line::from(Span::styled("Movement", app.theme.help_section_title())),
        Line::from(vec![
            Span::styled(format!("  {move_keys:<9}"), app.theme.help_key()),
            Span::styled("Move", app.theme.help_desc()),
            Span::styled(format!("   {edge_keys:<9}"), app.theme.help_key()),
            Span::styled("Top/bottom", app.theme.help_desc()),
            Span::styled(format!("   {page_keys:<11}"), app.theme.help_key()),
            Span::styled("Page", app.theme.help_desc()),
        ]),
        Line::from(vec![
            Span::styled(format!("  {focus_key:<9}"), app.theme.help_key()),
            Span::styled("Next pane", app.theme.help_desc()),
            Span::styled(format!("  {detail_key:<9}"), app.theme.help_key()),
            Span::styled("Open detail", app.theme.help_desc()),
            Span::styled(format!("  {search_key:<9}"), app.theme.help_key()),
            Span::styled("Search", app.theme.help_desc()),
        ]),
        Line::from(""),
        Line::from(Span::styled("Views", app.theme.help_section_title())),
    ]);

    let available = app.data.available_views();
    let view_items = views::help_specs(app.specs(), available)
        .into_iter()
        .filter_map(|spec| {
            bindings::chord_for_action(app, &BindingAction::View(spec.view))
                .map(|chord| format!("{} {}", chord.display(), spec.title))
        })
        .collect::<Vec<_>>();
    for chunk in view_items.chunks(3) {
        lines.push(Line::from(vec![
            Span::styled("  ", app.theme.help_key()),
            Span::styled(chunk.join("   "), app.theme.help_desc()),
        ]));
    }

    lines.extend([
        Line::from(""),
        Line::from(Span::styled("Commands", app.theme.help_section_title())),
        Line::from(vec![
            Span::styled(
                format!(
                    "  {:<9}",
                    binding_key(app, &BindingAction::Palette, "Ctrl-P")
                ),
                app.theme.help_key(),
            ),
            Span::styled("Open palette", app.theme.help_desc()),
            Span::styled(
                format!("   {:<9}", binding_key(app, &BindingAction::Command, ":")),
                app.theme.help_key(),
            ),
            Span::styled("Enter command", app.theme.help_desc()),
            Span::styled(
                format!("   {:<9}", binding_key(app, &BindingAction::Help, "?")),
                app.theme.help_key(),
            ),
            Span::styled("Toggle help", app.theme.help_desc()),
        ]),
    ]);

    for command in prioritized_help_lines(app) {
        lines.push(Line::from(vec![
            Span::styled("  :        ", app.theme.help_key()),
            Span::styled(command, app.theme.help_desc()),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(
            format!(
                "  {:<9}",
                binding_key(app, &BindingAction::QuitConfirm, "q")
            ),
            app.theme.help_key(),
        ),
        Span::styled("Quit", app.theme.help_desc()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  j/k PgUp/PgDn ", app.theme.help_key()),
        Span::styled("Scroll help   Esc/? Close", app.theme.help_desc()),
    ]));

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((app.overlay.help_scroll.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(paragraph, inner);
}

fn binding_key<D: DataSource>(app: &TuiApp<D>, action: &BindingAction, fallback: &str) -> String {
    bindings::chord_for_action(app, action)
        .map(|chord| chord.display())
        .unwrap_or_else(|| fallback.to_string())
}

fn available_help_affordances<D: DataSource>(
    app: &TuiApp<D>,
    affordances: &'static [&'static str],
) -> Vec<&'static str> {
    affordances
        .iter()
        .copied()
        .filter(|affordance| *affordance != "Status")
        .filter(|affordance| help_affordance_is_available(app, affordance))
        .collect()
}

fn help_affordance_is_available<D: DataSource>(app: &TuiApp<D>, affordance: &str) -> bool {
    let action = match affordance {
        "Command" => Some(BindingAction::Command),
        "Palette" => Some(BindingAction::Palette),
        "Help" => Some(BindingAction::Help),
        "Search" => Some(BindingAction::Search),
        "Trace" => Some(BindingAction::ToggleTrace),
        "Close" | "Diff Close" => Some(BindingAction::CloseDiff),
        "Next" => Some(BindingAction::NavigateGroup(1)),
        "Previous" => Some(BindingAction::NavigateGroup(-1)),
        _ => None,
    };
    let bound = action
        .as_ref()
        .is_none_or(|action| bindings::chord_for_action(app, action).is_some());
    bound
        && match affordance {
            "Trace" => app.data.trace_len() > 0,
            "Close" | "Diff Close" | "Next" | "Previous" => app.diff.active,
            "Export" => help_export_is_available(app),
            _ => true,
        }
}

fn help_export_is_available<D: DataSource>(app: &TuiApp<D>) -> bool {
    matches!(
        app.data.export_availability(),
        OperationAvailability::Available
    )
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

/// Render the quit confirmation modal.
pub fn render_quit_modal<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    viewport: Rect,
    app: &TuiApp<D>,
) {
    let area = centered_rect(30, 10, viewport);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border_focused());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = vec![
        Line::from(""),
        Line::from(Span::styled("  Quit? (y/n)", app.theme.accent())),
    ];
    let paragraph = Paragraph::new(text);
    frame.render_widget(paragraph, inner);
}
