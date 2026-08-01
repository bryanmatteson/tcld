pub mod bridge;
pub mod core;
pub mod embedded;
#[cfg(test)]
mod production_readiness_tests;
pub mod protocol;
pub mod remote;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use bridge::Bridge;
use core::{ResumeAction, RunControlCore, StopReason};
use embedded::EmbeddedBackend;
use protocol::*;
use tauri::Emitter;

// ─── Debug state machine ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugState {
    Idle,
    Launching,
    Connected,
    Running,
    Pausing,
    Paused,
    Stopping,
    Terminated,
}

fn is_active(s: DebugState) -> bool {
    matches!(
        s,
        DebugState::Launching
            | DebugState::Connected
            | DebugState::Running
            | DebugState::Pausing
            | DebugState::Paused
    )
}

// ─── Commands from Tauri handlers to actor ───────────────────────────

pub enum DebugCmd {
    Launch {
        path: String,
        args: Vec<String>,
        tclsh_path: Option<String>,
        launch_options: DebugLaunchOptions,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Stop {
        reply: oneshot::Sender<Result<(), String>>,
    },
    StepInto {
        reply: oneshot::Sender<Result<(), String>>,
    },
    StepInstruction {
        reply: oneshot::Sender<Result<(), String>>,
    },
    StepOverInstruction {
        reply: oneshot::Sender<Result<(), String>>,
    },
    StepOutInstruction {
        reply: oneshot::Sender<Result<(), String>>,
    },
    RunToPc {
        pc: u32,
        reply: oneshot::Sender<Result<(), String>>,
    },
    StepOver {
        reply: oneshot::Sender<Result<(), String>>,
    },
    StepOut {
        reply: oneshot::Sender<Result<(), String>>,
    },
    Continue {
        reply: oneshot::Sender<Result<(), String>>,
    },
    Pause {
        reply: oneshot::Sender<Result<(), String>>,
    },
    SetBreakpoint {
        file: String,
        line: u32,
        offset: Option<u64>,
        len: Option<u64>,
        condition: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SetBreakpointEnabled {
        file: String,
        line: u32,
        offset: Option<u64>,
        enabled: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ClearBreakpoint {
        file: String,
        line: u32,
        offset: Option<u64>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    #[allow(dead_code)]
    ClearAllBreakpoints {
        reply: oneshot::Sender<Result<(), String>>,
    },
    ListBreakpoints {
        reply: oneshot::Sender<Vec<Breakpoint>>,
    },
    Eval {
        script: String,
        level: u32,
        reply: oneshot::Sender<Result<(), String>>,
    },
    EvalSync {
        script: String,
        level: u32,
        reply: oneshot::Sender<Result<EvalResult, String>>,
    },
    GetVariables {
        level: u32,
    },
    GetStack,
    SetVariable {
        name: String,
        value: String,
        level: u32,
        reply: oneshot::Sender<Result<(), String>>,
    },
    GetState {
        reply: oneshot::Sender<serde_json::Value>,
    },
    GetBytecodeChunks {
        reply: oneshot::Sender<Vec<DebugBytecodeChunkEvent>>,
    },
    Disassemble {
        path: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    ResolveCommand {
        name: String,
        namespace: Option<String>,
        reply: oneshot::Sender<Result<CommandResolution, String>>,
    },
    SetWatchpoint {
        name: String,
        scope: Option<String>,
        level: Option<u32>,
        condition: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ClearWatchpoint {
        name: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ListWatchpoints {
        reply: oneshot::Sender<Vec<WatchpointInfo>>,
    },
    ClearAllWatchpoints {
        reply: oneshot::Sender<Result<(), String>>,
    },
}

pub type CmdTx = mpsc::Sender<DebugCmd>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BytecodeHookMode {
    #[default]
    Passthrough,
    Intercept,
}

impl BytecodeHookMode {
    fn as_cli_value(self) -> &'static str {
        match self {
            Self::Passthrough => "passthrough",
            Self::Intercept => "intercept",
        }
    }
}

/// Which host-debugger transport a native desktop launch should use.
///
/// This is not the CLI Auto/Host/Hybrid/VmStrict execution policy. The native
/// desktop debugger executes Tcl source in a host interpreter; Hybrid/VmStrict
/// source-image compilation is exposed by a hybrid-capable CLI build.
///
/// - `Embedded` (default): host Tcl interpreter on a dedicated thread
///   (`embedded.rs`).
/// - `External`: spawns an external `tclsh` running the tcld nub and
///   drives it over the tcld-debug-wire protocol (`remote.rs`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugBackendKind {
    #[default]
    Embedded,
    External,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugLaunchOptions {
    #[serde(default)]
    pub bytecode_hook_mode: BytecodeHookMode,
    #[serde(default)]
    pub bytecode_hook_commands: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Host transport selector; `embedded` when omitted.
    #[serde(default)]
    pub backend: DebugBackendKind,
    /// External-backend override for the `tclsh` binary (falls back to
    /// the top-level `tclshPath` launch argument, then `TCLD_TCLSH`,
    /// then PATH discovery).
    #[serde(default)]
    pub tclsh_path: Option<String>,
    /// External-backend override for the `libtcld_dbg` nub dylib
    /// (falls back to `TCLD_DBG_DYLIB`, then workspace/bundle discovery).
    #[serde(default)]
    pub dylib_path: Option<String>,
}

impl Default for DebugLaunchOptions {
    fn default() -> Self {
        Self {
            bytecode_hook_mode: BytecodeHookMode::Passthrough,
            bytecode_hook_commands: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            backend: DebugBackendKind::Embedded,
            tclsh_path: None,
            dylib_path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    Embedded,
    /// The legacy agent-script bridge (`tcld-agent.tcl` over a JSON TCP
    /// bridge), selected by the top-level `tclshPath` launch argument
    /// when no explicit backend is requested.
    External,
    /// The tcld-debug-wire nub backend (`remote.rs`), selected by
    /// `DebugLaunchOptions::backend == DebugBackendKind::External`.
    Wire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendFeature {
    BytecodeHookInterceptMode,
    BytecodeHookCommandOverrides,
}

// `pub` (not `pub(crate)`) because `EmbeddedBackend::launch` takes it and
// the debugger module is publicly visible from the library crate; the
// fields stay private so the type is still constructed only here.
#[derive(Debug, Clone)]
pub struct HookPlan {
    mode: BytecodeHookMode,
    commands: Vec<String>,
}

impl HookPlan {
    fn new(mode: BytecodeHookMode, commands: Vec<String>) -> Self {
        Self { mode, commands }
    }
}

impl Default for HookPlan {
    fn default() -> Self {
        Self::new(
            BytecodeHookMode::Passthrough,
            default_bytecode_hook_commands(),
        )
    }
}

#[derive(Debug, Clone)]
struct EmbeddedLaunchPlan {
    path: String,
    args: Vec<String>,
    breakpoints: Vec<(String, u32, Option<String>)>,
    hook_plan: HookPlan,
    debug_extension_path: Option<String>,
    env: HashMap<String, String>,
    cwd: Option<String>,
}

#[derive(Debug, Clone)]
struct ExternalLaunchPlan {
    path: String,
    args: Vec<String>,
    tclsh: String,
    hook_plan: HookPlan,
    debug_extension_path: Option<String>,
    env: HashMap<String, String>,
    cwd: Option<String>,
    agent_path: String,
    port: u16,
}

fn default_bytecode_hook_commands() -> Vec<String> {
    vec![
        "::tbcload::bceval".into(),
        "::tbcload::bcproc".into(),
        "::tbceval".into(),
        "::tbcproc".into(),
    ]
}

fn normalize_hook_command_name(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("::") {
        Some(trimmed.to_string())
    } else {
        Some(format!("::{trimmed}"))
    }
}

fn normalize_hook_commands(commands: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for command in commands
        .iter()
        .filter_map(|cmd| normalize_hook_command_name(cmd))
    {
        if !normalized.contains(&command) {
            normalized.push(command);
        }
    }
    if normalized.is_empty() {
        return default_bytecode_hook_commands();
    }
    normalized
}

fn backend_supports_feature(backend: BackendKind, feature: BackendFeature) -> bool {
    match backend {
        BackendKind::Embedded => match feature {
            BackendFeature::BytecodeHookInterceptMode => true,
            BackendFeature::BytecodeHookCommandOverrides => true,
        },
        BackendKind::External => match feature {
            BackendFeature::BytecodeHookInterceptMode => true,
            BackendFeature::BytecodeHookCommandOverrides => true,
        },
        // The wire nub bootstrap has no bytecode-hook surface; requesting
        // hook features with the external (wire) backend is a defined
        // launch error rather than a silent no-op.
        BackendKind::Wire => match feature {
            BackendFeature::BytecodeHookInterceptMode => false,
            BackendFeature::BytecodeHookCommandOverrides => false,
        },
    }
}

fn ensure_paused(state: DebugState) -> Result<(), String> {
    if state == DebugState::Paused {
        Ok(())
    } else {
        Err(format!("not paused (state: {:?})", state))
    }
}

fn escape_tcl_braced(value: &str) -> String {
    value.replace('{', "\\{").replace('}', "\\}")
}

fn hook_feature_backend_label(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Embedded => "embedded debugger",
        BackendKind::External => "external agent debugger",
        BackendKind::Wire => "external debugger backend",
    }
}

fn resolve_hook_plan(
    backend: BackendKind,
    options: DebugLaunchOptions,
) -> Result<HookPlan, String> {
    if options.bytecode_hook_mode == BytecodeHookMode::Intercept
        && !backend_supports_feature(backend, BackendFeature::BytecodeHookInterceptMode)
    {
        return Err(format!(
            "{} does not support bytecodeHookMode=intercept",
            hook_feature_backend_label(backend)
        ));
    }

    let commands = normalize_hook_commands(&options.bytecode_hook_commands);
    let default_commands = default_bytecode_hook_commands();
    let explicitly_requested_overrides = !options.bytecode_hook_commands.is_empty();
    if explicitly_requested_overrides
        && commands != default_commands
        && !backend_supports_feature(backend, BackendFeature::BytecodeHookCommandOverrides)
    {
        return Err(format!(
            "{} does not support custom bytecodeHookCommands",
            hook_feature_backend_label(backend)
        ));
    }

    Ok(HookPlan::new(options.bytecode_hook_mode, commands))
}

fn build_embedded_launch_plan(
    breakpoints: &[Breakpoint],
    path: String,
    args: Vec<String>,
    hook_plan: HookPlan,
    debug_extension_path: Option<String>,
    launch_options: DebugLaunchOptions,
) -> EmbeddedLaunchPlan {
    let breakpoints = breakpoints
        .iter()
        // Command ranges are replayed through AgentCommand after Ready; the
        // legacy launch tuple cannot represent their offset/length and would
        // otherwise install an extra line-wide breakpoint.
        .filter(|bp| bp.enabled && bp.offset.is_none())
        .map(|bp| (bp.file.clone(), bp.line, bp.condition.clone()))
        .collect();
    EmbeddedLaunchPlan {
        path,
        args,
        breakpoints,
        hook_plan,
        debug_extension_path,
        env: launch_options.env,
        cwd: launch_options.cwd,
    }
}

// Pure plan constructor mirroring the ExternalLaunchPlan fields one-to-one;
// bundling arguments would just duplicate the struct it builds.
#[allow(clippy::too_many_arguments)]
fn build_external_launch_plan(
    path: String,
    args: Vec<String>,
    tclsh_path: Option<String>,
    hook_plan: HookPlan,
    debug_extension_path: Option<String>,
    launch_options: DebugLaunchOptions,
    agent_path: String,
    port: u16,
) -> ExternalLaunchPlan {
    ExternalLaunchPlan {
        path,
        args,
        tclsh: tclsh_path.unwrap_or_else(|| "tclsh".into()),
        hook_plan,
        debug_extension_path,
        env: launch_options.env,
        cwd: launch_options.cwd,
        agent_path,
        port,
    }
}

fn command_for_external_launch(plan: &ExternalLaunchPlan) -> Command {
    let mut cmd = Command::new(&plan.tclsh);
    if let Some(cwd) = plan.cwd.as_deref() {
        cmd.current_dir(cwd);
    }
    cmd.envs(plan.env.iter());
    cmd.arg(&plan.agent_path)
        .arg("--port")
        .arg(plan.port.to_string())
        .arg("--bytecode-hook-mode")
        .arg(plan.hook_plan.mode.as_cli_value());
    for command_name in plan.hook_plan.commands.iter().filter_map(|cmd| {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }) {
        cmd.arg("--bytecode-hook-command").arg(command_name);
    }
    if let Some(extension_path) = plan.debug_extension_path.as_deref() {
        cmd.arg("--debug-extension-path").arg(extension_path);
    }
    cmd.arg("--script").arg(&plan.path);
    for arg in &plan.args {
        cmd.arg(arg);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd
}

fn set_breakpoint_enabled_state(
    breakpoints: &mut [Breakpoint],
    file: String,
    line: u32,
    offset: Option<u64>,
    enabled: bool,
) -> Result<(), String> {
    let file = canonicalize_debug_path(file);
    let bp = breakpoints
        .iter_mut()
        .find(|bp| bp.file == file && bp.line == line && bp.offset == offset)
        .ok_or_else(|| format!("breakpoint not found: {file}:{line}"))?;
    bp.enabled = enabled;
    Ok(())
}

fn build_get_variables_command(
    state: DebugState,
    request_id: u64,
    level: u32,
) -> Result<AgentCommand, String> {
    ensure_paused(state)?;
    Ok(AgentCommand::GetVariables { request_id, level })
}

fn build_get_stack_command(state: DebugState, request_id: u64) -> Result<AgentCommand, String> {
    ensure_paused(state)?;
    Ok(AgentCommand::GetStack { request_id })
}

fn build_paused_snapshot_commands(
    state: DebugState,
    stack_request_id: u64,
    variables_request_id: u64,
    variables_level: u32,
) -> Result<Vec<AgentCommand>, String> {
    Ok(vec![
        build_get_stack_command(state, stack_request_id)?,
        build_get_variables_command(state, variables_request_id, variables_level)?,
    ])
}

fn build_eval_command(
    state: DebugState,
    request_id: u64,
    script: String,
    level: u32,
) -> Result<AgentCommand, String> {
    ensure_paused(state)?;
    Ok(AgentCommand::Eval {
        request_id,
        script,
        level,
    })
}

fn build_resolve_command(
    state: DebugState,
    request_id: u64,
    name: String,
    namespace: Option<String>,
) -> Result<AgentCommand, String> {
    ensure_paused(state)?;
    let ns_arg = namespace.as_deref().unwrap_or("");
    Ok(AgentCommand::Eval {
        request_id,
        script: format!(
            "::tcld::dbg::resolve_command {{{}}} {{{}}}",
            escape_tcl_braced(&name),
            escape_tcl_braced(ns_arg),
        ),
        level: 0,
    })
}

fn normalize_watchpoint_scope(scope: Option<String>) -> Result<String, String> {
    match scope
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("local")
    {
        "local" => Ok("local".to_string()),
        "global" => Ok("global".to_string()),
        "namespace" => Ok("namespace".to_string()),
        other => Err(format!(
            "invalid watchpoint scope '{other}' (expected local, global, or namespace)"
        )),
    }
}

fn build_set_watchpoint_command(
    state: DebugState,
    request_id: u64,
    watchpoint_id: u64,
    name: String,
    scope: Option<String>,
    level: Option<u32>,
    condition: Option<String>,
) -> Result<(AgentCommand, WatchpointInfo), String> {
    ensure_paused(state)?;
    let scope_str = normalize_watchpoint_scope(scope)?;
    let mut script_parts = vec![format!(
        "::tcld::dbg::watch_variable {{{}}} -scope {{{}}}",
        escape_tcl_braced(&name),
        scope_str,
    )];
    if let Some(lvl) = level {
        script_parts.push(format!("-level {lvl}"));
    }
    if let Some(ref cond) = condition {
        script_parts.push(format!("-condition {{{}}}", escape_tcl_braced(cond)));
    }
    let watchpoint = WatchpointInfo {
        id: watchpoint_id,
        name,
        scope: scope_str,
        level,
        condition,
        active: true,
    };
    Ok((
        AgentCommand::Eval {
            request_id,
            script: script_parts.join(" "),
            level: 0,
        },
        watchpoint,
    ))
}

fn build_clear_watchpoint_command(
    state: DebugState,
    request_id: u64,
    name: String,
) -> Result<AgentCommand, String> {
    ensure_paused(state)?;
    Ok(AgentCommand::Eval {
        request_id,
        script: format!(
            "::tcld::dbg::unwatch_variable {{{}}}",
            escape_tcl_braced(&name)
        ),
        level: 0,
    })
}

fn build_clear_all_watchpoints_command(
    state: DebugState,
    request_id: u64,
) -> Result<AgentCommand, String> {
    ensure_paused(state)?;
    Ok(AgentCommand::Eval {
        request_id,
        script: "::tcld::dbg::clear_all_watchpoints".to_string(),
        level: 0,
    })
}

struct StoppedEventParts {
    reason: String,
    file: String,
    line: u32,
    frame_count: u32,
    pc: Option<i32>,
    instruction_index: Option<usize>,
    block_id: Option<usize>,
    error: Option<String>,
    error_info: Option<String>,
}

fn build_debug_stopped_event(
    parts: StoppedEventParts,
    watchpoint_meta: Option<&DebugWatchpointHitEvent>,
) -> DebugStoppedEvent {
    DebugStoppedEvent {
        reason: parts.reason,
        file: parts.file,
        line: parts.line,
        frame_count: parts.frame_count,
        pc: parts.pc,
        instruction_index: parts.instruction_index,
        block_id: parts.block_id,
        error: parts.error,
        error_info: parts.error_info,
        watchpoint_name: watchpoint_meta.map(|h| h.name.clone()),
        watchpoint_element: watchpoint_meta.and_then(|h| h.element.clone()),
        watchpoint_old_value: watchpoint_meta.map(|h| h.old_value.clone()),
        watchpoint_new_value: watchpoint_meta.map(|h| h.new_value.clone()),
        watchpoint_operation: watchpoint_meta.map(|h| h.operation.clone()),
    }
}

#[cfg(test)]
mod launch_option_tests {
    use super::{BackendKind, BytecodeHookMode, DebugLaunchOptions, resolve_hook_plan};
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SCRIPT_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn embedded_supports_intercept_mode() {
        let options = DebugLaunchOptions {
            bytecode_hook_mode: BytecodeHookMode::Intercept,
            bytecode_hook_commands: Vec::new(),
            ..Default::default()
        };
        let plan = resolve_hook_plan(BackendKind::Embedded, options).unwrap();
        assert_eq!(plan.mode, BytecodeHookMode::Intercept);
        assert!(!plan.commands.is_empty());
    }

    #[test]
    fn embedded_supports_custom_hook_commands() {
        let options = DebugLaunchOptions {
            bytecode_hook_mode: BytecodeHookMode::Passthrough,
            bytecode_hook_commands: vec!["custom_loader".into()],
            ..Default::default()
        };
        let plan = resolve_hook_plan(BackendKind::Embedded, options).unwrap();
        assert_eq!(plan.commands, vec!["::custom_loader"]);
    }

    #[test]
    fn external_normalizes_and_dedupes_hook_commands() {
        let options = DebugLaunchOptions {
            bytecode_hook_mode: BytecodeHookMode::Passthrough,
            bytecode_hook_commands: vec![
                "tbcload::bceval".into(),
                " ::tbcload::bceval ".into(),
                "".into(),
                "tbcproc".into(),
            ],
            ..Default::default()
        };
        let plan = resolve_hook_plan(BackendKind::External, options).unwrap();
        assert_eq!(plan.commands, vec!["::tbcload::bceval", "::tbcproc"]);
    }

    fn make_temp_script(name: &str, content: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let counter = TEMP_SCRIPT_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!(
            "tcld-external-{name}-{}-{nanos}-{counter}.tcl",
            std::process::id()
        ));
        std::fs::write(&path, content).expect("write script");
        path
    }

    fn run_tcl_script(script_content: &str) -> Result<String, String> {
        let script = make_temp_script("hook-support", script_content);
        let output = match Command::new("tclsh").arg(&script).output() {
            Ok(output) => output,
            Err(e) => {
                let _ = std::fs::remove_file(script);
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Err("tclsh not found".into());
                }
                return Err(format!("failed to run tclsh: {e}"));
            }
        };
        let _ = std::fs::remove_file(script);

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!(
                "tcl script failed (status {}): {}{}",
                output.status, stdout, stderr
            ))
        }
    }

    fn hook_support_path() -> Result<PathBuf, String> {
        let agent = super::agent_script_path()?;
        let mut hooks = PathBuf::from(agent);
        hooks.set_file_name("tcld-bytecode-hooks.tcl");
        Ok(hooks)
    }

    fn debug_extension_support_path() -> Result<PathBuf, String> {
        let agent = super::agent_script_path()?;
        let mut support = PathBuf::from(agent);
        support.set_file_name("tcld-debug-extension.tcl");
        Ok(support)
    }

    #[test]
    fn external_hook_support_normalizes_commands() {
        let hooks = hook_support_path().expect("hook support path");
        let out = match run_tcl_script(&format!(
            "source {{{}}}\n\
             set cmds [::tcld_bytecode_hooks::normalize_hook_commands {{tbcload::bceval ::tbcload::bceval {{}} tbcproc}}]\n\
             puts [join $cmds \",\"]\n",
            hooks.display()
        )) {
            Ok(out) => out,
            Err(err) if err == "tclsh not found" => return,
            Err(err) => panic!("{err}"),
        };
        assert_eq!(out.trim(), "::tbcload::bceval,::tbcproc");
    }

    #[test]
    fn external_hook_support_intercept_shims_bceval() {
        let hooks = hook_support_path().expect("hook support path");
        let out = match run_tcl_script(&format!(
            "source {{{}}}\n\
             proc ::tbceval {{payload}} {{ return \"OK:$payload\" }}\n\
             ::tcld_bytecode_hooks::ensure_intercept_loader_shims\n\
             puts [::tbcload::bceval PAYLOAD]\n",
            hooks.display()
        )) {
            Ok(out) => out,
            Err(err) if err == "tclsh not found" => return,
            Err(err) => panic!("{err}"),
        };
        assert!(out.contains("OK:PAYLOAD"), "unexpected output: {out}");
    }

    #[test]
    fn external_debug_extension_support_reports_missing_file() {
        let support = debug_extension_support_path().expect("debug extension support path");
        let out = match run_tcl_script(&format!(
            "source {{{}}}\n\
             set r [::tcld_debug_extension::load_and_install /tmp/tcld-missing-debug-extension.dylib]\n\
             puts \"[dict get $r loaded],[dict get $r installed],[dict get $r reason]\"\n",
            support.display()
        )) {
            Ok(out) => out,
            Err(err) if err == "tclsh not found" => return,
            Err(err) => panic!("{err}"),
        };
        assert_eq!(out.trim(), "0,0,missing_file");
    }
}

// ─── Pending eval with timestamp ─────────────────────────────────────

enum PendingEval {
    Sync(oneshot::Sender<EvalResult>),
    Disassemble(oneshot::Sender<Result<String, String>>),
    SetWatchpoint {
        reply: oneshot::Sender<Result<(), String>>,
        watchpoint: WatchpointInfo,
    },
    ClearWatchpoint {
        reply: oneshot::Sender<Result<(), String>>,
        name: String,
    },
    ClearAllWatchpoints {
        reply: oneshot::Sender<Result<(), String>>,
    },
}

struct PendingRequest {
    sender: PendingEval,
    created: Instant,
}

#[derive(Debug, Clone, Copy)]
struct StopMetadata {
    pc: i32,
    instruction_index: usize,
    block_id: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct SourceLineSpan {
    line_start: u32,
    line_end: u32,
    pc: i32,
    instruction_index: usize,
    block_id: Option<usize>,
}

#[derive(Debug, Clone, Default)]
struct StopMetadataIndex {
    spans: Vec<SourceLineSpan>,
    by_pc: HashMap<i32, StopMetadata>,
}

impl StopMetadataIndex {
    fn clear(&mut self) {
        self.spans.clear();
        self.by_pc.clear();
    }

    fn ingest(&mut self, disassembly: &str, source: &str) {
        for (pc, instruction_index, block_id) in parse_disassembly_instruction_points(disassembly) {
            self.by_pc.insert(
                pc,
                StopMetadata {
                    pc,
                    instruction_index,
                    block_id,
                },
            );
        }

        let mut parsed = parse_disassembly_source_spans(disassembly, source);
        if parsed.is_empty() {
            return;
        }
        self.spans.append(&mut parsed);
        self.spans
            .sort_by_key(|s| (s.line_start, s.line_end, s.instruction_index));
        self.spans.dedup_by(|a, b| {
            a.line_start == b.line_start
                && a.line_end == b.line_end
                && a.instruction_index == b.instruction_index
                && a.pc == b.pc
                && a.block_id == b.block_id
        });
    }

    fn resolve(&self, line: u32) -> Option<StopMetadata> {
        for span in &self.spans {
            if line >= span.line_start && line <= span.line_end {
                return Some(StopMetadata {
                    pc: span.pc,
                    instruction_index: span.instruction_index,
                    block_id: span.block_id,
                });
            }
        }

        let mut best: Option<(u32, &SourceLineSpan)> = None;
        for span in &self.spans {
            let dist = if line < span.line_start {
                span.line_start - line
            } else {
                line.saturating_sub(span.line_end)
            };
            if dist > 4 {
                continue;
            }
            match best {
                Some((best_dist, _)) if best_dist <= dist => {}
                _ => best = Some((dist, span)),
            }
        }

        best.map(|(_, span)| StopMetadata {
            pc: span.pc,
            instruction_index: span.instruction_index,
            block_id: span.block_id,
        })
    }

    fn resolve_pc(&self, pc: i32) -> Option<StopMetadata> {
        self.by_pc.get(&pc).copied()
    }
}

// ─── Actor ───────────────────────────────────────────────────────────

pub struct DebugActor {
    state: DebugState,
    bridge: Option<Bridge>,
    embedded: Option<EmbeddedBackend>,
    wire: Option<remote::WireBackend>,
    process: Option<Child>,
    breakpoints: Vec<Breakpoint>,
    port: u16,
    script_path: String,
    script_source: Option<String>,
    tcl_version: Option<String>,
    pid: Option<u32>,
    bytecode_chunks: Vec<DebugBytecodeChunkEvent>,
    next_request_id: u64,
    pending_evals: HashMap<u64, PendingRequest>,
    event_rx: Option<mpsc::Receiver<AgentEvent>>,
    app_handle: tauri::AppHandle,
    run_control: Option<RunControlCore>,
    last_trace_level: i32,
    stop_metadata_index: StopMetadataIndex,
    watchpoints: Vec<WatchpointInfo>,
    next_watchpoint_id: u64,
    last_watchpoint_hit: Option<DebugWatchpointHitEvent>,
}

async fn stop_child(mut child: Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn find_free_port() -> Result<u16, String> {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind error: {e}"))?;
    listener
        .local_addr()
        .map_err(|e| format!("addr error: {e}"))
        .map(|a| a.port())
}

fn agent_script_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("exe path error: {e}"))?;
    let exe_dir = exe.parent().ok_or("no exe parent dir")?;

    let candidates = [
        exe_dir.join("../../../apps/tcld-tauri/src/resources/tcld-agent.tcl"),
        exe_dir.join("resources/tcld-agent.tcl"),
        exe_dir.join("../Resources/tcld-agent.tcl"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/resources/tcld-agent.tcl"),
    ];

    for path in &candidates {
        if path.exists() {
            return path
                .canonicalize()
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| format!("canonicalize error: {e}"));
        }
    }

    Err(format!(
        "tcld-agent.tcl not found, searched: {:?}",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    ))
}

fn canonicalize_debug_path(path: String) -> String {
    let path_buf = PathBuf::from(&path);
    match path_buf.canonicalize() {
        Ok(canonical) => canonical.to_string_lossy().into_owned(),
        Err(_) => path,
    }
}

fn dbg_extension_library_filename() -> &'static str {
    if cfg!(target_os = "macos") {
        "libtcld_dbg.dylib"
    } else if cfg!(target_os = "windows") {
        "tcld_dbg.dll"
    } else {
        "libtcld_dbg.so"
    }
}

fn debug_extension_library_path() -> Option<String> {
    if let Ok(path) = std::env::var("TCLD_DEBUG_EXTENSION_PATH") {
        let path_buf = PathBuf::from(path.trim());
        if path_buf.exists() {
            return path_buf
                .canonicalize()
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
        }
    }

    let file = dbg_extension_library_filename();
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let candidates = [
        exe_dir.join(format!("../../../target/debug/{file}")),
        exe_dir.join(format!("../../../target/release/{file}")),
        exe_dir.join(format!("resources/{file}")),
        exe_dir.join(format!("../Resources/{file}")),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/debug/{file}")),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/release/{file}")),
    ];

    for path in &candidates {
        if path.exists() {
            if let Ok(canonical) = path.canonicalize() {
                return Some(canonical.to_string_lossy().into_owned());
            }
            return Some(path.to_string_lossy().into_owned());
        }
    }

    None
}

fn parse_disassembly_command_header(line: &str) -> Option<u32> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("Command ")?;
    let cmd = rest.split(':').next()?.trim();
    cmd.parse::<u32>().ok()
}

fn parse_disassembly_instruction_pc(line: &str) -> Option<i32> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('(') {
        return None;
    }
    let end = trimmed.find(')')?;
    trimmed[1..end].trim().parse::<i32>().ok()
}

fn parse_disassembly_source_range(line: &str) -> Option<(u32, usize, usize)> {
    let trimmed = line.trim();
    let (cmd_part, rest) = trimmed.split_once(": pc ")?;
    let cmd_num = cmd_part.trim().parse::<u32>().ok()?;
    let (_, src_part) = rest.split_once(", src ")?;
    let token = src_part
        .split(|c: char| c.is_whitespace() || c == ',')
        .next()?
        .trim();
    let (start, end) = token.split_once('-')?;
    let start = start.trim().parse::<usize>().ok()?;
    let end = end.trim().parse::<usize>().ok()?;
    Some((cmd_num, start, end))
}

fn line_starts_for_text(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in text.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn byte_offset_to_line(offset: usize, line_starts: &[usize], text_len: usize) -> u32 {
    let capped = offset.min(text_len);
    let mut lo = 0usize;
    let mut hi = line_starts.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if line_starts[mid] <= capped {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    (lo.max(1)) as u32
}

fn parse_disassembly_source_spans(disassembly: &str, source: &str) -> Vec<SourceLineSpan> {
    let mut src_ranges: HashMap<u32, (usize, usize)> = HashMap::new();
    for line in disassembly.lines() {
        if let Some((cmd, start, end)) = parse_disassembly_source_range(line) {
            src_ranges.insert(cmd, (start, end));
        }
    }
    if src_ranges.is_empty() {
        return Vec::new();
    }

    let mut current_cmd: Option<u32> = None;
    let mut next_instruction_index: usize = 0;
    let mut first_instruction_by_cmd: HashMap<u32, (usize, i32)> = HashMap::new();
    for line in disassembly.lines() {
        if let Some(cmd) = parse_disassembly_command_header(line) {
            current_cmd = Some(cmd);
            continue;
        }
        if let Some(pc) = parse_disassembly_instruction_pc(line) {
            if let Some(cmd) = current_cmd {
                first_instruction_by_cmd
                    .entry(cmd)
                    .or_insert((next_instruction_index, pc));
            }
            next_instruction_index += 1;
        }
    }

    if first_instruction_by_cmd.is_empty() {
        return Vec::new();
    }

    let line_starts = line_starts_for_text(source);
    let source_len = source.len();
    let mut spans = Vec::new();

    for (cmd, (start, end)) in src_ranges {
        let Some((instruction_index, pc)) = first_instruction_by_cmd.get(&cmd).copied() else {
            continue;
        };
        let start_line = byte_offset_to_line(start, &line_starts, source_len);
        let end_line = byte_offset_to_line(start.max(end), &line_starts, source_len);
        spans.push(SourceLineSpan {
            line_start: start_line.min(end_line),
            line_end: start_line.max(end_line),
            pc,
            instruction_index,
            block_id: cmd.checked_sub(1).map(|group| group as usize),
        });
    }

    spans.sort_by_key(|s| (s.line_start, s.line_end, s.instruction_index));
    spans
}

fn parse_disassembly_instruction_points(disassembly: &str) -> Vec<(i32, usize, Option<usize>)> {
    let mut current_cmd: Option<u32> = None;
    let mut instruction_index: usize = 0;
    let mut out = Vec::new();

    for line in disassembly.lines() {
        if let Some(cmd) = parse_disassembly_command_header(line) {
            current_cmd = Some(cmd);
            continue;
        }

        if let Some(pc) = parse_disassembly_instruction_pc(line) {
            let block_id = current_cmd.and_then(|cmd| cmd.checked_sub(1).map(|g| g as usize));
            out.push((pc, instruction_index, block_id));
            instruction_index += 1;
        }
    }

    out
}

fn normalize_path_for_match(path: &str) -> String {
    if path.trim().is_empty() {
        return String::new();
    }
    let raw = path.trim().replace('\\', "/");
    let abs = {
        let pb = PathBuf::from(&raw);
        if pb.is_absolute() {
            pb
        } else if let Ok(cwd) = std::env::current_dir() {
            cwd.join(pb)
        } else {
            PathBuf::from(&raw)
        }
    };
    let normalized = abs
        .canonicalize()
        .unwrap_or(abs)
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn files_likely_match(a: &str, b: &str) -> bool {
    let na = normalize_path_for_match(a);
    let nb = normalize_path_for_match(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    if na == nb {
        return true;
    }
    let ta = Path::new(&na).file_name().and_then(|s| s.to_str());
    let tb = Path::new(&nb).file_name().and_then(|s| s.to_str());
    ta.is_some() && ta == tb
}

impl DebugActor {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self {
            state: DebugState::Idle,
            bridge: None,
            embedded: None,
            wire: None,
            process: None,
            breakpoints: Vec::new(),
            port: 0,
            script_path: String::new(),
            script_source: None,
            tcl_version: None,
            pid: None,
            bytecode_chunks: Vec::new(),
            next_request_id: 0,
            pending_evals: HashMap::new(),
            event_rx: None,
            app_handle,
            run_control: None,
            last_trace_level: 0,
            stop_metadata_index: StopMetadataIndex::default(),
            watchpoints: Vec::new(),
            next_watchpoint_id: 0,
            last_watchpoint_hit: None,
        }
    }

    fn next_watchpoint_id(&mut self) -> u64 {
        self.next_watchpoint_id += 1;
        self.next_watchpoint_id
    }

    fn next_id(&mut self) -> u64 {
        self.next_request_id += 1;
        self.next_request_id
    }

    fn clear_runtime(&mut self) {
        self.bridge = None;
        if let Some(mut emb) = self.embedded.take() {
            emb.stop();
        }
        if let Some(mut wire) = self.wire.take() {
            wire.stop();
        }
        self.event_rx = None;
        self.port = 0;
        self.tcl_version = None;
        self.pid = None;
        self.script_source = None;
        self.bytecode_chunks.clear();
        self.fail_pending_evals("session ended");
        self.run_control = None;
        self.last_trace_level = 0;
        self.stop_metadata_index.clear();
        self.watchpoints.clear();
        self.next_watchpoint_id = 0;
        self.last_watchpoint_hit = None;
    }

    fn fail_pending_evals(&mut self, reason: &str) {
        for (_, pending) in self.pending_evals.drain() {
            match pending.sender {
                PendingEval::Sync(tx) => {
                    let _ = tx.send(EvalResult {
                        result: reason.to_string(),
                        error: true,
                        error_info: None,
                        error_code: None,
                    });
                }
                PendingEval::Disassemble(tx) => {
                    let _ = tx.send(Err(reason.to_string()));
                }
                PendingEval::SetWatchpoint { reply, .. }
                | PendingEval::ClearWatchpoint { reply, .. }
                | PendingEval::ClearAllWatchpoints { reply } => {
                    let _ = reply.send(Err(reason.to_string()));
                }
            }
        }
    }

    fn cleanup_stale_requests(&mut self) {
        let cutoff = Instant::now() - std::time::Duration::from_secs(30);
        let stale: Vec<u64> = self
            .pending_evals
            .iter()
            .filter(|(_, p)| p.created < cutoff)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(pending) = self.pending_evals.remove(&id) {
                match pending.sender {
                    PendingEval::Sync(tx) => {
                        let _ = tx.send(EvalResult {
                            result: "request timed out (30s)".to_string(),
                            error: true,
                            error_info: None,
                            error_code: None,
                        });
                    }
                    PendingEval::Disassemble(tx) => {
                        let _ = tx.send(Err("request timed out (30s)".to_string()));
                    }
                    PendingEval::SetWatchpoint { reply, .. }
                    | PendingEval::ClearWatchpoint { reply, .. }
                    | PendingEval::ClearAllWatchpoints { reply } => {
                        let _ = reply.send(Err("request timed out (30s)".to_string()));
                    }
                }
            }
        }
    }

    fn state_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "epoch": self.next_request_id,
            "state": self.state,
            "scriptPath": self.script_path,
            "tclVersion": self.tcl_version,
            "pid": self.pid,
            "breakpointCount": self.breakpoints.len(),
            "chunkCount": self.bytecode_chunks.len(),
        })
    }

    fn emit_state_snapshot(&self) {
        let _ = self
            .app_handle
            .emit("debugger:state", self.state_snapshot());
    }

    fn has_live_backend(&self) -> bool {
        self.bridge.is_some() || self.embedded.is_some() || self.wire.is_some()
    }

    async fn send_to_agent(&self, cmd: &AgentCommand) -> Result<(), String> {
        if let Some(emb) = &self.embedded {
            return emb.send(cmd);
        }
        if let Some(wire) = &self.wire {
            return wire.send(cmd);
        }
        let bridge = self.bridge.as_ref().ok_or("no bridge")?;
        bridge.send(cmd).await
    }

    // ── Main run loop ────────────────────────────────────────────────

    pub async fn run(mut self, mut cmd_rx: mpsc::Receiver<DebugCmd>) {
        let mut cleanup_interval = tokio::time::interval(std::time::Duration::from_secs(5));

        loop {
            tokio::select! {
                biased;

                Some(event) = async {
                    if let Some(rx) = &mut self.event_rx {
                        rx.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    self.handle_agent_event(event).await;
                    self.emit_state_snapshot();
                }

                Some(cmd) = cmd_rx.recv() => {
                    self.handle_cmd(cmd).await;
                    self.emit_state_snapshot();
                }

                _ = cleanup_interval.tick() => {
                    self.cleanup_stale_requests();
                }

                else => break,
            }
        }
    }

    // ── Command handling ─────────────────────────────────────────────

    async fn handle_cmd(&mut self, cmd: DebugCmd) {
        match cmd {
            DebugCmd::Launch {
                path,
                args,
                tclsh_path,
                launch_options,
                reply,
            } => {
                let _ = reply.send(
                    self.cmd_launch(path, args, tclsh_path, launch_options)
                        .await,
                );
            }
            DebugCmd::Stop { reply } => {
                let _ = reply.send(self.cmd_stop().await);
            }
            DebugCmd::StepInto { reply } => {
                let _ = reply.send(self.cmd_step(AgentCommand::StepInto).await);
            }
            DebugCmd::StepInstruction { reply } => {
                let _ = reply.send(self.cmd_opcode_step(AgentCommand::StepInstruction).await);
            }
            DebugCmd::StepOverInstruction { reply } => {
                let _ = reply.send(
                    self.cmd_opcode_step(AgentCommand::StepOverInstruction)
                        .await,
                );
            }
            DebugCmd::StepOutInstruction { reply } => {
                let _ = reply.send(self.cmd_opcode_step(AgentCommand::StepOutInstruction).await);
            }
            DebugCmd::RunToPc { pc, reply } => {
                let _ = reply.send(self.cmd_opcode_step(AgentCommand::RunToPc { pc }).await);
            }
            DebugCmd::StepOver { reply } => {
                let _ = reply.send(self.cmd_step(AgentCommand::StepOver).await);
            }
            DebugCmd::StepOut { reply } => {
                let _ = reply.send(self.cmd_step(AgentCommand::StepOut).await);
            }
            DebugCmd::Continue { reply } => {
                let _ = reply.send(self.cmd_step(AgentCommand::Continue).await);
            }
            DebugCmd::Pause { reply } => {
                let _ = reply.send(self.cmd_pause().await);
            }
            DebugCmd::SetBreakpoint {
                file,
                line,
                offset,
                len,
                condition,
                reply,
            } => {
                let _ = reply.send(
                    self.cmd_set_breakpoint(file, line, offset, len, condition)
                        .await,
                );
            }
            DebugCmd::SetBreakpointEnabled {
                file,
                line,
                offset,
                enabled,
                reply,
            } => {
                let _ = reply.send(
                    self.cmd_set_breakpoint_enabled(file, line, offset, enabled)
                        .await,
                );
            }
            DebugCmd::ClearBreakpoint {
                file,
                line,
                offset,
                reply,
            } => {
                let _ = reply.send(self.cmd_clear_breakpoint(file, line, offset).await);
            }
            DebugCmd::ClearAllBreakpoints { reply } => {
                let _ = reply.send(self.cmd_clear_all_breakpoints().await);
            }
            DebugCmd::ListBreakpoints { reply } => {
                let _ = reply.send(self.breakpoints.clone());
            }
            DebugCmd::Eval {
                script,
                level,
                reply,
            } => {
                let _ = reply.send(self.cmd_eval(script, level).await);
            }
            DebugCmd::EvalSync {
                script,
                level,
                reply,
            } => {
                self.cmd_eval_sync(script, level, reply).await;
            }
            DebugCmd::GetVariables { level } => {
                self.cmd_get_variables(level).await;
            }
            DebugCmd::GetStack => {
                self.cmd_get_stack().await;
            }
            DebugCmd::SetVariable {
                name,
                value,
                level,
                reply,
            } => {
                let _ = reply.send(self.cmd_set_variable(name, value, level).await);
            }
            DebugCmd::GetState { reply } => {
                let _ = reply.send(self.state_snapshot());
            }
            DebugCmd::GetBytecodeChunks { reply } => {
                let _ = reply.send(self.bytecode_chunks.clone());
            }
            DebugCmd::Disassemble { path, reply } => {
                self.cmd_disassemble(path, reply).await;
            }
            DebugCmd::ResolveCommand {
                name,
                namespace,
                reply,
            } => {
                self.cmd_resolve_command(name, namespace, reply).await;
            }
            DebugCmd::SetWatchpoint {
                name,
                scope,
                level,
                condition,
                reply,
            } => {
                self.cmd_set_watchpoint(name, scope, level, condition, reply)
                    .await;
            }
            DebugCmd::ClearWatchpoint { name, reply } => {
                self.cmd_clear_watchpoint(name, reply).await;
            }
            DebugCmd::ListWatchpoints { reply } => {
                let _ = reply.send(self.watchpoints.clone());
            }
            DebugCmd::ClearAllWatchpoints { reply } => {
                self.cmd_clear_all_watchpoints_wp(reply).await;
            }
        }
    }

    async fn cmd_launch(
        &mut self,
        path: String,
        args: Vec<String>,
        tclsh_path: Option<String>,
        launch_options: DebugLaunchOptions,
    ) -> Result<(), String> {
        let path = canonicalize_debug_path(path);
        let script_source = tokio::fs::read_to_string(&path).await.ok();
        let debug_extension_path = debug_extension_library_path();
        let backend = if launch_options.backend == DebugBackendKind::External {
            BackendKind::Wire
        } else if tclsh_path.is_some() {
            BackendKind::External
        } else {
            BackendKind::Embedded
        };
        let hook_plan = resolve_hook_plan(backend, launch_options.clone())?;

        if self.state != DebugState::Idle && self.state != DebugState::Terminated {
            return Err("debug session already active".into());
        }

        // Clean up any leftover state
        if let Some(bridge) = self.bridge.take() {
            bridge.disconnect().await;
        }
        if let Some(child) = self.process.take() {
            stop_child(child).await;
        }
        self.clear_runtime();

        self.state = DebugState::Launching;
        self.script_path = path.clone();
        self.script_source = script_source;
        self.bytecode_chunks.clear();

        match backend {
            BackendKind::Embedded => {
                self.launch_embedded(path, args, hook_plan, debug_extension_path, launch_options)
            }
            BackendKind::External => {
                self.launch_external(
                    path,
                    args,
                    tclsh_path,
                    hook_plan,
                    debug_extension_path,
                    launch_options,
                )
                .await
            }
            BackendKind::Wire => self.launch_wire(path, args, tclsh_path, launch_options),
        }
    }

    /// Launch the external wire-protocol backend: spawn `tclsh` with the
    /// tcld nub and drive it via the tcld-debug-wire controller. Path
    /// discovery errors (missing tclsh / missing nub dylib) surface here
    /// synchronously with actionable diagnostics.
    fn launch_wire(
        &mut self,
        path: String,
        args: Vec<String>,
        tclsh_path: Option<String>,
        launch_options: DebugLaunchOptions,
    ) -> Result<(), String> {
        let non_empty = |s: &String| !s.trim().is_empty();
        let tclsh_override = launch_options
            .tclsh_path
            .clone()
            .filter(non_empty)
            .or(tclsh_path);
        let dylib_override = launch_options.dylib_path.clone().filter(non_empty);
        let paths = remote::locate_nub_paths(tclsh_override.as_deref(), dylib_override.as_deref())
            .inspect_err(|_| {
                self.state = DebugState::Idle;
                self.script_path.clear();
            })?;

        let mut backend = remote::WireBackend::launch(remote::WireLaunchConfig {
            script_path: path,
            args,
            env: launch_options.env,
            cwd: launch_options.cwd,
            paths,
        })
        .inspect_err(|_| {
            self.state = DebugState::Idle;
            self.script_path.clear();
        })?;
        let event_rx = backend.take_event_rx();
        self.wire = Some(backend);
        self.event_rx = Some(event_rx);
        // The nub owns run control (breakpoint matching, step anchors);
        // the actor-side RunControlCore is only for TracePoint-driven
        // backends, so leave it unset. cmd_pause then routes Pause to the
        // backend instead of the local core.
        self.run_control = None;
        self.last_trace_level = 0;
        self.state = DebugState::Connected;
        Ok(())
    }

    fn launch_embedded(
        &mut self,
        path: String,
        args: Vec<String>,
        hook_plan: HookPlan,
        debug_extension_path: Option<String>,
        launch_options: DebugLaunchOptions,
    ) -> Result<(), String> {
        let plan = build_embedded_launch_plan(
            &self.breakpoints,
            path,
            args,
            hook_plan,
            debug_extension_path,
            launch_options,
        );

        let mut backend = EmbeddedBackend::launch(
            plan.path,
            plan.args,
            plan.breakpoints,
            plan.hook_plan,
            plan.debug_extension_path,
            plan.env,
            plan.cwd,
        )?;
        let event_rx = backend.take_event_rx();
        self.embedded = Some(backend);
        self.event_rx = Some(event_rx);
        self.run_control = Some(RunControlCore::new_start_step_into());
        self.last_trace_level = 0;
        self.state = DebugState::Connected;
        Ok(())
    }

    async fn launch_external(
        &mut self,
        path: String,
        args: Vec<String>,
        tclsh_path: Option<String>,
        hook_plan: HookPlan,
        debug_extension_path: Option<String>,
        launch_options: DebugLaunchOptions,
    ) -> Result<(), String> {
        let port = find_free_port()?;
        let agent_path = agent_script_path()?;
        let plan = build_external_launch_plan(
            path,
            args,
            tclsh_path,
            hook_plan,
            debug_extension_path,
            launch_options,
            agent_path,
            port,
        );

        self.port = plan.port;

        let tclsh = plan.tclsh.clone();
        let mut cmd = command_for_external_launch(&plan);

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.state = DebugState::Idle;
                self.script_path.clear();
                return Err(format!("failed to spawn tclsh: {e}. Is '{tclsh}' in PATH?"));
            }
        };

        self.process = Some(child);

        let mut last_err = String::new();
        let mut connected = None;
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            match Bridge::connect(port, 2).await {
                Ok(b) => {
                    connected = Some(b);
                    break;
                }
                Err(e) => {
                    last_err = e;
                }
            }
        }

        let mut bridge = match connected {
            Some(b) => b,
            None => {
                let child = self.process.take();
                self.state = DebugState::Idle;
                self.clear_runtime();
                if let Some(child) = child {
                    stop_child(child).await;
                }
                return Err(format!("failed to connect to agent: {last_err}"));
            }
        };

        let event_rx = bridge.take_event_rx();
        self.bridge = Some(bridge);
        self.event_rx = Some(event_rx);
        self.run_control = Some(RunControlCore::new_start_step_into());
        self.last_trace_level = 0;
        self.state = DebugState::Connected;

        Ok(())
    }

    async fn cmd_stop(&mut self) -> Result<(), String> {
        if matches!(self.state, DebugState::Idle) {
            return Ok(());
        }

        self.state = DebugState::Stopping;

        let bridge = self.bridge.take();
        let embedded = self.embedded.take();
        let child = self.process.take();
        self.clear_runtime();

        if let Some(bridge) = bridge {
            bridge.disconnect().await;
        }
        if let Some(mut embedded) = embedded {
            embedded.stop();
        }
        if let Some(child) = child {
            stop_child(child).await;
        }

        self.state = DebugState::Terminated;
        self.pid = None;
        self.tcl_version = None;

        Ok(())
    }

    async fn cmd_step(&mut self, cmd: AgentCommand) -> Result<(), String> {
        if self.state != DebugState::Paused {
            return Err(format!("not paused (state: {:?})", self.state));
        }
        if let Some(core) = &mut self.run_control
            && let Some(action) = command_to_resume_action(&cmd)
        {
            core.on_resume(action, self.last_trace_level);
        }
        self.send_to_agent(&cmd).await
        // Do NOT change state — wait for Resumed event from agent
    }

    async fn cmd_opcode_step(&mut self, cmd: AgentCommand) -> Result<(), String> {
        if self.state != DebugState::Paused {
            return Err(format!("not paused (state: {:?})", self.state));
        }
        self.send_to_agent(&cmd).await
    }

    async fn cmd_pause(&mut self) -> Result<(), String> {
        if self.state != DebugState::Running {
            return Err(format!("not running (state: {:?})", self.state));
        }
        if let Some(core) = &mut self.run_control {
            core.request_pause();
        } else {
            self.send_to_agent(&AgentCommand::Pause).await?;
        }
        self.state = DebugState::Pausing;
        Ok(())
    }

    async fn cmd_set_breakpoint(
        &mut self,
        file: String,
        line: u32,
        offset: Option<u64>,
        len: Option<u64>,
        condition: Option<String>,
    ) -> Result<(), String> {
        let file = canonicalize_debug_path(file);

        if self.has_live_backend() {
            self.send_to_agent(&AgentCommand::SetBreakpoint {
                file: file.clone(),
                line,
                offset,
                len,
                condition: condition.clone(),
            })
            .await?;
        }
        self.breakpoints
            .retain(|bp| bp.file != file || bp.line != line || bp.offset != offset);
        self.breakpoints.push(Breakpoint {
            file,
            line,
            offset,
            len,
            condition,
            enabled: true,
        });
        Ok(())
    }

    async fn cmd_clear_breakpoint(
        &mut self,
        file: String,
        line: u32,
        offset: Option<u64>,
    ) -> Result<(), String> {
        let file = canonicalize_debug_path(file);

        if self.has_live_backend() {
            self.send_to_agent(&AgentCommand::ClearBreakpoint {
                file: file.clone(),
                line,
                offset,
            })
            .await?;
        }
        self.breakpoints
            .retain(|bp| bp.file != file || bp.line != line || bp.offset != offset);
        Ok(())
    }

    async fn cmd_set_breakpoint_enabled(
        &mut self,
        file: String,
        line: u32,
        offset: Option<u64>,
        enabled: bool,
    ) -> Result<(), String> {
        let file = canonicalize_debug_path(file);
        self.breakpoints
            .iter()
            .position(|bp| bp.file == file && bp.line == line && bp.offset == offset)
            .ok_or_else(|| format!("breakpoint not found: {file}:{line}"))?;

        if self.has_live_backend() {
            self.send_to_agent(&AgentCommand::SetBreakpointEnabled {
                file: file.clone(),
                line,
                offset,
                enabled,
            })
            .await?;
        }
        set_breakpoint_enabled_state(&mut self.breakpoints, file, line, offset, enabled)
    }

    async fn cmd_clear_all_breakpoints(&mut self) -> Result<(), String> {
        if self.has_live_backend() {
            self.send_to_agent(&AgentCommand::ClearAllBreakpoints)
                .await?;
        }
        self.breakpoints.clear();
        Ok(())
    }

    async fn cmd_eval(&mut self, script: String, level: u32) -> Result<(), String> {
        ensure_paused(self.state)?;
        let id = self.next_id();
        let command = build_eval_command(self.state, id, script, level)?;
        self.send_to_agent(&command).await
    }

    async fn cmd_eval_sync(
        &mut self,
        script: String,
        level: u32,
        reply: oneshot::Sender<Result<EvalResult, String>>,
    ) {
        if let Err(err) = ensure_paused(self.state) {
            let _ = reply.send(Err(err));
            return;
        }
        let id = self.next_id();
        let command = match build_eval_command(self.state, id, script, level) {
            Ok(command) => command,
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        match self.send_to_agent(&command).await {
            Ok(()) => {
                self.pending_evals.insert(
                    id,
                    PendingRequest {
                        sender: PendingEval::Sync(
                            // We need to wrap: the reply expects Result<EvalResult, String>
                            // but PendingEval::Sync sends EvalResult directly.
                            // Use an intermediary oneshot for the EvalResult, then map.
                            {
                                let (inner_tx, inner_rx) = oneshot::channel();
                                // Spawn a micro-task to bridge inner → outer
                                let outer = reply;
                                tokio::spawn(async move {
                                    match inner_rx.await {
                                        Ok(result) => {
                                            let _ = outer.send(Ok(result));
                                        }
                                        Err(_) => {
                                            let _ =
                                                outer.send(Err("eval channel closed".to_string()));
                                        }
                                    }
                                });
                                inner_tx
                            },
                        ),
                        created: Instant::now(),
                    },
                );
            }
            Err(e) => {
                let _ = reply.send(Err(e));
            }
        }
    }

    async fn cmd_get_variables(&mut self, level: u32) {
        if ensure_paused(self.state).is_err() {
            return;
        }
        let id = self.next_id();
        if let Ok(command) = build_get_variables_command(self.state, id, level) {
            let _ = self.send_to_agent(&command).await;
        }
    }

    async fn cmd_get_stack(&mut self) {
        if ensure_paused(self.state).is_err() {
            return;
        }
        let id = self.next_id();
        if let Ok(command) = build_get_stack_command(self.state, id) {
            let _ = self.send_to_agent(&command).await;
        }
    }

    async fn request_paused_snapshot(&mut self) {
        if ensure_paused(self.state).is_err() {
            return;
        }
        let stack_id = self.next_id();
        let variables_id = self.next_id();
        if let Ok(commands) = build_paused_snapshot_commands(self.state, stack_id, variables_id, 1)
        {
            for command in commands {
                let _ = self.send_to_agent(&command).await;
            }
        }
    }

    async fn cmd_set_variable(
        &mut self,
        name: String,
        value: String,
        level: u32,
    ) -> Result<(), String> {
        ensure_paused(self.state)?;
        self.send_to_agent(&AgentCommand::SetVariable { name, value, level })
            .await
    }

    async fn cmd_disassemble(
        &mut self,
        path: String,
        reply: oneshot::Sender<Result<String, String>>,
    ) {
        if let Err(err) = ensure_paused(self.state) {
            let _ = reply.send(Err(err));
            return;
        }

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                let _ = reply.send(Err(format!("failed to read {path}: {e}")));
                return;
            }
        };

        let script = format!(
            "::tcl::unsupported::disassemble script {{{}}}",
            content.replace('{', "\\{").replace('}', "\\}")
        );

        let id = self.next_id();
        match self
            .send_to_agent(&AgentCommand::Eval {
                request_id: id,
                script,
                level: 0,
            })
            .await
        {
            Ok(()) => {
                self.pending_evals.insert(
                    id,
                    PendingRequest {
                        sender: PendingEval::Disassemble(reply),
                        created: Instant::now(),
                    },
                );
            }
            Err(e) => {
                let _ = reply.send(Err(e));
            }
        }
    }

    async fn cmd_resolve_command(
        &mut self,
        name: String,
        namespace: Option<String>,
        reply: oneshot::Sender<Result<CommandResolution, String>>,
    ) {
        if let Err(err) = ensure_paused(self.state) {
            let _ = reply.send(Err(err));
            return;
        }
        let id = self.next_id();
        let command = match build_resolve_command(self.state, id, name, namespace) {
            Ok(command) => command,
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        match self.send_to_agent(&command).await {
            Ok(()) => {
                let (inner_tx, inner_rx) = oneshot::channel::<EvalResult>();
                tokio::spawn(async move {
                    match inner_rx.await {
                        Ok(eval_result) => {
                            if eval_result.error {
                                let _ = reply.send(Err(eval_result
                                    .error_info
                                    .unwrap_or(eval_result.result)));
                            } else {
                                let _ = reply.send(parse_resolve_result(&eval_result.result));
                            }
                        }
                        Err(_) => {
                            let _ = reply.send(Err("resolve channel closed".to_string()));
                        }
                    }
                });
                self.pending_evals.insert(
                    id,
                    PendingRequest {
                        sender: PendingEval::Sync(inner_tx),
                        created: Instant::now(),
                    },
                );
            }
            Err(e) => {
                let _ = reply.send(Err(e));
            }
        }
    }

    // ── Watchpoint commands ─────────────────────────────────────────

    async fn cmd_set_watchpoint(
        &mut self,
        name: String,
        scope: Option<String>,
        level: Option<u32>,
        condition: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    ) {
        if let Err(err) = ensure_paused(self.state) {
            let _ = reply.send(Err(err));
            return;
        }
        if let Err(err) = normalize_watchpoint_scope(scope.clone()) {
            let _ = reply.send(Err(err));
            return;
        }
        let id = self.next_watchpoint_id();
        let eval_id = self.next_id();
        let (command, watchpoint) = match build_set_watchpoint_command(
            self.state, eval_id, id, name, scope, level, condition,
        ) {
            Ok(result) => result,
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        let send_result = self.send_to_agent(&command).await;
        if let Err(err) = send_result {
            let _ = reply.send(Err(err));
            return;
        }
        self.pending_evals.insert(
            eval_id,
            PendingRequest {
                sender: PendingEval::SetWatchpoint { reply, watchpoint },
                created: Instant::now(),
            },
        );
    }

    async fn cmd_clear_watchpoint(
        &mut self,
        name: String,
        reply: oneshot::Sender<Result<(), String>>,
    ) {
        if let Err(err) = ensure_paused(self.state) {
            let _ = reply.send(Err(err));
            return;
        }
        let eval_id = self.next_id();
        let command = match build_clear_watchpoint_command(self.state, eval_id, name.clone()) {
            Ok(command) => command,
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        let send_result = self.send_to_agent(&command).await;
        if let Err(err) = send_result {
            let _ = reply.send(Err(err));
            return;
        }

        self.pending_evals.insert(
            eval_id,
            PendingRequest {
                sender: PendingEval::ClearWatchpoint { reply, name },
                created: Instant::now(),
            },
        );
    }

    async fn cmd_clear_all_watchpoints_wp(&mut self, reply: oneshot::Sender<Result<(), String>>) {
        if let Err(err) = ensure_paused(self.state) {
            let _ = reply.send(Err(err));
            return;
        }
        let eval_id = self.next_id();
        let command = match build_clear_all_watchpoints_command(self.state, eval_id) {
            Ok(command) => command,
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        let send_result = self.send_to_agent(&command).await;
        if let Err(err) = send_result {
            let _ = reply.send(Err(err));
            return;
        }

        self.pending_evals.insert(
            eval_id,
            PendingRequest {
                sender: PendingEval::ClearAllWatchpoints { reply },
                created: Instant::now(),
            },
        );
    }

    // ── Agent event handling ─────────────────────────────────────────

    async fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Ready {
                tcl_version,
                pid,
                debug_extension_loaded,
                debug_extension_installed,
                debug_extension_capabilities,
                debug_extension_reason,
            } => {
                self.state = DebugState::Connected;
                self.tcl_version = Some(tcl_version.clone());
                self.pid = Some(pid);

                // Sync breakpoints to agent
                let bps: Vec<Breakpoint> = self.breakpoints.clone();
                for bp in &bps {
                    if bp.enabled {
                        let _ = self
                            .send_to_agent(&AgentCommand::SetBreakpoint {
                                file: bp.file.clone(),
                                line: bp.line,
                                offset: bp.offset,
                                len: bp.len,
                                condition: bp.condition.clone(),
                            })
                            .await;
                    }
                }

                let _ = self.app_handle.emit(
                    "debugger:ready",
                    DebugReadyEvent {
                        tcl_version,
                        pid,
                        debug_extension_loaded,
                        debug_extension_installed,
                        debug_extension_capabilities,
                        debug_extension_reason,
                    },
                );
            }

            AgentEvent::Resumed { control_mode } => {
                self.state = DebugState::Running;
                let _ = self
                    .app_handle
                    .emit("debugger:resumed", DebugResumedEvent { control_mode });
            }

            AgentEvent::Stopped {
                reason,
                file,
                line,
                frame_count,
                pc,
                instruction_index,
                block_id,
                error,
                error_info,
            } => {
                self.state = DebugState::Paused;
                let mapped_by_pc =
                    pc.and_then(|pc_val| self.stop_metadata_index.resolve_pc(pc_val));
                let mapped_by_line =
                    if mapped_by_pc.is_none() && files_likely_match(&file, &self.script_path) {
                        self.stop_metadata_index.resolve(line)
                    } else {
                        None
                    };
                let stop_meta = mapped_by_pc.or(mapped_by_line);
                let watchpoint_meta = if reason == "watchpoint" {
                    self.last_watchpoint_hit.take()
                } else {
                    self.last_watchpoint_hit = None;
                    None
                };
                let event = build_debug_stopped_event(
                    StoppedEventParts {
                        reason,
                        file,
                        line,
                        frame_count,
                        pc: pc.or(stop_meta.map(|m| m.pc)),
                        instruction_index: instruction_index
                            .or(stop_meta.map(|m| m.instruction_index)),
                        block_id: block_id.or(stop_meta.and_then(|m| m.block_id)),
                        error,
                        error_info,
                    },
                    watchpoint_meta.as_ref(),
                );
                let _ = self.app_handle.emit("debugger:stopped", event);
                self.request_paused_snapshot().await;
            }

            AgentEvent::Output { stream, text } => {
                if !is_active(self.state) && self.state != DebugState::Terminated {
                    return;
                }
                let _ = self
                    .app_handle
                    .emit("debugger:output", DebugOutputEvent { stream, text });
            }

            AgentEvent::Exited { code, reason } => {
                self.state = DebugState::Terminated;
                let child = self.process.take();
                self.clear_runtime();
                if let Some(child) = child {
                    stop_child(child).await;
                }
                let _ = self
                    .app_handle
                    .emit("debugger:exited", DebugExitedEvent { code, reason });
            }

            AgentEvent::Variables {
                request_id: _,
                level,
                vars,
            } => {
                if !is_active(self.state) {
                    return;
                }
                let _ = self.app_handle.emit(
                    "debugger:variables",
                    serde_json::json!({ "level": level, "vars": vars }),
                );
            }

            AgentEvent::Stack {
                request_id: _,
                frames,
            } => {
                if !is_active(self.state) {
                    return;
                }
                let _ = self
                    .app_handle
                    .emit("debugger:stack", serde_json::json!({ "frames": frames }));
            }

            AgentEvent::EvalResult {
                request_id,
                result,
                error,
                error_info,
                error_code,
            } => {
                let eval_result = EvalResult {
                    result,
                    error,
                    error_info,
                    error_code,
                };
                if let Some(pending) = self.pending_evals.remove(&request_id) {
                    match pending.sender {
                        PendingEval::Sync(tx) => {
                            let _ = tx.send(eval_result);
                        }
                        PendingEval::Disassemble(tx) => {
                            if eval_result.error {
                                let _ = tx.send(Err(format!(
                                    "disassemble failed: {}",
                                    eval_result.error_info.unwrap_or(eval_result.result)
                                )));
                            } else {
                                let _ = tx.send(Ok(eval_result.result));
                            }
                        }
                        PendingEval::SetWatchpoint { reply, watchpoint } => {
                            if eval_result.error {
                                let _ = reply.send(Err(eval_result
                                    .error_info
                                    .unwrap_or(eval_result.result)));
                            } else {
                                self.watchpoints.retain(|w| w.name != watchpoint.name);
                                self.watchpoints.push(watchpoint);
                                let _ = reply.send(Ok(()));
                            }
                        }
                        PendingEval::ClearWatchpoint { reply, name } => {
                            if eval_result.error {
                                let _ = reply.send(Err(eval_result
                                    .error_info
                                    .unwrap_or(eval_result.result)));
                            } else {
                                self.watchpoints.retain(|w| w.name != name);
                                let _ = reply.send(Ok(()));
                            }
                        }
                        PendingEval::ClearAllWatchpoints { reply } => {
                            if eval_result.error {
                                let _ = reply.send(Err(eval_result
                                    .error_info
                                    .unwrap_or(eval_result.result)));
                            } else {
                                self.watchpoints.clear();
                                let _ = reply.send(Ok(()));
                            }
                        }
                    }
                } else {
                    // Unsolicited eval result — forward to frontend
                    let _ = self.app_handle.emit("debugger:eval_result", eval_result);
                }
            }

            AgentEvent::BytecodeChunk {
                sequence,
                command,
                proc_name,
                payload,
                payload_size,
                disassembly,
            } => {
                if !is_active(self.state) {
                    return;
                }

                let snapshot_result = crate::do_load_from_payload(payload.as_str(), &command);
                let (decompile_text, error) = match &snapshot_result {
                    Ok(load_result) => (Some(load_result.decompile_text.clone()), None),
                    Err(err) => (None, Some(err.clone())),
                };
                if let Some(disassembly_text) = disassembly.as_deref() {
                    let source_for_map = if payload.starts_with("TclPro ByteCode") {
                        self.script_source.as_deref().unwrap_or(payload.as_str())
                    } else {
                        payload.as_str()
                    };
                    self.stop_metadata_index
                        .ingest(disassembly_text, source_for_map);
                }

                let event = DebugBytecodeChunkEvent {
                    sequence,
                    command: command.clone(),
                    proc_name,
                    payload_size,
                    disassembly,
                    decompile_text,
                    error,
                };
                self.bytecode_chunks.push(event.clone());
                let _ = self.app_handle.emit("debugger:bytecode_chunk", event);

                if is_root_bytecode_command(&command)
                    && let Ok(load_result) = snapshot_result
                {
                    let _ = self.app_handle.emit("debugger:live_load", load_result);
                }
            }

            AgentEvent::CommandError {
                command,
                control_mode,
                code,
                message,
                error_info,
                error_code,
                argc,
            } => {
                if !is_active(self.state) {
                    return;
                }
                let _ = self.app_handle.emit(
                    "debugger:command_error",
                    DebugCommandErrorEvent {
                        command,
                        control_mode,
                        code,
                        message,
                        error_info,
                        error_code,
                        argc,
                    },
                );
            }

            AgentEvent::BreakpointAck { .. } | AgentEvent::BreakpointsCleared { .. } => {
                // Acknowledged — no frontend action needed currently
            }

            AgentEvent::TracePoint {
                file,
                line,
                frame_count: _,
                level,
                breakpoint_hit,
            } => {
                let _ = (&file, line);
                self.last_trace_level = level;

                let decision = if let Some(core) = &mut self.run_control {
                    core.decide_stop(level, breakpoint_hit)
                } else if breakpoint_hit {
                    Some(StopReason::Breakpoint)
                } else {
                    None
                };

                let (pause, reason) = if let Some(reason) = decision {
                    (true, Some(reason.as_str().to_string()))
                } else {
                    (false, None)
                };

                let _ = self
                    .send_to_agent(&AgentCommand::TraceDecision { pause, reason })
                    .await;
            }

            AgentEvent::WatchpointHit {
                watch_id,
                name,
                element,
                operation,
                old_value,
                new_value,
                file,
                line,
                frame_count,
            } => {
                if !is_active(self.state) {
                    return;
                }
                let event = DebugWatchpointHitEvent {
                    watch_id,
                    name,
                    element,
                    operation,
                    old_value,
                    new_value,
                    file,
                    line,
                    frame_count,
                };
                self.last_watchpoint_hit = Some(event.clone());
                let _ = self.app_handle.emit("debugger:watchpoint_hit", event);
            }

            // ── Reserved v0.4+ event kinds ───────────────────────────
            // Nothing emits these in v1 (their runtimes are gated:
            // live-disassembly-runtime, opcode-profiling-runtime,
            // epoch-tracking-runtime, compilation-introspection-runtime).
            // When a future agent sends one, it passes through verbatim on
            // its reserved channel; the GUI records it as an unhandled
            // event kind until the feature UI exists.
            AgentEvent::DisassemblyContext {
                proc_name,
                source_file,
                disassembly,
                pc_to_line,
            } => {
                let _ = self.app_handle.emit(
                    "debugger:disassembly_context",
                    DebugDisassemblyContextEvent {
                        proc_name,
                        source_file,
                        disassembly,
                        pc_to_line,
                    },
                );
            }

            AgentEvent::ProfileData {
                request_id,
                mode,
                line_data,
                opcode_data,
            } => {
                let _ = self.app_handle.emit(
                    "debugger:profile_data",
                    DebugProfileDataEvent {
                        request_id,
                        mode,
                        line_data,
                        opcode_data,
                    },
                );
            }

            AgentEvent::EpochChange {
                command,
                old_epoch,
                new_epoch,
                trigger,
                compile_epoch,
                file,
                line,
            } => {
                let _ = self.app_handle.emit(
                    "debugger:epoch_change",
                    DebugEpochChangeEvent {
                        command,
                        old_epoch,
                        new_epoch,
                        trigger,
                        compile_epoch,
                        file,
                        line,
                    },
                );
            }

            AgentEvent::CompilationEvent {
                trigger,
                command,
                file,
                line,
                bytecode_size,
                instruction_count,
                literal_count,
                disassembly,
                old_epoch,
                new_epoch,
            } => {
                let _ = self.app_handle.emit(
                    "debugger:compilation_event",
                    DebugCompilationEvent {
                        trigger,
                        command,
                        file,
                        line,
                        bytecode_size,
                        instruction_count,
                        literal_count,
                        disassembly,
                        old_epoch,
                        new_epoch,
                    },
                );
            }

            AgentEvent::Unknown => {
                // Forward-compat: an event kind newer than this build.
                // Log-and-ignore keeps the session alive against a newer
                // agent (mirrors the wire protocol's unknown-token rule).
                eprintln!(
                    "tcld-debugger: ignoring unknown agent event kind (newer than this build)"
                );
            }
        }
    }
}

fn parse_resolve_result(tcl_dict: &str) -> Result<CommandResolution, String> {
    fn dict_get<'a>(pairs: &'a [(String, String)], key: &str) -> &'a str {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    fn dict_get_bool(pairs: &[(String, String)], key: &str) -> bool {
        dict_get(pairs, key) == "1"
    }

    fn dict_get_opt(pairs: &[(String, String)], key: &str) -> Option<String> {
        let val = dict_get(pairs, key);
        if val.is_empty() {
            None
        } else {
            Some(val.to_string())
        }
    }

    let pairs = parse_tcl_dict_pairs(tcl_dict);
    if pairs.is_empty() {
        return Err(format!(
            "malformed resolve_command response: expected dict, got {:?}",
            tcl_dict
        ));
    }
    if dict_get(&pairs, "originalName").is_empty() || dict_get(&pairs, "exists").is_empty() {
        return Err(format!(
            "malformed resolve_command response: missing required fields in {:?}",
            tcl_dict
        ));
    }

    let import_chain_str = dict_get(&pairs, "importChain");
    let import_chain = parse_tcl_list(import_chain_str);

    let resolution_path_str = dict_get(&pairs, "resolutionPath");
    let resolution_path_items = parse_tcl_list(resolution_path_str);
    let resolution_path: Vec<ResolutionStep> = resolution_path_items
        .iter()
        .map(|item| {
            let step_pairs = parse_tcl_dict_pairs(item);
            ResolutionStep {
                step: dict_get(&step_pairs, "step").to_string(),
                namespace: dict_get_opt(&step_pairs, "namespace"),
                name: dict_get_opt(&step_pairs, "name"),
                source: dict_get_opt(&step_pairs, "source"),
                target: dict_get_opt(&step_pairs, "target"),
                result: dict_get(&step_pairs, "result").to_string(),
            }
        })
        .collect();

    Ok(CommandResolution {
        original_name: dict_get(&pairs, "originalName").to_string(),
        resolved_name: dict_get(&pairs, "resolvedName").to_string(),
        fully_qualified_name: dict_get(&pairs, "fullyQualifiedName").to_string(),
        namespace: dict_get(&pairs, "namespace").to_string(),
        resolved_in_namespace: dict_get(&pairs, "resolvedInNamespace").to_string(),
        is_imported: dict_get_bool(&pairs, "isImported"),
        import_source: dict_get_opt(&pairs, "importSource"),
        import_chain,
        is_ensemble: dict_get_bool(&pairs, "isEnsemble"),
        ensemble_namespace: dict_get_opt(&pairs, "ensembleNamespace"),
        is_alias: dict_get_bool(&pairs, "isAlias"),
        alias_target: dict_get_opt(&pairs, "aliasTarget"),
        command_type: dict_get(&pairs, "commandType").to_string(),
        exists: dict_get_bool(&pairs, "exists"),
        resolution_path,
    })
}

fn parse_tcl_dict_pairs(s: &str) -> Vec<(String, String)> {
    let items = parse_tcl_list(s);
    let mut pairs = Vec::new();
    let mut i = 0;
    while i + 1 < items.len() {
        pairs.push((items[i].clone(), items[i + 1].clone()));
        i += 2;
    }
    pairs
}

fn parse_tcl_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() {
        return vec![];
    }
    let mut items = vec![];
    let mut chars = s.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        if chars.peek() == Some(&'{') {
            chars.next();
            let mut depth = 1;
            let mut item = String::new();
            for c in chars.by_ref() {
                if c == '{' {
                    depth += 1;
                    item.push(c);
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    item.push(c);
                } else {
                    item.push(c);
                }
            }
            items.push(item);
        } else {
            let mut item = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\t' {
                    break;
                }
                item.push(c);
                chars.next();
            }
            items.push(item);
        }
    }
    items
}

fn command_to_resume_action(cmd: &AgentCommand) -> Option<ResumeAction> {
    match cmd {
        AgentCommand::StepInto => Some(ResumeAction::StepInto),
        AgentCommand::StepOver => Some(ResumeAction::StepOver),
        AgentCommand::StepOut => Some(ResumeAction::StepOut),
        AgentCommand::Continue => Some(ResumeAction::Continue),
        _ => None,
    }
}

fn is_root_bytecode_command(command: &str) -> bool {
    matches!(command, "::tbcload::bceval" | "::tbceval")
}

#[cfg(test)]
mod stop_metadata_tests {
    use super::{StopMetadataIndex, parse_disassembly_source_spans};

    #[test]
    fn parses_source_spans_from_disassembly() {
        let disassembly = r#"
ByteCode 0x0 refCt 1, epoch 1, interp 0x0 (epoch 1)
  Source "set a 1\nincr a\n"
  Cmds 2, src 14, inst 6, litObjs 2, aux 0, stkDepth 2, code/src 0.00
  Commands 2:
    1: pc 0-2, src 0-6
    2: pc 3-5, src 8-13
  Command 1: "set a 1"
      (0) push1 0         # "a"
      (2) pop
  Command 2: "incr a"
      (3) push1 0         # "a"
      (5) pop
"#;
        let source = "set a 1\nincr a\n";
        let spans = parse_disassembly_source_spans(disassembly, source);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].line_start, 1);
        assert_eq!(spans[0].instruction_index, 0);
        assert_eq!(spans[0].block_id, Some(0));
        assert_eq!(spans[1].line_start, 2);
        assert_eq!(spans[1].instruction_index, 2);
        assert_eq!(spans[1].block_id, Some(1));
    }

    #[test]
    fn resolves_line_to_instruction_index() {
        let disassembly = r#"
  Commands 2:
    1: pc 0-2, src 0-6
    2: pc 3-5, src 8-13
  Command 1: "set a 1"
      (0) push1 0
      (2) pop
  Command 2: "incr a"
      (3) push1 0
      (5) pop
"#;
        let source = "set a 1\nincr a\n";
        let mut index = StopMetadataIndex::default();
        index.ingest(disassembly, source);

        let first = index.resolve(1).expect("line 1 mapping");
        assert_eq!(first.instruction_index, 0);
        assert_eq!(first.pc, 0);
        assert_eq!(first.block_id, Some(0));

        let second = index.resolve(2).expect("line 2 mapping");
        assert_eq!(second.instruction_index, 2);
        assert_eq!(second.pc, 3);
        assert_eq!(second.block_id, Some(1));

        let by_pc = index.resolve_pc(5).expect("pc mapping");
        assert_eq!(by_pc.instruction_index, 3);
        assert_eq!(by_pc.pc, 5);
        assert_eq!(by_pc.block_id, Some(1));
    }
}

#[cfg(test)]
mod resolve_result_tests {
    use super::parse_resolve_result;

    #[test]
    fn parse_resolve_result_rejects_malformed_payload() {
        let err = parse_resolve_result("not_a_dict").expect_err("expected parse failure");
        assert!(err.contains("malformed resolve_command response"));
    }

    #[test]
    fn parse_resolve_result_accepts_not_found_dict() {
        let result = parse_resolve_result(
            "originalName {foo} resolvedName {} fullyQualifiedName {} namespace {::} resolvedInNamespace {} isImported 0 importChain {} isEnsemble 0 isAlias 0 commandType {unknown} exists 0 resolutionPath {}",
        )
        .expect("expected valid parsed result");
        assert_eq!(result.original_name, "foo");
        assert!(!result.exists);
    }
}

// ─── Tauri command handlers (thin wrappers) ──────────────────────────

#[tauri::command]
pub async fn debug_launch(
    path: String,
    args: Vec<String>,
    tclsh_path: Option<String>,
    launch_options: Option<DebugLaunchOptions>,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::Launch {
            path,
            args,
            tclsh_path,
            launch_options: launch_options.unwrap_or_default(),
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_stop(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::Stop { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_step_into(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::StepInto { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_step_instruction(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::StepInstruction { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_step_over_instruction(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::StepOverInstruction { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_step_out_instruction(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::StepOutInstruction { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_run_to_pc(cmd_tx: tauri::State<'_, CmdTx>, pc: u32) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::RunToPc { pc, reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_step_over(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::StepOver { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_step_out(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::StepOut { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_continue(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::Continue { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_pause(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::Pause { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_set_breakpoint(
    file: String,
    line: u32,
    offset: Option<u64>,
    len: Option<u64>,
    condition: Option<String>,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::SetBreakpoint {
            file,
            line,
            offset,
            len,
            condition,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_clear_breakpoint(
    file: String,
    line: u32,
    offset: Option<u64>,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::ClearBreakpoint {
            file,
            line,
            offset,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_set_breakpoint_enabled(
    file: String,
    line: u32,
    offset: Option<u64>,
    enabled: bool,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::SetBreakpointEnabled {
            file,
            line,
            offset,
            enabled,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_list_breakpoints(
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<Vec<Breakpoint>, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::ListBreakpoints { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped".to_string())
}

#[tauri::command]
pub async fn debug_eval(
    script: String,
    level: u32,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::Eval {
            script,
            level,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_eval_sync(
    script: String,
    level: u32,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<EvalResult, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::EvalSync {
            script,
            level,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    // Timeout at the caller level
    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "eval timeout (10s)".to_string())?
        .map_err(|_| "debug actor stopped".to_string())?
}

#[tauri::command]
pub async fn debug_get_variables(
    level: u32,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    cmd_tx
        .send(DebugCmd::GetVariables { level })
        .await
        .map_err(|_| "debug actor stopped")?;
    Ok(())
}

#[tauri::command]
pub async fn debug_get_stack(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    cmd_tx
        .send(DebugCmd::GetStack)
        .await
        .map_err(|_| "debug actor stopped")?;
    Ok(())
}

#[tauri::command]
pub async fn debug_set_variable(
    name: String,
    value: String,
    level: u32,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::SetVariable {
            name,
            value,
            level,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_get_state(cmd_tx: tauri::State<'_, CmdTx>) -> Result<serde_json::Value, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::GetState { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped".to_string())
}

#[tauri::command]
pub async fn debug_get_bytecode_chunks(
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<Vec<DebugBytecodeChunkEvent>, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::GetBytecodeChunks { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped".to_string())
}

#[tauri::command]
pub async fn debug_disassemble(
    path: String,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::Disassemble { path, reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "disassemble timeout (10s)".to_string())?
        .map_err(|_| "debug actor stopped".to_string())?
}

#[tauri::command]
pub async fn debug_resolve_command(
    name: String,
    namespace: Option<String>,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<CommandResolution, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::ResolveCommand {
            name,
            namespace,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "resolve timeout (10s)".to_string())?
        .map_err(|_| "debug actor stopped".to_string())?
}

#[tauri::command]
pub async fn debug_set_watchpoint(
    name: String,
    scope: Option<String>,
    level: Option<u32>,
    condition: Option<String>,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::SetWatchpoint {
            name,
            scope,
            level,
            condition,
            reply: tx,
        })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_clear_watchpoint(
    name: String,
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::ClearWatchpoint { name, reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}

#[tauri::command]
pub async fn debug_list_watchpoints(
    cmd_tx: tauri::State<'_, CmdTx>,
) -> Result<Vec<WatchpointInfo>, String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::ListWatchpoints { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped".to_string())
}

#[tauri::command]
pub async fn debug_clear_all_watchpoints(cmd_tx: tauri::State<'_, CmdTx>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    cmd_tx
        .send(DebugCmd::ClearAllWatchpoints { reply: tx })
        .await
        .map_err(|_| "debug actor stopped")?;
    rx.await.map_err(|_| "debug actor stopped")?
}
