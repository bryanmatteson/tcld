use std::io::IsTerminal;
use tcld::batch_color::BatchColorPolicy;
use tcld::cli::{TcldCommand, parse_cli};
use tcld::error::Result;
use tcld::{DecompileOptions, DisassembleOptions, DumpOptions};
use tcld_core::util::truncate;
use tcld_decompile::{AnalysisSnapshot, LoadInput};

const DEFAULT_BATCH_TERMINAL_WIDTH: u16 = 80;

fn main() {
    if let Err(err) = run() {
        let message = err.to_string();
        eprintln!("{message}");
        // Backend availability is a distinct, stable CLI capability failure.
        // Keep it aligned with `tcld run` and the documented package contract.
        let exit_code = if message.starts_with("hybrid_unavailable:")
            || message.starts_with("ui_unavailable:")
        {
            3
        } else {
            1
        };
        std::process::exit(exit_code);
    }
}

fn run() -> Result<()> {
    let cli = parse_cli();
    let no_colors = cli.no_colors;
    match cli.command {
        TcldCommand::Decompile(parsed) => {
            let output = tcld::decompile_path_with_options(&parsed.input_path, DecompileOptions)?;
            print!(
                "{}",
                tcld::batch_color::render_decompile_output(
                    &output,
                    BatchColorPolicy::from_environment(no_colors, std::io::stdout().is_terminal()),
                )
            );
        }
        TcldCommand::Disassemble(parsed) => {
            let output = tcld::disassemble_path_with_options(
                &parsed.input_path,
                DisassembleOptions {
                    text_truncate: None,
                    line_width: Some(usize::from(current_output_width())),
                },
            )?;
            print!(
                "{}",
                tcld::batch_color::render_disassemble_output(
                    &output,
                    BatchColorPolicy::from_environment(no_colors, std::io::stdout().is_terminal()),
                )
            );
        }
        TcldCommand::Dump(parsed) => {
            let output = tcld::dump_path_with_options(
                &parsed.input_path,
                DumpOptions {
                    show_all_literals: parsed.show_all_literals,
                    literal_filter: parsed.literal_filter,
                },
            )?;
            print!("{}", output);
        }
        TcldCommand::Coverage(parsed) => match parsed.command {
            tcld::cli::CoverageCommand::Report => {
                let report =
                    tcld::generate_coverage_matrix_by_version(Default::default()).render_report();
                print!("{report}");
            }
        },
        TcldCommand::SourceMap(parsed) => {
            let snapshot = AnalysisSnapshot::from_input(LoadInput::path(&parsed.input_path))?;
            print!("{}", render_source_map(&snapshot));
        }
        TcldCommand::Explore(parsed) => {
            tcld::explore::run_explorer(parsed)?;
        }
        TcldCommand::Debug(parsed) => {
            tcld::debug::run_debug(parsed)?;
        }
        TcldCommand::Run(parsed) => {
            tcld::run::run_script(parsed)?;
        }
        TcldCommand::Ui(parsed) => {
            tcld::ui::run_ui(parsed)?;
        }
    }
    Ok(())
}

fn current_output_width() -> u16 {
    let env_width = || {
        std::env::var("COLUMNS")
            .ok()
            .and_then(|raw| raw.parse::<u16>().ok())
            .filter(|width| *width > 0)
    };

    if std::io::stdout().is_terminal()
        && let Ok((width, _)) = crossterm::terminal::size()
        && width > 0
    {
        return width;
    }

    env_width().unwrap_or(DEFAULT_BATCH_TERMINAL_WIDTH)
}

fn render_source_map(snapshot: &AnalysisSnapshot) -> String {
    let mut out = String::new();
    for (index, entry) in snapshot.source_map.entries().iter().enumerate() {
        let kind = if entry.parent_command_group.is_some() {
            "nested"
        } else {
            "root"
        };
        let path = display_command_path(&entry.command_path.components);
        let parent = entry
            .parent_command_group
            .map(|group| format!("c{group:03}"))
            .unwrap_or_else(|| "-".to_string());
        let root = entry
            .command_path
            .components
            .first()
            .map(|group| format!("c{group:03}"))
            .unwrap_or_else(|| "-".to_string());
        let group = entry
            .command_group()
            .map(|group| format!("c{group:03}"))
            .unwrap_or_else(|| "-".to_string());
        let original = match &entry.original_span {
            Some(span) => format!("available:{}..{}", span.start, span.end),
            None => "unavailable".to_string(),
        };
        let snippet = entry
            .text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("");
        let snippet = truncate(snippet, 96, "...");
        out.push_str(&format!(
            "idx={index:04} kind={kind} path={path} parent={parent} root={root} group={group} bytecode={}..{} source_lines={}..{} original={original} snippet={snippet}\n",
            entry.bytecode_range.start,
            entry.bytecode_range.end,
            entry.source_lines.start,
            entry.source_lines.end,
        ));
    }
    out
}

fn display_command_path(components: &[usize]) -> String {
    let Some((root, rest)) = components.split_first() else {
        return "-".to_string();
    };
    let mut out = format!("c{root:03}");
    for nested in rest {
        out.push_str(&format!("/n{nested:03}"));
    }
    out
}
