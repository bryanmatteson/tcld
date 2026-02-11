//! Per-command execution coverage.
//!
//! [`CoverageMap`] is keyed by `(BlockId, body_byte_offset)` — the same
//! key shape the trace handler uses for breakpoint lookup. Stays
//! separate from `BreakpointStore.hit_count` (which is per-breakpoint,
//! not per-location) so coverage data persists across breakpoint
//! lifecycles.

use crate::block::BlockId;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct CoverageMap {
    counts: HashMap<(BlockId, usize), u64>,
}

impl CoverageMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, block: &BlockId, body_byte_offset: usize) {
        *self
            .counts
            .entry((block.clone(), body_byte_offset))
            .or_insert(0) += 1;
    }

    pub fn count(&self, block: &BlockId, body_byte_offset: usize) -> u64 {
        self.counts
            .get(&(block.clone(), body_byte_offset))
            .copied()
            .unwrap_or(0)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&BlockId, usize, u64)> {
        self.counts
            .iter()
            .map(|((block, off), n)| (block, *off, *n))
    }

    pub fn len(&self) -> usize {
        self.counts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    pub fn clear(&mut self) {
        self.counts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn record_and_count() {
        let mut map = CoverageMap::new();
        let block = BlockId::ProcBody(Arc::from("p"));
        map.record(&block, 0);
        map.record(&block, 0);
        map.record(&block, 8);
        assert_eq!(map.count(&block, 0), 2);
        assert_eq!(map.count(&block, 8), 1);
        assert_eq!(map.count(&block, 99), 0);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn clear_resets() {
        let mut map = CoverageMap::new();
        let block = BlockId::ProcBody(Arc::from("p"));
        map.record(&block, 0);
        map.clear();
        assert!(map.is_empty());
    }
}
