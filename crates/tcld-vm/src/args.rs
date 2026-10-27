//! Argument handling shared by the commands the VM implements: Tcl's
//! abbreviation rules for subcommands and options (`Tcl_GetIndexFromObj`,
//! ensemble prefixes), integer arguments, and Tcl's usage and lookup errors.

use crate::error::VmError;
use crate::list_codec::TclListCodec;
use crate::number::{self, Number};
use crate::value::VmObj;

/// A Tcl error whose error code is the list of `code` words.
pub fn tcl_error(message: impl Into<String>, code: &[&str]) -> VmError {
    VmError::TclError {
        message: message.into(),
        error_info: None,
        error_code: Some(
            String::from_utf8_lossy(&TclListCodec::format(
                code.iter().map(|word| word.as_bytes()),
            ))
            .into_owned(),
        ),
    }
}

/// `Tcl_WrongNumArgs`: `words` are the command's leading words, which Tcl
/// repeats as list elements before the usage.
pub fn wrong_args(words: &[&str], usage: &str) -> VmError {
    let words = TclListCodec::format(words.iter().map(|word| word.as_bytes()));
    tcl_error(
        format!(
            "wrong # args: should be \"{} {usage}\"",
            String::from_utf8_lossy(&words)
        ),
        &["TCL", "WRONGARGS"],
    )
}

/// The entry of `table` that `name` is, or else the one entry it abbreviates.
pub fn unique_prefix(table: &[&'static str], name: &str) -> Option<&'static str> {
    if let Some(exact) = table.iter().find(|entry| **entry == name) {
        return Some(exact);
    }
    let mut matches = table
        .iter()
        .filter(|entry| !name.is_empty() && entry.starts_with(name));
    match (matches.next(), matches.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// `Tcl_GetIndexFromObj`: the entry of `table` that `key` names or uniquely
/// abbreviates, else Tcl's `bad` (or `ambiguous`) `<what>` error.
pub fn lookup(table: &[&'static str], what: &str, key: &str) -> Result<&'static str, VmError> {
    if let Some(entry) = unique_prefix(table, key) {
        return Ok(entry);
    }
    let abbreviated = table.iter().filter(|entry| entry.starts_with(key)).count();
    Err(lookup_error(table, what, key, abbreviated > 1))
}

/// `Tcl_GetIndexFromObj` with `TCL_EXACT`: only a whole entry names it.
pub fn exact_lookup(
    table: &[&'static str],
    what: &str,
    key: &str,
) -> Result<&'static str, VmError> {
    table
        .iter()
        .find(|entry| **entry == key)
        .copied()
        .ok_or_else(|| lookup_error(table, what, key, false))
}

fn lookup_error(table: &[&'static str], what: &str, key: &str, ambiguous: bool) -> VmError {
    let choices = match table {
        [first, second] => format!("{first} or {second}"),
        [head @ .., last] => format!("{}, or {last}", head.join(", ")),
        [] => String::new(),
    };
    tcl_error(
        format!(
            "{} {what} \"{key}\": must be {choices}",
            if ambiguous { "ambiguous" } else { "bad" }
        ),
        &["TCL", "LOOKUP", "INDEX", what, key],
    )
}

/// An ensemble's subcommand: one of `table` or a unique abbreviation, else
/// the ensemble's `unknown or ambiguous subcommand` error.
pub fn subcommand(table: &[&'static str], name: &str) -> Result<&'static str, VmError> {
    unique_prefix(table, name).ok_or_else(|| {
        let (head, last) = table.split_at(table.len() - 1);
        tcl_error(
            format!(
                "unknown or ambiguous subcommand \"{name}\": must be {}, or {}",
                head.join(", "),
                last[0]
            ),
            &["TCL", "LOOKUP", "SUBCOMMAND", name],
        )
    })
}

/// `Tcl_GetWideIntFromObj`: an integer that fits in 64 bits.
pub fn wide(value: &VmObj) -> Result<i64, VmError> {
    match number::of(value) {
        Some(Number::Int(w)) => Ok(w),
        Some(Number::Big(_)) => Err(tcl_error(
            "integer value too large to represent",
            &["ARITH", "IOVERFLOW", "integer value too large to represent"],
        )),
        _ => Err(number::expected("integer", value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(error: VmError) -> (String, String) {
        match error {
            VmError::TclError {
                message,
                error_code,
                ..
            } => (message, error_code.unwrap_or_default()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lookups_follow_tcl_abbreviation_rules() {
        let table = ["-strict", "-failindex"];
        assert_eq!(lookup(&table, "option", "-s").unwrap(), "-strict");
        assert_eq!(
            message(lookup(&table, "option", "-x").unwrap_err()),
            (
                "bad option \"-x\": must be -strict or -failindex".into(),
                "TCL LOOKUP INDEX option -x".into()
            )
        );
        let classes = ["alnum", "alpha", "ascii"];
        assert_eq!(
            message(lookup(&classes, "class", "al").unwrap_err()).0,
            "ambiguous class \"al\": must be alnum, alpha, or ascii"
        );
        assert_eq!(
            message(lookup(&classes, "class", "").unwrap_err()).0,
            "ambiguous class \"\": must be alnum, alpha, or ascii"
        );
        assert_eq!(
            message(subcommand(&classes, "x").unwrap_err()),
            (
                "unknown or ambiguous subcommand \"x\": must be alnum, alpha, or ascii".into(),
                "TCL LOOKUP SUBCOMMAND x".into()
            )
        );
        assert_eq!(
            message(wrong_args(&["string", "is", "alpha"], "str")).0,
            "wrong # args: should be \"string is alpha str\""
        );
    }
}
