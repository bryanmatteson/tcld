use tcld::cli::ExploreArgs;
use tcld::explore::ExplorerCore;
use tcld::explore::data_source::{VIEW_DECOMPILE, VIEW_INSTRUCTIONS, VIEW_SUMMARY};
use tcld_decompile::{AnalysisSnapshot, LoadInput, SourceMap};
use tcld_tui::data::DataSource;

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn tui_data_source_is_populated_from_canonical_snapshot() {
    let args = ExploreArgs {
        input_path: fixture("expr.tbc").display().to_string(),
        dump_screen: false,
        mode: None,
        overlay: None,
        palette_query: None,
        diff_target: None,
        page_size: 20,
        theme: None,
        no_colors: true,
    };
    let snapshot =
        AnalysisSnapshot::from_input(LoadInput::path(&args.input_path)).expect("snapshot loads");
    let mut snapshot_backed = snapshot.clone();
    snapshot_backed.metadata.command_count = 1;
    snapshot_backed.instructions.truncate(1);
    snapshot_backed.instructions[0].pc = 4242;
    snapshot_backed.instructions[0].op = "snapshotOnlyOp".to_string();
    snapshot_backed.instructions[0].operands = vec![7, 11];
    snapshot_backed.instructions[0].group = Some(0);
    snapshot_backed.instructions[0].group_start = true;
    snapshot_backed.instructions[0].comment = "snapshot-only instruction comment".to_string();
    snapshot_backed.traces.clear();
    snapshot_backed.decompile.text = "snapshot-only decompile text\n".to_string();
    snapshot_backed.dump_text = "snapshot-only dump text\n".to_string();
    snapshot_backed.summary_text =
        "snapshot-only summary\nmode: bytecode commands=1 code_bytes=1 instructions=1 literals=0 exceptions=0 aux=0\n"
            .to_string();
    snapshot_backed.source_map =
        SourceMap::from_spans(&[(4242..4243, "snapshot-only decompile text".to_string(), 0..1)]);

    let mut core = ExplorerCore::from_snapshot(args, snapshot_backed.clone());

    assert_eq!(
        <ExplorerCore as DataSource>::list_len(&core, VIEW_INSTRUCTIONS),
        snapshot_backed.instructions.len()
    );
    let entry = <ExplorerCore as DataSource>::list_entry(&core, VIEW_INSTRUCTIONS, 0)
        .expect("snapshot instruction entry");
    assert!(entry.text.contains("snapshotOnlyOp 7 11"));
    assert!(entry.text.contains("snapshot-only instruction comment"));
    assert_eq!(entry.group, Some(0));
    assert!(entry.is_group_start);
    assert_eq!(
        core.command_group_count(),
        snapshot_backed.metadata.command_count
    );
    assert_eq!(core.instruction_group_for_index(0), Some(0));
    assert_eq!(core.first_instruction_for_group(0), Some(0));
    assert_eq!(
        <ExplorerCore as DataSource>::document_text(&mut core, VIEW_DECOMPILE).unwrap(),
        snapshot_backed.decompile.text
    );
    assert!(
        !DataSource::available_views(&core)
            .iter()
            .any(|view| view.0 == "dump")
    );
    assert!(
        <ExplorerCore as DataSource>::document_text(&mut core, VIEW_SUMMARY)
            .unwrap()
            .contains("snapshot-only summary")
    );
    assert_eq!(core.trace_len(), snapshot_backed.traces.len());
    assert_eq!(core.decompile_anchors().unwrap(), vec![0]);
    assert_eq!(
        core.all_decompile_group_line_spans().unwrap(),
        vec![Some((0, 0))]
    );
}
