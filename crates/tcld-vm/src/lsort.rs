/*
 * tclCmdIL.c --
 *
 * Copyright © 1987-1993 The Regents of the University of California.
 * Copyright © 1993-1997 Lucent Technologies.
 * Copyright © 1994-1997 Sun Microsystems, Inc.
 * Copyright © 1998-1999 Scriptics Corporation.
 * Copyright © 2001 Kevin B. Kenny. All rights reserved.
 * Copyright © 2005 Donal K. Fellows.
 *
 * See regex/license.terms for usage and redistribution terms and the
 * DISCLAIMER OF ALL WARRANTIES. The command merge sort is ported from Tcl
 * 9.0.2 using owned VM values and array indices in place of linked pointers.
 */

//! `lsort` (`Tcl_LsortObjCmd` in `tclCmdIL.c`): a stable sort with Tcl's
//! collations, `-index`, `-stride`, `-indices` and `-unique`. `-command`
//! calls a script for every comparison, so the caller decides how to run it.

use std::cmp::Ordering;

use crate::args::{self, lookup, tcl_error, wrong_args};
use crate::completion::Completion;
use crate::error::VmError;
use crate::lsearch::{decode_index, dictionary_compare, double, index_path, select, utf_casecmp};
use crate::value::VmObj;

pub enum Sort {
    Result(VmObj),
    /// The sort compares with `-command`.
    Command(CommandSort),
}

const OPTIONS: [&str; 12] = [
    "-ascii",
    "-command",
    "-decreasing",
    "-dictionary",
    "-increasing",
    "-index",
    "-indices",
    "-integer",
    "-nocase",
    "-real",
    "-stride",
    "-unique",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Collation {
    Ascii,
    Dictionary,
    Integer,
    Real,
    Command,
}

/// What an element sorts by.
enum Key {
    Text(String),
    Integer(i64),
    Real(f64),
}

/// Sort `lsort ?-option value ...? list`; `invoked` is the command word as
/// written, which the usage message repeats.
pub fn sort(invoked: &VmObj, args: &[VmObj]) -> Result<Sort, VmError> {
    let Some((list, options)) = args.split_last() else {
        return Err(wrong_args(
            &[&invoked.as_string()?],
            "?-option value ...? list",
        ));
    };
    let mut collation = Collation::Ascii;
    let (mut increasing, mut nocase, mut unique, mut indices) = (true, false, false, false);
    let mut stride = None;
    let mut path = Vec::new();
    let mut command = None;
    let mut i = 0;
    while i < options.len() {
        let option = lookup(&OPTIONS, "option", &options[i].as_string()?)?;
        // The options that take a value cannot take the list as theirs.
        let value = |what: &str| {
            options.get(i + 1).ok_or_else(|| {
                tcl_error(
                    format!("\"{option}\" option must be followed by {what}"),
                    &["TCL", "ARGUMENT", "MISSING"],
                )
            })
        };
        match option {
            "-ascii" => collation = Collation::Ascii,
            "-command" => {
                command = Some(value("comparison command")?.clone());
                collation = Collation::Command;
                i += 1;
            }
            "-decreasing" => increasing = false,
            "-dictionary" => collation = Collation::Dictionary,
            "-increasing" => increasing = true,
            "-index" => {
                path = index_path(value("list index")?)?;
                i += 1;
            }
            "-indices" => indices = true,
            "-integer" => collation = Collation::Integer,
            "-nocase" => nocase = true,
            "-real" => collation = Collation::Real,
            "-stride" => {
                let length = args::wide(value("stride length")?)?;
                if length < 2 {
                    return Err(tcl_error(
                        "stride length must be at least 2",
                        &["TCL", "OPERATION", "LSORT", "BADSTRIDE"],
                    ));
                }
                stride = Some(usize::try_from(length).unwrap_or(usize::MAX));
                i += 1;
            }
            "-unique" => unique = true,
            _ => unreachable!("lsort option {option}"),
        }
        i += 1;
    }
    let items = list.as_list_vec()?;
    // Tcl snapshots the input list before parsing the command prefix, even
    // for an empty list. Other collations ignore an overridden -command.
    let prefix = if collation == Collation::Command {
        Some(command.unwrap().as_list_vec()?)
    } else {
        None
    };
    if items.is_empty() {
        return Ok(Sort::Result(VmObj::empty()));
    }
    // `-stride` sorts groups of elements by one member of each, chosen by
    // the leading `-index` value.
    let group = stride.unwrap_or(1);
    let mut offset = 0;
    if stride.is_some() {
        if !items.len().is_multiple_of(group) {
            return Err(tcl_error(
                "list size must be a multiple of the stride length",
                &["TCL", "OPERATION", "LSORT", "BADSTRIDE"],
            ));
        }
        if let Some((&leading, rest)) = path.split_first() {
            offset = usize::try_from(decode_index(leading, group as i64 - 1))
                .ok()
                .filter(|offset| *offset < group)
                .ok_or_else(|| {
                    tcl_error(
                        "when used with \"-stride\", the leading \"-index\" value must be \
                         within the group",
                        &["TCL", "OPERATION", "LSORT", "BADINDEX"],
                    )
                })?;
            path = rest.to_vec();
        }
    }
    if let Some(prefix) = prefix {
        return Ok(Sort::Command(CommandSort {
            items,
            prefix,
            path,
            increasing,
            unique,
            indices,
            group,
            offset,
        }));
    }
    let mut entries = Vec::with_capacity(items.len() / group);
    for start in (0..items.len()).step_by(group) {
        let position = start + offset;
        let element = if path.is_empty() {
            items[position].clone()
        } else {
            select(&items[position], &path)?
        };
        let key = match collation {
            Collation::Integer => Key::Integer(args::wide(&element)?),
            Collation::Real => Key::Real(double(&element)?),
            _ => Key::Text(element.as_string()?),
        };
        entries.push((key, start));
    }
    let compare = |left: &Key, right: &Key| {
        let order = match (left, right) {
            (Key::Text(a), Key::Text(b)) if collation == Collation::Dictionary => {
                dictionary_compare(a, b).cmp(&0)
            }
            (Key::Text(a), Key::Text(b)) if nocase => utf_casecmp(a, b),
            (Key::Text(a), Key::Text(b)) => a.cmp(b),
            (Key::Integer(a), Key::Integer(b)) => a.cmp(b),
            (Key::Real(a), Key::Real(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            _ => Ordering::Equal,
        };
        if increasing { order } else { order.reverse() }
    };
    // Equal elements keep their order, as in Tcl's merge sort, and `-unique`
    // keeps the last of them.
    entries.sort_by(|(a, _), (b, _)| compare(a, b));
    if unique {
        let mut kept: Vec<(Key, usize)> = Vec::with_capacity(entries.len());
        for entry in entries {
            match kept.last_mut() {
                Some(last) if compare(&last.0, &entry.0) == Ordering::Equal => *last = entry,
                _ => kept.push(entry),
            }
        }
        entries = kept;
    }
    let mut sorted = Vec::with_capacity(entries.len() * group);
    for (_, start) in entries {
        let members = items[start..start + group].iter().enumerate();
        sorted.extend(members.map(|(member, item)| {
            if indices {
                VmObj::Int((start + member) as i64)
            } else {
                item.clone()
            }
        }));
    }
    Ok(Sort::Result(VmObj::List(sorted)))
}

/// A snapshot of the sort's inputs, independent of any variable or list
/// representation a comparator might change while it executes.
pub struct CommandSort {
    items: Vec<VmObj>,
    prefix: Vec<VmObj>,
    path: Vec<i64>,
    increasing: bool,
    unique: bool,
    indices: bool,
    group: usize,
    offset: usize,
}

impl CommandSort {
    pub fn sort(
        self,
        mut callback: impl FnMut(&[VmObj]) -> Result<Result<VmObj, Completion>, VmError>,
    ) -> Result<Result<VmObj, Completion>, VmError> {
        let mut failure = None;
        let mut compare = |a: &VmObj, b: &VmObj| -> Result<Ordering, VmError> {
            // Tcl stops calling user code after the first non-OK result,
            // but still validates the remaining indexed list elements.
            if failure.is_some() {
                return Ok(Ordering::Equal);
            }
            let mut words = self.prefix.clone();
            words.extend([a.clone(), b.clone()]);
            let result = match callback(&words)? {
                Ok(result) => result,
                Err(completion) => {
                    failure = Some(completion);
                    return Ok(Ordering::Equal);
                }
            };
            // Tcl_GetIntFromObj accepts the signed int range and unsigned
            // int bit patterns. Decreasing order uses the same int negation.
            let order = args::wide(&result)
                .ok()
                .filter(|value| (i64::from(i32::MIN)..=i64::from(u32::MAX)).contains(value));
            let Some(order) = order else {
                failure = Some(Completion::error(
                    VmObj::from_str("-compare command returned non-integer result"),
                    None,
                    Some(VmObj::from_str("TCL OPERATION LSORT COMPARISONFAILED")),
                ));
                return Ok(Ordering::Equal);
            };
            let order = order as i32;
            Ok(if self.increasing {
                order
            } else {
                order.wrapping_neg()
            }
            .cmp(&0))
        };
        let mut keys = Vec::with_capacity(self.items.len() / self.group);
        let mut starts = Vec::with_capacity(keys.capacity());
        let mut runs: Vec<Option<Vec<usize>>> = Vec::new();
        for start in (0..self.items.len()).step_by(self.group) {
            let value = &self.items[start + self.offset];
            keys.push(if self.path.is_empty() {
                value.clone()
            } else {
                select(value, &self.path)?
            });
            starts.push(start);
            let mut run = vec![keys.len() - 1];
            let mut level = 0;
            while level < runs.len() {
                let Some(left) = runs[level].take() else {
                    break;
                };
                run = merge(left, run, &keys, self.unique, &mut compare)?;
                level += 1;
            }
            if level == runs.len() {
                runs.push(Some(run));
            } else {
                runs[level] = Some(run);
            }
        }
        let mut sorted = Vec::new();
        for run in runs.into_iter().flatten() {
            sorted = merge(run, sorted, &keys, self.unique, &mut compare)?;
        }
        if let Some(completion) = failure {
            return Ok(Err(completion));
        }
        let mut result = Vec::with_capacity(sorted.len() * self.group);
        for index in sorted {
            let start = starts[index];
            result.extend(
                self.items[start..start + self.group]
                    .iter()
                    .enumerate()
                    .map(|(member, value)| {
                        if self.indices {
                            VmObj::Int((start + member) as i64)
                        } else {
                            value.clone()
                        }
                    }),
            );
        }
        Ok(Ok(VmObj::List(result)))
    }
}

/// Tcl's MergeLists: stable ties choose the left run, while -unique drops
/// the older equal element during the merge and retains the right one.
fn merge(
    left: Vec<usize>,
    right: Vec<usize>,
    keys: &[VmObj],
    unique: bool,
    compare: &mut impl FnMut(&VmObj, &VmObj) -> Result<Ordering, VmError>,
) -> Result<Vec<usize>, VmError> {
    if left.is_empty() {
        return Ok(right);
    }
    if right.is_empty() {
        return Ok(left);
    }
    let (mut a, mut b) = (0, 0);
    let mut result = Vec::with_capacity(left.len() + right.len());
    while a < left.len() && b < right.len() {
        let order = compare(&keys[left[a]], &keys[right[b]])?;
        if order == Ordering::Greater || (unique && order == Ordering::Equal) {
            if order == Ordering::Equal {
                a += 1;
            }
            result.push(right[b]);
            b += 1;
        } else {
            result.push(left[a]);
            a += 1;
        }
    }
    result.extend_from_slice(&left[a..]);
    result.extend_from_slice(&right[b..]);
    Ok(result)
}
