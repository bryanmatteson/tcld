//! Counted-byte Tcl list parser and serializer, ported from `tclUtil.c`
//! (`FindElement`, `TclCopyAndCollapse`, `TclScanElement`,
//! `TclConvertElement`) and `tclParse.c` (`TclParseBackslash`).
//!
//! This implements Tcl list syntax only: braces/quotes group elements and
//! backslashes substitute characters, but `$` and `[...]` are ordinary list
//! data. Dictionaries share the syntax and differ only in error messages.

use crate::error::VmError;

/// Malformed list or dictionary syntax (`FindElement`'s errors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TclListError {
    UnmatchedBrace,
    UnmatchedQuote,
    /// The characters after a closing brace, up to 20 bytes.
    JunkAfterBrace(Vec<u8>),
    /// The characters after a closing quote, up to 20 bytes.
    JunkAfterQuote(Vec<u8>),
}

/// What is being parsed, which names it in error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    List,
    Dict,
}

impl ListKind {
    fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Dict => "dict",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::List => "LIST",
            Self::Dict => "DICTIONARY",
        }
    }
}

impl TclListError {
    pub fn message(&self, kind: ListKind) -> String {
        let name = kind.name();
        match self {
            Self::UnmatchedBrace => format!("unmatched open brace in {name}"),
            Self::UnmatchedQuote => format!("unmatched open quote in {name}"),
            Self::JunkAfterBrace(junk) => format!(
                "{name} element in braces followed by \"{}\" instead of space",
                String::from_utf8_lossy(junk)
            ),
            Self::JunkAfterQuote(junk) => format!(
                "{name} element in quotes followed by \"{}\" instead of space",
                String::from_utf8_lossy(junk)
            ),
        }
    }

    pub fn into_vm_error(self, kind: ListKind) -> VmError {
        let detail = match self {
            Self::UnmatchedBrace => "BRACE",
            Self::UnmatchedQuote => "QUOTE",
            Self::JunkAfterBrace(_) | Self::JunkAfterQuote(_) => "JUNK",
        };
        VmError::TclError {
            message: self.message(kind),
            error_info: None,
            error_code: Some(format!("TCL VALUE {} {detail}", kind.code())),
        }
    }
}

impl std::fmt::Display for TclListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message(ListKind::List))
    }
}

impl std::error::Error for TclListError {}

impl From<TclListError> for VmError {
    fn from(value: TclListError) -> Self {
        value.into_vm_error(ListKind::List)
    }
}

pub struct TclListCodec;

impl TclListCodec {
    pub fn parse(bytes: &[u8]) -> Result<Vec<Vec<u8>>, TclListError> {
        let mut elements = Vec::new();
        let mut cursor = 0;
        while let Some((element, next)) = find_element(bytes, cursor)? {
            elements.push(element);
            cursor = next;
        }
        Ok(elements)
    }

    /// The canonical list string of `elements` (`UpdateStringOfList`): each
    /// element bare, brace-quoted or backslash-escaped as Tcl chooses, and a
    /// leading `#` quoted so the list cannot read as a comment.
    pub fn format<I, B>(elements: I) -> Vec<u8>
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
    {
        let mut out = Vec::new();
        for (index, element) in elements.into_iter().enumerate() {
            if index > 0 {
                out.push(b' ');
            }
            let element = element.as_ref();
            let quote_hash = index == 0;
            convert_element(
                &mut out,
                element,
                scan_element(element, quote_hash),
                quote_hash,
            );
        }
        out
    }
}

/// `FindElement` followed by `TclCopyAndCollapse`: the next element at or
/// after `start` and the offset just past the white space that follows it,
/// or `None` when only white space remains.
pub fn find_element(bytes: &[u8], start: usize) -> Result<Option<(Vec<u8>, usize)>, TclListError> {
    let limit = bytes.len();
    let mut p = start;
    while p < limit && is_list_space(bytes[p]) {
        p += 1;
    }
    if p == limit {
        return Ok(None);
    }
    let braced = bytes[p] == b'{';
    let quoted = bytes[p] == b'"';
    if braced || quoted {
        p += 1;
    }
    let element_start = p;
    let mut open_braces = usize::from(braced);
    let mut literal = true;
    let size = loop {
        let Some(&byte) = bytes.get(p) else {
            if open_braces != 0 {
                return Err(TclListError::UnmatchedBrace);
            }
            if quoted {
                return Err(TclListError::UnmatchedQuote);
            }
            break p - element_start;
        };
        match byte {
            b'{' if open_braces != 0 => open_braces += 1,
            b'}' if open_braces > 1 => open_braces -= 1,
            b'}' if open_braces == 1 => {
                let size = p - element_start;
                p += 1;
                if p >= limit || is_list_space(bytes[p]) {
                    break size;
                }
                return Err(TclListError::JunkAfterBrace(junk(bytes, p)));
            }
            b'\\' => {
                // Inside braces a backslash only shields the next
                // character; elsewhere it changes the element's value.
                if open_braces == 0 {
                    literal = false;
                }
                p += backslash(&bytes[p..]).1 - 1;
            }
            b'"' if quoted => {
                let size = p - element_start;
                p += 1;
                if p >= limit || is_list_space(bytes[p]) {
                    break size;
                }
                return Err(TclListError::JunkAfterQuote(junk(bytes, p)));
            }
            byte if is_list_space(byte) && open_braces == 0 && !quoted => {
                break p - element_start;
            }
            _ => {}
        }
        p += 1;
    };
    let raw = &bytes[element_start..element_start + size];
    let element = if literal { raw.to_vec() } else { collapse(raw) };
    while p < limit && is_list_space(bytes[p]) {
        p += 1;
    }
    Ok(Some((element, p)))
}

fn junk(bytes: &[u8], start: usize) -> Vec<u8> {
    bytes[start..]
        .iter()
        .take(20)
        .take_while(|byte| !is_list_space(**byte))
        .copied()
        .collect()
}

/// `TclCopyAndCollapse`: substitute every backslash sequence.
fn collapse(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' {
            let (value, read) = backslash(&raw[i..]);
            out.extend_from_slice(&value);
            i += read;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    out
}

/// `TclParseBackslash`: the UTF-8 value of the backslash sequence that
/// starts `src`, and how many bytes the sequence spans.
fn backslash(src: &[u8]) -> (Vec<u8>, usize) {
    let Some(&next) = src.get(1) else {
        return (b"\\".to_vec(), 1);
    };
    let (value, read) = match next {
        b'a' => (0x7, 2),
        b'b' => (0x8, 2),
        b'f' => (0xc, 2),
        b'n' => (0xa, 2),
        b'r' => (0xd, 2),
        b't' => (0x9, 2),
        b'v' => (0xb, 2),
        b'x' => match hex(&src[2..], 2) {
            (_, 0) => (u32::from(b'x'), 2),
            (value, digits) => (value & 0xff, 2 + digits),
        },
        b'u' => match hex(&src[2..], 4) {
            (_, 0) => (u32::from(b'u'), 2),
            (value, digits) => (value, 2 + digits),
        },
        b'U' => match hex(&src[2..], 8) {
            (_, 0) => (u32::from(b'U'), 2),
            (value, digits) => (value, 2 + digits),
        },
        b'\n' => {
            let blanks = src[2..]
                .iter()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count();
            (u32::from(b' '), 2 + blanks)
        }
        0 => return (b"\\".to_vec(), 1),
        b'0'..=b'7' => {
            let mut value = u32::from(next - b'0');
            let mut read = 2;
            if let Some(digit @ b'0'..=b'7') = src.get(2) {
                value = (value << 3) + u32::from(digit - b'0');
                read = 3;
                if let Some(digit @ b'0'..=b'7') = src.get(3).filter(|_| value < 0x20) {
                    value = ((value << 3) + u32::from(digit - b'0')) & 0xff;
                    read = 4;
                }
            }
            (value, read)
        }
        _ => {
            // A backslash before any other character yields that
            // character, keeping a multi-byte UTF-8 sequence whole.
            let width = utf8_width(next).min(src.len() - 1);
            return match std::str::from_utf8(&src[1..1 + width]) {
                Ok(text) => (text.as_bytes().to_vec(), 1 + width),
                Err(_) => (vec![next], 2),
            };
        }
    };
    (encode(value), read)
}

/// `ParseHex`: up to `max_digits` hex digits, never past U+10FFFF.
fn hex(src: &[u8], max_digits: usize) -> (u32, usize) {
    let mut value = 0u32;
    let mut digits = 0;
    for &byte in src.iter().take(max_digits) {
        let Some(digit) = (byte as char).to_digit(16) else {
            break;
        };
        if value > 0x10fff {
            break;
        }
        value = (value << 4) | digit;
        digits += 1;
    }
    (value, digits)
}

fn utf8_width(lead: u8) -> usize {
    match lead {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

/// UTF-8 for a code point; surrogates get the three-byte form Tcl writes.
fn encode(value: u32) -> Vec<u8> {
    match char::from_u32(value) {
        Some(ch) => ch.encode_utf8(&mut [0; 4]).as_bytes().to_vec(),
        None => vec![
            0xe0 | (value >> 12) as u8,
            0x80 | ((value >> 6) & 0x3f) as u8,
            0x80 | (value & 0x3f) as u8,
        ],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conversion {
    /// The element needs no quoting.
    None,
    /// Enclose the element in braces.
    Brace,
    /// Backslash-escape every special character.
    Escape,
    /// Backslash-escape every special character except braces.
    Mask,
}

/// `TclScanElement` with Tcl's compatibility preferences: brace-quote when
/// possible, but escape an element quoted only for `]` or `"`.
fn scan_element(src: &[u8], quote_hash: bool) -> Conversion {
    if src.is_empty() {
        return Conversion::Brace;
    }
    let mut nesting = 0isize;
    let mut forbid_none = false;
    let mut require_escape = false;
    let mut prefer_escape = false;
    let mut prefer_brace = quote_hash && src[0] == b'#';
    if matches!(src[0], b'{' | b'"') {
        forbid_none = true;
        prefer_brace = true;
    }
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'{' => nesting += 1,
            b'}' => {
                if nesting < 1 {
                    require_escape = true;
                }
                nesting -= 1;
            }
            b']' | b'"' => {
                forbid_none = true;
                prefer_escape = true;
            }
            b'[' | b'$' | b';' => {
                forbid_none = true;
                prefer_brace = true;
            }
            b'\\' => match src.get(i + 1) {
                // A final backslash or backslash-newline cannot be braced.
                None => require_escape = true,
                Some(b'\n') => {
                    require_escape = true;
                    i += 1;
                }
                Some(next) => {
                    if matches!(next, b'{' | b'}' | b'\\') {
                        i += 1;
                    }
                    forbid_none = true;
                    prefer_brace = true;
                }
            },
            byte if is_list_space(byte) => {
                forbid_none = true;
                prefer_brace = true;
            }
            _ => {}
        }
        i += 1;
    }
    if nesting > 0 || require_escape {
        Conversion::Escape
    } else if forbid_none && prefer_escape && !prefer_brace {
        Conversion::Mask
    } else if forbid_none {
        Conversion::Brace
    } else {
        Conversion::None
    }
}

/// `TclConvertElement`.
fn convert_element(out: &mut Vec<u8>, src: &[u8], mut conversion: Conversion, quote_hash: bool) {
    if src.is_empty() {
        out.extend_from_slice(b"{}");
        return;
    }
    let mut src = src;
    if quote_hash && src[0] == b'#' {
        if conversion == Conversion::Escape {
            out.extend_from_slice(br"\#");
            src = &src[1..];
        } else {
            conversion = Conversion::Brace;
        }
    }
    match conversion {
        Conversion::None => out.extend_from_slice(src),
        Conversion::Brace => {
            out.push(b'{');
            out.extend_from_slice(src);
            out.push(b'}');
        }
        Conversion::Escape | Conversion::Mask => {
            for &byte in src {
                match byte {
                    b']' | b'[' | b'$' | b';' | b' ' | b'\\' | b'"' => {
                        out.extend_from_slice(&[b'\\', byte]);
                    }
                    b'{' | b'}' if conversion == Conversion::Escape => {
                        out.extend_from_slice(&[b'\\', byte]);
                    }
                    0x0c => out.extend_from_slice(br"\f"),
                    b'\n' => out.extend_from_slice(br"\n"),
                    b'\r' => out.extend_from_slice(br"\r"),
                    b'\t' => out.extend_from_slice(br"\t"),
                    0x0b => out.extend_from_slice(br"\v"),
                    byte => out.push(byte),
                }
            }
        }
    }
}

fn is_list_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
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
            parse(br#"a\tb \x41 \u263a \101 \xff \777 a\ b\"#),
            vec![
                b"a\tb".to_vec(),
                b"A".to_vec(),
                "☺".as_bytes().to_vec(),
                b"A".to_vec(),
                "ÿ".as_bytes().to_vec(),
                b"?7".to_vec(),
                b"a b\\".to_vec(),
            ]
        );
    }

    #[test]
    fn bare_braces_and_quotes_are_data() {
        assert_eq!(
            parse(b"a } b\"c"),
            vec![b"a".to_vec(), b"}".to_vec(), b"b\"c".to_vec()]
        );
    }

    #[test]
    fn rejects_malformed_lists() {
        assert_eq!(
            TclListCodec::parse(b"{open").unwrap_err(),
            TclListError::UnmatchedBrace
        );
        assert_eq!(
            TclListCodec::parse(b"\"open").unwrap_err(),
            TclListError::UnmatchedQuote
        );
        let junk = TclListCodec::parse(b"{a}x y").unwrap_err();
        assert_eq!(junk, TclListError::JunkAfterBrace(b"x".to_vec()));
        assert_eq!(
            junk.message(ListKind::Dict),
            "dict element in braces followed by \"x\" instead of space"
        );
        assert_eq!(
            TclListCodec::parse(b"\"a\"x").unwrap_err(),
            TclListError::JunkAfterQuote(b"x".to_vec())
        );
    }

    #[test]
    fn formats_like_tcl() {
        for (elements, expected) in [
            (vec!["a", "b c"], "a {b c}"),
            (vec!["", "x"], "{} x"),
            (vec!["#a", "#b"], "{#a} #b"),
            (vec!["a]b", "a\"b"], "a\\]b a\\\"b"),
            (vec!["a{", "}b"], "a\\{ \\}b"),
            (vec!["a\\"], "a\\\\"),
            (vec!["{a}", "$x", "a;b"], "{{a}} {$x} {a;b}"),
            (vec!["a\nb"], "{a\nb}"),
            (vec!["a\\\nb"], "a\\\\\\nb"),
            (vec!["#{a"], "\\#\\{a"),
        ] {
            assert_eq!(
                String::from_utf8(TclListCodec::format(&elements)).unwrap(),
                expected,
                "{elements:?}"
            );
        }
    }

    #[test]
    fn format_round_trips_arbitrary_counted_bytes() {
        let elements = vec![
            Vec::new(),
            b"plain".to_vec(),
            b"two words".to_vec(),
            b"a{b}\\c;$[d]".to_vec(),
            vec![0, 0xff, b' ', b'\n'],
            b"}{".to_vec(),
            b"x\\".to_vec(),
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
            br#"a\tb \x41 \u263a \101 \xe9 \351 a\ b\"#.to_vec(),
            b"a } b\"c \\\n  d".to_vec(),
            TclListCodec::format([
                b"plain".as_slice(),
                b"two words".as_slice(),
                b"a{b}\\c;$[d]".as_slice(),
                b"#x".as_slice(),
                b"}{".as_slice(),
            ]),
        ];
        for encoded in cases {
            let ours = TclListCodec::parse(&encoded).expect("our parser");
            let tcl = interp.split_list_bytes(&encoded).expect("Tcl parser");
            assert_eq!(ours, tcl, "encoded={encoded:?}");
        }
        for elements in [
            vec![
                "a", "b c", "", "#x", "a]b", "a\"b", "a{", "}b", "x\\", "{y}",
            ],
            vec!["#first", "$v", "a;b", "tab\there", "nl\nx", "\\{", "q\"{"],
        ] {
            let ours = String::from_utf8(TclListCodec::format(&elements)).unwrap();
            let quoted = elements.iter().map(|element| {
                let mut word = String::from("\"");
                for ch in element.chars() {
                    match ch {
                        '\n' => word.push_str("\\n"),
                        '\t' => word.push_str("\\t"),
                        '\\' | '"' | '$' | '[' | ']' | '{' | '}' | ';' => {
                            word.push('\\');
                            word.push(ch);
                        }
                        ch => word.push(ch),
                    }
                }
                word.push('"');
                word
            });
            let script = format!("list {}", quoted.collect::<Vec<_>>().join(" "));
            let tcl = interp.eval(&script).unwrap();
            assert_eq!(ours, tcl, "{elements:?}");
        }
    }
}
