//! Tcl's character semantics: case mapping and the `string is` classes.
//!
//! This is a port of Tcl's own lookup logic (`generic/tclUtf.c`, plus the
//! `ascii` and `xdigit` helpers of `StringIsCmd` in `generic/tclCmdMZ.c`) over
//! Tcl's generated Unicode tables (`generic/tclUniData.c`, transcribed into
//! `unicode_data.rs` by `tools/gen_unicode_data.py`). The tables come from the
//! Tcl release named by [`TCL_PATCH_LEVEL`].
//!
//! It deliberately avoids Rust's `char` case and class APIs: Rust tracks a
//! newer Unicode version and applies full (one-to-many) case mappings, while
//! Tcl uses only the simple one-to-one mappings of its own tables.
//!
//! Two Tcl behaviors are easy to miss:
//!
//! * [`to_upper`], [`to_lower`] and [`to_title`] mirror `Tcl_UniCharToUpper`
//!   and friends and always apply the mapping. [`upper`], [`lower`] and
//!   [`title`] mirror `Tcl_UtfToUpper` and friends (`string toupper` and so
//!   on), which convert in place and therefore leave a character unchanged
//!   when its mapping needs more UTF-8 bytes: `string toupper ɐ` is `ɐ`, even
//!   though `Tcl_UniCharToUpper` maps U+0250 to U+2C6F.
//! * Code points past Tcl's tables (from U+323C0 in Tcl 9.0.2, so including
//!   planes 4 to 16) never change case and belong to no class, apart from
//!   Tcl's hard-coded ranges for `control` (U+E0001, U+E0020..=U+E007F) and
//!   for `graph` and `print` (U+E0100..=U+E01EF).

#![forbid(unsafe_code)]

use crate::unicode_data::category::{
    CLOSE_PUNCTUATION, COMBINING_SPACING_MARK, CONNECTOR_PUNCTUATION, CONTROL, CURRENCY_SYMBOL,
    DASH_PUNCTUATION, DECIMAL_DIGIT_NUMBER, ENCLOSING_MARK, FINAL_QUOTE_PUNCTUATION, FORMAT,
    INITIAL_QUOTE_PUNCTUATION, LETTER_NUMBER, LINE_SEPARATOR, LOWERCASE_LETTER, MATH_SYMBOL,
    MODIFIER_LETTER, MODIFIER_SYMBOL, NON_SPACING_MARK, OPEN_PUNCTUATION, OTHER_LETTER,
    OTHER_NUMBER, OTHER_PUNCTUATION, OTHER_SYMBOL, PARAGRAPH_SEPARATOR, SPACE_SEPARATOR,
    TITLECASE_LETTER, UPPERCASE_LETTER,
};
use crate::unicode_data::{GROUP_MAP, GROUPS, OFFSET_BITS, PAGE_MAP, UNICODE_LIMIT};

pub use crate::unicode_data::TCL_PATCH_LEVEL;

// Category sets, from `enum UnicodeCharacterCategoryMasks` in tclUtf.c. A set
// is tested by shifting it right by a character's category.
const ALPHA_BITS: u32 = (1 << UPPERCASE_LETTER)
    | (1 << LOWERCASE_LETTER)
    | (1 << TITLECASE_LETTER)
    | (1 << MODIFIER_LETTER)
    | (1 << OTHER_LETTER);
const CONTROL_BITS: u32 = (1 << CONTROL) | (1 << FORMAT);
const DIGIT_BITS: u32 = 1 << DECIMAL_DIGIT_NUMBER;
const SPACE_BITS: u32 = (1 << SPACE_SEPARATOR) | (1 << LINE_SEPARATOR) | (1 << PARAGRAPH_SEPARATOR);
const WORD_BITS: u32 = ALPHA_BITS | DIGIT_BITS | (1 << CONNECTOR_PUNCTUATION);
const PUNCT_BITS: u32 = (1 << CONNECTOR_PUNCTUATION)
    | (1 << DASH_PUNCTUATION)
    | (1 << OPEN_PUNCTUATION)
    | (1 << CLOSE_PUNCTUATION)
    | (1 << INITIAL_QUOTE_PUNCTUATION)
    | (1 << FINAL_QUOTE_PUNCTUATION)
    | (1 << OTHER_PUNCTUATION);
const GRAPH_BITS: u32 = WORD_BITS
    | PUNCT_BITS
    | (1 << NON_SPACING_MARK)
    | (1 << ENCLOSING_MARK)
    | (1 << COMBINING_SPACING_MARK)
    | (1 << LETTER_NUMBER)
    | (1 << OTHER_NUMBER)
    | (1 << MATH_SYMBOL)
    | (1 << CURRENCY_SYMBOL)
    | (1 << MODIFIER_SYMBOL)
    | (1 << OTHER_SYMBOL);

/// Tcl's "clear away extension bits" mask. It never changes a `char`.
const CHAR_MASK: u32 = 0x1F_FFFF;

/// `Tcl_UniCharToUpper`: the simple uppercase mapping of `c`.
pub fn to_upper(c: char) -> char {
    let mut ch = c as i32;
    if let Some(info) = char_info(c)
        && case_type(info) & 0x4 != 0
    {
        ch -= delta(info);
    }
    from_tcl(ch, c)
}

/// `Tcl_UniCharToLower`: the simple lowercase mapping of `c`.
pub fn to_lower(c: char) -> char {
    let mut ch = c as i32;
    if let Some(info) = char_info(c) {
        let mode = case_type(info);
        if mode & 0x2 != 0 && mode != 0x7 {
            ch += delta(info);
        }
    }
    from_tcl(ch, c)
}

/// `Tcl_UniCharToTitle`: the simple titlecase mapping of `c`.
pub fn to_title(c: char) -> char {
    let mut ch = c as i32;
    if let Some(info) = char_info(c) {
        let mode = case_type(info);
        if mode & 0x1 != 0 {
            // Subtract or add one depending on the original case.
            if mode != 0x7 {
                ch += if mode & 0x4 != 0 { -1 } else { 1 };
            }
        } else if mode == 0x4 {
            ch -= delta(info);
        }
    }
    from_tcl(ch, c)
}

/// `string toupper` on a whole string (`Tcl_UtfToUpper`).
///
/// A character whose uppercase form needs more UTF-8 bytes is kept as is.
pub fn upper(s: &str) -> String {
    s.chars().map(|c| shrink_only(c, to_upper(c))).collect()
}

/// `string tolower` on a whole string (`Tcl_UtfToLower`).
///
/// A character whose lowercase form needs more UTF-8 bytes is kept as is.
pub fn lower(s: &str) -> String {
    s.chars().map(|c| shrink_only(c, to_lower(c))).collect()
}

/// `string totitle` on a whole string (`Tcl_UtfToTitle`).
///
/// The first character is mapped to titlecase and the rest to lowercase,
/// except Georgian Mtavruli letters (U+1C90..=U+1CBF, which Tcl's source calls
/// Asomtavruli): they have no titlecase, so Tcl leaves them alone after the
/// first character. A character whose mapping needs more UTF-8 bytes is kept.
pub fn title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        out.push(shrink_only(first, to_title(first)));
    }
    for c in chars {
        let lowered = if (c as u32).wrapping_sub(0x1C90) >= 0x30 {
            to_lower(c)
        } else {
            c
        };
        out.push(shrink_only(c, lowered));
    }
    out
}

/// A per-character class of `string is`.
///
/// [`CharClass::ALL`] lists the classes in the order of Tcl's
/// `tclStringClassTable`, whose index is the operand of the `strclass`
/// instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CharClass {
    Alnum,
    Alpha,
    Ascii,
    Control,
    Digit,
    Graph,
    Lower,
    Print,
    Punct,
    Space,
    Upper,
    Wordchar,
    Xdigit,
}

/// Every class `string is` accepts, in the order of `isClasses` in
/// `StringIsCmd`. Abbreviations must be unique within this whole table.
const STRING_IS_CLASSES: [&str; 22] = [
    "alnum",
    "alpha",
    "ascii",
    "control",
    "boolean",
    "dict",
    "digit",
    "double",
    "entier",
    "false",
    "graph",
    "integer",
    "list",
    "lower",
    "print",
    "punct",
    "space",
    "true",
    "upper",
    "wideinteger",
    "wordchar",
    "xdigit",
];

impl CharClass {
    /// All classes, in `tclStringClassTable` (`strclass` operand) order.
    pub const ALL: [CharClass; 13] = [
        CharClass::Alnum,
        CharClass::Alpha,
        CharClass::Ascii,
        CharClass::Control,
        CharClass::Digit,
        CharClass::Graph,
        CharClass::Lower,
        CharClass::Print,
        CharClass::Punct,
        CharClass::Space,
        CharClass::Upper,
        CharClass::Wordchar,
        CharClass::Xdigit,
    ];

    /// Resolves a `string is` class argument the way `StringIsCmd` does
    /// (`Tcl_GetIndexFromObj` over all 22 classes): an exact name, or else a
    /// unique non-empty abbreviation such as `word` or `up`.
    ///
    /// Returns `None` when Tcl would reject the name (unknown or ambiguous,
    /// like `al`) and when it names a class that is not per-character, like
    /// `integer` or `boolean`.
    pub fn from_name(name: &str) -> Option<Self> {
        let mut found = None;
        let mut abbreviations = 0;
        for entry in STRING_IS_CLASSES {
            if entry == name {
                found = Some(entry);
                abbreviations = 1;
                break;
            }
            if entry.starts_with(name) {
                found = Some(entry);
                abbreviations += 1;
            }
        }
        if name.is_empty() || abbreviations != 1 {
            return None;
        }
        let entry = found?;
        Self::ALL.into_iter().find(|class| class.name() == entry)
    }

    /// The full class name, as `string is` documents it.
    pub const fn name(self) -> &'static str {
        match self {
            CharClass::Alnum => "alnum",
            CharClass::Alpha => "alpha",
            CharClass::Ascii => "ascii",
            CharClass::Control => "control",
            CharClass::Digit => "digit",
            CharClass::Graph => "graph",
            CharClass::Lower => "lower",
            CharClass::Print => "print",
            CharClass::Punct => "punct",
            CharClass::Space => "space",
            CharClass::Upper => "upper",
            CharClass::Wordchar => "wordchar",
            CharClass::Xdigit => "xdigit",
        }
    }
}

/// Whether `c` belongs to `class`, as `string is` tests each character.
///
/// Mirrors `Tcl_UniCharIsAlnum`, `Tcl_UniCharIsAlpha`, `Tcl_UniCharIsControl`,
/// `Tcl_UniCharIsDigit`, `Tcl_UniCharIsGraph`, `Tcl_UniCharIsLower`,
/// `Tcl_UniCharIsPrint`, `Tcl_UniCharIsPunct`, `Tcl_UniCharIsSpace`,
/// `Tcl_UniCharIsUpper` and `Tcl_UniCharIsWordChar`, plus `UniCharIsAscii` and
/// `UniCharIsHexDigit` from tclCmdMZ.c, which accept only ASCII.
pub fn is_class(class: CharClass, c: char) -> bool {
    let ch = c as u32;
    match class {
        CharClass::Alnum => in_categories(ALPHA_BITS | DIGIT_BITS, c),
        CharClass::Alpha => in_categories(ALPHA_BITS, c),
        CharClass::Ascii => ch < 0x80,
        CharClass::Control => match category(c) {
            Some(cat) => (CONTROL_BITS >> cat) & 1 != 0,
            None => ch == 0xE0001 || ch.wrapping_sub(0xE0020) <= 0x5F,
        },
        CharClass::Digit => category(c) == Some(DECIMAL_DIGIT_NUMBER),
        CharClass::Graph => match category(c) {
            Some(cat) => (GRAPH_BITS >> cat) & 1 != 0,
            None => ch.wrapping_sub(0xE0100) <= 0xEF,
        },
        CharClass::Lower => category(c) == Some(LOWERCASE_LETTER),
        CharClass::Print => match category(c) {
            Some(cat) => ((GRAPH_BITS | SPACE_BITS) >> cat) & 1 != 0,
            None => ch.wrapping_sub(0xE0100) <= 0xEF,
        },
        CharClass::Punct => in_categories(PUNCT_BITS, c),
        CharClass::Space => {
            if ch < 0x80 {
                // TclIsSpaceProcM: the whitespace that separates Tcl words.
                matches!(ch, 0x09..=0x0D | 0x20)
            } else if ch >= UNICODE_LIMIT {
                false
            } else if matches!(ch, 0x0085 | 0x180E | 0x200B | 0x202F | 0x2060 | 0xFEFF) {
                true
            } else {
                in_categories(SPACE_BITS, c)
            }
        }
        CharClass::Upper => category(c) == Some(UPPERCASE_LETTER),
        CharClass::Wordchar => in_categories(WORD_BITS, c),
        CharClass::Xdigit => c.is_ascii_hexdigit(),
    }
}

/// `GetUniCharInfo(ch)`, or `None` when `UNICODE_OUT_OF_RANGE(ch)`.
fn char_info(c: char) -> Option<i32> {
    let ch = (c as u32) & CHAR_MASK;
    if ch >= UNICODE_LIMIT {
        return None;
    }
    let page = usize::from(PAGE_MAP[(ch >> OFFSET_BITS) as usize]);
    let offset = (ch & ((1 << OFFSET_BITS) - 1)) as usize;
    Some(GROUPS[usize::from(GROUP_MAP[page | offset])])
}

/// `GetCaseType(info)`.
fn case_type(info: i32) -> i32 {
    (info & 0xE0) >> 5
}

/// `GetDelta(info)`: an arithmetic shift, so the delta keeps its sign.
fn delta(info: i32) -> i32 {
    info >> 8
}

/// `GetCategory(ch)`, or `None` when `UNICODE_OUT_OF_RANGE(ch)`.
fn category(c: char) -> Option<u32> {
    char_info(c).map(|info| (info & 0x1F) as u32)
}

/// Whether `c`'s category is in `mask`. Out-of-range code points never are.
fn in_categories(mask: u32, c: char) -> bool {
    category(c).is_some_and(|cat| (mask >> cat) & 1 != 0)
}

/// The final `ch & 0x1FFFFF` of the case functions. The generator checks that
/// every mapping is a Unicode scalar value, so the fallback is unreachable.
fn from_tcl(ch: i32, original: char) -> char {
    char::from_u32((ch as u32) & CHAR_MASK).unwrap_or(original)
}

/// `TclUtfCount`: the bytes of `c` in Tcl's internal UTF-8, where NUL takes
/// two (`C0 80`).
fn tcl_utf_count(c: char) -> usize {
    let ch = c as u32;
    if ch.wrapping_sub(1) < 0x7F {
        1
    } else if ch <= 0x7FF {
        2
    } else if ch.wrapping_sub(0x1_0000) <= 0xF_FFFF {
        4
    } else {
        3
    }
}

/// `Tcl_UtfToUpper` and friends convert in place: a mapping that needs more
/// bytes than the original character is not applied.
fn shrink_only(original: char, mapped: char) -> char {
    if tcl_utf_count(original) < tcl_utf_count(mapped) {
        original
    } else {
        mapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes_of(c: char) -> Vec<&'static str> {
        CharClass::ALL
            .into_iter()
            .filter(|class| is_class(*class, c))
            .map(CharClass::name)
            .collect()
    }

    #[test]
    fn simple_case_mappings_follow_tcl_tables() {
        // No simple uppercase for ß; U+1E9E lowercases to it.
        assert_eq!(to_upper('ß'), 'ß');
        assert_eq!(to_lower('ẞ'), 'ß');
        // Dotted and dotless i map without locale rules.
        assert_eq!(to_lower('İ'), 'i');
        assert_eq!(to_upper('ı'), 'I');
        // Digraph: all three forms differ.
        for c in ['Ǆ', 'ǅ', 'ǆ'] {
            assert_eq!((to_upper(c), to_lower(c), to_title(c)), ('Ǆ', 'ǆ', 'ǅ'));
        }
        // Final sigma has no context: Σ always lowercases to σ.
        assert_eq!(to_upper('ς'), 'Σ');
        assert_eq!(lower("ΟΔΟΣ"), "οδοσ");
        // Simple (not full) mapping of a precomposed iota-subscript letter.
        assert_eq!(to_upper('\u{1F80}'), '\u{1F88}');
        assert_eq!(to_title('\u{1F80}'), '\u{1F88}');
        // Georgian Mkhedruli has an uppercase but titlecases to itself.
        assert_eq!(to_upper('ა'), '\u{1C90}');
        assert_eq!(to_title('ა'), 'ა');
    }

    #[test]
    fn whole_string_conversions_never_grow_a_character() {
        assert_eq!(to_upper('ɐ'), 'Ɐ');
        assert_eq!(upper("ɐbc"), "ɐBC");
        assert_eq!(to_lower('Ⱥ'), 'ⱥ');
        assert_eq!(lower("ȺB"), "Ⱥb");
        assert_eq!(upper("a\0b"), "A\0B");
        assert_eq!(upper(""), "");
    }

    #[test]
    fn title_case_maps_first_character_and_lowers_the_rest() {
        assert_eq!(title("ǆemal"), "ǅemal");
        assert_eq!(title("ǄEMAL"), "ǅemal");
        assert_eq!(title("hELLO wORLD"), "Hello world");
        // Mtavruli after the first character is left alone.
        assert_eq!(title("\u{1C90}\u{1C91}"), "\u{1C90}\u{1C91}");
        assert_eq!(title(""), "");
    }

    #[test]
    fn classes_follow_tcl_categories_and_special_cases() {
        assert_eq!(classes_of('\0'), ["ascii", "control"]);
        assert_eq!(
            classes_of('ǅ'),
            ["alnum", "alpha", "graph", "print", "wordchar"]
        );
        assert_eq!(
            classes_of('中'),
            ["alnum", "alpha", "graph", "print", "wordchar"]
        );
        assert_eq!(
            classes_of('३'),
            ["alnum", "digit", "graph", "print", "wordchar"]
        );
        assert_eq!(classes_of('\u{200B}'), ["control", "space"]);
        assert_eq!(classes_of('\u{1C}'), ["ascii", "control"]);
        assert_eq!(classes_of('\u{E0001}'), ["control"]);
        assert_eq!(classes_of('\u{E0100}'), ["graph", "print"]);
        assert!(classes_of('\u{F0000}').is_empty());
        assert!(!is_class(CharClass::Xdigit, 'Ａ'));
    }

    #[test]
    fn class_names_resolve_like_string_is() {
        // The strclass operand table uses the abbreviation "word".
        for (index, name) in tcld_core::inst_table::TCL_STRING_CLASS_TABLE
            .iter()
            .enumerate()
        {
            assert_eq!(CharClass::from_name(name), Some(CharClass::ALL[index]));
        }
        for class in CharClass::ALL {
            assert_eq!(CharClass::from_name(class.name()), Some(class));
        }
        assert_eq!(CharClass::from_name("up"), Some(CharClass::Upper));
        assert_eq!(CharClass::from_name("x"), Some(CharClass::Xdigit));
        for rejected in [
            "",
            "al",
            "w",
            "d",
            "integer",
            "boolean",
            "Alpha",
            "wordchars",
        ] {
            assert_eq!(CharClass::from_name(rejected), None, "{rejected:?}");
        }
    }
}
