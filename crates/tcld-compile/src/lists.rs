//! The list commands and `error`, compiled as Tcl compiles them
//! (`tclCompCmdsGR.c`, `tclCompCmds.c`). The VM implements every other form
//! of these commands, and `join` and `split`, natively, so none of them
//! needs the host.

use super::*;

/// Commands the VM runs natively whatever their arguments.
pub(crate) const NATIVE_COMMANDS: [&str; 8] = [
    "list", "llength", "lindex", "lrange", "concat", "join", "split", "error",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListForm {
    List,
    Llength,
    /// `listIndex` for one index, `listIndexMulti` for none or several.
    Lindex,
    /// `lrange` with literal indices, encoded for `listRangeImm`.
    Lrange(i32, i32),
    Concat,
    /// `error message ?errorInfo? ?errorCode?` as `returnImm`.
    Error,
}

/// Whether `name` names a command the VM implements natively.
pub(crate) fn is_native_command(name: &str) -> bool {
    NATIVE_COMMANDS.contains(&name.strip_prefix("::").unwrap_or(name))
}

/// The instructions for a native command form Tcl compiles, or `None` when
/// Tcl invokes the command instead.
pub(crate) fn list_form(name: &str, text: &str, words: &[Range<usize>]) -> Option<ListForm> {
    if words.iter().any(|word| expanded_word(&text[word.clone()])) {
        return None;
    }
    let args = &words[1..];
    Some(match (name, args.len()) {
        ("list", _) => ListForm::List,
        ("llength", 1) => ListForm::Llength,
        ("lindex", 1..) => ListForm::Lindex,
        ("lrange", 3) => {
            ListForm::Lrange(encode_index(text, &args[1])?, encode_index(text, &args[2])?)
        }
        ("concat", _) => ListForm::Concat,
        ("error", 1..=3) => ListForm::Error,
        _ => return None,
    })
}

/// `TclIndexEncode` for the literal indices it encodes directly: a
/// non-negative integer, `end`, or `end-N`.
fn encode_index(text: &str, word: &Range<usize>) -> Option<i32> {
    const TCL_INDEX_END: i64 = -2;
    let literal = word_literal(text, word.clone()).ok()?;
    let number = |digits: &str| {
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse::<i64>().ok())
            .flatten()
            .filter(|value| *value <= 1_000_000_000)
    };
    let encoded = match literal.strip_prefix("end") {
        Some("") => TCL_INDEX_END,
        Some(offset) => TCL_INDEX_END - number(offset.strip_prefix('-')?)?,
        None => number(&literal)?,
    };
    i32::try_from(encoded).ok()
}

impl Compiler<'_> {
    /// Emit `form` for a command whose words are already captured for its
    /// trace, so each operand compiles to a copy of its word.
    pub(super) fn compile_list_form(
        &mut self,
        form: ListForm,
        text: &str,
        words: &[Range<usize>],
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let args = &words[1..];
        let operands = match form {
            ListForm::Lrange(..) | ListForm::Error => &args[..1],
            _ => args,
        };
        if operands.is_empty() {
            // `list` and `concat` without arguments.
            self.push_literal("");
            return Ok(());
        }
        for word in operands {
            self.compile_word(text, word.clone(), base, depth)?;
        }
        let count = operands.len() as u32;
        match form {
            ListForm::List => self.emit_arg(Opcode::List, count),
            ListForm::Concat => self.emit_arg(Opcode::ConcatStk, count),
            ListForm::Llength => self.emit(Opcode::ListLength),
            ListForm::Lindex if count == 2 => self.emit(Opcode::ListIndex),
            ListForm::Lindex => self.emit_arg(Opcode::ListIndexMulti, count),
            ListForm::Lrange(first, last) => {
                self.emit_args(Opcode::ListRangeImm, &[first as u32, last as u32]);
            }
            ListForm::Error => {
                // Return options without -code and -level, which the
                // instruction supplies.
                match args {
                    [_] => self.push_literal(""),
                    [_, rest @ ..] => {
                        for (option, word) in ["-errorinfo", "-errorcode"].iter().zip(rest) {
                            self.push_literal(option);
                            self.compile_word(text, word.clone(), base, depth)?;
                        }
                        self.emit_arg(Opcode::List, 2 * rest.len() as u32);
                    }
                    [] => unreachable!(),
                }
                self.emit_args(Opcode::ReturnImm, &[1, 0]);
            }
        }
        Ok(())
    }
}
