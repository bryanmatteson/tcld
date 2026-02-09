use tcld_decompile::{AnalysisSnapshot, LoadInput, ReconstructionStatus};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/tcld/tests/fixtures")
        .join(name)
}

#[test]
fn snapshot_contains_all_domain_facts_for_fixture_payloads() {
    let snapshot =
        AnalysisSnapshot::from_input(LoadInput::path(fixture("expr.tbc"))).expect("snapshot loads");

    assert_eq!(snapshot.metadata.filename, "expr.tbc");
    assert!(snapshot.metadata.instruction_count > 0);
    assert!(!snapshot.instructions.is_empty());
    assert!(!snapshot.literals.is_empty());
    assert!(!snapshot.exceptions.is_empty());
    assert!(!snapshot.basic_blocks.is_empty());
    assert_eq!(snapshot.traces.len(), snapshot.instructions.len());
    assert!(snapshot.decompile.text.contains("set result"));
    assert!(!snapshot.source_map.is_empty());
    assert!(!snapshot.diagnostics.pass_decisions.is_empty());
    assert_eq!(
        snapshot.diagnostics.reconstruction,
        ReconstructionStatus::Strict
    );
    assert!(!snapshot.coverage.rows.is_empty());
    assert!(snapshot.provenance.label.ends_with("expr.tbc"));
}

#[test]
fn instruction_snapshots_expose_backend_control_flow_and_command_ranges() {
    let snapshot =
        AnalysisSnapshot::from_input(LoadInput::path(fixture("expr.tbc"))).expect("snapshot loads");

    let grouped = snapshot
        .instructions
        .iter()
        .find(|inst| inst.group.is_some())
        .expect("grouped instruction");
    let command_range = grouped
        .command_pc_range
        .as_ref()
        .expect("command pc range is carried from bytecode command ranges");
    assert!(command_range[0] <= grouped.pc);
    assert!(grouped.pc < command_range[1]);

    let jump = snapshot
        .instructions
        .iter()
        .find(|inst| inst.is_jump)
        .expect("fixture contains branch instruction");
    assert_eq!(
        jump.is_jump,
        jump.is_jump_conditional || jump.is_jump_unconditional
    );
    assert_ne!(jump.is_jump_conditional, jump.is_jump_unconditional);
    assert_eq!(jump.jump_target, jump.primary_next_location);
    if jump.is_jump_conditional {
        assert_eq!(jump.fallthrough, jump.secondary_next_location);
        assert!(jump.next_locations.contains(&jump.fallthrough.unwrap()));
    } else {
        assert_eq!(jump.fallthrough, None);
        assert_eq!(jump.secondary_next_location, None);
    }
    assert!(jump.next_locations.contains(&jump.jump_target.unwrap()));
    assert_eq!(jump.argc, snapshot.traces[jump.idx].argc);
}

#[test]
fn loop_snapshot_trace_analysis_terminates_and_builds_maximal_blocks() {
    let snapshot = AnalysisSnapshot::from_input(LoadInput::path(fixture("break.tbc")))
        .expect("snapshot loads");

    assert_eq!(snapshot.traces.len(), snapshot.instructions.len());
    assert!(
        snapshot.basic_blocks.len() < snapshot.instructions.len(),
        "straight-line instruction runs must not become one block per instruction"
    );

    let covered = snapshot
        .basic_blocks
        .iter()
        .flat_map(|block| block.start..=block.end)
        .collect::<Vec<_>>();
    assert_eq!(
        covered,
        (0..snapshot.instructions.len()).collect::<Vec<_>>()
    );
}

#[test]
fn instruction_successors_match_trace_successors_for_terminal_and_multiway_control_flow() {
    let snapshot =
        AnalysisSnapshot::from_input(LoadInput::path(fixture("expr.tbc"))).expect("snapshot loads");

    for instruction in &snapshot.instructions {
        assert_eq!(
            instruction.next_locations, snapshot.traces[instruction.idx].next_locations,
            "instruction and trace successors diverged at pc {}",
            instruction.pc
        );
    }
    assert!(
        snapshot
            .instructions
            .iter()
            .filter(|instruction| instruction.op == "done")
            .all(|instruction| instruction.next_locations.is_empty()),
        "terminal instructions must not advertise fallthrough"
    );
}
