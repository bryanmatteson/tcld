pub mod detail_panel;
pub mod document_panel;
pub mod list_panel;
pub mod overlays;
pub mod status_bar;
pub mod tab_bar;

use crate::app::TuiApp;
use crate::chrome;
use crate::data::DataSource;
use crate::state::{HitRegion, ModeLayoutProfile, PaneFocus};
use crate::tokens;
use crate::views;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use std::path::Path;

pub const MIN_TERMINAL_WIDTH: u16 = 72;
pub const MIN_TERMINAL_HEIGHT: u16 = 20;

/// Top-level draw function: computes the active mode shell and delegates to sub-renderers.
pub fn draw<D: DataSource>(frame: &mut ratatui::Frame<'_>, app: &mut TuiApp<D>) {
    draw_in(frame, frame.area(), app);
}

/// Draw the explorer inside a host-provided surface rectangle.
pub fn draw_in<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &mut TuiApp<D>) {
    reset_hit_state(app, area);

    let terminal_area = frame.area();
    if terminal_area.width < MIN_TERMINAL_WIDTH || terminal_area.height < MIN_TERMINAL_HEIGHT {
        let message = format!(
            "tcld explore\n\nTerminal too small: {}x{}\nRequired: {}x{}\n\nCtrl-C exit | q quit",
            terminal_area.width, terminal_area.height, MIN_TERMINAL_WIDTH, MIN_TERMINAL_HEIGHT
        );
        frame.render_widget(
            Paragraph::new(message)
                .style(app.theme.fg())
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    match app.current_layout_profile() {
        ModeLayoutProfile::Browse => {
            render_product_header(frame, outer[0], app);
            render_browse_workspace(frame, outer[1], app);
            render_mode_footer(frame, outer[2], app, "Command");
        }
        ModeLayoutProfile::Compare => {
            render_compare_header(frame, outer[0], app);
            render_compare_workspace(frame, outer[1], app);
            render_mode_footer(frame, outer[2], app, "Diff command");
        }
    }

    // Overlays (drawn last, on top of everything)
    if app.overlay.show_help {
        overlays::render_help(frame, area, app);
    }
    if app.palette.active {
        overlays::render_palette(frame, area, app);
    }
    if app.overlay.show_summary {
        overlays::render_summary_modal(frame, area, app);
    }
    if app.overlay.quit_confirm {
        overlays::render_quit_modal(frame, area, app);
    }
}

fn reset_hit_state<D: DataSource>(app: &mut TuiApp<D>, area: Rect) {
    app.hit.last_frame_area = area;
    app.hit.mode_switches.clear();
    app.hit.regions.clear();
    app.hit.list_area = None;
    app.hit.list_view = None;
    app.hit.list_start = 0;
    app.hit.list_targets.clear();
    app.hit.decompile_text_area = None;
    app.hit.decompile_rail_area = None;
    app.hit.decompile_rail_groups.clear();
}

fn render_product_header<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
) {
    render_header_lines(frame, area, app, "tcld explore");
}

fn render_compare_header<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
) {
    render_header_lines(frame, area, app, "tcld explore");
}

fn render_header_lines<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
    title: &str,
) {
    let row = Rect::new(area.x, area.y, area.width, area.height.min(1));
    let width = row.width as usize;
    let file_name = Path::new(app.data.path())
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(app.data.path())
        .to_string();
    let info = app
        .data
        .header_info()
        .unwrap_or_else(|| "source: unknown".to_string())
        .replace(" | ", " \u{b7} ");

    let active_mode = app.current_mode_spec().id;
    let modes = app
        .data
        .mode_specs()
        .iter()
        .map(|spec| (spec.id, mode_switch_label(spec)))
        .collect::<Vec<_>>();

    // Identity is fixed cost; the mode chips carry both the current mode and its
    // click target, so they replace the separate `Mode:` and `Modes:` rows.
    let identity = || {
        chrome::Variant::new()
            .push(tokens::ACCENT, format!(" {title} "))
            .push(tokens::FG_DIM, "\u{2502} ")
            .push(tokens::FG, file_name.clone())
            .push(tokens::FG_DIM, "  \u{2502}  ")
    };
    let with_modes = |mut variant: chrome::Variant| {
        for (id, label) in &modes {
            let token = if *id == active_mode {
                tokens::TAB_ACTIVE
            } else {
                tokens::TAB_INACTIVE
            };
            variant = variant.push(token, label.clone()).push(tokens::FG, "  ");
        }
        variant
    };
    let with_info = |variant: chrome::Variant| {
        variant
            .push(tokens::FG_DIM, "\u{2502}  ")
            .push(tokens::FG_MUTED, info.clone())
    };

    // Provenance is the first thing to lose when the terminal is narrow, then
    // the mode chips, and the artifact's identity last.
    let header = chrome::fit(
        &[
            with_info(with_modes(identity())),
            with_modes(identity()),
            identity(),
        ],
        width,
        &app.theme,
    );

    // Chip positions are read back from the laid-out row: Laidout decides which
    // phrasing survives, so the caller cannot know where a chip landed — or
    // whether it survived at all — until the row exists.
    for (id, label) in &modes {
        if let Some(column) = chrome::column_of(&header.spans, label) {
            app.hit.mode_switches.push((
                id.to_owned(),
                Rect::new(
                    row.x.saturating_add(column as u16),
                    row.y,
                    cell_width(label) as u16,
                    1,
                ),
            ));
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(header.spans)).style(app.theme.header_bg()),
        row,
    );
}

pub(crate) fn capability_chip_labels(chips: &[String]) -> Vec<&str> {
    chips
        .iter()
        .map(|chip| capability_chip_label(chip))
        .collect()
}

fn capability_chip_label(chip: &str) -> &str {
    match chip {
        "generated-export" => "export",
        "semantic-diff" => "diff",
        other => other,
    }
}

fn render_browse_workspace<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
) {
    if app.nav.expanded {
        app.hit.regions.insert(HitRegion::Inspector, area);
        detail_panel::render(frame, area, app);
        return;
    }

    let rail_width = workspace_rail_width(
        area.width,
        app.data.available_views().len(),
        app.data.capability_chips().len(),
    );
    // Wide enough that qualified Tcl names stop wrapping across three lines.
    let inspector_width = if area.width >= 180 {
        46
    } else if area.width >= 160 {
        42
    } else if area.width >= 110 {
        34
    } else if area.width < 90 {
        20
    } else {
        28
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(rail_width),
            Constraint::Min(30),
            Constraint::Length(inspector_width),
        ])
        .split(area);
    app.hit.regions.insert(HitRegion::BrowseRail, chunks[0]);
    app.hit.regions.insert(HitRegion::Canvas, chunks[1]);
    app.hit.regions.insert(HitRegion::Inspector, chunks[2]);
    render_workspace_rail(frame, chunks[0], app);
    render_canvas(frame, chunks[1], app);
    detail_panel::render(frame, chunks[2], app);
}

fn workspace_rail_width(area_width: u16, view_count: usize, capability_count: usize) -> u16 {
    let dense_workspace = view_count > 4 || capability_count >= 5;
    if area_width >= 160 {
        34
    } else if area_width >= 118 && dense_workspace {
        28
    } else if area_width >= 96 {
        24
    } else {
        22
    }
}

fn render_workspace_rail<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
) {
    let content_width = area.width.saturating_sub(2);
    let available = app.data.available_views();
    let current = app.current_view();
    let specs = app.specs();
    let mut items = available
        .iter()
        .filter_map(|view| views::spec(specs, *view))
        .map(|spec| {
            let marker = if spec.view == current { ">" } else { " " };
            rail_view_item(
                marker,
                spec.icon,
                spec.title,
                spec.keybind,
                content_width,
                app,
            )
        })
        .collect::<Vec<_>>();
    let switcher_rows = items.len();
    let outline_capacity = (area.height as usize)
        .saturating_sub(switcher_rows + 3)
        .min(64);

    // The rail navigates; it never restates canvas content. List views are fully
    // rendered — and own their selection and hit targets — in `list_panel`, so
    // they get no outline at all. Document views get their structural anchors.
    if !views::is_list_view(specs, current) && outline_capacity > 0 {
        let before = items.len();
        render_rail_outline(
            &mut items,
            app,
            current,
            content_width as usize,
            outline_capacity,
        );
        if items.len() > before {
            items.insert(before, ListItem::new(Line::from("")));
        }
    }

    // Size the panel to its content: an empty bordered box the height of the
    // screen reads as a broken pane, not as a navigator with nothing to add.
    let height = (items.len() as u16 + 2).min(area.height);
    let area = Rect::new(area.x, area.y, area.width, height);
    app.hit.regions.insert(HitRegion::BrowseRail, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.layout.focused_pane == PaneFocus::List {
            app.theme.border_focused()
        } else {
            app.theme.border()
        })
        .title(Span::styled(" Workspace ", app.theme.panel_title()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(List::new(items), inner);
}

/// Reduce an anchor line to what distinguishes it. `proc ` leads every entry in a
/// procedure outline and the argument list rarely survives the rail's width.
fn outline_label(line: &str) -> String {
    match line.strip_prefix("proc ") {
        Some(rest) => rest
            .split_whitespace()
            .next()
            .unwrap_or(rest)
            .trim_end_matches('{')
            .to_string(),
        None => line.to_string(),
    }
}

/// Structural outline for a document view: the anchors a reader navigates by.
fn render_rail_outline<D: DataSource>(
    items: &mut Vec<ListItem<'static>>,
    app: &mut TuiApp<D>,
    view: crate::data::ViewId,
    width: usize,
    capacity: usize,
) {
    let Ok(document) = app.cached_document(view) else {
        return;
    };

    // Command groups are the outline. `document.anchors` is statement-level
    // (`c057/n011`) — listing those would just make the rail a second canvas.
    let mut outline = document
        .decompile_anchors
        .iter()
        .copied()
        .filter(|line| *line != usize::MAX && *line < document.lines.len())
        .map(|line| (line, outline_label(document.lines[line].trim())))
        .filter(|(_, label)| !label.is_empty())
        .collect::<Vec<_>>();
    if outline.is_empty() {
        outline = document
            .anchors
            .iter()
            .map(|anchor| (anchor.line_span.0, anchor.label.clone()))
            .collect();
    }
    if outline.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{} lines", document.lines.len()),
            app.theme.fg_muted(),
        ))));
        return;
    }

    // Follow the viewport: keep the anchor covering the current scroll position
    // visible so the outline tracks the canvas instead of pinning to the top.
    let scroll = app.nav.doc_scroll(view);
    let current = outline
        .iter()
        .rposition(|(line, _)| *line <= scroll)
        .unwrap_or(0);
    let start = current
        .saturating_sub(capacity / 2)
        .min(outline.len().saturating_sub(capacity));

    for (index, (_, label)) in outline.iter().enumerate().skip(start).take(capacity) {
        let selected = index == current;
        let marker = if selected { "▸ " } else { "  " };
        let style = if selected {
            app.theme.accent()
        } else {
            app.theme.fg()
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(marker, app.theme.accent()),
            Span::styled(fit_text(label, width.saturating_sub(2)), style),
        ])));
    }
}

fn render_canvas<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &mut TuiApp<D>) {
    if views::is_list_view(app.specs(), app.current_view()) {
        list_panel::render(
            frame,
            area,
            app,
            app.layout.focused_pane == PaneFocus::Decompile,
        );
    } else {
        document_panel::render(frame, area, app);
    }
}

fn render_compare_workspace<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(area);
    app.hit.regions.insert(HitRegion::CompareLeft, chunks[0]);
    app.hit.regions.insert(HitRegion::CompareRight, chunks[1]);
    render_compare_left(frame, chunks[0], app);
    render_compare_right(frame, chunks[1], app);
}

fn render_compare_left<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border())
        .title(Span::styled(
            " Left Source / Decompile ",
            app.theme.panel_title(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let view = app
        .data
        .decompile_view()
        .unwrap_or_else(|| app.current_view());
    render_document_plain(frame, inner, app, view, app.nav.doc_scroll(view));
}

fn render_compare_right<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if app.diff.focus_right {
            app.theme.border_focused()
        } else {
            app.theme.border()
        })
        .title(Span::styled(
            " Diff Report ",
            app.theme.panel_title_focused(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = app.diff.diff_document.clone().unwrap_or_else(|| {
        "no active semantic diff\nrun :diff <path> to compare with an existing artifact".to_string()
    });
    let text = format_compare_report_for_wrap(&text)
        .lines()
        .skip(app.diff.right_scroll)
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}

fn format_compare_report_for_wrap(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.contains('|') {
                line.split('|')
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" | ")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_document_plain<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
    view: crate::data::ViewId,
    scroll: usize,
) {
    let rendered = match app.cached_document(view) {
        Err(err) => Text::from(format!("error: {err}")),
        Ok(document) if document.text.is_empty() => {
            Text::from(app.data.empty_document_message(view))
        }
        Ok(document) => {
            let start = scroll.min(document.lines.len());
            let end = start
                .saturating_add(area.height as usize)
                .min(document.lines.len());
            app.data
                .render_document_window(view, &document.lines, &app.theme, start, end)
        }
    };
    frame.render_widget(Paragraph::new(rendered).wrap(Wrap { trim: false }), area);
}

fn render_mode_footer<D: DataSource>(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    app: &mut TuiApp<D>,
    command_label: &str,
) {
    let row = Rect::new(area.x, area.y, area.width, area.height.min(1));
    app.hit.regions.insert(HitRegion::FooterStatus, row);
    app.hit.regions.insert(HitRegion::FooterCommand, row);
    let width = row.width as usize;

    if app.cmd.active {
        let prompt = format!(" {command_label}: ");
        let room = width.saturating_sub(cell_width(&prompt) + 1);
        let (before_cursor, after_cursor) = command_window(&app.cmd.input, app.cmd.cursor, room);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prompt, app.theme.command_prompt()),
                Span::styled(before_cursor, app.theme.command_input()),
                Span::styled("█", app.theme.accent()),
                Span::styled(after_cursor, app.theme.command_input()),
            ])),
            row,
        );
        return;
    }

    let view = app.current_view();
    let profile = app.current_layout_profile();

    // The row is offered to Laidout in priority order rather than budgeted by
    // hand. Context always survives; key hints keep at least their most compact
    // form so discoverability never vanishes; a transient message outranks the
    // persistent fields it displaces, because a message truncated past
    // legibility helps nobody.
    let (_, context_value) = footer_context(app, profile, view);
    let position = footer_position(app, profile, view);
    let fields = footer_active_fields(app, profile);
    let feedback = command_row_feedback(app, profile);
    let affordances = footer_available_affordances(app);

    let context = || {
        let mut variant = chrome::Variant::new()
            .push(tokens::FG_DIM, " ")
            .push(tokens::ACCENT, context_value.clone());
        if let Some(position) = &position {
            variant = variant.push(tokens::FG_MUTED, format!("  {position}"));
        }
        variant
    };
    let with_fields = |mut variant: chrome::Variant| {
        for (label, value) in &fields {
            variant = variant
                .push(tokens::FG_DIM, format!("  {label} "))
                .push(tokens::FG_MUTED, value.clone());
        }
        variant
    };
    let with_message = |variant: chrome::Variant, message: Option<&str>| match message {
        Some(message) => variant
            .push(tokens::FG_DIM, "  \u{2502}  ")
            .push(tokens::FG_MUTED, message.to_string()),
        None => variant,
    };

    // Hints are guaranteed their tersest form and nothing more, so the left side
    // is fitted against that floor rather than against whatever the hints would
    // prefer. Only then do the hints expand into what the left side left behind.
    let key_variants = footer_key_variants(&affordances);
    let reserved = key_variants
        .last()
        .map(|compact| chrome::fit(std::slice::from_ref(compact), width, &app.theme).width)
        .unwrap_or(0);

    let budget = width.saturating_sub(reserved + FOOTER_GAP);

    // A shortened message is offered as its own phrasing, and Laidout does the
    // shortening too: prose ingested with `from_text` breaks at whitespace, so
    // the message loses whole words instead of being cut mid-word. Below
    // MIN_MESSAGE_CELLS what remains is unreadable and it is dropped instead.
    const MIN_MESSAGE_CELLS: usize = 16;
    let message_room =
        budget.saturating_sub(chrome::fit(&[context()], width, &app.theme).width + SEPARATOR_CELLS);
    let elided = feedback
        .as_deref()
        .filter(|_| message_room >= MIN_MESSAGE_CELLS)
        .and_then(|message| chrome::elide(message, message_room, &app.theme));

    let left = chrome::fit(
        &[
            with_fields(with_message(context(), feedback.as_deref())),
            with_message(context(), feedback.as_deref()),
            with_message(context(), elided.as_deref()),
            with_fields(context()),
            context(),
        ],
        budget,
        &app.theme,
    );

    let keys = chrome::fit(
        &key_variants,
        width.saturating_sub(left.width + FOOTER_GAP),
        &app.theme,
    );

    frame.render_widget(Paragraph::new(chrome::justify(left, keys, width)), row);
}

/// Space kept between the row's two ends so they never abut.
const FOOTER_GAP: usize = 2;

/// Width of the separator drawn before a message.
const SEPARATOR_CELLS: usize = 5;

/// Key hints, from the most descriptive phrasing to the tersest.
fn footer_key_variants(affordances: &[&str]) -> Vec<chrome::Variant> {
    [
        FooterHintDensity::Descriptive,
        FooterHintDensity::Balanced,
        FooterHintDensity::Concise,
        FooterHintDensity::Compact,
    ]
    .into_iter()
    .map(|density| {
        chrome::Variant::new().push(
            tokens::FG_DIM,
            join_footer_affordances(affordances, density),
        )
    })
    .collect()
}

/// Position within the active view: the one genuinely dynamic status value.
fn footer_position<D: DataSource>(
    app: &mut TuiApp<D>,
    profile: ModeLayoutProfile,
    view: crate::data::ViewId,
) -> Option<String> {
    if !matches!(profile, ModeLayoutProfile::Browse) {
        return None;
    }
    if views::is_list_view(app.specs(), view) {
        let total = app.data.list_len(view);
        if total == 0 {
            return None;
        }
        let selection = app.nav.selection(view).min(total - 1);
        return Some(format!("#{}/{total}", selection + 1));
    }
    let lines = app.cached_document(view).ok()?.lines.len();
    if lines == 0 {
        return None;
    }
    let line = app.nav.doc_scroll(view).min(lines - 1) + 1;
    Some(format!("ln {line}/{lines}"))
}

/// Status fields worth a permanent slot only while they hold a non-default value.
fn footer_active_fields<D: DataSource>(
    app: &TuiApp<D>,
    profile: ModeLayoutProfile,
) -> Vec<(&'static str, String)> {
    let mut fields = Vec::new();
    // Search, trace and pane focus describe the browse workspace. Carrying them
    // into Compare would report state that mode cannot act on.
    if matches!(profile, ModeLayoutProfile::Browse) {
        if app.layout.focused_pane == PaneFocus::Context {
            fields.push(("focus", "Inspector".to_string()));
        }
        if let Some(query) = app.search.query.as_deref().filter(|q| !q.is_empty()) {
            fields.push((
                "search",
                if app.search.hits.is_empty() {
                    format!("0/0 {query}")
                } else {
                    format!(
                        "{}/{} {query}",
                        app.search.cursor + 1,
                        app.search.hits.len()
                    )
                },
            ));
        }
        if app.nav.trace_mode && app.data.trace_len() > 0 {
            fields.push(("trace", footer_trace_status(app)));
        }
    }
    if app.diff.active {
        fields.push(("sync", "on".to_string()));
    }
    fields
}

fn footer_context<D: DataSource>(
    app: &TuiApp<D>,
    profile: ModeLayoutProfile,
    view: crate::data::ViewId,
) -> (&'static str, String) {
    match profile {
        ModeLayoutProfile::Browse => ("View", views::title(app.specs(), view).to_string()),
        ModeLayoutProfile::Compare => ("Mode", app.current_mode_spec().title.to_string()),
    }
}

fn footer_available_affordances<D: DataSource>(app: &TuiApp<D>) -> Vec<&'static str> {
    let bindings = crate::bindings::normal_bindings(app);
    app.mode_status_affordances()
        .iter()
        .copied()
        .filter(|affordance| *affordance != "Status")
        .filter(|affordance| match *affordance {
            "Command" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::Command),
            "Palette" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::Palette),
            "Help" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::Help),
            "Search" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::Search),
            "Trace" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::ToggleTrace),
            "Diff Close" | "Close" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::CloseDiff),
            "Next" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::NavigateGroup(1)),
            "Previous" => bindings
                .iter()
                .any(|binding| binding.action == crate::bindings::BindingAction::NavigateGroup(-1)),
            _ => true,
        })
        .filter(|affordance| footer_affordance_is_available(app, affordance))
        .collect()
}

fn footer_affordance_is_available<D: DataSource>(app: &TuiApp<D>, affordance: &str) -> bool {
    match affordance {
        "Trace" => app.data.trace_len() > 0,
        "Close" | "Diff Close" | "Next" | "Previous" => app.diff.active,
        "Export" => matches!(
            app.data.export_availability(),
            crate::data::OperationAvailability::Available
        ),
        _ => true,
    }
}

fn footer_trace_status<D: DataSource>(app: &TuiApp<D>) -> String {
    if app.data.trace_len() == 0 {
        return "n/a".to_string();
    }
    if app.nav.trace_mode {
        app.nav
            .trace_index
            .map(|idx| format!("on #{idx}"))
            .unwrap_or_else(|| "on".to_string())
    } else {
        "off".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterHintDensity {
    Descriptive,
    Balanced,
    Concise,
    Compact,
}

fn join_footer_affordances(affordances: &[&str], density: FooterHintDensity) -> String {
    affordances
        .iter()
        .map(|affordance| footer_affordance_label(affordance, density))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn footer_affordance_label(affordance: &str, density: FooterHintDensity) -> String {
    match (affordance, density) {
        ("Command", FooterHintDensity::Descriptive) => ": Command".to_string(),
        ("Palette", FooterHintDensity::Descriptive) => "Ctrl-P Palette".to_string(),
        ("Help", FooterHintDensity::Descriptive) => "? Help".to_string(),
        ("Search", FooterHintDensity::Descriptive) => "/ Search".to_string(),
        ("Trace", FooterHintDensity::Descriptive) => "t Trace".to_string(),
        ("Export", FooterHintDensity::Descriptive) => ":export Export".to_string(),
        ("Diff", FooterHintDensity::Descriptive) => ":diff Open".to_string(),
        ("Diff Close" | "Close", FooterHintDensity::Descriptive) => "Q/Esc Close".to_string(),
        ("Next", FooterHintDensity::Descriptive) => "] Next change".to_string(),
        ("Previous", FooterHintDensity::Descriptive) => "[ Previous change".to_string(),
        ("Status", FooterHintDensity::Descriptive) => "Status".to_string(),
        ("Command", FooterHintDensity::Balanced) => ": Cmd".to_string(),
        ("Palette", FooterHintDensity::Balanced) => "Ctrl-P".to_string(),
        ("Help", FooterHintDensity::Balanced) => "?".to_string(),
        ("Search", FooterHintDensity::Balanced) => "/ Search".to_string(),
        ("Trace", FooterHintDensity::Balanced) => "t Trace".to_string(),
        ("Export", FooterHintDensity::Balanced) => ":export Export".to_string(),
        ("Diff", FooterHintDensity::Balanced) => ":diff Open".to_string(),
        ("Diff Close" | "Close", FooterHintDensity::Balanced) => "Q/Esc Close".to_string(),
        ("Next", FooterHintDensity::Balanced) => "] Next".to_string(),
        ("Previous", FooterHintDensity::Balanced) => "[ Prev".to_string(),
        ("Status", FooterHintDensity::Balanced) => "Status".to_string(),
        ("Command", FooterHintDensity::Concise) => ": Cmd".to_string(),
        ("Palette", FooterHintDensity::Concise) => "Ctrl-P".to_string(),
        ("Help", FooterHintDensity::Concise) => "?".to_string(),
        ("Search", FooterHintDensity::Concise) => "/ Search".to_string(),
        ("Trace", FooterHintDensity::Concise) => "t Trace".to_string(),
        ("Export", FooterHintDensity::Concise) => ":export".to_string(),
        ("Diff", FooterHintDensity::Concise) => ":diff".to_string(),
        ("Diff Close" | "Close", FooterHintDensity::Concise) => "Q/Esc".to_string(),
        ("Next", FooterHintDensity::Concise) => "] Next".to_string(),
        ("Previous", FooterHintDensity::Concise) => "[ Prev".to_string(),
        ("Status", FooterHintDensity::Concise) => "Status".to_string(),
        ("Command", FooterHintDensity::Compact) => ":".to_string(),
        ("Palette", FooterHintDensity::Compact) => "Ctrl-P".to_string(),
        ("Help", FooterHintDensity::Compact) => "?".to_string(),
        ("Search", FooterHintDensity::Compact) => "/".to_string(),
        ("Trace", FooterHintDensity::Compact) => "t".to_string(),
        ("Export", FooterHintDensity::Compact) => ":export".to_string(),
        ("Diff", FooterHintDensity::Compact) => ":diff".to_string(),
        ("Diff Close" | "Close", FooterHintDensity::Compact) => "Q/Esc".to_string(),
        ("Next", FooterHintDensity::Compact) => "]".to_string(),
        ("Previous", FooterHintDensity::Compact) => "[".to_string(),
        ("Status", FooterHintDensity::Compact) => "status".to_string(),
        (other, _) => other.to_string(),
    }
}

fn command_row_feedback<D: DataSource>(
    app: &TuiApp<D>,
    profile: ModeLayoutProfile,
) -> Option<String> {
    if let Some(output) = app
        .cmd
        .output
        .as_deref()
        .map(str::trim)
        .filter(|output| !output.is_empty())
        .filter(|output| !is_ambient_command_feedback(output))
        .filter(|output| {
            !(matches!(profile, ModeLayoutProfile::Compare)
                && app.diff.active
                && output.starts_with("diff opened:"))
        })
    {
        return Some(output.to_string());
    }
    let status = app.status.trim();
    if !is_ambient_command_feedback(status)
        && !(matches!(profile, ModeLayoutProfile::Compare)
            && app.diff.active
            && status.starts_with("diff opened:"))
    {
        return Some(status.to_string());
    }
    if matches!(profile, ModeLayoutProfile::Compare)
        && app.diff.active
        && let Some(target) = app
            .diff
            .target_path
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
    {
        return Some(format!("Target: {target}"));
    }
    None
}

fn is_ambient_command_feedback(value: &str) -> bool {
    value.is_empty() || value == "ready" || value.starts_with("mode: ")
}

fn mode_switch_label(spec: &crate::state::ModeSpec) -> String {
    let icon = spec.icon.trim();
    let show_icon = !icon.is_empty() && !mode_icon_repeats_label(spec, icon);
    match (spec.keybind, show_icon) {
        (Some(key), true) => format!("[{key}] {icon} {}", spec.title),
        (Some(key), false) => format!("[{key}] {}", spec.title),
        (None, true) => format!("{icon} {}", spec.title),
        (None, false) => spec.title.to_string(),
    }
}

fn mode_icon_repeats_label(spec: &crate::state::ModeSpec, icon: &str) -> bool {
    if icon.chars().count() != 1 {
        return false;
    }
    let icon = icon.chars().next().expect("icon has one char");
    spec.keybind
        .is_some_and(|key| key.eq_ignore_ascii_case(&icon))
        || spec
            .title
            .chars()
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case(&icon))
}

fn rail_view_item<D: DataSource>(
    marker: &str,
    icon: &str,
    title: &str,
    keybind: Option<char>,
    width: u16,
    app: &TuiApp<D>,
) -> ListItem<'static> {
    let width = width as usize;
    let prefix_width = cell_width(marker) + 1 + cell_width(icon) + 1;
    let mut key = keybind.map(|key| format!(" [{key}]")).unwrap_or_default();
    if prefix_width + cell_width(&key) >= width {
        key.clear();
    }
    let title_width = width.saturating_sub(prefix_width + cell_width(&key));
    let title = fit_text(title, title_width);
    let used = prefix_width + cell_width(&title) + cell_width(&key);
    let padding = " ".repeat(width.saturating_sub(used));

    ListItem::new(Line::from(vec![
        Span::styled(marker.to_string(), app.theme.accent()),
        Span::raw(" "),
        Span::styled(icon.to_string(), app.theme.fg_dim()),
        Span::raw(" "),
        Span::styled(title, app.theme.fg()),
        Span::raw(padding),
        Span::styled(key, app.theme.fg_dim()),
    ]))
}

/// Display width in terminal cells.
///
/// Measured with the Unicode width tables rather than assumed from whether a
/// character is ASCII. The chrome is built from `│ · … ▸ ─`, every one of which
/// occupies a single cell despite being multi-byte; charging two for them made
/// every width budget over-count its separators.
fn cell_width(value: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(value)
}

/// Keep the last `max_cells` columns of `value`.
fn tail_cells(value: &str, max_cells: usize) -> String {
    if cell_width(value) <= max_cells {
        return value.to_string();
    }
    let mut kept = std::collections::VecDeque::new();
    let mut used = 0;
    for ch in value.chars().rev() {
        let next = char_cells(ch);
        if used + next > max_cells {
            break;
        }
        kept.push_front(ch);
        used += next;
    }
    kept.into_iter().collect()
}

fn head_cells(value: &str, max_cells: usize) -> String {
    let mut kept = String::new();
    let mut used = 0;
    for ch in value.chars() {
        let next = char_cells(ch);
        if used + next > max_cells {
            break;
        }
        kept.push(ch);
        used += next;
    }
    kept
}

fn command_window(input: &str, cursor: usize, max_cells: usize) -> (String, String) {
    let mut cursor = cursor.min(input.len());
    while !input.is_char_boundary(cursor) {
        cursor = cursor.saturating_sub(1);
    }
    let (before, after) = input.split_at(cursor);
    if after.is_empty() {
        return (tail_cells(before, max_cells), String::new());
    }
    let before_budget = max_cells.saturating_sub(max_cells / 2);
    let visible_before = tail_cells(before, before_budget);
    let after_budget = max_cells.saturating_sub(cell_width(&visible_before));
    let visible_after = head_cells(after, after_budget);
    (visible_before, visible_after)
}

fn char_cells(ch: char) -> usize {
    unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn fit_text(value: &str, max_chars: usize) -> String {
    let len = value.chars().count();
    if len <= max_chars {
        value.to_string()
    } else if max_chars == 0 {
        String::new()
    } else if max_chars == 1 {
        "…".to_string()
    } else {
        let mut out = value.chars().take(max_chars - 1).collect::<String>();
        let trimmed_len = out.trim_end().len();
        out.truncate(trimmed_len);
        out.push('…');
        out
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::data::{ListEntry, ViewId};
    use crate::state::{ModeId, ModeLayoutProfile, ModeSpec};
    use crate::views::{ViewOpenBehavior, ViewSpec};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const LIST: ViewId = ViewId("items");
    const SOURCE: ViewId = ViewId("source");
    const DECOMPILE: ViewId = ViewId("decompile");
    const SUMMARY: ViewId = ViewId("summary");

    const MODE_SPECS: &[ModeSpec] = &[
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
    ];

    const SPECS: &[ViewSpec] = &[
        ViewSpec {
            view: LIST,
            token: "items",
            title: "Items",
            aliases: &["items"],
            keybind: Some('i'),
            icon: "*",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: true,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: SOURCE,
            token: "source",
            title: "Source",
            aliases: &["source"],
            keybind: Some('s'),
            icon: "S",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: DECOMPILE,
            token: "decompile",
            title: "Decompile",
            aliases: &["decompile"],
            keybind: Some('d'),
            icon: "D",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: SUMMARY,
            token: "summary",
            title: "Summary",
            aliases: &["summary"],
            keybind: None,
            icon: "?",
            show_in_tab_bar: false,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::SummaryModal,
        },
    ];

    #[derive(Clone)]
    struct RenderData {
        path: &'static str,
        header: &'static str,
        capabilities: &'static [&'static str],
        summary: &'static str,
        source_line: Option<&'static str>,
        decompile: &'static str,
        no_colors: bool,
        empty: bool,
    }

    impl DataSource for RenderData {
        fn path(&self) -> &str {
            self.path
        }

        fn available_views(&self) -> &[ViewId] {
            if self.source_line.is_some() {
                &[SOURCE, DECOMPILE, SUMMARY]
            } else {
                &[LIST, DECOMPILE, SUMMARY]
            }
        }

        fn default_view(&self) -> ViewId {
            if self.source_line.is_some() {
                SOURCE
            } else {
                LIST
            }
        }

        fn no_colors(&self) -> bool {
            self.no_colors
        }

        fn page_size(&self) -> usize {
            5
        }

        fn summary_text(&self) -> String {
            self.summary.to_string()
        }

        fn view_specs(&self) -> &[ViewSpec] {
            SPECS
        }

        fn mode_specs(&self) -> &[ModeSpec] {
            MODE_SPECS
        }

        fn list_len(&self, view: ViewId) -> usize {
            match (view, self.source_line) {
                _ if self.empty => 0,
                (LIST, None) => 1,
                _ => 0,
            }
        }

        fn list_entry(&self, view: ViewId, _index: usize) -> Option<ListEntry> {
            match (view, self.source_line) {
                (LIST, None) => Some(ListEntry {
                    text: "c000> (0) push1 # value".to_string(),
                    group: Some(0),
                    is_group_start: true,
                }),
                _ => None,
            }
        }

        fn list_detail(&self, _view: ViewId, _index: usize) -> Option<String> {
            Some("detail panel".to_string())
        }

        fn jump_index(&self, _view: ViewId, _value: i32) -> Option<usize> {
            None
        }

        fn document_text(&mut self, view: ViewId) -> Result<String, String> {
            Ok(match view {
                DECOMPILE => self.decompile.to_string(),
                SUMMARY => self.summary.to_string(),
                SOURCE => self.source_line.unwrap_or("").to_string(),
                _ => String::new(),
            })
        }

        fn command_group_count(&self) -> usize {
            1
        }

        fn trace_len(&self) -> usize {
            usize::from(self.source_line.is_none() && !self.empty)
        }

        fn group_for_index(&self, _index: usize) -> Option<usize> {
            Some(0)
        }

        fn first_index_for_group(&self, _group: usize) -> Option<usize> {
            Some(0)
        }

        fn header_info(&self) -> Option<String> {
            Some(self.header.to_string())
        }

        fn capability_chips(&self) -> Vec<String> {
            self.capabilities
                .iter()
                .map(|chip| (*chip).to_string())
                .collect()
        }

        fn export_status(&self) -> String {
            if self.capabilities.contains(&"generated-export") {
                "Export ready".to_string()
            } else {
                "Export unavailable".to_string()
            }
        }

        fn export_availability(&self) -> crate::data::OperationAvailability {
            if self.capabilities.contains(&"generated-export") {
                crate::data::OperationAvailability::Available
            } else {
                crate::data::OperationAvailability::Unavailable {
                    reason: "unavailable".to_string(),
                }
            }
        }

        fn decompile_view(&self) -> Option<ViewId> {
            Some(DECOMPILE)
        }
    }

    #[test]
    fn render_workspaces_show_provenance_export_status_empty_states_and_theme() {
        let cases = [
            RenderData {
                path: "bytecode.tbc",
                header: "tcl 8.6 generated source",
                capabilities: &["bytecode", "source", "generated-export"],
                summary: "summary: bytecode with generated source",
                source_line: None,
                decompile: "generated decompiled Tcl source\nset a 1\n",
                no_colors: false,
                empty: false,
            },
            RenderData {
                path: "generated.tcl",
                header: "generated from bytecode",
                capabilities: &["source", "generated-export"],
                summary: "summary: generated-source workspace",
                source_line: Some("set generated 1"),
                decompile: "source rendering\nset generated 1\n",
                no_colors: false,
                empty: false,
            },
            RenderData {
                path: "plain.tcl",
                header: "plain source",
                capabilities: &["source"],
                summary: "summary: plain-source workspace",
                source_line: Some("set plain 1"),
                decompile: "source rendering\nset plain 1\n",
                no_colors: true,
                empty: false,
            },
            RenderData {
                path: "empty.tbc",
                header: "tcl 8.6 source: generated from empty.tbc",
                capabilities: &["bytecode", "source"],
                summary: "summary: empty bytecode workspace",
                source_line: None,
                decompile: "",
                no_colors: false,
                empty: true,
            },
        ];

        for data in cases {
            let mut app = crate::app::TuiApp::new_with_theme(data, false, Some("sunset"));
            app.switch_to_view(app.data.default_view());
            app.status = format!("theme: {}", app.theme.name());
            app.cmd.output = Some("exported generated source to /tmp/out.tcl".to_string());
            let backend = TestBackend::new(96, 28);
            let mut terminal = Terminal::new(backend).expect("terminal");
            terminal
                .draw(|frame| draw(frame, &mut app))
                .expect("render frame");
            let text = buffer_text(terminal.backend().buffer());
            assert!(text.contains("tcld"));
            assert!(text.contains("source: generated") || text.contains("generated source"));
            assert!(text.contains("[b] Browse"), "{text}");
            assert!(
                !text.contains("theme:") && !text.contains("colors:"),
                "persistent visual preferences should not occupy primary header space\n{text}"
            );
            assert!(text.contains("Decompile"));
            assert!(text.contains("exported generated"));
            if app.data.empty {
                assert!(text.contains("(empty)") || text.contains("No entries"));
            }
            if !app.theme.no_colors {
                assert_ne!(
                    app.theme.accent().fg,
                    crate::theme::Theme::new(false).accent().fg,
                    "selected theme should affect rendered styles"
                );
            }

            app.overlay.show_help = true;
            terminal
                .draw(|frame| draw(frame, &mut app))
                .expect("render help frame");
            let help_text = buffer_text(terminal.backend().buffer());
            assert!(help_text.contains("Quick Reference"));
            assert!(!help_text.contains("Runtime Debug"));

            app.overlay.show_help = false;
            app.palette.active = true;
            app.palette.query = "decompile".to_string();
            terminal
                .draw(|frame| draw(frame, &mut app))
                .expect("render palette frame");
            let palette_text = buffer_text(terminal.backend().buffer());
            assert!(palette_text.contains("Command Palette"));
            assert!(palette_text.contains("Decompile"));
        }
    }

    #[test]
    fn domain_workspace_renders_synchronized_panels() {
        let mut app = crate::app::TuiApp::new_with_theme(
            RenderData {
                path: "expr.tbc",
                header: "tcl 8.6 | source: generated from expr.tbc",
                capabilities: &["bytecode", "source", "generated-export"],
                summary: "summary: bytecode workspace",
                source_line: None,
                decompile: "generated decompiled Tcl source\nset value [expr {1 + 2}]\n",
                no_colors: false,
                empty: false,
            },
            false,
            Some("sunset"),
        );
        app.status = "ready".to_string();
        app.cmd.output = Some("exported generated source to /tmp/out.tcl".to_string());
        app.search.query = Some("expr".to_string());
        app.search.hits = vec![0, 1];
        app.nav.group_focus = Some(0);
        app.nav.trace_mode = true;
        app.nav.trace_index = Some(0);

        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("render frame");
        let text = buffer_text(terminal.backend().buffer());

        for needle in [
            "tcld",
            "source: generated",
            "Items",
            "Decompile",
            "Inspector",
            "search",
            "trace",
            "c000",
            ":export",
            "exported generated",
        ] {
            assert!(text.contains(needle), "missing {needle:?}\n{text}");
        }

        app.switch_to_view(DECOMPILE);
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("render decompile map");
        assert!(
            app.hit.decompile_rail_area.is_some(),
            "the compact decompile map should remain interactive"
        );

        app.overlay.show_help = true;
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("render help");
        assert!(buffer_text(terminal.backend().buffer()).contains("Quick Reference"));

        app.overlay.show_help = false;
        app.palette.active = true;
        app.palette.query = "view".to_string();
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("render palette");
        assert!(buffer_text(terminal.backend().buffer()).contains("Command Palette"));

        let mut empty = crate::app::TuiApp::new_with_theme(
            RenderData {
                path: "empty.tbc",
                header: "tcl 8.6 | source: generated from empty.tbc",
                capabilities: &["bytecode", "source"],
                summary: "summary: empty workspace",
                source_line: None,
                decompile: "",
                no_colors: false,
                empty: true,
            },
            false,
            Some("sunset"),
        );
        let backend = TestBackend::new(96, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &mut empty))
            .expect("render empty");
        let empty_text = buffer_text(terminal.backend().buffer());
        assert!(empty_text.contains("No entries") || empty_text.contains("(empty)"));
    }

    #[test]
    fn expanded_inspector_uses_workspace_width() {
        let mut app = crate::app::TuiApp::new_with_theme(
            RenderData {
                path: "expr.tbc",
                header: "tcl 8.6 | source: generated from expr.tbc",
                capabilities: &["bytecode", "source", "generated-export"],
                summary: "summary: bytecode workspace",
                source_line: None,
                decompile: "generated decompiled Tcl source\nset value [expr {1 + 2}]\n",
                no_colors: false,
                empty: false,
            },
            false,
            Some("sunset"),
        );
        app.nav.expanded = true;

        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("render expanded inspector");
        let text = buffer_text(terminal.backend().buffer());

        assert!(text.contains("Inspector Expanded"), "{text}");
        assert!(text.contains("detail panel"), "{text}");
        assert!(!text.contains("Workspace"), "{text}");
        assert!(!text.contains("Canvas"), "{text}");
    }

    #[test]
    fn responsive_domain_app_layout() {
        for (width, height, expected) in [
            (72, 24, "Workspace"),
            (120, 32, "Inspector"),
            (100, 28, "Canvas"),
        ] {
            let mut app = crate::app::TuiApp::new_with_theme(
                RenderData {
                    path: "expr.tbc",
                    header: "tcl 8.6 | source: generated from expr.tbc",
                    capabilities: &["bytecode", "source", "generated-export"],
                    summary: "summary: responsive workspace",
                    source_line: None,
                    decompile: "generated decompiled Tcl source\nset a 1\nset b 2\nset c 3\n",
                    no_colors: width == 100,
                    empty: false,
                },
                false,
                Some("sunset"),
            );
            app.search.query = Some("set".to_string());
            app.search.hits = vec![0, 1, 2];

            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("terminal");
            terminal
                .draw(|frame| draw(frame, &mut app))
                .expect("responsive render");
            let text = buffer_text(terminal.backend().buffer());
            assert!(text.contains("tcld"));
            assert!(
                text.lines().last().unwrap_or_default().contains("Search"),
                "every width should keep the footer key hints\n{text}"
            );
            assert!(text.contains(expected), "missing {expected:?}\n{text}");
        }
    }

    #[test]
    fn minimum_size_frame_does_not_build_workspace_panes() {
        let mut app = shell_app();
        for (width, height) in [
            (MIN_TERMINAL_WIDTH - 1, MIN_TERMINAL_HEIGHT),
            (MIN_TERMINAL_WIDTH, MIN_TERMINAL_HEIGHT - 1),
        ] {
            let frame = render_text(&mut app, width, height);
            assert!(frame.contains("Terminal too small"), "{frame}");
            assert!(frame.contains("Required: 72x20"), "{frame}");
            assert!(frame.contains("Ctrl-C exit | q quit"), "{frame}");
            assert!(!frame.contains("Workspace"), "{frame}");
            assert!(!frame.contains("Inspector"), "{frame}");
        }
    }

    #[test]
    fn browse_mode_geometry_rejects_three_pane_grid() {
        let mut app = shell_app();
        app.switch_mode(ModeId("browse"));
        let frame = render_text(&mut app, 120, 32);

        // One header row, one footer row: the panels own everything between.
        assert_geometry_token(&frame, "tcld explore", 0, 0);
        assert_geometry_token(&frame, "[b] Browse", 0, 0);
        assert_geometry_token(&frame, "Workspace", 1, 0);
        assert_geometry_token(&frame, "Canvas", 1, 20);
        assert_geometry_token(&frame, "Inspector", 1, 80);
        assert!(!frame.contains("Tabs:"), "{frame}");
        assert!(!frame.contains("\u{250c} Instructions"));
        assert!(
            frame
                .lines()
                .last()
                .unwrap_or_default()
                .starts_with(" Items"),
            "footer should name the active view\n{frame}"
        );

        let canvas_x = first_col(&frame, "Canvas").expect("canvas label");
        let inspector_x = first_col(&frame, "Inspector").expect("inspector label");
        let workspace_x = first_col(&frame, "Workspace").expect("workspace label");
        assert!(
            workspace_x < canvas_x && canvas_x < inspector_x,
            "expected rail < canvas < inspector\n{frame}"
        );
        assert!(
            inspector_x - canvas_x > canvas_x - workspace_x,
            "canvas should dominate the browse frame\n{frame}"
        );
    }

    #[test]
    fn browse_rail_expands_on_wide_terminals_without_starving_canvas() {
        let mut compact = shell_app();
        compact.switch_mode(ModeId("browse"));
        compact.data.capabilities = &["bytecode", "source", "generated-export", "semantic-diff"];
        let compact_frame = render_text(&mut compact, 72, 24);
        let compact_workspace_x =
            first_char_col(&compact_frame, "Workspace").expect("compact rail");
        let compact_canvas_x = first_char_col(&compact_frame, "Canvas").expect("compact canvas");
        let compact_rail_span = compact_canvas_x - compact_workspace_x;

        let mut wide = shell_app();
        wide.switch_mode(ModeId("browse"));
        wide.data.capabilities = &["bytecode", "source", "generated-export", "semantic-diff"];
        let wide_frame = render_text(&mut wide, 120, 32);
        let wide_workspace_x = first_char_col(&wide_frame, "Workspace").expect("wide rail");
        let wide_canvas_x = first_char_col(&wide_frame, "Canvas").expect("wide canvas");
        let wide_inspector_x = first_char_col(&wide_frame, "Inspector").expect("wide inspector");
        let wide_rail_span = wide_canvas_x - wide_workspace_x;
        let wide_canvas_span = wide_inspector_x - wide_canvas_x;

        assert!(
            wide_rail_span >= compact_rail_span + 2,
            "wide terminals should spend some extra room on the navigation rail\ncompact rail span: {compact_rail_span}, wide rail span: {wide_rail_span}\n{wide_frame}"
        );
        assert!(
            wide_canvas_span > wide_rail_span,
            "expanding the rail should still leave the canvas as the dominant browse pane\n{wide_frame}"
        );
    }

    #[test]
    fn sparse_browse_rail_preserves_canvas_width_on_wide_terminals() {
        let mut app = shell_app();
        app.switch_mode(ModeId("browse"));
        app.data.source_line = Some("set value [expr {1 + 2}]");

        let frame = render_text(&mut app, 120, 32);
        let workspace_x = first_char_col(&frame, "Workspace").expect("workspace");
        let canvas_x = first_char_col(&frame, "Canvas").expect("canvas");
        let inspector_x = first_char_col(&frame, "Inspector").expect("inspector");
        let rail_span = canvas_x - workspace_x;
        let canvas_span = inspector_x - canvas_x;

        assert!(
            rail_span <= 26,
            "sparse browse layouts should not over-allocate width to the navigation rail\nrail span: {rail_span}\n{frame}"
        );
        assert!(
            canvas_span >= 48,
            "sparse browse layouts should preserve canvas reading width\ncanvas span: {canvas_span}\n{frame}"
        );
    }

    #[test]
    fn footer_keeps_context_actions_and_command_feedback_in_one_row() {
        let mut browse = shell_app();
        browse.switch_mode(ModeId("browse"));
        browse.status = "ready".to_string();
        browse.cmd.output = None;
        let browse_frame = render_text(&mut browse, 120, 32);
        let browse_footer = browse_frame.lines().last().unwrap_or_default();

        assert!(
            browse_footer.contains("Items  #1/1"),
            "footer should carry the active view and position\n{browse_frame}"
        );
        assert!(
            !browse_frame.contains("ready"),
            "ambient ready status should not consume persistent chrome\n{browse_frame}"
        );

        browse.cmd.output = Some("exported generated source to /tmp/out.tcl".to_string());
        let output_frame = render_text(&mut browse, 120, 32);
        let output_footer = output_frame.lines().last().unwrap_or_default();
        assert!(
            output_footer.contains("Items  #1/1") && output_footer.contains("exported generated"),
            "real command output should share the single contextual footer row\n{output_frame}"
        );

        let mut compare = shell_app();
        compare.switch_mode(ModeId("compare"));
        compare.diff.active = true;
        compare.diff.target_path = Some("other.tcl".to_string());
        compare.status = "diff opened: other.tcl".to_string();
        let compare_frame = render_text(&mut compare, 120, 32);
        let compare_footer = compare_frame.lines().last().unwrap_or_default();
        assert!(
            compare_footer.contains("Compare") && compare_footer.contains("Target: other.tcl"),
            "compare footer should name the opened target\n{compare_frame}"
        );
        assert!(
            !compare_frame.contains("diff opened: other.tcl"),
            "compare footer should not repeat raw command-status wording\n{compare_frame}"
        );
        assert!(
            !compare_footer.contains("Items"),
            "compare footer should not leak browse selection context\n{compare_frame}"
        );
    }

    #[test]
    fn footer_omits_unavailable_generic_actions() {
        let mut app = shell_app();
        app.data.source_line = Some("set value [expr {1 + 2}]");
        app.data.capabilities = &["source"];
        app.switch_mode(ModeId("browse"));
        app.search.query = Some("expr".to_string());
        app.search.hits = vec![0, 1];

        let frame = render_text(&mut app, 120, 32);
        let footer = frame.lines().last().unwrap_or_default();

        assert!(
            footer.contains("search 1/2 expr"),
            "footer should carry live search context\n{frame}"
        );
        for absent in [
            "Graph n/a",
            "trace n/a",
            "Export n/a",
            "sync n/a",
            "focus List",
        ] {
            assert!(
                !footer.contains(absent),
                "footer should omit {absent:?} rather than showing n/a clutter\n{frame}"
            );
        }
        assert!(
            footer.contains(": Command") && footer.contains("/ Search"),
            "footer should keep always-available browse actions\n{frame}"
        );
    }

    #[test]
    fn status_row_keeps_only_contextual_live_fields() {
        let mut compare = shell_app();
        compare.switch_mode(ModeId("compare"));
        compare.diff.active = true;
        compare.search.query = Some("expr".to_string());
        compare.search.hits = vec![0, 1];
        compare.nav.trace_mode = true;
        compare.nav.trace_index = Some(0);

        let frame = render_text(&mut compare, 120, 32);
        let footer = frame.lines().last().unwrap_or_default();

        assert!(
            footer.contains("Compare") && footer.contains("sync on"),
            "compare footer should keep active sync context visible\n{frame}"
        );
        for stale in [
            "search 1/2 expr",
            "trace on",
            "Export ok",
            "focus",
            "sync yes",
        ] {
            assert!(
                !footer.contains(stale),
                "compare footer should not carry {stale:?}\n{frame}"
            );
        }
    }

    #[test]
    fn capabilities_live_in_help_not_in_persistent_chrome() {
        let mut app = shell_app();
        app.data.capabilities = &["bytecode", "source", "generated-export"];
        app.switch_mode(ModeId("browse"));

        let frame = render_text(&mut app, 120, 32);
        let header = frame.lines().next().unwrap_or_default();
        assert!(
            !header.contains("bytecode") && !header.contains("export"),
            "capabilities never change for an artifact and must not hold a header slot\n{frame}"
        );
        assert!(
            !frame.contains("generated source ready"),
            "live export status belongs in feedback, not chrome\n{frame}"
        );

        app.overlay.show_help = true;
        let help = render_text(&mut app, 120, 32);
        assert!(
            help.contains("bytecode \u{b7} source \u{b7} export"),
            "help should expose reader-facing capability labels\n{help}"
        );
        assert!(
            !help.contains("generated-export"),
            "help should not expose internal capability tokens\n{help}"
        );
    }

    #[test]
    fn mode_chrome_is_compressed_and_action_oriented() {
        let mut app = shell_app();
        app.switch_mode(ModeId("browse"));
        app.status = "ready".to_string();
        app.cmd.output = Some("exported generated source to /tmp/out.tcl".to_string());
        app.data.capabilities = &["bytecode", "source", "generated-export"];

        let frame = render_text(&mut app, 120, 32);
        let header = frame.lines().next().unwrap_or_default();
        let footer = frame.lines().last().unwrap_or_default();

        // Chrome is exactly two rows: the panels start immediately under the
        // header and the row above the footer is still a panel border.
        assert!(
            frame
                .lines()
                .nth(1)
                .unwrap_or_default()
                .starts_with("\u{250c}"),
            "panels should start on row 1\n{frame}"
        );
        assert!(
            frame
                .lines()
                .nth(frame.lines().count() - 2)
                .unwrap_or_default()
                .contains("\u{2518}"),
            "the row above the footer should still be panel, not chrome\n{frame}"
        );

        assert!(header.contains("[b] Browse"), "{frame}");
        assert!(
            !header.contains("[b] B Browse"),
            "mode switcher should avoid duplicate key/icon labels\n{frame}"
        );
        for banished in ["State: ready", "theme:", "Capabilities:", "Modes:", "Path:"] {
            assert!(
                !frame.contains(banished),
                "{banished:?} should no longer occupy chrome\n{frame}"
            );
        }

        assert!(
            footer.contains("Items  #1/1")
                && footer.contains("search 1/2 expr")
                && footer.contains("trace on #0"),
            "footer should carry position and live fields\n{frame}"
        );
        for clutter in [
            "focus List",
            "sync n/a",
            "Search:1/2",
            "trace on#0",
            "Position list 0",
            "Affordances:",
        ] {
            assert!(
                !frame.contains(clutter),
                "footer should not contain {clutter:?}\n{frame}"
            );
        }
        assert!(
            footer.contains(":export"),
            "key hints must survive alongside a message, even compacted\n{frame}"
        );

        // With no message competing for the row, hints render at full length.
        app.cmd.output = None;
        let quiet = render_text(&mut app, 120, 32);
        let quiet_footer = quiet.lines().last().unwrap_or_default();
        assert!(
            quiet_footer.contains(": Command") && quiet_footer.contains("/ Search"),
            "an uncontested footer should use descriptive key labels\n{quiet}"
        );
        app.cmd.output = Some("exported generated source to /tmp/out.tcl".to_string());

        // Under width pressure the row sheds fields and feedback but never the
        // active view or the key hints.
        let compact = render_text(&mut app, 72, 24);
        let compact_footer = compact.lines().last().unwrap_or_default();
        assert!(
            compact_footer.trim_start().starts_with("Items"),
            "compact footer should keep the active view\n{compact}"
        );
        assert!(
            compact_footer.contains(":export"),
            "key hints are guaranteed their compact form at any width\n{compact}"
        );
        assert!(
            compact_footer.contains("exported generated"),
            "a message outranks the persistent fields it displaces\n{compact}"
        );
        assert!(
            compact
                .lines()
                .next()
                .unwrap_or_default()
                .contains("[b] Browse"),
            "compact header should preserve the mode switch\n{compact}"
        );
    }

    #[test]
    fn footer_keys_step_down_density_as_room_runs_out() {
        let affordances = ["Command", "Palette", "Help", "Search", "Trace", "Export"];

        let theme = crate::theme::Theme::new(true);
        let variants = footer_key_variants(&affordances);
        let render = |width| {
            chrome::fit(&variants, width, &theme)
                .spans
                .iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        };
        let (roomy, mid, tight) = (render(120), render(50), render(30));

        assert_eq!(
            roomy,
            ": Command | Ctrl-P Palette | ? Help | / Search | t Trace | :export Export"
        );
        assert!(
            cell_width(&mid) <= 50 && mid.contains("Search"),
            "mid density should fit and keep readable labels: {mid}"
        );
        // Narrower than even the tersest phrasing, the row still emits it on one
        // line and the surface clips it; wrapping would corrupt the frame.
        assert_eq!(tight, ": | Ctrl-P | ? | / | t | :export");
        assert!(
            cell_width(&tight) < cell_width(&mid),
            "the tersest phrasing is still the shortest available: {tight}"
        );
    }

    #[test]
    fn header_keeps_identity_mode_and_provenance_on_one_row() {
        let mut app = shell_app();
        app.data.capabilities = &["bytecode", "source", "generated-export", "semantic-diff"];

        let browse = render_text(&mut app, 120, 32);
        let header = browse.lines().next().unwrap_or_default();
        assert!(
            header.contains("tcld explore")
                && header.contains("expr.tbc")
                && header.contains("[b] Browse")
                && header.contains("[C] Compare")
                && header.contains("source: generated"),
            "header row should carry identity, mode switch and provenance\n{browse}"
        );
        assert!(
            !browse.lines().nth(1).unwrap_or_default().contains("Modes:"),
            "the mode list should not need a row of its own\n{browse}"
        );

        app.switch_mode(ModeId("compare"));
        let compare = render_text(&mut app, 120, 32);
        assert!(
            compare
                .lines()
                .next()
                .unwrap_or_default()
                .contains("[C] Compare"),
            "compare header should keep the mode switch\n{compare}"
        );

        app.switch_mode(ModeId("browse"));
        let narrow = render_text(&mut app, MIN_TERMINAL_WIDTH, 24);
        let narrow_header = narrow.lines().next().unwrap_or_default();
        assert!(
            narrow_header.contains("[b] Browse"),
            "minimum-width header should retain the mode switch before provenance\n{narrow}"
        );
        assert!(
            !narrow_header.contains("generated-export"),
            "narrow headers should never expose internal chip names\n{narrow}"
        );
    }

    #[test]
    fn document_canvas_rows_signal_truncated_labels_with_ellipsis() {
        let mut app = shell_app();
        app.data.source_line = Some("set supercalifragilisticValue [expr {1 + 2}]");

        let frame = render_text(&mut app, 72, 24);

        assert!(
            frame.contains("supercalifragilistic") && frame.contains('…'),
            "document rows should show an ellipsis instead of hard terminal clipping\n{frame}"
        );
        assert!(
            !frame.contains("supercalifragilisticValue [expr"),
            "document rows should not overflow their panel before truncating\n{frame}"
        );
    }

    #[test]
    fn fitted_text_trims_padding_before_ellipsis() {
        assert_eq!(
            fit_text("push1 0     # value", 12),
            "push1 0…",
            "truncation should not preserve spacer padding before the ellipsis"
        );
    }

    #[test]
    fn command_window_keeps_text_on_both_sides_of_the_cursor() {
        let input = "export /very/long/path/generated.tcl";
        let cursor = input.find("long").expect("cursor marker");
        let (before, after) = command_window(input, cursor, 20);

        assert!(before.ends_with("/very/"), "{before:?}");
        assert!(after.starts_with("long"), "{after:?}");
        assert!(cell_width(&before) + cell_width(&after) <= 20);
    }

    #[test]
    fn compare_mode_geometry_is_distinct_when_diff_is_active() {
        let mut app = shell_app();
        app.execute_command("diff other.tcl");
        app.switch_mode(ModeId("compare"));
        app.diff.active = true;
        app.diff.target_path = Some("other.tcl".to_string());
        app.diff.diff_document = Some("semantic diff\nchanged c000: ops\n".to_string());
        app.status = "diff opened: other.tcl".to_string();
        let frame = render_text(&mut app, 120, 32);

        let title_row = frame.lines().next().unwrap_or_default();
        assert!(
            title_row.starts_with(" tcld explore \u{2502} expr.tbc"),
            "compare title row should keep product and file first\n{frame}"
        );
        assert!(
            !title_row.contains("Comparison"),
            "compare title row should not repeat the active mode as a noun suffix\n{frame}"
        );
        assert_geometry_token(&frame, "[C] Compare", 0, 0);
        assert_geometry_token(&frame, "Left Source", 1, 0);
        assert_geometry_token(&frame, "Diff Report", 1, 55);

        let footer = frame.lines().last().unwrap_or_default();
        assert!(
            footer.contains("Compare")
                && footer.contains("sync on")
                && footer.contains("Target: other.tcl"),
            "compare footer should carry mode, sync and diff target in one row\n{frame}"
        );
        assert!(
            footer.contains("Q/Esc Close"),
            "compare footer should keep its key hints\n{frame}"
        );
        assert!(
            !frame.contains("diff opened: other.tcl"),
            "compare chrome should not repeat raw command-status wording\n{frame}"
        );
        assert!(
            !frame.contains("sync n/a") && !frame.contains("Sync: yes"),
            "compare status should not spend space on inactive or boolean-looking sync\n{frame}"
        );
    }

    #[test]
    fn mode_shell_responsive_contract() {
        let mut compact = shell_app();
        compact.switch_mode(ModeId("browse"));
        compact.layout.focused_pane = PaneFocus::Decompile;
        let compact_frame = render_text(&mut compact, 72, 24);
        assert!(compact_frame.contains("[b] Browse"), "{compact_frame}");
        assert!(compact_frame.contains("Workspace"), "{compact_frame}");
        assert!(compact_frame.contains("Canvas"), "{compact_frame}");
        assert!(
            compact_frame
                .lines()
                .last()
                .unwrap_or_default()
                .starts_with(" Items"),
            "{compact_frame}"
        );
        assert_no_text_overlap(&compact_frame, 72);
    }

    #[test]
    fn mode_shell_overlays_are_contextual_app_surfaces() {
        let mut app = shell_app();
        app.switch_mode(ModeId("browse"));
        app.overlay.show_help = true;
        let help = render_text(&mut app, 100, 28);
        assert!(help.contains("Quick Reference"), "{help}");
        assert!(help.contains("Modes"), "{help}");
        assert!(help.contains("Movement"), "{help}");
        assert!(help.contains("Views"), "{help}");
        assert!(help.contains("Commands"), "{help}");
        assert!(help.contains("Browse"), "{help}");
        assert!(help.contains("Compare"), "{help}");
        assert!(
            help.contains("Browse - Command | Search | Trace | Export"),
            "help mode summaries should separate affordances for scanning\n{help}"
        );
        assert!(
            !help.contains("Runtime Debug") && !help.contains("rdebug"),
            "help must not advertise the removed Explore debugger\n{help}"
        );
        assert!(
            !help.contains("Command Search Trace Export"),
            "help should not collapse mode affordances into a word run\n{help}"
        );
        assert!(
            !help.contains("Registered Commands"),
            "help should avoid duplicate command dumps\n{help}"
        );
        assert!(
            help.contains("PgUp/PgDn  Page"),
            "help movement legend should spell out both page keys\n{help}"
        );
        assert!(
            !help.contains("PgUp/Dn"),
            "help movement legend should not abbreviate PgDn as Dn\n{help}"
        );

        let mut source_help_app = shell_app();
        source_help_app.data.source_line = Some("set value [expr {1 + 2}]");
        source_help_app.data.capabilities = &["source"];
        source_help_app.overlay.show_help = true;
        let source_help = render_text(&mut source_help_app, 100, 28);
        assert!(
            source_help.contains("Browse - Command | Search"),
            "source-only help should keep available browse actions visible\n{source_help}"
        );
        assert!(
            !source_help.contains("Browse - Command | Search | Trace | Export"),
            "source-only help should not advertise unavailable trace or export actions\n{source_help}"
        );

        app.overlay.show_help = false;
        app.palette.active = true;
        app.palette.query.clear();
        let empty_palette = render_text(&mut app, 100, 28);
        assert!(empty_palette.contains("Command Palette"), "{empty_palette}");
        assert!(
            empty_palette.contains("Filter: type to search actions"),
            "empty palette should invite filtering instead of showing a blank prompt\n{empty_palette}"
        );
        assert!(
            empty_palette.contains("available"),
            "empty palette should show how much it can search\n{empty_palette}"
        );

        app.palette.query = "compare".to_string();
        let palette = render_text(&mut app, 100, 28);
        assert!(palette.contains("Command Palette"), "{palette}");
        assert!(palette.contains("Filter: compare"), "{palette}");
        assert!(palette.contains("matches"), "{palette}");
        assert!(palette.contains("Enter: run"), "{palette}");
        assert!(palette.contains("Esc: close"), "{palette}");
        assert!(palette.contains("Showing"), "{palette}");
        assert!(palette.contains("Open Compare mode"), "{palette}");
        assert!(
            palette.contains("> Open Compare mode"),
            "palette should expose the selected result without relying on color\n{palette}"
        );
        assert!(
            !palette.contains("> mode: Compare"),
            "palette mode/view rows should not lead with internal command prefixes\n{palette}"
        );
        assert!(!palette.contains("rdebug"), "{palette}");
        assert!(!palette.contains("Enter run | Esc close"), "{palette}");

        app.palette.query = "zzzz".to_string();
        let no_matches = render_text(&mut app, 100, 28);
        assert!(no_matches.contains("Filter: zzzz"), "{no_matches}");
        assert!(no_matches.contains("0 matches"), "{no_matches}");
        assert!(no_matches.contains("No matching actions"), "{no_matches}");
        assert!(no_matches.contains("Esc: close"), "{no_matches}");
    }

    fn shell_app() -> crate::app::TuiApp<RenderData> {
        let mut app = crate::app::TuiApp::new_with_theme(
            RenderData {
                path: "expr.tbc",
                header: "tcl 8.6 | source: generated from expr.tbc",
                capabilities: &["bytecode", "source", "generated-export"],
                summary: "summary: bytecode workspace",
                source_line: None,
                decompile: "generated decompiled Tcl source\nset value [expr {1 + 2}]\n",
                no_colors: true,
                empty: false,
            },
            false,
            Some("sunset"),
        );
        app.search.query = Some("expr".to_string());
        app.search.hits = vec![0, 1];
        app.nav.group_focus = Some(0);
        app.nav.trace_mode = true;
        app.nav.trace_index = Some(0);
        app
    }

    fn render_text<D: DataSource>(
        app: &mut crate::app::TuiApp<D>,
        width: u16,
        height: u16,
    ) -> String {
        crate::render_frame_to_text(app, width, height).expect("render frame")
    }

    fn assert_geometry_token(frame: &str, token: &str, min_row: usize, min_col: usize) {
        let Some((row, col)) = find_token(frame, token) else {
            panic!("missing {token:?}\n{frame}");
        };
        assert!(
            row >= min_row,
            "expected {token:?} at or after row {min_row}, got {row}\n{frame}"
        );
        assert!(
            col >= min_col,
            "expected {token:?} at or after col {min_col}, got {col}\n{frame}"
        );
    }

    fn find_token(frame: &str, token: &str) -> Option<(usize, usize)> {
        frame
            .lines()
            .enumerate()
            .find_map(|(row, line)| line.find(token).map(|col| (row, col)))
    }

    fn first_col(frame: &str, token: &str) -> Option<usize> {
        find_token(frame, token).map(|(_, col)| col)
    }

    fn first_char_col(frame: &str, token: &str) -> Option<usize> {
        frame.lines().find_map(|line| {
            let byte_col = line.find(token)?;
            Some(line[..byte_col].chars().count())
        })
    }

    fn assert_no_text_overlap(frame: &str, width: usize) {
        for (row, line) in frame.lines().enumerate() {
            assert!(
                line.chars().count() <= width,
                "row {row} overflowed width {width}: {line:?}\n{frame}"
            );
        }
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for cell in buffer.content() {
            out.push_str(cell.symbol());
        }
        out
    }
}
