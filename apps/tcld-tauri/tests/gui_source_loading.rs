#[test]
fn every_bundled_example_compiles_matching_original_source() {
    for id in ["hello", "proc", "for-break", "expr", "foreach", "catch"] {
        let result =
            tcld_tauri::load_bundled_example(id).unwrap_or_else(|error| panic!("{id}: {error}"));
        assert!(!result.instructions.is_empty(), "{id}");
        assert!(!result.decompile_text.is_empty(), "{id}");
        assert!(!result.source_map.is_empty(), "{id}");
        assert!(
            result
                .original_source_text
                .as_deref()
                .is_some_and(|source| source.contains("set ") || source.contains("proc ")),
            "{id}"
        );
        let source = result.original_source_text.as_ref().unwrap();
        let edited = format!("{source}\nset visitor_result {{edited {id}}}\n");
        let fresh = tcld_tauri::do_load_source(&edited, "edited.tcl").unwrap();
        assert_eq!(fresh.original_source_text.as_deref(), Some(edited.as_str()));
        assert!(fresh.metadata.instruction_count > result.metadata.instruction_count);
        assert_eq!(result.provenance.kind, "TclSourceText", "{id}");
        assert!(result.provenance.original_source_available, "{id}");
        assert!(
            result
                .source_map
                .iter()
                .all(|entry| entry.source_lines[0] <= entry.source_lines[1]),
            "{id}"
        );
    }
    assert!(tcld_tauri::load_bundled_example("../private").is_err());
}

#[test]
fn plain_source_load_uses_compiler_and_never_executes_commands() {
    let result = tcld_tauri::do_load_source("set value 42\nputs $value\n", "sample.tcl").unwrap();
    assert_eq!(result.metadata.filename, "sample.tcl");
    assert_eq!(result.metadata.tcl_version, "8.6");
    assert!(!result.instructions.is_empty());
    assert_eq!(
        result.original_source_text.as_deref(),
        Some("set value 42\nputs $value\n")
    );
    // This command would fail if opening source executed it.
    assert!(tcld_tauri::do_load_source("puts $definitely_not_defined\n", "no-eval.tcl").is_ok());
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tcld/tests/fixtures/hello.tcl");
    let from_file = tcld_tauri::do_load(path.to_str().unwrap()).unwrap();
    assert_eq!(from_file.provenance.kind, "Path");
    assert_eq!(from_file.metadata.filename, "hello.tcl");
    assert!(!from_file.instructions.is_empty());
}

#[test]
fn wrappers_keep_their_real_payload_and_unsupported_source_reports_admission_failure() {
    let wrapper = include_str!("../../tcld/tests/fixtures/proc.tbc");
    let result = tcld_tauri::do_load_source(wrapper, "wrapper.tcl").unwrap();
    assert!(result.instructions.len() > 1);
    assert_eq!(result.provenance.kind, "TclSourceText");
    assert_eq!(result.original_source_text.as_deref(), Some(wrapper));
    // `namespace eval` bodies compile now; loading a package still cannot.
    let error =
        tcld_tauri::do_load_source("package require hidden\n", "unsupported.tcl").unwrap_err();
    assert!(error.contains("package require"), "{error}");
    assert!(!error.contains("invalid input at byte offset"), "{error}");
}
