use crate::app::{PaletteAction, TuiApp};
use crate::bindings::{self, BindingAction};
use crate::data::{DataSource, ViewId};
use crate::helpers::point_in_rect;
use crate::state::{HitRegion, ModeLayoutProfile, PaneFocus, RailTarget};
use crate::views;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

/// Handle a key press event. Returns nothing; modifies app state directly.
pub fn handle_key<D: DataSource>(app: &mut TuiApp<D>, key: KeyEvent) {
    if matches!(
        bindings::resolve_normal(app, key),
        Some(BindingAction::QuitImmediate)
    ) {
        app.overlay.should_quit = true;
    }

    // Help overlay
    if app.overlay.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                app.overlay.show_help = false;
                app.overlay.help_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::PageDown => {
                let step = if matches!(key.code, KeyCode::PageDown) {
                    app.data.page_size()
                } else {
                    1
                };
                app.overlay.help_scroll = app.overlay.help_scroll.saturating_add(step);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::PageUp => {
                let step = if matches!(key.code, KeyCode::PageUp) {
                    app.data.page_size()
                } else {
                    1
                };
                app.overlay.help_scroll = app.overlay.help_scroll.saturating_sub(step);
            }
            KeyCode::Home | KeyCode::Char('g') => app.overlay.help_scroll = 0,
            _ => {}
        }
        return;
    }

    // Summary modal
    if app.overlay.show_summary {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('s') => {
                app.overlay.show_summary = false;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.overlay.summary_scroll = app.overlay.summary_scroll.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.overlay.summary_scroll = app.overlay.summary_scroll.saturating_sub(1);
            }
            _ => {}
        }
        return;
    }

    // Command palette
    if app.palette.active {
        handle_palette_key(app, key);
        return;
    }

    // Command mode
    if app.cmd.active {
        handle_command_key(app, key);
        return;
    }

    // Normal mode
    handle_normal_key(app, key);
}

fn handle_palette_key<D: DataSource>(app: &mut TuiApp<D>, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.palette.active = false;
            app.palette.query.clear();
            app.palette.index = 0;
        }
        KeyCode::Enter => {
            app.palette.active = false;
            let action = app.selected_palette_action();
            app.palette.query.clear();
            app.palette.index = 0;
            if let Some(action) = action {
                match action {
                    PaletteAction::Execute(command) => app.execute_command(&command),
                    PaletteAction::Prefill(prefix) => {
                        app.cmd.active = true;
                        set_command_input(app, prefix);
                    }
                }
            }
        }
        KeyCode::Backspace => {
            app.palette.query.pop();
            app.palette.index = 0;
        }
        KeyCode::Down => {
            let max = app.palette_entries().len().saturating_sub(1);
            app.palette.index = app.palette.index.saturating_add(1).min(max);
        }
        KeyCode::Up => {
            app.palette.index = app.palette.index.saturating_sub(1);
        }
        KeyCode::Char(c) => {
            app.palette.query.push(c);
            app.palette.index = 0;
        }
        _ => {}
    }
}

fn handle_command_key<D: DataSource>(app: &mut TuiApp<D>, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.cmd.active = false;
            app.cmd.input.clear();
            app.cmd.cursor = 0;
        }
        KeyCode::Enter => {
            let line = std::mem::take(&mut app.cmd.input);
            app.cmd.cursor = 0;
            app.cmd.active = false;
            if !line.is_empty() {
                app.cmd.history.push(line.clone());
                app.cmd.history_index = None;
                app.execute_command(&line);
            }
        }
        KeyCode::Backspace => {
            command_backspace(app);
            if app.cmd.input.is_empty() {
                app.cmd.active = false;
            }
        }
        KeyCode::Delete => command_delete(app),
        KeyCode::Left => {
            app.cmd.cursor = previous_char_boundary(&app.cmd.input, app.cmd.cursor);
        }
        KeyCode::Right => {
            app.cmd.cursor = next_char_boundary(&app.cmd.input, app.cmd.cursor);
        }
        KeyCode::Home => app.cmd.cursor = 0,
        KeyCode::End => app.cmd.cursor = app.cmd.input.len(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cmd.input.drain(..app.cmd.cursor);
            app.cmd.cursor = 0;
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            command_delete_previous_word(app);
        }
        KeyCode::Up if !app.cmd.history.is_empty() => {
            let idx = app
                .cmd
                .history_index
                .map(|i| i.saturating_sub(1))
                .unwrap_or(app.cmd.history.len() - 1);
            set_command_input(app, app.cmd.history[idx].clone());
            app.cmd.history_index = Some(idx);
        }
        KeyCode::Down => {
            if let Some(idx) = app.cmd.history_index {
                if idx + 1 < app.cmd.history.len() {
                    let next = idx + 1;
                    set_command_input(app, app.cmd.history[next].clone());
                    app.cmd.history_index = Some(next);
                } else {
                    app.cmd.input.clear();
                    app.cmd.cursor = 0;
                    app.cmd.history_index = None;
                }
            }
        }
        KeyCode::Char(c) => {
            insert_command_text(app, &c.to_string());
        }
        _ => {}
    }
}

fn handle_normal_key<D: DataSource>(app: &mut TuiApp<D>, key: KeyEvent) {
    // Messages answer the key that produced them; the next key clears them
    // so a stale message never describes a state that has moved on.
    app.cmd.output = None;
    app.status = "ready".to_string();

    if let Some(action) = bindings::resolve_normal(app, key) {
        match action {
            BindingAction::QuitImmediate => app.overlay.should_quit = true,
            BindingAction::Help => app.overlay.show_help = true,
            BindingAction::Command => {
                app.cmd.active = true;
                set_command_input(app, String::new());
            }
            BindingAction::Search => {
                app.cmd.active = true;
                set_command_input(app, "search ".to_string());
            }
            BindingAction::Palette => {
                app.palette.active = true;
                app.palette.query.clear();
                app.palette.index = 0;
            }
            BindingAction::CloseDiff => {
                if app.diff.active {
                    close_diff(app);
                }
            }
            BindingAction::ScrollDetail(delta) => {
                if delta > 0 {
                    app.nav.detail_scroll = app.nav.detail_scroll.saturating_add(delta as usize);
                } else {
                    app.nav.detail_scroll = app.nav.detail_scroll.saturating_sub((-delta) as usize);
                }
            }
            BindingAction::ToggleMap => app.execute_command("map"),
            BindingAction::CycleMap => app.cycle_rail_mode(),
            BindingAction::AdjustMapContrast(delta) => app.adjust_rail_contrast(delta),
            BindingAction::ToggleTrace => app.execute_command("trace"),
            BindingAction::RevealDecompile => app.show_decompile_for_current(),
            BindingAction::NavigateGroup(delta) => {
                if app.current_layout_profile() == ModeLayoutProfile::Compare {
                    app.navigate_diff_change(delta.into());
                } else {
                    app.navigate_group(delta.into());
                }
            }
            BindingAction::Follow => {
                if app.current_layout_profile() == ModeLayoutProfile::Browse {
                    if app.layout.focused_pane == PaneFocus::List
                        && !views::is_list_view(app.specs(), app.current_view())
                    {
                        // From the outline, Enter opens the line in the canvas.
                        app.layout.focused_pane = PaneFocus::Decompile;
                    } else {
                        app.follow_link();
                    }
                }
            }
            BindingAction::Back => app.go_back(),
            BindingAction::ToggleInspector => {
                app.nav.expanded = !app.nav.expanded;
                app.nav.detail_scroll = 0;
                app.status = if app.nav.expanded {
                    "inspector zoomed · z restores".to_string()
                } else {
                    "inspector restored".to_string()
                };
            }
            BindingAction::FocusNext => focus_next(app),
            BindingAction::FocusPrevious => focus_previous(app),
            BindingAction::Navigate(delta) => navigate_active(app, delta.into()),
            BindingAction::First => navigate_to_edge(app, false),
            BindingAction::Last => navigate_to_edge(app, true),
            BindingAction::Page(direction) => {
                let page = app.data.page_size() as i32;
                navigate_active(app, page.saturating_mul(direction.into()));
            }
            BindingAction::SearchHit(direction) => {
                if app.search.query.is_some() && !app.search.hits.is_empty() {
                    app.advance_search(direction.into());
                }
            }
            BindingAction::Escape => handle_layered_escape(app),
            BindingAction::Mode(mode) => {
                app.switch_mode(mode);
            }
            BindingAction::View(view) => {
                if view != app.current_view() {
                    app.push_history();
                }
                app.switch_to_view(view);
            }
        }
    }
}

fn set_command_input<D: DataSource>(app: &mut TuiApp<D>, input: String) {
    app.cmd.cursor = input.len();
    app.cmd.input = input;
}

pub(crate) fn insert_command_text<D: DataSource>(app: &mut TuiApp<D>, text: &str) {
    app.cmd.cursor = app.cmd.cursor.min(app.cmd.input.len());
    while !app.cmd.input.is_char_boundary(app.cmd.cursor) {
        app.cmd.cursor = app.cmd.cursor.saturating_sub(1);
    }
    app.cmd.input.insert_str(app.cmd.cursor, text);
    app.cmd.cursor += text.len();
}

fn command_backspace<D: DataSource>(app: &mut TuiApp<D>) {
    let previous = previous_char_boundary(&app.cmd.input, app.cmd.cursor);
    if previous < app.cmd.cursor {
        app.cmd.input.drain(previous..app.cmd.cursor);
        app.cmd.cursor = previous;
    }
}

fn command_delete<D: DataSource>(app: &mut TuiApp<D>) {
    let next = next_char_boundary(&app.cmd.input, app.cmd.cursor);
    if next > app.cmd.cursor {
        app.cmd.input.drain(app.cmd.cursor..next);
    }
}

fn command_delete_previous_word<D: DataSource>(app: &mut TuiApp<D>) {
    let prefix = &app.cmd.input[..app.cmd.cursor.min(app.cmd.input.len())];
    let trimmed = prefix.trim_end_matches(char::is_whitespace);
    let word_start = trimmed
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    app.cmd.input.drain(word_start..app.cmd.cursor);
    app.cmd.cursor = word_start;
}

fn previous_char_boundary(input: &str, cursor: usize) -> usize {
    let cursor = cursor.min(input.len());
    input[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_char_boundary(input: &str, cursor: usize) -> usize {
    let cursor = cursor.min(input.len());
    input[cursor..]
        .chars()
        .next()
        .map(|character| cursor + character.len_utf8())
        .unwrap_or(cursor)
}

/// Whether the rail has anything for the keyboard to do: list views are
/// navigated in the canvas itself, documents also through their outline.
fn rail_is_focusable<D: DataSource>(app: &mut TuiApp<D>) -> bool {
    let view = app.current_view();
    !views::is_list_view(app.specs(), view) && !app.outline(view).is_empty()
}

fn focus_next<D: DataSource>(app: &mut TuiApp<D>) {
    if app.current_layout_profile() == ModeLayoutProfile::Compare {
        set_compare_focus(app, !app.diff.focus_right);
        return;
    }
    let rail = rail_is_focusable(app);
    app.layout.focused_pane = match app.layout.focused_pane {
        PaneFocus::List => PaneFocus::Decompile,
        PaneFocus::Decompile => PaneFocus::Context,
        PaneFocus::Context if rail => PaneFocus::List,
        PaneFocus::Context => PaneFocus::Decompile,
    };
    announce_focus(app);
}

fn focus_previous<D: DataSource>(app: &mut TuiApp<D>) {
    if app.current_layout_profile() == ModeLayoutProfile::Compare {
        set_compare_focus(app, !app.diff.focus_right);
        return;
    }
    let rail = rail_is_focusable(app);
    app.layout.focused_pane = match app.layout.focused_pane {
        PaneFocus::List => PaneFocus::Context,
        PaneFocus::Decompile if rail => PaneFocus::List,
        PaneFocus::Decompile => PaneFocus::Context,
        PaneFocus::Context => PaneFocus::Decompile,
    };
    announce_focus(app);
}

fn announce_focus<D: DataSource>(app: &mut TuiApp<D>) {
    app.status = match app.layout.focused_pane {
        PaneFocus::List => "outline focused · j/k jumps between procs".to_string(),
        PaneFocus::Decompile => {
            format!("{} focused", views::title(app.specs(), app.current_view()))
        }
        PaneFocus::Context => "inspector focused · j/k scrolls it".to_string(),
    };
}

fn set_compare_focus<D: DataSource>(app: &mut TuiApp<D>, focus_right: bool) {
    app.diff.focus_right = focus_right;
    app.status = if focus_right {
        "comparison diff focused".to_string()
    } else {
        "comparison source focused".to_string()
    };
}

fn navigate_to_edge<D: DataSource>(app: &mut TuiApp<D>, last: bool) {
    let view = app.current_view();
    if views::is_list_view(app.specs(), view) {
        let selection = if last {
            app.data.list_len(view).saturating_sub(1)
        } else {
            0
        };
        app.nav.selections.insert(view, selection);
        update_group_focus(app);
    } else {
        let line = if last {
            app.cached_document(view)
                .map(|document| document.lines.len().saturating_sub(1))
                .unwrap_or(0)
        } else {
            0
        };
        app.nav.doc_cursors.insert(view, line);
    }
}

fn close_diff<D: DataSource>(app: &mut TuiApp<D>) {
    app.diff.active = false;
    app.diff.target_path = None;
    app.diff.diff_document = None;
    app.diff.change_lines.clear();
    app.diff.right_scroll = 0;
    app.diff.focus_right = false;
    app.status = "closed diff view".to_string();
}

fn handle_layered_escape<D: DataSource>(app: &mut TuiApp<D>) {
    if app.diff.active {
        close_diff(app);
    } else if app.nav.expanded {
        app.nav.expanded = false;
        app.nav.detail_scroll = 0;
    } else if app.nav.trace_mode {
        app.nav.trace_mode = false;
        app.status = "trace mode disabled".to_string();
    } else if app.search.query.is_some() {
        app.search = Default::default();
        app.status = "search cleared".to_string();
    } else if app.layout.focused_pane != PaneFocus::Decompile {
        app.layout.focused_pane = PaneFocus::Decompile;
    } else {
        // Esc only ever backs out of things; it never quits.
        app.status = "nothing to close · q quits".to_string();
    }
}

/// Navigate based on the focused pane.
fn navigate_active<D: DataSource>(app: &mut TuiApp<D>, delta: i32) {
    if app.current_layout_profile() == ModeLayoutProfile::Compare {
        if app.diff.focus_right {
            let max = app
                .diff
                .diff_document
                .as_deref()
                .map(|document| document.lines().count().saturating_sub(1))
                .unwrap_or(0);
            app.diff.right_scroll = offset(app.diff.right_scroll, delta, max);
        } else if let Some(view) = app.data.decompile_view() {
            scroll_doc(app, view, delta);
        }
        return;
    }

    let view = app.current_view();
    match app.layout.focused_pane {
        PaneFocus::Context => {
            if delta > 0 {
                app.nav.detail_scroll = app.nav.detail_scroll.saturating_add(delta as usize);
            } else {
                app.nav.detail_scroll = app.nav.detail_scroll.saturating_sub((-delta) as usize);
            }
            return;
        }
        PaneFocus::List if !views::is_list_view(app.specs(), view) => {
            step_outline(app, view, delta);
            return;
        }
        _ if !views::is_list_view(app.specs(), view) => {
            move_doc_cursor(app, view, delta);
            return;
        }
        _ => {}
    }

    let max = app.data.list_len(view).saturating_sub(1);
    let sel = app.nav.selection(view);
    let new_sel = if delta > 0 {
        sel.saturating_add(delta as usize).min(max)
    } else {
        sel.saturating_sub((-delta) as usize)
    };
    app.nav.selections.insert(view, new_sel);
    update_group_focus(app);
}

fn scroll_doc<D: DataSource>(app: &mut TuiApp<D>, view: ViewId, delta: i32) {
    let max = app
        .cached_document(view)
        .map(|document| document.lines.len().saturating_sub(1))
        .unwrap_or(0);
    let scroll = offset(app.nav.doc_scroll(view), delta, max);
    app.nav.doc_scrolls.insert(view, scroll);
}

fn move_doc_cursor<D: DataSource>(app: &mut TuiApp<D>, view: ViewId, delta: i32) {
    let max = app
        .cached_document(view)
        .map(|document| document.lines.len().saturating_sub(1))
        .unwrap_or(0);
    let cursor = offset(app.nav.doc_cursor(view), delta, max);
    app.nav.doc_cursors.insert(view, cursor);
}

/// Move the document cursor `delta` outline entries (procs, top-level
/// commands) from where it is.
fn step_outline<D: DataSource>(app: &mut TuiApp<D>, view: ViewId, delta: i32) {
    let outline = app.outline(view);
    if outline.is_empty() {
        move_doc_cursor(app, view, delta);
        return;
    }
    let cursor = app.nav.doc_cursor(view);
    let current = outline
        .iter()
        .rposition(|(line, _)| *line <= cursor)
        .unwrap_or(0);
    let target = if delta > 0 {
        (current + delta as usize).min(outline.len() - 1)
    } else if outline[current].0 < cursor {
        // Mid-entry, "up" first returns to the entry's own start.
        current.saturating_sub((-delta) as usize - 1)
    } else {
        current.saturating_sub((-delta) as usize)
    };
    let (line, label) = outline[target].clone();
    app.place_doc_cursor(view, line);
    app.status = label;
}

fn offset(current: usize, delta: i32, max: usize) -> usize {
    if delta > 0 {
        current.saturating_add(delta as usize).min(max)
    } else {
        current.saturating_sub((-delta) as usize)
    }
}

fn update_group_focus<D: DataSource>(app: &mut TuiApp<D>) {
    let view = app.current_view();
    let sel = app.nav.selection(view);
    app.nav.group_focus = app.data.group_for_index(sel);
    app.nav.trace_index = Some(sel);
}

/// Handle a mouse event.
pub fn handle_mouse<D: DataSource>(app: &mut TuiApp<D>, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let x = mouse.column;
            let y = mouse.row;

            for (mode, area) in app.hit.mode_switches.clone() {
                if point_in_rect(x, y, area) {
                    app.switch_mode(mode);
                    return;
                }
            }

            for target in app.hit.list_targets.clone() {
                if point_in_rect(x, y, target.area) {
                    let index = target.start + (y - target.area.y) as usize;
                    if index < target.end {
                        app.nav.selections.insert(target.view, index);
                        update_group_focus(app);
                        app.layout.focused_pane = PaneFocus::Decompile;
                    }
                    return;
                }
            }

            for (area, target) in app.hit.rail_targets.clone() {
                if point_in_rect(x, y, area) {
                    match target {
                        RailTarget::View(view) => {
                            if view != app.current_view() {
                                app.push_history();
                            }
                            app.switch_to_view(view);
                            app.layout.focused_pane = PaneFocus::Decompile;
                        }
                        RailTarget::OutlineLine(line) => {
                            let view = app.current_view();
                            app.push_history();
                            app.place_doc_cursor(view, line);
                            app.layout.focused_pane = PaneFocus::List;
                        }
                    }
                    return;
                }
            }

            if let Some((area, view, top)) = app.hit.document_target
                && point_in_rect(x, y, area)
            {
                let line = top + (y - area.y) as usize;
                let max = app
                    .cached_document(view)
                    .map(|document| document.lines.len().saturating_sub(1))
                    .unwrap_or(0);
                app.nav.doc_cursors.insert(view, line.min(max));
                app.layout.focused_pane = PaneFocus::Decompile;
                return;
            }

            if hit_region(app, HitRegion::BrowseRail, x, y) {
                app.layout.focused_pane = PaneFocus::List;
                return;
            }

            if hit_region(app, HitRegion::Canvas, x, y)
                || hit_region(app, HitRegion::Minimap, x, y)
                || hit_region(app, HitRegion::CompareLeft, x, y)
            {
                if hit_region(app, HitRegion::CompareLeft, x, y) {
                    app.diff.focus_right = false;
                }
                app.layout.focused_pane = PaneFocus::Decompile;
                app.status = match app.current_layout_profile() {
                    ModeLayoutProfile::Compare => "comparison source focused".to_string(),
                    ModeLayoutProfile::Browse => "canvas focused".to_string(),
                };
                return;
            }

            if hit_region(app, HitRegion::Inspector, x, y) {
                app.layout.focused_pane = PaneFocus::Context;
                app.status = "inspector focused".to_string();
                return;
            }

            if hit_region(app, HitRegion::CompareRight, x, y) {
                if hit_region(app, HitRegion::CompareRight, x, y) {
                    app.diff.focus_right = true;
                }
                app.layout.focused_pane = PaneFocus::Decompile;
                app.status = "output region focused".to_string();
                return;
            }

            if hit_region(app, HitRegion::FooterCommand, x, y) {
                app.cmd.active = true;
                app.cmd.input.clear();
                app.cmd.cursor = 0;
                return;
            }

            // Compatibility fallback for callers that populate the legacy fields.
            if let Some(list_area) = app.hit.list_area
                && point_in_rect(x, y, list_area)
            {
                let row_offset = (y - list_area.y) as usize;
                let index = app.hit.list_start + row_offset;
                if let Some(view) = app.hit.list_view {
                    let max = app.data.list_len(view).saturating_sub(1);
                    if index <= max {
                        app.nav.selections.insert(view, index);
                        update_group_focus(app);
                        app.layout.focused_pane = PaneFocus::Decompile;
                    }
                }
                return;
            }

            // Click in decompile rail
            if let Some(rail_area) = app.hit.decompile_rail_area
                && point_in_rect(x, y, rail_area)
            {
                let row = (y - rail_area.y) as usize;
                if let Some(&group) = app.hit.decompile_rail_groups.get(row)
                    && group != usize::MAX
                {
                    app.nav.group_focus = Some(group);
                    if let Some(inst_idx) = app.data.first_index_for_group(group) {
                        let specs = app.specs();
                        let available = app.data.available_views();
                        for &v in available {
                            if views::is_list_view(specs, v) {
                                app.nav.selections.insert(v, inst_idx);
                                break;
                            }
                        }
                        app.nav.trace_index = Some(inst_idx);
                    }
                    // Move the decompile cursor to the group's anchor
                    if let Some(dv) = app.data.decompile_view()
                        && let Ok(anchors) = app.data.decompile_anchors()
                        && let Some(&anchor) = anchors.get(group)
                        && anchor != usize::MAX
                    {
                        app.place_doc_cursor(dv, anchor);
                    }
                    app.status = format!("focused group c{group:03}");
                }
            }
        }
        MouseEventKind::ScrollDown => {
            navigate_active(app, 3);
        }
        MouseEventKind::ScrollUp => {
            navigate_active(app, -3);
        }
        _ => {}
    }
}

fn hit_region<D: DataSource>(app: &TuiApp<D>, region: HitRegion, x: u16, y: u16) -> bool {
    app.hit
        .regions
        .get(&region)
        .is_some_and(|area| point_in_rect(x, y, *area))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::app::TuiApp;
    use crate::data::ListEntry;
    use crate::state::{HitRegion, ModeId, ModeLayoutProfile, ModeSpec};
    use crate::views::{ViewOpenBehavior, ViewSpec};
    use crossterm::event::KeyModifiers;
    use ratatui::layout::Rect;

    const LIST: ViewId = ViewId("list");
    const DOC: ViewId = ViewId("doc");
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
            token: "list",
            title: "List",
            aliases: &["list"],
            keybind: Some('l'),
            icon: "*",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: true,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: DOC,
            token: "doc",
            title: "Doc",
            aliases: &["doc"],
            keybind: Some('d'),
            icon: "#",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
    ];

    #[derive(Clone, Copy)]
    struct InputData;

    impl DataSource for InputData {
        fn path(&self) -> &str {
            "input.tcl"
        }

        fn available_views(&self) -> &[ViewId] {
            &[LIST, DOC]
        }

        fn default_view(&self) -> ViewId {
            LIST
        }

        fn no_colors(&self) -> bool {
            false
        }

        fn page_size(&self) -> usize {
            3
        }

        fn summary_text(&self) -> String {
            "summary".to_string()
        }

        fn view_specs(&self) -> &[ViewSpec] {
            SPECS
        }

        fn mode_specs(&self) -> &[ModeSpec] {
            MODE_SPECS
        }

        fn list_len(&self, view: ViewId) -> usize {
            if view == LIST { 5 } else { 0 }
        }

        fn list_entry(&self, view: ViewId, index: usize) -> Option<ListEntry> {
            (view == LIST).then(|| ListEntry {
                text: format!("row {index}"),
                group: Some(index),
                is_group_start: true,
            })
        }

        fn list_detail(&self, _view: ViewId, index: usize) -> Option<String> {
            Some(format!("detail {index}"))
        }

        fn jump_index(&self, _view: ViewId, _value: i32) -> Option<usize> {
            None
        }

        fn document_text(&mut self, _view: ViewId) -> Result<String, String> {
            Ok("doc\nsecond\nthird\n".to_string())
        }

        fn command_group_count(&self) -> usize {
            5
        }

        fn group_for_index(&self, index: usize) -> Option<usize> {
            Some(index.min(4))
        }

        fn first_index_for_group(&self, group: usize) -> Option<usize> {
            Some(group.min(4))
        }

        fn decompile_anchor_for_index(&mut self, index: usize) -> Result<Option<usize>, String> {
            Ok(Some(index.min(2)))
        }

        fn decompile_view(&self) -> Option<ViewId> {
            Some(DOC)
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn command_history_traversal_uses_real_command_input_path() {
        let mut app = TuiApp::new(InputData, false);

        handle_key(&mut app, key(KeyCode::Char(':')));
        for c in "doc".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.current_view(), DOC);

        handle_key(&mut app, key(KeyCode::Char(':')));
        for c in "view list".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.current_view(), LIST);
        assert_eq!(
            app.cmd.history,
            vec!["doc".to_string(), "view list".to_string()]
        );

        handle_key(&mut app, key(KeyCode::Char(':')));
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.cmd.input, "view list");
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.cmd.input, "doc");
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.cmd.input, "view list");
        handle_key(&mut app, key(KeyCode::Down));
        assert!(app.cmd.input.is_empty());
        assert_eq!(app.cmd.history_index, None);
    }

    #[test]
    fn command_editor_supports_cursor_delete_and_word_shortcuts() {
        let mut app = TuiApp::new(InputData, false);
        handle_key(&mut app, key(KeyCode::Char(':')));
        for character in "abc".chars() {
            handle_key(&mut app, key(KeyCode::Char(character)));
        }
        handle_key(&mut app, key(KeyCode::Left));
        handle_key(&mut app, key(KeyCode::Char('X')));
        assert_eq!(app.cmd.input, "abXc");
        assert_eq!(app.cmd.cursor, 3);

        handle_key(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.cmd.input, "abc");
        handle_key(&mut app, key(KeyCode::Home));
        handle_key(&mut app, key(KeyCode::Delete));
        assert_eq!(app.cmd.input, "bc");
        handle_key(&mut app, key(KeyCode::End));
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        );
        assert!(app.cmd.input.is_empty());
        assert_eq!(app.cmd.cursor, 0);

        insert_command_text(&mut app, "éx");
        handle_key(&mut app, key(KeyCode::Left));
        handle_key(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.cmd.input, "x");
        assert_eq!(app.cmd.cursor, 0);
    }

    #[test]
    fn help_overlay_scrolls_and_closes_explicitly() {
        let mut app = TuiApp::new(InputData, false);
        app.overlay.show_help = true;

        handle_key(&mut app, key(KeyCode::Char('j')));
        assert!(app.overlay.show_help);
        assert_eq!(app.overlay.help_scroll, 1);
        handle_key(&mut app, key(KeyCode::PageDown));
        assert_eq!(app.overlay.help_scroll, 4);
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(!app.overlay.show_help);
        assert_eq!(app.overlay.help_scroll, 0);
    }

    #[test]
    fn palette_argument_action_prefills_without_executing_bare_command() {
        let mut app = TuiApp::new(InputData, false);
        app.palette.active = true;
        app.palette.query = "search current".to_string();

        handle_key(&mut app, key(KeyCode::Enter));

        assert!(!app.palette.active);
        assert!(app.cmd.active);
        assert_eq!(app.cmd.input, "search ");
        assert_eq!(app.search.query, None);
    }

    #[test]
    fn ctrl_c_quits_from_overlay_and_prompt_contexts() {
        let mut app = TuiApp::new(InputData, false);
        app.overlay.show_help = true;
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(app.overlay.should_quit);

        let mut app = TuiApp::new(InputData, false);
        app.cmd.active = true;
        app.cmd.input = "search value".to_string();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(app.overlay.should_quit);
    }

    #[test]
    fn mouse_click_and_scroll_update_navigation() {
        let mut app = TuiApp::new(InputData, false);
        app.hit.list_area = Some(Rect::new(10, 5, 10, 4));
        app.hit.list_view = Some(LIST);
        app.hit.list_start = 1;

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 12,
                row: 6,
                modifiers: KeyModifiers::empty(),
            },
        );

        assert_eq!(app.nav.selection(LIST), 2);
        assert_eq!(app.nav.group_focus, Some(2));
        assert_eq!(app.layout.focused_pane, PaneFocus::Decompile);

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert_eq!(app.nav.selection(LIST), 4);

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert_eq!(app.nav.selection(LIST), 1);
    }

    #[test]
    fn mode_specific_hit_targets_drive_state() {
        let mut app = TuiApp::new(InputData, false);
        app.hit
            .mode_switches
            .push((ModeId("compare"), Rect::new(2, 1, 10, 1)));
        app.hit
            .regions
            .insert(HitRegion::BrowseRail, Rect::new(0, 4, 16, 8));
        app.hit
            .regions
            .insert(HitRegion::Canvas, Rect::new(18, 4, 42, 12));
        app.hit
            .regions
            .insert(HitRegion::Inspector, Rect::new(62, 4, 22, 12));
        app.hit
            .regions
            .insert(HitRegion::FooterCommand, Rect::new(0, 22, 84, 1));

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 3,
                row: 1,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert_eq!(app.current_mode_id(), ModeId("compare"));
        assert_eq!(app.layout.focused_pane, PaneFocus::Decompile);

        handle_key(&mut app, key(KeyCode::Char('b')));
        assert_eq!(app.current_mode_id(), ModeId("browse"));

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 63,
                row: 5,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert_eq!(app.layout.focused_pane, PaneFocus::Context);

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 20,
                row: 5,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert_eq!(app.layout.focused_pane, PaneFocus::Decompile);

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 3,
                row: 22,
                modifiers: KeyModifiers::empty(),
            },
        );
        assert!(app.cmd.active);
    }

    #[test]
    fn enter_follows_and_backspace_returns() {
        let mut app = TuiApp::new(InputData, false);
        handle_key(&mut app, key(KeyCode::Char('j')));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.nav.selection(LIST), 2);

        // Row 2 has no link of its own, so Enter reveals its decompiled source.
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.current_view(), DOC);
        assert!(app.status.contains("Backspace returns"), "{}", app.status);

        handle_key(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.current_view(), LIST);
        assert_eq!(app.nav.selection(LIST), 2);
        assert!(app.status.contains("back to List"), "{}", app.status);

        handle_key(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.status, "nothing to go back to");
    }

    #[test]
    fn escape_backs_out_but_never_quits_and_q_quits_at_once() {
        let mut app = TuiApp::new(InputData, false);
        for _ in 0..4 {
            handle_key(&mut app, key(KeyCode::Esc));
        }
        assert!(!app.overlay.should_quit);
        assert!(app.status.contains("q quits"), "{}", app.status);

        handle_key(&mut app, key(KeyCode::Char('q')));
        assert!(app.overlay.should_quit);
    }

    #[test]
    fn document_keys_move_a_cursor_and_messages_are_transient() {
        let mut app = TuiApp::new(InputData, false);
        app.switch_to_view(DOC);
        handle_key(&mut app, key(KeyCode::Char('j')));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.nav.doc_cursor(DOC), 2);
        assert_eq!(app.nav.doc_scroll(DOC), 0, "the viewport only follows");
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.nav.doc_cursor(DOC), 2, "the cursor stops at the end");

        app.status = "old message".to_string();
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.status, "ready");
    }

    #[test]
    fn compare_navigation_scrolls_the_focused_side_independently() {
        let mut app = TuiApp::new(InputData, false);
        app.switch_mode(ModeId("compare"));
        app.diff.active = true;
        app.diff.diff_document = Some("one\ntwo\nthree\n".to_string());

        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.nav.doc_scroll(DOC), 1);
        assert_eq!(app.diff.right_scroll, 0);

        handle_key(&mut app, key(KeyCode::Tab));
        assert!(app.diff.focus_right);
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.nav.doc_scroll(DOC), 1);
        assert_eq!(app.diff.right_scroll, 1);

        handle_key(&mut app, key(KeyCode::BackTab));
        assert!(!app.diff.focus_right);
    }
}
