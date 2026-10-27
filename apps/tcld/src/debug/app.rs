//! `tcld debug` driver loop.
//!
//! Owns one backend-neutral debug session, the local source/disassembly cache,
//! the rolling event log, and the [`DebugUiState`]. Rendering is
//! delegated to [`tcld_tui::debug::render`] so the view itself stays
//! reusable and themed.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use serde::{Deserialize, Serialize};
use tcld_tui::Theme;
use tcld_tui::debug::{
    AppState, BindingContext, CoverageRow, DebugAction, DebugSnapshot, DebugUiState, EventLine,
    EventSeverity, FocusedProc, LocalRow, Pane, PickerOverlay, ProcBreakpoint, PromptInput,
    PromptStep, SourceCommand, SourceLine, SourceOrigin, SourceSearch, StackFrame, ValuePopup,
    WatchEntry, prompt_step, render_in, resolve_binding,
};
use tcld_tui::terminal::{Event, Frame, Rect, RuntimeAction, RuntimeEvent, TerminalApplication};

use super::backend::{DebugBackend, WireSessionBackend};
use super::config::NubPaths;
use super::contract::{
    BackendCapabilities, BackendOperation, DebugBreakpoint, DebugBreakpointId,
    DebugBreakpointResolution, DebugBreakpointSpec, DebugDiagnosticSeverity, DebugEvent,
    DebugLifecycleState, DebugLocation, DebugOutputStream, DebugPauseReason, DebugProcedure,
    DebugSourceUnitIdentity, DebugStepKind, SourceCommandSpan,
};
use super::planning::{PreparedDebugTarget, TargetProvenance};

const EVENT_LOG_CAPACITY: usize = 1024;
const APP_TITLE: &str = "tcld debug";
const PERSIST_FILE: &str = ".tcld-debug.tcl";
const TRANSCRIPT_BASENAME: &str = "tcld-debug-transcript";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedDebugState {
    version: u32,
    breakpoints: String,
    watches: Vec<PersistedWatch>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedWatch {
    name: String,
    expression: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    Command,
    GotoProc,
    AddWatch,
    AddWatchpoint,
    EvalExpr,
    AddBreakpoint,
    EditCondition,
    SetIgnore,
    EventFilter,
    ProcPicker,
    PreInstrument,
    SearchSource,
}

struct ActivePrompt {
    kind: PromptKind,
    input: PromptInput,
    /// Breakpoint id the prompt operates on (for EditCondition / SetIgnore).
    bp_target: Option<String>,
    /// Pre-populated body text for PreInstrument.
    body_buffer: Option<String>,
    /// Cursor into [`DebugApp::command_history`] for `:` prompts.
    /// `None` means "above the most recent entry — first ↑ pulls the
    /// last command".
    history_pos: Option<usize>,
}

/// Source text under the cursor — either the user-script file or a
/// `proc_body` fetch keyed by qname.
struct SourceCache {
    user_script: Vec<SourceLine>,
    /// Origin label used when the source pane is showing the
    /// user-script (i.e. not a fetched proc body).
    user_script_origin: SourceOrigin,
    /// Most-recent proc body keyed by qname.
    proc_bodies: HashMap<String, Vec<SourceLine>>,
    /// Canonical runtime-owned source units keyed by the backend block id.
    /// Unlike `user_script`/`proc_bodies`, this includes sourced files and
    /// eval/anonymous units and carries generation/provenance.
    source_units: HashMap<String, CachedSourceUnit>,
    showing_block_id: Option<String>,
    /// Which qname the source pane is currently showing (None ⇒ user
    /// script).
    showing_qname: Option<String>,
    origin: SourceOrigin,
    secondary: Vec<SourceLine>,
    secondary_highlight_line: Option<u32>,
    pc_to_line: HashMap<u64, u32>,
    line_to_code: HashMap<u32, super::contract::DebugCodeLocation>,
    /// Where each proc's body sits among the files the debugger has text
    /// for, by qname. `None` records that no loaded file defines the proc
    /// with the body the runtime reports (it was generated, redefined, or
    /// its file has not been loaded).
    proc_placements: HashMap<String, Option<ProcPlacement>>,
    /// The proc whose paused body is being shown inside its file, and where.
    /// Showing a proc in its file keeps real line numbers and surrounding
    /// context instead of a detached, renumbered body.
    mapped_proc: Option<(String, ProcPlacement)>,
}

/// Where a proc's body sits in a file the debugger has text for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcPlacement {
    /// The file's runtime source unit. `None` is the main script read from
    /// disk before the runtime registered it.
    file_block: Option<String>,
    path: PathBuf,
    /// Body line N is file line N + offset.
    offset: u32,
}

struct CachedSourceUnit {
    registration: super::contract::RegisteredSource,
    lines: Vec<SourceLine>,
}

/// Locals last seen per frame, keyed by the frame's depth from the outermost
/// frame and its proc.
type LocalsByFrame = HashMap<(i64, Option<String>), Vec<(String, String)>>;

pub struct DebugApp {
    user_script: PathBuf,
    display_path: PathBuf,
    /// Lifetime guard for an OS-allocated temp dir created when the
    /// input was a `.tbc` we had to decompile. `None` for plain
    /// user-script inputs. Drop unlinks the directory + every file
    /// in it so we never litter `/tmp`.
    _temp_dir: Option<tempfile::TempDir>,
    backend: Option<Box<dyn DebugBackend>>,
    capabilities: BackendCapabilities,
    backend_label: String,

    cache: SourceCache,

    app_state: AppState,
    focused_proc: Option<FocusedProc>,
    paused_focus: Option<FocusedProc>,
    inspected_frame: i64,
    stack: Vec<StackFrame>,
    locals: Vec<LocalRow>,
    /// Snapshot of the previous pause's locals per frame — used to compute
    /// the `changed` flag for the current pass.
    locals_prev: LocalsByFrame,
    breakpoints: Vec<ProcBreakpoint>,
    watches: Vec<WatchEntry>,
    coverage: Vec<CoverageRow>,
    coverage_enabled: bool,
    events: VecDeque<EventLine>,
    event_filter: String,
    last_status_full: String,

    proc_picker: Vec<DebugProcedure>,
    proc_picker_filtered: Vec<String>,
    proc_picker_selected: Option<usize>,

    /// Source-search state. `query` is updated incrementally as the
    /// user types in the search prompt; `match_lines` is recomputed
    /// each time `query` changes.
    search_query: String,
    search_match_lines: Vec<usize>,
    search_cursor: Option<usize>,

    /// History of `:` commands; up/down navigate.
    command_history: VecDeque<String>,
    /// `.`-repeat target: the most recently run actionable command
    /// (a step, breakpoint toggle, etc.) that's safe to replay.
    last_action: Option<RepeatableAction>,

    ui: DebugUiState,
    prompt: Option<ActivePrompt>,
    error_banner: Option<String>,
    status: String,
    transcript_path: Option<PathBuf>,
    quitting: bool,

    theme: Theme,
    theme_name: String,
    /// Opcode table of the debugged bytecode, from the backend's disassembly.
    instruction_set: Option<String>,
}

/// Actions safe to replay via `.` (no destructive prompts, no
/// expressions that mutated state in a non-idempotent way).
#[derive(Debug, Clone)]
enum RepeatableAction {
    Continue,
    StepInto,
    StepOver,
    StepOut,
    Pause,
    ToggleBp,
    Refresh,
    Eval(String),
    DbgCommand(String),
}

impl DebugApp {
    pub fn new(target: PreparedDebugTarget, paths: NubPaths, theme: Theme) -> io::Result<Self> {
        let description = format!(
            "host Tcl {} ({}) via {}; debugger nub {}",
            paths.tcl_patchlevel,
            paths
                .tcl_abi
                .map(|abi| abi.to_string())
                .unwrap_or_else(|| "unprobed ABI".to_string()),
            paths.tclsh.display(),
            paths.dylib.display()
        );
        let backend = WireSessionBackend::connect(paths)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Self::with_backend(target, Box::new(backend), description, theme)
    }

    pub fn with_backend(
        target: PreparedDebugTarget,
        backend: Box<dyn DebugBackend>,
        backend_label: impl Into<String>,
        theme: Theme,
    ) -> io::Result<Self> {
        let compiled_source = target.compiled_image.is_some();
        let capabilities = backend.capabilities();
        let backend_label = backend_label.into();
        let PreparedDebugTarget {
            runtime_path: _,
            display_path: _,
            provenance,
            required_tcl_abi,
            ..
        } = target;
        let TargetProvenance {
            original_path: _,
            runtime_path,
            display_path,
            decompiled_from,
            bytecode_tcl_version,
            temp_dir,
        } = provenance;
        let direct_bytecode = capabilities
            .support(BackendOperation::DirectBytecode)
            .is_supported();
        let user_script_origin = if compiled_source {
            SourceOrigin::CompiledSource
        } else if decompiled_from.is_some() || direct_bytecode {
            SourceOrigin::Synthetic
        } else {
            SourceOrigin::UserScript
        };

        let highlighted = if direct_bytecode {
            placeholder_lines("Loading bytecode disassembly…".into(), &theme)
        } else {
            load_source_highlighted(&runtime_path, &theme)
        };
        let theme_name = theme.name().to_string();
        let mut initial_status = "Starting the debugger… press ? for help".to_string();
        let mut initial_event: Option<EventLine> = None;
        if let Some(orig) = decompiled_from.as_ref() {
            let version = bytecode_tcl_version
                .as_deref()
                .unwrap_or("unknown Tcl version");
            let abi = required_tcl_abi
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown ABI".to_string());
            initial_status = format!(
                "decompiled {} → {} ({version}; {abi}; showing decompiled Tcl)",
                orig.display(),
                runtime_path.display()
            );
            initial_event = Some(EventLine::info(format!(
                "decompiled {} → {} ({version}; {abi})",
                orig.display(),
                runtime_path.display()
            )));
        }

        let mut events: VecDeque<EventLine> = VecDeque::with_capacity(EVENT_LOG_CAPACITY);
        if let Some(line) = initial_event {
            events.push_back(line);
        }
        // The nub path matters when something is wrong, not every launch.
        let (backend_summary, backend_detail) = match backend_label.split_once("; ") {
            Some((summary, detail)) => (summary.to_string(), Some(detail.to_string())),
            None => (backend_label.clone(), None),
        };
        events.push_back(EventLine::info(format!("Debugging with {backend_summary}")));
        if let Some(detail) = backend_detail {
            events.push_back(EventLine::trace(detail));
        }
        Ok(Self {
            user_script: runtime_path,
            display_path,
            _temp_dir: temp_dir,
            backend: Some(backend),
            capabilities,
            backend_label,
            cache: SourceCache {
                user_script: highlighted,
                user_script_origin,
                proc_bodies: HashMap::new(),
                source_units: HashMap::new(),
                showing_block_id: None,
                showing_qname: None,
                origin: user_script_origin,
                secondary: Vec::new(),
                secondary_highlight_line: None,
                pc_to_line: HashMap::new(),
                line_to_code: HashMap::new(),
                proc_placements: HashMap::new(),
                mapped_proc: None,
            },
            app_state: AppState::Dead,
            focused_proc: None,
            paused_focus: None,
            inspected_frame: 0,
            stack: Vec::new(),
            locals: Vec::new(),
            locals_prev: HashMap::new(),
            breakpoints: Vec::new(),
            watches: Vec::new(),
            coverage: Vec::new(),
            // Direct-bytecode sessions historically surfaced VM coverage as a
            // primary view. Keep that useful default while source sessions
            // retain their quieter opt-in coverage pane.
            coverage_enabled: direct_bytecode,
            events,
            event_filter: String::new(),
            last_status_full: initial_status.clone(),
            proc_picker: Vec::new(),
            proc_picker_filtered: Vec::new(),
            proc_picker_selected: None,
            search_query: String::new(),
            search_match_lines: Vec::new(),
            search_cursor: None,
            command_history: VecDeque::with_capacity(64),
            last_action: None,
            ui: DebugUiState::new(),
            prompt: None,
            error_banner: None,
            status: initial_status,
            transcript_path: None,
            quitting: false,
            theme,
            theme_name,
            instruction_set: None,
        })
    }

    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.app_state = AppState::Starting;
        self.set_status("Starting debugger");
        tcld_tui::terminal::run(
            "debug",
            DebugSurface {
                app: self,
                startup: DebugStartup::Connect,
            },
        )
    }

    fn connect_debugger(&mut self) -> bool {
        self.app_state = AppState::Ready;
        self.set_status(format!("Ready · {}", self.backend_label));
        true
    }

    fn start_debug_target(&mut self) {
        self.try_autoload_persisted_breakpoints();
        let start = self
            .backend
            .as_mut()
            .expect("constructed backend")
            .start_target(&self.user_script, true);
        match start {
            Ok(()) => {
                self.app_state = AppState::Loading;
                self.set_status("Loading");
                self.push_event(EventLine::trace("target scheduled"));
                self.tick();
            }
            Err(error) => {
                self.set_status(format!("target start failed: {error}"));
                self.push_event(EventLine::error(format!("target start failed: {error}")));
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect) {
        // Labels are computed before the event ring is borrowed below.
        let source_label = self.source_label();
        let location_label = self.location_label();
        let paused_line = self.displayed_pause_line();
        let target_path = self.display_path.display().to_string();
        let displayed_unit = self
            .cache
            .showing_block_id
            .as_ref()
            .and_then(|block| self.cache.source_units.get(block));
        let source_lines: &[SourceLine] = displayed_unit.map_or_else(
            || match &self.cache.showing_qname {
                Some(qname) => self
                    .cache
                    .proc_bodies
                    .get(qname)
                    .map(Vec::as_slice)
                    .unwrap_or(&self.cache.user_script),
                None => &self.cache.user_script,
            },
            |unit| unit.lines.as_slice(),
        );
        let source_path_label = displayed_unit
            .map(|unit| unit.registration.identity.display_label())
            .unwrap_or_else(|| self.display_path.display().to_string());
        let source_qname = displayed_unit
            .and_then(|unit| unit.registration.qualified_proc.as_deref())
            .or(self.cache.showing_qname.as_deref());
        // Make the event ring buffer contiguous so we can hand a flat
        // slice to the renderer. All other borrows above are on disjoint
        // cache fields.
        let events_slice: &[EventLine] = self.events.make_contiguous();
        let picker_overlay = if matches!(
            self.prompt.as_ref().map(|p| p.kind),
            Some(PromptKind::ProcPicker)
        ) {
            Some(PickerOverlay {
                title: "Go to proc — type to filter, ↑/↓ choose, Enter opens",
                entries: &self.proc_picker_filtered,
                selected: self.proc_picker_selected,
            })
        } else {
            None
        };
        let source_search = SourceSearch {
            query: &self.search_query,
            match_lines: &self.search_match_lines,
            current: self.search_cursor,
        };
        let snapshot = DebugSnapshot {
            title: APP_TITLE,
            target_path: &target_path,
            source_path: &source_path_label,
            source_label: &source_label,
            source_origin: self.cache.origin,
            source_qname,
            source_lines,
            secondary_source_lines: &self.cache.secondary,
            secondary_highlight_line: self.cache.secondary_highlight_line,
            focused_proc: self.focused_proc.as_ref(),
            paused_line,
            location_label: location_label.as_deref(),
            app_state: self.app_state,
            stack: &self.stack,
            inspected_frame: self.inspected_frame,
            locals: &self.locals,
            breakpoints: &self.breakpoints,
            watches: &self.watches,
            events: events_slice,
            event_filter: &self.event_filter,
            coverage: &self.coverage,
            coverage_enabled: self.coverage_enabled,
            status: &self.status,
            source_search,
            picker: picker_overlay,
            theme_label: &self.theme_name,
            instruction_set: self.instruction_set.as_deref(),
            error_banner: self.error_banner.as_deref(),
        };
        let prompt = self.prompt.as_ref().map(|p| &p.input);
        render_in(frame, area, &snapshot, &mut self.ui, prompt, &self.theme);
    }

    /// What the source pane shows, for its title.
    fn source_label(&self) -> String {
        if let Some((qname, placement)) = &self.cache.mapped_proc {
            return format!("{} · in {qname}", self.file_display_name(&placement.path));
        }
        match self.displayed_source_unit() {
            Some(unit) => match &unit.registration.identity {
                DebugSourceUnitIdentity::File(path) => self.file_display_name(path),
                DebugSourceUnitIdentity::Procedure(qname) => qname.clone(),
                DebugSourceUnitIdentity::Anonymous { .. } => "generated code".to_string(),
            },
            None => match &self.cache.showing_qname {
                Some(qname) => qname.clone(),
                None => self.file_display_name(&self.user_script),
            },
        }
    }

    /// How a file is named in labels: the main script by the name the user
    /// gave, any other file by its own name.
    fn file_display_name(&self, path: &std::path::Path) -> String {
        let shown = if same_file(path, &self.user_script) {
            self.display_path.as_path()
        } else {
            path
        };
        shown
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| shown.display().to_string())
    }

    /// Where execution is paused (or the selected frame is), for the title bar.
    fn location_label(&self) -> Option<String> {
        let focus = self.focused_proc.as_ref()?;
        // Bytecode pauses are addressed by code object and PC.
        if focus.block_id.starts_with("code:") {
            return Some(format!("{} pc {}", focus.block_id, focus.offset_start));
        }
        let place = self.place_label(
            focus.block_id.as_str(),
            (!focus.qname.is_empty()).then_some(focus.qname.as_str()),
            focus.source_path.as_deref(),
            Some(focus.line).filter(|line| *line > 0),
        )?;
        Some(match focus.qname.as_str() {
            "" => place,
            qname => format!("{qname} · {place}"),
        })
    }

    /// `demo.tcl:12` for a source location, mapping proc-relative lines back
    /// onto the file that defines the proc.
    fn place_label(
        &self,
        block_id: &str,
        qname: Option<&str>,
        source_path: Option<&str>,
        line: Option<u32>,
    ) -> Option<String> {
        let line = line?;
        if let Some(qname) = qname.filter(|_| block_id.starts_with("proc:"))
            && let Some(Some(placement)) = self.cache.proc_placements.get(qname)
        {
            return Some(format!(
                "{}:{}",
                self.file_display_name(&placement.path),
                line + placement.offset
            ));
        }
        // A proc or eval unit not found in its file only knows its own line
        // numbers; claiming a file line would point at the wrong text.
        if block_id.starts_with("proc:") || block_id.starts_with("anon:") || qname.is_some() {
            return Some(format!("line {line}"));
        }
        if block_id.starts_with("code:") {
            return Some(format!("disassembly line {line}"));
        }
        let path = source_path
            .map(std::path::Path::new)
            .or_else(|| block_id.strip_prefix("file:").map(std::path::Path::new));
        Some(match path {
            Some(path) => format!("{}:{line}", self.file_display_name(path)),
            None => format!("line {line}"),
        })
    }

    /// The source-pane line holding the paused (or selected-frame) command,
    /// if the pane is showing the unit that command belongs to.
    fn displayed_pause_line(&self) -> Option<u32> {
        let focus = self.focused_proc.as_ref()?;
        if focus.line == 0 {
            return None;
        }
        if let Some((qname, placement)) = &self.cache.mapped_proc {
            return (qname == &focus.qname).then_some(focus.line + placement.offset);
        }
        if self.cache.origin == SourceOrigin::Synthetic
            || self.cache.showing_block_id.as_deref() == Some(focus.block_id.as_str())
        {
            return Some(focus.line);
        }
        let shows_paused_unit = match (&self.cache.showing_block_id, &self.cache.showing_qname) {
            (None, Some(qname)) => qname == &focus.qname && focus.block_id.starts_with("proc:"),
            (None, None) => {
                let in_script = |path: Option<&str>| {
                    path.is_some_and(|path| {
                        same_file(std::path::Path::new(path), &self.user_script)
                    })
                };
                in_script(focus.block_id.strip_prefix("file:"))
                    || (in_script(focus.source_path.as_deref())
                        && !focus.block_id.starts_with("proc:"))
            }
            _ => false,
        };
        shows_paused_unit.then_some(focus.line)
    }

    /// Where `qname`'s body sits among the loaded files, if one of them
    /// defines that proc with exactly the body the runtime reports.
    fn proc_placement(&mut self, qname: &str) -> Option<ProcPlacement> {
        if let Some(cached) = self.cache.proc_placements.get(qname) {
            return cached.clone();
        }
        let placement = self.locate_proc(qname);
        self.cache
            .proc_placements
            .insert(qname.to_string(), placement.clone());
        placement
    }

    fn locate_proc(&self, qname: &str) -> Option<ProcPlacement> {
        let body = join_source_lines(self.cache.proc_bodies.get(qname)?);
        let bare = qname.trim_start_matches("::");
        // The runtime records which script was being sourced when the proc
        // was defined. When that is known, only that file may claim it: the
        // same body text defined somewhere else is not this proc.
        let defined_in = self
            .cache
            .source_units
            .values()
            .find(|unit| {
                unit.registration
                    .qualified_proc
                    .as_deref()
                    .is_some_and(|proc| proc.trim_start_matches("::") == bare)
            })
            .and_then(|unit| unit.registration.definition_path.clone());
        let unit_for = |path: &std::path::Path| {
            self.cache
                .source_units
                .iter()
                .find(|(_, unit)| {
                    matches!(&unit.registration.identity,
                        DebugSourceUnitIdentity::File(unit_path) if same_file(unit_path, path))
                })
                .map(|(block, _)| block.clone())
        };
        let mut files = vec![(
            unit_for(&self.user_script),
            self.user_script.clone(),
            join_source_lines(&self.cache.user_script),
        )];
        let mut sourced: Vec<_> = self
            .cache
            .source_units
            .iter()
            .filter_map(|(block, unit)| match &unit.registration.identity {
                DebugSourceUnitIdentity::File(path) if !same_file(path, &self.user_script) => {
                    Some((
                        Some(block.clone()),
                        path.clone(),
                        join_source_lines(&unit.lines),
                    ))
                }
                _ => None,
            })
            .collect();
        sourced.sort_by(|left, right| left.1.cmp(&right.1));
        files.extend(sourced);
        files
            .into_iter()
            .filter(|(_, path, _)| {
                defined_in
                    .as_deref()
                    .is_none_or(|defined| same_file(path, defined))
            })
            .find_map(|(file_block, path, text)| {
                proc_body_line_offset(&text, qname, &body).map(|offset| ProcPlacement {
                    file_block,
                    path,
                    offset,
                })
            })
    }

    fn current_source_lines(&self) -> &[SourceLine] {
        if let Some(unit) = self.displayed_source_unit() {
            return &unit.lines;
        }
        match &self.cache.showing_qname {
            Some(q) => self
                .cache
                .proc_bodies
                .get(q)
                .map(Vec::as_slice)
                .unwrap_or(&self.cache.user_script),
            None => &self.cache.user_script,
        }
    }

    fn displayed_source_unit(&self) -> Option<&CachedSourceUnit> {
        self.cache
            .showing_block_id
            .as_ref()
            .and_then(|block| self.cache.source_units.get(block))
    }

    fn displayed_file_path(&self) -> Option<PathBuf> {
        self.displayed_source_unit()
            .and_then(|unit| unit.registration.path.clone())
            .or_else(|| {
                self.cache
                    .showing_block_id
                    .is_none()
                    .then(|| self.user_script.clone())
            })
    }

    fn current_source_lines_len(&self) -> usize {
        self.current_source_lines().len()
    }

    fn tick(&mut self) -> bool {
        let Some(backend) = self.backend.as_mut() else {
            return false;
        };
        let events = backend.drain_events();
        if events.is_empty() {
            return false;
        }
        let mut needs_refresh = false;
        let mut needs_revalidate = Vec::new();
        let mut pauses = Vec::new();
        for ev in events {
            match ev {
                DebugEvent::StateChanged(state) => {
                    self.app_state = state.into();
                    self.set_status(self.app_state.label());
                }
                DebugEvent::Completed { result } => {
                    self.app_state = AppState::Completed;
                    let result = result.filter(|value| !value.is_empty());
                    self.push_event(EventLine::pause(match &result {
                        Some(value) => format!("■ Program finished · result: {value}"),
                        None => "■ Program finished".to_owned(),
                    }));
                    self.set_status(match result {
                        Some(value) => {
                            format!("Completed · {value} — r restarts, q quits")
                        }
                        None => "Completed — r restarts, q quits".to_owned(),
                    });
                    self.error_banner = None;
                    self.clear_execution_state();
                }
                DebugEvent::Failed { message } => {
                    self.app_state = AppState::LoadFailed;
                    self.set_status(format!("Load Failed: {}", trim_status(&message)));
                    self.clear_execution_state();
                    self.error_banner = Some(message.clone());
                    self.push_event(EventLine::error(message));
                }
                DebugEvent::Paused(pause) => {
                    self.app_state = AppState::Stopped;
                    pauses.push(*pause);
                    needs_refresh = true;
                }
                DebugEvent::Output { stream, text } => {
                    let severity = match stream {
                        DebugOutputStream::Stdout => EventSeverity::Output,
                        DebugOutputStream::Stderr => EventSeverity::ErrorOutput,
                        DebugOutputStream::Console => EventSeverity::Info,
                    };
                    self.push_event_text(severity, text.trim_end_matches('\n'));
                }
                DebugEvent::Diagnostic { severity, message } => {
                    let severity = match severity {
                        DebugDiagnosticSeverity::Info => tcld_tui::debug::EventSeverity::Info,
                        DebugDiagnosticSeverity::Warning => tcld_tui::debug::EventSeverity::Warn,
                        DebugDiagnosticSeverity::Error => tcld_tui::debug::EventSeverity::Error,
                    };
                    self.push_event_text(severity, &message);
                }
                DebugEvent::TargetLoadFailed {
                    path,
                    status,
                    message,
                    error_code,
                    error_info,
                } => {
                    self.app_state = AppState::LoadFailed;
                    self.set_status(format!("Load Failed: {}", trim_status(&message)));
                    self.clear_execution_state();
                    self.error_banner = Some(message.clone());
                    self.push_event_text(
                        tcld_tui::debug::EventSeverity::Error,
                        &format!(
                            "target_load_failed path={} status={status} message={message}\nerror_code={error_code}\nerror_info={error_info}",
                            path.display()
                        ),
                    );
                }
                DebugEvent::SourceRegistered(source) => {
                    self.push_event(EventLine::trace(format!(
                        "source unit registered: {} generation {}",
                        source.identity.display_label(),
                        source.generation
                    )));
                    needs_revalidate.push(source.block_id);
                }
                DebugEvent::SourceRevalidated(source) => {
                    self.push_event(EventLine::trace(format!(
                        "source unit revalidated: {} generation {}",
                        source.identity.display_label(),
                        source.generation
                    )));
                    needs_revalidate.push(source.block_id);
                }
                DebugEvent::SourceUnavailable {
                    identity,
                    block_id,
                    reason,
                } => {
                    self.cache.source_units.remove(&block_id);
                    if self.cache.showing_block_id.as_deref() == Some(&block_id) {
                        self.cache.showing_block_id = None;
                        self.cache.showing_qname = None;
                        self.cache.origin = self.cache.user_script_origin;
                    }
                    self.push_event(EventLine::warn(format!(
                        "source unit unavailable: {}: {reason}",
                        identity.display_label()
                    )));
                }
                DebugEvent::CompanionSourceDiscovered(source) => {
                    self.push_event(EventLine::warn(format!(
                        "unverified companion source {}: {}",
                        source.path.display(),
                        source.reason
                    )))
                }
                DebugEvent::CodeRegistered(code) => self.push_event(EventLine::trace(format!(
                    "code object {} registered{}",
                    code.id.0,
                    code.name
                        .as_deref()
                        .map(|name| format!(" ({name})"))
                        .unwrap_or_default()
                ))),
            }
        }
        let source_registry_changed = !needs_revalidate.is_empty();
        for block in needs_revalidate {
            self.invalidate_block(&block);
        }
        if source_registry_changed {
            self.refresh_source_artifacts();
            self.reload_breakpoints();
            self.update_source_view();
        }
        if !pauses.is_empty() {
            self.refresh_source_artifacts();
            for pause in pauses {
                self.apply_pause(pause);
            }
        }
        if needs_refresh && self.app_state == AppState::Stopped {
            self.refresh_state();
        }
        true
    }

    fn clear_execution_state(&mut self) {
        self.cache.mapped_proc = None;
        self.focused_proc = None;
        self.paused_focus = None;
        self.inspected_frame = 0;
        self.stack.clear();
        self.locals.clear();
        self.cache.showing_qname = None;
        self.cache.showing_block_id = None;
        self.cache.origin = self.cache.user_script_origin;
        self.ui.selections.stack.select(None);
        self.ui.selections.locals.select(None);
    }

    fn invalidate_block(&mut self, block: &str) {
        // A file that is new or has new text can change where any proc
        // lives, not just the procs it defined last time.
        if block.starts_with("file:") {
            self.cache.proc_placements.clear();
        }
        if let Some(qname) = block.strip_prefix("proc:") {
            self.cache.proc_bodies.remove(qname);
            self.cache.proc_placements.remove(qname);
            if self.cache.showing_qname.as_deref() == Some(qname) {
                // The currently-displayed body is stale; trigger a re-fetch
                // on the next refresh by clearing the showing flag.
                self.cache.showing_qname = None;
                self.cache.origin = self.cache.user_script_origin;
            }
        }
        self.cache.source_units.remove(block);
        if self.cache.showing_block_id.as_deref() == Some(block) {
            self.cache.showing_block_id = None;
        }
    }

    fn apply_pause(&mut self, pause: super::contract::BackendPause) {
        let source = pause.location.source.as_ref();
        let source_hint = pause.location.source_hint.as_ref();
        let code_line = pause
            .location
            .code
            .and_then(|location| self.cache.pc_to_line.get(&location.pc).copied());
        let line = source
            .map(|location| location.line)
            .or(code_line)
            .or_else(|| source_hint.map(|hint| hint.line))
            .unwrap_or(0);
        self.cache.secondary_highlight_line = if source.is_some() {
            code_line
        } else {
            source_hint.map(|hint| hint.line)
        };
        let qname = pause.proc_qname.clone().unwrap_or_default();
        let block_id = source
            .map(|location| location.block_id.clone())
            .unwrap_or_else(|| {
                pause.location.code.map_or_else(
                    || "runtime".to_owned(),
                    |location| format!("code:{}", location.code_id.0),
                )
            });
        let block_kind =
            source.map_or(
                tcld_tui::debug::SourceBlockKind::Anonymous,
                |location| match location.block_kind {
                    tcld_debug_wire::SourceBlockKind::File => {
                        tcld_tui::debug::SourceBlockKind::File
                    }
                    tcld_debug_wire::SourceBlockKind::Proc => {
                        tcld_tui::debug::SourceBlockKind::Proc
                    }
                    tcld_debug_wire::SourceBlockKind::Anonymous => {
                        tcld_tui::debug::SourceBlockKind::Anonymous
                    }
                },
            );
        let pause_reason = match pause.reason {
            DebugPauseReason::Entry => tcld_tui::debug::PauseReasonLabel::Entry,
            DebugPauseReason::Step | DebugPauseReason::StepOver | DebugPauseReason::StepOut => {
                tcld_tui::debug::PauseReasonLabel::Step
            }
            DebugPauseReason::Breakpoint => tcld_tui::debug::PauseReasonLabel::Breakpoint,
            DebugPauseReason::Watchpoint => tcld_tui::debug::PauseReasonLabel::Watchpoint,
            DebugPauseReason::Pause => tcld_tui::debug::PauseReasonLabel::Pause,
            DebugPauseReason::InstructionStep | DebugPauseReason::RunToLocation => {
                tcld_tui::debug::PauseReasonLabel::InstructionStep
            }
            DebugPauseReason::InstructionBreakpoint => {
                tcld_tui::debug::PauseReasonLabel::InstructionBreakpoint
            }
            DebugPauseReason::Error => tcld_tui::debug::PauseReasonLabel::Error,
        };
        let focus = FocusedProc {
            block_id,
            block_kind,
            source_path: source
                .and_then(|location| location.source_path.clone())
                .map(|path| path.to_string()),
            qname,
            line,
            offset_start: source
                .map(|location| location.offset_start)
                .or_else(|| pause.location.code.map(|location| location.pc))
                .unwrap_or(0),
            offset_end: source
                .map(|location| location.offset_end)
                .or_else(|| {
                    pause
                        .location
                        .code
                        .map(|location| location.pc.saturating_add(1))
                })
                .unwrap_or(0),
            pause_reason,
            frame_count: pause.frame_count,
            breakpoint: pause.breakpoint.map(|id| id.0),
            pause_kind: if pause.error.is_some() {
                tcld_tui::debug::PauseKind::Error
            } else {
                tcld_tui::debug::PauseKind::Normal
            },
            error_message: pause.error,
        };
        let reason_text = match (&pause.reason, focus.breakpoint.as_deref()) {
            (_, Some(id)) => format!("breakpoint {id}"),
            (DebugPauseReason::Entry, _) => "entry".to_owned(),
            (DebugPauseReason::Error, _) => "error".to_owned(),
            (DebugPauseReason::Watchpoint, _) => "variable written".to_owned(),
            (DebugPauseReason::Pause, _) => "paused by you".to_owned(),
            (DebugPauseReason::RunToLocation, _) => "reached the cursor".to_owned(),
            (DebugPauseReason::InstructionStep, _) => "instruction step".to_owned(),
            (DebugPauseReason::InstructionBreakpoint, _) => "instruction breakpoint".to_owned(),
            (DebugPauseReason::Breakpoint, None) => "breakpoint".to_owned(),
            (
                DebugPauseReason::Step | DebugPauseReason::StepOver | DebugPauseReason::StepOut,
                _,
            ) => "step".to_owned(),
        };
        let error = focus.error_message.clone();
        let proc_block = focus
            .block_id
            .starts_with("proc:")
            .then(|| focus.qname.clone());
        self.paused_focus = Some(focus.clone());
        self.focused_proc = Some(focus);
        self.ui.source_hscroll = 0;
        if let Some(qname) = proc_block
            && self.ensure_proc_body(&qname)
        {
            self.proc_placement(&qname);
        }
        let place = self
            .location_label()
            .unwrap_or_else(|| format_debug_location(&pause.location));
        self.set_status(match (&pause.reason, &error) {
            (DebugPauseReason::Entry, _) => "Paused before the first command".to_owned(),
            (_, Some(error)) => format!("Stopped on error: {}", trim_status(error)),
            (_, None) => format!("Paused · {reason_text}"),
        });
        self.push_event(EventLine::pause(format!(
            "⏸ paused at {place} · {reason_text}"
        )));
    }

    fn refresh_source_artifacts(&mut self) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        if let Ok(sources) = backend.sources() {
            for source in sources {
                let raw: Vec<String> = source.text.lines().map(str::to_owned).collect();
                let highlighted = highlight_lines(&raw, &self.theme);
                if let Some(qname) = source.source.qualified_proc.clone() {
                    self.cache.proc_bodies.insert(qname, highlighted.clone());
                }
                self.cache.source_units.insert(
                    source.source.block_id.clone(),
                    CachedSourceUnit {
                        registration: source.source,
                        lines: highlighted,
                    },
                );
            }
        }
        if self
            .capabilities
            .support(BackendOperation::DisassemblyAccess)
            .is_supported()
            && let Ok(disassemblies) = backend.disassembly()
            && let Some(disassembly) = disassemblies.first()
        {
            self.cache.pc_to_line.clear();
            self.cache.line_to_code.clear();
            for (index, line) in disassembly.lines.iter().enumerate() {
                let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
                if let Some(pc) = parse_disassembly_pc(line) {
                    self.cache.pc_to_line.insert(pc, line_number);
                    self.cache.line_to_code.insert(
                        line_number,
                        super::contract::DebugCodeLocation {
                            code_id: disassembly.code.id,
                            pc,
                        },
                    );
                }
            }
            if let Some(instruction_set) = disassembly.instruction_set.as_deref() {
                self.instruction_set = Some(instruction_set.to_owned());
            }
            if self
                .capabilities
                .support(BackendOperation::DirectBytecode)
                .is_supported()
            {
                self.cache.user_script = disassembly
                    .lines
                    .iter()
                    .cloned()
                    .map(SourceLine::raw)
                    .collect();
                self.cache.origin = SourceOrigin::Synthetic;
                self.cache.user_script_origin = SourceOrigin::Synthetic;
            } else {
                self.cache.secondary = disassembly
                    .lines
                    .iter()
                    .cloned()
                    .map(SourceLine::raw)
                    .collect();
            }
        }
        if let Ok(companions) = backend.companion_sources()
            && let Some(companion) = companions.first()
        {
            self.cache.secondary = companion
                .text
                .lines()
                .map(|line| SourceLine::raw(line.to_owned()))
                .collect();
        }
    }

    /// Re-read watch values for the inspected frame, keeping the change
    /// markers the last execution action produced: switching frames is not
    /// a change in the program.
    fn reload_watches(&mut self) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let fresh = if self.app_state == AppState::Stopped {
            backend
                .evaluate_watch_entries(self.inspected_frame)
                .or_else(|_| backend.watch_entries())
                .ok()
        } else {
            backend.watch_entries().ok()
        };
        match fresh {
            Some(mut watches) => {
                for watch in &mut watches {
                    watch.changed = self
                        .watches
                        .iter()
                        .any(|previous| previous.id == watch.id && previous.changed);
                }
                self.watches = watches;
            }
            None => self.evaluate_frontend_watches(),
        }
    }

    fn evaluate_frontend_watches(&mut self) {
        if self.app_state != AppState::Stopped
            || !self
                .capabilities
                .support(BackendOperation::Evaluate)
                .is_supported()
        {
            return;
        }
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        for watch in &mut self.watches {
            let previous = (watch.last_value.clone(), watch.last_error.clone());
            match backend.evaluate(
                self.inspected_frame,
                &format!("expr {}", quote_tcl_word(&watch.expr)),
            ) {
                Ok(result) if result.completion_code == 0 => {
                    watch.last_value = Some(result.result);
                    watch.last_error = None;
                }
                Ok(result) => {
                    watch.last_value = None;
                    watch.last_error = Some(format!(
                        "code {}: {}",
                        result.completion_code, result.result
                    ));
                }
                Err(error) => {
                    watch.last_value = None;
                    watch.last_error = Some(error.to_string());
                }
            }
            watch.changed |= previous != (watch.last_value.clone(), watch.last_error.clone());
        }
    }

    fn refresh_state(&mut self) {
        if self.backend.is_none() {
            return;
        }
        // Hosts list frames outermost first; instruction backends list the
        // paused frame first. The Stack pane always reads top-down from the
        // paused frame, like every other debugger.
        let paused_frame_first = self
            .capabilities
            .support(BackendOperation::InstructionBreakpoints)
            .is_supported();
        let (state, stack, breakpoints, watches, coverage) = {
            let backend = self.backend.as_mut().unwrap();
            let state = backend.state().unwrap_or(DebugLifecycleState::Paused);
            let mut stack: Vec<StackFrame> = backend
                .stack_frames()
                .unwrap_or_default()
                .into_iter()
                .map(backend_stack_to_tui)
                .collect();
            if !paused_frame_first {
                stack.reverse();
            }
            let breakpoints = backend
                .breakpoints()
                .unwrap_or_default()
                .into_iter()
                .map(backend_breakpoint_to_tui)
                .collect();
            // Watches read the frame the user will be looking at: the paused
            // one, which now leads the list.
            let watch_level = stack.first().map_or(0, |frame| frame.level);
            let watches = if state == DebugLifecycleState::Paused {
                backend
                    .evaluate_watch_entries(watch_level)
                    .or_else(|_| backend.watch_entries())
                    .ok()
            } else {
                backend.watch_entries().ok()
            };
            let coverage = if self.coverage_enabled {
                backend.coverage_snapshot().unwrap_or_default()
            } else {
                Vec::new()
            };
            (state, stack, breakpoints, watches, coverage)
        };
        self.app_state = state.into();
        self.stack = stack;
        self.breakpoints = breakpoints;
        let previous_watches: HashMap<&str, (&Option<String>, &Option<String>, bool)> = self
            .watches
            .iter()
            .map(|watch| {
                (
                    watch.id.as_str(),
                    (&watch.last_value, &watch.last_error, watch.changed),
                )
            })
            .collect();
        if let Some(mut watches) = watches {
            for watch in &mut watches {
                watch.changed = previous_watches.get(watch.id.as_str()).is_some_and(
                    |(value, error, was_changed)| {
                        *was_changed
                            || (value.is_some() || error.is_some())
                                && (*value != &watch.last_value || *error != &watch.last_error)
                    },
                );
            }
            self.watches = watches;
        } else {
            self.evaluate_frontend_watches();
        }
        self.coverage = coverage;
        self.refresh_source_artifacts();
        // Each refresh inspects the paused frame again.
        if let Some(frame) = self.stack.first() {
            self.inspected_frame = frame.level;
            self.ui.selections.stack.select(Some(0));
        } else {
            self.inspected_frame = 0;
            self.ui.selections.stack.select(None);
        }
        self.update_locals();
        self.update_error_banner();
        self.update_source_view();
        self.label_stack();
        self.label_breakpoints();
        self.sync_selections();
    }

    /// Phrase each frame's location for the Stack pane, mapping proc lines
    /// onto their files where possible.
    fn label_stack(&mut self) {
        let procs: Vec<String> = self
            .stack
            .iter()
            .filter(|frame| {
                frame
                    .block
                    .as_deref()
                    .is_some_and(|b| b.starts_with("proc:"))
            })
            .filter_map(|frame| frame.proc_name.clone())
            .collect();
        for qname in procs {
            if self.ensure_proc_body(&qname) {
                self.proc_placement(&qname);
            }
        }
        let labels: Vec<Option<String>> = self
            .stack
            .iter()
            .map(|frame| match (frame.block.as_deref(), frame.offset_start) {
                (Some(block), Some(pc)) if block.starts_with("code:") => {
                    Some(format!("{block} pc {pc}"))
                }
                (block, _) => self.place_label(
                    block.unwrap_or(""),
                    frame.proc_name.as_deref(),
                    frame.source_path.as_deref(),
                    frame.line,
                ),
            })
            .collect();
        for (frame, label) in self.stack.iter_mut().zip(labels) {
            frame.location_label = label;
        }
    }

    /// Phrase each breakpoint's location (`demo.tcl:12`) from its requested
    /// span: file lines directly, proc offsets through their file's text.
    fn label_breakpoints(&mut self) {
        let script = join_source_lines(&self.cache.user_script);
        let labels: Vec<Option<String>> = self
            .breakpoints
            .iter()
            .map(|bp| {
                if bp.qname.starts_with("code:") {
                    return Some(format!("pc {}", bp.offset));
                }
                if let Some(line) = bp.line {
                    return bp.source_path.as_deref().map(|path| {
                        format!(
                            "{}:{line}",
                            self.file_display_name(std::path::Path::new(path))
                        )
                    });
                }
                if bp.qname.is_empty() {
                    // A command span in a file: byte offset into that file.
                    let path = std::path::Path::new(bp.source_path.as_deref()?);
                    let text = if same_file(path, &self.user_script) {
                        Some(script.clone())
                    } else {
                        self.cache
                            .source_units
                            .values()
                            .find(|unit| {
                                unit.registration
                                    .path
                                    .as_deref()
                                    .is_some_and(|unit_path| same_file(unit_path, path))
                            })
                            .map(|unit| join_source_lines(&unit.lines))
                    }?;
                    let line = line_for_offset(&text, bp.offset)?;
                    return Some(format!("{}:{line}", self.file_display_name(path)));
                }
                let body_line = self
                    .cache
                    .proc_bodies
                    .get(&bp.qname)
                    .and_then(|body| line_for_offset(&join_source_lines(body), bp.offset));
                Some(
                    match (self.cache.proc_placements.get(&bp.qname), body_line) {
                        (Some(Some(placement)), Some(line)) => format!(
                            "{}:{} · {}",
                            self.file_display_name(&placement.path),
                            line + placement.offset,
                            bp.qname
                        ),
                        (_, Some(line)) => format!("{} line {line}", bp.qname),
                        // Not defined yet: all that is known is the offset.
                        (_, None) => format!("{} +{}", bp.qname, bp.offset),
                    },
                )
            })
            .collect();
        for (bp, label) in self.breakpoints.iter_mut().zip(labels) {
            bp.location_label = label;
        }
    }

    fn update_locals(&mut self) {
        if self.backend.is_none() || self.app_state != AppState::Stopped {
            self.locals.clear();
            return;
        }
        let level = self.inspected_frame;
        let fresh = self
            .backend
            .as_mut()
            .unwrap()
            .variables_for_frame(level)
            .unwrap_or_default();
        let fresh: Vec<(String, String)> = fresh
            .into_iter()
            .map(|variable| (variable.name, variable.value))
            .collect();
        // Diff against the last snapshot of this frame. A frame keeps its
        // depth from the outermost frame across pauses, while some backends
        // number levels from the innermost one.
        let frame_key = self
            .stack
            .iter()
            .position(|frame| frame.level == level)
            .map_or((level, None), |index| {
                (
                    (self.stack.len() - 1 - index) as i64,
                    self.stack[index].proc_name.clone(),
                )
            });
        let prev = self
            .locals_prev
            .get(&frame_key)
            .cloned()
            .unwrap_or_default();
        let prev_map: HashMap<&str, &str> =
            prev.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        // Tcl's own globals only crowd the global frame; inside a proc a
        // variable called `env` is the program's.
        let global_frame = self
            .stack
            .iter()
            .find(|frame| frame.level == level)
            .is_none_or(|frame| frame.proc_name.is_none());
        let mut rows: Vec<LocalRow> = fresh
            .iter()
            .map(|(name, value)| {
                let changed = !prev.is_empty()
                    && prev_map
                        .get(name.as_str())
                        .map(|prev_v| *prev_v != value.as_str())
                        .unwrap_or(true);
                LocalRow {
                    name: name.clone(),
                    value: value.clone(),
                    changed,
                    builtin: global_frame && is_tcl_builtin_global(name),
                }
            })
            .collect();
        // `info vars` order is hash order; a stable alphabetical list lets
        // the eye find a variable again after every step.
        rows.sort_by(|left, right| {
            (left.builtin, left.name.to_lowercase())
                .cmp(&(right.builtin, right.name.to_lowercase()))
        });
        self.locals_prev.insert(frame_key, fresh);
        self.locals = rows;
    }

    fn update_error_banner(&mut self) {
        self.error_banner = self
            .focused_proc
            .as_ref()
            .filter(|f| matches!(f.pause_kind, tcld_tui::debug::PauseKind::Error))
            .and_then(|f| f.error_message.clone());
    }

    /// Choose what the source pane should be displaying based on the
    /// current pause. If the paused block resolves to a proc whose body
    /// we've cached, show that; otherwise fall back to the user script.
    fn update_source_view(&mut self) {
        self.place_source_view();
        self.select_paused_command();
    }

    fn place_source_view(&mut self) {
        self.cache.mapped_proc = None;
        let Some(focused) = self.focused_proc.clone() else {
            self.cache.showing_block_id = None;
            self.cache.showing_qname = None;
            self.cache.origin = self.cache.user_script_origin;
            return;
        };
        if self.show_proc_in_file(&focused) {
            return;
        }
        if let Some(unit) = self.cache.source_units.get(&focused.block_id) {
            self.cache.showing_block_id = Some(focused.block_id.clone());
            self.cache.showing_qname = unit.registration.qualified_proc.clone();
            self.cache.origin = match &unit.registration.identity {
                DebugSourceUnitIdentity::File(path) => {
                    if same_file(path, &self.user_script) {
                        self.cache.user_script_origin
                    } else {
                        SourceOrigin::SourcedFile
                    }
                }
                DebugSourceUnitIdentity::Procedure(_)
                    if self.cache.user_script_origin == SourceOrigin::CompiledSource =>
                {
                    SourceOrigin::CompiledSource
                }
                DebugSourceUnitIdentity::Procedure(_) => SourceOrigin::ProcBody,
                DebugSourceUnitIdentity::Anonymous { .. } => SourceOrigin::Anonymous,
            };
            let line = focused.line.saturating_sub(1) as usize;
            if line < self.current_source_lines().len() {
                self.ui.source_cursor = line;
                self.ui.center_source_cursor = true;
            }
            return;
        }
        let needs_proc_body = focused.block_id.starts_with("proc:");
        if !needs_proc_body {
            self.cache.showing_block_id = None;
            self.cache.showing_qname = None;
            self.cache.origin = self.cache.user_script_origin;
            if focused.line > 0 {
                self.ui.source_cursor = (focused.line - 1) as usize;
                self.ui.source_cursor = self
                    .ui
                    .source_cursor
                    .min(self.current_source_lines_len().saturating_sub(1));
                self.ui.center_source_cursor = true;
            }
            return;
        }

        let qname = focused.qname.clone();
        if self.ensure_proc_body(&qname) {
            self.cache.showing_block_id = None;
            self.cache.showing_qname = Some(qname);
            self.cache.origin = SourceOrigin::ProcBody;
            // Auto-position cursor on the paused line if it lands inside
            // the body.
            let line = focused.line.saturating_sub(1) as usize;
            if line < self.current_source_lines().len() {
                self.ui.source_cursor = line;
                self.ui.center_source_cursor = true;
            }
        } else {
            self.cache.showing_block_id = None;
            self.cache.showing_qname = None;
            self.cache.origin = self.cache.user_script_origin;
        }
    }

    /// Fetch and cache a proc's body if it is not cached yet. Returns
    /// whether the body is available.
    fn ensure_proc_body(&mut self, qname: &str) -> bool {
        if qname.is_empty() {
            return false;
        }
        // Fetching from a running program would interrupt it for something
        // only needed once it pauses.
        if !self.cache.proc_bodies.contains_key(qname)
            && self.app_state != AppState::Running
            && let Some(backend) = self.backend.as_mut()
            && let Ok(source) = backend.procedure_source(qname)
        {
            let raw: Vec<String> = source.text.lines().map(str::to_string).collect();
            let highlighted = highlight_lines(&raw, &self.theme);
            self.cache
                .proc_bodies
                .insert(qname.to_string(), highlighted);
        }
        self.cache.proc_bodies.contains_key(qname)
    }

    /// Show a paused proc inside the file that defines it with the body being
    /// executed: the main script or a file it sourced. Returns whether it did.
    fn show_proc_in_file(&mut self, focused: &FocusedProc) -> bool {
        if !focused.block_id.starts_with("proc:") || !self.ensure_proc_body(&focused.qname) {
            return false;
        }
        let Some(placement) = self.proc_placement(&focused.qname) else {
            return false;
        };
        self.cache.showing_block_id = placement.file_block.clone();
        self.cache.showing_qname = None;
        self.cache.origin = if same_file(&placement.path, &self.user_script) {
            self.cache.user_script_origin
        } else {
            SourceOrigin::SourcedFile
        };
        let line = (focused.line + placement.offset).saturating_sub(1) as usize;
        self.cache.mapped_proc = Some((focused.qname.clone(), placement));
        if line < self.current_source_lines_len() {
            self.ui.source_cursor = line;
            self.ui.center_source_cursor = true;
        }
        true
    }

    /// Mark the paused command on its line. Tcl runs nested commands before
    /// the command containing them, so one line can pause once per command;
    /// the marker shows which of them runs next.
    fn select_paused_command(&mut self) {
        let Some(focus) = self.focused_proc.as_ref() else {
            return;
        };
        let source_offsets = ["file:", "proc:", "anon:", "image:"]
            .iter()
            .any(|prefix| focus.block_id.starts_with(prefix));
        if !source_offsets
            || self.displayed_pause_line() != u32::try_from(self.ui.source_cursor + 1).ok()
        {
            return;
        }
        // A compiled image reports offsets in the file it shows. Otherwise a
        // proc shown inside its file lists its commands under that proc, and
        // any other view shows the paused unit's own top-level commands.
        let in_image = focus.block_id.starts_with("image:");
        let in_mapped_proc = self.cache.mapped_proc.is_some();
        let offset = focus.offset_start;
        let index = self
            .current_source_lines()
            .get(self.ui.source_cursor)
            .and_then(|line| {
                line.commands.iter().position(|command| {
                    if in_image {
                        command.offset == offset
                    } else {
                        command.target_offset == offset && command.qname.is_some() == in_mapped_proc
                    }
                })
            });
        self.ui.source_command_cursor = index.unwrap_or(0);
    }

    fn sync_selections(&mut self) {
        let s = &mut self.ui.selections;
        sync(&mut s.stack, self.stack.len());
        sync(&mut s.locals, self.locals.len());
        sync(&mut s.breakpoints, self.breakpoints.len());
        sync(&mut s.watches, self.watches.len());
        sync(&mut s.coverage, self.coverage.len());
    }

    fn push_event(&mut self, line: EventLine) {
        self.events.push_back(line);
        if self.events.len() > EVENT_LOG_CAPACITY {
            self.events.pop_front();
        }
    }

    fn push_event_text(&mut self, severity: tcld_tui::debug::EventSeverity, text: &str) {
        let mut wrote = false;
        for line in text.lines() {
            self.push_event(EventLine::new(severity, line.to_string()));
            wrote = true;
        }
        if !wrote {
            self.push_event(EventLine::new(severity, String::new()));
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        let full = msg.into();
        self.last_status_full = full.clone();
        self.status = full;
    }

    fn require_capability(&mut self, operation: BackendOperation, action: &str) -> bool {
        match self.capabilities.support(operation) {
            super::contract::CapabilitySupport::Supported => true,
            super::contract::CapabilitySupport::Unsupported { reason } => {
                let message = format!("{action} unavailable: {reason}");
                // The footer may shorten a refusal; the console keeps it whole.
                self.push_event(EventLine::warn(message.clone()));
                self.set_status(message);
                false
            }
        }
    }

    fn reload_breakpoints(&mut self) {
        self.breakpoints = self
            .backend
            .as_mut()
            .and_then(|backend| backend.breakpoints().ok())
            .unwrap_or_default()
            .into_iter()
            .map(backend_breakpoint_to_tui)
            .collect();
        for qname in self
            .breakpoints
            .iter()
            .filter(|bp| !bp.qname.is_empty() && !bp.qname.starts_with("code:"))
            .map(|bp| bp.qname.clone())
            .collect::<Vec<_>>()
        {
            if self.ensure_proc_body(&qname) {
                self.proc_placement(&qname);
            }
        }
        self.label_breakpoints();
    }

    fn toggle_instruction_breakpoint(&mut self, location: super::contract::DebugCodeLocation) {
        if !self.require_capability(
            BackendOperation::InstructionBreakpoints,
            "instruction breakpoint",
        ) {
            return;
        }
        let existing = self
            .backend
            .as_mut()
            .and_then(|backend| backend.breakpoints().ok())
            .unwrap_or_default()
            .into_iter()
            .find(|breakpoint| {
                matches!(
                    breakpoint.spec,
                    DebugBreakpointSpec::Instruction { location: candidate, .. }
                        if candidate == location
                )
            });
        let result = if let Some(existing) = existing {
            let id = existing.id.clone();
            self.backend
                .as_mut()
                .unwrap()
                .remove_breakpoint(&id)
                .map(|_| {
                    format!(
                        "cleared instruction breakpoint {} @ pc {}",
                        id.0, location.pc
                    )
                })
        } else {
            self.backend
                .as_mut()
                .unwrap()
                .add_breakpoint(DebugBreakpointSpec::Instruction {
                    location,
                    condition: None,
                })
                .map(|breakpoint| {
                    format!(
                        "set instruction breakpoint {} @ code {} pc {}",
                        breakpoint.id.0, location.code_id.0, location.pc
                    )
                })
        };
        match result {
            Ok(status) => self.set_status(status),
            Err(error) => self.set_status(format!("instruction breakpoint failed: {error}")),
        }
        self.reload_breakpoints();
    }

    fn run_to_cursor(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "run-to unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        if !self.require_capability(BackendOperation::RunToLocation, "run to cursor") {
            return;
        }
        let line = u32::try_from(self.ui.source_cursor + 1).unwrap_or(u32::MAX);
        if !self
            .capabilities
            .support(BackendOperation::DirectBytecode)
            .is_supported()
        {
            let command = self
                .current_source_lines()
                .get(self.ui.source_cursor)
                .and_then(|source_line| {
                    source_line.commands.get(
                        self.ui
                            .source_command_cursor
                            .min(source_line.commands.len().saturating_sub(1)),
                    )
                })
                .cloned();
            let unit = self
                .displayed_source_unit()
                .map(|unit| unit.registration.clone());
            if let (Some(command), Some(unit)) = (command, unit) {
                let source = tcld_debug_wire::SourceLocation {
                    block_id: unit.block_id,
                    block_kind: if unit.qualified_proc.is_some() {
                        tcld_debug_wire::SourceBlockKind::Proc
                    } else {
                        tcld_debug_wire::SourceBlockKind::File
                    },
                    source_path: unit.path.map(|path| path.display().to_string()),
                    qualified_proc: unit.qualified_proc,
                    line,
                    offset_start: command.target_offset,
                    offset_end: command.target_offset.saturating_add(command.len.max(1)),
                };
                self.run_backend_action("run to cursor", |backend| {
                    backend.run_to_location(DebugLocation::source(source))
                });
                return;
            }
        }
        let Some(location) = self.cache.line_to_code.get(&line).copied() else {
            self.set_status("run to cursor unavailable: selected disassembly row has no PC");
            return;
        };
        self.run_backend_action("run to cursor", |backend| {
            backend.run_to_location(DebugLocation::code(location))
        });
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let binding_context = if self.ui.show_help {
            BindingContext::Help
        } else if matches!(
            self.prompt.as_ref().map(|prompt| prompt.kind),
            Some(PromptKind::ProcPicker)
        ) {
            BindingContext::ProcPicker
        } else if self.prompt.is_some() {
            BindingContext::Prompt
        } else {
            BindingContext::Pane(self.ui.focus)
        };
        let action = resolve_binding(binding_context, key);
        if action == Some(DebugAction::ForceQuit) {
            self.quitting = true;
            return;
        }
        if self.ui.confirm_quit {
            self.ui.confirm_quit = false;
            if matches!(key.code, crossterm::event::KeyCode::Char('y' | 'Y')) {
                self.quitting = true;
            } else {
                self.set_status("Still debugging");
            }
            return;
        }
        if let Some(popup) = self.ui.popup.as_mut() {
            use crossterm::event::KeyCode;
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => self.ui.popup = None,
                KeyCode::Char('j') | KeyCode::Down => popup.scroll = popup.scroll.saturating_add(1),
                KeyCode::Char('k') | KeyCode::Up => popup.scroll = popup.scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    popup.scroll = popup.scroll.saturating_add(10);
                }
                KeyCode::PageUp => popup.scroll = popup.scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => popup.scroll = 0,
                _ => {}
            }
            return;
        }
        if self.ui.show_help {
            match action {
                Some(DebugAction::ToggleHelp) => {
                    self.ui.show_help = false;
                    self.ui.help_scroll = 0;
                }
                Some(DebugAction::HelpDown) => {
                    self.ui.help_scroll = self.ui.help_scroll.saturating_add(1);
                }
                Some(DebugAction::HelpUp) => {
                    self.ui.help_scroll = self.ui.help_scroll.saturating_sub(1);
                }
                Some(DebugAction::HelpPageDown) => {
                    self.ui.help_scroll = self.ui.help_scroll.saturating_add(10);
                }
                Some(DebugAction::HelpPageUp) => {
                    self.ui.help_scroll = self.ui.help_scroll.saturating_sub(10);
                }
                _ => {}
            }
            return;
        }

        if self.prompt.is_some() {
            // Intercept history (up/down) and tab completion in `:` prompts,
            // and proc-picker navigation, before delegating to the generic
            // line-edit handler.
            let prompt_kind = self.prompt.as_ref().map(|p| p.kind);
            match (prompt_kind, action) {
                (Some(PromptKind::Command), Some(DebugAction::PromptUp)) => {
                    self.command_history_prev();
                    return;
                }
                (Some(PromptKind::Command), Some(DebugAction::PromptDown)) => {
                    self.command_history_next();
                    return;
                }
                (Some(PromptKind::Command), Some(DebugAction::PromptComplete)) => {
                    self.complete_dbg_command();
                    return;
                }
                (Some(PromptKind::ProcPicker), Some(DebugAction::PromptUp)) => {
                    self.picker_move(-1);
                    return;
                }
                (Some(PromptKind::ProcPicker), Some(DebugAction::PromptDown)) => {
                    self.picker_move(1);
                    return;
                }
                _ => {}
            }
            let step = {
                let prompt = self.prompt.as_mut().unwrap();
                prompt_step(&mut prompt.input, key)
            };
            match step {
                PromptStep::Pending => {
                    // Refresh source-search results live as the user types.
                    if matches!(prompt_kind, Some(PromptKind::SearchSource)) {
                        let q = self.prompt.as_ref().unwrap().input.buffer.clone();
                        self.update_source_search(&q);
                    } else if matches!(prompt_kind, Some(PromptKind::ProcPicker)) {
                        let q = self.prompt.as_ref().unwrap().input.buffer.clone();
                        self.update_proc_picker_filter(&q);
                    }
                }
                PromptStep::Cancel => {
                    if matches!(prompt_kind, Some(PromptKind::SearchSource)) {
                        // Cancelling drops search highlighting.
                        self.search_query.clear();
                        self.search_match_lines.clear();
                        self.search_cursor = None;
                    }
                    self.prompt = None;
                    self.proc_picker_filtered.clear();
                    self.proc_picker_selected = None;
                    self.set_status("Cancelled");
                }
                PromptStep::Submit(value) => {
                    let active = self.prompt.take().unwrap();
                    handle_prompt_submit(self, active, value);
                    self.proc_picker_filtered.clear();
                    self.proc_picker_selected = None;
                }
            }
            return;
        }

        if let Some(action) = action {
            self.dispatch_debug_action(action);
        }
    }

    fn dispatch_debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::Quit => {
                // Quitting kills the program being debugged; ask while it is
                // still alive, not after it has finished.
                if matches!(self.app_state, AppState::Stopped | AppState::Running) {
                    self.ui.confirm_quit = true;
                } else {
                    self.quitting = true;
                }
            }
            DebugAction::ForceQuit => self.quitting = true,
            DebugAction::ToggleHelp => self.ui.show_help = true,
            DebugAction::StatusInspect => self.open_status_inspect(),
            DebugAction::InspectValue => self.inspect_selected_value(),
            DebugAction::ScrollLeft => {
                self.ui.source_hscroll = self.ui.source_hscroll.saturating_sub(8);
            }
            DebugAction::ScrollRight => {
                self.ui.source_hscroll = self.ui.source_hscroll.saturating_add(8);
            }
            DebugAction::FocusNext => self.ui.focus_pane(self.next_pane(1)),
            DebugAction::FocusPrev => self.ui.focus_pane(self.next_pane(-1)),
            DebugAction::FocusPane(pane) => {
                self.ui.focus_pane(pane);
                if pane == Pane::Coverage && !self.coverage_enabled {
                    self.set_status("Coverage is off — press C to start collecting it");
                }
            }
            DebugAction::Continue => self.do_continue(),
            DebugAction::StepOver => self.do_step_over(),
            DebugAction::StepInto => self.do_step_into(),
            DebugAction::StepOut => self.do_step_out(),
            DebugAction::NextOrStepOver if !self.search_query.is_empty() => {
                self.search_next(1);
            }
            DebugAction::NextOrStepOver => self.do_step_over(),
            DebugAction::PreviousSearch if !self.search_query.is_empty() => self.search_next(-1),
            DebugAction::PreviousSearch => {}
            DebugAction::Pause => self.do_pause(),
            DebugAction::Restart => self.do_restart(),
            DebugAction::Refresh => {
                self.refresh_state();
                self.last_action = Some(RepeatableAction::Refresh);
                self.set_status("State refreshed");
            }
            DebugAction::UserSource => self.reset_to_user_script(),
            DebugAction::Repeat => self.repeat_last_action(),
            DebugAction::CycleTheme => self.cycle_theme(),
            DebugAction::Search => self.open_source_search(),
            DebugAction::IgnoreError => self.do_ignore_error(),
            DebugAction::ToggleCoverage => self.toggle_coverage(),
            DebugAction::ListSlaves => self.do_list_slaves(),
            DebugAction::PreInstrument => {
                if self.require_capability(BackendOperation::SourceAccess, "pre-instrument") {
                    self.open_prompt_full(PromptKind::PreInstrument, "instrument", None, None);
                }
            }
            DebugAction::ToggleBreakpoint => self.toggle_breakpoint_at_cursor(),
            DebugAction::AddBreakpoint => {
                self.open_prompt(PromptKind::AddBreakpoint, "breakpoint");
            }
            DebugAction::GotoProc => {
                if self.require_capability(BackendOperation::ProcedureList, "go to procedure") {
                    self.open_prompt(PromptKind::GotoProc, "go to proc");
                }
            }
            DebugAction::RunToCursor => self.run_to_cursor(),
            DebugAction::ProcPicker => self.open_proc_picker(),
            DebugAction::AddWatch => {
                if self.require_capability(BackendOperation::Watches, "add watch") {
                    self.open_prompt(PromptKind::AddWatch, "watch");
                }
            }
            DebugAction::AddWatchpoint => {
                if self
                    .require_capability(BackendOperation::SourceBreakpoints, "variable watchpoint")
                {
                    self.open_prompt(PromptKind::AddWatchpoint, "break on write");
                }
            }
            DebugAction::Eval => {
                if self.require_capability(BackendOperation::Evaluate, "evaluate") {
                    self.open_prompt(PromptKind::EvalExpr, "eval");
                }
            }
            DebugAction::Command => self.open_prompt(PromptKind::Command, "command"),
            DebugAction::EventFilter => self.open_prompt_full(
                PromptKind::EventFilter,
                "filter console",
                Some(self.event_filter.clone()),
                None,
            ),
            DebugAction::ExportBreakpoints => self.do_export_breakpoints(),
            DebugAction::ImportBreakpoints => self.do_import_breakpoints(),
            DebugAction::ToggleBreakpointEnabled => self.toggle_selected_bp_enabled(),
            DebugAction::SetIgnore => self.open_set_ignore_prompt(),
            DebugAction::EditCondition => self.open_edit_condition_prompt(),
            DebugAction::DeleteBreakpoint => self.delete_selected_bp(),
            DebugAction::DeleteWatch => self.delete_selected_watch(),
            DebugAction::PreviousCommand => self.move_source_command(-1),
            DebugAction::NextCommand => self.move_source_command(1),
            DebugAction::MoveUp => self.move_selection(-1),
            DebugAction::MoveDown => self.move_selection(1),
            DebugAction::PageUp => self.move_selection(-10),
            DebugAction::PageDown => self.move_selection(10),
            DebugAction::Home => self.move_selection_abs(0),
            DebugAction::End => self.move_selection_abs(usize::MAX),
            DebugAction::HelpDown
            | DebugAction::HelpUp
            | DebugAction::HelpPageDown
            | DebugAction::HelpPageUp
            | DebugAction::CancelPrompt
            | DebugAction::SubmitPrompt
            | DebugAction::PromptUp
            | DebugAction::PromptDown
            | DebugAction::PromptComplete => {}
        }
    }

    /// The pane `delta` steps away in Tab order, skipping Coverage while it
    /// is off.
    fn next_pane(&self, delta: isize) -> Pane {
        let mut pane = self.ui.focus;
        loop {
            pane = if delta > 0 { pane.next() } else { pane.prev() };
            if pane != Pane::Coverage || self.coverage_enabled {
                return pane;
            }
        }
    }

    /// Show the selected local or watch in full.
    fn inspect_selected_value(&mut self) {
        let popup = match self.ui.focus {
            Pane::Locals => self
                .ui
                .selections
                .locals
                .selected()
                .and_then(|index| self.locals.get(index))
                .map(|row| ValuePopup {
                    title: format!("{} · {} characters", row.name, row.value.chars().count()),
                    body: row.value.clone(),
                    scroll: 0,
                }),
            Pane::Watches => self
                .ui
                .selections
                .watches
                .selected()
                .and_then(|index| self.watches.get(index))
                .map(|watch| ValuePopup {
                    title: if watch.name == watch.expr {
                        watch.name.clone()
                    } else {
                        format!("{} = {}", watch.name, watch.expr)
                    },
                    body: match (&watch.last_value, &watch.last_error) {
                        (_, Some(error)) => format!("error: {error}"),
                        (Some(value), None) => value.clone(),
                        (None, None) => "(not evaluated yet — pause the program)".to_string(),
                    },
                    scroll: 0,
                }),
            _ => None,
        };
        match popup {
            Some(popup) => self.ui.popup = Some(popup),
            None => self.set_status("Nothing selected"),
        }
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(tab) = self.ui.hit.inspector_tab_at(m.column, m.row) {
                    self.ui.focus_pane(tab);
                    return;
                }
                if let Some(pane) = self.ui.hit.pane_at(m.column, m.row) {
                    self.ui.focus_pane(pane);
                    if pane == Pane::Source {
                        if let Some(line_idx) = self.ui.hit.source_line_at(m.column, m.row) {
                            let max = self.current_source_lines().len().saturating_sub(1);
                            self.ui.source_cursor = line_idx.min(max);
                            self.ui.source_command_cursor = 0;
                            // The gutter is where breakpoints live, as in
                            // every editor.
                            if self.ui.hit.in_source_gutter(m.column) {
                                self.toggle_breakpoint_at_cursor();
                            }
                        }
                    } else if let Some(row) = list_row(pane, &self.ui.hit, m.row) {
                        self.set_selection_for_pane(pane, row);
                        if pane == Pane::Stack {
                            self.on_stack_selection_changed();
                        }
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some(Pane::Source) = self.ui.hit.pane_at(m.column, m.row)
                    && let Some(line_idx) = self.ui.hit.source_line_at(m.column, m.row)
                {
                    let max = self.current_source_lines().len().saturating_sub(1);
                    self.ui.source_cursor = line_idx.min(max);
                    self.toggle_breakpoint_at_cursor();
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if matches!(m.kind, MouseEventKind::ScrollDown) {
                    3
                } else {
                    -3
                };
                if let Some(pane) = self.ui.hit.pane_at(m.column, m.row) {
                    let prior_focus = self.ui.focus;
                    self.ui.focus = pane;
                    if pane == Pane::Source {
                        // The wheel scrolls the view, not just the cursor.
                        let max = self.current_source_lines_len().saturating_sub(1);
                        self.ui.source_view_top = (self.ui.source_view_top as isize + delta)
                            .clamp(0, max as isize)
                            as usize;
                        self.ui.source_cursor = (self.ui.source_cursor as isize + delta)
                            .clamp(0, max as isize)
                            as usize;
                    } else {
                        self.move_selection(delta);
                    }
                    self.ui.focus = prior_focus;
                }
            }
            _ => {}
        }
    }

    fn set_selection_for_pane(&mut self, pane: Pane, row: usize) {
        let s = &mut self.ui.selections;
        match pane {
            Pane::Source => {}
            Pane::Stack => DebugUiState::set_list(&mut s.stack, self.stack.len(), row),
            Pane::Locals => DebugUiState::set_list(&mut s.locals, self.locals.len(), row),
            Pane::Breakpoints => {
                DebugUiState::set_list(&mut s.breakpoints, self.breakpoints.len(), row);
            }
            Pane::Watches => DebugUiState::set_list(&mut s.watches, self.watches.len(), row),
            Pane::Events => DebugUiState::set_list(&mut s.events, self.events.len(), row),
            Pane::Coverage => DebugUiState::set_list(&mut s.coverage, self.coverage.len(), row),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.ui.focus {
            Pane::Source => {
                let new = (self.ui.source_cursor as isize + delta).max(0) as usize;
                self.ui.source_cursor =
                    new.min(self.current_source_lines().len().saturating_sub(1));
                self.ui.source_command_cursor = 0;
            }
            Pane::Stack => {
                DebugUiState::move_list(&mut self.ui.selections.stack, self.stack.len(), delta);
                self.on_stack_selection_changed();
            }
            Pane::Locals => {
                DebugUiState::move_list(&mut self.ui.selections.locals, self.locals.len(), delta);
            }
            Pane::Breakpoints => DebugUiState::move_list(
                &mut self.ui.selections.breakpoints,
                self.breakpoints.len(),
                delta,
            ),
            Pane::Watches => {
                DebugUiState::move_list(&mut self.ui.selections.watches, self.watches.len(), delta);
            }
            Pane::Events => {
                // The console reads bottom-up from the newest line: moving
                // up scrolls back in time.
                self.ui.console_scroll = if delta < 0 {
                    self.ui.console_scroll.saturating_add(delta.unsigned_abs())
                } else {
                    self.ui.console_scroll.saturating_sub(delta.unsigned_abs())
                };
            }
            Pane::Coverage => DebugUiState::move_list(
                &mut self.ui.selections.coverage,
                self.coverage.len(),
                delta,
            ),
        }
    }

    fn move_selection_abs(&mut self, target: usize) {
        match self.ui.focus {
            Pane::Source => {
                self.ui.source_cursor =
                    target.min(self.current_source_lines().len().saturating_sub(1));
                self.ui.source_command_cursor = 0;
            }
            Pane::Stack => {
                DebugUiState::set_list(&mut self.ui.selections.stack, self.stack.len(), target);
                self.on_stack_selection_changed();
            }
            Pane::Locals => {
                DebugUiState::set_list(&mut self.ui.selections.locals, self.locals.len(), target);
            }
            Pane::Breakpoints => DebugUiState::set_list(
                &mut self.ui.selections.breakpoints,
                self.breakpoints.len(),
                target,
            ),
            Pane::Watches => {
                DebugUiState::set_list(&mut self.ui.selections.watches, self.watches.len(), target);
            }
            Pane::Events => {
                // Home is the oldest line, End follows new output.
                self.ui.console_scroll = if target == 0 { usize::MAX } else { 0 };
            }
            Pane::Coverage => DebugUiState::set_list(
                &mut self.ui.selections.coverage,
                self.coverage.len(),
                target,
            ),
        }
    }

    fn on_stack_selection_changed(&mut self) {
        let Some(idx) = self.ui.selections.stack.selected() else {
            return;
        };
        let Some(frame) = self.stack.get(idx).cloned() else {
            return;
        };
        if frame.level == self.inspected_frame {
            return;
        }
        self.inspected_frame = frame.level;
        if let Some(mut focus) = self.paused_focus.clone() {
            if let Some(block) = &frame.block {
                focus.block_id = block.clone();
            }
            if let Some(kind) = frame.block_kind {
                focus.block_kind = kind;
            }
            focus.source_path = frame.source_path.clone();
            focus.qname = frame.proc_name.clone().unwrap_or_default();
            if let Some(line) = frame.line {
                focus.line = line;
            }
            if let Some(offset) = frame.offset_start {
                focus.offset_start = offset;
            }
            if let Some(offset) = frame.offset_end {
                focus.offset_end = offset;
            }
            self.focused_proc = Some(focus);
        }
        if matches!(
            frame.block_kind,
            Some(tcld_tui::debug::SourceBlockKind::File)
        ) {
            self.cache.showing_block_id = frame
                .block
                .as_ref()
                .filter(|block| self.cache.source_units.contains_key(*block))
                .cloned();
            self.cache.showing_qname = None;
            self.cache.origin = self
                .cache
                .showing_block_id
                .as_ref()
                .map_or(self.cache.user_script_origin, |_| SourceOrigin::SourcedFile);
            if let Some(line) = frame.line {
                self.ui.source_cursor = line.saturating_sub(1) as usize;
                self.ui.source_cursor = self
                    .ui
                    .source_cursor
                    .min(self.current_source_lines_len().saturating_sub(1));
                self.ui.center_source_cursor = true;
            }
        }
        // Switch the source pane to that frame's proc body when we
        // have one; fall back to the user script otherwise.
        if let Some(block) = frame.block.as_deref()
            && let Some(qname) = block.strip_prefix("proc:")
        {
            let qname = qname.to_string();
            if !self.cache.proc_bodies.contains_key(&qname)
                && let Some(backend) = self.backend.as_mut()
                && let Ok(source) = backend.procedure_source(&qname)
            {
                let raw: Vec<String> = source.text.lines().map(str::to_string).collect();
                let highlighted = highlight_lines(&raw, &self.theme);
                self.cache.proc_bodies.insert(qname.clone(), highlighted);
            }
            if self.cache.proc_bodies.contains_key(&qname) {
                self.cache.showing_block_id = frame
                    .block
                    .as_ref()
                    .filter(|block| self.cache.source_units.contains_key(*block))
                    .cloned();
                self.cache.showing_qname = Some(qname);
                self.cache.origin = SourceOrigin::ProcBody;
                if let Some(line) = frame.line {
                    let target = line.saturating_sub(1) as usize;
                    let max = self.current_source_lines_len().saturating_sub(1);
                    self.ui.source_cursor = target.min(max);
                    self.ui.center_source_cursor = true;
                }
            }
        }
        self.update_source_view();
        self.update_locals();
        self.reload_watches();
    }

    fn open_prompt(&mut self, kind: PromptKind, label: &'static str) {
        self.open_prompt_full(kind, label, None, None);
    }

    fn open_prompt_full(
        &mut self,
        kind: PromptKind,
        label: &'static str,
        initial: Option<String>,
        bp_target: Option<String>,
    ) {
        let input = match initial {
            Some(text) => PromptInput::with_initial(label, text),
            None => PromptInput::new(label),
        };
        let placeholder = self.prompt_placeholder(kind);
        self.prompt = Some(ActivePrompt {
            kind,
            input: input.with_placeholder(placeholder),
            bp_target,
            body_buffer: None,
            history_pos: None,
        });
    }

    /// What belongs in a prompt and how to finish it.
    fn prompt_placeholder(&self, kind: PromptKind) -> String {
        match kind {
            PromptKind::Command => {
                "debugger command · Tab completes · help lists them · ↑ history".into()
            }
            PromptKind::GotoProc => "proc name · Enter opens its body".into(),
            PromptKind::AddWatch => {
                "variable or expression (or: name expression) · Enter adds".into()
            }
            PromptKind::AddWatchpoint => "variable name, then optional r/w/u operations".into(),
            PromptKind::EvalExpr => {
                let scope = self
                    .stack
                    .iter()
                    .find(|frame| frame.level == self.inspected_frame)
                    .map(|frame| {
                        frame
                            .proc_name
                            .clone()
                            .unwrap_or_else(|| "the global scope".into())
                    })
                    .unwrap_or_else(|| "the global scope".into());
                format!("Tcl script, run in {scope} · Enter runs")
            }
            PromptKind::AddBreakpoint => {
                if self
                    .capabilities
                    .support(BackendOperation::InstructionBreakpoints)
                    .is_supported()
                {
                    "<pc> [condition]".into()
                } else {
                    "<proc> <body offset> [condition]".into()
                }
            }
            PromptKind::EditCondition => {
                "stop only when this expression is true · empty clears".into()
            }
            PromptKind::SetIgnore => "hits to skip before stopping".into(),
            PromptKind::EventFilter => "text to match · empty shows everything".into(),
            PromptKind::ProcPicker => "type to filter · ↑/↓ choose · Enter opens".into(),
            PromptKind::PreInstrument => "<proc> <body…>".into(),
            PromptKind::SearchSource => {
                "text · Enter keeps matches · n/N next/prev · Esc clears".into()
            }
        }
    }

    fn open_status_inspect(&mut self) {
        // The status line clips long messages; this is where they are read
        // in full (and can be selected with the terminal).
        self.ui.popup = Some(ValuePopup {
            title: "Last message".to_string(),
            body: self.last_status_full.clone(),
            scroll: 0,
        });
    }

    fn open_edit_condition_prompt(&mut self) {
        let Some(idx) = self.ui.selections.breakpoints.selected() else {
            self.set_status("select a breakpoint first");
            return;
        };
        let Some(bp) = self.breakpoints.get(idx).cloned() else {
            return;
        };
        let initial = bp.condition.clone().unwrap_or_default();
        self.open_prompt_full(
            PromptKind::EditCondition,
            "condition",
            Some(initial),
            Some(bp.id),
        );
    }

    fn open_set_ignore_prompt(&mut self) {
        let Some(idx) = self.ui.selections.breakpoints.selected() else {
            self.set_status("select a breakpoint first");
            return;
        };
        let Some(bp) = self.breakpoints.get(idx).cloned() else {
            return;
        };
        self.open_prompt_full(
            PromptKind::SetIgnore,
            "skip hits",
            Some(bp.ignore_count.to_string()),
            Some(bp.id),
        );
    }

    fn open_proc_picker(&mut self) {
        if !self.require_capability(BackendOperation::ProcedureList, "procedure picker") {
            return;
        }
        let list = self
            .backend
            .as_mut()
            .unwrap()
            .procedures()
            .unwrap_or_default();
        self.proc_picker = list;
        self.update_proc_picker_filter("");
        self.open_prompt(PromptKind::ProcPicker, "go to proc");
    }

    fn open_source_search(&mut self) {
        let initial = self.search_query.clone();
        self.open_prompt_full(PromptKind::SearchSource, "search", Some(initial), None);
    }

    fn update_source_search(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.search_match_lines.clear();
        if query.is_empty() {
            self.search_cursor = None;
            return;
        }
        let q = query.to_lowercase();
        let lines = self.current_source_lines();
        let mut matches = Vec::new();
        for (idx, line) in lines.iter().enumerate() {
            if line.raw.to_lowercase().contains(&q) {
                matches.push(idx);
            }
        }
        self.search_match_lines = matches;
        // Pick the first match at-or-after the cursor; otherwise wrap.
        let cursor = self.ui.source_cursor;
        self.search_cursor = self
            .search_match_lines
            .iter()
            .position(|&l| l >= cursor)
            .or_else(|| (!self.search_match_lines.is_empty()).then_some(0));
        if let Some(c) = self.search_cursor {
            self.ui.source_cursor = self.search_match_lines[c];
        }
    }

    fn search_next(&mut self, delta: isize) {
        if self.search_match_lines.is_empty() {
            return;
        }
        let len = self.search_match_lines.len() as isize;
        let cur = self.search_cursor.unwrap_or(0) as isize;
        let next = ((cur + delta).rem_euclid(len)) as usize;
        self.search_cursor = Some(next);
        self.ui.source_cursor = self.search_match_lines[next];
    }

    fn update_proc_picker_filter(&mut self, query: &str) {
        let q = query.to_lowercase();
        self.proc_picker_filtered = self
            .proc_picker
            .iter()
            .filter(|p| q.is_empty() || p.qualified_name.to_lowercase().contains(&q))
            .map(|p| p.qualified_name.clone())
            .take(20)
            .collect();
        self.proc_picker_selected = if self.proc_picker_filtered.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    fn picker_move(&mut self, delta: isize) {
        if self.proc_picker_filtered.is_empty() {
            self.proc_picker_selected = None;
            return;
        }
        let len = self.proc_picker_filtered.len() as isize;
        let cur = self.proc_picker_selected.unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len - 1) as usize;
        self.proc_picker_selected = Some(next);
    }

    fn command_history_prev(&mut self) {
        let len = self.command_history.len();
        if len == 0 {
            return;
        }
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let next = match prompt.history_pos {
            None => len - 1,
            Some(0) => 0,
            Some(n) => n - 1,
        };
        let entry = self.command_history.get(next).cloned().unwrap_or_default();
        prompt.input.buffer = entry;
        prompt.input.cursor = prompt.input.buffer.len();
        prompt.history_pos = Some(next);
    }

    fn command_history_next(&mut self) {
        let len = self.command_history.len();
        if len == 0 {
            return;
        }
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let next = match prompt.history_pos {
            None => return, // already past-the-end
            Some(n) if n + 1 >= len => {
                prompt.input.buffer.clear();
                prompt.input.cursor = 0;
                prompt.history_pos = None;
                return;
            }
            Some(n) => n + 1,
        };
        let entry = self.command_history.get(next).cloned().unwrap_or_default();
        prompt.input.buffer = entry;
        prompt.input.cursor = prompt.input.buffer.len();
        prompt.history_pos = Some(next);
    }

    fn complete_dbg_command(&mut self) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let buf = prompt.input.buffer.clone();
        let typed = buf.split_whitespace().next().unwrap_or("");
        let matches: Vec<&str> = DBG_COMMAND_NAMES
            .iter()
            .copied()
            .filter(|name| name.starts_with(typed))
            .collect();
        match matches.len() {
            0 => {}
            1 => {
                let completion = format!("{} ", matches[0]);
                if buf.starts_with(typed) && buf.len() <= completion.len() {
                    prompt.input.buffer = completion.clone();
                    prompt.input.cursor = completion.len();
                }
            }
            _ => {
                self.set_status(format!("matches: {}", matches.join(" ")));
            }
        }
    }

    fn reset_to_user_script(&mut self) {
        self.cache.showing_block_id = None;
        self.cache.showing_qname = None;
        self.cache.origin = self.cache.user_script_origin;
        self.set_status("source view → user script");
    }

    fn cycle_theme(&mut self) {
        self.theme.next_palette();
        self.theme_name = self.theme.name().to_string();
        // Re-highlight the user-script + cached proc bodies so colours match.
        let user_raw: Vec<String> = self
            .cache
            .user_script
            .iter()
            .map(|l| l.raw.clone())
            .collect();
        self.cache.user_script = highlight_lines(&user_raw, &self.theme);
        let qnames: Vec<String> = self.cache.proc_bodies.keys().cloned().collect();
        for qname in qnames {
            if let Some(body) = self.cache.proc_bodies.get(&qname) {
                let raw: Vec<String> = body.iter().map(|l| l.raw.clone()).collect();
                let rehilite = highlight_lines(&raw, &self.theme);
                self.cache.proc_bodies.insert(qname, rehilite);
            }
        }
        self.set_status(format!("theme → {}", self.theme_name));
    }

    fn repeat_last_action(&mut self) {
        let Some(action) = self.last_action.clone() else {
            self.set_status("no last action to repeat");
            return;
        };
        match action {
            RepeatableAction::Continue => self.do_continue(),
            RepeatableAction::StepInto => self.do_step_into(),
            RepeatableAction::StepOver => self.do_step_over(),
            RepeatableAction::StepOut => self.do_step_out(),
            RepeatableAction::Pause => self.do_pause(),
            RepeatableAction::ToggleBp => self.toggle_breakpoint_at_cursor(),
            RepeatableAction::Refresh => {
                self.refresh_state();
                self.set_status("state refreshed");
            }
            RepeatableAction::Eval(script) => eval_in_current_frame(self, &script),
            RepeatableAction::DbgCommand(line) => execute_dbg_command(self, &line),
        }
    }

    fn record_action(&mut self, action: RepeatableAction) {
        self.last_action = Some(action);
    }

    fn do_continue(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "continue unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        if !self.require_capability(BackendOperation::Continue, "continue") {
            return;
        }
        self.record_action(RepeatableAction::Continue);
        self.run_backend_action("continue", |backend| backend.continue_execution());
    }
    fn do_step_into(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "step into unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        let (operation, step) = if self
            .capabilities
            .support(BackendOperation::StepInto)
            .is_supported()
        {
            (BackendOperation::StepInto, DebugStepKind::Into)
        } else {
            (
                BackendOperation::StepInstruction,
                DebugStepKind::InstructionInto,
            )
        };
        if !self.require_capability(operation, "step into") {
            return;
        }
        self.record_action(RepeatableAction::StepInto);
        self.run_backend_action("step into", |backend| backend.step(step));
    }
    fn do_step_over(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "step over unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        let (operation, step) = if self
            .capabilities
            .support(BackendOperation::StepOver)
            .is_supported()
        {
            (BackendOperation::StepOver, DebugStepKind::Over)
        } else {
            (
                BackendOperation::StepOverInstruction,
                DebugStepKind::InstructionOver,
            )
        };
        if !self.require_capability(operation, "step over") {
            return;
        }
        self.record_action(RepeatableAction::StepOver);
        self.run_backend_action("step over", |backend| backend.step(step));
    }
    fn do_step_out(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "step out unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        let (operation, step) = if self
            .capabilities
            .support(BackendOperation::StepOut)
            .is_supported()
        {
            (BackendOperation::StepOut, DebugStepKind::Out)
        } else {
            (
                BackendOperation::StepOutInstruction,
                DebugStepKind::InstructionOut,
            )
        };
        if !self.require_capability(operation, "step out") {
            return;
        }
        self.record_action(RepeatableAction::StepOut);
        self.run_backend_action("step out", |backend| backend.step(step));
    }
    fn do_pause(&mut self) {
        if self.app_state != AppState::Running {
            self.set_status(format!(
                "pause unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        if !self.require_capability(BackendOperation::Pause, "pause") {
            return;
        }
        self.record_action(RepeatableAction::Pause);
        self.run_backend_action("pause", |backend| backend.pause_execution());
    }

    fn run_backend_action(
        &mut self,
        label: &str,
        action: impl FnOnce(&mut dyn DebugBackend) -> Result<(), super::backend::BackendError>,
    ) {
        let result = match self.backend.as_mut() {
            Some(backend) => action(&mut **backend),
            None => Err(super::backend::BackendError::Message(
                "debug backend is closed".into(),
            )),
        };
        match result {
            Ok(()) => {
                // Change markers describe the transition caused by this
                // execution action. Clear the preceding transition now; once
                // a new value is observed, keep its marker durable across any
                // identical refreshes until the next execution action.
                for watch in &mut self.watches {
                    watch.changed = false;
                }
                self.set_status(label);
                self.push_event(EventLine::trace(label));
                let processed_events = self.tick();
                // A pause event refreshes the complete snapshot in `tick`.
                // Refreshing again here erases one-frame change markers for
                // watches and locals before the terminal ever renders them.
                if !processed_events && self.app_state == AppState::Stopped {
                    self.refresh_state();
                }
            }
            Err(e) => {
                self.set_status(format!("{label} failed: {e}"));
                self.push_event(EventLine::error(format!("{label}: {e}")));
            }
        }
    }

    fn do_restart(&mut self) {
        if !self.require_capability(BackendOperation::Restart, "restart") {
            return;
        }
        let saved_watches: Vec<(String, String)> = self
            .watches
            .iter()
            .map(|watch| (watch.name.clone(), watch.expr.clone()))
            .collect();
        self.clear_execution_state();
        self.cache.proc_bodies.clear();
        self.cache.proc_placements.clear();
        let result = self.backend.as_mut().unwrap().restart_target(true);
        match result {
            Ok(()) => {
                let backend_managed = self.backend.as_mut().unwrap().watch_entries().is_ok();
                if backend_managed {
                    for (name, expression) in &saved_watches {
                        if let Err(error) =
                            self.backend.as_mut().unwrap().add_watch(name, expression)
                        {
                            self.push_event(EventLine::error(format!(
                                "restart watch restore failed for {name}: {error}"
                            )));
                        }
                    }
                }
                self.push_event(EventLine::pause("↻ restarted"));
                self.tick();
                self.refresh_state();
            }
            Err(error) => {
                self.set_status(format!("restart failed: {error}"));
                self.push_event(EventLine::error(format!("restart: {error}")));
            }
        }
    }

    fn do_ignore_error(&mut self) {
        let result = self
            .backend
            .as_mut()
            .ok_or_else(|| "debug backend is closed".to_owned())
            .and_then(|backend| {
                backend
                    .ignore_current_error()
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(_) => self.set_status("next exception will not pause"),
            Err(e) => self.set_status(format!("ignore_error failed: {e}")),
        }
    }

    fn toggle_coverage(&mut self) {
        if !self.require_capability(BackendOperation::Coverage, "coverage") {
            return;
        }
        let new = !self.coverage_enabled;
        let result = self.backend.as_mut().unwrap().set_coverage_enabled(new);
        match result {
            Ok(_) => {
                self.coverage_enabled = new;
                self.set_status(if new {
                    "coverage tracking ON"
                } else {
                    "coverage tracking OFF"
                });
                if new {
                    self.coverage = self
                        .backend
                        .as_mut()
                        .unwrap()
                        .coverage_snapshot()
                        .unwrap_or_default();
                } else {
                    self.coverage.clear();
                }
            }
            Err(e) => self.set_status(format!("coverage_enable: {e}")),
        }
    }

    fn do_list_slaves(&mut self) {
        let list = self
            .backend
            .as_mut()
            .unwrap()
            .slave_interpreters()
            .unwrap_or_default();
        if list.is_empty() {
            self.set_status("no slave interps registered");
        } else {
            self.set_status(format!("slaves: {}", list.join(", ")));
        }
    }

    fn toggle_breakpoint_at_cursor(&mut self) {
        self.record_action(RepeatableAction::ToggleBp);
        let line_no = (self.ui.source_cursor + 1) as u32;
        if self
            .capabilities
            .support(BackendOperation::DirectBytecode)
            .is_supported()
            && let Some(location) = self.cache.line_to_code.get(&line_no).copied()
        {
            self.toggle_instruction_breakpoint(location);
            return;
        }
        if !self.require_capability(BackendOperation::SourceBreakpoints, "source breakpoint") {
            return;
        }
        let command = self
            .current_source_lines()
            .get(self.ui.source_cursor)
            .and_then(|line| {
                let index = self
                    .ui
                    .source_command_cursor
                    .min(line.commands.len().saturating_sub(1));
                line.commands.get(index)
            })
            .cloned();
        if let Some(command) = command {
            self.toggle_command_breakpoint(line_no, &command);
            return;
        }
        let displayed_is_file = self.displayed_source_unit().is_none_or(|unit| {
            matches!(unit.registration.identity, DebugSourceUnitIdentity::File(_))
        });
        if self.cache.showing_qname.is_none() && displayed_is_file {
            self.toggle_file_breakpoint_at_line(line_no);
            return;
        }
        if self.cache.showing_qname.is_none()
            && self.displayed_source_unit().is_some_and(|unit| {
                matches!(
                    unit.registration.identity,
                    DebugSourceUnitIdentity::Anonymous { .. }
                )
            })
        {
            self.set_status(
                "eval source unit is session-only; durable breakpoint rebinding is unavailable",
            );
            return;
        }
        let qname = self
            .focused_proc
            .as_ref()
            .map(|p| p.qname.clone())
            .filter(|q| !q.is_empty())
            .or_else(|| self.cache.showing_qname.clone())
            .unwrap_or_default();
        if qname.is_empty() {
            self.set_status(
                "no proc context — pause inside a proc, or use B to add a breakpoint by qname",
            );
            return;
        }
        let existing = self
            .breakpoints
            .iter()
            .find(|bp| bp.qname == qname && bp.offset == u64::from(line_no))
            .cloned();
        let result = match existing {
            Some(bp) => self
                .backend
                .as_mut()
                .unwrap()
                .remove_breakpoint(&DebugBreakpointId(bp.id.clone()))
                .map(|_| format!("removed {} @ {qname}:{line_no}", bp.id)),
            None => self
                .backend
                .as_mut()
                .unwrap()
                .add_breakpoint(DebugBreakpointSpec::SourceCommand {
                    block_id: format!("proc:{qname}"),
                    source_path: None,
                    qualified_proc: Some(qname.clone()),
                    span: SourceCommandSpan::new(u64::from(line_no), u64::from(line_no) + 1)
                        .expect("line span"),
                    condition: None,
                })
                .map(|bp| format!("breakpoint {} @ {qname}:{line_no}", bp.id.0)),
        };
        let (status, severity_error) = match result {
            Ok(status) => (status, false),
            Err(error) => (format!("breakpoint failed: {error}"), true),
        };
        if severity_error {
            self.push_event(EventLine::error(status.clone()));
        } else {
            self.push_event(EventLine::info(status.clone()));
        }
        self.set_status(status);
        self.reload_breakpoints();
    }

    fn move_source_command(&mut self, delta: isize) {
        let Some(line) = self.current_source_lines().get(self.ui.source_cursor) else {
            self.set_status("no source line selected");
            return;
        };
        if line.commands.is_empty() {
            self.ui.source_command_cursor = 0;
            self.set_status(format!(
                "line {} has no executable command",
                self.ui.source_cursor + 1
            ));
            return;
        }
        let count = line.commands.len();
        let current = self.ui.source_command_cursor.min(count - 1);
        self.ui.source_command_cursor =
            (current as isize + delta).rem_euclid(count as isize) as usize;
        let selected = self.current_source_lines()[self.ui.source_cursor].commands
            [self.ui.source_command_cursor]
            .clone();
        self.set_status(format!(
            "command {}/{} on line {}: {} (bytes {}..{})",
            self.ui.source_command_cursor + 1,
            count,
            self.ui.source_cursor + 1,
            selected.label,
            selected.offset,
            selected.offset + selected.len
        ));
    }

    fn toggle_command_breakpoint(&mut self, line_no: u32, command: &SourceCommand) {
        if !self.require_capability(BackendOperation::SourceBreakpoints, "command breakpoint") {
            return;
        }

        let target_qname = command
            .qname
            .clone()
            .or_else(|| {
                self.displayed_source_unit()
                    .and_then(|unit| unit.registration.qualified_proc.clone())
            })
            .or_else(|| self.cache.showing_qname.clone());
        let preinstrument_body = command.qname.as_ref().and_then(|qname| {
            let mut source = self
                .current_source_lines()
                .iter()
                .map(|line| line.raw.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            source.push('\n');
            tcld_syntax::proc_body_spans(&source)
                .ok()?
                .into_iter()
                .find(|body| &body.qname == qname)
                .and_then(|body| {
                    source
                        .get(body.start_byte..body.end_byte)
                        .map(str::to_string)
                })
        });
        if self.cache.showing_qname.is_some() && target_qname.is_none() {
            self.set_status(
                "no proc context — pause inside a proc, or use B to add a breakpoint by qname",
            );
            return;
        }

        let target_file = target_qname
            .is_none()
            .then(|| self.displayed_file_path())
            .flatten();
        if target_qname.is_none() && target_file.is_none() {
            self.set_status(
                "eval source unit is session-only; durable source breakpoints are unavailable",
            );
            self.push_event(EventLine::warn(
                "breakpoint not armed: anonymous eval identity cannot be rebound safely",
            ));
            return;
        }
        let target_path = target_file
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let existing = self
            .breakpoints
            .iter()
            .find(|breakpoint| {
                breakpoint.kind == tcld_tui::debug::BreakpointKindLabel::Line
                    && breakpoint.offset == command.target_offset
                    && if let Some(qname) = target_qname.as_deref() {
                        breakpoint.qname == qname
                    } else {
                        breakpoint.source_path.as_deref() == Some(target_path.as_str())
                    }
            })
            .cloned();
        if existing.is_none()
            && let (Some(qname), Some(body)) =
                (command.qname.as_deref(), preinstrument_body.as_deref())
            && let Err(error) = self
                .backend
                .as_mut()
                .unwrap()
                .preinstrument_procedure(qname, body)
        {
            self.finish_command_breakpoint_toggle(
                format!("breakpoint pre-instrument failed: {error}"),
                true,
            );
            return;
        }

        let (status, failed) = {
            let backend = self.backend.as_mut().unwrap();
            match existing {
                Some(breakpoint) => {
                    match backend.remove_breakpoint(&DebugBreakpointId(breakpoint.id.clone())) {
                        Ok(_) => (
                            format!(
                                "Breakpoint {} removed from line {line_no}, {}",
                                breakpoint.id, command.label
                            ),
                            false,
                        ),
                        Err(error) => (format!("remove failed: {error}"), true),
                    }
                }
                None => {
                    let result = backend.add_breakpoint(DebugBreakpointSpec::SourceCommand {
                        block_id: self.cache.showing_block_id.clone().unwrap_or_else(|| {
                            target_qname.as_ref().map_or_else(
                                || format!("file:{target_path}"),
                                |qname| format!("proc:{qname}"),
                            )
                        }),
                        source_path: target_file.clone(),
                        qualified_proc: target_qname.clone(),
                        span: SourceCommandSpan::new(
                            command.target_offset,
                            command.target_offset.saturating_add(command.len.max(1)),
                        )
                        .expect("source command span"),
                        condition: None,
                    });
                    match result {
                        Ok(breakpoint) => (
                            format!(
                                "Breakpoint {} set at line {line_no}, {}{}",
                                breakpoint.id.0,
                                command.label,
                                target_qname
                                    .as_deref()
                                    .map(|qname| format!(" in {qname}"))
                                    .unwrap_or_default()
                            ),
                            false,
                        ),
                        Err(error) => (format!("breakpoint add failed: {error}"), true),
                    }
                }
            }
        };
        self.reload_breakpoints();
        self.push_event(if failed {
            EventLine::error(status.clone())
        } else {
            EventLine::info(status.clone())
        });
        self.set_status(status);
    }

    fn finish_command_breakpoint_toggle(&mut self, status: String, failed: bool) {
        self.reload_breakpoints();
        self.push_event(if failed {
            EventLine::error(status.clone())
        } else {
            EventLine::info(status.clone())
        });
        self.set_status(status);
    }

    fn toggle_file_breakpoint_at_line(&mut self, line_no: u32) {
        if !self.require_capability(BackendOperation::SourceBreakpoints, "file breakpoint") {
            return;
        }
        let Some(target_file) = self.displayed_file_path() else {
            self.set_status("current source unit is not a durable file breakpoint target");
            return;
        };
        let target_path = target_file.to_string_lossy().into_owned();
        let existing = self.breakpoints.iter().find(|breakpoint| {
            breakpoint.source_path.as_deref() == Some(target_path.as_str())
                && breakpoint.line == Some(line_no)
        });
        let (status, failed) = {
            let backend = self.backend.as_mut().unwrap();
            match existing {
                Some(breakpoint) => {
                    match backend.remove_breakpoint(&DebugBreakpointId(breakpoint.id.clone())) {
                        Ok(_) => (
                            format!(
                                "Breakpoint {} removed from {}:{line_no}",
                                breakpoint.id,
                                target_file
                                    .file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_default()
                            ),
                            false,
                        ),
                        Err(error) => (format!("remove failed: {error}"), true),
                    }
                }
                None => match backend.add_breakpoint(DebugBreakpointSpec::SourceLine {
                    path: target_file.clone(),
                    line: line_no,
                    condition: None,
                }) {
                    Ok(breakpoint) => (
                        format!(
                            "Breakpoint {} set at {}:{line_no}",
                            breakpoint.id.0,
                            target_file
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        ),
                        false,
                    ),
                    Err(error) => (format!("breakpoint add failed: {error}"), true),
                },
            }
        };
        self.reload_breakpoints();
        self.push_event(if failed {
            EventLine::error(status.clone())
        } else {
            EventLine::info(status.clone())
        });
        self.set_status(status);
    }

    fn toggle_selected_bp_enabled(&mut self) {
        let Some(idx) = self.ui.selections.breakpoints.selected() else {
            return;
        };
        let Some(bp) = self.breakpoints.get(idx).cloned() else {
            return;
        };
        let target = !bp.enabled;
        let result = self
            .backend
            .as_mut()
            .unwrap()
            .set_breakpoint_enabled(&DebugBreakpointId(bp.id.clone()), target);
        match result {
            Ok(_) => self.set_status(format!(
                "Breakpoint {} {}",
                bp.id,
                if target { "enabled" } else { "disabled" }
            )),
            Err(e) => {
                let message = format!("toggle failed: {e}");
                self.push_event(EventLine::error(message.clone()));
                self.set_status(message);
            }
        }
        self.reload_breakpoints();
    }

    fn delete_selected_bp(&mut self) {
        let Some(idx) = self.ui.selections.breakpoints.selected() else {
            return;
        };
        let Some(bp) = self.breakpoints.get(idx).cloned() else {
            return;
        };
        let (status, failed) = {
            let backend = self.backend.as_mut().unwrap();
            match backend.remove_breakpoint(&DebugBreakpointId(bp.id.clone())) {
                Ok(_) => (format!("Breakpoint {} deleted", bp.id), false),
                Err(e) => (format!("delete failed: {e}"), true),
            }
        };
        if failed {
            self.push_event(EventLine::error(status.clone()));
        }
        self.set_status(status);
        self.reload_breakpoints();
        if self.breakpoints.is_empty() {
            self.ui.selections.breakpoints.select(None);
        } else if idx >= self.breakpoints.len() {
            self.ui
                .selections
                .breakpoints
                .select(Some(self.breakpoints.len() - 1));
        }
    }

    fn delete_selected_watch(&mut self) {
        let Some(idx) = self.ui.selections.watches.selected() else {
            return;
        };
        let Some(w) = self.watches.get(idx).cloned() else {
            return;
        };
        let backend_managed = self.backend.as_mut().unwrap().watch_entries().is_ok();
        let status = if backend_managed {
            match self.backend.as_mut().unwrap().remove_watch(&w.id) {
                Ok(_) => format!("Watch {} removed", w.name),
                Err(e) => format!("watch remove failed: {e}"),
            }
        } else {
            format!("Watch {} removed", w.name)
        };
        self.set_status(status);
        if backend_managed {
            self.watches = self
                .backend
                .as_mut()
                .unwrap()
                .watch_entries()
                .unwrap_or_default();
        } else {
            self.watches.remove(idx);
        }
        if self.watches.is_empty() {
            self.ui.selections.watches.select(None);
        } else if idx >= self.watches.len() {
            self.ui
                .selections
                .watches
                .select(Some(self.watches.len() - 1));
        }
    }

    fn do_export_breakpoints(&mut self) {
        let serialized = match self.backend.as_mut().unwrap().export_breakpoint_state() {
            Ok(s) => s,
            Err(e) => {
                let message = format!("export failed: {e}");
                self.push_event(EventLine::error(message.clone()));
                self.set_status(message);
                return;
            }
        };
        let persisted = self.serialize_persisted_state(serialized);
        let path = persist_path(&self.user_script);
        match std::fs::write(&path, &persisted) {
            Ok(_) => self.set_status(format!("breakpoints exported → {}", path.display())),
            Err(e) => {
                let message = format!("write {} failed: {e}", path.display());
                self.push_event(EventLine::error(message.clone()));
                self.set_status(message);
            }
        }
    }

    fn do_import_breakpoints(&mut self) {
        let path = persist_path(&self.user_script);
        let serialized = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                let message = format!("read {} failed: {e}", path.display());
                self.push_event(EventLine::error(message.clone()));
                self.set_status(message);
                return;
            }
        };
        let result = self.import_persisted_state(&serialized);
        match result {
            Ok(_) => self.set_status(format!("breakpoints imported from {}", path.display())),
            Err(e) => {
                let message = format!("import failed: {e}");
                self.push_event(EventLine::error(message.clone()));
                self.set_status(message);
            }
        }
        self.reload_breakpoints();
    }

    fn try_autoload_persisted_breakpoints(&mut self) {
        let path = persist_path(&self.user_script);
        if path.exists()
            && self.backend.is_some()
            && let Ok(serialized) = std::fs::read_to_string(&path)
            && self.import_persisted_state(&serialized).is_ok()
        {
            self.push_event(EventLine::info(format!(
                "auto-imported breakpoints from {}",
                path.display()
            )));
            self.reload_breakpoints();
        }
    }

    fn maybe_export_breakpoints_on_quit(&mut self) {
        // Best-effort: if we still have a session and any breakpoints,
        // round-trip them through `bp_export` and write next to the
        // user script so the next launch auto-imports.
        if self.backend.is_none() || (self.breakpoints.is_empty() && self.watches.is_empty()) {
            return;
        }
        let serialized = match self.backend.as_mut().unwrap().export_breakpoint_state() {
            Ok(s) => s,
            Err(_) => return,
        };
        if serialized.trim().is_empty() && self.watches.is_empty() {
            return;
        }
        let serialized = self.serialize_persisted_state(serialized);
        let path = persist_path(&self.user_script);
        let _ = std::fs::write(&path, serialized);
    }

    fn serialize_persisted_state(&self, breakpoints: String) -> String {
        let state = PersistedDebugState {
            version: 1,
            breakpoints,
            watches: self
                .watches
                .iter()
                .map(|watch| PersistedWatch {
                    name: watch.name.clone(),
                    expression: watch.expr.clone(),
                })
                .collect(),
        };
        serde_json::to_string_pretty(&state).unwrap_or(state.breakpoints)
    }

    fn import_persisted_state(&mut self, serialized: &str) -> Result<(), String> {
        let parsed = serde_json::from_str::<PersistedDebugState>(serialized).ok();
        if let Some(state) = &parsed
            && state.version != 1
        {
            return Err(format!(
                "unsupported persisted debugger state version {}",
                state.version
            ));
        }
        let breakpoints = parsed
            .as_ref()
            .map(|state| state.breakpoints.as_str())
            .unwrap_or(serialized);
        let backend = self
            .backend
            .as_mut()
            .ok_or_else(|| "debug backend is closed".to_string())?;
        backend
            .import_breakpoint_state(breakpoints)
            .map_err(|error| error.to_string())?;
        if let Some(state) = parsed {
            let existing = backend.watch_entries().unwrap_or_default();
            for watch in existing {
                let _ = backend.remove_watch(&watch.id);
            }
            for watch in state.watches {
                backend
                    .add_watch(&watch.name, &watch.expression)
                    .map_err(|error| error.to_string())?;
            }
            self.watches = backend.watch_entries().unwrap_or_default();
        }
        Ok(())
    }

    fn maybe_save_transcript(&mut self) -> Option<PathBuf> {
        if self.events.is_empty() {
            return None;
        }
        let stem = self
            .user_script
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session");
        // Allocate a uniquely-named, atomically-created file under the
        // platform temp dir. `keep()` opts out of the auto-delete
        // behaviour so the user can read the transcript after exit.
        let mut named = match tempfile::Builder::new()
            .prefix(&format!("{TRANSCRIPT_BASENAME}-{stem}-"))
            .suffix(".log")
            .tempfile()
        {
            Ok(f) => f,
            Err(_) => return None,
        };
        let mut buf = String::new();
        for ev in &self.events {
            let secs = ev
                .when
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            buf.push_str(&format!("[{secs}] [{:?}] {}\n", ev.severity, ev.text));
        }
        use std::io::Write;
        if named.write_all(buf.as_bytes()).is_err() {
            return None;
        }
        let (_, path) = match named.keep() {
            Ok((file, path)) => (file, path),
            Err(_) => return None,
        };
        self.transcript_path = Some(path.clone());
        Some(path)
    }
}

struct DebugSurface {
    app: DebugApp,
    startup: DebugStartup,
}

#[derive(Clone, Copy)]
enum DebugStartup {
    Connect,
    StartTarget,
    Running,
}

impl Drop for DebugSurface {
    fn drop(&mut self) {
        self.app.maybe_export_breakpoints_on_quit();
        if let Some(path) = self.app.maybe_save_transcript() {
            eprintln!("transcript saved: {}", path.display());
        }
        if let Some(backend) = self.app.backend.take() {
            backend.close();
        }
    }
}

impl TerminalApplication for DebugSurface {
    type Exit = ();

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        self.app.draw(frame, area);
    }

    fn handle(&mut self, event: RuntimeEvent) -> RuntimeAction<Self::Exit> {
        let changed = match event {
            RuntimeEvent::Tick(_) if matches!(self.startup, DebugStartup::Connect) => {
                self.startup = if self.app.connect_debugger() {
                    DebugStartup::StartTarget
                } else {
                    DebugStartup::Running
                };
                true
            }
            RuntimeEvent::Tick(_) if matches!(self.startup, DebugStartup::StartTarget) => {
                self.startup = DebugStartup::Running;
                self.app.start_debug_target();
                true
            }
            RuntimeEvent::Tick(_) => self.app.tick(),
            RuntimeEvent::Terminal(Event::Key(key)) => {
                self.app.handle_key(key);
                true
            }
            RuntimeEvent::Terminal(Event::Mouse(mouse)) => {
                self.app.handle_mouse(mouse);
                true
            }
            RuntimeEvent::Terminal(Event::Paste(text)) => {
                let Some(prompt) = self.app.prompt.as_mut() else {
                    return RuntimeAction::Ignored;
                };
                let cursor = prompt.input.cursor.min(prompt.input.buffer.len());
                prompt.input.buffer.insert_str(cursor, &text);
                prompt.input.cursor = cursor + text.len();
                true
            }
            RuntimeEvent::Terminal(Event::Resize(..)) => true,
            _ => false,
        };

        if self.app.quitting {
            RuntimeAction::Exit(())
        } else if changed {
            RuntimeAction::Redraw
        } else {
            RuntimeAction::Ignored
        }
    }
}

// ── Prompt dispatch ────────────────────────────────────────────────────────

fn handle_prompt_submit(app: &mut DebugApp, prompt: ActivePrompt, value: String) {
    let trimmed = value.trim();
    let kind = prompt.kind;
    if matches!(kind, PromptKind::EventFilter) {
        app.event_filter = trimmed.to_string();
        app.set_status(if trimmed.is_empty() {
            "event filter cleared".to_string()
        } else {
            format!("event filter: /{trimmed}")
        });
        return;
    }
    if matches!(kind, PromptKind::SearchSource) {
        // Submitting confirms the current match; cancelling clears.
        app.update_source_search(trimmed);
        if app.search_match_lines.is_empty() {
            app.set_status(format!("no source match for /{trimmed}"));
        } else {
            app.set_status(format!(
                "{} match{} for /{}",
                app.search_match_lines.len(),
                if app.search_match_lines.len() == 1 {
                    ""
                } else {
                    "es"
                },
                trimmed
            ));
        }
        return;
    }
    if trimmed.is_empty() {
        app.set_status("(prompt cancelled)");
        return;
    }
    match kind {
        PromptKind::Command => execute_dbg_command(app, trimmed),
        PromptKind::GotoProc => goto_proc(app, trimmed),
        PromptKind::AddWatch => add_watch_from_prompt(app, trimmed),
        PromptKind::AddWatchpoint => add_watchpoint_from_prompt(app, trimmed),
        PromptKind::EvalExpr => {
            app.last_action = Some(RepeatableAction::Eval(trimmed.to_string()));
            eval_in_current_frame(app, trimmed);
        }
        PromptKind::AddBreakpoint => add_bp_from_prompt(app, trimmed),
        PromptKind::EditCondition => edit_condition(app, prompt.bp_target, trimmed),
        PromptKind::SetIgnore => set_ignore(app, prompt.bp_target, trimmed),
        PromptKind::ProcPicker => proc_picker_select(app, trimmed),
        PromptKind::PreInstrument => pre_instrument(app, prompt.body_buffer, trimmed),
        PromptKind::EventFilter | PromptKind::SearchSource => {
            unreachable!()
        }
    }
}

fn execute_dbg_command(app: &mut DebugApp, line: &str) {
    // Append to history (deduplicating consecutive entries) and
    // record as repeatable for `.`.
    let owned = line.to_string();
    if app.command_history.back() != Some(&owned) {
        if app.command_history.len() >= 64 {
            app.command_history.pop_front();
        }
        app.command_history.push_back(owned.clone());
    }
    app.last_action = Some(RepeatableAction::DbgCommand(owned));

    let mut tokens = line.split_whitespace();
    let Some(sub) = tokens.next().map(str::to_string) else {
        return;
    };
    let rest: Vec<String> = tokens.map(str::to_string).collect();
    if sub == "quit" {
        app.quitting = true;
        return;
    }
    if sub == "help" {
        let width = DBG_COMMANDS
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        let mut body = String::from(
            "Type these after `:`. Tab completes a command name; ↑/↓ recall earlier ones.\n\n",
        );
        for (name, description) in DBG_COMMANDS {
            body.push_str(&format!("{name:<width$}  {description}\n"));
        }
        app.ui.popup = Some(ValuePopup {
            title: "Debugger commands".to_string(),
            body,
            scroll: 0,
        });
        return;
    }
    // Pure-client commands — no nub round-trip required.
    if sub == "goto" || sub.parse::<usize>().is_ok() {
        let target_str = if sub == "goto" {
            rest.first().cloned().unwrap_or_default()
        } else {
            sub.clone()
        };
        match target_str.parse::<usize>() {
            Ok(n) if n > 0 => {
                let max = app.current_source_lines_len().saturating_sub(1);
                app.ui.source_cursor = (n - 1).min(max);
                app.set_status(format!("goto line {n}"));
            }
            _ => app.set_status(format!("usage: goto <line>; got {target_str}")),
        }
        return;
    }
    if app.backend.is_none() {
        app.set_status("debug backend is closed");
        return;
    }

    let result: Result<String, String> = {
        let backend = app.backend.as_mut().unwrap();
        match sub.as_str() {
            "events" => Ok(if app.events.is_empty() {
                "(no events)".to_owned()
            } else {
                app.events
                    .iter()
                    .map(|event| event.text.clone())
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            "state" => backend
                .state()
                .map(|state| AppState::from(state).label().to_owned())
                .map_err(|error| error.to_string()),
            "stack" => backend
                .stack_frames()
                .map(|frames| format!("{frames:#?}"))
                .map_err(|error| error.to_string()),
            "vars" => {
                let lvl: i64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                backend
                    .variables_for_frame(lvl)
                    .map(|variables| format!("{variables:#?}"))
                    .map_err(|error| error.to_string())
            }
            "get" => {
                let Some(name) = rest.first().cloned() else {
                    app.set_status("usage: get <name> [level]");
                    return;
                };
                let level: i64 = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                backend
                    .get_variable(level, &name)
                    .map_err(|error| error.to_string())
            }
            "set" => {
                let Some(name) = rest.first().cloned() else {
                    app.set_status("usage: set <name> <value> [level]");
                    return;
                };
                let Some(val) = rest.get(1).cloned() else {
                    app.set_status("usage: set <name> <value> [level]");
                    return;
                };
                let level: i64 = rest.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                backend
                    .set_variable(level, &name, &val)
                    .map(|_| val)
                    .map_err(|error| error.to_string())
            }
            "eval" => {
                if rest.is_empty() {
                    app.set_status("usage: eval <script>");
                    return;
                }
                backend
                    .evaluate(0, &rest.join(" "))
                    .map(|result| result.result)
                    .map_err(|error| error.to_string())
            }
            "procs" => backend
                .procedures()
                .map(|procedures| {
                    procedures
                        .into_iter()
                        .map(|procedure| procedure.qualified_name)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .map_err(|error| error.to_string()),
            "ploc" => {
                let Some(qname) = rest.first().cloned() else {
                    app.set_status("usage: ploc <qname>");
                    return;
                };
                backend
                    .procedure_location(&qname)
                    .map(|location| format!("{location:?}"))
                    .map_err(|error| error.to_string())
            }
            "pbody" => {
                let Some(qname) = rest.first().cloned() else {
                    app.set_status("usage: pbody <qname>");
                    return;
                };
                backend
                    .procedure_source(&qname)
                    .map(|source| source.text)
                    .map_err(|error| error.to_string())
            }
            "cont" | "continue" => backend
                .continue_execution()
                .map(|_| "continued".into())
                .map_err(|error| error.to_string()),
            "step" => backend
                .step(
                    if backend
                        .capabilities()
                        .support(BackendOperation::StepInto)
                        .is_supported()
                    {
                        DebugStepKind::Into
                    } else {
                        DebugStepKind::InstructionInto
                    },
                )
                .map(|_| "stepped".into())
                .map_err(|error| error.to_string()),
            "over" => backend
                .step(
                    if backend
                        .capabilities()
                        .support(BackendOperation::StepOver)
                        .is_supported()
                    {
                        DebugStepKind::Over
                    } else {
                        DebugStepKind::InstructionOver
                    },
                )
                .map(|_| "stepped over".into())
                .map_err(|error| error.to_string()),
            "out" => backend
                .step(
                    if backend
                        .capabilities()
                        .support(BackendOperation::StepOut)
                        .is_supported()
                    {
                        DebugStepKind::Out
                    } else {
                        DebugStepKind::InstructionOut
                    },
                )
                .map(|_| "stepped out".into())
                .map_err(|error| error.to_string()),
            "pause" => backend
                .pause_execution()
                .map(|_| "pause requested".into())
                .map_err(|error| error.to_string()),
            "coverage" => backend
                .coverage_snapshot()
                .map(|coverage| format!("{coverage:#?}"))
                .map_err(|error| error.to_string()),
            other => Err(format!("unknown command: {other}")),
        }
    };

    match result {
        Ok(text) => {
            app.set_status(format!(":{sub} → {}", trim_status(&text)));
            app.push_event_text(
                tcld_tui::debug::EventSeverity::Info,
                &format!(":{sub} {text}"),
            );
            app.refresh_state();
        }
        Err(e) => app.set_status(format!(":{sub} failed: {e}")),
    }
}

fn goto_proc(app: &mut DebugApp, qname: &str) {
    if !app.require_capability(BackendOperation::ProcedureList, "go to procedure") {
        return;
    }
    let result = app.backend.as_mut().unwrap().procedure_location(qname);
    match result {
        Ok(Some(off)) => {
            // Try to fetch the body so the source pane shows the actual proc.
            let source = app.backend.as_mut().unwrap().procedure_source(qname).ok();
            if let Some(source) = source {
                let raw: Vec<String> = source.text.lines().map(str::to_string).collect();
                let highlighted = highlight_lines(&raw, &app.theme);
                let _ = raw;
                app.cache
                    .proc_bodies
                    .insert(qname.to_string(), highlighted.clone());
                app.cache.showing_block_id = Some(source.source.block_id.clone());
                app.cache.source_units.insert(
                    source.source.block_id.clone(),
                    CachedSourceUnit {
                        registration: source.source,
                        lines: highlighted,
                    },
                );
                app.cache.showing_qname = Some(qname.to_string());
                app.cache.origin = SourceOrigin::ProcBody;
            }
            let target = off.saturating_sub(1) as usize;
            let max = app.current_source_lines().len().saturating_sub(1);
            app.ui.source_cursor = target.min(max);
            app.set_status(format!("jumped to {qname} (first_offset={off})"));
        }
        Ok(None) => app.set_status(format!("no first_offset reported for {qname}")),
        Err(e) => app.set_status(format!("proc_location: {e}")),
    }
}

fn proc_picker_select(app: &mut DebugApp, query: &str) {
    let q = query.to_lowercase();
    if let Some(entry) = app
        .proc_picker
        .iter()
        .find(|p| p.qualified_name.to_lowercase().contains(&q))
    {
        let qname = entry.qualified_name.clone();
        goto_proc(app, &qname);
    } else {
        app.set_status(format!("no proc matching {query:?}"));
    }
}

fn add_watch_from_prompt(app: &mut DebugApp, raw: &str) {
    let (name, expr) = match raw.split_once(' ') {
        Some((n, e)) => (n.trim().to_string(), e.trim().to_string()),
        None => (raw.to_string(), raw.to_string()),
    };
    // Watches are `expr` expressions, where a bare word is a syntax error.
    // Someone typing a variable name means that variable.
    let expr = if is_variable_reference(&expr) {
        format!("${expr}")
    } else {
        expr
    };
    if !app.require_capability(BackendOperation::Watches, "add watch") {
        return;
    }
    let backend_managed = app.backend.as_mut().unwrap().watch_entries().is_ok();
    if backend_managed {
        match app.backend.as_mut().unwrap().add_watch(&name, &expr) {
            Ok(()) => {
                app.reload_watches();
                app.set_status(if name == expr {
                    format!("Watching {name}")
                } else {
                    format!("Watching {name} = {expr}")
                });
            }
            Err(error) => app.set_status(format!("watch add failed: {error}")),
        }
    } else {
        let id = format!("ui-watch-{}", app.watches.len() + 1);
        app.watches.push(WatchEntry {
            id,
            name: name.clone(),
            expr: expr.clone(),
            last_value: None,
            last_error: None,
            changed: false,
        });
        app.evaluate_frontend_watches();
        app.set_status(if name == expr {
            format!("Watching {name}")
        } else {
            format!("Watching {name} = {expr}")
        });
    }
}

fn add_watchpoint_from_prompt(app: &mut DebugApp, raw: &str) {
    // Format: <name> [ops]   — ops defaults to "w" (write only).
    let (name, ops) = match raw.split_once(' ') {
        Some((n, o)) => (n.trim().to_string(), o.trim().to_string()),
        None => (raw.to_string(), "w".to_string()),
    };
    if !app.require_capability(BackendOperation::SourceBreakpoints, "variable watchpoint") {
        return;
    }
    let result = app.backend.as_mut().unwrap().add_watchpoint(&name, &ops);
    match result {
        Ok(()) => app.set_status(format!("var-write watchpoint on {name}")),
        Err(e) => app.set_status(format!("wp_add: {e}")),
    }
    app.reload_breakpoints();
}

fn eval_in_current_frame(app: &mut DebugApp, script: &str) {
    if !app.require_capability(BackendOperation::Evaluate, "evaluate") {
        return;
    }
    let level = app.inspected_frame;
    let result = app.backend.as_mut().unwrap().evaluate(level, script);
    app.push_event_text(EventSeverity::Info, &format!("» {script}"));
    match result {
        Ok(result) => {
            let failed = result.completion_code != 0;
            app.set_status(if failed {
                format!(
                    "eval returned code {}: {}",
                    result.completion_code,
                    trim_status(&result.result)
                )
            } else {
                format!("= {}", trim_status(&result.result))
            });
            let mut lines = result.result.lines();
            let first = lines.next().unwrap_or("");
            app.push_event(EventLine::new(
                if failed {
                    EventSeverity::Error
                } else {
                    EventSeverity::Output
                },
                format!("← {first}"),
            ));
            for line in lines {
                app.push_event(EventLine::new(EventSeverity::Output, format!("  {line}")));
            }
            app.update_locals();
            app.reload_watches();
        }
        Err(e) => {
            app.push_event(EventLine::error(format!("✗ {e}")));
            app.set_status(format!("eval failed: {e}"));
        }
    }
}

fn add_bp_from_prompt(app: &mut DebugApp, raw: &str) {
    let mut tokens = raw.split_whitespace();
    let instruction_backend = app
        .capabilities
        .support(BackendOperation::InstructionBreakpoints)
        .is_supported();
    let qname = if instruction_backend {
        None
    } else {
        match tokens.next() {
            Some(q) => Some(q.to_string()),
            None => {
                app.set_status("usage: B <qname> <offset> [condition]");
                return;
            }
        }
    };
    let off_tok = match tokens.next() {
        Some(t) => t.to_string(),
        None => {
            app.set_status(if instruction_backend {
                "usage: B <pc> [condition]"
            } else {
                "usage: B <qname> <offset> [condition]"
            });
            return;
        }
    };
    let offset = match off_tok.parse::<u64>() {
        Ok(n) => n,
        Err(_) => {
            app.set_status(format!(
                "offset must be a non-negative integer, got {off_tok}"
            ));
            return;
        }
    };
    let condition: Option<String> = {
        let rest: Vec<&str> = tokens.collect();
        if rest.is_empty() {
            None
        } else {
            Some(rest.join(" "))
        }
    };
    let result = if instruction_backend {
        let code_id = app
            .stack
            .first()
            .and_then(|frame| frame.block.as_deref())
            .and_then(|block| block.strip_prefix("code:"))
            .and_then(|id| id.parse::<u64>().ok())
            .map(super::contract::DebugCodeId);
        match code_id {
            Some(code_id) => {
                app.backend
                    .as_mut()
                    .unwrap()
                    .add_breakpoint(DebugBreakpointSpec::Instruction {
                        location: super::contract::DebugCodeLocation {
                            code_id,
                            pc: offset,
                        },
                        condition,
                    })
            }
            None => Err(super::backend::BackendError::Message(
                "no current code object".into(),
            )),
        }
    } else {
        let qname = qname.expect("source breakpoint parser requires a procedure name");
        app.backend
            .as_mut()
            .unwrap()
            .add_breakpoint(DebugBreakpointSpec::SourceCommand {
                block_id: format!("proc:{qname}"),
                source_path: None,
                qualified_proc: Some(qname),
                span: SourceCommandSpan::new(offset, offset.saturating_add(1))
                    .expect("breakpoint span"),
                condition,
            })
    };
    let status = match result {
        Ok(breakpoint) => format!("breakpoint added: {}", breakpoint.id.0),
        Err(error) => format!("breakpoint add failed: {error}"),
    };
    app.set_status(status);
    app.reload_breakpoints();
}

fn edit_condition(app: &mut DebugApp, target_id: Option<String>, raw: &str) {
    let Some(id) = target_id else {
        app.set_status("no breakpoint target");
        return;
    };
    let Some(bp) = app.breakpoints.iter().find(|bp| bp.id == id).cloned() else {
        app.set_status("breakpoint vanished");
        return;
    };
    let condition = if raw.is_empty() { None } else { Some(raw) };
    let result = app
        .backend
        .as_mut()
        .unwrap()
        .set_breakpoint_condition(&DebugBreakpointId(bp.id), condition);
    match result {
        Ok(new_id) => app.set_status(if condition.is_some() {
            format!("condition updated on {}: {raw}", new_id.0)
        } else {
            format!("condition cleared on {}", new_id.0)
        }),
        Err(e) => {
            let message = format!("condition update failed: {e}");
            app.push_event(EventLine::error(message.clone()));
            app.set_status(message);
        }
    }
    app.reload_breakpoints();
}

fn set_ignore(app: &mut DebugApp, target_id: Option<String>, raw: &str) {
    let Some(id) = target_id else {
        return;
    };
    let count: u64 = match raw.parse() {
        Ok(n) => n,
        Err(_) => {
            app.set_status(format!(
                "ignore count must be non-negative integer, got {raw}"
            ));
            return;
        }
    };
    let result = app
        .backend
        .as_mut()
        .unwrap()
        .set_breakpoint_ignore_count(&DebugBreakpointId(id.clone()), count);
    match result {
        Ok(_) => app.set_status(format!("ignore_count({id}) = {count}")),
        Err(e) => {
            let message = format!("set_ignore failed: {e}");
            app.push_event(EventLine::error(message.clone()));
            app.set_status(message);
        }
    }
    app.reload_breakpoints();
}

fn pre_instrument(app: &mut DebugApp, _body_buffer: Option<String>, raw: &str) {
    let mut tokens = raw.splitn(2, ' ');
    let qname = match tokens.next() {
        Some(q) => q.to_string(),
        None => {
            app.set_status("usage: P <qname> <body…>");
            return;
        }
    };
    let body = match tokens.next() {
        Some(b) => b.to_string(),
        None => {
            app.set_status("body required");
            return;
        }
    };
    let result = app
        .backend
        .as_mut()
        .unwrap()
        .preinstrument_procedure(&qname, &body);
    match result {
        Ok(()) => app.set_status(format!("pre-instrumented {qname}")),
        Err(e) => app.set_status(format!("pre_instrument: {e}")),
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn backend_stack_to_tui(frame: super::contract::DebugStackFrame) -> StackFrame {
    let source = frame.location.source;
    let code = frame.location.code;
    StackFrame {
        level: frame.level,
        kind: frame.kind,
        proc_name: frame.proc_name,
        block: source
            .as_ref()
            .map(|location| location.block_id.clone())
            .or_else(|| code.map(|location| format!("code:{}", location.code_id.0))),
        block_kind: source.as_ref().map(|location| match location.block_kind {
            tcld_debug_wire::SourceBlockKind::File => tcld_tui::debug::SourceBlockKind::File,
            tcld_debug_wire::SourceBlockKind::Proc => tcld_tui::debug::SourceBlockKind::Proc,
            tcld_debug_wire::SourceBlockKind::Anonymous => {
                tcld_tui::debug::SourceBlockKind::Anonymous
            }
        }),
        source_path: source
            .as_ref()
            .and_then(|location| location.source_path.clone())
            .map(|path| path.to_string()),
        line: source.as_ref().map(|location| location.line),
        offset_start: source
            .as_ref()
            .map(|location| location.offset_start)
            .or_else(|| code.map(|location| location.pc))
            .or(frame.location.observed_pc),
        offset_end: source
            .as_ref()
            .map(|location| location.offset_end)
            .or_else(|| code.map(|location| location.pc.saturating_add(1)))
            .or_else(|| frame.location.observed_pc.map(|pc| pc.saturating_add(1))),
        location_label: None,
    }
}

fn backend_breakpoint_to_tui(breakpoint: DebugBreakpoint) -> ProcBreakpoint {
    let (
        pending,
        pending_reason,
        resolved_block,
        resolved_line,
        resolved_offset,
        resolved_len,
        resolution_generation,
    ) = match &breakpoint.resolution {
        DebugBreakpointResolution::Pending { reason } => {
            (true, Some(reason.clone()), None, None, None, None, None)
        }
        DebugBreakpointResolution::Resolved {
            generation,
            locations,
        } => {
            let source = locations
                .first()
                .and_then(|location| location.source.as_ref());
            (
                false,
                None,
                source.map(|source| source.block_id.clone()),
                source.map(|source| source.line),
                source.map(|source| source.offset_start),
                source.map(|source| source.offset_end.saturating_sub(source.offset_start)),
                *generation,
            )
        }
    };
    let (qname, source_path, line, offset, len) = match &breakpoint.spec {
        DebugBreakpointSpec::SourceLine { path, line, .. } => (
            String::new(),
            Some(path.display().to_string()),
            Some(*line),
            u64::from(*line),
            1,
        ),
        DebugBreakpointSpec::SourceCommand {
            qualified_proc,
            source_path,
            span,
            ..
        } => (
            qualified_proc.clone().unwrap_or_default(),
            source_path.as_ref().map(|path| path.display().to_string()),
            None,
            span.offset_start,
            span.byte_len(),
        ),
        DebugBreakpointSpec::Instruction { location, .. } => (
            format!("code:{}", location.code_id.0),
            Some(format!("bytecode object {}", location.code_id.0)),
            None,
            location.pc,
            1,
        ),
    };
    let condition = match &breakpoint.spec {
        DebugBreakpointSpec::SourceLine { condition, .. }
        | DebugBreakpointSpec::SourceCommand { condition, .. }
        | DebugBreakpointSpec::Instruction { condition, .. } => condition.clone(),
    };
    ProcBreakpoint {
        id: breakpoint.id.0,
        qname,
        source_path,
        line,
        pending,
        pending_reason,
        offset,
        len,
        resolved_block,
        resolved_line,
        resolved_offset,
        resolved_len,
        resolution_generation,
        enabled: breakpoint.enabled,
        hits: breakpoint.hit_count,
        ignore_count: breakpoint.ignore_count,
        condition,
        kind: tcld_tui::debug::BreakpointKindLabel::Line,
        location_label: None,
    }
}

/// Command words the `:` prompt completes.
const DBG_COMMAND_NAMES: &[&str] = &[
    "help", "goto", "events", "state", "stack", "vars", "get", "set", "eval", "procs", "ploc",
    "pbody", "cont", "continue", "step", "over", "out", "pause", "coverage", "quit",
];

/// Commands accepted at the `:` prompt, with their arguments and what they do.
const DBG_COMMANDS: &[(&str, &str)] = &[
    ("help", "list these commands"),
    (
        "goto <line>",
        "move the source cursor to a line (or just type the number)",
    ),
    ("events", "print the full console history"),
    ("state", "show the debugger's state"),
    ("stack", "dump the raw stack frames"),
    ("vars [level]", "dump the variables of a frame"),
    ("get <name> [level]", "print one variable"),
    ("set <name> <value> [level]", "change a variable"),
    ("eval <script>", "evaluate Tcl at the global level"),
    ("procs", "list the procs the runtime knows"),
    ("ploc <proc>", "show where a proc starts"),
    ("pbody <proc>", "print a proc's body"),
    ("cont | continue", "continue running"),
    ("step | over | out", "step into, over, or out"),
    ("pause", "pause a running program"),
    ("coverage", "dump command coverage"),
    ("quit", "quit immediately"),
];

/// Tcl's own global variables. They are real, but listed after the
/// program's variables so they do not bury them.
fn is_tcl_builtin_global(name: &str) -> bool {
    matches!(
        name,
        "argc"
            | "argv"
            | "argv0"
            | "auto_index"
            | "auto_path"
            | "env"
            | "errorCode"
            | "errorInfo"
            | "tcl_interactive"
            | "tcl_library"
            | "tcl_libPath"
            | "tcl_nonwordchars"
            | "tcl_patchLevel"
            | "tcl_pkgPath"
            | "tcl_platform"
            | "tcl_precision"
            | "tcl_rcFileName"
            | "tcl_traceCompile"
            | "tcl_traceExec"
            | "tcl_version"
            | "tcl_wordchars"
            | "tclDefaultLibrary"
    )
}

/// Whether two paths name the same file, allowing for symlinked directories
/// such as macOS's `/var` → `/private/var`, which the runtime resolves.
fn same_file(left: &std::path::Path, right: &std::path::Path) -> bool {
    left == right
        || matches!(
            (std::fs::canonicalize(left), std::fs::canonicalize(right)),
            (Ok(left), Ok(right)) if left == right
        )
}

/// Whether `text` names a variable (`total`, `::ns::count`, `cfg(port)`)
/// rather than being an expression.
fn is_variable_reference(text: &str) -> bool {
    let (name, index) = match text.split_once('(') {
        Some((name, rest)) => match rest.strip_suffix(')') {
            Some(index) if !index.contains(['(', ')']) => (name, Some(index)),
            _ => return false,
        },
        None => (text, None),
    };
    let leading = name.trim_start_matches("::");
    !leading.is_empty()
        && !leading.starts_with(|c: char| c.is_ascii_digit())
        && leading
            .split("::")
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_'))
        && index.is_none_or(|index| !index.contains(['[', ']', '$', ' ']))
}

fn join_source_lines(lines: &[SourceLine]) -> String {
    lines
        .iter()
        .map(|line| line.raw.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 1-based line holding byte `offset` of `text`.
fn line_for_offset(text: &str, offset: u64) -> Option<u32> {
    let offset = usize::try_from(offset).ok()?;
    if offset > text.len() {
        return None;
    }
    let newlines = text.as_bytes()[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    u32::try_from(newlines + 1).ok()
}

/// Where `qname`'s body sits in `script`: body line N is script line
/// N + offset. `None` unless the script defines `qname` exactly once with
/// exactly `body`, so a redefined or generated proc is never shown against
/// text it does not run.
fn proc_body_line_offset(script: &str, qname: &str, body: &str) -> Option<u32> {
    let mut source = script.to_string();
    if !source.is_empty() {
        source.push('\n');
    }
    let wanted = qname.trim_start_matches("::");
    let body = body.trim_end_matches('\n');
    let mut spans = tcld_syntax::proc_body_spans(&source)
        .ok()?
        .into_iter()
        .filter(|span| {
            span.qname.trim_start_matches("::") == wanted
                && source
                    .get(span.start_byte..span.end_byte)
                    .is_some_and(|text| text.trim_end_matches('\n') == body)
        });
    let span = spans.next()?;
    if spans.next().is_some() {
        return None;
    }
    let header_line = source[..span.start_byte].matches('\n').count();
    u32::try_from(header_line).ok()
}

fn parse_disassembly_pc(line: &str) -> Option<u64> {
    let rest = line.trim_start().strip_prefix('(')?;
    let end = rest.find(')')?;
    rest[..end].trim().parse().ok()
}

fn format_debug_location(location: &DebugLocation) -> String {
    if let Some(source) = &location.source {
        return format!("{}:{}", source.block_id, source.line);
    }
    if let Some(code) = location.code {
        return format!("code:{} pc:{}", code.code_id.0, code.pc);
    }
    location
        .observed_pc
        .map_or_else(|| "unknown location".into(), |pc| format!("pc:{pc}"))
}

fn quote_tcl_word(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '$' => quoted.push_str("\\$"),
            '[' => quoted.push_str("\\["),
            '\n' => quoted.push_str("\\n"),
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn list_row(pane: Pane, hit: &tcld_tui::debug::HitTargets, y: u16) -> Option<usize> {
    let r = match pane {
        Pane::Stack => hit.stack_inner,
        Pane::Locals => hit.locals_inner,
        Pane::Breakpoints => hit.breakpoints_inner,
        Pane::Watches => hit.watches_inner,
        Pane::Events => hit.events_inner,
        Pane::Coverage => hit.coverage_inner,
        Pane::Source => return None,
    };
    tcld_tui::debug::HitTargets::list_row_at(r, y)
}

fn sync(state: &mut ratatui::widgets::ListState, len: usize) {
    if len == 0 {
        state.select(None);
    } else if state.selected().is_none() {
        state.select(Some(0));
    } else if state.selected().unwrap() >= len {
        state.select(Some(len - 1));
    }
}

fn trim_status(text: &str) -> String {
    let collapsed: String = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if collapsed.chars().count() > 120 {
        let mut out: String = collapsed.chars().take(119).collect();
        out.push('…');
        out
    } else {
        collapsed
    }
}

/// Persist breakpoints alongside the user script by default.
fn persist_path(user_script: &std::path::Path) -> PathBuf {
    let parent = user_script
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    parent.join(PERSIST_FILE)
}

/// Highlight a loaded script with the same Tree-sitter parse the explorer uses,
/// so the debugger and the explorer never disagree about the same source.
///
/// Called on load rather than per frame, so the parse is not cached.
fn highlight_lines(raw: &[String], theme: &Theme) -> Vec<SourceLine> {
    let highlighted = crate::highlight::HighlightCache::default().document(raw, theme);
    // `str::lines` drops the terminal newline, while tree-sitter-tcl needs a
    // command terminator to consistently surface the outermost command.
    // Restoring one does not move any real source offsets.
    let mut source = raw.join("\n");
    if !source.is_empty() {
        source.push('\n');
    }
    let proc_bodies = tcld_syntax::proc_body_spans(&source).unwrap_or_default();
    let mut commands_by_line = vec![Vec::new(); raw.len()];
    let mut containing_ends = Vec::new();
    for span in tcld_syntax::command_spans(&source).unwrap_or_default() {
        while containing_ends
            .last()
            .is_some_and(|end| *end <= span.start_byte)
        {
            containing_ends.pop();
        }
        let depth = containing_ends.len();
        containing_ends.push(span.end_byte);
        let Some(commands) = commands_by_line.get_mut(span.start_row) else {
            continue;
        };
        let line_len = raw.get(span.start_row).map_or(0, String::len);
        let proc_body = proc_bodies
            .iter()
            .filter(|body| body.start_byte <= span.start_byte && span.end_byte <= body.end_byte)
            .max_by_key(|body| body.start_byte);
        commands.push(SourceCommand {
            offset: span.start_byte as u64,
            len: span.end_byte.saturating_sub(span.start_byte) as u64,
            qname: proc_body.map(|body| body.qname.clone()),
            target_offset: proc_body
                .map(|body| span.start_byte.saturating_sub(body.start_byte) as u64)
                .unwrap_or(span.start_byte as u64),
            start_column: span.start_column.min(line_len),
            end_column: if span.end_row == span.start_row {
                span.end_column.min(line_len)
            } else {
                line_len
            },
            depth,
            label: span.name.unwrap_or_else(|| "command".to_string()),
        });
    }
    raw.iter()
        .cloned()
        .zip(highlighted)
        .zip(commands_by_line)
        .map(|((text, line), commands)| SourceLine::highlighted(text, line).with_commands(commands))
        .collect()
}

/// Read a source file from disk, detect binary content, and return a
/// theme-highlighted [`SourceLine`] list ready to feed the renderer.
/// Falls back to a friendly placeholder when the bytes are not
/// readable as text or look like a compiled artifact.
fn load_source_highlighted(path: &std::path::Path, theme: &Theme) -> Vec<SourceLine> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(err) => {
            return placeholder_lines(format!("(could not read source: {err})"), theme);
        }
    };
    if let Some(reason) = looks_binary(&bytes) {
        return placeholder_lines(
            format!(
                "(file looks like {reason} — pass a .tcl source, or run `tcld decompile` first)"
            ),
            theme,
        );
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return placeholder_lines(
                "(file is not valid UTF-8 — pass a Tcl source file)".to_string(),
                theme,
            );
        }
    };
    let raw: Vec<String> = text.lines().map(str::to_string).collect();
    highlight_lines(&raw, theme)
}

fn placeholder_lines(message: String, theme: &Theme) -> Vec<SourceLine> {
    vec![
        SourceLine::raw(""),
        SourceLine::highlighted(
            message.clone(),
            crate::highlight::HighlightCache::default()
                .document(std::slice::from_ref(&message), theme)
                .pop()
                .unwrap_or_default(),
        ),
    ]
}

/// Sniff a file's bytes for compiled-bytecode / binary markers. Returns
/// a short noun describing what we think we found, or `None` if the
/// bytes look like Tcl source. The detection is conservative: we
/// require either a TclPro bytecode signature or a meaningful share
/// of NUL / control bytes outside the first 4 KiB.
fn looks_binary(bytes: &[u8]) -> Option<&'static str> {
    if bytes.is_empty() {
        return None;
    }
    let head = &bytes[..bytes.len().min(4096)];
    if head
        .windows(b"TclPro ByteCode".len())
        .any(|w| w == b"TclPro ByteCode")
    {
        return Some("compiled .tbc bytecode");
    }
    let nuls = head.iter().filter(|b| **b == 0).count();
    if nuls >= 4 {
        return Some("a binary file");
    }
    let suspicious = head
        .iter()
        .filter(|b| {
            let b = **b;
            // Anything outside printable ASCII / common whitespace.
            !(b.is_ascii_graphic() || matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        })
        .count();
    let pct = (suspicious * 100) / head.len().max(1);
    if pct >= 8 {
        return Some("a binary file");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    struct TestBackend;

    impl DebugBackend for TestBackend {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::wire_host()
        }
        fn start_target(
            &mut self,
            _: &std::path::Path,
            _: bool,
        ) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn restart_target(&mut self, _: bool) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn continue_execution(&mut self) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn pause_execution(&mut self) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn step(&mut self, _: DebugStepKind) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn run_to_location(
            &mut self,
            _: DebugLocation,
        ) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn stack_frames(
            &mut self,
        ) -> Result<Vec<super::super::contract::DebugStackFrame>, super::super::backend::BackendError>
        {
            Ok(Vec::new())
        }
        fn variables_for_frame(
            &mut self,
            _: i64,
        ) -> Result<Vec<super::super::contract::DebugVariable>, super::super::backend::BackendError>
        {
            Ok(Vec::new())
        }
        fn evaluate(
            &mut self,
            _: i64,
            _: &str,
        ) -> Result<super::super::contract::DebugEvaluation, super::super::backend::BackendError>
        {
            Ok(super::super::contract::DebugEvaluation {
                completion_code: 0,
                result: String::new(),
            })
        }
        fn add_breakpoint(
            &mut self,
            spec: DebugBreakpointSpec,
        ) -> Result<DebugBreakpoint, super::super::backend::BackendError> {
            Ok(DebugBreakpoint {
                id: DebugBreakpointId("test-bp".into()),
                spec,
                enabled: true,
                resolution: DebugBreakpointResolution::Pending {
                    reason: "test".into(),
                },
                hit_count: 0,
                ignore_count: 0,
            })
        }
        fn remove_breakpoint(
            &mut self,
            _: &DebugBreakpointId,
        ) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn breakpoints(
            &mut self,
        ) -> Result<Vec<DebugBreakpoint>, super::super::backend::BackendError> {
            Ok(Vec::new())
        }
        fn set_breakpoint_enabled(
            &mut self,
            _: &DebugBreakpointId,
            _: bool,
        ) -> Result<(), super::super::backend::BackendError> {
            Ok(())
        }
        fn drain_events(&mut self) -> Vec<DebugEvent> {
            Vec::new()
        }
        fn state(&mut self) -> Result<DebugLifecycleState, super::super::backend::BackendError> {
            Ok(DebugLifecycleState::Ready)
        }
        fn sources(
            &mut self,
        ) -> Result<Vec<super::super::contract::SourceSnapshot>, super::super::backend::BackendError>
        {
            Ok(Vec::new())
        }
        fn companion_sources(
            &mut self,
        ) -> Result<
            Vec<super::super::contract::CompanionSourceSnapshot>,
            super::super::backend::BackendError,
        > {
            Ok(Vec::new())
        }
        fn disassembly(
            &mut self,
        ) -> Result<
            Vec<super::super::contract::DisassemblySnapshot>,
            super::super::backend::BackendError,
        > {
            Ok(Vec::new())
        }
        fn close(self: Box<Self>) {}
    }

    fn test_app(target: PreparedDebugTarget, theme: Theme) -> DebugApp {
        DebugApp::with_backend(target, Box::new(TestBackend), "test host backend", theme).unwrap()
    }

    fn key(code: KeyCode) -> KeyEvent {
        let modifiers = match code {
            KeyCode::BackTab => KeyModifiers::SHIFT,
            KeyCode::Char(value) if value.is_ascii_uppercase() => KeyModifiers::SHIFT,
            _ => KeyModifiers::NONE,
        };
        KeyEvent::new(code, modifiers)
    }

    fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn fake_app() -> DebugApp {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let user_script = std::env::temp_dir().join("tcld-debug-keymap-test.tcl");
        let _ = std::fs::write(&user_script, "puts hi\n");
        test_app(PreparedDebugTarget::user_script(user_script), theme)
    }

    #[test]
    fn pane_focus_cycles_with_tab_and_number_shortcuts() {
        let mut app = fake_app();
        assert_eq!(app.ui.focus, Pane::Source);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.ui.focus, Pane::Stack);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.ui.focus, Pane::Locals);
        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.ui.focus, Pane::Stack);
        app.handle_key(key(KeyCode::Char('5')));
        assert_eq!(app.ui.focus, Pane::Watches);
        app.handle_key(key(KeyCode::Char('7')));
        assert_eq!(app.ui.focus, Pane::Coverage);
        app.handle_key(key(KeyCode::Char('1')));
        assert_eq!(app.ui.focus, Pane::Source);
    }

    #[test]
    fn quitting_a_live_session_asks_first() {
        let mut app = fake_app();
        app.app_state = AppState::Stopped;
        app.handle_key(key(KeyCode::Char('q')));
        assert!(!app.quitting, "a paused program is not killed on one key");
        assert!(app.ui.confirm_quit);
        app.handle_key(key(KeyCode::Char('x')));
        assert!(
            !app.quitting && !app.ui.confirm_quit,
            "any other key keeps debugging"
        );
        app.handle_key(key(KeyCode::Char('q')));
        app.handle_key(key(KeyCode::Char('y')));
        assert!(app.quitting);

        let mut app = fake_app();
        app.app_state = AppState::Running;
        app.handle_key(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.quitting, "Ctrl-C never asks");
    }

    #[test]
    fn watching_a_bare_variable_name_watches_its_value() {
        assert!(is_variable_reference("total"));
        assert!(is_variable_reference("::ns::count"));
        assert!(is_variable_reference("cfg(port)"));
        assert!(!is_variable_reference("40+2"));
        assert!(!is_variable_reference("[llength $items]"));
        assert!(!is_variable_reference("$total"));
        assert!(!is_variable_reference("9lives"));

        let mut app = fake_app();
        add_watch_from_prompt(&mut app, "total");
        assert_eq!(app.watches[0].expr, "$total");
        assert_eq!(app.watches[0].name, "total");
    }

    #[test]
    fn proc_bodies_map_onto_their_defining_script() {
        let script = "# header\nproc greet {name} {\n    set g \"hi $name\"\n    return $g\n}\n";
        let body = "\n    set g \"hi $name\"\n    return $g\n";
        // Body line 2 (`set g …`) is script line 3.
        assert_eq!(proc_body_line_offset(script, "greet", body), Some(1));
        assert_eq!(proc_body_line_offset(script, "::greet", body), Some(1));
        assert_eq!(
            proc_body_line_offset(script, "greet", "\n    return other\n"),
            None,
            "a redefined body is never shown against text it does not run"
        );
        assert_eq!(proc_body_line_offset(script, "missing", body), None);
        assert_eq!(line_for_offset("a\nbb\nccc", 5), Some(3));
    }

    #[test]
    fn procs_defined_in_sourced_files_show_inside_those_files() {
        // The main script defines an identical body, so only the recorded
        // defining file can pick the right one.
        let proc_text = "proc ::child::work {} {\n    set marker child\n    return $marker\n}\n";
        let mut app = fake_app_with_source(&format!("{proc_text}child::work\n"));
        let child_path = PathBuf::from("/tmp/tcld-debug-sourced-child.tcl");
        let child_block = format!("file:{}", child_path.display());
        let child_text = format!("namespace eval ::child {{}}\n{proc_text}");
        let lines = |text: &str| {
            let raw: Vec<String> = text.lines().map(str::to_string).collect();
            highlight_lines(&raw, &app.theme)
        };
        let registration = |identity, block_id: &str, qname: Option<&str>, path| {
            super::super::contract::RegisteredSource {
                identity,
                block_id: block_id.to_string(),
                generation: 0,
                path,
                qualified_proc: qname.map(str::to_string),
                definition_path: Some(child_path.clone()),
                restart_limitation: None,
            }
        };
        let child_unit = CachedSourceUnit {
            registration: registration(
                DebugSourceUnitIdentity::File(child_path.clone()),
                &child_block,
                None,
                Some(child_path.clone()),
            ),
            lines: lines(&child_text),
        };
        let body = lines("\n    set marker child\n    return $marker\n");
        let proc_unit = CachedSourceUnit {
            registration: registration(
                DebugSourceUnitIdentity::Procedure("child::work".into()),
                "proc:child::work",
                Some("child::work"),
                None,
            ),
            lines: body.clone(),
        };
        app.cache
            .source_units
            .insert(child_block.clone(), child_unit);
        app.cache
            .source_units
            .insert("proc:child::work".into(), proc_unit);
        app.cache.proc_bodies.insert("child::work".into(), body);
        app.focused_proc = Some(FocusedProc {
            block_id: "proc:child::work".into(),
            block_kind: tcld_tui::debug::SourceBlockKind::Proc,
            source_path: None,
            qname: "child::work".into(),
            line: 2,
            offset_start: 5,
            offset_end: 21,
            pause_reason: tcld_tui::debug::PauseReasonLabel::Step,
            frame_count: 1,
            breakpoint: None,
            pause_kind: tcld_tui::debug::PauseKind::Normal,
            error_message: None,
        });

        app.update_source_view();
        // Body line 2 is line 3 of the child file, which starts one line later.
        assert_eq!(
            app.cache.showing_block_id.as_deref(),
            Some(child_block.as_str())
        );
        assert_eq!(app.cache.origin, SourceOrigin::SourcedFile);
        assert_eq!(app.displayed_pause_line(), Some(3));
        assert_eq!(app.ui.source_cursor, 2);
        assert_eq!(
            app.source_label(),
            "tcld-debug-sourced-child.tcl · in child::work"
        );
        assert_eq!(
            app.location_label().as_deref(),
            Some("child::work · tcld-debug-sourced-child.tcl:3")
        );

        // Without a recorded definition, the first file defining the body wins.
        if let Some(unit) = app.cache.source_units.get_mut("proc:child::work") {
            unit.registration.definition_path = None;
        }
        app.cache.proc_placements.clear();
        app.update_source_view();
        assert_eq!(app.cache.origin, SourceOrigin::UserScript);
        assert_eq!(app.displayed_pause_line(), Some(2));
    }

    #[test]
    fn the_paused_command_is_marked_on_its_line() {
        let mut app = fake_app_with_source(
            "proc ::calc::double {x} {\n    set y [expr {$x * 2}]\n    return $y\n}\ncalc::double 21\n",
        );
        let raw: Vec<String> = "\n    set y [expr {$x * 2}]\n    return $y\n"
            .lines()
            .map(str::to_string)
            .collect();
        let body = highlight_lines(&raw, &app.theme);
        app.cache.proc_bodies.insert("calc::double".into(), body);
        let pause_at = |app: &mut DebugApp, offset_start| {
            app.focused_proc = Some(FocusedProc {
                block_id: "proc:calc::double".into(),
                block_kind: tcld_tui::debug::SourceBlockKind::Proc,
                source_path: None,
                qname: "calc::double".into(),
                line: 2,
                offset_start,
                offset_end: offset_start + 4,
                pause_reason: tcld_tui::debug::PauseReasonLabel::Step,
                frame_count: 1,
                breakpoint: None,
                pause_kind: tcld_tui::debug::PauseKind::Normal,
                error_message: None,
            });
            app.update_source_view();
        };

        // Tcl runs the nested `expr` (body byte 12) before the `set` around it.
        pause_at(&mut app, 12);
        assert_eq!(app.ui.source_cursor, 1);
        assert_eq!(app.ui.source_command_cursor, 1);
        pause_at(&mut app, 5);
        assert_eq!(app.ui.source_command_cursor, 0);
    }

    #[test]
    fn locals_list_program_variables_before_tcl_globals() {
        assert!(is_tcl_builtin_global("tcl_version"));
        assert!(is_tcl_builtin_global("env"));
        assert!(!is_tcl_builtin_global("results"));
    }

    #[test]
    fn quit_keys_set_quitting() {
        let mut app = fake_app();
        app.handle_key(key(KeyCode::Char('q')));
        assert!(app.quitting);

        let mut app2 = fake_app();
        app2.handle_key(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app2.quitting);
    }

    #[test]
    fn opening_a_prompt_routes_subsequent_keys_to_the_buffer() {
        let mut app = fake_app();
        app.handle_key(key(KeyCode::Char(':')));
        assert!(app.prompt.is_some());
        app.handle_key(key(KeyCode::Char('v')));
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Char('s')));
        assert_eq!(app.prompt.as_ref().unwrap().input.buffer, "vars");
        app.handle_key(key(KeyCode::Esc));
        assert!(app.prompt.is_none());
    }

    #[test]
    fn slash_opens_event_filter_with_existing_value_preselected() {
        let mut app = fake_app();
        app.event_filter = "error".into();
        app.handle_key(key(KeyCode::Char('/')));
        let prompt = app.prompt.as_ref().unwrap();
        assert_eq!(prompt.kind, PromptKind::EventFilter);
        assert_eq!(prompt.input.buffer, "error");
    }

    #[test]
    fn submitting_event_filter_updates_filter_and_clears_prompt() {
        let mut app = fake_app();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('p')));
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Char('u')));
        app.handle_key(key(KeyCode::Char('s')));
        app.handle_key(key(KeyCode::Enter));
        assert!(app.prompt.is_none());
        assert_eq!(app.event_filter, "paus");
    }

    #[test]
    fn help_overlay_toggles_with_question_mark() {
        let mut app = fake_app();
        assert!(!app.ui.show_help);
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.ui.show_help);
        app.handle_key(key(KeyCode::Esc));
        assert!(!app.ui.show_help);
    }

    #[test]
    fn b_in_target_source_uses_source_location() {
        let mut app = fake_app();
        app.handle_key(key(KeyCode::Char('b')));
        let status = app.status.to_lowercase();
        assert!(
            status.contains("breakpoint")
                && (status.contains("line 1") || status.contains(".tcl:1")),
            "the canonical app should route b to a source breakpoint: {}",
            app.status
        );
    }

    #[test]
    fn moving_source_cursor_clamps_at_top_and_bottom() {
        let mut app = fake_app();
        app.handle_key(key(KeyCode::Char('1')));
        // Source has only 1 line in the fake script.
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.ui.source_cursor, 0);
        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.ui.source_cursor, 0);
    }

    #[test]
    fn delete_in_breakpoints_pane_does_nothing_with_empty_list() {
        let mut app = fake_app();
        app.handle_key(key(KeyCode::Char('4')));
        app.handle_key(key(KeyCode::Char('d')));
        // Did not panic; selection is None.
        assert!(app.ui.selections.breakpoints.selected().is_none());
    }

    #[test]
    fn persist_path_lives_alongside_user_script() {
        let path = persist_path(std::path::Path::new("/tmp/sub/demo.tcl"));
        assert_eq!(path.file_name().unwrap().to_str(), Some(PERSIST_FILE));
        assert!(path.parent().unwrap().ends_with("/tmp/sub"));
    }

    #[test]
    fn unsupported_persisted_state_is_rejected_before_session_access() {
        let mut app = fake_app();
        let error = app
            .import_persisted_state(r#"{"version":2,"breakpoints":"","watches":[]}"#)
            .unwrap_err();
        assert_eq!(error, "unsupported persisted debugger state version 2");
    }

    #[test]
    fn highlight_lines_handles_empty_input() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines = highlight_lines(&[], &theme);
        assert!(lines.is_empty());
    }

    #[test]
    fn highlight_lines_exposes_nested_commands_in_selection_order() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let lines = highlight_lines(&["set x [foo [bar]]".to_string()], &theme);
        let commands = &lines[0].commands;
        assert_eq!(
            commands
                .iter()
                .map(|command| (command.label.as_str(), command.depth, command.start_column))
                .collect::<Vec<_>>(),
            vec![("set", 0, 0), ("foo", 1, 7), ("bar", 2, 12)]
        );
        assert!(commands.iter().all(|command| command.len > 0));
    }

    #[test]
    fn command_keys_cycle_nested_commands_on_the_selected_line() {
        let mut app = fake_app();
        app.cache.user_script = highlight_lines(&["set x [foo [bar]]".to_string()], &app.theme);
        app.handle_key(key(KeyCode::Char(']')));
        assert_eq!(app.ui.source_command_cursor, 1);
        assert!(app.status.contains("foo"));
        app.handle_key(key(KeyCode::Char('[')));
        assert_eq!(app.ui.source_command_cursor, 0);
        assert!(app.status.contains("set"));
    }

    #[test]
    fn whole_file_commands_inside_proc_use_proc_relative_targets() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let raw = "proc demo {} {\n  set x [expr {1 + 1}]\n}\n"
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let lines = highlight_lines(&raw, &theme);
        let commands = &lines[1].commands;
        let set = commands
            .iter()
            .find(|command| command.label == "set")
            .unwrap();
        let expr = commands
            .iter()
            .find(|command| command.label == "expr")
            .unwrap();
        assert_eq!(set.qname.as_deref(), Some("demo"));
        assert_eq!(expr.qname.as_deref(), Some("demo"));
        assert_ne!(set.target_offset, expr.target_offset);
        assert!(set.target_offset < set.offset);
    }

    #[test]
    fn looks_binary_recognises_tclpro_bytecode_header() {
        let mut bytes =
            b"#!/usr/bin/env tclsh\nif {[catch {package require tbcload 1.6} err] == 1} { ... }\n"
                .to_vec();
        bytes.extend_from_slice(b"TclPro ByteCodeS2R0 1.7 8.4\n");
        bytes.extend_from_slice(&[0x00; 64]);
        assert_eq!(
            looks_binary(&bytes),
            Some("compiled .tbc bytecode"),
            "TclPro header should classify as bytecode"
        );
    }

    #[test]
    fn looks_binary_returns_none_for_plain_tcl_source() {
        let bytes = b"proc demo {} {\n    set a 1\n    set b 2\n}\nafter 100 demo\n";
        assert_eq!(looks_binary(bytes), None);
    }

    #[test]
    fn looks_binary_flags_dense_nul_bytes() {
        let mut bytes = vec![0u8; 8];
        bytes.extend_from_slice(b"\nrest is text\n");
        assert_eq!(looks_binary(&bytes), Some("a binary file"));
    }

    #[test]
    fn placeholder_lines_appear_when_source_is_binary() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let path = std::env::temp_dir().join(format!(
            "tcld-debug-binary-test-{}.tbc",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"TclPro ByteCodeS2R0 1.7 8.4\n");
        bytes.extend_from_slice(&[0u8; 32]);
        std::fs::write(&path, bytes).unwrap();
        let lines = load_source_highlighted(&path, &theme);
        let combined: String = lines
            .iter()
            .map(|l| l.raw.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("compiled .tbc bytecode"),
            "expected binary-file placeholder — got:\n{combined}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dropping_provenance_unlinks_the_owned_temp_dir() {
        let dir = tempfile::Builder::new()
            .prefix("tcld-debug-drop-test-")
            .tempdir()
            .unwrap();
        let dir_path = dir.path().to_path_buf();
        let runtime = dir_path.join("decompiled.tcl");
        std::fs::write(&runtime, "puts hi\n").unwrap();
        assert!(dir_path.exists());
        let provenance =
            TargetProvenance::decompiled(runtime.clone(), PathBuf::from("/tmp/orig.tbc"), dir);
        // The temp dir is still alive while the provenance owns it.
        assert!(dir_path.exists());
        drop(provenance);
        assert!(
            !dir_path.exists(),
            "tempfile::TempDir should unlink the directory on drop"
        );
    }

    #[test]
    fn dropping_debug_app_unlinks_the_owned_temp_dir() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let dir = tempfile::Builder::new()
            .prefix("tcld-debug-app-drop-")
            .tempdir()
            .unwrap();
        let dir_path = dir.path().to_path_buf();
        let runtime = dir_path.join("decompiled.tcl");
        std::fs::write(&runtime, "puts hi\n").unwrap();
        let provenance =
            TargetProvenance::decompiled(runtime.clone(), PathBuf::from("/tmp/orig.tbc"), dir);
        let app = test_app(PreparedDebugTarget::new(provenance, None), theme);
        assert!(dir_path.exists(), "DebugApp should keep the dir alive");
        drop(app);
        assert!(
            !dir_path.exists(),
            "DebugApp drop should propagate the temp-dir cleanup"
        );
    }

    #[test]
    fn decompiled_provenance_marks_origin_synthetic_and_logs_event() {
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let runtime = std::env::temp_dir().join(format!(
            "tcld-debug-runtime-{}.tcl",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&runtime, "puts hi\n").unwrap();
        let display = PathBuf::from("/tmp/orig.tbc");
        // Allocate a real TempDir so the provenance owns a guard;
        // the runtime path doesn't have to live inside it for this
        // test — we're verifying the synthetic-origin plumbing.
        let temp_dir = tempfile::TempDir::with_prefix("tcld-debug-test-").unwrap();
        let provenance = TargetProvenance::decompiled(runtime.clone(), display.clone(), temp_dir);
        // Re-target runtime_path for clarity (decompiled() already
        // accepted it).
        let _ = display;
        let app = test_app(PreparedDebugTarget::new(provenance, None), theme);
        assert_eq!(app.cache.user_script_origin, SourceOrigin::Synthetic);
        assert_eq!(app.cache.origin, SourceOrigin::Synthetic);
        assert!(
            app.events.iter().any(|e| e.text.starts_with("decompiled")),
            "expected `decompiled …` info event in the log"
        );
        assert!(
            app.status.starts_with("decompiled"),
            "status should advertise the decompile"
        );
        let _ = std::fs::remove_file(&runtime);
    }

    #[test]
    fn highlight_lines_attaches_spans_for_tcl_source() {
        let theme = Theme::with_palette(false, "obsidian").unwrap();
        let raw = vec!["proc demo {} { set a 1 }".to_string()];
        let lines = highlight_lines(&raw, &theme);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].spans.is_empty(), "expected highlighter spans");
        assert_eq!(lines[0].raw, raw[0]);
    }

    fn fake_app_with_source(text: &str) -> DebugApp {
        // Parallel tests must never share a script: a clock can tick
        // coarser than the nanoseconds it reports.
        static NEXT_SCRIPT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let user_script = std::env::temp_dir().join(format!(
            "tcld-debug-test-{}-{}.tcl",
            std::process::id(),
            NEXT_SCRIPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::write(&user_script, text).unwrap();
        test_app(PreparedDebugTarget::user_script(user_script), theme)
    }

    #[test]
    fn source_search_finds_substring_and_n_cycles_through_matches() {
        let mut app = fake_app_with_source(
            "proc demo {} {\n    set alpha 1\n    set beta 2\n    set alpha 3\n}\n",
        );
        app.update_source_search("alpha");
        assert_eq!(app.search_match_lines, vec![1, 3]);
        assert_eq!(app.search_cursor, Some(0));
        assert_eq!(app.ui.source_cursor, 1);

        app.search_next(1);
        assert_eq!(app.search_cursor, Some(1));
        assert_eq!(app.ui.source_cursor, 3);

        app.search_next(1);
        assert_eq!(app.search_cursor, Some(0), "wraps around");
        assert_eq!(app.ui.source_cursor, 1);

        app.search_next(-1);
        assert_eq!(app.search_cursor, Some(1));
    }

    #[test]
    fn canonical_source_cache_follows_sourced_and_anonymous_owner_units() {
        let mut app = fake_app_with_source("set entry 1\n");
        let child_path = PathBuf::from("/tmp/canonical-child.tcl");
        let child_block = format!("file:{}", child_path.display());
        app.cache.source_units.insert(
            child_block.clone(),
            CachedSourceUnit {
                registration: super::super::contract::RegisteredSource {
                    identity: DebugSourceUnitIdentity::File(child_path.clone()),
                    block_id: child_block.clone(),
                    generation: 2,
                    path: Some(child_path),
                    qualified_proc: None,
                    definition_path: Some(PathBuf::from("/tmp/canonical-child.tcl")),
                    restart_limitation: None,
                },
                lines: vec![SourceLine::raw("set child 1")],
            },
        );
        app.focused_proc = Some(FocusedProc {
            block_id: child_block.clone(),
            block_kind: tcld_tui::debug::SourceBlockKind::File,
            source_path: Some("/tmp/canonical-child.tcl".into()),
            qname: String::new(),
            line: 1,
            offset_start: 0,
            offset_end: 11,
            pause_reason: tcld_tui::debug::PauseReasonLabel::Step,
            frame_count: 1,
            breakpoint: None,
            pause_kind: tcld_tui::debug::PauseKind::Normal,
            error_message: None,
        });
        app.update_source_view();
        assert_eq!(app.cache.origin, SourceOrigin::SourcedFile);
        assert_eq!(app.current_source_lines()[0].raw, "set child 1");

        let anonymous_block = "anon:0123456789abcdef".to_owned();
        app.cache.source_units.insert(
            anonymous_block.clone(),
            CachedSourceUnit {
                registration: super::super::contract::RegisteredSource {
                    identity: DebugSourceUnitIdentity::Anonymous {
                        session_key: anonymous_block.clone(),
                    },
                    block_id: anonymous_block.clone(),
                    generation: 0,
                    path: None,
                    qualified_proc: None,
                    definition_path: Some(PathBuf::from("/tmp/entry.tcl")),
                    restart_limitation: Some("session-only".into()),
                },
                lines: vec![SourceLine::raw("set dynamic 1")],
            },
        );
        app.focused_proc.as_mut().unwrap().block_id = anonymous_block.clone();
        app.focused_proc.as_mut().unwrap().block_kind = tcld_tui::debug::SourceBlockKind::Anonymous;
        app.update_source_view();
        assert_eq!(app.cache.origin, SourceOrigin::Anonymous);
        assert_eq!(app.current_source_lines()[0].raw, "set dynamic 1");

        app.invalidate_block(&anonymous_block);
        assert!(!app.cache.source_units.contains_key(&anonymous_block));
        assert!(app.cache.showing_block_id.is_none());
    }

    #[test]
    fn source_search_clears_when_query_empty() {
        let mut app = fake_app_with_source("set a 1\nset b 2\n");
        app.update_source_search("set");
        assert_eq!(app.search_match_lines.len(), 2);
        app.update_source_search("");
        assert!(app.search_match_lines.is_empty());
        assert!(app.search_cursor.is_none());
    }

    #[test]
    fn ctrl_f_opens_source_search_prompt_with_existing_query() {
        let mut app = fake_app_with_source("set x 1\n");
        app.search_query = "x".into();
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        let prompt = app.prompt.as_ref().expect("prompt opens");
        assert_eq!(prompt.kind, PromptKind::SearchSource);
        assert_eq!(prompt.input.buffer, "x");
    }

    #[test]
    fn n_after_search_jumps_through_matches_without_stepping() {
        let mut app = fake_app_with_source("set a 1\nset b 2\nset a 3\n");
        app.update_source_search("a");
        app.search_cursor = Some(0);
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(
            app.ui.source_cursor,
            app.search_match_lines[1 % app.search_match_lines.len()]
        );
    }

    #[test]
    fn locals_diff_marks_changed_values() {
        let mut app = fake_app_with_source("puts hi\n");
        app.app_state = AppState::Stopped;
        // Seed previous-locals snapshot manually.
        app.locals_prev.insert(
            (0, None),
            vec![("x".into(), "1".into()), ("y".into(), "2".into())],
        );
        // Build LocalRow input as if update_locals had run on (x=1, y=42).
        let prev = app.locals_prev.get(&(0, None)).cloned().unwrap();
        let prev_map: HashMap<&str, &str> =
            prev.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let fresh = [
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "42".to_string()),
        ];
        let rows: Vec<LocalRow> = fresh
            .iter()
            .map(|(name, value)| LocalRow {
                name: name.clone(),
                value: value.clone(),
                changed: prev_map
                    .get(name.as_str())
                    .map(|p| *p != value)
                    .unwrap_or(true),
                builtin: false,
            })
            .collect();
        assert!(!rows[0].changed, "x stayed 1 — should not be flagged");
        assert!(rows[1].changed, "y went 2 → 42 — should be flagged");
    }

    #[test]
    fn reset_to_user_script_drops_focused_proc_view() {
        let mut app = fake_app_with_source("set a 1\n");
        app.cache.showing_qname = Some("demo".to_string());
        app.cache.origin = SourceOrigin::ProcBody;
        app.handle_key(key(KeyCode::Char('u')));
        assert!(app.cache.showing_qname.is_none());
        assert_eq!(app.cache.origin, SourceOrigin::UserScript);
    }

    #[test]
    fn theme_cycle_updates_label_and_rehighlights() {
        let theme = Theme::with_palette(false, "obsidian").unwrap();
        let user_script = std::env::temp_dir().join("tcld-debug-theme-test.tcl");
        std::fs::write(&user_script, "set a 1\n").unwrap();
        let mut app = test_app(PreparedDebugTarget::user_script(user_script), theme);
        let initial = app.theme_name.clone();
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_ne!(
            app.theme_name, initial,
            "theme name should rotate after Ctrl-T"
        );
    }

    #[test]
    fn dot_repeats_last_step_action() {
        let mut app = fake_app_with_source("set a 1\n");
        // Pretend last action was a step over.
        app.last_action = Some(RepeatableAction::StepOver);
        // No session attached; we just want to verify the dispatch runs.
        app.handle_key(key(KeyCode::Char('.')));
        assert!(app.status.contains("step over") || app.status.contains("no nub"));
    }

    #[test]
    fn dot_with_no_history_reports_so() {
        let mut app = fake_app_with_source("\n");
        app.handle_key(key(KeyCode::Char('.')));
        assert!(app.status.contains("no last action"));
    }

    #[test]
    fn command_history_navigates_with_up_and_down() {
        let mut app = fake_app_with_source("\n");
        app.command_history.push_back("vars 0".into());
        app.command_history.push_back("stack".into());
        app.handle_key(key(KeyCode::Char(':')));
        // Up should pull the most recent entry.
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.prompt.as_ref().unwrap().input.buffer, "stack");
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.prompt.as_ref().unwrap().input.buffer, "vars 0");
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.prompt.as_ref().unwrap().input.buffer, "stack");
        app.handle_key(key(KeyCode::Down));
        assert!(app.prompt.as_ref().unwrap().input.buffer.is_empty());
    }

    #[test]
    fn proc_picker_filter_narrows_entries_as_user_types() {
        let mut app = fake_app_with_source("\n");
        app.proc_picker = vec![
            DebugProcedure {
                qualified_name: "alpha".into(),
                block_id: None,
                first_offset: None,
            },
            DebugProcedure {
                qualified_name: "beta".into(),
                block_id: None,
                first_offset: None,
            },
            DebugProcedure {
                qualified_name: "::ns::alpha".into(),
                block_id: None,
                first_offset: None,
            },
        ];
        app.update_proc_picker_filter("");
        assert_eq!(app.proc_picker_filtered.len(), 3);
        app.update_proc_picker_filter("alpha");
        assert_eq!(app.proc_picker_filtered.len(), 2);
        assert!(app.proc_picker_filtered.iter().all(|q| q.contains("alpha")));
        app.update_proc_picker_filter("nope");
        assert!(app.proc_picker_filtered.is_empty());
        assert!(app.proc_picker_selected.is_none());
    }

    #[test]
    fn picker_move_clamps_inside_filtered_range() {
        let mut app = fake_app_with_source("\n");
        app.proc_picker_filtered = vec!["a".into(), "b".into(), "c".into()];
        app.proc_picker_selected = Some(0);
        app.picker_move(2);
        assert_eq!(app.proc_picker_selected, Some(2));
        app.picker_move(2);
        assert_eq!(app.proc_picker_selected, Some(2), "clamps at top");
        app.picker_move(-5);
        assert_eq!(app.proc_picker_selected, Some(0));
    }

    #[test]
    fn tab_completion_completes_unique_dbg_command() {
        let mut app = fake_app_with_source("\n");
        app.handle_key(key(KeyCode::Char(':')));
        for c in "vars".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Tab));
        let prompt = app.prompt.as_ref().unwrap();
        assert_eq!(prompt.input.buffer, "vars ");
    }

    #[test]
    fn tab_completion_lists_matches_when_ambiguous() {
        let mut app = fake_app_with_source("\n");
        app.handle_key(key(KeyCode::Char(':')));
        for c in "p".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Tab));
        // "procs", "ploc", "pbody", "pause" all start with 'p'.
        assert!(app.status.contains("matches:"));
    }

    #[test]
    fn goto_command_jumps_source_cursor() {
        let mut app = fake_app_with_source("a\nb\nc\nd\ne\n");
        execute_dbg_command(&mut app, "goto 4");
        assert_eq!(app.ui.source_cursor, 3);
        execute_dbg_command(&mut app, "2");
        assert_eq!(app.ui.source_cursor, 1);
        execute_dbg_command(&mut app, "goto 999");
        assert_eq!(app.ui.source_cursor, 4, "clamps at last line");
    }

    #[test]
    fn help_overlay_scrolls_with_j_and_k() {
        let mut app = fake_app_with_source("\n");
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.ui.show_help);
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.ui.help_scroll, 1);
        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.ui.help_scroll, 2);
        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.ui.help_scroll, 1);
        app.handle_key(key(KeyCode::Char('k')));
        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.ui.help_scroll, 0, "saturates at 0");
        app.handle_key(key(KeyCode::Esc));
        assert!(!app.ui.show_help);
        assert_eq!(app.ui.help_scroll, 0);
    }

    fn rendered_text(app: &mut DebugApp, width: u16, height: u16) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend as RenderBackend;
        let backend = RenderBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .expect("draw debugger");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer.cell((x, y)).expect("cell").symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn source_backend_renders_the_canonical_debugger_chrome() {
        let mut app = fake_app_with_source("set answer 42\nputs $answer\n");
        app.app_state = AppState::Stopped;
        let text = rendered_text(&mut app, 120, 32);
        for expected in [
            "tcld debug",
            "Source",
            "Stack",
            "Locals",
            "Breakpoints",
            "Watches",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
        assert!(
            text.contains("set answer 42"),
            "source text missing:\n{text}"
        );
    }

    #[test]
    #[cfg(feature = "hybrid")]
    fn hybrid_backend_uses_the_same_chrome_with_tcl9_disassembly_and_capability_reason() {
        use super::super::planning::{
            DebugBackendPolicy, DebugRequest, plan_debug, prepare_debug_target,
        };
        use tcld_vm::RuntimeMode;

        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/bytecode/tcl9/tc1.tbc")
            .canonicalize()
            .expect("Tcl 9 fixture");
        let target = prepare_debug_target(
            plan_debug(DebugRequest::new(path, DebugBackendPolicy::VmStrict), true)
                .expect("hybrid plan"),
        )
        .expect("prepared hybrid target");
        let backend = super::super::backend::HybridInProcessBackend::new(RuntimeMode::VmStrict)
            .expect("hybrid backend");
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let mut app =
            DebugApp::with_backend(target, Box::new(backend), "hybrid TEBC (VmStrict)", theme)
                .expect("canonical debugger app");
        app.start_debug_target();
        let text = rendered_text(&mut app, 120, 32);
        // Raw bytecode has no verified source, so its listing is synthetic.
        assert!(
            text.contains("tc1.tbc [synthetic]"),
            "raw bytecode identity is explicit:\n{text}"
        );
        assert!(
            text.contains("Source · tc1.tbc · synthetic"),
            "synthetic listing pane label:\n{text}"
        );
        assert!(
            app.backend_label.contains("VmStrict"),
            "backend mode is named: {}",
            app.backend_label
        );
        for expected in [
            "tcld debug",
            "Source",
            "Stack",
            "Locals",
            "Breakpoints",
            "Watches",
            "returnImm",
        ] {
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
        assert!(
            text.contains("tcl9.0"),
            "opcode table provenance missing:\n{text}"
        );

        app.handle_key(key(KeyCode::Char('e')));
        assert!(
            app.status.contains("unavailable") && app.status.contains("vm-strict"),
            "unsupported action must explain its capability reason: {}",
            app.status
        );
    }

    #[test]
    #[cfg(feature = "hybrid")]
    fn compiled_source_hybrid_renders_original_source_as_canonical_with_disassembly() {
        use super::super::planning::{
            DebugBackendPolicy, DebugRequest, plan_debug, prepare_debug_target,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source-image.tcl");
        std::fs::write(&path, "set answer 40\nset answer [expr {$answer + 2}]\n").unwrap();
        let path = path.canonicalize().unwrap();
        let target = prepare_debug_target(
            plan_debug(DebugRequest::new(&path, DebugBackendPolicy::VmStrict), true).unwrap(),
        )
        .unwrap();
        let image = target
            .compiled_image
            .clone()
            .expect("prepared source image");
        let backend = super::super::backend::HybridInProcessBackend::new_with_image(
            tcld_vm::RuntimeMode::VmStrict,
            image,
        )
        .unwrap();
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let mut app = DebugApp::with_backend(
            target,
            Box::new(backend),
            "compiled Tcl source (VmStrict)",
            theme,
        )
        .unwrap();
        app.start_debug_target();
        let text = rendered_text(&mut app, 120, 32);
        assert!(
            text.contains("set answer 40"),
            "original source is primary:\n{text}"
        );
        assert!(
            text.contains("expr {$answer + 2}"),
            "canonical command text:\n{text}"
        );
        assert!(
            text.contains("nop") && text.contains("invokeStk1"),
            "secondary disassembly remains accessible:\n{text}"
        );
        assert!(
            !text.contains("companion (unverified)"),
            "compiled source must not be mislabeled:\n{text}"
        );
    }
}
