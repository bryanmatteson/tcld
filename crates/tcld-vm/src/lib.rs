//! Hybrid TEBC virtual machine for tcld.
//!
//! Layering: domain crate — no CLI/TUI/Tauri/React. Hybrid debug adapters live
//! in apps; this crate does **not** replace `tcld-dbg-ext` (host nub).

pub mod error;
pub mod mode;
pub mod number;
pub mod opcode_class;
pub mod policy;

#[cfg(feature = "host")]
pub mod bridge;

pub mod args;
pub mod builtin;
pub mod cmd;
pub mod completion;
pub mod dict;
pub mod dict_command;
pub mod engine;
pub mod list_codec;
pub mod lists;
pub mod load;
pub mod lsearch;
pub mod lsort;
pub mod mathfunc;
pub mod output;
pub mod regex;
pub mod regexp_command;
pub mod string_command;
pub mod strings;
pub mod tbcload;
pub mod unicode;
mod unicode_data;
pub mod value;
pub mod var;
pub mod world;

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

#[cfg(feature = "debug")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugImageBinding {
    pub logical_unit_id: String,
    pub code_id: load::CodeId,
}

/// A verified code unit, including generated text and its producing call site.
#[cfg(feature = "debug")]
#[derive(Debug, Clone)]
pub struct DebugSource {
    pub code_id: load::CodeId,
    pub image: std::sync::Arc<tcld_compile::DebugImage>,
    pub unit_index: usize,
    pub origin: Option<CodeAddress>,
}

#[cfg(feature = "debug")]
#[derive(Debug)]
pub struct DebugImageStart {
    pub outcome: DebugOutcome,
    pub bindings: Vec<DebugImageBinding>,
}

use std::marker::PhantomData;
use std::path::Path;

use tcld_core::ByteCode;

use crate::engine::HybridEngineInner;

/// Hybrid (or vm-strict) TEBC engine.
///
/// Entry points consume `ByteCode` / `.tbc` paths only — never a decompiled
/// temp `.tcl` execution target (I1).
pub struct HybridEngine {
    inner: Box<HybridEngineInner>,
    _not_send: PhantomData<*mut ()>,
}

impl HybridEngine {
    pub fn new(mode: RuntimeMode) -> Result<Self, VmError> {
        Ok(Self {
            inner: Box::new(HybridEngineInner::new(mode)?),
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

    pub fn vm_proc_names(&self) -> Vec<String> {
        self.inner.registry().vm_proc_names()
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

    /// Start a compiler-owned source image. Procedure bytecode is loaded for
    /// stable code binding up front, but each command becomes callable only
    /// when its mapped top-level declaration executes. Every fresh runtime
    /// `CodeId` is returned alongside its stable logical image identity.
    #[cfg(feature = "debug")]
    pub fn debug_start_image(
        &mut self,
        image: &tcld_compile::DebugImage,
        stop_on_entry: bool,
    ) -> Result<DebugImageStart, VmError> {
        image.verify().map_err(VmError::Decode)?;
        self.inner.debug_start_image(image, stop_on_entry)
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

    /// All compiler-owned units discovered in this session, retained after return.
    #[cfg(feature = "debug")]
    pub fn debug_sources(&self) -> Vec<DebugSource> {
        self.inner.debug_sources()
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

    /// Bound retained instruction telemetry without limiting execution or
    /// coverage counters. None preserves the native full-history default.
    #[cfg(feature = "debug")]
    pub fn set_observation_limit(&mut self, limit: Option<usize>) {
        self.inner.set_observation_limit(limit);
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

impl Drop for HybridEngine {
    fn drop(&mut self) {
        #[cfg(feature = "debug")]
        self.inner.cancel_suspension();
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
    fn regexp_opcode_executes_without_bootstrapping_a_host() {
        let opcode = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .position(|opcode| *opcode == tcld_core::inst_table::Opcode::Regexp)
            .unwrap() as u8;
        let literals = ["^(héllo|world)$", "HÉLLO"]
            .into_iter()
            .map(|text| LiteralObject {
                kind: LiteralObjectType::String,
                value: SourceRef::Static(text),
                proc_body: None,
            })
            .collect();
        let bytecode = minimal_bc(vec![1, 0, 1, 1, opcode, 0o33, 0], literals);
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        assert_eq!(engine.run_tbc(bytecode).unwrap().as_i64().unwrap(), 1);
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    fn regsub_callbacks_execute_from_bytecode_without_bootstrapping_a_host() {
        let invoke = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .position(|opcode| *opcode == tcld_core::inst_table::Opcode::InvokeStk1)
            .unwrap() as u8;
        let literals = ["regsub", "-all", "-command", ".", "aé🙂", "string toupper"]
            .into_iter()
            .map(|text| LiteralObject {
                kind: LiteralObjectType::String,
                value: SourceRef::Static(text),
                proc_body: None,
            })
            .collect();
        let bytecode = minimal_bc(
            vec![1, 0, 1, 1, 1, 2, 1, 3, 1, 4, 1, 5, invoke, 6, 0],
            literals,
        );
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        assert_eq!(
            engine.run_tbc(bytecode).unwrap().as_string().unwrap(),
            "AÉ🙂"
        );
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn regsub_callback_source_image_executes_without_host_support() {
        let image = tcld_compile::compile_source(
            "/workspace/regsub-callback.tcl".into(),
            "proc cb {m} {upvar 1 n n; incr n; return [string toupper $m]}\n\
             proc f {} {set n 0; set out [regsub -all -command . aé🙂 cb]; list $out $n}\n\
             f\n"
            .into(),
            tcld_compile::CompileMode::VmStrict,
        )
        .unwrap();
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        match engine.debug_start_image(&image, false).unwrap().outcome {
            DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "AÉ🙂 3"),
            other => panic!("{other:?}"),
        }
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    fn lsort_callbacks_execute_from_bytecode_without_bootstrapping_a_host() {
        let invoke = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .position(|opcode| *opcode == tcld_core::inst_table::Opcode::InvokeStk1)
            .unwrap() as u8;
        let literals = ["lsort", "-command", "string compare", "b aé a"]
            .into_iter()
            .map(|text| LiteralObject {
                kind: LiteralObjectType::String,
                value: SourceRef::Static(text),
                proc_body: None,
            })
            .collect();
        let bytecode = minimal_bc(vec![1, 0, 1, 1, 1, 2, 1, 3, invoke, 4, 0], literals);
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        assert_eq!(
            engine.run_tbc(bytecode).unwrap().as_string().unwrap(),
            "a aé b"
        );
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn lmap_iteration_completions_and_callbacks_execute_without_host_support() {
        let image = tcld_compile::compile_source(
            "/workspace/lmap.tcl".into(),
            "set mapped [lmap {x y} {1 2 3} z {a b c} {list $x $y $z}]\n\
             set filtered [lmap x {1 2 3 4} {if {$x == 2} {continue}; if {$x == 4} {break}; expr {$x*10}}]\n\
             set replaced [regsub -all -command . abc {apply {{m} {lindex [lmap x [list $m] {string toupper $x}] 0}}}]\n\
             list $mapped $filtered $replaced\n".into(),
            tcld_compile::CompileMode::VmStrict,
        ).unwrap();
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        match engine.debug_start_image(&image, false).unwrap().outcome {
            DebugOutcome::Finished { result } => assert_eq!(
                result.as_string().unwrap(),
                "{{1 2 a} {3 {} b} {{} {} c}} {10 30} ABC"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn try_handlers_cleanup_and_callbacks_execute_without_host_support() {
        let image = tcld_compile::compile_source(
            "/workspace/try.tcl".into(),
            "set out {}; set handled [try {error missing {} {APP MISSING}} trap APP {m o} {lappend out handler; list $m [dict get $o -errorcode]} finally {lappend out cleanup}]\n\
             set replaced [regsub -all -command . abc {apply {{m} {try {string toupper $m} finally {set cleaned yes}}}}]\n\
             list $handled $out $replaced\n".into(),
            tcld_compile::CompileMode::VmStrict,
        ).unwrap();
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        match engine.debug_start_image(&image, false).unwrap().outcome {
            DebugOutcome::Finished { result } => assert_eq!(
                result.as_string().unwrap(),
                "{missing {APP MISSING}} {handler cleanup} ABC"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn try_executes_from_bytecode_without_a_parent_source_image() {
        let invoke = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .position(|opcode| *opcode == tcld_core::inst_table::Opcode::InvokeStk1)
            .unwrap() as u8;
        let literals = [
            "try",
            "error missing {} {APP MISSING}",
            "trap",
            "APP",
            "m o",
            "list $m [dict get $o -errorcode]",
            "finally",
            "set cleaned yes",
        ]
        .into_iter()
        .map(|text| LiteralObject {
            kind: LiteralObjectType::String,
            value: SourceRef::Static(text),
            proc_body: None,
        })
        .collect();
        let bytecode = minimal_bc(
            vec![1, 0, 1, 1, 1, 2, 1, 3, 1, 4, 1, 5, 1, 6, 1, 7, invoke, 8, 0],
            literals,
        );
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        assert_eq!(
            engine.run_tbc(bytecode).unwrap().as_string().unwrap(),
            "missing {APP MISSING}"
        );
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn lmap_executes_from_bytecode_without_a_parent_source_image() {
        let invoke = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .position(|opcode| *opcode == tcld_core::inst_table::Opcode::InvokeStk1)
            .unwrap() as u8;
        let literals = ["lmap", "x", "1 2 3", "expr {$x*2}"]
            .into_iter()
            .map(|text| LiteralObject {
                kind: LiteralObjectType::String,
                value: SourceRef::Static(text),
                proc_body: None,
            })
            .collect();
        let bytecode = minimal_bc(vec![1, 0, 1, 1, 1, 2, 1, 3, invoke, 4, 0], literals);
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        assert_eq!(
            engine.run_tbc(bytecode).unwrap().as_string().unwrap(),
            "2 4 6"
        );
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn switch_matching_captures_and_callback_bodies_execute_without_host_support() {
        let image = tcld_compile::compile_source(
            "/workspace/switch.tcl".into(),
            "set sorted [lsort -command {apply {{a b} {switch $a {default {expr {$a-$b}}}}}} {3 1 2}]\n\
             set captures [switch -r -m m -i i abc {(a)(bc)} {list $m $i}]\n\
             list $sorted $captures [switch absent {default {expr {6*7}}}]\n".into(),
            tcld_compile::CompileMode::VmStrict,
        ).unwrap();
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        match engine.debug_start_image(&image, false).unwrap().outcome {
            DebugOutcome::Finished { result } => assert_eq!(
                result.as_string().unwrap(),
                "{1 2 3} {{abc a bc} {{0 2} {0 0} {1 2}}} 42",
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn switch_executes_from_bytecode_without_a_parent_source_image() {
        let invoke = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .position(|opcode| *opcode == tcld_core::inst_table::Opcode::InvokeStk1)
            .unwrap() as u8;
        let literals = ["switch", "a", "a {expr {6*7}} default {error wrong}"]
            .into_iter()
            .map(|text| LiteralObject {
                kind: LiteralObjectType::String,
                value: SourceRef::Static(text),
                proc_body: None,
            })
            .collect();
        let bytecode = minimal_bc(vec![1, 0, 1, 1, 1, 2, invoke, 3, 0], literals);
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        assert_eq!(engine.run_tbc(bytecode).unwrap().as_string().unwrap(), "42");
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn apply_lambdas_and_callbacks_execute_without_host_support() {
        let image = tcld_compile::compile_source(
            "/workspace/apply.tcl".into(),
            "namespace eval n {variable suffix !}\n\
             set sorted [lsort -command {apply {{a b} {expr {$a-$b}}}} {3 1 2}]\n\
             set replaced [regsub -all -command . abc {apply {{m} {variable suffix; string cat $m $suffix} ::n}}]\n\
             list $sorted $replaced [apply {{x {y 2} args} {list $x $y $args}} 40]\n".into(),
            tcld_compile::CompileMode::VmStrict,
        ).unwrap();
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        match engine.debug_start_image(&image, false).unwrap().outcome {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), "{1 2 3} a!b!c! {40 2 {}}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn apply_executes_from_bytecode_without_a_parent_source_image() {
        let invoke = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET
            .opcode_table
            .iter()
            .position(|opcode| *opcode == tcld_core::inst_table::Opcode::InvokeStk1)
            .unwrap() as u8;
        let literals = ["apply", "{x {y 2}} {expr {$x+$y}}", "40"]
            .into_iter()
            .map(|text| LiteralObject {
                kind: LiteralObjectType::String,
                value: SourceRef::Static(text),
                proc_body: None,
            })
            .collect();
        let bytecode = minimal_bc(vec![1, 0, 1, 1, 1, 2, invoke, 3, 0], literals);
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        assert_eq!(engine.run_tbc(bytecode).unwrap().as_string().unwrap(), "42");
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn lsort_callback_source_image_executes_without_host_support() {
        let image = tcld_compile::compile_source(
            "/workspace/lsort-callback.tcl".into(),
            "proc compare {a b} {upvar 1 calls counter; incr counter; expr {$a-$b}}\n\
             proc f {} {set calls 0; set sorted [lsort -command compare {3 1 2}]; list $sorted $calls}\n\
             f\n".into(),
            tcld_compile::CompileMode::VmStrict,
        ).unwrap();
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        match engine.debug_start_image(&image, false).unwrap().outcome {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), "{1 2 3} 3")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn compiled_source_image_executes_nested_proc_without_host() {
        let image = tcld_compile::compile_source(
            std::path::PathBuf::from("/tmp/compiled-image.tcl"),
            "proc inc {x} { return [expr {$x + 1}] }\nset n 2\nset n [inc $n]\n".into(),
            tcld_compile::CompileMode::VmStrict,
        )
        .expect("compile strict image");
        let mut engine = HybridEngine::new(RuntimeMode::VmStrict).expect("strict engine");
        let started = engine
            .debug_start_image(&image, false)
            .expect("execute image");
        assert_eq!(started.bindings.len(), 2);
        let DebugOutcome::Finished { result } = started.outcome else {
            panic!("image should finish when stop-on-entry is disabled")
        };
        assert_eq!(result.as_i64().unwrap(), 3);
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
    }

    #[test]
    #[cfg(all(feature = "debug", feature = "host"))]
    fn compiled_proc_activation_and_declaration_breakpoint_match_source_truth() {
        let image = tcld_compile::compile_source(
            std::path::PathBuf::from("/tmp/declarations.tcl"),
            "proc first {} { return 1 }\nproc second {} { return 2 }\nset done [second]\n".into(),
            tcld_compile::CompileMode::Hybrid,
        )
        .unwrap();
        let second_declaration = image
            .units
            .iter()
            .find_map(|unit| match &unit.kind {
                tcld_compile::DebugCodeUnitKind::Procedure {
                    qname,
                    declaration_span,
                    ..
                } if qname == "::second" => Some(declaration_span.clone()),
                _ => None,
            })
            .unwrap();

        let mut engine = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
        engine.bootstrap(None).unwrap();
        let started = engine.debug_start_image(&image, true).unwrap();
        let DebugOutcome::Paused(entry) = started.outcome else {
            panic!("stop on entry")
        };
        assert_eq!(entry.reason, PauseReason::Entry);
        assert_eq!(engine.vm_proc_count(), 0, "line 1 has not executed yet");
        let before = engine
            .eval_in_frame(0, VmObj::from_str("info commands ::first"))
            .unwrap();
        assert_eq!(before.result.as_string().unwrap(), "");

        let top_code = started
            .bindings
            .iter()
            .find(|binding| binding.logical_unit_id == "file:toplevel")
            .unwrap()
            .code_id;
        let second_map = image
            .top_level()
            .commands
            .iter()
            .find(|map| map.source_span == second_declaration)
            .unwrap();
        let second_address = CodeAddress {
            code_id: top_code,
            pc: second_map.anchor_pc,
        };
        engine.add_breakpoint(second_address);

        while engine.vm_proc_count() == 0 {
            assert!(matches!(
                engine.step_instruction().unwrap(),
                DebugOutcome::Paused(_)
            ));
        }
        assert_eq!(engine.vm_proc_count(), 1);
        let after_first = engine
            .eval_in_frame(0, VmObj::from_str("info commands ::first"))
            .unwrap();
        assert_eq!(after_first.result.as_string().unwrap(), "::first");

        let DebugOutcome::Paused(second_pause) = engine.continue_run().unwrap() else {
            panic!("second declaration breakpoint")
        };
        assert_eq!(second_pause.reason, PauseReason::Breakpoint);
        assert_eq!(second_pause.address, Some(second_address));
        assert_eq!(
            engine.vm_proc_count(),
            1,
            "second is pending at its breakpoint"
        );
        let before_second = engine
            .eval_in_frame(0, VmObj::from_str("info commands ::second"))
            .unwrap();
        assert_eq!(before_second.result.as_string().unwrap(), "");

        while engine.vm_proc_count() == 1 {
            assert!(matches!(
                engine.step_instruction().unwrap(),
                DebugOutcome::Paused(_)
            ));
        }
        assert_eq!(engine.vm_proc_count(), 2);
        let after_second = engine
            .eval_in_frame(0, VmObj::from_str("info commands ::second"))
            .unwrap();
        assert_eq!(after_second.result.as_string().unwrap(), "::second");
    }

    #[test]
    fn source_compile_failure_cannot_start_or_mutate_the_vm() {
        let engine = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        for (source, mode) in [
            ("exec not-admitted\n", tcld_compile::CompileMode::VmStrict),
            ("source child.tcl\n", tcld_compile::CompileMode::VmStrict),
            (
                "interp eval {} {set x 1}\n",
                tcld_compile::CompileMode::Hybrid,
            ),
            (
                "package require hidden\n",
                tcld_compile::CompileMode::Hybrid,
            ),
            (
                "oo::class new {method x {} {}}\n",
                tcld_compile::CompileMode::Hybrid,
            ),
        ] {
            let failure = tcld_compile::compile_source(
                std::path::PathBuf::from("/tmp/rejected.tcl"),
                source.into(),
                mode,
            );
            assert!(failure.is_err());
        }
        assert_eq!(engine.vm_proc_count(), 0);
        assert_eq!(engine.coverage_snapshot().executed_instruction_total, 0);
    }

    #[test]
    #[cfg(feature = "debug")]
    fn compiled_source_return_routes_normal_top_level_and_error_completions() {
        let compile = |source: &str| {
            tcld_compile::compile_source(
                std::path::PathBuf::from("/tmp/compiled-return.tcl"),
                source.into(),
                tcld_compile::CompileMode::VmStrict,
            )
            .unwrap()
        };
        let mut top = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        let started = top
            .debug_start_image(&compile("return 7\n"), false)
            .unwrap();
        let DebugOutcome::Finished { result } = started.outcome else {
            panic!("finished")
        };
        assert_eq!(result.as_i64().unwrap(), 7);

        let mut error = HybridEngine::new(RuntimeMode::VmStrict).unwrap();
        let failure = error
            .debug_start_image(&compile("return -code error boom\n"), false)
            .unwrap_err();
        assert!(matches!(failure, VmError::TclError { ref message, .. } if message == "boom"));
    }

    #[test]
    #[cfg(all(feature = "debug", feature = "host"))]
    fn compiled_source_hybrid_world_host_dispatch_preserves_mapped_command_execution() {
        let image = tcld_compile::compile_source(
            std::path::PathBuf::from("/tmp/compiled-hybrid.tcl"),
            "puts hello\nset result 42\n".into(),
            tcld_compile::CompileMode::Hybrid,
        )
        .unwrap();
        let mut engine = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
        engine.bootstrap(None).unwrap();
        engine.begin_output_capture().unwrap();
        let started = engine.debug_start_image(&image, false).unwrap();
        let DebugOutcome::Finished { result } = started.outcome else {
            panic!("finished")
        };
        assert_eq!(result.as_i64().unwrap(), 42);
        let (stdout, stderr) = engine.captured_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&stdout), "hello\n");
        assert!(stderr.is_empty());
        assert_eq!(engine.coverage_snapshot().host_command_invocations, 1);
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
