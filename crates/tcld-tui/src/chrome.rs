//! Laying out single-row chrome.
//!
//! The header and footer must say as much as the terminal has room for and
//! never wrap. That is a choice problem, not a truncation problem: rather than
//! writing a full label and cutting it, the row offers Laidout a ladder of
//! progressively terser phrasings and Laidout picks the densest that fits.
//!
//! This replaces hand-rolled width arithmetic that repeatedly got the
//! measurement wrong — the chrome is drawn with `│ · … ▸`, each one cell wide
//! but multi-byte, and every budget that assumed otherwise over-counted its own
//! separators.

use std::num::NonZeroU16;

use ratatui::text::{Line, Span};
use termosaic::{Doc, HumanLayoutOptions, StyledRenderWorkspace, TextLayout, TokenId};

use crate::theme::Theme;

/// One phrasing of a row, and the tokens that style it.
pub struct Variant {
    fragments: Vec<(TokenId, String)>,
}

impl Variant {
    pub fn new() -> Self {
        Self {
            fragments: Vec::new(),
        }
    }

    /// Append a styled fragment.
    pub fn push(mut self, token: TokenId, text: impl Into<String>) -> Self {
        let text = text.into();
        if !text.is_empty() {
            self.fragments.push((token, text));
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    fn doc(&self) -> Doc {
        Doc::concat(
            self.fragments
                .iter()
                .map(|(token, text)| Doc::token_text(token.clone(), text)),
        )
    }
}

impl Default for Variant {
    fn default() -> Self {
        Self::new()
    }
}

/// A laid-out row: ratatui spans plus the width they occupy.
pub struct Row {
    pub spans: Vec<Span<'static>>,
    pub width: usize,
}

impl Row {
    pub fn empty() -> Self {
        Self {
            spans: Vec::new(),
            width: 0,
        }
    }
}

/// Lay out the densest variant that fits `width`.
///
/// Variants are offered most to least preferred. Laidout selects among them; if
/// even the last overflows it is still emitted on one line and the caller's
/// surface clips it, because a chrome row that wrapped would corrupt the frame.
pub fn fit(variants: &[Variant], width: usize, theme: &Theme) -> Row {
    let variants: Vec<&Variant> = variants.iter().filter(|v| !v.is_empty()).collect();
    let Some((last, rest)) = variants.split_last() else {
        return Row::empty();
    };
    let Some(columns) = NonZeroU16::new(u16::try_from(width).unwrap_or(u16::MAX).max(1)) else {
        return Row::empty();
    };

    let ladder = rest.iter().rev().fold(last.doc(), |alternative, variant| {
        Doc::choice(variant.doc(), alternative)
    });

    let Ok(prepared) = ladder.prepare() else {
        return Row::empty();
    };
    let mut workspace = StyledRenderWorkspace::new();
    let Ok(lines) = prepared.render_styled_into(
        columns,
        theme.semantic(),
        HumanLayoutOptions::exact(),
        &mut workspace,
    ) else {
        return Row::empty();
    };
    let Some(line) = lines.first() else {
        return Row::empty();
    };

    let spans: Vec<Span<'static>> = line
        .runs()
        .iter()
        // Style through the theme's own accessor rather than a converter built
        // here, so chrome honours whatever colour policy the theme is under.
        .map(|run| Span::styled(run.text().to_string(), theme.style(run.token())))
        .collect();
    let width = spans
        .iter()
        .map(|span| unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    Row { spans, width }
}

/// Join `left` and `right` on one row of `width`, with `right` pushed to the end.
///
/// Laidout lays out lines, not columns; right alignment is the one thing it
/// cannot express, so it is a single subtraction over widths Laidout measured.
pub fn justify(left: Row, right: Row, width: usize) -> Line<'static> {
    let mut spans = left.spans;
    let used = left.width + right.width;
    if !right.spans.is_empty() {
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used).max(1))));
        spans.extend(right.spans);
    }
    Line::from(spans)
}

/// Shorten free-running text to one row of `width`.
///
/// `Doc::from_text` turns prose into a reflowable document, so Laidout breaks it
/// at whitespace and measures the result. Keeping the first line yields an
/// elision at a word boundary rather than a cut through the middle of a word,
/// and the width is Laidout's own rather than a second opinion computed here.
///
/// Returns `None` when nothing survives, so a caller can drop the text instead
/// of showing a stub.
pub fn elide(text: &str, width: usize, theme: &Theme) -> Option<String> {
    if width == 0 {
        return None;
    }
    if unicode_width::UnicodeWidthStr::width(text) <= width {
        return Some(text.to_string());
    }

    // One column is held back for the marker that says text was dropped.
    let columns = NonZeroU16::new(
        u16::try_from(width.saturating_sub(1))
            .unwrap_or(u16::MAX)
            .max(1),
    )?;
    let prepared = Doc::from_text(text, TextLayout::Prose).prepare().ok()?;
    let mut workspace = StyledRenderWorkspace::new();
    let lines = prepared
        .render_styled_into(
            columns,
            theme.semantic(),
            HumanLayoutOptions::exact(),
            &mut workspace,
        )
        .ok()?;

    let kept = lines.first()?.text().trim_end().to_string();
    let budget = width.saturating_sub(1);

    // Prose breaks at whitespace, but a single unbroken token — a path, a hash,
    // an identifier — offers nowhere to break and comes back whole. Falling back
    // to a break opportunity between every character keeps the cut inside
    // Laidout, so the width that decides it is the width Laidout measured.
    let kept = if unicode_width::UnicodeWidthStr::width(kept.as_str()) > budget {
        first_line(&per_character(text), budget, theme)?
    } else {
        kept
    };
    if kept.is_empty() {
        return None;
    }
    Some(format!("{kept}\u{2026}"))
}

/// A document that may break between any two characters.
///
/// `choice` is decided per position, unlike `soft_line`, which is always taken
/// outside a flat group. Preferring the empty alternative means a break costs
/// nothing until the line is full.
fn per_character(text: &str) -> Doc {
    Doc::concat(text.chars().flat_map(|ch| {
        [
            Doc::text(ch.to_string()),
            Doc::choice(Doc::empty(), Doc::hard_line()),
        ]
    }))
}

/// Render `doc` at `width` and return its first line.
///
/// Uses the cost-minimizing strategy: the fast strategy decides fits in constant
/// time and so takes every break it is offered, which would put one character on
/// each line.
fn first_line(doc: &Doc, width: usize, theme: &Theme) -> Option<String> {
    let columns = NonZeroU16::new(u16::try_from(width).unwrap_or(u16::MAX).max(1))?;
    let prepared = doc.prepare().ok()?;
    let mut workspace = StyledRenderWorkspace::new();
    let lines = prepared
        .render_styled_into(
            columns,
            theme.semantic(),
            HumanLayoutOptions::exact(),
            &mut workspace,
        )
        .ok()?;
    Some(lines.first()?.text().trim_end().to_string())
}

/// Column at which `needle` starts within a laid-out row, in terminal cells.
///
/// Mouse targets need positions, and a row's fragments are chosen by Laidout
/// rather than placed by the caller, so positions are read back from the result.
pub fn column_of(spans: &[Span<'_>], needle: &str) -> Option<usize> {
    let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
    let byte = text.find(needle)?;
    Some(unicode_width::UnicodeWidthStr::width(&text[..byte]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens;

    fn variant(text: &str) -> Variant {
        Variant::new().push(tokens::FG, text)
    }

    fn text_of(row: &Row) -> String {
        row.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_densest_fitting_variant_is_chosen() {
        let theme = Theme::new(false);
        let ladder = [
            variant("Command | Palette | Help | Search"),
            variant("Cmd | Ctrl-P | ? | Search"),
            variant(": | / |"),
        ];

        assert_eq!(
            text_of(&fit(&ladder, 60, &theme)),
            "Command | Palette | Help | Search"
        );
        assert_eq!(
            text_of(&fit(&ladder, 30, &theme)),
            "Cmd | Ctrl-P | ? | Search"
        );
        assert_eq!(text_of(&fit(&ladder, 12, &theme)), ": | / |");
    }

    #[test]
    fn the_last_variant_is_used_even_when_nothing_fits() {
        let theme = Theme::new(false);
        let ladder = [variant("far too long to fit"), variant("still too long")];

        let row = fit(&ladder, 4, &theme);
        assert_eq!(
            text_of(&row),
            "still too long",
            "a chrome row must stay on one line for the surface to clip"
        );
    }

    #[test]
    fn width_is_measured_in_cells_not_bytes() {
        let theme = Theme::new(false);
        // Each of these glyphs is multi-byte but occupies one cell.
        let row = fit(&[variant("│ · … ▸")], 40, &theme);
        assert_eq!(row.width, 7, "four one-cell glyphs and three spaces");
    }

    #[test]
    fn justify_pushes_the_right_row_to_the_end() {
        let theme = Theme::new(false);
        let left = fit(&[variant("left")], 40, &theme);
        let right = fit(&[variant("right")], 40, &theme);

        let line = justify(left, right, 20);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.len(), 20);
        assert!(text.starts_with("left"));
        assert!(text.ends_with("right"));
    }

    #[test]
    fn a_column_is_reported_in_cells_past_multi_byte_glyphs() {
        let theme = Theme::new(false);
        let row = fit(&[variant("│ tcld │ [b] Browse")], 40, &theme);
        assert_eq!(column_of(&row.spans, "[b] Browse"), Some(9));
    }

    #[test]
    fn eliding_breaks_at_a_word_boundary() {
        let theme = Theme::new(false);
        let message = "exported generated source to /tmp/out.tcl";

        let short = elide(message, 24, &theme).expect("some text survives");
        assert!(short.ends_with('\u{2026}'), "elision is marked: {short:?}");
        assert!(
            unicode_width::UnicodeWidthStr::width(short.as_str()) <= 24,
            "elided text fits its budget: {short:?}"
        );
        assert!(
            !short
                .trim_end_matches('\u{2026}')
                .ends_with(char::is_alphanumeric)
                || message.starts_with(short.trim_end_matches('\u{2026}')),
            "elision keeps a prefix of the message: {short:?}"
        );
        assert!(
            short
                .trim_end_matches('\u{2026}')
                .trim_end()
                .split(' ')
                .all(|word| message.contains(word)),
            "every surviving word is a whole word: {short:?}"
        );
    }

    #[test]
    fn an_unbreakable_token_is_broken_between_characters() {
        let theme = Theme::new(false);
        // No whitespace anywhere, so there is no word boundary to break at and
        // the per-character fallback has to hold the budget.
        let token = "/very/long/path/without/any/whitespace/at/all/file.tcl";

        let short = elide(token, 20, &theme).expect("some text survives");
        let cells = unicode_width::UnicodeWidthStr::width(short.as_str());
        assert!(cells <= 20, "must fit: {short:?} is {cells} cells");
        assert_eq!(
            cells, 20,
            "and must fill the budget rather than breaking early: {short:?}"
        );
        assert!(short.ends_with('\u{2026}'));
        assert!(token.starts_with(short.trim_end_matches('\u{2026}')));
    }

    #[test]
    fn wide_characters_are_counted_as_two_cells() {
        let theme = Theme::new(false);
        let wide = "変数の値がとても長い場合のテスト";

        let short = elide(wide, 10, &theme).expect("some text survives");
        assert!(
            unicode_width::UnicodeWidthStr::width(short.as_str()) <= 10,
            "wide characters occupy two cells each: {short:?} is {} cells",
            unicode_width::UnicodeWidthStr::width(short.as_str())
        );
    }

    #[test]
    fn text_that_already_fits_is_returned_whole() {
        let theme = Theme::new(false);
        assert_eq!(elide("short", 40, &theme).as_deref(), Some("short"));
    }

    #[test]
    fn nothing_survives_a_budget_of_nothing() {
        let theme = Theme::new(false);
        assert_eq!(elide("anything", 0, &theme), None);
    }

    #[test]
    fn an_empty_ladder_lays_out_nothing() {
        let theme = Theme::new(false);
        assert!(fit(&[], 40, &theme).spans.is_empty());
        assert!(fit(&[Variant::new()], 40, &theme).spans.is_empty());
    }
}
