//! `lsearch` (`Tcl_LsearchObjCmd` in `tclCmdIL.c`): linear and sorted list
//! search with Tcl's options and errors.

use std::cmp::Ordering;

use crate::args::{lookup, tcl_error, wide};
use crate::error::VmError;
use crate::number::{self, Number};
use crate::strings;
use crate::unicode::{self, CharClass};
use crate::value::{VmObj, glob_match};

const OPTIONS: [&str; 19] = [
    "-all",
    "-ascii",
    "-bisect",
    "-decreasing",
    "-dictionary",
    "-exact",
    "-glob",
    "-increasing",
    "-index",
    "-inline",
    "-integer",
    "-nocase",
    "-not",
    "-real",
    "-regexp",
    "-sorted",
    "-start",
    "-stride",
    "-subindices",
];

const TCL_INDEX_END: i64 = -2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Exact,
    Glob,
    Regexp,
    Sorted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataType {
    Ascii,
    Dictionary,
    Integer,
    Real,
}

/// `Tcl_GetIndexFromObj` over the option table, with Tcl's messages.
fn option(word: &VmObj) -> Result<&'static str, VmError> {
    lookup(&OPTIONS, "option", &word.as_string()?)
}

/// `TclIndexEncode` with indices before or after the list encoded as none.
fn encode_index(value: &VmObj) -> Result<Option<i64>, VmError> {
    const END: i64 = 2 * i32::MAX as i64;
    let position = strings::index(value, END)?;
    let end_relative = !matches!(value, VmObj::Int(_)) && value.as_string()?.starts_with('e');
    Ok(if end_relative {
        let offset = position - END;
        (offset <= 0 && offset > i64::from(i32::MIN) - TCL_INDEX_END)
            .then_some(TCL_INDEX_END + offset)
    } else {
        (0..=i64::from(i32::MAX))
            .contains(&position)
            .then_some(position)
    })
}

/// The `-index` option's list of indices, each encoded by `TclIndexEncode`;
/// one before the start of a list is out of range.
pub(crate) fn index_path(word: &VmObj) -> Result<Vec<i64>, VmError> {
    let mut indices = Vec::new();
    for index in word.as_list_vec()? {
        match encode_index(&index)? {
            Some(encoded) => indices.push(encoded),
            None => {
                return Err(tcl_error(
                    format!("index \"{}\" out of range", index.as_string()?),
                    &["TCL", "VALUE", "INDEX", "OUTOFRANGE"],
                ));
            }
        }
    }
    Ok(indices)
}

/// `TclIndexDecode`.
pub(crate) fn decode_index(encoded: i64, end: i64) -> i64 {
    if encoded > TCL_INDEX_END {
        return encoded;
    }
    // Any position before the start of the list is none.
    (end + encoded - TCL_INDEX_END).max(-1)
}

/// `SelectObjFromSublist`: follow the `-index` path into an element.
pub(crate) fn select(element: &VmObj, indices: &[i64]) -> Result<VmObj, VmError> {
    let mut current = element.clone();
    for &encoded in indices {
        let items = current.as_list_vec()?;
        let index = decode_index(encoded, items.len() as i64 - 1);
        match usize::try_from(index).ok().and_then(|i| items.get(i)) {
            Some(item) => current = item.clone(),
            None => {
                let text = current.as_string()?;
                let message = if index == -1 {
                    format!(
                        "element end-{} missing from sublist \"{text}\"",
                        TCL_INDEX_END - encoded
                    )
                } else {
                    format!("element {index} missing from sublist \"{text}\"")
                };
                return Err(tcl_error(
                    message,
                    &["TCL", "OPERATION", "LSORT", "INDEXFAILED"],
                ));
            }
        }
    }
    Ok(current)
}

pub(crate) fn double(value: &VmObj) -> Result<f64, VmError> {
    match number::of(value) {
        Some(Number::Double(d)) if d.is_nan() => Err(number::nan_error()),
        Some(number) => Ok(number.to_f64()),
        None => Err(number::expected("floating-point number", value)),
    }
}

/// `TclUtfCasecmp`: code points compared through Tcl's lowercase mapping.
pub(crate) fn utf_casecmp(left: &str, right: &str) -> Ordering {
    let mut left = left.chars();
    let mut right = right.chars();
    loop {
        match (left.next(), right.next()) {
            (Some(a), Some(b)) => {
                if a != b {
                    let (a, b) = (unicode::to_lower(a), unicode::to_lower(b));
                    if a != b {
                        return a.cmp(&b);
                    }
                }
            }
            (a, b) => return a.is_some().cmp(&b.is_some()),
        }
    }
}

/// `DictionaryCompare`: case-insensitive, with embedded decimal numbers
/// compared numerically; leading zeros and case break ties.
pub fn dictionary_compare(left: &str, right: &str) -> i64 {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
    let (mut l, mut r) = (0usize, 0usize);
    let mut secondary = 0i64;
    let mut diff;
    loop {
        if at(right, r).is_ascii_digit() && at(left, l).is_ascii_digit() {
            let mut zeros = 0i64;
            while at(right, r) == b'0' && at(right, r + 1).is_ascii_digit() {
                r += 1;
                zeros -= 1;
            }
            while at(left, l) == b'0' && at(left, l + 1).is_ascii_digit() {
                l += 1;
                zeros += 1;
            }
            if secondary == 0 {
                secondary = zeros;
            }
            diff = 0;
            loop {
                if diff == 0 {
                    diff = i64::from(at(left, l)) - i64::from(at(right, r));
                }
                r += 1;
                l += 1;
                if !at(right, r).is_ascii_digit() {
                    if at(left, l).is_ascii_digit() {
                        return 1;
                    }
                    if diff != 0 {
                        return diff;
                    }
                    break;
                } else if !at(left, l).is_ascii_digit() {
                    return -1;
                }
            }
            continue;
        }
        if l >= left.len() || r >= right.len() {
            diff = i64::from(at(left, l)) - i64::from(at(right, r));
            break;
        }
        let (a, a_len) = decode(&left[l..]);
        let (b, b_len) = decode(&right[r..]);
        l += a_len;
        r += b_len;
        diff =
            i64::from(u32::from(unicode::to_lower(a))) - i64::from(u32::from(unicode::to_lower(b)));
        if diff != 0 {
            return diff;
        }
        if secondary == 0 {
            let upper = |c| unicode::is_class(CharClass::Upper, c);
            let lower = |c| unicode::is_class(CharClass::Lower, c);
            if upper(a) && lower(b) {
                secondary = -1;
            } else if upper(b) && lower(a) {
                secondary = 1;
            }
        }
    }
    if diff == 0 { secondary } else { diff }
}

/// One character from UTF-8, or the byte itself when it is malformed.
fn decode(bytes: &[u8]) -> (char, usize) {
    let width = match bytes[0] {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
    .min(bytes.len());
    match std::str::from_utf8(&bytes[..width]) {
        Ok(text) => (text.chars().next().unwrap(), width),
        Err(_) => (char::from(bytes[0]), 1),
    }
}

fn sign(ordering: Ordering) -> i64 {
    match ordering {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

enum Pattern {
    Text(String),
    Integer(i64),
    Real(f64),
}

pub fn search(invoked: &VmObj, args: &[VmObj]) -> Result<VmObj, VmError> {
    if args.len() < 2 {
        return Err(tcl_error(
            format!(
                "wrong # args: should be \"{} ?-option value ...? list pattern\"",
                invoked.as_string()?
            ),
            &["TCL", "WRONGARGS"],
        ));
    }
    let mut mode = Mode::Glob;
    let mut data = DataType::Ascii;
    let mut increasing = true;
    let mut all = false;
    let mut inline = false;
    let mut subindices = false;
    let mut negate = false;
    let mut bisect = false;
    let mut nocase = false;
    let mut start_index: Option<VmObj> = None;
    let mut stride = 1i64;
    let mut indices: Vec<i64> = Vec::new();
    let options_end = args.len() - 2;
    let mut i = 0;
    while i < options_end {
        match option(&args[i])? {
            "-all" => all = true,
            "-ascii" => data = DataType::Ascii,
            "-bisect" => {
                mode = Mode::Sorted;
                bisect = true;
            }
            "-decreasing" => increasing = false,
            "-dictionary" => data = DataType::Dictionary,
            "-exact" => mode = Mode::Exact,
            "-glob" => mode = Mode::Glob,
            "-increasing" => increasing = true,
            "-inline" => inline = true,
            "-integer" => data = DataType::Integer,
            "-nocase" => nocase = true,
            "-not" => negate = true,
            "-real" => data = DataType::Real,
            "-regexp" => mode = Mode::Regexp,
            "-sorted" => mode = Mode::Sorted,
            "-subindices" => subindices = true,
            "-start" => {
                if i + 1 >= options_end {
                    return Err(tcl_error(
                        "missing starting index",
                        &["TCL", "ARGUMENT", "MISSING"],
                    ));
                }
                i += 1;
                start_index = Some(args[i].clone());
            }
            "-stride" => {
                if i + 1 >= options_end {
                    return Err(tcl_error(
                        "\"-stride\" option must be followed by stride length",
                        &["TCL", "ARGUMENT", "MISSING"],
                    ));
                }
                let length = wide(&args[i + 1])?;
                if length < 1 {
                    return Err(tcl_error(
                        "stride length must be at least 1",
                        &["TCL", "OPERATION", "LSEARCH", "BADSTRIDE"],
                    ));
                }
                stride = length;
                i += 1;
            }
            "-index" => {
                if i + 1 >= options_end {
                    return Err(tcl_error(
                        "\"-index\" option must be followed by list index",
                        &["TCL", "ARGUMENT", "MISSING"],
                    ));
                }
                i += 1;
                indices = index_path(&args[i])?;
            }
            _ => unreachable!(),
        }
        i += 1;
    }
    if subindices && indices.is_empty() {
        return Err(tcl_error(
            "-subindices cannot be used without -index option",
            &["TCL", "OPERATION", "LSEARCH", "BAD_OPTION_MIX"],
        ));
    }
    if bisect && (all || negate) {
        return Err(tcl_error(
            "-bisect is not compatible with -all or -not",
            &["TCL", "OPERATION", "LSEARCH", "BAD_OPTION_MIX"],
        ));
    }
    // The expression compiles before the list is parsed, as in Tcl.
    let regexp = match mode {
        Mode::Regexp => Some(crate::regexp_command::search_pattern(
            &args[options_end + 1].as_string()?,
            nocase,
        )?),
        _ => None,
    };
    let list = args[options_end].as_list_vec()?;
    let count = list.len() as i64;
    let mut group_offset = 0i64;
    if stride > 1 {
        if count % stride != 0 {
            return Err(tcl_error(
                "list size must be a multiple of the stride length",
                &["TCL", "OPERATION", "LSEARCH", "BADSTRIDE"],
            ));
        }
        if !indices.is_empty() {
            group_offset = decode_index(indices[0], stride - 1);
            if !(0..stride).contains(&group_offset) {
                return Err(tcl_error(
                    "when used with \"-stride\", the leading \"-index\" value must be within the group",
                    &["TCL", "OPERATION", "LSEARCH", "BADINDEX"],
                ));
            }
            indices.remove(0);
        }
    }
    let mut start = 0i64;
    if let Some(index) = &start_index {
        start = strings::index(index, count - 1)?.max(0);
        if start >= count {
            return Ok(if all || inline {
                VmObj::empty()
            } else {
                VmObj::Int(-1)
            });
        }
        start -= start % stride;
    }
    let pattern_word = &args[options_end + 1];
    let pattern = match (mode, data) {
        (Mode::Exact | Mode::Sorted, DataType::Integer) => Pattern::Integer(wide(pattern_word)?),
        (Mode::Exact | Mode::Sorted, DataType::Real) => Pattern::Real(double(pattern_word)?),
        _ => Pattern::Text(pattern_word.as_string()?),
    };
    let item_at = |position: i64| -> Result<VmObj, VmError> {
        select(&list[(position + group_offset) as usize], &indices)
    };
    let mut index = -1i64;
    let mut last = start;
    if mode == Mode::Sorted && !all && !negate {
        let mut lower = start - stride;
        let mut upper = count;
        while lower + stride != upper {
            let mut probe = (lower + upper) / 2;
            probe -= probe % stride;
            last = probe;
            let item = item_at(probe)?;
            let order = match (&pattern, data) {
                (Pattern::Integer(p), _) => sign(p.cmp(&wide(&item)?)),
                (Pattern::Real(p), _) => {
                    let value = double(&item)?;
                    if *p == value {
                        0
                    } else if *p < value {
                        -1
                    } else {
                        1
                    }
                }
                (Pattern::Text(p), DataType::Dictionary) => {
                    dictionary_compare(p, &item.as_string()?)
                }
                (Pattern::Text(p), _) if nocase => sign(utf_casecmp(p, &item.as_string()?)),
                (Pattern::Text(p), _) => sign(p.as_str().cmp(item.as_string()?.as_str())),
            };
            if order == 0 {
                index = probe;
                if bisect {
                    lower = probe;
                } else {
                    upper = probe;
                }
            } else if (order > 0) == increasing {
                lower = probe;
            } else {
                upper = probe;
            }
        }
        if bisect && index < 0 {
            index = lower;
        }
    } else {
        let mut matches = Vec::new();
        let mut position = start;
        while position < count {
            last = position;
            let item = item_at(position)?;
            let mut matched = match (mode, &pattern) {
                (Mode::Regexp, _) => crate::regexp_command::matches(
                    regexp.as_ref().expect("compiled expression"),
                    &item.as_string()?,
                )?,
                (Mode::Glob, Pattern::Text(p)) => glob_match(p, &item.as_string()?, nocase),
                (_, Pattern::Integer(p)) => wide(&item)? == *p,
                (_, Pattern::Real(p)) => double(&item)? == *p,
                (_, Pattern::Text(p)) => match data {
                    DataType::Dictionary => dictionary_compare(&item.as_string()?, p) == 0,
                    _ => {
                        let text = item.as_string()?;
                        text.len() == p.len()
                            && if nocase {
                                utf_casecmp(&text, p) == Ordering::Equal
                            } else {
                                text == *p
                            }
                    }
                },
            };
            if negate {
                matched = !matched;
            }
            if matched {
                if !all {
                    index = position;
                    break;
                }
                if inline {
                    if subindices {
                        matches.push(item);
                    } else if stride > 1 {
                        matches.extend_from_slice(
                            &list[position as usize..(position + stride) as usize],
                        );
                    } else {
                        matches.push(list[position as usize].clone());
                    }
                } else if subindices {
                    let mut path = vec![VmObj::Int(position + group_offset)];
                    path.extend(indices.iter().map(|&e| VmObj::Int(decode_index(e, count))));
                    matches.push(VmObj::List(path));
                } else {
                    matches.push(VmObj::Int(position));
                }
            }
            position += stride;
        }
        if all {
            return Ok(VmObj::List(matches));
        }
    }
    Ok(if !inline {
        if subindices {
            let mut path = vec![VmObj::Int(index + group_offset)];
            path.extend(indices.iter().map(|&e| VmObj::Int(decode_index(e, count))));
            VmObj::List(path)
        } else {
            VmObj::Int(index)
        }
    } else if index < 0 {
        VmObj::empty()
    } else if subindices {
        item_at(last)?
    } else if stride > 1 {
        VmObj::List(list[index as usize..(index + stride) as usize].to_vec())
    } else {
        list[index as usize].clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_order() {
        assert!(dictionary_compare("a2", "a10") < 0);
        assert!(dictionary_compare("Abc", "abc") < 0);
        assert!(dictionary_compare("x01", "x1") > 0);
        assert_eq!(dictionary_compare("same", "same"), 0);
    }
}
