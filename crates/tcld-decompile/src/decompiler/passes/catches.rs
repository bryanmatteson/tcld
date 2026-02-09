use super::super::*;

pub(crate) fn run(this: &mut Decompiler, range: Range<i32>) {
    let insts: Vec<CodeRef> = this
        .graph
        .code_in_range(range.clone())
        .into_iter()
        .filter(|c| matches!(&*c.borrow(), Code::Instruction(_)))
        .collect();
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for code in insts {
        if let Code::Instruction(inst) = &*code.borrow() {
            if inst.opcode == Opcode::BeginCatch4 {
                starts.push(inst.location);
            }
            if inst.opcode == Opcode::EndCatch {
                ends.push(inst.location);
            }
        }
    }
    if ends.is_empty() || starts.is_empty() {
        return;
    }
    if ends.len() != starts.len() {
        return;
    }

    let stitched = this.stitch_ranges(&starts, &ends);
    for catch_range in stitched {
        let codes = this.graph.code_in_range(catch_range.clone());
        let end_catch_location = catch_range.end - 1;
        let jmps: Vec<CodeRef> = codes
            .iter()
            .filter(|c| {
                if let Code::Instruction(inst) = &*c.borrow() {
                    inst.is_jump_unconditional() && inst.jump_target() == Some(end_catch_location)
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        if jmps.len() != 1 {
            continue;
        }
        let code_range = {
            let Some(first) = codes.first() else {
                continue;
            };
            let start = match &*first.borrow() {
                Code::Instruction(inst) => match inst.fallthrough_location() {
                    Some(v) => v,
                    None => continue,
                },
                _ => continue,
            };
            let end = jmps[0].borrow().location();
            start..end
        };
        this.process(code_range.clone());
        let mut catch_codes = this.graph.code_in_range(code_range);
        let should_pop_zero = catch_codes
            .last()
            .map(|last| {
                if let Code::Command(cmd) = &*last.borrow()
                    && let CommandKind::Literal { value } = &cmd.kind
                {
                    return value.repr_eq_str("0");
                }
                false
            })
            .unwrap_or(false);
        if should_pop_zero {
            catch_codes.pop();
        }

        let body = CmdBody::new(this.filter_codes(catch_codes));
        let codes = this.graph.code_in_range(catch_range);
        let cmd = cmd_catch(codes, body);
        this.graph.replace_code(cmd);
        this.process_instructions(range.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{ByteCodeInfo, Instruction};
    use crate::inst_table::DEFAULT_INSTRUCTION_SET;

    fn instruction(
        location: i32,
        opcode: Opcode,
        operands: Vec<i32>,
        value: Option<SourceRef>,
    ) -> Instruction {
        let desc = *DEFAULT_INSTRUCTION_SET
            .instruction_table
            .iter()
            .find(|desc| DEFAULT_INSTRUCTION_SET.opcode_from_i32(desc.opcode) == Some(opcode))
            .expect("instruction descriptor");
        Instruction::new(location, opcode, desc, operands, value)
    }

    fn decompiler_with_graph(instructions: Vec<Instruction>) -> Decompiler {
        let bytecode = ByteCode {
            header: None,
            info: ByteCodeInfo {
                num_commands: 0,
                num_src_bytes: 0,
                num_code_bytes: 0,
                num_lit_objects: 0,
                num_except_ranges: 0,
                num_aux_data_items: 0,
                num_cmd_loc_bytes: 0,
                max_except_depth: 0,
                max_stack_depth: 0,
                code_delta_size: 0,
                code_length_size: 0,
                src_delta_size: 0,
                src_length_size: 0,
            },
            code_bytes: Vec::new(),
            cmd_code_ranges: Vec::new(),
            cmd_source_ranges: None,
            literals: Vec::new(),
            exceptions: Vec::new(),
            auxiliaries: Vec::new(),
            proc_num_args: 0,
            proc_num_locals: 0,
            locals: Vec::new(),
        };
        let mut decompiler = Decompiler::new(bytecode).expect("decompiler");
        decompiler.graph = CodeGraph::new(instructions);
        decompiler
    }

    fn replace_literal(decompiler: &mut Decompiler, location: i32, value: &'static str) {
        let child = decompiler
            .graph
            .code_at_location(location)
            .expect("literal instruction");
        decompiler
            .graph
            .replace_code(cmd_literal(vec![child], sr(value)));
    }

    #[test]
    fn nested_catches_recover_innermost_first() {
        let mut decompiler = decompiler_with_graph(vec![
            instruction(0, Opcode::BeginCatch4, vec![0], None),
            instruction(5, Opcode::BeginCatch4, vec![1], None),
            instruction(10, Opcode::Push1, vec![0], Some(sr("inner-body"))),
            instruction(12, Opcode::Push1, vec![0], Some(sr("0"))),
            instruction(14, Opcode::Jump1, vec![3], None),
            instruction(16, Opcode::PushReturnCode, Vec::new(), None),
            instruction(17, Opcode::EndCatch, Vec::new(), None),
            instruction(18, Opcode::Pop, Vec::new(), None),
            instruction(19, Opcode::Push1, vec![0], Some(sr("0"))),
            instruction(21, Opcode::Jump1, vec![3], None),
            instruction(23, Opcode::PushReturnCode, Vec::new(), None),
            instruction(24, Opcode::EndCatch, Vec::new(), None),
            instruction(25, Opcode::Done, Vec::new(), None),
        ]);
        replace_literal(&mut decompiler, 10, "inner-body");
        replace_literal(&mut decompiler, 12, "0");
        replace_literal(&mut decompiler, 19, "0");

        run(&mut decompiler, 0..26);

        let recovered = decompiler.graph.code_at_location(0).expect("outer catch");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("outer catch must reduce to a command");
        };
        let CommandKind::Catch { body } = &command.kind else {
            panic!("expected outer catch");
        };
        assert!(
            body.block.children.values().any(|child| matches!(
                &*child.borrow(),
                Code::Command(command) if matches!(command.kind, CommandKind::Catch { .. })
            )),
            "outer catch body must retain the recovered inner catch"
        );
    }

    #[test]
    fn catch_exit_is_distinguished_from_internal_branch_jumps() {
        let mut decompiler = decompiler_with_graph(vec![
            instruction(0, Opcode::BeginCatch4, vec![0], None),
            instruction(5, Opcode::Push1, vec![0], Some(sr("condition"))),
            instruction(7, Opcode::JumpFalse1, vec![6], None),
            instruction(9, Opcode::Push1, vec![0], Some(sr("body"))),
            instruction(11, Opcode::Jump1, vec![4], None),
            instruction(13, Opcode::Push1, vec![0], Some(sr(""))),
            instruction(15, Opcode::Pop, Vec::new(), None),
            instruction(16, Opcode::Push1, vec![0], Some(sr("0"))),
            instruction(18, Opcode::Jump1, vec![3], None),
            instruction(20, Opcode::PushReturnCode, Vec::new(), None),
            instruction(21, Opcode::EndCatch, Vec::new(), None),
            instruction(22, Opcode::Done, Vec::new(), None),
        ]);
        for (location, value) in [(5, "condition"), (9, "body"), (13, ""), (16, "0")] {
            replace_literal(&mut decompiler, location, value);
        }

        run(&mut decompiler, 0..23);

        let recovered = decompiler.graph.code_at_location(0).expect("catch");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("catch must reduce to a command");
        };
        let CommandKind::Catch { body } = &command.kind else {
            panic!("expected catch command");
        };
        assert!(
            body.block.children.values().any(|child| matches!(
                &*child.borrow(),
                Code::Command(command) if matches!(command.kind, CommandKind::IfElse { .. })
            )),
            "the catch body must retain its reduced internal conditional"
        );
    }

    #[test]
    fn catch_ranges_reject_end_before_start() {
        let mut decompiler = decompiler_with_graph(vec![
            instruction(0, Opcode::EndCatch, Vec::new(), None),
            instruction(1, Opcode::BeginCatch4, vec![0], None),
            instruction(6, Opcode::Jump1, vec![2], None),
            instruction(8, Opcode::Done, Vec::new(), None),
        ]);

        run(&mut decompiler, 0..9);

        assert!(matches!(
            &*decompiler
                .graph
                .code_at_location(1)
                .expect("begin catch")
                .borrow(),
            Code::Instruction(inst) if inst.opcode == Opcode::BeginCatch4
        ));
    }
}
