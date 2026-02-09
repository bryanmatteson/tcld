use super::super::*;

pub(crate) fn run(this: &mut Decompiler, range: Range<i32>) {
    let codes = this.graph.code_in_range(range.clone());
    for (idx, code) in codes.iter().enumerate() {
        let (opcode, aux_idx, jump_loc, jump_size) = match &*code.borrow() {
            Code::Instruction(inst) => (
                inst.opcode,
                match inst.operands.first() {
                    Some(v) => *v as usize,
                    None => continue,
                },
                inst.location,
                inst.size(),
            ),
            _ => continue,
        };
        if !is_switch_jump_opcode(opcode) {
            continue;
        }
        let aux = match this.bytecode.auxiliaries.get(aux_idx) {
            Some(aux) => aux,
            None => continue,
        };
        let table = match &aux.value {
            AuxDataValue::JumpTable(info) => info,
            _ => continue,
        };

        let (_value_start_idx, value_cmds) = match this.collect_args_skipping(&codes, idx, 1) {
            Some(v) => v,
            None => continue,
        };
        let value_cmd = value_cmds[0].clone();
        let mut nocase = false;
        let mut integer = false;
        let mut value = code_as_source(&value_cmd);
        if let Code::Command(cmd) = &*value_cmd.borrow()
            && let CommandKind::ProcCall { args } = &cmd.kind
            && args.len() == 2
            && args[0].repr_eq_str("string tolower")
        {
            nocase = true;
            value = args[1].clone();
        }

        let jump_next = this.graph.code_at_location(jump_loc + jump_size);
        let jump_next = match jump_next {
            Some(c) => c,
            None => continue,
        };
        let (jump_default_loc, default_loc) = match &*jump_next.borrow() {
            Code::Instruction(inst) if inst.is_jump_unconditional() => match inst.jump_target() {
                Some(target) => (inst.location, target),
                None => continue,
            },
            _ => continue,
        };

        let mut arms_by_target: BTreeMap<i32, Vec<SourceRef>> = BTreeMap::new();
        let mut all_numeric = true;
        for (pat, offset) in table.dict.iter() {
            let target = jump_loc + *offset;
            if pat.parse::<i64>().is_err() {
                all_numeric = false;
            }
            arms_by_target
                .entry(target)
                .or_default()
                .push(sr(pat.clone()));
        }
        if !integer && !nocase && all_numeric {
            integer = true;
        }
        for pats in arms_by_target.values_mut() {
            if integer {
                pats.sort_by(|a, b| {
                    let a_num = a.repr_parse_i64().unwrap_or(0);
                    let b_num = b.repr_parse_i64().unwrap_or(0);
                    a_num.cmp(&b_num).then_with(|| a.repr_cmp(b))
                });
            } else {
                pats.sort_by(|a, b| a.repr_cmp(b));
            }
        }
        if integer {
            nocase = false;
        }

        let mut end_counts: BTreeMap<i32, usize> = BTreeMap::new();
        for code in codes.iter() {
            let (is_unconditional, inst_loc, target) = match &*code.borrow() {
                Code::Instruction(inst) => (
                    inst.is_jump_unconditional(),
                    inst.location,
                    inst.jump_target(),
                ),
                _ => continue,
            };
            if !is_unconditional || inst_loc <= jump_loc {
                continue;
            }
            let target = match target {
                Some(v) => v,
                None => continue,
            };
            if target <= inst_loc {
                continue;
            }
            if inst_loc == jump_default_loc {
                continue;
            }
            *end_counts.entry(target).or_insert(0) += 1;
        }
        let max_end_count = end_counts.values().copied().max().unwrap_or(0);
        let mut likely_ends = end_counts
            .iter()
            .filter(|(_, count)| **count == max_end_count)
            .map(|(location, _)| *location);
        let end_loc = match (likely_ends.next(), likely_ends.next()) {
            (Some(location), None) => location,
            (None, None) => range.end,
            _ => continue,
        };
        if end_loc <= jump_loc || end_loc > range.end || default_loc > end_loc {
            continue;
        }

        let mut targets: Vec<i32> = arms_by_target.keys().cloned().collect();
        if !targets.contains(&default_loc) {
            targets.push(default_loc);
        }
        targets.sort();

        let mut arms: Vec<(Vec<SourceRef>, CmdBody)> = Vec::new();
        let mut default_body: Option<CmdBody> = None;

        for (tidx, target) in targets.iter().enumerate() {
            let next_target = targets.get(tidx + 1).cloned().unwrap_or(end_loc);
            let mut body_end = next_target.min(end_loc);
            for code in codes.iter() {
                let (inst_loc, is_unconditional, inst_target) = match &*code.borrow() {
                    Code::Instruction(inst) => (
                        inst.location,
                        inst.is_jump_unconditional(),
                        inst.jump_target(),
                    ),
                    _ => continue,
                };
                if inst_loc < *target || inst_loc >= body_end {
                    continue;
                }
                if is_unconditional && inst_target == Some(end_loc) {
                    body_end = inst_loc;
                    break;
                }
            }
            if *target >= body_end {
                continue;
            }
            let body_range = *target..body_end;
            this.process(body_range.clone());
            let body_codes = this.filter_codes(this.graph.code_in_range(body_range));
            let body = CmdBody::new(body_codes);

            if *target == default_loc && !arms_by_target.contains_key(target) {
                let body_rep = body.source_repr();
                if !body_rep.is_empty() && body_rep != "{}" {
                    default_body = Some(body);
                }
                continue;
            }

            if let Some(patterns) = arms_by_target.get(target) {
                arms.push((patterns.clone(), body));
            }
        }

        let start_loc = value_cmd.borrow().location();
        if start_loc >= end_loc {
            continue;
        }
        let cmd_codes = this.graph.code_in_range(start_loc..end_loc);
        let cmd = cmd_switch(cmd_codes, value, arms, default_body, nocase, integer);
        this.graph.replace_code(cmd);
        let check_range = start_loc..range.end;
        run(this, check_range);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{AuxData, AuxDataType, ByteCodeInfo, Instruction, JumptableInfo};
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

    fn switch_decompiler(arm_end_targets: [i32; 3]) -> Decompiler {
        let table = BTreeMap::from([
            ("10".to_string(), 10),
            ("2".to_string(), 10),
            ("3".to_string(), 17),
        ]);
        let bytecode = ByteCode {
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
            auxiliaries: vec![AuxData {
                kind: AuxDataType::JumpTable,
                value: AuxDataValue::JumpTable(JumptableInfo { dict: table }),
            }],
            proc_num_args: 0,
            proc_num_locals: 0,
            locals: Vec::new(),
        };
        let mut decompiler = Decompiler::new(bytecode).expect("decompiler");
        decompiler.graph = CodeGraph::new(vec![
            instruction(0, Opcode::Push1, vec![0], Some(sr("selector"))),
            instruction(2, Opcode::JumpTable, vec![0], None),
            instruction(7, Opcode::Jump4, vec![19], None),
            instruction(12, Opcode::Push1, vec![0], Some(sr("first"))),
            instruction(14, Opcode::Jump4, vec![arm_end_targets[0] - 14], None),
            instruction(19, Opcode::Push1, vec![0], Some(sr("second"))),
            instruction(21, Opcode::Jump4, vec![arm_end_targets[1] - 21], None),
            instruction(26, Opcode::Push1, vec![0], Some(sr("default"))),
            instruction(28, Opcode::Jump4, vec![arm_end_targets[2] - 28], None),
            instruction(33, Opcode::Done, Vec::new(), None),
            instruction(34, Opcode::Done, Vec::new(), None),
            instruction(35, Opcode::Done, Vec::new(), None),
        ]);
        let selector = decompiler.graph.code_at_location(0).expect("selector");
        decompiler
            .graph
            .replace_code(cmd_literal(vec![selector], sr("selector")));
        decompiler
    }

    #[test]
    fn recovers_numeric_switch_with_shared_patterns_and_default() {
        let mut decompiler = switch_decompiler([33, 33, 33]);
        run(&mut decompiler, 0..36);

        let recovered = decompiler.graph.code_at_location(0).expect("switch");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("switch must reduce to a command");
        };
        let CommandKind::Switch {
            value,
            arms,
            default,
            nocase,
            integer,
        } = &command.kind
        else {
            panic!("expected switch command");
        };
        assert!(value.repr_eq_str("selector"));
        assert!(*integer);
        assert!(!*nocase);
        assert_eq!(arms.len(), 2);
        assert_eq!(
            arms[0]
                .0
                .iter()
                .map(|pattern| pattern.repr_cow().into_owned())
                .collect::<Vec<_>>(),
            ["2", "10"]
        );
        assert!(default.is_some());
    }

    #[test]
    fn rejects_switch_with_ambiguous_end_targets() {
        let mut decompiler = switch_decompiler([33, 34, 35]);
        run(&mut decompiler, 0..36);

        assert!(matches!(
            &*decompiler
                .graph
                .code_at_location(2)
                .expect("unreduced jump table")
                .borrow(),
            Code::Instruction(inst) if inst.opcode == Opcode::JumpTable
        ));
    }

    #[test]
    fn recovers_nocase_switch_without_numeric_inference() {
        let mut decompiler = switch_decompiler([33, 33, 33]);
        let AuxDataValue::JumpTable(table) = &mut decompiler.bytecode.auxiliaries[0].value else {
            panic!("jump table auxiliary");
        };
        table.dict = BTreeMap::from([("Zulu".to_string(), 10), ("alpha".to_string(), 17)]);
        let selector = decompiler.graph.code_at_location(0).expect("selector");
        decompiler.graph.replace_code(cmd_proc_call(
            vec![selector],
            vec![sr("string tolower"), sr("selector")],
        ));

        run(&mut decompiler, 0..36);

        let recovered = decompiler.graph.code_at_location(0).expect("switch");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("switch must reduce to a command");
        };
        let CommandKind::Switch {
            value,
            nocase,
            integer,
            ..
        } = &command.kind
        else {
            panic!("expected switch command");
        };
        assert!(value.repr_eq_str("selector"));
        assert!(*nocase);
        assert!(!*integer);
    }
}
