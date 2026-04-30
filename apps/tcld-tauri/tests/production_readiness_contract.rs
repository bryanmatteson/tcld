fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tcld/tests/fixtures")
        .join(name)
}

#[test]
fn static_and_live_loads_share_canonical_dto_instruction_fact_path() {
    let path = fixture("expr.tbc");
    let payload = std::fs::read_to_string(&path).expect("fixture payload");

    let static_dto = tcld_tauri::do_load(path.to_str().unwrap()).expect("static dto");
    let live_dto = tcld_tauri::do_load_from_payload(&payload, "debugger:expr").expect("live dto");

    assert_eq!(static_dto.metadata.syntax_engine, "tree-sitter-tcl");
    assert_eq!(
        live_dto.metadata.syntax_engine, static_dto.metadata.syntax_engine,
        "debugger live loads must use the same canonical AnalysisSnapshot -> LoadResult DTO path as static loads"
    );

    let grouped = live_dto
        .instructions
        .iter()
        .find(|inst| inst.group.is_some() && inst.command_pc_range.is_some())
        .expect("live dto exposes grouped instruction command pc ranges");
    let range = grouped.command_pc_range.expect("command pc range");
    assert!(range[0] <= grouped.pc && grouped.pc < range[1]);
    assert!(grouped.size > 0);

    let jump = live_dto
        .instructions
        .iter()
        .find(|inst| inst.is_jump)
        .expect("live dto exposes branch instruction facts");
    assert_ne!(jump.is_jump_conditional, jump.is_jump_unconditional);
    assert!(
        jump.next_locations
            .contains(&jump.jump_target.expect("jump target"))
    );
    assert_eq!(jump.fallthrough.is_some(), jump.is_jump_conditional);
}

#[test]
fn load_result_serializes_backend_owned_instruction_and_diagnostic_fields() {
    let dto = tcld_tauri::do_load(fixture("expr.tbc").to_str().unwrap()).expect("static dto");
    let json = serde_json::to_value(&dto).expect("serializes load result");
    let instruction = json
        .get("instructions")
        .and_then(|value| value.as_array())
        .and_then(|items| {
            items
                .iter()
                .find(|inst| inst.get("isJump") == Some(&serde_json::Value::Bool(true)))
        })
        .expect("serialized jump instruction");

    for key in [
        "size",
        "operands",
        "argc",
        "commandPcRange",
        "jumpTarget",
        "fallthrough",
        "primaryNextLocation",
        "secondaryNextLocation",
        "nextLocations",
    ] {
        assert!(
            instruction.get(key).is_some(),
            "missing serialized instruction field {key}"
        );
    }

    assert!(json.pointer("/diagnostics/approximations").is_some());
    assert!(json.pointer("/diagnostics/passDecisions").is_some());
    assert!(
        json.pointer("/provenance/originalSourceAvailable")
            .is_some()
    );
    assert!(json.get("sourceMap").is_some());
    assert!(json.get("sourceCommandAnchors").is_some());
    assert!(json.get("decompileFoldingRanges").is_some());
}

#[test]
fn approximation_dto_preserves_kind_scope_and_reason() {
    let dto = tcld_tauri::DiagnosticApproximationDTO {
        kind: "UnresolvedInstruction".to_string(),
        bytecode_range: Some([12, 17]),
        reason: "instruction omitted from reconstructed source".to_string(),
    };
    let json = serde_json::to_value(dto).expect("serialize approximation");
    assert_eq!(
        json.get("kind").and_then(|value| value.as_str()),
        Some("UnresolvedInstruction")
    );
    assert_eq!(
        json.get("bytecodeRange"),
        Some(&serde_json::json!([12, 17]))
    );
    assert_eq!(
        json.get("reason").and_then(|value| value.as_str()),
        Some("instruction omitted from reconstructed source")
    );
}
