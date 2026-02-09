use tcld_decompile::{FixtureHarnessConfig, RegenerationStatus, run_golden_fixture_harness};

#[test]
fn fixture_harness_discovers_compares_and_reports_regeneration_capability() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let report = run_golden_fixture_harness(FixtureHarnessConfig {
        fixture_root: root,
        compiler_command: None,
        regenerate: false,
    })
    .expect("harness runs");

    assert!(report.fixtures.len() > 10);
    assert!(
        report
            .fixtures
            .iter()
            .all(|fixture| fixture.tbc_path.exists())
    );
    assert!(
        report
            .fixtures
            .iter()
            .any(|fixture| fixture.normalized_match)
    );
    assert_eq!(
        report.regeneration_status,
        RegenerationStatus::Unavailable {
            reason: "no compiler command configured".to_string()
        }
    );
    assert!(report.fixtures.iter().all(|fixture| {
        fixture
            .round_trip_status
            .contains("deterministic-normalize")
    }));
}
