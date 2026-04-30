//! External (wire-protocol) debugger backend.
//!
//! Spawns a separate `tclsh` process running the tcld nub
//! (`libtcld_dbg` + `tcld_nub_bootstrap.tcl`) and drives it with the
//! sync-blocking [`tcld_debug_wire::Controller`], mirroring the
//! embedded backend's `AgentCommand` → `AgentEvent` contract so the
//! debug actor and the GUI are backend-agnostic.
//!
//! ## Session shape
//!
//! The controller listens on `127.0.0.1:0` and the nub dials out (the
//! loopback topology). The controller stages argv while the nub is Ready,
//! then starts the real target through protocol v2 with stop-on-entry.
//! The backend maps that exact pre-command pause to the embedded backend's
//! existing Ready + Stopped contract.
//!
//! ## Semantics vs the embedded backend (grounded against a real nub)
//!
//! - Wire breakpoints address **proc bodies by byte offset**
//!   (`bp_add qname offset len`); the engine only ever arms
//!   `BlockId::ProcBody` blocks. `SetBreakpoint {file, line}` is
//!   translated with a local source map built from the launched script
//!   (`tcld_syntax::command_spans`, the same parser the nub's wrappers
//!   use to register blocks). Lines outside a proc body — top-level
//!   file code — get a defined "not supported by external backend"
//!   error, as do breakpoints in files other than the launched script.
//! - Breakpoints installed before their proc exists survive: `bp_add`
//!   registers a placeholder block that the nub revalidates when the
//!   real `proc` is defined (verified against tclsh 9.0.3).
//! - `interrupt`'s wire response can be swallowed by the very pause it
//!   triggers (the engine pauses inside the nub's response path), so
//!   `Pause` is sent with a short timeout and the timeout is not an
//!   error — the subsequent `paused` event is the real signal.
//! - Stepping from the entry gate would single-step through the nub's
//!   own `source`/`uplevel` wrapper machinery; it is rejected with a
//!   defined error instead (use continue-to-breakpoint from the gate).

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tcld_debug_wire::{
    BreakpointTarget, Controller, ControllerError, Op, PauseReason, SourceLocation, WireEvent,
    validate_nub_tcl_abi,
};
use tokio::sync::mpsc as tokio_mpsc;

use super::embedded::{
    WATCHPOINT_SUPPORT_SCRIPT, parse_variable_list, split_tcl_list, tcl_quote_for_list,
};
use super::protocol::{AgentCommand, AgentEvent};

/// Nub bootstrap embedded at build time so installed app bundles work
/// outside a workspace checkout (mirrors `apps/tcld/src/debug/config.rs`).
const EMBEDDED_BOOTSTRAP: &str =
    include_str!("../../../../crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl");

const HANDSHAKE_WAIT: Duration = Duration::from_secs(15);
const GATE_WAIT: Duration = Duration::from_secs(10);
const OP_TIMEOUT: Duration = Duration::from_secs(5);
const INTERRUPT_TIMEOUT: Duration = Duration::from_millis(750);
const EVENT_POLL: Duration = Duration::from_millis(25);
const TCLSH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Defined error for step requests while holding the launch gate.
pub const STEP_AT_GATE_ERROR: &str = "not supported by external backend: stepping from \
     the launch pause (the nub is paused before the script is sourced). Set a breakpoint and \
     continue instead.";

/// The defined diagnostic for a machine without the debugger nub.
/// Mirrors the wording contract of
/// `apps/tcld/src/debug/config.rs::nub_unavailable_message`, adapted to
/// the GUI's override surface (run-configuration fields instead of CLI
/// flags).
pub fn nub_unavailable_message() -> String {
    "external debugger unavailable: the debugger nub (libtcld_dbg.{dylib,so}) was not found.\n\
     Official CLI release archives bundle Tcl 8.5, 8.6, and 9 nubs when available. For a custom desktop build:\n\
       1. build a matching nub: `cargo build -p tcld-dbg-ext --features loadable-extension`\n\
          (use --no-default-features --features tcl8_5,loadable-extension for stock macOS 8.5), then\n\
       2. point the GUI at it with the run configuration's dylib path or TCLD_DBG_DYLIB=<path>.\n\
     A tclsh 8.5, 8.6, or 9 is also required (run configuration tclsh path or TCLD_TCLSH=<path>).\n\
     The embedded debugger backend works without the nub."
        .to_string()
}

fn no_tclsh_message() -> String {
    "external debugger unavailable: no tclsh found. Set the run configuration's tclsh path, \
     TCLD_TCLSH=<path>, or put tclsh9.0/tclsh8.6/tclsh8.5 on PATH."
        .to_string()
}

// ─── Nub path discovery ──────────────────────────────────────────────
//
// Minimal copy of `apps/tcld/src/debug/config.rs::locate_paths` (that
// crate is a binary; reusing it across crates would mean restructuring
// the CLI app, so the discovery contract is replicated here — noted in
// the task report). Resolution order per piece: explicit override →
// environment variable → discovery.

pub struct NubPaths {
    pub tclsh: PathBuf,
    pub dylib: PathBuf,
    pub bootstrap: PathBuf,
    /// Keep-alive guard for a temp file holding the embedded bootstrap
    /// copy; `None` when the bootstrap came from the workspace checkout.
    _bootstrap_guard: Option<TempFileGuard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TclAbi {
    Tcl85,
    Tcl86,
    Tcl9,
}

impl TclAbi {
    fn from_patchlevel(patchlevel: &str) -> Result<Self, String> {
        if patchlevel.starts_with("8.5.") {
            Ok(Self::Tcl85)
        } else if patchlevel.starts_with("8.6.") {
            Ok(Self::Tcl86)
        } else if patchlevel.starts_with("9.") {
            Ok(Self::Tcl9)
        } else {
            Err(format!(
                "unsupported Tcl patchlevel `{patchlevel}`; expected Tcl 8.5, 8.6, or 9"
            ))
        }
    }

    fn bundle_dir(self) -> &'static str {
        match self {
            Self::Tcl85 => "tcl8.5",
            Self::Tcl86 => "tcl8.6",
            Self::Tcl9 => "tcl9.0",
        }
    }

    fn target_dirs(self) -> [&'static str; 2] {
        match self {
            Self::Tcl85 => ["release-nub-tcl85", "debug-nub-tcl85"],
            Self::Tcl86 => ["release-nub-tcl86", "debug-nub-tcl86"],
            Self::Tcl9 => ["release-nub-tcl9", "debug-nub-tcl9"],
        }
    }

    fn versioned_shell_names(self) -> &'static [&'static str] {
        match self {
            Self::Tcl85 => &["tclsh8.5"],
            Self::Tcl86 => &["tclsh8.6"],
            Self::Tcl9 => &["tclsh9.0", "tclsh9"],
        }
    }

    fn prefix_env_key(self) -> &'static str {
        match self {
            Self::Tcl85 => "TCL8_5_PREFIX",
            Self::Tcl86 => "TCL8_6_PREFIX",
            Self::Tcl9 => "TCL9_PREFIX",
        }
    }
}

pub fn locate_nub_paths(
    tclsh_override: Option<&str>,
    dylib_override: Option<&str>,
) -> Result<NubPaths, String> {
    let (bootstrap, bootstrap_guard) = locate_bootstrap()?;
    let tclsh = locate_tclsh(tclsh_override)?;
    let abi = probe_tclsh_abi(&tclsh)?;
    Ok(NubPaths {
        tclsh,
        dylib: locate_dylib(dylib_override, abi)?,
        bootstrap,
        _bootstrap_guard: bootstrap_guard,
    })
}

fn locate_tclsh(override_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = override_path {
        let path = PathBuf::from(p);
        if !path.is_file() {
            return Err(format!(
                "tclsh not found at {} (run configuration tclsh path)",
                path.display()
            ));
        }
        return Ok(path);
    }
    if let Ok(p) = std::env::var("TCLD_TCLSH") {
        let path = PathBuf::from(&p);
        if !path.is_file() {
            return Err(format!("TCLD_TCLSH is set to {p} but no file exists there"));
        }
        return Ok(path);
    }
    locate_tclsh_in(std::env::var_os("PATH").as_deref(), &runtime_tcl_prefixes())
}

fn locate_tclsh_in(
    path_var: Option<&std::ffi::OsStr>,
    prefixes: &[(Option<TclAbi>, PathBuf)],
) -> Result<PathBuf, String> {
    for abi in [TclAbi::Tcl9, TclAbi::Tcl86, TclAbi::Tcl85] {
        let mut candidates = Vec::new();
        for name in abi.versioned_shell_names() {
            if let Some(path) = find_in_path_var(name, path_var) {
                candidates.push(path);
            }
        }
        for (_, prefix) in prefixes
            .iter()
            .filter(|(prefix_abi, _)| prefix_abi.is_none_or(|value| value == abi))
        {
            for name in abi.versioned_shell_names() {
                let path = prefix.join("bin").join(name);
                if path.is_file() && !candidates.contains(&path) {
                    candidates.push(path);
                }
            }
        }
        if let Some(path) = find_in_path_var("tclsh", path_var)
            && !candidates.contains(&path)
        {
            candidates.push(path);
        }
        for candidate in candidates {
            if probe_tclsh_abi(&candidate).ok() == Some(abi) {
                return Ok(candidate);
            }
        }
    }
    Err(no_tclsh_message())
}

fn find_in_path_var(name: &str, path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path_var?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn runtime_tcl_prefixes() -> Vec<(Option<TclAbi>, PathBuf)> {
    let mut prefixes = Vec::new();
    for abi in [TclAbi::Tcl9, TclAbi::Tcl86, TclAbi::Tcl85] {
        if let Some(prefix) = std::env::var_os(abi.prefix_env_key()) {
            prefixes.push((Some(abi), PathBuf::from(prefix)));
        }
    }
    if let Some(prefix) = std::env::var_os("TCL_PREFIX") {
        prefixes.push((None, PathBuf::from(prefix)));
    }
    for (abi, prefix) in [
        (TclAbi::Tcl9, "/opt/homebrew/opt/tcl-tk"),
        (TclAbi::Tcl9, "/opt/homebrew/opt/tcl-tk@9"),
        (TclAbi::Tcl9, "/usr/local/opt/tcl-tk"),
        (TclAbi::Tcl9, "/usr/local/opt/tcl-tk@9"),
        (TclAbi::Tcl86, "/opt/homebrew/opt/tcl-tk@8"),
        (TclAbi::Tcl86, "/usr/local/opt/tcl-tk@8"),
        (TclAbi::Tcl86, "/opt/local"),
        (TclAbi::Tcl9, "/opt/local"),
    ] {
        prefixes.push((Some(abi), PathBuf::from(prefix)));
    }
    prefixes
}

fn probe_tclsh_abi(path: &std::path::Path) -> Result<TclAbi, String> {
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to probe {}: {error}", path.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(b"puts -nonewline [info patchlevel]\n")
            .map_err(|error| format!("failed to probe {}: {error}", path.display()))?;
    }
    let deadline = Instant::now() + TCLSH_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_string(&mut stdout);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                if !status.success() {
                    return Err(format!(
                        "{} exited with {status} while probing Tcl: {}",
                        path.display(),
                        stderr.trim()
                    ));
                }
                return TclAbi::from_patchlevel(stdout.trim());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} timed out while probing Tcl patchlevel",
                    path.display()
                ));
            }
            Err(error) => return Err(format!("failed to probe {}: {error}", path.display())),
        }
    }
}

fn locate_dylib(override_path: Option<&str>, abi: TclAbi) -> Result<PathBuf, String> {
    if let Some(p) = override_path {
        let path = PathBuf::from(p);
        if !path.exists() {
            return Err(format!("debugger dylib not found at {}", path.display()));
        }
        return Ok(path);
    }
    if let Ok(p) = std::env::var("TCLD_DBG_DYLIB") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
    }
    if let Ok(p) = std::env::var("TCLD_DEBUG_EXTENSION_PATH") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let current_exe = std::env::current_exe().ok();
    for candidate in automatic_dylib_candidates(abi, &workspace, current_exe.as_deref()) {
        if candidate.exists() {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    Err(nub_unavailable_message())
}

fn automatic_dylib_candidates(
    abi: TclAbi,
    workspace: &std::path::Path,
    current_exe: Option<&std::path::Path>,
) -> Vec<PathBuf> {
    let file = super::dbg_extension_library_filename();
    let mut candidates = Vec::new();
    if let Some(bin_dir) = current_exe.and_then(std::path::Path::parent) {
        for base in [
            bin_dir.join("lib/tcld/nub"),
            bin_dir.join("../lib/tcld/nub"),
            bin_dir.join("resources/lib/tcld/nub"),
            bin_dir.join("../Resources/lib/tcld/nub"),
        ] {
            candidates.push(base.join(abi.bundle_dir()).join(file));
        }
    }
    for target_dir in abi.target_dirs() {
        for profile in ["release", "debug"] {
            candidates.push(
                workspace
                    .join("target")
                    .join(target_dir)
                    .join(profile)
                    .join(file),
            );
        }
    }
    // Generic Cargo and adjacent extension paths are default-Tcl-9 artifacts
    // without ABI metadata. Never consider them for an older interpreter.
    if abi == TclAbi::Tcl9 {
        for profile in ["debug", "release"] {
            candidates.push(workspace.join("target").join(profile).join(file));
        }
        if let Some(bin_dir) = current_exe.and_then(std::path::Path::parent) {
            candidates.push(bin_dir.join(file));
            candidates.push(bin_dir.join("../lib").join(file));
        }
    }
    candidates
}

fn locate_bootstrap() -> Result<(PathBuf, Option<TempFileGuard>), String> {
    // Workspace checkout: use the canonical script on disk.
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl");
    if workspace.is_file() {
        return Ok((workspace, None));
    }
    // Installed binary: materialize the embedded copy into a temp file
    // that lives as long as the debug session.
    let guard = TempFileGuard::create("tcld-nub-bootstrap", "tcl", EMBEDDED_BOOTSTRAP)?;
    Ok((guard.path.clone(), Some(guard)))
}

/// Owns a uniquely-named temp file; deletes it on drop.
struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    fn create(stem: &str, ext: &str, contents: &str) -> Result<Self, String> {
        use std::sync::atomic::AtomicU64;
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{stem}-{}-{n}.{ext}", std::process::id()));
        std::fs::write(&path, contents)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ─── Nub process guard ───────────────────────────────────────────────

/// Kill-on-drop guard for the spawned nub, mirroring
/// `tcld_debug_wire::NubChild` semantics (grace period for voluntary
/// exit, then a hard kill) so no `tclsh` is ever orphaned.
///
/// `Controller::spawn_nub` (which returns `NubChild`) is not used
/// because it offers no hooks for launch-option env vars, a working
/// directory, or piped stdio — and output capture (`AgentEvent::Output`
/// parity with the embedded backend) requires piped stdio. The spawn
/// here honors the exact same documented bootstrap env contract
/// (`TCLD_DBG_DYLIB` / `TCLD_NUB_HOST` / `TCLD_NUB_PORT`). Targets
/// start only after the v2 `start_target` request.
struct NubProcGuard {
    child: Option<Child>,
}

impl NubProcGuard {
    fn try_exit_code(&mut self) -> Option<i32> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            _ => None,
        }
    }

    /// Best-effort termination: brief grace for voluntary exit, then
    /// SIGKILL (short grace instead of NubChild's 5 s because a nub
    /// parked in `vwait forever` never exits voluntarily and GUI stop
    /// should be snappy).
    fn kill_now(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                _ => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for NubProcGuard {
    fn drop(&mut self) {
        self.kill_now();
    }
}

// ─── Source map: file:line ⇄ (proc qname, body byte offset) ──────────

#[derive(Debug, Clone)]
struct CommandPoint {
    /// Byte offset of the command start within the proc body — the
    /// exact coordinate `bp_add` and the nub's trace resolution use.
    offset: u64,
    /// 1-based line in the script file where that command starts.
    file_line: u32,
}

#[derive(Debug, Clone)]
struct ProcSpan {
    qname: String,
    /// Byte offset of the body text (inside the braces) in the file.
    body_start_byte: usize,
    body_end_byte: usize,
    body_first_line: u32,
    body_last_line: u32,
    commands: Vec<CommandPoint>,
}

pub(crate) struct SourceMap {
    script_path: String,
    procs: Vec<ProcSpan>,
    command_ranges: Vec<(u64, u64)>,
    first_statement_line: u32,
}

impl SourceMap {
    pub(crate) fn build(script_path: &str, source: &str) -> Self {
        let line_starts = line_starts_for(source);
        let line_of = |pos: usize| byte_to_line(pos, &line_starts, source.len());

        let normalized;
        let parse_input: &str = if source.ends_with('\n') || source.is_empty() {
            source
        } else {
            normalized = format!("{source}\n");
            &normalized
        };
        let spans = tcld_syntax::command_spans(parse_input).unwrap_or_default();
        let resolved_proc_bodies = tcld_syntax::proc_body_spans(parse_input).unwrap_or_default();

        let first_statement_line = spans
            .iter()
            .map(|s| (s.start_row + 1) as u32)
            .min()
            .unwrap_or(1);
        let command_ranges = spans
            .iter()
            .map(|span| {
                (
                    span.start_byte as u64,
                    span.end_byte.saturating_sub(span.start_byte) as u64,
                )
            })
            .collect();

        let mut procs = Vec::new();
        for span in &spans {
            if span.name.as_deref() != Some("proc") {
                continue;
            }
            let words = scan_tcl_words(&span.text);
            if words.len() != 4 {
                continue;
            }
            let name = span.text[words[1].0..words[1].1].trim();
            if name.is_empty() || name.contains('$') || name.contains('[') {
                continue; // dynamically-computed name; cannot map statically
            }
            let qname = name.trim_start_matches("::").to_string();
            let (body_word_start, body_word_end) = words[3];
            let body_text = &span.text[body_word_start..body_word_end];
            let (body_rel_start, body) =
                if body_text.starts_with('{') && body_text.ends_with('}') && body_text.len() >= 2 {
                    (body_word_start + 1, &body_text[1..body_text.len() - 1])
                } else {
                    continue; // unbraced body; runtime text differs from source
                };
            let body_start_byte = span.start_byte + body_rel_start;
            let resolved_qname = resolved_proc_bodies
                .iter()
                .find(|proc_body| proc_body.start_byte == body_start_byte)
                .map(|proc_body| proc_body.qname.clone())
                .unwrap_or(qname);

            let body_normalized;
            let body_parse: &str = if body.ends_with('\n') || body.is_empty() {
                body
            } else {
                body_normalized = format!("{body}\n");
                &body_normalized
            };
            let body_spans = tcld_syntax::command_spans(body_parse).unwrap_or_default();
            let mut ranges: Vec<(usize, usize)> = body_spans
                .iter()
                .map(|span| {
                    (
                        span.start_byte,
                        span.end_byte.saturating_sub(span.start_byte),
                    )
                })
                .collect();
            ranges.sort_unstable();
            ranges.dedup();
            let commands = ranges
                .into_iter()
                .filter(|(off, _)| *off < body.len().max(1))
                .map(|(off, _len)| CommandPoint {
                    offset: off as u64,
                    file_line: line_of(body_start_byte + off),
                })
                .collect();

            procs.push(ProcSpan {
                qname: resolved_qname,
                body_start_byte,
                body_end_byte: body_start_byte + body.len(),
                body_first_line: line_of(body_start_byte),
                body_last_line: line_of(body_start_byte + body.len()),
                commands,
            });
        }

        Self {
            script_path: script_path.to_string(),
            procs,
            command_ranges,
            first_statement_line,
        }
    }

    pub(crate) fn first_statement_line(&self) -> u32 {
        self.first_statement_line
    }

    pub(crate) fn script_path(&self) -> &str {
        &self.script_path
    }

    /// Translate a GUI `file:line` breakpoint into the wire's
    /// `(proc qname, body byte offset)` coordinates, or a defined
    /// "not supported by external backend" error.
    fn resolve_breakpoint(&self, file: &str, line: u32) -> Result<(String, u64), String> {
        if !super::files_likely_match(file, &self.script_path) {
            return Err(format!(
                "not supported by external backend: breakpoint in {file} — only breakpoints \
                 in the launched script ({}) can be set over the wire protocol",
                self.script_path
            ));
        }
        let containing = self
            .procs
            .iter()
            .filter(|p| p.body_first_line <= line && line <= p.body_last_line)
            .max_by_key(|p| p.body_start_byte);
        let Some(proc_span) = containing else {
            return Err(format!(
                "not supported by external backend: breakpoint at {file}:{line} is outside a \
                 proc body — the wire protocol addresses breakpoints by proc-body offsets. \
                 Move the breakpoint onto a statement inside a proc."
            ));
        };
        let point = proc_span
            .commands
            .iter()
            .filter(|c| c.file_line >= line)
            .min_by_key(|c| (c.file_line, c.offset));
        match point {
            Some(c) => Ok((proc_span.qname.clone(), c.offset)),
            None => Err(format!(
                "no statement at or after line {line} inside proc {} — cannot place a \
                 breakpoint there via the external backend",
                proc_span.qname
            )),
        }
    }

    fn resolve_command_breakpoint(
        &self,
        file: &str,
        offset: u64,
        len: u64,
    ) -> Result<BreakpointTarget, String> {
        if !super::files_likely_match(file, &self.script_path) {
            return Err(format!(
                "not supported by external backend: breakpoint in {file} — only breakpoints \
                 in the launched script ({}) can be set over the wire protocol",
                self.script_path
            ));
        }
        if !self.command_ranges.contains(&(offset, len)) {
            return Err(format!(
                "source range {offset}..{} is not an exact Tcl command",
                offset.saturating_add(len)
            ));
        }
        if let Some(proc_span) = self.procs.iter().find(|proc_span| {
            proc_span.body_start_byte as u64 <= offset && offset < proc_span.body_end_byte as u64
        }) {
            return Ok(BreakpointTarget::Source {
                qname: proc_span.qname.clone(),
                offset: offset.saturating_sub(proc_span.body_start_byte as u64),
                len,
            });
        }
        Ok(BreakpointTarget::File {
            path: self.script_path.clone(),
            offset,
            len,
        })
    }

    /// Map a wire pause location (`proc:<qname>` block + body byte
    /// offset) back to a file line. Falls back to the location's own
    /// line field (accurate for top-level source frames) when the block
    /// is unknown.
    fn line_for_pause(&self, block: &str, offset: u64, location_line: u32) -> u32 {
        let Some(qname) = block.strip_prefix("proc:") else {
            return location_line;
        };
        let qname = qname.trim_start_matches("::");
        let Some(proc_span) = self.procs.iter().find(|p| p.qname == qname) else {
            return location_line;
        };
        proc_span
            .commands
            .iter()
            .filter(|c| c.offset <= offset)
            .max_by_key(|c| c.offset)
            .map(|c| c.file_line)
            .unwrap_or(proc_span.body_first_line)
    }
}

fn line_starts_for(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in text.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn byte_to_line(pos: usize, line_starts: &[usize], text_len: usize) -> u32 {
    let capped = pos.min(text_len);
    let idx = line_starts.partition_point(|start| *start <= capped);
    idx.max(1) as u32
}

/// Split a Tcl command's text into words, returning byte ranges. Brace
/// and quote aware (enough for the canonical `proc name args {body}`
/// form; escapes inside words don't move word boundaries we care about).
fn scan_tcl_words(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        match bytes[i] {
            b'{' => {
                let mut depth = 0i32;
                while i < bytes.len() {
                    match bytes[i] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        b'\\' => i += 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'"' => {
                            i += 1;
                            break;
                        }
                        b'\\' => i += 2,
                        _ => i += 1,
                    }
                }
            }
            _ => {
                while i < bytes.len() && !(bytes[i] as char).is_whitespace() {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
        }
        words.push((start, i.min(text.len())));
    }
    words
}

// ─── Backend handle ──────────────────────────────────────────────────

/// Session phase shared between `send()` (actor thread) and the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WirePhase {
    /// Listening / handshaking / waiting for the gate pause.
    Starting,
    /// Paused at the launch gate (before any user code has run).
    GateHold,
    /// Past the gate: running or paused under nub control.
    Active,
}

enum WorkerMsg {
    Agent(AgentCommand),
    AddBreakpoint {
        file: String,
        line: u32,
        source_offset: Option<u64>,
        target: BreakpointTarget,
        condition: Option<String>,
    },
}

pub struct WireLaunchConfig {
    pub script_path: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    pub paths: NubPaths,
}

pub struct WireBackend {
    cmd_tx: std_mpsc::Sender<WorkerMsg>,
    event_rx: tokio_mpsc::Receiver<AgentEvent>,
    thread: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    phase: Arc<Mutex<WirePhase>>,
    child: Arc<Mutex<NubProcGuard>>,
    source_map: Arc<SourceMap>,
}

impl WireBackend {
    pub fn launch(config: WireLaunchConfig) -> Result<Self, String> {
        let source = std::fs::read_to_string(&config.script_path)
            .map_err(|e| format!("cannot read script {}: {e}", config.script_path))?;
        let source_map = Arc::new(SourceMap::build(&config.script_path, &source));

        let launcher = TempFileGuard::create(
            "tcld-wire-launcher",
            "tcl",
            &launcher_script(&config.script_path, &config.args),
        )?;

        let (event_tx, event_rx) = tokio_mpsc::channel(256);
        let (cmd_tx, cmd_rx) = std_mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(Mutex::new(WirePhase::Starting));
        let child = Arc::new(Mutex::new(NubProcGuard { child: None }));

        let worker = SessionWorker {
            config,
            launcher,
            source_map: source_map.clone(),
            event_tx,
            cmd_rx,
            stop_flag: stop_flag.clone(),
            phase: phase.clone(),
            child: child.clone(),
        };
        let thread = thread::Builder::new()
            .name("tcld-wire-controller".into())
            .spawn(move || worker.run())
            .map_err(|e| format!("failed to spawn wire controller thread: {e}"))?;

        Ok(Self {
            cmd_tx,
            event_rx,
            thread: Some(thread),
            stop_flag,
            phase,
            child,
            source_map,
        })
    }

    pub fn send(&self, cmd: &AgentCommand) -> Result<(), String> {
        let phase = *self.phase.lock().unwrap();
        match cmd {
            AgentCommand::StepInto
            | AgentCommand::StepOver
            | AgentCommand::StepOut
            | AgentCommand::StepInstruction
            | AgentCommand::StepOverInstruction
            | AgentCommand::StepOutInstruction
            | AgentCommand::RunToPc { .. }
                if phase == WirePhase::GateHold =>
            {
                return Err(STEP_AT_GATE_ERROR.to_string());
            }
            AgentCommand::SetBreakpoint {
                file,
                line,
                offset,
                len,
                condition,
            } => {
                // Resolve synchronously so unsupported placements are a
                // defined error at the call site, not a silent no-op.
                let target = match (*offset, *len) {
                    (Some(offset), Some(len)) => self
                        .source_map
                        .resolve_command_breakpoint(file, offset, len)?,
                    (None, None) => {
                        let (qname, offset) = self.source_map.resolve_breakpoint(file, *line)?;
                        BreakpointTarget::Source {
                            qname,
                            offset,
                            len: 1,
                        }
                    }
                    _ => return Err("command breakpoint requires both offset and len".to_string()),
                };
                return self
                    .cmd_tx
                    .send(WorkerMsg::AddBreakpoint {
                        file: file.clone(),
                        line: *line,
                        source_offset: *offset,
                        target,
                        condition: condition.clone(),
                    })
                    .map_err(|_| "wire controller thread exited".to_string());
            }
            _ => {}
        }
        self.cmd_tx
            .send(WorkerMsg::Agent(cmd.clone()))
            .map_err(|_| "wire controller thread exited".to_string())
    }

    pub fn take_event_rx(&mut self) -> tokio_mpsc::Receiver<AgentEvent> {
        let (_, empty) = tokio_mpsc::channel(1);
        std::mem::replace(&mut self.event_rx, empty)
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        // Killing the nub closes the socket, which unblocks the worker
        // from any in-flight blocking wire op.
        self.child.lock().unwrap().kill_now();
        if let Some(handle) = self.thread.take() {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if handle.is_finished() {
                    let _ = handle.join();
                    break;
                }
                if Instant::now() >= deadline {
                    eprintln!("[wire-debugger] stop timeout; detaching controller thread");
                    drop(handle);
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

impl Drop for WireBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

fn launcher_script(script_path: &str, args: &[String]) -> String {
    let argv = if args.is_empty() {
        "[list]".to_string()
    } else {
        format!(
            "[list {}]",
            args.iter()
                .map(|a| tcl_quote_for_list(a))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    format!(
        "# Generated by tcld-tauri's external debugger backend.\n\
         set ::argv {argv}\n\
         set ::argc {argc}\n\
         set ::argv0 {q_script}\n\
         ::tcld::dbg::request_pause\n\
         source {q_script}\n\
         exit 0\n",
        argc = args.len(),
        q_script = tcl_quote_for_list(script_path),
    )
}

// ─── Session worker ──────────────────────────────────────────────────

struct SessionWorker {
    config: WireLaunchConfig,
    launcher: TempFileGuard,
    source_map: Arc<SourceMap>,
    event_tx: tokio_mpsc::Sender<AgentEvent>,
    cmd_rx: std_mpsc::Receiver<WorkerMsg>,
    stop_flag: Arc<AtomicBool>,
    phase: Arc<Mutex<WirePhase>>,
    child: Arc<Mutex<NubProcGuard>>,
}

struct WireBp {
    id: String,
}

struct SessionState {
    ctrl: Controller,
    bp_map: HashMap<(String, u32, Option<u64>), WireBp>,
    pause_requested: bool,
    watch_support_installed: bool,
    /// Control mode of the most recent resume op, reported on the nub's
    /// `resumed` wire event (which is the single source of `Resumed`
    /// agent events — the worker does not synthesize its own).
    last_resume_mode: &'static str,
}

impl SessionWorker {
    fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.blocking_send(event);
    }

    fn emit_stderr(&self, text: String) {
        self.emit(AgentEvent::Output {
            stream: "stderr".into(),
            text,
        });
    }

    fn set_phase(&self, phase: WirePhase) {
        *self.phase.lock().unwrap() = phase;
    }

    fn run(self) {
        match self.establish_session() {
            Ok(state) => self.event_loop(state),
            Err(err) => {
                if !self.stop_flag.load(Ordering::SeqCst) {
                    self.emit(AgentEvent::Exited {
                        code: 1,
                        reason: Some(err),
                    });
                }
                self.child.lock().unwrap().kill_now();
            }
        }
    }

    /// Listen → spawn nub → handshake → wait for the launch-gate pause →
    /// emit `Ready` + the synthetic entry `Stopped`.
    fn establish_session(&self) -> Result<SessionState, String> {
        let (mut ctrl, port) = Controller::listen(
            "127.0.0.1:0"
                .parse()
                .expect("loopback listen address parses"),
        )
        .map_err(|e| format!("wire listen failed: {e}"))?;

        let mut cmd = Command::new(&self.config.paths.tclsh);
        cmd.arg(&self.config.paths.bootstrap)
            .env("TCLD_DBG_DYLIB", &self.config.paths.dylib)
            .env("TCLD_NUB_HOST", "127.0.0.1")
            .env("TCLD_NUB_PORT", port.to_string())
            .envs(self.config.env.iter())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = self.config.cwd.as_deref() {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "failed to spawn tclsh {}: {e}. Set the run configuration's tclsh path or \
                 TCLD_TCLSH.",
                self.config.paths.tclsh.display()
            )
        })?;

        // Forward the nub's stdio as Output events (embedded parity:
        // script `puts` output reaches the GUI console).
        if let Some(stdout) = child.stdout.take() {
            spawn_output_reader(stdout, "stdout", self.event_tx.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_output_reader(stderr, "stderr", self.event_tx.clone());
        }
        let pid = child.id();
        self.child.lock().unwrap().child = Some(child);

        let capabilities = ctrl
            .accept_with_timeout(HANDSHAKE_WAIT)
            .map_err(|e| format!("nub handshake failed: {e}"))?;
        validate_nub_tcl_abi(&capabilities)?;

        let argv = if self.config.args.is_empty() {
            "[list]".to_string()
        } else {
            format!(
                "[list {}]",
                self.config
                    .args
                    .iter()
                    .map(|arg| tcl_quote_for_list(arg))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        let prelude = format!(
            "set ::argv {argv}; set ::argc {}; set ::argv0 {}",
            self.config.args.len(),
            tcl_quote_for_list(&self.config.script_path)
        );
        ctrl.send_with_timeout(
            Op::EvalInFrame {
                level: 0,
                script: prelude,
            },
            OP_TIMEOUT,
        )
        .map_err(|error| format!("failed to stage target argv: {error}"))?;
        ctrl.send_with_timeout(
            Op::StartTarget {
                path: self.config.script_path.clone(),
                stop_on_entry: true,
            },
            OP_TIMEOUT,
        )
        .map_err(|error| format!("failed to start debug target: {error}"))?;

        // Wait for the exact pre-command entry pause.
        let gate_frame_count = self.wait_for_gate(&mut ctrl)?;

        let tcl_version = match ctrl.send_with_timeout(
            Op::EvalInFrame {
                level: 0,
                script: "info patchlevel".into(),
            },
            OP_TIMEOUT,
        ) {
            Ok(resp) => resp.into_payload(),
            Err(_) => String::new(),
        };

        let mut state = SessionState {
            ctrl,
            bp_map: HashMap::new(),
            pause_requested: false,
            watch_support_installed: false,
            last_resume_mode: "continue",
        };
        self.install_watchpoint_support(&mut state);

        self.emit(AgentEvent::Ready {
            tcl_version,
            pid,
            debug_extension_loaded: Some(true),
            debug_extension_installed: Some(true),
            debug_extension_capabilities: if capabilities.is_empty() {
                None
            } else {
                Some(capabilities.join(" "))
            },
            debug_extension_reason: None,
        });

        // Synthetic entry stop: the nub is really paused (at the gate),
        // presented as the embedded backend's stop-on-first-statement.
        self.set_phase(WirePhase::GateHold);
        self.emit(AgentEvent::Stopped {
            reason: "step".into(),
            file: self.source_map.script_path().to_string(),
            line: self.source_map.first_statement_line(),
            frame_count: gate_frame_count,
            pc: None,
            instruction_index: None,
            block_id: None,
            error: None,
            error_info: None,
        });

        Ok(state)
    }

    fn wait_for_gate(&self, ctrl: &mut Controller) -> Result<u32, String> {
        let deadline = Instant::now() + GATE_WAIT;
        while Instant::now() < deadline {
            if self.stop_flag.load(Ordering::SeqCst) {
                return Err("session stopped during launch".into());
            }
            match ctrl.next_event(Duration::from_millis(100)) {
                Ok(WireEvent::Paused { frame_count, .. }) => return Ok(frame_count),
                Ok(WireEvent::TargetLoadFailed {
                    message,
                    error_code,
                    error_info,
                    ..
                }) => {
                    return Err(format!(
                        "target load failed: {message}\nerror code: {error_code}\n{error_info}"
                    ));
                }
                // `error_paused` is advisory (outermost-catch reporting),
                // never an actual pause — ignore it like all other
                // pre-gate chatter.
                Ok(_) => continue,
                Err(ControllerError::ResponseTimeout) => continue,
                Err(e) => return Err(format!("nub disconnected during launch: {e}")),
            }
        }
        Err(format!(
            "nub never reached the launch pause within {GATE_WAIT:?}"
        ))
    }

    fn install_watchpoint_support(&self, state: &mut SessionState) {
        // The embedded backend's pure-Tcl watchpoint surface, plus a
        // wire-side chain that turns a hit into an engine pause so the
        // hit surfaces as a `paused` event.
        let chain = r#"
if {[llength [info procs ::tcld::dbg::_watch_callback_wire_base]] == 0} {
    rename ::tcld::dbg::_watch_callback ::tcld::dbg::_watch_callback_wire_base
    proc ::tcld::dbg::_watch_callback {watchId oldValue name1 name2 op} {
        ::tcld::dbg::_watch_callback_wire_base $watchId $oldValue $name1 $name2 $op
        if {$::__tcld_wp_hit} { ::tcld::dbg::request_pause }
    }
}
"#;
        let script = format!("{WATCHPOINT_SUPPORT_SCRIPT}\n{chain}");
        match state
            .ctrl
            .send_with_timeout(Op::EvalInFrame { level: 0, script }, OP_TIMEOUT)
        {
            Ok(_) => state.watch_support_installed = true,
            Err(e) => self.emit_stderr(format!(
                "external backend: failed to install watchpoint support: {e}"
            )),
        }
    }

    fn event_loop(self, mut state: SessionState) {
        loop {
            if self.stop_flag.load(Ordering::SeqCst) {
                break;
            }

            // Drain queued commands first so ops issued while paused are
            // dispatched before we block on the event poll.
            loop {
                match self.cmd_rx.try_recv() {
                    Ok(msg) => self.handle_msg(&mut state, msg),
                    Err(std_mpsc::TryRecvError::Empty) => break,
                    Err(std_mpsc::TryRecvError::Disconnected) => {
                        self.shutdown(state);
                        return;
                    }
                }
            }

            match state.ctrl.next_event(EVENT_POLL) {
                Ok(event) => self.handle_wire_event(&mut state, event),
                Err(ControllerError::ResponseTimeout) => {}
                Err(_) => {
                    // Socket closed: clean exit, crash, or `exit 0` from
                    // the launcher after the script completed.
                    self.finish_exited();
                    return;
                }
            }
        }
        self.shutdown(state);
    }

    fn shutdown(&self, mut state: SessionState) {
        let _ = state.ctrl.close();
        self.child.lock().unwrap().kill_now();
    }

    fn finish_exited(&self) {
        let code = {
            let mut guard = self.child.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(code) = guard.try_exit_code() {
                    break code;
                }
                if Instant::now() >= deadline {
                    guard.kill_now();
                    break -1;
                }
                thread::sleep(Duration::from_millis(20));
            }
        };
        if !self.stop_flag.load(Ordering::SeqCst) {
            self.emit(AgentEvent::Exited { code, reason: None });
        }
    }

    // ── Wire events → AgentEvents ────────────────────────────────────

    fn handle_wire_event(&self, state: &mut SessionState, event: WireEvent) {
        match event {
            WireEvent::TargetStarted { .. } => {
                self.set_phase(WirePhase::Active);
            }
            WireEvent::TargetCompleted { .. } => {
                self.emit(AgentEvent::Exited {
                    code: 0,
                    reason: None,
                });
                self.stop_flag.store(true, Ordering::SeqCst);
            }
            WireEvent::TargetLoadFailed {
                path,
                message,
                error_code,
                error_info,
                ..
            } => {
                self.emit_stderr(format!(
                    "target load failed ({path}): {message}\nerror code: {error_code}\n{error_info}"
                ));
                self.emit(AgentEvent::Exited {
                    code: 1,
                    reason: Some(message),
                });
                self.stop_flag.store(true, Ordering::SeqCst);
            }
            WireEvent::Paused {
                location,
                frame_count,
                breakpoint,
                reason,
                pc,
            } => {
                self.set_phase(WirePhase::Active);
                let (file, line) = self.map_location(&location);
                let watchpoint = self.consume_watchpoint_hit(state, &file, line, frame_count);
                let pause_requested = std::mem::take(&mut state.pause_requested);
                let reason = if reason == PauseReason::Step && watchpoint {
                    "watchpoint"
                } else if reason == PauseReason::Step && breakpoint.is_some() {
                    "breakpoint"
                } else if reason == PauseReason::Step && pause_requested {
                    "pause"
                } else {
                    reason.as_str()
                };
                state.pause_requested = false;
                self.emit(AgentEvent::Stopped {
                    reason: reason.into(),
                    file,
                    line,
                    frame_count,
                    pc: pc.and_then(|value| i32::try_from(value).ok()),
                    instruction_index: None,
                    block_id: None,
                    error: None,
                    error_info: None,
                });
            }
            WireEvent::Resumed => {
                self.set_phase(WirePhase::Active);
                self.emit(AgentEvent::Resumed {
                    control_mode: state.last_resume_mode.to_string(),
                });
            }
            WireEvent::ErrorPaused { .. } => {
                // Advisory only: the current nub fires `error_paused` for
                // every error observed by an *outermost* `catch` (see
                // `on_catch_error` in crates/tcld-dbg-ext/src/wrappers.rs)
                // and the engine keeps running — it is not a pause. The
                // embedded backend emits nothing for caught errors either;
                // genuinely uncaught script errors terminate the nub and
                // surface as captured stderr + a non-zero Exited event.
            }
            WireEvent::ConditionError {
                breakpoint,
                message,
            } => {
                self.emit_stderr(format!(
                    "breakpoint condition error ({breakpoint}): {message}"
                ));
            }
            WireEvent::BlocksRevalidated { .. } => {}
            WireEvent::NubStarted { .. } => {}
            WireEvent::NubStopped { reason } => {
                self.emit_stderr(format!("nub stopped: {reason}"));
            }
        }
    }

    fn map_location(&self, location: &SourceLocation) -> (String, u32) {
        let line = self.source_map.line_for_pause(
            &location.block_id,
            location.offset_start,
            location.line,
        );
        (
            location
                .source_path
                .clone()
                .unwrap_or_else(|| self.source_map.script_path().to_string()),
            line,
        )
    }

    /// After a pause, check whether it was caused by a chained
    /// watchpoint hit; if so emit the `WatchpointHit` event the actor
    /// expects immediately before the `Stopped`.
    fn consume_watchpoint_hit(
        &self,
        state: &mut SessionState,
        file: &str,
        line: u32,
        frame_count: u32,
    ) -> bool {
        if !state.watch_support_installed {
            return false;
        }
        let script = "if {[info exists ::__tcld_wp_hit] && $::__tcld_wp_hit} { \
                          set ::__tcld_wp_hit 0; list 1 [array get ::__tcld_wp_info] \
                      } else { list 0 {} }";
        let Ok(resp) = state.ctrl.send_with_timeout(
            Op::EvalInFrame {
                level: 0,
                script: script.into(),
            },
            OP_TIMEOUT,
        ) else {
            return false;
        };
        let payload = resp.into_payload();
        let parts = split_tcl_list(&payload);
        if parts.first().map(String::as_str) != Some("1") {
            return false;
        }
        let info = parts.get(1).cloned().unwrap_or_default();
        let pairs = split_tcl_list(&info);
        let get = |key: &str| -> String {
            let mut i = 0;
            while i + 1 < pairs.len() {
                if pairs[i] == key {
                    return pairs[i + 1].clone();
                }
                i += 2;
            }
            String::new()
        };
        let element = get("element");
        self.emit(AgentEvent::WatchpointHit {
            watch_id: get("watchId").parse().unwrap_or(0),
            name: get("name"),
            element: if element.is_empty() {
                None
            } else {
                Some(element)
            },
            operation: {
                let op = get("operation");
                if op.is_empty() { "write".into() } else { op }
            },
            old_value: get("oldValue"),
            new_value: get("newValue"),
            file: file.to_string(),
            line,
            frame_count,
        });
        true
    }

    // ── AgentCommands → wire ops ─────────────────────────────────────

    fn handle_msg(&self, state: &mut SessionState, msg: WorkerMsg) {
        match msg {
            WorkerMsg::AddBreakpoint {
                file,
                line,
                source_offset,
                target,
                condition,
            } => self.add_breakpoint(state, file, line, source_offset, target, condition),
            WorkerMsg::Agent(cmd) => self.handle_agent_cmd(state, cmd),
        }
    }

    fn add_breakpoint(
        &self,
        state: &mut SessionState,
        file: String,
        line: u32,
        source_offset: Option<u64>,
        target: BreakpointTarget,
        condition: Option<String>,
    ) {
        // Upsert semantics matching the actor's breakpoint list.
        let key = (file.clone(), line, source_offset);
        if let Some(existing) = state.bp_map.remove(&key) {
            let _ = state
                .ctrl
                .send_with_timeout(Op::BpRemove { id: existing.id }, OP_TIMEOUT);
        }
        match state
            .ctrl
            .send_with_timeout(Op::BpAdd { target, condition }, OP_TIMEOUT)
        {
            Ok(resp) => {
                let id = resp.into_payload().trim().to_string();
                state.bp_map.insert(key, WireBp { id });
            }
            Err(e) => {
                self.emit_stderr(format!("failed to set breakpoint {file}:{line}: {e}"));
            }
        }
    }

    fn handle_agent_cmd(&self, state: &mut SessionState, cmd: AgentCommand) {
        match cmd {
            AgentCommand::StepInto => self.resume(state, Op::StepInto, "step_into"),
            AgentCommand::StepOver => self.resume(state, Op::StepOver, "step_over"),
            AgentCommand::StepOut => self.resume(state, Op::StepOut, "step_out"),
            AgentCommand::Continue => self.resume(state, Op::Continue, "continue"),
            AgentCommand::Pause => {
                // The interrupt response is often swallowed by the pause
                // it causes (the engine pauses inside the nub's response
                // path) — a timeout here is not an error; the `paused`
                // event is the real signal.
                match state
                    .ctrl
                    .send_with_timeout(Op::Interrupt, INTERRUPT_TIMEOUT)
                {
                    Ok(_) | Err(ControllerError::ResponseTimeout) => {
                        state.pause_requested = true;
                    }
                    Err(e) => self.emit_stderr(format!("pause failed: {e}")),
                }
            }
            AgentCommand::ClearBreakpoint { file, line, offset } => {
                let file = super::canonicalize_debug_path(file);
                if let Some(bp) = state.bp_map.remove(&(file, line, offset)) {
                    let _ = state
                        .ctrl
                        .send_with_timeout(Op::BpRemove { id: bp.id }, OP_TIMEOUT);
                }
            }
            AgentCommand::SetBreakpointEnabled {
                file,
                line,
                offset,
                enabled,
            } => {
                let file = super::canonicalize_debug_path(file);
                if let Some(bp) = state.bp_map.get(&(file.clone(), line, offset)) {
                    let op = if enabled {
                        Op::BpEnable { id: bp.id.clone() }
                    } else {
                        Op::BpDisable { id: bp.id.clone() }
                    };
                    if let Err(e) = state.ctrl.send_with_timeout(op, OP_TIMEOUT) {
                        self.emit_stderr(format!("failed to toggle breakpoint {file}:{line}: {e}"));
                    }
                }
            }
            AgentCommand::ClearAllBreakpoints => {
                for (_, bp) in state.bp_map.drain() {
                    let _ = state
                        .ctrl
                        .send_with_timeout(Op::BpRemove { id: bp.id }, OP_TIMEOUT);
                }
            }
            AgentCommand::GetVariables { request_id, level } => {
                let vars = self.fetch_variables(state, level);
                self.emit(AgentEvent::Variables {
                    request_id,
                    level,
                    vars,
                });
            }
            AgentCommand::GetStack { request_id } => {
                let frames = self.fetch_stack(state);
                self.emit(AgentEvent::Stack { request_id, frames });
            }
            AgentCommand::Eval {
                request_id,
                script,
                level,
            } => {
                let (result, error, error_info, error_code) = self.eval(state, &script, level);
                self.emit(AgentEvent::EvalResult {
                    request_id,
                    result,
                    error,
                    error_info,
                    error_code,
                });
            }
            AgentCommand::SetVariable { name, value, level } => {
                // Base64 transport for the same reason as eval: the
                // engine's `set_var` op corrupts brace/backslash values.
                let script = format!(
                    "set [encoding convertfrom utf-8 [binary decode base64 {{{}}}]] \
                     [encoding convertfrom utf-8 [binary decode base64 {{{}}}]]",
                    base64_encode(name.as_bytes()),
                    base64_encode(value.as_bytes())
                );
                if let Err(e) = state.ctrl.send_with_timeout(
                    Op::EvalInFrame {
                        level: level as i64,
                        script,
                    },
                    OP_TIMEOUT,
                ) {
                    self.emit_stderr(format!("set variable failed: {e}"));
                }
            }
            // Trace decisions are for TracePoint-driven backends; the nub
            // owns run control over the wire.
            AgentCommand::TraceDecision { .. } => {}
            // The actor drives watchpoints through Eval scripts; these
            // protocol variants are unused (same as the embedded backend).
            AgentCommand::SetWatchpoint { .. }
            | AgentCommand::ClearWatchpoint { .. }
            | AgentCommand::ClearAllWatchpoints
            | AgentCommand::ListWatchpoints { .. } => {}
            AgentCommand::StepInstruction => {
                self.resume(state, Op::StepInstruction, "step_instruction");
            }
            AgentCommand::StepOverInstruction => {
                self.resume(state, Op::StepOverInstruction, "step_over_instruction");
            }
            AgentCommand::StepOutInstruction => {
                self.resume(state, Op::StepOutInstruction, "step_out_instruction");
            }
            AgentCommand::RunToPc { pc } => {
                self.resume(state, Op::RunToPc { pc: pc as u64 }, "run_to_pc");
            }
            // SetBreakpoint arrives pre-resolved as WorkerMsg::AddBreakpoint.
            AgentCommand::SetBreakpoint { .. } => {}
        }
    }

    fn resume(&self, state: &mut SessionState, op: Op, mode: &'static str) {
        match state.ctrl.send_with_timeout(op, OP_TIMEOUT) {
            Ok(_) => {
                self.set_phase(WirePhase::Active);
                // The nub's `resumed` wire event produces the Resumed
                // agent event; remember the mode it should report.
                state.last_resume_mode = mode;
            }
            Err(e) => self.emit_stderr(format!("{mode} failed: {e}")),
        }
    }

    fn fetch_variables(&self, state: &mut SessionState, level: u32) -> Vec<serde_json::Value> {
        // Runs IN the target frame (eval_in_frame does the `uplevel #N`
        // hop from the nub's dispatch context, where absolute levels are
        // valid). Inner `uplevel #N` calls would go through the nub's
        // ::uplevel shim from the wrong frame and resolve incorrectly —
        // verified against a real nub — so the script is uplevel-free
        // and the scratch variables it leaves behind are filtered by
        // their __tcld_ prefix.
        const IN_FRAME_VARS_SCRIPT: &str = r#"
set __tcld_vars [list]
foreach __tcld_v [info vars] {
    if {[string match __tcld_* $__tcld_v]} { continue }
    if {[array exists $__tcld_v]} {
        lappend __tcld_vars [list $__tcld_v array [array get $__tcld_v]]
    } else {
        set __tcld_val {}
        catch {set __tcld_val [set $__tcld_v]}
        lappend __tcld_vars [list $__tcld_v scalar $__tcld_val]
    }
}
set __tcld_vars
"#;
        match state.ctrl.send_with_timeout(
            Op::EvalInFrame {
                level: level as i64,
                script: IN_FRAME_VARS_SCRIPT.into(),
            },
            OP_TIMEOUT,
        ) {
            Ok(resp) => parse_variable_list(&resp.into_payload()),
            Err(_) => Vec::new(),
        }
    }

    fn fetch_stack(&self, state: &mut SessionState) -> Vec<serde_json::Value> {
        const STACK_SCRIPT: &str = r#"
set __tcld_frames [list]
for {set __tcld_i 1} {$__tcld_i <= [info frame]} {incr __tcld_i} {
    set __tcld_d [info frame $__tcld_i]
    set __tcld_row [list idx $__tcld_i]
    foreach __tcld_k {type proc line file cmd} {
        if {[dict exists $__tcld_d $__tcld_k]} {
            lappend __tcld_row $__tcld_k [dict get $__tcld_d $__tcld_k]
        }
    }
    lappend __tcld_frames $__tcld_row
}
set __tcld_frames
"#;
        let Ok(resp) = state.ctrl.send_with_timeout(
            Op::EvalInFrame {
                level: 0,
                script: STACK_SCRIPT.into(),
            },
            OP_TIMEOUT,
        ) else {
            return Vec::new();
        };
        let payload = resp.into_payload();
        let mut frames = Vec::new();
        for row in split_tcl_list(&payload) {
            let tokens = split_tcl_list(&row);
            let mut level = 0u32;
            let mut frame_type = String::from("source");
            let mut proc_name: Option<String> = None;
            let mut file: Option<String> = None;
            let mut line: Option<u32> = None;
            let mut cmd: Option<String> = None;
            let mut i = 0;
            while i + 1 < tokens.len() {
                match tokens[i].as_str() {
                    "idx" => level = tokens[i + 1].parse().unwrap_or(0),
                    "type" => frame_type = tokens[i + 1].clone(),
                    "proc" => proc_name = Some(tokens[i + 1].clone()),
                    "file" => file = Some(tokens[i + 1].clone()),
                    "line" => line = tokens[i + 1].parse().ok(),
                    "cmd" => cmd = Some(tokens[i + 1].clone()),
                    _ => {}
                }
                i += 2;
            }
            if self.is_internal_frame(proc_name.as_deref(), file.as_deref()) {
                continue;
            }
            let mut frame = serde_json::json!({
                "level": level,
                "type": frame_type,
            });
            if let Some(p) = proc_name {
                frame["procName"] = serde_json::json!(p);
            }
            if let Some(f) = file {
                frame["file"] = serde_json::json!(f);
            }
            if let Some(l) = line {
                frame["line"] = serde_json::json!(l);
            }
            if let Some(c) = cmd {
                frame["cmd"] = serde_json::json!(c);
            }
            frames.push(frame);
        }
        frames
    }

    /// Nub machinery frames the GUI stack should not show: the bootstrap
    /// and launcher files, the nub's own namespace, and the command-shim
    /// procs (when they carry no user file).
    fn is_internal_frame(&self, proc_name: Option<&str>, file: Option<&str>) -> bool {
        const SHIM_PROCS: [&str; 10] = [
            "::source",
            "::uplevel",
            "::catch",
            "::eval",
            "::apply",
            "::namespace",
            "::vwait",
            "::update",
            "::rename",
            "::interp",
        ];
        if let Some(f) = file {
            if f == self.launcher.path.to_string_lossy()
                || f == self.config.paths.bootstrap.to_string_lossy()
            {
                return true;
            }
            // A user file wins even under a shim proc (e.g. the sourced
            // script's top-level frame runs inside the ::source shim).
            return false;
        }
        match proc_name {
            Some(p) => p.starts_with("::tcld::dbg") || SHIM_PROCS.contains(&p),
            None => false,
        }
    }

    fn eval(
        &self,
        state: &mut SessionState,
        script: &str,
        level: u32,
    ) -> (String, bool, Option<String>, Option<String>) {
        // Stash the user script in a global (shipped as base64 — the
        // engine's `set_var` op corrupts values containing braces or
        // backslashes via its eval_helper escaping, and eval_in_frame
        // brace-wraps raw scripts, so base64 is the only lossless
        // transport for arbitrary text). Then evaluate it IN the target
        // frame with a fixed, brace-balanced wrapper (`catch $script`);
        // evaluating in-frame avoids the nub's ::uplevel shim, which
        // mis-resolves nested absolute levels. `expr` is deliberately
        // not used on the error strings (it re-parses and mangles them).
        let stash = format!(
            "set ::__tcld_wire_eval_script [encoding convertfrom utf-8 \
             [binary decode base64 {{{}}}]]; list stashed",
            base64_encode(script.as_bytes())
        );
        if let Err(e) = state.ctrl.send_with_timeout(
            Op::EvalInFrame {
                level: 0,
                script: stash,
            },
            OP_TIMEOUT,
        ) {
            return (format!("eval failed: {e}"), true, None, wire_error_code(&e));
        }
        const EVAL_WRAPPER: &str = r#"
if {[catch $::__tcld_wire_eval_script __tcld_eval_result]} {
    set __tcld_ei {}
    if {[info exists ::errorInfo]} { set __tcld_ei $::errorInfo }
    set __tcld_ec {}
    if {[info exists ::errorCode]} { set __tcld_ec $::errorCode }
    set __tcld_eval_out [list err $__tcld_eval_result $__tcld_ei $__tcld_ec]
} else {
    set __tcld_eval_out [list ok $__tcld_eval_result]
}
set __tcld_eval_out
"#;
        match state.ctrl.send_with_timeout(
            Op::EvalInFrame {
                level: level as i64,
                script: EVAL_WRAPPER.into(),
            },
            OP_TIMEOUT,
        ) {
            Ok(resp) => {
                let payload = resp.into_payload();
                let parts = split_tcl_list(&payload);
                match parts.first().map(String::as_str) {
                    Some("ok") => (parts.get(1).cloned().unwrap_or_default(), false, None, None),
                    Some("err") => {
                        let non_empty =
                            |s: &String| if s.is_empty() { None } else { Some(s.clone()) };
                        (
                            parts.get(1).cloned().unwrap_or_default(),
                            true,
                            parts.get(2).and_then(non_empty),
                            parts.get(3).and_then(non_empty),
                        )
                    }
                    _ => (
                        format!("malformed eval response: {payload}"),
                        true,
                        None,
                        None,
                    ),
                }
            }
            Err(e) => (format!("eval failed: {e}"), true, None, wire_error_code(&e)),
        }
    }
}

/// Minimal standard-alphabet base64 (with padding). Used to ship
/// arbitrary script/value text into the nub losslessly — the wire's
/// string-bearing ops corrupt brace/backslash content (see `eval`).
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

fn wire_error_code(err: &ControllerError) -> Option<String> {
    match err {
        ControllerError::Wire(w) => Some(w.code.as_str().to_string()),
        _ => None,
    }
}

fn spawn_output_reader(
    stream: impl std::io::Read + Send + 'static,
    name: &'static str,
    event_tx: tokio_mpsc::Sender<AgentEvent>,
) {
    thread::Builder::new()
        .name(format!("tcld-wire-{name}"))
        .spawn(move || {
            let reader = std::io::BufReader::new(stream);
            for line in reader.lines() {
                let Ok(text) = line else { break };
                if event_tx
                    .blocking_send(AgentEvent::Output {
                        stream: name.to_string(),
                        text,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .expect("spawn wire output reader thread");
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "# heading comment\n\
                          set top 1\n\
                          proc demo {} {\n    \
                              set a 1\n    \
                              set b 2\n    \
                              set c 3\n\
                          }\n\
                          demo\n";

    fn map() -> SourceMap {
        SourceMap::build("/tmp/wire-map-test.tcl", SCRIPT)
    }

    #[test]
    fn source_map_resolves_proc_body_lines_to_offsets() {
        let map = map();
        // Body is "\n    set a 1\n    set b 2\n    set c 3\n":
        // set a 1 @ 5 (line 4), set b 2 @ 17 (line 5), set c 3 @ 29 (line 6).
        assert_eq!(
            map.resolve_breakpoint("/tmp/wire-map-test.tcl", 4).unwrap(),
            ("demo".to_string(), 5)
        );
        assert_eq!(
            map.resolve_breakpoint("/tmp/wire-map-test.tcl", 5).unwrap(),
            ("demo".to_string(), 17)
        );
        assert_eq!(
            map.resolve_breakpoint("/tmp/wire-map-test.tcl", 6).unwrap(),
            ("demo".to_string(), 29)
        );
    }

    #[test]
    fn source_map_maps_pause_offsets_back_to_file_lines() {
        let map = map();
        assert_eq!(map.line_for_pause("proc:demo", 5, 1), 4);
        assert_eq!(map.line_for_pause("proc:demo", 17, 1), 5);
        assert_eq!(map.line_for_pause("proc:demo", 29, 1), 6);
        // Unknown blocks fall back to the location's own line.
        assert_eq!(map.line_for_pause("proc:<unknown>", 0, 8), 8);
    }

    #[test]
    fn source_map_resolves_distinct_nested_commands_on_one_line() {
        let script = "proc demo {} {\n    set x [expr {1 + 1}]\n}\nputs done\n";
        let map = SourceMap::build("/tmp/nested.tcl", script);
        let spans = tcld_syntax::command_spans(script).unwrap();
        let set = spans
            .iter()
            .find(|span| span.name.as_deref() == Some("set"))
            .unwrap();
        let expr = spans
            .iter()
            .find(|span| span.name.as_deref() == Some("expr"))
            .unwrap();

        let set_target = map
            .resolve_command_breakpoint(
                "/tmp/nested.tcl",
                set.start_byte as u64,
                (set.end_byte - set.start_byte) as u64,
            )
            .unwrap();
        let expr_target = map
            .resolve_command_breakpoint(
                "/tmp/nested.tcl",
                expr.start_byte as u64,
                (expr.end_byte - expr.start_byte) as u64,
            )
            .unwrap();
        assert!(matches!(
            (set_target, expr_target),
            (
                BreakpointTarget::Source {
                    qname: set_qname,
                    offset: set_offset,
                    ..
                },
                BreakpointTarget::Source {
                    qname: expr_qname,
                    offset: expr_offset,
                    ..
                }
            ) if set_qname == "demo" && expr_qname == "demo" && set_offset != expr_offset
        ));
    }

    #[test]
    fn source_map_rejects_top_level_and_foreign_file_breakpoints() {
        let map = map();
        let err = map
            .resolve_breakpoint("/tmp/wire-map-test.tcl", 2)
            .expect_err("top-level line is a defined error");
        assert!(err.contains("not supported by external backend"));
        assert!(err.contains("outside a proc body"));

        let err = map
            .resolve_breakpoint("/some/other/file.tcl", 4)
            .expect_err("foreign file is a defined error");
        assert!(err.contains("not supported by external backend"));
    }

    #[test]
    fn source_map_reports_first_statement_line_for_entry_stop() {
        assert_eq!(map().first_statement_line(), 2);
    }

    #[test]
    fn launcher_script_arms_gate_before_sourcing() {
        let script = launcher_script("/tmp/user.tcl", &["one".into(), "two words".into()]);
        let pause = script.find("::tcld::dbg::request_pause").expect("gate");
        let source = script.find("source ").expect("source");
        assert!(pause < source, "gate must precede source: {script}");
        assert!(script.contains("set ::argc 2"));
        assert!(script.contains("two words"));
        assert!(script.trim_end().ends_with("exit 0"));
    }

    #[test]
    fn base64_encoder_matches_reference_vectors() {
        // RFC 4648 vectors plus brace-heavy Tcl content.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"expr {2 + 3}"), "ZXhwciB7MiArIDN9");
    }

    #[test]
    fn missing_nub_reports_actionable_diagnostic() {
        // Wording contract mirrors apps/tcld/src/debug/config.rs: names
        // the build command, both override channels, and the fallback.
        let msg = nub_unavailable_message();
        assert!(msg.contains("cargo build -p tcld-dbg-ext"));
        assert!(msg.contains("TCLD_DBG_DYLIB"));
        assert!(msg.contains("TCLD_TCLSH"));
        assert!(msg.contains("embedded debugger backend"));
    }

    #[test]
    fn explicit_dylib_override_must_exist() {
        let err = locate_dylib(Some("/nonexistent/libtcld_dbg.dylib"), TclAbi::Tcl9).unwrap_err();
        assert!(err.contains("/nonexistent/libtcld_dbg.dylib"));
    }

    #[test]
    fn older_wire_abis_never_auto_select_generic_tcl9_artifacts() {
        let workspace = PathBuf::from("/checkout");
        let executable = PathBuf::from("/archive/tcld");
        let file = super::super::dbg_extension_library_filename();

        for abi in [TclAbi::Tcl85, TclAbi::Tcl86] {
            let candidates = automatic_dylib_candidates(abi, &workspace, Some(&executable));
            assert!(
                !candidates
                    .iter()
                    .any(|path| path == &workspace.join("target/debug").join(file)),
                "{abi:?} admitted the generic Tcl 9 debugger nub: {candidates:?}"
            );
            assert!(
                !candidates
                    .iter()
                    .any(|path| path == &PathBuf::from("/archive").join(file)),
                "{abi:?} admitted an ABI-unknown adjacent debugger nub: {candidates:?}"
            );
        }

        let tcl9 = automatic_dylib_candidates(TclAbi::Tcl9, &workspace, Some(&executable));
        assert!(tcl9.contains(&workspace.join("target/debug").join(file)));
    }

    #[test]
    fn explicit_tclsh_override_must_exist() {
        let err = locate_tclsh(Some("/nonexistent/tclsh9.0")).unwrap_err();
        assert!(err.contains("/nonexistent/tclsh9.0"));
    }

    #[cfg(unix)]
    #[test]
    fn automatic_shell_selection_probes_bare_shell_before_older_fallback() {
        use std::os::unix::fs::PermissionsExt;

        let path = test_directory("shell-path");
        for (name, version) in [("tclsh", "9.0.3"), ("tclsh8.5", "8.5.9")] {
            let shell = path.join(name);
            std::fs::write(&shell, format!("#!/bin/sh\nprintf {version}\n")).unwrap();
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let selected = locate_tclsh_in(Some(path.as_os_str()), &[]).unwrap();
        assert_eq!(selected, path.join("tclsh"));
        let _ = std::fs::remove_dir_all(path);
    }

    #[cfg(unix)]
    #[test]
    fn automatic_shell_selection_finds_versioned_install_prefix() {
        use std::os::unix::fs::PermissionsExt;

        let path = test_directory("fallback-path");
        let fallback = path.join("tclsh8.5");
        std::fs::write(&fallback, "#!/bin/sh\nprintf 8.5.9\n").unwrap();
        std::fs::set_permissions(&fallback, std::fs::Permissions::from_mode(0o755)).unwrap();

        let prefix = test_directory("versioned-prefix");
        let bin = prefix.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let preferred = bin.join("tclsh8.6");
        std::fs::write(&preferred, "#!/bin/sh\nprintf 8.6.17\n").unwrap();
        std::fs::set_permissions(&preferred, std::fs::Permissions::from_mode(0o755)).unwrap();

        let selected = locate_tclsh_in(
            Some(path.as_os_str()),
            &[(Some(TclAbi::Tcl86), prefix.clone())],
        )
        .unwrap();
        assert_eq!(selected, preferred);
        let _ = std::fs::remove_dir_all(path);
        let _ = std::fs::remove_dir_all(prefix);
    }

    fn test_directory(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tcld-tauri-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn embedded_bootstrap_matches_canonical_script() {
        let canonical = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/tcld-dbg-ext/scripts/tcld_nub_bootstrap.tcl");
        let on_disk = std::fs::read_to_string(canonical).expect("canonical bootstrap");
        assert_eq!(EMBEDDED_BOOTSTRAP, on_disk);
    }
}
