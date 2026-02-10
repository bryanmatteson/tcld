pub mod module_core;
pub mod module_doc;
pub mod module_trace;

use crate::views::{self, ViewSpec};

/// Trace mode sub-commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceCommand {
    Toggle,
    On,
    Off,
    Next,
    Prev,
    Goto(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeCommand {
    List,
    Next,
    Set(String),
}

/// A command parsed from user input in the TUI command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExploreCommand {
    Help,
    Summary,
    Views,
    View(String),
    Mode(String),
    Next {
        count: Option<usize>,
    },
    Prev {
        count: Option<usize>,
    },
    Goto {
        value: i32,
    },
    Show {
        index: Option<usize>,
    },
    Search {
        text: String,
    },
    ShowDecompile,
    Trace(TraceCommand),
    Map {
        enabled: Option<bool>,
    },
    Diff {
        path: String,
    },
    DiffClose,
    Export {
        path: String,
    },
    Theme(ThemeCommand),
    Quit,
    Noop,
    /// Extension point for consumer-defined commands.
    Custom(String, String),
    Unknown(String),
}

/// A pluggable command parsing module.
#[derive(Clone, Copy)]
pub struct CommandModule {
    pub id: &'static str,
    pub description: &'static str,
    pub help_lines: &'static [&'static str],
    pub palette_actions: &'static [PaletteActionSpec],
    pub parse: fn(command: &str, rest: &str) -> Option<ExploreCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteActionKind {
    Execute(&'static str),
    Prefill(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteActionSpec {
    pub label: &'static str,
    pub action: PaletteActionKind,
}

pub type CustomCommandHandler = Box<dyn FnMut(&str, &str) -> Option<String>>;

const BUILTIN_MODULES: &[CommandModule] = &[
    module_core::MODULE,
    module_doc::MODULE,
    module_trace::MODULE,
];

/// Parse a command with additional consumer-provided modules and view specs.
pub fn parse_command_with(
    line: &str,
    extra_modules: &[CommandModule],
    specs: &[ViewSpec],
) -> ExploreCommand {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ExploreCommand::Noop;
    }

    let (command, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((cmd, remainder)) => (cmd.to_ascii_lowercase(), remainder.trim()),
        None => (trimmed.to_ascii_lowercase(), ""),
    };

    for module in BUILTIN_MODULES.iter().chain(extra_modules.iter()) {
        if let Some(parsed) = (module.parse)(command.as_str(), rest) {
            return parsed;
        }
    }

    // Bare view alias (e.g. "inst" → View("instructions"))
    if rest.is_empty()
        && let Some(view) = views::parse_alias(specs, command.as_str())
    {
        return ExploreCommand::View(views::token(specs, view).to_string());
    }

    ExploreCommand::Unknown(format!(
        "unknown command: {command} (type help for commands)"
    ))
}

/// All built-in command modules.
pub fn command_modules() -> &'static [CommandModule] {
    BUILTIN_MODULES
}

pub(crate) fn help_line_priority(line: &str) -> usize {
    match line {
        _ if line.starts_with("search ") => 0,
        _ if line.starts_with("view ") => 1,
        _ if line.starts_with("mode ") => 2,
        _ => 20,
    }
}

// ── Shared argument parsers ─────────────────────────────────────

pub fn parse_optional_positive_usize(raw: &str, cmd: &str) -> Result<Option<usize>, String> {
    if raw.is_empty() {
        return Ok(None);
    }
    let value = parse_positive_usize(raw, cmd)?;
    Ok(Some(value))
}

pub fn parse_required_usize(raw: &str, cmd: &str) -> Result<usize, String> {
    if raw.is_empty() {
        return Err(format!("{cmd} requires a numeric argument"));
    }
    raw.parse::<usize>()
        .map_err(|_| format!("{cmd} expects an integer, got '{raw}'"))
}

pub fn parse_required_i32(raw: &str, cmd: &str) -> Result<i32, String> {
    if raw.is_empty() {
        return Err(format!("{cmd} requires an integer"));
    }
    raw.parse::<i32>()
        .map_err(|_| format!("{cmd} expects an integer, got '{raw}'"))
}

pub fn parse_positive_usize(raw: &str, cmd: &str) -> Result<usize, String> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("{cmd} expects an integer, got '{raw}'"))?;
    if value == 0 {
        return Err(format!("{cmd} expects a positive integer, got '{raw}'"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_command(line: &str) -> ExploreCommand {
        parse_command_with(line, &[], &[])
    }

    #[test]
    fn parse_basic_commands() {
        assert_eq!(parse_command("help"), ExploreCommand::Help);
        assert_eq!(parse_command("quit"), ExploreCommand::Quit);
        assert!(matches!(parse_command(""), ExploreCommand::Noop));
    }

    #[test]
    fn parse_unknown_command() {
        assert!(matches!(
            parse_command("foobar"),
            ExploreCommand::Unknown(_)
        ));
    }

    #[test]
    fn parse_diff_requires_path() {
        assert!(matches!(parse_command("diff"), ExploreCommand::Unknown(_)));
    }

    #[test]
    fn parse_diff_close_variants() {
        assert_eq!(parse_command("diff close"), ExploreCommand::DiffClose);
        assert_eq!(parse_command("diff off"), ExploreCommand::DiffClose);
    }

    #[test]
    fn parse_trace_commands() {
        assert_eq!(
            parse_command("trace"),
            ExploreCommand::Trace(TraceCommand::Toggle)
        );
        assert_eq!(
            parse_command("trace on"),
            ExploreCommand::Trace(TraceCommand::On)
        );
        assert_eq!(
            parse_command("trace off"),
            ExploreCommand::Trace(TraceCommand::Off)
        );
        assert_eq!(
            parse_command("trace next"),
            ExploreCommand::Trace(TraceCommand::Next)
        );
        assert_eq!(
            parse_command("trace prev"),
            ExploreCommand::Trace(TraceCommand::Prev)
        );
        assert_eq!(
            parse_command("trace 42"),
            ExploreCommand::Trace(TraceCommand::Goto(42))
        );
        assert_eq!(
            parse_command("tr on"),
            ExploreCommand::Trace(TraceCommand::On)
        );
    }

    #[test]
    fn parse_doc_commands() {
        assert_eq!(
            parse_command("decompile"),
            ExploreCommand::View("decompile".to_string())
        );
        assert_eq!(
            parse_command("dec"),
            ExploreCommand::View("decompile".to_string())
        );
        assert!(matches!(parse_command("dump"), ExploreCommand::Unknown(_)));
        assert!(matches!(parse_command("graph"), ExploreCommand::Unknown(_)));
        assert!(matches!(parse_command("cfg"), ExploreCommand::Unknown(_)));
        assert_eq!(parse_command("showdecomp"), ExploreCommand::ShowDecompile);
        assert_eq!(parse_command("sd"), ExploreCommand::ShowDecompile);
    }

    #[test]
    fn parse_map_command() {
        assert_eq!(parse_command("map"), ExploreCommand::Map { enabled: None });
        assert_eq!(
            parse_command("map on"),
            ExploreCommand::Map {
                enabled: Some(true)
            }
        );
        assert_eq!(
            parse_command("map off"),
            ExploreCommand::Map {
                enabled: Some(false)
            }
        );
        assert!(matches!(parse_command("multi"), ExploreCommand::Unknown(_)));
    }

    #[test]
    fn parse_export_and_theme_commands() {
        assert_eq!(
            parse_command("export /tmp/out.tcl"),
            ExploreCommand::Export {
                path: "/tmp/out.tcl".to_string()
            }
        );
        assert_eq!(
            parse_command("theme list"),
            ExploreCommand::Theme(ThemeCommand::List)
        );
        assert_eq!(
            parse_command("theme next"),
            ExploreCommand::Theme(ThemeCommand::Next)
        );
        assert_eq!(
            parse_command("theme sunset"),
            ExploreCommand::Theme(ThemeCommand::Set("sunset".to_string()))
        );
    }

    #[test]
    fn parse_custom_via_extra_module() {
        let module = CommandModule {
            id: "test",
            description: "test module",
            help_lines: &["test <args>  do a test"],
            palette_actions: &[],
            parse: |cmd, rest| {
                if cmd == "testcmd" {
                    Some(ExploreCommand::Custom(
                        "testcmd".to_string(),
                        rest.to_string(),
                    ))
                } else {
                    None
                }
            },
        };
        let result = parse_command_with("testcmd hello world", &[module], &[]);
        assert_eq!(
            result,
            ExploreCommand::Custom("testcmd".to_string(), "hello world".to_string())
        );
    }
}
