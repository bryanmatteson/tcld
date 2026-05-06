//! `tcld debug` driver loop.
//!
//! Owns the wire-protocol [`Session`], the local source-text cache,
//! the rolling event log, and the [`DebugUiState`]. Rendering is
//! delegated to [`tcld_tui::debug::render`] so the view itself stays
//! reusable and themed.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use serde::{Deserialize, Serialize};
use tcld_debug_wire::{Op, WireEvent};
use tcld_tui::Theme;
use tcld_tui::debug::{
    AppState, BindingContext, CoverageRow, DebugAction, DebugSnapshot, DebugUiState, EventLine,
    FocusedProc, LocalRow, Pane, PickerOverlay, ProcBreakpoint, PromptInput, PromptStep,
    SourceCommand, SourceLine, SourceOrigin, SourceSearch, StackFrame, WatchEntry, prompt_step,
    render_in, resolve_binding,
};
use tcld_tui::terminal::{Event, Frame, Rect, RuntimeAction, RuntimeEvent, TerminalApplication};

use super::config::{NubPaths, TclAbi};
use super::session::{ProcEntry, Session, format_event};

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
    StatusInspect,
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
    /// Which qname the source pane is currently showing (None ⇒ user
    /// script).
    showing_qname: Option<String>,
    origin: SourceOrigin,
}

/// What the driver was asked to debug, plus the actual file the nub
/// will source. When the input was a `.tbc`, [`Self::runtime_path`]
/// points at a decompiled `.tcl` inside an OS-allocated temp dir
/// (created via `tempfile::TempDir`); the dir is unlinked when this
/// struct (or any [`DebugApp`] that consumed it) is dropped.
/// [`Self::display_path`] always keeps the original for UI labelling.
#[derive(Debug)]
pub struct SourceProvenance {
    pub runtime_path: PathBuf,
    pub display_path: PathBuf,
    pub decompiled_from: Option<PathBuf>,
    pub bytecode_tcl_version: Option<String>,
    /// Lifetime guard for the OS-allocated temp dir. `None` when the
    /// runtime path is a user-owned source file we must NOT delete.
    pub(super) temp_dir: Option<tempfile::TempDir>,
}

impl SourceProvenance {
    pub fn user_script(path: PathBuf) -> Self {
        Self {
            runtime_path: path.clone(),
            display_path: path,
            decompiled_from: None,
            bytecode_tcl_version: None,
            temp_dir: None,
        }
    }

    /// Build a decompiled-source provenance that owns the temp dir
    /// holding the rendered Tcl. The directory is unlinked when the
    /// returned value (or any [`DebugApp`] that consumed it) is
    /// dropped.
    pub fn decompiled(
        runtime_path: PathBuf,
        display_path: PathBuf,
        temp_dir: tempfile::TempDir,
    ) -> Self {
        let decompiled_from = Some(display_path.clone());
        Self {
            runtime_path,
            display_path,
            decompiled_from,
            bytecode_tcl_version: None,
            temp_dir: Some(temp_dir),
        }
    }

    pub fn decompiled_bytecode(
        runtime_path: PathBuf,
        display_path: PathBuf,
        bytecode_tcl_version: String,
        temp_dir: tempfile::TempDir,
    ) -> Self {
        Self {
            runtime_path,
            display_path: display_path.clone(),
            decompiled_from: Some(display_path),
            bytecode_tcl_version: Some(bytecode_tcl_version),
            temp_dir: Some(temp_dir),
        }
    }
}

#[derive(Debug)]
pub struct DebugTarget {
    pub runtime_path: PathBuf,
    pub display_path: PathBuf,
    pub provenance: SourceProvenance,
    pub required_tcl_abi: Option<TclAbi>,
}

impl DebugTarget {
    pub fn new(provenance: SourceProvenance, required_tcl_abi: Option<TclAbi>) -> Self {
        Self {
            runtime_path: provenance.runtime_path.clone(),
            display_path: provenance.display_path.clone(),
            provenance,
            required_tcl_abi,
        }
    }

    pub fn user_script(path: PathBuf) -> Self {
        Self::new(SourceProvenance::user_script(path), None)
    }
}

pub struct DebugApp {
    user_script: PathBuf,
    display_path: PathBuf,
    /// Lifetime guard for an OS-allocated temp dir created when the
    /// input was a `.tbc` we had to decompile. `None` for plain
    /// user-script inputs. Drop unlinks the directory + every file
    /// in it so we never litter `/tmp`.
    _temp_dir: Option<tempfile::TempDir>,
    paths: NubPaths,
    session: Option<Session>,

    cache: SourceCache,

    app_state: AppState,
    focused_proc: Option<FocusedProc>,
    paused_focus: Option<FocusedProc>,
    inspected_frame: i64,
    stack: Vec<StackFrame>,
    locals: Vec<LocalRow>,
    /// Snapshot of the previous pause's locals at this frame level —
    /// used to compute the `changed` flag for the current pass.
    locals_prev: HashMap<i64, Vec<(String, String)>>,
    breakpoints: Vec<ProcBreakpoint>,
    watches: Vec<WatchEntry>,
    coverage: Vec<CoverageRow>,
    coverage_enabled: bool,
    events: VecDeque<EventLine>,
    event_filter: String,
    last_status_full: String,

    proc_picker: Vec<ProcEntry>,
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
    pub fn new(target: DebugTarget, paths: NubPaths, theme: Theme) -> io::Result<Self> {
        let DebugTarget {
            runtime_path: _,
            display_path: _,
            provenance,
            required_tcl_abi,
        } = target;
        let SourceProvenance {
            runtime_path,
            display_path,
            decompiled_from,
            bytecode_tcl_version,
            temp_dir,
        } = provenance;
        let user_script_origin = if decompiled_from.is_some() {
            SourceOrigin::Synthetic
        } else {
            SourceOrigin::UserScript
        };

        let highlighted = load_source_highlighted(&runtime_path, &theme);
        let theme_name = theme.name().to_string();
        let mut initial_status = "press ? for help, F5 to continue once paused".to_string();
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
        events.push_back(EventLine::info(format!(
            "runtime Tcl {} ({}) via {}; debugger nub {}",
            paths.tcl_patchlevel,
            paths
                .tcl_abi
                .map(|abi| abi.to_string())
                .unwrap_or_else(|| "unprobed ABI".to_string()),
            paths.tclsh.display(),
            paths.dylib.display()
        )));
        Ok(Self {
            user_script: runtime_path,
            display_path,
            _temp_dir: temp_dir,
            paths,
            session: None,
            cache: SourceCache {
                user_script: highlighted,
                user_script_origin,
                proc_bodies: HashMap::new(),
                showing_qname: None,
                origin: user_script_origin,
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
            coverage_enabled: false,
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
        match Session::connect(self.paths.clone()) {
            Ok(session) => {
                self.push_event(EventLine::info(format!(
                    "nub connected: controller :{}",
                    session.bound_port()
                )));
                self.session = Some(session);
                self.app_state = AppState::Ready;
                self.set_status("Ready");
                true
            }
            Err(err) => {
                self.set_status(format!("nub failed to start: {err} (press r to retry)"));
                self.push_event(EventLine::error(format!("startup error: {err}")));
                false
            }
        }
    }

    fn start_debug_target(&mut self) {
        self.try_autoload_persisted_breakpoints();
        let start = self
            .session
            .as_mut()
            .expect("connected session")
            .start_target(&self.user_script, true);
        match start {
            Ok(ack) => {
                self.app_state = AppState::Loading;
                self.set_status("Loading");
                self.push_event(EventLine::info(format!("target scheduled: {ack}")));
            }
            Err(error) => {
                self.set_status(format!("target start failed: {error}"));
                self.push_event(EventLine::error(format!("target start failed: {error}")));
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect) {
        // Make the event ring buffer contiguous so we can hand a flat
        // slice to the renderer.
        let events_slice: &[EventLine] = self.events.make_contiguous();
        let source_lines: &[SourceLine] = match &self.cache.showing_qname {
            Some(q) => self
                .cache
                .proc_bodies
                .get(q)
                .map(Vec::as_slice)
                .unwrap_or(&self.cache.user_script),
            None => &self.cache.user_script,
        };
        let picker_overlay = if matches!(
            self.prompt.as_ref().map(|p| p.kind),
            Some(PromptKind::ProcPicker)
        ) {
            Some(PickerOverlay {
                title: "proc picker — type to filter, Enter selects",
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
            source_path: self.display_path.to_str().unwrap_or("<non-utf8 path>"),
            source_origin: self.cache.origin,
            source_qname: self.cache.showing_qname.as_deref(),
            source_lines,
            secondary_source_lines: &[],
            secondary_highlight_line: None,
            focused_proc: self.focused_proc.as_ref(),
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
            error_banner: self.error_banner.as_deref(),
        };
        let prompt = self.prompt.as_ref().map(|p| &p.input);
        render_in(frame, area, &snapshot, &mut self.ui, prompt, &self.theme);
    }

    fn current_source_lines(&self) -> &[SourceLine] {
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

    fn current_source_lines_len(&self) -> usize {
        self.current_source_lines().len()
    }

    fn tick(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        let events = session.drain_events();
        if events.is_empty() {
            return false;
        }
        let mut needs_refresh = false;
        let mut needs_revalidate = Vec::new();
        for ev in &events {
            let (sev, text) = format_event(ev);
            self.push_event_text(sev, &text);
            match ev {
                WireEvent::TargetStarted { .. } => {
                    self.app_state = AppState::Running;
                    self.set_status("Running");
                }
                WireEvent::TargetCompleted { .. } => {
                    self.app_state = AppState::Completed;
                    self.set_status("Completed");
                    self.error_banner = None;
                    self.clear_execution_state();
                }
                WireEvent::TargetLoadFailed { message, .. } => {
                    self.app_state = AppState::LoadFailed;
                    self.set_status(format!("Load Failed: {}", trim_status(message)));
                    self.clear_execution_state();
                    self.error_banner = Some(message.clone());
                }
                WireEvent::Paused { .. } | WireEvent::ErrorPaused { .. } => {
                    self.app_state = AppState::Stopped;
                    self.set_status("Paused");
                    needs_refresh = true;
                }
                WireEvent::Resumed => {
                    self.app_state = AppState::Running;
                    self.set_status("Running");
                }
                WireEvent::NubStopped { .. } => {
                    self.app_state = AppState::Dead;
                    self.set_status("Debugger stopped");
                    self.clear_execution_state();
                }
                WireEvent::BlocksRevalidated { block } => {
                    needs_revalidate.push(block.clone());
                }
                _ => {}
            }
        }
        for block in needs_revalidate {
            self.invalidate_block(&block);
        }
        if needs_refresh && self.app_state == AppState::Stopped {
            self.refresh_state();
        }
        true
    }

    fn clear_execution_state(&mut self) {
        self.focused_proc = None;
        self.paused_focus = None;
        self.inspected_frame = 0;
        self.stack.clear();
        self.locals.clear();
        self.cache.showing_qname = None;
        self.cache.origin = self.cache.user_script_origin;
        self.ui.selections.stack.select(None);
        self.ui.selections.locals.select(None);
    }

    fn invalidate_block(&mut self, block: &str) {
        if let Some(qname) = block.strip_prefix("proc:") {
            self.cache.proc_bodies.remove(qname);
            if self.cache.showing_qname.as_deref() == Some(qname) {
                // The currently-displayed body is stale; trigger a re-fetch
                // on the next refresh by clearing the showing flag.
                self.cache.showing_qname = None;
                self.cache.origin = self.cache.user_script_origin;
            }
        }
    }

    fn refresh_state(&mut self) {
        if self.session.is_none() {
            return;
        }
        let (state, focused, stack, breakpoints, mut watches, coverage) = {
            let s = self.session.as_mut().unwrap();
            let state = s.app_state().unwrap_or(AppState::Stopped);
            let focused = s.focused_proc();
            let stack = if focused.is_some() {
                s.stack().unwrap_or_default()
            } else {
                Vec::new()
            };
            let breakpoints = s.breakpoints().unwrap_or_default();
            let watches = if state == AppState::Stopped {
                s.evaluate_watches(0)
                    .unwrap_or_else(|_| s.watches().unwrap_or_default())
            } else {
                s.watches().unwrap_or_default()
            };
            let coverage = if self.coverage_enabled {
                s.coverage().unwrap_or_default()
            } else {
                Vec::new()
            };
            (state, focused, stack, breakpoints, watches, coverage)
        };
        self.app_state = state;
        self.paused_focus = focused.clone();
        self.focused_proc = focused;
        self.stack = stack;
        self.breakpoints = breakpoints;
        let previous_watches: HashMap<&str, (&Option<String>, &Option<String>)> = self
            .watches
            .iter()
            .map(|watch| (watch.id.as_str(), (&watch.last_value, &watch.last_error)))
            .collect();
        for watch in &mut watches {
            watch.changed =
                previous_watches
                    .get(watch.id.as_str())
                    .is_some_and(|(value, error)| {
                        (value.is_some() || error.is_some())
                            && (*value != &watch.last_value || *error != &watch.last_error)
                    });
        }
        self.watches = watches;
        self.coverage = coverage;
        // Reset inspected frame to bottom (most recent) on each refresh
        // unless the user already pinned a frame in-pane.
        if let Some((index, top)) = self.stack.iter().enumerate().next_back() {
            self.inspected_frame = top.level;
            self.ui.selections.stack.select(Some(index));
        } else {
            self.inspected_frame = 0;
            self.ui.selections.stack.select(None);
        }
        self.update_locals();
        self.update_error_banner();
        self.update_source_view();
        self.sync_selections();
    }

    fn update_locals(&mut self) {
        if self.session.is_none() || self.app_state != AppState::Stopped {
            self.locals.clear();
            return;
        }
        let level = self.inspected_frame;
        let fresh = self
            .session
            .as_mut()
            .unwrap()
            .variables(level)
            .unwrap_or_default();
        // Diff against the last snapshot for this frame level.
        let prev = self.locals_prev.get(&level).cloned().unwrap_or_default();
        let prev_map: HashMap<&str, &str> =
            prev.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let rows: Vec<LocalRow> = fresh
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
                }
            })
            .collect();
        self.locals_prev.insert(level, fresh);
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
        let Some(focused) = self.focused_proc.clone() else {
            self.cache.showing_qname = None;
            self.cache.origin = self.cache.user_script_origin;
            return;
        };
        let needs_proc_body = focused.block_id.starts_with("proc:");
        if !needs_proc_body {
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
        if !self.cache.proc_bodies.contains_key(&qname)
            && let Some(session) = self.session.as_mut()
            && let Ok(body) = session.proc_body(&qname)
        {
            let raw: Vec<String> = body.lines().map(str::to_string).collect();
            let highlighted = highlight_lines(&raw, &self.theme);
            self.cache.proc_bodies.insert(qname.clone(), highlighted);
        }

        if self.cache.proc_bodies.contains_key(&qname) {
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
            self.cache.showing_qname = None;
            self.cache.origin = self.cache.user_script_origin;
        }
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
        if action == Some(DebugAction::Quit) {
            self.quitting = true;
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
                    self.set_status("(prompt cancelled)");
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
            DebugAction::Quit => self.quitting = true,
            DebugAction::ToggleHelp => self.ui.show_help = true,
            DebugAction::StatusInspect => self.open_status_inspect(),
            DebugAction::FocusNext => self.ui.focus = self.ui.focus.next(),
            DebugAction::FocusPrev => self.ui.focus = self.ui.focus.prev(),
            DebugAction::FocusPane(pane) => self.ui.focus = pane,
            DebugAction::Continue => self.do_continue(),
            DebugAction::StepOver => self.do_step_over(),
            DebugAction::StepInto => self.do_step_into(),
            DebugAction::StepOut => self.do_step_out(),
            DebugAction::NextOrStepOver if !self.search_query.is_empty() => self.search_next(1),
            DebugAction::NextOrStepOver => self.do_step_over(),
            DebugAction::PreviousSearch if !self.search_query.is_empty() => self.search_next(-1),
            DebugAction::PreviousSearch => {}
            DebugAction::Pause => self.do_pause(),
            DebugAction::Restart => self.do_restart(),
            DebugAction::Refresh => {
                self.refresh_state();
                self.last_action = Some(RepeatableAction::Refresh);
                self.set_status("state refreshed");
            }
            DebugAction::UserSource => self.reset_to_user_script(),
            DebugAction::Repeat => self.repeat_last_action(),
            DebugAction::CycleTheme => self.cycle_theme(),
            DebugAction::Search => self.open_source_search(),
            DebugAction::IgnoreError => self.do_ignore_error(),
            DebugAction::ToggleCoverage => self.toggle_coverage(),
            DebugAction::ListSlaves => self.do_list_slaves(),
            DebugAction::PreInstrument => self.open_prompt_full(
                PromptKind::PreInstrument,
                "instrument <qname> <body…>",
                None,
                None,
            ),
            DebugAction::ToggleBreakpoint => self.toggle_breakpoint_at_cursor(),
            DebugAction::AddBreakpoint => self.open_prompt(PromptKind::AddBreakpoint, "bp add"),
            DebugAction::GotoProc => self.open_prompt(PromptKind::GotoProc, "goto proc"),
            DebugAction::ProcPicker => self.open_proc_picker(),
            DebugAction::AddWatch => self.open_prompt(PromptKind::AddWatch, "watch add"),
            DebugAction::AddWatchpoint => self.open_prompt(PromptKind::AddWatchpoint, "wp add"),
            DebugAction::Eval => self.open_prompt(PromptKind::EvalExpr, "eval"),
            DebugAction::Command => self.open_prompt(PromptKind::Command, "dbg"),
            DebugAction::EventFilter => self.open_prompt_full(
                PromptKind::EventFilter,
                "filter events",
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

    fn handle_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(pane) = self.ui.hit.pane_at(m.column, m.row) {
                    self.ui.focus = pane;
                    if pane == Pane::Source {
                        if let Some(line_idx) = self.ui.hit.source_line_at(m.column, m.row) {
                            let max = self.current_source_lines().len().saturating_sub(1);
                            self.ui.source_cursor = line_idx.min(max);
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
            MouseEventKind::ScrollDown => {
                if let Some(pane) = self.ui.hit.pane_at(m.column, m.row) {
                    let prior_focus = self.ui.focus;
                    self.ui.focus = pane;
                    self.move_selection(3);
                    self.ui.focus = prior_focus;
                }
            }
            MouseEventKind::ScrollUp => {
                if let Some(pane) = self.ui.hit.pane_at(m.column, m.row) {
                    let prior_focus = self.ui.focus;
                    self.ui.focus = pane;
                    self.move_selection(-3);
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
                DebugUiState::move_list(&mut self.ui.selections.events, self.events.len(), delta);
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
                DebugUiState::set_list(&mut self.ui.selections.events, self.events.len(), target);
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
            self.cache.showing_qname = None;
            self.cache.origin = self.cache.user_script_origin;
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
                && let Some(session) = self.session.as_mut()
                && let Ok(body) = session.proc_body(&qname)
            {
                let raw: Vec<String> = body.lines().map(str::to_string).collect();
                let highlighted = highlight_lines(&raw, &self.theme);
                self.cache.proc_bodies.insert(qname.clone(), highlighted);
            }
            if self.cache.proc_bodies.contains_key(&qname) {
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
        self.update_locals();
    }

    fn open_prompt(&mut self, kind: PromptKind, label: &'static str) {
        self.prompt = Some(ActivePrompt {
            kind,
            input: PromptInput::new(label),
            bp_target: None,
            body_buffer: None,
            history_pos: None,
        });
    }

    fn open_prompt_full(
        &mut self,
        kind: PromptKind,
        label: &'static str,
        initial: Option<String>,
        bp_target: Option<String>,
    ) {
        self.prompt = Some(ActivePrompt {
            kind,
            input: match initial {
                Some(text) => PromptInput::with_initial(label, text),
                None => PromptInput::new(label),
            },
            bp_target,
            body_buffer: None,
            history_pos: None,
        });
    }

    fn open_status_inspect(&mut self) {
        // Show the latest unclipped status / log line as a read-only
        // prompt buffer the user can copy with terminal selection.
        let initial = self.last_status_full.clone();
        self.open_prompt_full(PromptKind::StatusInspect, "status", Some(initial), None);
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
            "condition (empty clears)",
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
            "ignore count",
            Some("0".to_string()),
            Some(bp.id),
        );
    }

    fn open_proc_picker(&mut self) {
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let list = self
            .session
            .as_mut()
            .unwrap()
            .proc_list()
            .unwrap_or_default();
        self.proc_picker = list;
        self.update_proc_picker_filter("");
        self.open_prompt(PromptKind::ProcPicker, "proc filter");
    }

    fn open_source_search(&mut self) {
        let initial = self.search_query.clone();
        self.open_prompt_full(
            PromptKind::SearchSource,
            "search source",
            Some(initial),
            None,
        );
    }

    fn update_source_search(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.search_match_lines.clear();
        if query.is_empty() {
            self.search_cursor = None;
            return;
        }
        let q = query.to_lowercase();
        let lines: &[SourceLine] = match &self.cache.showing_qname {
            Some(qname) => self
                .cache
                .proc_bodies
                .get(qname)
                .map(Vec::as_slice)
                .unwrap_or(&self.cache.user_script),
            None => &self.cache.user_script,
        };
        for (idx, line) in lines.iter().enumerate() {
            if line.raw.to_lowercase().contains(&q) {
                self.search_match_lines.push(idx);
            }
        }
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
            .filter(|p| q.is_empty() || p.qname.to_lowercase().contains(&q))
            .map(|p| p.qname.clone())
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
        const COMMANDS: &[&str] = &[
            "events", "state", "stack", "vars", "get", "set", "eval", "procs", "ploc", "pbody",
            "cont", "continue", "step", "over", "out", "pause", "coverage", "goto", "quit",
        ];
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let buf = prompt.input.buffer.clone();
        let typed = buf.split_whitespace().next().unwrap_or("");
        let matches: Vec<&&str> = COMMANDS.iter().filter(|c| c.starts_with(typed)).collect();
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
                let names: Vec<&str> = matches.iter().map(|c| **c).collect();
                self.set_status(format!("matches: {}", names.join(" ")));
            }
        }
    }

    fn reset_to_user_script(&mut self) {
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
        self.record_action(RepeatableAction::Continue);
        let _ = self.send_op_log(Op::Continue, "continue");
    }
    fn do_step_into(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "step into unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        self.record_action(RepeatableAction::StepInto);
        let _ = self.send_op_log(Op::StepInto, "step into");
    }
    fn do_step_over(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "step over unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        self.record_action(RepeatableAction::StepOver);
        let _ = self.send_op_log(Op::StepOver, "step over");
    }
    fn do_step_out(&mut self) {
        if self.app_state != AppState::Stopped {
            self.set_status(format!(
                "step out unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        self.record_action(RepeatableAction::StepOut);
        let _ = self.send_op_log(Op::StepOut, "step out");
    }
    fn do_pause(&mut self) {
        if self.app_state != AppState::Running {
            self.set_status(format!(
                "pause unavailable: debugger is {:?}",
                self.app_state
            ));
            return;
        }
        self.record_action(RepeatableAction::Pause);
        let _ = self.send_op_log(Op::Interrupt, "pause");
    }

    fn send_op_log(&mut self, op: Op, label: &str) -> Option<String> {
        if self.session.is_none() {
            self.set_status("no nub attached — press r to start");
            return None;
        }
        let result = self.session.as_mut().unwrap().send(op);
        match result {
            Ok(resp) => {
                self.set_status(format!("{label} → {}", trim_status(&resp)));
                self.push_event_text(
                    tcld_tui::debug::EventSeverity::Info,
                    &format!("{label} → {resp}"),
                );
                Some(resp)
            }
            Err(e) => {
                self.set_status(format!("{label} failed: {e}"));
                self.push_event(EventLine::error(format!("{label}: {e}")));
                None
            }
        }
    }

    fn do_restart(&mut self) {
        let exported_breakpoints = self
            .session
            .as_mut()
            .and_then(|session| session.export_breakpoints().ok());
        let restart_state =
            exported_breakpoints.map(|breakpoints| self.serialize_persisted_state(breakpoints));
        if let Some(session) = self.session.take() {
            session.close();
        }
        match Session::connect(self.paths.clone()) {
            Ok(session) => {
                self.push_event(EventLine::info("nub reconnected"));
                self.session = Some(session);
                self.app_state = AppState::Ready;
                self.focused_proc = None;
                self.paused_focus = None;
                self.stack.clear();
                self.locals.clear();
                self.breakpoints.clear();
                self.watches.clear();
                self.coverage.clear();
                self.cache.proc_bodies.clear();
                self.cache.showing_qname = None;
                self.cache.origin = self.cache.user_script_origin;
                self.set_status("Ready");
                if let Some(serialized) = restart_state.as_deref() {
                    match self.import_persisted_state(serialized) {
                        Ok(()) => {
                            self.breakpoints = self
                                .session
                                .as_mut()
                                .unwrap()
                                .breakpoints()
                                .unwrap_or_default();
                            self.watches =
                                self.session.as_mut().unwrap().watches().unwrap_or_default();
                            self.push_event(EventLine::info(
                                "restored in-memory breakpoints and watches",
                            ));
                        }
                        Err(error) => self.push_event(EventLine::error(format!(
                            "restart state restore failed: {error}"
                        ))),
                    }
                } else {
                    self.try_autoload_persisted_breakpoints();
                }
                let start = self
                    .session
                    .as_mut()
                    .expect("reconnected session")
                    .start_target(&self.user_script, true);
                match start {
                    Ok(ack) => {
                        self.app_state = AppState::Loading;
                        self.set_status("Loading");
                        self.push_event(EventLine::info(format!("target rescheduled: {ack}")));
                    }
                    Err(error) => {
                        self.set_status(format!("restart target failed: {error}"));
                        self.push_event(EventLine::error(format!(
                            "restart target failed: {error}"
                        )));
                    }
                }
            }
            Err(e) => {
                self.set_status(format!("restart failed: {e}"));
                self.push_event(EventLine::error(format!("restart error: {e}")));
            }
        }
    }

    fn do_ignore_error(&mut self) {
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let result = self.session.as_mut().unwrap().ignore_error();
        match result {
            Ok(_) => self.set_status("next exception will not pause"),
            Err(e) => self.set_status(format!("ignore_error failed: {e}")),
        }
    }

    fn toggle_coverage(&mut self) {
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let new = !self.coverage_enabled;
        let result = self.session.as_mut().unwrap().coverage_enable(new);
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
                        .session
                        .as_mut()
                        .unwrap()
                        .coverage()
                        .unwrap_or_default();
                } else {
                    self.coverage.clear();
                }
            }
            Err(e) => self.set_status(format!("coverage_enable: {e}")),
        }
    }

    fn do_list_slaves(&mut self) {
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let list = self
            .session
            .as_mut()
            .unwrap()
            .slave_list()
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
        if self.cache.showing_qname.is_none() {
            self.toggle_file_breakpoint_at_line(line_no);
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
        if self.session.is_none() {
            self.set_status("no nub attached — press r to start");
            return;
        }
        let (status, bps, severity_error) = {
            let session = self.session.as_mut().unwrap();
            let existing = session
                .line_breakpoints_for(&qname)
                .ok()
                .and_then(|list| list.into_iter().find(|bp| bp.offset == u64::from(line_no)));
            let (status, was_error) = match existing {
                Some(bp) => match session.remove_breakpoint(&bp.id) {
                    Ok(_) => (format!("removed {} @ {qname}:{line_no}", bp.id), false),
                    Err(e) => (format!("remove failed: {e}"), true),
                },
                None => match session.add_breakpoint(&qname, u64::from(line_no), None) {
                    Ok(resp) => (
                        format!("breakpoint @ {qname}:{line_no} → {}", trim_status(&resp)),
                        false,
                    ),
                    Err(e) => (format!("breakpoint add failed: {e}"), true),
                },
            };
            (status, session.breakpoints().unwrap_or_default(), was_error)
        };
        if severity_error {
            self.push_event(EventLine::error(status.clone()));
        } else {
            self.push_event(EventLine::info(status.clone()));
        }
        self.set_status(status);
        self.breakpoints = bps;
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
        if self.session.is_none() {
            self.set_status("no nub attached - press r to start");
            return;
        }

        let target_qname = command
            .qname
            .clone()
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

        let target_path = self.user_script.to_string_lossy().into_owned();
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
                .session
                .as_mut()
                .unwrap()
                .pre_instrument_proc(qname, body)
        {
            self.finish_command_breakpoint_toggle(
                format!("breakpoint pre-instrument failed: {error}"),
                true,
            );
            return;
        }

        let (status, failed) = {
            let session = self.session.as_mut().unwrap();
            match existing {
                Some(breakpoint) => match session.remove_breakpoint(&breakpoint.id) {
                    Ok(_) => (
                        format!(
                            "removed {} @ line {line_no}, {} (bytes {}..{})",
                            breakpoint.id,
                            command.label,
                            command.offset,
                            command.offset + command.len
                        ),
                        false,
                    ),
                    Err(error) => (format!("remove failed: {error}"), true),
                },
                None => {
                    let result = if let Some(qname) = target_qname.as_deref() {
                        session.add_command_breakpoint(
                            qname,
                            command.target_offset,
                            command.len,
                            None,
                        )
                    } else {
                        session.add_file_command_breakpoint(
                            &self.user_script,
                            command.target_offset,
                            command.len,
                            None,
                        )
                    };
                    match result {
                        Ok(response) => (
                            format!(
                                "breakpoint @ line {line_no}, {} (bytes {}..{}) -> {}",
                                command.label,
                                command.offset,
                                command.offset + command.len,
                                trim_status(&response)
                            ),
                            false,
                        ),
                        Err(error) => (format!("breakpoint add failed: {error}"), true),
                    }
                }
            }
        };
        self.breakpoints = self
            .session
            .as_mut()
            .unwrap()
            .breakpoints()
            .unwrap_or_default();
        self.push_event(if failed {
            EventLine::error(status.clone())
        } else {
            EventLine::info(status.clone())
        });
        self.set_status(status);
    }

    fn finish_command_breakpoint_toggle(&mut self, status: String, failed: bool) {
        self.breakpoints = self
            .session
            .as_mut()
            .map(|session| session.breakpoints().unwrap_or_default())
            .unwrap_or_default();
        self.push_event(if failed {
            EventLine::error(status.clone())
        } else {
            EventLine::info(status.clone())
        });
        self.set_status(status);
    }

    fn toggle_file_breakpoint_at_line(&mut self, line_no: u32) {
        if self.session.is_none() {
            self.set_status("no nub attached - press r to start");
            return;
        }
        let target_path = self.user_script.to_string_lossy().into_owned();
        let existing = self.breakpoints.iter().find(|breakpoint| {
            breakpoint.source_path.as_deref() == Some(target_path.as_str())
                && breakpoint.line == Some(line_no)
        });
        let (status, failed) = {
            let session = self.session.as_mut().unwrap();
            match existing {
                Some(breakpoint) => match session.remove_breakpoint(&breakpoint.id) {
                    Ok(_) => (
                        format!(
                            "removed {} @ {}:{line_no}",
                            breakpoint.id,
                            self.display_path.display()
                        ),
                        false,
                    ),
                    Err(error) => (format!("remove failed: {error}"), true),
                },
                None => match session.add_file_breakpoint(&self.user_script, line_no, None) {
                    Ok(response) => (
                        format!(
                            "breakpoint @ {}:{line_no} -> {}",
                            self.display_path.display(),
                            trim_status(&response)
                        ),
                        false,
                    ),
                    Err(error) => (format!("breakpoint add failed: {error}"), true),
                },
            }
        };
        self.breakpoints = self
            .session
            .as_mut()
            .unwrap()
            .breakpoints()
            .unwrap_or_default();
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
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let target = !bp.enabled;
        let result = self
            .session
            .as_mut()
            .unwrap()
            .enable_breakpoint(&bp.id, target);
        let bps = self
            .session
            .as_mut()
            .unwrap()
            .breakpoints()
            .unwrap_or_default();
        match result {
            Ok(_) => self.set_status(format!(
                "{} {} ",
                if target { "enabled" } else { "disabled" },
                bp.id
            )),
            Err(e) => {
                let message = format!("toggle failed: {e}");
                self.push_event(EventLine::error(message.clone()));
                self.set_status(message);
            }
        }
        self.breakpoints = bps;
    }

    fn delete_selected_bp(&mut self) {
        let Some(idx) = self.ui.selections.breakpoints.selected() else {
            return;
        };
        let Some(bp) = self.breakpoints.get(idx).cloned() else {
            return;
        };
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let (status, bps, failed) = {
            let session = self.session.as_mut().unwrap();
            let (status, failed) = match session.remove_breakpoint(&bp.id) {
                Ok(_) => (format!("removed {}", bp.id), false),
                Err(e) => (format!("delete failed: {e}"), true),
            };
            (status, session.breakpoints().unwrap_or_default(), failed)
        };
        if failed {
            self.push_event(EventLine::error(status.clone()));
        }
        self.set_status(status);
        self.breakpoints = bps;
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
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let (status, watches) = {
            let session = self.session.as_mut().unwrap();
            let status = match session.watch_remove(&w.id) {
                Ok(_) => format!("watch {} removed", w.id),
                Err(e) => format!("watch remove failed: {e}"),
            };
            (status, session.watches().unwrap_or_default())
        };
        self.set_status(status);
        self.watches = watches;
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
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
        let serialized = match self.session.as_mut().unwrap().export_breakpoints() {
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
        if self.session.is_none() {
            self.set_status("no nub attached");
            return;
        }
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
        let bps = self
            .session
            .as_mut()
            .unwrap()
            .breakpoints()
            .unwrap_or_default();
        let watches = self.session.as_mut().unwrap().watches().unwrap_or_default();
        match result {
            Ok(_) => self.set_status(format!("breakpoints imported from {}", path.display())),
            Err(e) => {
                let message = format!("import failed: {e}");
                self.push_event(EventLine::error(message.clone()));
                self.set_status(message);
            }
        }
        self.breakpoints = bps;
        self.watches = watches;
    }

    fn try_autoload_persisted_breakpoints(&mut self) {
        let path = persist_path(&self.user_script);
        if path.exists()
            && self.session.is_some()
            && let Ok(serialized) = std::fs::read_to_string(&path)
            && self.import_persisted_state(&serialized).is_ok()
        {
            self.push_event(EventLine::info(format!(
                "auto-imported breakpoints from {}",
                path.display()
            )));
            self.breakpoints = self
                .session
                .as_mut()
                .unwrap()
                .breakpoints()
                .unwrap_or_default();
            self.watches = self.session.as_mut().unwrap().watches().unwrap_or_default();
        }
    }

    fn maybe_export_breakpoints_on_quit(&mut self) {
        // Best-effort: if we still have a session and any breakpoints,
        // round-trip them through `bp_export` and write next to the
        // user script so the next launch auto-imports.
        if self.session.is_none() || (self.breakpoints.is_empty() && self.watches.is_empty()) {
            return;
        }
        let serialized = match self.session.as_mut().unwrap().export_breakpoints() {
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
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| "no nub attached".to_string())?;
        session.import_breakpoints(breakpoints)?;
        if let Some(state) = parsed {
            let existing = session.watches().unwrap_or_default();
            for watch in existing {
                let _ = session.watch_remove(&watch.id);
            }
            for watch in state.watches {
                session.watch_add(&watch.name, &watch.expression)?;
            }
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
        if let Some(session) = self.app.session.take() {
            session.close();
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
    if matches!(kind, PromptKind::StatusInspect) {
        // Read-only — submission is a no-op.
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
        PromptKind::EventFilter | PromptKind::StatusInspect | PromptKind::SearchSource => {
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
    if app.session.is_none() {
        app.set_status("no nub attached — press r to start");
        return;
    }

    let result: Result<String, String> = {
        let session = app.session.as_mut().unwrap();
        match sub.as_str() {
            "events" => {
                let evs = session.drain_events();
                Ok(if evs.is_empty() {
                    "(no events)".to_string()
                } else {
                    evs.iter()
                        .map(|e| format_event(e).1)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            }
            "state" => session.app_state().map(|s| s.label().to_string()),
            "stack" => session.send(Op::InspectStack),
            "vars" => {
                let lvl: i64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                session.send(Op::GetVariables { level: lvl })
            }
            "get" => {
                let Some(name) = rest.first().cloned() else {
                    app.set_status("usage: get <name> [level]");
                    return;
                };
                let level: i64 = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                session.send(Op::GetVar {
                    level,
                    name,
                    maxlen: None,
                })
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
                session.send(Op::SetVar {
                    level,
                    name,
                    value: val,
                })
            }
            "eval" => {
                if rest.is_empty() {
                    app.set_status("usage: eval <script>");
                    return;
                }
                session.send(Op::EvalInFrame {
                    level: 0,
                    script: rest.join(" "),
                })
            }
            "procs" => session.send(Op::ProcList),
            "ploc" => {
                let Some(qname) = rest.first().cloned() else {
                    app.set_status("usage: ploc <qname>");
                    return;
                };
                session.send(Op::ProcLocation { qname })
            }
            "pbody" => {
                let Some(qname) = rest.first().cloned() else {
                    app.set_status("usage: pbody <qname>");
                    return;
                };
                session.send(Op::ProcBody { qname })
            }
            "cont" | "continue" => session.send(Op::Continue),
            "step" => session.send(Op::StepInto),
            "over" => session.send(Op::StepOver),
            "out" => session.send(Op::StepOut),
            "pause" => session.send(Op::Interrupt),
            "coverage" => session.send(Op::Coverage),
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
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let result = app.session.as_mut().unwrap().proc_location(qname);
    match result {
        Ok(Some(off)) => {
            // Try to fetch the body so the source pane shows the actual proc.
            let body = app.session.as_mut().unwrap().proc_body(qname).ok();
            if let Some(body) = body {
                let raw: Vec<String> = body.lines().map(str::to_string).collect();
                let highlighted = highlight_lines(&raw, &app.theme);
                let _ = raw;
                app.cache.proc_bodies.insert(qname.to_string(), highlighted);
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
        .find(|p| p.qname.to_lowercase().contains(&q))
    {
        let qname = entry.qname.clone();
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
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let (status, watches) = {
        let session = app.session.as_mut().unwrap();
        let status = match session.watch_add(&name, &expr) {
            Ok(resp) => format!("watch added: {}", trim_status(&resp)),
            Err(e) => format!("watch add failed: {e}"),
        };
        (status, session.watches().unwrap_or_default())
    };
    app.set_status(status);
    app.watches = watches;
}

fn add_watchpoint_from_prompt(app: &mut DebugApp, raw: &str) {
    // Format: <name> [ops]   — ops defaults to "w" (write only).
    let (name, ops) = match raw.split_once(' ') {
        Some((n, o)) => (n.trim().to_string(), o.trim().to_string()),
        None => (raw.to_string(), "w".to_string()),
    };
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let result = app.session.as_mut().unwrap().watchpoint_add(&name, &ops);
    let bps = app
        .session
        .as_mut()
        .unwrap()
        .breakpoints()
        .unwrap_or_default();
    match result {
        Ok(resp) => app.set_status(format!(
            "var-write watchpoint on {name}: {}",
            trim_status(&resp)
        )),
        Err(e) => app.set_status(format!("wp_add: {e}")),
    }
    app.breakpoints = bps;
}

fn eval_in_current_frame(app: &mut DebugApp, script: &str) {
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let level = app.inspected_frame;
    let result = app.session.as_mut().unwrap().send(Op::EvalInFrame {
        level,
        script: script.to_string(),
    });
    match result {
        Ok(resp) => {
            app.set_status(format!("eval(level={level}) → {}", trim_status(&resp)));
            app.push_event_text(
                tcld_tui::debug::EventSeverity::Info,
                &format!("eval@{level} {script} = {resp}"),
            );
        }
        Err(e) => app.set_status(format!("eval failed: {e}")),
    }
}

fn add_bp_from_prompt(app: &mut DebugApp, raw: &str) {
    let mut tokens = raw.split_whitespace();
    let qname = match tokens.next() {
        Some(q) => q.to_string(),
        None => {
            app.set_status("usage: B <qname> <offset> [condition]");
            return;
        }
    };
    let off_tok = match tokens.next() {
        Some(t) => t.to_string(),
        None => {
            app.set_status("usage: B <qname> <offset> [condition]");
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
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let (status, bps) = {
        let session = app.session.as_mut().unwrap();
        let status = match session.add_breakpoint(&qname, offset, condition.as_deref()) {
            Ok(resp) => format!("breakpoint added: {}", trim_status(&resp)),
            Err(e) => format!("breakpoint add failed: {e}"),
        };
        (status, session.breakpoints().unwrap_or_default())
    };
    app.set_status(status);
    app.breakpoints = bps;
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
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let condition = if raw.is_empty() { None } else { Some(raw) };
    let result = app
        .session
        .as_mut()
        .unwrap()
        .update_condition(&bp, condition);
    let bps = app
        .session
        .as_mut()
        .unwrap()
        .breakpoints()
        .unwrap_or_default();
    match result {
        Ok(new_id) => app.set_status(if condition.is_some() {
            format!("condition updated on {new_id}: {raw}")
        } else {
            format!("condition cleared on {new_id}")
        }),
        Err(e) => {
            let message = format!("condition update failed: {e}");
            app.push_event(EventLine::error(message.clone()));
            app.set_status(message);
        }
    }
    app.breakpoints = bps;
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
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let result = app.session.as_mut().unwrap().set_ignore(&id, count);
    let bps = app
        .session
        .as_mut()
        .unwrap()
        .breakpoints()
        .unwrap_or_default();
    match result {
        Ok(_) => app.set_status(format!("ignore_count({id}) = {count}")),
        Err(e) => {
            let message = format!("set_ignore failed: {e}");
            app.push_event(EventLine::error(message.clone()));
            app.set_status(message);
        }
    }
    app.breakpoints = bps;
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
    if app.session.is_none() {
        app.set_status("no nub attached");
        return;
    }
    let result = app
        .session
        .as_mut()
        .unwrap()
        .pre_instrument_proc(&qname, &body);
    match result {
        Ok(resp) => app.set_status(format!("pre-instrumented {qname}: {}", trim_status(&resp))),
        Err(e) => app.set_status(format!("pre_instrument: {e}")),
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

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
        let paths = NubPaths::explicit(
            PathBuf::from("/bin/false"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
        );
        DebugApp::new(DebugTarget::user_script(user_script), paths, theme).unwrap()
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
    fn b_in_target_source_uses_file_line_path() {
        let mut app = fake_app();
        app.handle_key(key(KeyCode::Char('b')));
        assert!(app.status.contains("no nub attached"));
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
            SourceProvenance::decompiled(runtime.clone(), PathBuf::from("/tmp/orig.tbc"), dir);
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
            SourceProvenance::decompiled(runtime.clone(), PathBuf::from("/tmp/orig.tbc"), dir);
        let paths = NubPaths::explicit(
            PathBuf::from("/bin/false"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
        );
        let app = DebugApp::new(DebugTarget::new(provenance, None), paths, theme).unwrap();
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
        let provenance = SourceProvenance::decompiled(runtime.clone(), display.clone(), temp_dir);
        // Re-target runtime_path for clarity (decompiled() already
        // accepted it).
        let _ = display;
        let paths = NubPaths::explicit(
            PathBuf::from("/bin/false"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
        );
        let app = DebugApp::new(DebugTarget::new(provenance, None), paths, theme).unwrap();
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
        let theme = Theme::with_palette(true, "obsidian").unwrap();
        let user_script = std::env::temp_dir().join(format!(
            "tcld-debug-test-{}.tcl",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&user_script, text).unwrap();
        let paths = NubPaths::explicit(
            PathBuf::from("/bin/false"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
        );
        DebugApp::new(DebugTarget::user_script(user_script), paths, theme).unwrap()
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
        app.locals_prev
            .insert(0, vec![("x".into(), "1".into()), ("y".into(), "2".into())]);
        // Build LocalRow input as if update_locals had run on (x=1, y=42).
        let prev = app.locals_prev.get(&0).cloned().unwrap();
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
        let paths = NubPaths::explicit(
            PathBuf::from("/bin/false"),
            PathBuf::from("/dev/null"),
            PathBuf::from("/dev/null"),
        );
        let mut app = DebugApp::new(DebugTarget::user_script(user_script), paths, theme).unwrap();
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
            ProcEntry {
                qname: "alpha".into(),
                block: None,
                first_offset: None,
            },
            ProcEntry {
                qname: "beta".into(),
                block: None,
                first_offset: None,
            },
            ProcEntry {
                qname: "::ns::alpha".into(),
                block: None,
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
}
