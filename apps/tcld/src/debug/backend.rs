//! `DebugBackend` trait + hybrid / wire adapters (executable plan S10).
//!
//! Hybrid path never decompiles bytecode to a temp script target (I1).
//! Wire path remains the existing nub Session adapter.

use std::path::Path;
#[cfg(feature = "hybrid")]
use std::path::PathBuf;

use tcld_tui::debug::{AppState, StackFrame};

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
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write;
                let _ = write!(quoted, "\\U{:08x}", character as u32);
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// Capabilities advertised to the TUI.
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    pub instruction_step: bool,
    pub nested_step: bool,
    pub step_over: bool,
    pub step_out: bool,
    pub command_step: bool,
    pub run_to_pc: bool,
    pub pc_breakpoints: bool,
    pub code_qualified_breakpoints: bool,
    pub watches: bool,
    pub eval_in_frame: bool,
    pub dynamic_eval_reason: Option<String>,
    pub nested_frames: bool,
    pub source_disassembly: bool,
    pub coverage: bool,
    pub proc_list: bool,
    pub bytecode_direct: bool,
}

impl BackendCapabilities {
    #[cfg(feature = "hybrid")]
    pub fn hybrid(mode: tcld_vm::RuntimeMode) -> Self {
        let dynamic_eval = mode == tcld_vm::RuntimeMode::Hybrid;
        Self {
            instruction_step: true,
            nested_step: true,
            step_over: true,
            step_out: true,
            command_step: false,
            run_to_pc: true,
            pc_breakpoints: true,
            code_qualified_breakpoints: true,
            watches: dynamic_eval,
            eval_in_frame: dynamic_eval,
            dynamic_eval_reason: (!dynamic_eval).then(|| "dynamic_eval denied by vm-strict".into()),
            nested_frames: true,
            source_disassembly: true,
            coverage: true,
            proc_list: false,
            bytecode_direct: true,
        }
    }

    pub fn wire_host() -> Self {
        Self {
            instruction_step: false, // stock shells
            nested_step: true,
            step_over: true,
            step_out: true,
            command_step: true,
            run_to_pc: false,
            pc_breakpoints: false,
            code_qualified_breakpoints: false,
            watches: true,
            eval_in_frame: true,
            dynamic_eval_reason: None,
            nested_frames: true,
            source_disassembly: true,
            coverage: true,
            proc_list: true,
            bytecode_direct: false,
        }
    }
}

#[derive(Debug)]
pub enum BackendError {
    Unsupported { op: &'static str, reason: String },
    Message(String),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { op, reason } => write!(f, "unsupported {op}: {reason}"),
            Self::Message(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<String> for BackendError {
    fn from(s: String) -> Self {
        Self::Message(s)
    }
}

/// Pause event drained by the UI (backend-neutral).
#[derive(Debug, Clone)]
pub struct BackendPause {
    pub pc: Option<u32>,
    /// Always None on hybrid TEBC (Model B).
    pub location: Option<()>,
    pub in_host: bool,
    pub reason: String,
    pub proc_qname: Option<String>,
    pub source_line: Option<u32>,
    pub locals: Vec<(String, String)>,
}

/// App-level debug backend (design §10.2).
pub trait DebugBackend {
    fn capabilities(&self) -> BackendCapabilities;
    fn start_target(&mut self, path: &Path, stop_on_entry: bool) -> Result<(), BackendError>;
    fn continue_(&mut self) -> Result<(), BackendError>;
    fn step_instruction(&mut self) -> Result<(), BackendError>;
    fn step_over_instruction(&mut self) -> Result<(), BackendError>;
    fn step_out(&mut self) -> Result<(), BackendError>;
    fn run_to_pc(&mut self, pc: u64) -> Result<(), BackendError>;
    fn stack(&mut self) -> Result<Vec<StackFrame>, BackendError>;
    fn variables(&mut self, level: i64) -> Result<Vec<(String, String)>, BackendError>;
    fn eval_in_frame(&mut self, level: i64, script: &str) -> Result<(i32, String), BackendError>;
    fn expr_in_frame(
        &mut self,
        level: i64,
        expression: &str,
    ) -> Result<(i32, String), BackendError>;
    fn add_pc_breakpoint(&mut self, pc: u64) -> Result<(), BackendError>;
    fn remove_pc_breakpoint(&mut self, pc: u64) -> Result<(), BackendError>;
    fn drain_pauses(&mut self) -> Vec<BackendPause>;
    fn app_state(&mut self) -> Result<AppState, BackendError>;
    fn close(self: Box<Self>);
}

// ── Hybrid in-process backend ────────────────────────────────────────

/// In-process hybrid TEBC debugger. Loads `.tbc` only (I1: no temp script target).
#[cfg(feature = "hybrid")]
pub struct HybridInProcessBackend {
    mode: tcld_vm::RuntimeMode,
    engine: tcld_vm::HybridEngine,
    target: Option<PathBuf>,
    pauses: Vec<BackendPause>,
    finished: bool,
    last_result: Option<String>,
    /// Disassembly text for the current target (source pane).
    disassembly: Vec<String>,
    source: Vec<String>,
    source_text: Option<String>,
    /// Absolute TEBC PC → 1-based disassembly line.
    pc_to_line: std::collections::HashMap<u32, u32>,
    current_code_id: Option<tcld_vm::load::CodeId>,
    current_bytecode: Option<tcld_core::ByteCode>,
    current_proc_qname: Option<String>,
    /// Code-qualified breakpoints currently armed (for UI).
    pc_breakpoints: std::collections::BTreeSet<tcld_vm::CodeAddress>,
    pending_source_breakpoints: std::collections::BTreeMap<u32, String>,
}

#[cfg(feature = "hybrid")]
impl HybridInProcessBackend {
    pub fn new(mode: tcld_vm::RuntimeMode) -> Result<Self, BackendError> {
        let mut engine =
            tcld_vm::HybridEngine::new(mode).map_err(|e| BackendError::Message(e.to_string()))?;
        engine
            .bootstrap(std::env::args().next().as_deref())
            .map_err(|e| BackendError::Message(e.to_string()))?;
        Ok(Self {
            mode,
            engine,
            target: None,
            pauses: Vec::new(),
            finished: false,
            last_result: None,
            disassembly: Vec::new(),
            source: Vec::new(),
            source_text: None,
            pc_to_line: std::collections::HashMap::new(),
            current_code_id: None,
            current_bytecode: None,
            current_proc_qname: None,
            pc_breakpoints: std::collections::BTreeSet::new(),
            pending_source_breakpoints: std::collections::BTreeMap::new(),
        })
    }

    pub fn mode(&self) -> tcld_vm::RuntimeMode {
        self.mode
    }

    pub fn target_path(&self) -> Option<&Path> {
        self.target.as_deref()
    }

    pub fn disassembly_lines(&self) -> &[String] {
        &self.disassembly
    }

    pub fn source_lines(&self) -> &[String] {
        &self.source
    }

    pub fn line_for_pc(&self, pc: u32) -> Option<u32> {
        self.pc_to_line.get(&pc).copied()
    }

    pub fn pc_breakpoints(&self) -> std::collections::BTreeSet<u32> {
        let current = self.current_code_id;
        self.pc_breakpoints
            .iter()
            .filter(|address| Some(address.code_id) == current)
            .map(|address| address.pc)
            .collect()
    }

    pub fn current_code_id(&self) -> Option<tcld_vm::load::CodeId> {
        self.current_code_id
    }

    pub fn instruction_set_name(&self) -> Option<&'static str> {
        self.current_bytecode
            .as_ref()?
            .instruction_set()
            .ok()
            .map(|set| set.name)
    }

    pub fn qualified_breakpoints(&self) -> &std::collections::BTreeSet<tcld_vm::CodeAddress> {
        &self.pc_breakpoints
    }

    pub fn pending_source_breakpoints(&self) -> &std::collections::BTreeMap<u32, String> {
        &self.pending_source_breakpoints
    }

    pub fn toggle_source_breakpoint(
        &mut self,
        line: u32,
    ) -> Result<Vec<tcld_vm::CodeAddress>, BackendError> {
        let addresses = self.source_addresses_for_line(line);
        if addresses.is_empty() {
            self.pending_source_breakpoints.insert(
                line,
                "no command location for this source line in the current code object".into(),
            );
            return Ok(Vec::new());
        }
        self.pending_source_breakpoints.remove(&line);
        let removing = addresses
            .iter()
            .all(|address| self.pc_breakpoints.contains(address));
        for address in &addresses {
            if removing {
                self.engine.remove_breakpoint(*address);
                self.pc_breakpoints.remove(address);
            } else {
                self.engine.add_breakpoint(*address);
                self.pc_breakpoints.insert(*address);
            }
        }
        Ok(addresses)
    }

    pub fn add_command_breakpoint(
        &mut self,
        command_index: usize,
    ) -> Result<tcld_vm::CodeAddress, BackendError> {
        let code_id = self
            .current_code_id
            .ok_or_else(|| BackendError::Message("no current TEBC code object".into()))?;
        let pc = self
            .current_bytecode
            .as_ref()
            .and_then(|bytecode| bytecode.cmd_code_ranges.get(command_index))
            .and_then(|range| u32::try_from(range.start).ok())
            .ok_or_else(|| {
                BackendError::Message(format!(
                    "command index {command_index} has no current code address"
                ))
            })?;
        let address = tcld_vm::CodeAddress { code_id, pc };
        self.engine.add_breakpoint(address);
        self.pc_breakpoints.insert(address);
        Ok(address)
    }

    pub fn last_result(&self) -> Option<&str> {
        self.last_result.as_deref()
    }

    pub fn coverage_snapshot(&self) -> tcld_vm::CoverageSnapshot {
        self.engine.coverage_snapshot()
    }

    /// Restart target from scratch (new engine + bootstrap).
    pub fn restart(&mut self, stop_on_entry: bool) -> Result<(), BackendError> {
        let path = self
            .target
            .clone()
            .ok_or_else(|| BackendError::Message("no target".into()))?;
        let mode = self.mode;
        *self = Self::new(mode)?;
        self.start_target(&path, stop_on_entry)
    }

    fn load_disassembly(&mut self, path: &Path) -> Result<(), BackendError> {
        let path_str = path
            .to_str()
            .ok_or_else(|| BackendError::Message("path not UTF-8".into()))?;
        let text = tcld_decompile::disassemble_path(path_str)
            .map_err(|e| BackendError::Message(format!("disassemble: {e}")))?;
        self.disassembly.clear();
        self.source.clear();
        self.source_text = None;
        self.pc_to_line.clear();
        for (idx, line) in text.lines().enumerate() {
            let line_no = (idx + 1) as u32;
            self.disassembly.push(line.to_string());
            // Lines look like: "   (12) push1 0             # ..."
            if let Some(rest) = line.trim_start().strip_prefix('(')
                && let Some(end) = rest.find(')')
                && let Ok(pc) = rest[..end].trim().parse::<u32>()
            {
                self.pc_to_line.insert(pc, line_no);
            }
        }
        let source_path = path.with_extension("tcl");
        if let Ok(source) = std::fs::read_to_string(source_path) {
            self.source.extend(source.lines().map(str::to_owned));
            self.source_text = Some(source);
        }
        Ok(())
    }

    fn refresh_current_disassembly(&mut self) {
        let Some((code_id, bytecode)) = self.engine.current_code() else {
            return;
        };
        if self.current_code_id == Some(code_id) {
            return;
        }
        let Ok(text) = tcld_decompile::disassemble_bytecode(
            &bytecode,
            tcld_decompile::DisassembleOptions::default(),
        ) else {
            return;
        };
        self.current_code_id = Some(code_id);
        self.current_bytecode = Some(bytecode.clone());
        self.disassembly.clear();
        self.pc_to_line.clear();
        for (index, line) in text.lines().enumerate() {
            let line_no = (index + 1) as u32;
            self.disassembly.push(line.to_string());
            if let Some(rest) = line.trim_start().strip_prefix('(')
                && let Some(end) = rest.find(')')
                && let Ok(pc) = rest[..end].trim().parse::<u32>()
            {
                self.pc_to_line.insert(pc, line_no);
            }
        }
        self.resolve_pending_source_breakpoints();
    }

    fn source_base_for_proc(&self, qname: Option<&str>) -> Option<usize> {
        let source = self.source_text.as_deref()?;
        let Some(qname) = qname else {
            return Some(0);
        };
        let leaf = qname.rsplit("::").next().unwrap_or(qname);
        let declaration = source.lines().enumerate().find_map(|(index, line)| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("proc ")?.trim_start();
            let name = rest.split_whitespace().next()?;
            (name == qname || name.trim_start_matches("::") == leaf).then_some(index + 1)
        })?;
        Some(
            source
                .lines()
                .take(declaration)
                .map(|line| line.len() + 1)
                .sum(),
        )
    }

    fn source_addresses_for_line(&self, line: u32) -> Vec<tcld_vm::CodeAddress> {
        let (Some(code_id), Some(bytecode), Some(source)) = (
            self.current_code_id,
            self.current_bytecode.as_ref(),
            self.source_text.as_deref(),
        ) else {
            return Vec::new();
        };
        let Some(source_ranges) = bytecode.cmd_source_ranges.as_ref() else {
            return Vec::new();
        };
        let Some(base) = self.source_base_for_proc(self.current_proc_qname.as_deref()) else {
            return Vec::new();
        };
        let mut addresses = bytecode
            .cmd_code_ranges
            .iter()
            .zip(source_ranges)
            .filter_map(|(code_range, source_range)| {
                let offset = base.saturating_add(usize::try_from(source_range.start).ok()?);
                let absolute = offset.min(source.len());
                let mapped_line = source.as_bytes()[..absolute]
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count() as u32
                    + 1;
                if mapped_line != line {
                    return None;
                }
                Some(tcld_vm::CodeAddress {
                    code_id,
                    pc: u32::try_from(code_range.start).ok()?,
                })
            })
            .collect::<Vec<_>>();
        addresses.sort();
        addresses.dedup();
        addresses
    }

    fn resolve_pending_source_breakpoints(&mut self) {
        let pending = self
            .pending_source_breakpoints
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for line in pending {
            let addresses = self.source_addresses_for_line(line);
            if addresses.is_empty() {
                continue;
            }
            for address in addresses {
                self.engine.add_breakpoint(address);
                self.pc_breakpoints.insert(address);
            }
            self.pending_source_breakpoints.remove(&line);
        }
    }

    fn source_line_for_frame(&self, frame: &tcld_vm::UiStackFrame) -> Option<u32> {
        let offset = frame.source_offset?;
        let source = self.source_text.as_deref()?;
        let base = self.source_base_for_proc(frame.proc_qname.as_deref())?;
        let absolute = base.saturating_add(offset).min(source.len());
        Some(
            source.as_bytes()[..absolute]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32
                + 1,
        )
    }

    fn push_outcome(&mut self, outcome: tcld_vm::DebugOutcome) {
        match outcome {
            tcld_vm::DebugOutcome::Paused(s) => {
                self.current_proc_qname = s.proc_qname.clone();
                self.refresh_current_disassembly();
                let source_line = s
                    .frames
                    .first()
                    .and_then(|frame| self.source_line_for_frame(frame));
                self.pauses.push(BackendPause {
                    pc: s.pc,
                    location: s.location,
                    in_host: s.in_host,
                    reason: format!("{:?}", s.reason),
                    proc_qname: s.proc_qname,
                    source_line,
                    locals: s.locals,
                });
            }
            tcld_vm::DebugOutcome::Finished { result } => {
                self.finished = true;
                self.last_result = result.as_string().ok();
                self.pauses.push(BackendPause {
                    pc: None,
                    location: None,
                    in_host: false,
                    reason: "Completed".into(),
                    proc_qname: None,
                    source_line: None,
                    locals: vec![],
                });
            }
        }
    }
}

#[cfg(feature = "hybrid")]
impl DebugBackend for HybridInProcessBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::hybrid(self.mode)
    }

    fn start_target(&mut self, path: &Path, stop_on_entry: bool) -> Result<(), BackendError> {
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("tbc"))
        {
            return Err(BackendError::Message(
                "hybrid debug requires a .tbc target".into(),
            ));
        }
        self.target = Some(path.to_path_buf());
        self.finished = false;
        self.pc_breakpoints.clear();
        self.pending_source_breakpoints.clear();
        self.current_code_id = None;
        self.current_bytecode = None;
        self.current_proc_qname = None;
        self.load_disassembly(path)?;
        let outcome = self
            .engine
            .debug_start_tbc_path(path, stop_on_entry)
            .map_err(|e| BackendError::Message(e.to_string()))?;
        self.push_outcome(outcome);
        Ok(())
    }

    fn continue_(&mut self) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        let outcome = self
            .engine
            .continue_run()
            .map_err(|e| BackendError::Message(e.to_string()))?;
        self.push_outcome(outcome);
        Ok(())
    }

    fn step_instruction(&mut self) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        let outcome = self
            .engine
            .step_instruction()
            .map_err(|e| BackendError::Message(e.to_string()))?;
        self.push_outcome(outcome);
        Ok(())
    }

    fn step_over_instruction(&mut self) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        let outcome = self
            .engine
            .step_over()
            .map_err(|e| BackendError::Message(e.to_string()))?;
        self.push_outcome(outcome);
        Ok(())
    }

    fn step_out(&mut self) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        let outcome = self
            .engine
            .step_out()
            .map_err(|e| BackendError::Message(e.to_string()))?;
        self.push_outcome(outcome);
        Ok(())
    }

    fn run_to_pc(&mut self, pc: u64) -> Result<(), BackendError> {
        if self.finished {
            return Ok(());
        }
        let outcome = self
            .engine
            .run_to_pc(pc as u32)
            .map_err(|e| BackendError::Message(e.to_string()))?;
        self.push_outcome(outcome);
        Ok(())
    }

    fn stack(&mut self) -> Result<Vec<StackFrame>, BackendError> {
        let frames = self.engine.ui_stack_frames();
        Ok(frames
            .into_iter()
            .enumerate()
            .map(|(i, f)| StackFrame {
                level: i as i64,
                kind: f.kind_label,
                proc_name: f.proc_qname.clone(),
                block: f.proc_qname.clone(),
                block_kind: None,
                source_path: None,
                line: None,
                offset_start: f.pc.map(|p| p as u64),
                offset_end: f.pc.map(|p| p as u64),
            })
            .collect())
    }

    fn variables(&mut self, level: i64) -> Result<Vec<(String, String)>, BackendError> {
        Ok(self.engine.ui_frame_locals(level.max(0) as usize))
    }

    fn eval_in_frame(&mut self, level: i64, script: &str) -> Result<(i32, String), BackendError> {
        let completion = self
            .engine
            .eval_in_frame(level.max(0) as usize, tcld_vm::VmObj::from_str(script))
            .map_err(|error| BackendError::Message(error.to_string()))?;
        Ok((
            completion.code,
            completion
                .result
                .as_string()
                .unwrap_or_else(|_| format!("{:?}", completion.result)),
        ))
    }

    fn expr_in_frame(
        &mut self,
        level: i64,
        expression: &str,
    ) -> Result<(i32, String), BackendError> {
        let completion = self
            .engine
            .expr_in_frame(level.max(0) as usize, tcld_vm::VmObj::from_str(expression))
            .map_err(|error| BackendError::Message(error.to_string()))?;
        Ok((
            completion.code,
            completion
                .result
                .as_string()
                .unwrap_or_else(|_| format!("{:?}", completion.result)),
        ))
    }

    fn add_pc_breakpoint(&mut self, pc: u64) -> Result<(), BackendError> {
        let pc = pc as u32;
        let code_id = self
            .current_code_id
            .ok_or_else(|| BackendError::Message("no current code object for breakpoint".into()))?;
        let address = tcld_vm::CodeAddress { code_id, pc };
        self.engine.add_breakpoint(address);
        self.pc_breakpoints.insert(address);
        Ok(())
    }

    fn remove_pc_breakpoint(&mut self, pc: u64) -> Result<(), BackendError> {
        let pc = pc as u32;
        let code_id = self
            .current_code_id
            .ok_or_else(|| BackendError::Message("no current code object for breakpoint".into()))?;
        let address = tcld_vm::CodeAddress { code_id, pc };
        self.engine.remove_breakpoint(address);
        self.pc_breakpoints.remove(&address);
        Ok(())
    }

    fn drain_pauses(&mut self) -> Vec<BackendPause> {
        std::mem::take(&mut self.pauses)
    }

    fn app_state(&mut self) -> Result<AppState, BackendError> {
        if self.finished {
            Ok(AppState::Completed)
        } else if self.engine.pause_snapshot().is_some() {
            Ok(AppState::Stopped)
        } else {
            Ok(AppState::Running)
        }
    }

    fn close(self: Box<Self>) {}
}

// ── Wire session adapter ─────────────────────────────────────────────

/// Adapter around the existing wire `Session` for host-shell debug.
pub struct WireSessionBackend {
    session: super::session::Session,
    pauses: Vec<BackendPause>,
}

impl WireSessionBackend {
    pub fn connect(paths: super::config::NubPaths) -> Result<Self, BackendError> {
        let session = super::session::Session::connect(paths).map_err(BackendError::Message)?;
        Ok(Self {
            session,
            pauses: Vec::new(),
        })
    }

    pub fn session_mut(&mut self) -> &mut super::session::Session {
        &mut self.session
    }

    pub fn into_session(self) -> super::session::Session {
        self.session
    }
}

impl DebugBackend for WireSessionBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::wire_host()
    }

    fn start_target(&mut self, path: &Path, stop_on_entry: bool) -> Result<(), BackendError> {
        self.session
            .start_target(path, stop_on_entry)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn continue_(&mut self) -> Result<(), BackendError> {
        use tcld_debug_wire::Op;
        self.session
            .send(Op::Continue)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn step_instruction(&mut self) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            op: "step_instruction",
            reason: "host wire backend requires patched Tcl 9 for instruction step".into(),
        })
    }

    fn step_over_instruction(&mut self) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            op: "step_over_instruction",
            reason: "host wire backend: use command step (StepOver)".into(),
        })
    }

    fn step_out(&mut self) -> Result<(), BackendError> {
        use tcld_debug_wire::Op;
        self.session
            .send(Op::StepOut)
            .map_err(BackendError::Message)?;
        Ok(())
    }

    fn run_to_pc(&mut self, _pc: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            op: "run_to_pc",
            reason: "not available on host wire backend".into(),
        })
    }

    fn stack(&mut self) -> Result<Vec<StackFrame>, BackendError> {
        self.session.stack().map_err(BackendError::Message)
    }

    fn variables(&mut self, level: i64) -> Result<Vec<(String, String)>, BackendError> {
        self.session.variables(level).map_err(BackendError::Message)
    }

    fn eval_in_frame(&mut self, level: i64, script: &str) -> Result<(i32, String), BackendError> {
        use tcld_debug_wire::Op;
        let result = self
            .session
            .send(Op::EvalInFrame {
                level,
                script: script.to_string(),
            })
            .map_err(BackendError::Message)?;
        Ok((0, result))
    }

    fn expr_in_frame(
        &mut self,
        level: i64,
        expression: &str,
    ) -> Result<(i32, String), BackendError> {
        let script = format!("expr {}", quote_tcl_word(expression));
        self.eval_in_frame(level, &script)
    }

    fn add_pc_breakpoint(&mut self, _pc: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            op: "add_pc_breakpoint",
            reason: "host uses line/proc breakpoints".into(),
        })
    }

    fn remove_pc_breakpoint(&mut self, _pc: u64) -> Result<(), BackendError> {
        Err(BackendError::Unsupported {
            op: "remove_pc_breakpoint",
            reason: "host uses line/proc breakpoints".into(),
        })
    }

    fn drain_pauses(&mut self) -> Vec<BackendPause> {
        // Wire events are consumed via Session::drain_events in DebugApp today.
        std::mem::take(&mut self.pauses)
    }

    fn app_state(&mut self) -> Result<AppState, BackendError> {
        self.session.app_state().map_err(BackendError::Message)
    }

    fn close(self: Box<Self>) {
        self.session.close();
    }
}
