//! Closed Hybrid-max opcode partition.

use tcld_core::inst_table::Opcode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcodeClass {
    Vm,
    HostValue,
    HostFrame,
    Trap,
}

pub fn classify(op: Opcode) -> OpcodeClass {
    use Opcode::*;
    match op {
        Done | Nop | StartCmd | Push1 | Push4 | Pop | Dup | Over | Reverse | Jump1 | Jump4
        | JumpTrue1 | JumpTrue4 | JumpFalse1 | JumpFalse4 | LoadScalar1 | LoadScalar4
        | StoreScalar1 | StoreScalar4 | LoadArray1 | LoadArray4 | StoreArray1 | StoreArray4
        | LoadScalarStk | StoreScalarStk | IncrScalarStkImm | IncrScalar1 | IncrScalar1Imm
        | ExistScalar | TryCvtToNumeric | TryCvtToBoolean | NumType | Add | Sub | Mult | Div
        | Mod | Expon | Uplus | Uminus | Eq | Neq | Lt | Gt | Le | Ge | Not | Land | Lor
        | StrEq | StrNeq | StrCmp | StrLt | StrGt | StrLe | StrGe | StrLen | StrIndex
        | StrMatch | StrConcat1 | List | ListIndex | ListLength | BeginCatch4 | EndCatch
        | PushReturnCode | PushResult | PushReturnOptions | ReturnImm | ReturnStk
        | ReturnCodeBranch | Break | Continue | Variable | Upvar | InvokeStk1 | InvokeStk4
        | ForeachStart4 | ForeachStep4 | ForeachStart | ForeachStep | ForeachEnd | LmapCollect
        | JumpTable | NsCurrent | LoadArrayStk | LoadStk | StoreArrayStk | StoreStk
        | IncrScalarStk | IncrArray1 | IncrArrayStk | IncrStk | IncrArray1Imm | IncrArrayStkImm
        | IncrStkImm | Bitor | Bitxor | Bitand | Lshift | Rshift | Bitnot | AppendScalar1
        | AppendScalar4 | AppendArray1 | AppendArray4 | AppendArrayStk | AppendStk
        | LappendScalar1 | LappendScalar4 | LappendArray1 | LappendArray4 | LappendArrayStk
        | LappendStk | LappendList | LappendListArray | LappendListArrayStk | LappendListStk
        | ListIndexMulti | ListIndexImm | ListRangeImm | ListIn | ListNotIn | ListConcat
        | LsetList | LsetFlat | ConcatStk | ExistArray | ExistArrayStk | ExistStk | UnsetScalar
        | UnsetArray | UnsetArrayStk | UnsetStk | ArrayExistsStk | ArrayExistsImm
        | ArrayMakeStk | ArrayMakeImm | Syntax | ExpandStart | ExpandStktop | ExpandDrop
        | InvokeExpanded | InvokeReplace | CallBuiltinFunc1 | CallFunc1 | StrMap | StrFind
        | StrFindLast | StrRangeImm | StrRange | StrTrim | StrTrimLeft | StrTrimRight
        | StrUpper | StrLower | StrTitle | StrReplace | StrClass | ClockRead | DictGet
        | DictSet | DictUnset | DictIncrImm | DictAppend | DictLappend | DictFirst | DictNext
        | DictDone | DictUpdateStart | DictUpdateEnd | DictExpand | DictRecombineStk
        | DictRecombineImm | DictExists | DictVerify | DictGetDef | Lreplace4 | Regexp => {
            OpcodeClass::Vm
        }

        ExprStk | EvalStk => OpcodeClass::HostFrame,

        Nsupvar => OpcodeClass::HostValue,

        Yield | CoroutineName | Tailcall | YieldToInvoke | TclooSelf | TclooClass | TclooNs
        | TclooIsObject | TclooNext | TclooNextClass | InfoLevelNum | InfoLevelArgs
        | ResolveCommand | OriginCommand | ConstImm | ConstStk => OpcodeClass::Trap,
    }
}

pub const DELIBERATE_TRAP_WITNESSES: &[Opcode] = &[
    Opcode::Yield,
    Opcode::TclooSelf,
    Opcode::InfoLevelNum,
    Opcode::OriginCommand,
];

#[cfg(test)]
mod tests {
    use super::*;
    use tcld_core::inst_table::{DEFAULT_INSTRUCTION_SET, TCL_9_INSTRUCTION_SET};

    #[test]
    fn deliberate_trap_witnesses_remain_trap() {
        for &opcode in DELIBERATE_TRAP_WITNESSES {
            assert_eq!(classify(opcode), OpcodeClass::Trap, "{opcode:?}");
        }
    }

    #[test]
    fn core_vm_opcodes() {
        for opcode in [
            Opcode::Done,
            Opcode::InvokeStk1,
            Opcode::ForeachStart4,
            Opcode::LmapCollect,
            Opcode::JumpTable,
            Opcode::NsCurrent,
            Opcode::InvokeExpanded,
        ] {
            assert_eq!(classify(opcode), OpcodeClass::Vm, "{opcode:?}");
        }
    }

    #[test]
    fn tcl9_tail_opcodes_have_explicit_runtime_dispositions() {
        for opcode in [
            Opcode::StrLt,
            Opcode::StrGt,
            Opcode::StrLe,
            Opcode::StrGe,
            Opcode::DictGetDef,
            Opcode::Lreplace4,
        ] {
            assert_eq!(classify(opcode), OpcodeClass::Vm, "{opcode:?}");
        }
        for opcode in [Opcode::ConstImm, Opcode::ConstStk] {
            assert_eq!(classify(opcode), OpcodeClass::Trap, "{opcode:?}");
        }
    }

    #[test]
    fn only_namespace_upvar_needs_the_host_for_values() {
        for &opcode in DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .chain(TCL_9_INSTRUCTION_SET.opcode_table)
        {
            assert_eq!(
                classify(opcode) == OpcodeClass::HostValue,
                opcode == Opcode::Nsupvar,
                "{opcode:?}"
            );
        }
    }

    #[test]
    fn partition_matches_hybrid_max_snapshot() {
        let mut counts = [0usize; 4];
        for &opcode in DEFAULT_INSTRUCTION_SET.opcode_table {
            counts[match classify(opcode) {
                OpcodeClass::Vm => 0,
                OpcodeClass::HostValue => 1,
                OpcodeClass::HostFrame => 2,
                OpcodeClass::Trap => 3,
            }] += 1;
        }
        assert_eq!(counts, [173, 1, 2, 14]);
        assert_eq!(counts.into_iter().sum::<usize>(), 190);
    }

    #[test]
    fn tcl9_partition_is_version_specific_and_complete() {
        let mut counts = [0usize; 4];
        for &opcode in TCL_9_INSTRUCTION_SET.opcode_table {
            counts[match classify(opcode) {
                OpcodeClass::Vm => 0,
                OpcodeClass::HostValue => 1,
                OpcodeClass::HostFrame => 2,
                OpcodeClass::Trap => 3,
            }] += 1;
        }
        assert_eq!(counts, [172, 1, 2, 16]);
        assert_eq!(counts.into_iter().sum::<usize>(), 191);
    }

    #[test]
    fn sorted_opcode_membership_matches_hybrid_max_snapshot() {
        use sha2::{Digest, Sha256};

        let mut membership = DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .map(|&opcode| format!("{:?}:{opcode:?}", classify(opcode)))
            .collect::<Vec<_>>();
        membership.sort();
        let digest = Sha256::digest(membership.join("\n").as_bytes());
        assert_eq!(
            format!("{digest:x}"),
            "fdbec9f67c2c03eb9a6a784c96d2cb1b121873024fa45fad3bd34c124a22656e",
            "sorted opcode disposition membership changed"
        );
    }

    #[test]
    fn every_promoted_disposition_is_named_by_the_runtime_dispatch() {
        let engine = include_str!("engine.rs");
        for &opcode in DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .chain(TCL_9_INSTRUCTION_SET.opcode_table)
        {
            if classify(opcode) == OpcodeClass::Trap {
                continue;
            }
            let needle = format!("Opcode::{opcode:?}");
            assert!(
                engine.contains(&needle),
                "{opcode:?} is promoted but absent from the runtime handler source"
            );
        }
        let policy = engine.find("OpcodeClass::HostValue if !self.policy.allow_host_opcode");
        let dispatch = engine.find(".execution_boundary(inst.pc)");
        assert!(
            policy.is_some() && policy < dispatch,
            "HostValue policy denial must occur before runtime dispatch can mutate state"
        );
    }
}
