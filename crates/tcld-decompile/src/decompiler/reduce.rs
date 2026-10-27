use super::*;
use crate::decompiler::opcode_handlers::*;

impl Decompiler {
    pub(super) fn reduce_instructions(&mut self, codes: Vec<CodeRef>) -> Option<Vec<CodeRef>> {
        let mut replacements = Vec::new();
        let mut loc_to_idx = BTreeMap::new();
        for (i, c) in codes.iter().enumerate() {
            loc_to_idx.insert(c.borrow().location(), i);
        }
        for (idx, code) in codes.iter().enumerate() {
            let code_borrow = code.borrow();
            let inst = match &*code_borrow {
                Code::Instruction(inst) => inst,
                _ => continue,
            };
            if inst.opcode == Opcode::Over {
                let depth = match inst.operands.first() {
                    Some(v) if *v >= 0 => *v as usize,
                    _ => continue,
                };
                let target = match self.command_at_depth(&codes, idx, depth) {
                    Some(v) => v,
                    None => continue,
                };
                let kind = match &*target.borrow() {
                    Code::Command(cmd) => cmd.kind.clone(),
                    _ => continue,
                };
                let replacement = cmd_literal(vec![code.clone()], sr(""));
                if let Code::Command(cmd) = &mut *replacement.borrow_mut() {
                    cmd.kind = kind;
                }
                replacements.push(replacement);
                continue;
            }
            if inst.opcode == Opcode::ExpandStktop {
                let Some(target) = self.command_at_depth(&codes, idx, 0) else {
                    continue;
                };
                let target_loc = target.borrow().location();
                let Some(&arg_idx) = loc_to_idx.get(&target_loc) else {
                    continue;
                };
                let children = codes[arg_idx..=idx].to_vec();
                let expanded = cmd_expanded_arg(children, code_as_source(&target));
                replacements.push(expanded);
                continue;
            }
            if inst.opcode == Opcode::InvokeExpanded {
                // Unlike InvokeStk*, invokeExpanded has no encoded argc. Tcl
                // records the dynamic argument boundary with expandStart.
                let Some(start_idx) = codes[..idx].iter().rposition(|candidate| {
                    matches!(
                        &*candidate.borrow(),
                        Code::Instruction(candidate)
                            if candidate.opcode == Opcode::ExpandStart
                    )
                }) else {
                    continue;
                };
                let mut args = Vec::new();
                let mut valid = true;
                for candidate in &codes[start_idx + 1..idx] {
                    let borrowed = candidate.borrow();
                    match &*borrowed {
                        Code::Command(_) => args.push(candidate.clone()),
                        Code::Instruction(candidate) if is_ir_ignorable_inst(candidate.opcode) => {}
                        _ => {
                            valid = false;
                            break;
                        }
                    }
                }
                if !valid || args.is_empty() {
                    continue;
                }
                let children = codes[start_idx..=idx].to_vec();
                if let Some(replacement) = handle_invoke_opcodes(inst, &args, children) {
                    replacements.push(replacement);
                }
                continue;
            }
            let argc = inst.argc();
            if argc < 0 {
                continue;
            }
            let argc = match inst.opcode {
                // These encoded counts exclude one implicit stack value: the
                // dictionary for dict operations or the replacement
                // implementation for InvokeReplace.
                Opcode::DictGet | Opcode::DictExists | Opcode::DictSet | Opcode::InvokeReplace => {
                    match argc.checked_add(1) {
                        Some(argc) => argc,
                        None => continue,
                    }
                }
                // dictGetDef's encoded count is the path length. The
                // dictionary and default value are additional stack values.
                Opcode::DictGetDef => match argc.checked_add(2) {
                    Some(argc) => argc,
                    None => continue,
                },
                _ => argc,
            };
            let argc = match usize::try_from(argc) {
                Ok(argc) => argc,
                Err(_) => continue,
            };
            if idx < argc {
                continue;
            }
            let use_skipping = matches!(
                inst.opcode,
                Opcode::StrConcat1
                    | Opcode::ExprStk
                    | Opcode::StoreScalarStk
                    | Opcode::StoreStk
                    | Opcode::InvokeReplace
            );
            let (args, children) = if use_skipping {
                let (start_idx, args) = match self.collect_args_skipping(&codes, idx, argc) {
                    Some(v) => v,
                    None => continue,
                };
                let children = codes[start_idx..=idx].to_vec();
                (args, children)
            } else {
                let (start_idx, args) = match self.collect_args_ignoring_noops(&codes, idx, argc) {
                    Some(v) => v,
                    None => continue,
                };
                let children = codes[start_idx..=idx].to_vec();
                (args, children)
            };

            let replacement: Option<CodeRef> = handle_load_opcodes(inst, &args, children.clone())
                .or_else(|| handle_store_opcodes(self, inst, &args, children.clone()))
                .or_else(|| handle_arithmetic_opcodes(inst, &args, children.clone()))
                .or_else(|| handle_string_opcodes(inst, &args, children.clone()))
                .or_else(|| handle_list_opcodes(inst, &args, children.clone()))
                .or_else(|| handle_dict_opcodes(self, inst, &args, children.clone()))
                .or_else(|| handle_incr_opcodes(self, inst, &args, children.clone()))
                .or_else(|| handle_invoke_opcodes(inst, &args, children.clone()))
                .or_else(|| handle_unset_opcodes(self, inst, &args, children.clone()))
                .or_else(|| handle_exist_opcodes(inst, &args, children.clone()))
                .or_else(|| handle_append_opcodes(inst, &args, children.clone()))
                .or_else(|| handle_misc_opcodes(self, inst, &args, children.clone()));

            if let Some(r) = replacement {
                replacements.push(r);
            }
        }

        if replacements.is_empty() {
            None
        } else {
            Some(replacements)
        }
    }

    /// Render each inline-traced command region (see [`InlineTraceCapture`])
    /// as one command built from its captured words. Regions inside another
    /// region's implementation are absorbed by that outer command, so only
    /// the rest are collapsed, innermost first so a capture nested in an
    /// outer command's words is a finished word by the time the outer one is.
    pub(super) fn collapse_inline_trace_captures(&mut self) {
        let captures = std::mem::take(&mut self.trace_captures);
        let mut pending: Vec<InlineTraceCapture> = captures
            .iter()
            .copied()
            .filter(|capture| {
                !captures.iter().any(|outer| {
                    outer.trace_pc < capture.anchor_pc && capture.trace_end_pc < outer.trace_end_pc
                })
            })
            .collect();
        pending.sort_by_key(|capture| capture.trace_end_pc - capture.anchor_pc);
        for capture in pending {
            self.collapse_inline_trace_capture(capture);
        }
    }

    /// Collapse one region, or leave it untouched when its shape doesn't
    /// match the compiler's layout (words, `list argv_count`, then `Nop`s).
    fn collapse_inline_trace_capture(&mut self, capture: InlineTraceCapture) -> bool {
        let words = self
            .graph
            .code_in_range(capture.anchor_pc..capture.trace_pc);
        let Some(list) = words.last().cloned() else {
            return false;
        };
        let list_pc = {
            let borrowed = list.borrow();
            let Code::Instruction(inst) = &*borrowed else {
                return false;
            };
            if inst.opcode != Opcode::List
                || usize::try_from(inst.argc()).ok() != Some(capture.argv_count)
            {
                return false;
            }
            inst.location
        };

        self.process_instructions(capture.anchor_pc..list_pc);
        let codes = self.graph.code_in_range(capture.anchor_pc..list_pc);
        let Some((start_idx, args)) =
            self.collect_args_ignoring_noops(&codes, codes.len(), capture.argv_count)
        else {
            return false;
        };
        // Anything else left on the stack ahead of the words means the region
        // isn't the plain capture layout; don't guess.
        let only_noops_before = codes[..start_idx].iter().all(|code| {
            matches!(&*code.borrow(), Code::Instruction(inst) if is_ir_ignorable_inst(inst.opcode))
        });
        if !only_noops_before {
            return false;
        }

        let children = self
            .graph
            .code_in_range(capture.anchor_pc..capture.trace_end_pc + 1);
        let vals: Vec<SourceRef> = args.iter().map(code_as_source).collect();
        self.graph.replace_code(cmd_proc_call(children, vals));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{ByteCodeInfo, Instruction};
    use crate::code::code_ref;
    use crate::inst_table::{DEFAULT_INSTRUCTION_SET, TCL_9_INSTRUCTION_SET};

    fn empty_bytecode() -> ByteCode {
        ByteCode {
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
        }
    }

    fn instruction(location: i32, opcode: Opcode, operands: Vec<i32>) -> CodeRef {
        let desc = *[DEFAULT_INSTRUCTION_SET, TCL_9_INSTRUCTION_SET]
            .into_iter()
            .find_map(|set| {
                set.instruction_table
                    .iter()
                    .find(|desc| set.opcode_from_i32(desc.opcode) == Some(opcode))
            })
            .expect("instruction descriptor");
        code_ref(Code::Instruction(Instruction::new(
            location, opcode, desc, operands, None,
        )))
    }

    fn literal(location: i32, value: &'static str) -> CodeRef {
        let child = instruction(location, Opcode::Push1, vec![0]);
        cmd_literal(vec![child], sr(value))
    }

    #[test]
    fn variable_arity_adjustment_rejects_overflow() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        for opcode in [
            Opcode::DictGet,
            Opcode::DictExists,
            Opcode::DictSet,
            Opcode::InvokeReplace,
            Opcode::DictGetDef,
        ] {
            let codes = vec![instruction(0, opcode, vec![i32::MAX, 0])];
            assert!(
                decompiler.reduce_instructions(codes).is_none(),
                "{opcode:?} must fail closed for an overflowing argument count"
            );
        }
    }

    #[test]
    fn dict_get_default_consumes_dictionary_path_and_default() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        let replacements = decompiler
            .reduce_instructions(vec![
                literal(0, "dictionary"),
                literal(2, "outer"),
                literal(4, "inner"),
                literal(6, "fallback"),
                instruction(8, Opcode::DictGetDef, vec![2]),
            ])
            .expect("dictGetDef reduction");
        assert_eq!(replacements.len(), 1);
        let borrowed = replacements[0].borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("dictGetDef command");
        };
        let CommandKind::ProcCall { args } = &command.kind else {
            panic!("dictGetDef proc call");
        };
        assert_eq!(
            args.iter()
                .map(|arg| arg.repr_cow().into_owned())
                .collect::<Vec<_>>(),
            [
                "dict getwithdefault",
                "dictionary",
                "outer",
                "inner",
                "fallback"
            ]
        );
    }

    #[test]
    fn malformed_stack_transform_operands_fail_closed() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        for codes in [
            vec![instruction(0, Opcode::Over, Vec::new())],
            vec![instruction(0, Opcode::Over, vec![-1])],
            vec![instruction(0, Opcode::Over, vec![0])],
            vec![instruction(0, Opcode::ExpandStktop, Vec::new())],
            vec![instruction(0, Opcode::DictGet, vec![-1])],
        ] {
            assert!(decompiler.reduce_instructions(codes).is_none());
        }
    }

    #[test]
    fn skipping_reduction_preserves_separators_between_stack_arguments() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        let separator = instruction(2, Opcode::Nop, Vec::new());
        let replacements = decompiler
            .reduce_instructions(vec![
                literal(0, "a"),
                separator.clone(),
                literal(3, "b"),
                instruction(5, Opcode::StrConcat1, vec![2]),
            ])
            .expect("concat reduction");

        assert_eq!(replacements.len(), 1);
        let borrowed = replacements[0].borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("concat must reduce to a command");
        };
        assert!(matches!(command.kind, CommandKind::StrCat { .. }));
        assert!(
            command
                .base
                .block
                .children
                .values()
                .any(|child| child.ptr_eq(&separator)),
            "reduction separators must remain attached for source mapping"
        );
    }

    #[test]
    fn invoke_reduces_across_compiler_step_boundaries() {
        // tcld-compiled source: `nop` boundaries sit before the invoke and
        // before the following pop (`puts hi` as a discarded statement).
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        let boundary = instruction(4, Opcode::Nop, Vec::new());
        let replacements = decompiler
            .reduce_instructions(vec![
                literal(0, "puts"),
                literal(2, "hi"),
                boundary.clone(),
                instruction(5, Opcode::InvokeStk1, vec![2]),
            ])
            .expect("invoke reduction");

        assert_eq!(replacements.len(), 1);
        let borrowed = replacements[0].borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("invokeStk1 must reduce to a command");
        };
        let CommandKind::ProcCall { args } = &command.kind else {
            panic!("invokeStk1 proc call");
        };
        assert_eq!(
            args.iter()
                .map(|arg| arg.repr_cow().into_owned())
                .collect::<Vec<_>>(),
            ["puts", "hi"]
        );
        assert!(
            command
                .base
                .block
                .children
                .values()
                .any(|child| child.ptr_eq(&boundary)),
            "step boundaries must remain attached for source mapping"
        );
    }

    #[test]
    fn strict_reduction_still_stops_at_control_flow() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        assert!(
            decompiler
                .reduce_instructions(vec![
                    literal(0, "puts"),
                    instruction(2, Opcode::Jump1, vec![2]),
                    literal(4, "hi"),
                    instruction(6, Opcode::InvokeStk1, vec![2]),
                ])
                .is_none()
        );
    }

    #[test]
    fn skipping_reduction_rejects_raw_stack_arguments() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        assert!(
            decompiler
                .reduce_instructions(vec![
                    instruction(0, Opcode::Push1, vec![0]),
                    instruction(2, Opcode::ExprStk, Vec::new()),
                ])
                .is_none()
        );
    }

    #[test]
    fn invoke_expanded_recovers_dynamic_arguments_from_expand_marker() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        let marker = instruction(0, Opcode::ExpandStart, Vec::new());
        let separator = instruction(2, Opcode::Nop, Vec::new());
        let replacements = decompiler
            .reduce_instructions(vec![
                marker.clone(),
                literal(1, "command"),
                separator.clone(),
                literal(3, "argument"),
                instruction(5, Opcode::InvokeExpanded, Vec::new()),
            ])
            .expect("invokeExpanded reduction");

        assert_eq!(replacements.len(), 1);
        let borrowed = replacements[0].borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("invokeExpanded must reduce to a command");
        };
        let CommandKind::ProcCall { args } = &command.kind else {
            panic!("invokeExpanded proc call");
        };
        assert_eq!(
            args.iter()
                .map(|arg| arg.repr_cow().into_owned())
                .collect::<Vec<_>>(),
            ["command", "argument"]
        );
        assert!(
            command
                .base
                .block
                .children
                .values()
                .any(|child| child.ptr_eq(&marker))
        );
        assert!(
            command
                .base
                .block
                .children
                .values()
                .any(|child| child.ptr_eq(&separator))
        );
    }

    #[test]
    fn invoke_expanded_without_marker_fails_closed() {
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        assert!(
            decompiler
                .reduce_instructions(vec![
                    literal(0, "command"),
                    instruction(2, Opcode::InvokeExpanded, Vec::new()),
                ])
                .is_none()
        );
    }
}
