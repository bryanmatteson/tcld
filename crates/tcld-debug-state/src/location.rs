//! TclPro's `{block line range}` triple, in Rust.
//!
//! Every breakpoint, every paused PC, every event payload uses
//! [`Location`]. Constructed only through [`Location::new`] which enforces
//! `byte_range.end > byte_range.start` — locations never represent zero-byte
//! spans. The empty-range case shows up as constructor failure rather than
//! as a sentinel value because every consumer (highlighter, gutter,
//! breakpoint targeting) needs a real range.

use crate::block::BlockId;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    block: BlockId,
    line: usize,
    byte_range: Range<usize>,
}

impl Location {
    /// Build a location, rejecting empty `byte_range`. The line is treated
    /// as a hint only — the canonical position is `byte_range.start`.
    pub fn new(
        block: BlockId,
        line: usize,
        byte_range: Range<usize>,
    ) -> Result<Self, LocationError> {
        if byte_range.end <= byte_range.start {
            return Err(LocationError::EmptyByteRange { byte_range });
        }
        Ok(Self {
            block,
            line,
            byte_range,
        })
    }

    pub fn block(&self) -> &BlockId {
        &self.block
    }

    pub fn line(&self) -> usize {
        self.line
    }

    pub fn byte_range(&self) -> Range<usize> {
        self.byte_range.clone()
    }

    pub fn byte_offset(&self) -> usize {
        self.byte_range.start
    }

    pub fn byte_len(&self) -> usize {
        self.byte_range.end - self.byte_range.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationError {
    EmptyByteRange { byte_range: Range<usize> },
}

impl std::fmt::Display for LocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyByteRange { byte_range } => write!(
                f,
                "Location byte_range must be non-empty (got {byte_range:?})"
            ),
        }
    }
}

impl std::error::Error for LocationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn proc_id(name: &str) -> BlockId {
        BlockId::ProcBody(Arc::from(name))
    }

    #[test]
    fn location_constructor_rejects_empty_range() {
        let err = Location::new(proc_id("p"), 0, 5..5).unwrap_err();
        assert!(matches!(err, LocationError::EmptyByteRange { .. }));
    }

    #[test]
    // The inverted range is the invalid fixture under test: Location::new must
    // reject it, so constructing it here is the point.
    #[allow(clippy::reversed_empty_ranges)]
    fn location_constructor_rejects_inverted_range() {
        let err = Location::new(proc_id("p"), 0, 10..5).unwrap_err();
        assert!(matches!(err, LocationError::EmptyByteRange { .. }));
    }

    #[test]
    fn location_constructor_accepts_nonempty_range() {
        let loc = Location::new(proc_id("p"), 3, 10..20).unwrap();
        assert_eq!(loc.line(), 3);
        assert_eq!(loc.byte_offset(), 10);
        assert_eq!(loc.byte_len(), 10);
        assert_eq!(loc.byte_range(), 10..20);
    }
}
