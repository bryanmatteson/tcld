//! Wire-debugger session for the `tcld debug` TUI.
//!
//! Owns a `tcld_debug_wire::Controller` plus the spawned tclsh nub,
//! and exposes the higher-level operations the TUI needs (state
//! refresh, breakpoint toggle, location lookup) on top of the raw
//! op vocabulary. Translates wire-protocol payloads into the neutral
//! types defined by `tcld_tui::debug` so the view layer stays
//! controller-agnostic.

use std::collections::BTreeMap;
use std::path::Path;

use tcld_debug_wire::{
    BreakpointTarget, Controller, ControllerError, NubChild, NubTclAbi, Op, PauseReason, WireEvent,
    tcl_list_parse, validate_nub_tcl_abi,
};
use tcld_tui::debug::{
    AppState, BreakpointKindLabel, CoverageRow, EventSeverity, FocusedProc, PauseKind,
    PauseReasonLabel, ProcBreakpoint, SourceBlockKind, StackFrame, WatchEntry,
};

use super::config::NubPaths;

pub struct Session {
    ctrl: Controller,
    nub: NubChild,
    last_paused: Option<WireEvent>,
    last_error: Option<WireEvent>,
    bound_port: u16,
    _paths: NubPaths,
}

fn validate_session_nub_abi(
    capabilities: &[String],
    paths: &NubPaths,
) -> Result<NubTclAbi, String> {
    validate_nub_tcl_abi(capabilities).map_err(|error| {
        format!(
            "{error}\nselected debugger nub: {}\nselected Tcl host: {} ({})\n\
             The nub is stale or was built for an older controller contract. Reinstall with \
             `mise run tcld:install`, or rebuild it with `cargo build -p tcld-dbg-ext` and \
             pass the resulting library with `--dylib`.",
            paths.dylib.display(),
            paths.tclsh.display(),
            paths.tcl_patchlevel,
        )
    })
}

#[derive(Debug, Clone)]
pub struct ProcEntry {
    pub qname: String,
    pub block: Option<String>,
    pub first_offset: Option<u64>,
}

impl Session {
    /// Connect a v2 nub and return only after the Ready handshake. No user
    /// target is passed through the process environment or executed here.
    pub fn connect(paths: NubPaths) -> Result<Self, String> {
        let listen: std::net::SocketAddr = "127.0.0.1:0"
            .parse()
            .map_err(|e: std::net::AddrParseError| e.to_string())?;
        let (mut ctrl, port) = Controller::listen(listen).map_err(|e| format!("listen: {e}"))?;

        let mut nub = ctrl
            .spawn_nub(&paths.tclsh, &paths.bootstrap, &paths.dylib)
            .map_err(|e| {
                format!(
                    "failed to spawn debugger nub {} with {} (Tcl {}, expected ABI {}): {e}",
                    paths.dylib.display(),
                    paths.tclsh.display(),
                    paths.tcl_patchlevel,
                    paths
                        .tcl_abi
                        .map(|abi| abi.to_string())
                        .unwrap_or_else(|| "unprobed".to_string())
                )
            })?;

        let capabilities = ctrl.accept().map_err(|e| {
            let stderr = nub.exited_stderr().unwrap_or_default();
            let loader = if stderr.is_empty() {
                String::new()
            } else {
                format!("; loader output: {stderr}")
            };
            format!(
                "debugger handshake failed for nub {} with {} (Tcl {}, expected ABI {}): {e}",
                paths.dylib.display(),
                paths.tclsh.display(),
                paths.tcl_patchlevel,
                paths
                    .tcl_abi
                    .map(|abi| abi.to_string())
                    .unwrap_or_else(|| "unprobed".to_string()),
            ) + &loader
        })?;
        let compiled_abi = validate_session_nub_abi(&capabilities, &paths)?;
        if let Some(expected) = paths.tcl_abi {
            let expected = match expected {
                super::config::TclAbi::Tcl85 => NubTclAbi::Tcl85,
                super::config::TclAbi::Tcl86 => NubTclAbi::Tcl86,
                super::config::TclAbi::Tcl9 => NubTclAbi::Tcl9,
            };
            if compiled_abi != expected {
                return Err(format!(
                    "debugger nub ABI mismatch: selected host is {expected}, but {} reports {compiled_abi}",
                    paths.dylib.display()
                ));
            }
        }

        Ok(Self {
            ctrl,
            nub,
            last_paused: None,
            last_error: None,
            bound_port: port,
            _paths: paths,
        })
    }

    pub fn start_target(&mut self, path: &Path, stop_on_entry: bool) -> Result<String, String> {
        self.send(Op::StartTarget {
            path: path.to_string_lossy().into_owned(),
            stop_on_entry,
        })
    }

    /// Replace this session with a newly spawned nub and Tcl interpreter.
    ///
    /// A debugger restart must not re-source the target into the existing
    /// interpreter: globals, loaded packages, namespaces, and external side
    /// effects would survive. The replacement is connected first so a spawn
    /// failure leaves the existing session intact; after a successful
    /// handshake the old nub is closed.
    pub fn reconnect(&mut self) -> Result<(), String> {
        let paths = self._paths.clone();
        replace_with_fresh_session(self, || Self::connect(paths), Self::close)
    }

    pub fn close(mut self) {
        let _ = self.ctrl.close();
    }

    pub fn bound_port(&self) -> u16 {
        self.bound_port
    }

    /// Parsed view of the most recent `Paused` / `ErrorPaused` event.
    pub fn focused_proc(&self) -> Option<FocusedProc> {
        if let Some(err) = &self.last_error {
            return error_focused(err);
        }
        self.last_paused.as_ref().and_then(focused_from_paused)
    }

    pub fn last_error_event(&self) -> Option<&WireEvent> {
        self.last_error.as_ref()
    }

    pub fn clear_last_error(&mut self) {
        self.last_error = None;
    }

    /// Send an op and return the response payload string.
    pub fn send(&mut self, op: Op) -> Result<String, String> {
        match self.ctrl.send(op) {
            Ok(resp) => Ok(resp.into_payload()),
            Err(ControllerError::Wire(w)) => Err(format!("{}: {}", w.code.as_str(), w.message)),
            Err(other) => Err(other.to_string()),
        }
    }

    /// Lines the debugged program wrote since the last call.
    pub fn drain_output(&mut self) -> Vec<tcld_debug_wire::NubOutput> {
        self.nub.drain_output()
    }

    pub fn drain_events(&mut self) -> Vec<WireEvent> {
        let events = self.ctrl.drain_events();
        for ev in &events {
            match ev {
                WireEvent::Paused { .. } => {
                    self.last_paused = Some(ev.clone());
                }
                WireEvent::ErrorPaused { .. } => {
                    self.last_error = Some(ev.clone());
                }
                WireEvent::Resumed => {
                    // Keep last_paused for UI continuity, but clear
                    // the error overlay so a subsequent normal pause
                    // is rendered cleanly.
                    self.last_error = None;
                }
                _ => {}
            }
        }
        events
    }

    pub fn app_state(&mut self) -> Result<AppState, String> {
        let payload = self.send(Op::AppState)?;
        Ok(parse_app_state(&payload))
    }

    pub fn breakpoints(&mut self) -> Result<Vec<ProcBreakpoint>, String> {
        let payload = self.send(Op::BpList)?;
        Ok(parse_bp_list(&payload))
    }

    pub fn line_breakpoints_for(&mut self, qname: &str) -> Result<Vec<ProcBreakpoint>, String> {
        let payload = self.send(Op::LineBreakpoints {
            qname: Some(qname.to_string()),
            offset: None,
        })?;
        Ok(parse_bp_list(&payload))
    }

    pub fn watches(&mut self) -> Result<Vec<WatchEntry>, String> {
        let payload = self.send(Op::WatchList)?;
        Ok(parse_watch_list(&payload))
    }

    pub fn evaluate_watches(&mut self, level: i64) -> Result<Vec<WatchEntry>, String> {
        let payload = self.send(Op::WatchEvaluate { level })?;
        Ok(parse_watch_list(&payload))
    }

    pub fn variables(&mut self, level: i64) -> Result<Vec<(String, String)>, String> {
        let payload = self.send(Op::GetVariables { level })?;
        let descriptors = parse_variable_descriptors(&payload);
        let mut variables = Vec::with_capacity(descriptors.len());
        for (name, is_array) in descriptors {
            let value = self.send(Op::GetVar {
                level,
                name: name.clone(),
                maxlen: Some(512),
            })?;
            variables.push((
                name,
                if is_array {
                    format!("array {{{value}}}")
                } else {
                    value
                },
            ));
        }
        Ok(variables)
    }

    pub fn stack(&mut self) -> Result<Vec<StackFrame>, String> {
        let payload = self.send(Op::InspectStack)?;
        Ok(parse_stack(&payload)
            .into_iter()
            .filter(|frame| !is_internal_stack_frame(frame))
            .collect())
    }

    pub fn proc_location(&mut self, qname: &str) -> Result<Option<u64>, String> {
        let payload = self.send(Op::ProcLocation {
            qname: qname.to_string(),
        })?;
        Ok(parse_first_offset(&payload))
    }

    pub fn proc_body(&mut self, qname: &str) -> Result<String, String> {
        self.send(Op::ProcBody {
            qname: qname.to_string(),
        })
    }

    pub fn block_source(&mut self, owner_block_id: &str) -> Result<String, String> {
        self.send(Op::BlockSource {
            block: owner_block_id.to_owned(),
        })
    }

    pub fn proc_list(&mut self) -> Result<Vec<ProcEntry>, String> {
        let payload = self.send(Op::ProcList)?;
        Ok(parse_proc_list(&payload))
    }

    pub fn add_breakpoint(
        &mut self,
        qname: &str,
        offset: u64,
        condition: Option<&str>,
    ) -> Result<String, String> {
        self.send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: qname.to_string(),
                offset,
                len: 1,
            },
            condition: condition.map(str::to_string),
        })
    }

    pub fn add_command_breakpoint(
        &mut self,
        qname: &str,
        offset: u64,
        len: u64,
        condition: Option<&str>,
    ) -> Result<String, String> {
        self.send(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: qname.to_string(),
                offset,
                len,
            },
            condition: condition.map(str::to_string),
        })
    }

    pub fn add_file_breakpoint(
        &mut self,
        path: &Path,
        line: u32,
        condition: Option<&str>,
    ) -> Result<String, String> {
        self.send(Op::BpAdd {
            target: BreakpointTarget::FileLine {
                path: path.to_string_lossy().into_owned(),
                line: u64::from(line),
            },
            condition: condition.map(str::to_string),
        })
    }

    pub fn add_file_command_breakpoint(
        &mut self,
        path: &Path,
        offset: u64,
        len: u64,
        condition: Option<&str>,
    ) -> Result<String, String> {
        self.send(Op::BpAdd {
            target: BreakpointTarget::File {
                path: path.to_string_lossy().into_owned(),
                offset,
                len,
            },
            condition: condition.map(str::to_string),
        })
    }

    pub fn remove_breakpoint(&mut self, id: &str) -> Result<String, String> {
        self.send(Op::BpRemove { id: id.to_string() })
    }

    pub fn enable_breakpoint(&mut self, id: &str, enabled: bool) -> Result<String, String> {
        if enabled {
            self.send(Op::BpEnable { id: id.to_string() })
        } else {
            self.send(Op::BpDisable { id: id.to_string() })
        }
    }

    pub fn set_ignore(&mut self, id: &str, count: u64) -> Result<String, String> {
        self.send(Op::BpSetIgnore {
            id: id.to_string(),
            count,
        })
    }

    pub fn move_breakpoint(
        &mut self,
        id: &str,
        qname: &str,
        offset: u64,
    ) -> Result<String, String> {
        self.send(Op::BpMove {
            id: id.to_string(),
            target: BreakpointTarget::Source {
                qname: qname.to_string(),
                offset,
                len: 1,
            },
        })
    }

    /// Update the condition on an existing breakpoint by removing and
    /// re-adding (the wire `BpAdd` is the only way to set condition).
    pub fn update_condition(
        &mut self,
        bp: &ProcBreakpoint,
        condition: Option<&str>,
    ) -> Result<String, String> {
        self.send(Op::BpRemove { id: bp.id.clone() })?;
        let target = match (&bp.source_path, bp.line) {
            (Some(path), Some(line)) => BreakpointTarget::FileLine {
                path: path.clone(),
                line: u64::from(line),
            },
            (Some(path), _) => BreakpointTarget::File {
                path: path.clone(),
                offset: bp.offset,
                len: bp.len,
            },
            _ => BreakpointTarget::Source {
                qname: bp.qname.clone(),
                offset: bp.offset,
                len: bp.len,
            },
        };
        let new_id = self.send(Op::BpAdd {
            target,
            condition: condition.map(str::to_string),
        })?;
        if !bp.enabled {
            self.send(Op::BpDisable { id: new_id.clone() })?;
        }
        if bp.ignore_count > 0 {
            self.send(Op::BpSetIgnore {
                id: new_id.clone(),
                count: bp.ignore_count,
            })?;
        }
        Ok(new_id)
    }

    pub fn watch_add(&mut self, name: &str, expr: &str) -> Result<String, String> {
        self.send(Op::WatchAdd {
            name: name.to_string(),
            expr: expr.to_string(),
        })
    }

    pub fn watch_remove(&mut self, id: &str) -> Result<String, String> {
        self.send(Op::WatchRemove { id: id.to_string() })
    }

    pub fn watchpoint_add(&mut self, name: &str, ops: &str) -> Result<String, String> {
        self.send(Op::WpAdd {
            name: name.to_string(),
            ops: ops.to_string(),
        })
    }

    pub fn coverage(&mut self) -> Result<Vec<CoverageRow>, String> {
        let payload = self.send(Op::Coverage)?;
        Ok(parse_coverage(&payload))
    }

    pub fn coverage_enable(&mut self, on: bool) -> Result<String, String> {
        self.send(Op::CoverageEnable { on })
    }

    pub fn ignore_error(&mut self) -> Result<String, String> {
        self.send(Op::IgnoreError)
    }

    pub fn slave_list(&mut self) -> Result<Vec<String>, String> {
        let payload = self.send(Op::SlaveList)?;
        Ok(tcl_list_parse(&payload).unwrap_or_default())
    }

    pub fn pre_instrument_proc(&mut self, qname: &str, body: &str) -> Result<String, String> {
        self.send(Op::PreInstrumentProc {
            qname: qname.to_string(),
            body: body.to_string(),
        })
    }

    pub fn export_breakpoints(&mut self) -> Result<String, String> {
        self.send(Op::BpExport)
    }

    pub fn import_breakpoints(&mut self, serialized: &str) -> Result<String, String> {
        self.send(Op::BpImport {
            serialized: serialized.to_string(),
        })
    }
}

fn replace_with_fresh_session<T, E>(
    current: &mut T,
    connect: impl FnOnce() -> Result<T, E>,
    close: impl FnOnce(T),
) -> Result<(), E> {
    let fresh = connect()?;
    let old = std::mem::replace(current, fresh);
    close(old);
    Ok(())
}

fn parse_app_state(payload: &str) -> AppState {
    match payload.trim() {
        "stopped" | "Stopped" => AppState::Stopped,
        "running" | "Running" => AppState::Running,
        _ => AppState::Dead,
    }
}

fn focused_from_paused(ev: &WireEvent) -> Option<FocusedProc> {
    let WireEvent::Paused {
        location,
        frame_count,
        breakpoint,
        reason,
        ..
    } = ev
    else {
        return None;
    };
    let block = location.block_id.clone();
    let qname = location
        .qualified_proc
        .clone()
        .unwrap_or_else(|| block_to_qname(&block));
    Some(FocusedProc {
        block_id: block,
        block_kind: map_block_kind(location.block_kind),
        source_path: location.source_path.clone(),
        qname,
        line: location.line,
        offset_start: location.offset_start,
        offset_end: location.offset_end,
        pause_reason: map_pause_reason(*reason),
        frame_count: *frame_count,
        breakpoint: breakpoint.clone(),
        pause_kind: PauseKind::Normal,
        error_message: None,
    })
}

fn error_focused(ev: &WireEvent) -> Option<FocusedProc> {
    let WireEvent::ErrorPaused {
        location,
        message,
        breakpoint,
        ..
    } = ev
    else {
        return None;
    };
    let block = location
        .as_ref()
        .map(|loc| loc.block_id.clone())
        .unwrap_or_default();
    let line = location.as_ref().map(|loc| loc.line).unwrap_or(0);
    let qname = location
        .as_ref()
        .and_then(|loc| loc.qualified_proc.clone())
        .unwrap_or_else(|| block_to_qname(&block));
    Some(FocusedProc {
        block_id: block,
        block_kind: location
            .as_ref()
            .map(|loc| map_block_kind(loc.block_kind))
            .unwrap_or(SourceBlockKind::Anonymous),
        source_path: location.as_ref().and_then(|loc| loc.source_path.clone()),
        qname,
        line,
        offset_start: location.as_ref().map(|loc| loc.offset_start).unwrap_or(0),
        offset_end: location.as_ref().map(|loc| loc.offset_end).unwrap_or(1),
        pause_reason: PauseReasonLabel::Error,
        frame_count: 0,
        breakpoint: breakpoint.clone(),
        pause_kind: PauseKind::Error,
        error_message: Some(message.clone()),
    })
}

fn map_block_kind(kind: tcld_debug_wire::SourceBlockKind) -> SourceBlockKind {
    match kind {
        tcld_debug_wire::SourceBlockKind::File => SourceBlockKind::File,
        tcld_debug_wire::SourceBlockKind::Proc => SourceBlockKind::Proc,
        tcld_debug_wire::SourceBlockKind::Anonymous => SourceBlockKind::Anonymous,
    }
}

fn map_pause_reason(reason: PauseReason) -> PauseReasonLabel {
    match reason {
        PauseReason::Entry => PauseReasonLabel::Entry,
        PauseReason::Step => PauseReasonLabel::Step,
        PauseReason::Breakpoint => PauseReasonLabel::Breakpoint,
        PauseReason::Watchpoint => PauseReasonLabel::Watchpoint,
        PauseReason::Pause => PauseReasonLabel::Pause,
        PauseReason::InstructionStep => PauseReasonLabel::InstructionStep,
        PauseReason::InstructionBreakpoint => PauseReasonLabel::InstructionBreakpoint,
    }
}

fn parse_kv_list(payload: &str) -> Vec<(String, String)> {
    let parts = match tcl_list_parse(payload) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::with_capacity(parts.len() / 2);
    let mut iter = parts.into_iter();
    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
        out.push((k, v));
    }
    out
}

fn parse_variable_descriptors(payload: &str) -> Vec<(String, bool)> {
    let Ok(rows) = tcl_list_parse(payload) else {
        return Vec::new();
    };
    rows.into_iter()
        .filter_map(|row| {
            let fields = tcl_list_parse(&row).ok()?;
            let name = fields.first()?.clone();
            let is_array = fields.get(1).is_some_and(|kind| kind == "a");
            Some((name, is_array))
        })
        .collect()
}

fn parse_kv_dict(payload: &str) -> BTreeMap<String, String> {
    parse_kv_list(payload).into_iter().collect()
}

/// Walk an already-parsed list of tokens as k/v pairs without going
/// through the tcl-list parser a second time. Critical for fields
/// whose values contain whitespace (e.g. `condition {$x > 5}`):
/// re-joining the tokens with spaces would drop the brace grouping.
fn pairs_from_tokens(tokens: &[String]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut iter = tokens.iter();
    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
        map.insert(k.clone(), v.clone());
    }
    map
}

fn parse_first_offset(payload: &str) -> Option<u64> {
    parse_kv_dict(payload)
        .get("first_offset")
        .and_then(|s| s.parse().ok())
}

fn parse_bp_list(payload: &str) -> Vec<ProcBreakpoint> {
    let Ok(parts) = tcl_list_parse(payload) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let Ok(tokens) = tcl_list_parse(&part) else {
            continue;
        };
        let dict = pairs_from_tokens(&tokens);
        if let Some(id) = dict.get("id").cloned() {
            let kind_token = dict
                .get("kind")
                .cloned()
                .unwrap_or_else(|| "line".to_string());
            let kind = match kind_token.as_str() {
                "var" | "var_write" | "varwrite" => BreakpointKindLabel::VarWrite,
                "exception" | "error" => BreakpointKindLabel::Exception,
                _ => BreakpointKindLabel::Line,
            };
            out.push(ProcBreakpoint {
                id,
                qname: dict
                    .get("qname")
                    .cloned()
                    .or_else(|| {
                        dict.get("block")
                            .and_then(|block| block.strip_prefix("proc:").map(str::to_string))
                    })
                    .unwrap_or_default(),
                source_path: dict.get("path").cloned().or_else(|| {
                    dict.get("block")
                        .and_then(|block| block.strip_prefix("file:").map(str::to_string))
                }),
                line: dict.get("line").and_then(|s| s.parse().ok()),
                pending: dict
                    .get("pending")
                    .map(|s| matches!(s.as_str(), "1" | "true" | "yes" | "on"))
                    .unwrap_or(false),
                pending_reason: dict.get("pending_reason").cloned(),
                offset: dict.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0),
                len: dict.get("len").and_then(|s| s.parse().ok()).unwrap_or(1),
                resolved_block: dict.get("resolved_block").cloned(),
                resolved_line: dict.get("resolved_line").and_then(|s| s.parse().ok()),
                resolved_offset: dict.get("resolved_offset").and_then(|s| s.parse().ok()),
                resolved_len: dict.get("resolved_len").and_then(|s| s.parse().ok()),
                resolution_generation: dict
                    .get("resolution_generation")
                    .and_then(|s| s.parse().ok()),
                enabled: dict
                    .get("enabled")
                    .map(|s| matches!(s.as_str(), "1" | "true" | "yes" | "on"))
                    .unwrap_or(true),
                hits: dict.get("hits").and_then(|s| s.parse().ok()).unwrap_or(0),
                ignore_count: dict.get("ignore").and_then(|s| s.parse().ok()).unwrap_or(0),
                condition: dict.get("condition").cloned(),
                kind,
                location_label: None,
            });
        }
    }
    out
}

fn parse_watch_list(payload: &str) -> Vec<WatchEntry> {
    let Ok(parts) = tcl_list_parse(payload) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let Ok(tokens) = tcl_list_parse(&part) else {
            continue;
        };
        let dict = pairs_from_tokens(&tokens);
        if let Some(id) = dict.get("id").cloned() {
            out.push(WatchEntry {
                id,
                name: dict.get("name").cloned().unwrap_or_default(),
                expr: dict.get("expr").cloned().unwrap_or_default(),
                last_value: dict.get("value").cloned(),
                last_error: dict.get("error").cloned(),
                changed: false,
            });
        }
    }
    out
}

fn parse_stack(payload: &str) -> Vec<StackFrame> {
    let Ok(parts) = tcl_list_parse(payload) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let Ok(tokens) = tcl_list_parse(&part) else {
            continue;
        };
        let dict = pairs_from_tokens(&tokens);
        out.push(StackFrame {
            level: dict.get("level").and_then(|s| s.parse().ok()).unwrap_or(0),
            kind: dict.get("kind").cloned().unwrap_or_else(|| "?".to_string()),
            proc_name: dict
                .get("proc")
                .cloned()
                .or_else(|| dict.get("name").cloned()),
            block: dict.get("block").cloned(),
            block_kind: dict.get("block_kind").and_then(|kind| match kind.as_str() {
                "file" => Some(SourceBlockKind::File),
                "proc" => Some(SourceBlockKind::Proc),
                "anonymous" => Some(SourceBlockKind::Anonymous),
                _ => None,
            }),
            source_path: dict.get("source_path").cloned(),
            line: dict.get("line").and_then(|s| s.parse().ok()),
            offset_start: dict.get("offset_start").and_then(|s| s.parse().ok()),
            offset_end: dict.get("offset_end").and_then(|s| s.parse().ok()),
            location_label: None,
        });
    }
    out
}

fn is_internal_stack_frame(frame: &StackFrame) -> bool {
    frame
        .proc_name
        .as_deref()
        .is_some_and(|name| name.trim_start_matches("::").starts_with("tcld::dbg::"))
}

fn parse_proc_list(payload: &str) -> Vec<ProcEntry> {
    let Ok(parts) = tcl_list_parse(payload) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        if let Ok(tokens) = tcl_list_parse(&part)
            && tokens.len() >= 2
        {
            let mut iter = tokens.into_iter();
            let qname = iter.next().unwrap_or_default();
            let rest: Vec<String> = iter.collect();
            let dict = pairs_from_tokens(&rest);
            out.push(ProcEntry {
                qname,
                block: dict.get("block").cloned(),
                first_offset: dict.get("first_offset").and_then(|s| s.parse().ok()),
            });
        } else {
            out.push(ProcEntry {
                qname: part,
                block: None,
                first_offset: None,
            });
        }
    }
    out
}

fn parse_coverage(payload: &str) -> Vec<CoverageRow> {
    let Ok(parts) = tcl_list_parse(payload) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let Ok(tokens) = tcl_list_parse(&part) else {
            continue;
        };
        let dict = pairs_from_tokens(&tokens);
        if let (Some(block), Some(offset_str)) = (dict.get("block"), dict.get("offset")) {
            let offset = offset_str.parse().unwrap_or(0);
            let hits = dict.get("hits").and_then(|s| s.parse().ok()).unwrap_or(0);
            out.push(CoverageRow {
                block: block.clone(),
                offset,
                hits,
            });
        }
    }
    out
}

pub fn format_event(ev: &WireEvent) -> (EventSeverity, String) {
    match ev {
        WireEvent::TargetStarted { path, status } => (
            EventSeverity::Info,
            format!("target_started path={path} status={status}"),
        ),
        WireEvent::TargetCompleted {
            path,
            status,
            result,
        } => (
            EventSeverity::Info,
            format!("target_completed path={path} status={status} result={result}"),
        ),
        WireEvent::TargetLoadFailed {
            path,
            status,
            message,
            error_code,
            error_info,
        } => (
            EventSeverity::Error,
            format!(
                "target_load_failed path={path} status={status} message={message}\nerror_code={error_code}\nerror_info={error_info}"
            ),
        ),
        WireEvent::Paused {
            location,
            frame_count,
            breakpoint,
            reason,
            pc,
        } => {
            let bp = breakpoint.as_deref().unwrap_or("-");
            let pc_part = pc.map(|pc| format!(" pc={pc}")).unwrap_or_default();
            (
                EventSeverity::Pause,
                format!(
                    "paused reason={} frame={frame_count} bp={bp}{pc_part} loc={}:{} offset={}..{}",
                    reason.as_str(),
                    location
                        .source_path
                        .as_deref()
                        .or(location.qualified_proc.as_deref())
                        .unwrap_or(&location.block_id),
                    location.line,
                    location.offset_start,
                    location.offset_end,
                ),
            )
        }
        WireEvent::Resumed => (EventSeverity::Info, "resumed".to_string()),
        WireEvent::ErrorPaused {
            message,
            breakpoint,
            ..
        } => {
            let bp = breakpoint.as_deref().unwrap_or("-");
            (
                EventSeverity::Error,
                format!("error_paused bp={bp} message={message}"),
            )
        }
        WireEvent::ConditionError {
            breakpoint,
            message,
        } => (
            EventSeverity::Warn,
            format!("condition_error bp={breakpoint} message={message}"),
        ),
        WireEvent::BlocksRevalidated {
            block,
            generation,
            change,
            ..
        } => (
            EventSeverity::Info,
            format!(
                "blocks_revalidated block={block} generation={generation} change={}",
                change.as_str()
            ),
        ),
        WireEvent::NubStarted {
            pid,
            version,
            tcl_version,
        } => (
            EventSeverity::Info,
            format!("nub_started pid={pid} version={version} tcl={tcl_version}"),
        ),
        WireEvent::NubStopped { reason } => {
            (EventSeverity::Warn, format!("nub_stopped reason={reason}"))
        }
    }
}

/// Map a block id to a `bp_add`-friendly qname. For procs the engine
/// expects the bare qualified name; for files / anonymous blocks we
/// pass the id through unchanged so the user gets a precise wire
/// error rather than a silent address fallback.
pub fn block_to_qname(block: &str) -> String {
    if let Some(rest) = block.strip_prefix("proc:") {
        rest.to_string()
    } else if block.is_empty() {
        String::new()
    } else {
        block.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc_location(line: u32, start: u64, end: u64) -> tcld_debug_wire::SourceLocation {
        tcld_debug_wire::SourceLocation {
            block_id: "proc:demo".into(),
            block_kind: tcld_debug_wire::SourceBlockKind::Proc,
            source_path: Some("/tmp/demo.tcl".into()),
            qualified_proc: Some("demo".into()),
            line,
            offset_start: start,
            offset_end: end,
        }
    }

    #[test]
    fn block_to_qname_handles_proc_file_and_anon() {
        assert_eq!(block_to_qname("proc:demo"), "demo");
        assert_eq!(block_to_qname("proc:::ns::deep"), "::ns::deep");
        assert_eq!(block_to_qname("file:/tmp/x.tcl"), "file:/tmp/x.tcl");
        assert_eq!(block_to_qname("anon:DEADBEEF"), "anon:DEADBEEF");
        assert_eq!(block_to_qname(""), "");
    }

    #[test]
    fn parse_app_state_recognises_engine_words() {
        assert_eq!(parse_app_state("stopped"), AppState::Stopped);
        assert_eq!(parse_app_state(" Running"), AppState::Running);
        assert_eq!(parse_app_state("anything-else"), AppState::Dead);
        assert_eq!(parse_app_state(""), AppState::Dead);
    }

    #[test]
    fn parse_kv_list_preserves_engine_order() {
        let kv = parse_kv_list("alpha 1 beta two gamma {three four}");
        assert_eq!(
            kv,
            vec![
                ("alpha".to_string(), "1".to_string()),
                ("beta".to_string(), "two".to_string()),
                ("gamma".to_string(), "three four".to_string()),
            ]
        );
    }

    #[test]
    fn parse_variable_descriptors_preserves_rows_and_array_kind() {
        assert_eq!(
            parse_variable_descriptors("{alpha s} {items a}"),
            vec![("alpha".to_string(), false), ("items".to_string(), true)]
        );
    }

    #[test]
    fn parse_bp_list_round_trips_line_var_and_exception_kinds() {
        let payload = "{id bp0 qname demo offset 8 enabled 1 hits 2 condition {$x > 5} kind line} \
                       {id bp1 qname demo offset 0 enabled 0 hits 0 kind var} \
                       {id bp2 qname * offset 0 enabled 1 hits 0 kind exception}";
        let bps = parse_bp_list(payload);
        assert_eq!(bps.len(), 3);
        assert_eq!(bps[0].id, "bp0");
        assert_eq!(bps[0].kind, BreakpointKindLabel::Line);
        assert_eq!(bps[0].condition.as_deref(), Some("$x > 5"));
        assert_eq!(bps[0].hits, 2);
        assert!(bps[0].enabled);
        assert_eq!(bps[1].kind, BreakpointKindLabel::VarWrite);
        assert!(!bps[1].enabled);
        assert_eq!(bps[2].kind, BreakpointKindLabel::Exception);
    }

    #[test]
    fn parse_bp_list_preserves_resolved_file_line_identity() {
        let payload = "{id bp7 kind line block {file:/tmp/demo.tcl} offset 18 len 7 \
                       line 3 enabled 1 hits 4 ignore 2 pending 0 condition {$ready}}";
        let bps = parse_bp_list(payload);
        assert_eq!(bps.len(), 1);
        assert_eq!(bps[0].source_path.as_deref(), Some("/tmp/demo.tcl"));
        assert_eq!(bps[0].line, Some(3));
        assert_eq!(bps[0].offset, 18);
        assert_eq!(bps[0].len, 7);
        assert_eq!(bps[0].ignore_count, 2);
        assert_eq!(bps[0].condition.as_deref(), Some("$ready"));
        assert!(!bps[0].pending);
    }

    #[test]
    fn parse_watch_list_returns_value_and_error_separately() {
        let payload = "{id w0 name size expr {[llength $items]} value 42} \
                       {id w1 name first expr {[lindex $items 0]} error {bad index}}";
        let watches = parse_watch_list(payload);
        assert_eq!(watches.len(), 2);
        assert_eq!(watches[0].name, "size");
        assert_eq!(watches[0].last_value.as_deref(), Some("42"));
        assert!(watches[0].last_error.is_none());
        assert_eq!(watches[1].name, "first");
        assert_eq!(watches[1].last_error.as_deref(), Some("bad index"));
    }

    #[test]
    fn parse_stack_round_trips_levels_and_blocks() {
        let payload = "{level 1 kind proc proc demo block proc:demo line 2} \
                       {level 0 kind toplevel name :: line 6}";
        let frames = parse_stack(payload);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].level, 1);
        assert_eq!(frames[0].proc_name.as_deref(), Some("demo"));
        assert_eq!(frames[0].block.as_deref(), Some("proc:demo"));
        assert_eq!(frames[0].line, Some(2));
        assert_eq!(frames[1].level, 0);
        assert_eq!(frames[1].proc_name.as_deref(), Some("::"));
    }

    #[test]
    fn internal_nub_frames_are_identified_without_hiding_toplevel() {
        let frames = parse_stack(
            "{level 0 kind toplevel name :: line 1} \
             {level 3 kind proc proc tcld::dbg::__nub__::run_target line 22} \
             {level 4 kind proc proc ::tcld::dbg::__nub__::call_op line 33} \
             {level 1 kind proc proc outer line 8}",
        );
        let visible = frames
            .iter()
            .filter(|frame| !is_internal_stack_frame(frame))
            .collect::<Vec<_>>();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].proc_name.as_deref(), Some("::"));
        assert_eq!(visible[1].proc_name.as_deref(), Some("outer"));
    }

    #[test]
    fn parse_proc_list_handles_flat_and_dictionary_forms() {
        let flat = parse_proc_list("foo bar ::ns::baz");
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].qname, "foo");
        assert!(flat[0].first_offset.is_none());

        let rich =
            parse_proc_list("{demo block proc:demo first_offset 8} {alpha block proc:alpha}");
        assert_eq!(rich.len(), 2);
        assert_eq!(rich[0].qname, "demo");
        assert_eq!(rich[0].first_offset, Some(8));
        assert_eq!(rich[0].block.as_deref(), Some("proc:demo"));
    }

    #[test]
    fn parse_first_offset_extracts_field() {
        assert_eq!(parse_first_offset("first_offset 8 last_offset 10"), Some(8));
        assert!(parse_first_offset("not present").is_none());
    }

    #[test]
    fn parse_coverage_collects_block_offset_hits_triples() {
        let payload = "{block proc:demo offset 0 hits 5} {block proc:demo offset 8 hits 3}";
        let rows = parse_coverage(payload);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].block, "proc:demo");
        assert_eq!(rows[0].offset, 0);
        assert_eq!(rows[0].hits, 5);
        assert_eq!(rows[1].offset, 8);
    }

    #[test]
    fn format_event_assigns_severity_per_kind() {
        let (sev, _) = format_event(&WireEvent::Resumed);
        assert_eq!(sev, EventSeverity::Info);
        let (sev, msg) = format_event(&WireEvent::Paused {
            location: proc_location(1, 0, 1),
            frame_count: 1,
            breakpoint: Some("bp0".into()),
            reason: tcld_debug_wire::PauseReason::Breakpoint,
            pc: None,
        });
        assert_eq!(sev, EventSeverity::Pause);
        assert!(msg.contains("paused"));
        let (sev, _) = format_event(&WireEvent::ErrorPaused {
            location: None,
            message: "boom".into(),
            error_code: None,
            breakpoint: None,
        });
        assert_eq!(sev, EventSeverity::Error);
        let (sev, _) = format_event(&WireEvent::ConditionError {
            breakpoint: "bp0".into(),
            message: "syntax".into(),
        });
        assert_eq!(sev, EventSeverity::Warn);
    }

    #[test]
    fn focused_from_paused_extracts_location_and_breakpoint() {
        let f = focused_from_paused(&WireEvent::Paused {
            location: proc_location(2, 8, 10),
            frame_count: 1,
            breakpoint: Some("bp0".into()),
            reason: tcld_debug_wire::PauseReason::Breakpoint,
            pc: None,
        })
        .unwrap();
        assert_eq!(f.qname, "demo");
        assert_eq!(f.block_id, "proc:demo");
        assert_eq!(f.line, 2);
        assert_eq!(f.breakpoint.as_deref(), Some("bp0"));
        assert!(matches!(f.pause_kind, PauseKind::Normal));
    }

    #[test]
    fn error_focused_carries_message_and_marks_pause_kind() {
        let f = error_focused(&WireEvent::ErrorPaused {
            location: Some(proc_location(5, 0, 1)),
            message: "divide by zero".into(),
            error_code: Some("ARITH DIVZERO".into()),
            breakpoint: None,
        })
        .unwrap();
        assert!(matches!(f.pause_kind, PauseKind::Error));
        assert_eq!(f.error_message.as_deref(), Some("divide by zero"));
        assert_eq!(f.line, 5);
    }

    #[test]
    fn restart_replacement_installs_fresh_state_and_closes_the_old_state() {
        let mut current = String::from("dirty interpreter");
        let mut closed = None;
        replace_with_fresh_session(
            &mut current,
            || Ok::<_, ()>(String::from("fresh interpreter")),
            |old| closed = Some(old),
        )
        .unwrap();
        assert_eq!(current, "fresh interpreter");
        assert_eq!(closed.as_deref(), Some("dirty interpreter"));
    }

    #[test]
    fn failed_restart_connection_preserves_the_existing_session() {
        let mut current = String::from("still connected");
        let mut closed = false;
        let result = replace_with_fresh_session(
            &mut current,
            || Err::<String, _>("spawn failed"),
            |_| closed = true,
        );
        assert_eq!(result, Err("spawn failed"));
        assert_eq!(current, "still connected");
        assert!(!closed);
    }

    #[test]
    fn stale_nub_handshake_names_the_selected_artifact_and_durable_repair() {
        let paths = NubPaths::explicit(
            "/opt/homebrew/bin/tclsh9.0".into(),
            "/tmp/tcld_nub_bootstrap.tcl".into(),
            "/checkout/target/debug/libtcld_dbg.dylib".into(),
        );
        let error = validate_session_nub_abi(&[], &paths).unwrap_err();
        assert!(
            error.contains("/checkout/target/debug/libtcld_dbg.dylib"),
            "{error}"
        );
        assert!(error.contains("/opt/homebrew/bin/tclsh9.0"), "{error}");
        assert!(error.contains("mise run tcld:install"), "{error}");
        assert!(error.contains("cargo build -p tcld-dbg-ext"), "{error}");
    }
}
