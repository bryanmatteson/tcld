use crate::error::Result;
use crate::loader::{LoadedBytecode, load_bytecode_from_path};
use crate::source::SourceRepresentable;
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DumpOptions {
    pub show_all_literals: bool,
    pub literal_filter: Option<String>,
}

pub fn dump_path(
    path: &str,
    show_all_literals: bool,
    literal_filter: Option<&str>,
) -> Result<String> {
    dump_path_with_options(
        path,
        DumpOptions {
            show_all_literals,
            literal_filter: literal_filter.map(|s| s.to_string()),
        },
    )
}

pub fn dump_path_with_options(path: &str, options: DumpOptions) -> Result<String> {
    let bytecode = load_bytecode_from_path(path)?;
    dump_bytecode(&bytecode, options)
}

pub fn dump_loaded(loaded: &LoadedBytecode) -> Result<String> {
    dump_bytecode(&loaded.bytecode, DumpOptions::default())
}

pub fn dump_bytecode(bytecode: &crate::bytecode::ByteCode, options: DumpOptions) -> Result<String> {
    let mut output = String::new();

    output.push_str("Exceptions:\n");
    for (idx, ex) in bytecode.exceptions.iter().enumerate() {
        let _ = writeln!(
            output,
            "{}: {:?} nesting {} code {}..{} break {} continue {} catch {}",
            idx,
            ex.kind,
            ex.nesting_level,
            ex.code_offset,
            ex.code_offset + ex.num_code_bytes,
            ex.break_offset,
            ex.continue_offset,
            ex.catch_offset
        );
    }

    if options.show_all_literals {
        output.push_str("\nLiterals (all):\n");
        for (idx, lit) in bytecode.literals.iter().enumerate() {
            let mut s = lit.source_repr();
            if s.contains('\n') {
                s = s.replace('\n', "\\n");
            }
            let _ = writeln!(output, "{}: {:?} {}", idx, lit.kind, s);
        }
    } else if let Some(filter) = options.literal_filter.as_deref() {
        let _ = writeln!(output, "\nLiterals (matching '{filter}'):");
        for (idx, lit) in bytecode.literals.iter().enumerate() {
            let s = lit.source_repr();
            if s.contains(filter) {
                let _ = writeln!(output, "{}: {}", idx, s);
            }
        }
    }

    output.push_str("\nInstructions:\n");
    for inst in bytecode.get_instructions()? {
        output.push_str(&inst.debug_description());
        output.push('\n');
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{LoadInput, load_bytecode};

    fn fixture() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/tcld/tests/fixtures/expr.tbc")
    }

    #[test]
    fn dump_entrypoints_cover_default_all_and_filtered_literals() {
        let path = fixture();
        let path_str = path.to_str().expect("UTF-8 fixture path");
        let default_dump = dump_path(path_str, false, None).expect("default dump");
        assert!(default_dump.contains("Exceptions:\n"));
        assert!(default_dump.contains("Instructions:\n"));
        assert!(!default_dump.contains("Literals (all):"));

        let all = dump_path_with_options(
            path_str,
            DumpOptions {
                show_all_literals: true,
                literal_filter: Some("ignored when showing all".to_string()),
            },
        )
        .expect("all literals");
        assert!(all.contains("Literals (all):"));

        let filtered = dump_path(path_str, false, Some("expr")).expect("filtered literals");
        assert!(filtered.contains("Literals (matching 'expr'):"));

        let loaded = load_bytecode(LoadInput::path(path)).expect("loaded fixture");
        assert_eq!(dump_loaded(&loaded).unwrap(), default_dump);
    }
}
