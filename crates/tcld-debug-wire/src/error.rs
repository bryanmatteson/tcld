//! Wire error response shape.
//!
//! `Err` frames carry a closed-set [`ErrorCode`] plus a free-form
//! human-readable message. The code set is the v2 contract; new
//! codes require a v2 protocol bump.

use crate::tcl_list::{tcl_list_format, tcl_list_parse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    BadArgs,
    NotAttached,
    NotStopped,
    NotFound,
    Unimplemented,
    Internal,
    Disconnected,
    BadHandshake,
    ProtocolViolation,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BadArgs => "bad_args",
            Self::NotAttached => "not_attached",
            Self::NotStopped => "not_stopped",
            Self::NotFound => "not_found",
            Self::Unimplemented => "unimplemented",
            Self::Internal => "internal",
            Self::Disconnected => "disconnected",
            Self::BadHandshake => "bad_handshake",
            Self::ProtocolViolation => "protocol_violation",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bad_args" => Some(Self::BadArgs),
            "not_attached" => Some(Self::NotAttached),
            "not_stopped" => Some(Self::NotStopped),
            "not_found" => Some(Self::NotFound),
            "unimplemented" => Some(Self::Unimplemented),
            "internal" => Some(Self::Internal),
            "disconnected" => Some(Self::Disconnected),
            "bad_handshake" => Some(Self::BadHandshake),
            "protocol_violation" => Some(Self::ProtocolViolation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
}

impl WireError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Encode payload as `code <enum> message <text>`.
    pub fn to_payload(&self) -> String {
        tcl_list_format([
            "code".to_string(),
            self.code.as_str().to_string(),
            "message".to_string(),
            self.message.clone(),
        ])
    }

    pub fn from_payload(payload: &str) -> Result<Self, WireErrorParseError> {
        let parts = tcl_list_parse(payload).map_err(WireErrorParseError::TclList)?;
        if parts.len() % 2 != 0 {
            return Err(WireErrorParseError::OddPairs);
        }
        let mut code = None;
        let mut message = None;
        let mut iter = parts.into_iter();
        while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
            match k.as_str() {
                "code" => {
                    code = Some(ErrorCode::parse(&v).ok_or(WireErrorParseError::UnknownCode(v))?);
                }
                "message" => message = Some(v),
                _ => {} // forward-compat: ignore unknown keys
            }
        }
        Ok(Self {
            code: code.ok_or(WireErrorParseError::MissingCode)?,
            message: message.unwrap_or_default(),
        })
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for WireError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireErrorParseError {
    TclList(crate::tcl_list::TclListError),
    OddPairs,
    UnknownCode(String),
    MissingCode,
}

impl std::fmt::Display for WireErrorParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TclList(e) => write!(f, "tcl list error: {e}"),
            Self::OddPairs => write!(f, "error payload must be key/value pairs"),
            Self::UnknownCode(c) => write!(f, "unknown error code: {c}"),
            Self::MissingCode => write!(f, "error payload missing `code`"),
        }
    }
}

impl std::error::Error for WireErrorParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_every_code() {
        for code in [
            ErrorCode::BadArgs,
            ErrorCode::NotAttached,
            ErrorCode::NotStopped,
            ErrorCode::NotFound,
            ErrorCode::Unimplemented,
            ErrorCode::Internal,
            ErrorCode::Disconnected,
            ErrorCode::BadHandshake,
            ErrorCode::ProtocolViolation,
        ] {
            let original = WireError::new(code, "test");
            let parsed = WireError::from_payload(&original.to_payload()).unwrap();
            assert_eq!(parsed, original);
        }
    }

    #[test]
    fn unknown_code_rejected() {
        let err = WireError::from_payload("code totally_made_up message ignored").unwrap_err();
        assert!(matches!(err, WireErrorParseError::UnknownCode(_)));
    }

    #[test]
    fn missing_code_rejected() {
        let err = WireError::from_payload("message {only message}").unwrap_err();
        assert_eq!(err, WireErrorParseError::MissingCode);
    }

    #[test]
    fn ignores_unknown_keys_for_forward_compat() {
        let payload = "code internal message {it broke} extra_field {ignored}";
        let parsed = WireError::from_payload(payload).unwrap();
        assert_eq!(parsed.code, ErrorCode::Internal);
        assert_eq!(parsed.message, "it broke");
    }
}
