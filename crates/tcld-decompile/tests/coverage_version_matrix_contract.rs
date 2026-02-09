use tcld_decompile::{
    CoverageClassification, CoverageMatrixConfig, TclVersionCategory,
    generate_coverage_matrix_by_version,
};

#[test]
fn all_tcl_version_categories_are_reported_independently() {
    let matrix = generate_coverage_matrix_by_version(CoverageMatrixConfig::default());
    for category in [
        TclVersionCategory::Tcl80,
        TclVersionCategory::Tcl84,
        TclVersionCategory::Tcl85,
        TclVersionCategory::Tcl86,
        TclVersionCategory::Tcl90,
    ] {
        let rows = matrix.rows_for_version(category);
        assert!(!rows.is_empty(), "{category:?} should have coverage rows");
        assert!(
            rows.iter().all(|row| row.version_category == category),
            "{category:?} rows must stay independently classified"
        );
    }
    for category in [TclVersionCategory::Tcl87Plus, TclVersionCategory::Unknown] {
        assert!(
            matrix.rows_for_version(category).is_empty(),
            "{category:?} must not fabricate opcode rows from the Tcl 8.6 table"
        );
        assert!(
            matrix.version_is_unverified(category),
            "{category:?} must remain explicitly represented as unverified"
        );
    }
    let report = matrix.render_report();
    assert!(report.contains("Tcl 8.7+\t<table unavailable>\tVersionUnverified"));
    assert!(report.contains("Unknown\t<table unavailable>\tVersionUnverified"));
    let tcl9_rows = matrix.rows_for_version(TclVersionCategory::Tcl90);
    assert!(
        tcl9_rows
            .iter()
            .any(|row| row.classification != CoverageClassification::VersionUnverified),
        "Tcl 9 must use its version-specific opcode and fixture evidence"
    );
    assert!(
        tcl9_rows.iter().any(|row| row.opcode_name == "dictGetDef"),
        "Tcl 9-only opcodes must appear in the Tcl 9 coverage rows"
    );
    for opcode_name in [
        "dictGetDef",
        "strlt",
        "strgt",
        "strle",
        "strge",
        "lreplace4",
        "constImm",
        "constStk",
    ] {
        let row = tcl9_rows
            .iter()
            .find(|row| row.opcode_name == opcode_name)
            .unwrap_or_else(|| panic!("missing Tcl 9 coverage row for {opcode_name}"));
        assert!(row.evidence.handler, "{opcode_name} must have a renderer");
        assert_ne!(
            row.classification,
            CoverageClassification::ExplicitlyUnsupported,
            "{opcode_name} must not regress to unsupported"
        );
    }
}

#[test]
fn verified_opcode_tables_remain_above_ninety_five_percent_coverage() {
    let matrix = generate_coverage_matrix_by_version(CoverageMatrixConfig::default());
    let rows = matrix
        .rows
        .iter()
        .filter(|row| {
            matches!(
                row.version_category,
                TclVersionCategory::Tcl80
                    | TclVersionCategory::Tcl84
                    | TclVersionCategory::Tcl85
                    | TclVersionCategory::Tcl86
                    | TclVersionCategory::Tcl90
            )
        })
        .collect::<Vec<_>>();
    let covered = rows
        .iter()
        .filter(|row| {
            !matches!(
                row.classification,
                CoverageClassification::VersionUnverified
                    | CoverageClassification::ExplicitlyUnsupported
                    | CoverageClassification::Unclassified
            )
        })
        .count();
    let ratio = covered as f64 / rows.len() as f64;
    assert!(
        ratio > 0.95,
        "verified opcode coverage regressed to {covered}/{} ({:.2}%)",
        rows.len(),
        ratio * 100.0
    );
}
