use super::super::*;

pub(crate) fn run(this: &mut Decompiler, range: Range<i32>) {
    let codes = this.graph.code_in_range(range.clone());
    for (idx, code) in codes.iter().enumerate() {
        let (dict_idx, aux_idx) = match &*code.borrow() {
            Code::Instruction(inst) if inst.opcode == Opcode::DictUpdateStart => {
                if inst.operands.len() < 2 {
                    continue;
                }
                (inst.operands[0] as usize, inst.operands[1] as usize)
            }
            _ => continue,
        };
        let dict_var = match this.bytecode.locals.get(dict_idx) {
            Some(v) => sr_dyn(v.clone()),
            None => continue,
        };
        let aux = match this.bytecode.auxiliaries.get(aux_idx) {
            Some(aux) => aux,
            None => continue,
        };
        let var_indices = match &aux.value {
            AuxDataValue::DictUpdate(info) => info.indices.as_slice(),
            _ => continue,
        };
        let (_list_start_idx, list_cmds) = match this.collect_args_skipping(&codes, idx, 1) {
            Some(v) => v,
            None => continue,
        };
        let list_cmd = list_cmds[0].clone();
        let mut keys: Vec<SourceRef> = Vec::new();
        if let Code::Command(cmd) = &*list_cmd.borrow()
            && let CommandKind::ProcCall { args } = &cmd.kind
            && !args.is_empty()
            && args[0].repr_eq_str("list")
        {
            keys.extend(args.iter().skip(1).cloned());
        }
        if keys.is_empty() {
            keys.push(code_as_source(&list_cmd));
        }
        if keys.len() != var_indices.len() {
            continue;
        }
        let Some(pairs) = keys
            .into_iter()
            .zip(var_indices.iter())
            .map(|(key, var_idx)| {
                this.bytecode
                    .locals
                    .get(usize::try_from(*var_idx).ok()?)
                    .cloned()
                    .map(sr_dyn)
                    .map(|var| (key, var))
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };

        let mut begin_catch = None;
        let mut end_catch = None;
        for code in codes.iter().skip(idx + 1) {
            if let Code::Instruction(inst) = &*code.borrow() {
                if inst.opcode == Opcode::BeginCatch4 {
                    begin_catch = Some(code.clone());
                    continue;
                }
                if inst.opcode == Opcode::EndCatch && begin_catch.is_some() {
                    end_catch = Some(code.clone());
                    break;
                }
            }
        }
        let begin_catch = match begin_catch {
            Some(c) => c,
            None => continue,
        };
        let end_catch = match end_catch {
            Some(c) => c,
            None => continue,
        };
        let body_start = begin_catch.borrow().location() + begin_catch.borrow().size();
        let body_end = end_catch.borrow().location();
        if body_start >= body_end {
            continue;
        }
        let body_range = body_start..body_end;
        this.process(body_range.clone());
        let body_codes = this.filter_codes(this.graph.code_in_range(body_range));

        let mut end_loc = None;
        for j in (idx + 1)..codes.len().saturating_sub(1) {
            let is_dict_update_end = match &*codes[j].borrow() {
                Code::Instruction(inst) => inst.opcode == Opcode::DictUpdateEnd,
                _ => continue,
            };
            if !is_dict_update_end {
                continue;
            }
            let next = match codes.get(j + 1) {
                Some(c) => c,
                None => continue,
            };
            let next_target = match &*next.borrow() {
                Code::Instruction(inst) if inst.is_jump_unconditional() => inst.jump_target(),
                _ => continue,
            };
            end_loc = next_target;
            break;
        }
        let end_loc = match end_loc {
            Some(v) => v,
            None => continue,
        };
        let start_loc = list_cmd.borrow().location();
        if start_loc >= end_loc {
            continue;
        }
        let cmd_codes = this.graph.code_in_range(start_loc..end_loc);
        let cmd = cmd_dict_update(cmd_codes, dict_var, pairs, CmdBody::new(body_codes));
        this.graph.replace_code(cmd);
        let check_range = start_loc..range.end;
        run(this, check_range);
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{
        AuxData, AuxDataType, ByteCodeInfo, CompiledLocal, DictUpdateInfo, Instruction,
    };
    use crate::inst_table::DEFAULT_INSTRUCTION_SET;

    fn local(name: &str, frame_index: i32) -> CompiledLocal {
        CompiledLocal {
            name: name.to_string(),
            default_value: None,
            frame_index,
            flag_mask: 0,
        }
    }

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

    fn decompiler_with_dict_update(var_indices: Vec<i32>) -> Decompiler {
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
                kind: AuxDataType::DictUpdate,
                value: AuxDataValue::DictUpdate(DictUpdateInfo {
                    indices: var_indices,
                }),
            }],
            proc_num_args: 0,
            proc_num_locals: 2,
            locals: vec![local("dictionary", 0), local("value", 1)],
        };
        let mut decompiler = Decompiler::new(bytecode).expect("decompiler");
        decompiler.graph = CodeGraph::new(vec![
            instruction(0, Opcode::Push1, vec![0], Some(sr("key"))),
            instruction(2, Opcode::DictUpdateStart, vec![0, 0], None),
            instruction(11, Opcode::BeginCatch4, vec![0], None),
            instruction(16, Opcode::Push1, vec![0], Some(sr("body-result"))),
            instruction(18, Opcode::EndCatch, Vec::new(), None),
            instruction(19, Opcode::DictUpdateEnd, vec![0, 0], None),
            instruction(28, Opcode::Jump4, vec![5], None),
            instruction(33, Opcode::Done, Vec::new(), None),
        ]);
        let list_child = decompiler.graph.code_at_location(0).expect("list push");
        decompiler
            .graph
            .replace_code(cmd_proc_call(vec![list_child], vec![sr("list"), sr("key")]));
        decompiler
    }

    #[test]
    fn recovers_dict_update_pairs_and_body() {
        let mut decompiler = decompiler_with_dict_update(vec![1]);
        run(&mut decompiler, 0..34);

        let recovered = decompiler
            .graph
            .code_at_location(0)
            .expect("recovered code");
        let borrowed = recovered.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("dict update should reduce to a command");
        };
        let CommandKind::DictUpdate {
            dict_var,
            pairs,
            body,
        } = &command.kind
        else {
            panic!("expected dict update command");
        };
        assert!(dict_var.repr_eq_str("dictionary"));
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].0.repr_eq_str("key"));
        assert!(pairs[0].1.repr_eq_str("value"));
        assert!(!body.block.children.is_empty());
    }

    #[test]
    fn rejects_negative_dict_update_local_indices() {
        let mut decompiler = decompiler_with_dict_update(vec![-1]);
        run(&mut decompiler, 0..34);

        assert!(matches!(
            &*decompiler
                .graph
                .code_at_location(2)
                .expect("unreduced start")
                .borrow(),
            Code::Instruction(inst) if inst.opcode == Opcode::DictUpdateStart
        ));
    }
}
