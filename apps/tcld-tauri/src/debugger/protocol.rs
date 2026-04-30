use serde::{Deserialize, Serialize};

// ─── Commands sent from bridge to agent ──────────────────────────────

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum AgentCommand {
    StepInstruction,
    StepOverInstruction,
    StepOutInstruction,
    RunToPc {
        pc: u32,
    },
    StepInto,
    StepOver,
    StepOut,
    Continue,
    Pause,
    SetBreakpoint {
        file: String,
        line: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        len: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    SetBreakpointEnabled {
        file: String,
        line: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<u64>,
        enabled: bool,
    },
    ClearBreakpoint {
        file: String,
        line: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<u64>,
    },
    GetVariables {
        request_id: u64,
        level: u32,
    },
    GetStack {
        request_id: u64,
    },
    Eval {
        request_id: u64,
        script: String,
        level: u32,
    },
    SetVariable {
        name: String,
        value: String,
        level: u32,
    },
    TraceDecision {
        pause: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[allow(dead_code)] // Protocol type — used in later tasks
    ClearAllBreakpoints,
    #[allow(dead_code)] // Protocol type — used in later tasks
    SetWatchpoint {
        name: String,
        scope: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        condition: Option<String>,
    },
    #[allow(dead_code)] // Protocol type — used in later tasks
    ClearWatchpoint {
        name: String,
    },
    #[allow(dead_code)] // Protocol type — used in later tasks
    ClearAllWatchpoints,
    #[allow(dead_code)] // Protocol type — used in later tasks
    ListWatchpoints {
        request_id: u64,
    },
}

// ─── Events received from agent ──────────────────────────────────────

// `Serialize` exists for schema round-trip tests and tooling; production
// code only ever deserializes agent events.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum AgentEvent {
    Ready {
        #[serde(rename = "tclVersion")]
        tcl_version: String,
        pid: u32,
        #[serde(default, rename = "debugExtensionLoaded")]
        debug_extension_loaded: Option<bool>,
        #[serde(default, rename = "debugExtensionInstalled")]
        debug_extension_installed: Option<bool>,
        #[serde(default, rename = "debugExtensionCapabilities")]
        debug_extension_capabilities: Option<String>,
        #[serde(default, rename = "debugExtensionReason")]
        debug_extension_reason: Option<String>,
    },
    Stopped {
        reason: String,
        file: String,
        line: u32,
        #[serde(rename = "frameCount")]
        frame_count: u32,
        #[serde(default)]
        pc: Option<i32>,
        #[serde(default, rename = "instructionIndex")]
        instruction_index: Option<usize>,
        #[serde(default, rename = "blockId")]
        block_id: Option<usize>,
        #[serde(default)]
        error: Option<String>,
        #[serde(default, rename = "errorInfo")]
        error_info: Option<String>,
    },
    Output {
        stream: String,
        text: String,
    },
    Exited {
        code: i32,
        #[serde(default)]
        reason: Option<String>,
    },
    Variables {
        #[serde(default, rename = "requestId")]
        #[allow(dead_code)] // Protocol field — used in later tasks
        request_id: u64,
        level: u32,
        vars: Vec<serde_json::Value>,
    },
    Stack {
        #[serde(default, rename = "requestId")]
        #[allow(dead_code)] // Protocol field — used in later tasks
        request_id: u64,
        frames: Vec<serde_json::Value>,
    },
    EvalResult {
        #[serde(default, rename = "requestId")]
        #[allow(dead_code)] // Protocol field — used in later tasks
        request_id: u64,
        result: String,
        error: bool,
        #[serde(default, rename = "errorInfo")]
        error_info: Option<String>,
        #[serde(default, rename = "errorCode")]
        error_code: Option<String>,
    },
    #[allow(dead_code)] // Protocol variant — used in later tasks
    Resumed {
        #[serde(rename = "controlMode")]
        control_mode: String,
    },
    #[allow(dead_code)] // Protocol variant — used in later tasks
    BreakpointAck {
        file: String,
        line: u32,
        set: bool,
    },
    #[allow(dead_code)] // Protocol variant — used in later tasks
    BreakpointsCleared {
        count: u32,
    },
    BytecodeChunk {
        sequence: u32,
        command: String,
        #[serde(default, rename = "procName")]
        proc_name: Option<String>,
        payload: String,
        #[serde(rename = "payloadSize")]
        payload_size: usize,
        #[serde(default)]
        disassembly: Option<String>,
    },
    CommandError {
        command: String,
        #[serde(rename = "controlMode")]
        control_mode: String,
        code: i32,
        message: String,
        #[serde(default, rename = "errorInfo")]
        error_info: Option<String>,
        #[serde(default, rename = "errorCode")]
        error_code: Option<String>,
        argc: u32,
    },
    TracePoint {
        file: String,
        line: u32,
        #[allow(dead_code)] // Reserved for richer correlation and diagnostics
        #[serde(rename = "frameCount")]
        frame_count: u32,
        level: i32,
        #[serde(rename = "breakpointHit")]
        breakpoint_hit: bool,
    },
    #[allow(dead_code)] // Protocol variant — used in later tasks
    WatchpointHit {
        #[serde(rename = "watchId")]
        watch_id: u64,
        name: String,
        element: Option<String>,
        operation: String,
        #[serde(rename = "oldValue")]
        old_value: String,
        #[serde(rename = "newValue")]
        new_value: String,
        file: String,
        line: u32,
        #[serde(rename = "frameCount")]
        frame_count: u32,
    },

    // ── Reserved v0.4+ event kinds ────────────────────────────────────
    // The four variants below reserve DTO vocabulary for the gated runtime
    // features so they can land additively without a protocol revision.
    // Nothing emits them in v1; when a future agent/extension does, the
    // actor forwards them verbatim on their reserved `debugger:*` channels.
    // Fields are serde-tolerant (`#[serde(default)]` everywhere) so newer
    // backends can add payload fields without breaking older frontends.
    //
    /// Reserved for `gated:live-disassembly-runtime`. Emitted when the
    /// currently-executing
    /// compiled context changes so the frontend can display its
    /// disassembly without a separate request. Nothing emits this in v1.
    DisassemblyContext {
        #[serde(default, rename = "procName")]
        proc_name: Option<String>,
        #[serde(default, rename = "sourceFile")]
        source_file: Option<String>,
        #[serde(default)]
        disassembly: String,
        /// `(pc_offset, source_line)` pairs mapping the disassembly to source.
        #[serde(default, rename = "pcToLine")]
        pc_to_line: Vec<(u32, u32)>,
    },
    /// Reserved for `gated:opcode-profiling-runtime`. Carries
    /// accumulated profile counters for the active profiling mode
    /// (`line_counts` | `opcode_counts` | `line_timing`). Nothing emits
    /// this in v1.
    ProfileData {
        #[serde(default, rename = "requestId")]
        request_id: u64,
        #[serde(default)]
        mode: String,
        #[serde(default, rename = "lineData")]
        line_data: Option<Vec<LineProfileEntry>>,
        #[serde(default, rename = "opcodeData")]
        opcode_data: Option<Vec<OpcodeProfileEntry>>,
    },
    /// Reserved for `gated:epoch-tracking-runtime`. Reports a command
    /// redefinition/rename/delete that
    /// bumped `cmdEpoch` and invalidated dependent bytecode. Nothing emits
    /// this in v1.
    EpochChange {
        #[serde(default)]
        command: String,
        #[serde(default, rename = "oldEpoch")]
        old_epoch: u32,
        #[serde(default, rename = "newEpoch")]
        new_epoch: u32,
        /// "proc_redefine" | "rename" | "delete" | "namespace_delete" | "import"
        #[serde(default)]
        trigger: String,
        /// Global interpreter compile epoch at the time of the change.
        #[serde(default, rename = "compileEpoch")]
        compile_epoch: u32,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        line: Option<u32>,
    },
    /// Reserved for `gated:compilation-introspection-runtime`. Reports a
    /// runtime bytecode compilation
    /// (first call, recompile, eval, source, namespace_eval). Nothing
    /// emits this in v1.
    CompilationEvent {
        /// "first_call" | "recompile" | "eval" | "source" | "namespace_eval"
        #[serde(default)]
        trigger: String,
        #[serde(default)]
        command: String,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        line: Option<u32>,
        #[serde(default, rename = "bytecodeSize")]
        bytecode_size: usize,
        #[serde(default, rename = "instructionCount")]
        instruction_count: Option<usize>,
        #[serde(default, rename = "literalCount")]
        literal_count: Option<usize>,
        #[serde(default)]
        disassembly: Option<String>,
        #[serde(default, rename = "oldEpoch")]
        old_epoch: Option<u32>,
        #[serde(default, rename = "newEpoch")]
        new_epoch: Option<u32>,
    },
    /// Forward-compat catch-all: any event kind this build does not know
    /// (including kinds newer than the reserved set above) deserializes to
    /// this variant instead of failing the parse. The actor logs and
    /// ignores it, so an old GUI keeps working against a newer agent.
    #[serde(other)]
    Unknown,
}

impl AgentEvent {
    /// Parse an agent event line, tolerating event kinds this build does
    /// not know: they deserialize to [`AgentEvent::Unknown`] instead of
    /// failing (mirrors the wire protocol's "unknown tokens must be
    /// accepted silently" rule). Payload errors on KNOWN kinds still fail
    /// so real protocol bugs stay visible.
    ///
    /// The derive alone cannot do this: `#[serde(other)]` only matches
    /// unit variants, so an unknown tag that carries a `data` payload is
    /// rejected. On failure we strip `data` and retry — an unknown tag
    /// then resolves to `Unknown`, while a known tag reproduces a missing
    /// -field error and the original error is returned.
    pub fn parse_tolerant(raw: &str) -> Result<AgentEvent, serde_json::Error> {
        let err = match serde_json::from_str::<AgentEvent>(raw) {
            Ok(event) => return Ok(event),
            Err(err) => err,
        };
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return Err(err); // malformed JSON — keep the original error
        };
        if let Some(obj) = value.as_object_mut() {
            obj.remove("data");
        }
        match serde_json::from_value::<AgentEvent>(value) {
            Ok(AgentEvent::Unknown) => Ok(AgentEvent::Unknown),
            _ => Err(err),
        }
    }
}

/// Per-source-line profile bucket for the reserved `profile_data` event
/// (gated:opcode-profiling-runtime). Timing fields are only populated in
/// `line_timing` mode.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LineProfileEntry {
    pub file: String,
    pub line: u32,
    pub count: u64,
    pub total_ns: Option<u64>,
    pub min_ns: Option<u64>,
    pub max_ns: Option<u64>,
}

/// Per-instruction profile bucket for the reserved `profile_data` event
/// (gated:opcode-profiling-runtime).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct OpcodeProfileEntry {
    pub pc: u32,
    pub count: u64,
}

// ─── DTOs emitted to frontend via Tauri events ───────────────────────

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DebugStoppedEvent {
    pub reason: String,
    pub file: String,
    pub line: u32,
    pub frame_count: u32,
    pub pc: Option<i32>,
    pub instruction_index: Option<usize>,
    pub block_id: Option<usize>,
    pub error: Option<String>,
    pub error_info: Option<String>,
    pub watchpoint_name: Option<String>,
    pub watchpoint_element: Option<String>,
    pub watchpoint_old_value: Option<String>,
    pub watchpoint_new_value: Option<String>,
    pub watchpoint_operation: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DebugOutputEvent {
    pub stream: String,
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DebugExitedEvent {
    pub code: i32,
    pub reason: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DebugReadyEvent {
    pub tcl_version: String,
    pub pid: u32,
    pub debug_extension_loaded: Option<bool>,
    pub debug_extension_installed: Option<bool>,
    pub debug_extension_capabilities: Option<String>,
    pub debug_extension_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DebugBytecodeChunkEvent {
    pub sequence: u32,
    pub command: String,
    pub proc_name: Option<String>,
    pub payload_size: usize,
    pub disassembly: Option<String>,
    pub decompile_text: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Protocol type — used in later tasks
pub struct DebugResumedEvent {
    pub control_mode: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Protocol type — used in later tasks
pub struct WatchpointInfo {
    pub id: u64,
    pub name: String,
    pub scope: String,
    pub level: Option<u32>,
    pub condition: Option<String>,
    pub active: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Protocol type — used in later tasks
pub struct DebugWatchpointHitEvent {
    pub watch_id: u64,
    pub name: String,
    pub element: Option<String>,
    pub operation: String,
    pub old_value: String,
    pub new_value: String,
    pub file: String,
    pub line: u32,
    pub frame_count: u32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DebugCommandErrorEvent {
    pub command: String,
    pub control_mode: String,
    pub code: i32,
    pub message: String,
    pub error_info: Option<String>,
    pub error_code: Option<String>,
    pub argc: u32,
}

// ─── Reserved v0.4+ frontend DTOs ────────────────────────────────────
// Payloads for the reserved Tauri event channels. One per gated feature;
// each mirrors its AgentEvent variant above. Nothing emits these in v1 —
// they exist so the v0.4 runtimes land without a DTO revision. All fields
// are defaulted so partial payloads from a newer backend stay parseable.

/// Reserved DTO for `debugger:disassembly_context`
/// (`gated:live-disassembly-runtime`). Nothing emits it in v1.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DebugDisassemblyContextEvent {
    pub proc_name: Option<String>,
    pub source_file: Option<String>,
    pub disassembly: String,
    pub pc_to_line: Vec<(u32, u32)>,
}

/// Reserved DTO for `debugger:profile_data`
/// (`gated:opcode-profiling-runtime`).
/// Nothing emits it in v1.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DebugProfileDataEvent {
    pub request_id: u64,
    pub mode: String,
    pub line_data: Option<Vec<LineProfileEntry>>,
    pub opcode_data: Option<Vec<OpcodeProfileEntry>>,
}

/// Reserved DTO for `debugger:epoch_change`
/// (`gated:epoch-tracking-runtime`). Nothing emits it in v1.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DebugEpochChangeEvent {
    pub command: String,
    pub old_epoch: u32,
    pub new_epoch: u32,
    pub trigger: String,
    pub compile_epoch: u32,
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// Reserved DTO for `debugger:compilation_event`
/// (`gated:compilation-introspection-runtime`). Nothing emits it in v1.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DebugCompilationEvent {
    pub trigger: String,
    pub command: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub bytecode_size: usize,
    pub instruction_count: Option<usize>,
    pub literal_count: Option<usize>,
    pub disassembly: Option<String>,
    pub old_epoch: Option<u32>,
    pub new_epoch: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Breakpoint {
    pub file: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub len: Option<u64>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Protocol type — deserialized from agent events
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "type")]
    pub var_type: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_rep: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_rep_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_string_rep: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_count: Option<u32>,
    #[serde(default)]
    pub children: Option<Vec<Variable>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Protocol type — deserialized from agent events
pub struct StackFrame {
    pub level: u32,
    #[serde(rename = "type")]
    pub frame_type: String,
    pub proc_name: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub cmd: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EvalResult {
    pub result: String,
    pub error: bool,
    pub error_info: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommandResolution {
    pub original_name: String,
    pub resolved_name: String,
    pub fully_qualified_name: String,
    pub namespace: String,
    pub resolved_in_namespace: String,
    pub is_imported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_source: Option<String>,
    pub import_chain: Vec<String>,
    pub is_ensemble: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ensemble_namespace: Option<String>,
    pub is_alias: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_target: Option<String>,
    pub command_type: String,
    pub exists: bool,
    pub resolution_path: Vec<ResolutionStep>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionStep {
    pub step: String,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub source: Option<String>,
    pub target: Option<String>,
    pub result: String,
}

// ─── Reserved event vocabulary tests ─────────────────────────────────
// C1 forward-visibility contract: every reserved v0.4+ event kind must
// round-trip through the AgentEvent envelope, tolerate minimal payloads
// (draft plans may still drop fields), and any kind this build has never
// heard of must deserialize to `Unknown` instead of failing the parse.

#[cfg(test)]
mod reserved_event_vocabulary_tests {
    use super::*;
    use serde_json::json;

    /// Serialize → deserialize → serialize must reproduce the input JSON.
    fn assert_round_trip(payload: serde_json::Value) {
        let event: AgentEvent = serde_json::from_value(payload.clone())
            .unwrap_or_else(|e| panic!("deserialize failed for {payload}: {e}"));
        let back = serde_json::to_value(&event).expect("reserialize");
        assert_eq!(back, payload);
    }

    #[test]
    fn reserved_disassembly_context_round_trips() {
        assert_round_trip(json!({
            "type": "disassembly_context",
            "data": {
                "procName": "::foo",
                "sourceFile": "/tmp/app.tcl",
                "disassembly": "Command 1: \"set a 1\"\n  (0) push1 0\n",
                "pcToLine": [[0, 1], [5, 2]],
            },
        }));
    }

    #[test]
    fn reserved_profile_data_round_trips() {
        assert_round_trip(json!({
            "type": "profile_data",
            "data": {
                "requestId": 7,
                "mode": "line_timing",
                "lineData": [{
                    "file": "/tmp/app.tcl",
                    "line": 3,
                    "count": 1000,
                    "totalNs": 12_500_000,
                    "minNs": 800,
                    "maxNs": 90_000,
                }],
                "opcodeData": [{"pc": 4, "count": 1000}],
            },
        }));
    }

    #[test]
    fn reserved_epoch_change_round_trips() {
        assert_round_trip(json!({
            "type": "epoch_change",
            "data": {
                "command": "::foo",
                "oldEpoch": 2,
                "newEpoch": 3,
                "trigger": "proc_redefine",
                "compileEpoch": 47,
                "file": "/tmp/utils.tcl",
                "line": 12,
            },
        }));
    }

    #[test]
    fn reserved_compilation_event_round_trips() {
        assert_round_trip(json!({
            "type": "compilation_event",
            "data": {
                "trigger": "recompile",
                "command": "::bar",
                "file": "/tmp/main.tcl",
                "line": 5,
                "bytecodeSize": 192,
                "instructionCount": 56,
                "literalCount": 7,
                "disassembly": "ByteCode 0x0 ...",
                "oldEpoch": 3,
                "newEpoch": 4,
            },
        }));
    }

    #[test]
    fn reserved_kinds_tolerate_minimal_payloads() {
        // Draft-plan looseness: a future agent may omit any field. Every
        // reserved kind must deserialize from an empty payload object.
        let disasm: AgentEvent =
            serde_json::from_value(json!({"type": "disassembly_context", "data": {}}))
                .expect("minimal disassembly_context");
        assert!(matches!(
            disasm,
            AgentEvent::DisassemblyContext { proc_name: None, disassembly, .. } if disassembly.is_empty()
        ));

        let profile: AgentEvent =
            serde_json::from_value(json!({"type": "profile_data", "data": {}}))
                .expect("minimal profile_data");
        assert!(matches!(
            profile,
            AgentEvent::ProfileData {
                request_id: 0,
                line_data: None,
                opcode_data: None,
                ..
            }
        ));

        let epoch: AgentEvent = serde_json::from_value(json!({"type": "epoch_change", "data": {}}))
            .expect("minimal epoch_change");
        assert!(matches!(
            epoch,
            AgentEvent::EpochChange {
                old_epoch: 0,
                new_epoch: 0,
                file: None,
                line: None,
                ..
            }
        ));

        let compile: AgentEvent =
            serde_json::from_value(json!({"type": "compilation_event", "data": {}}))
                .expect("minimal compilation_event");
        assert!(matches!(
            compile,
            AgentEvent::CompilationEvent {
                bytecode_size: 0,
                instruction_count: None,
                ..
            }
        ));
    }

    #[test]
    fn unknown_event_kind_deserializes_to_catch_all() {
        // With a payload: the derive alone rejects this (`other` is
        // unit-only), so the bridge's entry point is parse_tolerant.
        let with_payload = AgentEvent::parse_tolerant(
            r#"{"type": "kind_this_build_has_never_heard_of", "data": {"whatever": true, "nested": {"x": 1}}}"#,
        )
        .expect("unknown kind with payload must parse");
        assert!(matches!(with_payload, AgentEvent::Unknown));

        // Without a payload: the plain derive already tolerates it.
        let without_payload: AgentEvent =
            serde_json::from_value(json!({"type": "another_future_kind"}))
                .expect("unknown kind without payload must parse");
        assert!(matches!(without_payload, AgentEvent::Unknown));
    }

    #[test]
    fn tolerant_parse_does_not_mask_errors_on_known_kinds() {
        // A KNOWN kind with a broken payload must still fail — forward
        // tolerance is for unknown vocabulary, not for protocol bugs.
        let broken_known = AgentEvent::parse_tolerant(
            r#"{"type": "ready", "data": {"tclVersion": "9.0.2", "pid": "not-a-number"}}"#,
        );
        assert!(broken_known.is_err());

        // Malformed JSON keeps its original parse error too.
        assert!(AgentEvent::parse_tolerant("{not json").is_err());
    }

    #[test]
    fn known_kinds_still_deserialize_precisely() {
        // The catch-all must not swallow the existing vocabulary.
        let ready: AgentEvent = serde_json::from_value(json!({
            "type": "ready",
            "data": {"tclVersion": "9.0.2", "pid": 4242},
        }))
        .expect("ready event");
        assert!(matches!(
            ready,
            AgentEvent::Ready { tcl_version, pid: 4242, .. } if tcl_version == "9.0.2"
        ));
    }

    /// Reserved frontend DTOs round-trip through their camelCase wire shape
    /// and tolerate empty payloads (struct-level `#[serde(default)]`).
    #[test]
    fn reserved_frontend_dtos_round_trip_and_default() {
        let disasm = json!({
            "procName": "::foo",
            "sourceFile": "/tmp/app.tcl",
            "disassembly": "  (0) push1 0\n",
            "pcToLine": [[0, 1]],
        });
        let typed: DebugDisassemblyContextEvent = serde_json::from_value(disasm.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), disasm);

        let profile = json!({
            "requestId": 1,
            "mode": "opcode_counts",
            "lineData": null,
            "opcodeData": [{"pc": 2, "count": 9}],
        });
        let typed: DebugProfileDataEvent = serde_json::from_value(profile.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), profile);

        let epoch = json!({
            "command": "::foo",
            "oldEpoch": 1,
            "newEpoch": 2,
            "trigger": "rename",
            "compileEpoch": 9,
            "file": null,
            "line": null,
        });
        let typed: DebugEpochChangeEvent = serde_json::from_value(epoch.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), epoch);

        let compile = json!({
            "trigger": "first_call",
            "command": "::foo",
            "file": null,
            "line": null,
            "bytecodeSize": 128,
            "instructionCount": null,
            "literalCount": null,
            "disassembly": null,
            "oldEpoch": null,
            "newEpoch": null,
        });
        let typed: DebugCompilationEvent = serde_json::from_value(compile.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), compile);

        // Empty-object tolerance for all four DTOs.
        let _: DebugDisassemblyContextEvent = serde_json::from_value(json!({})).unwrap();
        let _: DebugProfileDataEvent = serde_json::from_value(json!({})).unwrap();
        let _: DebugEpochChangeEvent = serde_json::from_value(json!({})).unwrap();
        let _: DebugCompilationEvent = serde_json::from_value(json!({})).unwrap();
    }
}
