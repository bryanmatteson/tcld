use super::super::*;

pub(crate) fn run_chained_logicals(this: &mut Decompiler, range: Range<i32>) {
    let codes = this.graph.code_in_range(range.clone());
    let jumps: Vec<CodeRef> = codes
        .iter()
        .filter(|c| {
            if let Code::Instruction(inst) = &*c.borrow() {
                inst.is_jump()
            } else {
                false
            }
        })
        .cloned()
        .collect();

    for idx in 0..jumps.len() {
        if idx + 2 >= jumps.len() {
            continue;
        }
        let j0 = jumps[idx].borrow();
        let j1 = jumps[idx + 1].borrow();
        let j2 = jumps[idx + 2].borrow();
        let j0_inst = match &*j0 {
            Code::Instruction(inst) => inst,
            _ => continue,
        };
        let j1_inst = match &*j1 {
            Code::Instruction(inst) => inst,
            _ => continue,
        };
        let j2_inst = match &*j2 {
            Code::Instruction(inst) => inst,
            _ => continue,
        };
        if !j0_inst.is_jump_conditional()
            || !j1_inst.is_jump_unconditional()
            || !j2_inst.is_jump_conditional()
        {
            continue;
        }

        let dup = match j1_inst
            .jump_target()
            .and_then(|target| this.graph.code_at_location(target))
        {
            Some(c) => c,
            None => continue,
        };
        let (dup_opcode, dup_fallthrough) = match &*dup.borrow() {
            Code::Instruction(inst) => (inst.opcode, inst.fallthrough_location()),
            _ => continue,
        };
        if dup_opcode != Opcode::Dup {
            continue;
        }
        if dup_fallthrough != Some(j2_inst.location) {
            continue;
        }
        let test_cmd = match this.graph.single_source_of(&jumps[idx]) {
            Some(c) => c,
            None => continue,
        };
        if !test_cmd.borrow().is_command() {
            continue;
        }
        let test_end = test_cmd.borrow().range().end;
        let chain_end = j2_inst.location + j2_inst.size();
        let chained_codes = this.graph.code_in_range(test_end..chain_end);
        if let Code::Command(cmd) = &mut *test_cmd.borrow_mut() {
            cmd.merge(chained_codes);
        }
        this.graph.replace_code(test_cmd.clone());
        this.process_instructions(range.clone());
        let check_range = test_cmd.borrow().location()..range.end;
        run_chained_logicals(this, check_range);
        this.process_stack(range.clone());
    }
}

pub(crate) fn run_if_else(this: &mut Decompiler, range: Range<i32>) -> bool {
    let codes = this.graph.code_in_range(range.clone());
    let jumps: Vec<CodeRef> = codes
        .iter()
        .filter(|c| {
            if let Code::Instruction(inst) = &*c.borrow() {
                inst.is_jump()
            } else {
                false
            }
        })
        .cloned()
        .collect();
    if jumps.len() < 2 {
        return false;
    }
    let in_code_len = codes.len();

    'conditional: for idx in 0..jumps.len() {
        let (jmp1_is_conditional, jmp1_target, jmp1_location, jmp1_size) =
            match &*jumps[idx].borrow() {
                Code::Instruction(inst) => (
                    inst.is_jump_conditional(),
                    inst.jump_target(),
                    inst.location,
                    inst.size(),
                ),
                _ => continue,
            };

        if !jmp1_is_conditional {
            continue;
        }

        for jmp2 in jumps.iter().skip(idx + 1) {
            let (jmp2_is_unconditional, jmp2_target, jmp2_location, jmp2_size) =
                match &*jmp2.borrow() {
                    Code::Instruction(inst) => (
                        inst.is_jump_unconditional(),
                        inst.jump_target(),
                        inst.location,
                        inst.size(),
                    ),
                    _ => continue,
                };

            if !jmp2_is_unconditional || jmp1_target != Some(jmp2_location + jmp2_size) {
                continue;
            }

            let Some(cmd_end) = jmp2_target else {
                continue;
            };
            if cmd_end <= jmp2_location {
                // Backward unconditional jumps are loop-like; skip if/else handling.
                continue;
            }
            let test_code = codes.iter().find(|c| {
                let c_b = c.borrow();
                c_b.location() + c_b.size() == jmp1_location
            });
            if let Some(test_code) = test_code {
                if !test_code.borrow().is_command() {
                    continue;
                }
                let if_body_range = (jmp1_location + jmp1_size)..jmp2_location;
                let else_body_range = (jmp2_location + jmp2_size)..cmd_end;
                this.process(if_body_range.clone());
                this.process(else_body_range.clone());
                let if_body = this.filter_codes(this.graph.code_in_range(if_body_range));
                let else_body = this.filter_codes(this.graph.code_in_range(else_body_range));
                let cmd_start = test_code.borrow().location();
                if cmd_start >= cmd_end {
                    continue;
                }
                let cmd_range = cmd_start..cmd_end;
                let cmd_codes = this.graph.code_in_range(cmd_range);
                if cmd_codes.is_empty() {
                    continue;
                }
                let cmd = cmd_if_else(
                    cmd_codes,
                    test_code.clone(),
                    CmdBody::new(if_body),
                    CmdBody::new(else_body),
                );
                this.graph.replace_code(cmd);
                this.process_instructions(range.clone());
                break 'conditional;
            }
        }
    }

    this.graph.code_count_in_range(range) != in_code_len
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

    fn if_else_decompiler(final_target: i32) -> Decompiler {
        let mut decompiler = decompiler_with_graph(vec![
            instruction(0, Opcode::Push1, vec![0], Some(sr("condition"))),
            instruction(2, Opcode::JumpFalse1, vec![6], None),
            instruction(4, Opcode::Push1, vec![0], Some(sr("if-body"))),
            instruction(6, Opcode::Jump1, vec![final_target - 6], None),
            instruction(8, Opcode::Push1, vec![0], Some(sr("else-body"))),
            instruction(10, Opcode::Done, Vec::new(), None),
        ]);
        let test = decompiler.graph.code_at_location(0).expect("test");
        decompiler
            .graph
            .replace_code(cmd_literal(vec![test], sr("condition")));
        decompiler
    }

    #[test]
    fn recovers_if_else_from_forward_jump_geometry() {
        let mut decompiler = if_else_decompiler(10);
        assert!(run_if_else(&mut decompiler, 0..11));

        let recovered = decompiler.graph.code_at_location(0).expect("if command");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("if/else must reduce to a command");
        };
        let CommandKind::IfElse {
            test,
            if_body,
            else_body,
        } = &command.kind
        else {
            panic!("expected if/else command");
        };
        assert!(code_as_source(test).repr_eq_str("condition"));
        assert!(!if_body.block.children.is_empty());
        assert!(!else_body.block.children.is_empty());
    }

    #[test]
    fn recovers_outer_if_with_nested_control_flow() {
        let mut decompiler = decompiler_with_graph(vec![
            instruction(0, Opcode::Push1, vec![0], Some(sr("outer-condition"))),
            instruction(2, Opcode::JumpFalse1, vec![14], None),
            instruction(4, Opcode::Push1, vec![0], Some(sr("inner-condition"))),
            instruction(6, Opcode::JumpFalse1, vec![6], None),
            instruction(8, Opcode::Push1, vec![0], Some(sr("inner-body"))),
            instruction(10, Opcode::Jump1, vec![4], None),
            instruction(12, Opcode::Push1, vec![0], Some(sr(""))),
            instruction(14, Opcode::Jump1, vec![6], None),
            instruction(16, Opcode::Push1, vec![0], Some(sr("outer-else"))),
            instruction(18, Opcode::Pop, Vec::new(), None),
            instruction(19, Opcode::Done, Vec::new(), None),
        ]);
        for (location, value) in [
            (0, "outer-condition"),
            (4, "inner-condition"),
            (8, "inner-body"),
            (12, ""),
            (16, "outer-else"),
        ] {
            let child = decompiler
                .graph
                .code_at_location(location)
                .expect("literal instruction");
            decompiler
                .graph
                .replace_code(cmd_literal(vec![child], sr(value)));
        }

        assert!(run_if_else(&mut decompiler, 0..20));

        let recovered = decompiler.graph.code_at_location(0).expect("outer if");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("outer if must reduce to a command");
        };
        let CommandKind::IfElse { if_body, .. } = &command.kind else {
            panic!("expected outer if/else command");
        };
        assert!(
            if_body.block.children.values().any(|child| matches!(
                &*child.borrow(),
                Code::Command(command) if matches!(command.kind, CommandKind::IfElse { .. })
            )),
            "the outer body must retain its reduced nested conditional"
        );
    }

    #[test]
    fn rejects_backward_unconditional_jump_as_loop_geometry() {
        let mut decompiler = if_else_decompiler(0);
        assert!(!run_if_else(&mut decompiler, 0..11));
        assert!(matches!(
            &*decompiler
                .graph
                .code_at_location(2)
                .expect("conditional jump")
                .borrow(),
            Code::Instruction(inst) if inst.opcode == Opcode::JumpFalse1
        ));
    }

    #[test]
    fn chained_logical_recovery_uses_dup_fallthrough() {
        let mut decompiler = decompiler_with_graph(vec![
            instruction(0, Opcode::Push1, vec![0], Some(sr("left"))),
            instruction(2, Opcode::JumpFalse1, vec![7], None),
            instruction(4, Opcode::Jump1, vec![2], None),
            instruction(6, Opcode::Dup, Vec::new(), None),
            instruction(7, Opcode::JumpFalse1, vec![2], None),
            instruction(9, Opcode::Done, Vec::new(), None),
        ]);
        let test = decompiler.graph.code_at_location(0).expect("logical test");
        decompiler
            .graph
            .replace_code(cmd_literal(vec![test], sr("left")));

        run_chained_logicals(&mut decompiler, 0..10);

        let recovered = decompiler.graph.code_at_location(0).expect("merged test");
        assert!(recovered.borrow().is_command());
        assert_eq!(recovered.borrow().range().start, 0);
        assert!(recovered.borrow().range().end >= 9);
        assert!(decompiler.graph.code_at_location(2).is_none());
    }
}
