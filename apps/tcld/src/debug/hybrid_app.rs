//! Hybrid in-process debug TUI (S10 dual path).
//!
//! Disassembly-primary source pane, Model B instruction step, pc-primary
//! pauses. Does **not** decompile `.tbc` to a temp script target (I1).

#![cfg(feature = "hybrid")]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use tcld_tui::Theme;
use tcld_tui::debug::{
    AppState, CoverageRow, DebugSnapshot, DebugUiState, EventLine, FocusedProc, HitTargets,
    LocalRow, Pane, PauseKind, PauseReasonLabel, ProcBreakpoint, PromptInput, PromptStep,
    SourceBlockKind, SourceLine, SourceOrigin, SourceSearch, StackFrame, WatchEntry, prompt_step,
    render_in,
};
use tcld_tui::terminal::{Event, Frame, Rect, RuntimeAction, RuntimeEvent, TerminalApplication};

use super::backend::{BackendPause, DebugBackend, HybridInProcessBackend};
use tcld_vm::RuntimeMode;

const EVENT_LOG_CAPACITY: usize = 512;
const APP_TITLE: &str = "tcld debug · hybrid";

#[derive(Debug, Clone, Copy)]
enum HybridPromptKind {
    Eval,
    Watch,
}

pub struct HybridDebugApp {
    path: PathBuf,
    backend: HybridInProcessBackend,
    source_lines: Vec<SourceLine>,
    secondary_source_lines: Vec<SourceLine>,
    secondary_highlight_line: Option<u32>,
    /// Absolute PC → 1-based source line (from disassembly).
    pc_to_line: HashMap<u32, u32>,
    /// 1-based line → absolute PC (for toggle BP at cursor).
    line_to_pc: HashMap<u32, u32>,

    app_state: AppState,
    focused_proc: Option<FocusedProc>,
    stack: Vec<StackFrame>,
    locals: Vec<LocalRow>,
    locals_prev: HashMap<String, String>,
    breakpoints: Vec<ProcBreakpoint>,
    watches: Vec<WatchEntry>,
    coverage: Vec<CoverageRow>,
    events: VecDeque<EventLine>,
    status: String,
    error_banner: Option<String>,
    quitting: bool,
    ui: DebugUiState,
    theme: Theme,
    theme_name: String,
    cursor_line: usize, // 0-based into source_lines for scroll/BP
    inspected_frame: usize,
    prompt: Option<(HybridPromptKind, PromptInput)>,
}

impl HybridDebugApp {
    pub fn new(path: PathBuf, mode: RuntimeMode, theme: Theme) -> Result<Self, String> {
        let backend = HybridInProcessBackend::new(mode).map_err(|e| e.to_string())?;
        Ok(Self {
            path,
            backend,
            source_lines: Vec::new(),
            secondary_source_lines: Vec::new(),
            secondary_highlight_line: None,
            pc_to_line: HashMap::new(),
            line_to_pc: HashMap::new(),
            app_state: AppState::Dead,
            focused_proc: None,
            stack: Vec::new(),
            locals: Vec::new(),
            locals_prev: HashMap::new(),
            breakpoints: Vec::new(),
            watches: Vec::new(),
            coverage: Vec::new(),
            events: VecDeque::with_capacity(EVENT_LOG_CAPACITY),
            status: "Starting hybrid TEBC debugger".into(),
            error_banner: None,
            quitting: false,
            ui: DebugUiState::new(),
            theme,
            theme_name: "hybrid".into(),
            cursor_line: 0,
            inspected_frame: 0,
            prompt: None,
        })
    }

    pub fn run(mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.app_state = AppState::Starting;
        tcld_tui::terminal::run(
            "debug-hybrid",
            HybridDebugSurface {
                app: Some(self),
                started: false,
            },
        )
    }

    fn start_debugger(&mut self) {
        let capabilities = self.backend.capabilities();
        self.push_event(EventLine::info(format!(
            "hybrid backend ({:?}): instruction_step=1 nested_frames={} eval_in_frame={} watches={} source_disassembly={}",
            self.backend.mode(),
            capabilities.nested_frames,
            capabilities.eval_in_frame,
            capabilities.watches,
            capabilities.source_disassembly,
        )));
        if let Some(reason) = capabilities.dynamic_eval_reason {
            self.push_event(EventLine::info(reason));
        }

        match self.backend.start_target(&self.path, true) {
            Ok(()) => {
                self.app_state = AppState::Stopped;
                self.ingest_disassembly();
                self.apply_pauses();
                self.refresh_from_backend();
                let instruction_set = self
                    .backend
                    .instruction_set_name()
                    .unwrap_or("unknown opcode table");
                self.set_status(format!(
                    "Paused at entry (instruction step · {instruction_set})"
                ));
                self.push_event(EventLine::info(format!(
                    "decoded with opcode table {instruction_set}"
                )));
                self.push_event(EventLine::pause("stop on entry"));
            }
            Err(e) => {
                self.app_state = AppState::LoadFailed;
                self.error_banner = Some(e.to_string());
                self.set_status(format!("load failed: {e}"));
                self.push_event(EventLine::error(format!("start_target: {e}")));
            }
        }
    }

    fn ingest_disassembly(&mut self) {
        self.source_lines = self
            .backend
            .disassembly_lines()
            .iter()
            .map(|l| SourceLine::raw(l.clone()))
            .collect();
        self.secondary_source_lines = self
            .backend
            .source_lines()
            .iter()
            .map(|line| SourceLine::raw(line.clone()))
            .collect();
        self.pc_to_line.clear();
        self.line_to_pc.clear();
        for (idx, line) in self.backend.disassembly_lines().iter().enumerate() {
            let line_no = (idx + 1) as u32;
            if let Some(rest) = line.trim_start().strip_prefix('(')
                && let Some(end) = rest.find(')')
                && let Ok(abs_pc) = rest[..end].trim().parse::<u32>()
            {
                self.pc_to_line.insert(abs_pc, line_no);
                self.line_to_pc.insert(line_no, abs_pc);
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let events_slice: &[EventLine] = self.events.make_contiguous();
        let empty_search = SourceSearch::default();
        let snapshot = DebugSnapshot {
            title: APP_TITLE,
            source_path: self.path.to_str().unwrap_or("<tbc>"),
            source_origin: SourceOrigin::Synthetic,
            source_qname: None,
            source_lines: &self.source_lines,
            secondary_source_lines: &self.secondary_source_lines,
            secondary_highlight_line: self.secondary_highlight_line,
            focused_proc: self.focused_proc.as_ref(),
            app_state: self.app_state,
            stack: &self.stack,
            inspected_frame: self.inspected_frame as i64,
            locals: &self.locals,
            breakpoints: &self.breakpoints,
            watches: &self.watches,
            events: events_slice,
            event_filter: "",
            coverage: &self.coverage,
            coverage_enabled: true,
            status: &self.status,
            source_search: empty_search,
            picker: None,
            theme_label: &self.theme_name,
            error_banner: self.error_banner.as_deref(),
        };
        let prompt = self.prompt.as_ref().map(|(_, input)| input);
        render_in(frame, area, &snapshot, &mut self.ui, prompt, &self.theme);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.prompt.is_some() {
            let step = {
                let (_, input) = self.prompt.as_mut().expect("prompt checked");
                prompt_step(input, key)
            };
            match step {
                PromptStep::Pending => {}
                PromptStep::Cancel => {
                    self.prompt = None;
                    self.set_status("prompt cancelled");
                }
                PromptStep::Submit(value) => {
                    let (kind, _) = self.prompt.take().expect("prompt checked");
                    self.submit_prompt(kind, value);
                }
            }
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quitting = true;
            return;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quitting = true,
            KeyCode::Char('?') | KeyCode::Char('h') => {
                let capabilities = self.backend.capabilities();
                self.set_status(
                    format!(
                        "hybrid: F11/s=into, F10/n=over, Shift-F11/o=out, c=continue, b=PC bp, B=source bp, e=eval, w=watch, r=restart, q=quit; watches={} eval_in_frame={} coverage=1{}",
                        i32::from(capabilities.watches),
                        i32::from(capabilities.eval_in_frame),
                        capabilities
                            .dynamic_eval_reason
                            .as_deref()
                            .map(|reason| format!(" ({reason})"))
                            .unwrap_or_default(),
                    ),
                );
            }
            KeyCode::F(11) if key.modifiers.contains(KeyModifiers::SHIFT) => self.do_step_out(),
            KeyCode::F(10) | KeyCode::Char('n') => self.do_step_over(),
            KeyCode::F(11) | KeyCode::Char('s') | KeyCode::Enter => self.do_step(),
            KeyCode::Char('c') => self.do_continue(),
            KeyCode::Char('o') => self.do_step_out(),
            KeyCode::Char('r') => self.do_restart(),
            KeyCode::Char('b') => self.toggle_bp_at_cursor(),
            KeyCode::Char('B') => self.toggle_source_bp_at_pause(),
            KeyCode::Char('e') if self.backend.capabilities().eval_in_frame => {
                self.prompt = Some((HybridPromptKind::Eval, PromptInput::new("eval")));
            }
            KeyCode::Char('w') if self.backend.capabilities().watches => {
                self.prompt = Some((HybridPromptKind::Watch, PromptInput::new("watch add")));
            }
            KeyCode::Char('e') | KeyCode::Char('w') => self.set_status(
                self.backend
                    .capabilities()
                    .dynamic_eval_reason
                    .unwrap_or_else(|| "dynamic eval unavailable for selected frame".into()),
            ),
            KeyCode::Char('j') | KeyCode::Down => self.move_active(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_active(-1),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::Home => {
                self.cursor_line = 0;
                self.scroll_to_cursor();
            }
            KeyCode::End => {
                self.cursor_line = self.source_lines.len().saturating_sub(1);
                self.scroll_to_cursor();
            }
            KeyCode::Tab => {
                self.ui.focus = self.ui.focus.next();
            }
            KeyCode::BackTab => self.ui.focus = self.ui.focus.prev(),
            KeyCode::Char('1') => self.ui.focus = Pane::Source,
            KeyCode::Char('2') => self.ui.focus = Pane::Stack,
            KeyCode::Char('3') => self.ui.focus = Pane::Locals,
            KeyCode::Char('4') => self.ui.focus = Pane::Breakpoints,
            KeyCode::Char('5') => self.ui.focus = Pane::Watches,
            KeyCode::Char('6') => self.ui.focus = Pane::Events,
            KeyCode::Char('7') => self.ui.focus = Pane::Coverage,
            _ => {}
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => self.move_active(1),
            MouseEventKind::ScrollUp => self.move_active(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(pane) = self.ui.hit.pane_at(mouse.column, mouse.row) else {
                    return;
                };
                self.ui.focus = pane;
                match pane {
                    Pane::Source => {
                        if let Some(line) = self
                            .ui
                            .hit
                            .source_line_at(mouse.column, mouse.row)
                            .filter(|&line| line < self.source_lines.len())
                        {
                            self.cursor_line = line;
                            self.scroll_to_cursor();
                        }
                    }
                    Pane::Stack => {
                        if let Some(row) =
                            HitTargets::list_row_at(self.ui.hit.stack_inner, mouse.row)
                                .filter(|&row| row < self.stack.len())
                        {
                            self.ui.selections.stack.select(Some(row));
                            self.inspected_frame = row;
                            self.refresh_selected_locals();
                            self.evaluate_watches();
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn submit_prompt(&mut self, kind: HybridPromptKind, value: String) {
        let value = value.trim();
        if value.is_empty() {
            self.set_status("empty prompt ignored");
            return;
        }
        match kind {
            HybridPromptKind::Eval => {
                match self
                    .backend
                    .eval_in_frame(self.inspected_frame as i64, value)
                {
                    Ok((code, result)) => {
                        self.set_status(format!(
                            "eval(level={}) code={code} → {result}",
                            self.inspected_frame
                        ));
                        self.push_event(EventLine::info(format!(
                            "eval@{} {value} = {result} (code {code})",
                            self.inspected_frame
                        )));
                        self.refresh_selected_locals();
                        self.evaluate_watches();
                    }
                    Err(error) => self.set_status(format!("eval failed: {error}")),
                }
            }
            HybridPromptKind::Watch => {
                let (name, expression) = value
                    .split_once(' ')
                    .map(|(name, expression)| (name.trim(), expression.trim()))
                    .unwrap_or((value, value));
                let id = format!("hybrid-watch-{}", self.watches.len() + 1);
                self.watches.push(WatchEntry {
                    id,
                    name: name.to_string(),
                    expr: expression.to_string(),
                    last_value: None,
                    last_error: None,
                    changed: false,
                });
                self.evaluate_watches();
                self.set_status(format!("watch added: {name} = {expression}"));
            }
        }
    }

    fn move_active(&mut self, delta: isize) {
        if self.ui.focus == Pane::Stack {
            DebugUiState::move_list(&mut self.ui.selections.stack, self.stack.len(), delta);
            self.inspected_frame = self.ui.selections.stack.selected().unwrap_or(0);
            self.refresh_selected_locals();
            self.evaluate_watches();
        } else {
            self.move_cursor(delta);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.source_lines.len() as isize;
        if len == 0 {
            return;
        }
        let next = (self.cursor_line as isize + delta).clamp(0, len - 1);
        self.cursor_line = next as usize;
        self.scroll_to_cursor();
    }

    fn scroll_to_cursor(&mut self) {
        // Keep cursor near middle of source pane when possible.
        self.ui.source_cursor = self.cursor_line;
        self.ui.source_view_top = self.cursor_line.saturating_sub(8);
        self.ui.center_source_cursor = true;
    }

    fn do_step(&mut self) {
        if self.app_state == AppState::Completed {
            self.set_status("already completed");
            return;
        }
        match self.backend.step_instruction() {
            Ok(()) => {
                self.apply_pauses();
                self.refresh_from_backend();
                self.set_status("step instruction");
            }
            Err(e) => {
                self.error_banner = Some(e.to_string());
                self.set_status(format!("step failed: {e}"));
                self.push_event(EventLine::error(format!("step: {e}")));
            }
        }
    }

    fn do_step_over(&mut self) {
        if self.app_state == AppState::Completed {
            self.set_status("already completed");
            return;
        }
        match self.backend.step_over_instruction() {
            Ok(()) => {
                self.apply_pauses();
                self.refresh_from_backend();
                self.set_status("step over");
            }
            Err(e) => self.set_status(format!("step over failed: {e}")),
        }
    }

    fn do_continue(&mut self) {
        if self.app_state == AppState::Completed {
            self.set_status("already completed");
            return;
        }
        match self.backend.continue_() {
            Ok(()) => {
                self.apply_pauses();
                self.refresh_from_backend();
                self.set_status("continue");
            }
            Err(e) => {
                self.error_banner = Some(e.to_string());
                self.set_status(format!("continue failed: {e}"));
            }
        }
    }

    fn do_step_out(&mut self) {
        match self.backend.step_out() {
            Ok(()) => {
                self.apply_pauses();
                self.refresh_from_backend();
                self.set_status("step out / continue");
            }
            Err(e) => self.set_status(format!("step out failed: {e}")),
        }
    }

    fn do_restart(&mut self) {
        match self.backend.restart(true) {
            Ok(()) => {
                self.app_state = AppState::Stopped;
                self.error_banner = None;
                self.ingest_disassembly();
                self.apply_pauses();
                self.refresh_from_backend();
                let instruction_set = self
                    .backend
                    .instruction_set_name()
                    .unwrap_or("unknown opcode table");
                self.set_status(format!("restarted · {instruction_set}"));
                self.push_event(EventLine::info("target restarted"));
            }
            Err(e) => {
                self.app_state = AppState::LoadFailed;
                self.error_banner = Some(e.to_string());
                self.set_status(format!("restart failed: {e}"));
            }
        }
    }

    fn toggle_bp_at_cursor(&mut self) {
        let line_no = (self.cursor_line + 1) as u32;
        let Some(&pc) = self.line_to_pc.get(&line_no) else {
            self.set_status("no TEBC PC on this disassembly line");
            return;
        };
        if self.backend.pc_breakpoints().contains(&pc) {
            let _ = self.backend.remove_pc_breakpoint(pc as u64);
            self.set_status(format!("cleared PC breakpoint @{pc}"));
            self.push_event(EventLine::info(format!("bp clear pc={pc}")));
        } else {
            let _ = self.backend.add_pc_breakpoint(pc as u64);
            self.set_status(format!("set PC breakpoint @{pc}"));
            self.push_event(EventLine::info(format!("bp add pc={pc}")));
        }
        self.rebuild_breakpoint_rows();
    }

    fn toggle_source_bp_at_pause(&mut self) {
        let Some(line) = self.secondary_highlight_line else {
            self.set_status("no mapped original-source line at this pause");
            return;
        };
        match self.backend.toggle_source_breakpoint(line) {
            Ok(addresses) if addresses.is_empty() => {
                self.set_status(format!("pending source breakpoint at line {line}"));
            }
            Ok(addresses) => {
                self.set_status(format!(
                    "toggled source line {line} at {} qualified address(es)",
                    addresses.len()
                ));
            }
            Err(error) => self.set_status(format!("source breakpoint failed: {error}")),
        }
        self.rebuild_breakpoint_rows();
    }

    fn apply_pauses(&mut self) {
        let pauses = self.backend.drain_pauses();
        for p in pauses {
            self.on_pause(p);
        }
    }

    fn on_pause(&mut self, p: BackendPause) {
        if p.reason == "Completed" {
            self.app_state = AppState::Completed;
            self.secondary_highlight_line = None;
            self.focused_proc = None;
            self.stack.clear();
            self.locals.clear();
            let result = self.backend.last_result().unwrap_or("");
            self.set_status(if result.is_empty() {
                "Completed".into()
            } else {
                format!("Completed → {result}")
            });
            self.push_event(EventLine::info("target completed"));
            return;
        }

        self.app_state = AppState::Stopped;
        self.ingest_disassembly();
        self.secondary_highlight_line = p.source_line;
        let line =
            p.pc.and_then(|pc| self.pc_to_line.get(&pc).copied())
                .or_else(|| p.pc.and_then(|pc| self.backend.line_for_pc(pc)))
                .unwrap_or(0);

        if line > 0 {
            self.cursor_line = (line as usize).saturating_sub(1);
            self.scroll_to_cursor();
        }

        let qname = p.proc_qname.clone().unwrap_or_default();
        self.focused_proc = Some(FocusedProc {
            block_id: if qname.is_empty() {
                "toplevel".into()
            } else {
                format!("proc:{qname}")
            },
            block_kind: if qname.is_empty() {
                SourceBlockKind::File
            } else {
                SourceBlockKind::Proc
            },
            source_path: Some(self.path.display().to_string()),
            qname: qname.clone(),
            line,
            offset_start: p.pc.map(|pc| pc as u64).unwrap_or(0),
            offset_end: p.pc.map(|pc| pc as u64).unwrap_or(0),
            pause_reason: if p.reason.contains("Breakpoint") {
                PauseReasonLabel::InstructionBreakpoint
            } else if p.reason.contains("Entry") {
                PauseReasonLabel::Entry
            } else {
                PauseReasonLabel::InstructionStep
            },
            frame_count: 1,
            breakpoint: None,
            pause_kind: PauseKind::Normal,
            error_message: None,
        });

        // Locals from pause payload
        let mut new_locals = Vec::new();
        for (name, value) in &p.locals {
            let changed = self
                .locals_prev
                .get(name)
                .map(|prev| prev != value)
                .unwrap_or(false);
            new_locals.push(LocalRow {
                name: name.clone(),
                value: value.clone(),
                changed,
            });
        }
        self.locals_prev = p.locals.iter().cloned().collect();
        self.locals = new_locals;

        self.push_event(EventLine::pause(format!(
            "pause pc={:?} line={line} in_host={} reason={}",
            p.pc, p.in_host, p.reason
        )));
        // Location must stay None on hybrid (Model B) — we never set a source Location.
        debug_assert!(p.location.is_none());
        debug_assert!(!p.in_host || p.reason == "Completed");
    }

    fn refresh_from_backend(&mut self) {
        if let Ok(state) = self.backend.app_state() {
            self.app_state = state;
        }
        if let Ok(stack) = self.backend.stack() {
            self.stack = stack;
            if self.stack.is_empty() {
                self.inspected_frame = 0;
                self.ui.selections.stack.select(None);
            } else {
                self.inspected_frame = self.inspected_frame.min(self.stack.len() - 1);
                self.ui.selections.stack.select(Some(self.inspected_frame));
            }
            if let Some(fp) = self.focused_proc.as_mut() {
                fp.frame_count = self.stack.len() as u32;
            }
        }
        self.refresh_selected_locals();
        self.evaluate_watches();
        let coverage = self.backend.coverage_snapshot();
        self.coverage = coverage
            .executed_by_opcode
            .into_iter()
            .map(|(opcode, hits)| CoverageRow {
                block: opcode,
                offset: 0,
                hits,
            })
            .collect();
        self.rebuild_breakpoint_rows();
    }

    fn refresh_selected_locals(&mut self) {
        let Ok(vars) = self.backend.variables(self.inspected_frame as i64) else {
            return;
        };
        let previous: HashMap<&str, &str> = self
            .locals
            .iter()
            .map(|local| (local.name.as_str(), local.value.as_str()))
            .collect();
        let locals = vars
            .into_iter()
            .map(|(name, value)| LocalRow {
                changed: previous
                    .get(name.as_str())
                    .is_some_and(|old| **old != value),
                name,
                value,
            })
            .collect();
        self.locals = locals;
    }

    fn evaluate_watches(&mut self) {
        if self.app_state != AppState::Stopped || !self.backend.capabilities().watches {
            return;
        }
        for watch in &mut self.watches {
            let old_value = watch.last_value.clone();
            let old_error = watch.last_error.clone();
            match self
                .backend
                .expr_in_frame(self.inspected_frame as i64, &watch.expr)
            {
                Ok((0, value)) => {
                    watch.last_value = Some(value);
                    watch.last_error = None;
                }
                Ok((code, result)) => {
                    watch.last_value = None;
                    watch.last_error = Some(format!("code {code}: {result}"));
                }
                Err(error) => {
                    watch.last_value = None;
                    watch.last_error = Some(error.to_string());
                }
            }
            watch.changed = old_value != watch.last_value || old_error != watch.last_error;
        }
    }

    fn rebuild_breakpoint_rows(&mut self) {
        let mut breakpoints: Vec<ProcBreakpoint> = self
            .backend
            .pc_breakpoints()
            .iter()
            .map(|&pc| {
                let line = self.pc_to_line.get(&pc).copied();
                let code_id = self
                    .backend
                    .current_code_id()
                    .map(|id| id.0)
                    .unwrap_or_default();
                ProcBreakpoint {
                    id: format!("code:{code_id}:pc:{pc}"),
                    qname: "tebc".into(),
                    source_path: Some(self.path.display().to_string()),
                    line,
                    pending: false,
                    offset: pc as u64,
                    len: 1,
                    enabled: true,
                    hits: 0,
                    ignore_count: 0,
                    condition: None,
                    kind: tcld_tui::debug::BreakpointKindLabel::Line,
                }
            })
            .collect();
        breakpoints.extend(self.backend.pending_source_breakpoints().iter().map(
            |(&line, reason)| ProcBreakpoint {
                id: format!("source-pending:{line}"),
                qname: format!("source: {reason}"),
                source_path: Some(self.path.display().to_string()),
                line: Some(line),
                pending: true,
                offset: 0,
                len: 1,
                enabled: true,
                hits: 0,
                ignore_count: 0,
                condition: None,
                kind: tcld_tui::debug::BreakpointKindLabel::Line,
            },
        ));
        self.breakpoints = breakpoints;
    }

    fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    fn push_event(&mut self, line: EventLine) {
        if self.events.len() >= EVENT_LOG_CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(line);
    }
}

struct HybridDebugSurface {
    app: Option<HybridDebugApp>,
    started: bool,
}

impl Drop for HybridDebugSurface {
    fn drop(&mut self) {
        if let Some(app) = self.app.take() {
            Box::new(app.backend).close();
        }
    }
}

impl TerminalApplication for HybridDebugSurface {
    type Exit = ();

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if let Some(app) = self.app.as_mut() {
            app.draw(frame, area);
        }
    }

    fn handle(&mut self, event: RuntimeEvent) -> RuntimeAction<Self::Exit> {
        let Some(app) = self.app.as_mut() else {
            return RuntimeAction::Exit(());
        };
        let changed = match event {
            RuntimeEvent::Tick(_) if !self.started => {
                self.started = true;
                app.start_debugger();
                true
            }
            RuntimeEvent::Tick(_) => false,
            RuntimeEvent::Terminal(Event::Key(key)) => {
                app.handle_key(key);
                true
            }
            RuntimeEvent::Terminal(Event::Mouse(mouse)) => {
                app.handle_mouse(mouse);
                true
            }
            RuntimeEvent::Terminal(Event::Paste(text)) => {
                let Some((_, input)) = app.prompt.as_mut() else {
                    return RuntimeAction::Ignored;
                };
                let cursor = input.cursor.min(input.buffer.len());
                input.buffer.insert_str(cursor, &text);
                input.cursor = cursor + text.len();
                true
            }
            RuntimeEvent::Terminal(Event::Resize(..)) => true,
            _ => false,
        };

        if app.quitting {
            RuntimeAction::Exit(())
        } else if changed {
            RuntimeAction::Redraw
        } else {
            RuntimeAction::Ignored
        }
    }
}

/// Entry used by `run_debug` when hybrid feature is linked and target is `.tbc`.
pub fn run_hybrid_debug_tui(
    path: &Path,
    mode: RuntimeMode,
    theme: Theme,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = HybridDebugApp::new(path.to_path_buf(), mode, theme)?;
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn rendered_text(app: &mut HybridDebugApp, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| app.draw(frame, frame.area()))
            .expect("render hybrid debugger");
        let buffer = terminal.backend().buffer();
        let width = usize::from(width);
        let mut text = String::new();
        for row in 0..usize::from(height) {
            let start = row * width;
            let end = start + width;
            text.extend(
                buffer.content()[start..end]
                    .iter()
                    .map(|cell| cell.symbol()),
            );
            text.push('\n');
        }
        text
    }

    #[test]
    fn tcl9_opcode_table_and_mnemonic_reach_the_rendered_debugger() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/bytecode/tcl9/tc1.tbc");
        let theme = Theme::with_palette(false, "obsidian").expect("debug theme");
        let mut app =
            HybridDebugApp::new(path, RuntimeMode::VmStrict, theme).expect("hybrid debugger");

        app.start_debugger();
        let text = rendered_text(&mut app, 160, 40);

        assert!(
            text.contains("returnImm"),
            "Tcl 9 mnemonic must reach the visible disassembly pane:\n{text}"
        );
        assert!(
            text.contains("tcl9.0"),
            "the selected opcode table must be visible in debugger status:\n{text}"
        );
        assert!(
            !text.contains("lappendArrayStk"),
            "the Tcl 8.6 meaning for byte 92 leaked into the Tcl 9 debugger:\n{text}"
        );
    }
}
