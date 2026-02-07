//! Tcl 9 bytecode instruction assignments.
//!
//! The byte ordering, widths, stack effects, and operand types are transcribed
//! from Tcl 9.0.4's `generic/tclCompile.c::tclInstructionTable`. `Opcode`
//! remains a version-independent semantic identity; this module maps Tcl 9
//! bytes onto those identities.

use super::{FROM_OP1, InstructionDesc, InstructionSet, OpType, Opcode};

macro_rules! inst {
    ($opcode:expr, $argc:expr, $name:literal, $bytes:expr, $stack:expr $(, $operand:expr)* $(,)?) => {
        InstructionDesc::new(
            $opcode,
            $argc,
            $name,
            $bytes,
            $stack,
            &[$($operand),*],
        )
    };
}

pub static TCL_9_INSTRUCTION_TABLE: &[InstructionDesc] = &[
    inst!(0, 1, "done", 1, -1),
    inst!(1, 0, "push1", 2, 1, OpType::Lit1),
    inst!(2, 0, "push4", 5, 1, OpType::Lit4),
    inst!(3, 1, "pop", 1, -1),
    inst!(4, 1, "dup", 1, 1),
    inst!(5, FROM_OP1, "strcat", 2, FROM_OP1, OpType::Uint1),
    inst!(6, FROM_OP1, "invokeStk1", 2, FROM_OP1, OpType::Uint1),
    inst!(7, FROM_OP1, "invokeStk4", 5, FROM_OP1, OpType::Uint4),
    inst!(8, 1, "evalStk", 1, 0),
    inst!(9, 1, "exprStk", 1, 0),
    inst!(10, 0, "loadScalar1", 2, 1, OpType::Lvt1),
    inst!(11, 0, "loadScalar4", 5, 1, OpType::Lvt4),
    inst!(12, 1, "loadScalarStk", 1, 0),
    inst!(13, 1, "loadArray1", 2, 0, OpType::Lvt1),
    inst!(14, 1, "loadArray4", 5, 0, OpType::Lvt4),
    inst!(15, 2, "loadArrayStk", 1, -1),
    inst!(16, 1, "loadStk", 1, 0),
    inst!(17, 1, "storeScalar1", 2, 0, OpType::Lvt1),
    inst!(18, 1, "storeScalar4", 5, 0, OpType::Lvt4),
    inst!(19, 2, "storeScalarStk", 1, -1),
    inst!(20, 2, "storeArray1", 2, -1, OpType::Lvt1),
    inst!(21, 2, "storeArray4", 5, -1, OpType::Lvt4),
    inst!(22, 3, "storeArrayStk", 1, -2),
    inst!(23, 2, "storeStk", 1, -1),
    inst!(24, 1, "incrScalar1", 2, 0, OpType::Lvt1),
    inst!(25, 2, "incrScalarStk", 1, -1),
    inst!(26, 2, "incrArray1", 2, -1, OpType::Lvt1),
    inst!(27, 3, "incrArrayStk", 1, -2),
    inst!(28, 2, "incrStk", 1, -1),
    inst!(29, 0, "incrScalar1Imm", 3, 1, OpType::Lvt1, OpType::Int1),
    inst!(30, 1, "incrScalarStkImm", 2, 0, OpType::Int1),
    inst!(31, 1, "incrArray1Imm", 3, 0, OpType::Lvt1, OpType::Int1),
    inst!(32, 2, "incrArrayStkImm", 2, -1, OpType::Int1),
    inst!(33, 1, "incrStkImm", 2, 0, OpType::Int1),
    inst!(34, 0, "jump1", 2, 0, OpType::Offset1),
    inst!(35, 0, "jump4", 5, 0, OpType::Offset4),
    inst!(36, 1, "jumpTrue1", 2, -1, OpType::Offset1),
    inst!(37, 1, "jumpTrue4", 5, -1, OpType::Offset4),
    inst!(38, 1, "jumpFalse1", 2, -1, OpType::Offset1),
    inst!(39, 1, "jumpFalse4", 5, -1, OpType::Offset4),
    inst!(40, 2, "bitor", 1, -1),
    inst!(41, 2, "bitxor", 1, -1),
    inst!(42, 2, "bitand", 1, -1),
    inst!(43, 2, "eq", 1, -1),
    inst!(44, 2, "neq", 1, -1),
    inst!(45, 2, "lt", 1, -1),
    inst!(46, 2, "gt", 1, -1),
    inst!(47, 2, "le", 1, -1),
    inst!(48, 2, "ge", 1, -1),
    inst!(49, 2, "lshift", 1, -1),
    inst!(50, 2, "rshift", 1, -1),
    inst!(51, 2, "add", 1, -1),
    inst!(52, 2, "sub", 1, -1),
    inst!(53, 2, "mult", 1, -1),
    inst!(54, 2, "div", 1, -1),
    inst!(55, 2, "mod", 1, -1),
    inst!(56, 1, "uplus", 1, 0),
    inst!(57, 1, "uminus", 1, 0),
    inst!(58, 1, "bitnot", 1, 0),
    inst!(59, 1, "not", 1, 0),
    inst!(60, 1, "tryCvtToNumeric", 1, 0),
    inst!(61, 0, "break", 1, 0),
    inst!(62, 0, "continue", 1, 0),
    inst!(63, 0, "beginCatch4", 5, 0, OpType::Uint4),
    inst!(64, 0, "endCatch", 1, 0),
    inst!(65, 0, "pushResult", 1, 1),
    inst!(66, 0, "pushReturnCode", 1, 1),
    inst!(67, 2, "streq", 1, -1),
    inst!(68, 2, "strneq", 1, -1),
    inst!(69, 2, "strcmp", 1, -1),
    inst!(70, 1, "strlen", 1, 0),
    inst!(71, 2, "strindex", 1, -1),
    inst!(72, 2, "strmatch", 2, -1, OpType::Int1),
    inst!(73, FROM_OP1, "list", 5, FROM_OP1, OpType::Uint4),
    inst!(74, 2, "listIndex", 1, -1),
    inst!(75, 1, "listLength", 1, 0),
    inst!(76, 1, "appendScalar1", 2, 0, OpType::Lvt1),
    inst!(77, 1, "appendScalar4", 5, 0, OpType::Lvt4),
    inst!(78, 2, "appendArray1", 2, -1, OpType::Lvt1),
    inst!(79, 2, "appendArray4", 5, -1, OpType::Lvt4),
    inst!(80, 3, "appendArrayStk", 1, -2),
    inst!(81, 2, "appendStk", 1, -1),
    inst!(82, 1, "lappendScalar1", 2, 0, OpType::Lvt1),
    inst!(83, 1, "lappendScalar4", 5, 0, OpType::Lvt4),
    inst!(84, 2, "lappendArray1", 2, -1, OpType::Lvt1),
    inst!(85, 2, "lappendArray4", 5, -1, OpType::Lvt4),
    inst!(86, 3, "lappendArrayStk", 1, -2),
    inst!(87, 2, "lappendStk", 1, -1),
    inst!(88, FROM_OP1, "lindexMulti", 5, FROM_OP1, OpType::Uint4),
    inst!(89, -1, "over", 5, 1, OpType::Uint4),
    inst!(90, 3, "lsetList", 1, -2),
    inst!(91, FROM_OP1, "lsetFlat", 5, FROM_OP1, OpType::Uint4),
    inst!(92, 2, "returnImm", 9, -1, OpType::Int4, OpType::Uint4),
    inst!(93, 2, "expon", 1, -1),
    inst!(94, -1, "expandStart", 1, 0),
    inst!(95, -1, "expandStkTop", 5, 0, OpType::Uint4),
    inst!(96, -1, "invokeExpanded", 1, 0),
    inst!(97, 1, "listIndexImm", 5, 0, OpType::Idx4),
    inst!(98, 1, "listRangeImm", 9, 0, OpType::Idx4, OpType::Idx4),
    inst!(99, -1, "startCommand", 9, 0, OpType::Offset4, OpType::Uint4),
    inst!(100, 2, "listIn", 1, -1),
    inst!(101, 2, "listNotIn", 1, -1),
    inst!(102, 0, "pushReturnOpts", 1, 1),
    inst!(103, 2, "returnStk", 1, -1),
    inst!(104, FROM_OP1, "dictGet", 5, FROM_OP1, OpType::Uint4),
    inst!(
        105,
        FROM_OP1,
        "dictSet",
        9,
        FROM_OP1,
        OpType::Uint4,
        OpType::Lvt4
    ),
    inst!(
        106,
        FROM_OP1,
        "dictUnset",
        9,
        FROM_OP1,
        OpType::Uint4,
        OpType::Lvt4
    ),
    inst!(107, 1, "dictIncrImm", 9, 0, OpType::Int4, OpType::Lvt4),
    inst!(108, 2, "dictAppend", 5, -1, OpType::Lvt4),
    inst!(109, 2, "dictLappend", 5, -1, OpType::Lvt4),
    inst!(110, 1, "dictFirst", 5, 2, OpType::Lvt4),
    inst!(111, 0, "dictNext", 5, 3, OpType::Lvt4),
    inst!(112, 1, "dictUpdateStart", 9, 0, OpType::Lvt4, OpType::Aux4),
    inst!(113, 1, "dictUpdateEnd", 9, -1, OpType::Lvt4, OpType::Aux4),
    inst!(114, 1, "jumpTable", 5, -1, OpType::Aux4),
    inst!(115, 2, "upvar", 5, -1, OpType::Lvt4),
    inst!(116, 2, "nsupvar", 5, -1, OpType::Lvt4),
    inst!(117, 2, "variable", 5, -1, OpType::Lvt4),
    inst!(118, 1, "syntax", 9, -1, OpType::Int4, OpType::Uint4),
    inst!(119, FROM_OP1, "reverse", 5, 0, OpType::Uint4),
    inst!(120, 2, "regexp", 2, -1, OpType::Int1),
    inst!(121, 0, "existScalar", 5, 1, OpType::Lvt4),
    inst!(122, 1, "existArray", 5, 0, OpType::Lvt4),
    inst!(123, 2, "existArrayStk", 1, -1),
    inst!(124, 1, "existStk", 1, 0),
    inst!(125, 0, "nop", 1, 0),
    inst!(126, 1, "returnCodeBranch", 1, -1),
    inst!(127, 0, "unsetScalar", 6, 0, OpType::Uint1, OpType::Lvt4),
    inst!(128, 1, "unsetArray", 6, -1, OpType::Uint1, OpType::Lvt4),
    inst!(129, 2, "unsetArrayStk", 2, -2, OpType::Uint1),
    inst!(130, 1, "unsetStk", 2, -1, OpType::Uint1),
    inst!(131, 2, "dictExpand", 1, -1),
    inst!(132, 3, "dictRecombineStk", 1, -3),
    inst!(133, 2, "dictRecombineImm", 5, -2, OpType::Lvt4),
    inst!(134, FROM_OP1, "dictExists", 5, FROM_OP1, OpType::Uint4),
    inst!(135, 1, "verifyDict", 1, -1),
    inst!(136, 3, "strmap", 1, -2),
    inst!(137, 2, "strfind", 1, -1),
    inst!(138, 2, "strrfind", 1, -1),
    inst!(139, 1, "strrangeImm", 9, 0, OpType::Idx4, OpType::Idx4),
    inst!(140, 3, "strrange", 1, -2),
    inst!(141, 1, "yield", 1, 0),
    inst!(142, 0, "coroName", 1, 1),
    inst!(143, FROM_OP1, "tailcall", 2, FROM_OP1, OpType::Uint1),
    inst!(144, 0, "currentNamespace", 1, 1),
    inst!(145, 0, "infoLevelNumber", 1, 1),
    inst!(146, 1, "infoLevelArgs", 1, 0),
    inst!(147, 1, "resolveCmd", 1, 0),
    inst!(148, 0, "tclooSelf", 1, 1),
    inst!(149, 1, "tclooClass", 1, 0),
    inst!(150, 0, "tclooNamespace", 1, 0),
    inst!(151, 1, "tclooIsObject", 1, 0),
    inst!(152, 1, "arrayExistsStk", 1, 0),
    inst!(153, 0, "arrayExistsImm", 5, 1, OpType::Lvt4),
    inst!(154, 1, "arrayMakeStk", 1, -1),
    inst!(155, 0, "arrayMakeImm", 5, 0, OpType::Lvt4),
    inst!(
        156,
        FROM_OP1,
        "invokeReplace",
        6,
        FROM_OP1,
        OpType::Uint4,
        OpType::Uint1
    ),
    inst!(157, 2, "listConcat", 1, -1),
    inst!(158, -1, "expandDrop", 1, 0),
    inst!(159, 1, "foreach_start", 5, 2, OpType::Aux4),
    inst!(160, 3, "foreach_step", 1, 0),
    inst!(161, 3, "foreach_end", 1, 0),
    inst!(162, 5, "lmap_collect", 1, -1),
    inst!(163, 2, "strtrim", 1, -1),
    inst!(164, 2, "strtrimLeft", 1, -1),
    inst!(165, 2, "strtrimRight", 1, -1),
    inst!(166, FROM_OP1, "concatStk", 5, FROM_OP1, OpType::Uint4),
    inst!(167, 1, "strcaseUpper", 1, 0),
    inst!(168, 1, "strcaseLower", 1, 0),
    inst!(169, 1, "strcaseTitle", 1, 0),
    inst!(170, 4, "strreplace", 1, -3),
    inst!(171, 1, "originCmd", 1, 0),
    inst!(172, FROM_OP1, "tclooNext", 2, FROM_OP1, OpType::Uint1),
    inst!(173, FROM_OP1, "tclooNextClass", 2, FROM_OP1, OpType::Uint1),
    inst!(174, 1, "yieldToInvoke", 1, 0),
    inst!(175, 1, "numericType", 1, 0),
    inst!(176, 1, "tryCvtToBoolean", 1, 1),
    inst!(177, 1, "strclass", 2, 0, OpType::Scls1),
    inst!(178, 1, "lappendList", 5, 0, OpType::Lvt4),
    inst!(179, 2, "lappendListArray", 5, -1, OpType::Lvt4),
    inst!(180, 3, "lappendListArrayStk", 1, -2),
    inst!(181, 2, "lappendListStk", 1, -1),
    inst!(182, 0, "clockRead", 2, 1, OpType::Uint1),
    inst!(183, FROM_OP1, "dictGetDef", 5, FROM_OP1, OpType::Uint4),
    inst!(184, 2, "strlt", 1, -1),
    inst!(185, 2, "strgt", 1, -1),
    inst!(186, 2, "strle", 1, -1),
    inst!(187, 2, "strge", 1, -1),
    inst!(
        188,
        FROM_OP1,
        "lreplace4",
        6,
        FROM_OP1,
        OpType::Uint4,
        OpType::Uint1
    ),
    inst!(189, 1, "constImm", 5, -1, OpType::Lvt4),
    inst!(190, 2, "constStk", 1, -2),
];

pub static TCL_9_OPCODE_TABLE: &[Opcode] = &[
    Opcode::Done,
    Opcode::Push1,
    Opcode::Push4,
    Opcode::Pop,
    Opcode::Dup,
    Opcode::StrConcat1,
    Opcode::InvokeStk1,
    Opcode::InvokeStk4,
    Opcode::EvalStk,
    Opcode::ExprStk,
    Opcode::LoadScalar1,
    Opcode::LoadScalar4,
    Opcode::LoadScalarStk,
    Opcode::LoadArray1,
    Opcode::LoadArray4,
    Opcode::LoadArrayStk,
    Opcode::LoadStk,
    Opcode::StoreScalar1,
    Opcode::StoreScalar4,
    Opcode::StoreScalarStk,
    Opcode::StoreArray1,
    Opcode::StoreArray4,
    Opcode::StoreArrayStk,
    Opcode::StoreStk,
    Opcode::IncrScalar1,
    Opcode::IncrScalarStk,
    Opcode::IncrArray1,
    Opcode::IncrArrayStk,
    Opcode::IncrStk,
    Opcode::IncrScalar1Imm,
    Opcode::IncrScalarStkImm,
    Opcode::IncrArray1Imm,
    Opcode::IncrArrayStkImm,
    Opcode::IncrStkImm,
    Opcode::Jump1,
    Opcode::Jump4,
    Opcode::JumpTrue1,
    Opcode::JumpTrue4,
    Opcode::JumpFalse1,
    Opcode::JumpFalse4,
    Opcode::Bitor,
    Opcode::Bitxor,
    Opcode::Bitand,
    Opcode::Eq,
    Opcode::Neq,
    Opcode::Lt,
    Opcode::Gt,
    Opcode::Le,
    Opcode::Ge,
    Opcode::Lshift,
    Opcode::Rshift,
    Opcode::Add,
    Opcode::Sub,
    Opcode::Mult,
    Opcode::Div,
    Opcode::Mod,
    Opcode::Uplus,
    Opcode::Uminus,
    Opcode::Bitnot,
    Opcode::Not,
    Opcode::TryCvtToNumeric,
    Opcode::Break,
    Opcode::Continue,
    Opcode::BeginCatch4,
    Opcode::EndCatch,
    Opcode::PushResult,
    Opcode::PushReturnCode,
    Opcode::StrEq,
    Opcode::StrNeq,
    Opcode::StrCmp,
    Opcode::StrLen,
    Opcode::StrIndex,
    Opcode::StrMatch,
    Opcode::List,
    Opcode::ListIndex,
    Opcode::ListLength,
    Opcode::AppendScalar1,
    Opcode::AppendScalar4,
    Opcode::AppendArray1,
    Opcode::AppendArray4,
    Opcode::AppendArrayStk,
    Opcode::AppendStk,
    Opcode::LappendScalar1,
    Opcode::LappendScalar4,
    Opcode::LappendArray1,
    Opcode::LappendArray4,
    Opcode::LappendArrayStk,
    Opcode::LappendStk,
    Opcode::ListIndexMulti,
    Opcode::Over,
    Opcode::LsetList,
    Opcode::LsetFlat,
    Opcode::ReturnImm,
    Opcode::Expon,
    Opcode::ExpandStart,
    Opcode::ExpandStktop,
    Opcode::InvokeExpanded,
    Opcode::ListIndexImm,
    Opcode::ListRangeImm,
    Opcode::StartCmd,
    Opcode::ListIn,
    Opcode::ListNotIn,
    Opcode::PushReturnOptions,
    Opcode::ReturnStk,
    Opcode::DictGet,
    Opcode::DictSet,
    Opcode::DictUnset,
    Opcode::DictIncrImm,
    Opcode::DictAppend,
    Opcode::DictLappend,
    Opcode::DictFirst,
    Opcode::DictNext,
    Opcode::DictUpdateStart,
    Opcode::DictUpdateEnd,
    Opcode::JumpTable,
    Opcode::Upvar,
    Opcode::Nsupvar,
    Opcode::Variable,
    Opcode::Syntax,
    Opcode::Reverse,
    Opcode::Regexp,
    Opcode::ExistScalar,
    Opcode::ExistArray,
    Opcode::ExistArrayStk,
    Opcode::ExistStk,
    Opcode::Nop,
    Opcode::ReturnCodeBranch,
    Opcode::UnsetScalar,
    Opcode::UnsetArray,
    Opcode::UnsetArrayStk,
    Opcode::UnsetStk,
    Opcode::DictExpand,
    Opcode::DictRecombineStk,
    Opcode::DictRecombineImm,
    Opcode::DictExists,
    Opcode::DictVerify,
    Opcode::StrMap,
    Opcode::StrFind,
    Opcode::StrFindLast,
    Opcode::StrRangeImm,
    Opcode::StrRange,
    Opcode::Yield,
    Opcode::CoroutineName,
    Opcode::Tailcall,
    Opcode::NsCurrent,
    Opcode::InfoLevelNum,
    Opcode::InfoLevelArgs,
    Opcode::ResolveCommand,
    Opcode::TclooSelf,
    Opcode::TclooClass,
    Opcode::TclooNs,
    Opcode::TclooIsObject,
    Opcode::ArrayExistsStk,
    Opcode::ArrayExistsImm,
    Opcode::ArrayMakeStk,
    Opcode::ArrayMakeImm,
    Opcode::InvokeReplace,
    Opcode::ListConcat,
    Opcode::ExpandDrop,
    Opcode::ForeachStart,
    Opcode::ForeachStep,
    Opcode::ForeachEnd,
    Opcode::LmapCollect,
    Opcode::StrTrim,
    Opcode::StrTrimLeft,
    Opcode::StrTrimRight,
    Opcode::ConcatStk,
    Opcode::StrUpper,
    Opcode::StrLower,
    Opcode::StrTitle,
    Opcode::StrReplace,
    Opcode::OriginCommand,
    Opcode::TclooNext,
    Opcode::TclooNextClass,
    Opcode::YieldToInvoke,
    Opcode::NumType,
    Opcode::TryCvtToBoolean,
    Opcode::StrClass,
    Opcode::LappendList,
    Opcode::LappendListArray,
    Opcode::LappendListArrayStk,
    Opcode::LappendListStk,
    Opcode::ClockRead,
    Opcode::DictGetDef,
    Opcode::StrLt,
    Opcode::StrGt,
    Opcode::StrLe,
    Opcode::StrGe,
    Opcode::Lreplace4,
    Opcode::ConstImm,
    Opcode::ConstStk,
];

pub static TCL_9_INSTRUCTION_SET: InstructionSet = InstructionSet {
    name: "tcl9.0",
    instruction_table: TCL_9_INSTRUCTION_TABLE,
    opcode_table: TCL_9_OPCODE_TABLE,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inst_table::{
        TCL_80_INSTRUCTION_SET, TCL_84_INSTRUCTION_SET, TCL_85_INSTRUCTION_SET,
        TCL_86_INSTRUCTION_SET, TCL_86_INSTRUCTION_TABLE, instruction_set_for_version,
    };
    use std::collections::BTreeMap;

    fn fingerprint_instruction_set(set: InstructionSet) -> u64 {
        const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;

        fn write(hash: &mut u64, bytes: &[u8]) {
            for byte in bytes {
                *hash ^= u64::from(*byte);
                *hash = hash.wrapping_mul(PRIME);
            }
        }

        fn write_field(hash: &mut u64, value: &str) {
            write(hash, &(value.len() as u64).to_be_bytes());
            write(hash, value.as_bytes());
        }

        let mut hash = OFFSET_BASIS;
        write_field(&mut hash, set.name);
        for (desc, opcode) in set.instruction_table.iter().zip(set.opcode_table) {
            write(&mut hash, &desc.opcode.to_be_bytes());
            write_field(&mut hash, desc.name);
            write(&mut hash, &desc.argc.to_be_bytes());
            write(&mut hash, &desc.byte_count.to_be_bytes());
            write(&mut hash, &desc.stack_effect.to_be_bytes());
            write(
                &mut hash,
                &u64::try_from(desc.operands.len())
                    .expect("operand count fits u64")
                    .to_be_bytes(),
            );
            for operand in desc.operands {
                write_field(&mut hash, &format!("{operand:?}"));
            }
            write_field(&mut hash, &format!("{opcode:?}"));
        }
        hash
    }

    #[test]
    fn table_is_complete_and_structurally_consistent() {
        assert_eq!(TCL_9_INSTRUCTION_TABLE.len(), 191);
        assert_eq!(TCL_9_OPCODE_TABLE.len(), TCL_9_INSTRUCTION_TABLE.len());

        for (index, desc) in TCL_9_INSTRUCTION_TABLE.iter().enumerate() {
            assert_eq!(desc.opcode, index as i32, "{}", desc.name);
            let encoded_operands: i32 =
                desc.operands.iter().map(|operand| operand.mem_size()).sum();
            assert_eq!(desc.byte_count, encoded_operands + 1, "{}", desc.name);
        }
    }

    #[test]
    fn version_selection_keeps_tcl8_and_tcl9_assignments_distinct() {
        let tcl86 = instruction_set_for_version(8, 6).expect("Tcl 8.6 table");
        let tcl90 = instruction_set_for_version(9, 0).expect("Tcl 9.0 table");

        assert_eq!(tcl86.name, "tcl8.6");
        assert_eq!(tcl90.name, "tcl9.0");
        assert_eq!(tcl86.opcode_from_i32(40), Some(Opcode::Lor));
        assert_eq!(tcl90.opcode_from_i32(40), Some(Opcode::Bitor));
        assert_eq!(tcl86.opcode_from_i32(189), Some(Opcode::ClockRead));
        assert_eq!(tcl86.opcode_from_i32(190), None);
        assert_eq!(tcl90.opcode_from_i32(190), Some(Opcode::ConstStk));
        assert!(instruction_set_for_version(8, 7).is_none());
        assert!(instruction_set_for_version(9, 1).is_none());
        assert!(instruction_set_for_version(10, 0).is_none());
        assert_eq!(
            tcl90.desc_from_i32(183).map(|desc| desc.name),
            Some("dictGetDef")
        );
        assert_eq!(TCL_86_INSTRUCTION_SET.name, tcl86.name);
        assert_eq!(TCL_86_INSTRUCTION_TABLE[40].name, "lor");
    }

    #[test]
    fn tcl86_table_matches_the_8_6_17_boundary_and_dynamic_invoke_shape() {
        assert_eq!(TCL_86_INSTRUCTION_TABLE.len(), 190);
        assert_eq!(TCL_86_INSTRUCTION_SET.opcode_table.len(), 190);
        assert_eq!(TCL_86_INSTRUCTION_TABLE[189].name, "clockRead");

        let invoke = &TCL_86_INSTRUCTION_TABLE[102];
        assert_eq!(invoke.name, "invokeExpanded");
        assert_eq!(invoke.byte_count, 1);
        assert_eq!(invoke.stack_effect, 0);
        assert_eq!(invoke.argc, -1);
        assert!(invoke.operands.is_empty());
    }

    #[test]
    fn legacy_tables_are_bounded_to_their_actual_opcode_ranges() {
        let expected = [
            (TCL_80_INSTRUCTION_SET, 73, "pushReturnCode"),
            (TCL_84_INSTRUCTION_SET, 98, "lsetFlat"),
            (TCL_85_INSTRUCTION_SET, 132, "existStk"),
            (TCL_86_INSTRUCTION_SET, 190, "clockRead"),
        ];
        for (set, len, last_name) in expected {
            assert_eq!(set.instruction_table.len(), len, "{}", set.name);
            assert_eq!(set.opcode_table.len(), len, "{}", set.name);
            assert_eq!(
                set.instruction_table[len - 1].name,
                last_name,
                "{}",
                set.name
            );
            assert_eq!(set.instruction_table[5].opcode, 5, "{}", set.name);
            assert!(set.desc_from_i32(len as i32).is_none(), "{}", set.name);
        }
        assert_eq!(TCL_80_INSTRUCTION_SET.instruction_table[5].name, "concat1");
        assert_eq!(
            TCL_84_INSTRUCTION_SET.instruction_table[80].name,
            "listindex"
        );
        assert_eq!(
            TCL_84_INSTRUCTION_SET.instruction_table[81].name,
            "listlength"
        );
        assert_eq!(
            TCL_85_INSTRUCTION_SET.instruction_table[80].name,
            "listIndex"
        );
    }

    #[test]
    fn tcl9_only_opcode_tail_matches_tcl_9_0_4() {
        let expected = [
            (183, "dictGetDef", Opcode::DictGetDef),
            (184, "strlt", Opcode::StrLt),
            (185, "strgt", Opcode::StrGt),
            (186, "strle", Opcode::StrLe),
            (187, "strge", Opcode::StrGe),
            (188, "lreplace4", Opcode::Lreplace4),
            (189, "constImm", Opcode::ConstImm),
            (190, "constStk", Opcode::ConstStk),
        ];
        for (byte, name, opcode) in expected {
            assert_eq!(TCL_9_INSTRUCTION_TABLE[byte].name, name);
            assert_eq!(TCL_9_OPCODE_TABLE[byte], opcode);
        }
    }

    #[test]
    fn shared_instruction_names_keep_the_same_semantic_opcode() {
        let tcl86_semantics = TCL_86_INSTRUCTION_SET
            .instruction_table
            .iter()
            .zip(TCL_86_INSTRUCTION_SET.opcode_table)
            .map(|(desc, opcode)| (desc.name, *opcode))
            .collect::<BTreeMap<_, _>>();

        for (desc, opcode) in TCL_9_INSTRUCTION_SET
            .instruction_table
            .iter()
            .zip(TCL_9_INSTRUCTION_SET.opcode_table)
        {
            if let Some(expected) = tcl86_semantics.get(desc.name) {
                assert_eq!(
                    opcode, expected,
                    "Tcl 9 instruction {:?} changed semantic identity",
                    desc.name
                );
            }
        }
    }

    #[test]
    fn verified_instruction_tables_have_stable_source_audited_fingerprints() {
        // These fingerprints cover every descriptor field plus the semantic
        // opcode mapping. They were established after comparing all entries
        // against Tcl 8.6.17 and Tcl 9.0.4's tclInstructionTable sources.
        assert_eq!(
            fingerprint_instruction_set(TCL_86_INSTRUCTION_SET),
            0xdf48_afdf_1839_0436
        );
        assert_eq!(
            fingerprint_instruction_set(TCL_9_INSTRUCTION_SET),
            0xe4c3_7a0c_480f_a14c
        );
    }
}
