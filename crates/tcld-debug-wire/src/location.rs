//! Structured source provenance shared by execution events and stack frames.

use crate::tcl_list::{tcl_list_format, tcl_list_parse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceBlockKind {
    File,
    Proc,
    Anonymous,
}

impl SourceBlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Proc => "proc",
            Self::Anonymous => "anonymous",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "file" => Self::File,
            "proc" => Self::Proc,
            "anonymous" => Self::Anonymous,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub block_id: String,
    pub block_kind: SourceBlockKind,
    pub source_path: Option<String>,
    pub qualified_proc: Option<String>,
    pub line: u32,
    pub offset_start: u64,
    pub offset_end: u64,
}

impl SourceLocation {
    pub fn to_payload(&self) -> String {
        let mut parts = vec![
            "block_id".to_string(),
            self.block_id.clone(),
            "block_kind".to_string(),
            self.block_kind.as_str().to_string(),
        ];
        if let Some(path) = &self.source_path {
            parts.push("source_path".to_string());
            parts.push(path.clone());
        }
        if let Some(qname) = &self.qualified_proc {
            parts.push("qualified_proc".to_string());
            parts.push(qname.clone());
        }
        parts.extend([
            "line".to_string(),
            self.line.to_string(),
            "offset_start".to_string(),
            self.offset_start.to_string(),
            "offset_end".to_string(),
            self.offset_end.to_string(),
        ]);
        tcl_list_format(parts.iter())
    }

    pub fn from_payload(payload: &str) -> Result<Self, SourceLocationParseError> {
        let parts = tcl_list_parse(payload).map_err(|err| {
            SourceLocationParseError::new("location", format!("invalid Tcl list: {err}"))
        })?;
        if !parts.len().is_multiple_of(2) {
            return Err(SourceLocationParseError::new(
                "location",
                "fields must be key/value pairs",
            ));
        }
        let dict: std::collections::BTreeMap<_, _> = parts
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();
        let required = |field: &'static str| {
            dict.get(field)
                .cloned()
                .ok_or_else(|| SourceLocationParseError::new(field, "required"))
        };
        let parse_u32 = |field: &'static str| -> Result<u32, SourceLocationParseError> {
            required(field)?.parse().map_err(|_| {
                SourceLocationParseError::new(field, "must be an unsigned 32-bit integer")
            })
        };
        let parse_u64 = |field: &'static str| -> Result<u64, SourceLocationParseError> {
            required(field)?.parse().map_err(|_| {
                SourceLocationParseError::new(field, "must be an unsigned 64-bit integer")
            })
        };
        let block_kind_value = required("block_kind")?;
        let block_kind = SourceBlockKind::parse(&block_kind_value).ok_or_else(|| {
            SourceLocationParseError::new("block_kind", "unknown source block kind")
        })?;
        let offset_start = parse_u64("offset_start")?;
        let offset_end = parse_u64("offset_end")?;
        if offset_end <= offset_start {
            return Err(SourceLocationParseError::new(
                "offset_end",
                "must be greater than offset_start",
            ));
        }
        Ok(Self {
            block_id: required("block_id")?,
            block_kind,
            source_path: dict.get("source_path").cloned(),
            qualified_proc: dict.get("qualified_proc").cloned(),
            line: parse_u32("line")?,
            offset_start,
            offset_end,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocationParseError {
    pub field: String,
    pub reason: String,
}

impl SourceLocationParseError {
    fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for SourceLocationParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad location field `{}`: {}", self.field, self.reason)
    }
}

impl std::error::Error for SourceLocationParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_file_location_with_spaces() {
        let location = SourceLocation {
            block_id: "file:/tmp/a file.tcl".into(),
            block_kind: SourceBlockKind::File,
            source_path: Some("/tmp/a file.tcl".into()),
            qualified_proc: None,
            line: 4,
            offset_start: 19,
            offset_end: 28,
        };
        assert_eq!(
            SourceLocation::from_payload(&location.to_payload()).unwrap(),
            location
        );
    }

    #[test]
    fn rejects_empty_range() {
        let err = SourceLocation::from_payload(
            "block_id proc:x block_kind proc qualified_proc x line 1 offset_start 5 offset_end 5",
        )
        .unwrap_err();
        assert_eq!(err.field, "offset_end");
    }
}
