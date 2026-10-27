//! Tcl's list operations on VM values (`tclListObj.c`, `tclCmdIL.c`,
//! `tclUtil.c`): nested element access, ranges and concatenation, with Tcl's
//! index syntax and errors.

use crate::error::VmError;
use crate::strings;
use crate::value::VmObj;

/// The largest end value Tcl uses when it only checks an index's syntax.
const ANY_END: i64 = i64::MAX - 1;

/// `TclLindexFlat`: follow `indices` into nested lists. An index outside its
/// list yields the empty string, though every later index must still parse.
pub fn index_flat(list: &VmObj, indices: &[VmObj]) -> Result<VmObj, VmError> {
    let mut current = list.clone();
    for (position, index) in indices.iter().enumerate() {
        let items = current.as_list_vec()?;
        let at = strings::index(index, items.len() as i64 - 1)?;
        match usize::try_from(at).ok().and_then(|at| items.get(at)) {
            Some(item) => current = item.clone(),
            None => {
                for later in &indices[position + 1..] {
                    strings::index(later, ANY_END)?;
                }
                return Ok(VmObj::empty());
            }
        }
    }
    Ok(current)
}

/// `TclLindexList`: a single index argument, which is either one index or a
/// list of indices.
pub fn index_list(list: &VmObj, argument: &VmObj) -> Result<VmObj, VmError> {
    let single = std::slice::from_ref(argument);
    if !matches!(argument, VmObj::List(_)) && strings::index(argument, ANY_END).is_ok() {
        return index_flat(list, single);
    }
    match argument.as_list_vec() {
        Ok(indices) => index_flat(list, &indices),
        // Neither an index nor a list: report it as a bad index.
        Err(_) => index_flat(list, single),
    }
}

/// `lrange` (`TclListObjRange`): positions outside the list clamp to it, and
/// the result is always a canonical list.
pub fn range(list: &VmObj, first: &VmObj, last: &VmObj) -> Result<VmObj, VmError> {
    let items = list.as_list_vec()?;
    let end = items.len() as i64 - 1;
    let first = strings::index(first, end)?.max(0);
    let last = strings::index(last, end)?.min(end);
    Ok(if first > last {
        VmObj::empty()
    } else {
        VmObj::List(items[first as usize..=last as usize].to_vec())
    })
}

/// `Tcl_ConcatObj`: list values (and empty strings) join as one list;
/// otherwise each value loses its surrounding white space and the non-empty
/// ones join with single spaces.
pub fn concat(values: &[VmObj]) -> Result<VmObj, VmError> {
    if let Some(items) = concat_lists(values)? {
        return Ok(VmObj::List(items));
    }
    let mut out = Vec::new();
    for value in values {
        let bytes = value.ensure_bytes()?;
        let bytes = bytes.as_bytes();
        let start = bytes
            .iter()
            .position(|byte| !is_concat_space(*byte))
            .unwrap_or(bytes.len());
        let mut end = bytes
            .iter()
            .rposition(|byte| !is_concat_space(*byte))
            .map_or(start, |last| last + 1);
        // Trimming must not leave the value ending in a lone backslash.
        if end < bytes.len() && end > start && bytes[end - 1] == b'\\' {
            end += 1;
        }
        if end == start {
            continue;
        }
        if !out.is_empty() {
            out.push(b' ');
        }
        out.extend_from_slice(&bytes[start..end]);
    }
    Ok(VmObj::from_bytes(out))
}

/// The list form of a concatenation, when every value is a list or empty and
/// no later list starts with `#`, which a list string would have to quote.
fn concat_lists(values: &[VmObj]) -> Result<Option<Vec<VmObj>>, VmError> {
    for value in values {
        if !matches!(value, VmObj::List(_)) && !value.ensure_bytes()?.is_empty() {
            return Ok(None);
        }
    }
    let mut joined: Option<Vec<VmObj>> = None;
    for value in values {
        let VmObj::List(items) = value else {
            continue;
        };
        match &mut joined {
            None => joined = Some(items.clone()),
            Some(joined) => {
                let Some(first) = items.first() else {
                    continue;
                };
                if first.ensure_bytes()?.as_bytes().starts_with(b"#") {
                    return Ok(None);
                }
                joined.extend(items.iter().cloned());
            }
        }
    }
    Ok(Some(joined.unwrap_or_default()))
}

fn is_concat_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: Result<VmObj, VmError>) -> String {
        value.unwrap().as_string().unwrap()
    }

    #[test]
    fn index_lists_and_ranges() {
        let list = VmObj::from_str("a {b {c d}} e");
        assert_eq!(text(index_list(&list, &VmObj::from_str("end"))), "e");
        assert_eq!(text(index_list(&list, &VmObj::from_str("1 1 0"))), "c");
        assert_eq!(
            text(index_list(&list, &VmObj::from_str(""))),
            "a {b {c d}} e"
        );
        assert_eq!(text(index_list(&list, &VmObj::from_str("9"))), "");
        assert!(index_flat(&list, &[VmObj::from_str("9"), VmObj::from_str("x")]).is_err());
        assert_eq!(
            text(range(
                &list,
                &VmObj::from_str("-3"),
                &VmObj::from_str("end-1")
            )),
            "a {b {c d}}"
        );
        assert_eq!(
            text(range(&list, &VmObj::from_str("2"), &VmObj::from_str("1"))),
            ""
        );
    }

    #[test]
    fn concat_trims_strings_and_joins_lists() {
        let strings = [
            VmObj::from_str("  a  b "),
            VmObj::from_str(""),
            VmObj::from_str("c\\ "),
        ];
        assert_eq!(text(concat(&strings)), "a  b c\\ ");
        let lists = [
            VmObj::List(vec![VmObj::from_str("x")]),
            VmObj::List(vec![VmObj::from_str("#a")]),
        ];
        assert_eq!(text(concat(&lists)), "x {#a}");
        let lists = [
            VmObj::List(vec![VmObj::from_str("#a")]),
            VmObj::List(vec![]),
            VmObj::List(vec![VmObj::from_str("b c")]),
        ];
        assert_eq!(text(concat(&lists)), "{#a} {b c}");
    }
}
