/* never again */

// Ported from Tcl 9.0.2 generic/regex.h. Modified for Tcld: native Rust
// data structures and VM integration. Tcl licensing terms: license.terms.

//! Tcl's regular expressions: a port of Henry Spencer's engine as Tcl 9.0.2
//! builds it (`generic/regcomp.c` with the files it includes, `regexec.c`
//! with `rege_dfa.c`, and the class tables of `regc_locale.c`).
//!
//! The port keeps the C structure function for function, with arenas and
//! indices in place of pointers, so that matches, the submatches Tcl's
//! dissection picks, the `regexp -about` facts and the compile errors are
//! Tcl's own. Debugging switches that only Tcl's test commands reach
//! (`REG_DUMP`, `REG_FAKE`, `REG_FTRACE`, `REG_SMALL`, ...) are left out.

mod color;
mod compile;
mod exec;
mod locale;
mod nfa;
mod tables;

use std::cell::Cell;
use std::fmt;

/// A character as the engine sees it: a code point (`Tcl_UniChar`).
pub type Chr = u32;

/// Compile flags (`regex.h`).
pub const REG_BASIC: i32 = 0o0;
pub const REG_EXTENDED: i32 = 0o1;
pub const REG_ADVF: i32 = 0o2;
pub const REG_ADVANCED: i32 = 0o3;
pub const REG_QUOTE: i32 = 0o4;
pub const REG_ICASE: i32 = 0o10;
pub const REG_NOSUB: i32 = 0o20;
pub const REG_EXPANDED: i32 = 0o40;
pub const REG_NLSTOP: i32 = 0o100;
pub const REG_NLANCH: i32 = 0o200;
pub const REG_NEWLINE: i32 = 0o300;
pub const REG_EXPECT: i32 = 0o1000;
pub const REG_BOSONLY: i32 = 0o2000;

/// Execution flags.
pub const REG_NOTBOL: i32 = 0o1;
pub const REG_NOTEOL: i32 = 0o2;

/// The facts `re_info` reports about a compiled expression, in the order
/// `TclRegAbout` lists them.
pub const INFO_NAMES: [(i64, &str); 14] = [
    (REG_UBACKREF, "REG_UBACKREF"),
    (REG_ULOOKAHEAD, "REG_ULOOKAHEAD"),
    (REG_UBOUNDS, "REG_UBOUNDS"),
    (REG_UBRACES, "REG_UBRACES"),
    (REG_UBSALNUM, "REG_UBSALNUM"),
    (REG_UPBOTCH, "REG_UPBOTCH"),
    (REG_UBBS, "REG_UBBS"),
    (REG_UNONPOSIX, "REG_UNONPOSIX"),
    (REG_UUNSPEC, "REG_UUNSPEC"),
    (REG_UUNPORT, "REG_UUNPORT"),
    (REG_ULOCALE, "REG_ULOCALE"),
    (REG_UEMPTYMATCH, "REG_UEMPTYMATCH"),
    (REG_UIMPOSSIBLE, "REG_UIMPOSSIBLE"),
    (REG_USHORTEST, "REG_USHORTEST"),
];
pub const REG_UBACKREF: i64 = 0o1;
pub const REG_ULOOKAHEAD: i64 = 0o2;
pub const REG_UBOUNDS: i64 = 0o4;
pub const REG_UBRACES: i64 = 0o10;
pub const REG_UBSALNUM: i64 = 0o20;
pub const REG_UPBOTCH: i64 = 0o40;
pub const REG_UBBS: i64 = 0o100;
pub const REG_UNONPOSIX: i64 = 0o200;
pub const REG_UUNSPEC: i64 = 0o400;
pub const REG_UUNPORT: i64 = 0o1000;
pub const REG_ULOCALE: i64 = 0o2000;
pub const REG_UEMPTYMATCH: i64 = 0o4000;
pub const REG_UIMPOSSIBLE: i64 = 0o10000;
pub const REG_USHORTEST: i64 = 0o20000;

/// Error codes.
pub(crate) const REG_OKAY: i32 = 0;
pub(crate) const REG_NOMATCH: i32 = 1;
pub(crate) const REG_BADPAT: i32 = 2;
pub(crate) const REG_ECOLLATE: i32 = 3;
pub(crate) const REG_ECTYPE: i32 = 4;
pub(crate) const REG_EESCAPE: i32 = 5;
pub(crate) const REG_ESUBREG: i32 = 6;
pub(crate) const REG_EBRACK: i32 = 7;
pub(crate) const REG_EPAREN: i32 = 8;
pub(crate) const REG_EBRACE: i32 = 9;
pub(crate) const REG_BADBR: i32 = 10;
pub(crate) const REG_ERANGE: i32 = 11;
pub(crate) const REG_ESPACE: i32 = 12;
pub(crate) const REG_BADRPT: i32 = 13;
pub(crate) const REG_ASSERT: i32 = 15;
pub(crate) const REG_INVARG: i32 = 16;
pub(crate) const REG_BADOPT: i32 = 18;
pub(crate) const REG_ETOOBIG: i32 = 19;
pub(crate) const REG_ECOLORS: i32 = 20;

/// `regerrs.h`: each code's name and explanation.
const ERRORS: [(i32, &str, &str); 20] = [
    (REG_OKAY, "REG_OKAY", "no errors detected"),
    (REG_NOMATCH, "REG_NOMATCH", "failed to match"),
    (REG_BADPAT, "REG_BADPAT", "invalid regexp (reg version 0.8)"),
    (REG_ECOLLATE, "REG_ECOLLATE", "invalid collating element"),
    (REG_ECTYPE, "REG_ECTYPE", "invalid character class"),
    (REG_EESCAPE, "REG_EESCAPE", "invalid escape \\ sequence"),
    (REG_ESUBREG, "REG_ESUBREG", "invalid backreference number"),
    (REG_EBRACK, "REG_EBRACK", "brackets [] not balanced"),
    (REG_EPAREN, "REG_EPAREN", "parentheses () not balanced"),
    (REG_EBRACE, "REG_EBRACE", "braces {} not balanced"),
    (REG_BADBR, "REG_BADBR", "invalid repetition count(s)"),
    (REG_ERANGE, "REG_ERANGE", "invalid character range"),
    (REG_ESPACE, "REG_ESPACE", "out of memory"),
    (REG_BADRPT, "REG_BADRPT", "invalid quantifier operand"),
    (
        REG_ASSERT,
        "REG_ASSERT",
        "\"cannot happen\" -- you found a bug",
    ),
    (
        REG_INVARG,
        "REG_INVARG",
        "invalid argument to regex function",
    ),
    (
        17,
        "REG_MIXED",
        "character widths of regex and string differ",
    ),
    (REG_BADOPT, "REG_BADOPT", "invalid embedded option"),
    (
        REG_ETOOBIG,
        "REG_ETOOBIG",
        "regular expression is too complex",
    ),
    (REG_ECOLORS, "REG_ECOLORS", "too many colors"),
];

/// A compile or match failure, by `regex.h` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegexError(pub i32);

impl RegexError {
    /// The code's name (`REG_ITOA`): `REG_EBRACK`.
    pub fn name(self) -> String {
        match ERRORS.iter().find(|(code, ..)| *code == self.0) {
            Some((_, name, _)) => (*name).to_string(),
            None => format!("REG_{}", self.0 as u32),
        }
    }

    /// What `regerror` says about the code.
    pub fn explain(self) -> String {
        match ERRORS.iter().find(|(code, ..)| *code == self.0) {
            Some((.., explain)) => (*explain).to_string(),
            None => format!("*** unknown regex error code 0x{:x} ***", self.0),
        }
    }
}

impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.explain())
    }
}

/// The first error a compilation or match records (`v->err`), shared by the
/// compiler and the NFAs it builds, with the compile-space budget that
/// `REG_MAX_COMPILE_SPACE` limits.
#[derive(Debug, Default)]
pub(crate) struct Budget {
    err: Cell<i32>,
    space: Cell<usize>,
}

impl Budget {
    fn err(&self) -> i32 {
        self.err.get()
    }

    fn is_err(&self) -> bool {
        self.err.get() != 0
    }

    /// Record `e` unless an error is already recorded.
    fn set_err(&self, e: i32) {
        if self.err.get() == 0 {
            self.err.set(e);
        }
    }

    fn restore_err(&self, e: i32) {
        self.err.set(e);
    }
}

/// A compiled regular expression (`regex_t`).
pub struct Regex {
    guts: compile::Guts,
}

/// One subexpression's match: a range of character offsets, or `None` when
/// the subexpression took no part.
pub type Submatch = Option<(usize, usize)>;

impl Regex {
    /// Compile `pattern` (`TclReComp`).
    pub fn compile(pattern: &[Chr], flags: i32) -> Result<Regex, RegexError> {
        compile::compile(pattern, flags).map(|guts| Regex { guts })
    }

    /// The number of capturing subexpressions (`re_nsub`).
    pub fn nsub(&self) -> usize {
        self.guts.nsub
    }

    /// The facts the compiler noted (`re_info`).
    pub fn info(&self) -> i64 {
        self.guts.info
    }

    /// Match against `text` (`TclReExec`), reporting the whole match and up
    /// to `nmatch - 1` subexpressions; `Ok(None)` when there is no match.
    pub fn exec(
        &self,
        text: &[Chr],
        nmatch: usize,
        eflags: i32,
    ) -> Result<Option<Vec<Submatch>>, RegexError> {
        exec::exec(&self.guts, text, nmatch, eflags)
    }
}

impl fmt::Debug for Regex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Regex")
            .field("nsub", &self.guts.nsub)
            .field("info", &self.guts.info)
            .finish_non_exhaustive()
    }
}

/// Token types, also the NFA arc types that share their codes.
pub(crate) mod token {
    pub const EMPTY: u8 = b'n';
    pub const EOS: u8 = b'e';
    pub const PLAIN: u8 = b'p';
    pub const DIGIT: u8 = b'd';
    pub const BACKREF: u8 = b'b';
    pub const COLLEL: u8 = b'I';
    pub const ECLASS: u8 = b'E';
    pub const CCLASS: u8 = b'C';
    pub const END: u8 = b'X';
    pub const RANGE: u8 = b'R';
    pub const LACON: u8 = b'L';
    pub const AHEAD: u8 = b'a';
    pub const BEHIND: u8 = b'r';
    pub const WBDRY: u8 = b'w';
    pub const NWBDRY: u8 = b'W';
    pub const SBEGIN: u8 = b'A';
    pub const SEND: u8 = b'Z';
}

/// Colors: the character classes the color map partitions characters into.
pub(crate) type Color = i32;
pub(crate) const COLORLESS: Color = -1;
pub(crate) const WHITE: Color = 0;

/// Bounded repetition limits.
pub(crate) const DUPMAX: i32 = 255;
pub(crate) const DUPINF: i32 = DUPMAX + 1;

/// A compacted NFA (`struct cnfa`), what the matcher runs.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cnfa {
    pub nstates: usize,
    pub ncolors: usize,
    pub flags: u8,
    pub pre: usize,
    pub post: usize,
    pub bos: [Color; 2],
    pub eos: [Color; 2],
    pub stflags: Vec<u8>,
    /// Where each state's arcs start in `arcs`; each list ends with an arc
    /// of color `COLORLESS`.
    pub states: Vec<usize>,
    pub arcs: Vec<Carc>,
}

pub(crate) const HASLACONS: u8 = 0o1;
pub(crate) const CNFA_NOPROGRESS: u8 = 0o1;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Carc {
    pub co: Color,
    pub to: usize,
}

/// Subexpression-tree flags (`struct subre`).
pub(crate) mod subflags {
    pub const LONGER: u8 = 0o1;
    pub const SHORTER: u8 = 0o2;
    pub const MIXED: u8 = 0o4;
    pub const CAP: u8 = 0o10;
    pub const BACKR: u8 = 0o20;
    pub const INUSE: u8 = 0o100;
    pub const NOPROP: u8 = 0o3;

    pub const fn lmix(f: u8) -> u8 {
        f << 2
    }
    pub const fn smix(f: u8) -> u8 {
        f << 1
    }
    pub const fn up(f: u8) -> u8 {
        (f & !NOPROP) | (lmix(f) & smix(f) & MIXED)
    }
    pub const fn messy(f: u8) -> bool {
        f & (MIXED | CAP | BACKR) != 0
    }
    pub const fn pref(f: u8) -> u8 {
        f & NOPROP
    }
    pub const fn pref2(f1: u8, f2: u8) -> u8 {
        if pref(f1) != 0 { pref(f1) } else { pref(f2) }
    }
    pub const fn combine(f1: u8, f2: u8) -> u8 {
        up(f1 | f2) | pref2(f1, f2)
    }
}

#[cfg(test)]
mod tests;
