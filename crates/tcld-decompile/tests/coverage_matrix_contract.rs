use tcld_decompile::{
    CoverageClassification, CoverageMatrixConfig, TclVersionCategory, generate_coverage_matrix,
    generate_coverage_matrix_by_version,
};

#[test]
fn every_instruction_table_opcode_is_classified_and_evidence_changes_status() {
    let matrix = generate_coverage_matrix(CoverageMatrixConfig::default());
    assert!(!matrix.rows.is_empty());
    assert!(matrix.rows.iter().all(|row| !row.opcode_name.is_empty()));
    assert!(matrix.rows.iter().all(|row| row.classification
        != CoverageClassification::Unclassified
        || row.evidence.decoded));

    let by_version = generate_coverage_matrix_by_version(CoverageMatrixConfig::default());
    let push = by_version
        .rows_for_version(TclVersionCategory::Tcl80)
        .into_iter()
        .find(|row| row.opcode_name == "push1")
        .expect("Tcl 8.0 push1 row");
    assert_eq!(
        push.classification,
        CoverageClassification::GoldenTested,
        "push1 should be golden-tested from real fixture opcode evidence and in-tree test evidence"
    );
    assert!(push.evidence.handler);
    assert!(push.evidence.golden);
    assert!(push.evidence.test);
    let tcl86_push = matrix.row("push1").expect("Tcl 8.6 push1 row");
    assert!(
        !tcl86_push.evidence.golden,
        "Tcl 8.0 fixtures must not be reported as Tcl 8.6 golden evidence"
    );
    assert!(tcl86_push.evidence.fixture);
    assert!(tcl86_push.evidence.test);
    assert_eq!(
        tcl86_push.classification,
        CoverageClassification::FixtureTested,
        "Tcl 8.6 push1 executes in the checked-in version fixture corpus"
    );

    let begin_catch = matrix.row("beginCatch4").expect("beginCatch4 row");
    assert!(
        begin_catch.evidence.pass,
        "pass evidence should come from real pass/core source references"
    );
    assert!(
        matrix
            .rows
            .iter()
            .all(|row| row.classification != CoverageClassification::ExplicitlyUnsupported),
        "every current Tcl 8.6 opcode has a handler or structural recovery path"
    );
}
