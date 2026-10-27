//! Statement-step engine: Tcl_CreateObjTrace bridge to tcld-debug-state.
//!
//! ## Design notes
//!
//! - **Trace flag**: `TCL_TRACE_ENTER_EXEC` (level 0 ⇒ all commands). Fires
//!   *before* a command executes, so we have a chance to pause without
//!   side-effects.
//! - **Re-entry guard**: thread-local `Cell<bool>`. The trace handler sets
//!   it before invoking any Tcl_Eval (e.g. for condition evaluation or
//!   `info frame`); reads it on entry and short-circuits if already set.
//! - **Per-interp lookup**: a `Mutex<BTreeMap<InterpPtr, EngineCtx>>`. The
//!   handler dereferences the ctx via the `client_data` parameter so the
//!   hot path needs no map lookup.
//! - **Pause mechanism**: when the handler decides to pause it transitions
//!   AppState → Stopped, emits `DebugEvent::Paused`, and re-enters the
//!   Tcl event loop via `vwait ::tcld::dbg::__resume__`. Step / continue
//!   commands set that variable; vwait returns; handler emits `Resumed`
//!   and returns control to the executing command.
//! - **Line→byte_offset mapping**: in this Tier-0 scope the handler uses
//!   the line reported by `info frame` directly. The Block's
//!   `valid_offsets` are inspected via `find_at_offset` for byte-exact
//!   matching; if the host hasn't pre-registered a Block for the running
//!   proc, the handler still records the event but cannot fire
//!   breakpoints. A future refinement (Tier 1 #5 wrapper commands) will
//!   register every proc body's source on definition.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::CString;
use std::ops::Range;
use std::os::raw::{c_int, c_void};
use std::sync::{Arc, Mutex, OnceLock};

use tcl_sys::{
    TCL_DONT_WAIT, TCL_OK, TCL_SERVICE_ALL, Tcl_Command, Tcl_CreateObjTrace, Tcl_DeleteTrace,
    Tcl_DoOneEvent, Tcl_Interp, Tcl_Obj, Tcl_SetServiceMode, Tcl_Trace,
};

const TCL_EVAL_GLOBAL: c_int = 0x20000;
use arc_swap::ArcSwap;
use tcld_debug_state::{
    AppState, ArmedLocations, BlockId, DebugEvent, DebugState, FrameKind, FrameSnapshot, Location,
    PauseReason,
};

thread_local! {
    /// Trace re-entry guard. Set when the handler is mid-pause; cleared
    /// on exit. Prevents infinite recursion if the handler triggers a
    /// Tcl_Eval that itself fires the trace.
    static IN_HANDLER: Cell<bool> = const { Cell::new(false) };
}

/// Per-interp engine context. Boxed and registered with Tcl as the trace
/// `client_data`; the handler dereferences this directly without a map
/// lookup.
pub struct EngineCtx {
    pub state: Arc<Mutex<DebugState>>,
    /// Hot-path lock-free handle to the armed-locations vec. Cloned once
    /// at install time; subsequent reads from the trace handler use a
    /// single atomic load (no mutex).
    pub armed: Arc<ArcSwap<ArmedLocations>>,
    /// Trace handle returned by `Tcl_CreateObjTrace`. Used by `uninstall`.
    pub trace: *mut Tcl_Trace,
    /// Tcl level at the moment the most recent step command was issued.
    /// Used by `StepOver` / `StepOut` to gate matches on frame depth.
    /// NOTE: this must be on the same scale as the trace handler's
    /// `level` argument (`Tcl_CreateObjTrace` command-nesting depth),
    /// NOT `info level` proc-frame depth — see `paused_trace_level`.
    pub step_anchor_level: c_int,
    /// `Tcl_CreateObjTrace` nesting level of the command the engine is
    /// currently paused at; `None` while running. Step commands issued
    /// mid-pause anchor `StepOver` / `StepOut` to this value: it is
    /// the only depth expressed on the same scale the trace handler's
    /// `level` argument uses. Sampling `info level` at step-command
    /// time is wrong twice over — proc-frame depth is a different
    /// scale than command-nesting depth, and the dispatch context (the
    /// nub's synchronous pause loop, a vwait event callback, an
    /// EmitOnly subscriber) adds frames of its own, so the anchor
    /// would be captured levels away from the paused user frame.
    pub paused_trace_level: Option<c_int>,
    /// True when the current command pause belongs to the active target file.
    /// File StepOver uses this provenance to step over the paused command's
    /// complete subtree.
    pub paused_in_target_file: bool,
    /// File block and source span of the command a file StepOver steps over.
    /// The step ends at the first command traced in that file outside the
    /// span: a later sibling substitution, the command enclosing it, or the
    /// first command run for the next statement, which is often a
    /// substitution Tcl runs before the command around it. File execution can
    /// enter nested evaluators at trace depths indistinguishable from
    /// siblings, so depth alone is not a sound target.
    pub source_step_target: Option<(BlockId, Range<usize>)>,
    /// Opcode callback frame depth at the most recent opcode pause.
    /// `None` until the patched Tcl runtime has paused through the
    /// opcode hook at least once.
    pub paused_opcode_level: Option<tcl_sys::Tcl_Size>,
    pub mode: StepMode,
    pub opcode_mode: OpcodeMode,
    /// Non-pausing opcode profiler counter state. When enabled, the Tcl 9
    /// opcode callback increments `pc -> count` and immediately continues.
    pub opcode_profile_enabled: bool,
    pub opcode_profile_counts: BTreeMap<u64, u64>,
    /// How the handler should suspend execution when it decides to
    /// pause. See [`PauseStrategy`] for the variants and when each is
    /// appropriate.
    pub pause_strategy: PauseStrategy,
    /// Hint for `Block::offset_for_cmd` so loops with repeating
    /// command text resolve to the correct iteration. Reset on
    /// each pause cycle isn't necessary; the hint biases search
    /// forward and wraps around.
    pub last_resolved_offset: usize,
    /// Per-user-block counterpart to `last_resolved_offset`. StartTarget
    /// breakpoints resolve against a file block while the pause UI resolves
    /// against a proc block, so each block needs its own repeated-command hint.
    pub last_frame_offsets: BTreeMap<BlockId, usize>,
    /// When true, the handler runs even when no breakpoints are armed
    /// and no step mode is active — needed for coverage to record
    /// every executed command. Default false (lock-free hot path
    /// preserved). Toggle via `::tcld::dbg::coverage_enable on|off`.
    pub coverage_enabled: bool,
    /// Tier 3: lock-free async-pause flag. Cloned at install time from
    /// `DebugState::pause_requested` so the trace handler reads it via
    /// a single atomic load without acquiring the state mutex. External
    /// actors set it via `request_pause`; the handler swap-resets it on
    /// the next traced command and forces a step pause.
    pub pause_requested: Arc<std::sync::atomic::AtomicBool>,
    /// Tier 4: set when a wire controller has attached. Disables the
    /// `defer_depth > 0` pause-suppression because the wire is the
    /// out-of-band wake-up channel — pausing during the user's vwait
    /// is fine when the controller can still send `step_into` /
    /// `continue` over the socket.
    pub wire_attached: Arc<std::sync::atomic::AtomicBool>,
    /// Exact file block/offset for the next stop-on-entry pause.
    pub stop_on_entry: Option<(BlockId, usize)>,
    /// File block currently being executed by StartTarget.
    pub active_target_file: Option<BlockId>,
    /// Nested `source` ownership. The last entry is the file whose commands
    /// are executing now; it prevents a sourced child from being attributed
    /// to the entry target merely because both share one interpreter.
    pub active_source_files: Vec<BlockId>,
    /// Runtime owner stack for eval/apply/namespace bodies. Anonymous block
    /// identity comes only from wrapper registration; frame filenames are
    /// intentionally not used to guess it.
    pub active_dynamic_blocks: Vec<BlockId>,
    /// Controller-visible logical breakpoint id by generation-specific core
    /// id. The event forwarder reads this without re-entering Tcl, including
    /// while a condition evaluation is already active.
    pub logical_breakpoint_ids: BTreeMap<String, String>,
    /// User stack captured at the exact pause boundary, before the nub's
    /// synchronous wire loop adds debugger-owned frames. Absolute Tcl levels
    /// remain valid for eval/variable operations while that loop is active.
    pub paused_stack: Vec<FrameSnapshot>,
    /// Most recent user command observed at each absolute proc level. Tcl's
    /// trace callback omits caller frames from `info frame`; retaining them as
    /// execution enters nested procs lets a later pause reconstruct the real
    /// caller chain without including nub frames.
    pub observed_frames: BTreeMap<u32, FrameSnapshot>,
}

unsafe impl Send for EngineCtx {}

/// How the trace handler suspends when it decides to pause.
///
/// The choice depends on the host environment, not on whether code is
/// "production" or "test":
///
/// - [`PauseStrategy::Vwait`] enters Tcl's event loop via
///   `vwait ::tcld::dbg::__resume__`. Returns when an external actor
///   modifies that variable. Requires a working Tcl notifier (set up
///   by `Tcl_FindExecutable`) and an external pump that can signal
///   resume — typically a separate UI thread, or an embedding host
///   that calls `Tcl_AsyncMark` from another thread. This is the
///   strategy a real GUI debugger uses.
///
/// - [`PauseStrategy::EmitOnly`] returns immediately after emitting
///   the `Paused` event, without entering vwait. The pause is
///   logically observable through the [`crate::DebugState`] event
///   stream but the traced interpreter never blocks. Suitable for
///   single-threaded test harnesses, headless inspection tools, and
///   embedding scenarios that drive resume by polling rather than by
///   blocking. Counters and event emissions are identical to Vwait;
///   only the suspend behavior differs.
///
/// Default is `Vwait` because that's what an interactive debugger
/// expects. Tests in this crate use `EmitOnly` and assert the
/// observable event stream rather than the suspend behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PauseStrategy {
    #[default]
    Vwait,
    EmitOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    /// No pause unless a breakpoint matches.
    Continue,
    /// Pause on the next command at any level (including deeper).
    StepInto,
    /// Pause on the next command at the same level or shallower than
    /// `step_anchor_level`. Skips into nested calls.
    StepOver,
    /// Pause on the next command at strictly shallower level than
    /// `step_anchor_level`. Resumes until the current frame returns.
    StepOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcodeMode {
    Continue,
    StepInto,
    StepOver {
        anchor_level: Option<tcl_sys::Tcl_Size>,
    },
    StepOut {
        anchor_level: Option<tcl_sys::Tcl_Size>,
    },
    RunToPc {
        pc: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcodeResumeRequest {
    StepInto,
    StepOver,
    StepOut,
    RunToPc { pc: u64 },
}

pub(crate) fn opcode_mode_for_resume_request(
    request: OpcodeResumeRequest,
    paused_opcode_level: Option<tcl_sys::Tcl_Size>,
) -> Result<OpcodeMode, String> {
    Ok(match request {
        OpcodeResumeRequest::StepInto => OpcodeMode::StepInto,
        OpcodeResumeRequest::RunToPc { pc } => OpcodeMode::RunToPc { pc },
        OpcodeResumeRequest::StepOver => OpcodeMode::StepOver {
            anchor_level: Some(paused_opcode_level.ok_or_else(|| {
                "cannot step over instruction: not paused at an opcode boundary".to_string()
            })?),
        },
        OpcodeResumeRequest::StepOut => OpcodeMode::StepOut {
            anchor_level: Some(paused_opcode_level.ok_or_else(|| {
                "cannot step out instruction: not paused at an opcode boundary".to_string()
            })?),
        },
    })
}

impl EngineCtx {
    pub fn new(state: Arc<Mutex<DebugState>>) -> Self {
        let (armed, pause_requested) = {
            let g = state
                .lock()
                .expect("DebugState mutex poisoned at install time");
            (g.armed_locations(), g.pause_requested.clone())
        };
        Self {
            state,
            armed,
            trace: std::ptr::null_mut(),
            step_anchor_level: 0,
            paused_trace_level: None,
            paused_in_target_file: false,
            source_step_target: None,
            paused_opcode_level: None,
            mode: StepMode::Continue,
            opcode_mode: OpcodeMode::Continue,
            opcode_profile_enabled: false,
            opcode_profile_counts: BTreeMap::new(),
            pause_strategy: PauseStrategy::default(),
            last_resolved_offset: 0,
            last_frame_offsets: BTreeMap::new(),
            coverage_enabled: false,
            pause_requested,
            wire_attached: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            stop_on_entry: None,
            active_target_file: None,
            active_source_files: Vec::new(),
            active_dynamic_blocks: Vec::new(),
            logical_breakpoint_ids: BTreeMap::new(),
            paused_stack: Vec::new(),
            observed_frames: BTreeMap::new(),
        }
    }
}

/// Mark a wire controller as attached. Called by `nub_connect`. Disables
/// the in-vwait pause-defer gate.
///
/// # Safety
///
/// `interp` must be the same pointer used in [`install`].
pub fn set_wire_attached(interp: *mut Tcl_Interp, on: bool) {
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr)
                .wire_attached
                .store(on, std::sync::atomic::Ordering::Release);
        }
    }
}

/// Toggle coverage recording on the trace handler's hot path. When
/// enabled, the handler runs even with zero armed breakpoints and
/// `Continue` mode, recording every traced command's location.
///
/// # Safety
///
/// `interp` must be the same pointer used in [`install`].
pub unsafe fn set_coverage_enabled(interp: *mut Tcl_Interp, on: bool) {
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr).coverage_enabled = on;
        }
    }
}

/// Walk the Tcl frame stack and return [`FrameSnapshot`] entries from
/// outermost to innermost. Used by the typed `inspect_stack` Rust API
/// (the Tcl-side `::tcld::dbg::inspect_stack` returns the same shape
/// as a stringified list of dicts; this function is the parallel
/// strongly-typed entry point for in-process consumers).
///
/// ## Field fidelity
///
/// Each [`FrameSnapshot`] is populated from `info frame N`:
///
/// - `level` — the proc-call depth for that frame (`info level`
///   convention, 0 == toplevel).
/// - `kind` — translated from `info frame.type`. `proc` / `source` /
///   `eval` / `precompiled` map directly; `eval` is also the fallback
///   for procs whose body was defined via the `eval` command rather
///   than `source`d from a file (a Tcl 9 quirk where `info frame.proc`
///   is omitted on those frames).
/// - `proc_qname` — `info frame.proc` with leading `::` stripped, or
///   `args[0]` from `info level N` as fallback when `proc` is absent.
/// - `location` — populated when (a) the frame has a `proc_qname`,
///   (b) the frame's `cmd` field appears verbatim in the matching
///   `Block`'s source. For a proc body running `inner`, the proc's
///   frame `cmd` is `inner`, which appears in the proc's body source
///   → Location resolves. For a frame whose `cmd` is the proc's own
///   name (call-site frame, where caller is "running" the proc), the
///   cmd doesn't appear in the proc's body source → Location stays
///   `None`. This is correct: that frame is not currently inside
///   the proc's body.
/// - `via_uplevel_from` — set when a [`crate::DebugState::uplevel_hops`]
///   entry's `to_level` matches.
///
/// ## Tcl 9 quirk: depth from inside the trace handler
///
/// `info frame` called from inside a `Tcl_CreateObjTrace` callback
/// (i.e. before the traced command's frame is allocated) reports a
/// reduced depth — typically only the script-eval frames above the
/// command being entered, not the body frame the command will run in.
/// This affects callers using `PauseStrategy::EmitOnly` which run
/// subscriber callbacks during the trace path. With
/// `PauseStrategy::Vwait` the handler re-enters Tcl's event loop via
/// vwait; subsequent `inspect_stack` calls from external script
/// contexts (or a GUI thread that signal_resume's into the interp)
/// see the full frame stack. Production interactive debuggers use
/// Vwait; the reduced depth is a test-mode-only observation.
///
/// # Safety
///
/// `interp` must be a valid Tcl interpreter pointer. The DebugState
/// gate (`require_stopped`) is the caller's responsibility — this
/// function does not enforce it.
pub unsafe fn inspect_stack(interp: *mut Tcl_Interp) -> Vec<FrameSnapshot> {
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        let cached = unsafe { &(*ctx_ptr).paused_stack };
        if !cached.is_empty() {
            return cached.clone();
        }
    }

    unsafe { inspect_stack_live(interp) }
}

unsafe fn inspect_stack_live(interp: *mut Tcl_Interp) -> Vec<FrameSnapshot> {
    // Re-entry guard so frame queries don't trigger the trace handler.
    let previous_handler_state = IN_HANDLER.with(|c| c.replace(true));
    let depth = unsafe {
        let script = b"info level\0";
        if tcl_sys::Tcl_EvalEx(
            interp,
            script.as_ptr() as *const std::os::raw::c_char,
            -1,
            0,
        ) != TCL_OK
        {
            IN_HANDLER.with(|c| c.set(previous_handler_state));
            return Vec::new();
        }
        let result = tcl_sys::Tcl_GetObjResult(interp);
        let mut n: c_int = 0;
        if tcl_sys::Tcl_GetIntFromObj(interp, result, &mut n) != TCL_OK {
            IN_HANDLER.with(|c| c.set(previous_handler_state));
            return Vec::new();
        }
        n
    };

    // Resolve state handle once. inspect_stack is a public API used
    // outside the trace handler so we look up via the engine
    // registry.
    let state = lookup_ctx(interp).map(|ctx_ptr| unsafe { (*ctx_ptr).state.clone() });

    // info frame indexes are 1..=depth. info frame 0 returns the
    // introspection call itself (or errors on some Tcl versions); we
    // skip it. The `level` field WITHIN each frame's dict is the
    // proc-call depth (a different concept from the index) — that's
    // what FrameSnapshot.level should hold.
    let mut frames = Vec::with_capacity(depth.max(0) as usize);
    // Always emit a toplevel frame so callers can rely on at least
    // one frame being returned even from a paused interpreter at
    // global scope.
    frames.push(tcld_debug_state::FrameSnapshot {
        level: 0,
        kind: tcld_debug_state::FrameKind::Toplevel,
        proc_qname: None,
        source_path: active_target_path(interp).map(Arc::from),
        location: None,
        args: Vec::new(),
        via_uplevel_from: None,
    });
    for index in 1..=depth {
        let frame_dict = unsafe {
            let script = format!("info frame {index}\0");
            let cstr = std::ffi::CString::new(&script[..script.len() - 1])
                .ok()
                .and_then(|c| {
                    if tcl_sys::Tcl_EvalEx(interp, c.as_ptr(), -1, 0) == TCL_OK {
                        Some(())
                    } else {
                        None
                    }
                });
            if cstr.is_none() {
                continue;
            }
            let obj = tcl_sys::Tcl_GetObjResult(interp);
            let mut len: tcl_sys::Tcl_Size = 0;
            let ptr = tcl_sys::Tcl_GetStringFromObj(obj, &mut len);
            if ptr.is_null() || len <= 0 {
                continue;
            }
            let bytes = std::slice::from_raw_parts(ptr as *const u8, len as usize);
            String::from_utf8_lossy(bytes).into_owned()
        };

        let mut tokens = simple_dict_tokens(&frame_dict).into_iter();
        let mut kind = FrameKind::Other;
        let mut proc_name: Option<Arc<str>> = None;
        let mut source_path: Option<Arc<str>> = None;
        let mut line: usize = 0;
        let mut cmd: Option<String> = None;
        let mut frame_level: u32 = 0;
        while let (Some(k), Some(v)) = (tokens.next(), tokens.next()) {
            match k.as_str() {
                "type" => kind = FrameKind::from_tcl_frame_type(&v),
                "proc" => {
                    // Normalize: strip leading `::` so the qname matches
                    // BlockId::ProcBody keys (which our wrappers
                    // register without the global prefix).
                    proc_name = Some(Arc::from(v.trim_start_matches("::")));
                }
                "file" => source_path = Some(Arc::from(v)),
                "line" => line = v.parse().unwrap_or(0),
                "cmd" => cmd = Some(v),
                "level" => frame_level = v.parse().unwrap_or(0),
                _ => {}
            }
        }

        // `info level N` takes the proc-call depth (frame_level) and
        // returns the call expression as a list. For toplevel
        // (frame_level == 0) the call returns an error; skip and
        // leave args empty.
        let args: Vec<String> = if frame_level > 0 {
            unsafe { fetch_info_level_args(interp, frame_level as c_int).unwrap_or_default() }
        } else {
            Vec::new()
        };

        // Tcl 9 reports type=eval (no `proc` field) for proc bodies
        // defined via `eval`, even though `info level N` returns the
        // proc call. Fall back to args[0] as the proc name when the
        // dict lacks one. Strip leading `::` for consistency.
        let resolved_qname =
            proc_name.or_else(|| args.first().map(|s| Arc::from(s.trim_start_matches("::"))));
        // A procedure may have been defined by any sourced file or by eval.
        // The frame itself does not prove definition provenance, so leave it
        // absent rather than mis-tagging it with the entry target.

        // Recompute Location now that we have a possibly-recovered
        // qname (the earlier resolution used the literal proc field).
        let location = cmd.as_ref().and_then(|cmd_text| {
            let block_id = resolved_qname
                .as_ref()
                .map(|qname| BlockId::ProcBody(qname.clone()))
                .or_else(|| source_path.as_ref().map(|path| BlockId::File(path.clone())))?;
            state.as_ref().and_then(|state_arc| {
                state_arc.lock().ok().and_then(|guard| {
                    let block = guard.blocks.get(&block_id)?;
                    let offset = block
                        .offset_for_cmd(cmd_text, 0)
                        .or_else(|| block.offset_for_line(line as u32))?;
                    let len = cmd_text.len().max(1);
                    Location::new(block_id.clone(), line, offset..offset + len).ok()
                })
            })
        });

        frames.push(FrameSnapshot {
            level: frame_level,
            kind,
            proc_qname: resolved_qname,
            source_path,
            location,
            args,
            via_uplevel_from: None,
        });
    }

    // Annotate frames with via_uplevel_from based on currently-active
    // uplevel hops. A frame at level T is currently servicing an
    // uplevel'd body iff there's a hop with to_level == T; the source
    // frame is from_level. The annotation is the closest enclosing
    // hop (last one pushed) when multiple match.
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        let state = unsafe { (*ctx_ptr).state.clone() };
        if let Ok(guard) = state.lock() {
            for frame in frames.iter_mut() {
                if let Some(hop) = guard
                    .uplevel_hops
                    .iter()
                    .rev()
                    .find(|h| h.to_level == frame.level)
                {
                    frame.via_uplevel_from = Some(hop.from_level);
                }
            }
        }
    }

    IN_HANDLER.with(|c| c.set(previous_handler_state));
    frames
}

unsafe fn capture_paused_stack(
    interp: *mut Tcl_Interp,
    ctx_ptr: *mut EngineCtx,
    frame: &FrameInfo,
    pause_location: &Location,
) -> u32 {
    let mut frames = unsafe { inspect_stack_live(interp) };
    frames.retain(|candidate| {
        !candidate
            .proc_qname
            .as_deref()
            .is_some_and(|name| name.trim_start_matches("::").starts_with("tcld::dbg::"))
    });
    let current = unsafe { user_frame_snapshot(interp, ctx_ptr, frame, pause_location) };
    let level = current.level;

    // `info frame` inside a trace omits callers. Merge the most recent exact
    // command observed at each still-active parent level before replacing the
    // current row below.
    for observed in unsafe { (*ctx_ptr).observed_frames.values() } {
        if observed.level > level {
            continue;
        }
        match frames
            .iter_mut()
            .find(|candidate| candidate.level == observed.level)
        {
            // The synthetic global row knows no command; the observed one does.
            Some(candidate) if candidate.location.is_none() => *candidate = observed.clone(),
            Some(_) => {}
            None => frames.push(observed.clone()),
        }
    }

    // The trace callback runs before Tcl allocates the command's next frame,
    // so replace any partial row for this absolute level with the authoritative
    // frame assembled from `info frame -1`.
    frames.retain(|candidate| candidate.level < level);
    frames.push(current);
    frames.sort_by_key(|candidate| candidate.level);
    frames.dedup_by_key(|candidate| candidate.level);
    unsafe {
        (*ctx_ptr).paused_stack = frames;
    }
    level
}

unsafe fn observe_user_frame(
    interp: *mut Tcl_Interp,
    ctx_ptr: *mut EngineCtx,
    frame: &FrameInfo,
    location: &Location,
) {
    let snapshot = unsafe { user_frame_snapshot(interp, ctx_ptr, frame, location) };
    unsafe {
        (*ctx_ptr)
            .observed_frames
            .retain(|level, _| *level <= snapshot.level);
        // Level 0 is kept too: it is the caller line a paused proc frame
        // returns to, which the bare global row cannot report.
        (*ctx_ptr).observed_frames.insert(snapshot.level, snapshot);
    }
}

unsafe fn user_frame_snapshot(
    interp: *mut Tcl_Interp,
    ctx_ptr: *mut EngineCtx,
    frame: &FrameInfo,
    location: &Location,
) -> FrameSnapshot {
    // Commands in a sourced file report the nub's `::source` wrapper as their
    // proc, but the wrapper evaluates them with `uplevel 1` in its caller's
    // variable frame. Attribute them to that frame; otherwise a sourced script
    // claims a proc level of its own, and `uplevel #N` for that level resolves
    // to whichever debugger frame happens to be active while paused.
    let runs_in_caller_frame = frame
        .proc_name
        .as_deref()
        .is_some_and(|name| name.trim_start_matches("::") == "source")
        && frame.frame_type.as_deref() != Some("proc");
    let proc_qname = frame
        .proc_name
        .as_deref()
        .filter(|_| !runs_in_caller_frame)
        .map(|name| Arc::from(name.trim_start_matches("::")));
    let level = if runs_in_caller_frame {
        unsafe { fetch_info_level_depth(interp) }.unwrap_or(0)
    } else if proc_qname.is_some() {
        unsafe { fetch_info_level_depth(interp) }
            .unwrap_or(frame.level)
            .max(frame.level)
            .max(1)
    } else {
        0
    };
    let source_path = frame
        .file
        .as_deref()
        .map(Arc::from)
        .or_else(|| active_target_path(interp).map(Arc::from));
    let args = if level > 0 {
        unsafe { fetch_info_level_args(interp, level as c_int) }.unwrap_or_default()
    } else {
        Vec::new()
    };
    let via_uplevel_from = unsafe { (*ctx_ptr).state.clone() }
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .uplevel_hops
                .iter()
                .rev()
                .find(|hop| hop.to_level == level)
                .map(|hop| hop.from_level)
        });
    let kind = if proc_qname.is_some() {
        FrameKind::Proc
    } else if frame.file.is_some() {
        FrameKind::File
    } else {
        FrameKind::Toplevel
    };
    FrameSnapshot {
        level,
        kind,
        proc_qname,
        source_path,
        location: Some(location.clone()),
        args,
        via_uplevel_from,
    }
}

unsafe fn fetch_info_level_depth(interp: *mut Tcl_Interp) -> Option<u32> {
    let script = b"info level\0";
    if unsafe {
        tcl_sys::Tcl_EvalEx(
            interp,
            script.as_ptr() as *const std::os::raw::c_char,
            -1,
            0,
        )
    } != TCL_OK
    {
        return None;
    }
    let result = unsafe { tcl_sys::Tcl_GetObjResult(interp) };
    let mut depth: c_int = 0;
    if unsafe { tcl_sys::Tcl_GetIntFromObj(interp, result, &mut depth) } != TCL_OK {
        return None;
    }
    u32::try_from(depth).ok()
}

unsafe fn fetch_info_level_args(interp: *mut Tcl_Interp, level: c_int) -> Option<Vec<String>> {
    let script = format!("info level {level}");
    let cstr = std::ffi::CString::new(script).ok()?;
    if unsafe { tcl_sys::Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) } != TCL_OK {
        return None;
    }
    let obj = unsafe { tcl_sys::Tcl_GetObjResult(interp) };
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { tcl_sys::Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len <= 0 {
        return Some(Vec::new());
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    Some(simple_dict_tokens(&text))
}

/// Set the pause strategy on an installed engine. Hosts choose
/// `PauseStrategy::Vwait` for interactive debugging (real suspend) or
/// `PauseStrategy::EmitOnly` for poll-based observation. Either is a
/// real production-shape strategy.
///
/// # Safety
///
/// `interp` must be the same pointer used in [`install`].
pub unsafe fn set_pause_strategy(interp: *mut Tcl_Interp, strategy: PauseStrategy) {
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr).pause_strategy = strategy;
        }
    }
}

/// Tier 3: ask the engine to pause on the next traced command. Safe to
/// call from any thread — the flag is an `AtomicBool`, no mutex acquired.
/// The trace handler swap-resets the flag on the next command, so a
/// single call produces at most one extra pause.
///
/// # Safety
///
/// `interp` must be the same pointer used in [`install`]. Returns
/// without effect if no engine is installed on `interp`.
pub fn request_pause(interp: *mut Tcl_Interp) -> bool {
    let Some(ctx_ptr) = lookup_ctx(interp) else {
        return false;
    };
    unsafe {
        (*ctx_ptr)
            .pause_requested
            .store(true, std::sync::atomic::Ordering::Release);
    }
    true
}

/// Arm one exact source location as the next stop-on-entry pause.
pub fn activate_target_file(
    interp: *mut Tcl_Interp,
    block: BlockId,
    stop_offset: Option<usize>,
) -> Result<(), String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    unsafe {
        (*ctx_ptr).active_target_file = Some(block.clone());
        (*ctx_ptr).stop_on_entry = stop_offset.map(|offset| (block, offset));
    }
    Ok(())
}

pub fn clear_active_target_file(interp: *mut Tcl_Interp) {
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr).mode = StepMode::Continue;
            (*ctx_ptr).step_anchor_level = 0;
            (*ctx_ptr).paused_in_target_file = false;
            (*ctx_ptr).source_step_target = None;
            (*ctx_ptr).active_target_file = None;
            (*ctx_ptr).stop_on_entry = None;
            (*ctx_ptr).paused_stack.clear();
            (*ctx_ptr).last_frame_offsets.clear();
            (*ctx_ptr).observed_frames.clear();
        }
    }
}

pub fn active_target_path(interp: *mut Tcl_Interp) -> Option<String> {
    let ctx_ptr = lookup_ctx(interp)?;
    match unsafe { (*ctx_ptr).active_target_file.as_ref() } {
        Some(BlockId::File(path)) => Some(path.to_string()),
        _ => None,
    }
}

pub fn bind_logical_breakpoint(interp: *mut Tcl_Interp, logical_id: String, core_id: String) {
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr)
                .logical_breakpoint_ids
                .insert(core_id, logical_id);
        }
    }
}

pub fn unbind_logical_breakpoint(interp: *mut Tcl_Interp, core_id: &str) {
    if let Some(ctx_ptr) = lookup_ctx(interp) {
        unsafe {
            (*ctx_ptr).logical_breakpoint_ids.remove(core_id);
        }
    }
}

pub fn logical_breakpoint_id(interp: *mut Tcl_Interp, core_id: &str) -> Option<String> {
    let ctx_ptr = lookup_ctx(interp)?;
    unsafe { (*ctx_ptr).logical_breakpoint_ids.get(core_id).cloned() }
}

/// Suspend at a pre-registered target location before Tcl begins evaluating
/// the target. This is the only way to precede command substitutions in the
/// first command; object traces run after those substitutions have executed.
///
/// # Safety
///
/// `interp` must be a live interpreter previously registered with this step
/// engine, and the caller must run on the interpreter's owning thread.
pub unsafe fn pause_at_target_entry(
    interp: *mut Tcl_Interp,
    location: Location,
) -> Result<(), String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    let state = unsafe { (*ctx_ptr).state.clone() };
    {
        let mut guard = state
            .lock()
            .map_err(|_| "state mutex poisoned".to_string())?;
        guard.app_state = AppState::Stopped;
        guard.step_pauses = guard.step_pauses.saturating_add(1);
    }
    unsafe {
        (*ctx_ptr).paused_trace_level = Some(0);
        (*ctx_ptr).paused_in_target_file = true;
        // This synthetic pause is the stop-on-entry event. Do not leave the
        // same offset armed for the first real object-trace callback.
        (*ctx_ptr).stop_on_entry = None;
        (*ctx_ptr).paused_stack = vec![FrameSnapshot {
            level: 0,
            kind: FrameKind::Toplevel,
            proc_qname: None,
            source_path: match location.block() {
                BlockId::File(path) => Some(path.clone()),
                _ => active_target_path(interp).map(Arc::from),
            },
            location: Some(location.clone()),
            args: Vec::new(),
            via_uplevel_from: None,
        }];
    }
    tcld_debug_state::fire_event_unlocked(
        &state,
        DebugEvent::Paused {
            location,
            frame_count: 0,
            breakpoint: None,
            reason: PauseReason::Entry,
            pc: None,
        },
    );

    let pause_loop = b"::tcld::dbg::__nub__::pause_loop\0";
    let rc = unsafe {
        tcl_sys::Tcl_EvalEx(
            interp,
            pause_loop.as_ptr() as *const std::os::raw::c_char,
            -1,
            0,
        )
    };
    unsafe {
        (*ctx_ptr).paused_trace_level = None;
        (*ctx_ptr).paused_in_target_file = false;
        (*ctx_ptr).paused_stack.clear();
    }
    if let Ok(mut guard) = state.lock() {
        guard.app_state = AppState::Running;
    }
    tcld_debug_state::fire_event_unlocked(&state, DebugEvent::Resumed);
    if rc == TCL_OK {
        Ok(())
    } else {
        Err("entry pause loop failed".to_string())
    }
}

/// Newtype around the raw EngineCtx pointer so the registry's Mutex is
/// Send/Sync. Each interp owns one EngineCtx; the pointer is only used
/// from the Tcl thread that owns the interp. We assert Send/Sync here to
/// satisfy the BTreeMap's bounds — the safety contract is documented
/// at every dereference site.
#[derive(Clone, Copy)]
struct CtxPtr(*mut EngineCtx);
unsafe impl Send for CtxPtr {}
unsafe impl Sync for CtxPtr {}

/// Global registry of installed engines keyed by interp pointer. Used by
/// the Tcl commands (`step_into` etc.) which receive the interp ptr but
/// not the EngineCtx directly. The trace handler itself bypasses this
/// because Tcl gives it the ctx via `client_data`.
fn engine_registry() -> &'static Mutex<BTreeMap<usize, CtxPtr>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<usize, CtxPtr>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Look up the engine ctx for an interp. Returns a raw pointer because
/// the caller needs &mut EngineCtx; safety contract documented at call
/// sites.
pub fn lookup_ctx(interp: *mut Tcl_Interp) -> Option<*mut EngineCtx> {
    let key = interp as usize;
    let map = engine_registry().lock().ok()?;
    map.get(&key).map(|c| c.0)
}

/// Replace the local TCL_EVAL_GLOBAL constant where `tcl_sys` doesn't
/// expose one.
const _: () = ();

/// Install the step engine on `interp`. Idempotent: returns Ok with no
/// side-effect if already installed.
///
/// # Safety
///
/// `interp` must be a valid Tcl interpreter pointer. The returned engine
/// remains valid until [`uninstall`] is called or the interp is deleted.
pub unsafe fn install(
    interp: *mut Tcl_Interp,
    state: Arc<Mutex<DebugState>>,
) -> Result<(), String> {
    let key = interp as usize;
    {
        let map = engine_registry().lock().map_err(|e| e.to_string())?;
        if map.contains_key(&key) {
            // Idempotent.
            return Ok(());
        }
    }
    let ctx = Box::into_raw(Box::new(EngineCtx::new(state.clone())));
    let trace = unsafe {
        Tcl_CreateObjTrace(
            interp,
            0, // level 0: all commands
            0, // flags
            trace_handler,
            ctx as *mut c_void,
            Some(trace_delete),
        )
    };
    if trace.is_null() {
        // Reclaim the box.
        unsafe {
            drop(Box::from_raw(ctx));
        }
        return Err("Tcl_CreateObjTrace returned null".to_string());
    }
    unsafe {
        (*ctx).trace = trace;
    }
    {
        let mut map = engine_registry().lock().map_err(|e| e.to_string())?;
        map.insert(key, CtxPtr(ctx));
    }
    {
        let mut state_guard = state.lock().map_err(|e| e.to_string())?;
        // Transition Dead → Running on install. The handler will flip to
        // Stopped on the first pause.
        if state_guard.app_state == AppState::Dead {
            state_guard.app_state = AppState::Running;
        }
    }
    Ok(())
}

/// Uninstall the trace and free the engine context.
///
/// # Safety
///
/// `interp` must be the same pointer used in [`install`].
pub unsafe fn uninstall(interp: *mut Tcl_Interp) -> Result<(), String> {
    let key = interp as usize;
    let ctx_ptr = {
        let mut map = engine_registry().lock().map_err(|e| e.to_string())?;
        match map.remove(&key) {
            Some(p) => p.0,
            None => return Ok(()), // Idempotent.
        }
    };
    unsafe {
        let trace = (*ctx_ptr).trace;
        if !trace.is_null() {
            Tcl_DeleteTrace(interp, trace);
        }
        drop(Box::from_raw(ctx_ptr));
    }
    Ok(())
}

/// Trace-handler nesting level of the command the engine is currently
/// paused at, or `None` when no pause is in progress. Step commands
/// use this as the `StepOver` / `StepOut` anchor — it is the paused
/// user frame's depth on the same scale the trace handler's `level`
/// argument uses, unlike `info level` (proc-frame depth) sampled from
/// whatever dispatch context the step command happens to run in.
pub fn paused_trace_level(interp: *mut Tcl_Interp) -> Option<c_int> {
    let ctx_ptr = lookup_ctx(interp)?;
    unsafe { (*ctx_ptr).paused_trace_level }
}

/// Opcode callback nesting level of the instruction the engine is currently
/// paused at, or `None` when the current pause did not come from the opcode
/// runtime.
pub fn paused_opcode_level(interp: *mut Tcl_Interp) -> Option<tcl_sys::Tcl_Size> {
    let ctx_ptr = lookup_ctx(interp)?;
    unsafe { (*ctx_ptr).paused_opcode_level }
}

/// Set the engine's step mode. Called by the step_* Tcl commands.
///
/// # Safety
///
/// `interp` must be the same pointer used in [`install`]. Caller must
/// hold `DebugState::require_stopped()` precondition (commands enforce).
pub unsafe fn set_step_mode(
    interp: *mut Tcl_Interp,
    mode: StepMode,
    anchor_level: c_int,
) -> Result<(), String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    unsafe {
        let paused_in_target_file = (*ctx_ptr).paused_in_target_file;
        (*ctx_ptr).source_step_target = (mode == StepMode::StepOver && paused_in_target_file)
            .then(|| {
                let location = (*ctx_ptr).paused_stack.last()?.location.as_ref()?;
                let block = (*ctx_ptr)
                    .state
                    .lock()
                    .ok()?
                    .blocks
                    .get(location.block())?
                    .clone();
                let stepped = command_span_at(block.source(), location.byte_offset())?;
                Some((location.block().clone(), stepped))
            })
            .flatten();
        if (*ctx_ptr).source_step_target.is_some() {
            // Continue mode never reads the anchor; the file step bounds its
            // target's depth with it instead.
            (*ctx_ptr).mode = StepMode::Continue;
            (*ctx_ptr).step_anchor_level = anchor_level;
        } else {
            // Everything else steps by depth, including a file StepOver from a
            // pause outside every parsed command.
            (*ctx_ptr).mode = match (mode, paused_in_target_file) {
                (StepMode::StepOut, true) => StepMode::Continue,
                _ => mode,
            };
            (*ctx_ptr).step_anchor_level = anchor_level;
        }
    }
    Ok(())
}

/// Source span of the innermost command containing `offset`, including its
/// nested substitutions and bodies.
fn command_span_at(source: &str, offset: usize) -> Option<Range<usize>> {
    tcld_syntax::command_spans(source)
        .ok()?
        .into_iter()
        .filter(|span| (span.start_byte..span.end_byte).contains(&offset))
        .max_by_key(|span| (span.start_byte, std::cmp::Reverse(span.end_byte)))
        .map(|span| span.start_byte..span.end_byte)
}

/// Set the opcode engine's resume mode.
///
/// # Safety
///
/// `interp` must be a valid interpreter previously installed with this step
/// engine, and the caller must run on the interpreter's owning thread.
pub unsafe fn set_opcode_mode(interp: *mut Tcl_Interp, mode: OpcodeMode) -> Result<(), String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    unsafe {
        (*ctx_ptr).opcode_mode = mode;
    }
    Ok(())
}

pub fn opcode_hook_needed(interp: *mut Tcl_Interp) -> bool {
    let Some(ctx_ptr) = lookup_ctx(interp) else {
        return false;
    };
    let profile_on = unsafe { (*ctx_ptr).opcode_profile_enabled };
    profile_on || has_instruction_breakpoints(interp)
}

/// Enable non-pausing opcode counting and reset any previous counts.
///
/// # Safety
///
/// `interp` must be a valid interpreter previously installed with this step
/// engine, and the caller must run on the interpreter's owning thread.
pub unsafe fn profile_start_opcode_counts(interp: *mut Tcl_Interp) -> Result<(), String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    unsafe {
        (*ctx_ptr).opcode_profile_counts.clear();
        (*ctx_ptr).opcode_profile_enabled = true;
    }
    Ok(())
}

/// Disable non-pausing opcode counting without clearing accumulated counts.
///
/// # Safety
///
/// `interp` must be a valid interpreter previously installed with this step
/// engine, and the caller must run on the interpreter's owning thread.
pub unsafe fn profile_stop(interp: *mut Tcl_Interp) -> Result<(), String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    unsafe {
        (*ctx_ptr).opcode_profile_enabled = false;
    }
    Ok(())
}

/// Clear accumulated opcode profile counts.
///
/// # Safety
///
/// `interp` must be a valid interpreter previously installed with this step
/// engine, and the caller must run on the interpreter's owning thread.
pub unsafe fn profile_reset(interp: *mut Tcl_Interp) -> Result<(), String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    unsafe {
        (*ctx_ptr).opcode_profile_counts.clear();
    }
    Ok(())
}

pub fn opcode_profile_snapshot(interp: *mut Tcl_Interp) -> Result<Vec<(u64, u64)>, String> {
    let ctx_ptr = lookup_ctx(interp).ok_or_else(|| "step engine not installed".to_string())?;
    let snapshot = unsafe {
        (*ctx_ptr)
            .opcode_profile_counts
            .iter()
            .map(|(pc, count)| (*pc, *count))
            .collect()
    };
    Ok(snapshot)
}

pub fn has_instruction_breakpoints(interp: *mut Tcl_Interp) -> bool {
    let Some(ctx_ptr) = lookup_ctx(interp) else {
        return false;
    };
    let state_arc = unsafe { (*ctx_ptr).state.clone() };
    state_arc
        .lock()
        .map(|guard| {
            guard.breakpoints().list().any(|bp| {
                bp.enabled
                    && matches!(
                        bp.kind,
                        tcld_debug_state::BreakpointKind::Instruction { .. }
                    )
            })
        })
        .unwrap_or(false)
}

/// Tcl_CmdObjTraceProc — the runtime hot path.
///
/// Called once per command at level 0. Decides pause vs continue. The
/// "no-pause" path executes a single re-entry-guard read and a single
/// Mutex lock; the implementation is deliberately Vec/String free in
/// that path.
unsafe extern "C" fn trace_handler(
    client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    level: c_int,
    command: *const std::os::raw::c_char,
    _command_token: Tcl_Command,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    // Re-entry guard: if we're already inside the handler (because the
    // handler caused a Tcl_Eval), short-circuit immediately.
    if IN_HANDLER.with(|c| c.get()) {
        return TCL_OK;
    }
    if client_data.is_null() {
        return TCL_OK;
    }
    let ctx_ptr = client_data as *mut EngineCtx;
    IN_HANDLER.with(|c| c.set(true));

    // A target loaded with `source` runs synchronously. Pump one pending
    // notifier event at each command boundary so wire commands such as
    // Interrupt remain responsive while the target is executing. The
    // re-entry guard is already raised, so the Tcl fileevent callback's
    // own commands cannot recurse into debugger tracing.
    let should_pump_wire = unsafe {
        (*ctx_ptr)
            .wire_attached
            .load(std::sync::atomic::Ordering::Acquire)
            && (*ctx_ptr).active_target_file.is_some()
    };
    if should_pump_wire {
        unsafe {
            let previous_mode = Tcl_SetServiceMode(TCL_SERVICE_ALL);
            Tcl_DoOneEvent(TCL_DONT_WAIT);
            Tcl_SetServiceMode(previous_mode);
        }
    }

    // Extract the command text directly from Tcl's trace argument —
    // this is the actual command about to execute, not the result of an
    // `info frame` introspection (which would observe our own
    // introspection call). Re-entry guard wraps any subsequent Tcl_Eval.
    let cmd_text = unsafe {
        if command.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(command)
                .to_string_lossy()
                .into_owned()
        }
    };
    let result = unsafe { handler_body(interp, ctx_ptr, level, &cmd_text) };
    IN_HANDLER.with(|c| c.set(false));
    result
}

unsafe fn handler_body(
    interp: *mut Tcl_Interp,
    ctx_ptr: *mut EngineCtx,
    level: c_int,
    cmd_text: &str,
) -> c_int {
    // Read mode / anchor / armed handle from the ctx with no lock —
    // single-thread interp access is the Tcl invariant.
    let (mode, anchor_level) = unsafe { ((*ctx_ptr).mode, (*ctx_ptr).step_anchor_level) };

    // Tier 3: async-pause request. Lock-free atomic swap; if the flag
    // was set by an external actor since the last trace, force this
    // trace event to act as a step pause. The flag is consumed (swap
    // to false) so subsequent commands run normally until the user
    // requests pause again.
    let async_pause = unsafe {
        (*ctx_ptr)
            .pause_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    };

    let step_match = async_pause
        || match mode {
            StepMode::Continue => false,
            StepMode::StepInto => true,
            StepMode::StepOver => level <= anchor_level,
            StepMode::StepOut => level < anchor_level,
        };

    // Lock-free hot-path early exit: if no breakpoints are armed AND
    // we aren't in a step mode AND coverage isn't recording, return
    // without taking any mutex or running any Tcl_Eval.
    let armed = unsafe { (*ctx_ptr).armed.load_full() };
    let entry_armed = unsafe { (*ctx_ptr).stop_on_entry.is_some() };
    let source_step_armed = unsafe { (*ctx_ptr).source_step_target.is_some() };
    let coverage_on = unsafe { (*ctx_ptr).coverage_enabled };
    if !step_match && !entry_armed && !source_step_armed && armed.is_empty() && !coverage_on {
        return TCL_OK;
    }

    let frame = match unsafe { fetch_frame_info(interp) } {
        Some(f) => f,
        None => {
            if step_match {
                emit_step_pause(ctx_ptr, level, None);
            }
            return TCL_OK;
        }
    };

    // E101 resolution: use `info frame.cmd` to find the body byte
    // offset of the executing command. cmd-text matching is the
    // reliable signal because Tcl's `info frame.line` collapses to a
    // single value for every command in `eval`-loaded proc bodies.
    // Fall back to line-based resolution and finally to the
    // placeholder offset for blocks the wrapper hasn't registered yet.
    let frame_location_opt = build_location_from_frame(&frame);
    let dynamic_location =
        unsafe { (*ctx_ptr).active_dynamic_blocks.last().cloned() }.and_then(|block_id| {
            let state = unsafe { (*ctx_ptr).state.clone() };
            state.lock().ok().and_then(|guard| {
                let block = guard.blocks.get(&block_id)?;
                let offset = block.offset_for_cmd(cmd_text, 0)?;
                let line = block.line_for_offset(offset).unwrap_or(1) as usize;
                Location::new(
                    block_id,
                    line,
                    offset..offset.saturating_add(cmd_text.len().max(1)),
                )
                .ok()
            })
        });
    let mut location_opt = dynamic_location
        .clone()
        .or_else(|| frame_location_opt.clone());
    let last_offset = unsafe { (*ctx_ptr).last_resolved_offset };
    let active_file = unsafe {
        (*ctx_ptr)
            .active_source_files
            .last()
            .cloned()
            .or_else(|| (*ctx_ptr).active_target_file.clone())
    };
    if dynamic_location.is_none()
        && let Some(active_file) = active_file
    {
        let state_arc = unsafe { (*ctx_ptr).state.clone() };
        if let Ok(guard) = state_arc.lock()
            && let Some(block) = guard.blocks.get(&active_file)
            && let Some(offset) = block.offset_for_cmd(cmd_text, last_offset)
        {
            let line = block
                .line_for_offset(offset)
                .map(|line| line as usize)
                .unwrap_or(frame.line.max(1));
            location_opt = Location::new(
                active_file,
                line,
                offset..offset.saturating_add(cmd_text.len().max(1)),
            )
            .ok();
        }
    }
    let mut block_disabled = false;
    let mut location_registered = false;
    let wrapper_frame = frame.proc_name.as_deref().is_some_and(|name| {
        let name = name.trim_start_matches("::");
        let dynamic_user_command = dynamic_location.is_some();
        let internal_wrapper_name = name.starts_with("tcld::dbg::__wrap__::")
            || name
                .rsplit("::")
                .next()
                .is_some_and(|leaf| leaf.starts_with("orig_"));
        !dynamic_user_command
            && (internal_wrapper_name
                || matches!(
                    name,
                    "proc"
                        | "catch"
                        | "apply"
                        | "namespace"
                        | "eval"
                        | "vwait"
                        | "update"
                        | "uplevel"
                        | "interp"
                        | "rename"
                ))
    });
    let mut debugger_internal_location = wrapper_frame;
    let resolved_offset = if let Some(loc) = location_opt.as_ref() {
        let state_arc = unsafe { (*ctx_ptr).state.clone() };
        let resolved = {
            let mut guard = match state_arc.lock() {
                Ok(g) => g,
                Err(_) => return TCL_OK,
            };
            location_registered = guard.blocks.contains(loc.block());
            debugger_internal_location |= matches!(
                loc.block(),
                BlockId::ProcBody(name)
                    if name.trim_start_matches("::").starts_with("tcld::dbg::")
            );
            block_disabled = guard.is_block_disabled(loc.block());
            let off = guard
                .blocks
                .get(loc.block())
                .and_then(|block| {
                    block
                        .offset_for_cmd(cmd_text, last_offset)
                        .or_else(|| block.offset_for_line(frame.line as u32))
                })
                .unwrap_or_else(|| loc.byte_offset());
            // Record coverage at every traced command, not just paused
            // ones. Coverage is "did the runtime ever execute this
            // command", not "did the debugger see this command".
            // Skip coverage for disabled blocks — disabling a block
            // detaches it from the debugger entirely.
            if !block_disabled {
                guard.record_coverage(loc.block(), off);
            }
            off
        };
        unsafe {
            (*ctx_ptr).last_resolved_offset = resolved + 1;
        }
        resolved
    } else {
        0
    };
    // StartTarget file blocks are the authoritative lookup surface for line
    // breakpoints, but a command inside a proc must still be reported as that
    // proc body. Resolve a separate display location without changing the
    // exact file offset used above for breakpoint matching.
    let frame_pause_location = dynamic_location.clone().or_else(|| {
        frame_location_opt.as_ref().and_then(|location| {
            let state_arc = unsafe { (*ctx_ptr).state.clone() };
            state_arc.lock().ok().and_then(|guard| {
                let block = guard.blocks.get(location.block())?;
                let hint = unsafe {
                    (*ctx_ptr)
                        .last_frame_offsets
                        .get(location.block())
                        .copied()
                        .unwrap_or(0)
                };
                let offset = block.offset_for_cmd(cmd_text, hint).or_else(|| {
                    matches!(location.block(), BlockId::File(_))
                        .then(|| block.offset_for_line(frame.line as u32))
                        .flatten()
                })?;
                unsafe {
                    (*ctx_ptr)
                        .last_frame_offsets
                        .insert(location.block().clone(), offset.saturating_add(1));
                }
                let line = block
                    .line_for_offset(offset)
                    .map(|line| line as usize)
                    .unwrap_or(frame.line);
                Location::new(
                    location.block().clone(),
                    line,
                    offset..offset.saturating_add(cmd_text.len().max(1)),
                )
                .ok()
            })
        })
    });
    // Disabled blocks short-circuit before breakpoint or step matching.
    // Mirrors TclPro's `dbg::uninstrumentProc`: the debugger pretends
    // the block isn't traced.
    if block_disabled {
        return TCL_OK;
    }

    // Source instrumentation executes helper commands such as `uplevel`
    // between user commands. They can satisfy the depth rule for StepOver or
    // StepOut, but they have no registered source block and must never become
    // a user-visible pause. While a target file is active, defer step and
    // async-pause requests until the next registered file/proc command.
    if step_match
        && unsafe { (*ctx_ptr).active_target_file.is_some() }
        && (!location_registered || debugger_internal_location)
    {
        return TCL_OK;
    }
    let is_user_proc_frame = frame_location_opt.as_ref().is_some_and(|location| {
        matches!(location.block(), BlockId::ProcBody(name) if !name.starts_with("tcld::dbg::"))
            && unsafe { (*ctx_ptr).state.clone() }
                .lock()
                .is_ok_and(|guard| guard.blocks.contains(location.block()))
    });
    if step_match && is_user_proc_frame && frame_pause_location.is_none() {
        return TCL_OK;
    }

    if let Some(user_location) = frame_pause_location.as_ref().or(location_opt.as_ref()) {
        unsafe {
            observe_user_frame(interp, ctx_ptr, &frame, user_location);
        }
    }

    // Lock-free breakpoint lookup against the armed snapshot.
    let bp_id = location_opt
        .as_ref()
        .and_then(|loc| armed.find(loc.block(), resolved_offset))
        .or_else(|| {
            frame_pause_location
                .as_ref()
                .and_then(|location| armed.find(location.block(), location.byte_offset()))
        });

    let entry_match = location_opt.as_ref().is_some_and(|location| unsafe {
        (*ctx_ptr)
            .stop_on_entry
            .as_ref()
            .is_some_and(|(block, offset)| location.block() == block && resolved_offset == *offset)
    });
    // File StepOver ends at the first command whose pause would be reported in
    // the stepped file outside the stepped command. Procs, eval scripts, and
    // other files it runs report locations of their own, and event callbacks
    // it services run deeper than the paused command (an entry pause has no
    // traced depth to compare against).
    let pause_offset = frame_pause_location
        .as_ref()
        .map(|location| (location.block(), location.byte_offset()))
        .or_else(|| {
            location_opt
                .as_ref()
                .map(|location| (location.block(), resolved_offset))
        });
    let source_step_match = !debugger_internal_location
        && !is_user_proc_frame
        && (anchor_level == 0 || level <= anchor_level)
        && unsafe { (*ctx_ptr).source_step_target.as_ref() }.is_some_and(|(block, stepped)| {
            pause_offset.is_some_and(|(pause_block, offset)| {
                pause_block == block && !stepped.contains(&offset)
            })
        });
    if entry_match {
        unsafe {
            (*ctx_ptr).stop_on_entry = None;
        }
    }
    if source_step_match {
        unsafe {
            (*ctx_ptr).source_step_target = None;
        }
    }
    let forced_pause = step_match || entry_match || source_step_match;

    if !forced_pause && bp_id.is_none() {
        return TCL_OK;
    }

    // Conditional breakpoint evaluation. Closes E006: if the matched
    // breakpoint has a condition, eval it in the paused frame's scope.
    // A truthy result causes the pause; a falsy result skips. A Tcl
    // error in the condition increments condition_eval_errors and is
    // treated as falsy — a bad condition must never prevent execution.
    let bp_id = if let Some(id) = bp_id {
        let condition = unsafe { (*ctx_ptr).state.clone() }
            .lock()
            .ok()
            .and_then(|guard| guard.breakpoints().get(id).cloned())
            .and_then(|bp| bp.condition.clone());
        match condition {
            Some(expr) => match unsafe { evaluate_condition(interp, level, &expr) } {
                Ok(true) => Some(id),
                Ok(false) => {
                    // Condition false; skip pause. Don't emit
                    // ConditionError, just resume silently.
                    if !forced_pause {
                        return TCL_OK;
                    }
                    None
                }
                Err(message) => {
                    let state_arc = unsafe { (*ctx_ptr).state.clone() };
                    if let Ok(mut guard) = state_arc.lock() {
                        guard.condition_eval_errors = guard.condition_eval_errors.saturating_add(1);
                    }
                    tcld_debug_state::fire_event_unlocked(
                        &state_arc,
                        DebugEvent::ConditionError {
                            breakpoint: id,
                            error: tcld_debug_state::ConditionEvalError { message },
                        },
                    );
                    if !forced_pause {
                        return TCL_OK;
                    }
                    None
                }
            },
            None => Some(id),
        }
    } else {
        None
    };

    if !forced_pause && bp_id.is_none() {
        return TCL_OK;
    }

    // Defer pauses while inside vwait/update — pausing during event-loop
    // re-entry would block the loop with no path for an external resume
    // to reach us. Increment a deferred-pauses counter so the host can
    // surface "you'd have paused but vwait was running" from telemetry.
    //
    // The defer only applies to `PauseStrategy::Vwait` because emitting
    // (EmitOnly) is non-blocking — wire-driven hosts use `update` (or
    // `vwait` on a controller var) as their event pump, and they NEED
    // pause events to fire during that pump. Suppressing them under
    // EmitOnly is a wire-mode bug.
    let strategy_now = unsafe { (*ctx_ptr).pause_strategy };
    let wire_on = unsafe {
        (*ctx_ptr)
            .wire_attached
            .load(std::sync::atomic::Ordering::Acquire)
    };
    if matches!(strategy_now, PauseStrategy::Vwait) && !wire_on {
        let state_arc = unsafe { (*ctx_ptr).state.clone() };
        if let Ok(mut guard) = state_arc.lock()
            && guard.defer_depth > 0
        {
            guard.deferred_pauses = guard.deferred_pauses.saturating_add(1);
            return TCL_OK;
        }
    }

    // Pause. Transition state, then dispatch the event WITHOUT
    // holding the state mutex so subscribers can safely re-lock state.
    let state_arc = unsafe { (*ctx_ptr).state.clone() };
    let pause_location = frame_pause_location.unwrap_or_else(|| {
        location_opt
            .as_ref()
            .map(|loc| {
                tcld_debug_state::Location::new(
                    loc.block().clone(),
                    loc.line(),
                    resolved_offset..resolved_offset + loc.byte_len().max(1),
                )
                .unwrap_or_else(|_| loc.clone())
            })
            .unwrap_or_else(|| placeholder_location(&frame))
    });
    // Hit-count gating. The breakpoint's ignore_count says "skip the
    // first N hits". `record_hit_and_check_pause` increments hit_count
    // and returns whether THIS hit should pause. A skipped hit still
    // counts but doesn't pause; temporary breakpoints aren't consumed
    // by skipped hits.
    let bp_id = if let Some(id) = bp_id {
        let pause = match state_arc.lock() {
            Ok(mut guard) => {
                guard.with_breakpoints_mut(|store, _| store.record_hit_and_check_pause(id))
            }
            Err(_) => return TCL_OK,
        };
        if !pause && !forced_pause {
            return TCL_OK;
        }
        if pause { Some(id) } else { None }
    } else {
        None
    };
    {
        let mut guard = match state_arc.lock() {
            Ok(g) => g,
            Err(_) => return TCL_OK,
        };
        guard.app_state = AppState::Stopped;
        if bp_id.is_some() {
            guard.breakpoint_hits = guard.breakpoint_hits.saturating_add(1);
        } else {
            guard.step_pauses = guard.step_pauses.saturating_add(1);
        }
    }
    // Record the paused command's trace level BEFORE emitting Paused:
    // step commands issued mid-pause (from the nub's pause loop, a
    // vwait event callback, or an EmitOnly subscriber) read it as the
    // StepOver/StepOut anchor. It is the paused USER frame's depth on
    // the exact scale the handler compares against.
    unsafe {
        (*ctx_ptr).paused_trace_level = Some(level);
        // Whatever stopped here, a file step still in flight is over.
        (*ctx_ptr).source_step_target = None;
        (*ctx_ptr).paused_in_target_file = (*ctx_ptr)
            .active_target_file
            .as_ref()
            .is_some_and(|active| pause_location.block() == active);
    }
    let paused_frame_level =
        unsafe { capture_paused_stack(interp, ctx_ptr, &frame, &pause_location) };
    tcld_debug_state::fire_event_unlocked(
        &state_arc,
        DebugEvent::Paused {
            location: pause_location,
            frame_count: paused_frame_level,
            breakpoint: bp_id,
            reason: if entry_match {
                PauseReason::Entry
            } else if bp_id.is_some() {
                PauseReason::Breakpoint
            } else if async_pause {
                PauseReason::Pause
            } else {
                PauseReason::Step
            },
            pc: None,
        },
    );

    // Suspend per the engine's pause strategy. The re-entry guard
    // prevents the trace handler from re-firing during vwait's own
    // event-loop machinery if Vwait is chosen.
    let strategy = unsafe { (*ctx_ptr).pause_strategy };
    let wire_on_pause = unsafe {
        (*ctx_ptr)
            .wire_attached
            .load(std::sync::atomic::Ordering::Acquire)
    };
    match strategy {
        PauseStrategy::Vwait if wire_on_pause => unsafe {
            // Wire-driven: synchronous read loop keeps the user's
            // paused frame on the stack so wire ops resolve `uplevel
            // #N` correctly. Mirrors TclPro's nub-side wait.
            let script = b"::tcld::dbg::__nub__::pause_loop\0";
            tcl_sys::Tcl_EvalEx(
                interp,
                script.as_ptr() as *const std::os::raw::c_char,
                -1,
                0,
            );
        },
        PauseStrategy::Vwait => unsafe {
            wait_for_resume(interp);
        },
        PauseStrategy::EmitOnly => {
            // Counters and events were already emitted above. The
            // suspend behavior is deliberately absent — the host
            // observes pause via the event stream and resumes by
            // issuing step/continue commands, which immediately
            // change `pause_strategy`'s next-pause behavior. No
            // blocking, no event loop required.
        }
    }

    // Resumed. The pause is over; there is no paused frame to anchor
    // step commands to until the next pause records one.
    unsafe {
        (*ctx_ptr).paused_trace_level = None;
        (*ctx_ptr).paused_opcode_level = None;
        (*ctx_ptr).paused_in_target_file = false;
        (*ctx_ptr).paused_stack.clear();
    }
    {
        let state_arc = unsafe { (*ctx_ptr).state.clone() };
        if let Ok(mut guard) = state_arc.lock() {
            guard.app_state = AppState::Running;
        }
        tcld_debug_state::fire_event_unlocked(&state_arc, DebugEvent::Resumed);
    }

    TCL_OK
}

/// Entrypoint called by the patched Tcl runtime at each opcode boundary.
///
/// # Safety
///
/// `interp` must be the live interpreter passed by Tcl on its owning thread.
/// `pc_offset` and `num_levels` must describe the current TEBC frame supplied
/// by the patched runtime.
pub unsafe fn opcode_boundary(
    interp: *mut Tcl_Interp,
    pc_offset: tcl_sys::Tcl_Size,
    _bc_flags: u32,
    num_levels: tcl_sys::Tcl_Size,
) -> c_int {
    if IN_HANDLER.with(|c| c.get()) {
        return TCL_OK;
    }
    let Some(ctx_ptr) = lookup_ctx(interp) else {
        return TCL_OK;
    };
    IN_HANDLER.with(|c| c.set(true));
    let result = unsafe { opcode_boundary_body(interp, ctx_ptr, pc_offset, num_levels) };
    IN_HANDLER.with(|c| c.set(false));
    result
}

unsafe fn opcode_boundary_body(
    interp: *mut Tcl_Interp,
    ctx_ptr: *mut EngineCtx,
    pc_offset: tcl_sys::Tcl_Size,
    num_levels: tcl_sys::Tcl_Size,
) -> c_int {
    if pc_offset < 0 {
        return TCL_OK;
    }
    let pc = pc_offset as u64;
    let level = num_levels.max(0);
    unsafe {
        if (*ctx_ptr).opcode_profile_enabled {
            let count = (*ctx_ptr).opcode_profile_counts.entry(pc).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    let step_match = unsafe {
        let mode = &mut (*ctx_ptr).opcode_mode;
        let mut clear_mode = false;
        let matched = match mode {
            OpcodeMode::Continue => false,
            OpcodeMode::StepInto => {
                clear_mode = true;
                true
            }
            OpcodeMode::StepOver { anchor_level } => {
                let anchor = *anchor_level.get_or_insert(level);
                if level <= anchor {
                    clear_mode = true;
                    true
                } else {
                    false
                }
            }
            OpcodeMode::StepOut { anchor_level } => {
                let anchor = *anchor_level.get_or_insert(level);
                if level < anchor {
                    clear_mode = true;
                    true
                } else {
                    false
                }
            }
            OpcodeMode::RunToPc { pc: target } => {
                if pc == *target {
                    clear_mode = true;
                    true
                } else {
                    false
                }
            }
        };
        if clear_mode {
            *mode = OpcodeMode::Continue;
        }
        matched
    };

    let state_arc = unsafe { (*ctx_ptr).state.clone() };
    let bp_id = state_arc.lock().ok().and_then(|guard| {
        guard
            .breakpoints()
            .find_instruction_at_pc(pc as usize)
            .map(|bp| bp.id)
    });
    if !step_match && bp_id.is_none() {
        return TCL_OK;
    }

    let bp_id = if let Some(id) = bp_id {
        let condition = state_arc
            .lock()
            .ok()
            .and_then(|guard| guard.breakpoints().get(id).cloned())
            .and_then(|bp| bp.condition.clone());
        match condition {
            Some(expr) => match unsafe { evaluate_condition(interp, level as c_int, &expr) } {
                Ok(true) => Some(id),
                Ok(false) => {
                    if !step_match {
                        return TCL_OK;
                    }
                    None
                }
                Err(message) => {
                    if let Ok(mut guard) = state_arc.lock() {
                        guard.condition_eval_errors = guard.condition_eval_errors.saturating_add(1);
                    }
                    tcld_debug_state::fire_event_unlocked(
                        &state_arc,
                        DebugEvent::ConditionError {
                            breakpoint: id,
                            error: tcld_debug_state::ConditionEvalError { message },
                        },
                    );
                    if !step_match {
                        return TCL_OK;
                    }
                    None
                }
            },
            None => Some(id),
        }
    } else {
        None
    };
    if !step_match && bp_id.is_none() {
        return TCL_OK;
    }

    let strategy_now = unsafe { (*ctx_ptr).pause_strategy };
    let wire_on = unsafe {
        (*ctx_ptr)
            .wire_attached
            .load(std::sync::atomic::Ordering::Acquire)
    };
    if matches!(strategy_now, PauseStrategy::Vwait)
        && !wire_on
        && let Ok(mut guard) = state_arc.lock()
        && guard.defer_depth > 0
    {
        guard.deferred_pauses = guard.deferred_pauses.saturating_add(1);
        return TCL_OK;
    }

    let bp_id = if let Some(id) = bp_id {
        let pause = match state_arc.lock() {
            Ok(mut guard) => {
                guard.with_breakpoints_mut(|store, _| store.record_hit_and_check_pause(id))
            }
            Err(_) => return TCL_OK,
        };
        if !pause && !step_match {
            return TCL_OK;
        }
        if pause { Some(id) } else { None }
    } else {
        None
    };

    {
        let mut guard = match state_arc.lock() {
            Ok(g) => g,
            Err(_) => return TCL_OK,
        };
        guard.app_state = AppState::Stopped;
        if bp_id.is_some() {
            guard.breakpoint_hits = guard.breakpoint_hits.saturating_add(1);
        } else {
            guard.step_pauses = guard.step_pauses.saturating_add(1);
        }
    }

    unsafe {
        (*ctx_ptr).paused_trace_level = None;
        (*ctx_ptr).paused_opcode_level = Some(level);
        (*ctx_ptr).source_step_target = None;
    }
    let frame = unsafe { fetch_frame_info(interp) }.unwrap_or_default();
    let location = opcode_pause_location(interp, &frame);
    tcld_debug_state::fire_event_unlocked(
        &state_arc,
        DebugEvent::Paused {
            location,
            frame_count: level as u32,
            breakpoint: bp_id,
            reason: if bp_id.is_some() {
                PauseReason::InstructionBreakpoint
            } else {
                PauseReason::InstructionStep
            },
            pc: Some(pc),
        },
    );

    let strategy = unsafe { (*ctx_ptr).pause_strategy };
    let wire_on_pause = unsafe {
        (*ctx_ptr)
            .wire_attached
            .load(std::sync::atomic::Ordering::Acquire)
    };
    match strategy {
        PauseStrategy::Vwait if wire_on_pause => unsafe {
            let script = b"::tcld::dbg::__nub__::pause_loop\0";
            tcl_sys::Tcl_EvalEx(
                interp,
                script.as_ptr() as *const std::os::raw::c_char,
                -1,
                0,
            );
        },
        PauseStrategy::Vwait => unsafe {
            wait_for_resume(interp);
        },
        PauseStrategy::EmitOnly => {}
    }

    unsafe {
        (*ctx_ptr).paused_opcode_level = None;
    }
    {
        if let Ok(mut guard) = state_arc.lock() {
            guard.app_state = AppState::Running;
        }
        tcld_debug_state::fire_event_unlocked(&state_arc, DebugEvent::Resumed);
    }

    TCL_OK
}

/// Information collected from `info frame -1`.
#[derive(Debug, Clone, Default)]
struct FrameInfo {
    frame_type: Option<String>,
    proc_name: Option<String>,
    file: Option<String>,
    line: usize,
    cmd: Option<String>,
    level: u32,
}

unsafe fn fetch_frame_info(interp: *mut Tcl_Interp) -> Option<FrameInfo> {
    // Run "info frame" then "info frame [info frame]" to peel off the
    // active frame. Cheap because we set the re-entry guard before
    // calling.
    let script = b"info frame [info frame]\0";
    let res = unsafe {
        tcl_sys::Tcl_EvalEx(
            interp,
            script.as_ptr() as *const std::os::raw::c_char,
            -1,
            TCL_EVAL_GLOBAL,
        )
    };
    if res != TCL_OK {
        return None;
    }
    let result_obj = unsafe { tcl_sys::Tcl_GetObjResult(interp) };
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { tcl_sys::Tcl_GetStringFromObj(result_obj, &mut len) };
    if ptr.is_null() || len <= 0 {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();

    // Parse the dict-shaped result. Tcl returns "type source line N proc
    // X cmd Y level Z" etc. Simple key/value tokenizer is sufficient.
    let mut info = FrameInfo::default();
    let mut tokens = simple_dict_tokens(&text).into_iter();
    while let (Some(key), Some(value)) = (tokens.next(), tokens.next()) {
        match key.as_str() {
            "type" => info.frame_type = Some(value),
            "proc" => info.proc_name = Some(value),
            "file" => info.file = Some(value),
            "line" => {
                if let Ok(n) = value.parse::<usize>() {
                    info.line = n;
                }
            }
            "cmd" => info.cmd = Some(value),
            "level" => info.level = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    Some(info)
}

/// Tokenize a Tcl-list-shaped dict (alternating key/value tokens) into
/// owned `String`s. Handles balanced `{...}` brace-quoted values, which
/// `info frame` produces for `cmd`.
fn simple_dict_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        if chars[i] == '{' {
            let mut depth = 1;
            let mut j = i + 1;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    j += 1;
                }
            }
            tokens.push(chars[i + 1..j].iter().collect());
            i = j + 1;
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            tokens.push(chars[start..i].iter().collect());
        }
    }
    tokens
}

fn build_location_from_frame(frame: &FrameInfo) -> Option<Location> {
    let is_file_frame = matches!(frame.frame_type.as_deref(), Some("source" | "precompiled"));
    let block = if is_file_frame && let Some(file) = frame.file.as_deref() {
        BlockId::File(Arc::from(file))
    } else if let Some(qname) = frame.proc_name.as_deref() {
        BlockId::ProcBody(Arc::from(qname.trim_start_matches("::")))
    } else {
        BlockId::File(Arc::from(frame.file.as_deref()?))
    };
    let cmd_len = frame.cmd.as_deref().map(str::len).unwrap_or(1).max(1);
    // Without the proc body source available here, treat the line as a
    // body byte offset hint. The trace handler's breakpoint lookup is
    // exact via `find_at_offset`, so a wrong line just means no match —
    // not a false hit.
    Location::new(block, frame.line, frame.line..frame.line + cmd_len).ok()
}

fn placeholder_location(frame: &FrameInfo) -> Location {
    let qname: Arc<str> = frame
        .proc_name
        .clone()
        .map(|s| Arc::from(s.trim_start_matches("::").to_string()))
        .unwrap_or_else(|| Arc::from("<unknown>"));
    Location::new(BlockId::ProcBody(qname), frame.line.max(1), 0..1)
        .expect("placeholder location is non-empty")
}

fn opcode_pause_location(interp: *mut Tcl_Interp, frame: &FrameInfo) -> Location {
    if let Ok(Some((file, line))) = crate::opcode_current_source(interp) {
        let line = usize::try_from(line).unwrap_or(0).max(1);
        if let Ok(loc) = Location::new(BlockId::File(Arc::from(file)), line, line..line + 1) {
            return loc;
        }
    }
    placeholder_location(frame)
}

fn emit_step_pause(ctx_ptr: *mut EngineCtx, level: c_int, location: Option<Location>) {
    let state_arc = unsafe { (*ctx_ptr).state.clone() };
    if let Ok(mut guard) = state_arc.lock() {
        guard.step_pauses = guard.step_pauses.saturating_add(1);
    }
    tcld_debug_state::fire_event_unlocked(
        &state_arc,
        DebugEvent::Paused {
            location: location.unwrap_or_else(|| {
                Location::new(BlockId::ProcBody(Arc::from("<unknown>")), 0, 0..1)
                    .expect("placeholder")
            }),
            frame_count: level as u32,
            breakpoint: None,
            reason: PauseReason::Step,
            pc: None,
        },
    );
}

/// Evaluate `expr` as a Tcl boolean expression in the frame at `level`
/// (the current command's frame). Returns Ok(true)/Ok(false) for clean
/// boolean results, Err(message) for any Tcl error.
unsafe fn evaluate_condition(
    interp: *mut Tcl_Interp,
    _level: c_int,
    expr: &str,
) -> Result<bool, String> {
    use std::ffi::CString;
    // The trace handler invokes us with the interp positioned at the
    // very frame whose command is about to execute (e.g. demo's frame
    // for `set b 2`). Tcl_EvalEx with flags=0 evaluates the script in
    // that frame — exactly the frame the user's condition expects to
    // see. No uplevel needed; uplevel here would actually be wrong
    // because the wrappers' uplevel takes a user-code-relative offset.
    let wrapped = format!("expr {{{}}}", expr);
    let cstr = match CString::new(wrapped) {
        Ok(c) => c,
        Err(_) => return Err("condition contains NUL byte".to_string()),
    };
    let rc = unsafe { tcl_sys::Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
    let result_obj = unsafe { tcl_sys::Tcl_GetObjResult(interp) };
    if rc != TCL_OK {
        let mut len: tcl_sys::Tcl_Size = 0;
        let ptr = unsafe { tcl_sys::Tcl_GetStringFromObj(result_obj, &mut len) };
        if ptr.is_null() || len <= 0 {
            return Err("condition eval failed".to_string());
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
        return Err(String::from_utf8_lossy(bytes).into_owned());
    }
    let mut n: c_int = 0;
    let bool_rc = unsafe { tcl_sys::Tcl_GetBooleanFromObj(interp, result_obj, &mut n) };
    if bool_rc != TCL_OK {
        return Err("condition did not evaluate to a boolean".to_string());
    }
    Ok(n != 0)
}

unsafe fn wait_for_resume(interp: *mut Tcl_Interp) {
    // Eval at the CURRENT frame, not global. If we shifted to global
    // here, fileevent callbacks running during the vwait would see
    // global as their parent — hiding the paused proc's frame from
    // wire-dispatched `eval_in_frame` / `get_variables` requests.
    // Tier 4 parity requires those to land in the paused proc's
    // scope. Tcl supports nested vwaits regardless of frame depth.
    let script = b"vwait ::tcld::dbg::__resume__\0";
    unsafe {
        tcl_sys::Tcl_EvalEx(
            interp,
            script.as_ptr() as *const std::os::raw::c_char,
            -1,
            0,
        );
    }
}

/// Touch `::tcld::dbg::__resume__` to unblock a paused interp. Called by
/// step / continue commands.
///
/// # Safety
///
/// `interp` must be a valid, live `Tcl_Interp` pointer, and the caller must
/// hold the interp's owning thread (Tcl interps are not thread-safe).
pub unsafe fn signal_resume(interp: *mut Tcl_Interp) {
    let varname = match CString::new("::tcld::dbg::__resume__") {
        Ok(c) => c,
        Err(_) => return,
    };
    let value = unsafe { tcl_sys::Tcl_NewStringObj(c"1".as_ptr(), 1) };
    unsafe {
        tcl_sys::Tcl_SetVar2Ex(
            interp,
            varname.as_ptr(),
            std::ptr::null(),
            value,
            tcl_sys::TCL_GLOBAL_ONLY,
        );
    }
}

unsafe extern "C" fn trace_delete(client_data: *mut c_void) {
    if client_data.is_null() {
        return;
    }
    // Tcl deletes the trace either from `uninstall`, which takes the ctx out
    // of the registry first and frees it afterwards, or while deleting an
    // interpreter nobody uninstalled (a child given `slave_install`, say).
    // Only the second still finds the ctx registered. Reclaim it, or the
    // next interpreter allocated at that address would "already" have an
    // engine: this stale one, with a dead interp and someone else's state.
    let ctx = client_data as *mut EngineCtx;
    let orphaned = engine_registry().lock().ok().and_then(|mut map| {
        let key = map
            .iter()
            .find(|(_, entry)| entry.0 == ctx)
            .map(|(key, _)| *key)?;
        map.remove(&key)
    });
    if let Some(entry) = orphaned {
        unsafe { drop(Box::from_raw(entry.0)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_step_skips_the_innermost_command_at_the_paused_offset() {
        let source = "package provide demo [lindex {Revision: 1.0} 1]\n\
namespace eval demo {\n    set nested 1\n}\n\
proc demo::run {} {\n    puts nested\n}\n";
        let lindex = source.find("lindex").unwrap();
        let namespace = source.find("namespace eval").unwrap();
        let nested_set = source.find("set nested").unwrap();
        let procedure = source.find("proc demo::run").unwrap();
        let span = |offset| command_span_at(source, offset).unwrap();

        // A statement's span covers the substitutions Tcl ran before it.
        assert_eq!(span(0).start, 0);
        assert!(span(0).contains(&lindex) && !span(0).contains(&namespace));
        // A substitution is stepped over on its own, from anywhere inside it.
        assert_eq!(span(lindex).start, lindex);
        assert_eq!(span(lindex + 7), span(lindex));
        assert!(span(lindex).end < namespace);
        // A body belongs to the command that evaluates it.
        assert!(span(namespace).contains(&nested_set) && !span(namespace).contains(&procedure));
        assert_eq!(span(nested_set).start, nested_set);
        assert!(!span(nested_set).contains(&procedure));
        assert_eq!(command_span_at(source, source.len()), None);
    }

    #[test]
    fn step_mode_arithmetic_skips_into_calls() {
        // Pure logic test of the step-mode predicate. No interp needed.
        // Anchor at level 2; inside a callee at level 3 should NOT match.
        let anchor: c_int = 2;
        // Replicate the inner predicate.
        let do_match = |mode: StepMode, level: c_int| match mode {
            StepMode::Continue => false,
            StepMode::StepInto => true,
            StepMode::StepOver => level <= anchor,
            StepMode::StepOut => level < anchor,
        };
        assert!(!do_match(StepMode::StepOver, 3));
        assert!(do_match(StepMode::StepOver, 2));
        assert!(do_match(StepMode::StepOver, 1));
        assert!(!do_match(StepMode::StepOut, 2));
        assert!(do_match(StepMode::StepOut, 1));
        assert!(do_match(StepMode::StepInto, 99));
        assert!(!do_match(StepMode::Continue, 2));
    }

    #[test]
    fn opcode_over_out_modes_are_anchored_to_paused_opcode_level() {
        assert_eq!(
            opcode_mode_for_resume_request(OpcodeResumeRequest::StepInto, None).unwrap(),
            OpcodeMode::StepInto
        );
        assert_eq!(
            opcode_mode_for_resume_request(OpcodeResumeRequest::RunToPc { pc: 42 }, None).unwrap(),
            OpcodeMode::RunToPc { pc: 42 }
        );
        assert_eq!(
            opcode_mode_for_resume_request(OpcodeResumeRequest::StepOver, Some(7)).unwrap(),
            OpcodeMode::StepOver {
                anchor_level: Some(7)
            }
        );
        assert_eq!(
            opcode_mode_for_resume_request(OpcodeResumeRequest::StepOut, Some(7)).unwrap(),
            OpcodeMode::StepOut {
                anchor_level: Some(7)
            }
        );
        assert!(
            opcode_mode_for_resume_request(OpcodeResumeRequest::StepOver, None).is_err(),
            "step-over-instruction must not lazily anchor after resume"
        );
        assert!(
            opcode_mode_for_resume_request(OpcodeResumeRequest::StepOut, None).is_err(),
            "step-out-instruction must not lazily anchor after resume"
        );
    }

    #[test]
    fn simple_dict_tokens_parses_braces() {
        let text = "type source line 5 cmd {puts hello} proc ::foo";
        let tokens = simple_dict_tokens(text);
        assert_eq!(
            tokens,
            vec![
                "type".to_string(),
                "source".to_string(),
                "line".to_string(),
                "5".to_string(),
                "cmd".to_string(),
                "puts hello".to_string(),
                "proc".to_string(),
                "::foo".to_string()
            ]
        );
    }
}
