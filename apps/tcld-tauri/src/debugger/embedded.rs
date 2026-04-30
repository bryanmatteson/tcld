use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc as tokio_mpsc;

use super::protocol::{AgentCommand, AgentEvent};
use super::{BytecodeHookMode, HookPlan};
use tcld_decompile::{DecompileOptions, decompile_path_with_options};

// ─── Backend handle ──────────────────────────────────────────────────

pub struct EmbeddedBackend {
    cmd_tx: std_mpsc::Sender<AgentCommand>,
    event_rx: tokio_mpsc::Receiver<AgentEvent>,
    thread: Option<thread::JoinHandle<()>>,
    runtime_temp_dir: Option<PathBuf>,
}

impl EmbeddedBackend {
    pub fn launch(
        script_path: String,
        args: Vec<String>,
        breakpoints: Vec<(String, u32, Option<String>)>,
        hook_plan: HookPlan,
        debug_extension_path: Option<String>,
        env: HashMap<String, String>,
        cwd: Option<String>,
    ) -> Result<Self, String> {
        let launch_source = prepare_launch_source(&script_path)?;
        let (event_tx, event_rx) = tokio_mpsc::channel(256);
        let (cmd_tx, cmd_rx) = std_mpsc::channel();

        let runtime_script_path = launch_source.runtime_path;
        let display_script_path = launch_source.display_path;
        let compat_namespaces = launch_source.compat_namespaces;
        let thread = thread::Builder::new()
            .name("tcl-interp".into())
            .spawn(move || {
                tcl_thread_main(
                    runtime_script_path,
                    display_script_path,
                    compat_namespaces,
                    args,
                    breakpoints,
                    hook_plan,
                    debug_extension_path,
                    env,
                    cwd,
                    event_tx,
                    cmd_rx,
                );
            })
            .map_err(|e| format!("failed to spawn Tcl thread: {e}"))?;

        Ok(EmbeddedBackend {
            cmd_tx,
            event_rx,
            thread: Some(thread),
            runtime_temp_dir: launch_source.temp_dir,
        })
    }

    pub fn send(&self, cmd: &AgentCommand) -> Result<(), String> {
        self.cmd_tx
            .send(cmd.clone())
            .map_err(|_| "Tcl thread exited".to_string())
    }

    pub fn take_event_rx(&mut self) -> tokio_mpsc::Receiver<AgentEvent> {
        let (_, empty) = tokio_mpsc::channel(1);
        std::mem::replace(&mut self.event_rx, empty)
    }

    pub fn stop(&mut self) {
        let _ = self.cmd_tx.send(AgentCommand::Continue);
        if let Some(handle) = self.thread.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if handle.is_finished() {
                    let _ = handle.join();
                    break;
                }
                if Instant::now() >= deadline {
                    eprintln!("[embedded-debugger] stop timeout; detaching Tcl thread");
                    drop(handle);
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        if let Some(dir) = self.runtime_temp_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Drop for EmbeddedBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

// ─── Tcl thread ──────────────────────────────────────────────────────

struct LaunchSource {
    runtime_path: String,
    display_path: String,
    temp_dir: Option<PathBuf>,
    compat_namespaces: Vec<String>,
}

fn prepare_launch_source(script_path: &str) -> Result<LaunchSource, String> {
    if !is_bytecode_path(script_path) {
        return Ok(LaunchSource {
            runtime_path: script_path.to_string(),
            display_path: script_path.to_string(),
            temp_dir: None,
            compat_namespaces: Vec::new(),
        });
    }

    let source = decompile_path_with_options(script_path, DecompileOptions).map_err(|err| {
        format!("failed to decompile {script_path} for embedded debugging: {err}")
    })?;
    let compat_namespaces = namespace_eval_names_for_compat(&source);
    let temp_dir = unique_temp_dir("tcld-embedded-debug")?;
    let file_name = Path::new(script_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("bytecode.tbc");
    let runtime_path = temp_dir.join(file_name);
    std::fs::write(&runtime_path, source)
        .map_err(|err| format!("failed to write {}: {err}", runtime_path.display()))?;

    Ok(LaunchSource {
        runtime_path: runtime_path.to_string_lossy().into_owned(),
        display_path: script_path.to_string(),
        temp_dir: Some(temp_dir),
        compat_namespaces,
    })
}

fn is_bytecode_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tbc"))
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    for attempt in 0..128u32 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| format!("clock error: {err}"))?
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{attempt}", std::process::id()));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("failed to create {}: {err}", dir.display())),
        }
    }
    Err(format!(
        "failed to allocate temporary directory for {prefix}"
    ))
}

fn namespace_eval_names_for_compat(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("namespace eval ") else {
            continue;
        };
        let mut token = rest.split_whitespace().next().unwrap_or("").trim();
        token = token.trim_matches(|c| c == '{' || c == '"');
        if token.is_empty() {
            continue;
        }
        if !names.iter().any(|name| name == token) {
            names.push(token.to_string());
        }
    }
    names
}

fn install_generated_bytecode_compat(
    interp: &tcld_interp::Interpreter,
    namespaces: &[String],
) -> Result<(), String> {
    if namespaces.is_empty() {
        return Ok(());
    }
    let ns_list = namespaces
        .iter()
        .map(|name| tcl_quote_for_list(name))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        r#"
foreach __tcld_ns [list {ns_list}] {{
    namespace eval $__tcld_ns {{
        if {{[array exists ::tcl_platform] && ![array exists tcl_platform]}} {{
            array set tcl_platform [array get ::tcl_platform]
        }}
        if {{[array exists ::env] && ![array exists env]}} {{
            array set env [array get ::env]
        }}
    }}
}}
"#
    );
    interp
        .eval(&script)
        .map(|_| ())
        .map_err(|err| format!("failed to install generated bytecode compatibility prelude: {err}"))
}

// Thread entry point: each argument is moved across the thread boundary once,
// so a params struct would be built and destructured at the same two sites.
#[allow(clippy::too_many_arguments)]
fn tcl_thread_main(
    runtime_script_path: String,
    display_script_path: String,
    compat_namespaces: Vec<String>,
    script_args: Vec<String>,
    initial_breakpoints: Vec<(String, u32, Option<String>)>,
    hook_plan: HookPlan,
    debug_extension_path: Option<String>,
    env: HashMap<String, String>,
    cwd: Option<String>,
    event_tx: tokio_mpsc::Sender<AgentEvent>,
    cmd_rx: std_mpsc::Receiver<AgentCommand>,
) {
    for (key, value) in env {
        // Embedded Tcl runs in-process, so honoring launch env necessarily updates
        // the process environment before the interpreter thread starts executing.
        unsafe { std::env::set_var(key, value) };
    }
    if let Some(cwd) = cwd
        && let Err(err) = std::env::set_current_dir(&cwd)
    {
        let _ = event_tx.blocking_send(AgentEvent::Exited {
            code: 1,
            reason: Some(format!("failed to set cwd {cwd}: {err}")),
        });
        return;
    }
    let interp = match tcld_interp::Interpreter::new() {
        Ok(i) => i,
        Err(e) => {
            let _ = event_tx.blocking_send(AgentEvent::Exited {
                code: 1,
                reason: Some(format!("failed to create Tcl interp: {e}")),
            });
            return;
        }
    };

    let tcl_version = interp
        .eval("info patchlevel")
        .unwrap_or_else(|_| "8.6".into());
    let pid = std::process::id();

    configure_script_argv(&interp, &display_script_path, &script_args);
    if let Err(err) = install_generated_bytecode_compat(&interp, &compat_namespaces) {
        let _ = event_tx.blocking_send(AgentEvent::Exited {
            code: 1,
            reason: Some(err),
        });
        return;
    }
    if let Err(err) = install_loader_intercept_if_needed(&interp, hook_plan.mode) {
        let _ = event_tx.blocking_send(AgentEvent::Exited {
            code: 1,
            reason: Some(err),
        });
        return;
    }
    let debug_extension_status =
        maybe_load_debug_extension(&interp, debug_extension_path.as_deref());

    let state = Arc::new(Mutex::new(TraceState {
        breakpoints: initial_breakpoints
            .into_iter()
            .map(|(file, line, condition)| BreakpointSpec {
                file: normalize_path_for_match(&file),
                line,
                offset: None,
                command: None,
                condition,
                enabled: true,
            })
            .collect(),
        control_mode: ControlMode::StepInto,
        last_stop_location: None,
        resume_origin_location: None,
        watchpoints: Vec::new(),
        next_watchpoint_id: 0,
    }));

    let state_for_trace = state.clone();
    let event_tx_for_trace = event_tx.clone();
    let cmd_rx_cell: Arc<Mutex<Option<std_mpsc::Receiver<AgentCommand>>>> =
        Arc::new(Mutex::new(Some(cmd_rx)));
    let cmd_rx_for_trace = cmd_rx_cell.clone();

    if let Err(err) =
        install_command_error_capture(&interp, &event_tx, state.clone(), &hook_plan.commands)
    {
        let _ = event_tx.blocking_send(AgentEvent::Exited {
            code: 1,
            reason: Some(err),
        });
        return;
    }

    override_puts(&interp, &event_tx);

    let _ = event_tx.blocking_send(AgentEvent::Ready {
        tcl_version: tcl_version.clone(),
        pid,
        debug_extension_loaded: Some(debug_extension_status.loaded),
        debug_extension_installed: Some(debug_extension_status.installed),
        debug_extension_capabilities: debug_extension_status.capabilities.clone(),
        debug_extension_reason: debug_extension_status.reason.clone(),
    });

    install_watchpoint_support(&interp);

    install_trace(
        &interp,
        state_for_trace,
        event_tx_for_trace,
        cmd_rx_for_trace,
        display_script_path,
        hook_plan,
    );

    let result = interp.eval_file(&runtime_script_path);
    let (code, reason) = match result {
        Ok(_) => (0, None),
        Err(e) => {
            if e.message.contains("exit") {
                (0, Some("script called exit".to_string()))
            } else {
                (1, Some(e.message))
            }
        }
    };

    let _ = event_tx.blocking_send(AgentEvent::Exited { code, reason });
}

// ─── Trace state ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlMode {
    StepInto,
    StepOver,
    StepOut,
    Continue,
    Pause,
}

struct TraceState {
    breakpoints: Vec<BreakpointSpec>,
    control_mode: ControlMode,
    last_stop_location: Option<TraceLocation>,
    resume_origin_location: Option<TraceLocation>,
    #[allow(dead_code)]
    // Watchpoint state — managed by Tcl scripts, reserved for C API enhancement
    watchpoints: Vec<WatchpointSpec>,
    #[allow(dead_code)]
    next_watchpoint_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraceLocation {
    file: String,
    line: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Watchpoint tracking — reserved for C API enhancement on Tcl 9.0+
struct WatchpointSpec {
    id: u64,
    name: String,
    scope: String,
    level: Option<u32>,
    condition: Option<String>,
    active: bool,
}

#[derive(Debug, Clone)]
struct BreakpointSpec {
    file: String,
    line: u32,
    offset: Option<u64>,
    command: Option<String>,
    condition: Option<String>,
    enabled: bool,
}

fn normalize_path_for_match(path: &str) -> String {
    let trimmed = path.trim_matches(|c| c == '"' || c == '{' || c == '}');
    if trimmed.is_empty() {
        return String::new();
    }

    let raw = trimmed.replace('\\', "/");
    let abs_or_raw = {
        let path_buf = PathBuf::from(&raw);
        if path_buf.is_absolute() {
            path_buf
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&path_buf))
                .unwrap_or(path_buf)
        }
    };

    let normalized_path = abs_or_raw
        .canonicalize()
        .unwrap_or(abs_or_raw)
        .to_string_lossy()
        .replace('\\', "/");

    if cfg!(windows) {
        normalized_path.to_ascii_lowercase()
    } else {
        normalized_path
    }
}

fn files_match(breakpoint_file: &str, frame_file: &str) -> bool {
    let bp = normalize_path_for_match(breakpoint_file);
    let frame = normalize_path_for_match(frame_file);
    if bp.is_empty() || frame.is_empty() {
        return false;
    }
    if bp == frame {
        return true;
    }

    let bp_name = Path::new(&bp).file_name().and_then(|s| s.to_str());
    let frame_name = Path::new(&frame).file_name().and_then(|s| s.to_str());
    if bp_name.is_some() && bp_name == frame_name {
        return true;
    }

    frame.ends_with(&format!("/{bp}")) || bp.ends_with(&format!("/{frame}"))
}

fn upsert_breakpoint(
    breakpoints: &mut Vec<BreakpointSpec>,
    file: String,
    line: u32,
    offset: Option<u64>,
    len: Option<u64>,
    condition: Option<String>,
) {
    let file = normalize_path_for_match(&file);
    let command = offset.zip(len).and_then(|(offset, len)| {
        let source = std::fs::read_to_string(&file).ok()?;
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(usize::try_from(len).ok()?)?;
        source.get(start..end).map(str::to_string)
    });
    breakpoints.retain(|bp| !(bp.file == file && bp.line == line && bp.offset == offset));
    breakpoints.push(BreakpointSpec {
        file,
        line,
        offset,
        command,
        condition,
        enabled: true,
    });
}

fn remove_breakpoint(
    breakpoints: &mut Vec<BreakpointSpec>,
    file: String,
    line: u32,
    offset: Option<u64>,
) {
    let file = normalize_path_for_match(&file);
    breakpoints.retain(|bp| !(bp.file == file && bp.line == line && bp.offset == offset));
}

fn set_breakpoint_enabled(
    breakpoints: &mut [BreakpointSpec],
    file: String,
    line: u32,
    offset: Option<u64>,
    enabled: bool,
) {
    let file = normalize_path_for_match(&file);
    if let Some(bp) = breakpoints
        .iter_mut()
        .find(|bp| bp.file == file && bp.line == line && bp.offset == offset)
    {
        bp.enabled = enabled;
    }
}

fn breakpoint_matches(bp: &BreakpointSpec, file: &str, line: u32, command: &str) -> bool {
    bp.enabled
        && bp.line == line
        && files_match(&bp.file, file)
        && bp
            .command
            .as_deref()
            .is_none_or(|expected| expected.trim() == command.trim())
}

fn trace_location(file: &str, line: u32) -> Option<TraceLocation> {
    if file.is_empty() || file == "unknown" || line == 0 {
        return None;
    }
    Some(TraceLocation {
        file: normalize_path_for_match(file),
        line,
    })
}

fn record_stop_location(state: &Arc<Mutex<TraceState>>, file: &str, line: u32) {
    let mut st = state.lock().unwrap();
    st.last_stop_location = trace_location(file, line);
    st.resume_origin_location = None;
}

fn arm_resume_origin(state: &Arc<Mutex<TraceState>>) {
    let mut st = state.lock().unwrap();
    st.resume_origin_location = st.last_stop_location.clone();
}

fn suppress_resume_origin_trace(state: &Arc<Mutex<TraceState>>, file: &str, line: u32) -> bool {
    let Some(location) = trace_location(file, line) else {
        return false;
    };
    let mut st = state.lock().unwrap();
    match st.resume_origin_location.as_ref() {
        Some(origin) if origin == &location => true,
        Some(_) => {
            st.resume_origin_location = None;
            false
        }
        None => false,
    }
}

fn normalize_command_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("::") {
        trimmed.to_string()
    } else {
        format!("::{trimmed}")
    }
}

fn is_bcproc_command(command: &str) -> bool {
    matches!(command, "::tbcload::bcproc" | "::tbcproc")
}

struct CommandErrorContext {
    tx: tokio_mpsc::Sender<AgentEvent>,
    state: Arc<Mutex<TraceState>>,
}

fn control_mode_from_state(state: &TraceState) -> String {
    match state.control_mode {
        ControlMode::StepInto => "step_into".to_string(),
        ControlMode::StepOver => "step_over".to_string(),
        ControlMode::StepOut => "step_out".to_string(),
        ControlMode::Continue => "continue".to_string(),
        ControlMode::Pause => "pause".to_string(),
    }
}

fn install_command_error_capture(
    interp: &tcld_interp::Interpreter,
    event_tx: &tokio_mpsc::Sender<AgentEvent>,
    state: Arc<Mutex<TraceState>>,
    hook_commands: &[String],
) -> Result<(), String> {
    if hook_commands.is_empty() {
        return Ok(());
    }

    let interp_ptr = unsafe { interp.raw() };
    let ctx_ptr = Box::into_raw(Box::new(CommandErrorContext {
        tx: event_tx.clone(),
        state,
    }));
    let cmd_name = std::ffi::CString::new("tcld_emit_command_error")
        .map_err(|e| format!("failed to build command name: {e}"))?;

    unsafe {
        tcl_sys::Tcl_CreateObjCommand(
            interp_ptr,
            cmd_name.as_ptr(),
            Some(command_error_emit_proc),
            ctx_ptr as *mut std::os::raw::c_void,
            Some(command_error_emit_delete),
        );
    }

    let hook_list = if hook_commands.is_empty() {
        "list".to_string()
    } else {
        format!(
            "list {}",
            hook_commands
                .iter()
                .map(|command| tcl_quote_for_list(command))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };

    let script = format!(
        r#"
set ::tcld_hook_cmds [{hook_list}]

proc ::tcld_wrap_command {{cmd}} {{
    if {{[llength [info commands $cmd]] == 0}} return

    set alias "::tcld_embedded_orig_[string map {{: _}} $cmd]"
    if {{[llength [info commands $alias]] > 0}} return

    rename $cmd $alias
    set body [string map [list __CMD__ $cmd __ALIAS__ $alias] {{
        set __tcld_result ""
        set __tcld_options {{}}
        set __tcld_code [catch {{uplevel 1 [list __ALIAS__ {{*}}$args]}} __tcld_result __tcld_options]
        if {{$__tcld_code == 1}} {{
            catch {{::tcld_emit_command_error "__CMD__" $__tcld_code $__tcld_result $__tcld_options [llength $args]}}
        }}
        if {{$__tcld_code == 0}} {{
            return $__tcld_result
        }}
        return -options $__tcld_options $__tcld_result
    }}]
    proc $cmd {{args}} $body
}}

proc ::tcld_wrap_hook_commands {{}} {{
    foreach __cmd $::tcld_hook_cmds {{
        catch {{::tcld_wrap_command $__cmd}}
    }}
}}

::tcld_wrap_hook_commands
if {{[llength [info commands ::tcld_embedded_wrap_original_proc]] == 0}} {{
    rename ::proc ::tcld_embedded_wrap_original_proc
    ::tcld_embedded_wrap_original_proc ::proc {{name arglist body}} {{
        ::tcld_embedded_wrap_original_proc $name $arglist $body
        set ns [uplevel 1 {{namespace current}}]
        if {{[string match "::*" $name]}} {{
            set fqn $name
        }} elseif {{$ns eq "::"}} {{
            set fqn "::$name"
        }} else {{
            set fqn "${{ns}}::${{name}}"
        }}
        if {{![string match "::tcld_*" $fqn] && [lsearch -exact $::tcld_hook_cmds $fqn] >= 0}} {{
            catch {{::tcld_wrap_command $fqn}}
        }}
    }}
}}

"#
    );

    interp
        .eval(&script)
        .map(|_| ())
        .map_err(|e| format!("failed to install command error capture: {e}"))
}

fn load_shared_bytecode_hook_support(interp: &tcld_interp::Interpreter) -> Result<(), String> {
    let shared = include_str!("../resources/tcld-bytecode-hooks.tcl");
    interp
        .eval(shared)
        .map(|_| ())
        .map_err(|e| format!("failed to load shared bytecode hook support: {e}"))
}

fn load_shared_debug_extension_support(interp: &tcld_interp::Interpreter) -> Result<(), String> {
    let shared = include_str!("../resources/tcld-debug-extension.tcl");
    interp
        .eval(shared)
        .map(|_| ())
        .map_err(|e| format!("failed to load shared debug extension support: {e}"))
}

#[derive(Debug, Clone, Default)]
struct DebugExtensionStatus {
    loaded: bool,
    installed: bool,
    capabilities: Option<String>,
    reason: Option<String>,
}

fn install_loader_intercept_if_needed(
    interp: &tcld_interp::Interpreter,
    mode: BytecodeHookMode,
) -> Result<(), String> {
    if mode != BytecodeHookMode::Intercept {
        return Ok(());
    }

    load_shared_bytecode_hook_support(interp)?;

    let script = r#"
if {[llength [info commands ::tcld_embedded_original_package]] == 0} {
    rename ::package ::tcld_embedded_original_package
}

proc ::package {args} {
    if {[llength $args] >= 2
        && [lindex $args 0] eq "require"
        && [string tolower [lindex $args 1]] eq "tbcload"} {
        ::tcld_bytecode_hooks::ensure_intercept_loader_shims
        if {[llength $args] >= 3 && [string trim [lindex $args 2]] ne ""} {
            return [lindex $args 2]
        }
        return 1.6
    }
    return [uplevel 1 [list ::tcld_embedded_original_package {*}$args]]
}
"#;

    interp
        .eval(script)
        .map(|_| ())
        .map_err(|e| format!("failed to install intercept loader shims: {e}"))
}

fn maybe_load_debug_extension(
    interp: &tcld_interp::Interpreter,
    debug_extension_path: Option<&str>,
) -> DebugExtensionStatus {
    let mut status = DebugExtensionStatus {
        reason: Some("disabled".to_string()),
        ..DebugExtensionStatus::default()
    };

    let Some(path) = debug_extension_path else {
        return status;
    };
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return status;
    }

    if let Err(err) = load_shared_debug_extension_support(interp) {
        status.reason = Some(format!("support_load_failed:{err}"));
        return status;
    }

    let script = format!(
        r#"
set ::__tcld_dbg_ext_result [::tcld_debug_extension::load_and_install {}]
"#,
        tcl_quote_for_list(trimmed),
    );

    if interp.eval(&script).is_err() {
        status.reason = Some("extension_load_failed".to_string());
        return status;
    }

    let loaded = interp
        .eval(
            r#"if {[dict exists $::__tcld_dbg_ext_result loaded]} {
    expr {[dict get $::__tcld_dbg_ext_result loaded] ? 1 : 0}
} else {
    0
}"#,
        )
        .unwrap_or_else(|_| "0".to_string());
    status.loaded = loaded.trim() == "1";

    let installed = interp
        .eval(
            r#"if {[dict exists $::__tcld_dbg_ext_result installed]} {
    expr {[dict get $::__tcld_dbg_ext_result installed] ? 1 : 0}
} else {
    0
}"#,
        )
        .unwrap_or_else(|_| "0".to_string());
    status.installed = installed.trim() == "1";

    let caps = interp
        .eval(
            r#"if {[dict exists $::__tcld_dbg_ext_result capabilities]} {
    dict get $::__tcld_dbg_ext_result capabilities
} else {
    ""
}"#,
        )
        .unwrap_or_default();
    let caps = caps.trim().to_string();
    if !caps.is_empty() {
        status.capabilities = Some(caps);
    }

    let reason = interp
        .eval(
            r#"if {[dict exists $::__tcld_dbg_ext_result reason]} {
    dict get $::__tcld_dbg_ext_result reason
} else {
    ""
}"#,
        )
        .unwrap_or_default();
    let reason = reason.trim().to_string();
    if !reason.is_empty() {
        status.reason = Some(reason);
    }

    let _ = interp.eval("unset -nocomplain ::__tcld_dbg_ext_result");
    status
}

// ─── Watchpoint support ──────────────────────────────────────────────

/// Pure-Tcl watchpoint support installed into the debugged interpreter.
/// Shared with the wire backend (`remote.rs`), which installs the same
/// script into the external nub so `::tcld::dbg::watch_variable` &c.
/// behave identically in both backends.
pub(super) const WATCHPOINT_SUPPORT_SCRIPT: &str = r#"
set ::__tcld_wp_hit 0
array set ::__tcld_wp_info {}

namespace eval ::tcld::dbg {
    variable _next_watch_id 0
    variable _watches
    catch {unset _watches}
    array set _watches {}

    proc ::tcld::dbg::watch_variable {name args} {
        variable _watches
        variable _next_watch_id

        set scope "local"
        set level ""
        set condition ""
        set i 0
        while {$i < [llength $args]} {
            set opt [lindex $args $i]
            switch -- $opt {
                -scope {
                    incr i
                    set scope [lindex $args $i]
                }
                -level {
                    incr i
                    set level [lindex $args $i]
                }
                -condition {
                    incr i
                    set condition [lindex $args $i]
                }
            }
            incr i
        }

        incr _next_watch_id
        set watchId $_next_watch_id

        set _watches($watchId) [dict create \
            name $name scope $scope level $level condition $condition active 1]

        set oldValue ""
        switch -- $scope {
            local {
                if {$level ne ""} {
                    catch {set oldValue [uplevel #$level [list set $name]]}
                } else {
                    catch {set oldValue [uplevel 1 [list set $name]]}
                }
                if {$level ne ""} {
                    uplevel #$level [list trace add variable $name write \
                        [list ::tcld::dbg::_watch_callback $watchId $oldValue]]
                } else {
                    uplevel 1 [list trace add variable $name write \
                        [list ::tcld::dbg::_watch_callback $watchId $oldValue]]
                }
            }
            global {
                catch {set oldValue [set ::$name]}
                trace add variable ::$name write \
                    [list ::tcld::dbg::_watch_callback $watchId $oldValue]
            }
            namespace {
                catch {set oldValue [set $name]}
                trace add variable $name write \
                    [list ::tcld::dbg::_watch_callback $watchId $oldValue]
            }
        }
        return $watchId
    }

    proc ::tcld::dbg::unwatch_variable {name} {
        variable _watches
        foreach watchId [array names _watches] {
            set info $_watches($watchId)
            if {[dict get $info name] eq $name} {
                set scope [dict get $info scope]
                catch {
                    switch -- $scope {
                        global {
                            foreach traceInfo [trace info variable ::$name] {
                                if {[string match "*::tcld::dbg::_watch_callback*" [lindex $traceInfo 1]]} {
                                    trace remove variable ::$name {*}$traceInfo
                                }
                            }
                        }
                        default {
                            foreach traceInfo [trace info variable $name] {
                                if {[string match "*::tcld::dbg::_watch_callback*" [lindex $traceInfo 1]]} {
                                    trace remove variable $name {*}$traceInfo
                                }
                            }
                        }
                    }
                }
                unset _watches($watchId)
            }
        }
    }

    proc ::tcld::dbg::list_watchpoints {} {
        variable _watches
        set result [list]
        foreach watchId [array names _watches] {
            set info $_watches($watchId)
            lappend result [dict merge $info [dict create id $watchId]]
        }
        return $result
    }

    proc ::tcld::dbg::clear_all_watchpoints {} {
        variable _watches
        set names [list]
        foreach watchId [array names _watches] {
            set name [dict get $_watches($watchId) name]
            if {[lsearch -exact $names $name] < 0} {
                lappend names $name
            }
        }
        foreach name $names {
            catch {::tcld::dbg::unwatch_variable $name}
        }
        array unset _watches *
    }

    proc ::tcld::dbg::_watch_callback {watchId oldValue name1 name2 op} {
        variable _watches

        if {![info exists _watches($watchId)]} return

        set watchInfo $_watches($watchId)
        if {![dict get $watchInfo active]} return

        set newValue ""
        if {$name2 ne ""} {
            catch {upvar 1 ${name1}($name2) __tcld_new_var; set newValue $__tcld_new_var}
        } else {
            catch {upvar 1 $name1 __tcld_new_var; set newValue $__tcld_new_var}
        }

        set condition [dict get $watchInfo condition]
        if {$condition ne ""} {
            if {[catch {set condResult [uplevel 1 [list expr $condition]]}] || !$condResult} {
                return
            }
        }

        array set ::__tcld_wp_info [list \
            name $name1 \
            element $name2 \
            operation $op \
            oldValue $oldValue \
            newValue $newValue \
            watchId $watchId \
        ]
        set ::__tcld_wp_hit 1
    }
}
"#;

fn install_watchpoint_support(interp: &tcld_interp::Interpreter) {
    if let Err(e) = interp.eval(WATCHPOINT_SUPPORT_SCRIPT) {
        eprintln!("[watchpoint-support] failed to install: {e}");
    }
}

struct WatchpointHitInfo {
    watch_id: u64,
    name: String,
    element: Option<String>,
    operation: String,
    old_value: String,
    new_value: String,
}

fn read_global_var(interp: *mut tcl_sys::Tcl_Interp, name: &str) -> Option<String> {
    let bare_name = name.strip_prefix("::").unwrap_or(name);
    let c_name = std::ffi::CString::new(bare_name).ok()?;
    unsafe {
        let obj = tcl_sys::Tcl_GetVar2Ex(
            interp,
            c_name.as_ptr(),
            std::ptr::null(),
            tcl_sys::TCL_GLOBAL_ONLY,
        );
        if obj.is_null() {
            return None;
        }
        let mut len: tcl_sys::Tcl_Size = 0;
        let ptr = tcl_sys::Tcl_GetStringFromObj(obj, &mut len);
        if ptr.is_null() {
            return None;
        }
        let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
        Some(String::from_utf8_lossy(slice).into_owned())
    }
}

fn read_array_element(
    interp: *mut tcl_sys::Tcl_Interp,
    name: &str,
    element: &str,
) -> Option<String> {
    let bare_name = name.strip_prefix("::").unwrap_or(name);
    let c_name = std::ffi::CString::new(bare_name).ok()?;
    let c_element = std::ffi::CString::new(element).ok()?;
    unsafe {
        let obj = tcl_sys::Tcl_GetVar2Ex(
            interp,
            c_name.as_ptr(),
            c_element.as_ptr(),
            tcl_sys::TCL_GLOBAL_ONLY,
        );
        if obj.is_null() {
            return None;
        }
        let mut len: tcl_sys::Tcl_Size = 0;
        let ptr = tcl_sys::Tcl_GetStringFromObj(obj, &mut len);
        if ptr.is_null() {
            return None;
        }
        let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
        Some(String::from_utf8_lossy(slice).into_owned())
    }
}

fn check_watchpoint_hit(interp: *mut tcl_sys::Tcl_Interp) -> Option<WatchpointHitInfo> {
    let hit_flag = read_global_var(interp, "::__tcld_wp_hit")?;
    if hit_flag.trim() != "1" {
        return None;
    }

    set_global_var(interp, "::__tcld_wp_hit", "0");

    let name = read_array_element(interp, "::__tcld_wp_info", "name").unwrap_or_default();
    let element_str = read_array_element(interp, "::__tcld_wp_info", "element").unwrap_or_default();
    let operation = read_array_element(interp, "::__tcld_wp_info", "operation")
        .unwrap_or_else(|| "write".to_string());
    let old_value = read_array_element(interp, "::__tcld_wp_info", "oldValue").unwrap_or_default();
    let new_value = read_array_element(interp, "::__tcld_wp_info", "newValue").unwrap_or_default();
    let watch_id = read_array_element(interp, "::__tcld_wp_info", "watchId")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let element = if element_str.is_empty() {
        None
    } else {
        Some(element_str)
    };

    Some(WatchpointHitInfo {
        watch_id,
        name,
        element,
        operation,
        old_value,
        new_value,
    })
}

// ─── Trace installation ──────────────────────────────────────────────

fn install_trace(
    interp: &tcld_interp::Interpreter,
    state: Arc<Mutex<TraceState>>,
    event_tx: tokio_mpsc::Sender<AgentEvent>,
    cmd_rx_cell: Arc<Mutex<Option<std_mpsc::Receiver<AgentCommand>>>>,
    target_script: String,
    hook_plan: HookPlan,
) {
    let interp_ptr = unsafe { interp.raw() };
    let hook_commands: HashSet<String> = hook_plan
        .commands
        .iter()
        .map(|command| normalize_command_name(command))
        .collect();

    unsafe {
        let state_clone = state.clone();
        let event_tx_clone = event_tx.clone();

        tcl_sys::Tcl_CreateObjTrace(
            interp_ptr,
            0,
            0,
            raw_trace_callback,
            Box::into_raw(Box::new(TraceContext {
                state: state_clone,
                event_tx: event_tx_clone,
                cmd_rx_cell,
                interp: interp_ptr,
                target_script,
                hook_commands,
                chunk_seq: AtomicU32::new(0),
            })) as *mut std::os::raw::c_void,
            Some(raw_trace_delete),
        );
    }
}

fn handle_running_command(state: &Arc<Mutex<TraceState>>, cmd: AgentCommand) {
    match cmd {
        AgentCommand::Pause => {
            let mut st = state.lock().unwrap();
            st.control_mode = ControlMode::Pause;
        }
        AgentCommand::SetBreakpoint {
            file,
            line,
            offset,
            len,
            condition,
        } => {
            let mut st = state.lock().unwrap();
            upsert_breakpoint(&mut st.breakpoints, file, line, offset, len, condition);
        }
        AgentCommand::ClearBreakpoint { file, line, offset } => {
            let mut st = state.lock().unwrap();
            remove_breakpoint(&mut st.breakpoints, file, line, offset);
        }
        AgentCommand::SetBreakpointEnabled {
            file,
            line,
            offset,
            enabled,
        } => {
            let mut st = state.lock().unwrap();
            set_breakpoint_enabled(&mut st.breakpoints, file, line, offset, enabled);
        }
        AgentCommand::ClearAllBreakpoints => {
            let mut st = state.lock().unwrap();
            st.breakpoints.clear();
        }
        // Step/continue commands are only valid from the paused command loop.
        AgentCommand::StepInstruction
        | AgentCommand::StepOverInstruction
        | AgentCommand::StepOutInstruction
        | AgentCommand::RunToPc { .. }
        | AgentCommand::StepInto
        | AgentCommand::StepOver
        | AgentCommand::StepOut
        | AgentCommand::Continue
        | AgentCommand::TraceDecision { .. } => {}
        AgentCommand::GetVariables { .. }
        | AgentCommand::GetStack { .. }
        | AgentCommand::Eval { .. }
        | AgentCommand::SetVariable { .. } => {}
        AgentCommand::SetWatchpoint { .. }
        | AgentCommand::ClearWatchpoint { .. }
        | AgentCommand::ClearAllWatchpoints
        | AgentCommand::ListWatchpoints { .. } => {}
    }
}

fn wait_for_trace_decision(
    cmd_rx_cell: &Arc<Mutex<Option<std_mpsc::Receiver<AgentCommand>>>>,
    state: &Arc<Mutex<TraceState>>,
) -> Option<(bool, Option<String>)> {
    let cmd_rx_guard = cmd_rx_cell.lock().unwrap();
    let cmd_rx = cmd_rx_guard.as_ref()?;

    loop {
        match cmd_rx.recv() {
            Ok(AgentCommand::TraceDecision { pause, reason }) => return Some((pause, reason)),
            Ok(AgentCommand::SetBreakpoint {
                file,
                line,
                offset,
                len,
                condition,
            }) => {
                let mut st = state.lock().unwrap();
                upsert_breakpoint(&mut st.breakpoints, file, line, offset, len, condition);
            }
            Ok(AgentCommand::ClearBreakpoint { file, line, offset }) => {
                let mut st = state.lock().unwrap();
                remove_breakpoint(&mut st.breakpoints, file, line, offset);
            }
            Ok(AgentCommand::SetBreakpointEnabled {
                file,
                line,
                offset,
                enabled,
            }) => {
                let mut st = state.lock().unwrap();
                set_breakpoint_enabled(&mut st.breakpoints, file, line, offset, enabled);
            }
            Ok(AgentCommand::ClearAllBreakpoints) => {
                let mut st = state.lock().unwrap();
                st.breakpoints.clear();
            }
            Ok(
                AgentCommand::Pause
                | AgentCommand::StepInstruction
                | AgentCommand::StepOverInstruction
                | AgentCommand::StepOutInstruction
                | AgentCommand::RunToPc { .. }
                | AgentCommand::StepInto
                | AgentCommand::StepOver
                | AgentCommand::StepOut
                | AgentCommand::Continue
                | AgentCommand::GetVariables { .. }
                | AgentCommand::GetStack { .. }
                | AgentCommand::Eval { .. }
                | AgentCommand::SetVariable { .. }
                | AgentCommand::SetWatchpoint { .. }
                | AgentCommand::ClearWatchpoint { .. }
                | AgentCommand::ClearAllWatchpoints
                | AgentCommand::ListWatchpoints { .. },
            ) => {}
            Err(_) => return None,
        }
    }
}

fn drain_running_commands(
    cmd_rx_cell: &Arc<Mutex<Option<std_mpsc::Receiver<AgentCommand>>>>,
    state: &Arc<Mutex<TraceState>>,
) {
    let cmd_rx_guard = cmd_rx_cell.lock().unwrap();
    let cmd_rx = match cmd_rx_guard.as_ref() {
        Some(rx) => rx,
        None => return,
    };

    while let Ok(cmd) = cmd_rx.try_recv() {
        handle_running_command(state, cmd);
    }
}

struct TraceContext {
    state: Arc<Mutex<TraceState>>,
    event_tx: tokio_mpsc::Sender<AgentEvent>,
    cmd_rx_cell: Arc<Mutex<Option<std_mpsc::Receiver<AgentCommand>>>>,
    interp: *mut tcl_sys::Tcl_Interp,
    target_script: String,
    hook_commands: HashSet<String>,
    chunk_seq: AtomicU32,
}

unsafe fn get_objv_string(objv: *const *mut tcl_sys::Tcl_Obj, index: usize) -> Option<String> {
    let obj = unsafe { *objv.add(index) };
    if obj.is_null() {
        return None;
    }
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { tcl_sys::Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    Some(String::from_utf8_lossy(slice).into_owned())
}

fn command_name_from_trace(
    command: &str,
    objc: std::os::raw::c_int,
    objv: *const *mut tcl_sys::Tcl_Obj,
) -> String {
    if !objv.is_null()
        && objc > 0
        && let Some(name) = unsafe { get_objv_string(objv, 0) }
    {
        return normalize_command_name(&name);
    }

    let fallback = command.split_whitespace().next().unwrap_or_default();
    normalize_command_name(fallback)
}

fn maybe_emit_bytecode_chunk(
    ctx: &TraceContext,
    normalized_command: &str,
    objc: std::os::raw::c_int,
    objv: *const *mut tcl_sys::Tcl_Obj,
) {
    if normalized_command.is_empty() || !ctx.hook_commands.contains(normalized_command) {
        return;
    }
    if objv.is_null() || objc <= 1 {
        return;
    }

    let (proc_name, payload_idx) = if is_bcproc_command(normalized_command) {
        (unsafe { get_objv_string(objv, 1) }, 3usize)
    } else {
        (None, 1usize)
    };

    if objc as usize <= payload_idx {
        return;
    }

    let Some(payload) = (unsafe { get_objv_string(objv, payload_idx) }) else {
        return;
    };
    if payload.is_empty() {
        return;
    }

    let sequence = ctx.chunk_seq.fetch_add(1, Ordering::Relaxed) + 1;
    let disassembly = disassemble_payload(ctx.interp, &payload);
    let _ = ctx.event_tx.blocking_send(AgentEvent::BytecodeChunk {
        sequence,
        command: normalized_command.to_string(),
        proc_name,
        payload: payload.clone(),
        payload_size: payload.len(),
        disassembly,
    });
}

fn disassemble_payload(interp: *mut tcl_sys::Tcl_Interp, payload: &str) -> Option<String> {
    if !set_global_var(interp, "::tcld_chunk_payload", payload) {
        return None;
    }
    tcl_eval_to_string(
        interp,
        "if {[catch {::tcl::unsupported::disassemble script $::tcld_chunk_payload} __tcld_chunk_dis]} {set __tcld_chunk_dis \"\"}; set __tcld_chunk_dis",
    )
    .filter(|text| !text.is_empty())
}

unsafe extern "C" fn command_error_emit_delete(client_data: *mut std::os::raw::c_void) {
    unsafe {
        let _ = Box::from_raw(client_data as *mut CommandErrorContext);
    }
}

unsafe extern "C" fn command_error_emit_proc(
    client_data: *mut std::os::raw::c_void,
    _interp: *mut tcl_sys::Tcl_Interp,
    objc: std::os::raw::c_int,
    objv: *const *mut tcl_sys::Tcl_Obj,
) -> std::os::raw::c_int {
    unsafe {
        if client_data.is_null() || objv.is_null() || objc < 6 {
            return tcl_sys::TCL_OK;
        }

        let ctx = &*(client_data as *const CommandErrorContext);
        let Some(command) = get_objv_string(objv, 1) else {
            return tcl_sys::TCL_OK;
        };
        let code = get_objv_string(objv, 2)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0);
        let message = get_objv_string(objv, 3).unwrap_or_default();
        let options_text = get_objv_string(objv, 4).unwrap_or_default();
        let argc = get_objv_string(objv, 5)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);

        let options = parse_tcl_dict(&options_text);
        let error_info = dict_lookup(&options, "-errorinfo").map(|s| s.to_string());
        let error_code = dict_lookup(&options, "-errorcode").map(|s| s.to_string());
        let control_mode = {
            let st = ctx.state.lock().unwrap();
            control_mode_from_state(&st)
        };

        let _ = ctx.tx.blocking_send(AgentEvent::CommandError {
            command,
            control_mode,
            code,
            message,
            error_info,
            error_code,
            argc,
        });

        tcl_sys::TCL_OK
    }
}

unsafe extern "C" fn raw_trace_callback(
    client_data: *mut std::os::raw::c_void,
    interp: *mut tcl_sys::Tcl_Interp,
    level: std::os::raw::c_int,
    command: *const std::os::raw::c_char,
    _command_token: tcl_sys::Tcl_Command,
    objc: std::os::raw::c_int,
    objv: *const *mut tcl_sys::Tcl_Obj,
) -> std::os::raw::c_int {
    unsafe {
        let ctx = &*(client_data as *const TraceContext);
        let cmd_str = std::ffi::CStr::from_ptr(command).to_string_lossy();
        let normalized_command = command_name_from_trace(&cmd_str, objc, objv);

        // Ignore our internal puts shim command, but allow user-level `puts`
        // to participate in stepping and first-line stop behavior.
        if normalized_command == "::tcld_puts_impl" {
            return tcl_sys::TCL_OK;
        }

        maybe_emit_bytecode_chunk(ctx, &normalized_command, objc, objv);

        // While running, we still need to process breakpoint updates so they
        // take effect at the next trace point.
        drain_running_commands(&ctx.cmd_rx_cell, &ctx.state);

        let (file, line, frame_count) = get_location(interp, &ctx.target_script);
        if suppress_resume_origin_trace(&ctx.state, &file, line) {
            return tcl_sys::TCL_OK;
        }

        let watchpoint_hit_info = check_watchpoint_hit(interp);

        let breakpoints = {
            let st = ctx.state.lock().unwrap();
            st.breakpoints.clone()
        };

        let breakpoint_hit = breakpoints.into_iter().any(|bp| {
            if !breakpoint_matches(&bp, &file, line, &cmd_str) {
                return false;
            }
            if let Some(condition) = bp.condition {
                return evaluate_breakpoint_condition(interp, level as u32, &condition);
            }
            true
        });

        if let Some(ref hit) = watchpoint_hit_info {
            let _ = ctx.event_tx.blocking_send(AgentEvent::WatchpointHit {
                watch_id: hit.watch_id,
                name: hit.name.clone(),
                element: hit.element.clone(),
                operation: hit.operation.clone(),
                old_value: hit.old_value.clone(),
                new_value: hit.new_value.clone(),
                file: file.clone(),
                line,
                frame_count,
            });
        }

        let effective_breakpoint_hit = breakpoint_hit || watchpoint_hit_info.is_some();

        let _ = ctx.event_tx.blocking_send(AgentEvent::TracePoint {
            file: file.clone(),
            line,
            frame_count,
            level,
            breakpoint_hit: effective_breakpoint_hit,
        });

        let Some((pause, reason)) = wait_for_trace_decision(&ctx.cmd_rx_cell, &ctx.state) else {
            return tcl_sys::TCL_OK;
        };
        if !pause {
            return tcl_sys::TCL_OK;
        }

        let stop_reason = if watchpoint_hit_info.is_some() && !breakpoint_hit {
            "watchpoint".to_string()
        } else {
            reason.unwrap_or_else(|| "step".to_string())
        };
        record_stop_location(&ctx.state, &file, line);

        let _ = ctx.event_tx.blocking_send(AgentEvent::Stopped {
            reason: stop_reason,
            file: file.clone(),
            line,
            frame_count,
            pc: None,
            instruction_index: None,
            block_id: None,
            error: None,
            error_info: None,
        });

        let cmd_rx_guard = ctx.cmd_rx_cell.lock().unwrap();
        let cmd_rx = match cmd_rx_guard.as_ref() {
            Some(rx) => rx,
            None => return tcl_sys::TCL_OK,
        };

        while let Ok(cmd) = cmd_rx.recv() {
            match cmd {
                AgentCommand::StepInstruction
                | AgentCommand::StepOverInstruction
                | AgentCommand::StepOutInstruction => {
                    match step_instruction_once(ctx.interp, &ctx.target_script) {
                        Ok(step_stop) => {
                            let _ = ctx.event_tx.blocking_send(AgentEvent::Stopped {
                                reason: "instruction_step".to_string(),
                                file: step_stop.file,
                                line: step_stop.line,
                                frame_count: step_stop.frame_count,
                                pc: step_stop.pc,
                                instruction_index: step_stop.instruction_index,
                                block_id: step_stop.block_id,
                                error: None,
                                error_info: None,
                            });
                        }
                        Err(err) => {
                            let _ = ctx.event_tx.blocking_send(AgentEvent::Output {
                                stream: "stderr".to_string(),
                                text: format!("opcode step failed: {err}"),
                            });
                        }
                    }
                }
                AgentCommand::RunToPc { .. } => {
                    let _ = ctx.event_tx.blocking_send(AgentEvent::Output {
                        stream: "stderr".to_string(),
                        text: "run-to-PC is not supported by the embedded backend".to_string(),
                    });
                }
                AgentCommand::StepInto => {
                    arm_resume_origin(&ctx.state);
                    let mut st = ctx.state.lock().unwrap();
                    st.control_mode = ControlMode::StepInto;
                    let _ = ctx.event_tx.blocking_send(AgentEvent::Resumed {
                        control_mode: "step_into".into(),
                    });
                    break;
                }
                AgentCommand::StepOver => {
                    arm_resume_origin(&ctx.state);
                    let mut st = ctx.state.lock().unwrap();
                    st.control_mode = ControlMode::StepOver;
                    let _ = ctx.event_tx.blocking_send(AgentEvent::Resumed {
                        control_mode: "step_over".into(),
                    });
                    break;
                }
                AgentCommand::StepOut => {
                    arm_resume_origin(&ctx.state);
                    let mut st = ctx.state.lock().unwrap();
                    st.control_mode = ControlMode::StepOut;
                    let _ = ctx.event_tx.blocking_send(AgentEvent::Resumed {
                        control_mode: "step_out".into(),
                    });
                    break;
                }
                AgentCommand::Continue => {
                    arm_resume_origin(&ctx.state);
                    let mut st = ctx.state.lock().unwrap();
                    st.control_mode = ControlMode::Continue;
                    let _ = ctx.event_tx.blocking_send(AgentEvent::Resumed {
                        control_mode: "continue".into(),
                    });
                    break;
                }
                AgentCommand::Pause => {}
                AgentCommand::SetBreakpoint {
                    file,
                    line,
                    offset,
                    len,
                    condition,
                } => {
                    let mut st = ctx.state.lock().unwrap();
                    upsert_breakpoint(&mut st.breakpoints, file, line, offset, len, condition);
                }
                AgentCommand::ClearBreakpoint { file, line, offset } => {
                    let mut st = ctx.state.lock().unwrap();
                    remove_breakpoint(&mut st.breakpoints, file, line, offset);
                }
                AgentCommand::SetBreakpointEnabled {
                    file,
                    line,
                    offset,
                    enabled,
                } => {
                    let mut st = ctx.state.lock().unwrap();
                    set_breakpoint_enabled(&mut st.breakpoints, file, line, offset, enabled);
                }
                AgentCommand::ClearAllBreakpoints => {
                    let mut st = ctx.state.lock().unwrap();
                    st.breakpoints.clear();
                }
                AgentCommand::GetVariables {
                    request_id,
                    level: var_level,
                } => {
                    let mut vars = get_variables(ctx.interp, var_level);
                    if has_obj_introspection(ctx.interp) {
                        enrich_variables_with_intrep(ctx.interp, &mut vars, var_level);
                    }
                    let _ = ctx.event_tx.blocking_send(AgentEvent::Variables {
                        request_id,
                        level: var_level,
                        vars,
                    });
                }
                AgentCommand::GetStack { request_id } => {
                    let frames = get_stack(ctx.interp);
                    let _ = ctx
                        .event_tx
                        .blocking_send(AgentEvent::Stack { request_id, frames });
                }
                AgentCommand::Eval {
                    request_id,
                    script,
                    level: eval_level,
                } => {
                    let (result, is_error, error_info, error_code) =
                        eval_in_context(ctx.interp, &script, eval_level);
                    let _ = ctx.event_tx.blocking_send(AgentEvent::EvalResult {
                        request_id,
                        result,
                        error: is_error,
                        error_info,
                        error_code,
                    });
                }
                AgentCommand::SetVariable {
                    name,
                    value,
                    level: set_level,
                } => {
                    let _ = set_variable(ctx.interp, &name, &value, set_level);
                }
                AgentCommand::TraceDecision { .. } => {}
                AgentCommand::SetWatchpoint { .. } => {}
                AgentCommand::ClearWatchpoint { .. } => {}
                AgentCommand::ClearAllWatchpoints => {}
                AgentCommand::ListWatchpoints { .. } => {}
            }
        }

        tcl_sys::TCL_OK
    }
}

unsafe extern "C" fn raw_trace_delete(client_data: *mut std::os::raw::c_void) {
    unsafe {
        let _ = Box::from_raw(client_data as *mut TraceContext);
    }
}

// ─── Tcl helpers (called from trace callback on the Tcl thread) ──────

pub(super) fn tcl_quote_for_list(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '[' => out.push_str("\\["),
            ']' => out.push_str("\\]"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn configure_script_argv(interp: &tcld_interp::Interpreter, script_path: &str, args: &[String]) {
    let list_script = if args.is_empty() {
        "list".to_string()
    } else {
        format!(
            "list {}",
            args.iter()
                .map(|arg| tcl_quote_for_list(arg))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };

    let argv_value = interp.eval(&list_script).unwrap_or_else(|_| String::new());
    let _ = interp.set_var("argv", &argv_value);
    let _ = interp.set_var("argc", &args.len().to_string());
    let _ = interp.set_var("argv0", script_path);
}

fn set_global_var(interp: *mut tcl_sys::Tcl_Interp, name: &str, value: &str) -> bool {
    let c_name = match std::ffi::CString::new(name) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let c_value = match std::ffi::CString::new(value) {
        Ok(s) => s,
        Err(_) => return false,
    };

    unsafe {
        let obj = tcl_sys::Tcl_NewStringObj(c_value.as_ptr(), -1);
        let result = tcl_sys::Tcl_SetVar2Ex(
            interp,
            c_name.as_ptr(),
            std::ptr::null(),
            obj,
            tcl_sys::TCL_GLOBAL_ONLY | tcl_sys::TCL_LEAVE_ERR_MSG,
        );
        !result.is_null()
    }
}

fn tcl_result_string(interp: *mut tcl_sys::Tcl_Interp) -> Option<String> {
    let obj = unsafe { tcl_sys::Tcl_GetObjResult(interp) };
    if obj.is_null() {
        return None;
    }

    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { tcl_sys::Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len < 0 {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn tcl_eval_to_string(interp: *mut tcl_sys::Tcl_Interp, script: &str) -> Option<String> {
    let c_script = std::ffi::CString::new(script).ok()?;
    let ret = unsafe { tcl_sys::Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if ret != tcl_sys::TCL_OK {
        return None;
    }
    tcl_result_string(interp)
}

fn parse_tcl_dict(dict_text: &str) -> Vec<(String, String)> {
    let items = split_tcl_list(dict_text);
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < items.len() {
        out.push((items[i].clone(), items[i + 1].clone()));
        i += 2;
    }
    out
}

fn dict_lookup<'a>(entries: &'a [(String, String)], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find_map(|(k, v)| if k == key { Some(v.as_str()) } else { None })
}

#[derive(Debug, Clone)]
struct StepStopMeta {
    file: String,
    line: u32,
    frame_count: u32,
    pc: Option<i32>,
    instruction_index: Option<usize>,
    block_id: Option<usize>,
}

fn parse_tcl_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn step_instruction_once(
    interp: *mut tcl_sys::Tcl_Interp,
    target_script: &str,
) -> Result<StepStopMeta, String> {
    let result = tcl_eval_to_string(interp, "::tcld::dbg::step_instruction").ok_or_else(|| {
        tcl_result_string(interp).unwrap_or_else(|| "step_instruction failed".to_string())
    })?;

    let fields = parse_tcl_dict(&result);
    if fields.is_empty() {
        return Err(format!("malformed step_instruction result: {result}"));
    }
    if let Some(err) = dict_lookup(&fields, "error")
        && !err.trim().is_empty()
    {
        return Err(err.to_string());
    }
    if let Some(ok) = dict_lookup(&fields, "ok")
        && !parse_tcl_bool(ok)
    {
        return Err(format!("step_instruction reported failure: {result}"));
    }

    let fallback = get_location(interp, target_script);
    let file = dict_lookup(&fields, "file")
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or(fallback.0);
    let file = display_file_for_target(file, target_script);
    let line = dict_lookup(&fields, "line")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(fallback.1);
    let frame_count = dict_lookup(&fields, "frameCount")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(fallback.2);
    let pc = dict_lookup(&fields, "pc").and_then(|s| s.parse::<i32>().ok());
    let instruction_index =
        dict_lookup(&fields, "instructionIndex").and_then(|s| s.parse::<usize>().ok());
    let block_id = dict_lookup(&fields, "blockId").and_then(|s| s.parse::<usize>().ok());

    Ok(StepStopMeta {
        file,
        line,
        frame_count,
        pc,
        instruction_index,
        block_id,
    })
}

fn has_obj_introspection(interp: *mut tcl_sys::Tcl_Interp) -> bool {
    tcl_eval_to_string(
        interp,
        "llength [info commands ::tcld::dbg::inspect_all_vars]",
    )
    .map(|s| s.trim() != "0")
    .unwrap_or(false)
}

fn enrich_variables_with_intrep(
    interp: *mut tcl_sys::Tcl_Interp,
    vars: &mut [serde_json::Value],
    level: u32,
) {
    let script = format!("::tcld::dbg::inspect_all_vars {level}");
    let Some(result) = tcl_eval_to_string(interp, &script) else {
        return;
    };

    let intrep_entries = split_tcl_list(&result);
    let mut intrep_map = std::collections::HashMap::new();
    for entry in &intrep_entries {
        let fields = parse_tcl_dict(entry);
        if let Some(name) = dict_lookup(&fields, "name") {
            intrep_map.insert(name.to_string(), fields);
        }
    }

    for var in vars.iter_mut() {
        let Some(name) = var.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(fields) = intrep_map.get(name) else {
            continue;
        };

        if let Some(type_name) = dict_lookup(fields, "type") {
            var["internalRep"] = serde_json::json!(type_name);
        }
        if let Some(detail) = dict_lookup(fields, "detail")
            && !detail.is_empty()
        {
            var["internalRepDetail"] = serde_json::json!(detail);
        }
        if let Some(has_str) = dict_lookup(fields, "hasStringRep") {
            var["hasStringRep"] = serde_json::json!(has_str == "1");
        }
        if let Some(rc) = dict_lookup(fields, "refCount")
            && let Ok(n) = rc.parse::<u32>()
        {
            var["refCount"] = serde_json::json!(n);
        }
    }
}

fn get_location(interp: *mut tcl_sys::Tcl_Interp, target_script: &str) -> (String, u32, u32) {
    let frame_count: u32 = tcl_eval_to_string(interp, "info frame")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    if frame_count == 0 {
        return ("unknown".into(), 0, 0);
    }

    let target_norm = normalize_path_for_match(target_script);
    let target_name = Path::new(&target_norm)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    let mut fallback_file = String::new();
    let mut fallback_line = 0u32;

    for i in (1..=frame_count).rev() {
        let Some(info_text) = tcl_eval_to_string(interp, &format!("info frame {i}")) else {
            continue;
        };
        let dict = parse_tcl_dict(&info_text);

        if let Some(proc_name) = dict_lookup(&dict, "proc")
            && proc_name.starts_with("::tcld_")
        {
            continue;
        }

        let file = dict_lookup(&dict, "file").unwrap_or("").to_string();
        let line = dict_lookup(&dict, "line")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);

        if fallback_file.is_empty() && (!file.is_empty() || line > 0) {
            fallback_file = file.clone();
            fallback_line = line;
        }

        if file.is_empty() {
            continue;
        }

        let file_norm = normalize_path_for_match(&file);
        if target_norm.is_empty() {
            return (file, line, frame_count);
        }

        if file_norm == target_norm {
            return (file, line, frame_count);
        }

        let file_name = Path::new(&file_norm).file_name().and_then(|s| s.to_str());
        if file_name == target_name.as_deref() {
            return (target_script.to_string(), line, frame_count);
        }
    }

    if !fallback_file.is_empty() || fallback_line > 0 {
        let file = if fallback_file.is_empty() && !target_script.is_empty() {
            target_script.to_string()
        } else if !target_script.is_empty() {
            let fallback_norm = normalize_path_for_match(&fallback_file);
            let fallback_name = Path::new(&fallback_norm)
                .file_name()
                .and_then(|s| s.to_str());
            if fallback_name == target_name.as_deref() {
                target_script.to_string()
            } else {
                fallback_file
            }
        } else {
            fallback_file
        };
        return (file, fallback_line, frame_count);
    }

    ("unknown".into(), 0, frame_count)
}

fn display_file_for_target(file: String, target_script: &str) -> String {
    if file.is_empty() || target_script.is_empty() {
        return file;
    }
    let file_norm = normalize_path_for_match(&file);
    let target_norm = normalize_path_for_match(target_script);
    let file_name = Path::new(&file_norm).file_name().and_then(|s| s.to_str());
    let target_name = Path::new(&target_norm).file_name().and_then(|s| s.to_str());
    if file_name == target_name {
        target_script.to_string()
    } else {
        file
    }
}

/// Tcl script that lists frame `level`'s variables as `{name type value}`
/// triples. Shared with the wire backend (`remote.rs`) so both backends
/// produce byte-identical variable payloads for the same frame.
pub(super) fn variables_script(level: u32) -> String {
    format!(
        "set __tcld_vars [list]; \
         foreach __tcld_v [uplevel #{level} {{info vars}}] {{ \
             if {{[uplevel #{level} [list array exists $__tcld_v]]}} {{ \
                 set __tcld_type array; \
                 set __tcld_val [uplevel #{level} [list array get $__tcld_v]]; \
             }} else {{ \
                 set __tcld_type scalar; \
                 catch {{set __tcld_val [uplevel #{level} [list set $__tcld_v]]}} __tcld_val; \
             }}; \
             lappend __tcld_vars [list $__tcld_v $__tcld_type $__tcld_val]; \
         }}; \
         set __tcld_vars"
    )
}

fn get_variables(interp: *mut tcl_sys::Tcl_Interp, level: u32) -> Vec<serde_json::Value> {
    let script = variables_script(level);
    let c_script = std::ffi::CString::new(script).unwrap();
    let ret = unsafe { tcl_sys::Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if ret != tcl_sys::TCL_OK {
        return vec![];
    }

    let Some(result) = tcl_result_string(interp) else {
        return vec![];
    };

    parse_variable_list(&result)
}

pub(super) fn parse_variable_list(tcl_list: &str) -> Vec<serde_json::Value> {
    let mut vars = vec![];

    let items = split_tcl_list(tcl_list);
    for item in items {
        let parts = split_tcl_list(&item);
        if parts.len() >= 3 {
            let name = &parts[0];
            let var_type = &parts[1];
            let value = &parts[2];

            let mut entry = serde_json::json!({
                "name": name,
                "type": var_type,
                "value": value,
                "scope": "local",
            });

            if var_type == "array" {
                let children = parse_array_children(name, value);
                entry["children"] = serde_json::json!(children);
            }

            vars.push(entry);
        }
    }
    vars
}

fn parse_array_children(name: &str, array_get: &str) -> Vec<serde_json::Value> {
    let items = split_tcl_list(array_get);
    let mut children = vec![];
    let mut i = 0;
    while i + 1 < items.len() {
        children.push(serde_json::json!({
            "name": format!("{}({})", name, items[i]),
            "type": "scalar",
            "value": items[i + 1],
            "scope": "local",
        }));
        i += 2;
    }
    children
}

pub(super) fn split_tcl_list(s: &str) -> Vec<String> {
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
        } else if chars.peek() == Some(&'"') {
            chars.next();
            let mut item = String::new();
            while let Some(c) = chars.next() {
                if c == '"' {
                    break;
                } else if c == '\\' {
                    if let Some(next) = chars.next() {
                        item.push(next);
                    }
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

fn get_stack(interp: *mut tcl_sys::Tcl_Interp) -> Vec<serde_json::Value> {
    let count_script = std::ffi::CString::new("info frame").unwrap();
    let ret = unsafe { tcl_sys::Tcl_EvalEx(interp, count_script.as_ptr(), -1, 0) };
    if ret != tcl_sys::TCL_OK {
        return vec![];
    }
    let Some(count_str) = tcl_result_string(interp) else {
        return vec![];
    };
    let count: u32 = count_str.parse().unwrap_or(0);

    let mut frames = vec![];
    for i in 1..=count {
        let script = format!("info frame {i}");
        let c_script = std::ffi::CString::new(script).unwrap();
        let ret = unsafe { tcl_sys::Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
        if ret != tcl_sys::TCL_OK {
            continue;
        }
        let Some(info) = tcl_result_string(interp) else {
            continue;
        };

        let parts: Vec<&str> = info.split_whitespace().collect();
        let mut frame = serde_json::json!({
            "level": i,
            "type": "source",
        });

        let mut j = 0;
        while j < parts.len() {
            match parts[j] {
                "type" if j + 1 < parts.len() => {
                    frame["type"] = serde_json::json!(parts[j + 1]);
                    j += 2;
                }
                "proc" if j + 1 < parts.len() => {
                    frame["procName"] = serde_json::json!(parts[j + 1]);
                    j += 2;
                }
                "file" if j + 1 < parts.len() => {
                    frame["file"] = serde_json::json!(parts[j + 1]);
                    j += 2;
                }
                "line" if j + 1 < parts.len() => {
                    if let Ok(line) = parts[j + 1].parse::<u32>() {
                        frame["line"] = serde_json::json!(line);
                    }
                    j += 2;
                }
                "cmd" if j + 1 < parts.len() => {
                    frame["cmd"] = serde_json::json!(parts[j + 1]);
                    j += 2;
                }
                _ => j += 1,
            }
        }

        frames.push(frame);
    }
    frames
}

fn evaluate_breakpoint_condition(
    interp: *mut tcl_sys::Tcl_Interp,
    level: u32,
    condition: &str,
) -> bool {
    if condition.trim().is_empty() {
        return true;
    }
    if !set_global_var(interp, "::tcld_bp_condition", condition) {
        return false;
    }

    let script = format!(
        "if {{[catch {{uplevel #{} [list expr $::tcld_bp_condition]}} __tcld_bp_result]}} {{ \
            if {{[catch {{expr $::tcld_bp_condition}} __tcld_bp_result]}} {{set __tcld_bp_result 0}} \
         }}; expr {{!!$__tcld_bp_result}}",
        level
    );
    tcl_eval_to_string(interp, &script)
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn eval_in_context(
    interp: *mut tcl_sys::Tcl_Interp,
    script: &str,
    level: u32,
) -> (String, bool, Option<String>, Option<String>) {
    if !set_global_var(interp, "::tcld_eval_script", script) {
        return ("failed to set eval script".to_string(), true, None, None);
    }
    let wrapped = format!("uplevel #{level} $::tcld_eval_script");
    let c_script = match std::ffi::CString::new(wrapped) {
        Ok(s) => s,
        Err(e) => return (format!("invalid eval wrapper: {e}"), true, None, None),
    };
    let ret = unsafe { tcl_sys::Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    let result = tcl_result_string(interp).unwrap_or_default();

    if ret == tcl_sys::TCL_OK {
        (result, false, None, None)
    } else {
        let error_info = get_var_str(interp, "errorInfo");
        let error_code = get_var_str(interp, "errorCode");
        (result, true, error_info, error_code)
    }
}

fn set_variable(interp: *mut tcl_sys::Tcl_Interp, name: &str, value: &str, level: u32) -> bool {
    if !set_global_var(interp, "::tcld_set_name", name)
        || !set_global_var(interp, "::tcld_set_value", value)
    {
        return false;
    }

    let script = format!("uplevel #{level} [list set $::tcld_set_name $::tcld_set_value]");
    let c_script = match std::ffi::CString::new(script) {
        Ok(s) => s,
        Err(_) => return false,
    };
    unsafe { tcl_sys::Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) == tcl_sys::TCL_OK }
}

fn get_var_str(interp: *mut tcl_sys::Tcl_Interp, name: &str) -> Option<String> {
    let c_name = std::ffi::CString::new(name).ok()?;
    unsafe {
        let obj = tcl_sys::Tcl_GetVar2Ex(interp, c_name.as_ptr(), std::ptr::null(), 0);
        if obj.is_null() {
            None
        } else {
            let mut len: tcl_sys::Tcl_Size = 0;
            let ptr = tcl_sys::Tcl_GetStringFromObj(obj, &mut len);
            if ptr.is_null() {
                None
            } else {
                let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
                Some(String::from_utf8_lossy(slice).into_owned())
            }
        }
    }
}

// ─── Output capture ──────────────────────────────────────────────────

fn override_puts(interp: &tcld_interp::Interpreter, event_tx: &tokio_mpsc::Sender<AgentEvent>) {
    let tx = event_tx.clone();
    let interp_ptr = unsafe { interp.raw() };

    let ctx = Box::new(PutsContext { tx });
    let ctx_ptr = Box::into_raw(ctx);

    unsafe {
        PUTS_CTX = ctx_ptr as *mut std::os::raw::c_void;
        PUTS_INTERP = interp_ptr;
    }

    let _ = interp.eval(
        "rename puts ::tcld_original_puts; \
         proc puts {args} { ::tcld_puts_impl {*}$args }",
    );

    let cmd_name = std::ffi::CString::new("tcld_puts_impl").unwrap();
    unsafe {
        tcl_sys::Tcl_CreateObjCommand(
            interp_ptr,
            cmd_name.as_ptr(),
            Some(puts_command_proc),
            std::ptr::null_mut(),
            None,
        );
    }
}

static mut PUTS_CTX: *mut std::os::raw::c_void = std::ptr::null_mut();
static mut PUTS_INTERP: *mut tcl_sys::Tcl_Interp = std::ptr::null_mut();

unsafe extern "C" fn puts_command_proc(
    _client_data: *mut std::os::raw::c_void,
    _interp: *mut tcl_sys::Tcl_Interp,
    objc: std::os::raw::c_int,
    objv: *const *mut tcl_sys::Tcl_Obj,
) -> std::os::raw::c_int {
    unsafe {
        if PUTS_CTX.is_null() {
            return tcl_sys::TCL_OK;
        }

        let ctx = &*(PUTS_CTX as *const PutsContext);

        let mut stream = "stdout".to_string();
        let mut text = String::new();
        let mut newline = true;

        let args: Vec<String> = (1..objc as usize)
            .filter_map(|i| {
                let obj = *objv.add(i);
                if obj.is_null() {
                    None
                } else {
                    let mut len: tcl_sys::Tcl_Size = 0;
                    let ptr = tcl_sys::Tcl_GetStringFromObj(obj, &mut len);
                    if ptr.is_null() {
                        None
                    } else {
                        let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
                        Some(String::from_utf8_lossy(slice).into_owned())
                    }
                }
            })
            .collect();

        match args.len() {
            1 => {
                text = args[0].clone();
            }
            2 => {
                if args[0] == "-nonewline" {
                    newline = false;
                    text = args[1].clone();
                } else {
                    stream = args[0].clone();
                    text = args[1].clone();
                }
            }
            3 => {
                if args[0] == "-nonewline" {
                    newline = false;
                    stream = args[1].clone();
                    text = args[2].clone();
                } else {
                    stream = args[0].clone();
                    text = args[1].clone();
                }
            }
            _ => {}
        }

        if newline {
            text.push('\n');
        }

        if stream == "stderr" {
            let _ = ctx.tx.blocking_send(AgentEvent::Output {
                stream: "stderr".into(),
                text,
            });
        } else {
            let _ = ctx.tx.blocking_send(AgentEvent::Output {
                stream: "stdout".into(),
                text,
            });
        }

        tcl_sys::TCL_OK
    }
}

struct PutsContext {
    tx: tokio_mpsc::Sender<AgentEvent>,
}

#[cfg(test)]
mod tests {
    use super::{
        AgentCommand, AgentEvent, BreakpointSpec, BytecodeHookMode, EmbeddedBackend, HookPlan,
        breakpoint_matches, files_match, load_shared_debug_extension_support,
        maybe_load_debug_extension, normalize_path_for_match,
    };
    use crate::debugger::core::{ResumeAction, RunControlCore};
    use std::collections::HashMap;
    use std::future::Future;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use tokio::sync::mpsc::Receiver;

    fn run_async_test<F>(fut: F)
    where
        F: Future<Output = ()>,
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(fut);
    }

    fn make_temp_script(name: &str, content: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        path.push(format!("tcld-{name}-{nanos}.tcl"));
        std::fs::write(&path, content).expect("write temp script");
        path
    }

    #[test]
    fn command_breakpoints_distinguish_nested_commands_on_one_line() {
        let breakpoint = BreakpointSpec {
            file: "/tmp/nested.tcl".into(),
            line: 2,
            offset: Some(24),
            command: Some("expr {1 + 1}".into()),
            condition: None,
            enabled: true,
        };
        assert!(breakpoint_matches(
            &breakpoint,
            "/tmp/nested.tcl",
            2,
            "expr {1 + 1}"
        ));
        assert!(!breakpoint_matches(
            &breakpoint,
            "/tmp/nested.tcl",
            2,
            "set x [expr {1 + 1}]"
        ));
    }

    fn repo_fixture(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
            .canonicalize()
            .expect("repo fixture")
    }

    async fn recv_event(rx: &mut Receiver<AgentEvent>) -> AgentEvent {
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed")
    }

    struct TestRunControl {
        core: RunControlCore,
        last_trace_level: i32,
    }

    impl TestRunControl {
        fn new() -> Self {
            Self {
                core: RunControlCore::new_start_step_into(),
                last_trace_level: 0,
            }
        }
    }

    fn resume_action_from_mode(control_mode: &str) -> Option<ResumeAction> {
        match control_mode {
            "step_into" => Some(ResumeAction::StepInto),
            "step_over" => Some(ResumeAction::StepOver),
            "step_out" => Some(ResumeAction::StepOut),
            "continue" => Some(ResumeAction::Continue),
            _ => None,
        }
    }

    async fn recv_event_with_control(
        backend: &EmbeddedBackend,
        rx: &mut Receiver<AgentEvent>,
        run: &mut TestRunControl,
    ) -> AgentEvent {
        loop {
            let evt = recv_event(rx).await;
            match evt {
                AgentEvent::TracePoint {
                    level,
                    breakpoint_hit,
                    ..
                } => {
                    run.last_trace_level = level;
                    let decision = run.core.decide_stop(level, breakpoint_hit);
                    let (pause, reason) = if let Some(reason) = decision {
                        (true, Some(reason.as_str().to_string()))
                    } else {
                        (false, None)
                    };
                    backend
                        .send(&AgentCommand::TraceDecision { pause, reason })
                        .expect("send trace decision");
                }
                AgentEvent::Resumed { control_mode } => {
                    if let Some(action) = resume_action_from_mode(&control_mode) {
                        run.core.on_resume(action, run.last_trace_level);
                    }
                    return AgentEvent::Resumed { control_mode };
                }
                other => return other,
            }
        }
    }

    async fn recv_stopped(
        backend: &EmbeddedBackend,
        rx: &mut Receiver<AgentEvent>,
        run: &mut TestRunControl,
    ) -> AgentEvent {
        loop {
            let evt = recv_event_with_control(backend, rx, run).await;
            if let AgentEvent::Exited { code, reason } = &evt {
                panic!("expected stopped event but received exit code={code}, reason={reason:?}");
            }
            if matches!(evt, AgentEvent::Stopped { .. }) {
                return evt;
            }
        }
    }

    async fn recv_ready_allowing_early_output(
        backend: &EmbeddedBackend,
        rx: &mut Receiver<AgentEvent>,
        run: &mut TestRunControl,
    ) {
        let mut seen = Vec::new();
        for _ in 0..16 {
            match recv_event_with_control(backend, rx, run).await {
                AgentEvent::Ready { .. } => return,
                AgentEvent::Output { stream, .. } => seen.push(format!("output:{stream}")),
                other => panic!("expected ready, got {other:?}; seen={seen:?}"),
            }
        }
        panic!("expected ready event; seen={seen:?}");
    }

    #[test]
    fn normalize_path_makes_relative_paths_stable() {
        let cwd = std::env::current_dir().expect("cwd");
        let rel = "target/tcld-path-normalize-test.tcl";
        let mut expected = cwd.join(rel).to_string_lossy().replace('\\', "/");
        if cfg!(windows) {
            expected = expected.to_ascii_lowercase();
        }
        let got = normalize_path_for_match(rel);
        assert!(got.ends_with("/target/tcld-path-normalize-test.tcl"));
        assert_eq!(got, expected);
    }

    #[test]
    fn files_match_handles_relative_vs_absolute() {
        let rel = PathBuf::from("target/tcld-files-match-test.tcl");
        let abs = std::env::current_dir().expect("cwd").join(&rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(&abs, "").expect("write");

        let abs_s = abs.to_string_lossy().into_owned();
        let rel_s = rel.to_string_lossy().into_owned();
        assert!(files_match(&abs_s, &rel_s));
        assert!(files_match(&rel_s, &abs_s));

        let _ = std::fs::remove_file(abs);
    }

    #[test]
    fn embedded_debug_extension_support_reports_missing_file() {
        let interp = tcld_interp::Interpreter::new().expect("interp");
        load_shared_debug_extension_support(&interp).expect("load shared support");
        let status = maybe_load_debug_extension(&interp, Some("/tmp/tcld-missing-debug-ext.dylib"));
        assert!(!status.loaded);
        assert!(!status.installed);
        assert_eq!(status.reason.as_deref(), Some("missing_file"));
    }

    #[test]
    fn embedded_stops_on_first_line_when_first_command_is_puts() {
        run_async_test(async {
            let script = make_temp_script("first-line-puts", "puts \"hello\"\nset x 1\n");
            let script_str = script.to_string_lossy().into_owned();

            let mut backend = EmbeddedBackend::launch(
                script_str,
                vec![],
                vec![],
                HookPlan::default(),
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            match recv_event_with_control(&backend, &mut rx, &mut run).await {
                AgentEvent::Ready {
                    debug_extension_reason,
                    ..
                } => {
                    assert_eq!(debug_extension_reason.as_deref(), Some("disabled"));
                }
                other => panic!("expected ready, got {other:?}"),
            }

            match recv_stopped(&backend, &mut rx, &mut run).await {
                AgentEvent::Stopped { line, reason, .. } => {
                    assert_eq!(line, 1, "first stop should be line 1");
                    assert_eq!(reason, "step");
                }
                other => panic!("expected stopped, got {other:?}"),
            }

            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let mut saw_exit = false;
            for _ in 0..16 {
                if let AgentEvent::Exited { .. } =
                    recv_event_with_control(&backend, &mut rx, &mut run).await
                {
                    saw_exit = true;
                    break;
                }
            }

            assert!(saw_exit, "expected process exit");

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }

    #[test]
    fn embedded_conditional_breakpoint_honors_condition() {
        run_async_test(async {
            let script =
                make_temp_script("conditional-bp", "set x 0\nincr x\nincr x\nset done ok\n");
            let script_str = script.to_string_lossy().into_owned();

            let mut backend = EmbeddedBackend::launch(
                script_str.clone(),
                vec![],
                vec![(script_str, 2, Some("$x == 0".to_string()))],
                HookPlan::default(),
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            match recv_event_with_control(&backend, &mut rx, &mut run).await {
                AgentEvent::Ready { .. } => {}
                other => panic!("expected ready, got {other:?}"),
            }

            match recv_stopped(&backend, &mut rx, &mut run).await {
                AgentEvent::Stopped { line, reason, .. } => {
                    assert_eq!(line, 1, "first stop should still be step at line 1");
                    assert_eq!(reason, "step");
                }
                other => panic!("expected stopped, got {other:?}"),
            }

            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let bp_stop = recv_stopped(&backend, &mut rx, &mut run).await;
            match bp_stop {
                AgentEvent::Stopped { line, reason, .. } => {
                    assert_eq!(line, 2, "conditional breakpoint should hit line 2");
                    assert_eq!(reason, "breakpoint");
                }
                other => panic!("expected stopped, got {other:?}"),
            }

            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let mut exited = false;
            for _ in 0..16 {
                if let AgentEvent::Exited { .. } =
                    recv_event_with_control(&backend, &mut rx, &mut run).await
                {
                    exited = true;
                    break;
                }
            }
            assert!(exited, "expected process exit");

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }

    #[test]
    fn embedded_sets_argv_argc_argv0_from_launch_args() {
        run_async_test(async {
            let script = make_temp_script("argv-bridge", "set sentinel 1\n");
            let script_str = script.to_string_lossy().into_owned();

            let mut backend = EmbeddedBackend::launch(
                script_str.clone(),
                vec!["foo".to_string(), "bar baz".to_string()],
                vec![],
                HookPlan::default(),
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            recv_ready_allowing_early_output(&backend, &mut rx, &mut run).await;
            let _ = recv_stopped(&backend, &mut rx, &mut run).await;

            backend
                .send(&AgentCommand::Eval {
                    request_id: 42,
                    script: "list $argc $argv0 [lindex $argv 0] [lindex $argv 1]".to_string(),
                    level: 0,
                })
                .expect("send eval");

            let mut eval_result: Option<String> = None;
            for _ in 0..8 {
                if let AgentEvent::EvalResult {
                    request_id,
                    result,
                    error,
                    ..
                } = recv_event_with_control(&backend, &mut rx, &mut run).await
                    && request_id == 42
                {
                    assert!(!error, "eval should succeed");
                    eval_result = Some(result);
                    break;
                }
            }
            let eval_result = eval_result.expect("expected eval result");
            assert!(
                eval_result.starts_with("2 "),
                "expected argc=2 in eval result: {eval_result}"
            );
            assert!(
                eval_result.contains(&script_str),
                "expected argv0 path in eval result: {eval_result}"
            );
            assert!(
                eval_result.contains(" foo "),
                "expected first arg in eval result: {eval_result}"
            );
            assert!(
                eval_result.contains("bar baz") || eval_result.contains("{bar baz}"),
                "expected second arg in eval result: {eval_result}"
            );

            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let mut saw_exit = false;
            for _ in 0..16 {
                if let AgentEvent::Exited { .. } =
                    recv_event_with_control(&backend, &mut rx, &mut run).await
                {
                    saw_exit = true;
                    break;
                }
            }

            assert!(saw_exit, "expected process exit");

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }

    #[test]
    fn embedded_streams_bytecode_chunk_for_custom_hook_command() {
        run_async_test(async {
            let script = make_temp_script(
                "custom-hook-chunk",
                "proc ::foo_loader {payload} { return $payload }\n::foo_loader {PAYLOAD}\n",
            );
            let script_str = script.to_string_lossy().into_owned();

            let hook_plan = HookPlan {
                mode: BytecodeHookMode::Passthrough,
                commands: vec!["::foo_loader".to_string()],
            };

            let mut backend = EmbeddedBackend::launch(
                script_str,
                vec![],
                vec![],
                hook_plan,
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            recv_ready_allowing_early_output(&backend, &mut rx, &mut run).await;
            let _ = recv_stopped(&backend, &mut rx, &mut run).await;
            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let mut saw_chunk = false;
            let mut saw_exit = false;

            for _ in 0..32 {
                match recv_event_with_control(&backend, &mut rx, &mut run).await {
                    AgentEvent::BytecodeChunk {
                        command, payload, ..
                    } if command == "::foo_loader" => {
                        saw_chunk = true;
                        assert_eq!(payload, "PAYLOAD");
                    }
                    AgentEvent::Stopped { reason, .. } if reason == "exception" => {
                        panic!("unexpected exception while waiting for custom hook chunk");
                    }
                    AgentEvent::Exited { code, reason } => {
                        assert_eq!(code, 0, "unexpected exit code: {reason:?}");
                        saw_exit = true;
                        break;
                    }
                    _ => {}
                }
            }

            assert!(saw_chunk, "expected a bytecode chunk for ::foo_loader");
            assert!(saw_exit, "expected script to exit cleanly");

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }

    #[test]
    fn embedded_emits_command_error_for_hook_command_failure() {
        run_async_test(async {
            let script = make_temp_script(
                "command-error-hook",
                "proc ::boom_loader {} { error \"boom\" }\ncatch {::boom_loader}\n",
            );
            let script_str = script.to_string_lossy().into_owned();

            let hook_plan = HookPlan {
                mode: BytecodeHookMode::Passthrough,
                commands: vec!["::boom_loader".to_string()],
            };

            let mut backend = EmbeddedBackend::launch(
                script_str,
                vec![],
                vec![],
                hook_plan,
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            recv_ready_allowing_early_output(&backend, &mut rx, &mut run).await;
            let _ = recv_stopped(&backend, &mut rx, &mut run).await;
            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let mut saw_error = false;
            let mut saw_exit = false;
            let mut seen = Vec::new();
            for _ in 0..32 {
                match recv_event_with_control(&backend, &mut rx, &mut run).await {
                    AgentEvent::CommandError {
                        command,
                        code,
                        message,
                        argc,
                        ..
                    } => {
                        if command == "::boom_loader" {
                            saw_error = true;
                            assert_eq!(code, 1);
                            assert_eq!(argc, 0);
                            assert!(message.contains("boom"));
                        }
                        seen.push(format!("command_error:{command}:{code}"));
                    }
                    AgentEvent::Exited { code, reason } => {
                        assert_eq!(code, 0, "unexpected exit code: {reason:?}");
                        saw_exit = true;
                        seen.push("exited".to_string());
                        break;
                    }
                    AgentEvent::Output { stream, .. } => seen.push(format!("output:{stream}")),
                    AgentEvent::Stopped { reason, line, .. } => {
                        seen.push(format!("stopped:{reason}:{line}"))
                    }
                    AgentEvent::Resumed { control_mode } => {
                        seen.push(format!("resumed:{control_mode}"))
                    }
                    AgentEvent::BytecodeChunk { command, .. } => {
                        seen.push(format!("chunk:{command}"))
                    }
                    other => seen.push(format!("other:{other:?}")),
                }
            }

            assert!(saw_error, "expected command_error event; seen={seen:?}");
            assert!(saw_exit, "expected clean exit");

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }

    #[test]
    fn embedded_watchpoint_support_installed() {
        run_async_test(async {
            let script = make_temp_script("watchpoint-support", "set sentinel 1\n");
            let script_str = script.to_string_lossy().into_owned();

            let mut backend = EmbeddedBackend::launch(
                script_str,
                vec![],
                vec![],
                HookPlan::default(),
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            match recv_event_with_control(&backend, &mut rx, &mut run).await {
                AgentEvent::Ready { .. } => {}
                other => panic!("expected ready, got {other:?}"),
            }
            let _ = recv_stopped(&backend, &mut rx, &mut run).await;

            backend
                .send(&AgentCommand::Eval {
                    request_id: 99,
                    script: "list [info exists ::__tcld_wp_hit] [info commands ::tcld::dbg::*] [namespace exists ::tcld::dbg]".to_string(),
                    level: 0,
                })
                .expect("send eval");

            let mut result = None;
            for _ in 0..8 {
                if let AgentEvent::EvalResult {
                    request_id: 99,
                    result: r,
                    error,
                    ..
                } = recv_event_with_control(&backend, &mut rx, &mut run).await
                {
                    assert!(!error, "eval should succeed: {r}");
                    result = Some(r);
                    break;
                }
            }
            let result = result.expect("expected eval result");
            assert!(
                result.contains("1"),
                "expected __tcld_wp_hit to exist; got: {result}"
            );
            assert!(
                result.contains("::tcld::dbg::watch_variable"),
                "expected watch_variable command; got: {result}"
            );

            backend.send(&AgentCommand::Continue).expect("continue");
            for _ in 0..8 {
                if let AgentEvent::Exited { .. } =
                    recv_event_with_control(&backend, &mut rx, &mut run).await
                {
                    break;
                }
            }

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }

    #[test]
    fn embedded_watchpoint_fires_on_variable_write() {
        run_async_test(async {
            let script = make_temp_script("watchpoint-basic", "set x 0\nset x 42\nset done ok\n");
            let script_str = script.to_string_lossy().into_owned();

            let mut backend = EmbeddedBackend::launch(
                script_str,
                vec![],
                vec![],
                HookPlan::default(),
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            match recv_event_with_control(&backend, &mut rx, &mut run).await {
                AgentEvent::Ready { .. } => {}
                other => panic!("expected ready, got {other:?}"),
            }

            match recv_stopped(&backend, &mut rx, &mut run).await {
                AgentEvent::Stopped { line, reason, .. } => {
                    assert_eq!(line, 1, "first stop should be line 1");
                    assert_eq!(reason, "step");
                }
                other => panic!("expected stopped, got {other:?}"),
            }

            backend
                .send(&AgentCommand::Eval {
                    request_id: 1,
                    script: "::tcld::dbg::watch_variable x -scope global".to_string(),
                    level: 0,
                })
                .expect("send eval to set watchpoint");

            let mut got_eval_result = false;
            for _ in 0..8 {
                if let AgentEvent::EvalResult { request_id: 1, .. } =
                    recv_event_with_control(&backend, &mut rx, &mut run).await
                {
                    got_eval_result = true;
                    break;
                }
            }
            assert!(got_eval_result, "expected eval result for watchpoint setup");

            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let mut saw_watchpoint_hit = false;
            let mut saw_watchpoint_stop = false;
            let mut saw_exit = false;
            let mut seen = Vec::new();

            for _ in 0..32 {
                match recv_event_with_control(&backend, &mut rx, &mut run).await {
                    AgentEvent::WatchpointHit { name, .. } => {
                        seen.push(format!("watchpoint_hit:{name}"));
                        assert_eq!(name, "x");
                        saw_watchpoint_hit = true;
                    }
                    AgentEvent::Stopped { reason, line, .. } => {
                        seen.push(format!("stopped:{reason}:{line}"));
                        if reason == "watchpoint" {
                            saw_watchpoint_stop = true;
                        }
                        backend
                            .send(&AgentCommand::Continue)
                            .expect("send continue after stop");
                    }
                    AgentEvent::Exited { code, reason } => {
                        seen.push(format!("exited:{code}:{reason:?}"));
                        saw_exit = true;
                        break;
                    }
                    other => {
                        seen.push(format!("other:{other:?}"));
                    }
                }
            }

            assert!(
                saw_watchpoint_hit,
                "expected watchpoint hit event for variable x; seen={seen:?}"
            );
            assert!(
                saw_watchpoint_stop,
                "expected stopped event with reason 'watchpoint'; seen={seen:?}"
            );
            assert!(saw_exit, "expected process exit; seen={seen:?}");

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }

    #[test]
    fn embedded_launches_public_fixture_breakpoint_and_source_step() {
        run_async_test(async {
            let script = repo_fixture("testing/fixtures/public/public_complex_fixture.tbc");
            let script_str = script.to_string_lossy().into_owned();

            let mut backend = EmbeddedBackend::launch(
                script_str.clone(),
                vec![],
                vec![(script_str.clone(), 15, None)],
                HookPlan::default(),
                crate::debugger::debug_extension_library_path(),
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            let can_step_instruction =
                match recv_event_with_control(&backend, &mut rx, &mut run).await {
                    AgentEvent::Ready {
                        debug_extension_capabilities,
                        ..
                    } => debug_extension_capabilities
                        .as_deref()
                        .is_some_and(|caps| caps.contains("instruction_step 1")),
                    other => panic!("expected ready, got {other:?}"),
                };

            match recv_stopped(&backend, &mut rx, &mut run).await {
                AgentEvent::Stopped {
                    file, line, reason, ..
                } => {
                    assert_eq!(file, script_str);
                    assert_eq!(line, 1, "first stop should be the decompiled source entry");
                    assert_eq!(reason, "step");
                }
                other => panic!("expected stopped, got {other:?}"),
            }

            backend
                .send(&AgentCommand::Continue)
                .expect("continue to breakpoint");

            match recv_stopped(&backend, &mut rx, &mut run).await {
                AgentEvent::Stopped {
                    file, line, reason, ..
                } => {
                    assert_eq!(file, script_str);
                    assert_eq!(
                        line, 15,
                        "breakpoint should map to the fixture::startlog definition"
                    );
                    assert_eq!(reason, "breakpoint");
                }
                other => panic!("expected breakpoint stop, got {other:?}"),
            }

            if can_step_instruction {
                backend
                    .send(&AgentCommand::StepInstruction)
                    .expect("instruction step from breakpoint");

                match recv_stopped(&backend, &mut rx, &mut run).await {
                    AgentEvent::Stopped {
                        file, reason, pc, ..
                    } => {
                        assert_eq!(file, script_str);
                        assert_eq!(reason, "instruction_step");
                        assert!(pc.is_some(), "instruction step should report a bytecode PC");
                    }
                    other => panic!("expected instruction step stop, got {other:?}"),
                }
            } else {
                backend
                    .send(&AgentCommand::StepInto)
                    .expect("source step from breakpoint");

                match recv_stopped(&backend, &mut rx, &mut run).await {
                    AgentEvent::Stopped {
                        file, line, reason, ..
                    } => {
                        assert_eq!(file, script_str);
                        assert!(
                            line > 15,
                            "source step should advance past breakpoint line 15, got line {line}"
                        );
                        assert_eq!(reason, "step");
                    }
                    other => panic!("expected source step stop, got {other:?}"),
                }
            }

            backend.send(&AgentCommand::Continue).expect("continue");
            for _ in 0..64 {
                if let AgentEvent::Exited { code, reason } =
                    recv_event_with_control(&backend, &mut rx, &mut run).await
                {
                    assert_eq!(
                        code, 0,
                        "unexpected public_complex_fixture.tbc exit reason: {reason:?}"
                    );
                    break;
                }
            }

            backend.stop();
        });
    }

    #[test]
    fn embedded_step_from_public_proc_definition_breakpoint_does_not_restop_at_same_line() {
        run_async_test(async {
            let script = repo_fixture("testing/fixtures/public/public_complex_fixture.tbc");
            let script_str = script.to_string_lossy().into_owned();

            let mut backend = EmbeddedBackend::launch(
                script_str.clone(),
                vec![],
                vec![(script_str.clone(), 74, None)],
                HookPlan::default(),
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            match recv_event_with_control(&backend, &mut rx, &mut run).await {
                AgentEvent::Ready { .. } => {}
                other => panic!("expected ready, got {other:?}"),
            }

            let first_stop = recv_stopped(&backend, &mut rx, &mut run).await;
            assert!(
                matches!(
                    first_stop,
                    AgentEvent::Stopped {
                        ref file,
                        line: 1,
                        ref reason,
                        ..
                    } if file == &script_str && reason == "step"
                ),
                "expected initial step stop at public_complex_fixture.tbc:1, got {first_stop:?}"
            );

            backend
                .send(&AgentCommand::Continue)
                .expect("continue to line 74");

            let breakpoint_stop = recv_stopped(&backend, &mut rx, &mut run).await;
            assert!(
                matches!(
                    breakpoint_stop,
                    AgentEvent::Stopped {
                        ref file,
                        line: 74,
                        ref reason,
                        ..
                    } if file == &script_str && reason == "breakpoint"
                ),
                "expected breakpoint stop at public_complex_fixture.tbc:74, got {breakpoint_stop:?}"
            );

            backend
                .send(&AgentCommand::StepInto)
                .expect("step away from line 74 breakpoint");

            match recv_stopped(&backend, &mut rx, &mut run).await {
                AgentEvent::Stopped { line, reason, .. } => {
                    assert!(
                        !(line == 74 && reason == "breakpoint"),
                        "step from public_complex_fixture.tbc:74 should not re-stop on the same breakpoint"
                    );
                }
                other => panic!("expected stop after stepping from line 74, got {other:?}"),
            }

            backend.send(&AgentCommand::Continue).expect("continue");
            for _ in 0..64 {
                if matches!(
                    recv_event_with_control(&backend, &mut rx, &mut run).await,
                    AgentEvent::Exited { .. }
                ) {
                    break;
                }
            }

            backend.stop();
        });
    }

    #[test]
    fn embedded_intercept_mode_bypasses_package_require_tbcload() {
        run_async_test(async {
            let script = make_temp_script(
                "intercept-hook-chunk",
                "proc ::tbceval {payload} { return $payload }\n\
                 if {[catch {package require tbcload 1.6} err]} { error \"require-failed:$err\" }\n\
                 tbcload::bceval {PAYLOAD}\n",
            );
            let script_str = script.to_string_lossy().into_owned();

            let hook_plan = HookPlan {
                mode: BytecodeHookMode::Intercept,
                commands: vec!["::tbcload::bceval".to_string()],
            };

            let mut backend = EmbeddedBackend::launch(
                script_str,
                vec![],
                vec![],
                hook_plan,
                None,
                HashMap::new(),
                None,
            )
            .expect("launch");
            let mut rx = backend.take_event_rx();
            let mut run = TestRunControl::new();

            match recv_event_with_control(&backend, &mut rx, &mut run).await {
                AgentEvent::Ready { .. } => {}
                other => panic!("expected ready, got {other:?}"),
            }
            let _ = recv_stopped(&backend, &mut rx, &mut run).await;
            backend
                .send(&AgentCommand::Continue)
                .expect("send continue");

            let mut saw_chunk = false;
            let mut saw_exit = false;

            for _ in 0..32 {
                match recv_event_with_control(&backend, &mut rx, &mut run).await {
                    AgentEvent::BytecodeChunk {
                        command, payload, ..
                    } if command == "::tbcload::bceval" => {
                        saw_chunk = true;
                        assert_eq!(payload, "PAYLOAD");
                    }
                    AgentEvent::Stopped { reason, error, .. } if reason == "exception" => {
                        panic!("unexpected exception in intercept mode: {error:?}");
                    }
                    AgentEvent::Exited { code, reason } => {
                        assert_eq!(code, 0, "unexpected exit code: {reason:?}");
                        saw_exit = true;
                        break;
                    }
                    _ => {}
                }
            }

            assert!(
                saw_chunk,
                "expected a bytecode chunk for ::tbcload::bceval in intercept mode"
            );
            assert!(saw_exit, "expected script to exit cleanly");

            backend.stop();
            let _ = std::fs::remove_file(script);
        });
    }
}
