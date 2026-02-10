//! Connection handshake.
//!
//! On TCP connect the nub sends [`Hello`]; the controller MUST reply
//! with [`HelloAck`] within 5 seconds (per the wire spec) or the
//! connection is torn down. Both messages carry a major
//! `protocol_version` — mismatch is a hard close (no further frames
//! exchanged).

use crate::tcl_list::{tcl_list_format, tcl_list_parse};

/// v2 protocol major version. Parsers MUST refuse a peer whose
/// major version differs.
pub const PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NubTclAbi {
    Tcl85,
    Tcl86,
    Tcl9,
}

impl NubTclAbi {
    fn from_capability(prefix: &str, capability: &str) -> Option<Self> {
        Some(match capability.strip_prefix(prefix)? {
            "tcl8_5" => Self::Tcl85,
            "tcl8_6" => Self::Tcl86,
            "tcl9" => Self::Tcl9,
            _ => return None,
        })
    }
}

impl std::fmt::Display for NubTclAbi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Tcl85 => "Tcl 8.5",
            Self::Tcl86 => "Tcl 8.6",
            Self::Tcl9 => "Tcl 9",
        })
    }
}

/// Validate the nub's compile-time ABI against the interpreter that loaded it.
///
/// A cross-ABI extension can complete the wire handshake because much of Tcl's
/// C API is source-compatible, then return corrupted trace/frame data. Current
/// controllers therefore require both capabilities and refuse to source a
/// target unless they identify the same ABI.
pub fn validate_nub_tcl_abi(capabilities: &[String]) -> Result<NubTclAbi, String> {
    let collect = |prefix: &str| {
        capabilities
            .iter()
            .filter_map(|capability| NubTclAbi::from_capability(prefix, capability))
            .collect::<Vec<_>>()
    };
    let compiled = collect("compiled_");
    let runtime = collect("runtime_");
    let [compiled] = compiled.as_slice() else {
        return Err(format!(
            "debugger nub handshake must report exactly one compiled Tcl ABI; capabilities: {}",
            capabilities.join(" ")
        ));
    };
    let [runtime] = runtime.as_slice() else {
        return Err(format!(
            "debugger nub handshake must report exactly one runtime Tcl ABI; capabilities: {}",
            capabilities.join(" ")
        ));
    };
    if compiled != runtime {
        return Err(format!(
            "debugger nub ABI mismatch: extension was compiled for {compiled}, but the host interpreter is {runtime}"
        ));
    }
    Ok(*compiled)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub nub_version: String,
    pub tcl_version: String,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
}

impl Hello {
    pub fn to_payload(&self) -> String {
        let cap_str = tcl_list_format(self.capabilities.iter());
        tcl_list_format([
            "nub_version".to_string(),
            self.nub_version.clone(),
            "tcl_version".to_string(),
            self.tcl_version.clone(),
            "protocol_version".to_string(),
            self.protocol_version.to_string(),
            "capabilities".to_string(),
            cap_str,
        ])
    }

    pub fn from_payload(payload: &str) -> Result<Self, HandshakeParseError> {
        let parts = tcl_list_parse(payload).map_err(HandshakeParseError::TclList)?;
        if parts.len() % 2 != 0 {
            return Err(HandshakeParseError::OddPairs);
        }
        let mut out = Hello {
            nub_version: String::new(),
            tcl_version: String::new(),
            protocol_version: 0,
            capabilities: Vec::new(),
        };
        let mut iter = parts.into_iter();
        while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
            match k.as_str() {
                "nub_version" => out.nub_version = v,
                "tcl_version" => out.tcl_version = v,
                "protocol_version" => {
                    out.protocol_version = v
                        .parse::<u32>()
                        .map_err(|_| HandshakeParseError::BadVersion(v))?;
                }
                "capabilities" => {
                    out.capabilities = tcl_list_parse(&v).map_err(HandshakeParseError::TclList)?;
                }
                _ => {} // forward-compat: ignore unknown fields
            }
        }
        if out.nub_version.is_empty() {
            return Err(HandshakeParseError::MissingField("nub_version"));
        }
        if out.protocol_version == 0 {
            return Err(HandshakeParseError::MissingField("protocol_version"));
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloAck {
    pub controller_version: String,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
}

impl HelloAck {
    pub fn to_payload(&self) -> String {
        let cap_str = tcl_list_format(self.capabilities.iter());
        tcl_list_format([
            "controller_version".to_string(),
            self.controller_version.clone(),
            "protocol_version".to_string(),
            self.protocol_version.to_string(),
            "capabilities".to_string(),
            cap_str,
        ])
    }

    pub fn from_payload(payload: &str) -> Result<Self, HandshakeParseError> {
        let parts = tcl_list_parse(payload).map_err(HandshakeParseError::TclList)?;
        if parts.len() % 2 != 0 {
            return Err(HandshakeParseError::OddPairs);
        }
        let mut controller_version = String::new();
        let mut protocol_version: u32 = 0;
        let mut capabilities: Vec<String> = Vec::new();
        let mut iter = parts.into_iter();
        while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
            match k.as_str() {
                "controller_version" => controller_version = v,
                "protocol_version" => {
                    protocol_version = v
                        .parse::<u32>()
                        .map_err(|_| HandshakeParseError::BadVersion(v))?;
                }
                "capabilities" => {
                    capabilities = tcl_list_parse(&v).map_err(HandshakeParseError::TclList)?;
                }
                _ => {}
            }
        }
        if controller_version.is_empty() {
            return Err(HandshakeParseError::MissingField("controller_version"));
        }
        if protocol_version == 0 {
            return Err(HandshakeParseError::MissingField("protocol_version"));
        }
        Ok(Self {
            controller_version,
            protocol_version,
            capabilities,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeParseError {
    TclList(crate::tcl_list::TclListError),
    OddPairs,
    BadVersion(String),
    MissingField(&'static str),
}

impl std::fmt::Display for HandshakeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TclList(e) => write!(f, "tcl list error: {e}"),
            Self::OddPairs => write!(f, "handshake payload must be key/value pairs"),
            Self::BadVersion(s) => write!(f, "bad version: {s}"),
            Self::MissingField(name) => write!(f, "handshake payload missing `{name}`"),
        }
    }
}

impl std::error::Error for HandshakeParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trip() {
        let h = Hello {
            nub_version: "0.1.0".into(),
            tcl_version: "9.0.3".into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec!["coverage".into(), "watches".into()],
        };
        let parsed = Hello::from_payload(&h.to_payload()).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn hello_round_trip_empty_capabilities() {
        let h = Hello {
            nub_version: "0.1.0".into(),
            tcl_version: "9.0.3".into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec![],
        };
        let parsed = Hello::from_payload(&h.to_payload()).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn hello_ack_round_trip() {
        let ack = HelloAck {
            controller_version: "0.1.0".into(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec!["coverage".into()],
        };
        let parsed = HelloAck::from_payload(&ack.to_payload()).unwrap();
        assert_eq!(parsed, ack);
    }

    #[test]
    fn hello_ignores_unknown_fields_for_forward_compat() {
        let payload = "nub_version 0.1.0 tcl_version 9.0.3 protocol_version 2 capabilities {} \
                       auth oauth2 compress zstd";
        let parsed = Hello::from_payload(payload).unwrap();
        assert_eq!(parsed.nub_version, "0.1.0");
        // Unknown fields silently ignored.
    }

    #[test]
    fn hello_missing_protocol_version_rejected() {
        let payload = "nub_version 0.1.0 tcl_version 9.0.3 capabilities {}";
        let err = Hello::from_payload(payload).unwrap_err();
        assert_eq!(err, HandshakeParseError::MissingField("protocol_version"));
    }

    #[test]
    fn hello_bad_version_rejected() {
        let payload =
            "nub_version 0.1.0 tcl_version 9.0.3 protocol_version notanint capabilities {}";
        let err = Hello::from_payload(payload).unwrap_err();
        assert!(matches!(err, HandshakeParseError::BadVersion(_)));
    }

    #[test]
    fn nub_tcl_abi_requires_matching_compiled_and_runtime_capabilities() {
        let capabilities = vec!["compiled_tcl8_6".into(), "runtime_tcl8_6".into()];
        assert_eq!(
            validate_nub_tcl_abi(&capabilities).unwrap(),
            NubTclAbi::Tcl86
        );

        let mismatch = vec!["compiled_tcl9".into(), "runtime_tcl8_6".into()];
        let error = validate_nub_tcl_abi(&mismatch).unwrap_err();
        assert!(error.contains("compiled for Tcl 9"), "{error}");
        assert!(error.contains("host interpreter is Tcl 8.6"), "{error}");
    }

    #[test]
    fn nub_tcl_abi_rejects_legacy_or_ambiguous_handshakes() {
        assert!(validate_nub_tcl_abi(&[]).unwrap_err().contains("compiled"));
        assert!(
            validate_nub_tcl_abi(&[
                "compiled_tcl8_6".into(),
                "compiled_tcl9".into(),
                "runtime_tcl8_6".into(),
            ])
            .unwrap_err()
            .contains("exactly one compiled")
        );
    }
}
