//! Hybrid TEBC engine loop (S04–S07).

use std::path::Path;
use std::sync::Arc;

use tcld_core::bytecode::{AuxDataValue, ByteCode, ExceptionRange, ExceptionType, NewForeachInfo};
use tcld_core::inst_table::Opcode;

use crate::builtin::{self, VarNameCtx};
use crate::cmd::{CommandRef, CommandRegistry, FacadeKind};
use crate::completion::{Completion, ReturnOptions};
use crate::dict::Dict;
use crate::error::VmError;
use crate::load::{LoadedCode, LocalMeta, locals_meta_from_bc};
use crate::mode::RuntimeMode;
use crate::opcode_class::{OpcodeClass, classify};
use crate::policy::ExecPolicy;
use crate::tbcload;
use crate::value::{
    ByteString, VmObj, glob_match, list_to_string, parse_array_name, parse_tcl_int,
    parse_tcl_number, split_tcl_list,
};
use crate::var::{FrameId, LocalSlot, VarLink, VarSlot, VarTable, split_var_name};

#[cfg(feature = "host")]
use crate::bridge::{
    BridgeResult, HostBridge, HostFrameInput, HostFrameLocalResult, HostFrameOutput,
    HostFrameStorage,
};

#[cfg(feature = "debug")]
mod apply;
#[cfg(feature = "debug")]
mod callbacks;
mod command_callback;
#[cfg(feature = "debug")]
mod control;
#[cfg(feature = "debug")]
mod coroutines;
#[cfg(feature = "debug")]
mod dynamic;
#[cfg(feature = "debug")]
mod execution;
mod introspection;
#[cfg(feature = "debug")]
mod lmap;
#[cfg(feature = "debug")]
mod suspend;
#[cfg(feature = "debug")]
mod switch;
#[cfg(feature = "debug")]
pub(crate) mod tailcalls;
#[cfg(feature = "debug")]
mod try_cmd;

const MAX_CALL_DEPTH: usize = 1000;

/// Canonical namespace used for command and variable resolution in one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceName(ByteString);

impl NamespaceName {
    pub fn root() -> Self {
        Self(ByteString::from("::"))
    }

    pub fn from_proc_qname(qname: &ByteString) -> Self {
        let bytes = qname.as_bytes();
        let leaf_start = bytes
            .windows(2)
            .rposition(|pair| pair == b"::")
            .unwrap_or(0);
        if leaf_start == 0 {
            Self::root()
        } else {
            Self(ByteString::from_bytes(bytes[..leaf_start].to_vec()))
        }
    }

    pub fn as_bytes(&self) -> &ByteString {
        &self.0
    }

    fn qualify(&self, name: &ByteString) -> ByteString {
        if name.as_bytes().starts_with(b"::") {
            return name.clone();
        }
        if self.0.as_bytes() == b"::" {
            ByteString::from(format!("::{}", name.as_str_lossy()).as_str())
        } else {
            ByteString::from(format!("{}::{}", self.0.as_str_lossy(), name.as_str_lossy()).as_str())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeAddress {
    pub code_id: crate::load::CodeId,
    pub pc: u32,
}

#[derive(Debug, Clone)]
pub struct CallContinuation {
    pub caller: FrameId,
    pub call_site: CodeAddress,
    pub resume_pc_index: usize,
    pub caller_stack_depth: usize,
}

/// App-facing stack frame row (executable plan §2.3 / §8).
#[derive(Debug, Clone)]
pub struct UiStackFrame {
    pub frame_id: FrameId,
    pub address: Option<CodeAddress>,
    pub kind_label: String,
    pub proc_qname: Option<String>,
    pub namespace: String,
    pub pc: Option<u32>,
    /// Command-source byte offset within this code object's source body.
    pub source_offset: Option<usize>,
    pub in_host: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSnapshot {
    pub opcode_total: usize,
    pub vm_count: usize,
    pub host_value_count: usize,
    pub host_frame_count: usize,
    pub trap_count: usize,
    pub executed_instruction_total: u64,
    pub executed_by_opcode: std::collections::BTreeMap<String, u64>,
    pub executed_by_class: std::collections::BTreeMap<String, u64>,
    pub host_command_invocations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcodeObservation {
    pub code_id: crate::load::CodeId,
    pub pc: u32,
    pub opcode: Opcode,
    pub class: OpcodeClass,
    pub frame_id: FrameId,
}

/// Host bootstrap: restore Tcl-8-style global array visibility inside
/// `namespace eval` bodies for common unqualified names used by legacy TEBC.
#[cfg(feature = "host")]
const TCL8_NAMESPACE_GLOBAL_FALLBACK_SHIM: &str = r#"
if {![info exists ::tcld::ns_global_fallback]} {
    namespace eval ::tcld {}
    set ::tcld::ns_global_fallback 1
    rename ::namespace ::tcld::_namespace_orig
    proc ::namespace {cmd args} {
        if {$cmd eq "eval" && [llength $args] >= 2} {
            set ns [lindex $args 0]
            set body [lindex $args 1]
            set rest [lrange $args 2 end]
            set inject {
                catch {::tcld::_frame_upvar_orig #0 ::tcl_platform tcl_platform}
                catch {::tcld::_frame_upvar_orig #0 ::env env}
                catch {::tcld::_frame_upvar_orig #0 ::auto_path auto_path}
                catch {::tcld::_frame_upvar_orig #0 ::errorInfo errorInfo}
                catch {::tcld::_frame_upvar_orig #0 ::errorCode errorCode}
                catch {::tcld::_frame_upvar_orig #0 ::argv argv}
                catch {::tcld::_frame_upvar_orig #0 ::argv0 argv0}
                catch {::tcld::_frame_upvar_orig #0 ::argc argc}
                catch {::tcld::_frame_upvar_orig #0 ::tcl_version tcl_version}
                catch {::tcld::_frame_upvar_orig #0 ::tcl_patchLevel tcl_patchLevel}
            }
            set body "$inject\n$body"
            return [::tcld::_frame_uplevel_orig 1 [list ::tcld::_namespace_orig eval $ns $body {*}$rest]]
        }
        return [::tcld::_frame_uplevel_orig 1 [list ::tcld::_namespace_orig $cmd {*}$args]]
    }
}
"#;

#[cfg(feature = "host")]
const HOST_FRAME_BASE_SHIM: &str = r#"
if {![llength [info commands ::tcld::_frame_upvar_orig]]} {
    rename ::upvar ::tcld::_frame_upvar_orig
    rename ::uplevel ::tcld::_frame_uplevel_orig
    rename ::yield ::tcld::_frame_yield_orig
    rename ::yieldto ::tcld::_frame_yieldto_orig
}
"#;

/// Wrappers used by the typed `frame_eval_internal` bridge. Request and result
/// transport stays in Rust client data; Tcl holds only cross-frame alias state
/// in the ephemeral namespace returned by `frame_active_namespace`.
#[cfg(feature = "host")]
const TYPED_HOST_FRAME_WRAPPER_SHIM: &str = r#"
proc ::upvar {args} {
    if {[::tcld::frame_active_namespace] ne {}} {
        tailcall ::tcld::typed_frame_upvar {*}$args
    }
    tailcall ::tcld::_frame_upvar_orig {*}$args
}
proc ::uplevel {args} {
    if {[::tcld::frame_active_namespace] ne {}} {
        tailcall ::tcld::typed_frame_uplevel {*}$args
    }
    tailcall ::tcld::_frame_uplevel_orig {*}$args
}
proc ::yield {args} {
    if {[::tcld::frame_active_namespace] ne {}} {
        return -code error -errorcode {TCLD HOSTFRAME SUSPENSION} {host frame suspension is unsupported}
    }
    tailcall ::tcld::_frame_yield_orig {*}$args
}
proc ::yieldto {args} {
    if {[::tcld::frame_active_namespace] ne {}} {
        return -code error -errorcode {TCLD HOSTFRAME SUSPENSION} {host frame suspension is unsupported}
    }
    tailcall ::tcld::_frame_yieldto_orig {*}$args
}
proc ::tcld::typed_frame_upvar {args} {
    if {[llength $args] < 2} {
        return -code error {wrong # args: should be "upvar ?level? otherVar localVar ?otherVar localVar ...?"}
    }
    if {[llength $args] % 2} {
        set level [lindex $args 0]
        set args [lrange $args 1 end]
    } else {
        set level 1
    }
    if {$level eq "0"} {
        return [::tcld::_frame_uplevel_orig 1 [list ::tcld::_frame_upvar_orig 0 {*}$args]]
    }
    set tmp [::tcld::frame_active_namespace]
    foreach {other local} $args {
        lassign [::tcld::frame_cross upvar $level $other $local] storage value
        set ${tmp}::__cross_level($local) $level
        set ${tmp}::__cross_other($local) $other
        ::tcld::_frame_uplevel_orig 1 [list unset -nocomplain $local]
        if {$storage eq "array"} {
            ::tcld::_frame_uplevel_orig 1 [list array set $local $value]
        } else {
            ::tcld::_frame_uplevel_orig 1 [list set $local $value]
        }
    }
    return {}
}
proc ::tcld::typed_frame_uplevel {args} {
    if {![llength $args]} {
        return -code error {wrong # args: should be "uplevel ?level? command ?arg ...?"}
    }
    set level 1
    if {[llength $args] > 1 && [regexp {^(#?[0-9]+|-[0-9]+)$} [lindex $args 0]]} {
        set level [lindex $args 0]
        set args [lrange $args 1 end]
    }
    if {$level eq "0"} {
        return [::tcld::_frame_uplevel_orig 1 [list ::tcld::_frame_uplevel_orig 0 {*}$args]]
    }
    set code [catch {::tcld::frame_cross uplevel $level [concat {*}$args]} result options]
    set tmp [::tcld::frame_active_namespace]
    if {[array exists ${tmp}::__cross_level]} {
        foreach local [array names ${tmp}::__cross_level] {
            set linked_level [set ${tmp}::__cross_level($local)]
            set other [set ${tmp}::__cross_other($local)]
            lassign [::tcld::frame_cross upvar $linked_level $other $local] storage value
            ::tcld::_frame_uplevel_orig 1 [list unset -nocomplain $local]
            if {$storage eq "array"} {
                ::tcld::_frame_uplevel_orig 1 [list array set $local $value]
            } else {
                ::tcld::_frame_uplevel_orig 1 [list set $local $value]
            }
        }
    }
    if {$code} {
        return -options $options $result
    }
    return $result
}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameKind {
    Toplevel,
    VmProc,
    Lambda,
    Dynamic,
    Namespace,
    Source,
}

struct CatchMark {
    range_index: usize,
    stack_depth: usize,
    aux_state_depth: usize,
}

#[derive(Debug, Clone)]
struct NewForeachState {
    id: i64,
    info: NewForeachInfo,
    lists: Vec<Vec<VmObj>>,
    iteration: usize,
    iteration_count: usize,
    stack_base: usize,
}

#[derive(Debug, Clone)]
struct DictIterState {
    lvt: usize,
    entries: Vec<(VmObj, VmObj)>,
    next: usize,
}

#[derive(Debug, Clone)]
struct ExpandMark {
    stack_depth: usize,
}

#[derive(Debug, Clone)]
enum AuxRuntimeState {
    NewForeach(NewForeachState),
    DictIter(DictIterState),
    Expand(ExpandMark),
}

enum VariableTarget {
    Lvt(usize),
    LvtArray(usize, ByteString),
    Named(ByteString, Option<ByteString>),
}

struct CallFrame {
    id: FrameId,
    code: Arc<LoadedCode>,
    pc_index: usize,
    stack: Vec<VmObj>,
    compiled_locals: Vec<LocalSlot>,
    local_vars: VarTable,
    scope_parent: Option<FrameId>,
    script_path: Option<String>,
    tcl_parent: Option<FrameId>,
    hidden_scope: bool,
    /// The words that created this level, which `info level` reports.
    invocation: Option<VmObj>,
    /// Context appended only when an error escapes this mapped script body.
    #[cfg(feature = "debug")]
    body_error_context: Option<String>,
    #[cfg(feature = "debug")]
    lmap: Option<lmap::LmapState>,
    #[cfg(feature = "debug")]
    try_state: Option<try_cmd::TryState>,
    tail_root: bool,
    catch_stack: Vec<CatchMark>,
    // Stack boundaries at entry to each protected loop body. Non-local
    // break/continue discard partial command operands and inner catch state.
    loop_marks: std::collections::BTreeMap<i32, (usize, usize, usize)>,
    aux_states: Vec<AuxRuntimeState>,
    namespace: NamespaceName,
    #[allow(dead_code)]
    kind: FrameKind,
    #[allow(dead_code)]
    proc_qname: Option<ByteString>,
    continuation: Option<CallContinuation>,
}

/// Internal exception / control signal.
enum Flow {
    Continue,
    /// Absolute PC jump in current frame.
    JumpPc(u32),
    /// Return from current frame with value.
    FrameDone(VmObj),
    /// Tcl-style exception (error/return/break/continue codes).
    Raise(Completion),
}

/// Outcome of command resolution / host invoke (no Internal string tags).
enum InvokeOutcome {
    Value(VmObj),
    Raise(Completion),
    FramePushed,
}

/// Internal eval-loop exit (pause vs done).
enum LoopExit {
    Done(VmObj),
    Completion(Completion),
    #[cfg(feature = "debug")]
    Paused,
}

pub(crate) struct HybridEngineInner {
    mode: RuntimeMode,
    policy: ExecPolicy,
    frames: Vec<CallFrame>,
    next_frame_id: u32,
    next_aux_state_id: i64,
    registry: CommandRegistry,
    /// State behind `rand()` and `srand()`.
    rand: crate::mathfunc::RandState,
    /// Namespace variables and namespaces, which vm-strict keeps in the VM.
    world: crate::world::World,
    /// The stdout and stderr vm-strict writes itself.
    output: crate::output::Output,
    last_completion: Completion,
    last_host_write: Option<VmObj>,
    executed_instruction_total: u64,
    executed_by_opcode: std::collections::BTreeMap<String, u64>,
    executed_by_class: std::collections::BTreeMap<String, u64>,
    host_command_invocations: u64,
    first_failure: Option<String>,
    #[cfg(feature = "debug")]
    observations: Vec<OpcodeObservation>,
    #[cfg(feature = "debug")]
    observation_limit: Option<usize>,
    /// Compiler-owned procedures loaded for this image but not callable until
    /// their mapped top-level declaration boundary executes.
    #[cfg(feature = "debug")]
    pending_image_procs: std::collections::BTreeMap<CodeAddress, crate::cmd::VmProc>,
    /// Host stubs activated by the current image, removed on restart/image
    /// replacement so pre-declaration host state remains truthful.
    #[cfg(feature = "debug")]
    image_stub_commands: std::collections::BTreeSet<String>,
    #[cfg(feature = "debug")]
    source_images: std::collections::BTreeMap<crate::load::CodeId, crate::DebugSource>,
    #[cfg(feature = "debug")]
    generated_cache: std::collections::BTreeMap<(CodeAddress, String, String), Arc<LoadedCode>>,
    /// Callback wrappers contain evaluated arguments, which can differ on
    /// every call. Bodies they invoke retain the original callback site's
    /// code identity so breakpoints survive those argument changes.
    #[cfg(feature = "debug")]
    command_callback_sites: std::collections::BTreeMap<crate::load::CodeId, CodeAddress>,
    #[cfg(feature = "debug")]
    events: std::collections::BTreeMap<u64, dynamic::PendingEvent>,
    #[cfg(feature = "debug")]
    next_event: u64,
    #[cfg(feature = "debug")]
    suspension: Option<suspend::Suspension>,
    #[cfg(feature = "debug")]
    coroutines: std::collections::BTreeMap<u64, coroutines::Coroutine>,
    #[cfg(feature = "debug")]
    coroutine_boundary: Option<usize>,
    #[cfg(feature = "debug")]
    coroutine_yield: Option<coroutines::Handoff>,
    #[cfg(feature = "debug")]
    coroutine_identity: (u64, u64),
    #[cfg(feature = "debug")]
    coroutine_task: Option<u64>,
    #[cfg(feature = "debug")]
    control_groups: std::collections::BTreeMap<u64, control::ControlGroup>,
    #[cfg(feature = "debug")]
    coroutine_completions: std::collections::BTreeMap<u32, Completion>,
    #[cfg(feature = "debug")]
    tailcalls: std::collections::BTreeMap<u32, tailcalls::Tailcall>,
    #[cfg(feature = "debug")]
    host_callbacks: std::collections::BTreeMap<u64, callbacks::HostCallback>,
    #[cfg(feature = "debug")]
    trace_scopes: std::collections::BTreeMap<u32, String>,
    #[cfg(feature = "debug")]
    traced_commands: Vec<execution::ActiveCommand>,
    #[cfg(feature = "debug")]
    hidden_trace_commands: std::collections::BTreeSet<CodeAddress>,
    #[cfg(feature = "debug")]
    delivering_execution_trace: bool,
    #[cfg(feature = "debug")]
    helper_trace_units: std::collections::BTreeSet<crate::load::CodeId>,
    #[cfg(feature = "host")]
    host: Option<HostBridge>,
    #[cfg(feature = "debug")]
    pub(crate) debug: crate::debug::DebugController,
    #[cfg(feature = "debug")]
    debug_frame_eval_active: bool,
    #[cfg(feature = "debug")]
    inspecting: std::cell::Cell<bool>,
    #[cfg(feature = "debug")]
    host_callback_depth: u32,
    #[cfg(feature = "debug")]
    variable_accesses: Vec<callbacks::VariableAccess>,
}

impl HybridEngineInner {
    pub(crate) fn new(mode: RuntimeMode) -> Result<Self, VmError> {
        Ok(Self {
            mode,
            policy: ExecPolicy::for_mode(mode),
            frames: Vec::new(),
            next_frame_id: 1,
            next_aux_state_id: 1,
            registry: CommandRegistry::new(),
            rand: Default::default(),
            world: Default::default(),
            output: Default::default(),
            last_completion: Completion::ok(VmObj::empty()),
            last_host_write: None,
            executed_instruction_total: 0,
            executed_by_opcode: Default::default(),
            executed_by_class: Default::default(),
            host_command_invocations: 0,
            first_failure: None,
            #[cfg(feature = "debug")]
            observations: Vec::new(),
            #[cfg(feature = "debug")]
            observation_limit: None,
            #[cfg(feature = "debug")]
            pending_image_procs: Default::default(),
            #[cfg(feature = "debug")]
            image_stub_commands: Default::default(),
            #[cfg(feature = "debug")]
            source_images: Default::default(),
            #[cfg(feature = "debug")]
            generated_cache: Default::default(),
            #[cfg(feature = "debug")]
            command_callback_sites: Default::default(),
            #[cfg(feature = "debug")]
            events: Default::default(),
            #[cfg(feature = "debug")]
            next_event: 1,
            #[cfg(feature = "debug")]
            suspension: None,
            #[cfg(feature = "debug")]
            coroutines: Default::default(),
            #[cfg(feature = "debug")]
            coroutine_boundary: None,
            #[cfg(feature = "debug")]
            coroutine_yield: None,
            #[cfg(feature = "debug")]
            coroutine_identity: (0, 0),
            #[cfg(feature = "debug")]
            coroutine_task: None,
            #[cfg(feature = "debug")]
            control_groups: Default::default(),
            #[cfg(feature = "debug")]
            coroutine_completions: Default::default(),
            #[cfg(feature = "debug")]
            tailcalls: Default::default(),
            #[cfg(feature = "debug")]
            host_callbacks: Default::default(),
            #[cfg(feature = "debug")]
            trace_scopes: Default::default(),
            #[cfg(feature = "debug")]
            traced_commands: Vec::new(),
            #[cfg(feature = "debug")]
            hidden_trace_commands: Default::default(),
            #[cfg(feature = "debug")]
            delivering_execution_trace: false,
            #[cfg(feature = "debug")]
            helper_trace_units: Default::default(),
            #[cfg(feature = "host")]
            host: None,
            #[cfg(feature = "debug")]
            debug: crate::debug::DebugController::default(),
            #[cfg(feature = "debug")]
            debug_frame_eval_active: false,
            #[cfg(feature = "debug")]
            inspecting: std::cell::Cell::new(false),
            #[cfg(feature = "debug")]
            host_callback_depth: 0,
            #[cfg(feature = "debug")]
            variable_accesses: Vec::new(),
        })
    }

    pub(crate) fn mode(&self) -> RuntimeMode {
        self.mode
    }

    pub(crate) fn policy(&self) -> &ExecPolicy {
        &self.policy
    }

    pub(crate) fn registry(&self) -> &CommandRegistry {
        &self.registry
    }

    pub(crate) fn coverage_snapshot(&self) -> CoverageSnapshot {
        let mut counts = [0usize; 4];
        for &opcode in tcld_core::inst_table::DEFAULT_INSTRUCTION_SET.opcode_table {
            counts[match classify(opcode) {
                OpcodeClass::Vm => 0,
                OpcodeClass::HostValue => 1,
                OpcodeClass::HostFrame => 2,
                OpcodeClass::Trap => 3,
            }] += 1;
        }
        CoverageSnapshot {
            opcode_total: counts.iter().sum(),
            vm_count: counts[0],
            host_value_count: counts[1],
            host_frame_count: counts[2],
            trap_count: counts[3],
            executed_instruction_total: self.executed_instruction_total,
            executed_by_opcode: self.executed_by_opcode.clone(),
            executed_by_class: self.executed_by_class.clone(),
            host_command_invocations: self.host_command_invocations,
        }
    }

    pub(crate) fn first_failure(&self) -> Option<&str> {
        self.first_failure.as_deref()
    }

    #[cfg(feature = "debug")]
    pub(crate) fn set_observation_limit(&mut self, limit: Option<usize>) {
        self.observation_limit = limit.map(|n| n.max(1));
        if let Some(limit) = self.observation_limit
            && self.observations.len() > limit
        {
            self.observations.drain(..self.observations.len() - limit);
        }
    }

    #[cfg(feature = "debug")]
    pub(crate) fn observations(&self) -> &[OpcodeObservation] {
        &self.observations
    }

    pub(crate) fn invoke_registered(
        &mut self,
        qname: &[u8],
        args: &[VmObj],
    ) -> Result<Completion, VmError> {
        if !self.frames.is_empty() {
            return Err(VmError::Internal(
                "invoke_registered requires an idle engine".into(),
            ));
        }
        let name = ByteString::from_bytes(qname.to_vec());
        let proc =
            self.registry.get_vm_proc(&name).cloned().ok_or_else(|| {
                VmError::RegisteredCommandNotFound(name.as_str_lossy().into_owned())
            })?;
        match self.run_code_exit(
            Arc::clone(&proc.code),
            FrameKind::VmProc,
            Some(proc.qname.clone()),
            args,
            NamespaceName::from_proc_qname(&proc.qname),
        )? {
            LoopExit::Done(value) => Ok(Completion::ok(value)),
            LoopExit::Completion(completion) => Ok(completion),
            #[cfg(feature = "debug")]
            LoopExit::Paused => Err(VmError::Internal(
                "invoke_registered paused without a debug entrypoint".into(),
            )),
        }
    }

    pub(crate) fn bootstrap(&mut self, argv0: Option<&str>) -> Result<(), VmError> {
        #[cfg(feature = "host")]
        {
            if self.mode == RuntimeMode::Hybrid || self.policy.allow_host_invoke {
                HostBridge::find_executable(argv0);
                let host = HostBridge::new()?;
                host.pkg_provide("tbcload", "1.6")?;
                // Tcl 9 dropped unqualified global variable fallback inside
                // `namespace eval`. Legacy TEBC bodies (e.g. public_complex_fixture.tbc) still use
                // `$tcl_platform(...)` / `$env(...)` without `::` or `global`.
                // Inject upvar links at the start of every `namespace eval` body.
                host.eval_script(TCL8_NAMESPACE_GLOBAL_FALLBACK_SHIM)?;
                host.eval_script(HOST_FRAME_BASE_SHIM)?;
                host.install_frame_commands()?;
                host.eval_script(TYPED_HOST_FRAME_WRAPPER_SHIM)?;
                // Option B façade stubs
                for name in ["tbcload::bcproc", "tbcload::bceval", "tbcproc", "tbceval"] {
                    host.install_stub_command(name)?;
                }
                host.install_stub_command("::tcld::frame_cross")?;
                #[cfg(feature = "debug")]
                host.install_stub_command("::tcld::callback")?;
                self.host = Some(host);
            }
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = argv0;
            if self.mode == RuntimeMode::Hybrid {
                return Err(VmError::HybridUnavailable(
                    "tcld-vm built without `host` feature".into(),
                ));
            }
        }
        Ok(())
    }

    /// Capture stdout and stderr: the host's in hybrid mode, the VM's own in
    /// vm-strict.
    pub(crate) fn begin_output_capture(&mut self) -> Result<(), VmError> {
        if self.mode == RuntimeMode::VmStrict {
            self.output.begin_capture();
            return Ok(());
        }
        #[cfg(feature = "host")]
        {
            self.host
                .as_ref()
                .ok_or_else(|| VmError::HostFrameUnavailable("host is not bootstrapped".into()))?
                .begin_output_capture()
        }
        #[cfg(not(feature = "host"))]
        Err(VmError::HybridUnavailable(
            "built without host feature".into(),
        ))
    }

    pub(crate) fn captured_output(&self) -> Result<(Vec<u8>, Vec<u8>), VmError> {
        if self.mode == RuntimeMode::VmStrict {
            return Ok(self.output.captured());
        }
        #[cfg(feature = "host")]
        {
            let (stdout, stderr) = if let Some(host) = self.host.as_ref() {
                host.captured_output()
            } else {
                #[cfg(feature = "debug")]
                {
                    self.with_suspended_bridge(|h| h.captured_output())
                        .ok_or_else(|| VmError::HybridUnavailable("no active bridge".into()))?
                }
                #[cfg(not(feature = "debug"))]
                {
                    return Err(VmError::HybridUnavailable("no host".into()));
                }
            };
            Ok((stdout.as_bytes().to_vec(), stderr.as_bytes().to_vec()))
        }
        #[cfg(not(feature = "host"))]
        Err(VmError::HybridUnavailable(
            "built without host feature".into(),
        ))
    }

    pub(crate) fn host_setup_eval(&mut self, script: &str) -> Result<Completion, VmError> {
        if !self.frames.is_empty() {
            return Err(VmError::Internal(
                "host setup requires an idle engine".into(),
            ));
        }
        if self.mode != RuntimeMode::Hybrid {
            return Err(VmError::PolicyDenied {
                action: "host_setup",
                detail: "setup is forbidden outside hybrid mode".into(),
            });
        }
        #[cfg(feature = "host")]
        {
            let result = self
                .host
                .as_mut()
                .ok_or_else(|| VmError::HostFrameUnavailable("host is not bootstrapped".into()))?
                .invoke_objv(&[VmObj::from_str("eval"), VmObj::from_str(script)])?;
            match self.map_bridge_result(result)? {
                InvokeOutcome::Value(value) => Ok(Completion::ok(value)),
                InvokeOutcome::Raise(completion) => Ok(completion),
                InvokeOutcome::FramePushed => {
                    Err(VmError::Internal("host setup pushed a VM frame".into()))
                }
            }
        }
        #[cfg(not(feature = "host"))]
        Err(VmError::HybridUnavailable(
            "built without host feature".into(),
        ))
    }

    pub(crate) fn run_tbc(&mut self, top: ByteCode) -> Result<VmObj, VmError> {
        let code = LoadedCode::from_bytecode(top)?;
        self.run_code(code, FrameKind::Toplevel, None, &[], NamespaceName::root())
    }

    pub(crate) fn run_tbc_path(&mut self, path: &Path) -> Result<VmObj, VmError> {
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("tbc"))
        {
            return Err(VmError::Decode(format!(
                "hybrid engine expects a .tbc path, got {}",
                path.display()
            )));
        }
        let code = LoadedCode::from_path(path)?;
        self.run_code(code, FrameKind::Toplevel, None, &[], NamespaceName::root())
    }

    fn run_code(
        &mut self,
        code: Arc<LoadedCode>,
        kind: FrameKind,
        proc_qname: Option<ByteString>,
        arg_values: &[VmObj],
        namespace: NamespaceName,
    ) -> Result<VmObj, VmError> {
        match self.run_code_exit(code, kind, proc_qname, arg_values, namespace)? {
            LoopExit::Done(v) => Ok(v),
            LoopExit::Completion(completion) => Err(completion_to_error(completion)),
            #[cfg(feature = "debug")]
            LoopExit::Paused => Err(VmError::Internal("unexpected debug pause".into())),
        }
    }

    fn run_code_exit(
        &mut self,
        code: Arc<LoadedCode>,
        kind: FrameKind,
        proc_qname: Option<ByteString>,
        arg_values: &[VmObj],
        namespace: NamespaceName,
    ) -> Result<LoopExit, VmError> {
        let boundary_depth = self.frames.len();
        self.push_frame(code, kind, proc_qname, arg_values, namespace, None)?;
        let result = self.eval_loop(boundary_depth, true);
        if let Err(error) = &result {
            if self.first_failure.is_none() {
                self.first_failure = Some(error.to_string());
            }
            self.frames.truncate(boundary_depth);
        }
        result
    }

    fn push_frame(
        &mut self,
        code: Arc<LoadedCode>,
        kind: FrameKind,
        proc_qname: Option<ByteString>,
        arg_values: &[VmObj],
        namespace: NamespaceName,
        continuation: Option<CallContinuation>,
    ) -> Result<FrameId, VmError> {
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(VmError::CallDepth);
        }

        let meta = locals_meta_from_bc(&code.bytecode.locals)?;
        let mut compiled_locals = Vec::with_capacity(code.bytecode.proc_num_locals as usize);
        for i in 0..code.bytecode.proc_num_locals as usize {
            if let Some(m) = meta.get(i) {
                compiled_locals.push(LocalSlot::undefined(m.name.clone(), m.flag_mask));
            } else {
                compiled_locals.push(LocalSlot::undefined(
                    ByteString::from(format!("tmp{i}").as_str()),
                    0,
                ));
            }
        }

        if kind == FrameKind::VmProc {
            self.bind_args(&mut compiled_locals, &meta, &code, arg_values)?;
        }

        // Seed local_vars name map from LVT for name-based ops that hit formals.
        let mut local_vars = VarTable::new();
        for slot in &compiled_locals {
            if let Some(v) = &slot.value {
                local_vars.store_scalar(slot.name.clone(), v.clone());
            }
        }

        let tcl_parent = if self.frames.is_empty() {
            None
        } else {
            Some(self.frames[self.scope_index()].id)
        };
        let id = FrameId(self.next_frame_id);
        let frame = CallFrame {
            id,
            code,
            pc_index: 0,
            stack: Vec::new(),
            compiled_locals,
            local_vars,
            scope_parent: None,
            script_path: None,
            tcl_parent,
            hidden_scope: false,
            invocation: None,
            #[cfg(feature = "debug")]
            body_error_context: None,
            #[cfg(feature = "debug")]
            lmap: None,
            #[cfg(feature = "debug")]
            try_state: None,
            tail_root: false,
            catch_stack: Vec::new(),
            loop_marks: std::collections::BTreeMap::new(),
            aux_states: Vec::new(),
            namespace,
            kind,
            proc_qname,
            continuation,
        };
        self.next_frame_id += 1;
        self.frames.push(frame);
        #[cfg(feature = "debug")]
        if matches!(kind, FrameKind::Toplevel | FrameKind::Namespace)
            && self
                .source_images
                .contains_key(&self.frames[self.fi()].code.id)
        {
            let names = self.frames[self.fi()]
                .compiled_locals
                .iter()
                .filter(|s| !s.is_temporary())
                .map(|s| s.name.clone())
                .collect::<Vec<_>>();
            for name in names {
                let qualified = self.frames[self.fi()].namespace.qualify(&name);
                self.install_host_link_named(&name, qualified, None)?;
            }
            self.frames.last_mut().unwrap().scope_parent = Some(id);
        }
        #[cfg(feature = "debug")]
        if kind == FrameKind::VmProc
            && self
                .source_images
                .contains_key(&self.frames[self.fi()].code.id)
        {
            let names = self.frames[self.fi()]
                .compiled_locals
                .iter()
                .filter(|s| s.name.as_str_lossy().contains("::"))
                .map(|s| s.name.clone())
                .collect::<Vec<_>>();
            for name in names {
                let qualified = self.frames[self.fi()].namespace.qualify(&name);
                self.install_host_link_named(&name, qualified, None)?;
            }
        }
        Ok(id)
    }

    #[cfg(feature = "debug")]
    pub(crate) fn enable_debug(&mut self, stop_on_entry: bool) {
        self.debug = crate::debug::DebugController::new(stop_on_entry);
    }

    #[cfg(feature = "debug")]
    pub(crate) fn debug_start_code(
        &mut self,
        code: Arc<LoadedCode>,
        kind: FrameKind,
        proc_qname: Option<ByteString>,
        arg_values: &[VmObj],
    ) -> Result<crate::debug::DebugOutcome, VmError> {
        if !self.debug.enabled {
            self.enable_debug(true);
        }
        self.cancel_suspension();
        self.frames.clear();
        self.push_frame(
            code,
            kind,
            proc_qname,
            arg_values,
            NamespaceName::root(),
            None,
        )?;
        match self.resume_suspension()? {
            LoopExit::Paused => {
                let snap = self
                    .debug
                    .last_pause
                    .clone()
                    .ok_or_else(|| VmError::Internal("paused without snapshot".into()))?;
                Ok(crate::debug::DebugOutcome::Paused(snap))
            }
            LoopExit::Done(v) => {
                self.debug.suspended = false;
                Ok(crate::debug::DebugOutcome::Finished { result: v })
            }
            LoopExit::Completion(completion) => {
                self.debug.suspended = false;
                Err(completion_to_error(completion))
            }
        }
    }

    #[cfg(feature = "debug")]
    // Debug images hold Rc-backed bytecode; Arc is the VM's sharing handle
    // here too, not a promise of cross-thread use.
    #[allow(clippy::arc_with_non_send_sync)]
    pub(crate) fn debug_start_image(
        &mut self,
        image: &tcld_compile::DebugImage,
        stop_on_entry: bool,
    ) -> Result<crate::DebugImageStart, VmError> {
        self.cancel_suspension();
        self.clear_coroutines();
        self.tailcalls.clear();
        self.traced_commands.clear();
        self.hidden_trace_commands.clear();
        self.helper_trace_units.clear();
        self.clear_host_callbacks();
        // An image is a closed executable generation. Do not retain procedure
        // registrations from an earlier image/restart.
        #[cfg(feature = "host")]
        if let Some(host) = self.host.as_ref() {
            for qname in &self.image_stub_commands {
                host.uninstall_stub_command(qname)?;
            }
        }
        self.image_stub_commands.clear();
        self.registry = CommandRegistry::new();
        self.pending_image_procs.clear();
        self.source_images.clear();
        self.generated_cache.clear();
        self.command_callback_sites.clear();
        self.events.clear();
        self.next_event = 1;
        let top = self.register_source_image(Arc::new(image.clone()), None)?;
        let bindings = self
            .source_images
            .values()
            .map(|source| crate::DebugImageBinding {
                logical_unit_id: source.image.units[source.unit_index].logical_id.clone(),
                code_id: source.code_id,
            })
            .collect();
        self.enable_debug(stop_on_entry);
        let outcome = self.debug_start_code(top, FrameKind::Toplevel, None, &[])?;
        Ok(crate::DebugImageStart { outcome, bindings })
    }

    #[cfg(feature = "debug")]
    fn activate_image_proc(&mut self, address: CodeAddress) -> Result<(), VmError> {
        let Some(proc) = self.pending_image_procs.get(&address).cloned() else {
            return Ok(());
        };
        #[cfg(feature = "host")]
        {
            let qname = proc.qname.as_str_lossy().into_owned();
            if let Some(host) = self.host.as_ref() {
                host.install_stub_command(&qname)?;
            } else if let Some(result) =
                HostBridge::with_active_bridge(|h| h.install_stub_command(&qname))
            {
                result?;
            }
            self.image_stub_commands.insert(qname);
        }
        self.registry.register_vm_proc(proc);
        Ok(())
    }

    #[cfg(feature = "debug")]
    pub(crate) fn debug_resume(
        &mut self,
        arm: impl FnOnce(&mut crate::debug::DebugController),
    ) -> Result<crate::debug::DebugOutcome, VmError> {
        if self.frames.is_empty() {
            return Err(VmError::Internal(
                "debug_resume with no suspended frames".into(),
            ));
        }
        arm(&mut self.debug);
        self.debug.suspended = false;
        match self.resume_suspension() {
            Ok(LoopExit::Paused) => {
                let snap = self
                    .debug
                    .last_pause
                    .clone()
                    .ok_or_else(|| VmError::Internal("paused without snapshot".into()))?;
                Ok(crate::debug::DebugOutcome::Paused(snap))
            }
            Ok(LoopExit::Done(v)) => {
                self.debug.suspended = false;
                let snap = crate::debug::PauseSnapshot {
                    address: None,
                    frame_id: None,
                    frames: vec![],
                    pc: None,
                    location: None,
                    in_host: false,
                    reason: crate::debug::PauseReason::Completed,
                    opcode: None,
                    proc_qname: None,
                    frame_kind: "done".into(),
                    locals: vec![],
                    stack_depth: 0,
                    call_depth: 0,
                };
                self.debug.last_pause = Some(snap);
                Ok(crate::debug::DebugOutcome::Finished { result: v })
            }
            Ok(LoopExit::Completion(completion)) => {
                self.frames.clear();
                self.debug.suspended = false;
                self.debug.last_pause = None;
                Err(completion_to_error(completion))
            }
            Err(e) => {
                // Mirror run_code_exit: never leave a corrupt suspended stack.
                if self.first_failure.is_none() {
                    self.first_failure = Some(e.to_string());
                }
                self.frames.clear();
                self.debug.suspended = false;
                self.debug.last_pause = None;
                Err(e)
            }
        }
    }

    #[cfg(feature = "debug")]
    fn build_pause_snapshot(
        &self,
        reason: crate::debug::PauseReason,
        inst: &crate::load::DecodedInst,
    ) -> crate::debug::PauseSnapshot {
        let fi = self.frames.len().saturating_sub(1);
        let frame = &self.frames[fi];
        let locals = self.ui_frame_locals(0);
        let frame_kind = match frame.kind {
            FrameKind::Toplevel => "toplevel",
            FrameKind::VmProc => "proc",
            FrameKind::Lambda => "lambda",
            FrameKind::Dynamic => "generated",
            FrameKind::Namespace => "namespace",
            FrameKind::Source => "source",
        };
        let address = CodeAddress {
            code_id: frame.code.id,
            pc: inst.pc,
        };
        crate::debug::PauseSnapshot {
            address: Some(address),
            frame_id: Some(frame.id),
            frames: self.ui_stack_frames(),
            pc: Some(inst.pc),
            location: None,
            in_host: false,
            reason,
            opcode: Some(format!("{:?}", inst.opcode)),
            proc_qname: frame
                .proc_qname
                .as_ref()
                .map(|s| s.as_str_lossy().into_owned()),
            frame_kind: frame_kind.into(),
            locals,
            stack_depth: frame.stack.len(),
            call_depth: self.frames.len(),
        }
    }

    #[cfg(feature = "debug")]
    pub(crate) fn pause_snapshot(&self) -> Option<crate::debug::PauseSnapshot> {
        self.debug.last_pause.clone()
    }

    #[cfg(feature = "debug")]
    pub(crate) fn add_pc_breakpoint(&mut self, pc: u32) {
        if let Some(code_id) = self.frames.last().map(|frame| frame.code.id) {
            self.debug.breakpoints.insert(CodeAddress { code_id, pc });
        }
    }

    #[cfg(feature = "debug")]
    pub(crate) fn remove_pc_breakpoint(&mut self, pc: u32) {
        if let Some(code_id) = self.frames.last().map(|frame| frame.code.id) {
            self.debug.breakpoints.remove(&CodeAddress { code_id, pc });
        }
    }

    #[cfg(feature = "debug")]
    pub(crate) fn add_breakpoint(&mut self, address: CodeAddress) {
        self.debug.breakpoints.insert(address);
    }

    #[cfg(feature = "debug")]
    pub(crate) fn remove_breakpoint(&mut self, address: CodeAddress) {
        self.debug.breakpoints.remove(&address);
    }

    #[cfg(feature = "debug")]
    pub(crate) fn debug_step_over(&mut self) -> Result<crate::debug::DebugOutcome, VmError> {
        let frame_id = self
            .frames
            .last()
            .map(|frame| frame.id)
            .ok_or_else(|| VmError::Internal("step over without a frame".into()))?;
        self.debug_resume(|debug| debug.arm_step_over(frame_id))
    }

    #[cfg(feature = "debug")]
    pub(crate) fn debug_step_out(&mut self) -> Result<crate::debug::DebugOutcome, VmError> {
        let frame_id = self
            .frames
            .last()
            .map(|frame| frame.id)
            .ok_or_else(|| VmError::Internal("step out without a frame".into()))?;
        self.debug_resume(|debug| debug.arm_step_out(frame_id))
    }

    #[cfg(feature = "debug")]
    pub(crate) fn debug_run_to_pc(
        &mut self,
        pc: u32,
    ) -> Result<crate::debug::DebugOutcome, VmError> {
        let code_id = self
            .frames
            .last()
            .map(|frame| frame.code.id)
            .ok_or_else(|| VmError::Internal("run to pc without a frame".into()))?;
        self.debug_resume(|debug| debug.arm_run_to_address(CodeAddress { code_id, pc }))
    }

    #[cfg(feature = "debug")]
    pub(crate) fn debug_run_to_address(
        &mut self,
        address: CodeAddress,
    ) -> Result<crate::debug::DebugOutcome, VmError> {
        self.debug_resume(|debug| debug.arm_run_to_address(address))
    }

    #[cfg(feature = "debug")]
    pub(crate) fn ui_stack_frames(&self) -> Vec<UiStackFrame> {
        self.frames
            .iter()
            .rev()
            .map(|f| {
                let pc = f
                    .code
                    .instructions
                    .get(f.pc_index)
                    .map(|i| i.pc)
                    .or_else(|| f.code.instructions.last().map(|i| i.pc));
                UiStackFrame {
                    frame_id: f.id,
                    address: pc.map(|pc| CodeAddress {
                        code_id: f.code.id,
                        pc,
                    }),
                    kind_label: match f.kind {
                        FrameKind::Toplevel => "toplevel".into(),
                        FrameKind::VmProc => "proc".into(),
                        FrameKind::Lambda => "lambda".into(),
                        FrameKind::Dynamic => "generated".into(),
                        FrameKind::Namespace => "namespace".into(),
                        FrameKind::Source => "source".into(),
                    },
                    proc_qname: f.proc_qname.as_ref().map(|s| s.as_str_lossy().into_owned()),
                    namespace: f.namespace.as_bytes().as_str_lossy().into_owned(),
                    pc,
                    source_offset: pc.and_then(|pc| source_offset_for_pc(&f.code.bytecode, pc)),
                    in_host: false,
                }
            })
            .collect()
    }

    #[cfg(feature = "debug")]
    fn ui_link_value(&self, link: &VarLink, depth: usize) -> Option<VmObj> {
        if depth > MAX_CALL_DEPTH {
            return None;
        }
        match link {
            VarLink::HostVar { name1, name2 } => self.host_get(name1, name2.as_ref()).ok(),
            VarLink::Vm {
                target_frame,
                target_name,
                part2,
            } => {
                let frame = self.frames.iter().find(|f| f.id == *target_frame)?;
                if let Some(slot) = frame
                    .compiled_locals
                    .iter()
                    .find(|s| s.is_named(target_name))
                {
                    if let Some(link) = &slot.link {
                        return self.ui_link_value(link, depth + 1);
                    }
                    if part2.is_none() && slot.value.is_some() {
                        return slot.value.clone();
                    }
                }
                match frame.local_vars.get_slot(target_name)? {
                    VarSlot::Scalar(v) if part2.is_none() => Some(v.clone()),
                    VarSlot::Array(items) => part2.as_ref().and_then(|key| items.get(key).cloned()),
                    VarSlot::Link(link) => self.ui_link_value(link, depth + 1),
                    _ => None,
                }
            }
        }
    }

    #[cfg(feature = "debug")]
    pub(crate) fn ui_frame_locals(&self, level: usize) -> Vec<(String, String)> {
        let Some(frame) = self.frames.iter().rev().nth(level) else {
            return Vec::new();
        };
        let frame = frame
            .scope_parent
            .and_then(|id| self.frames.iter().find(|f| f.id == id))
            .unwrap_or(frame);
        let previous_inspecting = self.inspecting.replace(true);
        let mut values = std::collections::BTreeMap::new();
        for slot in frame.compiled_locals.iter().filter(|s| !s.is_temporary()) {
            values.insert(
                slot.name.as_str_lossy().into_owned(),
                slot.link
                    .as_ref()
                    .and_then(|link| self.ui_link_value(link, 0))
                    .or_else(|| slot.value.clone())
                    .map(|v| v.as_string().unwrap_or_else(|_| format!("{v:?}")))
                    .unwrap_or_else(|| "<undefined>".into()),
            );
        }
        for (name, slot) in frame.local_vars.iter_slots() {
            match slot {
                VarSlot::Scalar(value) => {
                    values.insert(
                        name.as_str_lossy().into_owned(),
                        value.as_string().unwrap_or_else(|_| format!("{value:?}")),
                    );
                }
                VarSlot::Array(items) => {
                    let mut entries = items
                        .iter()
                        .map(|(k, v)| {
                            format!("{}={}", k.as_str_lossy(), v.as_string().unwrap_or_default())
                        })
                        .collect::<Vec<_>>();
                    entries.sort();
                    values.insert(
                        name.as_str_lossy().into_owned(),
                        format!("array: {}", entries.join(", ")),
                    );
                }
                VarSlot::Link(link) => {
                    if let Some(value) = self.ui_link_value(link, 0) {
                        values.insert(
                            name.as_str_lossy().into_owned(),
                            value.as_string().unwrap_or_default(),
                        );
                    }
                }
                _ => {}
            }
        }
        self.inspecting.set(previous_inspecting);
        values.into_iter().collect()
    }

    #[cfg(feature = "debug")]
    pub(crate) fn debug_current_code(&self) -> Option<(crate::load::CodeId, tcld_core::ByteCode)> {
        self.frames
            .last()
            .map(|frame| (frame.code.id, frame.code.bytecode.clone()))
    }

    /// Evaluate Tcl in a suspended VM frame without resuming the scheduler.
    ///
    /// The selected frame is moved to the top only for the duration of the
    /// HostFrame mirror operation. Re-entry through a registered VM command is
    /// refused while this temporary ordering is active.
    #[cfg(feature = "debug")]
    pub(crate) fn debug_eval_in_frame(
        &mut self,
        level: usize,
        body: VmObj,
        expression: bool,
    ) -> Result<Completion, VmError> {
        if !self.debug.suspended {
            return Err(VmError::HostFrameUnavailable(
                "eval-in-frame requires a suspended VM frame".into(),
            ));
        }
        if !self.policy.allow_dynamic_eval {
            return Err(VmError::PolicyDenied {
                action: "dynamic_eval",
                detail: "dynamic_eval denied by vm-strict".into(),
            });
        }
        let frame_count = self.frames.len();
        let selected = frame_count.checked_sub(level + 1).ok_or_else(|| {
            VmError::HostFrameUnavailable(format!("frame level {level} is not available"))
        })?;
        let top = frame_count - 1;
        self.frames.swap(selected, top);
        self.debug_frame_eval_active = true;
        let evaluated = self
            .host_frame_eval(body, expression)
            .and_then(|flow| match flow {
                Flow::Continue => Ok(Completion::ok(self.pop(0, "debug eval result")?)),
                Flow::Raise(completion) => Ok(completion),
                Flow::JumpPc(_) | Flow::FrameDone(_) => Err(VmError::Internal(
                    "debug frame eval returned scheduler flow".into(),
                )),
            });
        self.debug_frame_eval_active = false;
        self.frames.swap(selected, top);
        evaluated
    }

    fn bind_args(
        &self,
        locals: &mut [LocalSlot],
        meta: &[LocalMeta],
        code: &LoadedCode,
        arg_values: &[VmObj],
    ) -> Result<(), VmError> {
        let num_args = code.bytecode.proc_num_args as usize;
        if num_args == 0 {
            return if arg_values.is_empty() {
                Ok(())
            } else {
                Err(self.wrong_args(code, meta))
            };
        }

        let has_args = meta
            .get(num_args.saturating_sub(1))
            .map(|m| m.name.as_bytes() == b"args")
            .unwrap_or(false);

        if has_args {
            let fixed = num_args - 1;
            for i in 0..fixed {
                if i < arg_values.len() {
                    locals[i].value = Some(arg_values[i].clone());
                } else if let Some(d) = meta.get(i).and_then(|m| m.default_value.clone()) {
                    locals[i].value = Some(d);
                } else {
                    return Err(self.wrong_args(code, meta));
                }
            }
            let rest: Vec<VmObj> = arg_values.get(fixed..).unwrap_or(&[]).to_vec();
            let list_s = list_to_string(&rest)?;
            locals[fixed].value = Some(VmObj::from_str(&list_s));
        } else {
            if arg_values.len() > num_args {
                return Err(self.wrong_args(code, meta));
            }
            for i in 0..num_args {
                if i < arg_values.len() {
                    locals[i].value = Some(arg_values[i].clone());
                } else if let Some(d) = meta.get(i).and_then(|m| m.default_value.clone()) {
                    locals[i].value = Some(d);
                } else {
                    return Err(self.wrong_args(code, meta));
                }
            }
        }
        Ok(())
    }

    /// Tcl's usage message, with optional parameters as `?name?` and a final
    /// `args` as `?arg ...?`. The invoking site replaces the leading `proc`
    /// with the command word the caller used, as Tcl reports it.
    fn wrong_args(&self, code: &LoadedCode, meta: &[LocalMeta]) -> VmError {
        let num_args = code.bytecode.proc_num_args as usize;
        let mut usage = vec!["proc".to_string()];
        for (index, m) in meta.iter().take(num_args).enumerate() {
            let name = m.name.as_str_lossy();
            usage.push(if index + 1 == num_args && name == "args" {
                "?arg ...?".to_string()
            } else if m.default_value.is_some() {
                format!("?{name}?")
            } else {
                name.into_owned()
            });
        }
        VmError::TclError {
            message: format!("wrong # args: should be \"{}\"", usage.join(" ")),
            error_info: None,
            error_code: Some("TCL WRONGARGS".into()),
        }
    }

    fn eval_loop(
        &mut self,
        boundary_depth: usize,
        allow_debug_pause: bool,
    ) -> Result<LoopExit, VmError> {
        loop {
            #[cfg(feature = "debug")]
            if self.coroutine_boundary == Some(boundary_depth) && self.coroutine_yield.is_some() {
                return Ok(LoopExit::Paused);
            }
            #[cfg(feature = "debug")]
            if self.suspension.as_ref().is_some_and(|s| s.cancelling) {
                return Err(VmError::Internal("debug session cancelled".into()));
            }
            let frame_len = self.frames.len();
            if frame_len <= boundary_depth {
                return Err(VmError::Internal(
                    "scheduler crossed its frame boundary".into(),
                ));
            }
            let fi = frame_len - 1;
            let pc_index = self.frames[fi].pc_index;
            let code = Arc::clone(&self.frames[fi].code);

            if pc_index >= code.instructions.len() {
                if let Some(exit) = self.finish_current_frame(VmObj::empty(), boundary_depth)? {
                    return Ok(exit);
                }
                continue;
            }

            let inst = code.instructions[pc_index].clone();

            #[cfg(feature = "debug")]
            if allow_debug_pause {
                let address = CodeAddress {
                    code_id: code.id,
                    pc: inst.pc,
                };
                let frame_id = self.frames[fi].id;
                let active_frames: Vec<FrameId> =
                    self.frames.iter().map(|frame| frame.id).collect();
                if let Some(reason) =
                    self.debug
                        .should_pause_before(address, frame_id, &active_frames)
                {
                    let snap = self.build_pause_snapshot(reason, &inst);
                    self.debug.last_pause = Some(snap);
                    self.debug.suspended = true;
                    self.debug.clear_entry_arm();
                    if !self.pause_suspension()? {
                        return Ok(LoopExit::Paused);
                    }
                }
            }
            #[cfg(not(feature = "debug"))]
            let _ = allow_debug_pause;

            #[cfg(feature = "debug")]
            self.activate_image_proc(CodeAddress {
                code_id: code.id,
                pc: inst.pc,
            })?;

            self.frames[fi].pc_index = pc_index + 1;

            let opcode_class = classify(inst.opcode);
            self.executed_instruction_total += 1;
            *self
                .executed_by_opcode
                .entry(format!("{:?}", inst.opcode))
                .or_default() += 1;
            *self
                .executed_by_class
                .entry(format!("{opcode_class:?}"))
                .or_default() += 1;
            #[cfg(feature = "debug")]
            {
                if let Some(limit) = self.observation_limit
                    && self.observations.len() >= limit
                {
                    let discard = (limit / 2).max(1);
                    self.observations.drain(..discard);
                }
                self.observations.push(OpcodeObservation {
                    code_id: code.id,
                    pc: inst.pc,
                    opcode: inst.opcode,
                    class: opcode_class,
                    frame_id: self.frames[fi].id,
                });
            }
            match opcode_class {
                OpcodeClass::Trap => {
                    return Err(VmError::UnimplementedOpcode {
                        opcode: format!("{:?}", inst.opcode),
                        pc: inst.pc as u64,
                    });
                }
                OpcodeClass::HostValue if !self.policy.allow_host_opcode => {
                    return Err(VmError::PolicyDenied {
                        action: "host_opcode",
                        detail: format!("{:?}", inst.opcode),
                    });
                }
                OpcodeClass::HostFrame if !self.policy.allow_dynamic_eval => {
                    return Err(VmError::PolicyDenied {
                        action: "dynamic_eval",
                        detail: format!("{:?}", inst.opcode),
                    });
                }
                _ => {}
            }

            if self.frames[fi]
                .code
                .bytecode
                .exceptions
                .iter()
                .any(|range| {
                    range.kind == ExceptionType::Loop && range.code_range.start == inst.pc as i32
                })
            {
                let frame = &mut self.frames[fi];
                frame.loop_marks.insert(
                    inst.pc as i32,
                    (
                        frame.stack.len(),
                        frame.aux_states.len(),
                        frame.catch_stack.len(),
                    ),
                );
            }
            self.last_host_write = None;
            #[cfg(feature = "debug")]
            let dispatched = self
                .execution_boundary(inst.pc)
                .and_then(|flow| match flow {
                    Some(flow) => Ok(flow),
                    None => self.dispatch(&inst),
                });
            #[cfg(not(feature = "debug"))]
            let dispatched = self.dispatch(&inst);
            let flow = match dispatched {
                Ok(flow) => flow,
                Err(VmError::TclError {
                    message,
                    error_info,
                    error_code,
                }) => Flow::Raise(Completion::error(
                    VmObj::from_str(&message),
                    error_info.map(ByteString::from),
                    error_code.map(|value| VmObj::from_str(&value)),
                )),
                Err(error) => return Err(error),
            };
            if matches!(flow, Flow::Continue) {
                let op = format!("{:?}", inst.opcode).to_ascii_lowercase();
                if ["store", "incr", "append", "lappend"]
                    .iter()
                    .any(|p| op.starts_with(p))
                    && let Some(value) = self.last_host_write.take()
                    && let Some(top) = self.frames[fi].stack.last_mut()
                {
                    *top = value;
                }
            }
            match flow {
                Flow::Continue => {}
                Flow::JumpPc(pc) => {
                    let idx = self.frames[fi].code.index_for_pc(pc).map_err(|e| match e {
                        VmError::Internal(msg) => VmError::Internal(format!(
                            "{msg} (from {:?} at pc {})",
                            inst.opcode, inst.pc
                        )),
                        other => other,
                    })?;
                    self.frames[fi].pc_index = idx;
                }
                Flow::FrameDone(value) => {
                    if let Some(exit) = self.finish_current_frame(value, boundary_depth)? {
                        return Ok(exit);
                    }
                }
                Flow::Raise(completion) => {
                    if let Some(exit) =
                        self.route_completion(completion, inst.pc, boundary_depth)?
                    {
                        return Ok(exit);
                    }
                }
            }
        }
    }

    fn finish_current_frame(
        &mut self,
        result: VmObj,
        boundary_depth: usize,
    ) -> Result<Option<LoopExit>, VmError> {
        #[cfg(feature = "debug")]
        let mut result = result;
        #[cfg(feature = "debug")]
        {
            let completion =
                self.unwind_execution_commands(None, Completion::ok(result.clone()))?;
            if completion.code != 0 {
                return self.route_completion(
                    completion,
                    self.frames[self.fi()].code.instructions
                        [self.frames[self.fi()].pc_index.saturating_sub(1)]
                    .pc,
                    boundary_depth,
                );
            }
            if self.frames[self.fi()].try_state.is_some() {
                return self.route_completion(
                    Completion::ok(result),
                    self.frames[self.fi()].code.instructions
                        [self.frames[self.fi()].pc_index.saturating_sub(1)]
                    .pc,
                    boundary_depth,
                );
            }
            if self.frames[self.fi()].lmap.is_some() {
                match self.resume_lmap(Completion::ok(result))? {
                    None => return Ok(None),
                    Some(completion) if completion.code == 0 => result = completion.result,
                    Some(completion) => {
                        return self.route_completion(
                            completion,
                            self.frames[self.fi()].code.instructions
                                [self.frames[self.fi()].pc_index.saturating_sub(1)]
                            .pc,
                            boundary_depth,
                        );
                    }
                }
            }
            self.cleanup_trace_scope();
        }
        let finished = self
            .frames
            .pop()
            .ok_or_else(|| VmError::Internal("frame completion with empty stack".into()))?;
        #[cfg(feature = "debug")]
        if let Some(tail) = self.tailcalls.remove(&finished.id.0) {
            return self.perform_tailcall(tail, finished, result, boundary_depth);
        }
        self.complete_popped_frame(finished, result, boundary_depth)
    }

    fn complete_popped_frame(
        &mut self,
        finished: CallFrame,
        result: VmObj,
        boundary_depth: usize,
    ) -> Result<Option<LoopExit>, VmError> {
        if let Some(continuation) = finished.continuation {
            let caller = self
                .frames
                .last_mut()
                .ok_or_else(|| VmError::Internal("VM continuation lost its caller".into()))?;
            if caller.id != continuation.caller {
                return Err(VmError::Internal(
                    "VM continuation caller does not match scheduler stack".into(),
                ));
            }
            caller.pc_index = continuation.resume_pc_index;
            caller.stack.truncate(continuation.caller_stack_depth);
            caller.stack.push(result);
            return Ok(None);
        }
        if self.frames.len() != boundary_depth {
            return Err(VmError::Internal(
                "boundary frame completed at the wrong scheduler depth".into(),
            ));
        }
        Ok(Some(LoopExit::Done(result)))
    }

    fn route_completion(
        &mut self,
        mut completion: Completion,
        mut producing_pc: u32,
        boundary_depth: usize,
    ) -> Result<Option<LoopExit>, VmError> {
        let origin_frame = self.frames[self.fi()].id;
        loop {
            let fi = self.fi();
            let catch_candidate = self.frames[fi].catch_stack.last().and_then(|mark| {
                self.frames[fi]
                    .code
                    .bytecode
                    .exceptions
                    .get(mark.range_index)
                    .filter(|range| {
                        range.kind == ExceptionType::Catch
                            && range.catch_offset >= 0
                            && range.code_range.contains(&(producing_pc as i32))
                    })
                    .map(|range| (mark.stack_depth, mark.aux_state_depth, range.clone()))
            });
            let loop_candidate = if completion.code == 3 || completion.code == 4 {
                find_loop_range(
                    &self.frames[fi].code.bytecode.exceptions,
                    producing_pc as i32,
                )
                .cloned()
            } else {
                None
            };

            let catch_is_closest = catch_candidate.as_ref().is_some_and(|(_, _, catch_range)| {
                loop_candidate
                    .as_ref()
                    .is_none_or(|loop_range| catch_range.nesting_level >= loop_range.nesting_level)
            });
            if catch_is_closest
                && let Some((stack_depth, aux_state_depth, catch_range)) = catch_candidate
            {
                #[cfg(feature = "debug")]
                {
                    completion = self.unwind_execution_commands(
                        Some(catch_range.catch_offset as u32),
                        completion,
                    )?;
                }
                self.last_completion = completion;
                self.frames[fi].stack.truncate(stack_depth);
                self.frames[fi].aux_states.truncate(aux_state_depth);
                self.frames[fi].pc_index = self.frames[fi]
                    .code
                    .index_for_pc(catch_range.catch_offset as u32)?;
                return Ok(None);
            }

            if (completion.code == 3 || completion.code == 4)
                && let Some(range) = loop_candidate
            {
                let target = if completion.code == 3 {
                    range.break_offset
                } else {
                    range.continue_offset
                };
                if target >= 0 {
                    #[cfg(feature = "debug")]
                    {
                        completion =
                            self.unwind_execution_commands(Some(target as u32), completion)?;
                        if completion.code != 3 && completion.code != 4 {
                            continue;
                        }
                    }
                    if let Some((stack, aux, catch)) = self.frames[fi]
                        .loop_marks
                        .get(&range.code_range.start)
                        .copied()
                    {
                        self.frames[fi].stack.truncate(stack);
                        self.frames[fi].aux_states.truncate(aux);
                        self.frames[fi].catch_stack.truncate(catch);
                    }
                    self.consume_loop_completion(&completion)?;
                    self.frames[fi].pc_index = self.frames[fi].code.index_for_pc(target as u32)?;
                    return Ok(None);
                }
            }

            #[cfg(feature = "debug")]
            {
                completion = self.unwind_execution_commands(None, completion)?;
            }
            #[cfg(feature = "debug")]
            if self.frames[fi].try_state.is_some() {
                match self.resume_try(completion, producing_pc)? {
                    None => return Ok(None),
                    Some(finished) => completion = finished,
                }
            }
            #[cfg(feature = "debug")]
            if self.frames[fi].lmap.is_some() && matches!(completion.code, 0 | 3 | 4) {
                match self.resume_lmap(completion)? {
                    None => return Ok(None),
                    Some(finished) => completion = finished,
                }
            }
            // Raw loop control cannot cross a procedure boundary. Do this
            // before consuming return levels: `return -code break` is allowed
            // to produce a break in the caller, just as in Tcl.
            if matches!(self.frames[fi].kind, FrameKind::VmProc | FrameKind::Lambda)
                && matches!(completion.code, 3 | 4)
            {
                completion.reject_loop_control()?;
            }
            #[cfg(feature = "debug")]
            if self.frames[fi].kind == FrameKind::Lambda {
                self.lambda_error_context(&mut completion, producing_pc)?;
            }
            #[cfg(feature = "debug")]
            if completion.code == 1
                && let Some(context) = &self.frames[fi].body_error_context
            {
                completion.append_error_context(&format!(
                    "\n    ({context} line {})",
                    self.mapped_error_line(producing_pc),
                ))?;
            }
            if !matches!(
                self.frames[fi].kind,
                FrameKind::Dynamic | FrameKind::Namespace
            ) && (!self.frames[fi].hidden_scope || self.frames[fi].id == origin_frame)
                && completion.code == 2
                && completion.options.level > 0
            {
                completion.options.consume_return_level()?;
                if completion.options.level == 0 {
                    completion.code = completion.options.code;
                } else if self.frames.len() == boundary_depth + 1 {
                    return Err(VmError::CrossRuntimeReturnLevel);
                }
            }

            #[cfg(feature = "debug")]
            self.cleanup_trace_scope();
            let finished = self
                .frames
                .pop()
                .ok_or_else(|| VmError::Internal("completion with empty frame stack".into()))?;
            #[cfg(feature = "debug")]
            if let Some(tail) = self.tailcalls.remove(&finished.id.0) {
                if completion.code == 0 {
                    return self.perform_tailcall(
                        tail,
                        finished,
                        completion.result,
                        boundary_depth,
                    );
                }
                // A pending native tail is preempted by an error/non-OK return.
                drop(tail);
            }
            if let Some(continuation) = finished.continuation {
                let caller = self
                    .frames
                    .last_mut()
                    .ok_or_else(|| VmError::Internal("completion lost its VM caller".into()))?;
                if caller.id != continuation.caller {
                    return Err(VmError::Internal(
                        "completion caller does not match scheduler stack".into(),
                    ));
                }
                caller.pc_index = continuation.resume_pc_index;
                caller.stack.truncate(continuation.caller_stack_depth);
                if completion.code == 0 {
                    self.last_completion = completion.clone();
                    caller.stack.push(completion.result);
                    return Ok(None);
                }
                producing_pc = continuation.call_site.pc;
                continue;
            }

            if self.frames.len() != boundary_depth {
                return Err(VmError::Internal(
                    "completion crossed an opaque scheduler boundary".into(),
                ));
            }
            return if completion.code == 0 {
                Ok(Some(LoopExit::Done(completion.result)))
            } else {
                Ok(Some(LoopExit::Completion(completion)))
            };
        }
    }

    fn dispatch(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let op = inst.opcode;
        match op {
            Opcode::Done => {
                let v = self.pop(pc, "done").unwrap_or_else(|_| VmObj::empty());
                Ok(Flow::FrameDone(v))
            }
            Opcode::Nop | Opcode::StartCmd => Ok(Flow::Continue),
            Opcode::Push1 | Opcode::Push4 => {
                let lit = inst
                    .lit
                    .clone()
                    .ok_or_else(|| VmError::Internal("push without lit".into()))?;
                self.push(lit);
                Ok(Flow::Continue)
            }
            Opcode::Pop => {
                let _ = self.pop(pc, "pop")?;
                Ok(Flow::Continue)
            }
            Opcode::Dup => {
                let v = self.tos(pc, "dup")?.clone();
                self.push(v);
                Ok(Flow::Continue)
            }
            Opcode::Over => {
                let n = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                let st = &self.frames[fi].stack;
                if st.len() <= n {
                    return Err(VmError::StackUnderflow {
                        pc,
                        op: "over".into(),
                    });
                }
                let v = st[st.len() - 1 - n].clone();
                self.push(v);
                Ok(Flow::Continue)
            }
            Opcode::Reverse => {
                let n = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                let st = &mut self.frames[fi].stack;
                if st.len() < n {
                    return Err(VmError::StackUnderflow {
                        pc,
                        op: "reverse".into(),
                    });
                }
                let len = st.len();
                st[len - n..].reverse();
                Ok(Flow::Continue)
            }
            Opcode::Jump1 | Opcode::Jump4 => {
                let off = *inst.operands.first().unwrap_or(&0);
                Ok(Flow::JumpPc((inst.pc as i32 + off) as u32))
            }
            Opcode::JumpTrue1 | Opcode::JumpTrue4 => {
                let off = *inst.operands.first().unwrap_or(&0);
                let v = self.pop(pc, "jumpTrue")?;
                if v.is_true()? {
                    Ok(Flow::JumpPc((inst.pc as i32 + off) as u32))
                } else {
                    Ok(Flow::Continue)
                }
            }
            Opcode::JumpFalse1 | Opcode::JumpFalse4 => {
                let off = *inst.operands.first().unwrap_or(&0);
                let v = self.pop(pc, "jumpFalse")?;
                if !v.is_true()? {
                    Ok(Flow::JumpPc((inst.pc as i32 + off) as u32))
                } else {
                    Ok(Flow::Continue)
                }
            }
            Opcode::LoadScalar1 | Opcode::LoadScalar4 => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let v = self.load_lvt(idx)?;
                self.push(v);
                Ok(Flow::Continue)
            }
            Opcode::StoreScalar1 | Opcode::StoreScalar4 => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let v = self.tos(pc, "storeScalar")?.clone();
                self.store_lvt(idx, v)?;
                Ok(Flow::Continue)
            }
            Opcode::LoadArray1 | Opcode::LoadArray4 => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let key = self.pop(pc, "loadArray")?.ensure_bytes()?;
                let v = self.load_lvt_array(idx, &key)?;
                self.push(v);
                Ok(Flow::Continue)
            }
            Opcode::StoreArray1 | Opcode::StoreArray4 => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                // stack: key, value  (value on TOS)
                let value = self.pop(pc, "storeArray")?;
                let key = self.pop(pc, "storeArray")?.ensure_bytes()?;
                self.store_lvt_array(idx, &key, value.clone())?;
                self.push(value);
                Ok(Flow::Continue)
            }
            Opcode::LoadScalarStk => {
                let name = self.pop(pc, "loadScalarStk")?.ensure_bytes()?;
                let (n1, n2) = split_var_name(&name);
                let v = self.var_get(&n1, n2.as_ref())?;
                self.push(v);
                Ok(Flow::Continue)
            }
            Opcode::StoreScalarStk => {
                // stack: name, value — effect -1 leaves value
                let value = self.pop(pc, "storeScalarStk")?;
                let name = self.pop(pc, "storeScalarStk")?.ensure_bytes()?;
                let (n1, n2) = split_var_name(&name);
                self.var_set(&n1, n2.as_ref(), value.clone())?;
                self.push(value);
                Ok(Flow::Continue)
            }
            Opcode::LoadArrayStk => {
                let key = self.pop(pc, "loadArrayStk")?.ensure_bytes()?;
                let name = self.pop(pc, "loadArrayStk")?.ensure_bytes()?;
                let value = self.var_get(&name, Some(&key))?;
                self.push(value);
                Ok(Flow::Continue)
            }
            Opcode::LoadStk => {
                let name = self.pop(pc, "loadStk")?.ensure_bytes()?;
                let (name1, name2) = split_var_name(&name);
                let value = self.var_get(&name1, name2.as_ref())?;
                self.push(value);
                Ok(Flow::Continue)
            }
            Opcode::StoreArrayStk => {
                let value = self.pop(pc, "storeArrayStk")?;
                let key = self.pop(pc, "storeArrayStk")?.ensure_bytes()?;
                let name = self.pop(pc, "storeArrayStk")?.ensure_bytes()?;
                self.var_set(&name, Some(&key), value.clone())?;
                self.push(value);
                Ok(Flow::Continue)
            }
            Opcode::StoreStk => {
                let value = self.pop(pc, "storeStk")?;
                let name = self.pop(pc, "storeStk")?.ensure_bytes()?;
                let (name1, name2) = split_var_name(&name);
                self.var_set(&name1, name2.as_ref(), value.clone())?;
                self.push(value);
                Ok(Flow::Continue)
            }
            Opcode::IncrScalarStkImm => {
                let imm = num_bigint::BigInt::from(*inst.operands.first().unwrap_or(&1));
                let name = self.pop(pc, "incrScalarStkImm")?.ensure_bytes()?;
                let (n1, n2) = split_var_name(&name);
                let cur = match self.var_get(&n1, n2.as_ref()) {
                    Ok(value) => value.as_bigint()?,
                    Err(VmError::TclError { message, .. })
                        if message.contains("no such variable")
                            || message.contains("no such element") =>
                    {
                        num_bigint::BigInt::from(0)
                    }
                    Err(error) => return Err(error),
                };
                let next = VmObj::from_bigint(cur + imm);
                self.var_set(&n1, n2.as_ref(), next.clone())?;
                self.push(next);
                Ok(Flow::Continue)
            }
            Opcode::IncrScalar1 => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let delta = self.pop(pc, "incrScalar1")?.as_bigint()?;
                let cur = match self.load_lvt(idx) {
                    Ok(value) => value.as_bigint()?,
                    Err(VmError::TclError { message, .. })
                        if message.contains("no such variable") =>
                    {
                        num_bigint::BigInt::from(0)
                    }
                    Err(error) => return Err(error),
                };
                let next = VmObj::from_bigint(cur + delta);
                self.store_lvt(idx, next.clone())?;
                self.push(next);
                Ok(Flow::Continue)
            }
            Opcode::IncrScalar1Imm => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let imm = num_bigint::BigInt::from(*inst.operands.get(1).unwrap_or(&1));
                let cur = match self.load_lvt(idx) {
                    Ok(value) => value.as_bigint()?,
                    Err(VmError::TclError { message, .. })
                        if message.contains("no such variable") =>
                    {
                        num_bigint::BigInt::from(0)
                    }
                    Err(error) => return Err(error),
                };
                let next = VmObj::from_bigint(cur + imm);
                self.store_lvt(idx, next.clone())?;
                self.push(next);
                Ok(Flow::Continue)
            }
            Opcode::IncrScalarStk
            | Opcode::IncrArray1
            | Opcode::IncrArrayStk
            | Opcode::IncrStk
            | Opcode::IncrArray1Imm
            | Opcode::IncrArrayStkImm
            | Opcode::IncrStkImm => self.incr_extended(inst),
            Opcode::ExistScalar => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let exists = self.lvt_exists(idx);
                self.push(VmObj::Int(if exists { 1 } else { 0 }));
                Ok(Flow::Continue)
            }
            Opcode::ExistArray
            | Opcode::ExistArrayStk
            | Opcode::ExistStk
            | Opcode::ArrayExistsStk
            | Opcode::ArrayExistsImm
            | Opcode::ArrayMakeStk
            | Opcode::ArrayMakeImm
            | Opcode::UnsetScalar
            | Opcode::UnsetArray
            | Opcode::UnsetArrayStk
            | Opcode::UnsetStk => self.variable_state_opcode(inst),
            Opcode::TryCvtToNumeric => {
                let value = self.pop(pc, "tryCvtToNumeric")?;
                self.push(crate::number::try_numeric(value)?);
                Ok(Flow::Continue)
            }
            Opcode::TryCvtToBoolean => {
                // Keeps the value and pushes whether it is a boolean literal.
                let literal = crate::number::is_boolean_literal(self.tos(pc, "tryCvtToBoolean")?);
                self.push(VmObj::Int(i64::from(literal)));
                Ok(Flow::Continue)
            }
            Opcode::NumType => {
                let value = self.pop(pc, "numType")?;
                self.push(VmObj::Int(crate::number::type_code(&value)));
                Ok(Flow::Continue)
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mult
            | Opcode::Div
            | Opcode::Mod
            | Opcode::Expon
            | Opcode::Bitor
            | Opcode::Bitxor
            | Opcode::Bitand
            | Opcode::Lshift
            | Opcode::Rshift => self.binop_num(op, pc),
            Opcode::Bitnot => {
                let value = self.pop(pc, "bitnot")?;
                self.push(crate::number::bit_not(&value)?);
                Ok(Flow::Continue)
            }
            Opcode::Uplus => {
                let value = self.pop(pc, "uplus")?;
                self.push(crate::number::plus(&value)?);
                Ok(Flow::Continue)
            }
            Opcode::Uminus => {
                let value = self.pop(pc, "uminus")?;
                self.push(crate::number::negate(&value)?);
                Ok(Flow::Continue)
            }
            Opcode::Eq | Opcode::Neq | Opcode::Lt | Opcode::Gt | Opcode::Le | Opcode::Ge => {
                self.binop_cmp(op, pc)
            }
            Opcode::Not => {
                let value = self.pop(pc, "not")?;
                self.push(crate::number::logical_not(&value)?);
                Ok(Flow::Continue)
            }
            Opcode::Land | Opcode::Lor => {
                let right = self.pop(pc, "logical operator")?;
                let left = self.pop(pc, "logical operator")?;
                let symbol = if op == Opcode::Land { "&&" } else { "||" };
                let truth = |value: &VmObj, side: &str| {
                    crate::number::boolean(value)
                        .map_err(|_| crate::number::operand_error(value, side, symbol))
                };
                let (a, b) = (truth(&left, "left ")?, truth(&right, "right ")?);
                let result = if op == Opcode::Land { a && b } else { a || b };
                self.push(VmObj::Int(i64::from(result)));
                Ok(Flow::Continue)
            }
            Opcode::StrEq => {
                let b = self.pop(pc, "streq")?.as_string()?;
                let a = self.pop(pc, "streq")?.as_string()?;
                self.push(VmObj::Int(if a == b { 1 } else { 0 }));
                Ok(Flow::Continue)
            }
            Opcode::StrNeq => {
                let b = self.pop(pc, "strneq")?.as_string()?;
                let a = self.pop(pc, "strneq")?.as_string()?;
                self.push(VmObj::Int(if a != b { 1 } else { 0 }));
                Ok(Flow::Continue)
            }
            Opcode::StrCmp => {
                let b = self.pop(pc, "strcmp")?.as_string()?;
                let a = self.pop(pc, "strcmp")?.as_string()?;
                self.push(VmObj::Int(match a.cmp(&b) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                }));
                Ok(Flow::Continue)
            }
            Opcode::StrLt | Opcode::StrGt | Opcode::StrLe | Opcode::StrGe => {
                let b = self.pop(pc, "string comparison")?.as_string()?;
                let a = self.pop(pc, "string comparison")?.as_string()?;
                let ordering = a.cmp(&b);
                let result = match op {
                    Opcode::StrLt => ordering.is_lt(),
                    Opcode::StrGt => ordering.is_gt(),
                    Opcode::StrLe => ordering.is_le(),
                    Opcode::StrGe => ordering.is_ge(),
                    _ => unreachable!(),
                };
                self.push(VmObj::Int(i64::from(result)));
                Ok(Flow::Continue)
            }
            Opcode::StrLen => {
                let s = self.pop(pc, "strlen")?.as_string()?;
                self.push(VmObj::Int(s.chars().count() as i64));
                Ok(Flow::Continue)
            }
            Opcode::StrIndex => {
                let position = self.pop(pc, "strindex")?;
                let string = self.pop(pc, "strindex")?;
                self.push(crate::strings::index_char(&string, &position)?);
                Ok(Flow::Continue)
            }
            Opcode::StrMatch => {
                let nocase = *inst.operands.first().unwrap_or(&0) != 0;
                let string = self.pop(pc, "strmatch")?.as_string()?;
                let pattern = self.pop(pc, "strmatch")?.as_string()?;
                self.push(VmObj::Int(if glob_match(&pattern, &string, nocase) {
                    1
                } else {
                    0
                }));
                Ok(Flow::Continue)
            }
            Opcode::StrConcat1 => {
                let n = *inst.operands.first().unwrap_or(&0) as usize;
                if n == 0 {
                    self.push(VmObj::empty());
                    return Ok(Flow::Continue);
                }
                let fi = self.fi();
                let st = &mut self.frames[fi].stack;
                if st.len() < n {
                    return Err(VmError::StackUnderflow {
                        pc,
                        op: "strcat".into(),
                    });
                }
                let parts: Vec<VmObj> = st.split_off(st.len() - n);
                let mut buf = Vec::new();
                for p in parts {
                    buf.extend_from_slice(p.ensure_bytes()?.as_bytes());
                }
                self.push(VmObj::from_bytes(buf));
                Ok(Flow::Continue)
            }
            Opcode::AppendScalar1
            | Opcode::AppendScalar4
            | Opcode::AppendArray1
            | Opcode::AppendArray4
            | Opcode::AppendArrayStk
            | Opcode::AppendStk
            | Opcode::LappendScalar1
            | Opcode::LappendScalar4
            | Opcode::LappendArray1
            | Opcode::LappendArray4
            | Opcode::LappendArrayStk
            | Opcode::LappendStk
            | Opcode::LappendList
            | Opcode::LappendListArray
            | Opcode::LappendListArrayStk
            | Opcode::LappendListStk => self.append_extended(inst),
            Opcode::List => {
                let n = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                let st = &mut self.frames[fi].stack;
                if st.len() < n {
                    return Err(VmError::StackUnderflow {
                        pc,
                        op: "list".into(),
                    });
                }
                let parts: Vec<VmObj> = st.split_off(st.len() - n);
                self.push(VmObj::List(parts));
                Ok(Flow::Continue)
            }
            Opcode::ListIndex => {
                let index = self.pop(pc, "listIndex")?;
                let list = self.pop(pc, "listIndex")?;
                self.push(crate::lists::index_list(&list, &index)?);
                Ok(Flow::Continue)
            }
            Opcode::ListLength => {
                let list_obj = self.pop(pc, "listLength")?;
                let list = list_obj.as_list_vec()?;
                self.push(VmObj::Int(list.len() as i64));
                Ok(Flow::Continue)
            }
            Opcode::ListIndexMulti
            | Opcode::ListIndexImm
            | Opcode::ListRangeImm
            | Opcode::ListIn
            | Opcode::ListNotIn
            | Opcode::ListConcat
            | Opcode::LsetList
            | Opcode::LsetFlat
            | Opcode::ConcatStk => self.list_extended(inst),
            Opcode::ForeachStart4 => {
                let aux_index = self.aux_index(inst, "ForeachStart4")?;
                let info = match &self.frames[self.fi()].code.bytecode.auxiliaries[aux_index].value
                {
                    AuxDataValue::Foreach(info) => info.clone(),
                    _ => {
                        return Err(VmError::InvalidAuxData {
                            opcode: "ForeachStart4".into(),
                            detail: format!("auxiliary {aux_index} is not legacy foreach"),
                        });
                    }
                };
                let counter =
                    usize::try_from(info.loop_counter).map_err(|_| VmError::InvalidAuxData {
                        opcode: "ForeachStart4".into(),
                        detail: format!("negative loop counter LVT {}", info.loop_counter),
                    })?;
                self.validate_legacy_foreach_lvt(&info, "ForeachStart4")?;
                self.store_lvt(counter, VmObj::Int(-1))?;
                Ok(Flow::Continue)
            }
            Opcode::ForeachStep4 => self.foreach_step_legacy(inst),
            Opcode::ForeachStart => self.foreach_start_new(inst),
            Opcode::ForeachStep => self.foreach_step_new(inst),
            Opcode::ForeachEnd => self.foreach_end_new(inst),
            Opcode::LmapCollect => self.lmap_collect(inst),
            Opcode::JumpTable => self.jump_table(inst),
            Opcode::BeginCatch4 => {
                let range_index = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                if !self.frames[fi]
                    .code
                    .bytecode
                    .exceptions
                    .get(range_index)
                    .is_some_and(|range| range.kind == ExceptionType::Catch)
                {
                    return Err(VmError::InvalidCatchState);
                }
                let depth = self.frames[fi].stack.len();
                let aux_state_depth = self.frames[fi].aux_states.len();
                self.frames[fi].catch_stack.push(CatchMark {
                    range_index,
                    stack_depth: depth,
                    aux_state_depth,
                });
                #[cfg(feature = "debug")]
                if self.source_images.contains_key(&self.frames[fi].code.id) {
                    self.last_completion = Completion::ok(VmObj::empty());
                }
                Ok(Flow::Continue)
            }
            Opcode::EndCatch => {
                let fi = self.fi();
                self.frames[fi]
                    .catch_stack
                    .pop()
                    .ok_or(VmError::InvalidCatchState)?;
                #[cfg(feature = "debug")]
                if self.source_images.contains_key(&self.frames[fi].code.id) {
                    self.last_completion = Completion::ok(VmObj::empty());
                }
                Ok(Flow::Continue)
            }
            Opcode::PushReturnCode => {
                self.push(VmObj::Int(self.last_completion.code as i64));
                Ok(Flow::Continue)
            }
            Opcode::PushResult => {
                self.push(self.last_completion.result.clone());
                Ok(Flow::Continue)
            }
            Opcode::PushReturnOptions => {
                self.push(self.last_completion.options.as_vm_obj());
                Ok(Flow::Continue)
            }
            Opcode::Syntax | Opcode::ReturnImm => {
                let code = *inst.operands.first().unwrap_or(&0);
                let level = *inst.operands.get(1).unwrap_or(&1);
                if level < 0 {
                    return Err(VmError::InvalidOpcodeOperand {
                        opcode: "ReturnImm".into(),
                        detail: format!("negative level {level}"),
                    });
                }
                let raw_options = self.pop(pc, "returnImm options")?;
                let result = self.pop(pc, "returnImm result")?;
                let mut options = ReturnOptions::from_vm_obj(raw_options)?;
                options.code = code;
                options.level = level as u32;
                // The code and level operands override the option dictionary.
                // Preserve its other fields when reporting complete options.
                #[cfg(feature = "debug")]
                if self
                    .source_images
                    .contains_key(&self.frames[self.fi()].code.id)
                {
                    options.sync_raw_code_level()?;
                }
                let routed = options.routed_code();
                if routed == 0 {
                    self.last_completion = Completion {
                        code: routed,
                        result: result.clone(),
                        options,
                    };
                    self.push(result);
                    Ok(Flow::Continue)
                } else {
                    Ok(Flow::Raise(Completion {
                        code: routed,
                        result,
                        options,
                    }))
                }
            }
            Opcode::ReturnStk => {
                let result = self.pop(pc, "returnStk result")?;
                let raw_options = self.pop(pc, "returnStk options")?;
                let options = ReturnOptions::from_return_options(raw_options)?;
                let routed = options.routed_code();
                if routed == 0 {
                    self.last_completion = Completion {
                        code: routed,
                        result: result.clone(),
                        options,
                    };
                    self.push(result);
                    Ok(Flow::Continue)
                } else {
                    Ok(Flow::Raise(Completion {
                        code: routed,
                        result,
                        options,
                    }))
                }
            }
            Opcode::ReturnCodeBranch => {
                let code = self.pop(pc, "returnCodeBranch")?.as_i64()? as i32;
                if code == 0 {
                    return Err(VmError::InvalidOpcodeOperand {
                        opcode: "ReturnCodeBranch".into(),
                        detail: "TCL_OK has no branch entry".into(),
                    });
                }
                let branch = if (1..=4).contains(&code) { code } else { 5 };
                let target = inst.pc as i64 + (2 * branch - 1) as i64;
                if target < 0 || target > u32::MAX as i64 {
                    return Err(VmError::InvalidJumpTarget {
                        code_id: self.frames[self.fi()].code.id.0,
                        pc: target.max(0) as u32,
                    });
                }
                Ok(Flow::JumpPc(target as u32))
            }
            Opcode::Break => Ok(Flow::Raise(Completion::with_code(3, VmObj::empty()))),
            Opcode::Continue => Ok(Flow::Raise(Completion::with_code(4, VmObj::empty()))),
            Opcode::Variable => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let name = self.pop(pc, "variable")?.ensure_bytes()?;
                let (n1, n2) = parse_array_name(name.as_bytes());
                let host_n1 = if n1.as_bytes().starts_with(b"::") {
                    n1
                } else {
                    // qualify with current ns
                    let ns = self.frames[self.fi()].namespace.as_bytes().as_str_lossy();
                    let ns = ns.trim_start_matches(':');
                    if ns.is_empty() {
                        ByteString::from(format!("::{}", n1.as_str_lossy()).as_str())
                    } else {
                        ByteString::from(format!("::{ns}::{}", n1.as_str_lossy()).as_str())
                    }
                };
                self.install_lvt_host_link(idx, host_n1, n2)?;
                Ok(Flow::Continue)
            }
            Opcode::NsCurrent => {
                let namespace = self.frames[self.fi()].namespace.as_bytes().clone();
                self.push(VmObj::Bytes(namespace));
                Ok(Flow::Continue)
            }
            Opcode::Nsupvar => self.nsupvar_opcode(inst),
            Opcode::ExprStk | Opcode::EvalStk => self.host_frame_opcode(inst),
            Opcode::Upvar => {
                let idx = *inst.operands.first().unwrap_or(&0) as usize;
                let other = self.pop(pc, "upvar")?.ensure_bytes()?;
                let level = self.tos(pc, "upvar level")?.ensure_bytes()?;
                let target_index = self.resolve_vm_level(&level, true)?;
                let target_frame = self.frames[target_index].id;
                let (tname, part2) = parse_array_name(other.as_bytes());
                self.install_lvt_vm_link(idx, target_frame, tname, part2)?;
                Ok(Flow::Continue)
            }
            Opcode::ExpandStart | Opcode::ExpandStktop | Opcode::ExpandDrop => {
                self.expand_state_opcode(inst)
            }
            Opcode::InvokeExpanded | Opcode::InvokeReplace => self.invoke_expansion_opcode(inst),
            Opcode::InvokeStk1 | Opcode::InvokeStk4 => {
                let argc = *inst.operands.first().unwrap_or(&0) as usize;
                if argc < 1 {
                    return Err(VmError::Internal("invokeStk argc < 1".into()));
                }
                let fi = self.fi();
                let st = &mut self.frames[fi].stack;
                if st.len() < argc {
                    return Err(VmError::StackUnderflow {
                        pc,
                        op: "invokeStk".into(),
                    });
                }
                let words: Vec<VmObj> = st.split_off(st.len() - argc);
                let call_site = CodeAddress {
                    code_id: self.frames[fi].code.id,
                    pc: inst.pc,
                };
                match self.dispatch_invoke(&words, call_site)? {
                    InvokeOutcome::Value(result) => {
                        self.push(result);
                        Ok(Flow::Continue)
                    }
                    InvokeOutcome::Raise(r) => Ok(Flow::Raise(r)),
                    InvokeOutcome::FramePushed => Ok(Flow::Continue),
                }
            }
            Opcode::CallBuiltinFunc1 | Opcode::CallFunc1 => self.math_function_opcode(inst),
            Opcode::StrMap
            | Opcode::StrFind
            | Opcode::StrFindLast
            | Opcode::StrRangeImm
            | Opcode::StrRange
            | Opcode::StrTrim
            | Opcode::StrTrimLeft
            | Opcode::StrTrimRight
            | Opcode::StrUpper
            | Opcode::StrLower
            | Opcode::StrTitle
            | Opcode::StrReplace
            | Opcode::StrClass => self.string_opcode(inst),
            Opcode::Regexp => self.regexp_opcode(inst),
            Opcode::ClockRead => self.clock_read_opcode(inst),
            Opcode::Lreplace4 => self.lreplace_opcode(inst),
            Opcode::DictGet
            | Opcode::DictSet
            | Opcode::DictUnset
            | Opcode::DictIncrImm
            | Opcode::DictAppend
            | Opcode::DictLappend
            | Opcode::DictFirst
            | Opcode::DictNext
            | Opcode::DictDone
            | Opcode::DictUpdateStart
            | Opcode::DictUpdateEnd
            | Opcode::DictExpand
            | Opcode::DictRecombineStk
            | Opcode::DictRecombineImm
            | Opcode::DictExists
            | Opcode::DictGetDef
            | Opcode::DictVerify => self.dict_opcode(inst),
            other if classify(other) == OpcodeClass::Trap => Err(VmError::UnimplementedOpcode {
                opcode: format!("{other:?}"),
                pc,
            }),
            other => Err(VmError::Internal(format!(
                "closed opcode disposition missing runtime handler for {other:?} at pc {pc}"
            ))),
        }
    }

    fn expand_state_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let fi = self.fi();
        match inst.opcode {
            Opcode::ExpandStart => {
                let depth = self.frames[fi].stack.len();
                self.frames[fi]
                    .aux_states
                    .push(AuxRuntimeState::Expand(ExpandMark { stack_depth: depth }));
            }
            Opcode::ExpandStktop => {
                if !self.frames[fi]
                    .aux_states
                    .iter()
                    .any(|state| matches!(state, AuxRuntimeState::Expand(_)))
                {
                    return Err(VmError::InvalidExpansionState);
                }
                let value = self.pop(inst.pc as u64, "expandStktop")?;
                let elements = value.as_list_vec()?;
                self.frames[fi].stack.extend(elements);
            }
            Opcode::ExpandDrop => {
                let position = self.frames[fi]
                    .aux_states
                    .iter()
                    .rposition(|state| matches!(state, AuxRuntimeState::Expand(_)))
                    .ok_or(VmError::InvalidExpansionState)?;
                if position + 1 != self.frames[fi].aux_states.len() {
                    return Err(VmError::InvalidExpansionState);
                }
                let AuxRuntimeState::Expand(mark) = self.frames[fi].aux_states.pop().unwrap()
                else {
                    unreachable!()
                };
                self.frames[fi].stack.truncate(mark.stack_depth);
            }
            _ => unreachable!(),
        }
        Ok(Flow::Continue)
    }

    fn invoke_expansion_opcode(
        &mut self,
        inst: &crate::load::DecodedInst,
    ) -> Result<Flow, VmError> {
        let fi = self.fi();
        let call_site = CodeAddress {
            code_id: self.frames[fi].code.id,
            pc: inst.pc,
        };
        let words = if inst.opcode == Opcode::InvokeExpanded {
            let position = self.frames[fi]
                .aux_states
                .iter()
                .rposition(|state| matches!(state, AuxRuntimeState::Expand(_)))
                .ok_or(VmError::InvalidExpansionState)?;
            if position + 1 != self.frames[fi].aux_states.len() {
                return Err(VmError::InvalidExpansionState);
            }
            let AuxRuntimeState::Expand(mark) = self.frames[fi].aux_states.pop().unwrap() else {
                unreachable!()
            };
            self.frames[fi].stack.split_off(mark.stack_depth)
        } else {
            let argc = *inst.operands.first().unwrap_or(&0) as usize;
            let prefix = *inst.operands.get(1).unwrap_or(&0) as usize;
            let implementation = self.pop(inst.pc as u64, "invokeReplace implementation")?;
            if prefix > argc || self.frames[fi].stack.len() < argc {
                return Err(VmError::InvalidOpcodeOperand {
                    opcode: "InvokeReplace".into(),
                    detail: format!("argc={argc} prefix={prefix}"),
                });
            }
            let split = self.frames[fi].stack.len() - argc;
            let original = self.frames[fi].stack.split_off(split);
            let mut rewritten = vec![implementation];
            rewritten.extend_from_slice(&original[prefix..]);
            rewritten
        };
        if words.is_empty() {
            self.push(VmObj::empty());
            return Ok(Flow::Continue);
        }
        match self.dispatch_invoke(&words, call_site)? {
            InvokeOutcome::Value(value) => {
                self.push(value);
                Ok(Flow::Continue)
            }
            InvokeOutcome::Raise(completion) => Ok(Flow::Raise(completion)),
            InvokeOutcome::FramePushed => Ok(Flow::Continue),
        }
    }

    fn host_value_result(&mut self, words: &[VmObj]) -> Result<Result<VmObj, Completion>, VmError> {
        let name = words
            .first()
            .ok_or_else(|| VmError::Internal("empty host value invocation".into()))?
            .ensure_bytes()?;
        match self.invoke_world_host(words, &name)? {
            InvokeOutcome::Value(value) => Ok(Ok(value)),
            InvokeOutcome::Raise(completion) => Ok(Err(completion)),
            InvokeOutcome::FramePushed => Err(VmError::Internal(
                "typed host value operation pushed a VM frame".into(),
            )),
        }
    }

    fn host_frame_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let body = self.pop(pc, "host frame body")?;
        #[cfg(feature = "debug")]
        if self
            .source_images
            .contains_key(&self.frames[self.fi()].code.id)
        {
            let site = CodeAddress {
                code_id: self.frames[self.fi()].code.id,
                pc: inst.pc,
            };
            let words = [
                VmObj::from_str(if inst.opcode == Opcode::ExprStk {
                    "expr"
                } else {
                    "eval"
                }),
                body,
            ];
            return match self
                .invoke_generated(&words, site)?
                .expect("generated eval dispatch")
            {
                InvokeOutcome::Value(value) => {
                    self.push(value);
                    Ok(Flow::Continue)
                }
                InvokeOutcome::Raise(completion) => Ok(Flow::Raise(completion)),
                InvokeOutcome::FramePushed => Ok(Flow::Continue),
            };
        }
        self.host_frame_eval(body, inst.opcode == Opcode::ExprStk)
    }

    #[cfg(not(feature = "host"))]
    fn host_frame_eval(&mut self, _body: VmObj, _expression: bool) -> Result<Flow, VmError> {
        Err(VmError::HybridUnavailable(
            "host frame evaluation requires the host feature".into(),
        ))
    }

    #[cfg(feature = "host")]
    fn host_frame_eval(&mut self, body: VmObj, expression: bool) -> Result<Flow, VmError> {
        use crate::var::VarSlot;
        let fi = self.scope_index();
        let namespace = self.frames[fi].namespace.as_bytes().clone();
        let mut seen = std::collections::HashSet::new();
        let mut rows = Vec::new();

        let compiled_names: Vec<ByteString> = self.frames[fi]
            .compiled_locals
            .iter()
            .map(|slot| slot.name.clone())
            .collect();
        for (index, name) in compiled_names.into_iter().enumerate() {
            seen.insert(name.clone());
            if let Some(link) = self.frames[fi].compiled_locals[index].link.clone() {
                if let Some(target) = host_link_target(&link) {
                    rows.push(HostFrameInput {
                        name,
                        storage: HostFrameStorage::HostLink(target),
                    });
                    continue;
                }
                let (storage, value) = self.snapshot_link_value(&link).map_err(|error| {
                    VmError::HostFrameMarshalUnsupported(format!(
                        "link {}: {error}",
                        name.as_str_lossy()
                    ))
                })?;
                rows.push(HostFrameInput {
                    name,
                    storage: typed_host_frame_storage(storage, value)?,
                });
                continue;
            }
            if let Some(VarSlot::Array(map)) = self.frames[fi].local_vars.get_slot(&name) {
                let mut pairs = Vec::with_capacity(map.len());
                for (key, value) in map {
                    pairs.push((key.clone(), value.ensure_bytes()?));
                }
                rows.push(HostFrameInput {
                    name,
                    storage: HostFrameStorage::Array(pairs),
                });
            } else if let Ok(value) = self.var_get(&name, None) {
                rows.push(HostFrameInput {
                    name,
                    storage: HostFrameStorage::Scalar(value.ensure_bytes()?),
                });
            }
        }
        let dynamic: Vec<(ByteString, VarSlot)> = self.frames[fi]
            .local_vars
            .iter_slots()
            .map(|(name, slot)| (name.clone(), slot.clone()))
            .collect();
        for (name, slot) in dynamic {
            if seen.contains(&name) {
                continue;
            }
            match slot {
                VarSlot::Scalar(value) => rows.push(HostFrameInput {
                    name,
                    storage: HostFrameStorage::Scalar(value.ensure_bytes()?),
                }),
                VarSlot::Array(map) => {
                    let mut pairs = Vec::with_capacity(map.len());
                    for (key, value) in map {
                        pairs.push((key, value.ensure_bytes()?));
                    }
                    rows.push(HostFrameInput {
                        name,
                        storage: HostFrameStorage::Array(pairs),
                    });
                }
                VarSlot::Link(link) => {
                    if let Some(target) = host_link_target(&link) {
                        rows.push(HostFrameInput {
                            name,
                            storage: HostFrameStorage::HostLink(target),
                        });
                    } else {
                        let (storage, value) =
                            self.snapshot_link_value(&link).map_err(|error| {
                                VmError::HostFrameMarshalUnsupported(format!(
                                    "link {}: {error}",
                                    name.as_str_lossy()
                                ))
                            })?;
                        rows.push(HostFrameInput {
                            name,
                            storage: typed_host_frame_storage(storage, value)?,
                        });
                    }
                }
                VarSlot::Undefined => {}
            }
        }

        let mirrored_names: Vec<ByteString> = rows.iter().map(|row| row.name.clone()).collect();
        let body = body.ensure_bytes()?;
        let frame_id = self.frames[fi].id.0;
        let output = self.invoke_host_frame_bridge(namespace, frame_id, expression, body, rows)?;
        let BridgeResult {
            code,
            result,
            error_info,
            error_code,
            return_options,
        } = output.completion;
        let options = return_options
            .map(ReturnOptions::from_vm_obj)
            .transpose()?
            .unwrap_or_else(|| ReturnOptions {
                code,
                error_info: error_info.map(ByteString::from),
                error_code: error_code.map(|value| VmObj::from_str(&value)),
                ..ReturnOptions::default()
            });
        let mut returned_names = std::collections::HashSet::new();
        for HostFrameLocalResult { name, storage } in output.locals {
            returned_names.insert(name.clone());
            if let HostFrameStorage::Array(pairs) = storage {
                let mut map = std::collections::HashMap::new();
                for (key, value) in pairs {
                    map.insert(key, VmObj::Bytes(value));
                }
                self.replace_named_array(&name, map)?;
            } else if let HostFrameStorage::Scalar(value) = storage {
                self.var_set(&name, None, VmObj::Bytes(value))?;
            }
        }
        for name in mirrored_names {
            if !returned_names.contains(&name) {
                let _ = self.var_unset(&name, None);
            }
        }
        if code == 1 {
            let message = result.as_string().unwrap_or_default();
            if message == VmError::CrossRuntimeUpvar.to_string() {
                return Err(VmError::CrossRuntimeUpvar);
            }
            if message == VmError::CrossRuntimeUplevel.to_string() {
                return Err(VmError::CrossRuntimeUplevel);
            }
            if message.starts_with("host frame command collision:") {
                return Err(VmError::HostFrameUnavailable(message));
            }
            if message.contains("yield can only be called in a coroutine")
                || message.contains("yieldto can only be called in a coroutine")
            {
                return Err(VmError::HostFrameSuspensionUnsupported);
            }
            if options.error_code.as_ref().is_some_and(|error_code| {
                error_code.as_list_vec().is_ok_and(|parts| {
                    parts.len() >= 3
                        && parts[0] == VmObj::from_str("TCLD")
                        && parts[1] == VmObj::from_str("HOSTFRAME")
                        && parts[2] == VmObj::from_str("SUSPENSION")
                })
            }) {
                return Err(VmError::HostFrameSuspensionUnsupported);
            }
        }
        if code == 0 {
            self.push(result);
            Ok(Flow::Continue)
        } else {
            Ok(Flow::Raise(Completion {
                code,
                result,
                options,
            }))
        }
    }

    fn replace_named_array(
        &mut self,
        name: &ByteString,
        map: std::collections::HashMap<ByteString, VmObj>,
    ) -> Result<(), VmError> {
        let fi = self.scope_index();
        #[cfg(feature = "debug")]
        if matches!(
            self.frames[fi].kind,
            FrameKind::Toplevel | FrameKind::Namespace
        ) && self.source_images.contains_key(&self.frames[fi].code.id)
            && !self.frames[fi]
                .compiled_locals
                .iter()
                .any(|s| s.is_named(name) && s.link.is_some())
            && !matches!(
                self.frames[fi].local_vars.get_slot(name),
                Some(VarSlot::Link(_))
            )
        {
            let qualified = self.frames[fi].namespace.qualify(name);
            self.install_host_link_named(name, qualified, None)?;
        }

        let link = self.frames[fi]
            .compiled_locals
            .iter()
            .find(|slot| slot.is_named(name))
            .and_then(|slot| slot.link.clone())
            .or_else(|| match self.frames[fi].local_vars.get_slot(name) {
                Some(VarSlot::Link(link)) => Some(link.clone()),
                _ => None,
            });
        if let Some(link) = link {
            return self.replace_link_array(&link, map);
        }
        self.frames[fi]
            .local_vars
            .set_slot(name.clone(), VarSlot::Array(map));
        if let Some(slot) = self.frames[fi]
            .compiled_locals
            .iter_mut()
            .find(|slot| slot.is_named(name))
        {
            slot.value = None;
        }
        Ok(())
    }

    fn replace_link_array(
        &mut self,
        link: &VarLink,
        map: std::collections::HashMap<ByteString, VmObj>,
    ) -> Result<(), VmError> {
        match link {
            VarLink::Vm {
                target_frame,
                target_name,
                part2: None,
            } => {
                let frame_index = self
                    .frames
                    .iter()
                    .position(|frame| frame.id == *target_frame)
                    .ok_or(VmError::CrossRuntimeUpvar)?;
                let nested = self.frames[frame_index]
                    .compiled_locals
                    .iter()
                    .find(|slot| slot.is_named(target_name))
                    .and_then(|slot| slot.link.clone())
                    .or_else(
                        || match self.frames[frame_index].local_vars.get_slot(target_name) {
                            Some(VarSlot::Link(link)) => Some(link.clone()),
                            _ => None,
                        },
                    );
                if let Some(nested) = nested {
                    return self.replace_link_array(&nested, map);
                }
                self.frames[frame_index]
                    .local_vars
                    .set_slot(target_name.clone(), VarSlot::Array(map));
                if let Some(slot) = self.frames[frame_index]
                    .compiled_locals
                    .iter_mut()
                    .find(|slot| slot.is_named(target_name))
                {
                    slot.value = None;
                }
                Ok(())
            }
            VarLink::Vm {
                target_name,
                part2: Some(part2),
                ..
            }
            | VarLink::HostVar {
                name1: target_name,
                name2: Some(part2),
            } => Err(VmError::TclError {
                message: format!(
                    "can't set \"{}({})\": variable isn't array",
                    target_name.as_str_lossy(),
                    part2.as_str_lossy()
                ),
                error_info: None,
                error_code: Some("TCL LOOKUP VARNAME".into()),
            }),
            VarLink::HostVar { name1, name2: None } if self.mode == RuntimeMode::VmStrict => {
                self.world.replace_array(name1, map);
                Ok(())
            }
            VarLink::HostVar { name1, name2: None } => {
                match self.host_value_result(&[
                    VmObj::from_str("unset"),
                    VmObj::from_str("-nocomplain"),
                    VmObj::Bytes(name1.clone()),
                ])? {
                    Ok(_) => {}
                    Err(completion) => return Err(completion_to_error(completion)),
                }
                let mut pairs = Vec::with_capacity(map.len() * 2);
                for (key, value) in map {
                    pairs.push(VmObj::Bytes(key));
                    pairs.push(value);
                }
                match self.host_value_result(&[
                    VmObj::from_str("array"),
                    VmObj::from_str("set"),
                    VmObj::Bytes(name1.clone()),
                    VmObj::List(pairs),
                ])? {
                    Ok(_) => Ok(()),
                    Err(completion) => Err(completion_to_error(completion)),
                }
            }
        }
    }

    fn nsupvar_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let index = *inst.operands.first().unwrap_or(&0) as usize;
        let name = self.pop(pc, "nsupvar name")?.ensure_bytes()?;
        let namespace = self.tos(pc, "nsupvar namespace")?.ensure_bytes()?;
        let namespace = if namespace.as_bytes().starts_with(b"::") {
            namespace
        } else {
            self.frames[self.fi()].namespace.qualify(&namespace)
        };
        let exists = self.host_value_result(&[
            VmObj::from_str("namespace"),
            VmObj::from_str("exists"),
            VmObj::Bytes(namespace.clone()),
        ])?;
        match exists {
            Err(completion) => return Ok(Flow::Raise(completion)),
            Ok(value) if !value.is_true()? => {
                return Err(VmError::TclError {
                    message: format!("unknown namespace \"{}\"", namespace.as_str_lossy()),
                    error_info: None,
                    error_code: Some("TCL LOOKUP NAMESPACE".into()),
                });
            }
            Ok(_) => {}
        }
        let qualified = NamespaceName(namespace).qualify(&name);
        let (name1, name2) = parse_array_name(qualified.as_bytes());
        self.install_lvt_host_link(index, name1, name2)?;
        Ok(Flow::Continue)
    }

    fn operand_count(inst: &crate::load::DecodedInst, index: usize) -> Result<usize, VmError> {
        usize::try_from(*inst.operands.get(index).unwrap_or(&-1)).map_err(|_| {
            VmError::InvalidOpcodeOperand {
                opcode: format!("{:?}", inst.opcode),
                detail: format!("operand {index} is not a count"),
            }
        })
    }

    /// A variable's value, or `None` when it is unset.
    fn load_lvt_optional(&mut self, index: usize) -> Result<Option<VmObj>, VmError> {
        match self.load_lvt(index) {
            Ok(value) => Ok(Some(value)),
            Err(error) if is_missing_tcl_value(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The dictionary instructions (`tclExecute.c`) on native values.
    fn dict_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let value = match inst.opcode {
            Opcode::DictGet | Opcode::DictExists | Opcode::DictGetDef => {
                let keys = Self::operand_count(inst, 0)?;
                if keys == 0 {
                    return Err(VmError::InvalidOpcodeOperand {
                        opcode: format!("{:?}", inst.opcode),
                        detail: "zero dictionary path length".into(),
                    });
                }
                let has_default = inst.opcode == Opcode::DictGetDef;
                let mut args =
                    self.take_stack_words(keys + 1 + usize::from(has_default), pc, "dict path")?;
                let default = if has_default { args.pop() } else { None };
                let dict = args.remove(0);
                let (key, path) = args.split_last().unwrap();
                match inst.opcode {
                    Opcode::DictExists => {
                        // Any failure, a malformed value included, means absent.
                        let found = Dict::trace_existing(&dict, path)
                            .ok()
                            .flatten()
                            .is_some_and(|leaf| leaf.get(key).is_ok_and(|value| value.is_some()));
                        VmObj::Int(i64::from(found))
                    }
                    Opcode::DictGet => match Dict::trace(&dict, path)?.get(key)? {
                        Some(value) => value.clone(),
                        None => return Err(crate::dict::unknown_key(key)?),
                    },
                    _ => {
                        let found = match Dict::trace_existing(&dict, path)? {
                            Some(leaf) => leaf.get(key)?.cloned(),
                            None => None,
                        };
                        found.or(default).unwrap_or_else(VmObj::empty)
                    }
                }
            }
            Opcode::DictVerify => {
                Dict::from_value(&self.pop(pc, "dictVerify")?)?;
                return Ok(Flow::Continue);
            }
            Opcode::DictSet
            | Opcode::DictUnset
            | Opcode::DictIncrImm
            | Opcode::DictAppend
            | Opcode::DictLappend => {
                let (index, operand_words) = match inst.opcode {
                    Opcode::DictSet => (
                        Self::operand_count(inst, 1)?,
                        Self::operand_count(inst, 0)? + 1,
                    ),
                    Opcode::DictUnset => {
                        (Self::operand_count(inst, 1)?, Self::operand_count(inst, 0)?)
                    }
                    Opcode::DictIncrImm => (Self::operand_count(inst, 1)?, 1),
                    _ => (Self::operand_count(inst, 0)?, 2),
                };
                let mut operands = self.take_stack_words(operand_words, pc, "dict update")?;
                let current = self.load_lvt_optional(index)?;
                let mut dict = Dict::from_optional(current.as_ref())?;
                match inst.opcode {
                    Opcode::DictSet => {
                        let value = operands.pop().unwrap();
                        dict.put_path(&operands, value)?;
                    }
                    Opcode::DictUnset => dict.remove_path(&operands)?,
                    Opcode::DictIncrImm => dict.incr(
                        &operands[0],
                        i64::from(*inst.operands.first().unwrap_or(&0)),
                    )?,
                    Opcode::DictAppend => dict.append(&operands[0], &operands[1])?,
                    _ => dict.lappend(&operands[0], &operands[1])?,
                }
                let value = dict.into_value();
                self.store_lvt(index, value.clone())?;
                value
            }
            Opcode::DictFirst => {
                let index = Self::operand_count(inst, 0)?;
                let dict = Dict::from_value(&self.pop(pc, "dictFirst")?)?;
                let fi = self.fi();
                self.frames[fi].aux_states.retain(
                    |state| !matches!(state, AuxRuntimeState::DictIter(iter) if iter.lvt == index),
                );
                self.frames[fi]
                    .aux_states
                    .push(AuxRuntimeState::DictIter(DictIterState {
                        lvt: index,
                        entries: dict.into_entries(),
                        next: 0,
                    }));
                return self.dict_iter_push(index);
            }
            Opcode::DictNext => return self.dict_iter_push(Self::operand_count(inst, 0)?),
            Opcode::DictDone => {
                let index = Self::operand_count(inst, 0)?;
                let fi = self.fi();
                let before = self.frames[fi].aux_states.len();
                self.frames[fi].aux_states.retain(
                    |state| !matches!(state, AuxRuntimeState::DictIter(iter) if iter.lvt == index),
                );
                if self.frames[fi].aux_states.len() == before {
                    return Err(VmError::InvalidAuxData {
                        opcode: "DictDone".into(),
                        detail: format!("iterator LVT {index} is not active"),
                    });
                }
                return Ok(Flow::Continue);
            }
            Opcode::DictUpdateStart | Opcode::DictUpdateEnd => {
                return self.dict_update_opcode(inst);
            }
            Opcode::DictExpand => {
                // `TclDictWithInit`: one variable per key of the dictionary
                // at the path; the keys come back for the write-back.
                let path = self.pop(pc, "dictExpand path")?.as_list_vec()?;
                let dict = Dict::trace(&self.pop(pc, "dictExpand dict")?, &path)?;
                let keys = dict.keys();
                for (key, value) in dict.into_entries() {
                    let name = key.ensure_bytes()?;
                    let (name1, name2) = split_var_name(&name);
                    self.var_set(&name1, name2.as_ref(), value)?;
                }
                VmObj::List(keys)
            }
            Opcode::DictRecombineStk | Opcode::DictRecombineImm => {
                return self.dict_recombine_opcode(inst);
            }
            _ => unreachable!(),
        };
        self.push(value);
        Ok(Flow::Continue)
    }

    fn dict_iter_push(&mut self, index: usize) -> Result<Flow, VmError> {
        let fi = self.fi();
        let iter = self.frames[fi]
            .aux_states
            .iter_mut()
            .find_map(|state| match state {
                AuxRuntimeState::DictIter(iter) if iter.lvt == index => Some(iter),
                _ => None,
            })
            .ok_or_else(|| VmError::InvalidAuxData {
                opcode: "DictNext".into(),
                detail: format!("iterator LVT {index} is not active"),
            })?;
        let entry = iter.entries.get(iter.next).cloned();
        if entry.is_some() {
            iter.next += 1;
        }
        if let Some((key, value)) = entry {
            self.push(value);
            self.push(key);
            self.push(VmObj::Int(0));
        } else {
            self.push(VmObj::empty());
            self.push(VmObj::empty());
            self.push(VmObj::Int(1));
        }
        Ok(Flow::Continue)
    }

    /// `dict update`: copy the named keys into local variables, and later
    /// write the variables back (an unset variable removes its key).
    fn dict_update_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let dict_index = Self::operand_count(inst, 0)?;
        let aux_index = *inst.operands.get(1).unwrap_or(&-1);
        let aux_index = usize::try_from(aux_index).map_err(|_| VmError::InvalidAuxData {
            opcode: format!("{:?}", inst.opcode),
            detail: "negative auxiliary index".into(),
        })?;
        let indices = match self.frames[self.fi()]
            .code
            .bytecode
            .auxiliaries
            .get(aux_index)
        {
            Some(aux) => match &aux.value {
                AuxDataValue::DictUpdate(info) => info.indices.clone(),
                _ => {
                    return Err(VmError::InvalidAuxData {
                        opcode: format!("{:?}", inst.opcode),
                        detail: "auxiliary is not DictUpdate".into(),
                    });
                }
            },
            None => {
                return Err(VmError::InvalidAuxData {
                    opcode: format!("{:?}", inst.opcode),
                    detail: format!("auxiliary {aux_index} out of bounds"),
                });
            }
        };
        let keys = self.tos(pc, "dictUpdate keys")?.as_list_vec()?;
        if keys.len() != indices.len() {
            return Err(VmError::InvalidAuxData {
                opcode: format!("{:?}", inst.opcode),
                detail: "key/local length mismatch".into(),
            });
        }
        let local_count = self.frames[self.fi()].compiled_locals.len();
        if dict_index >= local_count {
            return Err(VmError::InvalidAuxData {
                opcode: format!("{:?}", inst.opcode),
                detail: format!("dictionary LVT {dict_index} out of bounds"),
            });
        }
        let locals = indices
            .iter()
            .map(|raw| {
                usize::try_from(*raw)
                    .ok()
                    .filter(|index| *index < local_count)
                    .ok_or_else(|| VmError::InvalidAuxData {
                        opcode: format!("{:?}", inst.opcode),
                        detail: format!("LVT {raw} out of bounds"),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if inst.opcode == Opcode::DictUpdateStart {
            let dict = Dict::from_value(&self.load_lvt(dict_index)?)?;
            for (key, index) in keys.iter().zip(locals) {
                match dict.get(key)? {
                    Some(value) => self.store_lvt(index, value.clone())?,
                    None => {
                        let name = self.frames[self.fi()].compiled_locals[index].name.clone();
                        let _ = self.var_unset(&name, None);
                    }
                }
            }
            return Ok(Flow::Continue);
        }
        self.pop(pc, "dictUpdateEnd keys")?;
        let Some(current) = self.load_lvt_optional(dict_index)? else {
            return Ok(Flow::Continue);
        };
        let mut dict = Dict::from_value(&current)?;
        for (key, index) in keys.into_iter().zip(locals) {
            match self.load_lvt_optional(index)? {
                Some(value) => dict.put(key, value)?,
                None => dict.remove(&key)?,
            }
        }
        self.store_lvt(dict_index, dict.into_value())?;
        Ok(Flow::Continue)
    }

    /// `TclDictWithFinish`: write the key variables back into the
    /// dictionary at the path, unless the variable or the path is gone.
    fn dict_recombine_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let field_keys = self.pop(pc, "dictRecombine keys")?.as_list_vec()?;
        let path = self.pop(pc, "dictRecombine path")?.as_list_vec()?;
        let target = if inst.opcode == Opcode::DictRecombineStk {
            let name = self.pop(pc, "dictRecombine variable")?.ensure_bytes()?;
            let (name1, name2) = split_var_name(&name);
            VariableTarget::Named(name1, name2)
        } else {
            let index = Self::operand_count(inst, 0)?;
            if index >= self.frames[self.fi()].compiled_locals.len() {
                return Err(VmError::InvalidOpcodeOperand {
                    opcode: "DictRecombineImm".into(),
                    detail: format!("LVT {index} out of bounds"),
                });
            }
            VariableTarget::Lvt(index)
        };
        let outer_value = match self.target_get(&target) {
            Ok(value) => value,
            Err(error) if is_missing_tcl_value(&error) => return Ok(Flow::Continue),
            Err(error) => return Err(error),
        };
        let mut outer = Dict::from_value(&outer_value)?;
        let mut leaf = if path.is_empty() {
            outer.clone()
        } else {
            match Dict::trace_existing(&outer_value, &path)? {
                Some(leaf) => leaf,
                None => return Ok(Flow::Continue),
            }
        };
        for key in field_keys {
            let name = key.ensure_bytes()?;
            let (name1, name2) = split_var_name(&name);
            match self.var_get(&name1, name2.as_ref()) {
                Ok(value) => leaf.put(key, value)?,
                Err(error) if is_missing_tcl_value(&error) => leaf.remove(&key)?,
                Err(error) => return Err(error),
            }
        }
        let updated = if path.is_empty() {
            leaf
        } else {
            outer.put_path(&path, leaf.into_value())?;
            outer
        };
        self.target_set(&target, updated.into_value())?;
        Ok(Flow::Continue)
    }

    fn take_stack_words(&mut self, count: usize, pc: u64, op: &str) -> Result<Vec<VmObj>, VmError> {
        let fi = self.fi();
        if self.frames[fi].stack.len() < count {
            return Err(VmError::StackUnderflow { pc, op: op.into() });
        }
        let split = self.frames[fi].stack.len() - count;
        Ok(self.frames[fi].stack.split_off(split))
    }

    /// `name(args...)` in an expression. A procedure the program defines in
    /// `::tcl::mathfunc` wins, as in Tcl; otherwise the built-ins run natively
    /// and any other name is invoked as the command Tcl would call.
    fn math_function_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let (name, args) = if inst.opcode == Opcode::CallBuiltinFunc1 {
            let index = *inst.operands.first().unwrap_or(&-1);
            let function = tcld_core::inst_table::BuiltInMathFunction::from_operand(index)
                .ok_or_else(|| VmError::InvalidOpcodeOperand {
                    opcode: "CallBuiltinFunc1".into(),
                    detail: format!("builtin {index}"),
                })?;
            let args = self.take_stack_words(function.argc() as usize, pc, "callBuiltinFunc1")?;
            (function.name().to_string(), args)
        } else {
            let argc = *inst.operands.first().unwrap_or(&0) as usize;
            let mut args = self.take_stack_words(argc, pc, "callFunc1")?;
            if args.is_empty() {
                return Err(VmError::InvalidOpcodeOperand {
                    opcode: "CallFunc1".into(),
                    detail: "zero argc".into(),
                });
            }
            let name = args.remove(0).as_string()?;
            (name, args)
        };
        let command = ByteString::from(format!("::tcl::mathfunc::{name}"));
        let namespace = self.frames[self.fi()].namespace.as_bytes().clone();
        let overridden = matches!(
            self.registry.resolve(&command, &namespace, self.mode),
            CommandRef::VmProc(_)
        );
        if !overridden && let Some(result) = crate::mathfunc::call(&name, &args, &mut self.rand) {
            self.push(result?);
            return Ok(Flow::Continue);
        }
        let mut words = vec![VmObj::Bytes(command)];
        words.extend(args);
        let call_site = CodeAddress {
            code_id: self.frames[self.fi()].code.id,
            pc: inst.pc,
        };
        match self.dispatch_invoke(&words, call_site)? {
            InvokeOutcome::Value(result) => {
                self.push(result);
                Ok(Flow::Continue)
            }
            InvokeOutcome::Raise(completion) => Ok(Flow::Raise(completion)),
            InvokeOutcome::FramePushed => Ok(Flow::Continue),
        }
    }

    /// The `str*` instructions, computed natively (`crate::strings`).
    fn string_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        use crate::strings::{self, Case};
        let pc = inst.pc as u64;
        let operand = |index: usize, default: i32| *inst.operands.get(index).unwrap_or(&default);
        let value = match inst.opcode {
            Opcode::StrMap => {
                let args = self.take_stack_words(3, pc, "strMap")?;
                strings::map(&args[0], &args[1], &args[2])?
            }
            Opcode::StrFind | Opcode::StrFindLast => {
                let args = self.take_stack_words(2, pc, "strFind")?;
                strings::find(&args[0], &args[1], inst.opcode == Opcode::StrFindLast)?
            }
            Opcode::StrRangeImm => {
                let string = self.pop(pc, "strRangeImm")?;
                strings::range_immediate(&string, operand(0, 0), operand(1, -1))?
            }
            Opcode::StrRange => {
                let args = self.take_stack_words(3, pc, "strRange")?;
                strings::range(&args[0], &args[1], &args[2])?
            }
            Opcode::StrTrim | Opcode::StrTrimLeft | Opcode::StrTrimRight => {
                let args = self.take_stack_words(2, pc, "strTrim")?;
                let left = inst.opcode != Opcode::StrTrimRight;
                let right = inst.opcode != Opcode::StrTrimLeft;
                strings::trim(&args[0], &args[1], left, right)?
            }
            Opcode::StrUpper | Opcode::StrLower | Opcode::StrTitle => {
                let string = self.pop(pc, "string case")?;
                let case = match inst.opcode {
                    Opcode::StrUpper => Case::Upper,
                    Opcode::StrLower => Case::Lower,
                    _ => Case::Title,
                };
                strings::convert_case(&string, case)?
            }
            Opcode::StrReplace => {
                let args = self.take_stack_words(4, pc, "strReplace")?;
                strings::replace(&args[0], &args[1], &args[2], &args[3])?
            }
            Opcode::StrClass => {
                let string = self.pop(pc, "strClass")?;
                strings::in_class(operand(0, -1), &string)?
            }
            _ => unreachable!(),
        };
        self.push(value);
        Ok(Flow::Continue)
    }

    /// `INST_CLOCK_READ`: clicks, microseconds, milliseconds or seconds of
    /// the wall clock. Clicks count microseconds.
    fn clock_read_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let selector = *inst.operands.first().unwrap_or(&-1);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let micros = i64::try_from(now.as_micros()).unwrap_or(i64::MAX);
        let value = match selector {
            0 | 1 | 4 => micros,
            2 => micros / 1000,
            3 => i64::try_from(now.as_secs()).unwrap_or(i64::MAX),
            _ => {
                return Err(VmError::InvalidOpcodeOperand {
                    opcode: "ClockRead".into(),
                    detail: format!("selector {selector}"),
                });
            }
        };
        self.push(VmObj::Int(value));
        Ok(Flow::Continue)
    }

    /// Tcl 9's `INST_LREPLACE4` for `lreplace` and `linsert`: positions
    /// outside the list clamp to it, so nothing is out of range.
    fn lreplace_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        const END_IS_LAST: i32 = 1;
        const SINGLE_INDEX: i32 = 2;
        let pc = inst.pc as u64;
        let count = Self::operand_count(inst, 0)?;
        let flags = *inst.operands.get(1).unwrap_or(&0);
        let indices = if flags & SINGLE_INDEX != 0 { 1 } else { 2 };
        if count < 1 + indices {
            return Err(VmError::InvalidOpcodeOperand {
                opcode: "Lreplace4".into(),
                detail: format!("{count} stack arguments"),
            });
        }
        let mut args = self.take_stack_words(count, pc, "lreplace4")?;
        let replacements = args.split_off(1 + indices);
        let mut items = args[0].as_list_vec()?;
        let length = items.len() as i64;
        let end = length - i64::from(flags & END_IS_LAST != 0);
        let first = crate::strings::index(&args[1], end)?.clamp(0, length);
        let delete = match indices {
            2 => {
                let last = crate::strings::index(&args[2], end)?;
                if last < 0 {
                    0
                } else {
                    (last.min(length) - first + 1).clamp(0, length - first)
                }
            }
            _ => 0,
        };
        let first = first as usize;
        items.splice(first..first + delete as usize, replacements);
        self.push(VmObj::List(items));
        Ok(Flow::Continue)
    }

    /// `INST_REGEXP`: whether the string matches the pattern, compiled
    /// with the instruction's flags.
    fn regexp_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let args = self.take_stack_words(2, pc, "regexp")?;
        let flags = *inst.operands.first().unwrap_or(&0);
        let re = crate::regexp_command::compiled(&args[0].as_string()?, flags)?;
        let matched = crate::regexp_command::matches(&re, &args[1].as_string()?)?;
        self.push(VmObj::Int(i64::from(matched)));
        Ok(Flow::Continue)
    }

    fn target_get(&mut self, target: &VariableTarget) -> Result<VmObj, VmError> {
        match target {
            VariableTarget::Lvt(index) => self.load_lvt(*index),
            VariableTarget::LvtArray(index, key) => self.load_lvt_array(*index, key),
            VariableTarget::Named(name1, name2) => self.var_get(name1, name2.as_ref()),
        }
    }

    fn target_set(&mut self, target: &VariableTarget, value: VmObj) -> Result<(), VmError> {
        match target {
            VariableTarget::Lvt(index) => self.store_lvt(*index, value),
            VariableTarget::LvtArray(index, key) => self.store_lvt_array(*index, key, value),
            VariableTarget::Named(name1, name2) => self.var_set(name1, name2.as_ref(), value),
        }
    }

    fn target_get_or_empty(&mut self, target: &VariableTarget) -> Result<VmObj, VmError> {
        match self.target_get(target) {
            Ok(value) => Ok(value),
            Err(VmError::TclError { message, .. })
                if message.contains("no such variable") || message.contains("no such element") =>
            {
                Ok(VmObj::empty())
            }
            Err(error) => Err(error),
        }
    }

    fn incr_extended(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let immediate = matches!(
            inst.opcode,
            Opcode::IncrArray1Imm | Opcode::IncrArrayStkImm | Opcode::IncrStkImm
        );
        let increment = if immediate {
            num_bigint::BigInt::from(*inst.operands.last().unwrap_or(&1))
        } else {
            self.pop(pc, "incr increment")?.as_bigint()?
        };
        let target = match inst.opcode {
            Opcode::IncrArray1 | Opcode::IncrArray1Imm => {
                let index = *inst.operands.first().unwrap_or(&0) as usize;
                let key = self.pop(pc, "incr array key")?.ensure_bytes()?;
                VariableTarget::LvtArray(index, key)
            }
            Opcode::IncrArrayStk | Opcode::IncrArrayStkImm => {
                let key = self.pop(pc, "incr array key")?.ensure_bytes()?;
                let name = self.pop(pc, "incr array name")?.ensure_bytes()?;
                VariableTarget::Named(name, Some(key))
            }
            Opcode::IncrScalarStk | Opcode::IncrStk | Opcode::IncrStkImm => {
                let name = self.pop(pc, "incr variable name")?.ensure_bytes()?;
                let (name1, name2) = split_var_name(&name);
                VariableTarget::Named(name1, name2)
            }
            _ => unreachable!(),
        };
        let old = self.target_get_or_empty(&target)?;
        let base = if old.ensure_bytes()?.is_empty() {
            num_bigint::BigInt::from(0)
        } else {
            old.as_bigint()?
        };
        let value = VmObj::from_bigint(base + increment);
        self.target_set(&target, value.clone())?;
        self.push(value);
        Ok(Flow::Continue)
    }

    fn append_extended(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        let appended = self.pop(pc, "append value")?;
        let target = match inst.opcode {
            Opcode::AppendScalar1
            | Opcode::AppendScalar4
            | Opcode::LappendScalar1
            | Opcode::LappendScalar4
            | Opcode::LappendList => {
                VariableTarget::Lvt(*inst.operands.first().unwrap_or(&0) as usize)
            }
            Opcode::AppendArray1
            | Opcode::AppendArray4
            | Opcode::LappendArray1
            | Opcode::LappendArray4
            | Opcode::LappendListArray => {
                let key = self.pop(pc, "append array key")?.ensure_bytes()?;
                VariableTarget::LvtArray(*inst.operands.first().unwrap_or(&0) as usize, key)
            }
            Opcode::AppendArrayStk | Opcode::LappendArrayStk | Opcode::LappendListArrayStk => {
                let key = self.pop(pc, "append array key")?.ensure_bytes()?;
                let name = self.pop(pc, "append array name")?.ensure_bytes()?;
                VariableTarget::Named(name, Some(key))
            }
            Opcode::AppendStk | Opcode::LappendStk | Opcode::LappendListStk => {
                let name = self.pop(pc, "append variable name")?.ensure_bytes()?;
                let (name1, name2) = split_var_name(&name);
                VariableTarget::Named(name1, name2)
            }
            _ => unreachable!(),
        };
        let current = self.target_get_or_empty(&target)?;
        let value = if matches!(
            inst.opcode,
            Opcode::AppendScalar1
                | Opcode::AppendScalar4
                | Opcode::AppendArray1
                | Opcode::AppendArray4
                | Opcode::AppendArrayStk
                | Opcode::AppendStk
        ) {
            let mut bytes = current.ensure_bytes()?.as_bytes().to_vec();
            bytes.extend_from_slice(appended.ensure_bytes()?.as_bytes());
            VmObj::from_bytes(bytes)
        } else {
            let mut list = if current.ensure_bytes()?.is_empty() {
                Vec::new()
            } else {
                current.as_list_vec()?
            };
            if matches!(
                inst.opcode,
                Opcode::LappendList
                    | Opcode::LappendListArray
                    | Opcode::LappendListArrayStk
                    | Opcode::LappendListStk
            ) {
                list.extend(appended.as_list_vec()?);
            } else {
                list.push(appended);
            }
            VmObj::List(list)
        };
        self.target_set(&target, value.clone())?;
        self.push(value);
        Ok(Flow::Continue)
    }

    fn variable_state_opcode(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        match inst.opcode {
            Opcode::ExistArray => {
                let index = *inst.operands.first().unwrap_or(&0) as usize;
                let key = self.pop(pc, "existArray")?.ensure_bytes()?;
                let exists = self.load_lvt_array(index, &key).is_ok();
                self.push(VmObj::Int(i64::from(exists)));
            }
            Opcode::ExistArrayStk => {
                let key = self.pop(pc, "existArrayStk")?.ensure_bytes()?;
                let name = self.pop(pc, "existArrayStk")?.ensure_bytes()?;
                let exists = self.var_get(&name, Some(&key)).is_ok();
                self.push(VmObj::Int(i64::from(exists)));
            }
            Opcode::ExistStk => {
                let name = self.pop(pc, "existStk")?.ensure_bytes()?;
                let (name1, name2) = split_var_name(&name);
                let exists = self.var_get(&name1, name2.as_ref()).is_ok();
                self.push(VmObj::Int(i64::from(exists)));
            }
            Opcode::UnsetScalar | Opcode::UnsetArray => {
                let leave_error = *inst.operands.first().unwrap_or(&0) != 0;
                let index = *inst.operands.get(1).unwrap_or(&0) as usize;
                let name = self.frames[self.fi()]
                    .compiled_locals
                    .get(index)
                    .ok_or_else(|| VmError::InvalidOpcodeOperand {
                        opcode: format!("{:?}", inst.opcode),
                        detail: format!("LVT {index} out of bounds"),
                    })?
                    .name
                    .clone();
                let key = if inst.opcode == Opcode::UnsetArray {
                    Some(self.pop(pc, "unsetArray")?.ensure_bytes()?)
                } else {
                    None
                };
                if let Err(error) = self.var_unset(&name, key.as_ref())
                    && leave_error
                {
                    return Err(error);
                }
            }
            Opcode::UnsetArrayStk => {
                let leave_error = *inst.operands.first().unwrap_or(&0) != 0;
                let key = self.pop(pc, "unsetArrayStk")?.ensure_bytes()?;
                let name = self.pop(pc, "unsetArrayStk")?.ensure_bytes()?;
                if let Err(error) = self.var_unset(&name, Some(&key))
                    && leave_error
                {
                    return Err(error);
                }
            }
            Opcode::UnsetStk => {
                let leave_error = *inst.operands.first().unwrap_or(&0) != 0;
                let name = self.pop(pc, "unsetStk")?.ensure_bytes()?;
                let (name1, name2) = split_var_name(&name);
                if let Err(error) = self.var_unset(&name1, name2.as_ref())
                    && leave_error
                {
                    return Err(error);
                }
            }
            Opcode::ArrayExistsImm => {
                let index = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                let name = self.frames[fi]
                    .compiled_locals
                    .get(index)
                    .ok_or_else(|| VmError::InvalidOpcodeOperand {
                        opcode: "ArrayExistsImm".into(),
                        detail: format!("LVT {index} out of bounds"),
                    })?
                    .name
                    .clone();
                let exists = self.array_exists_named(&name)?;
                self.push(VmObj::Int(i64::from(exists)));
            }
            Opcode::ArrayExistsStk => {
                let name = self.pop(pc, "arrayExistsStk")?.ensure_bytes()?;
                let exists = self.array_exists_named(&name)?;
                self.push(VmObj::Int(i64::from(exists)));
            }
            Opcode::ArrayMakeImm => {
                let index = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                let name = self.frames[fi]
                    .compiled_locals
                    .get(index)
                    .ok_or_else(|| VmError::InvalidOpcodeOperand {
                        opcode: "ArrayMakeImm".into(),
                        detail: format!("LVT {index} out of bounds"),
                    })?
                    .name
                    .clone();
                self.make_array(&name)?;
            }
            Opcode::ArrayMakeStk => {
                let name = self.pop(pc, "arrayMakeStk")?.ensure_bytes()?;
                self.make_array(&name)?;
            }
            _ => unreachable!(),
        }
        Ok(Flow::Continue)
    }

    fn list_extended(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let pc = inst.pc as u64;
        match inst.opcode {
            Opcode::ListIndexImm => {
                let list = self.pop(pc, "listIndexImm")?.as_list_vec()?;
                let index =
                    decode_immediate_index(*inst.operands.first().unwrap_or(&-1), list.len());
                self.push(
                    index
                        .and_then(|i| list.get(i).cloned())
                        .unwrap_or_else(VmObj::empty),
                );
            }
            Opcode::ListRangeImm => {
                let list = self.pop(pc, "listRangeImm")?.as_list_vec()?;
                let start =
                    decode_immediate_index(*inst.operands.first().unwrap_or(&0), list.len())
                        .unwrap_or(0)
                        .min(list.len());
                let end = decode_immediate_index(*inst.operands.get(1).unwrap_or(&-1), list.len());
                let result = match end {
                    Some(end) if start <= end && start < list.len() => {
                        list[start..=end.min(list.len() - 1)].to_vec()
                    }
                    _ => Vec::new(),
                };
                self.push(VmObj::List(result));
            }
            Opcode::ListIndexMulti => {
                let count = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                if count == 0 || self.frames[fi].stack.len() < count {
                    return Err(VmError::StackUnderflow {
                        pc,
                        op: "lindexMulti".into(),
                    });
                }
                let split = self.frames[fi].stack.len() - count;
                let values = self.frames[fi].stack.split_off(split);
                self.push(crate::lists::index_flat(&values[0], &values[1..])?);
            }
            Opcode::ListIn | Opcode::ListNotIn => {
                let list = self.pop(pc, "list membership")?.as_list_vec()?;
                let needle = self.pop(pc, "list membership")?.ensure_bytes()?;
                let mut found = false;
                for value in &list {
                    if value.ensure_bytes()?.as_bytes() == needle.as_bytes() {
                        found = true;
                        break;
                    }
                }
                if inst.opcode == Opcode::ListNotIn {
                    found = !found;
                }
                self.push(VmObj::Int(i64::from(found)));
            }
            Opcode::ListConcat => {
                let right = self.pop(pc, "listConcat")?.as_list_vec()?;
                let mut left = self.pop(pc, "listConcat")?.as_list_vec()?;
                left.extend(right);
                self.push(VmObj::List(left));
            }
            Opcode::ConcatStk => {
                let count = *inst.operands.first().unwrap_or(&0) as usize;
                let fi = self.fi();
                if self.frames[fi].stack.len() < count {
                    return Err(VmError::StackUnderflow {
                        pc,
                        op: "concatStk".into(),
                    });
                }
                let split = self.frames[fi].stack.len() - count;
                let values = self.frames[fi].stack.split_off(split);
                self.push(crate::lists::concat(&values)?);
            }
            Opcode::LsetList | Opcode::LsetFlat => {
                let (list, indices, replacement) = if inst.opcode == Opcode::LsetList {
                    let list = self.pop(pc, "lsetList source")?;
                    let replacement = self.pop(pc, "lsetList value")?;
                    let indices = self.pop(pc, "lsetList indices")?.as_list_vec()?;
                    (list, indices, replacement)
                } else {
                    let count = *inst.operands.first().unwrap_or(&0) as usize;
                    let fi = self.fi();
                    if count < 2 || self.frames[fi].stack.len() < count {
                        return Err(VmError::StackUnderflow {
                            pc,
                            op: "lsetFlat".into(),
                        });
                    }
                    let split = self.frames[fi].stack.len() - count;
                    let values = self.frames[fi].stack.split_off(split);
                    (
                        values.last().cloned().unwrap(),
                        values[..values.len() - 2].to_vec(),
                        values[values.len() - 2].clone(),
                    )
                };
                self.push(lset_nested(list, &indices, replacement)?);
            }
            _ => unreachable!(),
        }
        Ok(Flow::Continue)
    }

    fn aux_index(&self, inst: &crate::load::DecodedInst, opcode: &str) -> Result<usize, VmError> {
        let raw = *inst
            .operands
            .first()
            .ok_or_else(|| VmError::InvalidAuxData {
                opcode: opcode.into(),
                detail: "missing auxiliary operand".into(),
            })?;
        let index = usize::try_from(raw).map_err(|_| VmError::InvalidAuxData {
            opcode: opcode.into(),
            detail: format!("negative auxiliary index {raw}"),
        })?;
        if index >= self.frames[self.fi()].code.bytecode.auxiliaries.len() {
            return Err(VmError::InvalidAuxData {
                opcode: opcode.into(),
                detail: format!("auxiliary index {index} is out of bounds"),
            });
        }
        Ok(index)
    }

    fn foreach_step_legacy(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let aux_index = self.aux_index(inst, "ForeachStep4")?;
        let info = match &self.frames[self.fi()].code.bytecode.auxiliaries[aux_index].value {
            AuxDataValue::Foreach(info) => info.clone(),
            _ => {
                return Err(VmError::InvalidAuxData {
                    opcode: "ForeachStep4".into(),
                    detail: format!("auxiliary {aux_index} is not legacy foreach"),
                });
            }
        };
        self.validate_legacy_foreach_lvt(&info, "ForeachStep4")?;
        let counter_index =
            usize::try_from(info.loop_counter).map_err(|_| VmError::InvalidAuxData {
                opcode: "ForeachStep4".into(),
                detail: format!("negative loop counter LVT {}", info.loop_counter),
            })?;
        let iteration = self.load_lvt(counter_index)?.as_i64()? + 1;
        if iteration < 0 {
            return Err(VmError::InvalidAuxData {
                opcode: "ForeachStep4".into(),
                detail: "loop counter overflow".into(),
            });
        }
        self.store_lvt(counter_index, VmObj::Int(iteration))?;
        let iteration = iteration as usize;

        let first_value =
            usize::try_from(info.first_value).map_err(|_| VmError::InvalidAuxData {
                opcode: "ForeachStep4".into(),
                detail: format!("negative first-value LVT {}", info.first_value),
            })?;
        let mut parsed = Vec::with_capacity(info.lists.len());
        let mut continue_loop = false;
        for (list_index, vars) in info.lists.iter().enumerate() {
            if vars.is_empty() {
                return Err(VmError::InvalidAuxData {
                    opcode: "ForeachStep4".into(),
                    detail: format!("variable group {list_index} is empty"),
                });
            }
            let values = self.load_lvt(first_value + list_index)?.as_list_vec()?;
            continue_loop |= values.len() > iteration.saturating_mul(vars.len());
            parsed.push(values);
        }

        if continue_loop {
            for (values, vars) in parsed.iter().zip(&info.lists) {
                let base = iteration.saturating_mul(vars.len());
                for (offset, raw_var) in vars.iter().enumerate() {
                    let variable =
                        usize::try_from(*raw_var).map_err(|_| VmError::InvalidAuxData {
                            opcode: "ForeachStep4".into(),
                            detail: format!("negative variable LVT {raw_var}"),
                        })?;
                    self.store_lvt(
                        variable,
                        values
                            .get(base + offset)
                            .cloned()
                            .unwrap_or_else(VmObj::empty),
                    )?;
                }
            }
        }
        self.push(VmObj::Int(i64::from(continue_loop)));
        Ok(Flow::Continue)
    }

    fn foreach_start_new(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let aux_index = self.aux_index(inst, "ForeachStart")?;
        let info = match &self.frames[self.fi()].code.bytecode.auxiliaries[aux_index].value {
            AuxDataValue::NewForeach(info) => info.clone(),
            _ => {
                return Err(VmError::InvalidAuxData {
                    opcode: "ForeachStart".into(),
                    detail: format!("auxiliary {aux_index} is not new foreach"),
                });
            }
        };
        if info.lists.is_empty() || info.lists.iter().any(Vec::is_empty) {
            return Err(VmError::InvalidAuxData {
                opcode: "ForeachStart".into(),
                detail: "foreach requires non-empty list variable groups".into(),
            });
        }
        for raw_variable in info.lists.iter().flatten() {
            let variable = usize::try_from(*raw_variable).map_err(|_| VmError::InvalidAuxData {
                opcode: "ForeachStart".into(),
                detail: format!("negative variable LVT {raw_variable}"),
            })?;
            self.validate_lvt_index(variable, "ForeachStart")?;
        }
        let fi = self.fi();
        let num_lists = info.lists.len();
        if self.frames[fi].stack.len() < num_lists {
            return Err(VmError::StackUnderflow {
                pc: inst.pc as u64,
                op: "foreachStart".into(),
            });
        }
        let stack_base = self.frames[fi].stack.len() - num_lists;
        let mut lists = Vec::with_capacity(num_lists);
        let mut iteration_count = 0usize;
        for (value, variables) in self.frames[fi].stack[stack_base..].iter().zip(&info.lists) {
            let parsed = value.as_list_vec()?;
            iteration_count = iteration_count.max(parsed.len().div_ceil(variables.len()));
            lists.push(parsed);
        }
        let id = self.next_aux_state_id;
        self.next_aux_state_id = self
            .next_aux_state_id
            .checked_add(1)
            .ok_or_else(|| VmError::Internal("auxiliary state id exhausted".into()))?;
        self.frames[fi]
            .aux_states
            .push(AuxRuntimeState::NewForeach(NewForeachState {
                id,
                info: info.clone(),
                lists,
                iteration: 0,
                iteration_count,
                stack_base,
            }));
        self.frames[fi]
            .stack
            .push(VmObj::Int(iteration_count as i64));
        self.frames[fi].stack.push(VmObj::Int(id));

        let target = inst.pc as i64 + inst.desc.byte_count as i64 - info.loop_counter as i64;
        self.validated_jump(inst, target)
    }

    fn foreach_step_new(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let state_id = self
            .frames
            .last()
            .and_then(|frame| frame.stack.last())
            .ok_or_else(|| VmError::InvalidAuxData {
                opcode: "ForeachStep".into(),
                detail: "no active foreach state word".into(),
            })?
            .as_i64()?;
        let fi = self.fi();
        let state_index = self.frames[fi]
            .aux_states
            .iter()
            .rposition(|state| matches!(state, AuxRuntimeState::NewForeach(s) if s.id == state_id))
            .ok_or_else(|| VmError::InvalidAuxData {
                opcode: "ForeachStep".into(),
                detail: format!("no active foreach state {state_id}"),
            })?;
        let (info, lists, iteration, iteration_count) =
            match &self.frames[fi].aux_states[state_index] {
                AuxRuntimeState::NewForeach(state) => (
                    state.info.clone(),
                    state.lists.clone(),
                    state.iteration,
                    state.iteration_count,
                ),
                AuxRuntimeState::DictIter(_) => unreachable!(),
                AuxRuntimeState::Expand(_) => unreachable!(),
            };
        if iteration >= iteration_count {
            return Ok(Flow::Continue);
        }
        for (values, vars) in lists.iter().zip(&info.lists) {
            let base = iteration.saturating_mul(vars.len());
            for (offset, raw_var) in vars.iter().enumerate() {
                let variable = usize::try_from(*raw_var).map_err(|_| VmError::InvalidAuxData {
                    opcode: "ForeachStep".into(),
                    detail: format!("negative variable LVT {raw_var}"),
                })?;
                self.store_lvt(
                    variable,
                    values
                        .get(base + offset)
                        .cloned()
                        .unwrap_or_else(VmObj::empty),
                )?;
            }
        }
        match &mut self.frames[fi].aux_states[state_index] {
            AuxRuntimeState::NewForeach(state) => state.iteration += 1,
            AuxRuntimeState::DictIter(_) => unreachable!(),
            AuxRuntimeState::Expand(_) => unreachable!(),
        }
        let target = inst.pc as i64 + info.loop_counter as i64;
        self.validated_jump(inst, target)
    }

    /// Handling break/continue changes the routed code, while retaining Tcl's
    /// custom return options. A plain loop command leaves earlier options intact.
    fn consume_loop_completion(&mut self, completion: &Completion) -> Result<(), VmError> {
        if completion.options.raw.is_some() {
            self.last_completion = completion.clone();
        }
        self.last_completion.code = 0;
        self.last_completion.options.code = 0;
        self.last_completion.options.level = 0;
        if self.last_completion.options.raw.is_some() {
            self.last_completion.options.sync_raw_code_level()?;
        }
        Ok(())
    }

    fn lmap_collect(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let result = self.pop(inst.pc as u64, "lmapCollect")?;
        let fi = self.fi();
        let frame = &mut self.frames[fi];
        let invalid = || VmError::InvalidAuxData {
            opcode: "LmapCollect".into(),
            detail: "expected foreach state above an lmap collecting list".into(),
        };
        let Some(AuxRuntimeState::NewForeach(state)) = frame.aux_states.last() else {
            return Err(invalid());
        };
        if frame.stack.len() != state.stack_base + state.info.lists.len() + 2
            || frame.stack.last().and_then(|value| value.as_i64().ok()) != Some(state.id)
        {
            return Err(invalid());
        }
        let index = state.stack_base.checked_sub(1).ok_or_else(invalid)?;
        match &mut frame.stack[index] {
            VmObj::List(collected) => collected.push(result),
            value => {
                let mut collected = value.as_list_vec()?;
                collected.push(result);
                *value = VmObj::List(collected);
            }
        }
        Ok(Flow::Continue)
    }

    fn foreach_end_new(&mut self, _inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let state_id = self
            .frames
            .last()
            .and_then(|frame| frame.stack.last())
            .ok_or_else(|| VmError::InvalidAuxData {
                opcode: "ForeachEnd".into(),
                detail: "no active foreach state word".into(),
            })?
            .as_i64()?;
        let fi = self.fi();
        let state = self.frames[fi]
            .aux_states
            .pop()
            .ok_or_else(|| VmError::InvalidAuxData {
                opcode: "ForeachEnd".into(),
                detail: "no active foreach state".into(),
            })?;
        let AuxRuntimeState::NewForeach(state) = state else {
            return Err(VmError::InvalidAuxData {
                opcode: "ForeachEnd".into(),
                detail: "top auxiliary state is not foreach".into(),
            });
        };
        if state.id != state_id {
            self.frames[fi]
                .aux_states
                .push(AuxRuntimeState::NewForeach(state));
            return Err(VmError::InvalidAuxData {
                opcode: "ForeachEnd".into(),
                detail: format!("foreach state mismatch for {state_id}"),
            });
        }
        self.frames[fi].stack.truncate(state.stack_base);
        Ok(Flow::Continue)
    }

    fn validate_lvt_index(&self, index: usize, opcode: &str) -> Result<(), VmError> {
        if index >= self.frames[self.fi()].compiled_locals.len() {
            return Err(VmError::InvalidAuxData {
                opcode: opcode.into(),
                detail: format!("LVT index {index} is out of bounds"),
            });
        }
        Ok(())
    }

    fn validate_legacy_foreach_lvt(
        &self,
        info: &tcld_core::bytecode::ForeachInfo,
        opcode: &str,
    ) -> Result<(), VmError> {
        let counter = usize::try_from(info.loop_counter).map_err(|_| VmError::InvalidAuxData {
            opcode: opcode.into(),
            detail: format!("negative loop counter LVT {}", info.loop_counter),
        })?;
        self.validate_lvt_index(counter, opcode)?;
        let first_value =
            usize::try_from(info.first_value).map_err(|_| VmError::InvalidAuxData {
                opcode: opcode.into(),
                detail: format!("negative first-value LVT {}", info.first_value),
            })?;
        for list_index in 0..info.lists.len() {
            self.validate_lvt_index(first_value + list_index, opcode)?;
        }
        for raw_variable in info.lists.iter().flatten() {
            let variable = usize::try_from(*raw_variable).map_err(|_| VmError::InvalidAuxData {
                opcode: opcode.into(),
                detail: format!("negative variable LVT {raw_variable}"),
            })?;
            self.validate_lvt_index(variable, opcode)?;
        }
        Ok(())
    }

    fn jump_table(&mut self, inst: &crate::load::DecodedInst) -> Result<Flow, VmError> {
        let opcode = "JumpTable";
        let aux_index = self.aux_index(inst, opcode)?;
        let table = match &self.frames[self.fi()].code.bytecode.auxiliaries[aux_index].value {
            AuxDataValue::JumpTable(info) => info.dict.clone(),
            _ => {
                return Err(VmError::InvalidAuxData {
                    opcode: opcode.into(),
                    detail: format!("auxiliary {aux_index} is not a jump table"),
                });
            }
        };
        let key = self.pop(inst.pc as u64, opcode)?.ensure_bytes()?;
        let offset = table
            .iter()
            .find(|(candidate, _)| candidate.as_bytes() == key.as_bytes())
            .map(|(_, offset)| *offset);
        match offset {
            Some(offset) => self.validated_jump(inst, inst.pc as i64 + offset as i64),
            None => Ok(Flow::Continue),
        }
    }

    fn validated_jump(
        &self,
        inst: &crate::load::DecodedInst,
        target: i64,
    ) -> Result<Flow, VmError> {
        let code = &self.frames[self.fi()].code;
        let target = u32::try_from(target).map_err(|_| VmError::InvalidJumpTarget {
            code_id: code.id.0,
            pc: target.max(0) as u32,
        })?;
        if !code.pc_to_index.contains_key(&target) {
            return Err(VmError::InvalidJumpTarget {
                code_id: code.id.0,
                pc: target,
            });
        }
        let _ = inst;
        Ok(Flow::JumpPc(target))
    }

    fn binop_num(&mut self, op: Opcode, pc: u64) -> Result<Flow, VmError> {
        use crate::number::BinaryOp;
        let b = self.pop(pc, "binop")?;
        let a = self.pop(pc, "binop")?;
        let op = match op {
            Opcode::Add => BinaryOp::Add,
            Opcode::Sub => BinaryOp::Sub,
            Opcode::Mult => BinaryOp::Mul,
            Opcode::Div => BinaryOp::Div,
            Opcode::Mod => BinaryOp::Mod,
            Opcode::Expon => BinaryOp::Pow,
            Opcode::Lshift => BinaryOp::Shl,
            Opcode::Rshift => BinaryOp::Shr,
            Opcode::Bitand => BinaryOp::And,
            Opcode::Bitor => BinaryOp::Or,
            _ => BinaryOp::Xor,
        };
        self.push(crate::number::binary(op, &a, &b)?);
        Ok(Flow::Continue)
    }

    fn binop_cmp(&mut self, op: Opcode, pc: u64) -> Result<Flow, VmError> {
        use crate::number::Comparison;
        let b = self.pop(pc, "cmp")?;
        let a = self.pop(pc, "cmp")?;
        let comparison = match op {
            Opcode::Eq => Comparison::Eq,
            Opcode::Neq => Comparison::Ne,
            Opcode::Lt => Comparison::Lt,
            Opcode::Gt => Comparison::Gt,
            Opcode::Le => Comparison::Le,
            _ => Comparison::Ge,
        };
        let holds = crate::number::compare(comparison, &a, &b)?;
        self.push(VmObj::Int(i64::from(holds)));
        Ok(Flow::Continue)
    }

    fn dispatch_invoke(
        &mut self,
        words: &[VmObj],
        call_site: CodeAddress,
    ) -> Result<InvokeOutcome, VmError> {
        #[cfg(feature = "debug")]
        let source = self.source_images.contains_key(&call_site.code_id);
        #[cfg(not(feature = "debug"))]
        let source = false;
        if (source || self.mode == RuntimeMode::VmStrict)
            && !matches!(
                self.registry.resolve(
                    &words[0].ensure_bytes()?,
                    self.frames[self.fi()].namespace.as_bytes(),
                    self.mode
                ),
                CommandRef::VmProc(_)
            )
            && let Some(outcome) = self.invoke_native(words, call_site)?
        {
            return Ok(outcome);
        }
        #[cfg(feature = "debug")]
        if self.source_images.contains_key(&call_site.code_id)
            && !matches!(
                self.registry.resolve(
                    &words[0].ensure_bytes()?,
                    self.frames[self.fi()].namespace.as_bytes(),
                    self.mode
                ),
                CommandRef::VmProc(_)
            )
            && let Some(outcome) = self.invoke_generated(words, call_site)?
        {
            return Ok(outcome);
        }
        let cmd = words[0].ensure_bytes()?;
        let namespace = self.frames[self.fi()].namespace.clone();
        let resolved = self.registry.resolve(&cmd, namespace.as_bytes(), self.mode);
        match resolved {
            CommandRef::Facade(FacadeKind::BcProc) => {
                tbcload::handle_bcproc_in_namespace(
                    words,
                    &mut self.registry,
                    namespace.as_bytes(),
                )?;
                #[cfg(feature = "host")]
                {
                    if words.len() >= 2 {
                        let name = words[1].ensure_bytes()?;
                        let qualified = namespace.qualify(&name);
                        let q = qualified.as_str_lossy().into_owned();
                        if let Some(host) = self.host.as_mut() {
                            host.install_stub_command(&q)?;
                        } else if let Some(result) =
                            self.inspection_bridge(|h| h.install_stub_command(&q))
                        {
                            result?;
                        }
                    }
                }
                Ok(InvokeOutcome::Value(VmObj::empty()))
            }
            CommandRef::Facade(FacadeKind::BcEval) => {
                let body = tbcload::handle_bceval_body(words)?;
                self.push_call_frame(body, FrameKind::Toplevel, None, &[], namespace, call_site)?;
                Ok(InvokeOutcome::FramePushed)
            }
            CommandRef::VmProc(proc) => {
                self.push_call_frame(
                    Arc::clone(&proc.code),
                    FrameKind::VmProc,
                    Some(proc.qname.clone()),
                    &words[1..],
                    NamespaceName::from_proc_qname(&proc.qname),
                    call_site,
                )
                .map_err(|error| match error {
                    VmError::TclError {
                        message,
                        error_info,
                        error_code,
                    } => VmError::TclError {
                        message: message.replacen(
                            "should be \"proc",
                            &format!("should be \"{}", words[0].as_string().unwrap_or_default()),
                            1,
                        ),
                        error_info,
                        error_code,
                    },
                    other => other,
                })?;
                let frame = self.frames.last_mut().unwrap();
                frame.invocation = Some(VmObj::List(words.to_vec()));
                Ok(InvokeOutcome::FramePushed)
            }
            CommandRef::VarNameBuiltin { name } => {
                let mut ctx = EngineVarCtx { engine: self };
                let v = builtin::run_var_name_builtin(name.as_bytes(), &words[1..], &mut ctx)?;
                Ok(InvokeOutcome::Value(v))
            }
            CommandRef::ValueBuiltin { name } => Ok(InvokeOutcome::Value(
                builtin::run_value_builtin(name.as_bytes(), &words[0], &words[1..])?,
            )),
            CommandRef::WorldHost { name } => self.invoke_world_host_command(words, &name),
            CommandRef::Denied { name } => Err(denied_command(&name, self.mode)),
        }
    }

    fn push_call_frame(
        &mut self,
        code: Arc<LoadedCode>,
        kind: FrameKind,
        proc_qname: Option<ByteString>,
        args: &[VmObj],
        namespace: NamespaceName,
        call_site: CodeAddress,
    ) -> Result<(), VmError> {
        let caller = self.frames[self.fi()].id;
        let resume_pc_index = self.frames[self.fi()].pc_index;
        let caller_stack_depth = self.frames[self.fi()].stack.len();
        self.push_frame(
            code,
            kind,
            proc_qname,
            args,
            namespace,
            Some(CallContinuation {
                caller,
                call_site,
                resume_pc_index,
                caller_stack_depth,
            }),
        )?;
        Ok(())
    }

    fn invoke_world_host_command(
        &mut self,
        words: &[VmObj],
        name: &ByteString,
    ) -> Result<InvokeOutcome, VmError> {
        if crate::cmd::normalize_cmd_key(name.as_bytes()) == b"catch" {
            return self.invoke_mirrored_catch(words);
        }
        if crate::cmd::normalize_cmd_key(name.as_bytes()) == b"switch"
            && let Some(outcome) = self.invoke_exact_switch(words)?
        {
            return Ok(outcome);
        }
        if crate::cmd::normalize_cmd_key(name.as_bytes()) == b"regexp" && words.len() > 3 {
            let mut pattern_index = 1usize;
            while pattern_index < words.len() {
                let word = words[pattern_index].ensure_bytes()?;
                if !word.as_bytes().starts_with(b"-") || word.as_bytes() == b"--" {
                    if word.as_bytes() == b"--" {
                        pattern_index += 1;
                    }
                    break;
                }
                pattern_index += 1;
            }
            let output_start = pattern_index.saturating_add(2);
            if output_start < words.len() {
                let token = self.next_aux_state_id;
                self.next_aux_state_id += 1;
                let mut bridged = words.to_vec();
                let mut outputs = Vec::new();
                for (offset, original) in words[output_start..].iter().enumerate() {
                    let original = original.ensure_bytes()?;
                    let temp =
                        ByteString::from(format!("::__tcld_regexp_{token}_{offset}").as_str());
                    bridged[output_start + offset] = VmObj::Bytes(temp.clone());
                    outputs.push((original, temp));
                }
                let outcome = self.invoke_world_host(&bridged, name)?;
                if matches!(outcome, InvokeOutcome::Value(_)) {
                    for (original, temp) in outputs {
                        if let Ok(value) = self.host_get(&temp, None) {
                            let (name1, name2) = split_var_name(&original);
                            self.var_set(&name1, name2.as_ref(), value)?;
                        }
                    }
                }
                return Ok(outcome);
            }
        }
        self.invoke_world_host(words, name)
    }

    fn invoke_mirrored_catch(&mut self, words: &[VmObj]) -> Result<InvokeOutcome, VmError> {
        if !(2..=4).contains(&words.len()) {
            return self.invoke_world_host(words, &words[0].ensure_bytes()?);
        }
        if !self.policy.allow_dynamic_eval {
            return Err(VmError::PolicyDenied {
                action: "dynamic_eval",
                detail: "catch body requires HostFrame".into(),
            });
        }
        let completion = match self.host_frame_eval(words[1].clone(), false)? {
            Flow::Continue => Completion::ok(self.pop(0, "catch result")?),
            Flow::Raise(completion) => completion,
            Flow::JumpPc(_) | Flow::FrameDone(_) => {
                return Err(VmError::Internal(
                    "catch HostFrame returned scheduler flow".into(),
                ));
            }
        };
        if let Some(result_name) = words.get(2) {
            let result_name = result_name.ensure_bytes()?;
            let (name1, name2) = split_var_name(&result_name);
            self.var_set(&name1, name2.as_ref(), completion.result.clone())?;
        }
        if let Some(options_name) = words.get(3) {
            let options_name = options_name.ensure_bytes()?;
            let (name1, name2) = split_var_name(&options_name);
            self.var_set(&name1, name2.as_ref(), completion.options.as_vm_obj())?;
        }
        Ok(InvokeOutcome::Value(VmObj::from_int(
            completion.code as i64,
        )))
    }

    fn invoke_exact_switch(&mut self, words: &[VmObj]) -> Result<Option<InvokeOutcome>, VmError> {
        let mut cursor = 1usize;
        let mut nocase = false;
        while cursor < words.len() {
            let option = words[cursor].ensure_bytes()?;
            match option.as_bytes() {
                b"-exact" => cursor += 1,
                b"-nocase" => {
                    nocase = true;
                    cursor += 1;
                }
                b"--" => {
                    cursor += 1;
                    break;
                }
                bytes if bytes.starts_with(b"-") => return Ok(None),
                _ => break,
            }
        }
        if cursor >= words.len() {
            return Ok(None);
        }
        let value = words[cursor].ensure_bytes()?;
        cursor += 1;
        let mut clauses = if words.len().saturating_sub(cursor) == 1 {
            words[cursor].as_list_vec()?
        } else {
            words[cursor..].to_vec()
        };
        if clauses.len() % 2 != 0 {
            return Err(VmError::TclError {
                message: "extra switch pattern with no body".into(),
                error_info: None,
                error_code: Some("TCL OPERATION SWITCH BADARM".into()),
            });
        }
        let value_cmp = if nocase {
            value.as_str_lossy().to_lowercase().into_bytes()
        } else {
            value.as_bytes().to_vec()
        };
        let mut selected = None;
        let mut default = None;
        for pair in clauses.chunks_exact_mut(2) {
            let pattern = pair[0].ensure_bytes()?;
            if pattern.as_bytes() == b"default" {
                default = Some(pair[1].clone());
                continue;
            }
            let pattern_cmp = if nocase {
                pattern.as_str_lossy().to_lowercase().into_bytes()
            } else {
                pattern.as_bytes().to_vec()
            };
            if pattern_cmp == value_cmp {
                selected = Some(pair[1].clone());
                break;
            }
        }
        let Some(body) = selected.or(default) else {
            return Ok(Some(InvokeOutcome::Value(VmObj::empty())));
        };
        let outcome = match self.host_frame_eval(body, false)? {
            Flow::Continue => InvokeOutcome::Value(self.pop(0, "switch result")?),
            Flow::Raise(completion) => InvokeOutcome::Raise(completion),
            Flow::JumpPc(_) | Flow::FrameDone(_) => {
                return Err(VmError::Internal(
                    "switch host frame returned scheduler flow".into(),
                ));
            }
        };
        Ok(Some(outcome))
    }

    fn invoke_world_host(
        &mut self,
        words: &[VmObj],
        _name: &ByteString,
    ) -> Result<InvokeOutcome, VmError> {
        if !self.policy.allow_host_invoke {
            return Err(VmError::PolicyDenied {
                action: "world_host",
                detail: "host invoke disabled".into(),
            });
        }
        self.host_command_invocations += 1;
        #[cfg(feature = "host")]
        {
            // Nested WorldHost during Option B re-entry: host lives on ACTIVE_BRIDGE.
            if self.host.is_none() {
                let br = self
                    .inspection_bridge(|host| host.invoke_objv(words))
                    .ok_or_else(|| {
                        VmError::HybridUnavailable(
                            "nested host invoke without active bridge".into(),
                        )
                    })??;
                return self.map_bridge_result(br);
            }

            // Take host out so stub re-entry can borrow engine without aliasing.
            let mut host = self
                .host
                .take()
                .ok_or_else(|| VmError::HybridUnavailable("host not bootstrapped".into()))?;
            let eng_ptr = self as *mut HybridEngineInner;
            let br = host.with_stub_dispatch(
                Box::new(move |args| {
                    // Safety: host is on outer stack; only engine fields (not host)
                    // are used by stub re-entry until nested host uses ACTIVE_BRIDGE.
                    let eng = unsafe { &mut *eng_ptr };
                    eng.dispatch_stub_reenter(args)
                }),
                |h| h.invoke_objv(words),
            );
            self.host = Some(host);
            self.map_bridge_result(br?)
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = words;
            Err(VmError::HybridUnavailable(
                "built without host feature".into(),
            ))
        }
    }

    #[cfg(feature = "host")]
    fn invoke_host_frame_bridge(
        &mut self,
        namespace: ByteString,
        frame_id: u32,
        expression: bool,
        body: ByteString,
        inputs: Vec<HostFrameInput>,
    ) -> Result<HostFrameOutput, VmError> {
        if self.host.is_none() {
            return self
                .inspection_bridge(|host| {
                    host.invoke_frame(&namespace, frame_id, expression, body, inputs)
                })
                .ok_or_else(|| {
                    VmError::HybridUnavailable("nested host frame without active bridge".into())
                })?;
        }

        let mut host = self
            .host
            .take()
            .ok_or_else(|| VmError::HybridUnavailable("host not bootstrapped".into()))?;
        let eng_ptr = self as *mut HybridEngineInner;
        let result = host.with_stub_dispatch(
            Box::new(move |args| {
                let engine = unsafe { &mut *eng_ptr };
                engine.dispatch_stub_reenter(args)
            }),
            |host| host.invoke_frame(&namespace, frame_id, expression, body, inputs),
        );
        self.host = Some(host);
        result
    }

    /// Option B: host obj-command stub re-enters VM for Facade/VmProc.
    #[cfg(feature = "host")]
    fn dispatch_stub_reenter(&mut self, words: &[VmObj]) -> Result<BridgeResult, VmError> {
        #[cfg(feature = "debug")]
        if words
            .first()
            .is_some_and(|w| w.as_string().ok().as_deref() == Some("::tcld::control_variable"))
        {
            return self.control_variable(words);
        }
        #[cfg(feature = "debug")]
        if words
            .first()
            .is_some_and(|w| w.as_string().ok().as_deref() == Some("::tcld::control_enter"))
        {
            return self.validate_control(words);
        }
        #[cfg(feature = "debug")]
        if words
            .first()
            .is_some_and(|w| w.as_string().ok().as_deref() == Some("::tcld::control_cleanup"))
        {
            return self.finish_control(words);
        }
        #[cfg(feature = "debug")]
        if words
            .first()
            .is_some_and(|w| w.as_string().ok().as_deref() == Some("::tcld::coroutine_body"))
        {
            return self.resume_coroutine(words);
        }
        #[cfg(feature = "debug")]
        if self.debug_frame_eval_active
            && self.host_callback_depth == 0
            && !words.first().is_some_and(|word| {
                word.ensure_bytes().is_ok_and(|name| {
                    matches!(
                        name.as_bytes(),
                        b"::tcld::frame_cross" | b"::tcld::callback"
                    )
                })
            })
        {
            return Err(VmError::HostFrameSuspensionUnsupported);
        }
        if words.is_empty() {
            return Err(VmError::Internal("stub re-enter with empty words".into()));
        }
        HostBridge::with_active_bridge(|host| host.enter_vm_from_stub())
            .ok_or_else(|| VmError::Internal("stub re-enter without active bridge".into()))??;

        let outcome = (|| -> Result<InvokeOutcome, VmError> {
            let cmd = words[0].ensure_bytes()?;
            #[cfg(feature = "debug")]
            if cmd.as_bytes() == b"::tcld::callback" {
                return self.invoke_host_callback(words);
            }
            if cmd.as_bytes() == b"::tcld::frame_cross" {
                return self.handle_frame_cross(words);
            }
            let namespace = self
                .frames
                .last()
                .map(|frame| frame.namespace.clone())
                .unwrap_or_else(NamespaceName::root);
            let resolved = self.registry.resolve(&cmd, namespace.as_bytes(), self.mode);
            match resolved {
                CommandRef::Facade(FacadeKind::BcProc) => {
                    tbcload::handle_bcproc_in_namespace(
                        words,
                        &mut self.registry,
                        namespace.as_bytes(),
                    )?;
                    if words.len() >= 2 {
                        let name = words[1].ensure_bytes()?;
                        let qualified = namespace.qualify(&name);
                        let q = std::str::from_utf8(qualified.as_bytes()).map_err(|_| {
                            VmError::HostFrameMarshalUnsupported(
                                "procedure command name is not UTF-8".into(),
                            )
                        })?;
                        if let Some(host) = self.host.as_mut() {
                            host.install_stub_command(q)?;
                        } else {
                            HostBridge::with_active_bridge(|host| host.install_stub_command(q))
                                .ok_or_else(|| {
                                VmError::HybridUnavailable(
                                    "procedure registration without active host bridge".into(),
                                )
                            })??;
                        }
                    }
                    Ok(InvokeOutcome::Value(VmObj::empty()))
                }
                CommandRef::Facade(FacadeKind::BcEval) => {
                    let body = tbcload::handle_bceval_body(words)?;
                    self.run_host_originated_frame(body, FrameKind::Toplevel, None, &[], namespace)
                }
                CommandRef::VmProc(proc) => self.run_host_originated_frame(
                    Arc::clone(&proc.code),
                    FrameKind::VmProc,
                    Some(proc.qname.clone()),
                    &words[1..],
                    NamespaceName::from_proc_qname(&proc.qname),
                ),
                CommandRef::VarNameBuiltin { name } => {
                    let mut ctx = EngineVarCtx { engine: self };
                    let v = builtin::run_var_name_builtin(name.as_bytes(), &words[1..], &mut ctx)?;
                    Ok(InvokeOutcome::Value(v))
                }
                CommandRef::ValueBuiltin { name } => Ok(InvokeOutcome::Value(
                    builtin::run_value_builtin(name.as_bytes(), &words[0], &words[1..])?,
                )),
                CommandRef::WorldHost { name } => self.invoke_world_host_command(words, &name),
                CommandRef::Denied { name } => Err(denied_command(&name, self.mode)),
            }
        })();

        HostBridge::with_active_bridge(|host| host.leave_vm_from_stub());

        match outcome {
            Ok(InvokeOutcome::Value(result)) => Ok(BridgeResult {
                code: 0, // TCL_OK to host
                result,
                error_info: None,
                error_code: None,
                return_options: None,
            }),
            Ok(InvokeOutcome::Raise(completion)) => {
                // Map VM raise into host error/break/continue codes.
                let return_options = Some(completion.options.as_vm_obj());
                let error_info = completion
                    .options
                    .error_info
                    .as_ref()
                    .map(|value| value.as_str_lossy().into_owned());
                let error_code = completion
                    .options
                    .error_code
                    .as_ref()
                    .and_then(|value| value.as_string().ok());
                Ok(BridgeResult {
                    code: completion.code,
                    result: completion.result,
                    error_info,
                    error_code,
                    return_options,
                })
            }
            Ok(InvokeOutcome::FramePushed) => Err(VmError::Internal(
                "host-originated dispatch left a deferred VM frame".into(),
            )),
            Err(e) => Err(e),
        }
    }

    #[cfg(feature = "host")]
    fn handle_frame_cross(&mut self, words: &[VmObj]) -> Result<InvokeOutcome, VmError> {
        let operation = words
            .get(1)
            .ok_or_else(|| VmError::Internal("frame_cross missing operation".into()))?
            .ensure_bytes()?;
        let level = words
            .get(2)
            .ok_or_else(|| VmError::Internal("frame_cross missing level".into()))?
            .ensure_bytes()?;
        match operation.as_bytes() {
            b"upvar" => {
                let other = words
                    .get(3)
                    .ok_or_else(|| VmError::Internal("frame_cross upvar missing variable".into()))?
                    .ensure_bytes()?;
                let local = words
                    .get(4)
                    .ok_or_else(|| VmError::Internal("frame_cross upvar missing local".into()))?
                    .ensure_bytes()?;
                let target_index = self.resolve_vm_level(&level, true)?;
                let target_frame = self.frames[target_index].id;
                let (target_name, part2) = parse_array_name(other.as_bytes());
                let link = VarLink::Vm {
                    target_frame,
                    target_name: target_name.clone(),
                    part2: part2.clone(),
                };
                let (storage, value) = self.snapshot_link_value(&link)?;
                let fi = self.fi();
                if let Some(slot) = self.frames[fi]
                    .compiled_locals
                    .iter_mut()
                    .find(|slot| slot.is_named(&local))
                {
                    slot.value = None;
                    slot.link = Some(link.clone());
                }
                self.frames[fi].local_vars.install_link(local, link);
                Ok(InvokeOutcome::Value(VmObj::List(vec![
                    VmObj::from_str(storage),
                    value,
                ])))
            }
            b"uplevel" => {
                let body = words
                    .get(3)
                    .ok_or_else(|| VmError::Internal("frame_cross uplevel missing body".into()))?
                    .clone();
                let target_index = self.resolve_vm_level(&level, false)?;
                let top = self.fi();
                self.frames.swap(target_index, top);
                let evaluated = self
                    .host_frame_eval(body, false)
                    .and_then(|flow| match flow {
                        Flow::Continue => Ok(InvokeOutcome::Value(self.pop(0, "uplevel result")?)),
                        Flow::Raise(completion) => Ok(InvokeOutcome::Raise(completion)),
                        Flow::JumpPc(_) | Flow::FrameDone(_) => Err(VmError::Internal(
                            "uplevel HostFrame returned scheduler flow".into(),
                        )),
                    });
                self.frames.swap(target_index, top);
                evaluated
            }
            _ => Err(VmError::Internal(format!(
                "unknown frame_cross operation {}",
                operation.as_str_lossy()
            ))),
        }
    }

    fn snapshot_link_value(&mut self, link: &VarLink) -> Result<(&'static str, VmObj), VmError> {
        match link {
            VarLink::Vm {
                target_frame,
                target_name,
                part2,
            } => {
                let frame_index = self
                    .frames
                    .iter()
                    .position(|frame| frame.id == *target_frame)
                    .ok_or(VmError::CrossRuntimeUpvar)?;
                if part2.is_none() {
                    if let Some(VarSlot::Array(map)) =
                        self.frames[frame_index].local_vars.get_slot(target_name)
                    {
                        let mut pairs = Vec::with_capacity(map.len() * 2);
                        for (key, value) in map {
                            pairs.push(VmObj::Bytes(key.clone()));
                            pairs.push(value.clone());
                        }
                        return Ok(("array", VmObj::List(pairs)));
                    }
                    if let Some(nested) = self.frames[frame_index]
                        .compiled_locals
                        .iter()
                        .find(|slot| slot.is_named(target_name))
                        .and_then(|slot| slot.link.clone())
                        .or_else(|| {
                            match self.frames[frame_index].local_vars.get_slot(target_name) {
                                Some(VarSlot::Link(link)) => Some(link.clone()),
                                _ => None,
                            }
                        })
                    {
                        return self.snapshot_link_value(&nested);
                    }
                }
                Ok(("scalar", self.follow_link_get(link)?))
            }
            VarLink::HostVar { name1, name2 } => match self.follow_link_get(link) {
                Ok(value) => Ok(("scalar", value)),
                Err(_) if name2.is_none() && self.mode == RuntimeMode::VmStrict => {
                    Ok(("array", self.world.array_get(name1)))
                }
                Err(_) if name2.is_none() => {
                    let result = self.host_value_result(&[
                        VmObj::from_str("array"),
                        VmObj::from_str("get"),
                        VmObj::Bytes(name1.clone()),
                    ])?;
                    match result {
                        Ok(value) => Ok(("array", value)),
                        Err(completion) => Err(completion_to_error(completion)),
                    }
                }
                Err(error) => Err(error),
            },
        }
    }

    #[cfg(feature = "host")]
    fn run_host_originated_frame(
        &mut self,
        code: Arc<LoadedCode>,
        kind: FrameKind,
        proc_qname: Option<ByteString>,
        args: &[VmObj],
        namespace: NamespaceName,
    ) -> Result<InvokeOutcome, VmError> {
        let boundary_depth = self.frames.len();
        self.push_frame(code, kind, proc_qname, args, namespace, None)?;
        #[cfg(feature = "debug")]
        let allow_pause = self.callbacks_can_pause();
        #[cfg(not(feature = "debug"))]
        let allow_pause = false;
        let outcome = self.eval_loop(boundary_depth, allow_pause);
        if outcome.is_err() {
            self.frames.truncate(boundary_depth);
        }
        match outcome? {
            LoopExit::Done(value) => Ok(InvokeOutcome::Value(value)),
            LoopExit::Completion(completion) => Ok(InvokeOutcome::Raise(completion)),
            #[cfg(feature = "debug")]
            LoopExit::Paused => Err(VmError::Internal(
                "host-originated VM subtree paused despite suppression".into(),
            )),
        }
    }

    #[cfg(feature = "host")]
    fn map_bridge_result(&mut self, br: BridgeResult) -> Result<InvokeOutcome, VmError> {
        #[cfg(feature = "debug")]
        if let Some(frame) = self.frames.last()
            && let Some(completion) = self.coroutine_completions.remove(&frame.id.0)
        {
            return Ok(InvokeOutcome::Raise(completion));
        }
        let parsed_options = br
            .return_options
            .clone()
            .map(ReturnOptions::from_vm_obj)
            .transpose()?;
        match br.code {
            0 => Ok(InvokeOutcome::Value(br.result)), // TCL_OK
            1 => {
                // A return that libtcl cannot unwind through the opaque host
                // boundary surfaces as ERROR/UNEXPECTED_RESULT_CODE.
                let msg = br.result.as_string().unwrap_or_default();
                let ec = br.error_code.as_deref().unwrap_or("");
                if ec
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .windows(2)
                    .any(|w| w == ["UNEXPECTED_RESULT_CODE", "2"])
                    || msg.ends_with("bad code: 2")
                {
                    return Err(VmError::CrossRuntimeReturnLevel);
                }
                Ok(InvokeOutcome::Raise(Completion {
                    code: 1,
                    result: br.result,
                    options: parsed_options.unwrap_or_else(|| ReturnOptions {
                        code: 1,
                        error_info: br.error_info.map(ByteString::from),
                        error_code: br.error_code.map(|value| VmObj::from_str(&value)),
                        ..ReturnOptions::default()
                    }),
                }))
            }
            2 => {
                let options = parsed_options.unwrap_or_else(|| ReturnOptions {
                    level: 1,
                    code: 0,
                    ..ReturnOptions::default()
                });
                Ok(InvokeOutcome::Raise(Completion {
                    code: 2,
                    result: br.result,
                    options,
                }))
            }
            code => Ok(InvokeOutcome::Raise(Completion {
                code,
                result: br.result,
                options: parsed_options.unwrap_or_else(|| ReturnOptions::for_code(code)),
            })),
        }
    }

    // ─── stack / frame helpers ───────────────────────────────────────

    fn fi(&self) -> usize {
        self.frames.len() - 1
    }

    fn scope_index(&self) -> usize {
        self.frames[self.fi()]
            .scope_parent
            .and_then(|id| self.frames.iter().position(|frame| frame.id == id))
            .unwrap_or_else(|| self.fi())
    }

    fn push(&mut self, v: VmObj) {
        let fi = self.fi();
        self.frames[fi].stack.push(v);
    }

    fn pop(&mut self, pc: u64, op: &str) -> Result<VmObj, VmError> {
        let fi = self.fi();
        self.frames[fi]
            .stack
            .pop()
            .ok_or_else(|| VmError::StackUnderflow { pc, op: op.into() })
    }

    fn tos(&self, pc: u64, op: &str) -> Result<&VmObj, VmError> {
        let fi = self.fi();
        self.frames[fi]
            .stack
            .last()
            .ok_or_else(|| VmError::StackUnderflow { pc, op: op.into() })
    }

    fn tcl_scope_chain(&self) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut current = Some(self.scope_index());
        while let Some(index) = current {
            let frame = &self.frames[index];
            if !frame.hidden_scope && !matches!(frame.kind, FrameKind::Dynamic | FrameKind::Source)
            {
                chain.push(index);
            }
            current = frame
                .tcl_parent
                .and_then(|id| self.frames.iter().position(|f| f.id == id));
        }
        chain.reverse();
        chain
    }

    fn resolve_vm_level(
        &self,
        level: &ByteString,
        crossing_is_upvar: bool,
    ) -> Result<usize, VmError> {
        let text = level.as_str_lossy();
        let scopes = self.tcl_scope_chain();
        if let Some(absolute) = text.strip_prefix('#') {
            let index = absolute.parse::<usize>().map_err(|_| bad_level(level))?;
            return scopes.get(index).copied().ok_or_else(|| bad_level(level));
        }
        let relative = text.parse::<usize>().map_err(|_| bad_level(level))?;
        let current = scopes.len().saturating_sub(1);
        current
            .checked_sub(relative)
            .and_then(|index| scopes.get(index).copied())
            .ok_or({
                if crossing_is_upvar {
                    VmError::CrossRuntimeUpvar
                } else {
                    VmError::CrossRuntimeUplevel
                }
            })
    }

    /// Whether a compiled local resolves by name in the scope its frame joins
    /// (a source script's top level) rather than in its own slot. Compiler
    /// temporaries always stay in their slots.
    fn lvt_by_name(&self, fi: usize, idx: usize) -> bool {
        self.frames[fi].scope_parent.is_some()
            && !self.frames[fi]
                .compiled_locals
                .get(idx)
                .is_some_and(|slot| slot.is_temporary())
    }

    fn load_lvt(&mut self, idx: usize) -> Result<VmObj, VmError> {
        let fi = self.fi();
        #[cfg(feature = "debug")]
        let _access = self.frames[fi]
            .compiled_locals
            .get(idx)
            .map(|s| s.name.clone())
            .map(|name| self.variable_access(name, None));
        if self.lvt_by_name(fi, idx) {
            let name = self.frames[fi]
                .compiled_locals
                .get(idx)
                .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?
                .name
                .clone();
            return self.var_get(&name, None);
        }

        let slot = self.frames[fi]
            .compiled_locals
            .get(idx)
            .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?;
        if let Some(link) = slot.link.clone() {
            return self.follow_link_get(&link);
        }
        slot.value.clone().ok_or_else(|| VmError::TclError {
            message: format!(
                "can't read \"{}\": no such variable",
                slot.name.as_str_lossy()
            ),
            error_info: None,
            error_code: None,
        })
    }

    fn store_lvt(&mut self, idx: usize, value: VmObj) -> Result<(), VmError> {
        let fi = self.fi();
        #[cfg(feature = "debug")]
        let _access = self.frames[fi]
            .compiled_locals
            .get(idx)
            .map(|s| s.name.clone())
            .map(|name| self.variable_access(name, None));
        if self.lvt_by_name(fi, idx) {
            let name = self.frames[fi]
                .compiled_locals
                .get(idx)
                .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?
                .name
                .clone();
            return self.var_set(&name, None, value);
        }

        let link = self.frames[fi]
            .compiled_locals
            .get(idx)
            .and_then(|s| s.link.clone());
        if let Some(link) = link {
            return self.follow_link_set(&link, value);
        }
        let slot = self.frames[fi]
            .compiled_locals
            .get(idx)
            .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?;
        if !slot.is_temporary()
            && matches!(
                self.frames[fi].local_vars.get_slot(&slot.name),
                Some(VarSlot::Array(_))
            )
        {
            return Err(variable_is_array(&slot.name));
        }
        let slot = &mut self.frames[fi].compiled_locals[idx];
        slot.value = Some(value.clone());
        // Mirror into name table; a temporary has no name to look up.
        if !slot.is_temporary() {
            let name = slot.name.clone();
            self.frames[fi].local_vars.store_scalar(name, value);
        }
        Ok(())
    }

    fn load_lvt_array(&mut self, idx: usize, key: &ByteString) -> Result<VmObj, VmError> {
        let fi = self.fi();
        #[cfg(feature = "debug")]
        let _access = self.frames[fi]
            .compiled_locals
            .get(idx)
            .map(|s| s.name.clone())
            .map(|name| self.variable_access(name, Some(key.clone())));
        if self.lvt_by_name(fi, idx) {
            let name = self.frames[fi]
                .compiled_locals
                .get(idx)
                .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?
                .name
                .clone();
            return self.var_get(&name, Some(key));
        }

        let (link, name) = {
            let slot = self.frames[fi]
                .compiled_locals
                .get(idx)
                .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?;
            (slot.link.clone(), slot.name.clone())
        };
        if let Some(link) = link {
            return self.follow_link_get(&link_element(&link, key.clone()));
        }
        match self.frames[fi].local_vars.get_slot(&name) {
            Some(crate::var::VarSlot::Array(map)) => {
                map.get(key).cloned().ok_or_else(|| VmError::TclError {
                    message: format!(
                        "can't read \"{}({})\": no such element in array",
                        name.as_str_lossy(),
                        key.as_str_lossy()
                    ),
                    error_info: None,
                    error_code: None,
                })
            }
            Some(_) => Err(VmError::TclError {
                message: format!(
                    "can't read \"{}({})\": variable isn't array",
                    name.as_str_lossy(),
                    key.as_str_lossy()
                ),
                error_info: None,
                error_code: None,
            }),
            None => Err(VmError::TclError {
                message: format!(
                    "can't read \"{}({})\": no such variable",
                    name.as_str_lossy(),
                    key.as_str_lossy()
                ),
                error_info: None,
                error_code: None,
            }),
        }
    }

    fn store_lvt_array(
        &mut self,
        idx: usize,
        key: &ByteString,
        value: VmObj,
    ) -> Result<(), VmError> {
        let fi = self.fi();
        #[cfg(feature = "debug")]
        let _access = self.frames[fi]
            .compiled_locals
            .get(idx)
            .map(|s| s.name.clone())
            .map(|name| self.variable_access(name, Some(key.clone())));
        if self.lvt_by_name(fi, idx) {
            let name = self.frames[fi]
                .compiled_locals
                .get(idx)
                .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?
                .name
                .clone();
            return self.var_set(&name, Some(key), value);
        }

        let slot = self.frames[fi]
            .compiled_locals
            .get(idx)
            .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?;
        if let Some(link) = slot.link.clone() {
            return self.follow_link_set(&link_element(&link, key.clone()), value);
        }
        let name = slot.name.clone();
        self.frames[fi]
            .local_vars
            .store_array_elem(name, key.clone(), value);
        Ok(())
    }

    fn lvt_exists(&self, idx: usize) -> bool {
        let fi = self.fi();
        self.frames[fi]
            .compiled_locals
            .get(idx)
            .map(|s| s.value.is_some() || s.link.is_some())
            .unwrap_or(false)
    }

    fn install_lvt_host_link(
        &mut self,
        idx: usize,
        name1: ByteString,
        name2: Option<ByteString>,
    ) -> Result<(), VmError> {
        let fi = self.fi();
        let slot = self.frames[fi]
            .compiled_locals
            .get_mut(idx)
            .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?;
        slot.value = None;
        slot.link = Some(VarLink::HostVar { name1, name2 });
        Ok(())
    }

    fn install_lvt_vm_link(
        &mut self,
        idx: usize,
        target_frame: FrameId,
        target_name: ByteString,
        part2: Option<ByteString>,
    ) -> Result<(), VmError> {
        let fi = self.fi();
        let slot = self.frames[fi]
            .compiled_locals
            .get_mut(idx)
            .ok_or_else(|| VmError::Internal(format!("LVT index {idx} OOB")))?;
        slot.value = None;
        slot.link = Some(VarLink::Vm {
            target_frame,
            target_name,
            part2,
        });
        Ok(())
    }

    fn follow_link_get(&mut self, link: &VarLink) -> Result<VmObj, VmError> {
        match link {
            VarLink::HostVar { name1, name2 } => self.host_get(name1, name2.as_ref()),
            VarLink::Vm {
                target_frame,
                target_name,
                part2,
            } => {
                let fi = self
                    .frames
                    .iter()
                    .position(|f| f.id == *target_frame)
                    .ok_or(VmError::CrossRuntimeUpvar)?;
                if let Some(nested) = self.frames[fi]
                    .compiled_locals
                    .iter()
                    .find(|s| s.is_named(target_name))
                    .and_then(|s| s.link.clone())
                    .or_else(|| match self.frames[fi].local_vars.get_slot(target_name) {
                        Some(VarSlot::Link(l)) => Some(l.clone()),
                        _ => None,
                    })
                {
                    let nested = part2
                        .as_ref()
                        .map(|p| link_element(&nested, p.clone()))
                        .unwrap_or(nested);
                    return self.follow_link_get(&nested);
                }
                // Read from that frame's local_vars / LVT by name
                if let Some(n2) = part2 {
                    match self.frames[fi].local_vars.get_slot(target_name) {
                        Some(crate::var::VarSlot::Array(map)) => {
                            map.get(n2).cloned().ok_or_else(|| VmError::TclError {
                                message: "no such element".into(),
                                error_info: None,
                                error_code: None,
                            })
                        }
                        _ => self.frames[fi].local_vars.load_scalar_local(target_name),
                    }
                } else {
                    // Try LVT by name
                    if let Some(slot) = self.frames[fi]
                        .compiled_locals
                        .iter()
                        .find(|s| s.is_named(target_name))
                        && let Some(v) = &slot.value
                    {
                        return Ok(v.clone());
                    }
                    self.frames[fi].local_vars.load_scalar_local(target_name)
                }
            }
        }
    }

    fn follow_link_set(&mut self, link: &VarLink, value: VmObj) -> Result<(), VmError> {
        match link {
            VarLink::HostVar { name1, name2 } => self.host_set(name1, name2.as_ref(), value),
            VarLink::Vm {
                target_frame,
                target_name,
                part2,
            } => {
                let fi = self
                    .frames
                    .iter()
                    .position(|f| f.id == *target_frame)
                    .ok_or(VmError::CrossRuntimeUpvar)?;
                if let Some(nested) = self.frames[fi]
                    .compiled_locals
                    .iter()
                    .find(|s| s.is_named(target_name))
                    .and_then(|s| s.link.clone())
                    .or_else(|| match self.frames[fi].local_vars.get_slot(target_name) {
                        Some(VarSlot::Link(l)) => Some(l.clone()),
                        _ => None,
                    })
                {
                    let nested = part2
                        .as_ref()
                        .map(|p| link_element(&nested, p.clone()))
                        .unwrap_or(nested);
                    return self.follow_link_set(&nested, value);
                }
                if let Some(n2) = part2 {
                    self.frames[fi].local_vars.store_array_elem(
                        target_name.clone(),
                        n2.clone(),
                        value,
                    );
                } else {
                    // Update LVT if present
                    if let Some(slot) = self.frames[fi]
                        .compiled_locals
                        .iter_mut()
                        .find(|s| s.is_named(target_name))
                    {
                        slot.value = Some(value.clone());
                    }
                    self.frames[fi]
                        .local_vars
                        .store_scalar(target_name.clone(), value);
                }
                Ok(())
            }
        }
    }

    fn follow_link_unset(&mut self, link: &VarLink) -> Result<(), VmError> {
        match link {
            VarLink::HostVar { name1, name2 } => self.host_unset(name1, name2.as_ref()),
            VarLink::Vm {
                target_frame,
                target_name,
                part2,
            } => {
                let frame_index = self
                    .frames
                    .iter()
                    .position(|frame| frame.id == *target_frame)
                    .ok_or(VmError::CrossRuntimeUpvar)?;
                if let Some(nested) = self.frames[frame_index]
                    .compiled_locals
                    .iter()
                    .find(|s| s.is_named(target_name))
                    .and_then(|s| s.link.clone())
                    .or_else(
                        || match self.frames[frame_index].local_vars.get_slot(target_name) {
                            Some(VarSlot::Link(l)) => Some(l.clone()),
                            _ => None,
                        },
                    )
                {
                    let nested = part2
                        .as_ref()
                        .map(|p| link_element(&nested, p.clone()))
                        .unwrap_or(nested);
                    return self.follow_link_unset(&nested);
                }
                if let Some(key) = part2 {
                    let removed = match self.frames[frame_index]
                        .local_vars
                        .get_slot_mut(target_name)
                    {
                        Some(crate::var::VarSlot::Array(map)) => map.remove(key).is_some(),
                        _ => false,
                    };
                    if !removed {
                        return Err(no_such_unset(target_name, Some(key)));
                    }
                } else {
                    let mut existed = self.frames[frame_index]
                        .local_vars
                        .remove(target_name)
                        .is_some();
                    if let Some(slot) = self.frames[frame_index]
                        .compiled_locals
                        .iter_mut()
                        .find(|slot| slot.is_named(target_name))
                    {
                        existed |= slot.value.is_some() || slot.link.is_some();
                        slot.value = None;
                        slot.link = None;
                    }
                    if !existed {
                        return Err(no_such_unset(target_name, None));
                    }
                }
                Ok(())
            }
        }
    }

    /// Code compiled from source, which resolves variables as Tcl 9 does:
    /// an unqualified name never falls back from a namespace to the global
    /// namespace. Legacy bytecode keeps that fallback.
    fn is_source_scope(&self, scope: usize) -> bool {
        #[cfg(feature = "debug")]
        {
            self.source_images.contains_key(&self.frames[scope].code.id)
        }
        #[cfg(not(feature = "debug"))]
        {
            let _ = scope;
            false
        }
    }

    /// A procedure or lambda compiled from source: like Tcl, it sees only its
    /// own variables and those it links. Other scopes, and legacy bytecode
    /// procedures, also fall back to namespace and global variables.
    fn is_source_procedure_scope(&self, scope: usize) -> bool {
        #[cfg(feature = "debug")]
        {
            matches!(
                self.frames[scope].kind,
                FrameKind::VmProc | FrameKind::Lambda
            ) && self.source_images.contains_key(&self.frames[scope].code.id)
        }
        #[cfg(not(feature = "debug"))]
        {
            let _ = scope;
            false
        }
    }

    fn var_get(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
    ) -> Result<VmObj, VmError> {
        #[cfg(feature = "debug")]
        let _access = self.variable_access(name1.clone(), name2.cloned());
        let fi = self.scope_index();
        #[cfg(feature = "debug")]
        if self.source_images.contains_key(&self.frames[fi].code.id)
            && name1.as_str_lossy().contains("::")
        {
            let qualified = self.frames[fi].namespace.qualify(name1);
            return self.host_get(&qualified, name2);
        }

        // LVT by name
        if name2.is_none()
            && let Some((idx, slot)) = self.frames[fi]
                .compiled_locals
                .iter()
                .enumerate()
                .find(|(_, s)| s.is_named(name1))
        {
            let _ = idx;
            if let Some(link) = slot.link.clone() {
                return self.follow_link_get(&link);
            }
            if let Some(v) = &slot.value {
                return Ok(v.clone());
            }
        }
        // local_vars
        if let Some(slot) = self.frames[fi].local_vars.get_slot(name1).cloned() {
            match slot {
                crate::var::VarSlot::Scalar(v) if name2.is_none() => return Ok(v),
                crate::var::VarSlot::Scalar(_) => {
                    return Err(VmError::TclError {
                        message: format!(
                            "can't read \"{}({})\": variable isn't array",
                            name1.as_str_lossy(),
                            name2.unwrap().as_str_lossy()
                        ),
                        error_info: None,
                        error_code: None,
                    });
                }
                crate::var::VarSlot::Link(link) => {
                    let link = name2
                        .map(|key| link_element(&link, key.clone()))
                        .unwrap_or(link);
                    return self.follow_link_get(&link);
                }
                crate::var::VarSlot::Array(map) => {
                    if let Some(k) = name2 {
                        return map.get(k).cloned().ok_or_else(|| VmError::TclError {
                            message: format!(
                                "can't read \"{}({})\": no such element in array",
                                name1.as_str_lossy(),
                                k.as_str_lossy()
                            ),
                            error_info: None,
                            error_code: None,
                        });
                    }
                    return Err(VmError::TclError {
                        message: format!(
                            "can't read \"{}\": variable is array",
                            name1.as_str_lossy()
                        ),
                        error_info: None,
                        error_code: None,
                    });
                }
                crate::var::VarSlot::Undefined => {}
            }
        }
        if !self.is_source_procedure_scope(fi) {
            let namespace = self.frames[fi].namespace.clone();
            let qualified = namespace.qualify(name1);
            if let Ok(value) = self.host_get(&qualified, name2) {
                return Ok(value);
            }
            if namespace.as_bytes().as_bytes() != b"::" && !self.is_source_scope(fi) {
                let global = NamespaceName::root().qualify(name1);
                if let Ok(value) = self.host_get(&global, name2) {
                    return Ok(value);
                }
            }
        }
        let name = match name2 {
            Some(element) => format!("{}({})", name1.as_str_lossy(), element.as_str_lossy()),
            None => name1.as_str_lossy().into_owned(),
        };
        Err(VmError::TclError {
            message: format!("can't read \"{name}\": no such variable"),
            error_info: None,
            error_code: None,
        })
    }

    fn var_set(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
        value: VmObj,
    ) -> Result<(), VmError> {
        #[cfg(feature = "debug")]
        let _access = self.variable_access(name1.clone(), name2.cloned());
        let fi = self.scope_index();
        #[cfg(feature = "debug")]
        if self.source_images.contains_key(&self.frames[fi].code.id)
            && name1.as_str_lossy().contains("::")
        {
            let qualified = self.frames[fi].namespace.qualify(name1);
            self.install_host_link_named(name1, qualified.clone(), None)?;
            return self.host_set(&qualified, name2, value);
        }

        #[cfg(feature = "debug")]
        if matches!(
            self.frames[fi].kind,
            FrameKind::Toplevel | FrameKind::Namespace
        ) && self.source_images.contains_key(&self.frames[fi].code.id)
        {
            let qualified = self.frames[fi].namespace.qualify(name1);
            if !self.frames[fi]
                .compiled_locals
                .iter()
                .any(|s| s.is_named(name1) && s.link.is_some())
                && !matches!(
                    self.frames[fi].local_vars.get_slot(name1),
                    Some(VarSlot::Link(_))
                )
            {
                self.install_host_link_named(name1, qualified, None)?;
            }
        }
        let local_array = name2.is_none()
            && matches!(
                self.frames[fi].local_vars.get_slot(name1),
                Some(VarSlot::Array(_))
            );
        if let Some(slot) = self.frames[fi]
            .compiled_locals
            .iter_mut()
            .find(|s| s.is_named(name1))
        {
            if let Some(link) = slot.link.clone() {
                let link = name2
                    .map(|key| link_element(&link, key.clone()))
                    .unwrap_or(link);
                return self.follow_link_set(&link, value);
            }
            if local_array {
                return Err(variable_is_array(name1));
            }
            if name2.is_none() {
                slot.value = Some(value.clone());
                self.frames[fi]
                    .local_vars
                    .store_scalar(name1.clone(), value);
                return Ok(());
            }
        }
        if let Some(crate::var::VarSlot::Link(link)) =
            self.frames[fi].local_vars.get_slot(name1).cloned()
        {
            let link = name2
                .map(|key| link_element(&link, key.clone()))
                .unwrap_or(link);
            return self.follow_link_set(&link, value);
        }
        if let Some(k) = name2 {
            match self.frames[fi].local_vars.get_slot_mut(name1) {
                Some(crate::var::VarSlot::Array(map)) => {
                    map.insert(k.clone(), value);
                }
                Some(crate::var::VarSlot::Scalar(_)) => {
                    return Err(crate::args::tcl_error(
                        format!(
                            "can't set \"{}({})\": variable isn't array",
                            name1.as_str_lossy(),
                            k.as_str_lossy()
                        ),
                        &["TCL", "LOOKUP", "VARNAME", &name1.as_str_lossy()],
                    ));
                }
                _ => self.frames[fi]
                    .local_vars
                    .store_array_elem(name1.clone(), k.clone(), value),
            }
        } else if local_array {
            return Err(variable_is_array(name1));
        } else {
            self.frames[fi]
                .local_vars
                .store_scalar(name1.clone(), value);
        }
        Ok(())
    }

    fn var_unset(&mut self, name1: &ByteString, name2: Option<&ByteString>) -> Result<(), VmError> {
        #[cfg(feature = "debug")]
        let _access = self.variable_access(name1.clone(), name2.cloned());
        let fi = self.scope_index();
        #[cfg(feature = "debug")]
        if self.source_images.contains_key(&self.frames[fi].code.id)
            && name1.as_str_lossy().contains("::")
        {
            let qualified = self.frames[fi].namespace.qualify(name1);
            return self.host_unset(&qualified, name2);
        }

        let link = self.frames[fi]
            .compiled_locals
            .iter()
            .find(|slot| slot.is_named(name1))
            .and_then(|slot| slot.link.clone())
            .or_else(|| match self.frames[fi].local_vars.get_slot(name1) {
                Some(crate::var::VarSlot::Link(link)) => Some(link.clone()),
                _ => None,
            });
        if let Some(link) = link {
            if name2.is_none() {
                return self.follow_link_unset(&link);
            }
            let target = match link {
                VarLink::HostVar { name1, .. } => VarLink::HostVar {
                    name1,
                    name2: name2.cloned(),
                },
                VarLink::Vm {
                    target_frame,
                    target_name,
                    ..
                } => VarLink::Vm {
                    target_frame,
                    target_name,
                    part2: name2.cloned(),
                },
            };
            return self.follow_link_unset(&target);
        }
        if let Some(k) = name2 {
            if let Some(crate::var::VarSlot::Array(map)) =
                self.frames[fi].local_vars.get_slot_mut(name1)
                && map.remove(k).is_some()
            {
                return Ok(());
            }
            return Err(no_such_unset(name1, Some(k)));
        }
        let mut existed = self.frames[fi].local_vars.remove(name1).is_some();
        if let Some(slot) = self.frames[fi]
            .compiled_locals
            .iter_mut()
            .find(|s| s.is_named(name1))
        {
            existed |= slot.value.is_some() || slot.link.is_some();
            slot.value = None;
            slot.link = None;
        }
        if existed {
            return Ok(());
        }
        if !self.is_source_procedure_scope(fi) {
            let namespace = self.frames[fi].namespace.clone();
            let qualified = namespace.qualify(name1);
            if self.host_unset(&qualified, None).is_ok() {
                return Ok(());
            }
            if namespace.as_bytes().as_bytes() != b"::" && !self.is_source_scope(fi) {
                let global = NamespaceName::root().qualify(name1);
                if self.host_unset(&global, None).is_ok() {
                    return Ok(());
                }
            }
        }
        Err(no_such_unset(name1, None))
    }

    fn make_array(&mut self, name: &ByteString) -> Result<(), VmError> {
        use crate::var::VarSlot;
        let fi = self.scope_index();
        #[cfg(feature = "debug")]
        if matches!(
            self.frames[fi].kind,
            FrameKind::Toplevel | FrameKind::Namespace
        ) && self.source_images.contains_key(&self.frames[fi].code.id)
            && !self.frames[fi]
                .compiled_locals
                .iter()
                .any(|s| s.is_named(name) && s.link.is_some())
            && !matches!(
                self.frames[fi].local_vars.get_slot(name),
                Some(VarSlot::Link(_))
            )
        {
            let qualified = self.frames[fi].namespace.qualify(name);
            self.install_host_link_named(name, qualified, None)?;
        }

        let link = self.frames[fi]
            .compiled_locals
            .iter()
            .find(|slot| slot.is_named(name))
            .and_then(|slot| slot.link.clone())
            .or_else(|| match self.frames[fi].local_vars.get_slot(name) {
                Some(VarSlot::Link(link)) => Some(link.clone()),
                _ => None,
            });
        if let Some(link) = link {
            return self.make_link_array(&link);
        }
        let compiled_value = self.frames[fi]
            .compiled_locals
            .iter()
            .find(|slot| slot.is_named(name))
            .and_then(|slot| slot.value.as_ref())
            .is_some();
        match self.frames[fi].local_vars.get_slot(name) {
            Some(VarSlot::Array(_)) => return Ok(()),
            Some(VarSlot::Scalar(_)) => {
                return Err(VmError::TclError {
                    message: format!(
                        "can't array set \"{}\": variable isn't array",
                        name.as_str_lossy()
                    ),
                    error_info: None,
                    error_code: Some("TCL WRITE VARNAME".into()),
                });
            }
            _ if compiled_value => {
                return Err(VmError::TclError {
                    message: format!(
                        "can't array set \"{}\": variable isn't array",
                        name.as_str_lossy()
                    ),
                    error_info: None,
                    error_code: Some("TCL WRITE VARNAME".into()),
                });
            }
            _ => {}
        }
        self.frames[fi]
            .local_vars
            .set_slot(name.clone(), VarSlot::Array(Default::default()));
        if let Some(slot) = self.frames[fi]
            .compiled_locals
            .iter_mut()
            .find(|slot| slot.is_named(name))
        {
            slot.value = None;
        }
        Ok(())
    }

    fn array_exists_named(&mut self, name: &ByteString) -> Result<bool, VmError> {
        use crate::var::VarSlot;
        let fi = self.scope_index();
        let link = self.frames[fi]
            .compiled_locals
            .iter()
            .find(|slot| slot.is_named(name))
            .and_then(|slot| slot.link.clone())
            .or_else(|| match self.frames[fi].local_vars.get_slot(name) {
                Some(VarSlot::Link(link)) => Some(link.clone()),
                _ => None,
            });
        if let Some(link) = link {
            return self.array_exists_link(&link);
        }
        match self.frames[fi].local_vars.get_slot(name) {
            Some(VarSlot::Array(_)) => return Ok(true),
            Some(_) => return Ok(false),
            None => {}
        }
        if self.is_source_procedure_scope(fi) {
            return Ok(false);
        }
        let namespace = self.frames[fi].namespace.clone();
        let mut candidates = vec![namespace.qualify(name)];
        if !self.is_source_scope(fi) {
            candidates.push(NamespaceName::root().qualify(name));
        }
        if self.mode == RuntimeMode::VmStrict {
            return Ok(candidates
                .iter()
                .any(|qualified| self.world.array_exists(qualified)));
        }
        if self.mode == RuntimeMode::Hybrid {
            for qualified in candidates {
                match self.host_value_result(&[
                    VmObj::from_str("array"),
                    VmObj::from_str("exists"),
                    VmObj::Bytes(qualified),
                ])? {
                    Ok(value) if value.is_true()? => return Ok(true),
                    Ok(_) => {}
                    Err(completion) => return Err(completion_to_error(completion)),
                }
            }
        }
        Ok(false)
    }

    fn array_exists_link(&mut self, link: &VarLink) -> Result<bool, VmError> {
        use crate::var::VarSlot;
        match link {
            VarLink::HostVar { name1, name2: None } if self.mode == RuntimeMode::VmStrict => {
                Ok(self.world.array_exists(name1))
            }
            VarLink::HostVar { name1, name2: None } => match self.host_value_result(&[
                VmObj::from_str("array"),
                VmObj::from_str("exists"),
                VmObj::Bytes(name1.clone()),
            ])? {
                Ok(value) => value.is_true(),
                Err(completion) => Err(completion_to_error(completion)),
            },
            VarLink::HostVar { name2: Some(_), .. } => Ok(false),
            VarLink::Vm {
                target_frame,
                target_name,
                part2: None,
            } => {
                let frame = self
                    .frames
                    .iter()
                    .find(|frame| frame.id == *target_frame)
                    .ok_or(VmError::CrossRuntimeUpvar)?;
                Ok(matches!(
                    frame.local_vars.get_slot(target_name),
                    Some(VarSlot::Array(_))
                ))
            }
            VarLink::Vm { part2: Some(_), .. } => Ok(false),
        }
    }

    fn make_link_array(&mut self, link: &VarLink) -> Result<(), VmError> {
        use crate::var::VarSlot;
        match link {
            VarLink::HostVar { name2: Some(_), .. } => Err(VmError::TclError {
                message: "can't array set array element: variable isn't array".into(),
                error_info: None,
                error_code: Some("TCL WRITE VARNAME".into()),
            }),
            VarLink::HostVar { name1, name2: None } if self.mode == RuntimeMode::VmStrict => {
                if self.world.array_exists(name1) {
                    return Ok(());
                }
                if self.world.get(name1, None).is_ok() {
                    return Err(VmError::TclError {
                        message: format!(
                            "can't array set \"{}\": variable isn't array",
                            name1.as_str_lossy()
                        ),
                        error_info: None,
                        error_code: Some("TCL WRITE VARNAME".into()),
                    });
                }
                self.world
                    .replace_array(name1, std::collections::HashMap::new());
                Ok(())
            }
            VarLink::HostVar { name1, name2: None } => {
                let exists = self.host_value_result(&[
                    VmObj::from_str("array"),
                    VmObj::from_str("exists"),
                    VmObj::Bytes(name1.clone()),
                ])?;
                if matches!(exists, Ok(ref value) if value.is_true().unwrap_or(false)) {
                    return Ok(());
                }
                if self.host_get(name1, None).is_ok() {
                    return Err(VmError::TclError {
                        message: format!(
                            "can't array set \"{}\": variable isn't array",
                            name1.as_str_lossy()
                        ),
                        error_info: None,
                        error_code: Some("TCL WRITE VARNAME".into()),
                    });
                }
                match self.host_value_result(&[
                    VmObj::from_str("array"),
                    VmObj::from_str("set"),
                    VmObj::Bytes(name1.clone()),
                    VmObj::List(Vec::new()),
                ])? {
                    Ok(_) => Ok(()),
                    Err(completion) => Err(completion_to_error(completion)),
                }
            }
            VarLink::Vm { part2: Some(_), .. } => Err(VmError::TclError {
                message: "can't array set array element: variable isn't array".into(),
                error_info: None,
                error_code: Some("TCL WRITE VARNAME".into()),
            }),
            VarLink::Vm {
                target_frame,
                target_name,
                part2: None,
            } => {
                let frame_index = self
                    .frames
                    .iter()
                    .position(|frame| frame.id == *target_frame)
                    .ok_or(VmError::CrossRuntimeUpvar)?;
                match self.frames[frame_index].local_vars.get_slot(target_name) {
                    Some(VarSlot::Array(_)) => Ok(()),
                    Some(VarSlot::Scalar(_)) => Err(VmError::TclError {
                        message: format!(
                            "can't array set \"{}\": variable isn't array",
                            target_name.as_str_lossy()
                        ),
                        error_info: None,
                        error_code: Some("TCL WRITE VARNAME".into()),
                    }),
                    _ => {
                        self.frames[frame_index]
                            .local_vars
                            .set_slot(target_name.clone(), VarSlot::Array(Default::default()));
                        Ok(())
                    }
                }
            }
        }
    }

    #[cfg(feature = "host")]
    fn inspection_bridge<R>(&self, f: impl FnOnce(&mut HostBridge) -> R) -> Option<R> {
        #[cfg(feature = "debug")]
        {
            self.with_suspended_bridge(f)
        }
        #[cfg(not(feature = "debug"))]
        {
            HostBridge::with_active_bridge(f)
        }
    }

    fn host_get(&self, name1: &ByteString, name2: Option<&ByteString>) -> Result<VmObj, VmError> {
        if self.mode == RuntimeMode::VmStrict {
            return self
                .world
                .get(name1, name2)
                .map_err(|error| self.variable_error(name1, error));
        }
        #[cfg(feature = "host")]
        {
            let name = if let Some(part) = name2 {
                format!("{}({})", name1.as_str_lossy(), part.as_str_lossy())
            } else {
                name1.as_str_lossy().into_owned()
            };
            let invoke = |host: &mut HostBridge| {
                host.invoke_objv(&[VmObj::from_str("set"), VmObj::from_str(&name)])
            };
            let result = if let Some(host) = self.host.as_ref() {
                // Outside execution there is no VM dispatcher; retain the
                // ordinary bridge read for bootstrap and raw bytecode callers.
                return host
                    .get_var_bytes(name1.as_bytes(), name2.map(|b| b.as_bytes()))
                    .map(VmObj::from_bytes)
                    .ok_or_else(|| VmError::TclError {
                        message: format!("can't read \"{name}\": no such variable"),
                        error_info: None,
                        error_code: None,
                    });
            } else {
                self.inspection_bridge(invoke)
                    .ok_or_else(|| VmError::HybridUnavailable("no active host".into()))??
            };
            if result.code == 0 {
                Ok(result.result)
            } else {
                Err(self.variable_error(
                    name1,
                    VmError::TclError {
                        message: result.result.as_string()?,
                        error_info: result.error_info,
                        error_code: result.error_code,
                    },
                ))
            }
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = (name1, name2);
            Err(VmError::HybridUnavailable("no host".into()))
        }
    }
    /// Name a variable in an error message as the program wrote it, not by
    /// the qualified name its link resolved to.
    fn variable_error(&self, actual: &ByteString, error: VmError) -> VmError {
        #[cfg(feature = "debug")]
        if let VmError::TclError {
            message,
            error_info,
            error_code,
        } = error
        {
            let fi = self.scope_index();
            let local = self.frames[fi].compiled_locals.iter().find(|s| matches!(&s.link, Some(VarLink::HostVar {name1,..}) if name1 == actual)).map(|s| &s.name)
                .or_else(|| self.frames[fi].local_vars.iter_slots().find(|(_,s)| matches!(s, VarSlot::Link(VarLink::HostVar {name1,..}) if name1 == actual)).map(|(n,_)| n));
            if let Some(local) = local {
                let error_code = error_code.map(|code| {
                    use crate::list_codec::TclListCodec;
                    if let Ok(mut words) = TclListCodec::parse(code.as_bytes())
                        && words.len() == 4
                        && words[0] == b"TCL"
                        && words[1] == b"LOOKUP"
                        && words[2] == b"VARNAME"
                        && words[3] == actual.as_bytes()
                    {
                        words[3] = local.as_bytes().to_vec();
                        String::from_utf8_lossy(&TclListCodec::format(
                            words.iter().map(Vec::as_slice),
                        ))
                        .into_owned()
                    } else {
                        code
                    }
                });
                return VmError::TclError {
                    message: message.replace(
                        actual.as_str_lossy().as_ref(),
                        local.as_str_lossy().as_ref(),
                    ),
                    error_info,
                    error_code,
                };
            }
            return VmError::TclError {
                message,
                error_info,
                error_code,
            };
        }
        error
    }

    fn host_set(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
        value: VmObj,
    ) -> Result<(), VmError> {
        if self.mode == RuntimeMode::VmStrict {
            self.last_host_write = None;
            return self
                .world
                .set(name1, name2, value)
                .map_err(|error| self.variable_error(name1, error));
        }
        #[cfg(feature = "host")]
        {
            let bytes = value.ensure_bytes()?;
            let n2 = name2.map(|b| b.as_bytes());
            let result = if let Some(host) = self.host.as_ref() {
                host.set_var_result_bytes(name1.as_bytes(), n2, bytes.as_bytes())
            } else {
                self.inspection_bridge(|host| {
                    host.set_var_result_bytes(name1.as_bytes(), n2, bytes.as_bytes())
                })
                .ok_or_else(|| VmError::HybridUnavailable("no active host".into()))?
            };
            let result = result.map_err(|e| self.variable_error(name1, e))?;
            self.last_host_write = Some(result);
            Ok(())
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = (name1, name2, value);
            Err(VmError::HybridUnavailable("no host".into()))
        }
    }

    fn host_unset(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
    ) -> Result<(), VmError> {
        if self.mode == RuntimeMode::VmStrict {
            return self
                .world
                .unset(name1, name2)
                .map_err(|error| self.variable_error(name1, error));
        }
        let name = if let Some(name2) = name2 {
            let mut bytes = name1.as_bytes().to_vec();
            bytes.push(b'(');
            bytes.extend_from_slice(name2.as_bytes());
            bytes.push(b')');
            VmObj::from_bytes(bytes)
        } else {
            VmObj::Bytes(name1.clone())
        };
        match self.host_value_result(&[VmObj::from_str("unset"), name])? {
            Ok(_) => Ok(()),
            Err(completion) => Err(completion_to_error(completion)),
        }
    }

    fn install_host_link_named(
        &mut self,
        local_name: &ByteString,
        name1: ByteString,
        name2: Option<ByteString>,
    ) -> Result<(), VmError> {
        let fi = self.scope_index();
        // Prefer LVT slot with same name
        if let Some(slot) = self.frames[fi]
            .compiled_locals
            .iter_mut()
            .find(|s| s.is_named(local_name))
        {
            slot.value = None;
            slot.link = Some(VarLink::HostVar {
                name1: name1.clone(),
                name2: name2.clone(),
            });
        }
        self.frames[fi]
            .local_vars
            .install_link(local_name.clone(), VarLink::HostVar { name1, name2 });
        Ok(())
    }
}

/// Adapter so builtins can call into the engine.
struct EngineVarCtx<'a> {
    engine: &'a mut HybridEngineInner,
}

impl VarNameCtx for EngineVarCtx<'_> {
    fn get_var(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
    ) -> Result<VmObj, VmError> {
        self.engine.var_get(name1, name2)
    }

    fn set_var(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
        value: VmObj,
    ) -> Result<(), VmError> {
        self.engine.var_set(name1, name2, value)
    }

    fn set_var_value(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
        value: VmObj,
    ) -> Result<VmObj, VmError> {
        self.engine.last_host_write = None;
        self.engine.var_set(name1, name2, value.clone())?;
        Ok(self.engine.last_host_write.take().unwrap_or(value))
    }

    fn unset_var(&mut self, name1: &ByteString, name2: Option<&ByteString>) -> Result<(), VmError> {
        self.engine.var_unset(name1, name2)
    }

    fn install_host_link(
        &mut self,
        local_name: &ByteString,
        name1: ByteString,
        name2: Option<ByteString>,
    ) -> Result<(), VmError> {
        self.engine
            .install_host_link_named(local_name, name1, name2)
    }

    fn current_ns(&self) -> &ByteString {
        self.engine.frames[self.engine.fi()].namespace.as_bytes()
    }
}

fn find_loop_range(exceptions: &[ExceptionRange], pc: i32) -> Option<&ExceptionRange> {
    exceptions
        .iter()
        .filter(|e| e.kind == ExceptionType::Loop && e.code_range.contains(&pc))
        .max_by_key(|e| e.nesting_level)
}

fn decode_immediate_index(encoded: i32, length: usize) -> Option<usize> {
    if encoded == -1 {
        return None;
    }
    let decoded = if encoded > -2 {
        encoded as i64
    } else {
        length as i64 - 1 + (encoded as i64 + 2)
    };
    usize::try_from(decoded).ok()
}

fn parse_list_index(index: &VmObj, length: usize) -> Result<Option<usize>, VmError> {
    let bytes = index.ensure_bytes()?;
    let text = bytes.as_str_lossy();
    let decoded = if text == "end" {
        length.checked_sub(1)
    } else if let Some(delta) = text.strip_prefix("end-") {
        let delta = parse_tcl_int(delta).ok_or_else(|| VmError::TclError {
            message: format!("bad index \"{text}\""),
            error_info: None,
            error_code: Some("TCL VALUE INDEX".into()),
        })?;
        length.checked_sub(1 + delta.max(0) as usize)
    } else if let Some(delta) = text.strip_prefix("end+") {
        let delta = parse_tcl_int(delta).ok_or_else(|| VmError::TclError {
            message: format!("bad index \"{text}\""),
            error_info: None,
            error_code: Some("TCL VALUE INDEX".into()),
        })?;
        length
            .checked_sub(1)
            .and_then(|end| end.checked_add(delta.max(0) as usize))
    } else {
        let value = parse_tcl_int(&text).ok_or_else(|| VmError::TclError {
            message: format!("bad index \"{text}\""),
            error_info: None,
            error_code: Some("TCL VALUE INDEX".into()),
        })?;
        usize::try_from(value).ok()
    };
    Ok(decoded)
}

fn lset_nested(list: VmObj, indices: &[VmObj], replacement: VmObj) -> Result<VmObj, VmError> {
    if indices.is_empty() {
        return Ok(replacement);
    }
    let mut elements = list.as_list_vec()?;
    let index =
        parse_list_index(&indices[0], elements.len())?.ok_or_else(|| VmError::TclError {
            message: "list index out of range".into(),
            error_info: None,
            error_code: Some("TCL OPERATION LSET BADINDEX".into()),
        })?;
    if index >= elements.len() {
        return Err(VmError::TclError {
            message: "list index out of range".into(),
            error_info: None,
            error_code: Some("TCL OPERATION LSET BADINDEX".into()),
        });
    }
    elements[index] = lset_nested(elements[index].clone(), &indices[1..], replacement)?;
    Ok(VmObj::List(elements))
}

fn source_offset_for_pc(bytecode: &tcld_core::ByteCode, pc: u32) -> Option<usize> {
    let source_ranges = bytecode.cmd_source_ranges.as_ref()?;
    bytecode
        .cmd_code_ranges
        .iter()
        .zip(source_ranges)
        .filter(|(code, _)| code.contains(&(pc as i32)))
        .min_by_key(|(code, _)| code.end.saturating_sub(code.start))
        .and_then(|(_, source)| usize::try_from(source.start).ok())
}

fn no_such_unset(name1: &ByteString, name2: Option<&ByteString>) -> VmError {
    let name = name2
        .map(|key| format!("{}({})", name1.as_str_lossy(), key.as_str_lossy()))
        .unwrap_or_else(|| name1.as_str_lossy().into_owned());
    VmError::TclError {
        message: format!("can't unset \"{name}\": no such variable"),
        error_info: None,
        error_code: Some("TCL LOOKUP VARNAME".into()),
    }
}

/// Tcl's error for a scalar write to an array variable (`TclPtrSetVarIdx`
/// reports every failed write as `TCL WRITE VARNAME`).
fn variable_is_array(name: &ByteString) -> VmError {
    VmError::TclError {
        message: format!("can't set \"{}\": variable is array", name.as_str_lossy()),
        error_info: None,
        error_code: Some("TCL WRITE VARNAME".into()),
    }
}

/// A command vm-strict cannot run: a Tcl host command is a policy denial;
/// any other name is simply not a command, as in Tcl.
fn denied_command(name: &ByteString, mode: RuntimeMode) -> VmError {
    if tcld_compile::is_tcl_builtin_command(&name.as_str_lossy()) {
        return VmError::PolicyDenied {
            action: "world_host",
            detail: format!(
                "command `{}` denied in {}",
                name.as_str_lossy(),
                mode.as_str()
            ),
        };
    }
    VmError::TclError {
        message: format!("invalid command name \"{}\"", name.as_str_lossy()),
        error_info: None,
        error_code: Some(
            String::from_utf8_lossy(&crate::list_codec::TclListCodec::format([
                b"TCL".as_slice(),
                b"LOOKUP",
                b"COMMAND",
                name.as_bytes(),
            ]))
            .into_owned(),
        ),
    }
}

fn is_missing_tcl_value(error: &VmError) -> bool {
    matches!(
        error,
        VmError::TclError { message, .. }
            if message.contains("no such variable") || message.contains("no such element")
    )
}

fn bad_level(level: &ByteString) -> VmError {
    VmError::TclError {
        message: format!("bad level \"{}\"", level.as_str_lossy()),
        error_info: None,
        error_code: Some("TCL LOOKUP LEVEL".into()),
    }
}

fn host_link_target(link: &VarLink) -> Option<ByteString> {
    let VarLink::HostVar { name1, name2 } = link else {
        return None;
    };
    if let Some(name2) = name2 {
        let mut bytes = name1.as_bytes().to_vec();
        bytes.push(b'(');
        bytes.extend_from_slice(name2.as_bytes());
        bytes.push(b')');
        Some(ByteString::from_bytes(bytes))
    } else {
        Some(name1.clone())
    }
}

#[cfg(feature = "host")]
fn typed_host_frame_storage(storage: &str, value: VmObj) -> Result<HostFrameStorage, VmError> {
    if storage == "array" {
        let words = value.as_list_vec()?;
        if words.len() % 2 != 0 {
            return Err(VmError::HostFrameMarshalUnsupported(
                "array snapshot contains an odd number of words".into(),
            ));
        }
        let mut entries = Vec::with_capacity(words.len() / 2);
        for pair in words.chunks_exact(2) {
            entries.push((pair[0].ensure_bytes()?, pair[1].ensure_bytes()?));
        }
        Ok(HostFrameStorage::Array(entries))
    } else {
        Ok(HostFrameStorage::Scalar(value.ensure_bytes()?))
    }
}

fn link_element(link: &VarLink, key: ByteString) -> VarLink {
    match link {
        VarLink::HostVar { name1, .. } => VarLink::HostVar {
            name1: name1.clone(),
            name2: Some(key),
        },
        VarLink::Vm {
            target_frame,
            target_name,
            ..
        } => VarLink::Vm {
            target_frame: *target_frame,
            target_name: target_name.clone(),
            part2: Some(key),
        },
    }
}

fn completion_to_error(completion: Completion) -> VmError {
    if completion.code == 1 {
        VmError::TclError {
            message: completion
                .result
                .as_string()
                .unwrap_or_else(|_| "error".into()),
            error_info: completion
                .options
                .error_info
                .map(|value| value.as_str_lossy().into_owned()),
            error_code: completion
                .options
                .error_code
                .and_then(|value| value.as_string().ok()),
        }
    } else if completion.code == 3 {
        VmError::BreakOutsideLoop
    } else if completion.code == 4 {
        VmError::ContinueOutsideLoop
    } else {
        VmError::TclError {
            message: completion.result.as_string().unwrap_or_default(),
            error_info: completion
                .options
                .error_info
                .map(|value| value.as_str_lossy().into_owned()),
            error_code: completion
                .options
                .error_code
                .and_then(|value| value.as_string().ok()),
        }
    }
}

// Silence unused imports in some feature combos.
#[allow(dead_code)]
fn _unused() {
    let _ = parse_tcl_int;
    let _ = parse_tcl_number;
    let _ = split_tcl_list;
}
