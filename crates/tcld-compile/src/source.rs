//! Exact source ranges for compiler-owned scripts and substitutions. Braced
//! data stays data; only statically identified script bodies are traversed.
use super::{Range, line_column, scan_words, word_literal};
use tcld_syntax::{TclCommandKind, TclCommandSpan};

pub(super) use super::words::script_ranges;

pub(super) fn command_spans(
    source: &str,
    procedure_context: bool,
) -> Result<Vec<TclCommandSpan>, String> {
    let mut spans = Vec::new();
    visit_script(source, 0..source.len(), &mut spans, procedure_context)?;
    spans.sort_by(|a, b| {
        a.start_byte
            .cmp(&b.start_byte)
            .then_with(|| b.end_byte.cmp(&a.end_byte))
    });
    Ok(spans)
}

fn visit_script(
    source: &str,
    script: Range<usize>,
    out: &mut Vec<TclCommandSpan>,
    procedure_context: bool,
) -> Result<(), String> {
    for relative in script_ranges(&source[script.clone()])? {
        let span = script.start + relative.start..script.start + relative.end;
        let text = &source[span.clone()];
        let words = scan_words(text)?;
        let name = words
            .first()
            .and_then(|w| word_literal(text, w.clone()).ok());
        let (start_row, start_column) = line_column(source, span.start);
        let (end_row, end_column) = line_column(source, span.end);
        out.push(TclCommandSpan {
            kind: if name.as_deref() == Some("proc") {
                TclCommandKind::Procedure
            } else {
                TclCommandKind::Command
            },
            name: name.clone(),
            text: text.into(),
            start_byte: span.start,
            end_byte: span.end,
            start_row: (start_row - 1) as usize,
            start_column: (start_column - 1) as usize,
            end_row: (end_row - 1) as usize,
            end_column: (end_column - 1) as usize,
        });
        for word in &words {
            let start = word.start
                + if super::expanded_word(&text[word.clone()]) {
                    3
                } else {
                    0
                };
            if !text[start..word.end].starts_with('{') {
                visit_substitutions(
                    source,
                    span.start + start..span.start + word.end,
                    out,
                    false,
                    procedure_context,
                )?;
            }
        }
        // Deferred control invocations own only their argument substitutions;
        // their script bodies acquire maps in the runtime-generated image.
        if super::runtime_command(
            text,
            &words,
            if script.start == 0 { 0 } else { 1 },
            script.start == 0,
            false,
            procedure_context,
        ) {
            continue;
        }
        let bodies: Vec<usize> = match name.as_deref() {
            Some("proc") => vec![3],
            Some("for") => vec![1, 3, 4],
            Some("while") => vec![2],
            Some("foreach" | "lmap" | "::lmap") => vec![words.len().saturating_sub(1)],
            Some("catch") => vec![1],
            Some("dict" | "::dict") => match super::dicts::dict_form(text, &words) {
                Some(
                    super::dicts::DictForm::For(..)
                    | super::dicts::DictForm::Map(..)
                    | super::dicts::DictForm::Update(..)
                    | super::dicts::DictForm::With(_),
                ) => vec![words.len() - 1],
                _ => Vec::new(),
            },
            Some("if") => {
                let mut bodies = Vec::new();
                let mut i = 1;
                while i < words.len() {
                    visit_expression_word(
                        source,
                        text,
                        span.start,
                        &words[i],
                        out,
                        procedure_context,
                    )?;
                    i += 1;
                    if words
                        .get(i)
                        .and_then(|w| word_literal(text, w.clone()).ok())
                        .as_deref()
                        == Some("then")
                    {
                        i += 1;
                    }
                    bodies.push(i);
                    i += 1;
                    match words
                        .get(i)
                        .and_then(|w| word_literal(text, w.clone()).ok())
                        .as_deref()
                    {
                        Some("elseif") => i += 1,
                        Some("else") => {
                            bodies.push(i + 1);
                            break;
                        }
                        _ => break,
                    }
                }
                bodies
            }
            _ => Vec::new(),
        };
        for index in bodies {
            if let Some(word) = words.get(index) {
                let raw = &text[word.clone()];
                if raw.starts_with('{') && raw.ends_with('}') {
                    visit_script(
                        source,
                        span.start + word.start + 1..span.start + word.end - 1,
                        out,
                        procedure_context || name.as_deref() == Some("proc"),
                    )?;
                } else if super::bare_script_word(text, word) {
                    visit_script(
                        source,
                        span.start + word.start..span.start + word.end,
                        out,
                        procedure_context,
                    )?;
                } else if matches!(name.as_deref(), Some("lmap" | "::lmap")) && raw.starts_with('"')
                {
                    visit_script(
                        source,
                        span.start + word.start + 1..span.start + word.end - 1,
                        out,
                        procedure_context,
                    )?;
                }
            }
        }
        let expression_index = match name.as_deref() {
            Some("while") => Some(1),
            Some("for") => Some(2),
            Some("expr") if words.len() == 2 => Some(1),
            _ => None,
        };
        if let Some(index) = expression_index.and_then(|i| words.get(i)) {
            visit_expression_word(source, text, span.start, index, out, procedure_context)?;
        }
    }
    Ok(())
}
fn visit_expression_word(
    source: &str,
    text: &str,
    base: usize,
    word: &Range<usize>,
    out: &mut Vec<TclCommandSpan>,
    procedure_context: bool,
) -> Result<(), String> {
    if text[word.clone()].starts_with('{') {
        visit_substitutions(
            source,
            base + word.start + 1..base + word.end - 1,
            out,
            true,
            procedure_context,
        )?;
    }
    Ok(())
}
/// The command substitutions in a word, found as `compile_word` compiles
/// them: inside array indices too, but not inside braced variable names. In
/// an `expression` a braced string is a literal, so it holds none, and a
/// quoted string substitutes like a word.
fn visit_substitutions(
    source: &str,
    span: Range<usize>,
    out: &mut Vec<TclCommandSpan>,
    expression: bool,
    procedure_context: bool,
) -> Result<(), String> {
    let text = &source[span.clone()];
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if expression => i = super::words::braced_end(bytes, i)?,
            b'"' if expression => {
                let end = super::words::quoted_end(bytes, i)?;
                visit_substitutions(
                    source,
                    span.start + i..span.start + end,
                    out,
                    false,
                    procedure_context,
                )?;
                i = end;
            }
            b'\\' => i = (i + 2).min(bytes.len()),
            b'$' => match super::words::variable(text, i)? {
                Some(reference) => {
                    if let Some(index) = reference.index {
                        let index = span.start + index.start..span.start + index.end;
                        visit_substitutions(source, index, out, false, procedure_context)?;
                    }
                    i = reference.end;
                }
                None => i += 1,
            },
            b'[' => {
                let close = super::matching_bracket(text, i)?;
                visit_script(
                    source,
                    span.start + i + 1..span.start + close,
                    out,
                    procedure_context,
                )?;
                i = close + 1;
            }
            _ => i += 1,
        }
    }
    Ok(())
}
