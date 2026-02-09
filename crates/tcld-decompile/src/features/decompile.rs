use crate::decompiler::Decompiler;
use crate::error::Result;
use crate::loader::{LoadInput, LoadedBytecode, load_bytecode, load_bytecode_from_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DecompileOptions;

pub fn decompile_path(path: &str) -> Result<String> {
    decompile_path_with_options(path, DecompileOptions)
}

pub fn decompile_path_with_options(path: &str, _options: DecompileOptions) -> Result<String> {
    let mut output = decompile_with_default_parser(path)?;

    if !output.ends_with('\n') {
        output.push('\n');
    }

    Ok(output)
}

fn decompile_with_default_parser(path: &str) -> Result<String> {
    let bytecode = load_bytecode_from_path(path)?;
    let decompiler = Decompiler::new(bytecode)?;
    decompiler.get_source()
}

pub fn decompile_loaded(loaded: &LoadedBytecode) -> Result<String> {
    Decompiler::new(loaded.bytecode.clone())?.get_source()
}

// StrictDecompileError intentionally carries the full DiagnosticsReport
// gathered before failure (documented contract); boxing it is a future
// API change, not a release-prep edit.
#[allow(clippy::result_large_err)]
pub fn decompile_input_strict_with_diagnostics(
    input: LoadInput,
) -> std::result::Result<
    crate::diagnostics::DecompileWithDiagnostics,
    crate::diagnostics::StrictDecompileError,
> {
    let loaded =
        load_bytecode(input).map_err(|source| crate::diagnostics::StrictDecompileError {
            source,
            diagnostics: crate::diagnostics::DiagnosticsReport {
                reconstruction: crate::diagnostics::ReconstructionStatus::Failed,
                suggested_next_work: vec!["fix load input or payload extraction".to_string()],
                ..Default::default()
            },
        })?;
    Decompiler::new(loaded.bytecode)
        .map_err(|source| crate::diagnostics::StrictDecompileError {
            source,
            diagnostics: crate::diagnostics::DiagnosticsReport {
                reconstruction: crate::diagnostics::ReconstructionStatus::Failed,
                suggested_next_work: vec!["fix bytecode instruction decoding".to_string()],
                ..Default::default()
            },
        })?
        .get_output_with_diagnostics()
}

pub fn decompile_input_approximate_with_diagnostics(
    input: LoadInput,
) -> Result<crate::diagnostics::DecompileWithDiagnostics> {
    let loaded = load_bytecode(input)?;
    Decompiler::new(loaded.bytecode)?.get_output_approximate_with_diagnostics()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/tcld/tests/fixtures/expr.tbc")
    }

    #[test]
    fn path_loaded_and_diagnostic_entrypoints_agree() {
        let path = fixture();
        let path_str = path.to_str().expect("UTF-8 fixture path");
        let direct = decompile_path(path_str).expect("decompile path");
        assert!(direct.ends_with('\n'));
        assert_eq!(
            decompile_path_with_options(path_str, DecompileOptions).unwrap(),
            direct
        );

        let loaded = load_bytecode(LoadInput::path(path)).expect("load fixture");
        assert_eq!(format!("{}\n", decompile_loaded(&loaded).unwrap()), direct);

        let strict = decompile_input_strict_with_diagnostics(LoadInput::raw_bytes(
            std::fs::read(fixture()).expect("fixture bytes"),
            "expr fixture",
        ))
        .expect("strict diagnostics");
        let approximation =
            decompile_input_approximate_with_diagnostics(LoadInput::path(fixture()))
                .expect("approximation diagnostics");
        assert_eq!(strict.output.text, approximation.output.text);
    }

    #[test]
    fn strict_load_failures_include_actionable_diagnostics() {
        let error = decompile_input_strict_with_diagnostics(LoadInput::raw_bytes(
            b"not bytecode",
            "invalid",
        ))
        .expect_err("invalid payload");
        assert_eq!(
            error.diagnostics.reconstruction,
            crate::diagnostics::ReconstructionStatus::Failed
        );
        assert!(
            error
                .diagnostics
                .suggested_next_work
                .iter()
                .any(|work| work.contains("load input"))
        );
    }
}
