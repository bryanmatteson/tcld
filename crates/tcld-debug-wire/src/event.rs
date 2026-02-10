//! Event vocabulary (nub → controller, fire-and-forget).
//!
//! Mirrors `tcld_debug_state::DebugEvent` field-for-field, with two
//! additions for connection lifecycle (`NubStarted` / `NubStopped`)
//! that don't exist in the in-process model because they're
//! meaningful only across a wire.

use crate::{
    location::SourceLocation,
    tcl_list::{tcl_list_format, tcl_list_parse},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Entry,
    Step,
    Breakpoint,
    Watchpoint,
    Pause,
    InstructionStep,
    InstructionBreakpoint,
}

impl PauseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Step => "step",
            Self::Breakpoint => "breakpoint",
            Self::Watchpoint => "watchpoint",
            Self::Pause => "pause",
            Self::InstructionStep => "instruction_step",
            Self::InstructionBreakpoint => "instruction_breakpoint",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "entry" => Self::Entry,
            "step" => Self::Step,
            "breakpoint" => Self::Breakpoint,
            "watchpoint" => Self::Watchpoint,
            "pause" => Self::Pause,
            "instruction_step" => Self::InstructionStep,
            "instruction_breakpoint" => Self::InstructionBreakpoint,
            _ => return None,
        })
    }
}

/// One event payload. The first element of the wire payload is the
/// event kind string; the remaining elements are k/v pairs whose
/// shape depends on the kind. The v2 schema is closed; new event kinds
/// require a protocol-major bump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireEvent {
    TargetStarted {
        path: String,
        status: String,
    },
    TargetCompleted {
        path: String,
        status: String,
        result: String,
    },
    TargetLoadFailed {
        path: String,
        status: String,
        message: String,
        error_code: String,
        error_info: String,
    },
    Paused {
        location: SourceLocation,
        frame_count: u32,
        breakpoint: Option<String>, // "bp<N>" or absent
        reason: PauseReason,
        pc: Option<u64>,
    },
    Resumed,
    ErrorPaused {
        location: Option<SourceLocation>,
        message: String,
        error_code: Option<String>,
        breakpoint: Option<String>,
    },
    ConditionError {
        breakpoint: String,
        message: String,
    },
    BlocksRevalidated {
        block: String, // "proc:foo" | "file:/path" | "anon:HEX"
    },
    NubStarted {
        pid: u32,
        version: String,
        tcl_version: String,
    },
    NubStopped {
        reason: String, // "killed" | "quit" | "socket_closed" | ...
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventParseError {
    TclList(crate::tcl_list::TclListError),
    Empty,
    UnknownKind(String),
    BadField {
        kind: String,
        field: String,
        reason: String,
    },
}

impl std::fmt::Display for EventParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TclList(e) => write!(f, "tcl list error: {e}"),
            Self::Empty => write!(f, "empty event payload"),
            Self::UnknownKind(s) => write!(f, "unknown event kind: {s}"),
            Self::BadField {
                kind,
                field,
                reason,
            } => {
                write!(f, "bad field `{field}` in event `{kind}`: {reason}")
            }
        }
    }
}

impl std::error::Error for EventParseError {}

impl WireEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TargetStarted { .. } => "target_started",
            Self::TargetCompleted { .. } => "target_completed",
            Self::TargetLoadFailed { .. } => "target_load_failed",
            Self::Paused { .. } => "paused",
            Self::Resumed => "resumed",
            Self::ErrorPaused { .. } => "error_paused",
            Self::ConditionError { .. } => "condition_error",
            Self::BlocksRevalidated { .. } => "blocks_revalidated",
            Self::NubStarted { .. } => "nub_started",
            Self::NubStopped { .. } => "nub_stopped",
        }
    }

    pub fn to_payload(&self) -> String {
        let mut parts: Vec<String> = vec![self.kind().to_string()];
        match self {
            Self::TargetStarted { path, status } => {
                push_kv(&mut parts, "path", path.clone());
                push_kv(&mut parts, "status", status.clone());
            }
            Self::TargetCompleted {
                path,
                status,
                result,
            } => {
                push_kv(&mut parts, "path", path.clone());
                push_kv(&mut parts, "status", status.clone());
                push_kv(&mut parts, "result", result.clone());
            }
            Self::TargetLoadFailed {
                path,
                status,
                message,
                error_code,
                error_info,
            } => {
                push_kv(&mut parts, "path", path.clone());
                push_kv(&mut parts, "status", status.clone());
                push_kv(&mut parts, "message", message.clone());
                push_kv(&mut parts, "error_code", error_code.clone());
                push_kv(&mut parts, "error_info", error_info.clone());
            }
            Self::Paused {
                location,
                frame_count,
                breakpoint,
                reason,
                pc,
            } => {
                push_kv(&mut parts, "location", location.to_payload());
                push_kv(&mut parts, "frame_count", frame_count.to_string());
                push_kv(&mut parts, "reason", reason.as_str().to_string());
                if let Some(b) = breakpoint {
                    push_kv(&mut parts, "breakpoint", b.clone());
                }
                if let Some(pc) = pc {
                    push_kv(&mut parts, "pc", pc.to_string());
                }
            }
            Self::Resumed => {}
            Self::ErrorPaused {
                location,
                message,
                error_code,
                breakpoint,
            } => {
                if let Some(loc) = location {
                    push_kv(&mut parts, "location", loc.to_payload());
                }
                push_kv(&mut parts, "message", message.clone());
                if let Some(c) = error_code {
                    push_kv(&mut parts, "error_code", c.clone());
                }
                if let Some(b) = breakpoint {
                    push_kv(&mut parts, "breakpoint", b.clone());
                }
            }
            Self::ConditionError {
                breakpoint,
                message,
            } => {
                push_kv(&mut parts, "breakpoint", breakpoint.clone());
                push_kv(&mut parts, "message", message.clone());
            }
            Self::BlocksRevalidated { block } => {
                push_kv(&mut parts, "block", block.clone());
            }
            Self::NubStarted {
                pid,
                version,
                tcl_version,
            } => {
                push_kv(&mut parts, "pid", pid.to_string());
                push_kv(&mut parts, "version", version.clone());
                push_kv(&mut parts, "tcl_version", tcl_version.clone());
            }
            Self::NubStopped { reason } => {
                push_kv(&mut parts, "reason", reason.clone());
            }
        }
        tcl_list_format(parts.iter())
    }

    pub fn from_payload(payload: &str) -> Result<Self, EventParseError> {
        let parts = tcl_list_parse(payload).map_err(EventParseError::TclList)?;
        let mut iter = parts.into_iter();
        let kind = iter.next().ok_or(EventParseError::Empty)?;
        let kvs: Vec<String> = iter.collect();
        if !kvs.len().is_multiple_of(2) {
            return Err(EventParseError::BadField {
                kind: kind.clone(),
                field: "(payload)".into(),
                reason: "k/v pairs must be even".into(),
            });
        }
        let dict = pairs_to_dict(kvs);
        match kind.as_str() {
            "target_started" => Ok(Self::TargetStarted {
                path: take_required(&dict, &kind, "path")?,
                status: take_required(&dict, &kind, "status")?,
            }),
            "target_completed" => Ok(Self::TargetCompleted {
                path: take_required(&dict, &kind, "path")?,
                status: take_required(&dict, &kind, "status")?,
                result: take_required(&dict, &kind, "result")?,
            }),
            "target_load_failed" => Ok(Self::TargetLoadFailed {
                path: take_required(&dict, &kind, "path")?,
                status: take_required(&dict, &kind, "status")?,
                message: take_required(&dict, &kind, "message")?,
                error_code: take_required(&dict, &kind, "error_code")?,
                error_info: take_required(&dict, &kind, "error_info")?,
            }),
            "paused" => Ok(Self::Paused {
                location: SourceLocation::from_payload(&take_required(&dict, &kind, "location")?)
                    .map_err(|err| EventParseError::BadField {
                    kind: kind.clone(),
                    field: format!("location.{}", err.field),
                    reason: err.reason,
                })?,
                frame_count: take_required(&dict, &kind, "frame_count")?
                    .parse()
                    .map_err(|_| EventParseError::BadField {
                        kind: kind.clone(),
                        field: "frame_count".into(),
                        reason: "must be u32".into(),
                    })?,
                breakpoint: dict.get("breakpoint").cloned(),
                reason: dict
                    .get("reason")
                    .map(|s| {
                        PauseReason::parse(s).ok_or_else(|| EventParseError::BadField {
                            kind: kind.clone(),
                            field: "reason".into(),
                            reason: "unknown pause reason".into(),
                        })
                    })
                    .transpose()?
                    .unwrap_or(PauseReason::Step),
                pc: dict
                    .get("pc")
                    .map(|s| {
                        s.parse::<u64>().map_err(|_| EventParseError::BadField {
                            kind: kind.clone(),
                            field: "pc".into(),
                            reason: "must be u64".into(),
                        })
                    })
                    .transpose()?,
            }),
            "resumed" => Ok(Self::Resumed),
            "error_paused" => Ok(Self::ErrorPaused {
                location: dict
                    .get("location")
                    .map(|payload| {
                        SourceLocation::from_payload(payload).map_err(|err| {
                            EventParseError::BadField {
                                kind: kind.clone(),
                                field: format!("location.{}", err.field),
                                reason: err.reason,
                            }
                        })
                    })
                    .transpose()?,
                message: take_required(&dict, &kind, "message")?,
                error_code: dict.get("error_code").cloned(),
                breakpoint: dict.get("breakpoint").cloned(),
            }),
            "condition_error" => Ok(Self::ConditionError {
                breakpoint: take_required(&dict, &kind, "breakpoint")?,
                message: take_required(&dict, &kind, "message")?,
            }),
            "blocks_revalidated" => Ok(Self::BlocksRevalidated {
                block: take_required(&dict, &kind, "block")?,
            }),
            "nub_started" => Ok(Self::NubStarted {
                pid: take_required(&dict, &kind, "pid")?.parse().map_err(|_| {
                    EventParseError::BadField {
                        kind: kind.clone(),
                        field: "pid".into(),
                        reason: "must be u32".into(),
                    }
                })?,
                version: take_required(&dict, &kind, "version")?,
                tcl_version: take_required(&dict, &kind, "tcl_version")?,
            }),
            "nub_stopped" => Ok(Self::NubStopped {
                reason: take_required(&dict, &kind, "reason")?,
            }),
            _ => Err(EventParseError::UnknownKind(kind)),
        }
    }
}

fn push_kv(parts: &mut Vec<String>, k: &str, v: String) {
    parts.push(k.to_string());
    parts.push(v);
}

fn pairs_to_dict(kvs: Vec<String>) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let mut iter = kvs.into_iter();
    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
        map.insert(k, v);
    }
    map
}

fn take_required(
    dict: &std::collections::BTreeMap<String, String>,
    kind: &str,
    field: &'static str,
) -> Result<String, EventParseError> {
    dict.get(field)
        .cloned()
        .ok_or_else(|| EventParseError::BadField {
            kind: kind.to_string(),
            field: field.to_string(),
            reason: "required".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceBlockKind;

    fn round_trip(e: WireEvent) {
        let parsed = WireEvent::from_payload(&e.to_payload()).unwrap();
        assert_eq!(parsed, e);
    }

    fn proc_location(start: u64, end: u64) -> SourceLocation {
        SourceLocation {
            block_id: "proc:demo".into(),
            block_kind: SourceBlockKind::Proc,
            source_path: Some("/tmp/demo.tcl".into()),
            qualified_proc: Some("demo".into()),
            line: 1,
            offset_start: start,
            offset_end: end,
        }
    }

    #[test]
    fn round_trip_paused_with_breakpoint() {
        round_trip(WireEvent::Paused {
            location: proc_location(5, 6),
            frame_count: 1,
            breakpoint: Some("bp0".into()),
            reason: PauseReason::Breakpoint,
            pc: None,
        });
    }

    #[test]
    fn round_trip_paused_without_breakpoint() {
        round_trip(WireEvent::Paused {
            location: proc_location(0, 1),
            frame_count: 0,
            breakpoint: None,
            reason: PauseReason::Step,
            pc: None,
        });
    }

    #[test]
    fn round_trip_paused_with_instruction_pc() {
        let event = WireEvent::Paused {
            location: proc_location(0, 1),
            frame_count: 0,
            breakpoint: None,
            reason: PauseReason::InstructionStep,
            pc: Some(12),
        };
        let payload = event.to_payload();
        assert!(payload.contains("reason instruction_step"));
        assert!(payload.contains("pc 12"));
        round_trip(event);
        round_trip(WireEvent::Paused {
            location: proc_location(0, 1),
            frame_count: 0,
            breakpoint: Some("bp3".into()),
            reason: PauseReason::InstructionBreakpoint,
            pc: Some(12),
        });
    }

    #[test]
    fn round_trip_resumed() {
        round_trip(WireEvent::Resumed);
    }

    #[test]
    fn round_trip_target_lifecycle() {
        round_trip(WireEvent::TargetStarted {
            path: "/tmp/target.tcl".into(),
            status: "running".into(),
        });
        round_trip(WireEvent::TargetCompleted {
            path: "/tmp/target.tcl".into(),
            status: "completed".into(),
            result: "42".into(),
        });
        round_trip(WireEvent::TargetLoadFailed {
            path: "/tmp/target.tcl".into(),
            status: "load_failed".into(),
            message: "missing close-brace".into(),
            error_code: "NONE".into(),
            error_info: "while sourcing target".into(),
        });
    }

    #[test]
    fn round_trip_error_paused_full() {
        round_trip(WireEvent::ErrorPaused {
            location: Some(proc_location(0, 1)),
            message: "bad thing happened".into(),
            error_code: Some("ARITH DIVZERO".into()),
            breakpoint: Some("bp1".into()),
        });
    }

    #[test]
    fn round_trip_error_paused_minimal() {
        round_trip(WireEvent::ErrorPaused {
            location: None,
            message: "anonymous".into(),
            error_code: None,
            breakpoint: None,
        });
    }

    #[test]
    fn round_trip_condition_error() {
        round_trip(WireEvent::ConditionError {
            breakpoint: "bp0".into(),
            message: "syntax error in {$x > }".into(),
        });
    }

    #[test]
    fn round_trip_blocks_revalidated() {
        round_trip(WireEvent::BlocksRevalidated {
            block: "proc:foo::bar".into(),
        });
    }

    #[test]
    fn round_trip_nub_started_and_stopped() {
        round_trip(WireEvent::NubStarted {
            pid: 12345,
            version: "0.1.0".into(),
            tcl_version: "9.0.3".into(),
        });
        round_trip(WireEvent::NubStopped {
            reason: "killed".into(),
        });
    }

    #[test]
    fn rejects_unknown_kind() {
        let err = WireEvent::from_payload("not_a_real_event_kind").unwrap_err();
        assert!(matches!(err, EventParseError::UnknownKind(_)));
    }

    #[test]
    fn rejects_missing_required_field() {
        // paused without location.
        let err = WireEvent::from_payload("paused frame_count 0").unwrap_err();
        assert!(matches!(err, EventParseError::BadField { .. }));
    }

    #[test]
    fn ignores_unknown_optional_keys_for_forward_compat() {
        let payload = "paused location {block_id proc:x block_kind proc qualified_proc x line 1 offset_start 0 offset_end 1} \
                       frame_count 0 future_extension {ignored}";
        let parsed = WireEvent::from_payload(payload).unwrap();
        assert!(matches!(
            parsed,
            WireEvent::Paused {
                reason: PauseReason::Step,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_pause_reason() {
        let payload = "paused location {block_id proc:x block_kind proc qualified_proc x line 1 offset_start 0 offset_end 1} \
                       frame_count 0 reason mystery";
        let err = WireEvent::from_payload(payload).unwrap_err();
        assert!(matches!(err, EventParseError::BadField { .. }));
    }
}
