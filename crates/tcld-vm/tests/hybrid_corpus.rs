use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tcld_vm::{HybridEngine, RuntimeMode, VmError, VmObj};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    synthetic_proof: Vec<SyntheticProof>,
    scenario: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SyntheticProof {
    group: String,
    test: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    id: String,
    kind: String,
    path: String,
    mode: String,
    invoke: Option<String>,
    #[serde(default)]
    setup: Vec<SetupAction>,
    args: Vec<String>,
    expect_code: i32,
    expect_result: String,
    expect_stdout: String,
    expect_stderr: String,
    expect_error_code: Option<String>,
    expect_error_info_contains: Option<String>,
    #[serde(default)]
    required_opcodes: Vec<String>,
    #[serde(default)]
    forbidden_classes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupAction {
    kind: String,
    script: String,
}

#[derive(Debug, Serialize)]
struct CorpusReport {
    opcode_total: usize,
    vm_count: usize,
    host_value_count: usize,
    host_frame_count: usize,
    trap_count: usize,
    scenario_total: usize,
    scenario_passed: usize,
    scenario_failed: usize,
    body_scenario_total: usize,
    body_scenario_passed: usize,
    executed_instruction_total: u64,
    executed_by_opcode: BTreeMap<String, u64>,
    executed_by_class: BTreeMap<String, u64>,
    first_failure_by_scenario: BTreeMap<String, String>,
    host_command_invocations: u64,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn pinned_hybrid_corpus_matches_exact_oracles_and_report_schema() {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hybrid-corpus.toml");
    let manifest: Manifest = toml::from_str(
        &std::fs::read_to_string(&manifest_path).expect("read hybrid corpus manifest"),
    )
    .expect("parse hybrid corpus manifest");
    assert_eq!(manifest.version, 1);
    assert!(!manifest.scenario.is_empty());
    let proof_groups = manifest
        .synthetic_proof
        .iter()
        .map(|proof| proof.group.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        proof_groups,
        [
            "catch_completion",
            "dict_core",
            "expand",
            "foreach_old_new",
            "jump_tables",
            "namespace",
            "pass_f1_f12",
            "semantic_failures",
            "variables",
        ]
        .into_iter()
        .collect()
    );
    assert!(
        manifest
            .synthetic_proof
            .iter()
            .all(|proof| !proof.test.trim().is_empty())
    );
    let fixture_source = include_str!("hybrid_fixtures.rs");
    for proof in &manifest.synthetic_proof {
        for test in proof.test.split(',').map(str::trim) {
            assert!(
                fixture_source.contains(&format!("fn {test}()")),
                "synthetic proof {} names missing test {test}",
                proof.group
            );
        }
    }

    let mut report = CorpusReport {
        opcode_total: 0,
        vm_count: 0,
        host_value_count: 0,
        host_frame_count: 0,
        trap_count: 0,
        scenario_total: manifest.scenario.len(),
        scenario_passed: 0,
        scenario_failed: 0,
        body_scenario_total: 0,
        body_scenario_passed: 0,
        executed_instruction_total: 0,
        executed_by_opcode: BTreeMap::new(),
        executed_by_class: BTreeMap::new(),
        first_failure_by_scenario: BTreeMap::new(),
        host_command_invocations: 0,
    };

    for scenario in &manifest.scenario {
        assert_eq!(scenario.mode, "hybrid", "{} mode", scenario.id);
        if scenario.kind == "body" {
            report.body_scenario_total += 1;
            assert!(
                scenario.invoke.is_some(),
                "body row {} lacks invoke",
                scenario.id
            );
        } else {
            assert_eq!(scenario.kind, "top_level", "{} kind", scenario.id);
        }

        let mut engine = HybridEngine::new(RuntimeMode::Hybrid).expect("engine");
        engine.bootstrap(Some("hybrid-corpus")).expect("bootstrap");
        for setup in &scenario.setup {
            assert_eq!(setup.kind, "host_eval", "{} setup kind", scenario.id);
            let completion = engine
                .host_setup_eval(&setup.script)
                .unwrap_or_else(|error| panic!("{} setup: {error}", scenario.id));
            assert_eq!(completion.code, 0, "{} setup completion", scenario.id);
        }
        engine
            .begin_output_capture()
            .expect("begin scenario stdout/stderr capture");
        let path = workspace_root().join(&scenario.path);
        let top = engine.run_tbc_path(&path);
        if scenario.path == "testing/fixtures/public/public_complex_fixture.tbc" {
            assert_eq!(engine.vm_proc_count(), 8, "{} registrations", scenario.id);
        }
        let completion = match (&scenario.invoke, top) {
            (Some(name), Ok(_)) => engine.invoke_registered(
                name.as_bytes(),
                &scenario
                    .args
                    .iter()
                    .map(|arg| VmObj::from_str(arg))
                    .collect::<Vec<_>>(),
            ),
            (None, Ok(value)) => Ok(tcld_vm::Completion::ok(value)),
            (_, Err(error)) => Err(error),
        };
        let (code, result, error_code, error_info) = match completion {
            Ok(completion) => (
                completion.code,
                completion.result.as_string().unwrap_or_default(),
                completion
                    .options
                    .error_code
                    .and_then(|value| value.as_string().ok()),
                completion
                    .options
                    .error_info
                    .map(|value| value.as_str_lossy().into_owned()),
            ),
            Err(VmError::TclError {
                message,
                error_code,
                error_info,
            }) => (1, message, error_code, error_info),
            Err(error) => (error.exit_code(), error.to_string(), None, None),
        };
        let (stdout, stderr) = engine.captured_output().expect("captured output");
        let stdout = String::from_utf8(stdout).expect("scenario stdout is UTF-8");
        let stderr = String::from_utf8(stderr).expect("scenario stderr is UTF-8");

        let coverage = engine.coverage_snapshot();
        report.opcode_total = coverage.opcode_total;
        report.vm_count = coverage.vm_count;
        report.host_value_count = coverage.host_value_count;
        report.host_frame_count = coverage.host_frame_count;
        report.trap_count = coverage.trap_count;
        report.executed_instruction_total += coverage.executed_instruction_total;
        report.host_command_invocations += coverage.host_command_invocations;
        for (opcode, count) in &coverage.executed_by_opcode {
            *report.executed_by_opcode.entry(opcode.clone()).or_default() += count;
        }
        for (class, count) in &coverage.executed_by_class {
            *report.executed_by_class.entry(class.clone()).or_default() += count;
        }

        let mut failure = None;
        if code != scenario.expect_code
            || result != scenario.expect_result
            || stdout != scenario.expect_stdout
            || stderr != scenario.expect_stderr
            || scenario.expect_error_code.as_ref().is_some_and(|expected| {
                if expected.is_empty() {
                    error_code.is_some()
                } else {
                    error_code.as_deref() != Some(expected)
                }
            })
            || scenario
                .expect_error_info_contains
                .as_ref()
                .is_some_and(|expected| {
                    if expected.is_empty() {
                        error_info.is_some()
                    } else {
                        !error_info
                            .as_deref()
                            .is_some_and(|actual| actual.contains(expected))
                    }
                })
        {
            failure = Some(format!(
                "expected code={} result={:?} stdout={:?} stderr={:?} error_code={:?} error_info~={:?}; got code={code} result={result:?} stdout={stdout:?} stderr={stderr:?} error_code={error_code:?} error_info={error_info:?}",
                scenario.expect_code,
                scenario.expect_result,
                scenario.expect_stdout,
                scenario.expect_stderr,
                scenario.expect_error_code,
                scenario.expect_error_info_contains,
            ));
        }
        for opcode in &scenario.required_opcodes {
            if coverage
                .executed_by_opcode
                .get(opcode)
                .copied()
                .unwrap_or(0)
                == 0
            {
                failure = Some(format!("required opcode {opcode} was not observed"));
            }
        }
        for class in &scenario.forbidden_classes {
            if coverage.executed_by_class.get(class).copied().unwrap_or(0) != 0 {
                failure = Some(format!("forbidden class {class} was observed"));
            }
        }
        if let Some(failure) = failure {
            report.scenario_failed += 1;
            report
                .first_failure_by_scenario
                .insert(scenario.id.clone(), failure);
        } else {
            report.scenario_passed += 1;
            if scenario.kind == "body" {
                report.body_scenario_passed += 1;
            }
        }
    }

    if let Ok(path) = std::env::var("TCLD_CORPUS_REPORT") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&report).expect("serialize report"),
        )
        .expect("write corpus report");
    }
    assert_eq!(report.opcode_total, 190);
    assert_eq!(
        report.scenario_failed, 0,
        "{:#?}",
        report.first_failure_by_scenario
    );
    assert_eq!(report.scenario_passed, report.scenario_total);
    assert_eq!(report.body_scenario_passed, report.body_scenario_total);
}

#[test]
fn full_app_fixture_sweep_has_only_pinned_semantic_boundaries() {
    let fixture_dir = workspace_root().join("apps/tcld/tests/fixtures");
    let mut paths = std::fs::read_dir(&fixture_dir)
        .expect("read app fixture directory")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "tbc"))
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(paths.len(), 37, "pinned app fixture count changed");

    let mut failures = BTreeMap::new();
    for path in paths {
        let mut engine = HybridEngine::new(RuntimeMode::Hybrid).expect("engine");
        engine.bootstrap(Some("fixture-sweep")).expect("bootstrap");
        if let Err(error) = engine.run_tbc_path(&path) {
            failures.insert(
                path.file_name()
                    .expect("fixture name")
                    .to_string_lossy()
                    .into_owned(),
                (error.exit_code(), error.to_string()),
            );
        }
    }

    let expected = BTreeMap::from([
        (
            "break1.tbc".into(),
            (1, "can't read \"top\": no such variable".into()),
        ),
        (
            "interp.tbc".into(),
            (1, "invalid command name \"test\"".into()),
        ),
        ("procbod1.tbc".into(), (1, "\"a\" isn't a procedure".into())),
        ("procbod2.tbc".into(), (1, "\"a\" isn't a procedure".into())),
        ("procbod3.tbc".into(), (1, "\"a\" isn't a procedure".into())),
        (
            "procshd2.tbc".into(),
            (1, "can't read \"x\": no such variable".into()),
        ),
        (
            "procvar1.tbc".into(),
            (1, "can't read \"x\": no such variable".into()),
        ),
    ]);
    assert_eq!(failures, expected);
}
