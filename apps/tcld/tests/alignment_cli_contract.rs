use std::process::Command;
use tcld::cli::{TcldCommand, parse_tcld_args};
use tcld::explore::ExplorerCore;
use tcld_decompile::{AnalysisSnapshot, LoadInput};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn coverage_report_command_and_no_default_explorer_use_domain_snapshot() {
    let parsed = parse_tcld_args(&[
        "tcld".to_string(),
        "coverage".to_string(),
        "report".to_string(),
    ])
    .expect("coverage args parse");
    assert!(matches!(parsed, TcldCommand::Coverage(_)));

    let exe = env!("CARGO_BIN_EXE_tcld");
    let output = Command::new(exe)
        .args(["coverage", "report"])
        .output()
        .expect("coverage command runs");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("opcode"));
    assert!(stdout.contains("Tcl 8.6"));

    let core = ExplorerCore::new(tcld::cli::ExploreArgs {
        input_path: fixture("expr.tbc").display().to_string(),
        dump_screen: false,
        mode: None,
        overlay: None,
        palette_query: None,
        diff_target: None,
        page_size: 20,
        theme: None,
        no_colors: true,
    })
    .expect("explorer loads");
    let snapshot = core.snapshot().expect("bytecode snapshot");
    assert_eq!(
        snapshot.metadata.instruction_count,
        core.list_len(tcld::explore::ExplorerView::Instructions)
    );
    assert!(core.summary_text().contains("snapshot: canonical-domain"));

    let args = tcld::cli::ExploreArgs {
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
    let mut snapshot =
        AnalysisSnapshot::from_input(LoadInput::path(&args.input_path)).expect("snapshot loads");
    snapshot.instructions.truncate(1);
    snapshot.instructions[0].group = Some(0);
    snapshot.instructions[0].group_start = true;
    snapshot.instructions[0].op = "alignmentSnapshotOnlyOp".to_string();
    snapshot.summary_text = "alignment snapshot-only summary\n".to_string();
    snapshot.metadata.command_count = 1;

    let snapshot_core = ExplorerCore::from_snapshot(args, snapshot);
    let entry = snapshot_core
        .list_entry(tcld::explore::ExplorerView::Instructions, 0)
        .expect("snapshot-backed instruction");
    assert!(entry.text.contains("alignmentSnapshotOnlyOp"));
    assert_eq!(entry.group, Some(0));
    assert_eq!(snapshot_core.command_group_count(), 1);
    assert_eq!(snapshot_core.first_instruction_for_group(0), Some(0));
    assert!(
        snapshot_core
            .summary_text()
            .contains("alignment snapshot-only summary")
    );
}
