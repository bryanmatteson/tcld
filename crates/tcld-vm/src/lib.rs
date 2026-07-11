//! Hybrid TEBC virtual machine for tcld.
//!
//! Layering: domain crate — no CLI/TUI/Tauri/React. Hybrid debug adapters live
//! in apps; this crate does **not** replace `tcld-dbg-ext` (host nub).

pub mod error;
pub mod mode;
pub mod opcode_class;
pub mod policy;

#[cfg(feature = "host")]
pub mod bridge;

pub mod builtin;
pub mod cmd;
pub mod completion;
pub mod engine;
pub mod list_codec;
pub mod load;
pub mod tbcload;
pub mod value;
pub mod var;

#[cfg(feature = "debug")]
pub mod debug;

pub use completion::{Completion, ReturnOptions};
#[cfg(feature = "debug")]
pub use debug::{DebugOutcome, PauseReason, PauseSnapshot};
#[cfg(feature = "debug")]
pub use engine::UiStackFrame;
pub use engine::{
    CallContinuation, CodeAddress, CoverageSnapshot, NamespaceName, OpcodeObservation,
};
pub use error::VmError;
pub use mode::RuntimeMode;
pub use policy::ExecPolicy;
pub use value::VmObj;

use std::marker::PhantomData;
use std::path::Path;

use tcld_core::ByteCode;

use crate::engine::HybridEngineInner;

/// Hybrid (or vm-strict) TEBC engine.
///
/// Entry points consume `ByteCode` / `.tbc` paths only — never a decompiled
/// temp `.tcl` execution target (I1).
pub struct HybridEngine {
    inner: HybridEngineInner,
    _not_send: PhantomData<*mut ()>,
}

impl HybridEngine {
    pub fn new(mode: RuntimeMode) -> Result<Self, VmError> {
        Ok(Self {
            inner: HybridEngineInner::new(mode)?,
            _not_send: PhantomData,
        })
    }

    pub fn mode(&self) -> RuntimeMode {
        self.inner.mode()
    }

    pub fn policy(&self) -> &ExecPolicy {
        self.inner.policy()
    }

    /// Number of VmProc registrations (for F8 / tests).
    pub fn vm_proc_count(&self) -> usize {
        self.inner.registry().vm_proc_count()
    }

    /// Bootstrap embedded host (FindExecutable, tbcload façade).
    pub fn bootstrap(&mut self, argv0: Option<&str>) -> Result<(), VmError> {
        self.inner.bootstrap(argv0)
    }

    pub fn begin_output_capture(&mut self) -> Result<(), VmError> {
        self.inner.begin_output_capture()
    }

    pub fn captured_output(&self) -> Result<(Vec<u8>, Vec<u8>), VmError> {
        self.inner.captured_output()
    }

    pub fn host_setup_eval(&mut self, script: &str) -> Result<Completion, VmError> {
        self.inner.host_setup_eval(script)
    }

    /// Run top-level TEBC.
    pub fn run_tbc(&mut self, top: ByteCode) -> Result<value::VmObj, VmError> {
        self.inner.run_tbc(top)
    }

    /// Load and run a `.tbc` path (syntax extract + decode; no decompile-to-temp).
    pub fn run_tbc_path(&mut self, path: &Path) -> Result<value::VmObj, VmError> {
        self.inner.run_tbc_path(path)
    }

    pub fn invoke_registered(
        &mut self,
        qname: &[u8],
        args: &[VmObj],
    ) -> Result<Completion, VmError> {
        self.inner.invoke_registered(qname, args)
    }

    pub fn coverage_snapshot(&self) -> CoverageSnapshot {
        self.inner.coverage_snapshot()
    }

    pub fn first_failure(&self) -> Option<&str> {
        self.inner.first_failure()
    }

    // ── Model B debug surface (feature `debug`) ──────────────────────

    /// Enable Model B pause hooks (`instruction_step` always available).
    #[cfg(feature = "debug")]
    pub fn enable_debug(&mut self, stop_on_entry: bool) {
        self.inner.enable_debug(stop_on_entry);
    }

    /// Start debugging a `.tbc` path (no decompile-to-temp).
    #[cfg(feature = "debug")]
    pub fn debug_start_tbc_path(
        &mut self,
        path: &Path,
        stop_on_entry: bool,
    ) -> Result<DebugOutcome, VmError> {
        self.inner.enable_debug(stop_on_entry);
        let code = load::LoadedCode::from_path(path)?;
        self.inner
            .debug_start_code(code, engine::FrameKind::Toplevel, None, &[])
    }

    /// Start debugging an in-memory `ByteCode` body.
    #[cfg(feature = "debug")]
    pub fn debug_start_tbc(
        &mut self,
        top: ByteCode,
        stop_on_entry: bool,
    ) -> Result<DebugOutcome, VmError> {
        self.inner.enable_debug(stop_on_entry);
        let code = load::LoadedCode::from_bytecode(top)?;
        self.inner
            .debug_start_code(code, engine::FrameKind::Toplevel, None, &[])
    }

    /// Execute the current instruction and pause before the next (Model B).
    #[cfg(feature = "debug")]
    pub fn step_instruction(&mut self) -> Result<DebugOutcome, VmError> {
        self.inner.debug_resume(|d| d.arm_step())
    }

    /// Execute the current instruction without stopping in a callee, then
    /// pause at the initiating frame's next instruction (or its caller).
    #[cfg(feature = "debug")]
    pub fn step_over(&mut self) -> Result<DebugOutcome, VmError> {
        self.inner.debug_step_over()
    }

    /// Run until the selected VM frame exits, honoring qualified breakpoints.
    #[cfg(feature = "debug")]
    pub fn step_out(&mut self) -> Result<DebugOutcome, VmError> {
        self.inner.debug_step_out()
    }

    /// Continue until breakpoint, run-to-pc, or completion.
    #[cfg(feature = "debug")]
    pub fn continue_run(&mut self) -> Result<DebugOutcome, VmError> {
        self.inner.debug_resume(|d| d.arm_continue())
    }

    /// Run until absolute TEBC PC (pause-before-execute at target).
    #[cfg(feature = "debug")]
    pub fn run_to_pc(&mut self, pc: u32) -> Result<DebugOutcome, VmError> {
        self.inner.debug_run_to_pc(pc)
    }

    #[cfg(feature = "debug")]
    pub fn run_to_address(&mut self, address: CodeAddress) -> Result<DebugOutcome, VmError> {
        self.inner.debug_run_to_address(address)
    }

    #[cfg(feature = "debug")]
    pub fn pause_snapshot(&self) -> Option<PauseSnapshot> {
        self.inner.pause_snapshot()
    }

    #[cfg(feature = "debug")]
    pub fn add_pc_breakpoint(&mut self, pc: u32) {
        self.inner.add_pc_breakpoint(pc);
    }

    #[cfg(feature = "debug")]
    pub fn remove_pc_breakpoint(&mut self, pc: u32) {
        self.inner.remove_pc_breakpoint(pc);
    }

    #[cfg(feature = "debug")]
    pub fn add_breakpoint(&mut self, address: CodeAddress) {
        self.inner.add_breakpoint(address);
    }

    #[cfg(feature = "debug")]
    pub fn remove_breakpoint(&mut self, address: CodeAddress) {
        self.inner.remove_breakpoint(address);
    }

    #[cfg(feature = "debug")]
    pub fn ui_stack_frames(&self) -> Vec<UiStackFrame> {
        self.inner.ui_stack_frames()
    }

    #[cfg(feature = "debug")]
    pub fn ui_frame_locals(&self, level: usize) -> Vec<(String, String)> {
        self.inner.ui_frame_locals(level)
    }

    #[cfg(feature = "debug")]
    pub fn current_code(&self) -> Option<(load::CodeId, tcld_core::ByteCode)> {
        self.inner.debug_current_code()
    }

    #[cfg(feature = "debug")]
    pub fn observations(&self) -> &[OpcodeObservation] {
        self.inner.observations()
    }

    /// Evaluate a script in a selected suspended VM frame. This does not
    /// resume execution and returns the Tcl completion exactly as observed.
    #[cfg(feature = "debug")]
    pub fn eval_in_frame(&mut self, level: usize, script: VmObj) -> Result<Completion, VmError> {
        self.inner.debug_eval_in_frame(level, script, false)
    }

    /// Evaluate an expression in a selected suspended VM frame.
    #[cfg(feature = "debug")]
    pub fn expr_in_frame(
        &mut self,
        level: usize,
        expression: VmObj,
    ) -> Result<Completion, VmError> {
        self.inner.debug_eval_in_frame(level, expression, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tcld_core::bytecode::{ByteCodeInfo, LiteralObject, LiteralObjectType};
    use tcld_core::source::SourceRef;

    #[test]
    fn hybrid_engine_constructs_in_both_modes() {
        static_assertions::assert_not_impl_any!(HybridEngine: Send, Sync);
        let h = HybridEngine::new(RuntimeMode::Hybrid).expect("hybrid");
        assert!(h.policy().allow_host_invoke);
        let s = HybridEngine::new(RuntimeMode::VmStrict).expect("strict");
        assert!(!s.policy().allow_host_invoke);
    }

    #[test]
    fn f1_hand_built_arith() {
        // push1 0; push1 1; add; done  → 2+3=5
        let mut eng = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        let bc = hand_arith_bytecode();
        let result = eng.run_tbc(bc).expect("arith");
        assert_eq!(result.as_i64().unwrap(), 5);
    }

    #[test]
    fn vm_strict_denies_host_frame_expr_before_dispatch() {
        let mut eng = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        let expr_stk = 9u8; // opcode for exprStk in default set
        let bc = minimal_bc(vec![expr_stk], vec![]);
        let err = eng.run_tbc(bc).unwrap_err();
        assert!(matches!(
            err,
            VmError::PolicyDenied {
                action: "dynamic_eval",
                ..
            }
        ));
    }

    fn hand_arith_bytecode() -> ByteCode {
        // opcodes: push1=1, add=53, done=0
        let code = vec![
            1, 0, // push1 lit0
            1, 1,  // push1 lit1
            53, // add
            0,  // done
        ];
        let lits = vec![
            LiteralObject {
                kind: LiteralObjectType::Int,
                value: SourceRef::Int(2),
                proc_body: None,
            },
            LiteralObject {
                kind: LiteralObjectType::Int,
                value: SourceRef::Int(3),
                proc_body: None,
            },
        ];
        minimal_bc(code, lits)
    }

    fn minimal_bc(code_bytes: Vec<u8>, literals: Vec<LiteralObject>) -> ByteCode {
        ByteCode {
            header: None,
            info: ByteCodeInfo {
                num_commands: 0,
                num_src_bytes: 0,
                num_code_bytes: code_bytes.len() as i32,
                num_lit_objects: literals.len() as i32,
                num_except_ranges: 0,
                num_aux_data_items: 0,
                num_cmd_loc_bytes: 0,
                max_except_depth: 0,
                max_stack_depth: 8,
                code_delta_size: 0,
                code_length_size: 0,
                src_delta_size: 0,
                src_length_size: 0,
            },
            code_bytes,
            cmd_code_ranges: vec![],
            cmd_source_ranges: None,
            literals,
            exceptions: vec![],
            auxiliaries: vec![],
            proc_num_args: 0,
            proc_num_locals: 0,
            locals: vec![],
        }
    }
}
