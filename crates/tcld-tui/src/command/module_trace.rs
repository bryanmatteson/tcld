use super::{
    CommandModule, ExploreCommand, PaletteActionKind, PaletteActionSpec, TraceCommand,
    parse_required_usize,
};

pub const MODULE: CommandModule = CommandModule {
    id: "trace",
    description: "bytecode stack/flow trace controls",
    help_lines: &["trace [on|off|next|prev|i] Inspect stack/flow trace"],
    palette_actions: &[
        PaletteActionSpec {
            label: "Toggle trace",
            action: PaletteActionKind::Execute("trace"),
        },
        PaletteActionSpec {
            label: "Go to trace frame",
            action: PaletteActionKind::Prefill("trace "),
        },
    ],
    parse,
};

fn parse(command: &str, rest: &str) -> Option<ExploreCommand> {
    if !matches!(command, "trace" | "tr") {
        return None;
    }

    let parsed = if rest.is_empty() {
        ExploreCommand::Trace(TraceCommand::Toggle)
    } else {
        match rest {
            "on" | "true" | "1" => ExploreCommand::Trace(TraceCommand::On),
            "off" | "false" | "0" => ExploreCommand::Trace(TraceCommand::Off),
            "next" | "n" => ExploreCommand::Trace(TraceCommand::Next),
            "prev" | "p" => ExploreCommand::Trace(TraceCommand::Prev),
            _ => match parse_required_usize(rest, "trace") {
                Ok(idx) => ExploreCommand::Trace(TraceCommand::Goto(idx)),
                Err(_) => ExploreCommand::Unknown(
                    "trace expects on|off|next|prev|<instruction-index>".to_string(),
                ),
            },
        }
    };

    Some(parsed)
}
