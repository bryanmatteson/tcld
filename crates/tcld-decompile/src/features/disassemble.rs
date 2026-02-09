use super::canonicalize::canonicalize_tcl;
use crate::error::Result;
use crate::loader::{LoadedBytecode, load_bytecode_from_path};
use crate::source::SourceRef;
use crate::util::{left_justified, right_justified, truncate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisassembleOptions {
    pub text_truncate: Option<usize>,
    pub line_width: Option<usize>,
}

impl Default for DisassembleOptions {
    fn default() -> Self {
        Self {
            text_truncate: Some(30),
            line_width: None,
        }
    }
}

pub fn disassemble_path(path: &str) -> Result<String> {
    disassemble_path_with_options(path, DisassembleOptions::default())
}

pub fn disassemble_path_with_options(path: &str, options: DisassembleOptions) -> Result<String> {
    let bytecode = load_bytecode_from_path(path)?;
    disassemble_bytecode(&bytecode, options)
}

pub fn disassemble_loaded(loaded: &LoadedBytecode) -> Result<String> {
    disassemble_bytecode(&loaded.bytecode, DisassembleOptions::default())
}

pub fn disassemble_bytecode(
    bytecode: &crate::bytecode::ByteCode,
    options: DisassembleOptions,
) -> Result<String> {
    let mut output = String::new();
    for inst in bytecode.get_instructions()? {
        output.push_str(&render_instruction_line(
            &inst,
            options.text_truncate,
            options.line_width,
        ));
        output.push('\n');
    }
    Ok(output)
}

fn render_instruction_line(
    inst: &crate::bytecode::Instruction,
    text_truncate: Option<usize>,
    line_width: Option<usize>,
) -> String {
    let mut result = format!("({}) ", inst.location);
    result = right_justified(&result, 9, false);

    let mut ops = String::new();
    for (idx, operand) in inst.operands.iter().enumerate() {
        if idx > 0 {
            ops.push(' ');
        }
        ops.push_str(&operand.to_string());
    }
    let mut name = if ops.is_empty() {
        inst.name().to_string()
    } else {
        let mut out = String::from(inst.name());
        out.push(' ');
        out.push_str(&ops);
        out
    };
    name = left_justified(&name, 20, false);
    result.push_str(&name);

    if inst.is_jump() {
        result.push_str("# pc ");
        if let Some(target) = inst.jump_target() {
            result.push_str(&target.to_string());
        } else {
            result.push_str("n/a");
        }
        return result;
    }

    if let Some(val) = &inst.value {
        let mut rendered = render_value_for_display(val);
        rendered = rendered
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        let line_limit = line_width.map(|width| {
            width.saturating_sub(
                result.chars().count() + "# ".chars().count() + "...".chars().count(),
            )
        });
        if let Some(limit) = text_truncate.or(line_limit) {
            rendered = truncate(&rendered, limit, "...");
        }
        result.push_str("# ");
        result.push_str(&rendered);
    }
    result
}

fn render_value_for_display(value: &SourceRef) -> String {
    let mut raw = String::new();
    value.write_repr(&mut raw);
    if raw.contains('\n') {
        canonicalize_tcl(&raw)
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inst_table::DEFAULT_INSTRUCTION_SET;
    use crate::source::sr;

    fn push1_instruction_with_value(value: SourceRef) -> crate::bytecode::Instruction {
        let opcode = DEFAULT_INSTRUCTION_SET
            .instruction_table
            .iter()
            .find(|desc| desc.name == "push1")
            .expect("push1 opcode exists")
            .opcode;
        let desc = *DEFAULT_INSTRUCTION_SET
            .desc_from_i32(opcode)
            .expect("instruction desc exists");
        crate::bytecode::Instruction::new(
            10,
            DEFAULT_INSTRUCTION_SET
                .opcode_from_i32(opcode)
                .expect("opcode exists"),
            desc,
            vec![0],
            Some(value),
        )
    }

    #[test]
    fn disassemble_canonicalizes_multiline_literal_values() {
        let inst = push1_instruction_with_value(sr("\n    set a 1  \n    set b 2\t\n"));
        let line = render_instruction_line(&inst, None, None);
        assert!(line.contains("#     set a 1\\n    set b 2"));
        assert!(!line.contains("\\t"));
    }

    #[test]
    fn disassemble_keeps_single_line_literal_values() {
        let inst = push1_instruction_with_value(sr("fixture::send_this"));
        let line = render_instruction_line(&inst, None, None);
        assert!(line.contains("# fixture::send_this"));
    }

    #[test]
    fn disassemble_line_width_truncates_from_actual_prefix() {
        let inst = push1_instruction_with_value(sr("abcdefghijklmnopqrstuvwxyz"));
        let line = render_instruction_line(&inst, None, Some(36));
        assert_eq!(line.chars().count(), 36);
        assert!(line.ends_with("ab..."));
    }
}
