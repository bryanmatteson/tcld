use tcld_decompile::bytecode::{ByteCode, ByteCodeInfo};
use tcld_decompile::source::sr_dyn;
use tcld_decompile::{
    Decompiler, DiagnosticsReport, LoadInput, ReconstructionStatus,
    decompile_input_approximate_with_diagnostics, decompile_input_strict_with_diagnostics,
};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/tcld/tests/fixtures")
        .join(name)
}

fn unsupported_bytecode() -> ByteCode {
    ByteCode {
        header: None,
        info: ByteCodeInfo {
            num_commands: 1,
            num_src_bytes: 0,
            num_code_bytes: 1,
            num_lit_objects: 0,
            num_except_ranges: 0,
            num_aux_data_items: 0,
            num_cmd_loc_bytes: 0,
            max_except_depth: 0,
            max_stack_depth: 0,
            code_delta_size: 0,
            code_length_size: 0,
            src_delta_size: -1,
            src_length_size: -1,
        },
        code_bytes: vec![8],
        cmd_code_ranges: std::iter::once(0..1).collect(),
        cmd_source_ranges: None,
        literals: Vec::new(),
        exceptions: Vec::new(),
        auxiliaries: Vec::new(),
        proc_num_args: 0,
        proc_num_locals: 0,
        locals: Vec::new(),
    }
}

#[test]
fn strict_approximation_and_failure_diagnostics_carry_reconstruction_state_and_causes() {
    let strict = decompile_input_strict_with_diagnostics(LoadInput::path(fixture("expr.tbc")))
        .expect("strict decompile diagnostics");
    assert!(strict.output.text.contains("set result"));
    assert!(strict.diagnostics.pass_decisions.len() >= 8);
    assert!(
        strict
            .diagnostics
            .recognized_constructs
            .iter()
            .any(|item| item.contains("expr") || item.contains("command"))
    );
    assert!(
        !strict.diagnostics.suppressed_fast_path_ranges.is_empty(),
        "expr fixture should report suppressed fast-path ranges"
    );
    assert_eq!(
        strict.diagnostics.reconstruction,
        ReconstructionStatus::Strict
    );

    let approximation =
        decompile_input_approximate_with_diagnostics(LoadInput::path(fixture("expr.tbc")))
            .expect("approximation diagnostics");
    assert_eq!(
        approximation.diagnostics.reconstruction,
        ReconstructionStatus::Strict,
        "permissive mode must still report an exact reconstruction when it omitted nothing"
    );
    assert!(
        approximation.diagnostics.approximations.is_empty(),
        "internal fast-path suppression is not a semantic approximation"
    );

    let approximation = Decompiler::new(unsupported_bytecode())
        .expect("decompiler")
        .get_output_approximate_with_diagnostics()
        .expect("permissive decompile should preserve diagnostics for unresolved instructions");
    assert_eq!(
        approximation.diagnostics.reconstruction,
        ReconstructionStatus::Approximation
    );
    assert!(
        approximation
            .diagnostics
            .approximations
            .iter()
            .all(|record| !record.reason.is_empty()
                && (record.kind == tcld_decompile::ApproximationKind::NestedProcedure
                    || record.bytecode_range.is_some())),
        "every approximation should explain what changed and locate it when a bytecode range exists"
    );

    let failure = Decompiler::new(unsupported_bytecode())
        .expect("decompiler")
        .get_output_with_diagnostics()
        .expect_err("strict decompile should fail on unreduced evalStk");
    let DiagnosticsReport {
        unreduced_opcodes,
        unsupported_instructions,
        reconstruction,
        pass_decisions,
        suggested_next_work,
        ..
    } = failure.diagnostics;
    assert!(
        unreduced_opcodes.iter().any(|op| op.contains("evalStk")),
        "strict failures must carry unreduced opcode diagnostics"
    );
    assert!(
        unsupported_instructions
            .iter()
            .any(|op| op.contains("evalStk"))
    );
    assert_eq!(reconstruction, ReconstructionStatus::Failed);
    assert!(!pass_decisions.is_empty());
    assert!(
        suggested_next_work
            .iter()
            .any(|item| item.contains("evalStk"))
    );
}

#[test]
fn strictness_propagates_through_nested_proc_bodies() {
    let mut loaded =
        tcld_decompile::load_bytecode(LoadInput::path(fixture("proc.tbc"))).expect("proc fixture");
    let literal = loaded
        .bytecode
        .literals
        .iter_mut()
        .find(|literal| literal.proc_body.is_some())
        .expect("compiled proc body literal");
    let proc_body = literal.proc_body.as_mut().expect("proc body");
    proc_body.bytecode = unsupported_bytecode();
    literal.value = sr_dyn(proc_body.clone());

    let strict = Decompiler::new(loaded.bytecode.clone())
        .expect("decompiler")
        .get_output_with_diagnostics()
        .expect_err("nested strict failure must fail the root decompilation");
    assert!(strict.to_string().contains("nested proc body"));
    assert_eq!(
        strict.diagnostics.reconstruction,
        ReconstructionStatus::Failed
    );

    let approximation = Decompiler::new(loaded.bytecode)
        .expect("decompiler")
        .get_output_approximate_with_diagnostics()
        .expect("approximated root decompilation");
    assert_eq!(
        approximation.diagnostics.reconstruction,
        ReconstructionStatus::Approximation
    );
    assert!(
        approximation
            .diagnostics
            .approximations
            .iter()
            .any(|record| {
                record.kind == tcld_decompile::ApproximationKind::NestedProcedure
                    && record.bytecode_range.is_none()
                    && record.reason.contains("nested proc body")
                    && record.reason.contains("UnresolvedInstruction")
                    && record.reason.contains("inner bytecode 0..1")
            })
    );
}
