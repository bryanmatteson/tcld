use crate::app::TuiApp;
use crate::data::{DataSource, OperationAvailability};
use crate::theme::{BREADCRUMB_SEP, BreadcrumbLevel};
use crate::views;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Render the two-line status bar: breadcrumbs + command line.
pub fn render<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &TuiApp<D>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    render_breadcrumbs(frame, rows[0], app);
    render_command_line(frame, rows[1], app);
}

fn render_breadcrumbs<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &TuiApp<D>) {
    let view = app.current_view();
    let view_title = views::title(app.specs(), view);
    let sel = app.nav.selection(view);

    let mut spans = Vec::new();
    spans.push(Span::raw(" "));

    spans.push(Span::styled("  Focus:", app.theme.fg_dim()));
    spans.push(Span::styled(
        format!("{:?}", app.layout.focused_pane),
        app.theme.accent(),
    ));
    spans.push(Span::styled(" Export:", app.theme.fg_dim()));
    let export = match app.data.export_availability() {
        OperationAvailability::Available => "ready".to_string(),
        OperationAvailability::Unavailable { .. } => "unavailable".to_string(),
    };
    spans.push(Span::styled(export, app.theme.fg_muted()));
    spans.push(Span::styled(" Search:", app.theme.fg_dim()));
    let search = match (&app.search.query, app.search.hits.is_empty()) {
        (Some(query), false) => {
            format!(
                "{}/{} {query}",
                app.search.cursor + 1,
                app.search.hits.len()
            )
        }
        (Some(query), true) => format!("0/0 {query}"),
        (None, _) => "0/0".to_string(),
    };
    spans.push(Span::styled(search, app.theme.fg_muted()));
    spans.push(Span::styled(" Trace:", app.theme.fg_dim()));
    spans.push(Span::styled(
        if app.nav.trace_mode {
            app.nav
                .trace_index
                .map(|idx| format!("on #{idx}"))
                .unwrap_or_else(|| "on".to_string())
        } else {
            "off".to_string()
        },
        app.theme.fg_muted(),
    ));
    spans.push(Span::styled("  Breadcrumb: ", app.theme.fg_dim()));
    spans.push(Span::styled(
        view_title.to_string(),
        app.theme.breadcrumb(BreadcrumbLevel::Block),
    ));

    if let Some(group) = app.nav.group_focus {
        spans.push(Span::styled(
            BREADCRUMB_SEP,
            app.theme.breadcrumb(BreadcrumbLevel::Separator),
        ));
        spans.push(Span::styled(
            format!("c{group:03}"),
            app.theme.breadcrumb(BreadcrumbLevel::Group),
        ));
    }

    spans.push(Span::styled(
        BREADCRUMB_SEP,
        app.theme.breadcrumb(BreadcrumbLevel::Separator),
    ));
    spans.push(Span::styled(
        format!("#{sel}"),
        app.theme.breadcrumb(BreadcrumbLevel::Instruction),
    ));
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_command_line<D: DataSource>(frame: &mut ratatui::Frame<'_>, area: Rect, app: &TuiApp<D>) {
    let view = app.current_view();
    let scroll = app.nav.list_scroll(view);
    let doc_scroll = app
        .data
        .decompile_view()
        .map(|view| app.nav.doc_scroll(view))
        .unwrap_or(0);
    let selected = app.nav.selection(view);
    let prefix = " command: ";
    if app.cmd.active {
        let spans = vec![
            Span::styled(prefix, app.theme.command_prompt()),
            Span::styled(app.cmd.input.clone(), app.theme.command_input()),
            Span::styled("█", app.theme.accent()),
        ];
        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line), area);
    } else if let Some(output) = &app.cmd.output {
        let line = Line::from(vec![
            Span::styled(prefix, app.theme.command_prompt()),
            Span::styled(format!("Selected:#{selected} "), app.theme.fg_dim()),
            Span::styled(
                format!(
                    "Scroll:list {scroll} doc {doc_scroll} detail {} ",
                    app.nav.detail_scroll
                ),
                app.theme.fg_dim(),
            ),
            Span::styled(output.clone(), app.theme.fg_muted()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    } else {
        let line = Line::from(vec![
            Span::styled(prefix, app.theme.command_prompt()),
            Span::styled(format!("Selected:#{selected} "), app.theme.fg_dim()),
            Span::styled(
                format!(
                    "Scroll:list {scroll} doc {doc_scroll} detail {} ",
                    app.nav.detail_scroll
                ),
                app.theme.fg_dim(),
            ),
            Span::styled(app.status.clone(), app.theme.fg_dim()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }
}
