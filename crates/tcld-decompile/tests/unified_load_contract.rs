use tcld_decompile::{
    LoadInput, LoadKind, decompile_loaded, disassemble_loaded, dump_loaded, load_bytecode,
};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/tcld/tests/fixtures")
        .join(name)
}

#[test]
fn load_routes_share_one_loaded_bytecode_representation() {
    let path = fixture("expr.tbc");
    let raw_text = std::fs::read_to_string(&path).expect("fixture");
    let raw_bytes = std::fs::read(&path).expect("fixture bytes");

    let from_path = load_bytecode(LoadInput::path(&path)).expect("path load");
    let from_payload =
        load_bytecode(LoadInput::raw_payload_text(raw_text.clone(), "expr-raw")).expect("payload");
    let from_wrapper =
        load_bytecode(LoadInput::tcl_source_text(raw_text.clone(), "expr-wrapper")).expect("src");
    let from_bytes =
        load_bytecode(LoadInput::raw_bytes(raw_bytes.clone(), "expr-bytes")).expect("bytes");
    let from_debugger = load_bytecode(LoadInput::debugger_chunk(raw_text.clone(), "live#1", 1))
        .expect("debugger chunk");
    let from_wasm =
        load_bytecode(LoadInput::wasm_bytes(raw_bytes, "expr-wasm")).expect("wasm bytes");

    let counts = [
        &from_path,
        &from_payload,
        &from_wrapper,
        &from_bytes,
        &from_debugger,
        &from_wasm,
    ]
    .map(|loaded| loaded.bytecode.get_instructions().unwrap().len());
    assert!(counts.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(from_debugger.provenance.kind, LoadKind::DebuggerChunk);
    assert_eq!(from_wasm.provenance.kind, LoadKind::WasmBytes);

    assert!(decompile_loaded(&from_path).unwrap().contains("set result"));
    assert!(disassemble_loaded(&from_wasm).unwrap().contains("push"));
    assert!(
        dump_loaded(&from_debugger)
            .unwrap()
            .contains("Instructions:")
    );
    assert!(
        tcld_decompile::wasm::disassemble_bytes(raw_text.as_bytes())
            .unwrap()
            .contains("push")
    );
}
