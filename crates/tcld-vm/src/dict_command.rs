//! The `dict` ensemble (`tclDictObj.c`) for the forms the compiler leaves to
//! runtime. The subcommands that evaluate a script (`for`, `map`, `update`,
//! `with`, and `filter` with a script) and `info`, which reports Tcl's own
//! hash table, are left to the caller.

use crate::args::{self, wrong_args};
use crate::builtin::VarNameCtx;
use crate::dict::{self, Dict};
use crate::error::VmError;
use crate::number::{self, Number};
use crate::value::{VmObj, glob_match, match_is_trivial};
use crate::var::split_var_name;

/// The subcommands, in the order Tcl lists them.
const SUBCOMMANDS: [&str; 22] = [
    "append",
    "create",
    "exists",
    "filter",
    "for",
    "get",
    "getdef",
    "getwithdefault",
    "incr",
    "info",
    "keys",
    "lappend",
    "map",
    "merge",
    "remove",
    "replace",
    "set",
    "size",
    "unset",
    "update",
    "values",
    "with",
];

pub enum Dispatch {
    Value(VmObj),
    /// A form only Tcl runs, and why.
    Host(String),
}

/// Run `dict subcommand ?arg ...?`; `words` include the command word. The
/// subcommands that update a variable read and write it through `vars`.
pub fn run(words: &[VmObj], vars: &mut dyn VarNameCtx) -> Result<Dispatch, VmError> {
    let command = words[0].as_string()?;
    let Some(name) = words.get(1) else {
        return Err(wrong_args(&[&command], "subcommand ?arg ...?"));
    };
    let subcommand = args::subcommand(&SUBCOMMANDS, &name.as_string()?)?;
    let args = &words[2..];
    let usage = |valid: bool, usage: &str| {
        if valid {
            Ok(())
        } else {
            Err(wrong_args(&[&command, subcommand], usage))
        }
    };
    let value = match subcommand {
        "for" | "map" | "update" | "with" => {
            return Ok(Dispatch::Host(format!(
                "dict {subcommand} runs script through the Tcl host"
            )));
        }
        "info" => {
            return Ok(Dispatch::Host("dict info reports Tcl's hash tables".into()));
        }
        "create" => {
            usage(args.len().is_multiple_of(2), "?key value ...?")?;
            let mut dict = Dict::default();
            for pair in args.chunks(2) {
                dict.put(pair[0].clone(), pair[1].clone())?;
            }
            dict.into_value()
        }
        "get" => {
            usage(!args.is_empty(), "dictionary ?key ...?")?;
            match args {
                [dictionary] => Dict::from_value(dictionary)?.into_value(),
                [dictionary, path @ .., key] => match Dict::trace(dictionary, path)?.get(key)? {
                    Some(value) => value.clone(),
                    None => return Err(dict::unknown_key(key)?),
                },
                [] => unreachable!(),
            }
        }
        "getdef" | "getwithdefault" => {
            usage(args.len() >= 3, "dictionary ?key ...? key default")?;
            let (default, rest) = args.split_last().unwrap();
            let (key, path) = rest[1..].split_last().unwrap();
            match Dict::trace_existing(&rest[0], path)? {
                Some(leaf) => leaf.get(key)?.unwrap_or(default).clone(),
                None => default.clone(),
            }
        }
        "exists" => {
            usage(args.len() >= 2, "dictionary key ?key ...?")?;
            let (key, path) = args[1..].split_last().unwrap();
            // Any failure, a malformed dictionary included, means absent.
            let found = Dict::trace_existing(&args[0], path)
                .ok()
                .flatten()
                .is_some_and(|leaf| leaf.get(key).is_ok_and(|value| value.is_some()));
            VmObj::Int(i64::from(found))
        }
        "keys" | "values" => {
            usage((1..=2).contains(&args.len()), "dictionary ?pattern?")?;
            let dict = Dict::from_value(&args[0])?;
            let pattern = args.get(1).map(VmObj::as_string).transpose()?;
            let mut items = Vec::new();
            match pattern {
                // A key pattern without wildcards names one key.
                Some(pattern) if subcommand == "keys" && match_is_trivial(&pattern) => {
                    if dict.get(&args[1])?.is_some() {
                        items.push(args[1].clone());
                    }
                }
                pattern => {
                    for (key, value) in dict.entries() {
                        let item = if subcommand == "keys" { key } else { value };
                        let matches = match &pattern {
                            Some(pattern) => glob_match(pattern, &item.as_string()?, false),
                            None => true,
                        };
                        if matches {
                            items.push(item.clone());
                        }
                    }
                }
            }
            VmObj::List(items)
        }
        "size" => {
            usage(args.len() == 1, "dictionary")?;
            VmObj::Int(Dict::from_value(&args[0])?.len() as i64)
        }
        "replace" => {
            usage(args.len() % 2 == 1, "dictionary ?key value ...?")?;
            let mut dict = Dict::from_value(&args[0])?;
            for pair in args[1..].chunks(2) {
                dict.put(pair[0].clone(), pair[1].clone())?;
            }
            dict.into_value()
        }
        "remove" => {
            usage(!args.is_empty(), "dictionary ?key ...?")?;
            let mut dict = Dict::from_value(&args[0])?;
            for key in &args[1..] {
                dict.remove(key)?;
            }
            dict.into_value()
        }
        "merge" => match args {
            [] => VmObj::empty(),
            // One dictionary comes back as given, once it is known to be one.
            [only] => {
                Dict::from_value(only)?;
                only.clone()
            }
            [first, rest @ ..] => {
                let mut dict = Dict::from_value(first)?;
                for other in rest {
                    for (key, value) in Dict::from_value(other)?.into_entries() {
                        dict.put(key, value)?;
                    }
                }
                dict.into_value()
            }
        },
        "filter" => {
            usage(args.len() >= 2, "dictionary filterType ?arg ...?")?;
            let filter = args::lookup(
                &["key", "script", "value"],
                "filterType",
                &args[1].as_string()?,
            )?;
            if filter == "script" {
                return Ok(Dispatch::Host(
                    "dict filter script runs script through the Tcl host".into(),
                ));
            }
            filter_pairs(&args[0], filter == "key", &args[2..])?
        }
        "set" => {
            usage(args.len() >= 3, "dictVarName key ?key ...? value")?;
            let (value, keys) = args[1..].split_last().unwrap();
            let mut dict = Dict::from_optional(read_variable(vars, &args[0])?.as_ref())?;
            dict.put_path(keys, value.clone())?;
            write_variable(vars, &args[0], dict.into_value())?
        }
        "unset" => {
            usage(args.len() >= 2, "dictVarName key ?key ...?")?;
            let mut dict = Dict::from_optional(read_variable(vars, &args[0])?.as_ref())?;
            dict.remove_path(&args[1..])?;
            write_variable(vars, &args[0], dict.into_value())?
        }
        "incr" => {
            usage((2..=3).contains(&args.len()), "dictVarName key ?increment?")?;
            let mut dict = Dict::from_optional(read_variable(vars, &args[0])?.as_ref())?;
            let key = &args[1];
            let value = match (dict.get(key)?, args.get(2)) {
                // A new key holds the increment as given.
                (None, Some(increment)) => match number::of(increment) {
                    Some(Number::Int(_) | Number::Big(_)) => increment.clone(),
                    _ => return Err(reading_increment(number::expected("integer", increment))),
                },
                (None, None) => VmObj::Int(1),
                (Some(current), increment) => incremented(current, increment)?,
            };
            dict.put(key.clone(), value)?;
            write_variable(vars, &args[0], dict.into_value())?
        }
        "append" => {
            usage(args.len() >= 2, "dictVarName key ?value ...?")?;
            let current = read_variable(vars, &args[0])?;
            let mut dict = Dict::from_optional(current.as_ref())?;
            let key = &args[1];
            let existing = dict.get(key)?.cloned();
            let value = match (existing, &args[2..]) {
                // Nothing to change, but Tcl still writes the variable.
                (Some(_), []) => {
                    return Ok(Dispatch::Value(write_variable(
                        vars,
                        &args[0],
                        current.unwrap_or_else(VmObj::empty),
                    )?));
                }
                (None, [only]) => only.clone(),
                (existing, values) => {
                    let mut bytes = match existing {
                        Some(existing) => existing.ensure_bytes()?.as_bytes().to_vec(),
                        None => Vec::new(),
                    };
                    for value in values {
                        bytes.extend_from_slice(value.ensure_bytes()?.as_bytes());
                    }
                    VmObj::from_bytes(bytes)
                }
            };
            dict.put(key.clone(), value)?;
            write_variable(vars, &args[0], dict.into_value())?
        }
        "lappend" => {
            usage(args.len() >= 2, "dictVarName key ?value ...?")?;
            let mut dict = Dict::from_optional(read_variable(vars, &args[0])?.as_ref())?;
            let key = &args[1];
            let value = match dict.get(key)? {
                None => VmObj::List(args[2..].to_vec()),
                Some(existing) if args.len() == 2 => existing.clone(),
                Some(existing) => {
                    let mut items = existing.as_list_vec()?;
                    items.extend_from_slice(&args[2..]);
                    VmObj::List(items)
                }
            };
            dict.put(key.clone(), value)?;
            write_variable(vars, &args[0], dict.into_value())?
        }
        _ => unreachable!("dict subcommand {subcommand}"),
    };
    Ok(Dispatch::Value(value))
}

/// `dict filter dictionary key|value ?pattern ...?`: the pairs whose key (or
/// value) matches any pattern.
fn filter_pairs(dictionary: &VmObj, by_key: bool, patterns: &[VmObj]) -> Result<VmObj, VmError> {
    let dict = Dict::from_value(dictionary)?;
    let mut kept = Dict::default();
    let texts = patterns
        .iter()
        .map(VmObj::as_string)
        .collect::<Result<Vec<_>, _>>()?;
    match (patterns, texts.as_slice()) {
        // One key pattern without wildcards names one key.
        ([key], [text]) if by_key && match_is_trivial(text) => {
            if let Some(value) = dict.get(key)? {
                kept.put(key.clone(), value.clone())?;
            }
        }
        _ => {
            for (key, value) in dict.entries() {
                let subject = if by_key { key } else { value }.as_string()?;
                if texts.iter().any(|text| glob_match(text, &subject, false)) {
                    kept.put(key.clone(), value.clone())?;
                }
            }
        }
    }
    Ok(kept.into_value())
}

/// The variable's value, or `None` where Tcl's `Tcl_ObjGetVar2` fails and
/// the subcommand starts from an empty dictionary.
fn read_variable(vars: &mut dyn VarNameCtx, name: &VmObj) -> Result<Option<VmObj>, VmError> {
    let name = name.ensure_bytes()?;
    let (name1, name2) = split_var_name(&name);
    match vars.get_var(&name1, name2.as_ref()) {
        Ok(value) => Ok(Some(value)),
        Err(VmError::TclError { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Store the updated dictionary; the result is the value the variable holds.
fn write_variable(vars: &mut dyn VarNameCtx, name: &VmObj, value: VmObj) -> Result<VmObj, VmError> {
    let name = name.ensure_bytes()?;
    let (name1, name2) = split_var_name(&name);
    vars.set_var_value(&name1, name2.as_ref(), value)
}

fn reading_increment(error: VmError) -> VmError {
    match error {
        VmError::TclError {
            message,
            error_code,
            ..
        } => VmError::TclError {
            error_info: Some(format!("{message}\n    (reading increment)")),
            message,
            error_code,
        },
        other => other,
    }
}

/// `TclIncrObj`: an integer value plus an integer increment (1 by default),
/// growing into a bignum. A non-number is reported before a double, and the
/// value before the increment.
fn incremented(current: &VmObj, increment: Option<&VmObj>) -> Result<VmObj, VmError> {
    let one = VmObj::Int(1);
    let increment = increment.unwrap_or(&one);
    let bad_value = || number::expected("integer", current);
    let bad_increment = || reading_increment(number::expected("integer", increment));
    let value = number::of(current).ok_or_else(bad_value)?;
    let amount = number::of(increment).ok_or_else(bad_increment)?;
    let value = value.to_bigint().ok_or_else(bad_value)?;
    let amount = amount.to_bigint().ok_or_else(bad_increment)?;
    Ok(VmObj::from_bigint(value + amount))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: VmObj) -> String {
        value.as_string().unwrap()
    }

    #[test]
    fn increments_grow_and_report_bad_operands() {
        assert_eq!(
            text(incremented(&VmObj::from_str(" 5 "), None).unwrap()),
            "6"
        );
        assert_eq!(
            text(incremented(&VmObj::Int(i64::MAX), Some(&VmObj::Int(1))).unwrap()),
            "9223372036854775808"
        );
        let error = incremented(&VmObj::from_str("1.5"), Some(&VmObj::from_str("x"))).unwrap_err();
        assert!(matches!(error, VmError::TclError { ref error_info, .. } if error_info.is_some()));
        let error = incremented(&VmObj::from_str("x"), Some(&VmObj::from_str("y"))).unwrap_err();
        assert!(
            matches!(error, VmError::TclError { message, error_info: None, .. }
            if message == "expected integer but got \"x\"")
        );
    }
}
