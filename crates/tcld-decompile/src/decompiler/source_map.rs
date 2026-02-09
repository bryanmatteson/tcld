//! Source mapping between bytecode offsets and decompiled source positions.
//!
//! Provides bidirectional mapping to support interactive features like:
//! - Clicking a bytecode instruction highlights the corresponding source line
//! - Clicking a source line highlights the originating bytecode instructions
//! - Side-by-side diff view with synchronized scrolling

use crate::decompiler::CommandIdentity;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandPath {
    pub components: Vec<usize>,
}

impl CommandPath {
    pub fn root(group: usize) -> Self {
        Self {
            components: vec![group],
        }
    }

    pub fn nested(parent: usize, group: usize) -> Self {
        Self {
            components: vec![parent, group],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalSourceStatus {
    Available,
    Unavailable,
}

/// A mapping entry linking a bytecode range to a source text range.
#[derive(Debug, Clone)]
pub struct SourceMapEntry {
    /// Byte offset range in the bytecode.
    pub bytecode_range: Range<i32>,
    /// Line range in the decompiled source (0-indexed).
    pub source_lines: Range<usize>,
    /// Character offset range in the decompiled source text.
    pub source_chars: Range<usize>,
    /// The decompiled text for this span.
    pub text: String,
    /// Identity of this span — bytecode-backed, synthetic (bcproc body), or unknown.
    pub identity: CommandIdentity,
    /// Stable path for nested command bodies.
    pub command_path: CommandPath,
    /// Parent command group for nested command-body mappings.
    pub parent_command_group: Option<usize>,
    /// Source span in the original Tcl input, when known.
    pub original_span: Option<Range<i32>>,
    /// Whether original source was available for this entry.
    pub original_status: OriginalSourceStatus,
}

impl SourceMapEntry {
    /// Bytecode command group, or `None` for synthetic / unknown entries.
    pub fn command_group(&self) -> Option<usize> {
        self.identity.command_group()
    }

    /// Synthetic identity key for runtime lookup, when applicable.
    pub fn synthetic_key(&self) -> Option<(&str, usize)> {
        match &self.identity {
            CommandIdentity::Synthetic {
                proc_qname: Some(qname),
                body_byte_offset,
                ..
            } => Some((qname.as_ref(), *body_byte_offset)),
            _ => None,
        }
    }
}

/// Bidirectional source map for a decompilation result.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    /// Entries sorted by bytecode offset.
    entries: Vec<SourceMapEntry>,
    /// Index: bytecode offset → entry indices.
    bytecode_index: BTreeMap<i32, Vec<usize>>,
    /// Index: source line → entry indices.
    line_index: BTreeMap<usize, Vec<usize>>,
    /// Index: decompiled char offset → entry indices.
    char_index: BTreeMap<usize, Vec<usize>>,
    /// Index: command path → entry indices.
    command_path_index: BTreeMap<CommandPath, Vec<usize>>,
    /// Index: original source offsets → entry indices.
    original_index: BTreeMap<i32, Vec<usize>>,
    /// Index: (qualified proc name, body byte offset) → entry indices.
    /// Powers debugger lookups: at runtime a `[info frame]` query yields the
    /// proc name and a position within the proc body source string; matching
    /// against this index identifies the synthetic span being executed.
    synthetic_index: BTreeMap<(Arc<str>, usize), Vec<usize>>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a source map from command spans and the full source text.
    pub fn from_spans(spans: &[(Range<i32>, String, Range<usize>)]) -> Self {
        let mut map = SourceMap::new();
        let mut char_cursor = 0usize;
        for (idx, (bytecode_range, text, source_lines)) in spans.iter().enumerate() {
            let char_start = char_cursor;
            let char_end = char_start + text.len();
            let entry = SourceMapEntry {
                bytecode_range: bytecode_range.clone(),
                source_lines: source_lines.clone(),
                source_chars: char_start..char_end,
                text: text.clone(),
                identity: CommandIdentity::Bytecode { command_group: idx },
                command_path: CommandPath::root(idx),
                parent_command_group: None,
                original_span: None,
                original_status: OriginalSourceStatus::Unavailable,
            };
            map.push_entry(entry);
            char_cursor = char_end + 1;
        }
        map
    }

    pub fn from_decompiled_output(
        bytecode: &crate::bytecode::ByteCode,
        output: &crate::decompiler::DecompiledOutput,
        original_source_available: bool,
    ) -> Self {
        let line_starts = line_start_offsets(&output.text);
        let mut map = SourceMap::new();

        for span in &output.commands {
            let Some(source_chars) = locate_span_chars(&output.text, &line_starts, span) else {
                continue;
            };
            let command_group = span.command_group();
            let parent_command_group = span.parent_command_group;
            let command_path = match (command_group, parent_command_group) {
                (Some(group), Some(parent)) => {
                    CommandPath::nested(parent, span.nested_command_index.unwrap_or(group))
                }
                (Some(group), None) => CommandPath::root(group),
                (None, Some(parent)) => {
                    CommandPath::nested(parent, span.nested_command_index.unwrap_or(0))
                }
                (None, None) => CommandPath { components: vec![] },
            };
            let original_span = if original_source_available {
                command_group
                    .and_then(|group| bytecode.cmd_source_ranges.as_ref()?.get(group).cloned())
            } else {
                None
            };
            let original_status = if original_span.is_some() {
                OriginalSourceStatus::Available
            } else {
                OriginalSourceStatus::Unavailable
            };

            map.push_entry(SourceMapEntry {
                bytecode_range: span.code_range.clone(),
                source_lines: span.start_line..(span.end_line + 1),
                source_chars,
                text: span.text.clone(),
                identity: span.identity.clone(),
                command_path,
                parent_command_group,
                original_span,
                original_status,
            });
        }

        map
    }

    fn push_entry(&mut self, entry: SourceMapEntry) {
        let idx = self.entries.len();
        if entry.identity.is_bytecode() && entry.parent_command_group.is_none() {
            for offset in entry.bytecode_range.clone() {
                self.bytecode_index.entry(offset).or_default().push(idx);
            }
        }
        for line in entry.source_lines.clone() {
            self.line_index.entry(line).or_default().push(idx);
        }
        for offset in entry.source_chars.clone() {
            self.char_index.entry(offset).or_default().push(idx);
        }
        self.command_path_index
            .entry(entry.command_path.clone())
            .or_default()
            .push(idx);
        if let Some(original_span) = &entry.original_span {
            for offset in original_span.clone() {
                self.original_index.entry(offset).or_default().push(idx);
            }
        }
        if let CommandIdentity::Synthetic {
            proc_qname: Some(qname),
            body_byte_offset,
            ..
        } = &entry.identity
        {
            self.synthetic_index
                .entry((qname.clone(), *body_byte_offset))
                .or_default()
                .push(idx);
        }
        self.entries.push(entry);
    }

    /// Look up source location(s) for a bytecode offset.
    pub fn source_for_bytecode(&self, offset: i32) -> Vec<&SourceMapEntry> {
        self.bytecode_index
            .get(&offset)
            .map(|indices| indices.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    /// Look up bytecode location(s) for a source line.
    pub fn bytecode_for_line(&self, line: usize) -> Vec<&SourceMapEntry> {
        self.line_index
            .get(&line)
            .map(|indices| indices.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    pub fn lookup_decompiled_position(
        &self,
        line: usize,
        character: usize,
    ) -> Vec<&SourceMapEntry> {
        self.bytecode_for_line(line)
            .into_iter()
            .filter(|entry| entry_contains_line_character(entry, line, character))
            .collect()
    }

    pub fn entries_for_command_path(&self, path: &CommandPath) -> Vec<&SourceMapEntry> {
        self.command_path_index
            .get(path)
            .map(|indices| indices.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    pub fn lookup_original_offset(&self, offset: i32) -> Vec<&SourceMapEntry> {
        self.original_index
            .get(&offset)
            .map(|indices| indices.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    /// Look up synthetic entries by their stable runtime key.
    ///
    /// `proc_qname` is the qualified Tcl proc name (e.g. `"fixture::startlog"`)
    /// and `body_byte_offset` is the byte offset of the command within the
    /// proc body source — both readily available from a Tcl `[info frame]`
    /// query, which is what makes this index useful for debugger breakpoint
    /// lookups in bcproc bodies that have no own bytecode.
    pub fn lookup_synthetic(
        &self,
        proc_qname: &str,
        body_byte_offset: usize,
    ) -> Vec<&SourceMapEntry> {
        // Borrow-friendly lookup: build a temporary key without cloning into Arc.
        self.synthetic_index
            .iter()
            .find(|((qname, offset), _)| {
                qname.as_ref() == proc_qname && *offset == body_byte_offset
            })
            .map(|(_, indices)| indices.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    /// Get all entries.
    pub fn entries(&self) -> &[SourceMapEntry] {
        &self.entries
    }

    /// Total number of mapped spans.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn locate_span_chars(
    source: &str,
    line_starts: &[usize],
    span: &crate::decompiler::DecompiledCommandSpan,
) -> Option<Range<usize>> {
    let start_line_offset = *line_starts.get(span.start_line)?;
    let end_line_offset = *line_starts.get(span.end_line)?;
    let start_line_end = source[start_line_offset..]
        .find('\n')
        .map(|offset| start_line_offset + offset)
        .unwrap_or(source.len());
    let end_line_end = source[end_line_offset..]
        .find('\n')
        .map(|offset| end_line_offset + offset)
        .unwrap_or(source.len());
    let first = span.text.lines().next().unwrap_or_default().trim_start();
    let last = span.text.lines().last().unwrap_or_default().trim_end();
    let start_column = source[start_line_offset..start_line_end]
        .find(first)
        .unwrap_or(0);
    let end_column = source[end_line_offset..end_line_end]
        .rfind(last)
        .map(|column| column + last.len())
        .unwrap_or_else(|| end_line_end.saturating_sub(end_line_offset));
    Some((start_line_offset + start_column)..(end_line_offset + end_column))
}

fn entry_contains_line_character(entry: &SourceMapEntry, line: usize, character: usize) -> bool {
    if !entry.source_lines.contains(&line) {
        return false;
    }
    let mut absolute = entry.source_chars.start;
    for (relative_line, text_line) in entry.text.split('\n').enumerate() {
        let current_line = entry.source_lines.start + relative_line;
        if current_line == line {
            return entry
                .source_chars
                .contains(&(absolute.saturating_add(character)))
                && character <= text_line.len();
        }
        absolute = absolute.saturating_add(text_line.len()).saturating_add(1);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_map() {
        let map = SourceMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert!(map.source_for_bytecode(0).is_empty());
        assert!(map.bytecode_for_line(0).is_empty());
    }

    #[test]
    fn bidirectional_lookup() {
        let spans = vec![
            (0..10, "set a 1".to_string(), 0..1),
            (10..25, "if {$x} {\n    puts hello\n}".to_string(), 1..4),
        ];
        let map = SourceMap::from_spans(&spans);
        assert_eq!(map.len(), 2);

        // Bytecode → source
        let results = map.source_for_bytecode(5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "set a 1");

        // Source line → bytecode
        let results = map.bytecode_for_line(2);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].bytecode_range, 10..25);

        // Unmapped offset
        assert!(map.source_for_bytecode(50).is_empty());
    }
}
