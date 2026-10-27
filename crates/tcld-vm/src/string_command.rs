//! The `string` ensemble (`tclCmdMZ.c`) for the forms the compiler leaves to
//! runtime: every subcommand, with Tcl's usage, option and index errors.

use crate::args::{self, tcl_error, wrong_args};
use crate::builtin::VarNameCtx;
use crate::dict::Dict;
use crate::error::VmError;
use crate::list_codec::find_element;
use crate::number::{self, Number};
use crate::strings::{self, Case};
use crate::unicode::{self, CharClass};
use crate::value::{VmObj, glob_match};
use crate::var::split_var_name;

/// The subcommands, in the order Tcl lists them.
const SUBCOMMANDS: [&str; 23] = [
    "cat",
    "compare",
    "equal",
    "first",
    "index",
    "insert",
    "is",
    "last",
    "length",
    "map",
    "match",
    "range",
    "repeat",
    "replace",
    "reverse",
    "tolower",
    "totitle",
    "toupper",
    "trim",
    "trimleft",
    "trimright",
    "wordend",
    "wordstart",
];

/// `string is` classes, in `isClasses` order.
const CLASSES: [&str; 22] = [
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

fn chars(value: &VmObj) -> Result<Vec<char>, VmError> {
    Ok(value.as_string()?.chars().collect())
}

/// `TclIsSpaceProc`: the white space that separates list elements.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

fn is_word(c: char) -> bool {
    unicode::is_class(CharClass::Wordchar, c)
}

/// `TclGetIntForIndexM`: [`strings::index`], where a position before the
/// start of a non-empty string reads as -1.
fn int_for_index(value: &VmObj, end: i64) -> Result<i64, VmError> {
    let index = strings::index(value, end)?;
    Ok(if index < 0 && end >= 0 { -1 } else { index })
}

/// Run `string subcommand ?arg ...?`; `words` include the command word.
/// `string is ... -failindex var` stores through `vars`.
pub fn run(words: &[VmObj], vars: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    let command = words[0].as_string()?;
    let Some(name) = words.get(1) else {
        return Err(wrong_args(&[&command], "subcommand ?arg ...?"));
    };
    let subcommand = args::subcommand(&SUBCOMMANDS, &name.as_string()?)?;
    let args = &words[2..];
    let usage_error = |usage: &str| wrong_args(&[&command, subcommand], usage);
    let usage = |valid: bool, usage: &str| {
        if valid {
            Ok(())
        } else {
            Err(usage_error(usage))
        }
    };
    Ok(match subcommand {
        "cat" => match args {
            [only] => only.clone(),
            _ => {
                let mut out = Vec::new();
                for arg in args {
                    out.extend_from_slice(arg.ensure_bytes()?.as_bytes());
                }
                VmObj::from_bytes(out)
            }
        },
        "compare" | "equal" => {
            const USAGE: &str = "?-nocase? ?-length int? string1 string2";
            usage((2..=5).contains(&args.len()), USAGE)?;
            let mut nocase = false;
            let mut length = None;
            let mut i = 0;
            while i < args.len() - 2 {
                let option = args[i].as_string()?;
                if option.len() > 1 && "-nocase".starts_with(&option) {
                    nocase = true;
                } else if option.len() > 1 && "-length".starts_with(&option) {
                    if i + 1 >= args.len() - 2 {
                        return Err(usage_error(USAGE));
                    }
                    i += 1;
                    // A negative length compares the whole strings.
                    length = u64::try_from(args::wide(&args[i])?).ok();
                } else {
                    return Err(tcl_error(
                        format!("bad option \"{option}\": must be -nocase or -length"),
                        &["TCL", "LOOKUP", "INDEX", "option", &option],
                    ));
                }
                i += 1;
            }
            let order = compare(
                &args[args.len() - 2].as_string()?,
                &args[args.len() - 1].as_string()?,
                nocase,
                length,
            );
            VmObj::Int(if subcommand == "equal" {
                i64::from(order == 0)
            } else {
                order
            })
        }
        "first" => {
            usage(
                (2..=3).contains(&args.len()),
                "needleString haystackString ?startIndex?",
            )?;
            let haystack = chars(&args[1])?;
            let start = match args.get(2) {
                Some(index) => int_for_index(index, haystack.len() as i64 - 1)?,
                None => 0,
            };
            VmObj::Int(first(&chars(&args[0])?, &haystack, start))
        }
        "last" => {
            usage(
                (2..=3).contains(&args.len()),
                "needleString haystackString ?lastIndex?",
            )?;
            let haystack = chars(&args[1])?;
            let last_index = match args.get(2) {
                Some(index) => int_for_index(index, haystack.len() as i64 - 1)?,
                None => i64::MAX,
            };
            VmObj::Int(last(&chars(&args[0])?, &haystack, last_index))
        }
        "index" => {
            usage(args.len() == 2, "string charIndex")?;
            strings::index_char(&args[0], &args[1])?
        }
        "insert" => {
            usage(args.len() == 3, "string index insertString")?;
            let text = chars(&args[0])?;
            let length = text.len() as i64;
            let at = int_for_index(&args[1], length)?.clamp(0, length) as usize;
            let mut out: String = text[..at].iter().collect();
            out.push_str(&args[2].as_string()?);
            out.extend(&text[at..]);
            VmObj::from_str(&out)
        }
        "is" => string_is(&command, args, vars)?,
        "length" => {
            usage(args.len() == 1, "string")?;
            VmObj::Int(args[0].as_string()?.chars().count() as i64)
        }
        "map" => {
            usage((2..=3).contains(&args.len()), "?-nocase? charMap string")?;
            let nocase = args.len() == 3 && nocase_option(&args[0])?;
            let mapping = args[args.len() - 2].as_list_vec()?;
            let value = &args[args.len() - 1];
            if mapping.is_empty() {
                return Ok(value.clone());
            }
            if mapping.len() % 2 != 0 {
                return Err(tcl_error(
                    "char map list unbalanced",
                    &["TCL", "OPERATION", "MAP", "UNBALANCED"],
                ));
            }
            map(value, &mapping, nocase)?
        }
        "match" => {
            usage((2..=3).contains(&args.len()), "?-nocase? pattern string")?;
            let nocase = args.len() == 3 && nocase_option(&args[0])?;
            let pattern = args[args.len() - 2].as_string()?;
            let text = args[args.len() - 1].as_string()?;
            VmObj::Int(i64::from(glob_match(&pattern, &text, nocase)))
        }
        "range" => {
            usage(args.len() == 3, "string first last")?;
            strings::range(&args[0], &args[1], &args[2])?
        }
        "repeat" => {
            usage(args.len() == 2, "string count")?;
            repeat(&args[0], args::wide(&args[1])?)?
        }
        "replace" => {
            usage((3..=4).contains(&args.len()), "string first last ?string?")?;
            let replacement = args.get(3).cloned().unwrap_or_else(VmObj::empty);
            strings::replace(&args[0], &args[1], &args[2], &replacement)?
        }
        "reverse" => {
            usage(args.len() == 1, "string")?;
            VmObj::from_str(&args[0].as_string()?.chars().rev().collect::<String>())
        }
        "tolower" | "toupper" | "totitle" => {
            usage((1..=3).contains(&args.len()), "string ?first? ?last?")?;
            let case = match subcommand {
                "tolower" => Case::Lower,
                "toupper" => Case::Upper,
                _ => Case::Title,
            };
            match args {
                [value] => strings::convert_case(value, case)?,
                [value, first, rest @ ..] => convert_range(value, first, rest.first(), case)?,
                [] => unreachable!(),
            }
        }
        "trim" | "trimleft" | "trimright" => {
            usage((1..=2).contains(&args.len()), "string ?chars?")?;
            let set = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| VmObj::from_str(tcld_compile::DEFAULT_TRIM_SET));
            let left = subcommand != "trimright";
            let right = subcommand != "trimleft";
            strings::trim(&args[0], &set, left, right)?
        }
        "wordend" => {
            usage(args.len() == 2, "string index")?;
            let text = chars(&args[0])?;
            let length = text.len() as i64;
            let index = int_for_index(&args[1], length - 1)?.max(0);
            VmObj::Int(if index < length {
                match text[index as usize..].iter().position(|c| !is_word(*c)) {
                    Some(0) => index + 1,
                    Some(offset) => index + offset as i64,
                    None => length,
                }
            } else {
                length
            })
        }
        "wordstart" => {
            usage(args.len() == 2, "string index")?;
            let text = chars(&args[0])?;
            let index = int_for_index(&args[1], text.len() as i64 - 1)?.min(text.len() as i64 - 1);
            VmObj::Int(if index > 0 && is_word(text[index as usize]) {
                let mut start = index as usize;
                while start > 0 && is_word(text[start - 1]) {
                    start -= 1;
                }
                start as i64
            } else {
                index.max(0)
            })
        }
        _ => unreachable!("string subcommand {subcommand}"),
    })
}

/// The `-nocase` option of `string map` and `string match`: a prefix of it
/// longer than `-`.
fn nocase_option(word: &VmObj) -> Result<bool, VmError> {
    let option = word.as_string()?;
    if option.len() > 1 && "-nocase".starts_with(&option) {
        return Ok(true);
    }
    Err(tcl_error(
        format!("bad option \"{option}\": must be -nocase"),
        &["TCL", "LOOKUP", "INDEX", "option", &option],
    ))
}

/// `TclStringCmp`: -1, 0 or 1 comparing code points (through Tcl's
/// lowercase mapping with `nocase`), over at most `length` characters when
/// one is given.
fn compare(left: &str, right: &str, nocase: bool, length: Option<u64>) -> i64 {
    if length == Some(0) {
        return 0;
    }
    let (left, right): (Vec<char>, Vec<char>) = (left.chars().collect(), right.chars().collect());
    let shorter = left.len().min(right.len());
    let compared = length.map_or(shorter, |length| {
        shorter.min(usize::try_from(length).unwrap_or(usize::MAX))
    });
    for (&a, &b) in left[..compared].iter().zip(&right[..compared]) {
        let (a, b) = if nocase && a != b {
            (unicode::to_lower(a), unicode::to_lower(b))
        } else {
            (a, b)
        };
        if a != b {
            return if a < b { -1 } else { 1 };
        }
    }
    // Equal so far: the shorter string sorts first, unless the comparison
    // already covered the requested length.
    if length.is_some_and(|length| length <= compared as u64) {
        return 0;
    }
    (left.len() as i64 - right.len() as i64).signum()
}

/// `TclStringFirst`: the first match at or after `start`, or -1; an empty
/// needle is never found.
fn first(needle: &[char], haystack: &[char], start: i64) -> i64 {
    let start = start.max(0);
    if needle.is_empty()
        || haystack.len() < needle.len()
        || start > (haystack.len() - needle.len()) as i64
    {
        return -1;
    }
    (start as usize..=haystack.len() - needle.len())
        .find(|&at| haystack[at..at + needle.len()] == *needle)
        .map_or(-1, |at| at as i64)
}

/// `TclStringLast`: the last match ending at or before `last`, or -1.
fn last(needle: &[char], haystack: &[char], last: i64) -> i64 {
    if needle.is_empty() {
        return -1;
    }
    let last = last.min(haystack.len() as i64 - 1);
    let Some(latest) = last
        .checked_add(1)
        .and_then(|end| end.checked_sub(needle.len() as i64))
        .filter(|latest| *latest >= 0)
    else {
        return -1;
    };
    (0..=latest as usize)
        .rev()
        .find(|&at| haystack[at..at + needle.len()] == *needle)
        .map_or(-1, |at| at as i64)
}

/// `string map` (`StringMapCmd`): at each position the first key that
/// matches is replaced and skipped over; an empty key never matches.
fn map(value: &VmObj, mapping: &[VmObj], nocase: bool) -> Result<VmObj, VmError> {
    let text = chars(value)?;
    if text.is_empty() {
        return Ok(value.clone());
    }
    let pairs = mapping
        .chunks(2)
        .map(|pair| Ok((chars(&pair[0])?, pair[1].as_string()?)))
        .collect::<Result<Vec<_>, VmError>>()?;
    let same =
        |a: char, b: char| a == b || (nocase && unicode::to_lower(a) == unicode::to_lower(b));
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let found = pairs.iter().find(|(key, _)| {
            !key.is_empty()
                && text.len() - at >= key.len()
                && key.iter().zip(&text[at..]).all(|(k, c)| same(*c, *k))
        });
        match found {
            Some((key, replacement)) => {
                out.push_str(replacement);
                at += key.len();
            }
            None => {
                out.push(text[at]);
                at += 1;
            }
        }
    }
    Ok(VmObj::from_str(&out))
}

/// `string repeat` (`TclStringRepeat`).
fn repeat(value: &VmObj, count: i64) -> Result<VmObj, VmError> {
    if count == 1 {
        return Ok(value.clone());
    }
    let bytes = value.ensure_bytes()?;
    if count < 1 || bytes.is_empty() {
        return Ok(VmObj::empty());
    }
    if count == i64::MAX {
        return Err(tcl_error(
            format!("max size for a Tcl value ({} bytes) exceeded", i64::MAX),
            &["TCL", "MEMORY"],
        ));
    }
    let total = u128::from(count as u64) * bytes.as_bytes().len() as u128;
    let mut out = Vec::new();
    if usize::try_from(total).map_or(true, |total| out.try_reserve_exact(total).is_err()) {
        return Err(tcl_error(
            format!("string size overflow: unable to alloc {total} bytes"),
            &["TCL", "MEMORY"],
        ));
    }
    for _ in 0..count {
        out.extend_from_slice(bytes.as_bytes());
    }
    Ok(VmObj::from_bytes(out))
}

/// `string toupper`/`tolower`/`totitle` over the characters `first` through
/// `last` (`first` alone when `last` is missing); an empty range changes
/// nothing.
fn convert_range(
    value: &VmObj,
    first: &VmObj,
    last: Option<&VmObj>,
    case: Case,
) -> Result<VmObj, VmError> {
    let text = chars(value)?;
    let end = text.len() as i64 - 1;
    let first = int_for_index(first, end)?.max(0);
    let last = match last {
        Some(last) => int_for_index(last, end)?,
        None => first,
    }
    .min(end);
    if last < first {
        return Ok(value.clone());
    }
    let (first, last) = (first as usize, last as usize);
    let middle: String = text[first..=last].iter().collect();
    let mut out: String = text[..first].iter().collect();
    out.push_str(&match case {
        Case::Upper => unicode::upper(&middle),
        Case::Lower => unicode::lower(&middle),
        Case::Title => unicode::title(&middle),
    });
    out.extend(&text[last + 1..]);
    Ok(VmObj::from_str(&out))
}

/// `string is class ?-strict? ?-failindex var? str` (`StringIsCmd`).
fn string_is(command: &str, args: &[VmObj], vars: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    if !(2..=5).contains(&args.len()) {
        return Err(wrong_args(
            &[command, "is"],
            "class ?-strict? ?-failindex var? str",
        ));
    }
    let class = args::lookup(&CLASSES, "class", &args[0].as_string()?)?;
    let mut strict = false;
    let mut fail_variable = None;
    let mut i = 1;
    while i < args.len() - 1 {
        if args::lookup(&["-strict", "-failindex"], "option", &args[i].as_string()?)? == "-strict" {
            strict = true;
        } else {
            if i + 1 >= args.len() - 1 {
                return Err(wrong_args(
                    &[command, "is", class],
                    "?-strict? ?-failindex var? str",
                ));
            }
            i += 1;
            fail_variable = Some(&args[i]);
        }
        i += 1;
    }
    let (holds, failed_at) = classify(
        class,
        &args[args.len() - 1],
        strict,
        fail_variable.is_some(),
    )?;
    if !holds && let Some(variable) = fail_variable {
        let name = variable.ensure_bytes()?;
        let (name1, name2) = split_var_name(&name);
        vars.set_var(&name1, name2.as_ref(), VmObj::Int(failed_at))?;
    }
    Ok(VmObj::Int(i64::from(holds)))
}

/// Whether `value` is in `class`, and else where it fails: the index
/// `-failindex` reports.
fn classify(
    class: &str,
    value: &VmObj,
    strict: bool,
    want_index: bool,
) -> Result<(bool, i64), VmError> {
    let text = value.as_string()?;
    Ok(match class {
        "boolean" | "true" | "false" => {
            if number::is_boolean_literal(value) {
                let truth = number::boolean(value)?;
                let holds = match class {
                    "true" => truth,
                    "false" => !truth,
                    _ => true,
                };
                (holds, 0)
            } else {
                (!strict && text.is_empty(), 0)
            }
        }
        // Lists and dictionaries ignore `-strict`: an empty string is both.
        "list" if value.as_list_vec().is_ok() => (true, 0),
        "dict" if Dict::from_value(value).is_ok() => (true, 0),
        "list" | "dict" => (false, if want_index { list_failure(&text) } else { 0 }),
        _ if text.is_empty() => (!strict, 0),
        "double" | "integer" | "entier" if matches!(value, VmObj::Int(_)) => (true, 0),
        "double" => numeric(&text, false),
        "integer" | "entier" => numeric(&text, true),
        "wideinteger" if matches!(number::of(value), Some(Number::Int(_))) => (true, 0),
        "wideinteger" if !want_index => (false, 0),
        // A whole integer that fails here is too large, reported at -1.
        "wideinteger" => match numeric(&text, true) {
            (true, _) => (false, -1),
            failed => failed,
        },
        _ => {
            let class = CharClass::from_name(class)
                .ok_or_else(|| VmError::Internal(format!("string is class {class}")))?;
            match text.chars().position(|c| !unicode::is_class(class, c)) {
                Some(position) => (false, position as i64),
                None => (true, 0),
            }
        }
    })
}

/// Whether all of `text` is a number (an integer when `integer_only`), and
/// else where `-failindex` puts the failure: the byte offset where
/// [`scan_number`] stops, or 0 when it finds no number.
fn numeric(text: &str, integer_only: bool) -> (bool, i64) {
    match scan_number(text.as_bytes(), integer_only) {
        Some(stop) if stop == text.len() => (true, 0),
        Some(stop) => (false, stop as i64),
        None => (false, 0),
    }
}

/// The states of `TclParseNumber`'s scanner.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    Initial,
    Signum,
    Zero,
    ZeroX,
    ZeroO,
    ZeroB,
    ZeroD,
    Octal,
    Hexadecimal,
    Binary,
    Decimal,
    LeadingRadixPoint,
    Fraction,
    ExponentStart,
    ExponentSignum,
    Exponent,
    I,
    In,
    Inf,
    Infi,
    Infin,
    Infini,
    Infinit,
    Infinity,
    N,
    Na,
    Nan,
    NanParen,
    NanHex,
    NanFinish,
}

/// `TclParseNumber` with an end pointer, as `string is` calls it: the offset
/// after the last prefix of `bytes` its scanner accepted as a number, and
/// after the white space that follows; `None` when it accepted none. The
/// scanner records a prefix only as it reads the next character, so a digit
/// separator it rejects also drops the digit before it.
fn scan_number(bytes: &[u8], integer_only: bool) -> Option<usize> {
    use Scan::*;
    let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
    let mut integer_only = integer_only;
    let mut state = Initial;
    let mut accepted = None;
    let mut nan_digits = 0;
    let mut p = 0;
    loop {
        let c = at(p);
        // A digit separator joins two digits of the number's radix.
        if c == b'_' {
            if p == 0 {
                break;
            }
            let before = Some(at(p - 1)).filter(|b| *b != b'_');
            let mut after = p + 1;
            while at(after) == b'_' {
                after += 1;
            }
            let after = at(after);
            let joins = |digit: fn(&u8) -> bool| before.as_ref().is_none_or(digit) && digit(&after);
            let valid = match state {
                ZeroB | Binary => joins(|b| matches!(b, b'0' | b'1')),
                ZeroO | Octal => joins(|b| matches!(b, b'0'..=b'7')),
                Fraction | Zero | ZeroD | Decimal | LeadingRadixPoint | ExponentStart
                | ExponentSignum | Exponent => joins(u8::is_ascii_digit) || after == b'(',
                ZeroX | Hexadecimal => joins(u8::is_ascii_hexdigit),
                _ => true,
            };
            if !valid {
                break;
            }
            let read_as_character = match state {
                Fraction | Zero | ZeroD | Decimal | LeadingRadixPoint | ExponentStart
                | ExponentSignum | Exponent => !joins(u8::is_ascii_digit),
                ZeroB | Binary | ZeroO | Octal | ZeroX | Hexadecimal => false,
                _ => true,
            };
            if !read_as_character {
                p += 1;
                continue;
            }
        }
        if matches!(
            state,
            Zero | Octal
                | Hexadecimal
                | Binary
                | Fraction
                | Exponent
                | Inf
                | Nan
                | NanFinish
                | Infinity
        ) {
            accepted = Some(p);
        }
        let digit = c.is_ascii_digit();
        state = match state {
            Initial if is_space(c) => Initial,
            Initial if c == b'+' || c == b'-' => Signum,
            Initial | Signum if c == b'0' => Zero,
            Initial | Signum if digit => Decimal,
            Initial | Signum if integer_only => break,
            Initial | Signum if c == b'.' => LeadingRadixPoint,
            Initial | Signum if c == b'I' || c == b'i' => I,
            Initial | Signum if c == b'N' || c == b'n' => N,
            Zero if c == b'x' || c == b'X' => ZeroX,
            Zero if c == b'b' || c == b'B' => ZeroB,
            Zero if c == b'o' || c == b'O' => ZeroO,
            Zero if c == b'd' || c == b'D' => ZeroD,
            Octal | ZeroO if matches!(c, b'0'..=b'7') => Octal,
            Hexadecimal | ZeroX if c.is_ascii_hexdigit() => Hexadecimal,
            Binary | ZeroB if c == b'0' || c == b'1' => Binary,
            Zero | Decimal | ZeroD => {
                if state == ZeroD {
                    if !digit {
                        break;
                    }
                    integer_only = true;
                }
                accepted = Some(p);
                match c {
                    _ if digit => Decimal,
                    _ if integer_only => break,
                    b'.' => Fraction,
                    b'E' | b'e' => ExponentStart,
                    _ => break,
                }
            }
            Fraction if c == b'E' || c == b'e' => ExponentStart,
            Fraction | LeadingRadixPoint if digit => Fraction,
            ExponentStart if c == b'+' || c == b'-' => ExponentSignum,
            ExponentStart | ExponentSignum | Exponent if digit => Exponent,
            I if c == b'n' || c == b'N' => In,
            In if c == b'f' || c == b'F' => Inf,
            Inf if c == b'i' || c == b'I' => Infi,
            Infi if c == b'n' || c == b'N' => Infin,
            Infin if c == b'i' || c == b'I' => Infini,
            Infini if c == b't' || c == b'T' => Infinit,
            Infinit if c == b'y' || c == b'Y' => Infinity,
            N if c == b'a' || c == b'A' => Na,
            Na if c == b'n' || c == b'N' => Nan,
            Nan if c == b'(' => NanParen,
            NanHex if c == b')' => NanFinish,
            NanParen | NanHex if is_space(c) => state,
            NanParen | NanHex if nan_digits < 13 && c.is_ascii_hexdigit() => {
                nan_digits += 1;
                NanHex
            }
            _ => break,
        };
        p += 1;
    }
    let accepted = accepted?;
    Some(
        accepted
            + bytes[accepted..]
                .iter()
                .take_while(|b| is_space(**b))
                .count(),
    )
}

/// Where a malformed list stops parsing, in characters: the start of the
/// element `TclFindElement` rejects, or -1 when every element parses (a
/// dictionary with a key but no value).
fn list_failure(text: &str) -> i64 {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    loop {
        match find_element(bytes, cursor) {
            Ok(Some((_, next))) => cursor = next,
            Ok(None) => return -1,
            Err(_) => {
                let start = cursor + bytes[cursor..].iter().take_while(|b| is_space(**b)).count();
                return String::from_utf8_lossy(&bytes[..start]).chars().count() as i64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparisons_follow_tcl_string_cmp() {
        assert_eq!(compare("abc", "abd", false, None), -1);
        assert_eq!(compare("ab", "abc", false, None), -1);
        assert_eq!(compare("ab", "abc", false, Some(2)), 0);
        assert_eq!(compare("ab", "abc", false, Some(3)), -1);
        assert_eq!(compare("ABC", "abd", true, Some(2)), 0);
        assert_eq!(compare("x", "y", false, Some(0)), 0);
    }

    #[test]
    fn numeric_prefixes_end_where_tcl_stops() {
        assert_eq!(numeric("12", true), (true, 0));
        assert_eq!(numeric("12abc", true), (false, 2));
        assert_eq!(numeric(" 12 x", true), (false, 4));
        assert_eq!(numeric("1.5", true), (false, 1));
        assert_eq!(numeric("1.5x", false), (false, 3));
        assert_eq!(numeric("1e+", false), (false, 1));
        assert_eq!(numeric("x1", false), (false, 0));
        // A rejected separator drops the digit before it, as in Tcl.
        assert_eq!(numeric("1_", true), (false, 0));
        assert_eq!(numeric("12_x", true), (false, 1));
        assert_eq!(numeric("1_2x", true), (false, 3));
        assert_eq!(numeric("0x1F", false), (true, 0));
        assert_eq!(numeric("infx", false), (false, 3));
        assert_eq!(numeric("nan(12)", false), (true, 0));
    }

    #[test]
    fn list_failures_point_at_the_bad_element() {
        assert_eq!(list_failure("a {b"), 2);
        assert_eq!(list_failure("  {a}b"), 2);
        assert_eq!(list_failure("a b c"), -1);
    }
}
