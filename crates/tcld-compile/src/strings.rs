//! `string` subcommands compiled to Tcl's string instructions, following
//! `tclCompCmdsSZ.c`: only the arities and options Tcl itself compiles. Any
//! other form stays an ordinary command invocation.

use super::*;
use tcld_core::inst_table::TCL_STRING_CLASS_TABLE;

/// `tclDefaultTrimSet` (TIP #413): the characters `string trim` removes
/// when no set is given.
pub const DEFAULT_TRIM_SET: &str = "\t\n\u{b}\u{c}\r \0\u{85}\u{a0}\u{1680}\u{180e}\u{2000}\u{2001}\
    \u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{200b}\u{2028}\
    \u{2029}\u{202f}\u{205f}\u{2060}\u{3000}\u{feff}";

/// The `string` ensemble's subcommands (`stringImplMap`).
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
    "toupper",
    "totitle",
    "trim",
    "trimleft",
    "trimright",
    "wordend",
    "wordstart",
];

/// `string is` classes in `isClasses` order; abbreviations resolve against
/// the whole table.
const IS_CLASSES: [&str; 22] = [
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StringForm {
    Cat,
    /// One operand: `length`, `toupper`, `tolower`, `totitle`.
    Unary(Opcode),
    /// Two operands: `compare`, `equal`, `first`, `last`, `index`.
    Binary(Opcode),
    Match {
        nocase: bool,
    },
    /// `string map` with a literal one-pair map.
    Map {
        key: String,
        replacement: String,
    },
    Range,
    Replace,
    Trim(Opcode),
    Is {
        class: IsClass,
        strict: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IsClass {
    /// A per-character class: its `strclass` operand.
    Char(u32),
    Boolean,
    True,
    False,
    Double,
    /// `wideinteger` accepts number type 2, `integer`/`entier` up to 3.
    Integer {
        max_type: u8,
    },
    List,
    Dict,
}

/// The compiled form of a `string` command, or `None` when Tcl would
/// invoke the command at runtime instead.
pub(crate) fn string_form(text: &str, words: &[Range<usize>]) -> Option<StringForm> {
    if words.iter().any(|word| expanded_word(&text[word.clone()])) {
        return None;
    }
    let subcommand = unique_prefix(&SUBCOMMANDS, &static_subcommand(text, words)?)?;
    let args = &words[2..];
    let literal = |index: usize| word_literal(text, args[index].clone()).ok();
    let flag = |index: usize, name: &str| {
        literal(index).is_some_and(|value| value.len() > 1 && name.starts_with(value.as_str()))
    };
    Some(match (subcommand, args.len()) {
        ("cat", _) => StringForm::Cat,
        ("length", 1) => StringForm::Unary(Opcode::StrLen),
        ("toupper", 1) => StringForm::Unary(Opcode::StrUpper),
        ("tolower", 1) => StringForm::Unary(Opcode::StrLower),
        ("totitle", 1) => StringForm::Unary(Opcode::StrTitle),
        ("compare", 2) => StringForm::Binary(Opcode::StrCmp),
        ("equal", 2) => StringForm::Binary(Opcode::StrEq),
        ("first", 2) => StringForm::Binary(Opcode::StrFind),
        ("last", 2) => StringForm::Binary(Opcode::StrFindLast),
        ("index", 2) => StringForm::Binary(Opcode::StrIndex),
        ("match", 2) => StringForm::Match { nocase: false },
        ("match", 3) if flag(0, "-nocase") => StringForm::Match { nocase: true },
        ("map", 2) => {
            let [key, replacement] =
                <[String; 2]>::try_from(list_elements(&literal(0)?).ok()?).ok()?;
            StringForm::Map { key, replacement }
        }
        ("range", 3) => StringForm::Range,
        ("replace", 3 | 4) => StringForm::Replace,
        ("trim", 1 | 2) => StringForm::Trim(Opcode::StrTrim),
        ("trimleft", 1 | 2) => StringForm::Trim(Opcode::StrTrimLeft),
        ("trimright", 1 | 2) => StringForm::Trim(Opcode::StrTrimRight),
        ("is", 2 | 3) => {
            let strict = args.len() == 3;
            if strict && !flag(1, "-strict") {
                return None;
            }
            StringForm::Is {
                class: is_class(&literal(0)?)?,
                strict,
            }
        }
        _ => return None,
    })
}

fn is_class(name: &str) -> Option<IsClass> {
    Some(match unique_prefix(&IS_CLASSES, name)? {
        "boolean" => IsClass::Boolean,
        "true" => IsClass::True,
        "false" => IsClass::False,
        "double" => IsClass::Double,
        "wideinteger" => IsClass::Integer { max_type: 2 },
        "integer" | "entier" => IsClass::Integer { max_type: 3 },
        "list" => IsClass::List,
        "dict" => IsClass::Dict,
        class => {
            let class = if class == "wordchar" { "word" } else { class };
            let index = TCL_STRING_CLASS_TABLE.iter().position(|c| *c == class)?;
            IsClass::Char(index as u32)
        }
    })
}

impl Compiler<'_> {
    /// Emit `form` for a `string` command whose words are already captured
    /// for its trace, so each operand compiles to a copy of its word.
    pub(super) fn compile_string_form(
        &mut self,
        form: &StringForm,
        text: &str,
        words: &[Range<usize>],
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let args = &words[2..];
        match form {
            StringForm::Cat => {
                if args.is_empty() {
                    self.push_literal("");
                }
                // STR_CONCAT1 takes at most 255 operands; chain in chunks.
                let mut pending = 0;
                for word in args {
                    self.compile_word(text, word.clone(), base, depth)?;
                    pending += 1;
                    if pending == 254 {
                        self.emit_arg(Opcode::StrConcat1, pending);
                        pending = 1;
                    }
                }
                if pending > 1 {
                    self.emit_arg(Opcode::StrConcat1, pending);
                }
            }
            StringForm::Unary(opcode) | StringForm::Binary(opcode) => {
                for word in args {
                    self.compile_word(text, word.clone(), base, depth)?;
                }
                self.emit(*opcode);
            }
            StringForm::Match { nocase } => {
                for word in &args[args.len() - 2..] {
                    self.compile_word(text, word.clone(), base, depth)?;
                }
                self.emit_arg(Opcode::StrMatch, u32::from(*nocase));
            }
            StringForm::Map { key, replacement } => {
                self.push_literal(key);
                self.push_literal(replacement);
                self.compile_word(text, args[1].clone(), base, depth)?;
                self.emit(Opcode::StrMap);
            }
            StringForm::Range => {
                for word in args {
                    self.compile_word(text, word.clone(), base, depth)?;
                }
                self.emit(Opcode::StrRange);
            }
            StringForm::Replace => {
                for word in args {
                    self.compile_word(text, word.clone(), base, depth)?;
                }
                if args.len() == 3 {
                    self.push_literal("");
                }
                self.emit(Opcode::StrReplace);
            }
            StringForm::Trim(opcode) => {
                self.compile_word(text, args[0].clone(), base, depth)?;
                match args.get(1) {
                    Some(set) => self.compile_word(text, set.clone(), base, depth)?,
                    None => self.push_literal(DEFAULT_TRIM_SET),
                }
                self.emit(*opcode);
            }
            StringForm::Is { class, strict } => {
                self.compile_word(text, args.last().unwrap().clone(), base, depth)?;
                self.compile_string_is(*class, *strict, depth);
            }
        }
        Ok(())
    }

    /// `TclCompileStringIsCmd`'s instruction sequences, with the value on
    /// top of the stack. Without `-strict` the empty string qualifies.
    fn compile_string_is(&mut self, class: IsClass, strict: bool, depth: u32) {
        match class {
            IsClass::Char(index) if !strict => self.emit_arg(Opcode::StrClass, index),
            IsClass::Char(index) => {
                self.emit(Opcode::Dup);
                self.emit_arg(Opcode::StrClass, index);
                let matched = self.jump(Opcode::JumpTrue4);
                self.emit(Opcode::Pop);
                self.push_literal("0");
                let end = self.jump(Opcode::Jump4);
                self.patch_jump(matched, self.code.len());
                self.push_literal("");
                self.emit(Opcode::StrNeq);
                self.patch_jump(end, self.code.len());
            }
            IsClass::Boolean if strict => {
                self.emit(Opcode::TryCvtToBoolean);
                self.emit_arg(Opcode::Reverse, 2);
                self.emit(Opcode::Pop);
            }
            IsClass::Boolean => {
                self.emit(Opcode::TryCvtToBoolean);
                let boolean = self.jump(Opcode::JumpTrue4);
                self.push_literal("");
                self.emit(Opcode::StrEq);
                let end = self.jump(Opcode::Jump4);
                self.patch_jump(boolean, self.code.len());
                self.emit(Opcode::Pop);
                self.push_literal("1");
                self.patch_jump(end, self.code.len());
            }
            IsClass::True | IsClass::False => {
                let truth = class == IsClass::True;
                self.emit(Opcode::TryCvtToBoolean);
                let boolean = self.jump(Opcode::JumpTrue4);
                if strict {
                    self.emit(Opcode::Pop);
                    self.push_literal(if truth { "0" } else { "1" });
                } else {
                    self.push_literal("");
                    self.emit(if truth { Opcode::StrEq } else { Opcode::StrNeq });
                }
                self.patch_jump(boolean, self.code.len());
                self.emit(Opcode::Not);
                if truth {
                    self.emit(Opcode::Not);
                }
            }
            IsClass::Double => {
                let empty = (!strict).then(|| {
                    self.emit(Opcode::Dup);
                    self.push_literal("");
                    self.emit(Opcode::StrEq);
                    self.jump(Opcode::JumpTrue4)
                });
                self.emit(Opcode::NumType);
                let numeric = self.jump(Opcode::JumpTrue4);
                self.push_literal("0");
                let end = self.jump(Opcode::Jump4);
                if let Some(empty) = empty {
                    self.patch_jump(empty, self.code.len());
                    self.emit(Opcode::Pop);
                }
                self.patch_jump(numeric, self.code.len());
                self.push_literal("1");
                self.patch_jump(end, self.code.len());
            }
            IsClass::Integer { max_type } => {
                let end = if strict {
                    self.emit(Opcode::NumType);
                    self.emit(Opcode::Dup);
                    self.jump(Opcode::JumpFalse4)
                } else {
                    self.emit(Opcode::Dup);
                    self.emit(Opcode::NumType);
                    self.emit(Opcode::Dup);
                    let numeric = self.jump(Opcode::JumpTrue4);
                    self.emit(Opcode::Pop);
                    self.push_literal("");
                    self.emit(Opcode::StrEq);
                    let end = self.jump(Opcode::Jump4);
                    self.patch_jump(numeric, self.code.len());
                    self.emit_arg(Opcode::Reverse, 2);
                    self.emit(Opcode::Pop);
                    end
                };
                self.push_literal(&max_type.to_string());
                self.emit(Opcode::Le);
                self.patch_jump(end, self.code.len());
            }
            IsClass::List | IsClass::Dict => {
                // Whether parsing the value fails, caught like Tcl does. The
                // empty string is both an empty list and an empty dict.
                let index = self.exceptions.len();
                self.exceptions.push(ExceptionRange {
                    kind: ExceptionType::Catch,
                    nesting_level: self.exception_level(depth),
                    code_offset: 0,
                    num_code_bytes: 0,
                    break_offset: -1,
                    continue_offset: -1,
                    catch_offset: -1,
                    code_range: 0..0,
                });
                self.emit_arg(Opcode::BeginCatch4, index as u32);
                let start = self.code.len();
                self.emit(Opcode::Dup);
                if class == IsClass::List {
                    self.emit(Opcode::ListLength);
                    self.emit(Opcode::Pop);
                } else {
                    self.emit(Opcode::DictVerify);
                }
                let end = self.code.len();
                self.emit(Opcode::Pop);
                self.push_literal("1");
                let parsed = self.jump(Opcode::Jump4);
                let handler = self.code.len();
                self.emit(Opcode::Pop);
                self.push_literal("0");
                self.patch_jump(parsed, self.code.len());
                self.emit(Opcode::EndCatch);
                self.exceptions[index] = ExceptionRange {
                    kind: ExceptionType::Catch,
                    nesting_level: self.exception_level(depth),
                    code_offset: start as i32,
                    num_code_bytes: (end - start) as i32,
                    break_offset: -1,
                    continue_offset: -1,
                    catch_offset: handler as i32,
                    code_range: start as i32..end as i32,
                };
            }
        }
    }
}
