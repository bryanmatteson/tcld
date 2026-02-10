pub mod app;
pub mod bindings;
pub mod chrome;
pub mod command;
pub mod data;
pub mod debug;
pub mod helpers;
pub mod input;
pub mod render;
pub mod session;
pub mod state;
pub mod style;
pub mod terminal;
pub mod theme;
pub mod tokens;

/// Re-exported so hosts can name semantic tokens without depending on Termosaic directly.
pub use termosaic::TokenId;
pub mod views;

pub use app::TuiApp;
pub use command::{CommandModule, CustomCommandHandler, ExploreCommand, TraceCommand};
pub use data::{DataSource, ListEntry, ViewId};
pub use state::{HitRegion, ModeId, ModeLayoutProfile, ModeSpec, PaneFocus, RailMode};
pub use theme::Theme;
pub use views::{ViewOpenBehavior, ViewSpec};

/// Re-exported ratatui types needed for DataSource rendering methods.
pub mod ui {
    pub use ratatui::style::Style;
    pub use ratatui::text::{Line, Span, Text};
}

/// Configuration for starting the TUI.
#[derive(Default)]
pub struct TuiConfig {
    /// Whether to start with the command input active.
    pub start_in_command_mode: bool,
    /// Optional initial color palette.
    pub theme: Option<String>,
    /// Optional initial mode token.
    pub initial_mode: Option<String>,
    /// Additional command modules registered by the consumer.
    pub extra_modules: Vec<CommandModule>,
    /// Callback for handling `Custom` commands.
    /// Receives `(command_name, args)`, returns an optional status message.
    pub custom_handler: Option<CustomCommandHandler>,
}

/// Entry point: set up terminal, create app, run event loop, tear down.
pub fn run<D: DataSource + 'static>(
    data: D,
    config: TuiConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = TuiApp::new_with_terminal_theme(
        data,
        config.start_in_command_mode,
        config.theme.as_deref(),
    )?;
    app.extra_modules = config.extra_modules;
    app.custom_handler = config.custom_handler;
    if let Some(mode) = config.initial_mode {
        app.switch_mode_token(&mode);
    }
    app::run_loop(app)
}

/// Render one deterministic frame using Ratatui's test backend.
pub fn render_frame_to_text<D: DataSource>(
    app: &mut TuiApp<D>,
    width: u16,
    height: u16,
) -> Result<String, Box<dyn std::error::Error>> {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend)?;
    terminal.draw(|frame| render::draw(frame, app))?;
    Ok(buffer_to_text(terminal.backend().buffer(), width, height))
}

fn buffer_to_text(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
    let width = width as usize;
    let height = height as usize;
    let mut out = String::new();
    for row in 0..height {
        let start = row * width;
        let end = start + width;
        let line = buffer.content()[start..end]
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .trim_end()
            .to_string();
        out.push_str(&line);
        out.push('\n');
    }
    out
}
