use super::{
    CommandModule, ExploreCommand, PaletteActionKind, PaletteActionSpec, ThemeCommand,
    parse_optional_positive_usize, parse_required_i32, parse_required_usize,
};

pub const MODULE: CommandModule = CommandModule {
    id: "core",
    description: "navigation, view switching, diff, and session control",
    help_lines: &[
        "help                      Show help",
        "summary                   Show summary",
        "views                     List views",
        "view <name>               Open a view",
        "mode <browse|compare>       Switch app mode",
        "next [count]              Move current view forward",
        "prev [count]              Move current view backward",
        "goto <pc|line>            Jump by pc/line",
        "show [row]                Open inspector at a one-based row",
        "search <text>             Search current view",
        "map [on|off]              Toggle Decompile map",
        "diff <path>               Semantic diff vs another file",
        "diff close                Close diff",
        "export <path>             Export generated decompiled source",
        "theme <name|list|next>    Choose color palette",
        "quit                      Exit",
    ],
    palette_actions: &[
        PaletteActionSpec {
            label: "Show help",
            action: PaletteActionKind::Execute("help"),
        },
        PaletteActionSpec {
            label: "Show summary",
            action: PaletteActionKind::Execute("summary"),
        },
        PaletteActionSpec {
            label: "Search current view",
            action: PaletteActionKind::Prefill("search "),
        },
        PaletteActionSpec {
            label: "Open view by name",
            action: PaletteActionKind::Prefill("view "),
        },
        PaletteActionSpec {
            label: "Open semantic diff",
            action: PaletteActionKind::Prefill("diff "),
        },
        PaletteActionSpec {
            label: "Export generated source",
            action: PaletteActionKind::Prefill("export "),
        },
        PaletteActionSpec {
            label: "Choose theme",
            action: PaletteActionKind::Prefill("theme "),
        },
        PaletteActionSpec {
            label: "Quit",
            action: PaletteActionKind::Execute("quit"),
        },
    ],
    parse,
};

fn parse(command: &str, rest: &str) -> Option<ExploreCommand> {
    let parsed = match command {
        "help" | "h" | "?" => ExploreCommand::Help,
        "summary" | "s" => ExploreCommand::Summary,
        "views" => ExploreCommand::Views,
        "view" => {
            if rest.is_empty() {
                ExploreCommand::Unknown("view requires a view name".to_string())
            } else {
                // Pass through raw name — resolved at execution time via specs
                ExploreCommand::View(rest.to_string())
            }
        }
        "mode" => {
            if rest.is_empty() {
                ExploreCommand::Unknown("mode requires browse|compare".to_string())
            } else {
                ExploreCommand::Mode(rest.to_string())
            }
        }
        "next" | "n" => match parse_optional_positive_usize(rest, "next") {
            Ok(count) => ExploreCommand::Next { count },
            Err(err) => ExploreCommand::Unknown(err),
        },
        "prev" | "p" => match parse_optional_positive_usize(rest, "prev") {
            Ok(count) => ExploreCommand::Prev { count },
            Err(err) => ExploreCommand::Unknown(err),
        },
        "goto" | "g" => match parse_required_i32(rest, "goto") {
            Ok(value) => ExploreCommand::Goto { value },
            Err(err) => ExploreCommand::Unknown(err),
        },
        "show" => {
            if rest.is_empty() {
                ExploreCommand::Show { index: None }
            } else {
                match parse_required_usize(rest, "show") {
                    Ok(index) => ExploreCommand::Show { index: Some(index) },
                    Err(err) => ExploreCommand::Unknown(err),
                }
            }
        }
        "search" | "/" => {
            if rest.is_empty() {
                ExploreCommand::Unknown("search requires text".to_string())
            } else {
                ExploreCommand::Search {
                    text: rest.to_string(),
                }
            }
        }
        "map" => {
            if rest.is_empty() {
                ExploreCommand::Map { enabled: None }
            } else {
                match rest {
                    "on" | "true" | "1" => ExploreCommand::Map {
                        enabled: Some(true),
                    },
                    "off" | "false" | "0" => ExploreCommand::Map {
                        enabled: Some(false),
                    },
                    _ => ExploreCommand::Unknown(
                        "map expects on|off (or empty to toggle)".to_string(),
                    ),
                }
            }
        }
        "diff" => {
            if rest.is_empty() {
                ExploreCommand::Unknown("diff requires a file path".to_string())
            } else if matches!(rest, "off" | "close" | "clear") {
                ExploreCommand::DiffClose
            } else {
                ExploreCommand::Diff {
                    path: rest.to_string(),
                }
            }
        }
        "export" => {
            if rest.is_empty() {
                ExploreCommand::Unknown("export requires a file path".to_string())
            } else {
                ExploreCommand::Export {
                    path: rest.to_string(),
                }
            }
        }
        "theme" => {
            if rest.is_empty() || rest == "list" {
                ExploreCommand::Theme(ThemeCommand::List)
            } else if matches!(rest, "next" | "cycle") {
                ExploreCommand::Theme(ThemeCommand::Next)
            } else {
                ExploreCommand::Theme(ThemeCommand::Set(rest.to_string()))
            }
        }
        "quit" | "q" | "exit" => ExploreCommand::Quit,
        _ => return None,
    };

    Some(parsed)
}
