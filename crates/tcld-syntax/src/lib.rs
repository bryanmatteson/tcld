use streaming_iterator::StreamingIterator;
use tcld_core::error::{ByteCodeError, Result, TcldError};
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};

const TCL_HIGHLIGHTS_QUERY: &str = include_str!("../queries/tcl/highlights.scm");
const TBCLOAD_PAYLOAD_QUERY: &str = r#"
(
  command
    name: (simple_word) @name
    arguments: (word_list (braced_word) @payload)
    (#eq? @name "tbcload::bceval")
)
"#;
const RAW_PAYLOAD_QUERY: &str = r#"
(
  command
    name: (simple_word) @name
    arguments: (word_list (simple_word) @kind)
    (#eq? @name "TclPro")
    (#eq? @kind "ByteCode")
)
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxToken {
    pub capture: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub start_column: usize,
    pub end_row: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TclCommandKind {
    /// A generic `command` node — the parsed command name lives in `name`.
    Command,
    Set,
    If,
    Procedure,
    Foreach,
    While,
    Try,
    Catch,
    Namespace,
    Regexp,
    Global,
    ExprCmd,
}

impl TclCommandKind {
    fn from_node_kind(kind: &str) -> Option<Self> {
        Some(match kind {
            "command" => Self::Command,
            "set" => Self::Set,
            "if" => Self::If,
            "procedure" => Self::Procedure,
            "foreach" => Self::Foreach,
            "while" => Self::While,
            "try" => Self::Try,
            "catch" => Self::Catch,
            "namespace" => Self::Namespace,
            "regexp" => Self::Regexp,
            "global" => Self::Global,
            "expr_cmd" => Self::ExprCmd,
            _ => return None,
        })
    }

    /// Surface keyword for special forms; `Command` returns `None` because the
    /// real name has to be read out of the node.
    pub fn canonical_keyword(self) -> Option<&'static str> {
        Some(match self {
            Self::Command => return None,
            Self::Set => "set",
            Self::If => "if",
            Self::Procedure => "proc",
            Self::Foreach => "foreach",
            Self::While => "while",
            Self::Try => "try",
            Self::Catch => "catch",
            Self::Namespace => "namespace",
            Self::Regexp => "regexp",
            Self::Global => "global",
            Self::ExprCmd => "expr",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TclCommandSpan {
    pub kind: TclCommandKind,
    /// First word of the command (e.g. `puts`, `dict`, `lappend`) or the
    /// canonical keyword for special forms (`set`, `if`, `proc`, ...).
    pub name: Option<String>,
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub start_column: usize,
    pub end_row: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TclProcBodySpan {
    pub qname: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldingRangeKind {
    Region,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldingRange {
    pub kind: FoldingRangeKind,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub start_column: usize,
    pub end_row: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    pub trim_trailing_whitespace: bool,
    pub collapse_blank_lines: bool,
    pub ensure_trailing_newline: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            trim_trailing_whitespace: true,
            collapse_blank_lines: false,
            ensure_trailing_newline: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatEdit {
    pub start_byte: usize,
    pub end_byte: usize,
    pub replacement: String,
}

pub fn parse_tcl(source: &str) -> Result<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_tcl::LANGUAGE.into())
        .map_err(|err| TcldError::Other(format!("failed to initialize Tcl parser: {err}")))?;

    parser
        .parse(source, None)
        .ok_or_else(|| TcldError::Other("failed to parse Tcl source".to_string()))
}

pub fn extract_tbc_payload_from_file(path: &str) -> Result<String> {
    extract_from_file(path)
}

pub fn extract_from_file(path: &str) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let contents = match String::from_utf8(bytes) {
        Ok(contents) => contents,
        Err(err) => String::from_utf8_lossy(&err.into_bytes()).into_owned(),
    };
    extract_from_contents(&contents)
}

pub fn extract_from_contents(contents: &str) -> Result<String> {
    let tree = parse_tcl(contents)?;

    if let Some(payload) = extract_tbcload_payload_with_query(contents, &tree)? {
        return normalize_payload(&payload).ok_or(ByteCodeError::invalid_input().into());
    }

    if let Some(offset) = extract_raw_payload_offset_with_query(contents, &tree)? {
        return normalize_payload(&contents[offset..]).ok_or(ByteCodeError::invalid_input().into());
    }

    // Some valid compiler wrappers parse without errors but use a specialized
    // command node shape that the narrow query above does not match (notably
    // wrappers with a leading `if {[catch ...]}` guard). Walk the parsed
    // command nodes before limiting the remaining recovery to malformed trees.
    if let Some(payload) = recover_tbcload_payload_from_error_tree(contents, &tree) {
        return normalize_payload(&payload).ok_or(ByteCodeError::invalid_input().into());
    }

    if tree.root_node().has_error()
        && let Some(offset) = recover_raw_payload_offset_from_error_tree(contents, &tree)
    {
        return normalize_payload(&contents[offset..]).ok_or(ByteCodeError::invalid_input().into());
    }

    Err(ByteCodeError::invalid_input().into())
}

pub fn highlight_tcl(source: &str) -> Result<Vec<SyntaxToken>> {
    highlight_tcl_with_query(source, TCL_HIGHLIGHTS_QUERY)
}

pub fn command_spans(source: &str) -> Result<Vec<TclCommandSpan>> {
    let tree = parse_tcl(source)?;
    let mut commands = command_spans_from_tree(source, &tree);

    // tree-sitter-tcl currently misparses Tcl array-variable substitutions
    // such as `$::env(NAME)` and can absorb the following top-level command
    // into the error node. Reparse error trees with only the `$name(index)`
    // delimiters neutralized. The replacement is byte-for-byte the same
    // length, so recovered node ranges still index the untouched source.
    if tree.root_node().has_error() {
        let recovered_source = neutralize_array_variable_substitutions(source);
        if recovered_source != source
            && let Ok(recovered_tree) = parse_tcl(&recovered_source)
        {
            let recovered = command_spans_from_tree(source, &recovered_tree);
            if recovered.len() > commands.len() {
                commands = recovered;
            }
        }
    }

    // Outer commands sort before nested ones at the same start offset by
    // descending end_byte; consumers can walk the list as a containment-ordered
    // tree without recomputing the parent for each span.
    commands.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then_with(|| b.end_byte.cmp(&a.end_byte))
    });
    Ok(commands)
}

/// Return statically-addressable braced proc bodies in file byte coordinates.
///
/// Dynamic proc names and unbraced bodies are intentionally omitted because
/// neither can be mapped reliably to the runtime proc-body block.
pub fn proc_body_spans(source: &str) -> Result<Vec<TclProcBodySpan>> {
    let spans = command_spans(source)?;
    let namespace_bodies = spans
        .iter()
        .filter_map(|span| {
            if span.kind != TclCommandKind::Namespace {
                return None;
            }
            let words = scan_tcl_words(&span.text);
            if words.len() != 4 || span.text[words[1].0..words[1].1].trim() != "eval" {
                return None;
            }
            let name = span.text[words[2].0..words[2].1].trim();
            let (body_start, body_end) = words[3];
            let body = &span.text[body_start..body_end];
            if name.is_empty()
                || name.contains('$')
                || name.contains('[')
                || !body.starts_with('{')
                || !body.ends_with('}')
            {
                return None;
            }
            Some((
                name.to_string(),
                span.start_byte + body_start + 1,
                span.start_byte + body_end - 1,
            ))
        })
        .collect::<Vec<_>>();
    let mut bodies = Vec::new();
    for span in spans {
        if span.kind != TclCommandKind::Procedure {
            continue;
        }
        let words = scan_tcl_words(&span.text);
        if words.len() != 4 {
            continue;
        }
        let qname = span.text[words[1].0..words[1].1].trim();
        if qname.is_empty() || qname.contains('$') || qname.contains('[') {
            continue;
        }
        let (body_start, body_end) = words[3];
        let body = &span.text[body_start..body_end];
        if !body.starts_with('{') || !body.ends_with('}') || body.len() < 2 {
            continue;
        }
        let mut namespace = String::new();
        let mut containing_namespaces = namespace_bodies
            .iter()
            .filter(|(_, start, end)| *start <= span.start_byte && span.end_byte <= *end)
            .collect::<Vec<_>>();
        containing_namespaces.sort_by_key(|(_, start, _)| *start);
        for (name, _, _) in containing_namespaces {
            if name.starts_with("::") || namespace.is_empty() {
                namespace = name.trim_matches(':').to_string();
            } else {
                namespace.push_str("::");
                namespace.push_str(name.trim_matches(':'));
            }
        }
        let bare_qname = qname.trim_start_matches("::");
        let resolved_qname = if qname.starts_with("::") || namespace.is_empty() {
            bare_qname.to_string()
        } else {
            format!("{namespace}::{bare_qname}")
        };
        bodies.push(TclProcBodySpan {
            qname: resolved_qname,
            start_byte: span.start_byte + body_start + 1,
            end_byte: span.start_byte + body_end - 1,
        });
    }
    Ok(bodies)
}

fn scan_tcl_words(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let start = index;
        match bytes[index] {
            b'{' => {
                let mut depth = 1usize;
                index += 1;
                while index < bytes.len() && depth > 0 {
                    match bytes[index] {
                        b'\\' => index = (index + 2).min(bytes.len()),
                        b'{' => {
                            depth += 1;
                            index += 1;
                        }
                        b'}' => {
                            depth -= 1;
                            index += 1;
                        }
                        _ => index += 1,
                    }
                }
            }
            b'"' => {
                index += 1;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index = (index + 2).min(bytes.len()),
                        b'"' => {
                            index += 1;
                            break;
                        }
                        _ => index += 1,
                    }
                }
            }
            _ => {
                while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else {
                        index += 1;
                    }
                }
            }
        }
        words.push((start, index.min(text.len())));
    }
    words
}

fn command_spans_from_tree(source: &str, tree: &Tree) -> Vec<TclCommandSpan> {
    collect_command_like_nodes(tree.root_node())
        .into_iter()
        .filter_map(|(kind, node)| {
            let start = node.start_position();
            let end = node.end_position();
            let text = node_text(source, node)?.trim_end().to_string();
            if text.trim().is_empty() {
                return None;
            }
            let name = command_name(source, kind, node);
            Some(TclCommandSpan {
                kind,
                name,
                text,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                start_row: start.row,
                start_column: start.column,
                end_row: end.row,
                end_column: end.column,
            })
        })
        .collect()
}

fn neutralize_array_variable_substitutions(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' || bytes.get(index + 1) == Some(&b'{') {
            index += 1;
            continue;
        }
        let mut open = index + 1;
        while open < bytes.len()
            && (bytes[open].is_ascii_alphanumeric() || matches!(bytes[open], b'_' | b':' | b'\''))
        {
            open += 1;
        }
        if bytes.get(open) != Some(&b'(') {
            index += 1;
            continue;
        }
        let mut depth = 1usize;
        let mut close = open + 1;
        while close < bytes.len() && depth > 0 {
            match bytes[close] {
                b'\\' => close = close.saturating_add(1),
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            close += 1;
        }
        if depth == 0 {
            bytes[index] = b'v';
            bytes[open] = b'_';
            bytes[close - 1] = b'_';
            index = close;
        } else {
            index += 1;
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| source.to_string())
}

fn collect_command_like_nodes(root: Node<'_>) -> Vec<(TclCommandKind, Node<'_>)> {
    let mut nodes = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        // tree-sitter-tcl promotes the literal keyword (e.g. `set`, `if`) to a
        // same-named child of the special-form node. Real special forms always
        // have at least one named child; the keyword itself does not.
        if let Some(kind) = TclCommandKind::from_node_kind(node.kind())
            && node.named_child_count() > 0
        {
            nodes.push((kind, node));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    nodes
}

fn command_name(source: &str, kind: TclCommandKind, node: Node<'_>) -> Option<String> {
    if let Some(keyword) = kind.canonical_keyword() {
        return Some(keyword.to_string());
    }
    let name_node = node.child_by_field_name("name")?;
    let raw = node_text(source, name_node)?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

pub fn folding_ranges(source: &str) -> Result<Vec<FoldingRange>> {
    let tree = parse_tcl(source)?;
    let mut ranges = collect_region_folding_ranges(source, tree.root_node());
    ranges.extend(collect_comment_folding_ranges(source, tree.root_node()));

    ranges.sort_by_key(|range| {
        (
            range.start_byte,
            range.end_byte,
            folding_range_kind_order(range.kind),
        )
    });
    ranges.dedup_by(|left, right| {
        left.kind == right.kind
            && left.start_row == right.start_row
            && left.start_column == right.start_column
            && left.end_row == right.end_row
            && left.end_column == right.end_column
    });
    Ok(ranges)
}

pub fn highlight_tcl_with_query(source: &str, query_source: &str) -> Result<Vec<SyntaxToken>> {
    let tree = parse_tcl(source)?;
    let language = tree_sitter_tcl::LANGUAGE.into();
    let query = Query::new(&language, query_source)
        .map_err(|err| TcldError::Other(format!("invalid highlight query: {err}")))?;

    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut tokens = Vec::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(matched) = matches.next() {
        for capture in matched.captures {
            let start = capture.node.start_position();
            let end = capture.node.end_position();
            tokens.push(SyntaxToken {
                capture: capture_names[capture.index as usize].to_string(),
                start_byte: capture.node.start_byte(),
                end_byte: capture.node.end_byte(),
                start_row: start.row,
                start_column: start.column,
                end_row: end.row,
                end_column: end.column,
            });
        }
    }

    tokens.sort_by_key(|token| (token.start_byte, token.end_byte, token.capture.clone()));
    Ok(tokens)
}

pub fn format_tcl(source: &str) -> Result<String> {
    format_tcl_with_options(source, FormatOptions::default())
}

pub fn format_tcl_with_options(source: &str, options: FormatOptions) -> Result<String> {
    // Parse first so formatting stays coupled to valid Tcl syntax.
    let _tree = parse_tcl(source)?;

    let mut lines: Vec<String> = source
        .lines()
        .map(|line| {
            if options.trim_trailing_whitespace {
                line.trim_end().to_string()
            } else {
                line.to_string()
            }
        })
        .collect();

    if options.collapse_blank_lines {
        let mut collapsed = Vec::with_capacity(lines.len());
        let mut previous_blank = false;
        for line in lines {
            let blank = line.trim().is_empty();
            if blank && previous_blank {
                continue;
            }
            previous_blank = blank;
            collapsed.push(line);
        }
        lines = collapsed;
    }

    let mut output = lines.join("\n");
    if options.ensure_trailing_newline && !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }

    Ok(output)
}

pub fn plan_format_edits(source: &str, options: FormatOptions) -> Result<Vec<FormatEdit>> {
    let formatted = format_tcl_with_options(source, options)?;
    if formatted == source {
        return Ok(Vec::new());
    }

    Ok(vec![FormatEdit {
        start_byte: 0,
        end_byte: source.len(),
        replacement: formatted,
    }])
}

fn extract_tbcload_payload_with_query(source: &str, tree: &Tree) -> Result<Option<String>> {
    let captures = run_query_captures(source, tree, TBCLOAD_PAYLOAD_QUERY)?;
    for capture in captures {
        if capture.name == "payload" {
            let raw = node_text(source, capture.node).map(str::trim).unwrap_or("");
            return Ok(Some(strip_outer_braces(raw).trim().to_string()));
        }
    }
    Ok(None)
}

fn extract_raw_payload_offset_with_query(source: &str, tree: &Tree) -> Result<Option<usize>> {
    let captures = run_query_captures(source, tree, RAW_PAYLOAD_QUERY)?;
    for capture in captures {
        if capture.name == "name" {
            return Ok(Some(capture.node.start_byte()));
        }
    }
    Ok(None)
}

fn recover_tbcload_payload_from_error_tree(source: &str, tree: &Tree) -> Option<String> {
    for command in collect_nodes_of_kind(tree.root_node(), "command") {
        let mut words = collect_nodes_of_kind(command, "simple_word");
        words.sort_by_key(|node| node.start_byte());
        if words
            .first()
            .and_then(|node| node_text(source, *node))
            .map(str::trim)
            != Some("tbcload::bceval")
        {
            continue;
        }

        let mut braced_words = collect_nodes_of_kind(command, "braced_word");
        braced_words.sort_by_key(|node| node.start_byte());
        let payload_node = braced_words
            .into_iter()
            .find(|node| node.start_byte() > words[0].end_byte())?;
        let raw = node_text(source, payload_node)?;
        return Some(strip_outer_braces(raw).trim().to_string());
    }
    None
}

fn recover_raw_payload_offset_from_error_tree(source: &str, tree: &Tree) -> Option<usize> {
    for command in collect_nodes_of_kind(tree.root_node(), "command") {
        let mut words = collect_nodes_of_kind(command, "simple_word");
        words.sort_by_key(|node| node.start_byte());
        if words.len() < 2 {
            continue;
        }
        if node_text(source, words[0])?.trim() == "TclPro"
            && node_text(source, words[1])?.trim() == "ByteCode"
        {
            return Some(words[0].start_byte());
        }
    }

    // If recovery produced no command nodes, fall back to token order.
    let mut words = collect_nodes_of_kind(tree.root_node(), "simple_word");
    words.sort_by_key(|node| node.start_byte());
    for pair in words.windows(2) {
        if node_text(source, pair[0])?.trim() == "TclPro"
            && node_text(source, pair[1])?.trim() == "ByteCode"
        {
            return Some(pair[0].start_byte());
        }
    }
    None
}

struct QueryCaptureNode<'tree> {
    name: String,
    node: Node<'tree>,
}

fn run_query_captures<'tree>(
    source: &str,
    tree: &'tree Tree,
    query_source: &str,
) -> Result<Vec<QueryCaptureNode<'tree>>> {
    let language = tree_sitter_tcl::LANGUAGE.into();
    let query = Query::new(&language, query_source)
        .map_err(|err| TcldError::Other(format!("invalid extraction query: {err}")))?;
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut captures = Vec::new();

    while let Some(matched) = matches.next() {
        for capture in matched.captures {
            captures.push(QueryCaptureNode {
                name: capture_names[capture.index as usize].to_string(),
                node: capture.node,
            });
        }
    }
    Ok(captures)
}

fn collect_nodes_of_kind<'tree>(root: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut nodes = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == kind {
            nodes.push(node);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    nodes
}

fn collect_region_folding_ranges(source: &str, root: Node<'_>) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    let mut stack = vec![root];

    while let Some(node) = stack.pop() {
        if node.kind() == "if" {
            if let Some(range) = if_consequence_folding_range(source, node) {
                ranges.push(range);
            }
        } else if node.kind() == "procedure" {
            if let Some(range) = procedure_folding_range(node) {
                ranges.push(range);
            }
        } else if is_foldable_region_node(node)
            && let Some(range) = folding_range_from_node(node, FoldingRangeKind::Region)
        {
            ranges.push(range);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    ranges
}

fn procedure_folding_range(node: Node<'_>) -> Option<FoldingRange> {
    let start = node.start_position();
    let body = node.child_by_field_name("body")?;
    let end = body.end_position();

    Some(FoldingRange {
        kind: FoldingRangeKind::Region,
        start_byte: node.start_byte(),
        end_byte: body.end_byte(),
        start_row: start.row,
        start_column: start.column,
        end_row: end.row,
        end_column: end.column,
    })
}

fn if_consequence_folding_range(source: &str, node: Node<'_>) -> Option<FoldingRange> {
    let start = node.start_position();
    let consequence = node.child_by_field_name("consequence")?;
    let mut end = consequence.end_position();
    let mut end_byte = consequence.end_byte();

    if let Some(alternative) = node.child_by_field_name("alternative") {
        let alternative_start = alternative.start_position();
        if alternative_start.row <= start.row {
            return None;
        }
        let previous_row = alternative_start.row - 1;
        let (_, line_end_byte) = line_byte_bounds(source, previous_row)?;
        end_byte = line_end_byte;
        end = tree_sitter::Point {
            row: previous_row,
            column: line_end_byte - line_start_byte(source, previous_row)?,
        };
    }

    if start.row >= end.row {
        return None;
    }

    Some(FoldingRange {
        kind: FoldingRangeKind::Region,
        start_byte: node.start_byte(),
        end_byte,
        start_row: start.row,
        start_column: start.column,
        end_row: end.row,
        end_column: end.column,
    })
}

fn line_start_byte(source: &str, row: usize) -> Option<usize> {
    line_byte_bounds(source, row).map(|(start, _)| start)
}

fn line_byte_bounds(source: &str, row: usize) -> Option<(usize, usize)> {
    let mut start_byte = 0;
    for (current_row, line) in source.split_inclusive('\n').enumerate() {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let line_without_terminator = line_without_newline
            .strip_suffix('\r')
            .unwrap_or(line_without_newline);
        let end_byte = start_byte + line_without_terminator.len();
        if current_row == row {
            return Some((start_byte, end_byte));
        }
        start_byte += line.len();
    }

    if !source.ends_with('\n') {
        return None;
    }

    if row == source.split_inclusive('\n').count() {
        return Some((source.len(), source.len()));
    }

    None
}

fn collect_comment_folding_ranges(source: &str, root: Node<'_>) -> Vec<FoldingRange> {
    let mut comments = collect_nodes_of_kind(root, "comment")
        .into_iter()
        .filter(|comment| is_full_line_comment(source, *comment))
        .collect::<Vec<_>>();
    comments.sort_by_key(|node| (node.start_position().row, node.start_byte()));

    let mut ranges = Vec::new();
    let mut group_start: Option<Node<'_>> = None;
    let mut group_end: Option<Node<'_>> = None;

    for comment in comments {
        match group_end {
            Some(previous) if comment.start_position().row == previous.end_position().row + 1 => {
                group_end = Some(comment);
            }
            _ => {
                push_comment_group(&mut ranges, group_start, group_end);
                group_start = Some(comment);
                group_end = Some(comment);
            }
        }
    }
    push_comment_group(&mut ranges, group_start, group_end);

    ranges
}

fn is_full_line_comment(source: &str, comment: Node<'_>) -> bool {
    let line_start = source[..comment.start_byte()]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    source[line_start..comment.start_byte()]
        .chars()
        .all(char::is_whitespace)
}

fn push_comment_group(
    ranges: &mut Vec<FoldingRange>,
    start: Option<Node<'_>>,
    end: Option<Node<'_>>,
) {
    let (Some(start), Some(end)) = (start, end) else {
        return;
    };
    if start.start_position().row == end.end_position().row {
        return;
    }

    let start_position = start.start_position();
    let end_position = end.end_position();
    ranges.push(FoldingRange {
        kind: FoldingRangeKind::Comment,
        start_byte: start.start_byte(),
        end_byte: end.end_byte(),
        start_row: start_position.row,
        start_column: start_position.column,
        end_row: end_position.row,
        end_column: end_position.column,
    });
}

fn folding_range_from_node(node: Node<'_>, kind: FoldingRangeKind) -> Option<FoldingRange> {
    let start = node.start_position();
    let end = node.end_position();
    if start.row >= end.row {
        return None;
    }

    Some(FoldingRange {
        kind,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_row: start.row,
        start_column: start.column,
        end_row: end.row,
        end_column: end.column,
    })
}

fn is_foldable_region_node(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "catch"
            | "command"
            | "else"
            | "elseif"
            | "finally"
            | "foreach"
            | "global"
            | "namespace"
            | "regexp"
            | "set"
            | "try"
            | "while"
    )
}

fn folding_range_kind_order(kind: FoldingRangeKind) -> u8 {
    match kind {
        FoldingRangeKind::Region => 0,
        FoldingRangeKind::Comment => 1,
    }
}

fn node_text<'a>(source: &'a str, node: Node<'_>) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
}

fn strip_outer_braces(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('{') && trimmed.ends_with('}') {
        return &trimmed[1..trimmed.len() - 1];
    }
    trimmed
}

fn normalize_payload(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_payload_from_tbcload_wrapper() {
        let source = "prefix tbcload::bceval { TclPro ByteCode 1 0 1.0 8.0\\nabc } suffix";
        let extracted = extract_from_contents(source).expect("extracts wrapper payload");
        assert!(extracted.starts_with("TclPro ByteCode"));
    }

    #[test]
    fn extracts_payload_from_guarded_compiler_wrapper() {
        let source = r#"if {[catch {package require tbcload 2.0} err] == 1} {
    return -code error "bytecode loader unavailable"
}
tbcload::bceval {
TclPro ByteCode 3 2.0a0 9.0
payload
}
"#;
        let extracted = extract_from_contents(source).expect("extracts guarded wrapper payload");
        assert_eq!(extracted, "TclPro ByteCode 3 2.0a0 9.0\npayload");
    }

    #[test]
    fn extracts_repository_tcl9_compiler_fixture() {
        let source = include_str!("../../../testing/fixtures/bytecode/tcl9/tc1.tbc");
        let extracted = extract_from_contents(source).expect("extracts Tcl 9 fixture payload");
        assert!(
            extracted.starts_with("TclPro ByteCode 3 2.0a0 9.0"),
            "unexpected extraction: {extracted:?}"
        );
        let bytecode = tcld_core::bytecode::ByteCode::from_compiled_payload(&extracted)
            .expect("decodes Tcl 9 fixture payload");
        bytecode
            .get_instructions()
            .expect("decodes Tcl 9 fixture instructions");
    }

    #[test]
    fn extracts_payload_from_raw_bytecode() {
        let source = "TclPro ByteCode 1 0 1.0 8.0\\nabc\\nxyz";
        let extracted = extract_from_contents(source).expect("extracts raw payload");
        assert_eq!(extracted, source);
    }

    #[test]
    fn highlight_tokens_include_keywords() {
        let source = "proc greet {name} {\n  puts $name\n}\n";
        let tokens = highlight_tcl(source).expect("highlight succeeds");
        assert!(!tokens.is_empty());
        assert!(tokens.iter().any(|token| token.capture.contains("keyword")));
    }

    #[test]
    fn formatter_trims_trailing_whitespace() {
        let source = "set x 1   \nputs $x\t\n";
        let formatted = format_tcl(source).expect("format succeeds");
        assert_eq!(formatted, "set x 1\nputs $x\n");
    }

    #[test]
    fn plan_format_edits_empty_for_stable_input() {
        let source = "set x 1\n";
        let edits = plan_format_edits(source, FormatOptions::default()).expect("plans edits");
        assert!(edits.is_empty());
    }

    #[test]
    fn command_spans_capture_outer_and_nested_for_each_command() {
        let source = "        lappend entries [parse_line $line]\n    set counts [count_by_level $entries]\n    puts \"INFO:  [dict get $counts INFO]\"\n    puts \"Latest error: [latest_error_message $entries]\"\n";
        let spans = command_spans(source).expect("command spans");
        let summary: Vec<(TclCommandKind, Option<&str>, usize, usize)> = spans
            .iter()
            .map(|span| {
                (
                    span.kind,
                    span.name.as_deref(),
                    span.start_row,
                    span.end_row,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (TclCommandKind::Command, Some("lappend"), 0, 0),
                (TclCommandKind::Command, Some("parse_line"), 0, 0),
                (TclCommandKind::Set, Some("set"), 1, 1),
                (TclCommandKind::Command, Some("count_by_level"), 1, 1),
                (TclCommandKind::Command, Some("puts"), 2, 2),
                (TclCommandKind::Command, Some("dict"), 2, 2),
                (TclCommandKind::Command, Some("puts"), 3, 3),
                (TclCommandKind::Command, Some("latest_error_message"), 3, 3),
            ]
        );
    }

    #[test]
    fn command_spans_recover_after_array_variable_substitutions() {
        let source = "exec touch $::env(TCLD_DEBUG_PTY_ENTRY_SENTINEL)\nexec touch $::env(TCLD_DEBUG_PTY_BREAKPOINT_SENTINEL)\nset completed 1\n";
        let spans = command_spans(source).expect("command spans");
        let top_level = spans
            .iter()
            .filter(|span| span.start_column == 0)
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            top_level,
            [
                "exec touch $::env(TCLD_DEBUG_PTY_ENTRY_SENTINEL)",
                "exec touch $::env(TCLD_DEBUG_PTY_BREAKPOINT_SENTINEL)",
                "set completed 1",
            ]
        );
    }

    #[test]
    fn command_spans_capture_special_form_kinds() {
        let source = "proc greet {name} {\n  if {$name eq \"\"} { set greeting \"hi\" } else { set greeting \"hi $name\" }\n  puts $greeting\n}\n";
        let spans = command_spans(source).expect("command spans");
        let kinds: Vec<TclCommandKind> = spans.iter().map(|span| span.kind).collect();
        assert!(kinds.contains(&TclCommandKind::Procedure));
        assert!(kinds.contains(&TclCommandKind::If));
        assert!(kinds.iter().filter(|k| **k == TclCommandKind::Set).count() >= 2);
        assert!(spans.iter().any(
            |span| span.kind == TclCommandKind::Command && span.name.as_deref() == Some("puts")
        ));
    }

    #[test]
    fn command_spans_order_outer_before_nested() {
        let source = "set x [foo [bar]]\n";
        let spans = command_spans(source).expect("command spans");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].kind, TclCommandKind::Set);
        assert_eq!(spans[1].name.as_deref(), Some("foo"));
        assert_eq!(spans[2].name.as_deref(), Some("bar"));
        for window in spans.windows(2) {
            assert!(window[0].start_byte <= window[1].start_byte);
        }
    }

    #[test]
    fn proc_body_spans_expose_runtime_relative_origin() {
        let source = "proc ::demo {value} {\n  set x [expr {$value + 1}]\n  return $x\n}\n";
        let bodies = proc_body_spans(source).unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].qname, "demo");
        assert_eq!(
            &source[bodies[0].start_byte..bodies[0].end_byte],
            "\n  set x [expr {$value + 1}]\n  return $x\n"
        );
        let body = &source[bodies[0].start_byte..bodies[0].end_byte];
        let body_commands = command_spans(body).unwrap();
        let file_commands = command_spans(source).unwrap();
        for body_command in body_commands {
            let file_command = file_commands
                .iter()
                .find(|command| {
                    command.start_byte == bodies[0].start_byte + body_command.start_byte
                        && command.name == body_command.name
                })
                .expect("whole-file and proc-body parses identify the same command");
            assert_eq!(
                file_command.start_byte - bodies[0].start_byte,
                body_command.start_byte
            );
        }
    }

    #[test]
    fn proc_body_spans_resolve_static_namespace_eval_context() {
        let source = "namespace eval outer {\n  namespace eval inner {\n    proc demo {} { return ok }\n  }\n}\n";
        let bodies = proc_body_spans(source).unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].qname, "outer::inner::demo");
    }

    #[test]
    fn folding_ranges_include_tcl_command_blocks() {
        let source = "proc greet {name} {\n  if {$name eq \"\"} {\n    puts empty\n  } else {\n    puts $name\n  }\n}\n";
        let ranges = folding_ranges(source).expect("folding ranges succeed");

        assert!(ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Region
                && range.start_row == 0
                && range.start_column == 0
                && range.end_row == 6
        }));
        assert!(ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Region
                && range.start_row == 1
                && range.start_column == 2
                && range.end_row == 2
        }));
        assert!(ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Region
                && range.start_row == 3
                && range.start_column == 4
                && range.end_row == 5
        }));
        assert!(!ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Region && range.start_row == 1 && range.end_row == 5
        }));
    }

    #[test]
    fn folding_ranges_include_arbitrary_multiline_commands() {
        let source = "set payload {\n  line one\n  line two\n}\n";
        let ranges = folding_ranges(source).expect("folding ranges succeed");

        assert_eq!(
            ranges,
            vec![FoldingRange {
                kind: FoldingRangeKind::Region,
                start_byte: 0,
                end_byte: source.trim_end().len(),
                start_row: 0,
                start_column: 0,
                end_row: 3,
                end_column: 1,
            }]
        );
    }

    #[test]
    fn folding_ranges_include_single_line_proc_bodies() {
        let source = "proc fixture::lock_file {lockname filename} {java::call lockFile $lockname $filename}\n";
        let ranges = folding_ranges(source).expect("folding ranges succeed");

        assert!(ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Region
                && range.start_row == 0
                && range.end_row == 0
                && range.start_column == 0
                && range.end_column == source.trim_end().len()
        }));
    }

    #[test]
    fn folding_ranges_do_not_include_single_line_commands() {
        let source = "fixture::setValue debug_level_ctrl $debug_level(control)\n";
        let ranges = folding_ranges(source).expect("folding ranges succeed");

        assert!(
            ranges.is_empty(),
            "plain single-line commands are not user-foldable"
        );
    }

    #[test]
    fn folding_ranges_group_contiguous_comments() {
        let source = "# first\n# second\n\n# alone\nproc greet {} {\n  puts hi\n}\n";
        let ranges = folding_ranges(source).expect("folding ranges succeed");

        assert!(ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Comment && range.start_row == 0 && range.end_row == 1
        }));
        assert!(!ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Comment && range.start_row == 3 && range.end_row == 3
        }));
    }

    #[test]
    fn folding_ranges_ignore_trailing_comment_runs() {
        let source = "variable SSLv2Flag 0 ;# disable SSL V2\nvariable SSLv3Flag 1 ;# enable SSL V3\nvariable TLSv1Flag 1 ;# enable TLS\n";
        let ranges = folding_ranges(source).expect("folding ranges succeed");

        assert!(!ranges.iter().any(|range| {
            range.kind == FoldingRangeKind::Comment && range.start_row == 0 && range.end_row == 2
        }));
    }
}
