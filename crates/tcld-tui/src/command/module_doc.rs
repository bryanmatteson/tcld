use super::{CommandModule, ExploreCommand, PaletteActionKind, PaletteActionSpec};

pub const MODULE: CommandModule = CommandModule {
    id: "document",
    description: "document-level decompile commands",
    help_lines: &[
        "decompile               Switch to decompile view",
        "showdecomp              Reveal decompile for current instruction",
    ],
    palette_actions: &[PaletteActionSpec {
        label: "Reveal selected instruction in Decompile",
        action: PaletteActionKind::Execute("showdecomp"),
    }],
    parse,
};

fn parse(command: &str, _rest: &str) -> Option<ExploreCommand> {
    let parsed = match command {
        "decompile" | "dec" => ExploreCommand::View("decompile".to_string()),
        "showdecomp" | "sd" => ExploreCommand::ShowDecompile,
        "dump" => {
            ExploreCommand::Unknown("Dump is a batch command; use `tcld dump <file>`".to_string())
        }
        "graph" | "cfg" => ExploreCommand::Unknown(
            "Graph is not an Explore view; use `tcld disassemble <file>`".to_string(),
        ),
        _ => return None,
    };
    Some(parsed)
}
