//! Tcl's string operations on VM values, as the `str*` instructions perform
//! them (`tclExecute.c`, `tclStringObj.c`): character-indexed ranges and
//! replacement, search, mapping, trimming, case conversion and `string is`
//! character classes, plus Tcl's index syntax (`tclUtil.c`).

use num_traits::{Signed, ToPrimitive};

use crate::error::VmError;
use crate::number::{self, Number};
use crate::unicode::{self, CharClass};
use crate::value::{VmObj, split_tcl_list};

fn chars(value: &VmObj) -> Result<Vec<char>, VmError> {
    Ok(value.as_string()?.chars().collect())
}

fn from_chars(chars: &[char]) -> VmObj {
    VmObj::from_str(&chars.iter().collect::<String>())
}

fn bad_index(text: &str) -> VmError {
    VmError::TclError {
        message: format!("bad index \"{text}\": must be integer?[+-]integer? or end?[+-]integer?"),
        error_info: None,
        error_code: Some("TCL VALUE INDEX".into()),
    }
}

/// `GetWideForIndex` with `end` as the last valid position: plain integers,
/// `end`, `end±N` and `M±N`. Positions before the start come back negative
/// and positions past the end exceed `end`.
pub fn index(value: &VmObj, end: i64) -> Result<i64, VmError> {
    match number::of(value) {
        Some(Number::Int(w)) if w < 0 => return Ok(if end == -1 { i64::MIN } else { -1 }),
        Some(Number::Int(w)) => return Ok(w),
        Some(Number::Big(b)) => {
            return Ok(if b.is_negative() { i64::MIN } else { i64::MAX });
        }
        _ => {}
    }
    let text = value.as_string()?;
    Ok(match end_offset(&text).ok_or_else(|| bad_index(&text))? {
        i64::MAX if end == -1 => i64::MAX,
        i64::MAX => end + 1,
        i64::MIN if end == -1 => i64::MIN,
        i64::MIN => -1,
        offset if offset < 0 => end.saturating_add(offset).saturating_add(1),
        offset => offset,
    })
}

/// `GetEndOffsetFromObj`'s encoding: `-1` is `end` and `-1-n` is `end-n`;
/// `i64::MAX` is `end+1` and `i64::MAX-1` anything later; `i64::MIN` (or
/// `i64::MIN+1`) lies before the start; other values are absolute.
fn end_offset(text: &str) -> Option<i64> {
    if text.starts_with('e') {
        let rest = text.strip_prefix("end")?;
        if rest.is_empty() {
            return Some(-1);
        }
        let (subtract, digits) = match rest.as_bytes()[0] {
            b'-' => (true, &rest[1..]),
            b'+' => (false, &rest[1..]),
            _ => return None,
        };
        if digits.is_empty() || digits.starts_with(|c: char| c.is_ascii_whitespace()) {
            return None;
        }
        return Some(match integer(digits)? {
            Number::Big(b) => {
                if b.is_negative() == subtract {
                    i64::MAX
                } else {
                    i64::MIN
                }
            }
            Number::Int(w) => {
                let offset = match (subtract, w) {
                    (true, i64::MIN) => i64::MAX,
                    (true, w) => -w,
                    (false, w) => w,
                };
                match offset {
                    1 => i64::MAX,
                    offset if offset > 1 => i64::MAX - 1,
                    i64::MIN => i64::MIN,
                    offset => offset - 1,
                }
            }
            Number::Double(_) => return None,
        });
    }
    // A list of several elements is never one index.
    if split_tcl_list(text.as_bytes()).is_ok_and(|items| items.len() > 1) {
        return None;
    }
    let (left, subtract, right) = split_index_sum(text)?;
    let (left, right) = (left.to_bigint()?, right.to_bigint()?);
    let sum = if subtract { left - right } else { left + right };
    let offset = sum.to_i64().unwrap_or(if sum.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    });
    Some(match offset {
        -1 => i64::MIN,
        offset if offset < 0 => i64::MIN + 1,
        offset => offset,
    })
}

fn integer(text: &str) -> Option<Number> {
    Number::parse(text.as_bytes()).filter(|n| !matches!(n, Number::Double(_)))
}

/// Split `M+N` / `M-N` into its two integers.
fn split_index_sum(text: &str) -> Option<(Number, bool, Number)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    let start = i;
    if matches!(bytes.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let radix = match bytes.get(i..i + 2) {
        Some([b'0', b'x' | b'X']) => 16,
        Some([b'0', b'o' | b'O']) => 8,
        Some([b'0', b'b' | b'B']) => 2,
        Some([b'0', b'd' | b'D']) => 10,
        _ => 0,
    };
    if radix != 0 {
        i += 2;
    }
    let radix = if radix == 0 { 10 } else { radix };
    while bytes
        .get(i)
        .is_some_and(|b| *b == b'_' || (*b as char).is_digit(radix))
    {
        i += 1;
    }
    let left_end = i;
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    let subtract = match bytes.get(i)? {
        b'-' => true,
        b'+' => false,
        _ => return None,
    };
    let left = integer(&text[start..left_end])?;
    let right = integer(&text[i + 1..])?;
    Some((left, subtract, right))
}

/// `string index` (`strIndex`): one character, or empty outside the string.
pub fn index_char(value: &VmObj, position: &VmObj) -> Result<VmObj, VmError> {
    let chars = chars(value)?;
    let position = index(position, chars.len() as i64 - 1)?;
    Ok(usize::try_from(position)
        .ok()
        .and_then(|i| chars.get(i))
        .map_or_else(VmObj::empty, |c| {
            VmObj::from_str(c.encode_utf8(&mut [0; 4]))
        }))
}

/// `Tcl_GetRange`: positions outside the string are clamped to it.
fn range_of(chars: &[char], first: i64, last: i64) -> VmObj {
    let first = first.max(0);
    let last = last.min(chars.len() as i64 - 1);
    if last < first {
        VmObj::empty()
    } else {
        from_chars(&chars[first as usize..=last as usize])
    }
}

/// `string range` (`strRange`).
pub fn range(value: &VmObj, first: &VmObj, last: &VmObj) -> Result<VmObj, VmError> {
    let chars = chars(value)?;
    let end = chars.len() as i64 - 1;
    let first = index(first, end)?;
    let last = index(last, end)?;
    Ok(range_of(&chars, first, last))
}

/// `TclIndexDecode` for indices compiled into an instruction.
fn decode_index(encoded: i32, end: i64) -> i64 {
    const TCL_INDEX_END: i64 = -2;
    let encoded = i64::from(encoded);
    if encoded > TCL_INDEX_END {
        return encoded;
    }
    let position = end + encoded - TCL_INDEX_END;
    if position >= 0 { position } else { -1 }
}

/// `strRangeImm`.
pub fn range_immediate(value: &VmObj, first: i32, last: i32) -> Result<VmObj, VmError> {
    let chars = chars(value)?;
    if chars.is_empty() {
        return Ok(value.clone());
    }
    let end = chars.len() as i64 - 1;
    let last = decode_index(last, end);
    if last == -1 {
        return Ok(VmObj::empty());
    }
    Ok(range_of(&chars, decode_index(first, end), last))
}

/// `string replace` (`strReplace`): a range outside the string changes
/// nothing.
pub fn replace(
    value: &VmObj,
    first: &VmObj,
    last: &VmObj,
    replacement: &VmObj,
) -> Result<VmObj, VmError> {
    let chars = chars(value)?;
    let end = chars.len() as i64 - 1;
    let first = index(first, end)?;
    let last = index(last, end)?;
    if last < 0 || first > end || last < first {
        return Ok(value.clone());
    }
    let (first, last) = (first.max(0) as usize, last.min(end) as usize);
    if first == 0 && last as i64 == end {
        return Ok(replacement.clone());
    }
    let mut out: String = chars[..first].iter().collect();
    out.push_str(&replacement.as_string()?);
    out.extend(&chars[last + 1..]);
    Ok(VmObj::from_str(&out))
}

/// `string map {key replacement} value` (`strMap`): every non-overlapping
/// occurrence, left to right; an empty key maps nothing.
pub fn map(key: &VmObj, replacement: &VmObj, value: &VmObj) -> Result<VmObj, VmError> {
    let text = chars(value)?;
    let key = chars(key)?;
    if text.is_empty() || key.is_empty() || key.len() > text.len() {
        return Ok(value.clone());
    }
    if key.len() == text.len() {
        return Ok(if key == text {
            replacement.clone()
        } else {
            value.clone()
        });
    }
    let replacement = replacement.as_string()?;
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(&key) {
            out.push_str(&replacement);
            i += key.len();
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    Ok(VmObj::from_str(&out))
}

/// `string first` / `string last` (`strFind`, `strFindLast`): a character
/// index, or -1; an empty needle is never found.
pub fn find(needle: &VmObj, haystack: &VmObj, last: bool) -> Result<VmObj, VmError> {
    let needle = chars(needle)?;
    let haystack = chars(haystack)?;
    if needle.is_empty() || needle.len() > haystack.len() {
        return Ok(VmObj::Int(-1));
    }
    let matches = |start: &usize| haystack[*start..*start + needle.len()] == needle[..];
    let starts = 0..=haystack.len() - needle.len();
    let found = if last {
        starts.rev().find(matches)
    } else {
        starts.into_iter().find(matches)
    };
    Ok(VmObj::Int(found.map_or(-1, |i| i as i64)))
}

/// `string trim`/`trimleft`/`trimright` with an explicit character set.
pub fn trim(value: &VmObj, set: &VmObj, left: bool, right: bool) -> Result<VmObj, VmError> {
    let text = value.as_string()?;
    let set: Vec<char> = set.as_string()?.chars().collect();
    let in_set = |c: char| set.contains(&c);
    let trimmed = match (left, right) {
        (true, true) => text.trim_matches(in_set),
        (true, false) => text.trim_start_matches(in_set),
        _ => text.trim_end_matches(in_set),
    };
    Ok(if trimmed.len() == text.len() {
        value.clone()
    } else {
        VmObj::from_str(trimmed)
    })
}

#[derive(Debug, Clone, Copy)]
pub enum Case {
    Upper,
    Lower,
    Title,
}

/// `string toupper`/`tolower`/`totitle` over a whole string.
pub fn convert_case(value: &VmObj, case: Case) -> Result<VmObj, VmError> {
    let text = value.as_string()?;
    Ok(VmObj::from_str(&match case {
        Case::Upper => unicode::upper(&text),
        Case::Lower => unicode::lower(&text),
        Case::Title => unicode::title(&text),
    }))
}

/// `strClass`: whether every character is in the class at `class_index` of
/// Tcl's string class table (an empty string qualifies).
pub fn in_class(class_index: i32, value: &VmObj) -> Result<VmObj, VmError> {
    let class = usize::try_from(class_index)
        .ok()
        .and_then(|i| CharClass::ALL.get(i).copied())
        .ok_or_else(|| VmError::InvalidOpcodeOperand {
            opcode: "StrClass".into(),
            detail: format!("class {class_index}"),
        })?;
    let all = value
        .as_string()?
        .chars()
        .all(|c| unicode::is_class(class, c));
    Ok(VmObj::Int(i64::from(all)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_value(text: &str, length: usize) -> Result<i64, VmError> {
        index(&VmObj::from_str(text), length as i64 - 1)
    }

    #[test]
    fn index_grammar_matches_tcl() {
        for (text, length, expected) in [
            ("0", 5, 0),
            ("end", 5, 4),
            ("end-1", 5, 3),
            ("end+1", 5, 5),
            ("end+2", 5, i64::MAX - 1),
            ("end--1", 5, 5),
            ("1+2", 5, 3),
            ("4-5", 5, -1),
            ("0x10", 20, 16),
            ("-3", 5, -1),
        ] {
            assert_eq!(index_value(text, length).unwrap(), expected, "{text}");
        }
        for bad in ["foo", "end-", "end- 1", "e", "1 2", "1+", "1.5", "end-1.5"] {
            assert!(index_value(bad, 5).is_err(), "{bad}");
        }
    }
}
