//! Tree-sitter backed Tcl highlighting for the explorer's document views.
//!
//! `tcld-syntax` already parses Tcl with Tree-sitter for the batch commands.
//! This routes the same captures into the terminal UI, so decompiled output is
//! highlighted by a real parse instead of a second, lexical approximation that
//! could disagree with it.
//!
//! Two constraints shape this module. A parse must cover the whole document —
//! highlighting only the visible window would parse a fragment mid-expression
//! and produce nonsense — and it must not repeat per frame, because the explorer
//! redraws on every keystroke over artifacts thousands of lines long. So a parse
//! is cached and reused until the document's content changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use tcld_syntax::SyntaxToken;
use tcld_tui::tokens;
use tcld_tui::ui::{Line, Span, Text};
use tcld_tui::{Theme, TokenId};

/// Captures for one document, grouped by line and sorted by column.
#[derive(Debug, Default)]
struct LineSpans {
    /// `(start_column, end_column, capture)` in byte offsets within the line.
    spans: Vec<Vec<(usize, usize, Arc<str>)>>,
}

impl LineSpans {
    fn from_tokens(line_count: usize, tokens: &[SyntaxToken]) -> Self {
        let mut spans: Vec<Vec<(usize, usize, Arc<str>)>> = vec![Vec::new(); line_count.max(1)];
        // Interning keeps one allocation per distinct capture name rather than
        // one per token; a large artifact yields tens of thousands of tokens
        // drawn from a vocabulary of about fifteen names.
        let mut names: HashMap<&str, Arc<str>> = HashMap::new();

        for token in tokens {
            // Multi-line captures are rare in Tcl and would need splitting per
            // line; skip them rather than mis-attribute a range to one row.
            if token.start_row != token.end_row || token.start_row >= spans.len() {
                continue;
            }
            if token.start_column >= token.end_column {
                continue;
            }
            let name = names
                .entry(token.capture.as_str())
                .or_insert_with(|| Arc::from(token.capture.as_str()))
                .clone();
            spans[token.start_row].push((token.start_column, token.end_column, name));
        }

        for line in &mut spans {
            line.sort_by_key(|(start, _, _)| *start);
        }
        Self { spans }
    }

    fn line(&self, index: usize) -> &[(usize, usize, Arc<str>)] {
        self.spans.get(index).map_or(&[], Vec::as_slice)
    }
}

/// A parse cached against a fingerprint of the document it came from.
#[derive(Default)]
pub struct HighlightCache {
    inner: RefCell<Option<(u64, Arc<LineSpans>)>>,
}

impl std::fmt::Debug for HighlightCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HighlightCache")
            .finish_non_exhaustive()
    }
}

impl Clone for HighlightCache {
    /// A clone starts cold. The cache is derived state, and sharing it would
    /// make two explorer cores contend over one `RefCell`.
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Hashing the document each frame costs one pass over memory; reparsing it
/// costs a Tree-sitter parse. The fingerprint is cheaper by orders of magnitude,
/// and unlike a slice address it cannot alias a different document.
fn fingerprint(lines: &[String]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lines.len().hash(&mut hasher);
    for line in lines {
        line.hash(&mut hasher);
    }
    hasher.finish()
}

impl HighlightCache {
    fn spans_for(&self, lines: &[String]) -> Arc<LineSpans> {
        let key = fingerprint(lines);
        if let Some((cached_key, spans)) = self.inner.borrow().as_ref()
            && *cached_key == key
        {
            return Arc::clone(spans);
        }

        // The parse must see the whole document, never just the visible window.
        let source = lines.join("\n");
        // A parse failure is not fatal: unhighlighted text still reads.
        let parsed = tcld_syntax::highlight_tcl(&source).unwrap_or_default();
        let spans = Arc::new(LineSpans::from_tokens(lines.len(), &parsed));
        *self.inner.borrow_mut() = Some((key, Arc::clone(&spans)));
        spans
    }

    /// Highlight one line in the context of the whole document.
    pub fn line(&self, lines: &[String], index: usize, theme: &Theme) -> Line<'static> {
        let spans = self.spans_for(lines);
        match lines.get(index) {
            Some(text) => render_line(text, spans.line(index), theme),
            None => Line::from(String::new()),
        }
    }

    /// Highlight a contiguous window of a document.
    pub fn window(
        &self,
        lines: &[String],
        start: usize,
        end: usize,
        theme: &Theme,
    ) -> Text<'static> {
        let spans = self.spans_for(lines);
        let start = start.min(lines.len());
        let end = end.min(lines.len()).max(start);
        Text::from(
            (start..end)
                .map(|index| render_line(&lines[index], spans.line(index), theme))
                .collect::<Vec<_>>(),
        )
    }

    /// Highlight every line of a document at once.
    ///
    /// For callers that highlight on load rather than per frame, so the result
    /// is stored rather than recomputed.
    pub fn document(&self, lines: &[String], theme: &Theme) -> Vec<Line<'static>> {
        let spans = self.spans_for(lines);
        lines
            .iter()
            .enumerate()
            .map(|(index, text)| render_line(text, spans.line(index), theme))
            .collect()
    }

    /// Highlight a standalone block of Tcl that is not part of a cached view.
    pub fn text(&self, source: &str, theme: &Theme) -> Text<'static> {
        let lines: Vec<String> = source.lines().map(str::to_string).collect();
        self.window(&lines, 0, lines.len(), theme)
    }
}

/// One run of a line and the token that styles it.
///
/// Segmentation is shared: the terminal UI turns segments into ratatui spans and
/// the batch commands turn them into Termosaic spans, so both surfaces colour
/// identical Tcl identically by construction rather than by coincidence.
#[derive(Debug, Clone)]
pub struct Segment {
    pub token: TokenId,
    pub text: String,
}

/// Split a line into styled runs covering the whole line, gaps included.
fn segment_line(text: &str, captures: &[(usize, usize, Arc<str>)]) -> Vec<Segment> {
    let plain = |text: &str| Segment {
        token: termosaic::tokens::TEXT,
        text: text.to_string(),
    };
    if captures.is_empty() {
        return vec![plain(text)];
    }

    let mut segments = Vec::with_capacity(captures.len() * 2 + 1);
    let mut cursor = 0usize;
    for (start, end, capture) in captures {
        let (start, end) = (*start, *end);
        // Captures can nest; an inner capture already consumed by an outer one
        // is skipped rather than reordering the line.
        if start < cursor
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }
        if start > cursor {
            segments.push(plain(&text[cursor..start]));
        }
        segments.push(Segment {
            token: tokens::token_for_capture(capture),
            text: text[start..end].to_string(),
        });
        cursor = end;
    }
    if cursor < text.len() {
        segments.push(plain(&text[cursor..]));
    }
    segments
}

/// Segment every line of a document.
///
/// For callers that render once — the batch commands — and so have no use for
/// the frame-to-frame cache.
pub fn document_segments(lines: &[String]) -> Vec<Vec<Segment>> {
    let source = lines.join("\n");
    let parsed = tcld_syntax::highlight_tcl(&source).unwrap_or_default();
    let spans = LineSpans::from_tokens(lines.len(), &parsed);
    lines
        .iter()
        .enumerate()
        .map(|(index, text)| segment_line(text, spans.line(index)))
        .collect()
}

fn render_line(text: &str, captures: &[(usize, usize, Arc<str>)], theme: &Theme) -> Line<'static> {
    Line::from(
        segment_line(text, captures)
            .into_iter()
            .map(|segment| Span::styled(segment.text, theme.style(&segment.token)))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source: &str) -> Vec<String> {
        source.lines().map(str::to_string).collect()
    }

    fn rendered(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn highlighting_never_alters_the_text() {
        let theme = Theme::new(false);
        let cache = HighlightCache::default();
        let source = lines(
            "proc fixture::demo {a b} {\n    set x [expr {$a + 1}]\n    # a comment\n    return \"$x\"\n}",
        );

        for index in 0..source.len() {
            assert_eq!(
                rendered(&cache.line(&source, index, &theme)),
                source[index],
                "line {index} must survive highlighting unchanged"
            );
        }
    }

    #[test]
    fn a_parse_distinguishes_strings_comments_and_keywords() {
        let theme = Theme::new(false);
        let cache = HighlightCache::default();
        let source = lines("set greeting \"hello\"\n# trailing note");

        let code = cache.line(&source, 0, &theme);
        let styles: std::collections::HashSet<_> =
            code.spans.iter().map(|span| span.style).collect();
        assert!(
            styles.len() > 1,
            "a parsed line is not uniformly styled: {code:?}"
        );
        assert!(
            code.spans
                .iter()
                .any(|span| span.content.contains("hello") && span.style == theme.string()),
            "the quoted word is styled as a string: {code:?}"
        );

        let comment = cache.line(&source, 1, &theme);
        assert!(
            comment
                .spans
                .iter()
                .any(|span| span.style == theme.comment()),
            "the comment is styled as a comment: {comment:?}"
        );
    }

    #[test]
    fn a_window_is_parsed_in_the_context_of_the_whole_document() {
        let theme = Theme::new(false);
        let cache = HighlightCache::default();
        // The brace opened on line 0 closes only on line 3. Parsing rows 1..3 in
        // isolation would feed Tree-sitter an unbalanced fragment.
        let source = lines("proc demo {} {\n    set x 1\n    set y 2\n}");

        let text = cache.window(&source, 1, 3, &theme);
        assert_eq!(text.lines.len(), 2);
        assert_eq!(rendered(&text.lines[0]), "    set x 1");
        assert_eq!(rendered(&text.lines[1]), "    set y 2");
        assert!(
            text.lines[0].spans.iter().any(|s| s.style != theme.fg()),
            "a mid-document window is still highlighted: {:?}",
            text.lines[0]
        );
    }

    #[test]
    fn unparseable_text_still_renders_verbatim() {
        let theme = Theme::new(false);
        let cache = HighlightCache::default();
        let source = lines("{{{ this is not valid tcl");

        assert_eq!(rendered(&cache.line(&source, 0, &theme)), source[0]);
    }

    #[test]
    fn the_parse_is_reused_until_the_document_changes() {
        let theme = Theme::new(false);
        let cache = HighlightCache::default();
        let first = lines("set a 1");
        let second = lines("set b 2");

        let _ = cache.line(&first, 0, &theme);
        let key = cache.inner.borrow().as_ref().map(|(key, _)| *key);
        let _ = cache.line(&first, 0, &theme);
        assert_eq!(
            cache.inner.borrow().as_ref().map(|(key, _)| *key),
            key,
            "an unchanged document must not reparse"
        );

        let _ = cache.line(&second, 0, &theme);
        assert_ne!(
            cache.inner.borrow().as_ref().map(|(key, _)| *key),
            key,
            "a changed document must reparse"
        );
    }

    #[test]
    fn utf8_content_is_split_on_character_boundaries() {
        let theme = Theme::new(false);
        let cache = HighlightCache::default();
        let source = lines("set label \"héllo → wörld\"");

        assert_eq!(rendered(&cache.line(&source, 0, &theme)), source[0]);
    }
}
