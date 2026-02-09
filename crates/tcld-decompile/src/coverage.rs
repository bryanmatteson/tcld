use crate::inst_table::{DEFAULT_INSTRUCTION_SET, InstructionSet, instruction_set_for_version};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

const OPCODE_HANDLER_SOURCE: &str = include_str!("decompiler/opcode_handlers.rs");
const DECOMPILER_MOD_SOURCE: &str = include_str!("decompiler/mod.rs");
const CORE_SOURCE: &str = include_str!("decompiler/core.rs");
const CATCHES_PASS_SOURCE: &str = include_str!("decompiler/passes/catches.rs");
const CONDITIONALS_PASS_SOURCE: &str = include_str!("decompiler/passes/conditionals.rs");
const DICT_UPDATE_PASS_SOURCE: &str = include_str!("decompiler/passes/dict_update.rs");
const FOREACH_PASS_SOURCE: &str = include_str!("decompiler/passes/foreach.rs");
const LOOPS_PASS_SOURCE: &str = include_str!("decompiler/passes/loops.rs");
const SWITCH_TABLES_PASS_SOURCE: &str = include_str!("decompiler/passes/switch_tables.rs");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TclVersionCategory {
    Tcl80,
    Tcl84,
    Tcl85,
    Tcl86,
    Tcl87Plus,
    Tcl90,
    Unknown,
}

impl TclVersionCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tcl80 => "Tcl 8.0",
            Self::Tcl84 => "Tcl 8.4",
            Self::Tcl85 => "Tcl 8.5",
            Self::Tcl86 => "Tcl 8.6",
            Self::Tcl87Plus => "Tcl 8.7+",
            Self::Tcl90 => "Tcl 9.0",
            Self::Unknown => "Unknown",
        }
    }

    fn instruction_set(self) -> Option<InstructionSet> {
        match self {
            Self::Tcl80 => instruction_set_for_version(8, 0),
            Self::Tcl84 => instruction_set_for_version(8, 4),
            Self::Tcl85 => instruction_set_for_version(8, 5),
            Self::Tcl86 => Some(DEFAULT_INSTRUCTION_SET),
            Self::Tcl87Plus | Self::Unknown => None,
            Self::Tcl90 => instruction_set_for_version(9, 0),
        }
    }

    pub fn all() -> [Self; 7] {
        [
            Self::Tcl80,
            Self::Tcl84,
            Self::Tcl85,
            Self::Tcl86,
            Self::Tcl87Plus,
            Self::Tcl90,
            Self::Unknown,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageClassification {
    VersionUnverified,
    Decoded,
    Rendered,
    StructurallyRecovered,
    FixtureTested,
    GoldenTested,
    ExplicitlyUnsupported,
    Unclassified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoverageEvidence {
    pub decoded: bool,
    pub handler: bool,
    pub pass: bool,
    /// Exercised end-to-end by a checked-in, version-specific bytecode fixture.
    pub fixture: bool,
    pub golden: bool,
    pub test: bool,
    pub unsupported: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CoverageMatrixConfig {
    evidence_overrides: BTreeMap<String, CoverageEvidence>,
}

impl CoverageMatrixConfig {
    pub fn with_opcode_evidence(
        mut self,
        opcode_name: impl Into<String>,
        evidence: CoverageEvidence,
    ) -> Self {
        self.evidence_overrides.insert(opcode_name.into(), evidence);
        self
    }
}

#[derive(Debug, Clone)]
pub struct CoverageRow {
    pub version_category: TclVersionCategory,
    pub opcode: i32,
    pub opcode_name: String,
    pub classification: CoverageClassification,
    pub evidence: CoverageEvidence,
}

#[derive(Debug, Clone)]
pub struct CoverageMatrix {
    pub rows: Vec<CoverageRow>,
    /// Version categories for which no verified opcode table exists.
    ///
    /// These are kept separate from `rows`: assigning another version's
    /// opcodes to an unverified category would make the report itself claim a
    /// table that the decoder correctly refuses to use.
    pub unverified_versions: Vec<TclVersionCategory>,
}

impl CoverageMatrix {
    pub fn row(&self, opcode_name: &str) -> Option<&CoverageRow> {
        self.rows.iter().find(|row| row.opcode_name == opcode_name)
    }

    pub fn rows_for_version(&self, category: TclVersionCategory) -> Vec<&CoverageRow> {
        self.rows
            .iter()
            .filter(|row| row.version_category == category)
            .collect()
    }

    pub fn version_is_unverified(&self, category: TclVersionCategory) -> bool {
        self.unverified_versions.contains(&category)
    }

    pub fn render_report(&self) -> String {
        let mut out = String::from("version\topcode\tclassification\n");
        for category in TclVersionCategory::all() {
            if self.version_is_unverified(category) {
                out.push_str(category.label());
                out.push_str("\t<table unavailable>\tVersionUnverified\n");
                continue;
            }
            for row in self
                .rows
                .iter()
                .filter(|row| row.version_category == category)
            {
                out.push_str(row.version_category.label());
                out.push('\t');
                out.push_str(&row.opcode_name);
                out.push('\t');
                out.push_str(&format!("{:?}", row.classification));
                out.push('\n');
            }
        }
        out
    }
}

pub fn generate_coverage_matrix(config: CoverageMatrixConfig) -> CoverageMatrix {
    generate_for_categories([TclVersionCategory::Tcl86], config)
}

pub fn generate_coverage_matrix_by_version(config: CoverageMatrixConfig) -> CoverageMatrix {
    generate_for_categories(TclVersionCategory::all(), config)
}

fn generate_for_categories<const N: usize>(
    categories: [TclVersionCategory; N],
    config: CoverageMatrixConfig,
) -> CoverageMatrix {
    let mut rows = Vec::new();
    let mut unverified_versions = Vec::new();
    for category in categories {
        let Some(instruction_set) = category.instruction_set() else {
            unverified_versions.push(category);
            continue;
        };
        for desc in instruction_set.instruction_table {
            let opcode_variant = instruction_set
                .opcode_from_i32(desc.opcode)
                .map(|opcode| format!("{opcode:?}"))
                .unwrap_or_else(|| desc.name.to_string());
            let mut evidence = repository_evidence(category, desc.name, &opcode_variant);
            if let Some(override_evidence) = config.evidence_overrides.get(desc.name) {
                evidence = *override_evidence;
            }
            let classification = classify(evidence);
            rows.push(CoverageRow {
                version_category: category,
                opcode: desc.opcode,
                opcode_name: desc.name.to_string(),
                classification,
                evidence,
            });
        }
    }
    CoverageMatrix {
        rows,
        unverified_versions,
    }
}

fn classify(evidence: CoverageEvidence) -> CoverageClassification {
    if evidence.unsupported {
        CoverageClassification::ExplicitlyUnsupported
    } else if evidence.golden && evidence.test {
        CoverageClassification::GoldenTested
    } else if evidence.fixture && evidence.test {
        CoverageClassification::FixtureTested
    } else if evidence.pass {
        CoverageClassification::StructurallyRecovered
    } else if evidence.handler {
        CoverageClassification::Rendered
    } else if evidence.decoded {
        CoverageClassification::Decoded
    } else {
        CoverageClassification::Unclassified
    }
}

fn repository_evidence(
    category: TclVersionCategory,
    name: &str,
    opcode_variant: &str,
) -> CoverageEvidence {
    let opcode_token = format!("Opcode::{opcode_variant}");
    let handler = production_source(OPCODE_HANDLER_SOURCE).contains(&opcode_token);
    let pass = [
        DECOMPILER_MOD_SOURCE,
        CORE_SOURCE,
        CATCHES_PASS_SOURCE,
        CONDITIONALS_PASS_SOURCE,
        DICT_UPDATE_PASS_SOURCE,
        FOREACH_PASS_SOURCE,
        LOOPS_PASS_SOURCE,
        SWITCH_TABLES_PASS_SOURCE,
    ]
    .iter()
    .any(|source| production_source(source).contains(&opcode_token));
    let pass = pass
        || matches!(
            opcode_variant,
            // These opcodes are consumed indirectly: the legacy foreach pass
            // starts from ForeachStart4, while the two stack transforms have
            // dedicated reductions before grouped opcode handlers run.
            "ForeachStep4" | "Over" | "ExpandStktop"
        );
    let golden = golden_opcode_evidence()
        .get(&category)
        .is_some_and(|evidence| evidence.contains(name));
    let fixture = executable_fixture_opcode_evidence()
        .get(&category)
        .is_some_and(|evidence| evidence.contains(name));
    // Golden evidence is end-to-end: the opcode was decoded from a fixture
    // compiled for this version category and the fixture participates in the
    // exact normalized golden comparison. Source-code token mentions are not
    // test evidence.
    let test = golden || fixture;
    let unsupported = !handler && !pass && !golden && !fixture;
    CoverageEvidence {
        decoded: true,
        handler,
        pass,
        fixture,
        golden,
        test,
        unsupported,
    }
}

fn production_source(source: &str) -> &str {
    source
        .split_once("\n#[cfg(test)]")
        .map_or(source, |(production, _)| production)
}

fn executable_fixture_opcode_evidence() -> &'static BTreeMap<TclVersionCategory, BTreeSet<String>> {
    static EVIDENCE: OnceLock<BTreeMap<TclVersionCategory, BTreeSet<String>>> = OnceLock::new();
    EVIDENCE.get_or_init(|| {
        let mut evidence: BTreeMap<TclVersionCategory, BTreeSet<String>> = BTreeMap::new();
        for fixture_dir in ["tcl8", "tcl9"] {
            let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../testing/fixtures/bytecode")
                .join(fixture_dir);
            let Ok(entries) = std::fs::read_dir(fixture_root) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("tbc") {
                    continue;
                }
                let Ok(loaded) = crate::load_bytecode(crate::LoadInput::path(&path)) else {
                    continue;
                };
                // Only admit fixtures that complete strict decompilation.
                // Merely decoding an opcode is insufficient evidence that its
                // semantics are recovered.
                if crate::decompile_loaded(&loaded).is_err() {
                    continue;
                }
                let Ok(instructions) = loaded.bytecode.get_instructions() else {
                    continue;
                };
                let category = loaded
                    .bytecode
                    .header
                    .as_ref()
                    .map(|header| {
                        category_for_version(header.tcl_version.major, header.tcl_version.minor)
                    })
                    .unwrap_or(TclVersionCategory::Unknown);
                for instruction in instructions {
                    evidence
                        .entry(category)
                        .or_default()
                        .insert(instruction.name().to_string());
                }
            }
        }
        evidence
    })
}

fn golden_opcode_evidence() -> &'static BTreeMap<TclVersionCategory, BTreeSet<String>> {
    static EVIDENCE: OnceLock<BTreeMap<TclVersionCategory, BTreeSet<String>>> = OnceLock::new();
    EVIDENCE.get_or_init(|| {
        let fixture_root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/tcld/tests/fixtures");
        let mut evidence: BTreeMap<TclVersionCategory, BTreeSet<String>> = BTreeMap::new();
        let Ok(entries) = std::fs::read_dir(fixture_root) else {
            return evidence;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("tbc") {
                continue;
            }
            let Ok(loaded) = crate::load_bytecode(crate::LoadInput::path(&path)) else {
                continue;
            };
            let Ok(instructions) = loaded.bytecode.get_instructions() else {
                continue;
            };
            let category = loaded
                .bytecode
                .header
                .as_ref()
                .map(|header| {
                    category_for_version(header.tcl_version.major, header.tcl_version.minor)
                })
                .unwrap_or(TclVersionCategory::Unknown);
            for instruction in instructions {
                evidence
                    .entry(category)
                    .or_default()
                    .insert(instruction.name().to_string());
            }
        }
        evidence
    })
}

fn category_for_version(major: i32, minor: i32) -> TclVersionCategory {
    match (major, minor) {
        (8, 0..=3) => TclVersionCategory::Tcl80,
        (8, 4) => TclVersionCategory::Tcl84,
        (8, 5) => TclVersionCategory::Tcl85,
        (8, 6) => TclVersionCategory::Tcl86,
        (8, 7..) => TclVersionCategory::Tcl87Plus,
        (9, 0) => TclVersionCategory::Tcl90,
        _ => TclVersionCategory::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_labels_sets_and_version_mapping_are_complete() {
        let expected = [
            (TclVersionCategory::Tcl80, "Tcl 8.0"),
            (TclVersionCategory::Tcl84, "Tcl 8.4"),
            (TclVersionCategory::Tcl85, "Tcl 8.5"),
            (TclVersionCategory::Tcl86, "Tcl 8.6"),
            (TclVersionCategory::Tcl87Plus, "Tcl 8.7+"),
            (TclVersionCategory::Tcl90, "Tcl 9.0"),
            (TclVersionCategory::Unknown, "Unknown"),
        ];
        assert_eq!(TclVersionCategory::all().len(), expected.len());
        for (category, label) in expected {
            assert_eq!(category.label(), label);
            if matches!(
                category,
                TclVersionCategory::Tcl87Plus | TclVersionCategory::Unknown
            ) {
                assert!(category.instruction_set().is_none());
            } else {
                assert!(
                    !category
                        .instruction_set()
                        .expect("verified category")
                        .instruction_table
                        .is_empty()
                );
            }
        }

        assert_eq!(category_for_version(8, 0), TclVersionCategory::Tcl80);
        assert_eq!(category_for_version(8, 3), TclVersionCategory::Tcl80);
        assert_eq!(category_for_version(8, 4), TclVersionCategory::Tcl84);
        assert_eq!(category_for_version(8, 5), TclVersionCategory::Tcl85);
        assert_eq!(category_for_version(8, 6), TclVersionCategory::Tcl86);
        assert_eq!(category_for_version(8, 7), TclVersionCategory::Tcl87Plus);
        assert_eq!(category_for_version(9, 0), TclVersionCategory::Tcl90);
        assert_eq!(category_for_version(9, 1), TclVersionCategory::Unknown);
        assert_eq!(category_for_version(10, 0), TclVersionCategory::Unknown);
    }

    #[test]
    fn evidence_classification_precedence_is_explicit() {
        let evidence =
            |decoded, handler, pass, fixture, golden, test, unsupported| CoverageEvidence {
                decoded,
                handler,
                pass,
                fixture,
                golden,
                test,
                unsupported,
            };
        assert_eq!(
            classify(evidence(false, false, false, false, false, false, true)),
            CoverageClassification::ExplicitlyUnsupported
        );
        assert_eq!(
            classify(evidence(true, true, true, true, true, true, false)),
            CoverageClassification::GoldenTested
        );
        assert_eq!(
            classify(evidence(true, true, true, true, false, true, false)),
            CoverageClassification::FixtureTested
        );
        assert_eq!(
            classify(evidence(true, true, true, false, false, false, false)),
            CoverageClassification::StructurallyRecovered
        );
        assert_eq!(
            classify(evidence(true, true, false, false, false, false, false)),
            CoverageClassification::Rendered
        );
        assert_eq!(
            classify(evidence(true, false, false, false, false, false, false)),
            CoverageClassification::Decoded
        );
        assert_eq!(
            classify(CoverageEvidence::default()),
            CoverageClassification::Unclassified
        );
    }

    #[test]
    fn lexical_evidence_ignores_test_only_opcode_mentions() {
        let source =
            "fn production() { Opcode::Done; }\n#[cfg(test)]\nmod tests { Opcode::Yield; }";
        let production = production_source(source);
        assert!(production.contains("Opcode::Done"));
        assert!(!production.contains("Opcode::Yield"));
    }

    #[test]
    fn matrix_override_lookup_filter_and_report_are_consistent() {
        let override_evidence = CoverageEvidence {
            decoded: true,
            handler: true,
            test: true,
            golden: true,
            ..Default::default()
        };
        let matrix = generate_coverage_matrix(
            CoverageMatrixConfig::default().with_opcode_evidence("push1", override_evidence),
        );
        let push = matrix.row("push1").expect("push row");
        assert_eq!(push.evidence, override_evidence);
        assert_eq!(push.classification, CoverageClassification::GoldenTested);
        assert!(matrix.row("not-an-opcode").is_none());
        assert_eq!(
            matrix.rows_for_version(TclVersionCategory::Tcl86).len(),
            matrix.rows.len()
        );
        let report = matrix.render_report();
        assert!(report.starts_with("version\topcode\tclassification\n"));
        assert!(report.contains("Tcl 8.6\tpush1\tGoldenTested\n"));
    }
}
