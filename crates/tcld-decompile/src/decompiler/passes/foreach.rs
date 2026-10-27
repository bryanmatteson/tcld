use super::super::*;

pub(crate) fn run(this: &mut Decompiler, range: Range<i32>) {
    let codes = this.graph.code_in_range(range);
    let insts: Vec<CodeRef> = codes
        .iter()
        .filter(|c| matches!(&*c.borrow(), Code::Instruction(_)))
        .cloned()
        .collect();
    let starts: Vec<CodeRef> = insts
        .iter()
        .filter(|c| {
            if let Code::Instruction(inst) = &*c.borrow() {
                matches!(inst.opcode, Opcode::ForeachStart4 | Opcode::ForeachStart)
            } else {
                false
            }
        })
        .cloned()
        .collect();

    for start in starts {
        let (start_opcode, start_location, start_size, start_primary_next, aux_idx) =
            match &*start.borrow() {
                Code::Instruction(inst) => (
                    inst.opcode,
                    inst.location,
                    inst.size(),
                    inst.primary_next_location(),
                    match inst.operands.first() {
                        Some(v) => *v as usize,
                        None => continue,
                    },
                ),
                _ => continue,
            };
        // Processing an outer loop recursively replaces its nested starts.
        // The candidates above are a snapshot, so skip nodes no longer owned
        // by the live graph rather than rebuilding them from stale operands.
        if !this
            .graph
            .code_at_location(start_location)
            .is_some_and(|live| live.ptr_eq(&start))
        {
            continue;
        }
        let aux = match this.bytecode.auxiliaries.get(aux_idx) {
            Some(aux) => aux,
            None => continue,
        };
        let foreach_lists = match &aux.value {
            AuxDataValue::Foreach(info) => info.lists.as_slice(),
            AuxDataValue::NewForeach(info) => info.lists.as_slice(),
            _ => continue,
        };
        let num_lists = foreach_lists.len();
        if num_lists == 0 {
            continue;
        }
        let code_start_idx = match codes
            .iter()
            .position(|c| c.borrow().location() == start_location)
        {
            Some(idx) => idx,
            None => continue,
        };
        let (_values_start_idx, value_cmds) =
            match this.collect_args_skipping(&codes, code_start_idx, num_lists) {
                Some(v) => v,
                None => continue,
            };
        let values: Vec<SourceRef> = value_cmds.iter().map(code_as_source).collect();
        let start_loc = value_cmds
            .first()
            .map(|c| c.borrow().location())
            .unwrap_or(start_location);

        let lists: Vec<Vec<SourceRef>> = foreach_lists
            .iter()
            .map(|list| {
                list.iter()
                    .map(|idx| this.bytecode.locals.get(*idx as usize).cloned().map(sr_dyn))
                    .collect::<Option<Vec<_>>>()
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        if lists.is_empty() {
            continue;
        }

        match start_opcode {
            Opcode::ForeachStart4 => {
                let step = match start_primary_next.and_then(|loc| this.graph.code_at_location(loc))
                {
                    Some(c) => c,
                    None => continue,
                };
                let (step_location, step_next) = match &*step.borrow() {
                    Code::Instruction(inst) if inst.opcode == Opcode::ForeachStep4 => {
                        (inst.location, inst.primary_next_location())
                    }
                    _ => continue,
                };
                let cond_jmp = match step_next.and_then(|loc| this.graph.code_at_location(loc)) {
                    Some(c) => c,
                    None => continue,
                };
                let (cond_target, cond_body_start) = match &*cond_jmp.borrow() {
                    Code::Instruction(inst)
                        if matches!(inst.opcode, Opcode::JumpFalse1 | Opcode::JumpFalse4) =>
                    {
                        (inst.jump_target(), inst.secondary_next_location())
                    }
                    _ => (None, None),
                };
                let uncond_jmp = match cond_target.and_then(|loc| this.graph.code_before(loc)) {
                    Some(c) => c,
                    None => continue,
                };
                let has_step_backedge = matches!(
                    &*uncond_jmp.borrow(),
                    Code::Instruction(inst)
                        if inst.is_jump_unconditional()
                            && inst.jump_target() == Some(step_location)
                );
                if !has_step_backedge {
                    continue;
                }

                let end_loc = match cond_target {
                    Some(v) => v,
                    None => continue,
                };
                if start_loc >= end_loc {
                    continue;
                }
                let code_range = start_loc..end_loc;
                let foreach_codes = this.graph.code_in_range(code_range.clone());

                let body_start = match cond_body_start {
                    Some(v) => v,
                    None => continue,
                };
                let body_range = body_start..uncond_jmp.borrow().location();
                this.process(body_range.clone());
                let body_codes = this.filter_codes(this.graph.code_in_range(body_range));

                if foreach_codes.is_empty() {
                    continue;
                }
                let cmd = cmd_foreach(foreach_codes, CmdBody::new(body_codes), lists, values);
                this.graph.replace_code(cmd);
            }
            Opcode::ForeachStart => {
                let inst_start_idx = match insts
                    .iter()
                    .position(|c| c.borrow().location() == start_location)
                {
                    Some(idx) => idx,
                    None => continue,
                };
                let mut depth = 0;
                let mut step = None;
                let mut end = None;
                for idx in (inst_start_idx + 1)..insts.len() {
                    let opcode = match &*insts[idx].borrow() {
                        Code::Instruction(inst) => inst.opcode,
                        _ => continue,
                    };
                    if opcode == Opcode::ForeachStart {
                        depth += 1;
                        continue;
                    }
                    if opcode == Opcode::ForeachEnd {
                        if depth == 0 {
                            end = Some(insts[idx].clone());
                            if idx > 0 {
                                step = Some(insts[idx - 1].clone());
                            }
                            break;
                        }
                        depth -= 1;
                    }
                }
                let step = match step {
                    Some(s) => s,
                    None => continue,
                };
                let end = match end {
                    Some(e) => e,
                    None => continue,
                };
                let step_opcode = match &*step.borrow() {
                    Code::Instruction(inst) => inst.opcode,
                    _ => continue,
                };
                if step_opcode != Opcode::ForeachStep {
                    continue;
                }

                let body_start = start_location + start_size;
                let body_end = step.borrow().location();
                if body_start >= body_end {
                    continue;
                }
                let body_range = body_start..body_end;
                this.process(body_range.clone());
                let body_codes = this.filter_codes(this.graph.code_in_range(body_range));

                let end_loc = end.borrow().location() + end.borrow().size();
                if start_loc >= end_loc {
                    continue;
                }
                let foreach_codes = this.graph.code_in_range(start_loc..end_loc);
                if foreach_codes.is_empty() {
                    continue;
                }
                let cmd = cmd_foreach(foreach_codes, CmdBody::new(body_codes), lists, values);
                this.graph.replace_code(cmd);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{
        AuxData, AuxDataType, ByteCodeInfo, CompiledLocal, ForeachInfo, Instruction, NewForeachInfo,
    };
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

    fn local(name: &str, frame_index: i32) -> CompiledLocal {
        CompiledLocal {
            name: name.to_string(),
            default_value: None,
            frame_index,
            flag_mask: 0,
        }
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

    fn bytecode_with_auxiliary(auxiliary: AuxData) -> ByteCode {
        ByteCode {
            header: None,
            info: ByteCodeInfo {
                num_commands: 0,
                num_src_bytes: 0,
                num_code_bytes: 0,
                num_lit_objects: 0,
                num_except_ranges: 0,
                num_aux_data_items: 1,
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
            auxiliaries: vec![auxiliary],
            proc_num_args: 0,
            proc_num_locals: 1,
            locals: vec![local("item", 0)],
        }
    }

    #[test]
    fn legacy_foreach_uses_instruction_index_after_reduced_prefix_commands() {
        let bytecode = bytecode_with_auxiliary(AuxData {
            kind: AuxDataType::Foreach,
            value: AuxDataValue::Foreach(ForeachInfo {
                first_value: 0,
                loop_counter: 1,
                lists: vec![vec![0]],
            }),
        });
        let mut decompiler = Decompiler::new(bytecode).expect("decompiler");
        decompiler.graph = CodeGraph::new(vec![
            instruction(0, Opcode::Push1, vec![0], Some(sr("prefix-a"))),
            instruction(2, Opcode::Push1, vec![0], Some(sr("prefix-b"))),
            instruction(4, Opcode::Push1, vec![0], Some(sr("values"))),
            instruction(6, Opcode::ForeachStart, vec![0], None),
            instruction(11, Opcode::Push1, vec![0], Some(sr("body"))),
            instruction(13, Opcode::ForeachStep, Vec::new(), None),
            instruction(14, Opcode::ForeachEnd, Vec::new(), None),
            instruction(15, Opcode::Done, Vec::new(), None),
        ]);
        replace_literal(&mut decompiler, 0, "prefix-a");
        replace_literal(&mut decompiler, 2, "prefix-b");
        replace_literal(&mut decompiler, 4, "values");

        run(&mut decompiler, 0..16);

        let recovered = decompiler
            .graph
            .code_at_location(4)
            .expect("recovered foreach");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("foreach must reduce to a command");
        };
        let CommandKind::Foreach {
            body,
            lists,
            values,
        } = &command.kind
        else {
            panic!("expected foreach command");
        };
        assert_eq!(lists.len(), 1);
        assert_eq!(values.len(), 1);
        assert!(!body.block.children.is_empty());
    }

    fn modern_foreach(backedge_operand: i32) -> Decompiler {
        let bytecode = bytecode_with_auxiliary(AuxData {
            kind: AuxDataType::NewForeach,
            value: AuxDataValue::NewForeach(NewForeachInfo {
                loop_counter: 1,
                lists: vec![vec![0]],
            }),
        });
        let mut decompiler = Decompiler::new(bytecode).expect("decompiler");
        decompiler.graph = CodeGraph::new(vec![
            instruction(0, Opcode::Push1, vec![0], Some(sr("values"))),
            instruction(2, Opcode::ForeachStart4, vec![0], None),
            instruction(7, Opcode::ForeachStep4, vec![0], None),
            instruction(12, Opcode::JumpFalse1, vec![6], None),
            instruction(14, Opcode::Push1, vec![0], Some(sr("body"))),
            instruction(16, Opcode::Jump1, vec![backedge_operand], None),
            instruction(18, Opcode::Done, Vec::new(), None),
        ]);
        replace_literal(&mut decompiler, 0, "values");
        decompiler
    }

    #[test]
    fn recovers_modern_foreach_with_valid_control_flow() {
        let mut decompiler = modern_foreach(-9);

        run(&mut decompiler, 0..19);

        let recovered = decompiler
            .graph
            .code_at_location(0)
            .expect("recovered foreach");
        let borrowed = recovered.borrow();
        assert!(matches!(
            &*borrowed,
            Code::Command(command) if matches!(command.kind, CommandKind::Foreach { .. })
        ));
    }

    #[test]
    fn rejects_modern_foreach_without_step_backedge() {
        let mut decompiler = modern_foreach(2);

        run(&mut decompiler, 0..19);

        assert!(matches!(
            &*decompiler
                .graph
                .code_at_location(2)
                .expect("foreach start")
                .borrow(),
            Code::Instruction(inst) if inst.opcode == Opcode::ForeachStart4
        ));
    }
}
