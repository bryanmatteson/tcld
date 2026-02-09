use std::path::{Path, PathBuf};

use tcld_core::bytecode::{
    ByteCode, ByteCodeInfo, CompiledLocal, Header, LiteralObject, LiteralObjectType, Version,
};
use tcld_core::inst_table::{Opcode, TCL_9_INSTRUCTION_SET};
use tcld_core::source::SourceRef;
use tcld_decompile::{Decompiler, LoadInput, decompile_loaded, load_bytecode};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn assert_nested_bytecode_version(bytecode: &ByteCode, expected: (i32, i32), table_name: &str) {
    for literal in &bytecode.literals {
        let Some(proc_body) = literal.as_proc_body() else {
            continue;
        };
        let header = proc_body
            .bytecode
            .header
            .as_ref()
            .expect("nested proc inherits compiled header");
        assert_eq!(
            (header.tcl_version.major, header.tcl_version.minor),
            expected
        );
        assert_eq!(
            proc_body
                .bytecode
                .instruction_set()
                .expect("supported instruction set")
                .name,
            table_name
        );
        assert_nested_bytecode_version(&proc_body.bytecode, expected, table_name);
    }
}

fn tcl9_opcode(opcode: Opcode) -> u8 {
    TCL_9_INSTRUCTION_SET
        .opcode_table
        .iter()
        .position(|candidate| *candidate == opcode)
        .and_then(|index| u8::try_from(index).ok())
        .expect("Tcl 9 opcode")
}

fn literal(value: &str) -> LiteralObject {
    LiteralObject {
        kind: LiteralObjectType::String,
        value: SourceRef::Shared(value.into()),
        proc_body: None,
    }
}

fn tcl9_bytecode(code_bytes: Vec<u8>, values: &[&str]) -> ByteCode {
    let literals = values
        .iter()
        .map(|value| literal(value))
        .collect::<Vec<_>>();
    ByteCode {
        header: Some(Header {
            format: 3,
            build: 0,
            compiler_version: Version { major: 2, minor: 0 },
            tcl_version: Version { major: 9, minor: 0 },
        }),
        info: ByteCodeInfo {
            num_commands: 0,
            num_src_bytes: 0,
            num_code_bytes: code_bytes.len() as i32,
            num_lit_objects: literals.len() as i32,
            num_except_ranges: 0,
            num_aux_data_items: 0,
            num_cmd_loc_bytes: 0,
            max_except_depth: 0,
            max_stack_depth: 16,
            code_delta_size: 0,
            code_length_size: 0,
            src_delta_size: 0,
            src_length_size: 0,
        },
        code_bytes,
        cmd_code_ranges: vec![],
        cmd_source_ranges: None,
        literals,
        exceptions: vec![],
        auxiliaries: vec![],
        proc_num_args: 0,
        proc_num_locals: 0,
        locals: vec![],
    }
}

#[test]
fn tcl86_loader_corpus_decompiles_strictly_with_expected_constructs() {
    let cases = [
        ("testing/fixtures/bytecode/tcl8/tc1.tbc", "set a 1"),
        ("testing/fixtures/bytecode/tcl8/tc1pre.tbc", "set a 1"),
        ("testing/fixtures/bytecode/tcl8/tc2.tbc", "proc mytest"),
        ("testing/fixtures/bytecode/tcl8/tc3.tbc", "foreach m $me"),
        ("testing/fixtures/bytecode/tcl8/tc4.tbc", "switch -integer"),
        (
            "testing/fixtures/bytecode/tcl8/tc5.tbc",
            "return -code error",
        ),
        (
            "testing/fixtures/bytecode/tcl8/tc6.tbc",
            "namespace eval demo",
        ),
    ];

    for (relative, expected) in cases {
        let path = fixture(relative);
        let loaded = load_bytecode(LoadInput::path(&path))
            .unwrap_or_else(|error| panic!("{} failed to load: {error}", path.display()));
        let header = loaded.bytecode.header.as_ref().expect("compiled header");
        assert_eq!(
            (header.tcl_version.major, header.tcl_version.minor),
            (8, 6),
            "{}",
            path.display()
        );
        assert!(
            !loaded
                .bytecode
                .get_instructions()
                .expect("instructions decode")
                .is_empty(),
            "{}",
            path.display()
        );
        let source = decompile_loaded(&loaded)
            .unwrap_or_else(|error| panic!("{} failed strict decompile: {error}", path.display()));
        assert!(
            source.contains(expected),
            "{} did not recover {expected:?}:\n{source}",
            path.display()
        );
    }
}

#[test]
fn tcl9_loader_corpus_uses_the_tcl9_table_and_decompiles_strictly() {
    let cases = [
        ("testing/fixtures/bytecode/tcl9/tc1.tbc", "set a 1"),
        ("testing/fixtures/bytecode/tcl9/tc1pre.tbc", "set a 1"),
        ("testing/fixtures/bytecode/tcl9/tc2.tbc", "proc mytest"),
        ("testing/fixtures/bytecode/tcl9/tc3.tbc", "foreach m $me"),
        ("testing/fixtures/bytecode/tcl9/tc4.tbc", "switch -integer"),
        (
            "testing/fixtures/bytecode/tcl9/tc5.tbc",
            "return -code error",
        ),
        (
            "testing/fixtures/bytecode/tcl9/tc6.tbc",
            "namespace eval demo",
        ),
    ];

    for (relative, expected) in cases {
        let path = fixture(relative);
        let loaded = load_bytecode(LoadInput::path(&path))
            .unwrap_or_else(|error| panic!("{} failed to load: {error}", path.display()));
        let header = loaded.bytecode.header.as_ref().expect("compiled header");
        assert_eq!(
            (header.tcl_version.major, header.tcl_version.minor),
            (9, 0),
            "{}",
            path.display()
        );
        assert_eq!(
            loaded
                .bytecode
                .instruction_set()
                .expect("Tcl 9 instruction set")
                .name,
            "tcl9.0"
        );
        assert_nested_bytecode_version(&loaded.bytecode, (9, 0), "tcl9.0");
        let source = decompile_loaded(&loaded)
            .unwrap_or_else(|error| panic!("{} failed strict decompile: {error}", path.display()));
        assert!(
            source.contains(expected),
            "{} did not recover {expected:?}:\n{source}",
            path.display()
        );
    }
}

#[test]
fn hand_built_tcl9_tail_opcodes_decompile_through_the_versioned_table() {
    for (opcode, expected) in [
        (Opcode::StrLt, "lt"),
        (Opcode::StrGt, "gt"),
        (Opcode::StrLe, "le"),
        (Opcode::StrGe, "ge"),
    ] {
        let source = Decompiler::new(tcl9_bytecode(
            vec![
                tcl9_opcode(Opcode::Push1),
                0,
                tcl9_opcode(Opcode::Push1),
                1,
                tcl9_opcode(opcode),
                tcl9_opcode(Opcode::Done),
            ],
            &["left", "right"],
        ))
        .and_then(Decompiler::get_source)
        .unwrap_or_else(|error| panic!("{opcode:?}: {error}"));
        assert!(source.contains(expected), "{opcode:?}: {source}");
    }

    let mut dict_code = vec![
        tcl9_opcode(Opcode::Push1),
        0,
        tcl9_opcode(Opcode::Push1),
        1,
        tcl9_opcode(Opcode::Push1),
        2,
        tcl9_opcode(Opcode::DictGetDef),
    ];
    dict_code.extend(1i32.to_be_bytes());
    dict_code.push(tcl9_opcode(Opcode::Done));
    let source = Decompiler::new(tcl9_bytecode(dict_code, &["dictionary", "key", "fallback"]))
        .and_then(Decompiler::get_source)
        .expect("dictGetDef strict decompile");
    assert!(source.contains("dict getwithdefault"), "{source}");

    let mut list_code = vec![
        tcl9_opcode(Opcode::Push1),
        0,
        tcl9_opcode(Opcode::Push1),
        1,
        tcl9_opcode(Opcode::Push1),
        2,
        tcl9_opcode(Opcode::Lreplace4),
    ];
    list_code.extend(3i32.to_be_bytes());
    list_code.push(2);
    list_code.push(tcl9_opcode(Opcode::Done));
    let source = Decompiler::new(tcl9_bytecode(list_code, &["items", "1", "new"]))
        .and_then(Decompiler::get_source)
        .expect("lreplace4 strict decompile");
    assert!(source.contains("linsert"), "{source}");

    let mut const_imm_code = vec![tcl9_opcode(Opcode::Push1), 0, tcl9_opcode(Opcode::ConstImm)];
    const_imm_code.extend(0i32.to_be_bytes());
    const_imm_code.extend([tcl9_opcode(Opcode::Push1), 1, tcl9_opcode(Opcode::Done)]);
    let mut const_imm = tcl9_bytecode(const_imm_code, &["42", ""]);
    const_imm.proc_num_locals = 1;
    const_imm.locals = vec![CompiledLocal {
        name: "answer".to_string(),
        default_value: None,
        frame_index: 0,
        flag_mask: 0,
    }];
    let source = Decompiler::new(const_imm)
        .and_then(Decompiler::get_source)
        .expect("constImm strict decompile");
    assert!(source.contains("const answer 42"), "{source}");

    let source = Decompiler::new(tcl9_bytecode(
        vec![
            tcl9_opcode(Opcode::Push1),
            0,
            tcl9_opcode(Opcode::Push1),
            1,
            tcl9_opcode(Opcode::ConstStk),
            tcl9_opcode(Opcode::Push1),
            2,
            tcl9_opcode(Opcode::Done),
        ],
        &["answer", "42", ""],
    ))
    .and_then(Decompiler::get_source)
    .expect("constStk strict decompile");
    assert!(source.contains("const answer 42"), "{source}");
}
