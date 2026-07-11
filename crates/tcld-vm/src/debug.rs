//! Model B pause hooks (S09) — pc-primary, Location always None on TEBC.

#![cfg(feature = "debug")]

use std::collections::BTreeSet;

use crate::engine::{CodeAddress, UiStackFrame};
use crate::value::VmObj;
use crate::var::FrameId;

/// Why the engine paused (Model B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// Stopped before the first instruction (`stop_on_entry`).
    Entry,
    /// Single-instruction step boundary.
    InstructionStep,
    StepOver,
    StepOut,
    /// Hit a bytecode PC breakpoint.
    Breakpoint,
    /// Reached `run_to_pc` target.
    RunToPc,
    /// Top-level TEBC finished.
    Completed,
}

/// App-visible pause snapshot (executable plan §8).
#[derive(Debug, Clone)]
pub struct PauseSnapshot {
    pub address: Option<CodeAddress>,
    pub frame_id: Option<FrameId>,
    pub frames: Vec<UiStackFrame>,
    /// Absolute TEBC PC of the instruction about to execute (or last Done).
    pub pc: Option<u32>,
    /// Always `None` on hybrid TEBC pauses (Model B).
    pub location: Option<()>,
    /// Always `false` on TEBC frames; host work is atomic (no pause-inside-host).
    pub in_host: bool,
    pub reason: PauseReason,
    pub opcode: Option<String>,
    pub proc_qname: Option<String>,
    pub frame_kind: String,
    pub locals: Vec<(String, String)>,
    pub stack_depth: usize,
    pub call_depth: usize,
}

/// Outcome of a debug run/step.
#[derive(Debug, Clone)]
pub enum DebugOutcome {
    Paused(PauseSnapshot),
    Finished { result: VmObj },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunMode {
    /// Run until BP / run_to / complete.
    Continue,
    /// Pause before every instruction (after the current one is skipped once).
    StepInto,
    StepOver(FrameId),
    StepOut(FrameId),
    /// Pause when absolute PC equals target (before execute).
    RunToAddress(CodeAddress),
}

#[derive(Debug)]
pub(crate) struct DebugController {
    pub enabled: bool,
    #[allow(dead_code)]
    pub stop_on_entry: bool,
    pub run_mode: RunMode,
    pub breakpoints: BTreeSet<CodeAddress>,
    pub last_pause: Option<PauseSnapshot>,
    /// True while frames are live and paused mid-execution.
    pub suspended: bool,
    /// First instruction of a freshly loaded body should stop when stop_on_entry.
    pub entry_armed: bool,
    /// After resume, do not re-pause on the instruction already shown.
    pub skip_once: bool,
}

impl DebugController {
    pub fn new(stop_on_entry: bool) -> Self {
        Self {
            enabled: true,
            stop_on_entry,
            run_mode: if stop_on_entry {
                RunMode::StepInto
            } else {
                RunMode::Continue
            },
            breakpoints: BTreeSet::new(),
            last_pause: None,
            suspended: false,
            entry_armed: stop_on_entry,
            skip_once: false,
        }
    }

    /// Decide whether to pause *before* executing `pc`.
    pub fn should_pause_before(
        &mut self,
        address: CodeAddress,
        frame_id: FrameId,
        active_frames: &[FrameId],
    ) -> Option<PauseReason> {
        if !self.enabled {
            return None;
        }
        if self.skip_once {
            self.skip_once = false;
            // Still honor hard breakpoints when continuing past the resume PC
            // only if run_mode is Continue and user set BP on same pc — rare.
            // For step, we must execute this instruction once.
            if matches!(
                self.run_mode,
                RunMode::StepInto
                    | RunMode::StepOver(_)
                    | RunMode::StepOut(_)
                    | RunMode::RunToAddress(_)
            ) {
                return None;
            }
            if self.breakpoints.contains(&address) && !self.entry_armed {
                // Allow continue to re-hit BP only on a later visit; skip once.
                return None;
            }
            return None;
        }
        if self.entry_armed {
            return Some(PauseReason::Entry);
        }
        if self.breakpoints.contains(&address) {
            return Some(PauseReason::Breakpoint);
        }
        match self.run_mode {
            RunMode::StepInto => Some(PauseReason::InstructionStep),
            RunMode::StepOver(anchor) if frame_id == anchor || !active_frames.contains(&anchor) => {
                Some(PauseReason::StepOver)
            }
            RunMode::StepOut(anchor) if !active_frames.contains(&anchor) => {
                Some(PauseReason::StepOut)
            }
            RunMode::RunToAddress(target) if target == address => Some(PauseReason::RunToPc),
            RunMode::Continue
            | RunMode::StepOver(_)
            | RunMode::StepOut(_)
            | RunMode::RunToAddress(_) => None,
        }
    }

    pub fn arm_step(&mut self) {
        self.run_mode = RunMode::StepInto;
        self.entry_armed = false;
        self.skip_once = true;
    }

    pub fn arm_step_over(&mut self, frame_id: FrameId) {
        self.run_mode = RunMode::StepOver(frame_id);
        self.entry_armed = false;
        self.skip_once = true;
    }

    pub fn arm_step_out(&mut self, frame_id: FrameId) {
        self.run_mode = RunMode::StepOut(frame_id);
        self.entry_armed = false;
        self.skip_once = true;
    }

    pub fn arm_continue(&mut self) {
        self.run_mode = RunMode::Continue;
        self.entry_armed = false;
        self.skip_once = true;
    }

    pub fn arm_run_to_address(&mut self, address: CodeAddress) {
        self.run_mode = RunMode::RunToAddress(address);
        self.entry_armed = false;
        self.skip_once = true;
    }

    pub fn clear_entry_arm(&mut self) {
        self.entry_armed = false;
    }
}

impl Default for DebugController {
    fn default() -> Self {
        Self {
            enabled: false,
            stop_on_entry: false,
            run_mode: RunMode::Continue,
            breakpoints: BTreeSet::new(),
            last_pause: None,
            suspended: false,
            entry_armed: false,
            skip_once: false,
        }
    }
}
