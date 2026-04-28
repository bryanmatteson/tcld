use tcld::cli::ExploreArgs;
use tcld::explore::data_source::{VIEW_DECOMPILE, VIEW_INSTRUCTIONS, VIEW_SOURCE, VIEW_SUMMARY};
use tcld::explore::{ArtifactKind, Capability, ExplorerCore};
use tcld_tui::data::DataSource;
use tcld_tui::{ModeId, ModeLayoutProfile};

fn fixture(name: &str) -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

fn args(name: &str) -> ExploreArgs {
    ExploreArgs {
        input_path: fixture(name),
        dump_screen: false,
        mode: None,
        overlay: None,
        palette_query: None,
        diff_target: None,
        page_size: 20,
        theme: None,
        no_colors: false,
    }
}

#[test]
fn tcl_adapter_exposes_only_static_explore_modes() {
    let core = ExplorerCore::new(args("expr.tbc")).expect("bytecode workspace loads");
    let modes = DataSource::mode_specs(&core);
    let browse = modes
        .iter()
        .find(|mode| mode.id == ModeId("browse"))
        .expect("browse mode");
    assert_eq!(browse.title, "Browse");
    assert_eq!(browse.layout, ModeLayoutProfile::Browse);
    assert!(browse.status_affordances.contains(&"Export"));

    let compare = modes
        .iter()
        .find(|mode| mode.id == ModeId("compare"))
        .expect("compare mode");
    assert_eq!(compare.title, "Compare");
    assert_eq!(compare.layout, ModeLayoutProfile::Compare);
    assert_eq!(compare.keybind, Some('C'));
    assert_eq!(modes.len(), 2);
    assert!(!modes.iter().any(|mode| mode.id == ModeId("debug")));
}

#[test]
fn bytecode_workspace_is_populated_from_domain_contract() {
    let mut core = ExplorerCore::new(args("expr.tbc")).expect("bytecode workspace loads");

    assert_eq!(core.artifact_kind(), ArtifactKind::RawBytecode);
    assert!(core.capabilities().contains(&Capability::BytecodeViews));
    assert!(
        core.capabilities()
            .contains(&Capability::GeneratedSourceExport)
    );
    assert!(DataSource::available_views(&core).contains(&VIEW_INSTRUCTIONS));
    assert!(DataSource::available_views(&core).contains(&VIEW_DECOMPILE));
    assert!(DataSource::list_len(&core, VIEW_INSTRUCTIONS) > 0);

    let first = DataSource::list_entry(&core, VIEW_INSTRUCTIONS, 0).expect("instruction");
    assert!(first.text.contains('('));
    assert!(first.group.is_some());
    assert!(
        DataSource::list_detail(&core, VIEW_INSTRUCTIONS, 0)
            .expect("instruction detail")
            .contains("command_group")
    );

    let generated = DataSource::document_text(&mut core, VIEW_SOURCE).expect("generated source");
    assert!(generated.contains("set result"));
    assert!(!generated.contains("generated decompiled Tcl source"));
    let source_detail = DataSource::document_detail(&core, VIEW_SOURCE, 0, Some("value"))
        .expect("source inspector detail");
    assert!(source_detail.contains("source line: 1"));
    assert!(source_detail.contains("provenance:"));
    assert!(source_detail.contains("search: value"));
    assert!(
        !DataSource::document_text(&mut core, VIEW_DECOMPILE)
            .expect("decompile")
            .trim()
            .is_empty()
    );
    let view_tokens = DataSource::available_views(&core)
        .iter()
        .map(|view| view.0)
        .collect::<Vec<_>>();
    assert!(!view_tokens.contains(&"graph"));
    assert!(!view_tokens.contains(&"dump"));
    assert!(!view_tokens.contains(&"runtime-debug"));
}

#[test]
fn plain_source_workspace_exposes_only_source_oriented_contract() {
    let mut core = ExplorerCore::new(args("expr.tcl")).expect("source workspace loads");

    assert_eq!(core.artifact_kind(), ArtifactKind::PlainTclSource);
    assert!(!core.capabilities().contains(&Capability::BytecodeViews));
    assert!(
        !core
            .capabilities()
            .contains(&Capability::GeneratedSourceExport)
    );
    assert_eq!(
        DataSource::available_views(&core),
        &[VIEW_SOURCE, VIEW_DECOMPILE, VIEW_SUMMARY,]
    );
    assert!(!DataSource::available_views(&core).contains(&VIEW_INSTRUCTIONS));
    assert!(
        DataSource::document_text(&mut core, VIEW_DECOMPILE)
            .expect("source rendering")
            .contains("Tcl source rendering")
    );
}

#[test]
fn plain_source_goto_is_one_based_source_line_navigation() {
    let mut app = tcld_tui::TuiApp::new(ExplorerCore::new(args("expr.tcl")).unwrap(), false);
    app.execute_command("goto 1");
    assert_eq!(app.nav.doc_scroll(VIEW_SOURCE), 0);
    assert_eq!(app.nav.group_focus, None);

    app.execute_command("goto 2");
    assert_eq!(app.nav.doc_scroll(VIEW_SOURCE), 1);
    assert_eq!(app.nav.group_focus, None);

    app.execute_command("goto 999999");
    assert_eq!(app.nav.doc_scroll(VIEW_SOURCE), 1);
    assert!(
        app.cmd
            .output
            .as_deref()
            .unwrap_or("")
            .contains("not found")
    );
}
