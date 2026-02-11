//! Stable container for executable code.
//!
//! A [`Block`] is one unit of source the runtime can execute and the
//! debugger can attach breakpoints to. Three flavors:
//!
//! * **File**: a Tcl file the interpreter `source`d.
//! * **ProcBody**: the body source of a `proc` / `tbcload::bcproc`. Keyed
//!   by the proc's qualified name — exactly the `proc_qname` that
//!   [`tcld_decompile::decompiler::CommandIdentity::Synthetic`] already
//!   carries.
//! * **Anonymous**: a dynamically-evaluated body (`eval`, `apply`,
//!   `namespace eval` argument) with no stable name. Keyed by content hash
//!   so re-evals of the same script collide intentionally — coverage and
//!   breakpoints on dynamic bodies want that.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

/// Stable identity for one [`Block`].
///
/// Implements `Ord` so callers can use `BTreeMap<BlockId, _>` as the
/// canonical storage; the ordering itself is not semantically meaningful.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockId {
    File(Arc<str>),
    ProcBody(Arc<str>),
    Anonymous { content_hash: u64 },
}

impl BlockId {
    /// Construct an `Anonymous` block id by hashing the source bytes.
    pub fn anonymous_for(source: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        Self::Anonymous {
            content_hash: hasher.finish(),
        }
    }

    /// String form suitable for serialization / debug logs.
    pub fn as_string(&self) -> String {
        match self {
            Self::File(path) => format!("file:{path}"),
            Self::ProcBody(name) => format!("proc:{name}"),
            Self::Anonymous { content_hash } => format!("anon:{content_hash:016x}"),
        }
    }
}

/// One unit of executable source.
///
/// Construction is restricted: every public constructor enforces that
/// `valid_offsets` is sorted, deduplicated, and within `source.len()`.
/// Without these invariants `validate_against` and the trace handler can't
/// rely on binary search.
#[derive(Debug, Clone)]
pub struct Block {
    id: BlockId,
    source: Arc<str>,
    /// Monotonic. Bumped each time the block's source changes. Breakpoints
    /// are keyed by `(BlockId, body_byte_offset)`; on version change the
    /// store calls `validate_against` to snap or disable stale entries.
    version: u64,
    /// Sorted, deduplicated list of byte offsets where a breakpoint may be
    /// set. Each offset is the start of a parsed Tcl command in the block's
    /// source.
    valid_offsets: Vec<usize>,
    /// Maps a 1-indexed body line number to the byte offset of the FIRST
    /// command starting on that line. Used by the trace handler to
    /// translate Tcl's `info frame` `line` field — which reports the
    /// line of the executing command within the proc body — into a body
    /// byte offset that aligns with [`valid_offsets`]. Without this map
    /// the line→offset projection collapses every body command onto a
    /// single offset; with it, breakpoints set at parsed offsets fire
    /// at exactly the matching command.
    line_to_offset: Vec<(u32, usize)>,
}

impl Block {
    /// Build a block, enforcing the sorted/in-range invariants. Returns
    /// `Err(BlockError::OffsetsNotSorted)` or `Err(BlockError::OffsetOutOfRange)`
    /// when violated, rather than silently sorting — the registry needs to
    /// surface bad data, not paper over it.
    pub fn new(
        id: BlockId,
        source: Arc<str>,
        valid_offsets: Vec<usize>,
    ) -> Result<Self, BlockError> {
        Self::with_version(id, source, valid_offsets, 0)
    }

    /// Same as [`Block::new`] but pins the version. Used by the registry
    /// when a re-source preserves identity but increments the version.
    pub fn with_version(
        id: BlockId,
        source: Arc<str>,
        valid_offsets: Vec<usize>,
        version: u64,
    ) -> Result<Self, BlockError> {
        for window in valid_offsets.windows(2) {
            if window[0] >= window[1] {
                return Err(BlockError::OffsetsNotSorted);
            }
        }
        if let Some(&last) = valid_offsets.last()
            && last > source.len()
        {
            return Err(BlockError::OffsetOutOfRange {
                offset: last,
                source_len: source.len(),
            });
        }
        let line_to_offset = build_line_to_offset(&source, &valid_offsets);
        Ok(Self {
            id,
            source,
            version,
            valid_offsets,
            line_to_offset,
        })
    }

    pub fn id(&self) -> &BlockId {
        &self.id
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn source_arc(&self) -> Arc<str> {
        self.source.clone()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn valid_offsets(&self) -> &[usize] {
        &self.valid_offsets
    }

    /// True iff a breakpoint at `body_byte_offset` would land on a parsed
    /// command boundary.
    pub fn is_valid_offset(&self, body_byte_offset: usize) -> bool {
        self.valid_offsets.binary_search(&body_byte_offset).is_ok()
    }

    /// Smallest valid offset `>= body_byte_offset`, if any. Used by
    /// [`crate::BreakpointStore::validate_against`] to snap stale
    /// breakpoints.
    pub fn snap_to_nearest(&self, body_byte_offset: usize) -> Option<usize> {
        match self.valid_offsets.binary_search(&body_byte_offset) {
            Ok(_) => Some(body_byte_offset),
            Err(idx) => self.valid_offsets.get(idx).copied(),
        }
    }

    /// Translate a 1-indexed body line number (as Tcl's `info frame`
    /// reports it) into a body byte offset that aligns with
    /// [`valid_offsets`]. Returns `None` if no command starts on that
    /// line. The trace handler calls this for every traced command to
    /// locate the canonical breakpoint key.
    pub fn offset_for_line(&self, line: u32) -> Option<usize> {
        self.line_to_offset
            .iter()
            .find(|(l, _)| *l == line)
            .map(|(_, off)| *off)
    }

    /// Return the source line containing a registered command offset.
    ///
    /// Multiple commands may share a line, while `line_to_offset` retains
    /// only the first command on each line. Choosing the nearest preceding
    /// entry locates every valid offset without inventing line 0.
    pub fn line_for_offset(&self, offset: usize) -> Option<u32> {
        self.line_to_offset
            .iter()
            .filter(|(_, candidate)| *candidate <= offset)
            .max_by_key(|(_, candidate)| *candidate)
            .map(|(line, _)| *line)
    }

    /// Find the first valid_offset whose source slice begins with the
    /// given command text. Used by the trace handler to resolve a
    /// `[info frame].cmd` value into a body byte offset when the
    /// info-frame `line` field is too coarse (Tcl 9 reports a single
    /// `line` for every command in an `eval`-loaded proc body, so
    /// line-based resolution alone fails).
    ///
    /// `hint_offset` may be supplied to bias the search — when matching
    /// commands inside a loop body the handler can pass the previous
    /// matched offset so the next match comes from the same iteration.
    /// Currently the lookup is "first match at-or-after hint", with a
    /// global fallback if hint exceeds source length. Identical command
    /// text at multiple offsets still resolves to the first match
    /// from the hint position; documented limitation, mitigated by the
    /// observation that most proc bodies don't repeat literal commands.
    pub fn offset_for_cmd(&self, cmd: &str, hint_offset: usize) -> Option<usize> {
        let needle = cmd.trim();
        if needle.is_empty() {
            return None;
        }
        let starting = self.valid_offsets.partition_point(|&off| off < hint_offset);
        for &offset in self.valid_offsets[starting..]
            .iter()
            .chain(self.valid_offsets[..starting].iter())
        {
            if let Some(slice) = self.source.get(offset..)
                && slice.starts_with(needle)
            {
                return Some(offset);
            }
        }
        None
    }

    /// Read access to the line→offset table. Mainly for diagnostics and
    /// tests; the runtime should call [`offset_for_line`].
    pub fn line_to_offset(&self) -> &[(u32, usize)] {
        &self.line_to_offset
    }
}

fn build_line_to_offset(source: &str, valid_offsets: &[usize]) -> Vec<(u32, usize)> {
    // Walk the source once, accumulating (line, byte) of each newline
    // start. For each valid offset, find the line that contains it and
    // record (line, offset). Lines are 1-indexed to match Tcl's
    // `info frame` convention.
    if valid_offsets.is_empty() || source.is_empty() {
        return Vec::new();
    }
    // Collect line-start byte positions. Lines are 1-indexed: line 1
    // starts at byte 0; line 2 starts after the first '\n', etc.
    let mut line_starts = vec![0usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let line_for_offset = |off: usize| -> u32 {
        let idx = line_starts.partition_point(|&start| start <= off);
        // partition_point returns the first index where the predicate
        // is false; that's the line AFTER ours, so subtract 1 and add
        // 1 to convert to 1-indexed → idx itself.
        idx as u32
    };
    let mut by_line: Vec<(u32, usize)> = Vec::with_capacity(valid_offsets.len());
    let mut seen_line = 0u32;
    for &off in valid_offsets {
        let line = line_for_offset(off);
        // Keep only the FIRST offset per line so a single Tcl `line`
        // value resolves deterministically. Subsequent commands on the
        // same line aren't individually addressable via `info frame`
        // alone — the breakpoint UI would need source-character info
        // for that, which is out of scope here.
        if line != seen_line {
            by_line.push((line, off));
            seen_line = line;
        }
    }
    by_line
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    OffsetsNotSorted,
    OffsetOutOfRange { offset: usize, source_len: usize },
    AlreadyRegisteredWithDifferentSource,
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OffsetsNotSorted => {
                write!(f, "valid_offsets must be strictly increasing and unique")
            }
            Self::OffsetOutOfRange { offset, source_len } => write!(
                f,
                "valid_offset {offset} is past end of source (len {source_len})"
            ),
            Self::AlreadyRegisteredWithDifferentSource => write!(
                f,
                "block already registered with different source; call register_replacement to bump version"
            ),
        }
    }
}

impl std::error::Error for BlockError {}

/// Owner of every known [`Block`]. Concurrent-safe access expected via
/// `Arc<Mutex<BlockRegistry>>` from the embedding host.
#[derive(Debug, Default, Clone)]
pub struct BlockRegistry {
    blocks: BTreeMap<BlockId, Block>,
}

impl BlockRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Canonical entry point. Three intended call sites: (a)
    /// `AnalysisSnapshot` population at decompile time for known proc
    /// bodies, (b) future wrapper commands at `source`/`proc` time, (c)
    /// explicit Tcl command from the user. Even though (b) is gated
    /// runtime-side, this signature is committed format.
    pub fn register(
        &mut self,
        id: BlockId,
        source: Arc<str>,
        valid_offsets: Vec<usize>,
    ) -> Result<&Block, BlockError> {
        if let Some(existing) = self.blocks.get(&id) {
            if Arc::ptr_eq(&existing.source, &source) || existing.source.as_ref() == source.as_ref()
            {
                // Idempotent re-registration of identical content.
                return Ok(self.blocks.get(&id).unwrap());
            }
            return Err(BlockError::AlreadyRegisteredWithDifferentSource);
        }
        let block = Block::new(id.clone(), source, valid_offsets)?;
        self.blocks.insert(id.clone(), block);
        Ok(self.blocks.get(&id).unwrap())
    }

    /// Replace an existing block's source, bumping its version. Returns
    /// the new block. Used when the runtime detects a `proc` redefinition
    /// or a re-`source` of a file.
    pub fn register_replacement(
        &mut self,
        id: BlockId,
        source: Arc<str>,
        valid_offsets: Vec<usize>,
    ) -> Result<&Block, BlockError> {
        let next_version = self.blocks.get(&id).map(|b| b.version + 1).unwrap_or(0);
        let block = Block::with_version(id.clone(), source, valid_offsets, next_version)?;
        self.blocks.insert(id.clone(), block);
        Ok(self.blocks.get(&id).unwrap())
    }

    pub fn get(&self, id: &BlockId) -> Option<&Block> {
        self.blocks.get(id)
    }

    pub fn contains(&self, id: &BlockId) -> bool {
        self.blocks.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&BlockId, &Block)> {
        self.blocks.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(s: &str) -> Arc<str> {
        Arc::from(s)
    }

    #[test]
    fn block_constructor_rejects_unsorted_offsets() {
        let err = Block::new(
            BlockId::ProcBody(arc("p")),
            arc("set x 1\nset y 2"),
            vec![8, 0],
        )
        .unwrap_err();
        assert_eq!(err, BlockError::OffsetsNotSorted);
    }

    #[test]
    fn block_constructor_rejects_duplicate_offsets() {
        let err = Block::new(
            BlockId::ProcBody(arc("p")),
            arc("set x 1\nset y 2"),
            vec![0, 0, 8],
        )
        .unwrap_err();
        assert_eq!(err, BlockError::OffsetsNotSorted);
    }

    #[test]
    fn block_constructor_rejects_offset_past_source_end() {
        let err =
            Block::new(BlockId::ProcBody(arc("p")), arc("set x 1"), vec![0, 100]).unwrap_err();
        assert!(matches!(err, BlockError::OffsetOutOfRange { .. }));
    }

    #[test]
    fn block_constructor_accepts_valid_offsets() {
        let block = Block::new(
            BlockId::ProcBody(arc("p")),
            arc("set x 1\nset y 2"),
            vec![0, 8],
        )
        .unwrap();
        assert_eq!(block.valid_offsets(), &[0, 8]);
        assert!(block.is_valid_offset(0));
        assert!(block.is_valid_offset(8));
        assert!(!block.is_valid_offset(4));
    }

    #[test]
    fn snap_to_nearest_picks_next_valid_offset() {
        let block = Block::new(
            BlockId::ProcBody(arc("p")),
            arc("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            vec![5, 10, 20],
        )
        .unwrap();
        assert_eq!(block.snap_to_nearest(0), Some(5));
        assert_eq!(block.snap_to_nearest(7), Some(10));
        assert_eq!(block.snap_to_nearest(10), Some(10));
        assert_eq!(block.snap_to_nearest(11), Some(20));
        assert_eq!(block.snap_to_nearest(21), None);
    }

    #[test]
    fn snap_to_nearest_returns_none_when_no_valid_offsets() {
        let block = Block::new(BlockId::ProcBody(arc("p")), arc(""), vec![]).unwrap();
        assert_eq!(block.snap_to_nearest(0), None);
        assert_eq!(block.snap_to_nearest(50), None);
    }

    #[test]
    fn registry_register_is_idempotent_for_identical_source() {
        let mut reg = BlockRegistry::new();
        let id = BlockId::ProcBody(arc("p"));
        reg.register(id.clone(), arc("set x 1"), vec![0]).unwrap();
        // Idempotent re-register: same source, no error, version stays at 0.
        reg.register(id.clone(), arc("set x 1"), vec![0]).unwrap();
        assert_eq!(reg.get(&id).unwrap().version(), 0);
    }

    #[test]
    fn registry_register_rejects_different_source_at_same_id() {
        let mut reg = BlockRegistry::new();
        let id = BlockId::ProcBody(arc("p"));
        reg.register(id.clone(), arc("set x 1"), vec![0]).unwrap();
        let err = reg
            .register(id.clone(), arc("set x 2"), vec![0])
            .unwrap_err();
        assert_eq!(err, BlockError::AlreadyRegisteredWithDifferentSource);
    }

    #[test]
    fn registry_register_replacement_bumps_version() {
        let mut reg = BlockRegistry::new();
        let id = BlockId::ProcBody(arc("p"));
        reg.register(id.clone(), arc("set x 1"), vec![0]).unwrap();
        reg.register_replacement(id.clone(), arc("set x 2"), vec![0])
            .unwrap();
        assert_eq!(reg.get(&id).unwrap().version(), 1);
        reg.register_replacement(id.clone(), arc("set x 3"), vec![0])
            .unwrap();
        assert_eq!(reg.get(&id).unwrap().version(), 2);
    }

    #[test]
    fn line_to_offset_maps_each_command_to_its_line() {
        // Body is `set a 1\nset b 2\nset c 3` — three commands, lines
        // 1/2/3. Each command's start_byte is its valid_offset.
        let body = "set a 1\nset b 2\nset c 3";
        let block = Block::new(BlockId::ProcBody(arc("p")), arc(body), vec![0, 8, 16]).unwrap();
        let mapping: Vec<(u32, usize)> = block.line_to_offset().to_vec();
        assert_eq!(mapping, vec![(1, 0), (2, 8), (3, 16)]);
        assert_eq!(block.offset_for_line(1), Some(0));
        assert_eq!(block.offset_for_line(2), Some(8));
        assert_eq!(block.offset_for_line(3), Some(16));
        assert_eq!(block.offset_for_line(4), None);
    }

    #[test]
    fn line_to_offset_picks_first_offset_when_multiple_commands_share_a_line() {
        // Single line with two commands separated by `;`.
        let body = "set a 1; set b 2";
        // Both `set` commands start on line 1; offsets 0 and 9.
        let block = Block::new(BlockId::ProcBody(arc("p")), arc(body), vec![0, 9]).unwrap();
        // First-only mapping; line 1 → 0.
        assert_eq!(block.offset_for_line(1), Some(0));
        assert_eq!(block.line_to_offset(), &[(1, 0)]);
        assert_eq!(block.line_for_offset(0), Some(1));
        assert_eq!(block.line_for_offset(9), Some(1));
    }

    #[test]
    fn offset_for_cmd_finds_unique_command() {
        let body = "set a 1\nset b 2\nset c 3";
        let block = Block::new(BlockId::ProcBody(arc("p")), arc(body), vec![0, 8, 16]).unwrap();
        assert_eq!(block.offset_for_cmd("set a 1", 0), Some(0));
        assert_eq!(block.offset_for_cmd("set b 2", 0), Some(8));
        assert_eq!(block.offset_for_cmd("set c 3", 0), Some(16));
        assert_eq!(block.offset_for_cmd("not in body", 0), None);
    }

    #[test]
    fn offset_for_cmd_uses_hint_to_disambiguate_duplicates() {
        let body = "puts hi\nputs bye\nputs hi";
        // Three commands. valid_offsets at each `puts`.
        let block = Block::new(BlockId::ProcBody(arc("p")), arc(body), vec![0, 8, 17]).unwrap();
        // No hint → returns first occurrence.
        assert_eq!(block.offset_for_cmd("puts hi", 0), Some(0));
        // Hint past first occurrence → returns second occurrence.
        assert_eq!(block.offset_for_cmd("puts hi", 5), Some(17));
        // Hint past last occurrence → wraps and returns first.
        assert_eq!(block.offset_for_cmd("puts hi", 100), Some(0));
    }

    #[test]
    fn line_to_offset_handles_empty_block() {
        let block = Block::new(BlockId::ProcBody(arc("p")), arc(""), vec![]).unwrap();
        assert_eq!(block.line_to_offset(), &[]);
        assert_eq!(block.offset_for_line(1), None);
    }

    #[test]
    fn anonymous_blocks_collide_on_identical_content() {
        let a = BlockId::anonymous_for("set x 1");
        let b = BlockId::anonymous_for("set x 1");
        let c = BlockId::anonymous_for("set x 2");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
