pub mod data_source;
mod model;

pub use model::{ExplorerCore, ExplorerView};
pub use tcld_explore::{ArtifactKind, Capability, Diagnostic, SourceArtifact, SourceProvenance};

use crate::cli::{ExploreArgs, ExploreMode, ExploreOverlay};
use crate::error::Result;
use crate::error::TcldError;
use std::io::IsTerminal;
use tcld_tui::data::DataSource;
use tcld_tui::{CommandModule, ExploreCommand, PaneFocus, Theme, TuiApp, TuiConfig};

pub struct DumpScreenOptions<'a> {
    pub args: ExploreArgs,
    pub width: u16,
    pub height: u16,
    pub focused: Option<&'a str>,
    pub empty: bool,
}

pub fn run_explorer(args: ExploreArgs) -> Result<()> {
    let initial_mode = args.mode;
    if args.dump_screen {
        let frame = dump_screen(DumpScreenOptions {
            args,
            width: 120,
            height: 32,
            focused: None,
            empty: false,
        })?;
        print!("{frame}");
        return Ok(());
    }

    let core = model::ExplorerCore::new(args)?;
    let is_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    if is_tty {
        run_tui(core, initial_mode)
    } else {
        Err(TcldError::Other(non_interactive_terminal_message()))
    }
}

fn run_tui(core: model::ExplorerCore, initial_mode: Option<ExploreMode>) -> Result<()> {
    let theme = core.theme().map(ToString::to_string);

    let config = TuiConfig {
        start_in_command_mode: false,
        theme,
        initial_mode: initial_mode.map(|mode| mode.token().to_string()),
        extra_modules: vec![rdebug_migration_tombstone()],
        custom_handler: Some(Box::new(move |cmd, args| match cmd {
            "rdebug-moved" if args.is_empty() => {
                Some("runtime debugging moved to tcld debug <file>".to_string())
            }
            _ => None,
        })),
    };

    tcld_tui::run(core, config).map_err(|e| crate::error::TcldError::Other(e.to_string()))
}

pub fn dump_screen(options: DumpScreenOptions<'_>) -> Result<String> {
    let requested_mode = options.args.mode;
    let requested_overlay = options.args.overlay;
    let palette_query = options.args.palette_query.clone();
    let diff_target = options.args.diff_target.clone();
    let core = model::ExplorerCore::new(options.args)?;
    let theme = core.theme().map(ToString::to_string);
    if let Some(theme_name) = theme.as_deref() {
        Theme::with_palette(core.no_colors(), theme_name).map_err(TcldError::Other)?;
    }
    let mut app = TuiApp::new_with_theme(core, false, theme.as_deref());
    app.switch_to_view(app.data.default_view());
    let view_index = app.nav.view_index;
    app.nav = Default::default();
    app.nav.view_index = view_index;
    app.layout.decompile_map_visible = true;
    app.extra_modules = vec![rdebug_migration_tombstone()];
    app.status = "ready".to_string();
    app.search.query = Some("expr".to_string());
    app.search.hits = vec![0];
    app.nav.group_focus = app.data.group_for_index(0);
    app.nav.trace_index = Some(0);

    if app.data.trace_len() > 0 {
        app.nav.trace_mode = true;
    }

    if let Some(target) = diff_target {
        app.execute_command(&format!("diff {target}"));
    }

    if let Some(mode) = requested_mode {
        app.switch_mode_token(mode.token());
    }

    match options.focused {
        Some("decompile") | Some("canvas") => app.layout.focused_pane = PaneFocus::Decompile,
        Some("context") | Some("inspector") => app.layout.focused_pane = PaneFocus::Context,
        Some("single") => app.layout.focused_pane = PaneFocus::List,
        Some("list") => app.layout.focused_pane = PaneFocus::List,
        None => {}
        Some(_) => app.layout.focused_pane = PaneFocus::List,
    }

    if options.empty {
        let view = app.current_view();
        let title = tcld_tui::views::title(app.specs(), view);
        app.cmd.output = Some(format!("No entries in {title}"));
    }

    match requested_overlay {
        Some(ExploreOverlay::Help) => {
            app.overlay.show_help = true;
        }
        Some(ExploreOverlay::Palette) => {
            app.palette.active = true;
            app.palette.query = palette_query.unwrap_or_default();
            app.palette.index = 0;
        }
        None => {}
    }

    tcld_tui::render_frame_to_text(&mut app, options.width, options.height)
        .map_err(|err| TcldError::Other(err.to_string()))
}

fn rdebug_migration_tombstone() -> CommandModule {
    CommandModule {
        id: "runtime-debugger-migration",
        description: "legacy runtime debugger migration",
        help_lines: &[],
        palette_actions: &[],
        parse: |command, rest| {
            if command != "rdebug" || !rest.trim().is_empty() {
                return None;
            }
            Some(ExploreCommand::Custom(
                "rdebug-moved".to_string(),
                String::new(),
            ))
        },
    }
}

fn non_interactive_terminal_message() -> String {
    "tcld explore requires an interactive terminal; use `tcld decompile`, `tcld disassemble`, or `tcld dump` for non-interactive batch output".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &str) -> String {
        format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn non_tty_explore_reports_batch_command_remediation() {
        let err = run_explorer(ExploreArgs {
            input_path: fixture("expr.tbc"),
            dump_screen: false,
            mode: None,
            overlay: None,
            palette_query: None,
            diff_target: None,
            page_size: 20,
            theme: None,
            no_colors: false,
        })
        .expect_err("cargo test is non-interactive and should not launch tui");

        let message = err.to_string();
        assert!(message.contains("interactive terminal"));
        assert!(message.contains("decompile"));
        assert!(message.contains("disassemble"));
        assert!(message.contains("dump"));
        assert!(!message.contains("repl"));
    }

    #[test]
    fn runtime_debug_tombstone_is_exact_and_unadvertised() {
        let module = rdebug_migration_tombstone();
        assert!(module.help_lines.is_empty());
        assert!((module.parse)("rdebug", "").is_some());
        assert!((module.parse)("rdebug", "on").is_none());
        assert!((module.parse)("rd", "").is_none());
        assert!((module.parse)("debug", "").is_none());
    }
}
