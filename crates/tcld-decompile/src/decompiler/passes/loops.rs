use super::super::*;

pub(crate) fn run(this: &mut Decompiler, range: Range<i32>) -> bool {
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
    if jumps.is_empty() {
        return false;
    }
    let in_code_len = codes.len();

    // Detect top-tested loops: conditional jump forward (exit) and unconditional back jump.
    for back_ref in &jumps {
        let (back_location, loop_start) = {
            let borrowed = back_ref.borrow();
            let Code::Instruction(inst) = &*borrowed else {
                continue;
            };
            if !inst.is_jump_unconditional() {
                continue;
            }
            let Some(target) = inst.jump_target() else {
                continue;
            };
            (inst.location, target)
        };
        if loop_start >= back_location {
            continue;
        }

        let mut cond_inst: Option<(i32, i32, i32)> = None;
        let mut best_target = i32::MAX;
        for jref in &jumps {
            let (inst_location, inst_size, target, is_conditional) = {
                let borrowed = jref.borrow();
                let Code::Instruction(inst) = &*borrowed else {
                    continue;
                };
                let Some(target) = inst.jump_target() else {
                    continue;
                };
                (
                    inst.location,
                    inst.size(),
                    target,
                    inst.is_jump_conditional(),
                )
            };
            if !is_conditional {
                continue;
            }
            if inst_location < loop_start || inst_location >= back_location {
                continue;
            }
            if target > back_location && target < best_target {
                best_target = target;
                cond_inst = Some((inst_location, inst_size, target));
            }
        }
        let (cond_location, cond_size, exit_target) = match cond_inst {
            Some(inst) => inst,
            None => continue,
        };

        let cond_cmd = match this.graph.code_before(cond_location) {
            Some(c) => c,
            None => continue,
        };
        if !cond_cmd.borrow().is_command() {
            continue;
        }

        let loop_ex = this
            .bytecode
            .exceptions
            .iter()
            .find(|e| e.kind == ExceptionType::Loop && e.break_offset == exit_target)
            .cloned();

        let body_range = if let Some(ex) = &loop_ex {
            ex.code_range.clone()
        } else {
            (cond_location + cond_size)..back_location
        };
        if body_range.start >= body_range.end {
            continue;
        }
        this.process(body_range.clone());
        let body_codes = this.filter_codes(this.graph.code_in_range(body_range.clone()));

        if let Some(ex) = &loop_ex
            && ex.continue_offset >= ex.code_range.end
            && ex.continue_offset >= 0
        {
            let setter = this.graph.code_before(loop_start);
            let incr_range = this
                .bytecode
                .exceptions
                .iter()
                .find(|e| {
                    e.kind == ExceptionType::Loop
                        && e.break_offset == ex.break_offset
                        && e.code_offset == ex.continue_offset
                })
                .map(|e| e.code_range.clone())
                .unwrap_or(ex.continue_offset..back_location);

            if incr_range.start < incr_range.end {
                this.process(incr_range.clone());
            }
            let incr_codes = this.filter_codes(this.graph.code_in_range(incr_range));
            let incr_cmd = incr_codes
                .iter()
                .rev()
                .find(|c| c.borrow().is_command())
                .cloned();

            if let (Some(setter), Some(incr_cmd)) = (setter, incr_cmd)
                && setter.borrow().is_command()
                && incr_cmd.borrow().is_command()
            {
                let cmd_start = setter.borrow().location();
                if cmd_start < exit_target {
                    let cmd_codes = this.graph.code_in_range(cmd_start..exit_target);
                    if !cmd_codes.is_empty() {
                        let cmd = cmd_for_loop(
                            cmd_codes,
                            CmdBody::new(body_codes),
                            setter,
                            cond_cmd,
                            incr_cmd,
                        );
                        this.graph.replace_code(cmd);
                        return true;
                    }
                }
            }
        }

        let cmd_start = loop_start;
        if cmd_start < exit_target {
            let cmd_codes = this.graph.code_in_range(cmd_start..exit_target);
            if !cmd_codes.is_empty() {
                let cmd =
                    cmd_while_loop(cmd_codes, CmdBody::new(body_codes), code_as_expr(&cond_cmd));
                this.graph.replace_code(cmd);
                return true;
            }
        }
    }

    if jumps.len() == 1 {
        let (is_unconditional, jump_target, jmp_location, jmp_size) = {
            let borrowed = jumps[0].borrow();
            let Code::Instruction(inst) = &*borrowed else {
                return false;
            };
            (
                inst.is_jump_unconditional(),
                inst.jump_target(),
                inst.location,
                inst.size(),
            )
        };
        if is_unconditional
            && let Some(jump_target) = jump_target
            && range.contains(&jump_target)
            && jump_target < jmp_location
        {
            let loop_range = jump_target..jmp_location;
            let loop_code_range = jump_target..(jmp_location + jmp_size);
            this.process(loop_range.clone());
            let cmd_codes = this.graph.code_in_range(loop_code_range);
            let loop_codes = this.filter_codes(this.graph.code_in_range(loop_range));
            let cmd = cmd_while_loop(cmd_codes, CmdBody::new(loop_codes), sr("1"));
            this.graph.replace_code(cmd);
        }
    } else {
        for idx in 0..jumps.len() - 1 {
            let (jmp1_is_unconditional, jmp1_target, jmp1_location, jmp1_size) = {
                let borrowed = jumps[idx].borrow();
                let Code::Instruction(inst) = &*borrowed else {
                    continue;
                };
                (
                    inst.is_jump_unconditional(),
                    inst.jump_target(),
                    inst.location,
                    inst.size(),
                )
            };
            let (jmp2_is_conditional, jmp2_target, jmp2_location, jmp2_size) = {
                let borrowed = jumps[idx + 1].borrow();
                let Code::Instruction(inst) = &*borrowed else {
                    continue;
                };
                (
                    inst.is_jump_conditional(),
                    inst.jump_target(),
                    inst.location,
                    inst.size(),
                )
            };

            let jmp1_target = match jmp1_target {
                Some(v) => v,
                None => continue,
            };
            let jmp2_target = match jmp2_target {
                Some(v) => v,
                None => continue,
            };

            if jmp1_is_unconditional
                && jmp2_is_conditional
                && jmp1_target < jmp2_location
                && jmp2_target < jmp2_location
                && jmp2_target > jmp1_location
            {
                let locations = [
                    jmp1_location,
                    jmp2_location,
                    jmp1_target,
                    jmp2_target,
                    jmp1_location + jmp1_size,
                    jmp2_location + jmp2_size,
                ];
                let loop_range = *locations.iter().min().unwrap()..*locations.iter().max().unwrap();
                let loop_codes = this.filter_codes(this.graph.code_in_range(loop_range.clone()));
                let loop_body_range = (jmp1_location + jmp1_size)..jmp1_target;
                this.process(loop_body_range.clone());
                let loop_body =
                    this.filter_codes(this.graph.code_in_range(loop_body_range.clone()));
                if loop_body
                    .iter()
                    .filter(|c| {
                        if let Code::Instruction(inst) = &*c.borrow() {
                            inst.is_jump()
                        } else {
                            false
                        }
                    })
                    .count()
                    > 0
                {
                    continue;
                }
                let conditional_range = jmp1_target..jmp2_location;
                let conditional_codes = this.graph.code_in_range(conditional_range);
                if conditional_codes.len() == 1 {
                    let cmd = cmd_while_loop(
                        loop_codes,
                        CmdBody::new(loop_body),
                        code_as_expr(&conditional_codes[0]),
                    );
                    this.graph.replace_code(cmd);
                }
                break;
            }
        }
    }

    this.graph.code_count_in_range(range) != in_code_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{ByteCodeInfo, ExceptionRange, Instruction};
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

    fn decompiler_with_graph(
        instructions: Vec<Instruction>,
        exceptions: Vec<ExceptionRange>,
    ) -> Decompiler {
        let bytecode = ByteCode {
            header: None,
            info: ByteCodeInfo {
                num_commands: 0,
                num_src_bytes: 0,
                num_code_bytes: 0,
                num_lit_objects: 0,
                num_except_ranges: exceptions.len() as i32,
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
            exceptions,
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
    fn recovers_top_tested_while_loop() {
        let mut decompiler = decompiler_with_graph(
            vec![
                instruction(0, Opcode::Push1, vec![0], Some(sr("condition"))),
                instruction(2, Opcode::JumpFalse1, vec![8], None),
                instruction(4, Opcode::Push1, vec![0], Some(sr("body"))),
                instruction(6, Opcode::Jump1, vec![-6], None),
                instruction(10, Opcode::Done, Vec::new(), None),
            ],
            Vec::new(),
        );
        replace_literal(&mut decompiler, 0, "condition");

        assert!(run(&mut decompiler, 0..11));
        let recovered = decompiler.graph.code_at_location(0).expect("while loop");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("while must reduce to a command");
        };
        let CommandKind::WhileLoop { body, condition } = &command.kind else {
            panic!("expected while loop");
        };
        assert!(condition.repr_eq_str("condition"));
        assert!(!body.block.children.is_empty());
    }

    #[test]
    fn recovers_exception_backed_for_loop() {
        let exceptions = vec![
            ExceptionRange {
                kind: ExceptionType::Loop,
                nesting_level: 0,
                code_offset: 6,
                num_code_bytes: 2,
                break_offset: 12,
                continue_offset: 8,
                catch_offset: -1,
                code_range: 6..8,
            },
            ExceptionRange {
                kind: ExceptionType::Loop,
                nesting_level: 0,
                code_offset: 8,
                num_code_bytes: 2,
                break_offset: 12,
                continue_offset: 8,
                catch_offset: -1,
                code_range: 8..10,
            },
        ];
        let mut decompiler = decompiler_with_graph(
            vec![
                instruction(0, Opcode::Push1, vec![0], Some(sr("setter"))),
                instruction(2, Opcode::Push1, vec![0], Some(sr("condition"))),
                instruction(4, Opcode::JumpFalse1, vec![8], None),
                instruction(6, Opcode::Push1, vec![0], Some(sr("body"))),
                instruction(8, Opcode::Push1, vec![0], Some(sr("increment"))),
                instruction(10, Opcode::Jump1, vec![-8], None),
                instruction(12, Opcode::Done, Vec::new(), None),
            ],
            exceptions,
        );
        replace_literal(&mut decompiler, 0, "setter");
        replace_literal(&mut decompiler, 2, "condition");

        assert!(run(&mut decompiler, 0..13));
        let recovered = decompiler.graph.code_at_location(0).expect("for loop");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("for must reduce to a command");
        };
        let CommandKind::ForLoop {
            body,
            setter,
            condition,
            incr,
        } = &command.kind
        else {
            panic!("expected for loop");
        };
        assert!(!body.block.children.is_empty());
        assert!(setter.borrow().is_command());
        assert!(condition.borrow().is_command());
        assert!(incr.borrow().is_command());
    }

    #[test]
    fn infinite_loop_processes_body_before_wrapping() {
        let mut decompiler = decompiler_with_graph(
            vec![
                instruction(0, Opcode::Push1, vec![0], Some(sr("body"))),
                instruction(2, Opcode::Jump1, vec![-2], None),
            ],
            Vec::new(),
        );

        assert!(run(&mut decompiler, 0..4));
        let recovered = decompiler.graph.code_at_location(0).expect("while loop");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("while must reduce to a command");
        };
        let CommandKind::WhileLoop { body, condition } = &command.kind else {
            panic!("expected while loop");
        };
        assert!(condition.repr_eq_str("1"));
        assert!(
            body.block
                .children
                .values()
                .all(|child| child.borrow().is_command()),
            "infinite-loop bodies must not retain raw instructions"
        );
    }

    #[test]
    fn rejects_single_forward_jump_as_a_loop() {
        let mut decompiler = decompiler_with_graph(
            vec![
                instruction(0, Opcode::Jump1, vec![2], None),
                instruction(2, Opcode::Done, Vec::new(), None),
            ],
            Vec::new(),
        );

        assert!(!run(&mut decompiler, 0..3));
        assert!(matches!(
            &*decompiler
                .graph
                .code_at_location(0)
                .expect("forward jump")
                .borrow(),
            Code::Instruction(inst) if inst.opcode == Opcode::Jump1
        ));
    }

    #[test]
    fn recovers_bottom_backedge_loop_geometry() {
        let mut decompiler = decompiler_with_graph(
            vec![
                instruction(10, Opcode::Jump1, vec![10], None),
                instruction(12, Opcode::Push1, vec![0], Some(sr("body-prefix"))),
                instruction(14, Opcode::Push1, vec![0], Some(sr("body"))),
                instruction(20, Opcode::Push1, vec![0], Some(sr("condition"))),
                instruction(24, Opcode::JumpTrue1, vec![-10], None),
                instruction(26, Opcode::Done, Vec::new(), None),
            ],
            Vec::new(),
        );
        replace_literal(&mut decompiler, 12, "body-prefix");
        replace_literal(&mut decompiler, 14, "body");
        replace_literal(&mut decompiler, 20, "condition");

        assert!(run(&mut decompiler, 10..27));

        let recovered = decompiler
            .graph
            .code_at_location(10)
            .expect("recovered loop");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("loop must reduce to a command");
        };
        let CommandKind::WhileLoop { body, condition } = &command.kind else {
            panic!("expected while loop");
        };
        assert!(condition.repr_eq_str("condition"));
        assert_eq!(body.block.children.len(), 2);
    }
}
