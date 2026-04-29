use tcld::cli::ExploreArgs;
use tcld::explore::ExplorerCore;
use tcld::explore::data_source::{VIEW_DECOMPILE, VIEW_SOURCE};
use tcld_tui::data::DataSource;

fn repo_fixture(path: &str) -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
        .display()
        .to_string()
}

fn args(path: &str) -> ExploreArgs {
    ExploreArgs {
        input_path: repo_fixture(path),
        dump_screen: false,
        mode: None,
        overlay: None,
        palette_query: None,
        diff_target: None,
        page_size: 20,
        theme: None,
        no_colors: true,
    }
}

#[test]
fn bcproc_source_commands_render_as_nested_navigation_anchors() {
    let mut source = ExplorerCore::new(args("testing/fixtures/public/public_complex_fixture.tbc"))
        .expect("public fixture workspace loads");

    let decompile_anchors =
        DataSource::document_anchors(&mut source, VIEW_DECOMPILE).expect("decompile anchors");
    let startlog_variable = decompile_anchors
        .iter()
        .find(|anchor| {
            anchor.display_path.contains("/n")
                && anchor.label.contains("variable baseDir")
                && anchor
                    .parent_path
                    .as_deref()
                    .is_some_and(|path| path.starts_with('c'))
        })
        .expect("nested variable baseDir anchor");
    let startlog_global = decompile_anchors
        .iter()
        .find(|anchor| {
            anchor.display_path.starts_with(
                startlog_variable
                    .parent_path
                    .as_deref()
                    .expect("nested anchor has parent path"),
            ) && anchor.label.contains("global fixture_startlog")
        })
        .expect("nested global _startlog anchor");

    assert!(startlog_variable.line_span.0 < startlog_variable.line_span.1);
    assert!(startlog_global.line_span.0 < startlog_global.line_span.1);

    let rendered = DataSource::document_text(&mut source, VIEW_DECOMPILE).expect("decompile text");
    assert!(rendered.contains("proc fixture::startlog"));
    assert!(rendered.contains("variable baseDir"));
    assert!(rendered.contains("global fixture_startlog"));

    let source_hits =
        DataSource::document_anchors(&mut source, VIEW_SOURCE).expect("source anchors");
    assert!(
        source_hits
            .iter()
            .any(|anchor| anchor.display_path == startlog_variable.display_path),
        "source and decompile views should expose the same nested anchor paths"
    );
}
