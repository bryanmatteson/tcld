//! Wire frame envelope.
//!
//! Every cross-process message is one of these. The frame's
//! `payload` is op-specific and parsed by the `op` / `event` /
//! `error` / `handshake` modules.

use crate::tcl_list::{TclListError, tcl_list_format, tcl_list_parse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// Request from controller to nub.
    Req,
    /// Successful response. Matches request `serial`.
    Resp,
    /// Async event from nub to controller (no response).
    Evt,
    /// Error response. Matches request `serial`.
    Err,
    /// Handshake hello (nub → controller, on connect).
    Hello,
    /// Handshake acknowledgement (controller → nub).
    HelloAck,
    /// Graceful shutdown notice from either side.
    Bye,
}

impl FrameKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Req => "req",
            Self::Resp => "resp",
            Self::Evt => "evt",
            Self::Err => "err",
            Self::Hello => "hello",
            Self::HelloAck => "hello_ack",
            Self::Bye => "bye",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "req" => Some(Self::Req),
            "resp" => Some(Self::Resp),
            "evt" => Some(Self::Evt),
            "err" => Some(Self::Err),
            "hello" => Some(Self::Hello),
            "hello_ack" => Some(Self::HelloAck),
            "bye" => Some(Self::Bye),
            _ => None,
        }
    }
}

/// One framed message. `payload` is the op-specific list (already
/// Tcl-list-encoded as a string from this layer's POV; downstream
/// parsers split it further).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFrame {
    pub kind: FrameKind,
    pub serial: u64,
    pub payload: String,
}

impl WireFrame {
    pub fn new(kind: FrameKind, serial: u64, payload: impl Into<String>) -> Self {
        Self {
            kind,
            serial,
            payload: payload.into(),
        }
    }

    /// Encode this frame into its on-wire Tcl-list form (without
    /// the length prefix — that's added by [`crate::codec::encode_frame`]).
    pub fn to_wire_body(&self) -> String {
        // Layout: `<kind> <serial> {<payload>}`. Brace-quoting
        // applies via tcl_list_format.
        tcl_list_format([
            self.kind.as_str().to_string(),
            self.serial.to_string(),
            self.payload.clone(),
        ])
    }

    pub fn from_wire_body(body: &str) -> Result<Self, FrameParseError> {
        let parts = tcl_list_parse(body).map_err(FrameParseError::TclList)?;
        if parts.len() != 3 {
            return Err(FrameParseError::WrongArity {
                expected: 3,
                got: parts.len(),
            });
        }
        let kind = FrameKind::parse(&parts[0])
            .ok_or_else(|| FrameParseError::UnknownKind(parts[0].clone()))?;
        let serial = parts[1]
            .parse::<u64>()
            .map_err(|_| FrameParseError::BadSerial(parts[1].clone()))?;
        let payload = parts[2].clone();
        Ok(Self {
            kind,
            serial,
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameParseError {
    TclList(TclListError),
    WrongArity { expected: usize, got: usize },
    UnknownKind(String),
    BadSerial(String),
}

impl std::fmt::Display for FrameParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TclList(e) => write!(f, "tcl list parse error: {e}"),
            Self::WrongArity { expected, got } => {
                write!(f, "frame arity mismatch: expected {expected}, got {got}")
            }
            Self::UnknownKind(k) => write!(f, "unknown frame kind: {k}"),
            Self::BadSerial(s) => write!(f, "bad serial: {s}"),
        }
    }
}

impl std::error::Error for FrameParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_req_frame() {
        let f = WireFrame::new(FrameKind::Req, 42, "step_into");
        let body = f.to_wire_body();
        let parsed = WireFrame::from_wire_body(&body).unwrap();
        assert_eq!(parsed, f);
    }

    #[test]
    fn round_trip_payload_with_whitespace() {
        let f = WireFrame::new(FrameKind::Req, 1, "bp_add demo 5 1 {$a > 0}");
        let body = f.to_wire_body();
        let parsed = WireFrame::from_wire_body(&body).unwrap();
        assert_eq!(parsed, f);
    }

    #[test]
    fn round_trip_every_kind() {
        for kind in [
            FrameKind::Req,
            FrameKind::Resp,
            FrameKind::Evt,
            FrameKind::Err,
            FrameKind::Hello,
            FrameKind::HelloAck,
            FrameKind::Bye,
        ] {
            let f = WireFrame::new(kind, 7, "payload");
            let body = f.to_wire_body();
            let parsed = WireFrame::from_wire_body(&body).unwrap();
            assert_eq!(parsed.kind, kind);
            assert_eq!(parsed.serial, 7);
            assert_eq!(parsed.payload, "payload");
        }
    }

    #[test]
    fn rejects_wrong_arity() {
        let err = WireFrame::from_wire_body("req 1").unwrap_err();
        assert!(matches!(err, FrameParseError::WrongArity { .. }));
    }

    #[test]
    fn rejects_unknown_kind() {
        let err = WireFrame::from_wire_body("notakind 1 payload").unwrap_err();
        assert!(matches!(err, FrameParseError::UnknownKind(_)));
    }

    #[test]
    fn rejects_bad_serial() {
        let err = WireFrame::from_wire_body("req notanint payload").unwrap_err();
        assert!(matches!(err, FrameParseError::BadSerial(_)));
    }

    #[test]
    fn rejects_malformed_braces() {
        let err = WireFrame::from_wire_body("{unterminated").unwrap_err();
        assert!(matches!(err, FrameParseError::TclList(_)));
    }
}
