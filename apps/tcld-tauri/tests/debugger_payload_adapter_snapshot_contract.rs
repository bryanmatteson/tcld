fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tcld/tests/fixtures")
        .join(name)
}

#[test]
fn debugger_payload_routes_through_loader_snapshot_and_dto_without_native_debugger() {
    let payload = std::fs::read_to_string(fixture("expr.tbc")).expect("fixture");
    let dto = tcld_tauri::do_load_from_payload(&payload, "live#1")
        .expect("production debugger payload snapshot");
    assert_eq!(dto.provenance.kind, "DebuggerChunk");
    assert!(!dto.provenance.original_source_available);
    assert!(dto.original_source_text.is_none());
    assert_eq!(dto.metadata.filename, "live#1");
    assert!(!dto.source_map.is_empty());
    assert_eq!(dto.diagnostics.reconstruction, dto.reconstruction);
    assert_eq!(dto.reconstruction, "Strict");
    assert!(!dto.traces.is_empty());
    assert!(dto.decompile_lines.iter().any(|line| line.group.is_some()));
    assert!(dto.dump_text.contains("Instructions:"));
    assert!(
        dto.instructions
            .iter()
            .any(|inst| inst.group.is_some() && inst.command_pc_range.is_some())
    );
    assert!(dto.instructions.iter().filter(|inst| inst.is_jump).all(
        |inst| inst.is_jump_conditional != inst.is_jump_unconditional
            && !inst.next_locations.is_empty()
    ));
}
