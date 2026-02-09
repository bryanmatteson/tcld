//! Grouped opcode handlers for the decompiler.
//!
//! This module organizes opcode handlers into logical categories, making the
//! large match statement in `reduce_instructions()` more maintainable. Each
//! handler function takes an instruction and its arguments, and returns the
//! decompiled command or None if the opcode cannot be handled.

use super::*;

/// Handle load opcodes (LoadScalar, LoadArray, LoadStk).
pub(super) fn handle_load_opcodes(
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::LoadArray1 | Opcode::LoadArray4 => {
            let mut vals: Vec<SourceRef> = vec![inst.value.clone()?];
            vals.push(code_as_source(&args[0]));
            Some(cmd_array_ref(children, vals))
        }
        Opcode::LoadArrayStk => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_source).collect();
            Some(cmd_array_ref(children, vals))
        }
        Opcode::LoadScalar1 | Opcode::LoadScalar4 => {
            Some(cmd_variable(children, inst.value.clone()?))
        }
        Opcode::LoadScalarStk => Some(cmd_variable(children, code_as_source(&args[0]))),
        Opcode::LoadStk => Some(cmd_variable(children, code_as_source(&args[0]))),
        _ => None,
    }
}

/// Handle store opcodes (StoreScalar, StoreArray, StoreStk).
pub(super) fn handle_store_opcodes(
    decompiler: &Decompiler,
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::StoreScalar1 | Opcode::StoreScalar4 => {
            let store_var = inst.value.clone()?;
            if let Some(lset_var) = decompiler.lset_var_from_value(&args[0]) {
                if lset_var.repr_eq(&store_var) {
                    if let Code::Command(cmd) = &mut *args[0].borrow_mut() {
                        cmd.merge_slice(&children);
                    }
                    Some(args[0].clone())
                } else {
                    Some(cmd_set(
                        children,
                        store_var.clone(),
                        code_as_source(&args[0]),
                    ))
                }
            } else {
                Some(cmd_set(
                    children,
                    store_var.clone(),
                    code_as_source(&args[0]),
                ))
            }
        }
        Opcode::StoreScalarStk | Opcode::StoreStk => {
            let store_var = code_as_source(&args[0]);
            if let Some(lset_var) = decompiler.lset_var_from_value(&args[1]) {
                if lset_var.repr_eq(&store_var) {
                    if let Code::Command(cmd) = &mut *args[1].borrow_mut() {
                        cmd.merge_slice(&children);
                    }
                    Some(args[1].clone())
                } else {
                    Some(cmd_set(
                        children,
                        store_var.clone(),
                        code_as_source(&args[1]),
                    ))
                }
            } else {
                Some(cmd_set(
                    children,
                    store_var.clone(),
                    code_as_source(&args[1]),
                ))
            }
        }
        Opcode::StoreArray1 | Opcode::StoreArray4 => {
            let elem = sr_dyn(ArrayElem::new(vec![
                inst.value.clone()?,
                code_as_source(&args[0]),
            ]));
            if let Some(lset_var) = decompiler.lset_var_from_value(&args[1]) {
                if lset_var.repr_eq(&elem) {
                    if let Code::Command(cmd) = &mut *args[1].borrow_mut() {
                        cmd.merge_slice(&children);
                    }
                    Some(args[1].clone())
                } else {
                    Some(cmd_set(children, elem.clone(), code_as_source(&args[1])))
                }
            } else {
                Some(cmd_set(children, elem.clone(), code_as_source(&args[1])))
            }
        }
        Opcode::StoreArrayStk => {
            let elem = sr_dyn(ArrayElem::new(vec![
                code_as_source(&args[0]),
                code_as_source(&args[1]),
            ]));
            if let Some(lset_var) = decompiler.lset_var_from_value(&args[2]) {
                if lset_var.repr_eq(&elem) {
                    if let Code::Command(cmd) = &mut *args[2].borrow_mut() {
                        cmd.merge_slice(&children);
                    }
                    Some(args[2].clone())
                } else {
                    Some(cmd_set(children, elem.clone(), code_as_source(&args[2])))
                }
            } else {
                Some(cmd_set(children, elem.clone(), code_as_source(&args[2])))
            }
        }
        Opcode::ConstImm => {
            let value = args.first()?;
            let var = inst
                .value
                .clone()
                .or_else(|| decompiler.lvt_value(*inst.operands.first()?))?;
            Some(cmd_proc_call(
                children,
                vec![sr("const"), var, code_as_source(value)],
            ))
        }
        Opcode::ConstStk => {
            let [var, value] = args else {
                return None;
            };
            Some(cmd_proc_call(
                children,
                vec![sr("const"), code_as_source(var), code_as_source(value)],
            ))
        }
        _ => None,
    }
}

/// Handle arithmetic opcodes (Neq, Gt, Lt, Add, Sub, Mult, etc.).
pub(super) fn handle_arithmetic_opcodes(
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::Neq
        | Opcode::Gt
        | Opcode::Lt
        | Opcode::Ge
        | Opcode::Eq
        | Opcode::Add
        | Opcode::Not
        | Opcode::Le
        | Opcode::Land
        | Opcode::Lor
        | Opcode::Bitxor
        | Opcode::Bitor
        | Opcode::Bitnot
        | Opcode::Bitand
        | Opcode::Lshift
        | Opcode::Rshift
        | Opcode::Uminus
        | Opcode::Uplus
        | Opcode::Sub
        | Opcode::Mult
        | Opcode::Div
        | Opcode::Mod
        | Opcode::Expon => {
            let op = expression_map(inst.opcode).unwrap();
            let vals: Vec<SourceRef> = args.iter().map(code_as_expr).collect();
            Some(cmd_op(children, op, vals))
        }
        _ => None,
    }
}

/// Handle string opcodes (StrConcat, StrIndex, StrLen, StrEq, etc.).
pub(super) fn handle_string_opcodes(
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::StrConcat1 => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_source).collect();
            Some(cmd_strcat(children, vals))
        }
        Opcode::StrIndex => {
            let mut vals = vec![sr("string index")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrLen => {
            let mut vals = vec![sr("string length")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrEq => {
            let mut vals = vec![sr("string equal")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrNeq => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_expr).collect();
            Some(cmd_op(children, "ne", vals))
        }
        Opcode::StrLt | Opcode::StrGt | Opcode::StrLe | Opcode::StrGe => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_expr).collect();
            Some(cmd_op(children, expression_map(inst.opcode)?, vals))
        }
        Opcode::StrCmp => {
            let mut vals = vec![sr("string compare")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrLower => {
            let mut vals = vec![sr("string tolower")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrMatch => {
            let mut vals = vec![sr("string match")];
            if inst.operands.first().cloned().unwrap_or(0) != 0 {
                vals.push(sr("-nocase"));
            }
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::Regexp => {
            let mut vals = vec![sr("regexp")];
            if inst.operands.first().cloned().unwrap_or(0) != 0 {
                vals.push(sr("-nocase"));
            }
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrUpper => {
            let mut vals = vec![sr("string toupper")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrTitle => {
            let mut vals = vec![sr("string totitle")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrTrim => {
            let mut vals = vec![sr("string trim")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrTrimLeft => {
            let mut vals = vec![sr("string trimleft")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrTrimRight => {
            let mut vals = vec![sr("string trimright")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrMap => {
            let mut vals = vec![sr("string map")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrFind => {
            let mut vals = vec![sr("string first")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrFindLast => {
            let mut vals = vec![sr("string last")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrRange => {
            let mut vals = vec![sr("string range")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrRangeImm => {
            let vals = vec![
                sr("string range"),
                code_as_source(&args[0]),
                idx_to_source(inst.operands.first().cloned().unwrap_or(0)),
                idx_to_source(inst.operands.get(1).cloned().unwrap_or(0)),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrReplace => {
            let mut vals = vec![sr("string replace")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::StrClass => {
            let vals = vec![
                sr("string is"),
                inst.value.clone()?,
                code_as_source(&args[0]),
            ];
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle list opcodes (List, ListLength, ListIndex, Lappend, etc.).
pub(super) fn handle_list_opcodes(
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::List => {
            let mut vals = vec![sr("list")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ListLength => {
            let mut vals = vec![sr("llength")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ListIndex => {
            let mut vals = vec![sr("lindex")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ListIndexMulti => {
            let mut vals = vec![sr("lindex")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ListIndexImm => {
            let vals = vec![
                sr("lindex"),
                code_as_source(&args[0]),
                idx_to_source(inst.operands.first().cloned().unwrap_or(0)),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ListRangeImm => {
            let vals = vec![
                sr("lrange"),
                code_as_source(&args[0]),
                idx_to_source(inst.operands.first().cloned().unwrap_or(0)),
                idx_to_source(inst.operands.get(1).cloned().unwrap_or(0)),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ListIn => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_expr).collect();
            Some(cmd_op(children, "in", vals))
        }
        Opcode::ListNotIn => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_expr).collect();
            Some(cmd_op(children, "ni", vals))
        }
        Opcode::LappendScalar1 | Opcode::LappendScalar4 => {
            let vals = vec![sr("lappend"), inst.value.clone()?, code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::LappendArray1 | Opcode::LappendArray4 => {
            if args.len() < 2 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    inst.value.clone()?,
                    code_as_source(&args[0]),
                ]));
                let vals = vec![sr("lappend"), elem, code_as_source(&args[1])];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::LappendArrayStk => {
            if args.len() < 3 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    code_as_source(&args[0]),
                    code_as_source(&args[1]),
                ]));
                let vals = vec![sr("lappend"), elem, code_as_source(&args[2])];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::LappendStk => {
            let mut vals = vec![sr("lappend")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::LappendList => {
            let vals = vec![
                sr("lappend"),
                inst.value.clone()?,
                expanded_arg(code_as_source(&args[0])),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::LappendListArray => {
            if args.len() < 2 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    inst.value.clone()?,
                    code_as_source(&args[0]),
                ]));
                let vals = vec![sr("lappend"), elem, expanded_arg(code_as_source(&args[1]))];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::LappendListArrayStk => {
            if args.len() < 3 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    code_as_source(&args[0]),
                    code_as_source(&args[1]),
                ]));
                let vals = vec![sr("lappend"), elem, expanded_arg(code_as_source(&args[2]))];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::LappendListStk => {
            let mut vals = vec![sr("lappend"), code_as_source(&args[0])];
            vals.push(expanded_arg(code_as_source(&args[1])));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::Lreplace4 => {
            let command = match inst.operands.get(1).copied()? {
                // Tcl 9 emits END_IS_LAST for lreplace and SINGLE_INDEX for
                // linsert. Other flag combinations have VM semantics that
                // cannot be represented faithfully by either surface command.
                1 => "lreplace",
                2 => "linsert",
                _ => return None,
            };
            let mut vals = vec![sr(command)];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle dict opcodes (DictGet, DictSet, DictUnset, DictExists, etc.).
pub(super) fn handle_dict_opcodes(
    decompiler: &Decompiler,
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::DictGet => {
            let mut vals = vec![sr("dict get")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::DictGetDef => {
            let mut vals = vec![sr("dict getwithdefault")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::DictSet => {
            let var = inst
                .value
                .clone()
                .or_else(|| decompiler.lvt_value(*inst.operands.get(1)?))?;
            let mut vals = vec![sr("dict set"), var];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::DictUnset => {
            let var = inst
                .value
                .clone()
                .or_else(|| decompiler.lvt_value(*inst.operands.get(1)?))?;
            let mut vals = vec![sr("dict unset"), var];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::DictIncrImm => {
            let var = inst
                .value
                .clone()
                .or_else(|| decompiler.lvt_value(*inst.operands.get(1)?))?;
            let mut vals = vec![sr("dict incr"), var, code_as_source(&args[0])];
            let amount = inst.operands.first().cloned().unwrap_or(1);
            if amount != 1 {
                vals.push(sr(amount));
            }
            Some(cmd_proc_call(children, vals))
        }
        Opcode::DictAppend => {
            let vals = vec![
                sr("dict append"),
                inst.value.clone()?,
                code_as_source(&args[0]),
                code_as_source(&args[1]),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::DictLappend => {
            let vals = vec![
                sr("dict lappend"),
                inst.value.clone()?,
                code_as_source(&args[0]),
                code_as_source(&args[1]),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::DictExists => {
            let mut vals = vec![sr("dict exists")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle incr opcodes (IncrScalar, IncrArray, IncrStk, etc.).
pub(super) fn handle_incr_opcodes(
    decompiler: &Decompiler,
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::IncrScalar1Imm => {
            let var = inst
                .value
                .clone()
                .or_else(|| decompiler.lvt_value(*inst.operands.first()?))?;
            let mut vals: Vec<SourceRef> = vec![sr("incr"), var];
            let amount = inst.operands.get(1).cloned().unwrap_or(1);
            if amount != 1 {
                vals.push(sr(amount));
            }
            Some(cmd_proc_call(children, vals))
        }
        Opcode::IncrScalar1 => {
            let vals = vec![sr("incr"), inst.value.clone()?, code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::IncrScalarStkImm => {
            let mut vals: Vec<SourceRef> = vec![sr("incr"), code_as_source(&args[0])];
            if inst.operands[0] != 1 {
                vals.push(inst.value.clone()?);
            }
            Some(cmd_proc_call(children, vals))
        }
        Opcode::IncrScalarStk => {
            let vals = vec![
                sr("incr"),
                code_as_source(&args[0]),
                code_as_source(&args[1]),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::IncrArray1 => {
            if args.len() < 2 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    inst.value.clone()?,
                    code_as_source(&args[0]),
                ]));
                let vals = vec![sr("incr"), elem, code_as_source(&args[1])];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::IncrArrayStk => {
            if args.len() < 3 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    code_as_source(&args[0]),
                    code_as_source(&args[1]),
                ]));
                let vals = vec![sr("incr"), elem, code_as_source(&args[2])];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::IncrStk => {
            let vals = vec![
                sr("incr"),
                code_as_source(&args[0]),
                code_as_source(&args[1]),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::IncrArray1Imm => {
            if args.is_empty() {
                None
            } else {
                let var = inst
                    .value
                    .clone()
                    .or_else(|| decompiler.lvt_value(*inst.operands.first()?))?;
                let elem = sr_dyn(ArrayElem::new(vec![var, code_as_source(&args[0])]));
                let mut vals = vec![sr("incr"), elem];
                let amount = inst.operands.get(1).cloned().unwrap_or(1);
                if amount != 1 {
                    vals.push(sr(amount));
                }
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::IncrArrayStkImm => {
            if args.len() < 2 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    code_as_source(&args[0]),
                    code_as_source(&args[1]),
                ]));
                let mut vals = vec![sr("incr"), elem];
                let amount = inst.operands.first().cloned().unwrap_or(1);
                if amount != 1 {
                    vals.push(sr(amount));
                }
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::IncrStkImm => {
            let mut vals = vec![sr("incr"), code_as_source(&args[0])];
            let amount = inst.operands.first().cloned().unwrap_or(1);
            if amount != 1 {
                vals.push(sr(amount));
            }
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle invoke opcodes (InvokeStk, InvokeExpanded, InvokeReplace).
pub(super) fn handle_invoke_opcodes(
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::InvokeStk1 | Opcode::InvokeStk4 => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_source).collect();
            Some(cmd_proc_call(children, vals))
        }
        Opcode::InvokeReplace => {
            // Tcl ensemble rewriting pushes the implementation command after
            // the original argc words. Decompilation should recover the
            // original surface command, not emit the internal implementation.
            let original = args.get(..args.len().checked_sub(1)?)?;
            let vals: Vec<SourceRef> = original.iter().map(code_as_source).collect();
            Some(cmd_proc_call(children, vals))
        }
        Opcode::InvokeExpanded => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_source).collect();
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle unset opcodes (UnsetScalar, UnsetArray, etc.).
pub(super) fn handle_unset_opcodes(
    decompiler: &Decompiler,
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::UnsetScalar => {
            let mut vals = vec![sr("unset")];
            if inst.operands.first().cloned().unwrap_or(1) == 0 {
                vals.push(sr("-nocomplain"));
            }
            let var = inst
                .value
                .clone()
                .or_else(|| decompiler.lvt_value(*inst.operands.get(1)?))?;
            vals.push(var);
            Some(cmd_proc_call(children, vals))
        }
        Opcode::UnsetArray => {
            let mut vals = vec![sr("unset")];
            if inst.operands.first().cloned().unwrap_or(1) == 0 {
                vals.push(sr("-nocomplain"));
            }
            let var = inst
                .value
                .clone()
                .or_else(|| decompiler.lvt_value(*inst.operands.get(1)?))?;
            let elem = sr_dyn(ArrayElem::new(vec![var, code_as_source(&args[0])]));
            vals.push(elem);
            Some(cmd_proc_call(children, vals))
        }
        Opcode::UnsetArrayStk => {
            let mut vals = vec![sr("unset")];
            if inst.operands.first().cloned().unwrap_or(1) == 0 {
                vals.push(sr("-nocomplain"));
            }
            let elem = sr_dyn(ArrayElem::new(vec![
                code_as_source(&args[0]),
                code_as_source(&args[1]),
            ]));
            vals.push(elem);
            Some(cmd_proc_call(children, vals))
        }
        Opcode::UnsetStk => {
            let mut vals = vec![sr("unset")];
            if inst.operands.first().cloned().unwrap_or(1) == 0 {
                vals.push(sr("-nocomplain"));
            }
            vals.push(code_as_source(&args[0]));
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle existence check opcodes (ExistScalar, ExistArray, ArrayExists, etc.).
pub(super) fn handle_exist_opcodes(
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::ExistScalar => {
            let vals = vec![sr("info exists"), inst.value.clone()?];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ExistArray => {
            let elem = sr_dyn(ArrayElem::new(vec![
                inst.value.clone()?,
                code_as_source(&args[0]),
            ]));
            let vals = vec![sr("info exists"), elem];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ExistArrayStk => {
            let elem = sr_dyn(ArrayElem::new(vec![
                code_as_source(&args[0]),
                code_as_source(&args[1]),
            ]));
            let vals = vec![sr("info exists"), elem];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ExistStk => {
            let vals = vec![sr("info exists"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ArrayExistsImm => {
            let vals = vec![sr("array exists"), inst.value.clone()?];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ArrayExistsStk => {
            let vals = vec![sr("array exists"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ArrayMakeImm => {
            let vals = vec![sr("array set"), inst.value.clone()?, sr("{}")];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ArrayMakeStk => {
            let vals = vec![sr("array set"), code_as_source(&args[0]), sr("{}")];
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle append opcodes (AppendScalar, AppendArray, AppendStk, etc.).
pub(super) fn handle_append_opcodes(
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::AppendScalar1 | Opcode::AppendScalar4 => {
            let vals = vec![sr("append"), inst.value.clone()?, code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::AppendArray1 | Opcode::AppendArray4 => {
            if args.len() < 2 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    inst.value.clone()?,
                    code_as_source(&args[0]),
                ]));
                let vals = vec![sr("append"), elem, code_as_source(&args[1])];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::AppendArrayStk => {
            if args.len() < 3 {
                None
            } else {
                let elem = sr_dyn(ArrayElem::new(vec![
                    code_as_source(&args[0]),
                    code_as_source(&args[1]),
                ]));
                let vals = vec![sr("append"), elem, code_as_source(&args[2])];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::AppendStk => {
            let mut vals = vec![sr("append")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        _ => None,
    }
}

/// Handle miscellaneous opcodes (Continue, Break, Push, CallBuiltinFunc, EvalStk, etc.).
pub(super) fn handle_misc_opcodes(
    decompiler: &Decompiler,
    inst: &Instruction,
    args: &[CodeRef],
    children: Vec<CodeRef>,
) -> Option<CodeRef> {
    match inst.opcode {
        Opcode::Continue => Some(cmd_proc_call(children, vec![sr("continue")])),
        Opcode::Break => Some(cmd_proc_call(children, vec![sr("break")])),
        Opcode::Push1 | Opcode::Push4 => {
            let value = inst.value.clone()?;
            Some(cmd_literal(children, value))
        }
        Opcode::CallBuiltinFunc1 => {
            let func = BuiltInMathFunction::try_from(inst.operands[0]).ok()?;
            let vals: Vec<SourceRef> = args.iter().map(code_as_expr).collect();
            Some(cmd_expr_func(children, func.name(), vals))
        }
        Opcode::CallFunc1 => {
            if args.is_empty() {
                None
            } else {
                let name = args[0].borrow().expr_repr();
                let vals: Vec<SourceRef> = args.iter().skip(1).map(code_as_expr).collect();
                Some(cmd_expr_func(children, name, vals))
            }
        }
        Opcode::EvalStk => {
            let extra_children: Vec<CodeRef> = children
                .iter()
                .filter(|c| **c != args[0])
                .cloned()
                .collect();
            if let Code::Command(cmd) = &mut *args[0].borrow_mut()
                && matches!(&cmd.kind, CommandKind::Literal { .. })
            {
                cmd.merge(extra_children);
                cmd.base.stackn += 1;
                return Some(args[0].clone());
            }
            let vals = vec![sr("eval"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ExprStk => Some(cmd_expr(children, code_as_expr(&args[0]))),
        Opcode::TryCvtToNumeric => {
            if let Code::Command(cmd) = &mut *args[0].borrow_mut() {
                cmd.merge_one(children[children.len() - 1].clone());
            }
            Some(args[0].clone())
        }
        Opcode::TryCvtToBoolean => {
            let vals = vec![
                sr("string is"),
                sr("boolean"),
                sr("-strict"),
                code_as_source(&args[0]),
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::NumType => {
            let vals: Vec<SourceRef> = args.iter().map(code_as_expr).collect();
            Some(cmd_expr_func(children, "numtype", vals))
        }
        Opcode::Upvar => {
            let vals = vec![
                sr("upvar"),
                code_as_source(&args[0]),
                code_as_source(&args[1]),
                inst.value.clone()?,
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::Nsupvar => {
            let vals = vec![
                sr("namespace upvar"),
                code_as_source(&args[0]),
                code_as_source(&args[1]),
                inst.value.clone()?,
            ];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::Variable => {
            let vals = vec![sr("variable"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ConcatStk | Opcode::ListConcat => {
            let mut vals = vec![sr("concat")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::LsetList => {
            let old_list = args.last()?;
            let var_name = decompiler.lset_var_name(old_list)?;
            if args.len() < 3 {
                None
            } else {
                let mut vals = vec![sr("lset"), var_name];
                vals.push(code_as_source(&args[0]));
                vals.push(code_as_source(&args[1]));
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::LsetFlat => {
            let old_list = args.last()?;
            let var_name = decompiler.lset_var_name(old_list)?;
            if args.len() < 2 {
                None
            } else {
                let mut vals = vec![sr("lset"), var_name];
                if args.len() > 2 {
                    for idx in &args[..args.len() - 2] {
                        vals.push(code_as_source(idx));
                    }
                }
                vals.push(code_as_source(&args[args.len() - 2]));
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::Tailcall => {
            let mut vals = vec![sr("tailcall")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::Yield => {
            let mut vals = vec![sr("yield")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::YieldToInvoke => {
            let mut vals = vec![sr("yieldto")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::CoroutineName => Some(cmd_proc_call(children, vec![sr("info coroutine")])),
        Opcode::NsCurrent => Some(cmd_proc_call(children, vec![sr("namespace current")])),
        Opcode::InfoLevelNum => Some(cmd_proc_call(children, vec![sr("info level")])),
        Opcode::InfoLevelArgs => {
            let vals = vec![sr("info level"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ResolveCommand => {
            let vals = vec![sr("info commands"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::OriginCommand => {
            let vals = vec![sr("namespace origin"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::TclooSelf => Some(cmd_proc_call(children, vec![sr("self")])),
        Opcode::TclooClass => {
            let vals = vec![sr("info object class"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::TclooNs => {
            let vals = vec![sr("info object namespace"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::TclooIsObject => {
            let vals = vec![sr("info object isa object"), code_as_source(&args[0])];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::TclooNext => {
            let mut vals = vec![sr("next")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::TclooNextClass => {
            let mut vals = vec![sr("nextto")];
            vals.extend(args.iter().map(code_as_source));
            Some(cmd_proc_call(children, vals))
        }
        Opcode::ClockRead => {
            let clock_name = clock_read_name(*inst.operands.first()?)?;
            let vals = vec![sr("clock"), sr(clock_name)];
            Some(cmd_proc_call(children, vals))
        }
        Opcode::Syntax => {
            if args.is_empty() {
                None
            } else {
                let vals = vec![
                    sr("return"),
                    sr("-code"),
                    sr("error"),
                    code_as_source(&args[0]),
                ];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::ReturnImm => {
            if args.len() < 2 || inst.operands.len() < 2 {
                None
            } else {
                let vals = vec![
                    sr("return"),
                    sr("-code"),
                    return_code_name(inst.operands[0]),
                    sr("-level"),
                    sr(inst.operands[1]),
                    sr("-options"),
                    code_as_source(&args[1]),
                    code_as_source(&args[0]),
                ];
                Some(cmd_proc_call(children, vals))
            }
        }
        Opcode::ReturnStk => {
            if args.len() < 2 {
                None
            } else {
                let vals = vec![
                    sr("return"),
                    sr("-options"),
                    code_as_source(&args[0]),
                    code_as_source(&args[1]),
                ];
                Some(cmd_proc_call(children, vals))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{ByteCodeInfo, Instruction};
    use crate::code::code_ref;
    use crate::inst_table::{DEFAULT_INSTRUCTION_SET, TCL_9_INSTRUCTION_SET};

    fn empty_decompiler() -> Decompiler {
        Decompiler::new(ByteCode {
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
        })
        .expect("empty decompiler")
    }

    fn instruction(opcode: Opcode, operands: Vec<i32>, value: Option<SourceRef>) -> Instruction {
        let desc = *[DEFAULT_INSTRUCTION_SET, TCL_9_INSTRUCTION_SET]
            .into_iter()
            .find_map(|set| {
                set.instruction_table
                    .iter()
                    .find(|desc| set.opcode_from_i32(desc.opcode) == Some(opcode))
            })
            .expect("instruction descriptor");
        Instruction::new(100, opcode, desc, operands, value)
    }

    fn arg(location: i32, value: &str) -> CodeRef {
        let child = instruction(Opcode::Done, Vec::new(), None);
        let child = code_ref(Code::Instruction(Instruction::new(
            location,
            child.opcode,
            child.desc,
            child.operands,
            child.value,
        )));
        cmd_literal(vec![child], sr(value.to_string()))
    }

    fn children(args: &[CodeRef]) -> Vec<CodeRef> {
        if args.is_empty() {
            vec![code_ref(Code::Instruction(instruction(
                Opcode::Done,
                Vec::new(),
                None,
            )))]
        } else {
            args.to_vec()
        }
    }

    fn proc_args(code: CodeRef) -> Vec<String> {
        let borrowed = code.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("expected command");
        };
        let CommandKind::ProcCall { args } = &command.kind else {
            panic!("expected proc call");
        };
        args.iter().map(|arg| arg.repr_cow().into_owned()).collect()
    }

    #[test]
    fn load_store_and_arithmetic_handlers_preserve_operands() {
        let decompiler = empty_decompiler();
        let index = arg(0, "key");
        let value = arg(10, "value");

        let load = instruction(Opcode::LoadArray1, vec![0], Some(sr("items")));
        let loaded = handle_load_opcodes(&load, std::slice::from_ref(&index), vec![index.clone()])
            .expect("loadArray1");
        let borrowed = loaded.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("load must produce command");
        };
        let CommandKind::ArrayRef { args } = &command.kind else {
            panic!("loadArray1 must produce array reference");
        };
        assert!(args[0].repr_eq_str("items"));
        assert!(args[1].repr_eq_str("key"));
        drop(borrowed);

        let store = instruction(Opcode::StoreScalar1, vec![0], Some(sr("target")));
        let stored = handle_store_opcodes(
            &decompiler,
            &store,
            std::slice::from_ref(&value),
            vec![value.clone()],
        )
        .expect("storeScalar1");
        let borrowed = stored.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("store must produce command");
        };
        let CommandKind::Set {
            var_name,
            value: stored_value,
        } = &command.kind
        else {
            panic!("storeScalar1 must produce set");
        };
        assert!(var_name.repr_eq_str("target"));
        assert!(stored_value.repr_eq_str("value"));
        drop(borrowed);

        let const_value = arg(40, "42");
        assert_eq!(
            proc_args(
                handle_store_opcodes(
                    &decompiler,
                    &instruction(Opcode::ConstImm, vec![0], Some(sr("answer"))),
                    std::slice::from_ref(&const_value),
                    vec![const_value.clone()],
                )
                .expect("constImm")
            ),
            ["const", "answer", "42"]
        );
        let const_args = [arg(50, "dynamicName"), arg(60, "value")];
        assert_eq!(
            proc_args(
                handle_store_opcodes(
                    &decompiler,
                    &instruction(Opcode::ConstStk, Vec::new(), None),
                    &const_args,
                    children(&const_args),
                )
                .expect("constStk")
            ),
            ["const", "dynamicName", "value"]
        );

        let add = instruction(Opcode::Add, Vec::new(), None);
        let arithmetic_args = [arg(20, "1"), arg(30, "2")];
        let added = handle_arithmetic_opcodes(&add, &arithmetic_args, children(&arithmetic_args))
            .expect("add");
        let borrowed = added.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("add must produce command");
        };
        let CommandKind::Op { op, args } = &command.kind else {
            panic!("add must produce expression operator");
        };
        assert_eq!(op, "+");
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn string_list_and_dict_handlers_render_immediate_options() {
        let decompiler = empty_decompiler();
        let string_args = [arg(0, "*.tcl"), arg(10, "FILE.TCL")];
        let str_match = instruction(Opcode::StrMatch, vec![1], None);
        assert_eq!(
            proc_args(
                handle_string_opcodes(&str_match, &string_args, children(&string_args),)
                    .expect("strMatch")
            ),
            ["string match", "-nocase", "*.tcl", "FILE.TCL"]
        );

        let list_arg = [arg(20, "values")];
        let range = instruction(Opcode::ListRangeImm, vec![1, -2], None);
        assert_eq!(
            proc_args(
                handle_list_opcodes(&range, &list_arg, children(&list_arg)).expect("listRangeImm")
            ),
            ["lrange", "values", "1", "end"]
        );

        let dict_args = [arg(30, "key")];
        let dict_incr = instruction(Opcode::DictIncrImm, vec![3, 0], Some(sr("counts")));
        assert_eq!(
            proc_args(
                handle_dict_opcodes(&decompiler, &dict_incr, &dict_args, children(&dict_args),)
                    .expect("dictIncrImm")
            ),
            ["dict incr", "counts", "key", "3"]
        );
    }

    #[test]
    fn array_mutation_handlers_reject_short_stacks_and_render_valid_stacks() {
        let decompiler = empty_decompiler();
        let short = [arg(0, "name")];
        let append = instruction(Opcode::AppendArrayStk, Vec::new(), None);
        assert!(handle_append_opcodes(&append, &short, children(&short)).is_none());

        let args = [arg(10, "items"), arg(20, "key"), arg(30, "value")];
        assert_eq!(
            proc_args(
                handle_append_opcodes(&append, &args, children(&args)).expect("appendArrayStk")
            ),
            ["append", "items(key)", "value"]
        );

        let incr = instruction(Opcode::IncrArrayStk, Vec::new(), None);
        assert!(handle_incr_opcodes(&decompiler, &incr, &short, children(&short)).is_none());
        assert_eq!(
            proc_args(
                handle_incr_opcodes(&decompiler, &incr, &args, children(&args))
                    .expect("incrArrayStk")
            ),
            ["incr", "items(key)", "value"]
        );

        let lappend = instruction(Opcode::LappendListArrayStk, Vec::new(), None);
        assert!(handle_list_opcodes(&lappend, &short, children(&short)).is_none());
        assert_eq!(
            proc_args(
                handle_list_opcodes(&lappend, &args, children(&args)).expect("lappendListArrayStk")
            ),
            ["lappend", "items(key)", "{*}value"]
        );
    }

    #[test]
    fn invoke_unset_existence_and_return_handlers_cover_boundary_contracts() {
        let decompiler = empty_decompiler();
        let invoke_args = [
            arg(0, "namespace"),
            arg(10, "eval"),
            arg(20, "demo"),
            arg(30, "::tcl::namespace::eval"),
        ];
        let invoke = instruction(Opcode::InvokeReplace, vec![3, 2], None);
        assert_eq!(
            proc_args(
                handle_invoke_opcodes(&invoke, &invoke_args, children(&invoke_args))
                    .expect("invokeReplace")
            ),
            ["namespace", "eval", "demo"]
        );
        assert!(
            handle_invoke_opcodes(&invoke, &[], children(&[])).is_none(),
            "invokeReplace requires its implementation stack value"
        );

        let unset = instruction(Opcode::UnsetScalar, vec![0, 0], Some(sr("temporary")));
        assert_eq!(
            proc_args(
                handle_unset_opcodes(&decompiler, &unset, &[], children(&[])).expect("unsetScalar")
            ),
            ["unset", "-nocomplain", "temporary"]
        );

        let exists = instruction(Opcode::ArrayMakeImm, vec![0], Some(sr("cache")));
        assert_eq!(
            proc_args(handle_exist_opcodes(&exists, &[], children(&[])).expect("arrayMakeImm")),
            ["array set", "cache", "{}"]
        );

        let return_inst = instruction(Opcode::ReturnImm, vec![2, 1], None);
        assert!(handle_misc_opcodes(&decompiler, &return_inst, &[], children(&[])).is_none());
        let return_args = [arg(40, "result"), arg(50, "options")];
        assert_eq!(
            proc_args(
                handle_misc_opcodes(
                    &decompiler,
                    &return_inst,
                    &return_args,
                    children(&return_args),
                )
                .expect("returnImm")
            ),
            [
                "return", "-code", "return", "-level", "1", "-options", "options", "result"
            ]
        );
    }

    #[test]
    fn clock_and_builtin_handlers_fail_closed_for_unknown_operands() {
        let decompiler = empty_decompiler();
        let unknown_clock = instruction(Opcode::ClockRead, vec![99], None);
        assert!(handle_misc_opcodes(&decompiler, &unknown_clock, &[], children(&[])).is_none());

        let microseconds = instruction(Opcode::ClockRead, vec![1], None);
        assert_eq!(
            proc_args(
                handle_misc_opcodes(&decompiler, &microseconds, &[], children(&[]))
                    .expect("clockRead microseconds")
            ),
            ["clock", "microseconds"]
        );

        let invalid_builtin = instruction(Opcode::CallBuiltinFunc1, vec![i32::MAX], None);
        assert!(
            handle_misc_opcodes(
                &decompiler,
                &invalid_builtin,
                std::slice::from_ref(&arg(0, "1")),
                children(&[])
            )
            .is_none()
        );
    }

    #[test]
    fn remaining_immediate_stack_and_boundary_handlers_preserve_surface_forms() {
        let decompiler = empty_decompiler();

        let load_arg = [arg(0, "dynamic")];
        let loaded = handle_load_opcodes(
            &instruction(Opcode::LoadStk, Vec::new(), None),
            &load_arg,
            children(&load_arg),
        )
        .expect("loadStk");
        assert!(matches!(
            &*loaded.borrow(),
            Code::Command(command)
                if matches!(command.kind, CommandKind::Variable { .. })
        ));

        let range_args = [arg(10, "text")];
        assert_eq!(
            proc_args(
                handle_string_opcodes(
                    &instruction(Opcode::StrRangeImm, vec![-3, -2], None),
                    &range_args,
                    children(&range_args),
                )
                .expect("strRangeImm")
            ),
            ["string range", "text", "end-1", "end"]
        );

        let array_args = [arg(20, "items"), arg(30, "key"), arg(40, "value")];
        assert_eq!(
            proc_args(
                handle_list_opcodes(
                    &instruction(Opcode::LappendArrayStk, Vec::new(), None),
                    &array_args,
                    children(&array_args),
                )
                .expect("lappendArrayStk")
            ),
            ["lappend", "items(key)", "value"]
        );

        let invoke_args = [arg(50, "command"), arg(60, "argument")];
        assert_eq!(
            proc_args(
                handle_invoke_opcodes(
                    &instruction(Opcode::InvokeExpanded, Vec::new(), None),
                    &invoke_args,
                    children(&invoke_args),
                )
                .expect("invokeExpanded")
            ),
            ["command", "argument"]
        );

        let unset_args = [arg(70, "key")];
        assert_eq!(
            proc_args(
                handle_unset_opcodes(
                    &decompiler,
                    &instruction(Opcode::UnsetArray, vec![0, 0], Some(sr("items"))),
                    &unset_args,
                    children(&unset_args),
                )
                .expect("unsetArray")
            ),
            ["unset", "-nocomplain", "items(key)"]
        );

        let append_args = [arg(80, "value")];
        assert_eq!(
            proc_args(
                handle_append_opcodes(
                    &instruction(Opcode::AppendScalar1, vec![0], Some(sr("target"))),
                    &append_args,
                    children(&append_args),
                )
                .expect("appendScalar1")
            ),
            ["append", "target", "value"]
        );

        let numeric_args = [arg(90, "value")];
        let numeric = handle_misc_opcodes(
            &decompiler,
            &instruction(Opcode::NumType, Vec::new(), None),
            &numeric_args,
            children(&numeric_args),
        )
        .expect("numType");
        assert!(matches!(
            &*numeric.borrow(),
            Code::Command(command)
                if matches!(
                    &command.kind,
                    CommandKind::ExprFunc { name, .. } if name == "numtype"
                )
        ));

        assert!(
            handle_misc_opcodes(
                &decompiler,
                &instruction(Opcode::Syntax, Vec::new(), None),
                &[],
                children(&[])
            )
            .is_none()
        );
        assert!(
            handle_misc_opcodes(
                &decompiler,
                &instruction(Opcode::ReturnImm, vec![0], None),
                &array_args[..1],
                children(&array_args[..1])
            )
            .is_none()
        );
    }

    #[test]
    fn unset_existence_and_append_handlers_cover_stack_and_immediate_forms() {
        let decompiler = empty_decompiler();
        let args = [arg(0, "items"), arg(10, "key"), arg(20, "value")];

        let unset_array = instruction(Opcode::UnsetArrayStk, vec![0], None);
        assert_eq!(
            proc_args(
                handle_unset_opcodes(&decompiler, &unset_array, &args[..2], children(&args[..2]),)
                    .expect("unsetArrayStk")
            ),
            ["unset", "-nocomplain", "items(key)"]
        );
        let unset_stack = instruction(Opcode::UnsetStk, vec![1], None);
        assert_eq!(
            proc_args(
                handle_unset_opcodes(&decompiler, &unset_stack, &args[..1], children(&args[..1]),)
                    .expect("unsetStk")
            ),
            ["unset", "items"]
        );

        let existence_cases = [
            (
                instruction(Opcode::ExistScalar, vec![0], Some(sr("scalar"))),
                Vec::new(),
                vec!["info exists", "scalar"],
            ),
            (
                instruction(Opcode::ExistArray, vec![0], Some(sr("items"))),
                vec![arg(30, "key")],
                vec!["info exists", "items(key)"],
            ),
            (
                instruction(Opcode::ExistArrayStk, Vec::new(), None),
                vec![arg(40, "items"), arg(50, "key")],
                vec!["info exists", "items(key)"],
            ),
            (
                instruction(Opcode::ExistStk, Vec::new(), None),
                vec![arg(60, "scalar")],
                vec!["info exists", "scalar"],
            ),
            (
                instruction(Opcode::ArrayExistsImm, vec![0], Some(sr("items"))),
                Vec::new(),
                vec!["array exists", "items"],
            ),
            (
                instruction(Opcode::ArrayExistsStk, Vec::new(), None),
                vec![arg(70, "items")],
                vec!["array exists", "items"],
            ),
            (
                instruction(Opcode::ArrayMakeStk, Vec::new(), None),
                vec![arg(80, "items")],
                vec!["array set", "items", "{}"],
            ),
        ];
        for (inst, case_args, expected) in existence_cases {
            assert_eq!(
                proc_args(
                    handle_exist_opcodes(&inst, &case_args, children(&case_args))
                        .unwrap_or_else(|| panic!("{} handler", inst.name()))
                ),
                expected
            );
        }

        let append_immediate = instruction(Opcode::AppendArray1, vec![0], Some(sr("items")));
        assert_eq!(
            proc_args(
                handle_append_opcodes(&append_immediate, &args[1..], children(&args[1..]),)
                    .expect("appendArray1")
            ),
            ["append", "items(key)", "value"]
        );
        let append_stack = instruction(Opcode::AppendStk, Vec::new(), None);
        assert_eq!(
            proc_args(
                handle_append_opcodes(&append_stack, &args[..2], children(&args[..2]),)
                    .expect("appendStk")
            ),
            ["append", "items", "key"]
        );
    }

    #[test]
    fn miscellaneous_command_handlers_preserve_tcl_surface_names() {
        let decompiler = empty_decompiler();
        let cases: Vec<(Instruction, Vec<CodeRef>, Vec<&str>)> = vec![
            (
                instruction(Opcode::TryCvtToBoolean, Vec::new(), None),
                vec![arg(0, "yes")],
                vec!["string is", "boolean", "-strict", "yes"],
            ),
            (
                instruction(Opcode::Upvar, vec![0], Some(sr("local"))),
                vec![arg(10, "1"), arg(20, "remote")],
                vec!["upvar", "1", "remote", "local"],
            ),
            (
                instruction(Opcode::Nsupvar, vec![0], Some(sr("local"))),
                vec![arg(30, "::source"), arg(40, "remote")],
                vec!["namespace upvar", "::source", "remote", "local"],
            ),
            (
                instruction(Opcode::Variable, Vec::new(), None),
                vec![arg(50, "name")],
                vec!["variable", "name"],
            ),
            (
                instruction(Opcode::ConcatStk, Vec::new(), None),
                vec![arg(60, "a"), arg(70, "b")],
                vec!["concat", "a", "b"],
            ),
            (
                instruction(Opcode::Tailcall, Vec::new(), None),
                vec![arg(80, "target"), arg(90, "value")],
                vec!["tailcall", "target", "value"],
            ),
            (
                instruction(Opcode::Yield, Vec::new(), None),
                vec![arg(100, "value")],
                vec!["yield", "value"],
            ),
            (
                instruction(Opcode::YieldToInvoke, Vec::new(), None),
                vec![arg(110, "target"), arg(120, "value")],
                vec!["yieldto", "target", "value"],
            ),
            (
                instruction(Opcode::CoroutineName, Vec::new(), None),
                Vec::new(),
                vec!["info coroutine"],
            ),
            (
                instruction(Opcode::NsCurrent, Vec::new(), None),
                Vec::new(),
                vec!["namespace current"],
            ),
            (
                instruction(Opcode::InfoLevelNum, Vec::new(), None),
                Vec::new(),
                vec!["info level"],
            ),
            (
                instruction(Opcode::InfoLevelArgs, Vec::new(), None),
                vec![arg(130, "2")],
                vec!["info level", "2"],
            ),
            (
                instruction(Opcode::ResolveCommand, Vec::new(), None),
                vec![arg(140, "pattern")],
                vec!["info commands", "pattern"],
            ),
            (
                instruction(Opcode::OriginCommand, Vec::new(), None),
                vec![arg(150, "command")],
                vec!["namespace origin", "command"],
            ),
            (
                instruction(Opcode::TclooSelf, Vec::new(), None),
                Vec::new(),
                vec!["self"],
            ),
            (
                instruction(Opcode::TclooClass, Vec::new(), None),
                vec![arg(160, "object")],
                vec!["info object class", "object"],
            ),
            (
                instruction(Opcode::TclooNs, Vec::new(), None),
                vec![arg(170, "object")],
                vec!["info object namespace", "object"],
            ),
            (
                instruction(Opcode::TclooIsObject, Vec::new(), None),
                vec![arg(180, "object")],
                vec!["info object isa object", "object"],
            ),
            (
                instruction(Opcode::TclooNext, Vec::new(), None),
                vec![arg(190, "value")],
                vec!["next", "value"],
            ),
            (
                instruction(Opcode::TclooNextClass, Vec::new(), None),
                vec![arg(200, "class"), arg(210, "value")],
                vec!["nextto", "class", "value"],
            ),
            (
                instruction(Opcode::Syntax, Vec::new(), None),
                vec![arg(220, "message")],
                vec!["return", "-code", "error", "message"],
            ),
            (
                instruction(Opcode::ReturnStk, Vec::new(), None),
                vec![arg(230, "options"), arg(240, "result")],
                vec!["return", "-options", "options", "result"],
            ),
        ];

        for (inst, args, expected) in cases {
            assert_eq!(
                proc_args(
                    handle_misc_opcodes(&decompiler, &inst, &args, children(&args))
                        .unwrap_or_else(|| panic!("{} handler", inst.name()))
                ),
                expected
            );
        }
    }

    #[test]
    fn eval_function_and_lset_handlers_cover_special_stack_rewrites() {
        let decompiler = empty_decompiler();

        let call_func = instruction(Opcode::CallFunc1, Vec::new(), None);
        assert!(handle_misc_opcodes(&decompiler, &call_func, &[], children(&[])).is_none());
        let function_args = [arg(0, "hypot"), arg(10, "3"), arg(20, "4")];
        let function = handle_misc_opcodes(
            &decompiler,
            &call_func,
            &function_args,
            children(&function_args),
        )
        .expect("callFunc1");
        let borrowed = function.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("callFunc1 command");
        };
        let CommandKind::ExprFunc { name, args } = &command.kind else {
            panic!("callFunc1 expression function");
        };
        assert_eq!(name, "hypot");
        assert_eq!(args.len(), 2);
        drop(borrowed);

        let eval = instruction(Opcode::EvalStk, Vec::new(), None);
        let literal = arg(30, "set x 1");
        let evaluated = handle_misc_opcodes(
            &decompiler,
            &eval,
            std::slice::from_ref(&literal),
            vec![literal.clone(), children(&[])[0].clone()],
        )
        .expect("literal eval");
        assert_eq!(evaluated, literal);
        let borrowed = evaluated.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("literal eval command");
        };
        assert_eq!(command.base.stackn, 2);
        drop(borrowed);

        let dynamic = cmd_variable(vec![children(&[])[0].clone()], sr("script"));
        assert_eq!(
            proc_args(
                handle_misc_opcodes(
                    &decompiler,
                    &eval,
                    std::slice::from_ref(&dynamic),
                    vec![dynamic.clone()],
                )
                .expect("dynamic eval")
            ),
            ["eval", "$script"]
        );

        let old_list = cmd_variable(vec![children(&[])[0].clone()], sr("target"));
        let lset_list = instruction(Opcode::LsetList, Vec::new(), None);
        let list_args = [arg(40, "index"), arg(50, "new"), old_list.clone()];
        assert_eq!(
            proc_args(
                handle_misc_opcodes(&decompiler, &lset_list, &list_args, children(&list_args),)
                    .expect("lsetList")
            ),
            ["lset", "target", "index", "new"]
        );

        let lset_flat = instruction(Opcode::LsetFlat, Vec::new(), None);
        let flat_args = [arg(60, "outer"), arg(70, "inner"), arg(80, "new"), old_list];
        assert_eq!(
            proc_args(
                handle_misc_opcodes(&decompiler, &lset_flat, &flat_args, children(&flat_args),)
                    .expect("lsetFlat")
            ),
            ["lset", "target", "outer", "inner", "new"]
        );
    }

    #[test]
    fn string_handlers_cover_command_expression_and_class_forms() {
        let cases: Vec<(Instruction, Vec<CodeRef>, Vec<&str>)> = vec![
            (
                instruction(Opcode::StrIndex, Vec::new(), None),
                vec![arg(0, "text"), arg(10, "2")],
                vec!["string index", "text", "2"],
            ),
            (
                instruction(Opcode::StrLen, Vec::new(), None),
                vec![arg(20, "text")],
                vec!["string length", "text"],
            ),
            (
                instruction(Opcode::StrEq, Vec::new(), None),
                vec![arg(30, "a"), arg(40, "b")],
                vec!["string equal", "a", "b"],
            ),
            (
                instruction(Opcode::StrCmp, Vec::new(), None),
                vec![arg(50, "a"), arg(60, "b")],
                vec!["string compare", "a", "b"],
            ),
            (
                instruction(Opcode::StrLower, Vec::new(), None),
                vec![arg(70, "TEXT")],
                vec!["string tolower", "TEXT"],
            ),
            (
                instruction(Opcode::Regexp, vec![1], None),
                vec![arg(80, "pattern"), arg(90, "text")],
                vec!["regexp", "-nocase", "pattern", "text"],
            ),
            (
                instruction(Opcode::StrUpper, Vec::new(), None),
                vec![arg(100, "text")],
                vec!["string toupper", "text"],
            ),
            (
                instruction(Opcode::StrTitle, Vec::new(), None),
                vec![arg(110, "text")],
                vec!["string totitle", "text"],
            ),
            (
                instruction(Opcode::StrTrim, Vec::new(), None),
                vec![arg(120, " text ")],
                vec!["string trim", "{ text }"],
            ),
            (
                instruction(Opcode::StrTrimLeft, Vec::new(), None),
                vec![arg(130, " text")],
                vec!["string trimleft", "{ text}"],
            ),
            (
                instruction(Opcode::StrTrimRight, Vec::new(), None),
                vec![arg(140, "text ")],
                vec!["string trimright", "{text }"],
            ),
            (
                instruction(Opcode::StrMap, Vec::new(), None),
                vec![arg(150, "a A"), arg(160, "data")],
                vec!["string map", "{a A}", "data"],
            ),
            (
                instruction(Opcode::StrFind, Vec::new(), None),
                vec![arg(170, "needle"), arg(180, "haystack")],
                vec!["string first", "needle", "haystack"],
            ),
            (
                instruction(Opcode::StrFindLast, Vec::new(), None),
                vec![arg(190, "needle"), arg(200, "haystack")],
                vec!["string last", "needle", "haystack"],
            ),
            (
                instruction(Opcode::StrRange, Vec::new(), None),
                vec![arg(210, "text"), arg(220, "1"), arg(230, "end")],
                vec!["string range", "text", "1", "end"],
            ),
            (
                instruction(Opcode::StrReplace, Vec::new(), None),
                vec![arg(240, "text"), arg(250, "1"), arg(260, "2")],
                vec!["string replace", "text", "1", "2"],
            ),
            (
                instruction(Opcode::StrClass, Vec::new(), Some(sr("integer"))),
                vec![arg(270, "42")],
                vec!["string is", "integer", "42"],
            ),
        ];
        for (inst, args, expected) in cases {
            assert_eq!(
                proc_args(
                    handle_string_opcodes(&inst, &args, children(&args))
                        .unwrap_or_else(|| panic!("{} handler", inst.name()))
                ),
                expected
            );
        }

        let concat_args = [arg(280, "a"), arg(290, "b")];
        let concat = handle_string_opcodes(
            &instruction(Opcode::StrConcat1, vec![2], None),
            &concat_args,
            children(&concat_args),
        )
        .expect("strConcat1");
        let borrowed = concat.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("strConcat1 command");
        };
        let CommandKind::StrCat { args } = &command.kind else {
            panic!("strConcat1 concatenation");
        };
        assert_eq!(args.len(), 2);
        drop(borrowed);

        let neq_args = [arg(300, "a"), arg(310, "b")];
        let neq = handle_string_opcodes(
            &instruction(Opcode::StrNeq, Vec::new(), None),
            &neq_args,
            children(&neq_args),
        )
        .expect("strNeq");
        let borrowed = neq.borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("strNeq command");
        };
        assert!(matches!(&command.kind, CommandKind::Op { op, .. } if op == "ne"));
        drop(borrowed);

        for (opcode, expected) in [
            (Opcode::StrLt, "lt"),
            (Opcode::StrGt, "gt"),
            (Opcode::StrLe, "le"),
            (Opcode::StrGe, "ge"),
        ] {
            let args = [arg(320, "a"), arg(330, "b")];
            let compared = handle_string_opcodes(
                &instruction(opcode, Vec::new(), None),
                &args,
                children(&args),
            )
            .unwrap_or_else(|| panic!("{opcode:?}"));
            let borrowed = compared.borrow();
            let Code::Command(command) = &*borrowed else {
                panic!("{opcode:?} command");
            };
            assert!(
                matches!(&command.kind, CommandKind::Op { op, .. } if op == expected),
                "{opcode:?}"
            );
        }
    }

    #[test]
    fn list_and_dict_handlers_cover_addressing_and_expansion_forms() {
        let decompiler = empty_decompiler();
        let list_cases: Vec<(Instruction, Vec<CodeRef>, Vec<&str>)> = vec![
            (
                instruction(Opcode::List, vec![2], None),
                vec![arg(0, "a"), arg(10, "b")],
                vec!["list", "a", "b"],
            ),
            (
                instruction(Opcode::ListLength, Vec::new(), None),
                vec![arg(20, "values")],
                vec!["llength", "values"],
            ),
            (
                instruction(Opcode::ListIndex, Vec::new(), None),
                vec![arg(30, "values"), arg(40, "1")],
                vec!["lindex", "values", "1"],
            ),
            (
                instruction(Opcode::ListIndexMulti, vec![3], None),
                vec![arg(50, "values"), arg(60, "1"), arg(70, "2")],
                vec!["lindex", "values", "1", "2"],
            ),
            (
                instruction(Opcode::ListIndexImm, vec![-3], None),
                vec![arg(80, "values")],
                vec!["lindex", "values", "end-1"],
            ),
            (
                instruction(Opcode::LappendScalar1, vec![0], Some(sr("items"))),
                vec![arg(90, "value")],
                vec!["lappend", "items", "value"],
            ),
            (
                instruction(Opcode::LappendArray1, vec![0], Some(sr("items"))),
                vec![arg(100, "key"), arg(110, "value")],
                vec!["lappend", "items(key)", "value"],
            ),
            (
                instruction(Opcode::LappendStk, Vec::new(), None),
                vec![arg(120, "items"), arg(130, "value")],
                vec!["lappend", "items", "value"],
            ),
            (
                instruction(Opcode::LappendList, vec![0], Some(sr("items"))),
                vec![arg(140, "values")],
                vec!["lappend", "items", "{*}values"],
            ),
            (
                instruction(Opcode::LappendListArray, vec![0], Some(sr("items"))),
                vec![arg(150, "key"), arg(160, "values")],
                vec!["lappend", "items(key)", "{*}values"],
            ),
            (
                instruction(Opcode::LappendListStk, Vec::new(), None),
                vec![arg(170, "items"), arg(180, "values")],
                vec!["lappend", "items", "{*}values"],
            ),
            (
                instruction(Opcode::Lreplace4, vec![3, 2], None),
                vec![arg(181, "items"), arg(182, "1"), arg(183, "new")],
                vec!["linsert", "items", "1", "new"],
            ),
            (
                instruction(Opcode::Lreplace4, vec![4, 1], None),
                vec![
                    arg(184, "items"),
                    arg(185, "0"),
                    arg(186, "1"),
                    arg(187, "new"),
                ],
                vec!["lreplace", "items", "0", "1", "new"],
            ),
        ];
        for (inst, args, expected) in list_cases {
            assert_eq!(
                proc_args(
                    handle_list_opcodes(&inst, &args, children(&args))
                        .unwrap_or_else(|| panic!("{} handler", inst.name()))
                ),
                expected
            );
        }
        let invalid_lreplace_args = [arg(188, "items"), arg(189, "1")];
        assert!(
            handle_list_opcodes(
                &instruction(Opcode::Lreplace4, vec![2, 0], None),
                &invalid_lreplace_args,
                children(&invalid_lreplace_args),
            )
            .is_none(),
            "non-surface lreplace4 flag combinations must not be guessed"
        );

        for (opcode, expected_op) in [(Opcode::ListIn, "in"), (Opcode::ListNotIn, "ni")] {
            let args = [arg(190, "needle"), arg(200, "values")];
            let code = handle_list_opcodes(
                &instruction(opcode, Vec::new(), None),
                &args,
                children(&args),
            )
            .expect("list membership");
            let borrowed = code.borrow();
            let Code::Command(command) = &*borrowed else {
                panic!("list membership command");
            };
            assert!(matches!(&command.kind, CommandKind::Op { op, .. } if op == expected_op));
        }

        let dict_cases: Vec<(Instruction, Vec<CodeRef>, Vec<&str>)> = vec![
            (
                instruction(Opcode::DictGet, vec![1], None),
                vec![arg(210, "dictionary"), arg(220, "key")],
                vec!["dict get", "dictionary", "key"],
            ),
            (
                instruction(Opcode::DictSet, vec![1, 0], Some(sr("dictionary"))),
                vec![arg(230, "key"), arg(240, "value")],
                vec!["dict set", "dictionary", "key", "value"],
            ),
            (
                instruction(Opcode::DictUnset, vec![1, 0], Some(sr("dictionary"))),
                vec![arg(250, "key")],
                vec!["dict unset", "dictionary", "key"],
            ),
            (
                instruction(Opcode::DictAppend, vec![0], Some(sr("dictionary"))),
                vec![arg(260, "key"), arg(270, "value")],
                vec!["dict append", "dictionary", "key", "value"],
            ),
            (
                instruction(Opcode::DictLappend, vec![0], Some(sr("dictionary"))),
                vec![arg(280, "key"), arg(290, "value")],
                vec!["dict lappend", "dictionary", "key", "value"],
            ),
            (
                instruction(Opcode::DictExists, vec![1], None),
                vec![arg(300, "dictionary"), arg(310, "key")],
                vec!["dict exists", "dictionary", "key"],
            ),
            (
                instruction(Opcode::DictGetDef, vec![1], None),
                vec![arg(320, "dictionary"), arg(330, "key"), arg(340, "default")],
                vec!["dict getwithdefault", "dictionary", "key", "default"],
            ),
        ];
        for (inst, args, expected) in dict_cases {
            assert_eq!(
                proc_args(
                    handle_dict_opcodes(&decompiler, &inst, &args, children(&args))
                        .unwrap_or_else(|| panic!("{} handler", inst.name()))
                ),
                expected
            );
        }
    }

    #[test]
    fn increment_handlers_cover_all_addressing_modes() {
        let decompiler = empty_decompiler();
        let cases: Vec<(Instruction, Vec<CodeRef>, Vec<&str>)> = vec![
            (
                instruction(Opcode::IncrScalar1Imm, vec![0, 2], Some(sr("counter"))),
                Vec::new(),
                vec!["incr", "counter", "2"],
            ),
            (
                instruction(Opcode::IncrScalar1, vec![0], Some(sr("counter"))),
                vec![arg(0, "2")],
                vec!["incr", "counter", "2"],
            ),
            (
                instruction(Opcode::IncrScalarStkImm, vec![2], Some(sr("2"))),
                vec![arg(10, "counter")],
                vec!["incr", "counter", "2"],
            ),
            (
                instruction(Opcode::IncrScalarStk, Vec::new(), None),
                vec![arg(20, "counter"), arg(30, "2")],
                vec!["incr", "counter", "2"],
            ),
            (
                instruction(Opcode::IncrArray1, vec![0], Some(sr("counts"))),
                vec![arg(40, "key"), arg(50, "2")],
                vec!["incr", "counts(key)", "2"],
            ),
            (
                instruction(Opcode::IncrArrayStk, Vec::new(), None),
                vec![arg(60, "counts"), arg(70, "key"), arg(80, "2")],
                vec!["incr", "counts(key)", "2"],
            ),
            (
                instruction(Opcode::IncrStk, Vec::new(), None),
                vec![arg(90, "counter"), arg(100, "2")],
                vec!["incr", "counter", "2"],
            ),
            (
                instruction(Opcode::IncrArray1Imm, vec![0, 3], Some(sr("counts"))),
                vec![arg(110, "key")],
                vec!["incr", "counts(key)", "3"],
            ),
            (
                instruction(Opcode::IncrArrayStkImm, vec![4], Some(sr("4"))),
                vec![arg(120, "counts"), arg(130, "key")],
                vec!["incr", "counts(key)", "4"],
            ),
            (
                instruction(Opcode::IncrStkImm, vec![5], Some(sr("5"))),
                vec![arg(140, "counter")],
                vec!["incr", "counter", "5"],
            ),
        ];
        for (inst, args, expected) in cases {
            assert_eq!(
                proc_args(
                    handle_incr_opcodes(&decompiler, &inst, &args, children(&args))
                        .unwrap_or_else(|| panic!("{} handler", inst.name()))
                ),
                expected
            );
        }
    }
}
