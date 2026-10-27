use crate::data::{DataSource, ViewId};
use crate::helpers::{decode_view_map, encode_view_map, normalize_session_path, session_file_path};
use crate::state::{DiffState, LayoutState, NavigationState, PaneFocus, RailMode};
use crate::views::{self, ViewSpec};
use std::collections::HashMap;
use std::fs;
use std::io;

/// Snapshot of session state that can be serialized/deserialized.
pub struct SessionSnapshot {
    pub view_id: Option<ViewId>,
    pub decompile_map_visible: bool,
    pub trace_mode: bool,
    pub focused_pane: PaneFocus,
    pub rail_mode: RailMode,
    pub rail_contrast: u8,
    pub group_focus: Option<usize>,
    pub trace_index: Option<usize>,
    pub detail_scroll: usize,
    pub selections: HashMap<ViewId, usize>,
    pub list_scrolls: HashMap<ViewId, usize>,
    pub doc_scrolls: HashMap<ViewId, usize>,
    pub doc_cursors: HashMap<ViewId, usize>,
    pub diff_target: Option<String>,
    pub theme: Option<String>,
}

/// Persist session state to `~/.tcld/explore-session.txt`.
pub fn persist<D: DataSource>(
    data: &D,
    nav: &NavigationState,
    layout: &LayoutState,
    diff: &DiffState,
    current_view: ViewId,
    specs: &[ViewSpec],
    theme: &str,
) -> io::Result<()> {
    let Some(path) = session_file_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut lines = Vec::new();
    lines.push(format!("path={}", normalize_session_path(data.path())));
    lines.push(format!("view={}", views::token(specs, current_view)));
    lines.push(format!(
        "decompile_map_visible={}",
        layout.decompile_map_visible
    ));
    lines.push(format!("trace_mode={}", nav.trace_mode));
    lines.push(format!(
        "focused_pane={}",
        match layout.focused_pane {
            PaneFocus::List => "list",
            PaneFocus::Decompile => "decompile",
            PaneFocus::Context => "context",
        }
    ));
    lines.push(format!(
        "rail_mode={}",
        match layout.rail_mode {
            RailMode::Adaptive => "adaptive",
            RailMode::Anchors => "anchors",
            RailMode::Spans => "spans",
        }
    ));
    lines.push(format!("rail_contrast={}", layout.rail_contrast));
    lines.push(format!("theme={theme}"));
    if let Some(group) = nav.group_focus {
        lines.push(format!("group_focus={group}"));
    }
    if let Some(index) = nav.trace_index {
        lines.push(format!("trace_index={index}"));
    }
    if let Some(diff_target) = &diff.target_path {
        lines.push(format!("diff_target={diff_target}"));
    }
    lines.push(format!("selections={}", encode_view_map(&nav.selections)));
    lines.push(format!(
        "list_scrolls={}",
        encode_view_map(&nav.list_scrolls)
    ));
    lines.push(format!("doc_scrolls={}", encode_view_map(&nav.doc_scrolls)));
    lines.push(format!("doc_cursors={}", encode_view_map(&nav.doc_cursors)));
    lines.push(format!("detail_scroll={}", nav.detail_scroll));
    fs::write(path, lines.join("\n"))
}

/// Restore session state from `~/.tcld/explore-session.txt`.
///
/// Returns `None` if no session file exists or the saved path does not match.
pub fn restore(
    data_path: &str,
    specs: &[ViewSpec],
    available_views: &[ViewId],
) -> Option<SessionSnapshot> {
    let path = session_file_path()?;
    let contents = fs::read_to_string(path).ok()?;

    let mut map = HashMap::new();
    for line in contents.lines() {
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    let saved_path = map.get("path")?;
    if saved_path != &normalize_session_path(data_path) {
        return None;
    }

    let view_id = map
        .get("view")
        .and_then(|v| views::parse_token(specs, v))
        .filter(|v| available_views.contains(v));

    let decompile_map_visible = map
        .get("decompile_map_visible")
        .or_else(|| map.get("multi_view"))
        .is_some_and(|v| matches!(v.as_str(), "true" | "1"));
    let trace_mode = map
        .get("trace_mode")
        .is_some_and(|v| matches!(v.as_str(), "true" | "1"));

    let focused_pane = match map.get("focused_pane").map(String::as_str) {
        Some("list") | Some("single") => PaneFocus::List,
        Some("context") => PaneFocus::Context,
        _ => PaneFocus::Decompile,
    };
    let rail_mode = match map.get("rail_mode").map(String::as_str) {
        Some("anchors") => RailMode::Anchors,
        Some("spans") => RailMode::Spans,
        _ => RailMode::Adaptive,
    };
    let rail_contrast = map
        .get("rail_contrast")
        .and_then(|v| v.parse::<u8>().ok())
        .map(|v| v.min(2))
        .unwrap_or(1);

    let group_focus = map.get("group_focus").and_then(|v| v.parse::<usize>().ok());
    let trace_index = map.get("trace_index").and_then(|v| v.parse::<usize>().ok());
    let detail_scroll = map
        .get("detail_scroll")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let selections = map
        .get("selections")
        .map(|v| decode_view_map(v, specs))
        .unwrap_or_default();
    let list_scrolls = map
        .get("list_scrolls")
        .map(|v| decode_view_map(v, specs))
        .unwrap_or_default();
    let doc_scrolls: HashMap<ViewId, usize> = map
        .get("doc_scrolls")
        .map(|v| decode_view_map(v, specs))
        .unwrap_or_default();
    // Sessions from before the cursor existed treated the top line as
    // current; start the cursor there.
    let doc_cursors = map
        .get("doc_cursors")
        .map(|v| decode_view_map(v, specs))
        .unwrap_or_else(|| doc_scrolls.clone());
    let diff_target = map.get("diff_target").cloned();
    let theme = map.get("theme").cloned();

    Some(SessionSnapshot {
        view_id,
        decompile_map_visible,
        trace_mode,
        focused_pane,
        rail_mode,
        rail_contrast,
        group_focus,
        trace_index,
        detail_scroll,
        selections,
        list_scrolls,
        doc_scrolls,
        doc_cursors,
        diff_target,
        theme,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::data::{ListEntry, ViewId};
    use crate::views::{ViewOpenBehavior, ViewSpec};
    use std::time::{SystemTime, UNIX_EPOCH};

    const LIST: ViewId = ViewId("list");
    const DOC: ViewId = ViewId("doc");
    const SPECS: &[ViewSpec] = &[
        ViewSpec {
            view: LIST,
            token: "list",
            title: "List",
            aliases: &["list"],
            keybind: Some('l'),
            icon: "*",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: true,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: DOC,
            token: "doc",
            title: "Doc",
            aliases: &["doc"],
            keybind: Some('d'),
            icon: "#",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
    ];

    struct SessionData {
        path: String,
    }

    impl DataSource for SessionData {
        fn path(&self) -> &str {
            &self.path
        }

        fn available_views(&self) -> &[ViewId] {
            &[LIST, DOC]
        }

        fn default_view(&self) -> ViewId {
            LIST
        }

        fn no_colors(&self) -> bool {
            false
        }

        fn page_size(&self) -> usize {
            10
        }

        fn summary_text(&self) -> String {
            "summary".to_string()
        }

        fn view_specs(&self) -> &[ViewSpec] {
            SPECS
        }

        fn list_len(&self, _view: ViewId) -> usize {
            1
        }

        fn list_entry(&self, _view: ViewId, _index: usize) -> Option<ListEntry> {
            None
        }

        fn list_detail(&self, _view: ViewId, _index: usize) -> Option<String> {
            None
        }

        fn jump_index(&self, _view: ViewId, _value: i32) -> Option<usize> {
            None
        }

        fn document_text(&mut self, _view: ViewId) -> Result<String, String> {
            Ok(String::new())
        }

        fn command_group_count(&self) -> usize {
            0
        }

        fn group_for_index(&self, _index: usize) -> Option<usize> {
            None
        }

        fn first_index_for_group(&self, _group: usize) -> Option<usize> {
            None
        }
    }

    fn isolated_home() -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time is available")
            .as_nanos();
        let home = std::env::temp_dir().join(format!(
            "tcld-tui-session-test-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&home).expect("create isolated home");
        // Rust 2024 marks process-wide environment mutation as unsafe. This
        // test isolates HOME so production session path code is exercised.
        unsafe {
            std::env::set_var("HOME", &home);
        }
        home
    }

    #[test]
    fn session_persistence_roundtrips_navigation_layout_diff_and_theme() {
        let home = isolated_home();
        let data = SessionData {
            path: "fixture.tcl".to_string(),
        };
        let mut nav = NavigationState {
            view_index: 1,
            trace_mode: true,
            group_focus: Some(7),
            trace_index: Some(2),
            detail_scroll: 4,
            ..Default::default()
        };
        nav.selections.insert(LIST, 9);
        nav.list_scrolls.insert(LIST, 5);
        nav.doc_scrolls.insert(DOC, 11);
        let layout = LayoutState {
            decompile_map_visible: true,
            focused_pane: PaneFocus::Context,
            rail_mode: RailMode::Spans,
            rail_contrast: 2,
            mode_index: 0,
        };
        let diff = DiffState {
            active: true,
            target_path: Some("other.tcl".to_string()),
            diff_document: None,
            change_lines: Vec::new(),
            right_scroll: 0,
            focus_right: false,
        };

        persist(&data, &nav, &layout, &diff, DOC, SPECS, "sunset").expect("persist session");

        let snapshot = restore("fixture.tcl", SPECS, &[LIST, DOC]).expect("restore session");
        assert_eq!(snapshot.view_id, Some(DOC));
        assert!(snapshot.decompile_map_visible);
        assert!(snapshot.trace_mode);
        assert_eq!(snapshot.focused_pane, PaneFocus::Context);
        assert_eq!(snapshot.rail_mode, RailMode::Spans);
        assert_eq!(snapshot.rail_contrast, 2);
        assert_eq!(snapshot.group_focus, Some(7));
        assert_eq!(snapshot.trace_index, Some(2));
        assert_eq!(snapshot.detail_scroll, 4);
        assert_eq!(snapshot.selections.get(&LIST), Some(&9));
        assert_eq!(snapshot.list_scrolls.get(&LIST), Some(&5));
        assert_eq!(snapshot.doc_scrolls.get(&DOC), Some(&11));
        assert_eq!(snapshot.diff_target.as_deref(), Some("other.tcl"));
        assert_eq!(snapshot.theme.as_deref(), Some("sunset"));
        assert!(restore("different.tcl", SPECS, &[LIST, DOC]).is_none());

        let legacy = [
            "path=fixture.tcl",
            "mode=debug",
            "view=graph",
            "multi_view=true",
            "focused_pane=single",
            "selections=graph:99,dump:88,runtime-debug:77,list:3",
            "list_scrolls=graph:99,list:2",
            "doc_scrolls=dump:55,doc:4",
        ]
        .join("\n");
        std::fs::write(session_file_path().expect("session path"), legacy)
            .expect("write legacy session");
        let migrated = restore("fixture.tcl", SPECS, &[LIST, DOC]).expect("migrate session");
        assert_eq!(migrated.view_id, None, "removed view must be ignored");
        assert!(migrated.decompile_map_visible, "old map field migrates");
        assert_eq!(migrated.focused_pane, PaneFocus::List);
        assert_eq!(migrated.selections.len(), 1);
        assert_eq!(migrated.selections.get(&LIST), Some(&3));
        assert_eq!(migrated.list_scrolls.get(&LIST), Some(&2));
        assert_eq!(migrated.doc_scrolls.get(&DOC), Some(&4));

        let _ = std::fs::remove_dir_all(home);
    }
}
