use std::ops::Range;
use std::sync::Arc;
use tcld_decompile::bytecode::ByteCode;
use tcld_decompile::code_graph::CodeGraph;
use tcld_decompile::decompiler::{DecompilePipelineConfig, Decompiler};
use tcld_decompile::{DecompilerPass, LoadInput, PassRegistry};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/tcld/tests/fixtures")
        .join(name)
}

struct MarkerPass {
    name: &'static str,
    priority: u32,
}

struct FailingPass;

impl DecompilerPass for FailingPass {
    fn name(&self) -> &str {
        "failing"
    }

    fn run(
        &self,
        _graph: &mut CodeGraph,
        _bytecode: &ByteCode,
        _range: Range<i32>,
    ) -> tcld_decompile::error::Result<bool> {
        Err(tcld_decompile::error::TcldError::Other(
            "intentional pass failure".to_string(),
        ))
    }
}

struct NonConvergingPass;

impl DecompilerPass for NonConvergingPass {
    fn name(&self) -> &str {
        "non-converging"
    }

    fn run(
        &self,
        _graph: &mut CodeGraph,
        _bytecode: &ByteCode,
        _range: Range<i32>,
    ) -> tcld_decompile::error::Result<bool> {
        Ok(true)
    }

    fn is_fixpoint(&self) -> bool {
        true
    }
}

impl DecompilerPass for MarkerPass {
    fn name(&self) -> &str {
        self.name
    }

    fn run(
        &self,
        _graph: &mut CodeGraph,
        _bytecode: &ByteCode,
        _range: Range<i32>,
    ) -> tcld_decompile::error::Result<bool> {
        Ok(true)
    }

    fn priority(&self) -> u32 {
        self.priority
    }
}

#[test]
fn builtins_custom_order_disabling_and_fixpoint_iterations_are_traced() {
    let loaded = tcld_decompile::load_bytecode(LoadInput::path(fixture("foreach.tbc")))
        .expect("fixture loads");
    let mut registry = PassRegistry::new();
    registry.register(Arc::new(MarkerPass {
        name: "custom-before-catches",
        priority: 350,
    }));
    registry.register(Arc::new(MarkerPass {
        name: "custom-after-loops",
        priority: 850,
    }));

    let config = DecompilePipelineConfig::default().with_custom_registry(registry);
    let report = Decompiler::new_with_config(loaded.bytecode.clone(), config)
        .expect("decompiler")
        .get_output_with_diagnostics()
        .expect("decompile");
    let names = report
        .diagnostics
        .pass_decisions
        .iter()
        .map(|decision| decision.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.starts_with(&["instructions"]));
    let catches = names.iter().position(|name| *name == "catches").unwrap();
    let custom_before = names
        .iter()
        .position(|name| *name == "custom-before-catches")
        .unwrap();
    assert!(custom_before < catches);
    assert!(names.contains(&"loops"));
    assert!(
        report
            .diagnostics
            .pass_decisions
            .iter()
            .any(|decision| decision.name == "loops" && decision.iteration > 0),
        "fixpoint passes should trace iterations"
    );

    let disabled = DecompilePipelineConfig::default().disable_pass("foreach");
    let disabled_report = Decompiler::new_with_config(loaded.bytecode, disabled)
        .expect("decompiler")
        .get_output_approximate_with_diagnostics()
        .expect("approximation");
    assert!(
        disabled_report
            .diagnostics
            .pass_decisions
            .iter()
            .any(|decision| decision.name == "foreach" && !decision.enabled)
    );
    assert!(
        disabled_report.output.text != report.output.text
            || disabled_report.diagnostics.pass_decisions.len()
                != report.diagnostics.pass_decisions.len(),
        "disabling a built-in pass should affect the executed trace or output"
    );
}

#[test]
fn custom_pass_failures_and_non_convergence_are_errors() {
    let loaded =
        tcld_decompile::load_bytecode(LoadInput::path(fixture("expr.tbc"))).expect("fixture loads");

    let mut failing = PassRegistry::new();
    failing.register(Arc::new(FailingPass));
    let error = Decompiler::new_with_config(
        loaded.bytecode.clone(),
        DecompilePipelineConfig::default().with_custom_registry(failing),
    )
    .expect("decompiler")
    .get_output()
    .expect_err("custom pass error must propagate");
    assert!(error.to_string().contains("intentional pass failure"));

    let mut non_converging = PassRegistry::new();
    non_converging.register(Arc::new(NonConvergingPass));
    let error = Decompiler::new_with_config(
        loaded.bytecode,
        DecompilePipelineConfig::default().with_custom_registry(non_converging),
    )
    .expect("decompiler")
    .get_output()
    .expect_err("non-converging pass must be bounded");
    assert!(error.to_string().contains("fixpoint iterations"));
}
