use crate::error::{Result, TcldError};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use std::ffi::OsString;

#[derive(Parser, Debug)]
#[command(
    name = "tcld",
    version,
    about = "Tcl bytecode decompiler, explorer, and debugger",
    long_about = "Tcl bytecode decompiler, explorer, and debugger.\n\n\
Build with `--features hybrid` to run bytecode or compile admitted Tcl source \
into the in-process debugger. Host debugging of Tcl source does not require \
the hybrid feature."
)]
pub struct Cli {
    /// Disable color
    #[arg(long, global = true)]
    pub no_colors: bool,

    #[command(subcommand)]
    pub command: TcldCommand,
}

impl Cli {
    pub fn resolve_global_options(mut self) -> Self {
        if let TcldCommand::Explore(args) = &mut self.command
            && self.no_colors
        {
            args.no_colors = true;
        }
        self
    }
}

#[derive(Subcommand, Debug)]
pub enum TcldCommand {
    /// Reconstruct Tcl source from bytecode
    Decompile(DecompileArgs),
    /// Print bytecode instructions sized to the terminal width
    Disassemble(DisassembleArgs),
    /// Dump exceptions, literals, and instructions
    Dump(DumpArgs),
    /// Report opcode and Tcl-version coverage
    Coverage(CoverageArgs),
    /// Print source-map command paths
    SourceMap(SourceMapArgs),
    /// Open the terminal explorer
    Explore(ExploreArgs),
    /// Debug Tcl source or bytecode
    ///
    /// Tcl source defaults to a host interpreter with the loadable debugger
    /// nub over the wire protocol. Explicit `--backend=hybrid` or
    /// `--backend=vm-strict` compiles admitted source into a deterministic
    /// in-process debug image.
    ///
    /// Bytecode uses the in-process TEBC VM when the binary includes the
    /// `hybrid` feature. The VM preserves bytecode program counters and supports
    /// instruction stepping, breakpoints, frame inspection, coverage, and
    /// synchronized source and disassembly.
    ///
    /// `--backend=host` reconstructs bytecode into temporary Tcl source and
    /// starts the host debugger. This path does not preserve bytecode program
    /// counters.
    Debug(DebugArgs),
    /// Run Tcl source or bytecode
    ///
    /// Tcl source runs in a host interpreter. Bytecode requires a binary built
    /// with `--features hybrid`.
    ///
    /// `--backend=vm-strict` and `--sandbox` deny WorldHost calls. A binary
    /// without hybrid support returns `hybrid_unavailable` with exit code 3
    /// when a VM backend is requested. Hybrid execution fails closed with an
    /// `UnimplementedOpcode` trap when an instruction is outside the VM's
    /// supported pass scope.
    Run(RunArgs),
    /// Open the native desktop workbench
    ///
    /// Hosts the Tauri desktop window in this process, optionally opening FILE
    /// on startup. The command returns when the last window closes.
    ///
    /// Requires a binary built with `--features native-ui` after building the
    /// frontend (`npm run build --prefix apps/tcld-gui`). Other builds return
    /// `ui_unavailable` with exit code 3.
    Ui(UiArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct UiArgs {
    /// Tcl source or bytecode file to open when the window starts
    #[arg(value_name = "FILE")]
    pub input_path: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct RunArgs {
    /// Tcl source or bytecode path
    pub input_path: String,

    /// Runtime backend
    #[arg(long, value_enum, default_value_t = RunBackend::Auto)]
    pub backend: RunBackend,

    /// Deny WorldHost calls; equivalent to `--backend=vm-strict`
    #[arg(long, conflicts_with = "backend")]
    pub sandbox: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunBackend {
    /// Select the backend from the input type
    Auto,
    /// Use the in-process TEBC VM
    Hybrid,
    /// Use the TEBC VM with WorldHost denied
    VmStrict,
    /// Use a host Tcl interpreter
    Host,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct DecompileArgs {
    /// Path to the input bytecode file
    pub input_path: String,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct DisassembleArgs {
    /// Path to the input bytecode file
    pub input_path: String,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct DumpArgs {
    /// Path to the input bytecode file
    pub input_path: String,

    /// Show all literals
    #[arg(long, env = "TCLD_DUMP_LITERALS")]
    pub show_all_literals: bool,

    /// Filter literals by text
    #[arg(long)]
    pub literal_filter: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct CoverageArgs {
    #[command(subcommand)]
    pub command: CoverageCommand,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct SourceMapArgs {
    /// Path to the input bytecode file
    pub input_path: String,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum CoverageCommand {
    /// Print the opcode/Tcl-version coverage report
    Report,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ExploreArgs {
    /// Path to Tcl bytecode or a Tcl source file
    pub input_path: String,

    /// Render one deterministic explorer frame and exit
    #[arg(long)]
    pub dump_screen: bool,

    /// Initial mode for deterministic rendering
    #[arg(long, value_enum)]
    pub mode: Option<ExploreMode>,

    /// Overlay for deterministic rendering
    #[arg(long, value_enum)]
    pub overlay: Option<ExploreOverlay>,

    /// Palette query to prefill when rendering the palette overlay
    #[arg(long)]
    pub palette_query: Option<String>,

    /// Comparison target for deterministic rendering
    #[arg(long)]
    pub diff_target: Option<String>,

    /// Number of rows to display per page
    #[arg(long, default_value = "20", value_parser = parse_page_size_arg)]
    pub page_size: usize,

    /// Terminal color theme
    #[arg(long)]
    pub theme: Option<String>,

    /// Disable color
    #[arg(skip)]
    pub no_colors: bool,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(
    after_help = "Backend selection:\n  auto       Tcl source uses the host debugger; bytecode uses hybrid when linked\n  host       Debug source directly in tclsh; bytecode requires exact reconstruction\n  hybrid     Compile admitted Tcl source or load bytecode into the in-process VM\n  vm-strict  Like hybrid, with WorldHost calls denied\n\nSource compilation is admitted by the verified V1 compiler before terminal entry. A rejected source reports source_compile_failed and suggests --backend=host; it is never silently weakened. See https://github.com/bryanmatteson/tcld/blob/main/DEBUGGING.md"
)]
pub struct DebugArgs {
    /// Tcl source or bytecode path
    pub input_path: String,

    /// Host Tcl interpreter; overrides `TCLD_TCLSH` and discovery
    #[arg(long, env = "TCLD_TCLSH")]
    pub tclsh: Option<String>,

    /// Debugger nub; overrides `TCLD_DBG_DYLIB` and target lookup
    #[arg(long, env = "TCLD_DBG_DYLIB")]
    pub dylib: Option<String>,

    /// Host debugger bootstrap script
    #[arg(long)]
    pub bootstrap: Option<String>,

    /// Terminal color theme
    #[arg(long)]
    pub theme: Option<String>,

    /// Disable color
    #[arg(long)]
    pub no_colors: bool,

    /// Debugger backend. Hybrid/vm-strict source compilation requires a hybrid build
    #[arg(long, value_enum, default_value = "auto")]
    pub backend: Option<RunBackend>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExploreMode {
    Browse,
    Compare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExploreOverlay {
    Help,
    Palette,
}

impl ExploreMode {
    pub fn token(self) -> &'static str {
        match self {
            Self::Browse => "browse",
            Self::Compare => "compare",
        }
    }
}

/// Parse page size argument: must be a positive integer (not 0)
fn parse_page_size_arg(s: &str) -> std::result::Result<usize, String> {
    let value: usize = s
        .parse()
        .map_err(|_| format!("invalid --page-size value: {s} (expected a positive integer)"))?;
    if value == 0 {
        Err("invalid --page-size value: 0 (expected a positive integer)".to_string())
    } else {
        Ok(value)
    }
}

/// Parse the CLI arguments and return the Cli struct.
pub fn parse_cli() -> Cli {
    Cli::parse().resolve_global_options()
}

/// Parse the CLI arguments from a list (for tests and compatibility wrappers).
pub fn parse_cli_from<I, T>(args: I) -> Result<Cli>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args)
        .map(Cli::resolve_global_options)
        .map_err(|e| TcldError::Other(e.to_string()))
}

/// Parse dump arguments from a list of strings (for standalone invocation).
/// Expects args[0] to be the program name, args[1] to be the input path.
pub fn parse_dump_args(args: &[String]) -> Result<DumpArgs> {
    DumpArgs::try_parse_from(args).map_err(|e| TcldError::Other(e.to_string()))
}

/// Parse decompile arguments from a list of strings (for standalone invocation).
/// Expects args[0] to be the program name, args[1] to be the input path.
pub fn parse_decompile_args(args: &[String]) -> Result<DecompileArgs> {
    DecompileArgs::try_parse_from(args).map_err(|e| TcldError::Other(e.to_string()))
}

/// Parse tcld arguments from a list of strings (for testing).
/// This is a compatibility wrapper that uses clap's try_parse_from.
pub fn parse_tcld_args(args: &[String]) -> Result<TcldCommand> {
    match Cli::try_parse_from(args).map(Cli::resolve_global_options) {
        Ok(cli) => Ok(cli.command),
        Err(e) => Err(TcldError::Other(e.to_string())),
    }
}

/// Generate help text from the same clap command graph used by the executable.
pub fn help_text(program: &str, topic: Option<&str>) -> String {
    let mut command = Cli::command().bin_name(program.to_string());
    if let Some(topic) = topic
        && let Some(subcommand) = command.find_subcommand_mut(topic)
    {
        return subcommand
            .clone()
            .bin_name(format!("{program} {topic}"))
            .render_long_help()
            .to_string();
    }
    command.render_long_help().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn parse_tcld_args_parses_decompile() {
        let args = vec![
            "tcld".to_string(),
            "decompile".to_string(),
            "in.tbc".to_string(),
        ];
        let parsed = parse_tcld_args(&args).expect("valid args");
        match parsed {
            TcldCommand::Decompile(args) => {
                assert_eq!(args.input_path, "in.tbc");
            }
            _ => panic!("expected Decompile command"),
        }
    }

    #[test]
    fn global_no_colors_is_available_to_decompile_disassemble_and_explore() {
        let decompile = parse_cli_from(["tcld", "--no-colors", "decompile", "in.tbc"])
            .expect("global --no-colors should parse before decompile");
        assert!(decompile.no_colors);
        assert!(matches!(decompile.command, TcldCommand::Decompile(_)));

        let disassemble = parse_cli_from(["tcld", "--no-colors", "disassemble", "in.tbc"])
            .expect("global --no-colors should parse before disassemble");
        assert!(disassemble.no_colors);
        assert!(matches!(disassemble.command, TcldCommand::Disassemble(_)));

        let explore_global = parse_cli_from(["tcld", "--no-colors", "explore", "in.tbc"])
            .expect("global --no-colors should parse before explore");
        assert!(explore_global.no_colors);
        match explore_global.command {
            TcldCommand::Explore(args) => assert!(args.no_colors),
            _ => panic!("expected explore command"),
        }

        let explore_local = parse_cli_from(["tcld", "explore", "in.tbc", "--no-colors"])
            .expect("existing explore --no-colors should keep parsing");
        assert!(explore_local.no_colors);
        match explore_local.command {
            TcldCommand::Explore(args) => assert!(args.no_colors),
            _ => panic!("expected explore command"),
        }

        let help = Cli::command().render_help().to_string();
        assert!(help.contains("--no-colors"));
        assert!(help.contains("Disable color"));
    }

    #[test]
    fn readme_documents_batch_color_controls() {
        let readme = include_str!("../../../README.md");
        for expected in [
            "terminal",
            "ANSI",
            "--no-colors",
            "NO_COLOR=1",
            "decompile",
            "disassemble",
        ] {
            assert!(
                readme.contains(expected),
                "README should document batch color control token {expected:?}"
            );
        }
    }

    #[test]
    fn parse_tcld_args_rejects_extractor_flag() {
        let args = vec![
            "tcld".to_string(),
            "decompile".to_string(),
            "in.tbc".to_string(),
            "--extractor".to_string(),
            "nom".to_string(),
        ];
        let err = parse_tcld_args(&args).expect_err("must fail");
        assert!(
            err.to_string().contains("unexpected argument") || err.to_string().contains("unknown")
        );
    }

    #[test]
    fn parse_tcld_args_parses_disassemble_defaults() {
        let args = vec![
            "tcld".to_string(),
            "disassemble".to_string(),
            "in.tbc".to_string(),
        ];
        let parsed = parse_tcld_args(&args).expect("valid args");
        match parsed {
            TcldCommand::Disassemble(args) => {
                assert_eq!(args.input_path, "in.tbc");
            }
            _ => panic!("expected Disassemble command"),
        }
    }

    #[test]
    fn parse_tcld_args_rejects_disassemble_text_truncate_flag() {
        let args = vec![
            "tcld".to_string(),
            "disassemble".to_string(),
            "in.tbc".to_string(),
            "--text-truncate".to_string(),
            "0".to_string(),
        ];
        let err = parse_tcld_args(&args).expect_err("removed flag should fail");
        assert!(err.to_string().contains("unexpected") || err.to_string().contains("unknown"));
    }

    #[test]
    fn parse_explore_rejects_removed_debug_mode() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "--dump-screen".to_string(),
            "--mode".to_string(),
            "debug".to_string(),
            "in.tbc".to_string(),
        ];
        let err = parse_tcld_args(&args).expect_err("debug is not an explore mode");
        assert!(err.to_string().contains("invalid value"));
    }

    #[test]
    fn disassemble_help_no_longer_advertises_text_truncate() {
        let help = help_text("tcld", Some("disassemble"));
        assert!(help.contains("terminal width"));
        assert!(!help.contains("text-truncate"));
    }

    #[test]
    fn parse_tcld_explore_rejects_text_truncate_flag() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "in.tbc".to_string(),
            "--text-truncate".to_string(),
            "0".to_string(),
        ];
        let err = parse_tcld_args(&args).expect_err("removed flag should fail");
        assert!(err.to_string().contains("unexpected") || err.to_string().contains("unknown"));
    }

    #[test]
    fn parse_tcld_args_parses_dump_flags() {
        let args = vec![
            "tcld".to_string(),
            "dump".to_string(),
            "input.tbc".to_string(),
            "--literal-filter".to_string(),
            "abc".to_string(),
            "--show-all-literals".to_string(),
        ];
        let parsed = parse_tcld_args(&args).expect("valid args");
        match parsed {
            TcldCommand::Dump(args) => {
                assert_eq!(args.input_path, "input.tbc");
                assert!(args.show_all_literals);
                assert_eq!(args.literal_filter, Some("abc".to_string()));
            }
            _ => panic!("expected Dump command"),
        }
    }

    #[test]
    fn standalone_parsers_consume_program_name_once() {
        let dump = parse_dump_args(&[
            "dump".to_string(),
            "input.tbc".to_string(),
            "--show-all-literals".to_string(),
        ])
        .expect("standalone dump args parse");
        assert_eq!(dump.input_path, "input.tbc");
        assert!(dump.show_all_literals);

        let decompile = parse_decompile_args(&["decompile".to_string(), "input.tbc".to_string()])
            .expect("standalone decompile args parse");
        assert_eq!(decompile.input_path, "input.tbc");
    }

    #[test]
    fn sandbox_rejects_an_explicit_backend_instead_of_overriding_it() {
        let err = parse_cli_from(["tcld", "run", "--backend=host", "--sandbox", "input.tbc"])
            .expect_err("conflicting backend selectors must fail");
        assert!(err.to_string().contains("cannot be used with"));
    }

    #[test]
    fn clap_help_exposes_version_and_current_command_graph() {
        let top = help_text("tcld", None);
        assert!(top.contains("--version"));
        assert!(top.contains("run"));

        let explore = help_text("tcld", Some("explore"));
        assert!(!explore.contains("debug]"));
        assert!(!explore.contains("debug|compare"));

        let dump = help_text("tcld", Some("dump"));
        assert!(dump.contains("--show-all-literals"));
        assert!(!dump.contains("--all-literals"));
    }

    #[test]
    fn parse_tcld_explore_defaults() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "apps/tcld/tests/fixtures/expr.tbc".to_string(),
        ];
        let parsed = parse_tcld_args(&args).expect("explore args parse");
        match parsed {
            TcldCommand::Explore(args) => {
                assert_eq!(args.input_path, "apps/tcld/tests/fixtures/expr.tbc");
                assert!(!args.dump_screen);
                assert_eq!(args.page_size, 20);
                assert_eq!(args.theme, None);
                assert!(!args.no_colors);
            }
            _ => panic!("expected Explore command"),
        }
    }

    #[test]
    fn parse_tcld_explore_custom_flags() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "apps/tcld/tests/fixtures/expr.tbc".to_string(),
            "--page-size".to_string(),
            "40".to_string(),
            "--theme".to_string(),
            "sunset".to_string(),
            "--no-colors".to_string(),
        ];
        let parsed = parse_tcld_args(&args).expect("explore args parse");
        match parsed {
            TcldCommand::Explore(args) => {
                assert_eq!(args.input_path, "apps/tcld/tests/fixtures/expr.tbc");
                assert!(!args.dump_screen);
                assert_eq!(args.page_size, 40);
                assert_eq!(args.theme, Some("sunset".to_string()));
                assert_eq!(args.overlay, None);
                assert_eq!(args.palette_query, None);
                assert_eq!(args.diff_target, None);
                assert!(args.no_colors);
            }
            _ => panic!("expected Explore command"),
        }
    }

    #[test]
    fn parse_tcld_explore_rejects_invalid_page_size() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "apps/tcld/tests/fixtures/expr.tbc".to_string(),
            "--page-size".to_string(),
            "0".to_string(),
        ];
        let err = parse_tcld_args(&args).expect_err("invalid page size should error");
        assert!(err.to_string().contains("invalid") || err.to_string().contains("invalid"));
    }

    #[test]
    fn parse_tcld_explore_rejects_removed_ui_mode_and_help_is_unified() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "apps/tcld/tests/fixtures/expr.tbc".to_string(),
            format!("--ui-{}", "mode"),
            "repl".to_string(),
        ];
        let err = parse_tcld_args(&args).expect_err("removed ui mode should error");
        assert!(err.to_string().contains("unexpected") || err.to_string().contains("unknown"));

        let help = help_text("tcld", Some("explore"));
        assert!(help.contains("--theme"));
        assert!(help.contains("--dump-screen"));
        assert!(help.contains("--overlay"));
        assert!(help.contains("--diff-target"));
        assert!(!help.contains("text-truncate"));
        assert!(!help.contains("repl"));
        assert!(!help.contains("tui|repl"));
    }

    #[test]
    fn parse_tcld_explore_supports_dump_screen() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "--dump-screen".to_string(),
            "apps/tcld/tests/fixtures/expr.tbc".to_string(),
        ];
        let parsed = parse_tcld_args(&args).expect("explore args parse");
        match parsed {
            TcldCommand::Explore(args) => {
                assert!(args.dump_screen);
                assert_eq!(args.input_path, "apps/tcld/tests/fixtures/expr.tbc");
            }
            _ => panic!("expected Explore command"),
        }
    }

    #[test]
    fn parse_tcld_args_parses_debug_defaults() {
        let parsed = parse_tcld_args(&[
            "tcld".to_string(),
            "debug".to_string(),
            "fixtures/demo.tcl".to_string(),
        ])
        .expect("debug args parse");
        match parsed {
            TcldCommand::Debug(args) => {
                assert_eq!(args.input_path, "fixtures/demo.tcl");
                assert_eq!(args.tclsh, std::env::var("TCLD_TCLSH").ok());
                assert_eq!(args.dylib, std::env::var("TCLD_DBG_DYLIB").ok());
                assert!(args.bootstrap.is_none());
                assert!(args.theme.is_none());
                assert!(!args.no_colors);
            }
            _ => panic!("expected Debug command"),
        }
    }

    #[test]
    fn parse_tcld_args_parses_debug_overrides() {
        let parsed = parse_tcld_args(&[
            "tcld".to_string(),
            "debug".to_string(),
            "demo.tcl".to_string(),
            "--tclsh".to_string(),
            "/usr/bin/tclsh".to_string(),
            "--dylib".to_string(),
            "/lib/libtcld_dbg.so".to_string(),
            "--bootstrap".to_string(),
            "/scripts/bootstrap.tcl".to_string(),
            "--theme".to_string(),
            "sunset".to_string(),
            "--no-colors".to_string(),
        ])
        .expect("debug overrides parse");
        match parsed {
            TcldCommand::Debug(args) => {
                assert_eq!(args.tclsh.as_deref(), Some("/usr/bin/tclsh"));
                assert_eq!(args.dylib.as_deref(), Some("/lib/libtcld_dbg.so"));
                assert_eq!(args.bootstrap.as_deref(), Some("/scripts/bootstrap.tcl"));
                assert_eq!(args.theme.as_deref(), Some("sunset"));
                assert!(args.no_colors);
            }
            _ => panic!("expected Debug command"),
        }
    }

    #[test]
    fn debug_help_documents_subcommand_and_appears_in_top_level_help() {
        let topic = help_text("tcld", Some("debug"));
        assert!(topic.contains("Usage: tcld debug"));
        assert!(topic.contains("--tclsh"));
        assert!(topic.contains("--dylib"));
        assert!(topic.contains("wire protocol"));
        assert!(topic.contains("source_compile_failed"));
        assert!(topic.contains("--backend=host"));
        assert!(topic.contains("[default: auto]"));
        assert!(topic.contains("DEBUGGING.md"));

        let unified = help_text("tcld", None);
        assert!(unified.contains("debug"));
        assert!(unified.contains("Debug Tcl source or bytecode"));
    }

    #[test]
    fn parse_tcld_ui_accepts_an_optional_file() {
        let bare = parse_cli_from(["tcld", "ui"]).expect("bare ui parses");
        match bare.command {
            TcldCommand::Ui(args) => assert_eq!(args, UiArgs { input_path: None }),
            _ => panic!("expected Ui command"),
        }

        let with_file = parse_cli_from(["tcld", "ui", "demo.tbc"]).expect("ui with a file parses");
        match with_file.command {
            TcldCommand::Ui(args) => assert_eq!(args.input_path.as_deref(), Some("demo.tbc")),
            _ => panic!("expected Ui command"),
        }

        let err = parse_cli_from(["tcld", "ui", "a.tbc", "b.tbc"])
            .expect_err("ui opens at most one file");
        assert!(err.to_string().contains("unexpected argument"));
    }

    #[test]
    fn ui_help_documents_subcommand_and_appears_in_top_level_help() {
        let topic = help_text("tcld", Some("ui"));
        assert!(topic.contains("Usage: tcld ui [FILE]"));
        assert!(topic.contains("--features native-ui"));
        assert!(topic.contains("npm run build --prefix apps/tcld-gui"));
        assert!(topic.contains("ui_unavailable"));

        let unified = help_text("tcld", None);
        assert!(unified.contains("Open the native desktop workbench"));
    }

    #[test]
    fn parse_tcld_explore_supports_dump_overlay_and_diff_target() {
        let args = vec![
            "tcld".to_string(),
            "explore".to_string(),
            "--dump-screen".to_string(),
            "--mode".to_string(),
            "compare".to_string(),
            "--overlay".to_string(),
            "palette".to_string(),
            "--palette-query".to_string(),
            "debug".to_string(),
            "--diff-target".to_string(),
            "other.tbc".to_string(),
            "apps/tcld/tests/fixtures/expr.tbc".to_string(),
        ];
        let parsed = parse_tcld_args(&args).expect("explore args parse");
        match parsed {
            TcldCommand::Explore(args) => {
                assert_eq!(args.mode, Some(ExploreMode::Compare));
                assert_eq!(args.overlay, Some(ExploreOverlay::Palette));
                assert_eq!(args.palette_query.as_deref(), Some("debug"));
                assert_eq!(args.diff_target.as_deref(), Some("other.tbc"));
            }
            _ => panic!("expected Explore command"),
        }
    }
}
