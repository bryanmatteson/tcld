use crate::error::Result;
use crate::{DecompileOptions, decompile_path_with_options};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FixtureHarnessConfig {
    pub fixture_root: PathBuf,
    /// Compiler command invoked as `<command> <source.tcl> <output.tbc>` when
    /// `regenerate` is true.
    pub compiler_command: Option<String>,
    pub regenerate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegenerationStatus {
    Available { command: String },
    Unavailable { reason: String },
}

#[derive(Debug, Clone)]
pub struct FixtureHarnessReport {
    pub fixtures: Vec<FixtureResult>,
    pub regeneration_status: RegenerationStatus,
}

#[derive(Debug, Clone)]
pub struct FixtureResult {
    pub tcl_path: PathBuf,
    pub tbc_path: PathBuf,
    pub normalized_match: bool,
    pub diagnostics: Vec<String>,
    pub round_trip_status: String,
}

pub fn run_golden_fixture_harness(config: FixtureHarnessConfig) -> Result<FixtureHarnessReport> {
    let regeneration_status = match &config.compiler_command {
        Some(command) => RegenerationStatus::Available {
            command: command.clone(),
        },
        None => RegenerationStatus::Unavailable {
            reason: "no compiler command configured".to_string(),
        },
    };
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&config.fixture_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("tbc") {
            continue;
        }
        paths.push(path);
    }
    paths.sort();

    let mut fixtures = Vec::new();
    for path in paths {
        let tcl_path = path.with_extension("tcl");
        if !tcl_path.exists() {
            continue;
        }
        let regenerated = if config.regenerate {
            let command = config.compiler_command.as_deref().ok_or_else(|| {
                crate::error::TcldError::Other(
                    "fixture regeneration requested without a compiler command".to_string(),
                )
            })?;
            let status = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("{command} \"$1\" \"$2\""))
                .arg("tcld-fixture-compiler")
                .arg(&tcl_path)
                .arg(&path)
                .status()?;
            if !status.success() {
                return Err(crate::error::TcldError::Other(format!(
                    "fixture compiler failed for {} with status {status}",
                    tcl_path.display()
                )));
            }
            true
        } else {
            false
        };
        let decompile_text =
            decompile_path_with_options(path.to_str().unwrap_or_default(), DecompileOptions)?;
        let expected = std::fs::read_to_string(&tcl_path)?;
        let actual_norm = normalize_tcl(&decompile_text);
        let expected_norm = normalize_tcl(&expected);
        fixtures.push(FixtureResult {
            tcl_path,
            tbc_path: path,
            normalized_match: !actual_norm.is_empty() && actual_norm == expected_norm,
            diagnostics: vec!["reconstruction: harness-decompiled".to_string()],
            round_trip_status: if regenerated {
                "compiler-round-trip-executed".to_string()
            } else {
                "deterministic-normalize-no-compiler".to_string()
            },
        });
    }
    fixtures.sort_by(|left, right| left.tbc_path.cmp(&right.tbc_path));
    Ok(FixtureHarnessReport {
        fixtures,
        regeneration_status,
    })
}

fn normalize_tcl(source: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/tcld/tests/fixtures")
    }

    fn temp_fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tcld-fixture-harness-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("temporary fixture directory");
        for extension in ["tbc", "tcl"] {
            std::fs::copy(
                fixture_root().join(format!("expr.{extension}")),
                root.join(format!("expr.{extension}")),
            )
            .expect("copy fixture");
        }
        root
    }

    #[test]
    fn harness_discovers_and_normalizes_repository_fixtures() {
        let report = run_golden_fixture_harness(FixtureHarnessConfig {
            fixture_root: fixture_root(),
            compiler_command: None,
            regenerate: false,
        })
        .expect("fixture harness");

        assert!(!report.fixtures.is_empty());
        assert!(matches!(
            report.regeneration_status,
            RegenerationStatus::Unavailable { .. }
        ));
        assert!(report.fixtures.iter().all(|fixture| {
            !fixture.diagnostics.is_empty()
                && fixture.round_trip_status == "deterministic-normalize-no-compiler"
        }));
    }

    #[test]
    fn regeneration_requires_and_reports_compiler_status() {
        let root = temp_fixture();
        let missing = run_golden_fixture_harness(FixtureHarnessConfig {
            fixture_root: root.clone(),
            compiler_command: None,
            regenerate: true,
        })
        .expect_err("regeneration without a compiler must fail");
        assert!(missing.to_string().contains("without a compiler command"));

        let failed = run_golden_fixture_harness(FixtureHarnessConfig {
            fixture_root: root.clone(),
            compiler_command: Some("false".to_string()),
            regenerate: true,
        })
        .expect_err("failed compiler status must propagate");
        assert!(failed.to_string().contains("fixture compiler failed"));

        let report = run_golden_fixture_harness(FixtureHarnessConfig {
            fixture_root: root.clone(),
            compiler_command: Some("true".to_string()),
            regenerate: true,
        })
        .expect("successful compiler command");
        assert!(matches!(
            report.regeneration_status,
            RegenerationStatus::Available { ref command } if command == "true"
        ));
        assert_eq!(report.fixtures.len(), 1);
        assert_eq!(
            report.fixtures[0].round_trip_status,
            "compiler-round-trip-executed"
        );

        std::fs::remove_dir_all(root).expect("remove temporary fixture directory");
    }

    #[test]
    fn normalization_removes_comments_blanks_and_platform_newlines() {
        assert_eq!(
            normalize_tcl("# heading\r\n\rset a 1  \r# trailing\rset b 2\r"),
            "set a 1\nset b 2"
        );
    }
}
