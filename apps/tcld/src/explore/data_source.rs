use crate::explore::model::{ExplorerCore, ExplorerView};
use std::fmt::Write as _;
use tcld_explore::{ExplorerCommandRequest, ExplorerCommandResult};
use tcld_tui::data::{
    DataSource, DecompileRenderContext, DocumentAnchor, Link, ListEntry, OperationAvailability,
    SemanticDiff, ViewId,
};
use tcld_tui::style;
use tcld_tui::ui::{Line, Span, Text};
use tcld_tui::views::{ViewOpenBehavior, ViewSpec};
use tcld_tui::{ModeId, ModeLayoutProfile, ModeSpec, Theme};

// ── View IDs ────────────────────────────────────────────────────

pub const VIEW_INSTRUCTIONS: ViewId = ViewId("instructions");
pub const VIEW_LITERALS: ViewId = ViewId("literals");
pub const VIEW_EXCEPTIONS: ViewId = ViewId("exceptions");
pub const VIEW_SOURCE: ViewId = ViewId("source");
pub const VIEW_DECOMPILE: ViewId = ViewId("decompile");
pub const VIEW_SUMMARY: ViewId = ViewId("summary");
const MODE_BROWSE: ModeId = ModeId("browse");
const MODE_COMPARE: ModeId = ModeId("compare");

pub const TCL_MODE_SPECS: &[ModeSpec] = &[
    ModeSpec {
        id: MODE_BROWSE,
        token: "browse",
        title: "Browse",
        icon: "B",
        keybind: Some('b'),
        layout: ModeLayoutProfile::Browse,
        status_affordances: &["Command", "Palette", "Help", "Search", "Trace", "Export"],
        unavailable_message: None,
    },
    ModeSpec {
        id: MODE_COMPARE,
        token: "compare",
        title: "Compare",
        icon: "C",
        keybind: Some('C'),
        layout: ModeLayoutProfile::Compare,
        status_affordances: &["Diff", "Close", "Next", "Previous", "Status"],
        unavailable_message: Some("semantic diff requires an opened diff target"),
    },
];

// ── View Specs ──────────────────────────────────────────────────

pub const TCL_VIEW_SPECS: &[ViewSpec] = &[
    ViewSpec {
        view: VIEW_INSTRUCTIONS,
        token: "instructions",
        title: "Instructions",
        aliases: &["instructions", "instruction", "inst", "i"],
        keybind: Some('i'),
        icon: "⊞",
        show_in_tab_bar: true,
        show_in_palette: true,
        is_list_view: true,
        open_behavior: ViewOpenBehavior::ReplaceMain,
    },
    ViewSpec {
        view: VIEW_LITERALS,
        token: "literals",
        title: "Literals",
        aliases: &["literals", "literal", "lit", "l"],
        keybind: Some('l'),
        icon: "◈",
        show_in_tab_bar: true,
        show_in_palette: true,
        is_list_view: true,
        open_behavior: ViewOpenBehavior::ReplaceMain,
    },
    ViewSpec {
        view: VIEW_EXCEPTIONS,
        token: "exceptions",
        title: "Exceptions",
        aliases: &["exceptions", "exception", "ex", "e"],
        keybind: Some('e'),
        icon: "⚡",
        show_in_tab_bar: true,
        show_in_palette: true,
        is_list_view: true,
        open_behavior: ViewOpenBehavior::ReplaceMain,
    },
    ViewSpec {
        view: VIEW_SOURCE,
        token: "source",
        title: "Source",
        aliases: &["source", "src"],
        keybind: Some('c'),
        icon: "☰",
        show_in_tab_bar: true,
        show_in_palette: true,
        is_list_view: false,
        open_behavior: ViewOpenBehavior::ReplaceMain,
    },
    ViewSpec {
        view: VIEW_DECOMPILE,
        token: "decompile",
        title: "Decompile",
        aliases: &["decompile", "dec", "d"],
        keybind: Some('d'),
        icon: "◇",
        show_in_tab_bar: true,
        show_in_palette: true,
        is_list_view: false,
        open_behavior: ViewOpenBehavior::ReplaceMain,
    },
    ViewSpec {
        view: VIEW_SUMMARY,
        token: "summary",
        title: "Summary",
        aliases: &["summary", "s"],
        keybind: Some('s'),
        icon: "◊",
        show_in_tab_bar: false,
        show_in_palette: true,
        is_list_view: false,
        open_behavior: ViewOpenBehavior::SummaryModal,
    },
];

// ── Available view sets ─────────────────────────────────────────

const BYTECODE_VIEWS: &[ViewId] = &[
    VIEW_INSTRUCTIONS,
    VIEW_LITERALS,
    VIEW_EXCEPTIONS,
    VIEW_SOURCE,
    VIEW_DECOMPILE,
    VIEW_SUMMARY,
];

const SOURCE_VIEWS: &[ViewId] = &[VIEW_SOURCE, VIEW_DECOMPILE, VIEW_SUMMARY];

// ── Helper: map ExplorerView enum to ViewId ─────────────────────

fn to_view_id(view: ExplorerView) -> ViewId {
    match view {
        ExplorerView::Instructions => VIEW_INSTRUCTIONS,
        ExplorerView::Literals => VIEW_LITERALS,
        ExplorerView::Exceptions => VIEW_EXCEPTIONS,
        ExplorerView::Source => VIEW_SOURCE,
        ExplorerView::Decompile => VIEW_DECOMPILE,
        ExplorerView::Summary => VIEW_SUMMARY,
    }
}

fn to_explorer_view(view: ViewId) -> Option<ExplorerView> {
    match view.0 {
        "instructions" => Some(ExplorerView::Instructions),
        "literals" => Some(ExplorerView::Literals),
        "exceptions" => Some(ExplorerView::Exceptions),
        "source" => Some(ExplorerView::Source),
        "decompile" => Some(ExplorerView::Decompile),
        "summary" => Some(ExplorerView::Summary),
        _ => None,
    }
}

// ── DataSource impl ─────────────────────────────────────────────

impl DataSource for ExplorerCore {
    fn path(&self) -> &str {
        self.core().path()
    }

    fn available_views(&self) -> &[ViewId] {
        let views = self.views();
        if views.contains(&ExplorerView::Instructions) {
            BYTECODE_VIEWS
        } else {
            SOURCE_VIEWS
        }
    }

    fn default_view(&self) -> ViewId {
        to_view_id(self.core().default_view())
    }

    fn no_colors(&self) -> bool {
        self.no_colors()
    }

    fn page_size(&self) -> usize {
        self.core().page_size()
    }

    fn summary_text(&self) -> String {
        self.core().summary_text()
    }

    fn view_specs(&self) -> &[ViewSpec] {
        TCL_VIEW_SPECS
    }

    fn mode_specs(&self) -> &[ModeSpec] {
        TCL_MODE_SPECS
    }

    fn list_len(&self, view: ViewId) -> usize {
        let Some(ev) = to_explorer_view(view) else {
            return 0;
        };
        self.core().list_len(ev)
    }

    fn list_entry(&self, view: ViewId, index: usize) -> Option<ListEntry> {
        let ev = to_explorer_view(view)?;
        let entry = self.core().list_entry(ev, index)?;
        Some(ListEntry {
            text: entry.text,
            group: entry.group,
            is_group_start: entry.is_group_start,
        })
    }

    fn list_detail(&self, view: ViewId, index: usize) -> Option<String> {
        let ev = to_explorer_view(view)?;
        self.core().list_detail(ev, index)
    }

    fn list_link(&self, view: ViewId, index: usize) -> Option<Link> {
        let snapshot = self.core().snapshot()?;
        let instruction_at = |pc: i32| {
            snapshot
                .instructions
                .iter()
                .position(|instruction| instruction.pc == pc)
        };
        match view {
            VIEW_INSTRUCTIONS => {
                let instruction = snapshot.instructions.get(index)?;
                let target = instruction.jump_target.filter(|_| instruction.is_jump)?;
                Some(Link {
                    view: VIEW_INSTRUCTIONS,
                    index: instruction_at(target)?,
                    label: format!("jump target pc {target}"),
                })
            }
            VIEW_LITERALS => {
                let literal = i32::try_from(index).ok()?;
                let (position, instruction) =
                    snapshot
                        .instructions
                        .iter()
                        .enumerate()
                        .find(|(_, instruction)| {
                            instruction.op.starts_with("push")
                                && instruction.operands.first() == Some(&literal)
                        })?;
                Some(Link {
                    view: VIEW_INSTRUCTIONS,
                    index: position,
                    label: format!("first use of literal {index} at pc {}", instruction.pc),
                })
            }
            VIEW_EXCEPTIONS => {
                let range = snapshot.exceptions.get(index)?;
                let start = range.code_range[0];
                let position = snapshot
                    .instructions
                    .iter()
                    .position(|instruction| instruction.pc >= start)?;
                Some(Link {
                    view: VIEW_INSTRUCTIONS,
                    index: position,
                    label: format!("start of exception range {index} at pc {start}"),
                })
            }
            _ => None,
        }
    }

    fn jump_index(&self, view: ViewId, value: i32) -> Option<usize> {
        let ev = to_explorer_view(view)?;
        let mut core = self.core().clone();
        match core.execute_command(ExplorerCommandRequest::Goto { view: ev, value }) {
            Ok(ExplorerCommandResult::GotoHit(index)) => Some(index),
            _ => None,
        }
    }

    fn document_text(&mut self, view: ViewId) -> Result<String, String> {
        let Some(ev) = to_explorer_view(view) else {
            return Ok(String::new());
        };
        if view == VIEW_SOURCE
            && let Some(source) = self.core().source_artifact()
        {
            return Ok(source.text);
        }
        self.core_mut().document_text(ev).map_err(|e| e.to_string())
    }

    fn document_anchors(&mut self, view: ViewId) -> Result<Vec<DocumentAnchor>, String> {
        let Some(ev) = to_explorer_view(view) else {
            return Ok(Vec::new());
        };
        if ev != ExplorerView::Decompile && ev != ExplorerView::Source {
            return Ok(Vec::new());
        }
        Ok(self
            .core()
            .source_command_anchors()
            .into_iter()
            .filter(|anchor| anchor.display_path.contains("/n"))
            .map(|anchor| {
                let parent_path = anchor.parent_root_group.map(|group| format!("c{group:03}"));
                DocumentAnchor {
                    id: format!("source-map:{}", anchor.source_map_index),
                    display_path: anchor.display_path,
                    parent_path,
                    label: anchor.snippet,
                    line_span: anchor.source_lines,
                    group: anchor.parent_root_group,
                }
            })
            .collect())
    }

    fn document_detail(
        &self,
        view: ViewId,
        line: usize,
        search_query: Option<&str>,
    ) -> Option<String> {
        let explorer_view = to_explorer_view(view)?;
        if explorer_view == ExplorerView::Source {
            let mut detail = self.core().list_detail(explorer_view, line)?;
            if let Some(query) = search_query {
                let _ = writeln!(detail, "search: {query}");
            }
            return Some(detail);
        }

        let source = self.core().source_artifact()?;
        let mut detail = format!(
            "document line: {}\nprovenance: {:?}\n",
            line + 1,
            source.provenance
        );
        if let Some(path) = source
            .source_path
            .as_deref()
            .or(source.generated_from.as_deref())
        {
            let _ = writeln!(detail, "source: {path}");
        }
        if let Some(anchor) = self
            .core()
            .source_command_anchors()
            .into_iter()
            .find(|anchor| anchor.source_lines.0 <= line && line < anchor.source_lines.1)
        {
            let _ = writeln!(detail, "anchor: {}", anchor.display_path);
            let _ = writeln!(detail, "content: {}", anchor.snippet);
        }
        if let Some(query) = search_query {
            let _ = writeln!(detail, "search: {query}");
        }
        Some(detail)
    }

    fn command_group_count(&self) -> usize {
        self.core().command_group_count()
    }

    fn group_for_index(&self, index: usize) -> Option<usize> {
        self.core().instruction_group_for_index(index)
    }

    fn first_index_for_group(&self, group: usize) -> Option<usize> {
        self.core().first_instruction_for_group(group)
    }

    fn decompile_anchors(&mut self) -> Result<Vec<usize>, String> {
        self.core_mut()
            .all_decompile_anchors()
            .map_err(|e| e.to_string())
    }

    fn decompile_group_spans(&mut self) -> Result<Vec<Option<(usize, usize)>>, String> {
        self.core_mut()
            .all_decompile_group_line_spans()
            .map_err(|e| e.to_string())
    }

    fn decompile_anchor_for_index(&mut self, index: usize) -> Result<Option<usize>, String> {
        self.core_mut()
            .decompile_anchor_for_instruction(index)
            .map_err(|e| e.to_string())
    }

    fn trace_len(&self) -> usize {
        self.core().trace_len()
    }

    fn trace_text(&self, index: usize) -> Option<String> {
        let frame = self.core().trace_frame(index)?;
        Some(format!(
            "instruction: {}\npc: {}\nopcode: {}\nargc: {}\nstack delta: {}\nstack depth: {} -> {}\npopped: {:?}\npushed: {:?}\nnext: {:?}",
            frame.instruction_index,
            frame.pc,
            frame.opcode,
            frame.argc,
            frame.stack_delta,
            frame.before_depth,
            frame.after_depth,
            frame.popped,
            frame.pushed,
            frame.next_locations
        ))
    }

    fn semantic_diff_report(&mut self, other_path: &str) -> Result<String, String> {
        self.core_mut()
            .semantic_diff_report(other_path)
            .map(|report| report.text)
            .map_err(|e| e.to_string())
    }

    fn semantic_diff(&mut self, other_path: &str) -> Result<SemanticDiff, String> {
        self.core_mut()
            .semantic_diff_report(other_path)
            .map(|report| SemanticDiff {
                text: report.text,
                change_lines: report.change_lines,
            })
            .map_err(|error| error.to_string())
    }

    fn header_info(&self) -> Option<String> {
        let summary = self.core().summary_text();
        let version = summary
            .lines()
            .find(|l| l.starts_with("header:"))
            .and_then(|line| {
                let idx = line.find("tcl=")?;
                Some(line[idx + 4..].split_whitespace().next().unwrap_or("?"))
            });
        let source = self.core().source_artifact()?;
        let provenance = match source.provenance {
            tcld_explore::SourceProvenance::Original => {
                format!(
                    "source: existing {}",
                    source.source_path.as_deref().unwrap_or(self.core().path())
                )
            }
            tcld_explore::SourceProvenance::GeneratedDecompiled => format!(
                "source: generated from {}",
                source
                    .generated_from
                    .as_deref()
                    .unwrap_or(self.core().path())
            ),
        };
        Some(match version {
            Some(version) => format!("tcl {version} | {provenance}"),
            None => provenance,
        })
    }

    fn capability_chips(&self) -> Vec<String> {
        self.core().capability_chips()
    }

    fn export_status(&self) -> String {
        self.core().export_status_label()
    }

    fn export_availability(&self) -> OperationAvailability {
        if self.core().generated_source_export_available() {
            OperationAvailability::Available
        } else {
            OperationAvailability::Unavailable {
                reason: self.core().export_status_label(),
            }
        }
    }

    fn empty_list_message(&self, view: ViewId) -> String {
        let title = tcld_tui::views::title(TCL_VIEW_SPECS, view);
        format!("No entries in {title}; use palette/help for valid domain commands")
    }

    fn empty_document_message(&self, view: ViewId) -> String {
        let title = tcld_tui::views::title(TCL_VIEW_SPECS, view);
        format!("No {title} document is available for this artifact")
    }

    // ── Rendering ────────────────────────────────────────────────

    fn render_list_entry(&self, view: ViewId, text: &str, theme: &Theme) -> Vec<Span<'static>> {
        if view == VIEW_INSTRUCTIONS {
            style::highlight_instruction_text(text, theme)
        } else if view == VIEW_SOURCE {
            style::highlight_source_list_text(text, theme).spans
        } else {
            vec![Span::styled(text.to_string(), theme.fg())]
        }
    }

    fn render_detail(&self, text: &str, theme: &Theme) -> Vec<Line<'static>> {
        style::highlight_detail_text(text, theme)
    }

    fn render_document(&self, view: ViewId, text: &str, theme: &Theme) -> Text<'static> {
        if view == VIEW_SUMMARY {
            Text::from(style::plain_lines(text, theme))
        } else {
            self.highlight().text(text, theme)
        }
    }

    /// Highlight only the visible rows, but parse the whole document, so a
    /// window that opens mid-procedure is still styled correctly.
    fn render_document_window(
        &self,
        view: ViewId,
        lines: &[String],
        theme: &Theme,
        start_line: usize,
        end_line: usize,
    ) -> Text<'static> {
        if view == VIEW_SUMMARY {
            let start = start_line.min(lines.len());
            let end = end_line.min(lines.len()).max(start);
            return Text::from(style::plain_lines(&lines[start..end].join("\n"), theme));
        }
        self.highlight().window(lines, start_line, end_line, theme)
    }

    fn render_decompile_window(&self, context: DecompileRenderContext<'_>) -> Text<'static> {
        style::build_decompile_text_window(
            context.lines,
            context.anchors,
            context.group_spans,
            context.document_anchors,
            context.focused_group,
            context.theme,
            context.start_line,
            context.end_line,
            &|index| self.highlight().line(context.lines, index, context.theme),
        )
    }

    fn decompile_view(&self) -> Option<ViewId> {
        Some(VIEW_DECOMPILE)
    }

    fn export_generated_source(&self, path: &str) -> Result<String, String> {
        let mut core = self.core().clone();
        match core.execute_command(ExplorerCommandRequest::ExportGeneratedSource {
            path: path.to_string(),
        }) {
            Ok(ExplorerCommandResult::Exported(result)) => Ok(format!(
                "exported generated source to {} ({} bytes)",
                result.path, result.bytes_written
            )),
            Ok(_) => Err("unexpected generated-source export result".to_string()),
            Err(diagnostic) => Err(diagnostic.message),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::cli::ExploreArgs;

    fn fixture(path: &str) -> String {
        format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn tui_data_source_delegates_to_tcld_explore_contract() {
        let mut source = ExplorerCore::new(ExploreArgs {
            input_path: fixture("expr.tcl"),
            dump_screen: false,
            mode: None,
            overlay: None,
            palette_query: None,
            diff_target: None,
            page_size: 20,
            theme: None,
            no_colors: false,
        })
        .expect("source loads");

        assert_eq!(DataSource::default_view(&source), VIEW_SOURCE);
        assert_eq!(DataSource::jump_index(&source, VIEW_SOURCE, 1), Some(0));
        assert_eq!(DataSource::jump_index(&source, VIEW_SOURCE, 99_999), None);
        let decompile =
            DataSource::document_text(&mut source, VIEW_DECOMPILE).expect("decompile renders");
        assert!(decompile.contains("source rendering"));
        assert!(
            DataSource::header_info(&source)
                .unwrap()
                .contains("source: existing")
        );

        let mut bytecode = ExplorerCore::new(ExploreArgs {
            input_path: fixture("expr.tbc"),
            dump_screen: false,
            mode: None,
            overlay: None,
            palette_query: None,
            diff_target: None,
            page_size: 20,
            theme: None,
            no_colors: false,
        })
        .expect("bytecode loads");

        assert!(DataSource::available_views(&bytecode).contains(&VIEW_SOURCE));
        let source = DataSource::document_text(&mut bytecode, VIEW_SOURCE).expect("source renders");
        assert!(source.contains("set result"));
        assert!(!source.contains("generated decompiled Tcl source"));
        assert!(
            DataSource::header_info(&bytecode)
                .unwrap()
                .contains("source: generated from")
        );
        let export_dir = tempfile::tempdir().expect("temporary export directory");
        assert!(
            bytecode
                .export_generated_source(export_dir.path().join("tcld-ds-export.tcl"))
                .is_ok()
        );

        let temp = std::env::temp_dir().join(format!("tcld-app-export-{}.tcl", std::process::id()));
        let _ = std::fs::remove_file(&temp);
        let mut app = tcld_tui::TuiApp::new(bytecode, false);
        app.execute_command("source");
        app.execute_command("search expr");
        assert_eq!(app.current_view(), VIEW_SOURCE);
        assert!(!app.search.hits.is_empty());
        app.execute_command("goto 1");
        assert_eq!(app.nav.selection(VIEW_SOURCE), 0);
        app.execute_command(&format!("export {}", temp.display()));
        assert!(
            app.cmd
                .output
                .as_deref()
                .unwrap_or("")
                .contains("exported generated source")
        );
        assert!(std::fs::read_to_string(&temp).unwrap().ends_with('\n'));
        let _ = std::fs::remove_file(temp);
    }
}
