use crate::theme::Theme;
use crate::views::ViewSpec;
use ratatui::text::{Line, Span, Text};
use std::fmt;

/// Identifies a view by its static string token (e.g. "instructions", "decompile").
///
/// Consumers define their own `ViewId` constants. The TUI framework treats
/// them as opaque identifiers — no domain-specific constants live here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewId(pub &'static str);

impl fmt::Display for ViewId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

pub struct DecompileRenderContext<'a> {
    pub lines: &'a [String],
    pub anchors: &'a [usize],
    pub group_spans: &'a [Option<(usize, usize)>],
    pub document_anchors: &'a [DocumentAnchor],
    pub focused_group: Option<usize>,
    pub theme: &'a Theme,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentAnchor {
    pub id: String,
    pub display_path: String,
    pub parent_path: Option<String>,
    pub label: String,
    pub line_span: (usize, usize),
}

pub const MAX_RETAINED_SEARCH_HITS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub hit_indices: Vec<usize>,
    pub total_matches: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiff {
    pub text: String,
    pub change_lines: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationAvailability {
    Available,
    Unavailable { reason: String },
}

/// A single entry in a list view.
#[derive(Debug, Clone)]
pub struct ListEntry {
    /// The formatted text content of this entry.
    pub text: String,
    /// The command group this entry belongs to, if any.
    pub group: Option<usize>,
    /// Whether this entry is the first instruction in its group.
    pub is_group_start: bool,
}

/// The central abstraction separating the TUI from domain-specific data.
///
/// The TUI crate never touches `ByteCode`, `Instruction`, or any tcld-core
/// types directly. All data flows through this trait, which the consumer
/// implements for their model type.
pub trait DataSource {
    // ── Identity ──────────────────────────────────────────────────

    /// Path to the loaded file.
    fn path(&self) -> &str;

    /// Which views are available for the loaded data.
    fn available_views(&self) -> &[ViewId];

    /// The view to display on startup.
    fn default_view(&self) -> ViewId;

    /// Whether to suppress colors (monochrome mode).
    fn no_colors(&self) -> bool;

    /// Page size for scrolling (number of items to skip per PgUp/PgDown).
    fn page_size(&self) -> usize;

    /// Summary text shown in the summary modal.
    fn summary_text(&self) -> String;

    /// View specifications for all views this data source supports.
    /// The framework uses these for tab bar, palette, keybinds, help overlay, etc.
    fn view_specs(&self) -> &[ViewSpec];

    /// Application mode specifications available for this data source.
    fn mode_specs(&self) -> &[crate::state::ModeSpec] {
        crate::state::DEFAULT_MODE_SPECS
    }

    // ── List views ────────────────────────────────────────────────

    /// Total number of entries in a list view.
    fn list_len(&self, view: ViewId) -> usize;

    /// Get a single list entry at `index`.
    fn list_entry(&self, view: ViewId, index: usize) -> Option<ListEntry>;

    /// Get detail text for the selected list entry (shown in the context panel).
    fn list_detail(&self, view: ViewId, index: usize) -> Option<String>;

    /// Bounded list search used by the interactive UI.
    fn search_result(&self, view: ViewId, needle: &str) -> SearchResult {
        let needle = needle.to_ascii_lowercase();
        let mut hit_indices = Vec::new();
        let mut total_matches = 0usize;
        for index in 0..self.list_len(view) {
            let Some(entry) = self.list_entry(view, index) else {
                continue;
            };
            if entry.text.to_ascii_lowercase().contains(&needle) {
                total_matches += 1;
                if hit_indices.len() < MAX_RETAINED_SEARCH_HITS {
                    hit_indices.push(index);
                }
            }
        }
        SearchResult {
            truncated: total_matches > hit_indices.len(),
            total_matches,
            hit_indices,
        }
    }

    /// Jump to a value (e.g. PC or line number). Returns the target list index.
    fn jump_index(&self, view: ViewId, value: i32) -> Option<usize>;

    // ── Document views ────────────────────────────────────────────

    /// Monotonic content revision for a document view.
    ///
    /// Implementors that can change document content while the TUI is open must
    /// increment this value whenever the text or anchors change. Static sources
    /// can use the default revision.
    fn document_revision(&self, _view: ViewId) -> u64 {
        0
    }

    /// Full text for a document-style view.
    fn document_text(&mut self, view: ViewId) -> Result<String, String>;

    /// Bounded document search used by the interactive UI.
    fn document_search_result(
        &mut self,
        view: ViewId,
        needle: &str,
    ) -> Result<SearchResult, String> {
        let text = self.document_text(view)?;
        let needle = needle.to_ascii_lowercase();
        let mut hit_indices = Vec::new();
        let mut total_matches = 0usize;
        for (index, line) in text.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&needle) {
                total_matches += 1;
                if hit_indices.len() < MAX_RETAINED_SEARCH_HITS {
                    hit_indices.push(index);
                }
            }
        }
        Ok(SearchResult {
            truncated: total_matches > hit_indices.len(),
            total_matches,
            hit_indices,
        })
    }

    /// Generic line-oriented anchors for document views.
    fn document_anchors(&mut self, _view: ViewId) -> Result<Vec<DocumentAnchor>, String> {
        Ok(Vec::new())
    }

    /// Context shown in the inspector for the visible document line.
    fn document_detail(
        &self,
        _view: ViewId,
        _line: usize,
        _search_query: Option<&str>,
    ) -> Option<String> {
        None
    }

    // ── Command groups ────────────────────────────────────────────

    /// Number of command groups in the bytecode.
    fn command_group_count(&self) -> usize;

    /// Map a list entry index to its command group.
    fn group_for_index(&self, index: usize) -> Option<usize>;

    /// Map a command group to its first list entry index.
    fn first_index_for_group(&self, group: usize) -> Option<usize>;

    // ── Decompile mapping (optional) ──────────────────────────────

    /// Per-group anchor line indices in the decompiled text.
    fn decompile_anchors(&mut self) -> Result<Vec<usize>, String> {
        Ok(vec![])
    }

    /// Per-group line spans `(start, end)` in the decompiled text.
    fn decompile_group_spans(&mut self) -> Result<Vec<Option<(usize, usize)>>, String> {
        Ok(vec![])
    }

    /// Decompile anchor line for a specific instruction index.
    fn decompile_anchor_for_index(&mut self, _index: usize) -> Result<Option<usize>, String> {
        Ok(None)
    }

    // ── Trace (optional) ──────────────────────────────────────────

    /// Number of trace frames available.
    fn trace_len(&self) -> usize {
        0
    }

    /// Formatted trace text for the instruction at `index`.
    fn trace_text(&self, _index: usize) -> Option<String> {
        None
    }

    // ── Diff (optional) ──────────────────────────────────────────

    /// Open a second file for side-by-side comparison.
    fn open_diff_target(&self, _path: &str) -> Result<Box<dyn DataSource>, String> {
        Err("Semantic comparison is unavailable.".to_string())
    }

    /// Semantic diff report comparing this file against `other_path`.
    fn semantic_diff_report(&mut self, _other_path: &str) -> Result<String, String> {
        Err("Semantic comparison is unavailable.".to_string())
    }

    /// Structured semantic diff used by Compare navigation.
    fn semantic_diff(&mut self, _other_path: &str) -> Result<SemanticDiff, String> {
        Err("Semantic comparison is unavailable.".to_string())
    }

    /// Export generated source, when a consumer has one.
    fn export_generated_source(&self, _path: &str) -> Result<String, String> {
        Err("This file has no generated source to export.".to_string())
    }

    // ── File header metadata (optional) ───────────────────────────

    /// Short header info shown next to the filename (e.g. "Tcl 8.6, 42 opcodes").
    fn header_info(&self) -> Option<String> {
        None
    }

    /// Domain capability chips shown in the header.
    fn capability_chips(&self) -> Vec<String> {
        Vec::new()
    }

    /// Generated-source or export control state shown in the status bar.
    fn export_status(&self) -> String {
        "Export unavailable".to_string()
    }

    /// Typed availability for generated-source export.
    fn export_availability(&self) -> OperationAvailability {
        OperationAvailability::Unavailable {
            reason: "This file has no generated source to export.".to_string(),
        }
    }

    /// Empty-state copy for list panes.
    fn empty_list_message(&self, _view: ViewId) -> String {
        "No entries".to_string()
    }

    /// Empty-state copy for document panes.
    fn empty_document_message(&self, _view: ViewId) -> String {
        "No document content for this view".to_string()
    }

    // ── Rendering (optional — plain-text defaults) ────────────────

    /// Render a single list entry as styled spans.
    /// Override to provide domain-specific syntax highlighting.
    fn render_list_entry(&self, _view: ViewId, text: &str, theme: &Theme) -> Vec<Span<'static>> {
        vec![Span::styled(text.to_string(), theme.fg())]
    }

    /// Render detail/context text as styled lines.
    /// Override to provide domain-specific formatting (e.g. key:value highlighting).
    fn render_detail(&self, text: &str, theme: &Theme) -> Vec<Line<'static>> {
        crate::style::plain_lines(text, theme)
    }

    /// Render document text as styled Text.
    /// Override to provide domain-specific syntax highlighting.
    fn render_document(&self, _view: ViewId, text: &str, theme: &Theme) -> Text<'static> {
        Text::from(crate::style::plain_lines(text, theme))
    }

    /// Style only the visible lines of a document view.
    fn render_document_window(
        &self,
        view: ViewId,
        lines: &[String],
        theme: &Theme,
        start_line: usize,
        end_line: usize,
    ) -> Text<'static> {
        let start = start_line.min(lines.len());
        let end = end_line.min(lines.len()).max(start);
        self.render_document(view, &lines[start..end].join("\n"), theme)
    }

    /// Render a decompile-style document view with group gutter.
    /// Only called when `decompile_view()` returns `Some(view)` for the current view.
    fn render_decompile_window(&self, context: DecompileRenderContext<'_>) -> Text<'static> {
        crate::style::build_plain_decompile_window(
            context.lines,
            context.start_line,
            context.end_line,
            context.theme,
        )
    }

    // ── View identity (optional) ──────────────────────────────────

    /// Which view, if any, should get decompile-style rendering (group gutter)?
    /// Returns `None` by default (no view gets special treatment).
    fn decompile_view(&self) -> Option<ViewId> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::{ViewOpenBehavior, ViewSpec};

    const LIST: ViewId = ViewId("list");
    const DOCUMENT: ViewId = ViewId("document");
    const SPECS: &[ViewSpec] = &[
        ViewSpec {
            view: LIST,
            token: "list",
            title: "List",
            aliases: &["list"],
            keybind: None,
            icon: "",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: true,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: DOCUMENT,
            token: "document",
            title: "Document",
            aliases: &["document"],
            keybind: None,
            icon: "",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
    ];

    struct RepetitiveData;

    impl DataSource for RepetitiveData {
        fn path(&self) -> &str {
            "large.tcl"
        }

        fn available_views(&self) -> &[ViewId] {
            &[LIST, DOCUMENT]
        }

        fn default_view(&self) -> ViewId {
            LIST
        }

        fn no_colors(&self) -> bool {
            true
        }

        fn page_size(&self) -> usize {
            20
        }

        fn summary_text(&self) -> String {
            String::new()
        }

        fn view_specs(&self) -> &[ViewSpec] {
            SPECS
        }

        fn list_len(&self, view: ViewId) -> usize {
            if view == LIST { 1_005 } else { 0 }
        }

        fn list_entry(&self, view: ViewId, index: usize) -> Option<ListEntry> {
            (view == LIST && index < 1_005).then(|| ListEntry {
                text: format!("matching row {index}"),
                group: None,
                is_group_start: false,
            })
        }

        fn list_detail(&self, _view: ViewId, _index: usize) -> Option<String> {
            None
        }

        fn jump_index(&self, _view: ViewId, _value: i32) -> Option<usize> {
            None
        }

        fn document_text(&mut self, view: ViewId) -> Result<String, String> {
            if view == DOCUMENT {
                Ok((0..1_005)
                    .map(|index| format!("matching line {index}"))
                    .collect::<Vec<_>>()
                    .join("\n"))
            } else {
                Ok(String::new())
            }
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

    #[test]
    fn typed_search_counts_all_matches_but_bounds_retained_navigation_hits() {
        let mut data = RepetitiveData;
        let list = data.search_result(LIST, "matching");
        assert_eq!(list.total_matches, 1_005);
        assert_eq!(list.hit_indices.len(), MAX_RETAINED_SEARCH_HITS);
        assert!(list.truncated);

        let document = data
            .document_search_result(DOCUMENT, "matching")
            .expect("document search");
        assert_eq!(document.total_matches, 1_005);
        assert_eq!(document.hit_indices.len(), MAX_RETAINED_SEARCH_HITS);
        assert!(document.truncated);

        let list_below_limit = data.search_result(LIST, "line 4");
        assert!(
            list_below_limit.total_matches < MAX_RETAINED_SEARCH_HITS,
            "{list_below_limit:?}"
        );
        assert_eq!(
            list_below_limit.hit_indices.len(),
            list_below_limit.total_matches
        );
        assert!(!list_below_limit.truncated);

        let document_below_limit = data
            .document_search_result(DOCUMENT, "line 4")
            .expect("document search below limit");
        assert!(
            document_below_limit.total_matches < MAX_RETAINED_SEARCH_HITS,
            "{document_below_limit:?}"
        );
        assert_eq!(
            document_below_limit.hit_indices.len(),
            document_below_limit.total_matches
        );
        assert!(!document_below_limit.truncated);
    }
}
