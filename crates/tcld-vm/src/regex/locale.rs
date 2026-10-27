/*
 * regc_locale.c --
 *
 *	This file contains the Unicode locale specific regexp routines.
 *	This file is #included by regcomp.c.
 *
 * Copyright © 1998 Scriptics Corporation.
 *
 * See the file "license.terms" for information on usage and redistribution of
 * this file, and for a DISCLAIMER OF ALL WARRANTIES.
 */

// Ported from Tcl 9.0.2 generic/regc_locale.c. Modified for Tcld: native Rust
// data structures and VM integration. Tcl licensing terms: license.terms.

//! Character data for the compiler (`regc_locale.c` and `regc_cvec.c`):
//! collating-element names, the `[:class:]` contents, case counterparts,
//! and the character predicates and case mappings of `regcustom.h`, which
//! are Tcl's own (`Tcl_UniCharIsAlnum`, `Tcl_UniCharToLower`, ...).

use super::Chr;
use super::tables::{
    ALPHA_CHAR_TABLE, ALPHA_RANGE_TABLE, CONTROL_CHAR_TABLE, CONTROL_RANGE_TABLE,
    DIGIT_RANGE_TABLE, GRAPH_CHAR_TABLE, GRAPH_RANGE_TABLE, LOWER_CHAR_TABLE, LOWER_RANGE_TABLE,
    PUNCT_CHAR_TABLE, PUNCT_RANGE_TABLE, SPACE_CHAR_TABLE, SPACE_RANGE_TABLE, UPPER_CHAR_TABLE,
    UPPER_RANGE_TABLE,
};
use crate::unicode::{self, CharClass};

/// A set of characters and character ranges (`struct cvec`).
#[derive(Debug, Default, Clone)]
pub(super) struct Cvec {
    pub chrs: Vec<Chr>,
    pub ranges: Vec<(Chr, Chr)>,
}

/// The ASCII character names `[.name.]` accepts.
const CNAMES: &[(&str, u8)] = &[
    ("NUL", 0x00),
    ("SOH", 0x01),
    ("STX", 0x02),
    ("ETX", 0x03),
    ("EOT", 0x04),
    ("ENQ", 0x05),
    ("ACK", 0x06),
    ("BEL", 0x07),
    ("alert", 0x07),
    ("BS", 0x08),
    ("backspace", 0x08),
    ("HT", 0x09),
    ("tab", 0x09),
    ("LF", 0x0A),
    ("newline", 0x0A),
    ("VT", 0x0B),
    ("vertical-tab", 0x0B),
    ("FF", 0x0C),
    ("form-feed", 0x0C),
    ("CR", 0x0D),
    ("carriage-return", 0x0D),
    ("SO", 0x0E),
    ("SI", 0x0F),
    ("DLE", 0x10),
    ("DC1", 0x11),
    ("DC2", 0x12),
    ("DC3", 0x13),
    ("DC4", 0x14),
    ("NAK", 0x15),
    ("SYN", 0x16),
    ("ETB", 0x17),
    ("CAN", 0x18),
    ("EM", 0x19),
    ("SUB", 0x1A),
    ("ESC", 0x1B),
    ("IS4", 0x1C),
    ("FS", 0x1C),
    ("IS3", 0x1D),
    ("GS", 0x1D),
    ("IS2", 0x1E),
    ("RS", 0x1E),
    ("IS1", 0x1F),
    ("US", 0x1F),
    ("space", b' '),
    ("exclamation-mark", b'!'),
    ("quotation-mark", b'"'),
    ("number-sign", b'#'),
    ("dollar-sign", b'$'),
    ("percent-sign", b'%'),
    ("ampersand", b'&'),
    ("apostrophe", b'\''),
    ("left-parenthesis", b'('),
    ("right-parenthesis", b')'),
    ("asterisk", b'*'),
    ("plus-sign", b'+'),
    ("comma", b','),
    ("hyphen", b'-'),
    ("hyphen-minus", b'-'),
    ("period", b'.'),
    ("full-stop", b'.'),
    ("slash", b'/'),
    ("solidus", b'/'),
    ("zero", b'0'),
    ("one", b'1'),
    ("two", b'2'),
    ("three", b'3'),
    ("four", b'4'),
    ("five", b'5'),
    ("six", b'6'),
    ("seven", b'7'),
    ("eight", b'8'),
    ("nine", b'9'),
    ("colon", b':'),
    ("semicolon", b';'),
    ("less-than-sign", b'<'),
    ("equals-sign", b'='),
    ("greater-than-sign", b'>'),
    ("question-mark", b'?'),
    ("commercial-at", b'@'),
    ("left-square-bracket", b'['),
    ("backslash", b'\\'),
    ("reverse-solidus", b'\\'),
    ("right-square-bracket", b']'),
    ("circumflex", b'^'),
    ("circumflex-accent", b'^'),
    ("underscore", b'_'),
    ("low-line", b'_'),
    ("grave-accent", b'`'),
    ("left-brace", b'{'),
    ("left-curly-bracket", b'{'),
    ("vertical-line", b'|'),
    ("right-brace", b'}'),
    ("right-curly-bracket", b'}'),
    ("tilde", b'~'),
    ("DEL", 0x7F),
];

/// Whether a name written in the pattern is `entry`: Tcl compares the
/// name's UTF-8 form byte for byte, so only ASCII names can match.
fn name_is(name: &[Chr], entry: &str) -> bool {
    name.len() == entry.len()
        && name
            .iter()
            .zip(entry.bytes())
            .all(|(&c, b)| c == Chr::from(b))
}

/// The character a multi-character collating-element name stands for.
pub(super) fn cname(name: &[Chr]) -> Option<Chr> {
    CNAMES
        .iter()
        .find(|(entry, _)| name_is(name, entry))
        .map(|&(_, code)| Chr::from(code))
}

/// `cclass`: the contents of `[:name:]`, or `None` for an unknown class.
/// Case-independent `lower` and `upper` mean `alnum`, as in Tcl.
pub(super) fn cclass(name: &[Chr], cases: bool) -> Option<Cvec> {
    const CLASSES: [&str; 13] = [
        "alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower", "print", "punct",
        "space", "upper", "xdigit",
    ];
    let mut class = *CLASSES.iter().find(|entry| name_is(name, entry))?;
    if cases && (class == "lower" || class == "upper") {
        class = "alnum";
    }
    let mut cv = Cvec::default();
    let ranges = |cv: &mut Cvec, table: &[(Chr, Chr)]| cv.ranges.extend_from_slice(table);
    let chars = |cv: &mut Cvec, table: &[Chr]| cv.chrs.extend_from_slice(table);
    match class {
        "alnum" => {
            chars(&mut cv, &ALPHA_CHAR_TABLE);
            ranges(&mut cv, &ALPHA_RANGE_TABLE);
            ranges(&mut cv, &DIGIT_RANGE_TABLE);
        }
        "alpha" => {
            ranges(&mut cv, &ALPHA_RANGE_TABLE);
            chars(&mut cv, &ALPHA_CHAR_TABLE);
        }
        "ascii" => cv.ranges.push((0, 0x7F)),
        "blank" => cv.chrs.extend([Chr::from(b'\t'), Chr::from(b' ')]),
        "cntrl" => {
            ranges(&mut cv, &CONTROL_RANGE_TABLE);
            chars(&mut cv, &CONTROL_CHAR_TABLE);
        }
        "digit" => ranges(&mut cv, &DIGIT_RANGE_TABLE),
        "punct" => {
            ranges(&mut cv, &PUNCT_RANGE_TABLE);
            chars(&mut cv, &PUNCT_CHAR_TABLE);
        }
        "xdigit" => cv.ranges.extend([
            (Chr::from(b'0'), Chr::from(b'9')),
            (Chr::from(b'a'), Chr::from(b'f')),
            (Chr::from(b'A'), Chr::from(b'F')),
        ]),
        "space" => {
            ranges(&mut cv, &SPACE_RANGE_TABLE);
            chars(&mut cv, &SPACE_CHAR_TABLE);
        }
        "lower" => {
            ranges(&mut cv, &LOWER_RANGE_TABLE);
            chars(&mut cv, &LOWER_CHAR_TABLE);
        }
        "upper" => {
            ranges(&mut cv, &UPPER_RANGE_TABLE);
            chars(&mut cv, &UPPER_CHAR_TABLE);
        }
        "print" => {
            // Spaces other than the control characters, and the graphics.
            ranges(&mut cv, &SPACE_RANGE_TABLE[1..]);
            chars(&mut cv, &SPACE_CHAR_TABLE);
            ranges(&mut cv, &GRAPH_RANGE_TABLE);
            chars(&mut cv, &GRAPH_CHAR_TABLE);
        }
        "graph" => {
            ranges(&mut cv, &GRAPH_RANGE_TABLE);
            chars(&mut cv, &GRAPH_CHAR_TABLE);
        }
        _ => unreachable!("character class {class}"),
    }
    Some(cv)
}

/// `range` once the range is known legal: a range as is, or with
/// `cases` every character with its case counterparts.
pub(super) fn range(a: Chr, b: Chr, cases: bool) -> Cvec {
    let mut cv = Cvec::default();
    if !cases {
        cv.ranges.push((a, b));
        return cv;
    }
    for c in a..=b {
        cv.chrs.push(c);
        let (lc, uc, tc) = (to_lower(c), to_upper(c), to_title(c));
        if c != lc {
            cv.chrs.push(lc);
        }
        if c != uc {
            cv.chrs.push(uc);
        }
        if c != tc && tc != uc {
            cv.chrs.push(tc);
        }
    }
    cv
}

/// `allcases`: a character's case counterparts, itself included.
pub(super) fn allcases(c: Chr) -> Cvec {
    let (lc, uc, tc) = (to_lower(c), to_upper(c), to_title(c));
    let mut cv = Cvec::default();
    if tc != uc {
        cv.chrs.push(tc);
    }
    cv.chrs.push(lc);
    if lc != uc {
        cv.chrs.push(uc);
    }
    cv
}

/// `casecmp`'s test: whether two equal-length runs match ignoring case.
pub(super) fn equal_nocase(x: &[Chr], y: &[Chr]) -> bool {
    x.iter()
        .zip(y)
        .all(|(&a, &b)| a == b || to_lower(a) == to_lower(b))
}

fn map(c: Chr, f: fn(char) -> char) -> Chr {
    char::from_u32(c).map_or(c, |ch| Chr::from(f(ch)))
}

pub(super) fn to_lower(c: Chr) -> Chr {
    map(c, unicode::to_lower)
}

pub(super) fn to_upper(c: Chr) -> Chr {
    map(c, unicode::to_upper)
}

pub(super) fn to_title(c: Chr) -> Chr {
    map(c, unicode::to_title)
}

fn is(class: CharClass, c: Chr) -> bool {
    char::from_u32(c).is_some_and(|ch| unicode::is_class(class, ch))
}

/// `iscalnum`.
pub(super) fn iscalnum(c: Chr) -> bool {
    is(CharClass::Alnum, c)
}

/// `iscalpha`.
pub(super) fn iscalpha(c: Chr) -> bool {
    is(CharClass::Alpha, c)
}

/// `iscdigit`.
pub(super) fn iscdigit(c: Chr) -> bool {
    is(CharClass::Digit, c)
}

/// `iscspace`.
pub(super) fn iscspace(c: Chr) -> bool {
    is(CharClass::Space, c)
}
