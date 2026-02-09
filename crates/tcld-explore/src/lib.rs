//! UI-agnostic explorer contract for Tcl artifacts.

use similar::{ChangeTag, TextDiff};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::path::Path;
use tcld_core::analysis::TraceFrame;
use tcld_core::error::{Result, TcldError};
use tcld_core::util::truncate;
use tcld_decompile::decompiler::DecompiledCommandSpan;
use tcld_decompile::snapshot::{ExceptionSnapshot, InstructionSnapshot, LiteralSnapshot};
use tcld_decompile::{AnalysisSnapshot, LoadInput, ReconstructionStatus, canonicalize_tcl};

const LIST_TEXT_TRUNCATE: usize = 90;
const MAX_DIFF_DETAILS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtifactKind {
    RawBytecode,
    TclSourceWithBytecode,
    PlainTclSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceProvenance {
    Original,
    GeneratedDecompiled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceArtifact {
    pub provenance: SourceProvenance,
    pub label: String,
    pub text: String,
    pub source_path: Option<String>,
    pub generated_from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExplorerView {
    Instructions,
    Literals,
    Exceptions,
    Source,
    Decompile,
    Summary,
}

impl ExplorerView {
    pub fn title(self) -> &'static str {
        match self {
            Self::Instructions => "Instructions",
            Self::Literals => "Literals",
            Self::Exceptions => "Exceptions",
            Self::Source => "Source",
            Self::Decompile => "Decompile",
            Self::Summary => "Summary",
        }
    }

    pub fn is_list_view(self) -> bool {
        matches!(self, Self::Instructions | Self::Literals | Self::Exceptions)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    BytecodeViews,
    SourceViews,
    GeneratedSourceExport,
    SemanticDiff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExploreOptions {
    pub input_path: String,
    pub text_truncate: Option<usize>,
    pub page_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub text: String,
    pub group: Option<usize>,
    pub is_group_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportResult {
    pub path: String,
    pub bytes_written: usize,
    pub provenance: SourceProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiffReport {
    pub text: String,
    pub change_lines: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableRenderPayload {
    pub artifact_kind: ArtifactKind,
    pub provenance: SourceProvenance,
    pub header: String,
    pub capability_chips: Vec<String>,
    pub available_views: Vec<ExplorerView>,
    pub default_view: ExplorerView,
    pub primary_list: Vec<String>,
    pub document_title: String,
    pub document_text: String,
    pub inspector_text: String,
    pub command_group_map: Vec<CommandGroupMapEntry>,
    pub source_command_anchors: Vec<SourceCommandAnchor>,
    pub diagnostics: Vec<Diagnostic>,
    pub export_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandGroupMapEntry {
    pub group: usize,
    pub first_instruction: usize,
    pub anchor_line: Option<usize>,
    pub line_span: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCommandAnchorKind {
    Root,
    Nested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommandAnchor {
    pub kind: SourceCommandAnchorKind,
    pub source_map_index: usize,
    pub command_path: Vec<usize>,
    pub display_path: String,
    pub parent_root_group: Option<usize>,
    pub command_group: Option<usize>,
    pub source_lines: (usize, usize),
    pub snippet: String,
    pub provenance: SourceProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerCommandRequest {
    SearchList {
        view: ExplorerView,
        needle: String,
        limit: usize,
    },
    SearchDocument {
        view: ExplorerView,
        needle: String,
        limit: usize,
    },
    Goto {
        view: ExplorerView,
        value: i32,
    },
    ExportGeneratedSource {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerCommandResult {
    SearchHits(Vec<(usize, String)>),
    GotoHit(usize),
    Miss(Diagnostic),
    Exported(ExportResult),
}

#[derive(Debug, Clone)]
struct GroupSignature {
    index: usize,
    content_signature: String,
    opcode_signature: String,
    instruction_count: usize,
    jump_count: usize,
    decompile_snippet: String,
}

#[derive(Debug, Clone)]
struct ExceptionSignature {
    index: usize,
    content_signature: String,
    detail: String,
}

#[derive(Clone)]
enum ExplorerMode {
    Bytecode,
    Source,
}

const BYTECODE_VIEWS: &[ExplorerView] = &[
    ExplorerView::Instructions,
    ExplorerView::Literals,
    ExplorerView::Exceptions,
    ExplorerView::Source,
    ExplorerView::Decompile,
    ExplorerView::Summary,
];

const SOURCE_VIEWS: &[ExplorerView] = &[
    ExplorerView::Source,
    ExplorerView::Decompile,
    ExplorerView::Summary,
];

#[derive(Clone)]
pub struct ExplorerCore {
    path: String,
    text_truncate: Option<usize>,
    page_size: usize,
    artifact_kind: ArtifactKind,
    mode: ExplorerMode,
    source_artifact: SourceArtifact,
    source_lines: Vec<String>,
    snapshot: Option<AnalysisSnapshot>,
    decompile_cache: Option<String>,
    decompile_group_anchors: Option<Vec<usize>>,
    decompile_group_line_spans: Option<Vec<Option<(usize, usize)>>>,
    decompile_command_spans: Option<Vec<DecompiledCommandSpan>>,
    group_signatures_cache: Option<Vec<GroupSignature>>,
}

impl ExplorerCore {
    pub fn new(options: ExploreOptions) -> Result<Self> {
        let bytecode_error =
            match AnalysisSnapshot::from_input(LoadInput::path(&options.input_path)) {
                Ok(snapshot) => return Ok(Self::from_snapshot(options, snapshot)),
                Err(error) => error,
            };

        let source = load_source_from_path_if_text(&options.input_path).map_err(|source_error| {
            TcldError::Other(format!(
                "failed to load {} as bytecode or Tcl source\n  bytecode: {bytecode_error}\n  source: {source_error}",
                options.input_path
            ))
        })?;
        let source = canonicalize_with_one_trailing_newline(&source);
        let source_artifact = SourceArtifact {
            provenance: SourceProvenance::Original,
            label: format!("plain Tcl source: {}", options.input_path),
            text: source,
            source_path: Some(options.input_path.clone()),
            generated_from: None,
        };
        Ok(Self::from_parts(
            options,
            ArtifactKind::PlainTclSource,
            ExplorerMode::Source,
            source_artifact,
            None,
        ))
    }

    pub fn from_snapshot(options: ExploreOptions, snapshot: AnalysisSnapshot) -> Self {
        let artifact_kind = classify_bytecode_artifact(&options.input_path);
        let generated = canonicalize_with_one_trailing_newline(&snapshot.decompile.text);
        let source_artifact = SourceArtifact {
            provenance: SourceProvenance::GeneratedDecompiled,
            label: format!(
                "generated decompiled Tcl source derived from bytecode: {}",
                options.input_path
            ),
            text: generated,
            source_path: None,
            generated_from: Some(options.input_path.clone()),
        };
        Self::from_parts(
            options,
            artifact_kind,
            ExplorerMode::Bytecode,
            source_artifact,
            Some(snapshot),
        )
    }

    fn from_parts(
        options: ExploreOptions,
        artifact_kind: ArtifactKind,
        mode: ExplorerMode,
        source_artifact: SourceArtifact,
        snapshot: Option<AnalysisSnapshot>,
    ) -> Self {
        let source_lines = source_artifact
            .text
            .lines()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        Self {
            path: options.input_path,
            text_truncate: options.text_truncate,
            page_size: options.page_size,
            artifact_kind,
            mode,
            source_artifact,
            source_lines,
            snapshot,
            decompile_cache: None,
            decompile_group_anchors: None,
            decompile_group_line_spans: None,
            decompile_command_spans: None,
            group_signatures_cache: None,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn page_size(&self) -> usize {
        self.page_size
    }

    pub fn artifact_kind(&self) -> ArtifactKind {
        self.artifact_kind
    }

    pub fn source_artifact(&self) -> Option<SourceArtifact> {
        Some(self.source_artifact.clone())
    }

    pub fn snapshot(&self) -> Option<&AnalysisSnapshot> {
        self.snapshot.as_ref()
    }

    fn bytecode_snapshot(&self) -> Option<&AnalysisSnapshot> {
        if matches!(self.mode, ExplorerMode::Bytecode) {
            self.snapshot.as_ref()
        } else {
            None
        }
    }

    pub fn views(&self) -> &'static [ExplorerView] {
        match self.mode {
            ExplorerMode::Bytecode => BYTECODE_VIEWS,
            ExplorerMode::Source => SOURCE_VIEWS,
        }
    }

    pub fn default_view(&self) -> ExplorerView {
        match self.mode {
            ExplorerMode::Bytecode => ExplorerView::Instructions,
            ExplorerMode::Source => ExplorerView::Source,
        }
    }

    pub fn capabilities(&self) -> BTreeSet<Capability> {
        let mut set = BTreeSet::new();
        set.insert(Capability::SourceViews);
        set.insert(Capability::SemanticDiff);
        if matches!(self.mode, ExplorerMode::Bytecode) {
            set.insert(Capability::BytecodeViews);
            set.insert(Capability::GeneratedSourceExport);
        }
        set
    }

    pub fn summary_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "path: {}", self.path);
        let _ = writeln!(out, "artifact: {:?}", self.artifact_kind);
        match &self.mode {
            ExplorerMode::Bytecode => {
                if let Some(snapshot) = self.bytecode_snapshot() {
                    out.push_str(snapshot.summary_text.trim_end());
                    out.push('\n');
                    let _ = writeln!(
                        out,
                        "decompile reconstruction: {:?}",
                        snapshot.diagnostics.reconstruction
                    );
                    for message in snapshot.diagnostics.summary_messages().into_iter().skip(1) {
                        let _ = writeln!(out, "decompile diagnostic: {message}");
                    }
                }
                let _ = writeln!(
                    out,
                    "source: generated source artifact ({})",
                    self.source_artifact.label
                );
            }
            ExplorerMode::Source => {
                let _ = writeln!(
                    out,
                    "mode: source rendering bytes={} lines={} (bytecode views unavailable)",
                    self.source_artifact.text.len(),
                    self.source_lines.len()
                );
                if let Some(path) = &self.source_artifact.source_path {
                    let _ = writeln!(out, "source path: {path}");
                }
            }
        }

        let _ = writeln!(
            out,
            "settings: text_truncate={} page_size={}",
            self.text_truncate
                .map(|n| n.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.page_size,
        );
        out
    }

    pub fn stable_render_payload(&mut self) -> Result<StableRenderPayload> {
        let default_view = self.default_view();
        let primary_list = (0..self.list_len(default_view).min(self.page_size.max(1)))
            .filter_map(|index| self.list_item(default_view, index))
            .collect::<Vec<_>>();
        let document_view = if self.views().contains(&ExplorerView::Decompile) {
            ExplorerView::Decompile
        } else {
            default_view
        };
        let document_text = self.document_text(document_view)?;
        let inspector_text = self
            .list_detail(default_view, 0)
            .unwrap_or_else(|| "select an item to see details".to_string());
        let anchors = self.all_decompile_anchors().unwrap_or_default();
        let spans = self.all_decompile_group_line_spans().unwrap_or_default();
        let command_group_map = (0..self.command_group_count())
            .filter_map(|group| {
                let first_instruction = self.first_instruction_for_group(group)?;
                Some(CommandGroupMapEntry {
                    group,
                    first_instruction,
                    anchor_line: anchors
                        .get(group)
                        .copied()
                        .filter(|value| *value != usize::MAX),
                    line_span: spans.get(group).copied().flatten(),
                })
            })
            .collect::<Vec<_>>();
        let provenance = self.source_artifact.provenance;
        Ok(StableRenderPayload {
            artifact_kind: self.artifact_kind,
            provenance,
            header: self.summary_text().lines().next().unwrap_or("").to_string(),
            capability_chips: self.capability_chips(),
            available_views: self.views().to_vec(),
            default_view,
            primary_list,
            document_title: document_view.title().to_string(),
            document_text,
            inspector_text,
            command_group_map,
            source_command_anchors: self.source_command_anchors(),
            diagnostics: self.diagnostics(),
            export_status: self.export_status_label(),
        })
    }

    pub fn capability_chips(&self) -> Vec<String> {
        self.capabilities()
            .into_iter()
            .map(|capability| match capability {
                Capability::BytecodeViews => "bytecode",
                Capability::SourceViews => "source",
                Capability::GeneratedSourceExport => "generated-export",
                Capability::SemanticDiff => "semantic-diff",
            })
            .map(ToString::to_string)
            .collect()
    }

    pub fn export_status_label(&self) -> String {
        match self.source_artifact.provenance {
            SourceProvenance::GeneratedDecompiled
                if self.decompilation_is_exact_enough_for_export() =>
            {
                "Export ready · exact reconstruction".to_string()
            }
            SourceProvenance::GeneratedDecompiled => {
                let reconstruction = self
                    .bytecode_snapshot()
                    .map(|snapshot| format!("{:?}", snapshot.diagnostics.reconstruction))
                    .unwrap_or_else(|| "unknown".to_string());
                format!("Export unavailable · {reconstruction} reconstruction")
            }
            SourceProvenance::Original => {
                "Export unavailable · source is already present".to_string()
            }
        }
    }

    pub fn generated_source_export_available(&self) -> bool {
        self.source_artifact.provenance == SourceProvenance::GeneratedDecompiled
            && self.decompilation_is_exact_enough_for_export()
    }

    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        match self.mode {
            ExplorerMode::Bytecode => {
                let Some(snapshot) = self.bytecode_snapshot() else {
                    return Vec::new();
                };
                let mut diagnostics = Vec::new();
                if snapshot.diagnostics.reconstruction != ReconstructionStatus::Strict {
                    diagnostics.push(Diagnostic {
                        code: "decompile-reconstruction",
                        message: format!(
                            "Reconstruction: {:?}",
                            snapshot.diagnostics.reconstruction
                        ),
                    });
                }
                if !snapshot.diagnostics.unreduced_opcodes.is_empty() {
                    diagnostics.push(Diagnostic {
                        code: "decompile-unreduced-opcodes",
                        message: format!(
                            "{} unreduced opcode(s): {}",
                            snapshot.diagnostics.unreduced_opcodes.len(),
                            snapshot.diagnostics.unreduced_opcodes.join(", ")
                        ),
                    });
                }
                for approximation in &snapshot.diagnostics.approximations {
                    let scope = approximation
                        .bytecode_range
                        .as_ref()
                        .map(|range| format!("bytecode {}..{}", range.start, range.end))
                        .unwrap_or_else(|| "nested procedure scope".to_string());
                    diagnostics.push(Diagnostic {
                        code: "decompile-approximation",
                        message: format!(
                            "{:?} approximation at {scope}: {}",
                            approximation.kind, approximation.reason
                        ),
                    });
                }
                if !snapshot.diagnostics.unsupported_instructions.is_empty() {
                    diagnostics.push(Diagnostic {
                        code: "decompile-unsupported-instructions",
                        message: format!(
                            "{} unsupported instruction(s): {}",
                            snapshot.diagnostics.unsupported_instructions.len(),
                            snapshot.diagnostics.unsupported_instructions.join(", ")
                        ),
                    });
                }
                diagnostics
            }
            ExplorerMode::Source => vec![Diagnostic {
                code: "source-only-artifact",
                message: "bytecode instruction and generated export views are unavailable for plain Tcl source".to_string(),
            }],
        }
    }

    pub fn list_len(&self, view: ExplorerView) -> usize {
        match (&self.mode, view) {
            (ExplorerMode::Bytecode, ExplorerView::Instructions) => self
                .bytecode_snapshot()
                .map(|snapshot| snapshot.instructions.len())
                .unwrap_or(0),
            (ExplorerMode::Bytecode, ExplorerView::Literals) => self
                .bytecode_snapshot()
                .map(|snapshot| snapshot.literals.len())
                .unwrap_or(0),
            (ExplorerMode::Bytecode, ExplorerView::Exceptions) => self
                .bytecode_snapshot()
                .map(|snapshot| snapshot.exceptions.len())
                .unwrap_or(0),
            (_, ExplorerView::Source) => self.source_lines.len(),
            _ => 0,
        }
    }

    pub fn list_entry(&self, view: ExplorerView, index: usize) -> Option<ListEntry> {
        match (&self.mode, view) {
            (ExplorerMode::Bytecode, ExplorerView::Instructions) => {
                let inst = self.bytecode_snapshot()?.instructions.get(index)?;
                Some(ListEntry {
                    text: render_instruction_snapshot(inst, self.text_truncate),
                    group: inst.group,
                    is_group_start: inst.group_start,
                })
            }
            (ExplorerMode::Bytecode, ExplorerView::Literals) => {
                let lit = self.bytecode_snapshot()?.literals.get(index)?;
                let rendered = render_literal_snapshot(lit, self.text_truncate);
                Some(ListEntry {
                    text: format!("{index:>4}: {} {rendered}", lit.kind),
                    group: None,
                    is_group_start: false,
                })
            }
            (ExplorerMode::Bytecode, ExplorerView::Exceptions) => {
                let ex = self.bytecode_snapshot()?.exceptions.get(index)?;
                Some(ListEntry {
                    text: format!(
                        "{index:>4}: {} nest={} code={}..{} break={} cont={} catch={}",
                        ex.kind,
                        ex.nesting_level,
                        ex.code_range[0],
                        ex.code_range[1],
                        ex.break_offset,
                        ex.continue_offset,
                        ex.catch_offset
                    ),
                    group: None,
                    is_group_start: false,
                })
            }
            (_, ExplorerView::Source) => {
                let line = self.source_lines.get(index)?;
                Some(ListEntry {
                    text: format!(
                        "{:>4}: {}",
                        index + 1,
                        truncate(line, LIST_TEXT_TRUNCATE, "...")
                    ),
                    group: None,
                    is_group_start: false,
                })
            }
            _ => None,
        }
    }

    pub fn list_item(&self, view: ExplorerView, index: usize) -> Option<String> {
        let entry = self.list_entry(view, index)?;
        Some(self.format_entry(view, &entry))
    }

    pub fn format_entry(&self, view: ExplorerView, entry: &ListEntry) -> String {
        match view {
            ExplorerView::Instructions => {
                let group = entry
                    .group
                    .map(|g| format!("c{g:03}"))
                    .unwrap_or_else(|| "c---".to_string());
                let marker = if entry.is_group_start { '>' } else { ' ' };
                format!("{group}{marker} {}", entry.text)
            }
            _ => entry.text.clone(),
        }
    }

    pub fn list_detail(&self, view: ExplorerView, index: usize) -> Option<String> {
        match (&self.mode, view) {
            (ExplorerMode::Bytecode, ExplorerView::Instructions) => {
                let snapshot = self.bytecode_snapshot()?;
                let inst = snapshot.instructions.get(index)?;
                let mut out = String::new();
                let _ = writeln!(out, "instruction index: {index}");
                let _ = writeln!(out, "opcode: {}", inst.op);
                let _ = writeln!(out, "pc: {}", inst.pc);
                let _ = writeln!(out, "size: {}", inst.size);
                let _ = writeln!(out, "argc: {}", inst.operands.len());
                let _ = writeln!(out, "operands: {:?}", inst.operands);
                if let Some(group) = inst.group {
                    let _ = writeln!(out, "command_group: c{group:03}");
                    if let Some(range) = command_group_range(snapshot, group) {
                        let _ = writeln!(out, "command_pc_range: {}..{}", range.0, range.1);
                    }
                } else {
                    let _ = writeln!(out, "command_group: n/a");
                }
                let _ = writeln!(out, "jumps: {}", inst.is_jump);
                let _ = writeln!(
                    out,
                    "jump_target: {}",
                    inst.jump_target
                        .map(|target| target.to_string())
                        .unwrap_or_else(|| "n/a".to_string())
                );
                if !inst.comment.is_empty() {
                    let _ = writeln!(out, "\nvalue:");
                    out.push_str(&inst.comment);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Some(out)
            }
            (ExplorerMode::Bytecode, ExplorerView::Literals) => {
                let lit = self.bytecode_snapshot()?.literals.get(index)?;
                let mut out = String::new();
                let _ = writeln!(out, "literal index: {index}");
                let _ = writeln!(out, "kind: {}", lit.kind);
                let _ = writeln!(out, "\nvalue:");
                out.push_str(&lit.value);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                Some(out)
            }
            (ExplorerMode::Bytecode, ExplorerView::Exceptions) => {
                let ex = self.bytecode_snapshot()?.exceptions.get(index)?;
                let mut out = String::new();
                let _ = writeln!(out, "exception index: {index}");
                let _ = writeln!(out, "kind: {}", ex.kind);
                let _ = writeln!(out, "nesting_level: {}", ex.nesting_level);
                let _ = writeln!(
                    out,
                    "code_range: {}..{}",
                    ex.code_range[0], ex.code_range[1]
                );
                let _ = writeln!(out, "break_offset: {}", ex.break_offset);
                let _ = writeln!(out, "continue_offset: {}", ex.continue_offset);
                let _ = writeln!(out, "catch_offset: {}", ex.catch_offset);
                Some(out)
            }
            (_, ExplorerView::Source) => {
                let line = self.source_lines.get(index)?;
                let mut out = String::new();
                let _ = writeln!(out, "source line: {}", index + 1);
                let _ = writeln!(out, "provenance: {:?}", self.source_artifact.provenance);
                if let Some(generated_from) = &self.source_artifact.generated_from {
                    let _ = writeln!(out, "derived from: {generated_from}");
                }
                let _ = writeln!(out, "\ncontent:");
                let _ = writeln!(out, "{line}");
                Some(out)
            }
            _ => None,
        }
    }

    pub fn document_text(&mut self, view: ExplorerView) -> Result<String> {
        match view {
            ExplorerView::Summary => Ok(self.summary_text()),
            ExplorerView::Decompile => self.decompiled_text(),
            ExplorerView::Source => Ok(self.source_document_text()),
            _ => Ok(String::new()),
        }
    }

    pub fn jump_index(&self, view: ExplorerView, value: i32) -> Option<usize> {
        match (&self.mode, view) {
            (ExplorerMode::Bytecode, ExplorerView::Instructions) => self
                .bytecode_snapshot()?
                .instructions
                .iter()
                .position(|inst| inst.pc >= value),
            (_, ExplorerView::Source) => {
                if value <= 0 {
                    return None;
                }
                let idx = value as usize - 1;
                (idx < self.source_lines.len()).then_some(idx)
            }
            _ => None,
        }
    }

    pub fn search_matches(
        &self,
        view: ExplorerView,
        needle: &str,
        limit: usize,
    ) -> Vec<(usize, String)> {
        let mut matches = Vec::new();
        let needle_lower = needle.to_ascii_lowercase();
        let len = self.list_len(view);
        for idx in 0..len {
            let Some(item) = self.list_item(view, idx) else {
                continue;
            };
            if item.to_ascii_lowercase().contains(&needle_lower) {
                matches.push((idx, item));
                if matches.len() >= limit {
                    break;
                }
            }
        }
        matches
    }

    pub fn document_search_matches(
        &mut self,
        view: ExplorerView,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<(usize, String)>> {
        let text = self.document_text(view)?;
        let needle_lower = needle.to_ascii_lowercase();
        let mut matches = Vec::new();
        for (idx, line) in text.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&needle_lower) {
                matches.push((idx, line.to_string()));
                if matches.len() >= limit {
                    break;
                }
            }
        }
        Ok(matches)
    }

    pub fn execute_command(
        &mut self,
        request: ExplorerCommandRequest,
    ) -> std::result::Result<ExplorerCommandResult, Diagnostic> {
        match request {
            ExplorerCommandRequest::SearchList {
                view,
                needle,
                limit,
            } => Ok(ExplorerCommandResult::SearchHits(
                self.search_matches(view, &needle, limit),
            )),
            ExplorerCommandRequest::SearchDocument {
                view,
                needle,
                limit,
            } => self
                .document_search_matches(view, &needle, limit)
                .map(ExplorerCommandResult::SearchHits)
                .map_err(|err| Diagnostic {
                    code: "document-search-failed",
                    message: err.to_string(),
                }),
            ExplorerCommandRequest::Goto { view, value } => {
                Ok(match self.jump_index(view, value) {
                    Some(index) => ExplorerCommandResult::GotoHit(index),
                    None => ExplorerCommandResult::Miss(Diagnostic {
                        code: "goto-miss",
                        message: format!("goto: value {value} not found"),
                    }),
                })
            }
            ExplorerCommandRequest::ExportGeneratedSource { path } => self
                .export_generated_source(path)
                .map(ExplorerCommandResult::Exported),
        }
    }

    pub fn trace_len(&self) -> usize {
        match &self.mode {
            ExplorerMode::Bytecode => self
                .bytecode_snapshot()
                .map(|snapshot| snapshot.traces.len())
                .unwrap_or(0),
            ExplorerMode::Source => 0,
        }
    }

    pub fn trace_frame(&self, index: usize) -> Option<TraceFrame> {
        match &self.mode {
            ExplorerMode::Bytecode => self.bytecode_snapshot()?.traces.get(index).cloned(),
            ExplorerMode::Source => None,
        }
    }

    pub fn semantic_diff_report(&mut self, other_path: &str) -> Result<SemanticDiffReport> {
        let mut other = ExplorerCore::new(ExploreOptions {
            input_path: other_path.to_string(),
            text_truncate: self.text_truncate,
            page_size: self.page_size,
        })?;
        self.semantic_diff_against(&mut other)
    }

    fn semantic_diff_against(&mut self, other: &mut ExplorerCore) -> Result<SemanticDiffReport> {
        self.require_exact_comparison("left")?;
        other.require_exact_comparison("right")?;

        let mut out = String::new();
        let _ = writeln!(out, "Semantic diff");
        let _ = writeln!(out, "Left: {}", self.comparison_identity());
        let _ = writeln!(out, "Right: {}", other.comparison_identity());

        let both_bytecode = matches!(self.mode, ExplorerMode::Bytecode)
            && matches!(other.mode, ExplorerMode::Bytecode);
        let change_lines = if both_bytecode {
            self.render_bytecode_diff(other, &mut out)?
        } else {
            render_source_diff(
                &self.source_artifact.text,
                &other.source_artifact.text,
                &mut out,
            )
        };
        if !out.ends_with('\n') {
            out.push('\n');
        }
        Ok(SemanticDiffReport {
            text: out,
            change_lines,
        })
    }

    pub fn command_group_count(&self) -> usize {
        match &self.mode {
            ExplorerMode::Bytecode => self
                .bytecode_snapshot()
                .map(|snapshot| snapshot.metadata.command_count)
                .unwrap_or(0),
            ExplorerMode::Source => 0,
        }
    }

    pub fn instruction_group_for_index(&self, instruction_index: usize) -> Option<usize> {
        match &self.mode {
            ExplorerMode::Bytecode => self
                .bytecode_snapshot()?
                .instructions
                .get(instruction_index)
                .and_then(|inst| inst.group),
            ExplorerMode::Source => None,
        }
    }

    pub fn first_instruction_for_group(&self, group: usize) -> Option<usize> {
        match &self.mode {
            ExplorerMode::Bytecode => self
                .bytecode_snapshot()?
                .instructions
                .iter()
                .position(|inst| inst.group == Some(group)),
            ExplorerMode::Source => None,
        }
    }

    pub fn decompile_anchor_for_instruction(
        &mut self,
        instruction_index: usize,
    ) -> Result<Option<usize>> {
        let Some(group) = self.instruction_group_for_index(instruction_index) else {
            return Ok(None);
        };
        self.decompile_anchor_for_group(group)
    }

    pub fn decompile_anchor_for_group(&mut self, group: usize) -> Result<Option<usize>> {
        self.ensure_decompile_anchor_map()?;
        let Some(anchors) = &self.decompile_group_anchors else {
            return Ok(None);
        };
        Ok(anchors
            .get(group)
            .copied()
            .filter(|&value| value != usize::MAX))
    }

    pub fn all_decompile_anchors(&mut self) -> Result<Vec<usize>> {
        self.ensure_decompile_anchor_map()?;
        Ok(self
            .decompile_group_anchors
            .as_ref()
            .cloned()
            .unwrap_or_default())
    }

    pub fn all_decompile_group_line_spans(&mut self) -> Result<Vec<Option<(usize, usize)>>> {
        self.ensure_decompile_anchor_map()?;
        Ok(self
            .decompile_group_line_spans
            .as_ref()
            .cloned()
            .unwrap_or_default())
    }

    pub fn source_command_anchors(&self) -> Vec<SourceCommandAnchor> {
        let Some(snapshot) = self.bytecode_snapshot() else {
            return Vec::new();
        };
        snapshot
            .source_map
            .entries()
            .iter()
            .enumerate()
            .filter_map(|(source_map_index, entry)| {
                if entry.command_path.components.is_empty() {
                    return None;
                }
                let kind = if entry.parent_command_group.is_some() {
                    SourceCommandAnchorKind::Nested
                } else {
                    SourceCommandAnchorKind::Root
                };
                let parent_root_group = entry
                    .parent_command_group
                    .or_else(|| entry.command_path.components.first().copied())
                    .or_else(|| entry.command_group());
                Some(SourceCommandAnchor {
                    kind,
                    source_map_index,
                    command_path: entry.command_path.components.clone(),
                    display_path: display_command_path(&entry.command_path.components),
                    parent_root_group,
                    command_group: entry.command_group(),
                    source_lines: (entry.source_lines.start, entry.source_lines.end),
                    snippet: command_snippet(&entry.text),
                    provenance: self.source_artifact.provenance,
                })
            })
            .collect()
    }

    pub fn export_generated_source(
        &self,
        path: impl AsRef<Path>,
    ) -> std::result::Result<ExportResult, Diagnostic> {
        if self.source_artifact.provenance != SourceProvenance::GeneratedDecompiled {
            let source = self
                .source_artifact
                .source_path
                .as_deref()
                .unwrap_or(self.path.as_str());
            return Err(Diagnostic {
                code: "source-already-exists",
                message: format!(
                    "Generated-source export applies to bytecode. Source is already present at {source}."
                ),
            });
        }
        if !self.decompilation_is_exact_enough_for_export() {
            let reconstruction = self
                .bytecode_snapshot()
                .map(|snapshot| format!("{:?}", snapshot.diagnostics.reconstruction))
                .unwrap_or_else(|| "unknown".to_string());
            return Err(Diagnostic {
                code: "generated-source-not-exact",
                message: format!(
                    "Generated-source export requires an exact reconstruction; this artifact is {reconstruction}. Review Summary > Approximations."
                ),
            });
        }
        let path = path.as_ref();
        let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(Diagnostic {
                    code: "export-target-exists",
                    message: format!(
                        "The export target already exists: {}. Choose another path or remove the existing file.",
                        path.display()
                    ),
                });
            }
            Err(err) => {
                return Err(Diagnostic {
                    code: "export-write-failed",
                    message: format!(
                        "failed to create generated source at {}: {err}",
                        path.display()
                    ),
                });
            }
        };
        if let Err(err) = file.write_all(self.source_artifact.text.as_bytes()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(Diagnostic {
                code: "export-write-failed",
                message: format!(
                    "failed to export generated source to {}: {err}",
                    path.display()
                ),
            });
        }
        Ok(ExportResult {
            path: path.display().to_string(),
            bytes_written: self.source_artifact.text.len(),
            provenance: self.source_artifact.provenance,
        })
    }

    fn decompilation_is_exact_enough_for_export(&self) -> bool {
        self.bytecode_snapshot().is_some_and(|snapshot| {
            snapshot.diagnostics.reconstruction == ReconstructionStatus::Strict
        })
    }

    fn require_exact_comparison(&self, side: &str) -> Result<()> {
        let Some(snapshot) = self.bytecode_snapshot() else {
            return Ok(());
        };
        if snapshot.diagnostics.reconstruction == ReconstructionStatus::Strict {
            return Ok(());
        }
        Err(TcldError::Other(format!(
            "Exact semantic comparison requires exact reconstruction. The {side} artifact {} is {:?}; review it separately.",
            self.path, snapshot.diagnostics.reconstruction
        )))
    }

    fn comparison_identity(&self) -> String {
        format!(
            "{} | artifact {:?} | source {:?} | reconstruction {}",
            self.path,
            self.artifact_kind,
            self.source_artifact.provenance,
            self.bytecode_snapshot()
                .map(|snapshot| format!("{:?}", snapshot.diagnostics.reconstruction))
                .unwrap_or_else(|| "n/a".to_string())
        )
    }

    fn source_document_text(&self) -> String {
        let mut out = String::new();
        match self.source_artifact.provenance {
            SourceProvenance::Original => {
                let _ = writeln!(out, "Tcl source rendering");
                if let Some(path) = &self.source_artifact.source_path {
                    let _ = writeln!(out, "source path: {path}");
                }
            }
            SourceProvenance::GeneratedDecompiled => {
                let _ = writeln!(out, "generated decompiled Tcl source");
                if let Some(path) = &self.source_artifact.generated_from {
                    let _ = writeln!(out, "derived from: {path}");
                }
                let _ = writeln!(out, "provenance: generated from bytecode");
            }
        }
        out.push('\n');
        out.push_str(&self.source_artifact.text);
        out
    }

    fn bytecode_group_signatures(&mut self) -> Result<Vec<GroupSignature>> {
        if let Some(cached) = &self.group_signatures_cache {
            return Ok(cached.clone());
        }
        let Some(snapshot) = self.bytecode_snapshot() else {
            return Ok(Vec::new());
        };
        let group_count = snapshot.metadata.command_count;
        let instructions = snapshot.instructions.clone();
        let decompile_text = self.decompiled_text()?;
        let decompile_lines = decompile_text.lines().collect::<Vec<_>>();
        let line_spans = self.all_decompile_group_line_spans()?;
        let mut opcode_groups = vec![Vec::<String>::new(); group_count];
        let mut content_groups = vec![Vec::<String>::new(); group_count];
        let mut jump_counts = vec![0usize; group_count];
        let mut group_base_pcs = vec![None; group_count];

        for inst in &instructions {
            if let Some(group_idx) = inst.group
                && let Some(base) = group_base_pcs.get_mut(group_idx)
            {
                *base = Some(base.map_or(inst.pc, |current: i32| current.min(inst.pc)));
            }
        }

        for inst in &instructions {
            let Some(group_idx) = inst.group else {
                continue;
            };
            if let Some(bucket) = opcode_groups.get_mut(group_idx) {
                bucket.push(inst.op.clone());
                let base_pc = group_base_pcs[group_idx].unwrap_or(inst.pc);
                content_groups[group_idx].push(normalized_instruction_signature(inst, base_pc));
                if inst.is_jump {
                    jump_counts[group_idx] += 1;
                }
            }
        }

        let mut signatures = Vec::with_capacity(group_count);
        for idx in 0..group_count {
            let opcodes = &opcode_groups[idx];
            let decompile_command = line_spans
                .get(idx)
                .and_then(|span| *span)
                .map(|(start, end)| {
                    let start_line = start.min(decompile_lines.len().saturating_sub(1));
                    let end_line = end.min(decompile_lines.len().saturating_sub(1));
                    (start_line..=end_line)
                        .filter_map(|line_idx| decompile_lines.get(line_idx))
                        .map(|line| line.trim_end())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| "<unmapped>".to_string());
            let decompile_snippet = decompile_command
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| truncate(line, 80, "..."))
                .unwrap_or_else(|| "<empty>".to_string());
            signatures.push(GroupSignature {
                index: idx,
                content_signature: format!(
                    "{}\n{}",
                    content_groups[idx].join("|"),
                    decompile_command
                ),
                opcode_signature: if opcodes.is_empty() {
                    "<none>".to_string()
                } else {
                    opcodes.join("|")
                },
                instruction_count: opcodes.len(),
                jump_count: jump_counts[idx],
                decompile_snippet,
            });
        }
        self.group_signatures_cache = Some(signatures.clone());
        Ok(signatures)
    }

    fn render_bytecode_diff(
        &mut self,
        other: &mut ExplorerCore,
        out: &mut String,
    ) -> Result<Vec<usize>> {
        let left_groups = self.bytecode_group_signatures()?;
        let right_groups = other.bytecode_group_signatures()?;
        let left_keys = left_groups
            .iter()
            .map(|group| group.content_signature.as_str())
            .collect::<Vec<_>>();
        let right_keys = right_groups
            .iter()
            .map(|group| group.content_signature.as_str())
            .collect::<Vec<_>>();
        let diff = TextDiff::from_slices(&left_keys, &right_keys);
        let mut changed = 0usize;
        let mut added = 0usize;
        let mut removed = 0usize;
        let mut details = Vec::new();
        let mut pending_removed = Vec::new();
        let mut pending_added = Vec::new();
        let _ = writeln!(
            out,
            "Mode: bytecode vs bytecode | groups: left {}, right {}",
            left_groups.len(),
            right_groups.len()
        );
        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Delete => {
                    if let Some(index) = change.old_index() {
                        pending_removed.push(index);
                    }
                }
                ChangeTag::Insert => {
                    if let Some(index) = change.new_index() {
                        pending_added.push(index);
                    }
                }
                ChangeTag::Equal => {
                    flush_group_changes(
                        &left_groups,
                        &right_groups,
                        &mut pending_removed,
                        &mut pending_added,
                        &mut changed,
                        &mut added,
                        &mut removed,
                        &mut details,
                    );
                }
            }
        }
        flush_group_changes(
            &left_groups,
            &right_groups,
            &mut pending_removed,
            &mut pending_added,
            &mut changed,
            &mut added,
            &mut removed,
            &mut details,
        );
        let (exception_changed, exception_added, exception_removed, exception_details) =
            diff_exception_signatures(
                self.bytecode_snapshot()
                    .map(|snapshot| snapshot.exceptions.as_slice())
                    .unwrap_or_default(),
                other
                    .bytecode_snapshot()
                    .map(|snapshot| snapshot.exceptions.as_slice())
                    .unwrap_or_default(),
                self.bytecode_snapshot()
                    .and_then(|snapshot| snapshot.instructions.first())
                    .map(|instruction| instruction.pc)
                    .unwrap_or(0),
                other
                    .bytecode_snapshot()
                    .and_then(|snapshot| snapshot.instructions.first())
                    .map(|instruction| instruction.pc)
                    .unwrap_or(0),
            );
        let _ = writeln!(
            out,
            "Summary: {} changed, {} added, {} removed",
            changed, added, removed
        );
        let _ = writeln!(
            out,
            "Exceptions: {} changed, {} added, {} removed",
            exception_changed, exception_added, exception_removed
        );
        details.extend(exception_details);
        let mut change_lines = Vec::new();
        for detail in details.iter().take(MAX_DIFF_DETAILS) {
            change_lines.push(out.lines().count());
            out.push_str(detail);
        }
        if details.len() > MAX_DIFF_DETAILS {
            let _ = writeln!(
                out,
                "... {} more difference(s) not shown",
                details.len() - MAX_DIFF_DETAILS
            );
        }
        Ok(change_lines)
    }

    fn ensure_decompile_anchor_map(&mut self) -> Result<()> {
        if self.decompile_group_anchors.is_some() {
            return Ok(());
        }
        if !matches!(self.mode, ExplorerMode::Bytecode) {
            self.decompile_group_anchors = Some(Vec::new());
            self.decompile_group_line_spans = Some(Vec::new());
            return Ok(());
        }
        let group_count = self.command_group_count();
        if group_count == 0 {
            self.decompile_group_anchors = Some(Vec::new());
            self.decompile_group_line_spans = Some(Vec::new());
            return Ok(());
        }
        let mut anchors = vec![usize::MAX; group_count];
        let mut line_spans: Vec<Option<(usize, usize)>> = vec![None; group_count];
        if let Some(snapshot) = self.bytecode_snapshot() {
            for entry in snapshot.source_map.entries() {
                let Some(group_idx) = entry.command_group() else {
                    continue;
                };
                if group_idx >= group_count {
                    continue;
                }
                let start = entry.source_lines.start;
                let end = entry.source_lines.end.saturating_sub(1);
                anchors[group_idx] = anchors[group_idx].min(start);
                line_spans[group_idx] = Some(match line_spans[group_idx] {
                    Some((existing_start, existing_end)) => {
                        (existing_start.min(start), existing_end.max(end))
                    }
                    None => (start, end),
                });
            }
        }
        for (group_idx, span) in line_spans.iter().enumerate() {
            if span.is_some() && anchors[group_idx] == usize::MAX {
                anchors[group_idx] = span.unwrap().0;
            }
        }
        self.decompile_group_anchors = Some(anchors);
        self.decompile_group_line_spans = Some(line_spans);
        Ok(())
    }

    fn decompiled_text(&mut self) -> Result<String> {
        if let Some(cached) = &self.decompile_cache {
            return Ok(cached.clone());
        }
        let rendered = match &self.mode {
            ExplorerMode::Bytecode => {
                let (commands, text) = {
                    let snapshot = self.bytecode_snapshot().ok_or_else(|| {
                        TcldError::Other("bytecode snapshot unavailable".to_string())
                    })?;
                    (
                        snapshot.decompile.commands.clone(),
                        canonicalize_with_one_trailing_newline(&snapshot.decompile.text),
                    )
                };
                self.decompile_command_spans = Some(commands);
                text
            }
            ExplorerMode::Source => {
                let mut out = String::from("Tcl source rendering\nprovenance: existing source\n\n");
                out.push_str(&self.source_artifact.text);
                out
            }
        };
        self.decompile_cache = Some(rendered.clone());
        Ok(rendered)
    }
}

fn canonicalize_with_one_trailing_newline(source: &str) -> String {
    let mut rendered = canonicalize_tcl(source);
    while rendered.ends_with("\n\n") {
        rendered.pop();
    }
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

fn classify_bytecode_artifact(path: &str) -> ArtifactKind {
    if is_raw_tbc_path(path) {
        ArtifactKind::RawBytecode
    } else {
        ArtifactKind::TclSourceWithBytecode
    }
}

fn is_raw_tbc_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tbc"))
}

fn load_source_from_path_if_text(path: &str) -> std::result::Result<String, String> {
    let bytes = fs::read(path).map_err(|err| format!("failed to read file: {err}"))?;
    if bytes.contains(&0) {
        return Err("file contains NUL bytes and is not Tcl source text".to_string());
    }
    let source =
        String::from_utf8(bytes).map_err(|err| format!("source is not valid UTF-8: {err}"))?;
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    let likely_tcl_ext = matches!(
        ext.as_deref(),
        Some("tcl") | Some("tk") | Some("itcl") | Some("tm")
    );
    let has_no_extension = Path::new(path).extension().is_none();
    if likely_tcl_ext || has_no_extension {
        return Ok(source);
    }
    let tcl_markers = [
        "proc ",
        "set ",
        "if ",
        "foreach ",
        "while ",
        "namespace ",
        "package ",
        "puts ",
        "return ",
        "list ",
        "dict ",
        "expr ",
        "catch ",
        "switch ",
    ];
    if tcl_markers.iter().any(|marker| source.contains(marker)) {
        Ok(source)
    } else {
        Err(format!(
            "valid UTF-8, but extension {:?} and contents do not identify Tcl source; rename the file with a .tcl extension",
            ext.as_deref().unwrap_or("<none>")
        ))
    }
}

fn render_instruction_snapshot(inst: &InstructionSnapshot, text_truncate: Option<usize>) -> String {
    let location = format!("({}) ", inst.pc);
    let operands = inst
        .operands
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let name = if operands.is_empty() {
        inst.op.clone()
    } else {
        format!("{} {operands}", inst.op)
    };
    let mut result = format!("{location:>9}{name:<20}");
    if inst.is_jump {
        result.push_str("# pc ");
        if let Some(target) = inst.jump_target {
            let _ = write!(result, "{target}");
        }
    } else if !inst.comment.is_empty() {
        let mut comment = inst.comment.clone();
        if let Some(limit) = text_truncate {
            comment = truncate(&comment, limit, "...");
        }
        result.push_str("# ");
        result.push_str(&comment);
    }
    result
}

fn render_literal_snapshot(literal: &LiteralSnapshot, text_truncate: Option<usize>) -> String {
    let mut rendered = literal.value.clone();
    rendered = rendered
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    if let Some(limit) = text_truncate {
        rendered = truncate(&rendered, limit, "...");
    }
    rendered
}

fn command_group_range(snapshot: &AnalysisSnapshot, group: usize) -> Option<(i32, i32)> {
    snapshot
        .source_map
        .entries()
        .iter()
        .filter(|entry| entry.command_group() == Some(group))
        .fold(None, |acc, entry| match acc {
            Some((start, end)) => Some((
                start.min(entry.bytecode_range.start),
                end.max(entry.bytecode_range.end),
            )),
            None => Some((entry.bytecode_range.start, entry.bytecode_range.end)),
        })
}

fn display_command_path(components: &[usize]) -> String {
    let Some((root, rest)) = components.split_first() else {
        return String::new();
    };
    let mut out = format!("c{root:03}");
    for nested in rest {
        let _ = write!(out, "/n{nested:03}");
    }
    out
}

fn command_snippet(text: &str) -> String {
    let snippet = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    truncate(snippet, LIST_TEXT_TRUNCATE, "...")
}

fn display_opcode_signature(signature: &str) -> String {
    signature.replace('|', ", ")
}

fn normalized_instruction_signature(inst: &InstructionSnapshot, group_base_pc: i32) -> String {
    let normalize_location = |location: Option<i32>| {
        location
            .map(|pc| pc - group_base_pc)
            .map_or_else(|| "none".to_string(), |relative| relative.to_string())
    };
    let next_locations = inst
        .next_locations
        .iter()
        .map(|pc| (pc - group_base_pc).to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{} operands={:?} argc={} size={} jump={} conditional={} unconditional={} target={} fallthrough={} primary={} secondary={} next=[{}] value={:?}",
        inst.op,
        inst.operands,
        inst.argc,
        inst.size,
        inst.is_jump,
        inst.is_jump_conditional,
        inst.is_jump_unconditional,
        normalize_location(inst.jump_target),
        normalize_location(inst.fallthrough),
        normalize_location(inst.primary_next_location),
        normalize_location(inst.secondary_next_location),
        next_locations,
        inst.comment,
    )
}

fn diff_exception_signatures(
    left: &[ExceptionSnapshot],
    right: &[ExceptionSnapshot],
    left_base_pc: i32,
    right_base_pc: i32,
) -> (usize, usize, usize, Vec<String>) {
    let left = normalized_exception_signatures(left, left_base_pc);
    let right = normalized_exception_signatures(right, right_base_pc);
    let left_keys = left
        .iter()
        .map(|exception| exception.content_signature.as_str())
        .collect::<Vec<_>>();
    let right_keys = right
        .iter()
        .map(|exception| exception.content_signature.as_str())
        .collect::<Vec<_>>();
    let diff = TextDiff::from_slices(&left_keys, &right_keys);
    let mut changed = 0usize;
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut details = Vec::new();
    let mut pending_removed = Vec::new();
    let mut pending_added = Vec::new();

    let flush = |pending_removed: &mut Vec<usize>,
                 pending_added: &mut Vec<usize>,
                 changed: &mut usize,
                 added: &mut usize,
                 removed: &mut usize,
                 details: &mut Vec<String>| {
        let removed_indices = std::mem::take(pending_removed);
        let added_indices = std::mem::take(pending_added);
        let replacements = removed_indices.len().min(added_indices.len());
        for (&left_index, &right_index) in removed_indices
            .iter()
            .zip(added_indices.iter())
            .take(replacements)
        {
            *changed += 1;
            details.push(format!(
                "Change exception e{:03} -> e{:03}: {} -> {}\n",
                left[left_index].index,
                right[right_index].index,
                left[left_index].detail,
                right[right_index].detail
            ));
        }
        for &left_index in removed_indices.iter().skip(replacements) {
            *removed += 1;
            details.push(format!(
                "Removed exception e{:03}: {}\n",
                left[left_index].index, left[left_index].detail
            ));
        }
        for &right_index in added_indices.iter().skip(replacements) {
            *added += 1;
            details.push(format!(
                "Added exception e{:03}: {}\n",
                right[right_index].index, right[right_index].detail
            ));
        }
    };

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                if let Some(index) = change.old_index() {
                    pending_removed.push(index);
                }
            }
            ChangeTag::Insert => {
                if let Some(index) = change.new_index() {
                    pending_added.push(index);
                }
            }
            ChangeTag::Equal => flush(
                &mut pending_removed,
                &mut pending_added,
                &mut changed,
                &mut added,
                &mut removed,
                &mut details,
            ),
        }
    }
    flush(
        &mut pending_removed,
        &mut pending_added,
        &mut changed,
        &mut added,
        &mut removed,
        &mut details,
    );
    (changed, added, removed, details)
}

fn normalized_exception_signatures(
    exceptions: &[ExceptionSnapshot],
    base_pc: i32,
) -> Vec<ExceptionSignature> {
    let normalize = |pc: i32| if pc < 0 { pc } else { pc - base_pc };
    exceptions
        .iter()
        .map(|exception| {
            let content_signature = format!(
                "{}|{}|{}..{}|{}|{}|{}",
                exception.kind,
                exception.nesting_level,
                normalize(exception.code_range[0]),
                normalize(exception.code_range[1]),
                normalize(exception.break_offset),
                normalize(exception.continue_offset),
                normalize(exception.catch_offset),
            );
            ExceptionSignature {
                index: exception.idx,
                detail: format!(
                    "{} nest={} code={}..{} break={} continue={} catch={}",
                    exception.kind,
                    exception.nesting_level,
                    normalize(exception.code_range[0]),
                    normalize(exception.code_range[1]),
                    normalize(exception.break_offset),
                    normalize(exception.continue_offset),
                    normalize(exception.catch_offset),
                ),
                content_signature,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn flush_group_changes(
    left_groups: &[GroupSignature],
    right_groups: &[GroupSignature],
    pending_removed: &mut Vec<usize>,
    pending_added: &mut Vec<usize>,
    changed: &mut usize,
    added: &mut usize,
    removed: &mut usize,
    details: &mut Vec<String>,
) {
    let removed_indices = std::mem::take(pending_removed);
    let added_indices = std::mem::take(pending_added);
    let replacements = removed_indices.len().min(added_indices.len());

    for (&left_index, &right_index) in removed_indices
        .iter()
        .zip(added_indices.iter())
        .take(replacements)
    {
        let left = &left_groups[left_index];
        let right = &right_groups[right_index];
        *changed += 1;
        let mut detail = String::new();
        let _ = writeln!(
            detail,
            "Change c{:03} -> c{:03}: {} ops -> {} ops | jumps {} -> {}",
            left.index,
            right.index,
            left.instruction_count,
            right.instruction_count,
            left.jump_count,
            right.jump_count
        );
        let _ = writeln!(detail, "  Left source: {}", left.decompile_snippet);
        let _ = writeln!(detail, "  Right source: {}", right.decompile_snippet);
        let _ = writeln!(
            detail,
            "  Left ops: {}",
            display_opcode_signature(&left.opcode_signature)
        );
        let _ = writeln!(
            detail,
            "  Right ops: {}",
            display_opcode_signature(&right.opcode_signature)
        );
        details.push(detail);
    }

    for &left_index in removed_indices.iter().skip(replacements) {
        let left = &left_groups[left_index];
        *removed += 1;
        let mut detail = String::new();
        let _ = writeln!(
            detail,
            "Removed c{:03}: {} ops",
            left.index, left.instruction_count
        );
        let _ = writeln!(detail, "  Source: {}", left.decompile_snippet);
        let _ = writeln!(
            detail,
            "  Ops: {}",
            display_opcode_signature(&left.opcode_signature)
        );
        details.push(detail);
    }

    for &right_index in added_indices.iter().skip(replacements) {
        let right = &right_groups[right_index];
        *added += 1;
        let mut detail = String::new();
        let _ = writeln!(
            detail,
            "Added c{:03}: {} ops",
            right.index, right.instruction_count
        );
        let _ = writeln!(detail, "  Source: {}", right.decompile_snippet);
        let _ = writeln!(
            detail,
            "  Ops: {}",
            display_opcode_signature(&right.opcode_signature)
        );
        details.push(detail);
    }
}

fn render_source_diff(left: &str, right: &str, out: &mut String) -> Vec<usize> {
    let diff = TextDiff::from_lines(left, right);
    let mut changed = 0usize;
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut pending_removed = Vec::new();
    let mut pending_added = Vec::new();
    let mut details = Vec::new();
    let _ = writeln!(out, "Mode: source/mixed | line-level source diff");
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => pending_removed.push((
                change.old_index().expect("delete has old index"),
                change.value().trim_end_matches(['\r', '\n']).to_string(),
            )),
            ChangeTag::Insert => pending_added.push((
                change.new_index().expect("insert has new index"),
                change.value().trim_end_matches(['\r', '\n']).to_string(),
            )),
            ChangeTag::Equal => flush_source_changes(
                &mut pending_removed,
                &mut pending_added,
                &mut changed,
                &mut added,
                &mut removed,
                &mut details,
            ),
        }
    }
    flush_source_changes(
        &mut pending_removed,
        &mut pending_added,
        &mut changed,
        &mut added,
        &mut removed,
        &mut details,
    );
    let _ = writeln!(
        out,
        "Summary: {} changed, {} added, {} removed",
        changed, added, removed
    );
    let mut change_lines = Vec::new();
    for detail in details.iter().take(MAX_DIFF_DETAILS) {
        change_lines.push(out.lines().count());
        out.push_str(detail);
    }
    if details.len() > MAX_DIFF_DETAILS {
        let _ = writeln!(
            out,
            "... {} more difference(s) not shown",
            details.len() - MAX_DIFF_DETAILS
        );
    }
    change_lines
}

#[allow(clippy::too_many_arguments)]
fn flush_source_changes(
    pending_removed: &mut Vec<(usize, String)>,
    pending_added: &mut Vec<(usize, String)>,
    changed: &mut usize,
    added: &mut usize,
    removed: &mut usize,
    details: &mut Vec<String>,
) {
    let removed_lines = std::mem::take(pending_removed);
    let added_lines = std::mem::take(pending_added);
    let replacements = removed_lines.len().min(added_lines.len());

    for ((old_index, old_line), (new_index, new_line)) in removed_lines
        .iter()
        .zip(added_lines.iter())
        .take(replacements)
    {
        *changed += 1;
        details.push(format!(
            "Change line {} -> {}: '{}' -> '{}'\n",
            old_index + 1,
            new_index + 1,
            truncate(old_line, 120, "..."),
            truncate(new_line, 120, "...")
        ));
    }
    for (old_index, old_line) in removed_lines.iter().skip(replacements) {
        *removed += 1;
        details.push(format!(
            "Removed line {}: '{}'\n",
            old_index + 1,
            truncate(old_line, 120, "...")
        ));
    }
    for (new_index, new_line) in added_lines.iter().skip(replacements) {
        *added += 1;
        details.push(format!(
            "Added line {}: '{}'\n",
            new_index + 1,
            truncate(new_line, 120, "...")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(path: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/tcld/tests/fixtures")
            .join(path)
            .display()
            .to_string()
    }

    fn options(path: &str) -> ExploreOptions {
        ExploreOptions {
            input_path: fixture(path),
            text_truncate: Some(30),
            page_size: 20,
        }
    }

    #[test]
    fn contract_model_covers_artifacts_views_capabilities_and_payloads() {
        let mut bytecode = ExplorerCore::new(options("expr.tbc")).expect("bytecode loads");
        assert_eq!(bytecode.artifact_kind(), ArtifactKind::RawBytecode);
        assert!(bytecode.views().contains(&ExplorerView::Instructions));
        assert!(bytecode.views().contains(&ExplorerView::Source));
        assert!(
            bytecode
                .capabilities()
                .contains(&Capability::GeneratedSourceExport)
        );
        assert!(
            bytecode
                .document_text(ExplorerView::Summary)
                .unwrap()
                .contains("generated source")
        );
        let payload = bytecode
            .stable_render_payload()
            .expect("stable render payload");
        assert_eq!(payload.artifact_kind, ArtifactKind::RawBytecode);
        assert!(payload.capability_chips.contains(&"bytecode".to_string()));
        assert!(
            payload
                .capability_chips
                .contains(&"generated-export".to_string())
        );
        assert_eq!(
            payload.available_views,
            vec![
                ExplorerView::Instructions,
                ExplorerView::Literals,
                ExplorerView::Exceptions,
                ExplorerView::Source,
                ExplorerView::Decompile,
                ExplorerView::Summary,
            ]
        );
        assert!(!payload.primary_list.is_empty());
        assert!(payload.document_text.contains("expr") || payload.document_text.contains("set"));
        assert!(payload.inspector_text.contains("instruction index"));
        assert!(!payload.command_group_map.is_empty());
        assert!(payload.export_status.contains("Export ready"));

        let mut source = ExplorerCore::new(options("expr.tcl")).expect("source loads");
        assert_eq!(source.artifact_kind(), ArtifactKind::PlainTclSource);
        assert_eq!(
            source.views(),
            &[
                ExplorerView::Source,
                ExplorerView::Decompile,
                ExplorerView::Summary
            ]
        );
        assert!(
            !source
                .capabilities()
                .contains(&Capability::GeneratedSourceExport)
        );
        assert!(
            source
                .document_text(ExplorerView::Decompile)
                .unwrap()
                .contains("source rendering")
        );
        let payload = source
            .stable_render_payload()
            .expect("source stable render payload");
        assert_eq!(payload.artifact_kind, ArtifactKind::PlainTclSource);
        assert_eq!(payload.default_view, ExplorerView::Source);
        assert!(payload.capability_chips.contains(&"source".to_string()));
        assert!(!payload.capability_chips.contains(&"bytecode".to_string()));
        assert!(
            payload
                .diagnostics
                .iter()
                .any(|d| d.code == "source-only-artifact")
        );
        assert!(payload.export_status.contains("unavailable"));
    }

    #[test]
    fn generated_source_participates_in_source_views() {
        let mut core = ExplorerCore::new(options("expr.tbc")).expect("bytecode loads");
        let artifact = core.source_artifact().expect("generated source artifact");
        assert_eq!(artifact.provenance, SourceProvenance::GeneratedDecompiled);
        assert!(
            artifact
                .generated_from
                .as_deref()
                .unwrap()
                .ends_with("expr.tbc")
        );
        assert!(core.list_len(ExplorerView::Source) > 0);
        assert!(
            !core
                .search_matches(ExplorerView::Source, "expr", 10)
                .is_empty()
        );
        match core
            .execute_command(ExplorerCommandRequest::SearchList {
                view: ExplorerView::Source,
                needle: "expr".to_string(),
                limit: 10,
            })
            .expect("shared search command")
        {
            ExplorerCommandResult::SearchHits(hits) => assert!(!hits.is_empty()),
            other => panic!("unexpected shared search result: {other:?}"),
        }
        match core
            .execute_command(ExplorerCommandRequest::SearchDocument {
                view: ExplorerView::Source,
                needle: "expr".to_string(),
                limit: 10,
            })
            .expect("shared document search command")
        {
            ExplorerCommandResult::SearchHits(hits) => assert!(!hits.is_empty()),
            other => panic!("unexpected shared document search result: {other:?}"),
        }
        assert_eq!(core.jump_index(ExplorerView::Source, 1), Some(0));
        assert_eq!(
            core.execute_command(ExplorerCommandRequest::Goto {
                view: ExplorerView::Source,
                value: 1,
            })
            .expect("shared goto command"),
            ExplorerCommandResult::GotoHit(0)
        );
        let source_doc = core
            .document_text(ExplorerView::Source)
            .expect("source doc");
        assert!(source_doc.contains("generated decompiled Tcl source"));
        assert!(source_doc.contains("derived from"));
    }

    #[test]
    fn semantic_diff_report_uses_reader_facing_summary_copy() {
        let mut core = ExplorerCore::new(options("expr.tbc")).expect("bytecode loads");
        let report = core
            .semantic_diff_report(&fixture("expr2.tbc"))
            .expect("semantic diff report")
            .text;

        assert!(report.contains("Semantic diff"), "{report}");
        assert!(
            report.contains("Mode: bytecode vs bytecode | groups: left 4, right 4"),
            "{report}"
        );
        assert!(
            report.contains("Summary: 2 changed, 0 added, 0 removed"),
            "{report}"
        );
        assert!(
            report.contains("Exceptions: 1 changed, 0 added, 0 removed"),
            "{report}"
        );
        assert!(
            report.contains("Change c000 -> c000: 20 ops -> 16 ops"),
            "{report}"
        );
        assert!(
            report.contains("Left source: set result [expr int(10 * [expr sqrt(16)])]"),
            "{report}"
        );
        assert!(report.contains("Right source: if 1 {"), "{report}");
        assert!(
            report.contains("Left ops:") && report.contains("Right ops:"),
            "{report}"
        );
        assert!(report.contains("Left ops: push1, push1, push1"), "{report}");
        assert!(report.contains("Right ops: beginCatch4, push1"), "{report}");
        let summary_offset = report.find("Summary:").expect("summary line");
        let change_offset = report.find("Change c000").expect("change line");
        assert!(
            summary_offset < change_offset,
            "summary should lead the detailed change list\n{report}"
        );
        for stale in [
            "semantic diff",
            "mode: bytecode-vs-bytecode",
            "groups(left=",
            "- changed c000: ops",
            "left :",
            "right: if",
            "Left ops: push1|",
            "Right ops: beginCatch4|",
            "summary: changed=",
            "risk_score",
        ] {
            assert!(
                !report.contains(stale),
                "semantic diff report should not expose internal syntax {stale:?}\n{report}"
            );
        }

        let mut source = ExplorerCore::new(options("expr.tcl")).expect("source loads");
        let source_report = source
            .semantic_diff_report(&fixture("expr.tbc"))
            .expect("source diff report")
            .text;
        assert!(
            source_report.contains("Mode: source/mixed | line-level source diff"),
            "{source_report}"
        );
        assert!(
            source_report.contains("Summary: "),
            "source diff report should use the same reader-facing summary label\n{source_report}"
        );
        assert!(
            !source_report.contains("source/mixed (line-level diff over source text)")
                && !source_report.contains("risk_score"),
            "source diff report should avoid internal mode/score syntax\n{source_report}"
        );
    }

    #[test]
    fn source_diff_aligns_insertions_without_cascading_false_changes() {
        let dir = tempfile::tempdir().expect("temporary source diff directory");
        let left = dir.path().join("left.tcl");
        let right = dir.path().join("right.tcl");
        fs::write(&left, "set a 1\nset b 2\nset c 3\n").unwrap();
        fs::write(&right, "set a 1\nputs inserted\nset b 2\nset c 3\n").unwrap();

        let mut core = ExplorerCore::new(ExploreOptions {
            input_path: left.display().to_string(),
            text_truncate: None,
            page_size: 20,
        })
        .unwrap();
        let report = core
            .semantic_diff_report(&right.display().to_string())
            .unwrap()
            .text;

        assert!(
            report.contains("Summary: 0 changed, 1 added, 0 removed"),
            "{report}"
        );
        assert!(report.contains("Added line 2: 'puts inserted'"), "{report}");
        assert!(!report.contains("set b 2' ->"), "{report}");
    }

    #[test]
    fn source_diff_reports_complete_totals_when_details_are_bounded() {
        let dir = tempfile::tempdir().expect("temporary source diff directory");
        let left = dir.path().join("left.tcl");
        let right = dir.path().join("right.tcl");
        let left_text = (0..205)
            .map(|index| format!("set value {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let right_text = (0..205)
            .map(|index| format!("set value changed-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&left, left_text).unwrap();
        fs::write(&right, right_text).unwrap();

        let mut core = ExplorerCore::new(ExploreOptions {
            input_path: left.display().to_string(),
            text_truncate: None,
            page_size: 20,
        })
        .unwrap();
        let report = core
            .semantic_diff_report(&right.display().to_string())
            .unwrap()
            .text;

        assert!(
            report.contains("Summary: 205 changed, 0 added, 0 removed"),
            "{report}"
        );
        assert!(
            report.contains("... 5 more difference(s) not shown"),
            "{report}"
        );
    }

    #[test]
    fn generated_source_export_refuses_to_overwrite() {
        let core = ExplorerCore::new(options("expr.tbc")).expect("bytecode loads");
        let dir = tempfile::tempdir().expect("temporary export directory");
        let target = dir.path().join("existing.tcl");
        fs::write(&target, "keep me\n").unwrap();

        let diagnostic = core
            .export_generated_source(&target)
            .expect_err("existing export target must be preserved");

        assert_eq!(diagnostic.code, "export-target-exists");
        assert!(diagnostic.message.contains("already exists"));
        assert_eq!(fs::read_to_string(target).unwrap(), "keep me\n");
    }

    #[test]
    fn approximated_generated_source_is_diagnosed_and_not_exportable() {
        let mut snapshot =
            AnalysisSnapshot::from_input(LoadInput::path(fixture("expr.tbc"))).unwrap();
        snapshot.diagnostics.reconstruction = ReconstructionStatus::Approximation;
        let core = ExplorerCore::from_snapshot(options("expr.tbc"), snapshot);

        assert!(
            core.summary_text()
                .contains("decompile reconstruction: Approximation")
        );
        assert!(
            core.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "decompile-reconstruction")
        );
        assert!(core.export_status_label().contains("unavailable"));
        let dir = tempfile::tempdir().unwrap();
        let diagnostic = core
            .export_generated_source(dir.path().join("approximation.tcl"))
            .expect_err("approximated source must not be exported");
        assert_eq!(diagnostic.code, "generated-source-not-exact");
    }

    #[test]
    fn non_strict_reconstruction_blocks_exact_export_and_comparison() {
        let base = AnalysisSnapshot::from_input(LoadInput::path(fixture("expr.tbc"))).unwrap();
        for reconstruction in [
            ReconstructionStatus::Approximation,
            ReconstructionStatus::Failed,
        ] {
            let mut snapshot = base.clone();
            snapshot.diagnostics.reconstruction = reconstruction;
            let mut left = ExplorerCore::from_snapshot(options("left.tbc"), snapshot);
            let mut right = ExplorerCore::from_snapshot(options("right.tbc"), base.clone());
            let directory = tempfile::tempdir().unwrap();

            let diagnostic = left
                .export_generated_source(directory.path().join("generated.tcl"))
                .expect_err("non-High export must be rejected");
            assert_eq!(diagnostic.code, "generated-source-not-exact");
            let error = left
                .semantic_diff_against(&mut right)
                .expect_err("non-strict comparison must be rejected")
                .to_string();
            assert!(error.contains(&format!("{reconstruction:?}")), "{error}");
        }

        let strict = ExplorerCore::from_snapshot(options("strict.tbc"), base);
        let directory = tempfile::tempdir().unwrap();
        strict
            .export_generated_source(directory.path().join("generated.tcl"))
            .expect("strictly reconstructed source remains exportable");
    }

    #[test]
    fn bytecode_diff_detects_operand_and_control_flow_mutations() {
        let base = AnalysisSnapshot::from_input(LoadInput::path(fixture("expr2.tbc"))).unwrap();

        let mut operand_snapshot = base.clone();
        let operand = operand_snapshot
            .instructions
            .iter_mut()
            .find(|instruction| !instruction.operands.is_empty())
            .expect("fixture has an instruction operand");
        operand.operands[0] = operand.operands[0].saturating_add(1);
        let mut left = ExplorerCore::from_snapshot(options("left.tbc"), base.clone());
        let mut right = ExplorerCore::from_snapshot(options("right.tbc"), operand_snapshot);
        let operand_report = left.semantic_diff_against(&mut right).unwrap();
        assert!(
            operand_report.text.contains("Summary: 1 changed"),
            "{}",
            operand_report.text
        );
        assert!(!operand_report.change_lines.is_empty());

        let mut jump_snapshot = base.clone();
        let jump = jump_snapshot
            .instructions
            .iter_mut()
            .find(|instruction| instruction.jump_target.is_some())
            .expect("fixture has a jump target");
        jump.jump_target = jump.jump_target.map(|target| target.saturating_add(1));
        let mut left = ExplorerCore::from_snapshot(options("left.tbc"), base.clone());
        let mut right = ExplorerCore::from_snapshot(options("right.tbc"), jump_snapshot);
        let jump_report = left.semantic_diff_against(&mut right).unwrap();
        assert!(
            jump_report.text.contains("Summary: 1 changed"),
            "{}",
            jump_report.text
        );

        let mut multiline_snapshot = base.clone();
        multiline_snapshot.decompile.text = multiline_snapshot.decompile.text.replacen(
            "    set result PASS",
            "    set result MUTATED",
            1,
        );
        assert_ne!(multiline_snapshot.decompile.text, base.decompile.text);
        let mut left = ExplorerCore::from_snapshot(options("left.tbc"), base);
        let mut right = ExplorerCore::from_snapshot(options("right.tbc"), multiline_snapshot);
        let multiline_report = left.semantic_diff_against(&mut right).unwrap();
        assert!(
            multiline_report.text.contains("Summary: 1 changed"),
            "{}",
            multiline_report.text
        );
    }

    #[test]
    fn bytecode_diff_detects_exception_mutation_and_ignores_pure_pc_relocation() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/public/public_complex_fixture.tbc")
            .display()
            .to_string();
        let mut base = AnalysisSnapshot::from_input(LoadInput::path(fixture)).unwrap();
        if base.exceptions.is_empty() {
            base.exceptions.push(ExceptionSnapshot {
                idx: 0,
                kind: "Catch".to_string(),
                nesting_level: 0,
                code_range: [10, 20],
                break_offset: -1,
                continue_offset: -1,
                catch_offset: 20,
            });
        }

        let mut exception_snapshot = base.clone();
        let exception = exception_snapshot
            .exceptions
            .first_mut()
            .expect("fixture fixture has an exception range");
        exception.code_range[1] = exception.code_range[1].saturating_add(1);
        let mut left = ExplorerCore::from_snapshot(options("left.tbc"), base.clone());
        let mut right = ExplorerCore::from_snapshot(options("right.tbc"), exception_snapshot);
        let exception_report = left.semantic_diff_against(&mut right).unwrap();
        assert!(
            exception_report
                .text
                .contains("Exceptions: 1 changed, 0 added, 0 removed"),
            "{}",
            exception_report.text
        );

        let mut relocated = base.clone();
        for instruction in &mut relocated.instructions {
            instruction.pc += 100;
            instruction.jump_target = instruction.jump_target.map(|pc| pc + 100);
            instruction.fallthrough = instruction.fallthrough.map(|pc| pc + 100);
            instruction.primary_next_location =
                instruction.primary_next_location.map(|pc| pc + 100);
            instruction.secondary_next_location =
                instruction.secondary_next_location.map(|pc| pc + 100);
            for location in &mut instruction.next_locations {
                *location += 100;
            }
        }
        for exception in &mut relocated.exceptions {
            for location in &mut exception.code_range {
                *location += 100;
            }
            for location in [
                &mut exception.break_offset,
                &mut exception.continue_offset,
                &mut exception.catch_offset,
            ] {
                if *location >= 0 {
                    *location += 100;
                }
            }
        }
        let mut left = ExplorerCore::from_snapshot(options("left.tbc"), base);
        let mut right = ExplorerCore::from_snapshot(options("right.tbc"), relocated);
        let relocation_report = left.semantic_diff_against(&mut right).unwrap();
        assert!(
            relocation_report
                .text
                .contains("Summary: 0 changed, 0 added, 0 removed"),
            "{}",
            relocation_report.text
        );
        assert!(
            relocation_report
                .text
                .contains("Exceptions: 0 changed, 0 added, 0 removed"),
            "{}",
            relocation_report.text
        );
    }

    #[test]
    fn empty_tcl_source_loads_and_invalid_input_keeps_both_causes() {
        let dir = tempfile::tempdir().expect("temporary source directory");
        let empty = dir.path().join("empty.tcl");
        fs::write(&empty, "").unwrap();
        let source = ExplorerCore::new(ExploreOptions {
            input_path: empty.display().to_string(),
            text_truncate: None,
            page_size: 20,
        })
        .expect("empty Tcl source is valid");
        assert_eq!(source.artifact_kind(), ArtifactKind::PlainTclSource);

        let invalid = dir.path().join("not-tcl.txt");
        fs::write(&invalid, "this is not bytecode or Tcl").unwrap();
        let error = match ExplorerCore::new(ExploreOptions {
            input_path: invalid.display().to_string(),
            text_truncate: None,
            page_size: 20,
        }) {
            Ok(_) => panic!("invalid input should preserve loader diagnostics"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("bytecode:"), "{error}");
        assert!(error.contains("source:"), "{error}");
        assert!(
            error.contains("rename the file with a .tcl extension"),
            "{error}"
        );
    }

    #[test]
    fn bcproc_nested_source_commands_are_addressable_in_shared_explorer_contract() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/public/public_complex_fixture.tbc")
            .display()
            .to_string();
        let mut core = ExplorerCore::new(ExploreOptions {
            input_path: path,
            text_truncate: Some(80),
            page_size: 20,
        })
        .expect("fixture bytecode loads");
        let snapshot = core.snapshot().expect("bytecode snapshot");
        let root_count = snapshot.metadata.command_count;

        assert_eq!(core.command_group_count(), root_count);
        assert!(root_count > 0);

        let anchors = core.source_command_anchors();
        let startlog_root = anchors
            .iter()
            .find(|anchor| {
                anchor.kind == SourceCommandAnchorKind::Root
                    && anchor.snippet.starts_with("proc fixture::startlog ")
            })
            .expect("fixture::startlog root source command anchor");
        let root_group = startlog_root
            .parent_root_group
            .expect("root anchor reports its root command group");
        assert!(root_group < root_count);

        for snippet in ["variable baseDir", "global fixture_startlog", "set err "] {
            let anchor = anchors
                .iter()
                .find(|anchor| {
                    anchor.kind == SourceCommandAnchorKind::Nested
                        && anchor.parent_root_group == Some(root_group)
                        && anchor
                            .display_path
                            .starts_with(&format!("c{root_group:03}/n"))
                        && anchor.snippet.trim_start().starts_with(snippet)
                })
                .unwrap_or_else(|| panic!("missing nested anchor for {snippet:?}"));
            assert_eq!(anchor.command_path.len(), 2);
            assert_eq!(anchor.command_path[0], root_group);
            assert!(anchor.source_lines.0 < anchor.source_lines.1);
            assert_eq!(anchor.provenance, SourceProvenance::GeneratedDecompiled);
        }

        let decompile_hits = core
            .document_search_matches(ExplorerView::Decompile, "variable baseDir", 10)
            .expect("decompile search");
        assert!(
            decompile_hits
                .iter()
                .any(|(_, line)| line.contains("variable baseDir"))
        );
    }

    #[test]
    fn export_generated_decompiled_source() {
        let temp =
            std::env::temp_dir().join(format!("tcld-generated-export-{}.tcl", std::process::id()));
        let _ = std::fs::remove_file(&temp);

        let mut bytecode = ExplorerCore::new(options("expr.tbc")).expect("bytecode loads");
        let expected = bytecode.source_artifact().unwrap().text;
        let result = bytecode
            .execute_command(ExplorerCommandRequest::ExportGeneratedSource {
                path: temp.display().to_string(),
            })
            .expect("generated export succeeds");
        let ExplorerCommandResult::Exported(result) = result else {
            panic!("unexpected export command result: {result:?}");
        };
        assert_eq!(result.path, temp.display().to_string());
        assert_eq!(std::fs::read_to_string(&temp).unwrap(), expected);
        assert!(expected.ends_with('\n'));

        let plain = ExplorerCore::new(options("expr.tcl")).expect("plain source loads");
        let err = plain
            .export_generated_source(&temp)
            .expect_err("plain source export rejected");
        assert_eq!(err.code, "source-already-exists");
        assert!(err.message.contains("expr.tcl"));

        let _ = std::fs::remove_file(&temp);
    }

    #[test]
    fn source_goto_reports_out_of_range_miss() {
        let core = ExplorerCore::new(options("expr.tcl")).expect("source loads");
        assert_eq!(core.jump_index(ExplorerView::Source, 1), Some(0));
        assert_eq!(core.jump_index(ExplorerView::Source, 0), None);
        assert_eq!(core.jump_index(ExplorerView::Source, 999_999), None);
    }
}
