//! Op vocabulary (request → response).
//!
//! Closed v2 set; mirrors every Tier 0–3 `::tcld::dbg::*` command.
//! Each variant knows how to encode itself to a Tcl-list payload
//! (op name + args) and decode incoming payloads back into typed
//! form. Responses are op-specific dicts, but for v2 we model them
//! as opaque [`OpResponse::Payload`] strings — the Rust controller
//! exposes the raw payload to the caller, which is sufficient for
//! the wire to be lossless. Higher-level typed accessors can be
//! added later without changing the wire format.

use crate::tcl_list::{tcl_list_format, tcl_list_parse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakpointTarget {
    Source {
        qname: String,
        offset: u64,
        len: u64,
    },
    File {
        path: String,
        offset: u64,
        len: u64,
    },
    FileLine {
        path: String,
        line: u64,
    },
    Instruction {
        pc: u64,
    },
}

impl BreakpointTarget {
    fn append_payload_parts(&self, parts: &mut Vec<String>) {
        match self {
            Self::Source { qname, offset, len } => {
                parts.push("source".into());
                parts.push(qname.clone());
                parts.push(offset.to_string());
                parts.push(len.to_string());
            }
            Self::File { path, offset, len } => {
                parts.push("file".into());
                parts.push(path.clone());
                parts.push(offset.to_string());
                parts.push(len.to_string());
            }
            Self::Instruction { pc } => {
                parts.push("instruction".into());
                parts.push(pc.to_string());
            }
            Self::FileLine { path, line } => {
                parts.push("file_line".into());
                parts.push(path.clone());
                parts.push(line.to_string());
            }
        }
    }
}

/// Closed list of v2 op names. Sent on the wire as the first
/// element of a `req` frame's payload list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    StartTarget {
        path: String,
        stop_on_entry: bool,
    },
    StepInto,
    StepOver,
    StepOut,
    StepInstruction,
    StepOverInstruction,
    StepOutInstruction,
    Continue,
    Interrupt,
    CurrentPc,
    RunToPc {
        pc: u64,
    },
    AppState,
    InspectStack,
    EvalInFrame {
        level: i64,
        script: String,
    },
    GetLevel,
    GetResult {
        maxlen: Option<u64>,
    },
    GetVariables {
        level: i64,
    },
    GetVar {
        level: i64,
        name: String,
        maxlen: Option<u64>,
    },
    SetVar {
        level: i64,
        name: String,
        value: String,
    },
    BpAdd {
        target: BreakpointTarget,
        condition: Option<String>,
    },
    BpRemove {
        id: String,
    },
    BpList,
    BpDisable {
        id: String,
    },
    BpEnable {
        id: String,
    },
    BpMove {
        id: String,
        target: BreakpointTarget,
    },
    BpValidate {
        qname: String,
    },
    BpSetIgnore {
        id: String,
        count: u64,
    },
    BpExport,
    BpImport {
        serialized: String,
    },
    WpAdd {
        name: String,
        ops: String,
    },
    VarBreakpoints {
        name: Option<String>,
    },
    LineBreakpoints {
        qname: Option<String>,
        offset: Option<u64>,
    },
    InstructionBreakpoints {
        pc: Option<u64>,
    },
    ProcList,
    ProcBody {
        qname: String,
    },
    ProcLocation {
        qname: String,
    },
    BlockDisable {
        qname: String,
    },
    BlockEnable {
        qname: String,
    },
    Coverage,
    CoverageEnable {
        on: bool,
    },
    WatchAdd {
        name: String,
        expr: String,
    },
    WatchRemove {
        id: String,
    },
    WatchList,
    WatchEvaluate {
        level: i64,
    },
    PreInstrumentProc {
        qname: String,
        body: String,
    },
    PreInstrumentFile {
        path: String,
        body: String,
    },
    SlaveList,
    SlaveInstall {
        name: String,
    },
    IgnoreError,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpParseError {
    TclList(crate::tcl_list::TclListError),
    Empty,
    UnknownOp(String),
    BadArgs { op: String, reason: String },
}

impl std::fmt::Display for OpParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TclList(e) => write!(f, "tcl list error: {e}"),
            Self::Empty => write!(f, "empty op payload"),
            Self::UnknownOp(s) => write!(f, "unknown op: {s}"),
            Self::BadArgs { op, reason } => write!(f, "bad args for op `{op}`: {reason}"),
        }
    }
}

impl std::error::Error for OpParseError {}

impl Op {
    /// Op name string (the first element of the wire payload).
    pub fn name(&self) -> &'static str {
        match self {
            Self::StartTarget { .. } => "start_target",
            Self::StepInto => "step_into",
            Self::StepOver => "step_over",
            Self::StepOut => "step_out",
            Self::StepInstruction => "step_instruction",
            Self::StepOverInstruction => "step_over_instruction",
            Self::StepOutInstruction => "step_out_instruction",
            Self::Continue => "continue",
            Self::Interrupt => "interrupt",
            Self::CurrentPc => "current_pc",
            Self::RunToPc { .. } => "run_to_pc",
            Self::AppState => "app_state",
            Self::InspectStack => "inspect_stack",
            Self::EvalInFrame { .. } => "eval_in_frame",
            Self::GetLevel => "get_level",
            Self::GetResult { .. } => "get_result",
            Self::GetVariables { .. } => "get_variables",
            Self::GetVar { .. } => "get_var",
            Self::SetVar { .. } => "set_var",
            Self::BpAdd { .. } => "bp_add",
            Self::BpRemove { .. } => "bp_remove",
            Self::BpList => "bp_list",
            Self::BpDisable { .. } => "bp_disable",
            Self::BpEnable { .. } => "bp_enable",
            Self::BpMove { .. } => "bp_move",
            Self::BpValidate { .. } => "bp_validate",
            Self::BpSetIgnore { .. } => "bp_set_ignore",
            Self::BpExport => "bp_export",
            Self::BpImport { .. } => "bp_import",
            Self::WpAdd { .. } => "wp_add",
            Self::VarBreakpoints { .. } => "var_breakpoints",
            Self::LineBreakpoints { .. } => "line_breakpoints",
            Self::InstructionBreakpoints { .. } => "instruction_breakpoints",
            Self::ProcList => "proc_list",
            Self::ProcBody { .. } => "proc_body",
            Self::ProcLocation { .. } => "proc_location",
            Self::BlockDisable { .. } => "block_disable",
            Self::BlockEnable { .. } => "block_enable",
            Self::Coverage => "coverage",
            Self::CoverageEnable { .. } => "coverage_enable",
            Self::WatchAdd { .. } => "watch_add",
            Self::WatchRemove { .. } => "watch_remove",
            Self::WatchList => "watch_list",
            Self::WatchEvaluate { .. } => "watch_evaluate",
            Self::PreInstrumentProc { .. } => "pre_instrument_proc",
            Self::PreInstrumentFile { .. } => "pre_instrument_file",
            Self::SlaveList => "slave_list",
            Self::SlaveInstall { .. } => "slave_install",
            Self::IgnoreError => "ignore_error",
            Self::Quit => "quit",
        }
    }

    pub fn to_payload(&self) -> String {
        let mut parts: Vec<String> = vec![self.name().to_string()];
        match self {
            Self::StepInto
            | Self::StepOver
            | Self::StepOut
            | Self::StepInstruction
            | Self::StepOverInstruction
            | Self::StepOutInstruction
            | Self::Continue
            | Self::Interrupt
            | Self::CurrentPc
            | Self::AppState
            | Self::InspectStack
            | Self::GetLevel
            | Self::BpList
            | Self::BpExport
            | Self::ProcList
            | Self::Coverage
            | Self::WatchList
            | Self::SlaveList
            | Self::IgnoreError
            | Self::Quit => {}
            Self::StartTarget {
                path,
                stop_on_entry,
            } => {
                parts.push(path.clone());
                parts.push(if *stop_on_entry { "1" } else { "0" }.to_string());
            }
            Self::RunToPc { pc } => {
                parts.push(pc.to_string());
            }
            Self::EvalInFrame { level, script } => {
                parts.push(level.to_string());
                parts.push(script.clone());
            }
            Self::GetResult { maxlen } => {
                if let Some(n) = maxlen {
                    parts.push(n.to_string());
                }
            }
            Self::GetVariables { level } => {
                parts.push(level.to_string());
            }
            Self::GetVar {
                level,
                name,
                maxlen,
            } => {
                parts.push(level.to_string());
                parts.push(name.clone());
                if let Some(n) = maxlen {
                    parts.push(n.to_string());
                }
            }
            Self::SetVar { level, name, value } => {
                parts.push(level.to_string());
                parts.push(name.clone());
                parts.push(value.clone());
            }
            Self::BpAdd { target, condition } => {
                target.append_payload_parts(&mut parts);
                if let Some(c) = condition {
                    parts.push(c.clone());
                }
            }
            Self::BpRemove { id } | Self::BpDisable { id } | Self::BpEnable { id } => {
                parts.push(id.clone());
            }
            Self::BpMove { id, target } => {
                parts.push(id.clone());
                target.append_payload_parts(&mut parts);
            }
            Self::BpValidate { qname }
            | Self::ProcBody { qname }
            | Self::ProcLocation { qname }
            | Self::BlockDisable { qname }
            | Self::BlockEnable { qname } => {
                parts.push(qname.clone());
            }
            Self::BpSetIgnore { id, count } => {
                parts.push(id.clone());
                parts.push(count.to_string());
            }
            Self::BpImport { serialized } => {
                parts.push(serialized.clone());
            }
            Self::WpAdd { name, ops } => {
                parts.push(name.clone());
                parts.push(ops.clone());
            }
            Self::VarBreakpoints { name } => {
                if let Some(n) = name {
                    parts.push(n.clone());
                }
            }
            Self::LineBreakpoints { qname, offset } => {
                if let Some(q) = qname {
                    parts.push(q.clone());
                    if let Some(o) = offset {
                        parts.push(o.to_string());
                    }
                }
            }
            Self::InstructionBreakpoints { pc } => {
                if let Some(pc) = pc {
                    parts.push(pc.to_string());
                }
            }
            Self::CoverageEnable { on } => {
                parts.push((if *on { "on" } else { "off" }).to_string());
            }
            Self::WatchAdd { name, expr } => {
                parts.push(name.clone());
                parts.push(expr.clone());
            }
            Self::WatchRemove { id } => {
                parts.push(id.clone());
            }
            Self::WatchEvaluate { level } => {
                parts.push(level.to_string());
            }
            Self::PreInstrumentProc { qname, body } => {
                parts.push(qname.clone());
                parts.push(body.clone());
            }
            Self::PreInstrumentFile { path, body } => {
                parts.push(path.clone());
                parts.push(body.clone());
            }
            Self::SlaveInstall { name } => {
                parts.push(name.clone());
            }
        }
        tcl_list_format(parts.iter())
    }

    pub fn from_payload(payload: &str) -> Result<Self, OpParseError> {
        let parts = tcl_list_parse(payload).map_err(OpParseError::TclList)?;
        let mut iter = parts.into_iter();
        let name = iter.next().ok_or(OpParseError::Empty)?;
        let rest: Vec<String> = iter.collect();
        let parse_int = |s: &str, op: &str, field: &str| -> Result<i64, OpParseError> {
            s.parse::<i64>().map_err(|_| OpParseError::BadArgs {
                op: op.to_string(),
                reason: format!("{field} must be integer, got {s}"),
            })
        };
        let parse_uint = |s: &str, op: &str, field: &str| -> Result<u64, OpParseError> {
            s.parse::<u64>().map_err(|_| OpParseError::BadArgs {
                op: op.to_string(),
                reason: format!("{field} must be non-negative integer, got {s}"),
            })
        };
        match name.as_str() {
            "start_target" => {
                if rest.len() != 2 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected path + stop_on_entry boolean".into(),
                    });
                }
                let stop_on_entry = match rest[1].as_str() {
                    "1" | "true" | "on" | "yes" => true,
                    "0" | "false" | "off" | "no" => false,
                    value => {
                        return Err(OpParseError::BadArgs {
                            op: name,
                            reason: format!("stop_on_entry must be boolean, got {value}"),
                        });
                    }
                };
                Ok(Self::StartTarget {
                    path: rest[0].clone(),
                    stop_on_entry,
                })
            }
            "step_into" => Ok(Self::StepInto),
            "step_over" => Ok(Self::StepOver),
            "step_out" => Ok(Self::StepOut),
            "step_instruction" => Ok(Self::StepInstruction),
            "step_over_instruction" => Ok(Self::StepOverInstruction),
            "step_out_instruction" => Ok(Self::StepOutInstruction),
            "continue" => Ok(Self::Continue),
            "interrupt" => Ok(Self::Interrupt),
            "current_pc" => Ok(Self::CurrentPc),
            "run_to_pc" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected pc".into(),
                    });
                }
                Ok(Self::RunToPc {
                    pc: parse_uint(&rest[0], "run_to_pc", "pc")?,
                })
            }
            "app_state" => Ok(Self::AppState),
            "inspect_stack" => Ok(Self::InspectStack),
            "get_level" => Ok(Self::GetLevel),
            "bp_list" => Ok(Self::BpList),
            "bp_export" => Ok(Self::BpExport),
            "proc_list" => Ok(Self::ProcList),
            "coverage" => Ok(Self::Coverage),
            "watch_list" => Ok(Self::WatchList),
            "slave_list" => Ok(Self::SlaveList),
            "ignore_error" => Ok(Self::IgnoreError),
            "quit" => Ok(Self::Quit),
            "eval_in_frame" => {
                if rest.len() != 2 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected level + script".into(),
                    });
                }
                Ok(Self::EvalInFrame {
                    level: parse_int(&rest[0], "eval_in_frame", "level")?,
                    script: rest[1].clone(),
                })
            }
            "get_result" => Ok(Self::GetResult {
                maxlen: rest
                    .first()
                    .map(|s| parse_uint(s, "get_result", "maxlen"))
                    .transpose()?,
            }),
            "get_variables" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected level".into(),
                    });
                }
                Ok(Self::GetVariables {
                    level: parse_int(&rest[0], "get_variables", "level")?,
                })
            }
            "get_var" => {
                if rest.len() < 2 || rest.len() > 3 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected level + name + ?maxlen?".into(),
                    });
                }
                Ok(Self::GetVar {
                    level: parse_int(&rest[0], "get_var", "level")?,
                    name: rest[1].clone(),
                    maxlen: rest
                        .get(2)
                        .map(|s| parse_uint(s, "get_var", "maxlen"))
                        .transpose()?,
                })
            }
            "set_var" => {
                if rest.len() != 3 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected level + name + value".into(),
                    });
                }
                Ok(Self::SetVar {
                    level: parse_int(&rest[0], "set_var", "level")?,
                    name: rest[1].clone(),
                    value: rest[2].clone(),
                })
            }
            "bp_add" => {
                let (target, condition) = parse_breakpoint_target("bp_add", &rest, true)?;
                Ok(Self::BpAdd { target, condition })
            }
            "bp_move" => {
                if rest.is_empty() {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected bp_id + target".into(),
                    });
                }
                let id = rest[0].clone();
                let (target, _) = parse_breakpoint_target("bp_move", &rest[1..], false)?;
                Ok(Self::BpMove { id, target })
            }
            "instruction_breakpoints" => Ok(Self::InstructionBreakpoints {
                pc: rest
                    .first()
                    .map(|s| parse_uint(s, "instruction_breakpoints", "pc"))
                    .transpose()?,
            }),
            "bp_remove" | "bp_disable" | "bp_enable" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name.clone(),
                        reason: "expected bp_id".into(),
                    });
                }
                let id = rest[0].clone();
                Ok(match name.as_str() {
                    "bp_remove" => Self::BpRemove { id },
                    "bp_disable" => Self::BpDisable { id },
                    "bp_enable" => Self::BpEnable { id },
                    _ => unreachable!(),
                })
            }
            "bp_validate" => one_arg_qname(name, &rest, |q| Self::BpValidate { qname: q }),
            "proc_body" => one_arg_qname(name, &rest, |q| Self::ProcBody { qname: q }),
            "proc_location" => one_arg_qname(name, &rest, |q| Self::ProcLocation { qname: q }),
            "block_disable" => one_arg_qname(name, &rest, |q| Self::BlockDisable { qname: q }),
            "block_enable" => one_arg_qname(name, &rest, |q| Self::BlockEnable { qname: q }),
            "bp_set_ignore" => {
                if rest.len() != 2 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected bp_id + count".into(),
                    });
                }
                Ok(Self::BpSetIgnore {
                    id: rest[0].clone(),
                    count: parse_uint(&rest[1], "bp_set_ignore", "count")?,
                })
            }
            "bp_import" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected serialized payload".into(),
                    });
                }
                Ok(Self::BpImport {
                    serialized: rest[0].clone(),
                })
            }
            "wp_add" => {
                if rest.len() != 2 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected name + ops".into(),
                    });
                }
                Ok(Self::WpAdd {
                    name: rest[0].clone(),
                    ops: rest[1].clone(),
                })
            }
            "var_breakpoints" => Ok(Self::VarBreakpoints {
                name: rest.first().cloned(),
            }),
            "line_breakpoints" => Ok(Self::LineBreakpoints {
                qname: rest.first().cloned(),
                offset: rest
                    .get(1)
                    .map(|s| parse_uint(s, "line_breakpoints", "offset"))
                    .transpose()?,
            }),
            "coverage_enable" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected on|off".into(),
                    });
                }
                Ok(Self::CoverageEnable {
                    on: matches!(rest[0].as_str(), "on" | "1" | "true" | "yes"),
                })
            }
            "watch_add" => {
                if rest.len() != 2 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected name + expr".into(),
                    });
                }
                Ok(Self::WatchAdd {
                    name: rest[0].clone(),
                    expr: rest[1].clone(),
                })
            }
            "watch_remove" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected watch_id".into(),
                    });
                }
                Ok(Self::WatchRemove {
                    id: rest[0].clone(),
                })
            }
            "watch_evaluate" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected level".into(),
                    });
                }
                Ok(Self::WatchEvaluate {
                    level: parse_int(&rest[0], "watch_evaluate", "level")?,
                })
            }
            "pre_instrument_proc" => {
                if rest.len() != 2 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected qname + body".into(),
                    });
                }
                Ok(Self::PreInstrumentProc {
                    qname: rest[0].clone(),
                    body: rest[1].clone(),
                })
            }
            "pre_instrument_file" => {
                if rest.len() != 2 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected path + body".into(),
                    });
                }
                Ok(Self::PreInstrumentFile {
                    path: rest[0].clone(),
                    body: rest[1].clone(),
                })
            }
            "slave_install" => {
                if rest.len() != 1 {
                    return Err(OpParseError::BadArgs {
                        op: name,
                        reason: "expected name".into(),
                    });
                }
                Ok(Self::SlaveInstall {
                    name: rest[0].clone(),
                })
            }
            other => Err(OpParseError::UnknownOp(other.to_string())),
        }
    }
}

fn one_arg_qname(
    op: String,
    rest: &[String],
    construct: impl FnOnce(String) -> Op,
) -> Result<Op, OpParseError> {
    if rest.len() != 1 {
        return Err(OpParseError::BadArgs {
            op,
            reason: "expected qname".into(),
        });
    }
    Ok(construct(rest[0].clone()))
}

fn parse_breakpoint_target(
    op: &str,
    rest: &[String],
    allow_condition: bool,
) -> Result<(BreakpointTarget, Option<String>), OpParseError> {
    let parse_uint = |s: &str, field: &str| -> Result<u64, OpParseError> {
        s.parse::<u64>().map_err(|_| OpParseError::BadArgs {
            op: op.to_string(),
            reason: format!("{field} must be non-negative integer, got {s}"),
        })
    };
    let bad = |reason: &str| OpParseError::BadArgs {
        op: op.to_string(),
        reason: reason.to_string(),
    };
    match rest.first().map(String::as_str) {
        Some("source" | "file") => {
            let expected = if allow_condition { 4..=5 } else { 4..=4 };
            if !expected.contains(&rest.len()) {
                return Err(bad(if allow_condition {
                    "expected source + qname + offset + len + ?condition?"
                } else {
                    "expected source + qname + offset + len"
                }));
            }
            let target = if rest[0] == "file" {
                BreakpointTarget::File {
                    path: rest[1].clone(),
                    offset: parse_uint(&rest[2], "offset")?,
                    len: parse_uint(&rest[3], "len")?,
                }
            } else {
                BreakpointTarget::Source {
                    qname: rest[1].clone(),
                    offset: parse_uint(&rest[2], "offset")?,
                    len: parse_uint(&rest[3], "len")?,
                }
            };
            Ok((target, rest.get(4).cloned()))
        }
        Some("instruction") => {
            let expected = if allow_condition { 2..=3 } else { 2..=2 };
            if !expected.contains(&rest.len()) {
                return Err(bad(if allow_condition {
                    "expected instruction + pc + ?condition?"
                } else {
                    "expected instruction + pc"
                }));
            }
            Ok((
                BreakpointTarget::Instruction {
                    pc: parse_uint(&rest[1], "pc")?,
                },
                rest.get(2).cloned(),
            ))
        }
        Some("file_line") => {
            let expected = if allow_condition { 3..=4 } else { 3..=3 };
            if !expected.contains(&rest.len()) {
                return Err(bad(if allow_condition {
                    "expected file_line + path + line + ?condition?"
                } else {
                    "expected file_line + path + line"
                }));
            }
            let line = parse_uint(&rest[2], "line")?;
            if line == 0 {
                return Err(bad("file line must be one-based"));
            }
            Ok((
                BreakpointTarget::FileLine {
                    path: rest[1].clone(),
                    line,
                },
                rest.get(3).cloned(),
            ))
        }
        _ => {
            let expected = if allow_condition { 3..=4 } else { 3..=3 };
            if !expected.contains(&rest.len()) {
                return Err(bad(if allow_condition {
                    "expected source + qname + offset + len + ?condition?"
                } else {
                    "expected source + qname + offset + len"
                }));
            }
            Ok((
                BreakpointTarget::Source {
                    qname: rest[0].clone(),
                    offset: parse_uint(&rest[1], "offset")?,
                    len: parse_uint(&rest[2], "len")?,
                },
                rest.get(3).cloned(),
            ))
        }
    }
}

/// v2 response shape — opaque payload string. The wire keeps op
/// responses lossless; higher-level typed accessors for specific ops
/// can be layered on top later without changing the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpResponse {
    Payload(String),
}

impl OpResponse {
    pub fn from_payload(payload: &str) -> Self {
        Self::Payload(payload.to_string())
    }

    pub fn into_payload(self) -> String {
        match self {
            Self::Payload(s) => s,
        }
    }

    pub fn payload(&self) -> &str {
        match self {
            Self::Payload(s) => s,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(op: Op) {
        let payload = op.to_payload();
        let parsed = Op::from_payload(&payload).unwrap();
        assert_eq!(parsed, op, "round-trip failed for payload {payload}");
    }

    #[test]
    fn round_trip_no_arg_ops() {
        round_trip(Op::StepInto);
        round_trip(Op::StepOver);
        round_trip(Op::StepOut);
        round_trip(Op::StepInstruction);
        round_trip(Op::StepOverInstruction);
        round_trip(Op::StepOutInstruction);
        round_trip(Op::Continue);
        round_trip(Op::Interrupt);
        round_trip(Op::CurrentPc);
        round_trip(Op::AppState);
        round_trip(Op::InspectStack);
        round_trip(Op::GetLevel);
        round_trip(Op::BpList);
        round_trip(Op::BpExport);
        round_trip(Op::ProcList);
        round_trip(Op::Coverage);
        round_trip(Op::WatchList);
        round_trip(Op::SlaveList);
        round_trip(Op::IgnoreError);
        round_trip(Op::Quit);
    }

    #[test]
    fn round_trip_start_target() {
        let op = Op::StartTarget {
            path: "/tmp/target with spaces.tcl".into(),
            stop_on_entry: true,
        };
        round_trip(op.clone());
        assert_eq!(Op::from_payload(&op.to_payload()).unwrap(), op);
        assert!(matches!(
            Op::from_payload("start_target target.tcl maybe"),
            Err(OpParseError::BadArgs { .. })
        ));
    }

    #[test]
    fn round_trip_run_to_pc() {
        round_trip(Op::RunToPc { pc: 42 });
        assert_eq!(Op::RunToPc { pc: 42 }.to_payload(), "run_to_pc 42");
    }

    #[test]
    fn round_trip_eval_in_frame() {
        round_trip(Op::EvalInFrame {
            level: 2,
            script: "set x 1".into(),
        });
    }

    #[test]
    fn round_trip_get_result_with_and_without_maxlen() {
        round_trip(Op::GetResult { maxlen: None });
        round_trip(Op::GetResult { maxlen: Some(50) });
    }

    #[test]
    fn round_trip_bp_add_with_and_without_condition() {
        round_trip(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: 5,
                len: 1,
            },
            condition: None,
        });
        round_trip(Op::BpAdd {
            target: BreakpointTarget::Source {
                qname: "demo".into(),
                offset: 5,
                len: 1,
            },
            condition: Some("$a > 5".into()),
        });
        assert_eq!(
            Op::BpAdd {
                target: BreakpointTarget::Source {
                    qname: "demo".into(),
                    offset: 5,
                    len: 1,
                },
                condition: None,
            }
            .to_payload(),
            "bp_add source demo 5 1"
        );
    }

    #[test]
    fn round_trip_instruction_breakpoint_targets() {
        round_trip(Op::BpAdd {
            target: BreakpointTarget::Instruction { pc: 12 },
            condition: None,
        });
        round_trip(Op::BpAdd {
            target: BreakpointTarget::Instruction { pc: 12 },
            condition: Some("$flag".into()),
        });
        round_trip(Op::BpMove {
            id: "bp2".into(),
            target: BreakpointTarget::Instruction { pc: 16 },
        });
        round_trip(Op::InstructionBreakpoints { pc: None });
        round_trip(Op::InstructionBreakpoints { pc: Some(12) });
        assert_eq!(
            Op::BpAdd {
                target: BreakpointTarget::Instruction { pc: 12 },
                condition: None,
            }
            .to_payload(),
            "bp_add instruction 12"
        );
    }

    #[test]
    fn round_trip_file_line_breakpoint_targets() {
        round_trip(Op::BpAdd {
            target: BreakpointTarget::FileLine {
                path: "/tmp/target.tcl".into(),
                line: 12,
            },
            condition: Some("$ready".into()),
        });
        assert_eq!(
            Op::BpAdd {
                target: BreakpointTarget::FileLine {
                    path: "/tmp/target.tcl".into(),
                    line: 12,
                },
                condition: None,
            }
            .to_payload(),
            "bp_add file_line /tmp/target.tcl 12"
        );
        assert!(matches!(
            Op::from_payload("bp_add file_line /tmp/target.tcl 0"),
            Err(OpParseError::BadArgs { .. })
        ));
    }

    #[test]
    fn round_trip_file_command_breakpoint_targets() {
        round_trip(Op::BpAdd {
            target: BreakpointTarget::File {
                path: "/tmp/target.tcl".into(),
                offset: 18,
                len: 7,
            },
            condition: Some("$ready".into()),
        });
        assert_eq!(
            Op::BpAdd {
                target: BreakpointTarget::File {
                    path: "/tmp/target.tcl".into(),
                    offset: 18,
                    len: 7,
                },
                condition: None,
            }
            .to_payload(),
            "bp_add file /tmp/target.tcl 18 7"
        );
    }

    #[test]
    fn parses_legacy_source_breakpoint_payload() {
        assert_eq!(
            Op::from_payload("bp_add demo 5 1").unwrap(),
            Op::BpAdd {
                target: BreakpointTarget::Source {
                    qname: "demo".into(),
                    offset: 5,
                    len: 1,
                },
                condition: None,
            }
        );
    }

    #[test]
    fn round_trip_var_breakpoints_filter() {
        round_trip(Op::VarBreakpoints { name: None });
        round_trip(Op::VarBreakpoints {
            name: Some("foo".into()),
        });
    }

    #[test]
    fn round_trip_line_breakpoints_filter() {
        round_trip(Op::LineBreakpoints {
            qname: None,
            offset: None,
        });
        round_trip(Op::LineBreakpoints {
            qname: Some("alpha".into()),
            offset: None,
        });
        round_trip(Op::LineBreakpoints {
            qname: Some("alpha".into()),
            offset: Some(8),
        });
    }

    #[test]
    fn round_trip_coverage_enable() {
        round_trip(Op::CoverageEnable { on: true });
        round_trip(Op::CoverageEnable { on: false });
    }

    #[test]
    fn round_trip_pre_instrument_with_multiline_body() {
        round_trip(Op::PreInstrumentProc {
            qname: "demo".into(),
            body: "set a 1\nset b 2\nset c 3".into(),
        });
    }

    #[test]
    fn rejects_unknown_op() {
        let err = Op::from_payload("not_a_real_op").unwrap_err();
        assert!(matches!(err, OpParseError::UnknownOp(_)));
    }

    #[test]
    fn rejects_empty_payload() {
        let err = Op::from_payload("").unwrap_err();
        assert_eq!(err, OpParseError::Empty);
    }

    #[test]
    fn rejects_bad_int_arg() {
        let err = Op::from_payload("eval_in_frame notanint script").unwrap_err();
        assert!(matches!(err, OpParseError::BadArgs { .. }));
    }

    #[test]
    fn rejects_wrong_arity_bp_add() {
        let err = Op::from_payload("bp_add demo 5").unwrap_err();
        assert!(matches!(err, OpParseError::BadArgs { .. }));
    }
}
