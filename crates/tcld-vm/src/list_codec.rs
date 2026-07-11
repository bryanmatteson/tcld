//! Counted-byte Tcl list parser and serializer.
//!
//! This implements Tcl list syntax only: braces/quotes group elements and
//! backslashes substitute bytes, but `$` and `[...]` are ordinary list data.

use crate::error::VmError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TclListError {
    UnterminatedBrace,
    UnterminatedQuote,
    TrailingBackslash,
    NonWhitespaceAfterBrace,
    NonWhitespaceAfterQuote,
    UnexpectedClosingBrace,
    InvalidUnicodeEscape,
}

impl std::fmt::Display for TclListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::UnterminatedBrace => "unmatched open brace in list",
            Self::UnterminatedQuote => "unmatched open quote in list",
            Self::TrailingBackslash => "trailing backslash in list",
            Self::NonWhitespaceAfterBrace => {
                "list element in braces followed by non-space character"
            }
            Self::NonWhitespaceAfterQuote => {
                "list element in quotes followed by non-space character"
            }
            Self::UnexpectedClosingBrace => "unmatched close brace in list",
            Self::InvalidUnicodeEscape => "invalid Unicode escape in list",
        };
        f.write_str(message)
    }
}

impl std::error::Error for TclListError {}

impl From<TclListError> for VmError {
    fn from(value: TclListError) -> Self {
        Self::TclError {
            message: value.to_string(),
            error_info: None,
            error_code: Some("TCL VALUE LIST".into()),
        }
    }
}

pub struct TclListCodec;

impl TclListCodec {
    pub fn parse(bytes: &[u8]) -> Result<Vec<Vec<u8>>, TclListError> {
        let mut elements = Vec::new();
        let mut cursor = 0usize;
        while cursor < bytes.len() {
            while cursor < bytes.len() && is_list_space(bytes[cursor]) {
                cursor += 1;
            }
            if cursor == bytes.len() {
                break;
            }
            if bytes[cursor] == b'}' {
                return Err(TclListError::UnexpectedClosingBrace);
            }

            let (element, next) = match bytes[cursor] {
                b'{' => parse_braced(bytes, cursor)?,
                b'"' => parse_quoted(bytes, cursor)?,
                _ => parse_bare(bytes, cursor)?,
            };
            elements.push(element);
            cursor = next;
        }
        Ok(elements)
    }

    pub fn format<I, B>(elements: I) -> Vec<u8>
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
    {
        let mut out = Vec::new();
        for element in elements {
            if !out.is_empty() {
                out.push(b' ');
            }
            append_element(&mut out, element.as_ref());
        }
        out
    }
}

fn parse_braced(bytes: &[u8], start: usize) -> Result<(Vec<u8>, usize), TclListError> {
    let mut cursor = start + 1;
    let mut depth = 1usize;
    let mut out = Vec::new();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => {
                out.push(b'\\');
                cursor += 1;
                if cursor < bytes.len() {
                    out.push(bytes[cursor]);
                    cursor += 1;
                }
            }
            b'{' => {
                depth += 1;
                out.push(b'{');
                cursor += 1;
            }
            b'}' => {
                depth -= 1;
                cursor += 1;
                if depth == 0 {
                    if cursor < bytes.len() && !is_list_space(bytes[cursor]) {
                        return Err(TclListError::NonWhitespaceAfterBrace);
                    }
                    return Ok((out, cursor));
                }
                out.push(b'}');
            }
            byte => {
                out.push(byte);
                cursor += 1;
            }
        }
    }
    Err(TclListError::UnterminatedBrace)
}

fn parse_quoted(bytes: &[u8], start: usize) -> Result<(Vec<u8>, usize), TclListError> {
    let mut cursor = start + 1;
    let mut out = Vec::new();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => {
                cursor += 1;
                if cursor < bytes.len() && !is_list_space(bytes[cursor]) {
                    return Err(TclListError::NonWhitespaceAfterQuote);
                }
                return Ok((out, cursor));
            }
            b'\\' => append_backslash_substitution(bytes, &mut cursor, &mut out)?,
            byte => {
                out.push(byte);
                cursor += 1;
            }
        }
    }
    Err(TclListError::UnterminatedQuote)
}

fn parse_bare(bytes: &[u8], start: usize) -> Result<(Vec<u8>, usize), TclListError> {
    let mut cursor = start;
    let mut out = Vec::new();
    while cursor < bytes.len() && !is_list_space(bytes[cursor]) {
        if bytes[cursor] == b'\\' {
            append_backslash_substitution(bytes, &mut cursor, &mut out)?;
        } else {
            out.push(bytes[cursor]);
            cursor += 1;
        }
    }
    Ok((out, cursor))
}

fn append_backslash_substitution(
    bytes: &[u8],
    cursor: &mut usize,
    out: &mut Vec<u8>,
) -> Result<(), TclListError> {
    debug_assert_eq!(bytes[*cursor], b'\\');
    *cursor += 1;
    if *cursor >= bytes.len() {
        return Err(TclListError::TrailingBackslash);
    }

    let escaped = bytes[*cursor];
    *cursor += 1;
    match escaped {
        b'a' => out.push(0x07),
        b'b' => out.push(0x08),
        b'f' => out.push(0x0c),
        b'n' => out.push(b'\n'),
        b'r' => out.push(b'\r'),
        b't' => out.push(b'\t'),
        b'v' => out.push(0x0b),
        b'\n' => {
            while *cursor < bytes.len() && matches!(bytes[*cursor], b' ' | b'\t') {
                *cursor += 1;
            }
            out.push(b' ');
        }
        b'x' => append_radix_escape(bytes, cursor, out, 16, usize::MAX, b'x')?,
        b'u' => append_unicode_escape(bytes, cursor, out, 4)?,
        b'U' => append_unicode_escape(bytes, cursor, out, 8)?,
        b'0'..=b'7' => {
            let mut value = (escaped - b'0') as u32;
            let mut count = 1usize;
            while count < 3 && *cursor < bytes.len() && matches!(bytes[*cursor], b'0'..=b'7') {
                value = value * 8 + (bytes[*cursor] - b'0') as u32;
                *cursor += 1;
                count += 1;
            }
            out.push((value & 0xff) as u8);
        }
        other => out.push(other),
    }
    Ok(())
}

fn append_radix_escape(
    bytes: &[u8],
    cursor: &mut usize,
    out: &mut Vec<u8>,
    radix: u32,
    max_digits: usize,
    fallback: u8,
) -> Result<(), TclListError> {
    let start = *cursor;
    let mut value = 0u32;
    let mut count = 0usize;
    while *cursor < bytes.len() && count < max_digits {
        let Some(digit) = (bytes[*cursor] as char).to_digit(radix) else {
            break;
        };
        value = value.saturating_mul(radix).saturating_add(digit);
        *cursor += 1;
        count += 1;
    }
    if *cursor == start {
        out.push(fallback);
    } else if value <= 0xff {
        out.push(value as u8);
    } else {
        let Some(ch) = char::from_u32(value) else {
            return Err(TclListError::InvalidUnicodeEscape);
        };
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
    Ok(())
}

fn append_unicode_escape(
    bytes: &[u8],
    cursor: &mut usize,
    out: &mut Vec<u8>,
    max_digits: usize,
) -> Result<(), TclListError> {
    let start = *cursor;
    let mut value = 0u32;
    let mut count = 0usize;
    while *cursor < bytes.len() && count < max_digits {
        let Some(digit) = (bytes[*cursor] as char).to_digit(16) else {
            break;
        };
        value = value.saturating_mul(16).saturating_add(digit);
        *cursor += 1;
        count += 1;
    }
    if *cursor == start {
        return Err(TclListError::InvalidUnicodeEscape);
    }
    let Some(ch) = char::from_u32(value) else {
        return Err(TclListError::InvalidUnicodeEscape);
    };
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    Ok(())
}

fn append_element(out: &mut Vec<u8>, element: &[u8]) {
    if element.is_empty() {
        out.extend_from_slice(b"{}");
        return;
    }
    if element.iter().copied().all(is_bare_safe) {
        out.extend_from_slice(element);
        return;
    }
    for &byte in element {
        match byte {
            b' ' => out.extend_from_slice(br"\ "),
            b'\t' => out.extend_from_slice(br"\t"),
            b'\n' => out.extend_from_slice(br"\n"),
            b'\r' => out.extend_from_slice(br"\r"),
            0x0c => out.extend_from_slice(br"\f"),
            0x0b => out.extend_from_slice(br"\v"),
            b'\\' => out.extend_from_slice(br"\\"),
            b'{' => out.extend_from_slice(br"\{"),
            b'}' => out.extend_from_slice(br"\}"),
            b'"' => out.extend_from_slice(br#"\""#),
            b'$' => out.extend_from_slice(br"\$"),
            b'[' => out.extend_from_slice(br"\["),
            b']' => out.extend_from_slice(br"\]"),
            b';' => out.extend_from_slice(br"\;"),
            other => out.push(other),
        }
    }
}

fn is_list_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn is_bare_safe(byte: u8) -> bool {
    !is_list_space(byte) && !matches!(byte, b'\\' | b'{' | b'}' | b'"' | b'$' | b'[' | b']' | b';')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Vec<Vec<u8>> {
        TclListCodec::parse(input).unwrap()
    }

    #[test]
    fn parses_all_element_forms() {
        assert_eq!(
            parse(br#"alpha {two words} "three words" four\ words {}"#),
            vec![
                b"alpha".to_vec(),
                b"two words".to_vec(),
                b"three words".to_vec(),
                b"four words".to_vec(),
                Vec::new(),
            ]
        );
    }

    #[test]
    fn braces_nest_and_preserve_backslashes() {
        assert_eq!(
            parse(
                b"{a {b} \\
              c \\t}"
            ),
            vec![
                b"a {b} \\
              c \\t"
                    .to_vec()
            ]
        );
    }

    #[test]
    fn substitutes_backslash_sequences_outside_braces() {
        assert_eq!(
            parse(br#"a\tb \x41 \u263a \101"#),
            vec![
                b"a\tb".to_vec(),
                b"A".to_vec(),
                "☺".as_bytes().to_vec(),
                b"A".to_vec()
            ]
        );
    }

    #[test]
    fn rejects_malformed_lists() {
        assert_eq!(
            TclListCodec::parse(b"{open").unwrap_err(),
            TclListError::UnterminatedBrace
        );
        assert_eq!(
            TclListCodec::parse(b"\"open").unwrap_err(),
            TclListError::UnterminatedQuote
        );
        assert_eq!(
            TclListCodec::parse(b"{a}x").unwrap_err(),
            TclListError::NonWhitespaceAfterBrace
        );
        assert_eq!(
            TclListCodec::parse(b"\"a\"x").unwrap_err(),
            TclListError::NonWhitespaceAfterQuote
        );
        assert_eq!(
            TclListCodec::parse(b"word\\").unwrap_err(),
            TclListError::TrailingBackslash
        );
    }

    #[test]
    fn format_round_trips_arbitrary_counted_bytes() {
        let elements = vec![
            Vec::new(),
            b"plain".to_vec(),
            b"two words".to_vec(),
            b"a{b}\\c;$[d]".to_vec(),
            vec![0, 0xff, b' ', b'\n'],
        ];
        let encoded = TclListCodec::format(&elements);
        assert_eq!(TclListCodec::parse(&encoded).unwrap(), elements);
    }

    #[cfg(feature = "host")]
    #[test]
    fn parser_and_serializer_match_embedded_tcl() {
        let interp = tcld_interp::Interpreter::new().expect("Tcl interpreter");
        let cases: Vec<Vec<u8>> = vec![
            br#"alpha {two words} "three words" four\ words {}"#.to_vec(),
            b"{a {b} \\
              c \\t}"
                .to_vec(),
            br#"a\tb \x41 \u263a \101"#.to_vec(),
            TclListCodec::format([
                b"plain".as_slice(),
                b"two words".as_slice(),
                b"a{b}\\c;$[d]".as_slice(),
                &[0, 0xff, b' ', b'\n'],
            ]),
        ];
        for encoded in cases {
            let ours = TclListCodec::parse(&encoded).expect("our parser");
            let tcl = interp.split_list_bytes(&encoded).expect("Tcl parser");
            assert_eq!(ours, tcl, "encoded={encoded:?}");
        }
    }
}
