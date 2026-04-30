fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tcld/tests/fixtures")
        .join(name)
}

#[test]
fn tauri_file_snapshot_contract_file_dto_is_constructed_from_canonical_snapshot_facts() {
    let dto = tcld_tauri::do_load(fixture("expr.tbc").to_str().unwrap())
        .expect("production file load dto");
    assert_eq!(dto.metadata.filename, "expr.tbc");
    assert!(dto.metadata.instruction_count > 0);
    assert!(!dto.source_map.is_empty());
    assert_eq!(dto.provenance.kind, "Path");
    assert!(dto.provenance.original_source_available);
    assert!(
        dto.original_source_text
            .as_deref()
            .is_some_and(|text| text.contains("Test script"))
    );
    assert_eq!(dto.diagnostics.reconstruction, dto.reconstruction);
    assert!(!dto.diagnostics.pass_decisions.is_empty());
    assert_eq!(dto.reconstruction, "Strict");
    assert!(!dto.traces.is_empty());
    assert!(dto.dump_text.contains("Instructions:"));
    assert!(
        dto.decompile_folding_ranges
            .iter()
            .all(|range| range.end_row >= range.start_row)
    );
    assert!(dto.decompile_lines.iter().any(|line| line.group.is_some()));
    assert!(
        dto.decompile_lines
            .iter()
            .any(|line| line.source_map_index.is_some())
    );
    let json = serde_json::to_value(&dto).expect("serializes as Tauri command payload");
    assert!(json.get("sourceMap").is_some());
    assert!(json.get("decompileFoldingRanges").is_some());
    assert!(json.get("diagnostics").is_some());
    assert!(json.get("reconstruction").is_some());
    assert!(json.get("provenance").is_some());
    assert!(json.get("originalSourceText").is_some());

    let grouped = dto
        .instructions
        .iter()
        .find(|inst| inst.group.is_some())
        .expect("grouped instruction");
    let command_range = grouped
        .command_pc_range
        .expect("command pc range is present on grouped instructions");
    assert!(command_range[0] <= grouped.pc && grouped.pc < command_range[1]);

    let jump = dto
        .instructions
        .iter()
        .find(|inst| inst.is_jump)
        .expect("branch instruction");
    assert_eq!(
        jump.is_jump,
        jump.is_jump_conditional || jump.is_jump_unconditional
    );
    assert_ne!(jump.is_jump_conditional, jump.is_jump_unconditional);
    assert!(jump.next_locations.contains(&jump.jump_target.unwrap()));
    assert_eq!(jump.fallthrough.is_some(), jump.is_jump_conditional);
}

#[test]
fn bcproc_source_command_anchors_are_serialized_for_gui() {
    let dto = tcld_tauri::do_load(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tcld/tests/fixtures/../../../../testing/fixtures/public/public_complex_fixture.tbc")
            .canonicalize()
            .expect("fixture fixture path")
            .to_str()
            .unwrap(),
    )
    .expect("fixture file load dto");

    let startlog = dto
        .source_command_anchors
        .iter()
        .find(|anchor| {
            anchor.kind == "root" && anchor.snippet.starts_with("proc fixture::startlog ")
        })
        .expect("fixture::startlog root anchor");
    let root_group = startlog
        .parent_root_group
        .expect("root anchor carries root command group");
    assert!(
        dto.decompile_folding_ranges
            .iter()
            .any(|range| range.kind == "region" && range.end_row > range.start_row),
        "fixture decompile source exposes syntax folding ranges for GUI"
    );

    for snippet in ["variable baseDir", "global fixture_startlog", "set err "] {
        let anchor = dto
            .source_command_anchors
            .iter()
            .find(|anchor| {
                anchor.kind == "nested"
                    && anchor.parent_root_group == Some(root_group)
                    && anchor
                        .display_path
                        .starts_with(&format!("c{root_group:03}/n"))
                    && anchor.snippet.trim_start().starts_with(snippet)
            })
            .unwrap_or_else(|| panic!("missing serialized nested anchor for {snippet:?}"));
        assert_eq!(anchor.command_path.len(), 2);
        assert!(anchor.source_map_index < dto.source_map.len());
        assert!(anchor.source_lines[0] < anchor.source_lines[1]);
    }

    let json = serde_json::to_value(&dto).expect("serializes as Tauri command payload");
    let anchors = json
        .get("sourceCommandAnchors")
        .and_then(|value| value.as_array())
        .expect("sourceCommandAnchors array");
    assert!(anchors.iter().any(|anchor| {
        anchor
            .get("displayPath")
            .and_then(|value| value.as_str())
            .is_some_and(|path| path.contains("/n"))
    }));
}
