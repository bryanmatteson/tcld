use std::process::Command;

use tcld::cli::{ExploreArgs, ExploreMode, ExploreOverlay};
use tcld::explore::data_source::{VIEW_INSTRUCTIONS, VIEW_SOURCE};
use tcld::explore::{DumpScreenOptions, ExplorerCore, dump_screen};
use tcld_tui::data::DataSource;
use tcld_tui::{TuiApp, render_frame_to_text};

const SNAPSHOT_DIR: &str = "tests/snapshots/explore_dump_screen";

fn fixture(name: &str) -> String {
    std::path::Path::new("tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

fn args(name: &str, no_colors: bool) -> ExploreArgs {
    ExploreArgs {
        input_path: fixture(name),
        dump_screen: false,
        mode: None,
        overlay: None,
        palette_query: None,
        diff_target: None,
        page_size: 20,
        theme: Some("sunset".to_string()),
        no_colors,
    }
}

fn render(options: DumpScreenOptions<'_>) -> String {
    let frame = dump_screen(options).expect("dump screen renders");
    assert_no_ansi(&frame);
    frame
}

fn assert_static_only(frame: &str) {
    for removed in ["Runtime Debug", "rdebug", "Graph", " Dump "] {
        assert!(
            !frame.contains(removed),
            "removed Explore surface {removed:?} is still visible\n{frame}"
        );
    }
}

fn assert_no_ansi(frame: &str) {
    assert!(
        !frame.as_bytes().windows(2).any(|bytes| bytes == b"\x1b["),
        "dump frame must not contain ANSI escapes"
    );
}

fn assert_snapshot(name: &str, frame: &str) {
    let normalized = frame.replace(env!("CARGO_MANIFEST_DIR"), "<APP>");
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(SNAPSHOT_DIR)
        .join(format!("{name}.txt"));
    if std::env::var_os("TCLD_UPDATE_EXPLORE_DUMP_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("snapshot parent")).unwrap();
        std::fs::write(&path, &normalized).unwrap();
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read snapshot {}: {err}", path.display()));
    assert_eq!(normalized, expected, "snapshot mismatch for {name}");
}

#[test]
fn bytecode_browse_frame_uses_selected_view_as_full_width_canvas() {
    let frame = render(DumpScreenOptions {
        args: args("expr.tbc", false),
        width: 120,
        height: 32,
        focused: None,
        empty: false,
    });

    for required in [
        "tcld explore",
        "[b] Browse",
        "[C] Compare",
        "Instructions · ",
        "Views",
        "Inspector",
        "source: generated",
        "Instructions  #1/",
    ] {
        assert!(frame.contains(required), "missing {required:?}\n{frame}");
    }
    assert_static_only(&frame);
    assert_snapshot("bytecode_wide", &frame);

    let no_color = render(DumpScreenOptions {
        args: args("expr.tbc", true),
        width: 96,
        height: 28,
        focused: None,
        empty: false,
    });
    assert!(
        !no_color.contains("theme:") && !no_color.contains("colors:"),
        "visual preferences should not occupy the primary header\n{no_color}"
    );
    assert_static_only(&no_color);
    assert_snapshot("bytecode_no_color", &no_color);
}

#[test]
fn source_is_a_document_canvas_not_a_narrow_list() {
    let frame = render(DumpScreenOptions {
        args: args("expr.tcl", false),
        width: 120,
        height: 32,
        focused: None,
        empty: false,
    });

    assert!(frame.contains("┏ Source "), "{frame}");
    assert!(frame.contains("set result"), "{frame}");
    assert!(!frame.contains("No entries in Source"), "{frame}");
    assert_static_only(&frame);
    assert_snapshot("source_wide", &frame);
}

#[test]
fn help_and_palette_are_generated_from_the_static_contract() {
    let mut help_args = args("expr.tbc", false);
    help_args.overlay = Some(ExploreOverlay::Help);
    let help = render(DumpScreenOptions {
        args: help_args,
        width: 120,
        height: 32,
        focused: None,
        empty: false,
    });
    for required in [
        "Quick Reference",
        "Following links",
        "Browse",
        "Compare",
        "Instructions",
        "Decompile",
    ] {
        assert!(help.contains(required), "missing {required:?}\n{help}");
    }
    assert_static_only(&help);
    assert_snapshot("bytecode_help_overlay", &help);

    let mut palette_args = args("expr.tbc", false);
    palette_args.overlay = Some(ExploreOverlay::Palette);
    palette_args.palette_query = Some("decompile".to_string());
    let palette = render(DumpScreenOptions {
        args: palette_args,
        width: 120,
        height: 32,
        focused: None,
        empty: false,
    });
    assert!(palette.contains("Command Palette"), "{palette}");
    assert!(palette.contains("Filter: decompile"), "{palette}");
    assert!(palette.contains("Open Decompile view"), "{palette}");
    assert_static_only(&palette);
    assert_snapshot("bytecode_palette_overlay", &palette);
}

#[test]
fn compare_frame_has_independent_source_and_diff_surfaces() {
    let mut compare_args = args("expr.tbc", false);
    compare_args.mode = Some(ExploreMode::Compare);
    compare_args.diff_target = Some(fixture("proc.tbc"));
    let frame = render(DumpScreenOptions {
        args: compare_args,
        width: 120,
        height: 32,
        focused: None,
        empty: false,
    });

    for required in [
        "[C] Compare",
        "Left Source / Decompile",
        "Diff Report",
        "Semantic diff",
        "Compare",
        "Target:",
    ] {
        assert!(frame.contains(required), "missing {required:?}\n{frame}");
    }
    assert_static_only(&frame);
    assert_snapshot("bytecode_compare_diff", &frame);
}

#[test]
fn follow_scroll_keeps_the_selected_instruction_visible() {
    let mut explore_args = args("proc.tbc", true);
    explore_args.page_size = 5;
    let core = ExplorerCore::new(explore_args).expect("workspace loads");
    let total = DataSource::list_len(&core, VIEW_INSTRUCTIONS);
    assert!(total > 8, "fixture needs enough instructions for scrolling");
    let selected = (total - 1).min(20);
    let selected_text = DataSource::list_entry(&core, VIEW_INSTRUCTIONS, selected)
        .expect("selected instruction")
        .text;

    let mut app = TuiApp::new(core, false);
    app.switch_to_view(VIEW_INSTRUCTIONS);
    app.nav.selections.insert(VIEW_INSTRUCTIONS, selected);
    let frame = render_frame_to_text(&mut app, 120, 24).expect("frame renders");

    assert!(
        frame.contains(&selected_text),
        "selected row is off-screen\n{frame}"
    );
    assert!(app.nav.list_scroll(VIEW_INSTRUCTIONS) > 0);
    assert!(
        app.hit
            .list_targets
            .iter()
            .any(|target| target.view == VIEW_INSTRUCTIONS
                && target.start > 0
                && selected >= target.start
                && selected < target.end),
        "no visible hit target contains selection #{selected}"
    );
    assert!(!frame.contains("Selection: #0 | At top"), "{frame}");
}

#[test]
fn real_cli_dump_screen_is_plain_and_static_only() {
    let output = Command::new(env!("CARGO_BIN_EXE_tcld"))
        .args(["explore", "--dump-screen", &fixture("expr.tbc")])
        .output()
        .expect("run tcld");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frame = String::from_utf8(output.stdout).expect("utf8 frame");
    assert!(frame.contains("Instructions · "), "{frame}");
    assert_no_ansi(&frame);
    assert_static_only(&frame);
}

#[test]
fn cli_help_names_both_supported_input_kinds() {
    let output = Command::new(env!("CARGO_BIN_EXE_tcld"))
        .args(["explore", "--help"])
        .output()
        .expect("run tcld help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert!(
        help.contains("Path to Tcl bytecode or a Tcl source file"),
        "{help}"
    );
}

#[test]
fn dump_screen_rejects_an_unknown_theme() {
    let output = Command::new(env!("CARGO_BIN_EXE_tcld"))
        .args([
            "explore",
            "--dump-screen",
            "--theme",
            "definitely-not-a-theme",
            &fixture("expr.tbc"),
        ])
        .output()
        .expect("run tcld");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown theme"), "{stderr}");
    assert!(stderr.contains("available:"), "{stderr}");
}

#[test]
fn source_search_and_goto_use_document_offsets() {
    let core = ExplorerCore::new(args("expr.tcl", true)).expect("source loads");
    let mut app = TuiApp::new(core, false);
    app.switch_to_view(VIEW_SOURCE);
    app.execute_command("goto 2");
    assert_eq!(app.nav.doc_cursor(VIEW_SOURCE), 1);
    app.execute_command("search result");
    assert_eq!(app.search.view, Some(VIEW_SOURCE));
    assert_eq!(
        app.nav.doc_cursor(VIEW_SOURCE),
        app.search.hits[app.search.cursor]
    );
}
