/*
 * re_*comp and friends - compile REs
 * This file #includes several others (see the bottom).
 *
 * Copyright © 1998, 1999 Henry Spencer. All rights reserved.
 *
 * Development of this software was funded, in part, by Cray Research Inc.,
 * UUNET Communications Services Inc., Sun Microsystems Inc., and Scriptics
 * Corporation, none of whom are responsible for the results. The author
 * thanks all of them.
 *
 * Redistribution and use in source and binary forms -- with or without
 * modification -- are permitted for any purpose, provided that
 * redistributions in source form retain this entire copyright notice and
 * indicate the origin and nature of any modifications.
 *
 * I'd appreciate being given credit for this package in the documentation of
 * software which uses it, but that is not a requirement.
 *
 * THIS SOFTWARE IS PROVIDED ``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES,
 * INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY
 * AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL
 * HENRY SPENCER BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 * EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 * PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS;
 * OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
 * WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
 * OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF
 * ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 *
 */

// Ported from Tcl 9.0.2 generic/regcomp.c. Modified for Tcld: native Rust
// data structures and VM integration. Tcl licensing terms: license.terms.

//! The compiler (`regcomp.c` with the lexer of `regc_lex.c`): it parses
//! the expression into one NFA plus a tree of subexpressions, then compacts
//! an NFA for every tree node and lookahead constraint, and a search NFA
//! for the whole.

use std::rc::Rc;

use super::color::ColorMap;
use super::locale::{self, Cvec, iscalnum, iscalpha, iscdigit, iscspace};
use super::nfa::{Nfa, S, dupnfa_into};
use super::subflags::{BACKR, CAP, INUSE, LONGER, SHORTER, combine, messy, pref, up};
use super::token::{
    AHEAD, BACKREF, BEHIND, CCLASS, COLLEL, DIGIT, ECLASS, EMPTY, END, EOS, LACON, NWBDRY, PLAIN,
    RANGE, SBEGIN, SEND, WBDRY,
};
use super::{
    Budget, COLORLESS, Chr, Cnfa, Color, DUPINF, DUPMAX, REG_ADVANCED, REG_ADVF, REG_ASSERT,
    REG_BADBR, REG_BADOPT, REG_BADPAT, REG_BADRPT, REG_BOSONLY, REG_EBRACE, REG_EBRACK,
    REG_ECOLLATE, REG_ECTYPE, REG_EESCAPE, REG_EPAREN, REG_ERANGE, REG_ESUBREG, REG_EXPANDED,
    REG_EXTENDED, REG_ICASE, REG_INVARG, REG_NEWLINE, REG_NLANCH, REG_NLSTOP, REG_NOSUB, REG_QUOTE,
    REG_UBACKREF, REG_UBBS, REG_UBOUNDS, REG_UBRACES, REG_UBSALNUM, REG_ULOCALE, REG_ULOOKAHEAD,
    REG_UNONPOSIX, REG_UPBOTCH, REG_USHORTEST, REG_UUNPORT, REG_UUNSPEC, RegexError,
};

/// A node of the subexpression tree (`struct subre`): `=` a plain
/// expression, `b` a back reference, `(` a capture, `.` a concatenation,
/// `|` an alternation (a chain of them), `*` an iteration.
#[derive(Debug, Clone)]
pub(super) struct Subre {
    pub op: u8,
    pub flags: u8,
    pub id: usize,
    pub subno: usize,
    pub min: i32,
    pub max: i32,
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub begin: S,
    pub end: S,
    pub cnfa: Cnfa,
}

/// A compiled expression's insides (`struct guts`).
pub(super) struct Guts {
    pub cflags: i32,
    pub info: i64,
    pub nsub: usize,
    pub tree: usize,
    pub subres: Vec<Subre>,
    pub search: Cnfa,
    pub ntree: usize,
    pub cmap: ColorMap,
    /// Back references compare ignoring case.
    pub icase: bool,
    /// Lookahead constraints by number; entry 0 is unused.
    pub lacons: Vec<Subre>,
}

/// Lexical contexts.
const L_ERE: u8 = 1;
const L_BRE: u8 = 2;
const L_Q: u8 = 3;
const L_EBND: u8 = 4;
const L_BBND: u8 = 5;
const L_BRACK: u8 = 6;
const L_CEL: u8 = 7;
const L_ECL: u8 = 8;
const L_CCL: u8 = 9;

const fn chrs<const N: usize>(text: &[u8; N]) -> [Chr; N] {
    let mut out = [0; N];
    let mut i = 0;
    while i < N {
        out[i] = text[i] as Chr;
        i += 1;
    }
    out
}

const fn join<const N: usize>(parts: &[&[Chr]]) -> [Chr; N] {
    let mut out = [0; N];
    let mut n = 0;
    let mut p = 0;
    while p < parts.len() {
        let mut i = 0;
        while i < parts[p].len() {
            out[n] = parts[p][i];
            n += 1;
            i += 1;
        }
        p += 1;
    }
    assert!(n == N);
    out
}

/// Connector punctuation, which `\w` adds to `[:alnum:]`.
const PUNCT_CONN: [Chr; 10] = [
    0x5F, 0x203F, 0x2040, 0x2054, 0xFE33, 0xFE34, 0xFE4D, 0xFE4E, 0xFE4F, 0xFF3F,
];

/// What `\d`, `\s`, `\w` and their complements expand to, outside and
/// inside brackets.
static BACKD: [Chr; 11] = chrs(b"[[:digit:]]");
static BACK_UPPER_D: [Chr; 12] = chrs(b"[^[:digit:]]");
static BRBACKD: [Chr; 9] = chrs(b"[:digit:]");
static BACKS: [Chr; 11] = chrs(b"[[:space:]]");
static BACK_UPPER_S: [Chr; 12] = chrs(b"[^[:space:]]");
static BRBACKS: [Chr; 9] = chrs(b"[:space:]");
static BACKW: [Chr; 21] = join(&[&chrs(b"[[:alnum:]"), &PUNCT_CONN, &chrs(b"]")]);
static BACK_UPPER_W: [Chr; 22] = join(&[&chrs(b"[^[:alnum:]"), &PUNCT_CONN, &chrs(b"]")]);
static BRBACKW: [Chr; 19] = join(&[&chrs(b"[:alnum:]"), &PUNCT_CONN]);
static ALERT: [Chr; 5] = chrs(b"alert");
static ESC: [Chr; 3] = chrs(b"ESC");

/// A character as a byte for matching against ASCII punctuation; other
/// characters (and NUL) are never special.
fn ascii(c: Chr) -> u8 {
    if (1..0x80).contains(&c) { c as u8 } else { 0 }
}

/// `compile`.
pub(super) fn compile(string: &[Chr], flags: i32) -> Result<Guts, RegexError> {
    if flags & REG_QUOTE != 0 && flags & (REG_ADVANCED | REG_EXPANDED | REG_NEWLINE) != 0 {
        return Err(RegexError(REG_INVARG));
    }
    if flags & REG_EXTENDED == 0 && flags & REG_ADVF != 0 {
        return Err(RegexError(REG_INVARG));
    }
    let budget = Rc::new(Budget::default());
    let cm = ColorMap::new(budget.clone());
    let Some(nfa) = Nfa::new(budget.clone(), &cm, None) else {
        return Err(RegexError(budget.err()));
    };
    Vars {
        cur: string,
        now: 0,
        stop: string.len(),
        saved: None,
        cflags: flags,
        lasttype: EMPTY,
        nexttype: EMPTY,
        nextvalue: 0,
        lexcon: 0,
        nsubexp: 0,
        subs: vec![None; 10],
        nfa,
        cm,
        nlcolor: COLORLESS,
        wordchrs: None,
        subres: Vec::new(),
        treefree: None,
        lacons: Vec::new(),
        info: 0,
        budget,
    }
    .run()
}

/// The compiler's state (`struct vars`).
struct Vars<'a> {
    /// The text being scanned: the pattern, or an expansion `lexnest`
    /// interpolates, with the scan position and end.
    cur: &'a [Chr],
    now: usize,
    stop: usize,
    saved: Option<(&'a [Chr], usize, usize)>,
    cflags: i32,
    lasttype: u8,
    nexttype: u8,
    nextvalue: usize,
    lexcon: u8,
    nsubexp: usize,
    subs: Vec<Option<usize>>,
    nfa: Nfa,
    cm: ColorMap,
    nlcolor: Color,
    wordchrs: Option<S>,
    subres: Vec<Subre>,
    treefree: Option<usize>,
    lacons: Vec<Subre>,
    info: i64,
    budget: Rc<Budget>,
}

impl<'a> Vars<'a> {
    fn run(mut self) -> Result<Guts, RegexError> {
        let failed = |v: &Vars| v.budget.is_err().then(|| RegexError(v.budget.err()));
        self.lexstart();
        if self.cflags & REG_NLSTOP != 0 || self.cflags & REG_NLANCH != 0 {
            // Newline gets a color of its own.
            self.nlcolor = self.cm.subcolor(Chr::from(b'\n'));
            self.cm.okcolors(&mut self.nfa);
        }
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        let (init, final_) = (self.nfa.init, self.nfa.final_);
        let tree = self.parse(EOS, PLAIN, init, final_);
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        let tree = tree.ok_or(RegexError(REG_ASSERT))?;
        self.nfa.specialcolors(&mut self.cm);
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        let ntree = self.numst(tree, 1);
        self.markst(tree);
        let info = self.nfatree(tree);
        self.info |= info;
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        for i in 1..self.lacons.len() {
            let (begin, end) = (self.lacons[i].begin, self.lacons[i].end);
            let (_, cnfa) = self.nfanode(begin, end);
            self.lacons[i].cnfa = cnfa;
        }
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        if self.subres[tree].flags & SHORTER != 0 {
            self.note(REG_USHORTEST);
        }
        // The main NFA can now serve as the work area for the search NFA.
        self.nfa.optimize();
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        self.makesearch();
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        let search = self.nfa.compact(&self.cm);
        if let Some(e) = failed(&self) {
            return Err(e);
        }
        Ok(Guts {
            cflags: self.cflags,
            info: self.info,
            nsub: self.nsubexp,
            tree,
            subres: self.subres,
            search,
            ntree,
            cmap: self.cm,
            icase: self.cflags & REG_ICASE != 0,
            lacons: self.lacons,
        })
    }

    // Errors and notes.

    fn iserr(&self) -> bool {
        self.budget.is_err()
    }

    /// `ERR`: record an error, which also ends the token stream.
    fn err(&mut self, e: i32) {
        self.nexttype = EOS;
        self.budget.set_err(e);
    }

    /// `INSIST`.
    fn insist(&mut self, ok: bool, e: i32) {
        if !ok {
            self.err(e);
        }
    }

    /// `NOTE`: note a visible condition in `re_info`.
    fn note(&mut self, b: i64) {
        self.info |= b;
    }

    /// The next token's type; any error, including one an NFA or the color
    /// map recorded, makes it EOS (`VERR` sets it).
    fn nexttype(&self) -> u8 {
        if self.iserr() { EOS } else { self.nexttype }
    }

    /// `SEE`.
    fn see(&self, t: u8) -> bool {
        self.nexttype() == t
    }

    /// `EAT`.
    fn eat(&mut self, t: u8) -> bool {
        self.see(t) && self.next()
    }

    fn newarc(&mut self, t: u8, co: Color, from: S, to: S) {
        self.nfa.newarc(t, co, from, to);
    }

    /// `EMPTYARC`.
    fn emptyarc(&mut self, from: S, to: S) {
        self.nfa.newarc(EMPTY, 0, from, to);
    }

    fn newstate(&mut self) -> S {
        self.nfa.newstate()
    }

    // The lexer (regc_lex.c).

    fn ateos(&self) -> bool {
        self.now >= self.stop
    }

    /// `HAVE`.
    fn have(&self, n: usize) -> bool {
        self.stop.saturating_sub(self.now) >= n
    }

    fn peek(&self, offset: usize) -> Chr {
        self.cur[self.now + offset]
    }

    /// `NEXT1`.
    fn next1(&self, c: u8) -> bool {
        !self.ateos() && self.peek(0) == Chr::from(c)
    }

    /// `NEXT2`.
    fn next2(&self, a: u8, b: u8) -> bool {
        self.have(2) && self.peek(0) == Chr::from(a) && self.peek(1) == Chr::from(b)
    }

    /// `NEXT3`.
    fn next3(&self, a: u8, b: u8, c: u8) -> bool {
        self.have(3)
            && self.peek(0) == Chr::from(a)
            && self.peek(1) == Chr::from(b)
            && self.peek(2) == Chr::from(c)
    }

    /// The character at the scan position, which advances; past the end
    /// it is NUL, the terminator C reads there.
    fn take(&mut self) -> Chr {
        let c = self.cur.get(self.now).copied().unwrap_or(0);
        self.now += 1;
        c
    }

    /// `RET`.
    fn ret(&mut self, t: u8) -> bool {
        self.nexttype = t;
        true
    }

    /// `RETV`.
    fn retv(&mut self, t: u8, value: usize) -> bool {
        self.nexttype = t;
        self.nextvalue = value;
        true
    }

    /// `FAILW`.
    fn failw(&mut self, e: i32) -> bool {
        self.err(e);
        false
    }

    /// `lexstart`: lexical setup, including the leading options.
    fn lexstart(&mut self) {
        self.prefixes();
        if self.iserr() {
            return;
        }
        self.lexcon = if self.cflags & REG_QUOTE != 0 {
            L_Q
        } else if self.cflags & REG_EXTENDED != 0 {
            L_ERE
        } else {
            L_BRE
        };
        self.nexttype = EMPTY;
        self.next();
    }

    /// `prefixes`: `***=`, `***:` and embedded options.
    fn prefixes(&mut self) {
        if self.cflags & REG_QUOTE != 0 {
            return;
        }
        if self.have(4) && self.next3(b'*', b'*', b'*') {
            match ascii(self.peek(3)) {
                b'?' => {
                    self.err(REG_BADPAT);
                    return;
                }
                b'=' => {
                    self.note(REG_UNONPOSIX);
                    self.cflags |= REG_QUOTE;
                    self.cflags &= !(REG_ADVANCED | REG_EXPANDED | REG_NEWLINE);
                    self.now += 4;
                    return;
                }
                b':' => {
                    self.note(REG_UNONPOSIX);
                    self.cflags |= REG_ADVANCED;
                    self.now += 4;
                }
                _ => {
                    self.err(REG_BADRPT);
                    return;
                }
            }
        }
        if self.cflags & REG_ADVANCED != REG_ADVANCED {
            return;
        }
        if self.have(3) && self.next2(b'(', b'?') && iscalpha(self.peek(2)) {
            self.note(REG_UNONPOSIX);
            self.now += 2;
            while !self.ateos() && iscalpha(self.peek(0)) {
                match ascii(self.peek(0)) {
                    b'b' => self.cflags &= !(REG_ADVANCED | REG_QUOTE),
                    b'c' => self.cflags &= !REG_ICASE,
                    b'e' => {
                        self.cflags |= REG_EXTENDED;
                        self.cflags &= !(REG_ADVF | REG_QUOTE);
                    }
                    b'i' => self.cflags |= REG_ICASE,
                    b'm' | b'n' => self.cflags |= REG_NEWLINE,
                    b'p' => {
                        self.cflags |= REG_NLSTOP;
                        self.cflags &= !REG_NLANCH;
                    }
                    b'q' => {
                        self.cflags |= REG_QUOTE;
                        self.cflags &= !REG_ADVANCED;
                    }
                    b's' => self.cflags &= !REG_NEWLINE,
                    b't' => self.cflags &= !REG_EXPANDED,
                    b'w' => {
                        self.cflags &= !REG_NLSTOP;
                        self.cflags |= REG_NLANCH;
                    }
                    b'x' => self.cflags |= REG_EXPANDED,
                    _ => {
                        self.err(REG_BADOPT);
                        return;
                    }
                }
                self.now += 1;
            }
            if !self.next1(b')') {
                self.err(REG_BADOPT);
                return;
            }
            self.now += 1;
            if self.cflags & REG_QUOTE != 0 {
                self.cflags &= !(REG_EXPANDED | REG_NEWLINE);
            }
        }
    }

    /// `lexnest`: interpolate a string at the lexical level.
    fn lexnest(&mut self, text: &'a [Chr]) {
        self.saved = Some((self.cur, self.now, self.stop));
        self.cur = text;
        self.now = 0;
        self.stop = text.len();
    }

    /// `lexword`: interpolate the bracket expression of word characters.
    fn lexword(&mut self) {
        self.lexnest(&BACKW);
    }

    /// `next`: get the next token; false on failure.
    fn next(&mut self) -> bool {
        if self.iserr() {
            return false;
        }
        self.lasttype = self.nexttype;
        if self.nexttype == EMPTY && self.cflags & REG_BOSONLY != 0 {
            return self.retv(SBEGIN, 0);
        }
        // Back from an interpolation at its end.
        if self.ateos()
            && let Some((cur, now, stop)) = self.saved.take()
        {
            self.cur = cur;
            self.now = now;
            self.stop = stop;
        }
        if self.cflags & REG_EXPANDED != 0 && matches!(self.lexcon, L_ERE | L_BRE | L_EBND | L_BBND)
        {
            self.skip();
        }
        if self.ateos() {
            return match self.lexcon {
                L_EBND | L_BBND => self.failw(REG_EBRACE),
                L_BRACK | L_CEL | L_ECL | L_CCL => self.failw(REG_EBRACK),
                _ => self.ret(EOS),
            };
        }
        let c = self.take();
        match self.lexcon {
            L_BRE => return self.brenext(c),
            L_ERE => {}
            L_Q => return self.retv(PLAIN, c as usize),
            L_BBND | L_EBND => {
                return match ascii(c) {
                    b'0'..=b'9' => self.retv(DIGIT, (c - Chr::from(b'0')) as usize),
                    b',' => self.ret(b','),
                    b'}' => {
                        if self.lexcon == L_EBND {
                            self.lexcon = L_ERE;
                            if self.cflags & REG_ADVF != 0 && self.next1(b'?') {
                                self.now += 1;
                                self.note(REG_UNONPOSIX);
                                return self.retv(b'}', 0);
                            }
                            self.retv(b'}', 1)
                        } else {
                            self.failw(REG_BADBR)
                        }
                    }
                    b'\\' => {
                        if self.lexcon == L_BBND && self.next1(b'}') {
                            self.now += 1;
                            self.lexcon = L_BRE;
                            self.retv(b'}', 1)
                        } else {
                            self.failw(REG_BADBR)
                        }
                    }
                    _ => self.failw(REG_BADBR),
                };
            }
            L_BRACK => {
                return match ascii(c) {
                    b']' => {
                        if self.lasttype == b'[' {
                            self.retv(PLAIN, c as usize)
                        } else {
                            self.lexcon = if self.cflags & REG_EXTENDED != 0 {
                                L_ERE
                            } else {
                                L_BRE
                            };
                            self.ret(b']')
                        }
                    }
                    b'\\' => {
                        self.note(REG_UBBS);
                        if self.cflags & REG_ADVF == 0 {
                            return self.retv(PLAIN, c as usize);
                        }
                        self.note(REG_UNONPOSIX);
                        if self.ateos() {
                            return self.failw(REG_EESCAPE);
                        }
                        self.lexescape();
                        match self.nexttype() {
                            PLAIN => true,
                            CCLASS => {
                                match self.nextvalue as u8 {
                                    b'd' => self.lexnest(&BRBACKD),
                                    b's' => self.lexnest(&BRBACKS),
                                    b'w' => self.lexnest(&BRBACKW),
                                    _ => return self.failw(REG_EESCAPE),
                                }
                                // Back up and try again.
                                self.nexttype = self.lasttype;
                                self.next()
                            }
                            // Not one of the escapes allowed here.
                            _ => self.failw(REG_EESCAPE),
                        }
                    }
                    b'-' => {
                        if self.lasttype == b'[' || self.next1(b']') {
                            self.retv(PLAIN, c as usize)
                        } else {
                            self.retv(RANGE, c as usize)
                        }
                    }
                    b'[' => {
                        if self.ateos() {
                            return self.failw(REG_EBRACK);
                        }
                        match ascii(self.take()) {
                            b'.' => {
                                self.lexcon = L_CEL;
                                self.ret(COLLEL)
                            }
                            b'=' => {
                                self.lexcon = L_ECL;
                                self.note(REG_ULOCALE);
                                self.ret(ECLASS)
                            }
                            b':' => {
                                self.lexcon = L_CCL;
                                self.note(REG_ULOCALE);
                                self.ret(CCLASS)
                            }
                            _ => {
                                self.now -= 1;
                                self.retv(PLAIN, c as usize)
                            }
                        }
                    }
                    _ => self.retv(PLAIN, c as usize),
                };
            }
            L_CEL | L_ECL | L_CCL => {
                let closer = match self.lexcon {
                    L_CEL => b'.',
                    L_ECL => b'=',
                    _ => b':',
                };
                return if ascii(c) == closer && self.next1(b']') {
                    self.now += 1;
                    self.lexcon = L_BRACK;
                    self.retv(END, closer as usize)
                } else {
                    self.retv(PLAIN, c as usize)
                };
            }
            _ => return self.failw(REG_ASSERT),
        }
        // EREs and AREs, except for backslashes.
        match ascii(c) {
            b'|' => return self.ret(b'|'),
            b'*' | b'+' | b'?' => {
                let t = c as u8;
                if self.cflags & REG_ADVF != 0 && self.next1(b'?') {
                    self.now += 1;
                    self.note(REG_UNONPOSIX);
                    return self.retv(t, 0);
                }
                return self.retv(t, 1);
            }
            b'{' => {
                // A bound, or a plain character.
                if self.cflags & REG_EXPANDED != 0 {
                    self.skip();
                }
                if self.ateos() || !iscdigit(self.peek(0)) {
                    self.note(REG_UBRACES);
                    self.note(REG_UUNSPEC);
                    return self.retv(PLAIN, c as usize);
                }
                self.note(REG_UBOUNDS);
                self.lexcon = L_EBND;
                return self.ret(b'{');
            }
            b'(' => {
                if self.cflags & REG_ADVF != 0 && self.next1(b'?') {
                    self.note(REG_UNONPOSIX);
                    self.now += 1;
                    return match ascii(self.take()) {
                        // Non-capturing parenthesis.
                        b':' => self.retv(b'(', 0),
                        // A comment.
                        b'#' => {
                            while !self.ateos() && self.peek(0) != Chr::from(b')') {
                                self.now += 1;
                            }
                            if !self.ateos() {
                                self.now += 1;
                            }
                            self.next()
                        }
                        b'=' => {
                            self.note(REG_ULOOKAHEAD);
                            self.retv(LACON, 1)
                        }
                        b'!' => {
                            self.note(REG_ULOOKAHEAD);
                            self.retv(LACON, 0)
                        }
                        _ => self.failw(REG_BADRPT),
                    };
                }
                let capturing = self.cflags & REG_NOSUB == 0;
                return self.retv(b'(', usize::from(capturing));
            }
            b')' => {
                if self.lasttype == b'(' {
                    self.note(REG_UUNSPEC);
                }
                return self.retv(b')', c as usize);
            }
            b'[' => {
                // Easy except for [[:<:]] and [[:>:]].
                if let Some(t) = self.word_boundary_bracket() {
                    return self.ret(t);
                }
                self.lexcon = L_BRACK;
                if self.next1(b'^') {
                    self.now += 1;
                    return self.retv(b'[', 0);
                }
                return self.retv(b'[', 1);
            }
            b'.' => return self.ret(b'.'),
            b'^' => return self.ret(b'^'),
            b'$' => return self.ret(b'$'),
            b'\\' => {
                if self.ateos() {
                    return self.failw(REG_EESCAPE);
                }
            }
            _ => return self.retv(PLAIN, c as usize),
        }
        // ERE/ARE backslashes; the backslash is eaten.
        if self.cflags & REG_ADVF == 0 {
            // Only AREs have non-trivial escapes.
            if iscalnum(self.peek(0)) {
                self.note(REG_UBSALNUM);
                self.note(REG_UUNSPEC);
            }
            let c = self.take();
            return self.retv(PLAIN, c as usize);
        }
        self.lexescape();
        if self.iserr() {
            return self.failw(REG_EESCAPE);
        }
        if self.nexttype == CCLASS {
            // Fudge at the lexical level.
            match self.nextvalue as u8 {
                b'd' => self.lexnest(&BACKD),
                b'D' => self.lexnest(&BACK_UPPER_D),
                b's' => self.lexnest(&BACKS),
                b'S' => self.lexnest(&BACK_UPPER_S),
                b'w' => self.lexnest(&BACKW),
                b'W' => self.lexnest(&BACK_UPPER_W),
                _ => return self.failw(REG_ASSERT),
            }
            // Back up and try again.
            self.nexttype = self.lasttype;
            return self.next();
        }
        !self.iserr()
    }

    /// `[[:<:]]` or `[[:>:]]` at the scan position: the word-boundary
    /// token, with the brackets eaten.
    fn word_boundary_bracket(&mut self) -> Option<u8> {
        if self.have(6)
            && self.peek(0) == Chr::from(b'[')
            && self.peek(1) == Chr::from(b':')
            && matches!(ascii(self.peek(2)), b'<' | b'>')
            && self.peek(3) == Chr::from(b':')
            && self.peek(4) == Chr::from(b']')
            && self.peek(5) == Chr::from(b']')
        {
            let t = ascii(self.peek(2));
            self.now += 6;
            self.note(REG_UNONPOSIX);
            Some(t)
        } else {
            None
        }
    }

    /// `lexescape`: an ARE backslash escape, the backslash eaten. Class
    /// escapes come back as CCLASS tokens.
    fn lexescape(&mut self) -> bool {
        let c = self.take();
        if !iscalnum(c) {
            return self.retv(PLAIN, c as usize);
        }
        self.note(REG_UNONPOSIX);
        match ascii(c) {
            b'a' => {
                let c = self.chrnamed(&ALERT, 0x07);
                self.retv(PLAIN, c as usize)
            }
            b'A' => self.retv(SBEGIN, 0),
            b'b' => self.retv(PLAIN, 0x08),
            b'B' => self.retv(PLAIN, usize::from(b'\\')),
            b'c' => {
                self.note(REG_UUNPORT);
                if self.ateos() {
                    return self.failw(REG_EESCAPE);
                }
                let c = self.take();
                self.retv(PLAIN, (c & 0o37) as usize)
            }
            b'd' | b'D' | b's' | b'S' | b'w' | b'W' => {
                self.note(REG_ULOCALE);
                self.retv(CCLASS, c as usize)
            }
            b'e' => {
                self.note(REG_UUNPORT);
                let c = self.chrnamed(&ESC, 0x1B);
                self.retv(PLAIN, c as usize)
            }
            b'f' => self.retv(PLAIN, 0x0C),
            b'm' => self.ret(b'<'),
            b'M' => self.ret(b'>'),
            b'n' => self.retv(PLAIN, 0x0A),
            b'r' => self.retv(PLAIN, 0x0D),
            b't' => self.retv(PLAIN, 0x09),
            b'u' => {
                let c = self.lexdigits(16, 1, 4);
                if self.iserr() {
                    return self.failw(REG_EESCAPE);
                }
                self.retv(PLAIN, c as usize)
            }
            b'U' => {
                let c = self.lexdigits(16, 1, 8);
                if self.iserr() {
                    return self.failw(REG_EESCAPE);
                }
                self.retv(PLAIN, c as usize)
            }
            b'v' => self.retv(PLAIN, 0x0B),
            b'x' => {
                self.note(REG_UUNPORT);
                let c = self.lexdigits(16, 1, 2);
                if self.iserr() {
                    return self.failw(REG_EESCAPE);
                }
                self.retv(PLAIN, c as usize)
            }
            b'y' => {
                self.note(REG_ULOCALE);
                self.retv(WBDRY, 0)
            }
            b'Y' => {
                self.note(REG_ULOCALE);
                self.retv(NWBDRY, 0)
            }
            b'Z' => self.retv(SEND, 0),
            b'1'..=b'9' => {
                let save = self.now;
                self.now -= 1;
                let c = self.lexdigits(10, 1, 255);
                if self.iserr() {
                    return self.failw(REG_EESCAPE);
                }
                // Ugly heuristic: exactly one digit, or a subexpression
                // number seen so far.
                if self.now == save || (c > 0 && c as usize <= self.nsubexp) {
                    self.note(REG_UBACKREF);
                    return self.retv(BACKREF, c as usize);
                }
                // Not a back reference after all.
                self.now = save;
                self.octal_escape()
            }
            b'0' => self.octal_escape(),
            // An unknown alphanumeric escape.
            _ => self.failw(REG_EESCAPE),
        }
    }

    /// The octal escape whose first digit was just eaten.
    fn octal_escape(&mut self) -> bool {
        self.note(REG_UUNPORT);
        self.now -= 1;
        let mut c = self.lexdigits(8, 1, 3);
        if self.iserr() {
            return self.failw(REG_EESCAPE);
        }
        if c > 0xFF {
            // Out of range, so one digit too many was taken.
            self.now -= 1;
            c >>= 3;
        }
        self.retv(PLAIN, c as usize)
    }

    /// `lexdigits`: digits in `base`, at least `minlen` and at most
    /// `maxlen` of them.
    fn lexdigits(&mut self, base: u32, minlen: usize, maxlen: usize) -> u32 {
        let mut n: u32 = 0;
        let mut len = 0;
        while len < maxlen && !self.ateos() {
            if n > 0x10FFF {
                // Stop where going on would overflow.
                break;
            }
            let c = self.take();
            let d = match ascii(c) {
                b @ b'0'..=b'9' => u32::from(b - b'0'),
                b @ b'a'..=b'f' => u32::from(b - b'a') + 10,
                b @ b'A'..=b'F' => u32::from(b - b'A') + 10,
                _ => {
                    self.now -= 1;
                    break;
                }
            };
            if d >= base {
                self.now -= 1;
                break;
            }
            n = n * base + d;
            len += 1;
        }
        if len < minlen {
            self.err(REG_EESCAPE);
        }
        n
    }

    /// `brenext`: the next BRE token.
    fn brenext(&mut self, c: Chr) -> bool {
        match ascii(c) {
            b'*' => {
                if matches!(self.lasttype, EMPTY | b'(' | b'^') {
                    return self.retv(PLAIN, c as usize);
                }
                return self.retv(b'*', 1);
            }
            b'[' => {
                if let Some(t) = self.word_boundary_bracket() {
                    return self.ret(t);
                }
                self.lexcon = L_BRACK;
                if self.next1(b'^') {
                    self.now += 1;
                    return self.retv(b'[', 0);
                }
                return self.retv(b'[', 1);
            }
            b'.' => return self.ret(b'.'),
            b'^' => {
                if self.lasttype == EMPTY {
                    return self.ret(b'^');
                }
                if self.lasttype == b'(' {
                    self.note(REG_UUNSPEC);
                    return self.ret(b'^');
                }
                return self.retv(PLAIN, c as usize);
            }
            b'$' => {
                if self.cflags & REG_EXPANDED != 0 {
                    self.skip();
                }
                if self.ateos() {
                    return self.ret(b'$');
                }
                if self.next2(b'\\', b')') {
                    self.note(REG_UUNSPEC);
                    return self.ret(b'$');
                }
                return self.retv(PLAIN, c as usize);
            }
            b'\\' => {}
            _ => return self.retv(PLAIN, c as usize),
        }
        if self.ateos() {
            return self.failw(REG_EESCAPE);
        }
        let c = self.take();
        match ascii(c) {
            b'{' => {
                self.lexcon = L_BBND;
                self.note(REG_UBOUNDS);
                self.ret(b'{')
            }
            b'(' => self.retv(b'(', 1),
            b')' => self.retv(b')', c as usize),
            b'<' => {
                self.note(REG_UNONPOSIX);
                self.ret(b'<')
            }
            b'>' => {
                self.note(REG_UNONPOSIX);
                self.ret(b'>')
            }
            b'1'..=b'9' => {
                self.note(REG_UBACKREF);
                self.retv(BACKREF, (c - Chr::from(b'0')) as usize)
            }
            _ => {
                if iscalnum(c) {
                    self.note(REG_UBSALNUM);
                    self.note(REG_UUNSPEC);
                }
                self.retv(PLAIN, c as usize)
            }
        }
    }

    /// `skip`: white space and comments in expanded syntax.
    fn skip(&mut self) {
        let start = self.now;
        loop {
            while !self.ateos() && iscspace(self.peek(0)) {
                self.now += 1;
            }
            if self.ateos() || self.peek(0) != Chr::from(b'#') {
                break;
            }
            while !self.ateos() && self.peek(0) != Chr::from(b'\n') {
                self.now += 1;
            }
        }
        if self.now != start {
            self.note(REG_UNONPOSIX);
        }
    }

    /// `chrnamed`: the character a name stands for, or `lastresort`.
    fn chrnamed(&mut self, name: &[Chr], lastresort: Chr) -> Chr {
        let errsave = self.budget.err();
        self.budget.restore_err(0);
        let c = self.element(name);
        let e = self.budget.err();
        self.budget.restore_err(errsave);
        if e != 0 {
            return lastresort;
        }
        // range() of one character without cases has no single characters.
        match self.range(c, c, false) {
            Some(cv) if !cv.chrs.is_empty() => cv.chrs[0],
            _ => lastresort,
        }
    }

    // The parser (regcomp.c).

    /// `moresubs`: enlarge the subexpression vector.
    fn moresubs(&mut self, wanted: usize) {
        let n = wanted * 3 / 2 + 1;
        self.subs.resize(n, None);
    }

    /// `parse`: branches joined by `|`, which hang as the left children of
    /// a chain of `|` nodes.
    fn parse(&mut self, stopper: u8, typ: u8, init: S, final_: S) -> Option<usize> {
        let branches = self.sub_re(b'|', LONGER, init, final_);
        if self.iserr() {
            return None;
        }
        let mut branch = branches;
        let mut firstbranch = true;
        loop {
            if !firstbranch {
                // A place to hang the branch.
                let next = self.sub_re(b'|', LONGER, init, final_);
                self.subres[branch].right = Some(next);
                if self.iserr() {
                    return None;
                }
                branch = next;
            }
            firstbranch = false;
            let left = self.newstate();
            let right = self.newstate();
            if self.iserr() {
                return None;
            }
            self.emptyarc(init, left);
            self.emptyarc(right, final_);
            if self.iserr() {
                return None;
            }
            let content = self.parsebranch(stopper, typ, left, right, false);
            self.subres[branch].left = content;
            let content = content?;
            if self.iserr() {
                return None;
            }
            let flags = self.subres[branch].flags | self.subres[content].flags;
            self.subres[branch].flags |= up(flags);
            let flags = self.subres[branch].flags;
            if flags & !self.subres[branches].flags != 0 {
                // New flags, which the earlier branches get too.
                let mut t = branches;
                while t != branch {
                    self.subres[t].flags |= flags;
                    t = self.subres[t].right.expect("branch chain");
                }
            }
            if !self.eat(b'|') {
                break;
            }
        }
        if !self.see(stopper) {
            self.err(REG_EPAREN);
        }
        // Optimize out simple cases.
        if branch == branches {
            let only = self.subres[branch].left.take();
            self.freesubre(Some(branches));
            return only;
        }
        if !messy(self.subres[branches].flags) {
            let left = self.subres[branches].left.take();
            self.freesubre(left);
            let right = self.subres[branches].right.take();
            self.freesubre(right);
            self.subres[branches].op = b'=';
        }
        Some(branches)
    }

    /// `parsebranch`: one branch, as concatenated atoms bundled into as few
    /// nodes as the substructure allows.
    fn parsebranch(
        &mut self,
        stopper: u8,
        typ: u8,
        left: S,
        right: S,
        partial: bool,
    ) -> Option<usize> {
        let mut lp = left;
        let mut seencontent = false;
        let t = self.sub_re(b'=', 0, left, right);
        if self.iserr() {
            return None;
        }
        while !self.see(b'|') && !self.see(stopper) && !self.see(EOS) {
            if seencontent {
                // An implicit concatenation operator.
                lp = self.newstate();
                if self.iserr() {
                    return None;
                }
                self.nfa.moveins(right, lp);
            }
            seencontent = true;
            // The recursion in parseqatom() may swallow the rest of the
            // branch.
            self.parseqatom(stopper, typ, lp, right, t);
            if self.iserr() {
                return None;
            }
        }
        if !seencontent {
            if !partial {
                self.note(REG_UUNSPEC);
            }
            self.emptyarc(left, right);
        }
        Some(t)
    }

    /// `parseqatom`: one quantified atom or constraint, strung between
    /// `lp` and `rp`; with substructure it parses the rest of the branch
    /// too.
    fn parseqatom(&mut self, stopper: u8, typ: u8, lp: S, rp: S, top: usize) {
        let mut atom: Option<usize> = None;
        let mut subno = 0;
        let mut atomtype = self.nexttype();
        match atomtype {
            // Constraints, which end by returning.
            b'^' => {
                self.newarc(b'^', 1, lp, rp);
                if self.cflags & REG_NLANCH != 0 {
                    self.newarc(BEHIND, self.nlcolor, lp, rp);
                }
                self.next();
                return;
            }
            b'$' => {
                self.newarc(b'$', 1, lp, rp);
                if self.cflags & REG_NLANCH != 0 {
                    self.newarc(AHEAD, self.nlcolor, lp, rp);
                }
                self.next();
                return;
            }
            SBEGIN => {
                self.newarc(b'^', 1, lp, rp);
                self.newarc(b'^', 0, lp, rp);
                self.next();
                return;
            }
            SEND => {
                self.newarc(b'$', 1, lp, rp);
                self.newarc(b'$', 0, lp, rp);
                self.next();
                return;
            }
            b'<' => {
                self.wordchrs();
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nonword(BEHIND, lp, s);
                self.word(AHEAD, s, rp);
                return;
            }
            b'>' => {
                self.wordchrs();
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.word(BEHIND, lp, s);
                self.nonword(AHEAD, s, rp);
                return;
            }
            WBDRY => {
                self.wordchrs();
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nonword(BEHIND, lp, s);
                self.word(AHEAD, s, rp);
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.word(BEHIND, lp, s);
                self.nonword(AHEAD, s, rp);
                return;
            }
            NWBDRY => {
                self.wordchrs();
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.word(BEHIND, lp, s);
                self.word(AHEAD, s, rp);
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nonword(BEHIND, lp, s);
                self.nonword(AHEAD, s, rp);
                return;
            }
            LACON => {
                let pos = self.nextvalue;
                self.next();
                let s = self.newstate();
                let s2 = self.newstate();
                if self.iserr() {
                    return;
                }
                let t = self.parse(b')', LACON, s, s2);
                // The internal structure does not matter.
                self.freesubre(t);
                self.next();
                let n = self.newlacon(s, s2, pos);
                if self.iserr() {
                    return;
                }
                self.newarc(LACON, n as Color, lp, rp);
                return;
            }
            // Errors, to get them out of the way.
            b'*' | b'+' | b'?' | b'{' => {
                self.err(REG_BADRPT);
                return;
            }
            // Plain characters, and minor variants on that theme.
            b')' | PLAIN => {
                if atomtype == b')' {
                    // An unbalanced parenthesis, legal in EREs due to a
                    // specification botch.
                    if self.cflags & REG_ADVANCED != REG_EXTENDED {
                        self.err(REG_EPAREN);
                        return;
                    }
                    self.note(REG_UPBOTCH);
                }
                self.onechr(self.nextvalue as Chr, lp, rp);
                self.cm.okcolors(&mut self.nfa);
                if self.iserr() {
                    return;
                }
                self.next();
            }
            b'[' => {
                if self.nextvalue == 1 {
                    self.bracket(lp, rp);
                } else {
                    self.cbracket(lp, rp);
                }
                self.next();
            }
            b'.' => {
                let but = if self.cflags & REG_NLSTOP != 0 {
                    self.nlcolor
                } else {
                    COLORLESS
                };
                self.nfa.rainbow(&self.cm, PLAIN, but, lp, rp);
                self.next();
            }
            // And finally the ugly stuff.
            b'(' => {
                let cap = typ != LACON && self.nextvalue != 0;
                if cap {
                    self.nsubexp += 1;
                    subno = self.nsubexp;
                    if subno >= self.subs.len() {
                        self.moresubs(subno);
                    }
                } else {
                    // Something that is not '('.
                    atomtype = PLAIN;
                }
                self.next();
                // New endpoints, since the tree will point at them.
                let s = self.newstate();
                let s2 = self.newstate();
                if self.iserr() {
                    return;
                }
                self.emptyarc(lp, s);
                self.emptyarc(s2, rp);
                if self.iserr() {
                    return;
                }
                atom = self.parse(b')', PLAIN, s, s2);
                self.next();
                if self.iserr() {
                    return;
                }
                if cap {
                    let inner = atom.expect("parenthesized atom");
                    self.subs[subno] = Some(inner);
                    let t = self.sub_re(b'(', self.subres[inner].flags | CAP, lp, rp);
                    if self.iserr() {
                        return;
                    }
                    self.subres[t].subno = subno;
                    self.subres[t].left = Some(inner);
                    atom = Some(t);
                }
                // Everything else waits for a possible {0}.
            }
            BACKREF => {
                // The Feature From The Black Lagoon.
                let n = self.nextvalue;
                self.insist(typ != LACON, REG_ESUBREG);
                self.insist(n < self.subs.len(), REG_ESUBREG);
                let known = self.subs.get(n).is_some_and(Option::is_some);
                self.insist(known, REG_ESUBREG);
                if self.iserr() {
                    return;
                }
                let t = self.sub_re(b'b', BACKR, lp, rp);
                if self.iserr() {
                    return;
                }
                subno = n;
                self.subres[t].subno = subno;
                atom = Some(t);
                // Temporarily, so there is something.
                self.emptyarc(lp, rp);
                self.next();
            }
            _ => {
                self.err(REG_ASSERT);
                return;
            }
        }

        // ...and an atom may be followed by a quantifier.
        let (m, n, qprefer) = match self.nexttype() {
            b'*' | b'+' | b'?' => {
                let (m, n) = match self.nexttype() {
                    b'*' => (0, DUPINF),
                    b'+' => (1, DUPINF),
                    _ => (0, 1),
                };
                let qprefer = if self.nextvalue != 0 { LONGER } else { SHORTER };
                self.next();
                (m, n, qprefer)
            }
            b'{' => {
                self.next();
                let m = self.scannum();
                let (n, qprefer) = if self.eat(b',') {
                    let n = if self.see(DIGIT) {
                        self.scannum()
                    } else {
                        DUPINF
                    };
                    if m > n {
                        self.err(REG_BADBR);
                        return;
                    }
                    // {m,n} exercises preference, even as {m,m}.
                    (n, if self.nextvalue != 0 { LONGER } else { SHORTER })
                } else {
                    // {m} passes the operand's preference through.
                    (m, 0)
                };
                if !self.see(b'}') {
                    // This catches errors too.
                    self.err(REG_BADBR);
                    return;
                }
                self.next();
                (m, n, qprefer)
            }
            _ => (1, 1, 0),
        };

        // Annoying special case: {0} or {0,0} cancels everything.
        if m == 0 && n == 0 {
            self.freesubre(atom);
            if atomtype == b'(' {
                self.subs[subno] = None;
            }
            self.nfa.delsub(lp, rp);
            self.emptyarc(lp, rp);
            return;
        }

        // Not a messy case: avoid the hard part.
        let atomflags = atom.map_or(0, |a| self.subres[a].flags);
        let f = self.subres[top].flags | qprefer | atomflags;
        if atomtype != b'(' && atomtype != BACKREF && !messy(up(f)) {
            if !(m == 1 && n == 1) {
                self.repeat(lp, rp, m, n);
            }
            self.freesubre(atom);
            self.subres[top].flags = f;
            return;
        }

        // The hard part: capturing parentheses, a back reference, a
        // short/long clash, or substructure containing one of those.
        let atom = match atom {
            Some(atom) => atom,
            None => {
                let atom = self.sub_re(b'=', 0, lp, rp);
                if self.iserr() {
                    return;
                }
                atom
            }
        };

        // A general-purpose skeleton: [lp] -> [s] -prefix-> [begin] -atom->
        // [end] -rest-> [rp], or with an iterator from [s] to [s2].
        let s = self.newstate();
        let s2 = self.newstate();
        if self.iserr() {
            return;
        }
        self.nfa.moveouts(lp, s);
        self.nfa.moveins(rp, s2);
        if self.iserr() {
            return;
        }
        self.subres[atom].begin = s;
        self.subres[atom].end = s2;
        let s = self.newstate();
        if self.iserr() {
            return;
        }
        self.emptyarc(lp, s);
        if self.iserr() {
            return;
        }

        // Split the remaining subexpression into x{...} and what follows.
        let t = self.sub_re(b'.', combine(qprefer, self.subres[atom].flags), lp, rp);
        if self.iserr() {
            return;
        }
        self.subres[t].left = Some(atom);
        // The atom's slot is t's left child (`atomp`).
        let atom_slot = t;

        // Split top into prefix and remaining.
        let prefix = self.sub_re(b'=', self.subres[top].flags, self.subres[top].begin, lp);
        self.subres[top].left = Some(prefix);
        if self.iserr() {
            return;
        }
        self.subres[top].op = b'.';
        self.subres[top].right = Some(t);

        // A back reference replicates the sub-NFA it refers to, now that
        // the skeleton is filled in.
        if atomtype == BACKREF {
            let (begin, end) = (self.subres[atom].begin, self.subres[atom].end);
            self.nfa.delsub(begin, end);
            let referenced = self.subs[subno].expect("back-referenced subexpression");
            let (rbegin, rend) = (self.subres[referenced].begin, self.subres[referenced].end);
            self.nfa.dupnfa(rbegin, rend, begin, end);
            if self.iserr() {
                return;
            }
        }

        // It's quantifier time.
        let (abegin, aend) = (self.subres[atom].begin, self.subres[atom].end);
        let s2 = if atomtype == BACKREF {
            // Back references take their quantifiers internally.
            self.emptyarc(s, abegin);
            self.repeat(abegin, aend, m, n);
            self.subres[atom].min = m;
            self.subres[atom].max = n;
            let aflags = self.subres[atom].flags;
            self.subres[atom].flags |= combine(qprefer, aflags);
            aend
        } else if m == 1 && n == 1 {
            // No quantifier, or a vacuous one.
            self.emptyarc(s, abegin);
            aend
        } else if m > 0 && self.subres[atom].flags & BACKR == 0 {
            // Without back references, x{m,n} is x{m-1,n-1}x with the
            // captures in the second x only, so the prefix can be a DFA.
            self.nfa.dupnfa(abegin, aend, s, abegin);
            self.repeat(s, abegin, m - 1, if n == DUPINF { n } else { n - 1 });
            let f = combine(qprefer, self.subres[atom].flags);
            let pair = self.sub_re(b'.', f, s, aend);
            if self.iserr() {
                return;
            }
            let pre = self.sub_re(b'=', pref(f), s, abegin);
            self.subres[pair].left = Some(pre);
            if self.iserr() {
                return;
            }
            self.subres[pair].right = Some(atom);
            self.subres[atom_slot].left = Some(pair);
            aend
        } else {
            // The general case needs an iteration node.
            let s2 = self.newstate();
            if self.iserr() {
                return;
            }
            self.nfa.moveouts(aend, s2);
            if self.iserr() {
                return;
            }
            self.nfa.dupnfa(abegin, aend, s, s2);
            self.repeat(s, s2, m, n);
            let f = combine(qprefer, self.subres[atom].flags);
            let iter = self.sub_re(b'*', f, s, s2);
            if self.iserr() {
                return;
            }
            self.subres[iter].min = m;
            self.subres[iter].max = n;
            self.subres[iter].left = Some(atom);
            self.subres[atom_slot].left = Some(iter);
            s2
        };

        // And finally the postponed recursion.
        let t = self.subres[top].right.expect("concatenation");
        let rest = if !(self.see(b'|') || self.see(stopper) || self.see(EOS)) {
            self.parsebranch(stopper, typ, s2, rp, true)
        } else {
            self.emptyarc(s2, rp);
            Some(self.sub_re(b'=', 0, s2, rp))
        };
        self.subres[t].right = rest;
        if self.iserr() {
            return;
        }
        let rest = rest.expect("rest of branch");
        let tflags = self.subres[t].flags;
        self.subres[t].flags |= combine(tflags, self.subres[rest].flags);
        let topflags = self.subres[top].flags;
        self.subres[top].flags |= combine(topflags, self.subres[t].flags);
    }

    /// `nonword`: arcs for a non-word character ahead or behind.
    fn nonword(&mut self, dir: u8, lp: S, rp: S) {
        let anchor = if dir == AHEAD { b'$' } else { b'^' };
        self.newarc(anchor, 1, lp, rp);
        self.newarc(anchor, 0, lp, rp);
        if let Some(wordchrs) = self.wordchrs {
            self.nfa.colorcomplement(&self.cm, dir, wordchrs, lp, rp);
        }
    }

    /// `word`: arcs for a word character ahead or behind.
    fn word(&mut self, dir: u8, lp: S, rp: S) {
        if let Some(wordchrs) = self.wordchrs {
            self.nfa.cloneouts(wordchrs, lp, rp, dir);
        }
    }

    /// `scannum`: a bound's number, at most `DUPMAX`.
    fn scannum(&mut self) -> i32 {
        let mut n = 0;
        while self.see(DIGIT) && n < DUPMAX {
            n = n * 10 + self.nextvalue as i32;
            self.next();
        }
        if self.see(DIGIT) || n > DUPMAX {
            self.err(REG_BADBR);
            return 0;
        }
        n
    }

    /// `repeat`: make the sub-NFA from `lp` to `rp` repeat `m` to `n`
    /// times.
    fn repeat(&mut self, lp: S, rp: S, m: i32, n: i32) {
        const SOME: i32 = 2;
        const INF: i32 = 3;
        let reduce = |x: i32| {
            if x == DUPINF {
                INF
            } else if x > 1 {
                SOME
            } else {
                x
            }
        };
        match (reduce(m), reduce(n)) {
            (0, 0) => {
                // The empty string.
                self.nfa.delsub(lp, rp);
                self.emptyarc(lp, rp);
            }
            (0, 1) => self.emptyarc(lp, rp),
            (0, SOME) => {
                // x{1,n}|
                self.repeat(lp, rp, 1, n);
                if self.iserr() {
                    return;
                }
                self.emptyarc(lp, rp);
            }
            (0, INF) => {
                // Loop x around.
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nfa.moveouts(lp, s);
                self.nfa.moveins(rp, s);
                self.emptyarc(lp, s);
                self.emptyarc(s, rp);
            }
            (1, 1) => {}
            (1, SOME) => {
                // x{0,n-1}x = (x{1,n-1}|)x
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nfa.moveouts(lp, s);
                self.nfa.dupnfa(s, rp, lp, s);
                if self.iserr() {
                    return;
                }
                self.repeat(lp, s, 1, n - 1);
                if self.iserr() {
                    return;
                }
                self.emptyarc(lp, s);
            }
            (1, INF) => {
                // A loopback arc.
                let s = self.newstate();
                let s2 = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nfa.moveouts(lp, s);
                self.nfa.moveins(rp, s2);
                self.emptyarc(lp, s);
                self.emptyarc(s2, rp);
                self.emptyarc(s2, s);
            }
            (SOME, SOME) => {
                // x{m-1,n-1}x
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nfa.moveouts(lp, s);
                self.nfa.dupnfa(s, rp, lp, s);
                if self.iserr() {
                    return;
                }
                self.repeat(lp, s, m - 1, n - 1);
            }
            (SOME, INF) => {
                // x{m-1,}x
                let s = self.newstate();
                if self.iserr() {
                    return;
                }
                self.nfa.moveouts(lp, s);
                self.nfa.dupnfa(s, rp, lp, s);
                if self.iserr() {
                    return;
                }
                self.repeat(lp, s, m - 1, n);
            }
            _ => self.err(REG_ASSERT),
        }
    }

    /// `bracket`: a bracket expression, not complemented.
    fn bracket(&mut self, lp: S, rp: S) {
        self.next();
        while !self.see(b']') && !self.see(EOS) {
            self.brackpart(lp, rp);
        }
        self.cm.okcolors(&mut self.nfa);
    }

    /// `cbracket`: a complemented bracket expression, as the complement of
    /// the arcs `bracket` makes between dummy states.
    fn cbracket(&mut self, lp: S, rp: S) {
        let left = self.newstate();
        let right = self.newstate();
        if self.iserr() {
            return;
        }
        self.bracket(left, right);
        if self.cflags & REG_NLSTOP != 0 {
            self.newarc(PLAIN, self.nlcolor, left, right);
        }
        if self.iserr() {
            return;
        }
        self.nfa.colorcomplement(&self.cm, PLAIN, left, lp, rp);
        if self.iserr() {
            return;
        }
        self.nfa.dropstate(left);
        self.nfa.freestate(right);
    }

    /// The text of a `[.` `[=` or `[:` item: the scan position before
    /// `scanplain`, and where it ended, in the text then being scanned.
    fn scanned(&mut self) -> Vec<Chr> {
        let (text, start) = (self.cur, self.now);
        let end = self.scanplain();
        text[start..end.max(start)].to_vec()
    }

    /// `brackpart`: one item or range of a bracket expression.
    fn brackpart(&mut self, lp: S, rp: S) {
        let startc = match self.nexttype() {
            RANGE => {
                // a-b-c or some other botch.
                self.err(REG_ERANGE);
                return;
            }
            PLAIN => {
                let c = self.nextvalue as Chr;
                self.next();
                // Shortcut for an ordinary character, not a range.
                if !self.see(RANGE) {
                    self.onechr(c, lp, rp);
                    return;
                }
                let startc = self.element(&[c]);
                if self.iserr() {
                    return;
                }
                startc
            }
            COLLEL => {
                let name = self.scanned();
                self.insist(!name.is_empty(), REG_ECOLLATE);
                if self.iserr() {
                    return;
                }
                let startc = self.element(&name);
                if self.iserr() {
                    return;
                }
                startc
            }
            ECLASS => {
                let name = self.scanned();
                self.insist(!name.is_empty(), REG_ECOLLATE);
                if self.iserr() {
                    return;
                }
                let startc = self.element(&name);
                if self.iserr() {
                    return;
                }
                let cv = self.eclass(startc, self.cflags & REG_ICASE != 0);
                if self.iserr() {
                    return;
                }
                self.dovec(&cv, lp, rp);
                return;
            }
            CCLASS => {
                let name = self.scanned();
                self.insist(!name.is_empty(), REG_ECTYPE);
                if self.iserr() {
                    return;
                }
                let cv = self.cclass(&name, self.cflags & REG_ICASE != 0);
                if self.iserr() {
                    return;
                }
                if let Some(cv) = cv {
                    self.dovec(&cv, lp, rp);
                }
                return;
            }
            _ => {
                self.err(REG_ASSERT);
                return;
            }
        };
        let endc = if self.see(RANGE) {
            self.next();
            match self.nexttype() {
                PLAIN | RANGE => {
                    let c = self.nextvalue as Chr;
                    self.next();
                    let endc = self.element(&[c]);
                    if self.iserr() {
                        return;
                    }
                    endc
                }
                COLLEL => {
                    let name = self.scanned();
                    self.insist(!name.is_empty(), REG_ECOLLATE);
                    if self.iserr() {
                        return;
                    }
                    let endc = self.element(&name);
                    if self.iserr() {
                        return;
                    }
                    endc
                }
                _ => {
                    self.err(REG_ERANGE);
                    return;
                }
            }
        } else {
            startc
        };
        // Ranges are unportable.
        if startc != endc {
            self.note(REG_UUNPORT);
        }
        let cv = self.range(startc, endc, self.cflags & REG_ICASE != 0);
        if self.iserr() {
            return;
        }
        if let Some(cv) = cv {
            self.dovec(&cv, lp, rp);
        }
    }

    /// `scanplain`: the PLAIN contents of `[.` and friends; returns the
    /// position just after them.
    fn scanplain(&mut self) -> usize {
        self.next();
        let mut endp = self.now;
        while self.see(PLAIN) {
            endp = self.now;
            self.next();
        }
        self.next();
        endp
    }

    /// `onechr`: arcs for a plain character, and its case counterparts.
    fn onechr(&mut self, c: Chr, lp: S, rp: S) {
        if self.cflags & REG_ICASE == 0 {
            let co = self.cm.subcolor(c);
            self.newarc(PLAIN, co, lp, rp);
            return;
        }
        self.dovec(&locale::allcases(c), lp, rp);
    }

    /// `dovec`: arcs for each member of a character set.
    fn dovec(&mut self, cv: &Cvec, lp: S, rp: S) {
        for &ch in &cv.chrs {
            let co = self.cm.subcolor(ch);
            self.newarc(PLAIN, co, lp, rp);
        }
        for &(from, to) in &cv.ranges {
            if from <= to {
                self.cm.subrange(&mut self.nfa, from, to, lp, rp);
            }
        }
    }

    /// `wordchrs`: set up the word-character arcs for word boundaries, if
    /// not done yet. Advances to the next token.
    fn wordchrs(&mut self) {
        if self.wordchrs.is_some() {
            self.next();
            return;
        }
        let left = self.newstate();
        let right = self.newstate();
        if self.iserr() {
            return;
        }
        self.lexword();
        self.next();
        self.bracket(left, right);
        self.next();
        if self.iserr() {
            return;
        }
        self.wordchrs = Some(left);
    }

    // Collating elements and classes (regc_locale.c).

    /// `element`: the character a collating-element name stands for.
    fn element(&mut self, name: &[Chr]) -> Chr {
        if name.len() == 1 {
            return name[0];
        }
        self.note(REG_ULOCALE);
        match locale::cname(name) {
            Some(c) => c,
            None => {
                self.err(REG_ECOLLATE);
                0
            }
        }
    }

    /// `range`: the set for a range, checked for legality.
    fn range(&mut self, a: Chr, b: Chr, cases: bool) -> Option<Cvec> {
        if a > b {
            self.err(REG_ERANGE);
            return None;
        }
        Some(locale::range(a, b, cases))
    }

    /// `eclass`: an equivalence class, which here is just the character
    /// (and its case counterparts).
    fn eclass(&mut self, c: Chr, cases: bool) -> Cvec {
        if cases {
            return locale::allcases(c);
        }
        Cvec {
            chrs: vec![c],
            ranges: Vec::new(),
        }
    }

    /// `cclass`: a named character class.
    fn cclass(&mut self, name: &[Chr], cases: bool) -> Option<Cvec> {
        let cv = locale::cclass(name, cases);
        if cv.is_none() {
            self.err(REG_ECTYPE);
        }
        cv
    }

    // The subexpression tree.

    /// `sub_re`: a new tree node.
    fn sub_re(&mut self, op: u8, flags: u8, begin: S, end: S) -> usize {
        let node = Subre {
            op,
            flags,
            id: 0,
            subno: 0,
            min: 1,
            max: 1,
            left: None,
            right: None,
            begin,
            end,
            cnfa: Cnfa::default(),
        };
        match self.treefree {
            Some(i) => {
                self.treefree = self.subres[i].left;
                self.subres[i] = node;
                i
            }
            None => {
                self.subres.push(node);
                self.subres.len() - 1
            }
        }
    }

    /// `freesubre`: free a subtree, keeping its nodes for reuse.
    fn freesubre(&mut self, sr: Option<usize>) {
        let Some(sr) = sr else {
            return;
        };
        let (left, right) = (self.subres[sr].left, self.subres[sr].right);
        self.freesubre(left);
        self.freesubre(right);
        let node = &mut self.subres[sr];
        node.cnfa = Cnfa::default();
        node.flags = 0;
        node.right = None;
        node.left = self.treefree;
        self.treefree = Some(sr);
    }

    /// `numst`: number the tree nodes, returning the next number.
    fn numst(&mut self, t: usize, start: usize) -> usize {
        let mut i = start;
        self.subres[t].id = i;
        i += 1;
        if let Some(left) = self.subres[t].left {
            i = self.numst(left, i);
        }
        if let Some(right) = self.subres[t].right {
            i = self.numst(right, i);
        }
        i
    }

    /// `markst`: mark the tree's nodes as in use.
    fn markst(&mut self, t: usize) {
        self.subres[t].flags |= INUSE;
        if let Some(left) = self.subres[t].left {
            self.markst(left);
        }
        if let Some(right) = self.subres[t].right {
            self.markst(right);
        }
    }

    /// `nfatree`: compacted NFAs for a subtree, returning the top node's
    /// `re_info` bits.
    fn nfatree(&mut self, t: usize) -> i64 {
        if let Some(left) = self.subres[t].left {
            self.nfatree(left);
        }
        if let Some(right) = self.subres[t].right {
            self.nfatree(right);
        }
        let (begin, end) = (self.subres[t].begin, self.subres[t].end);
        let (info, cnfa) = self.nfanode(begin, end);
        self.subres[t].cnfa = cnfa;
        info
    }

    /// `nfanode`: the compacted NFA of the part of the main NFA from
    /// `begin` to `end`.
    fn nfanode(&mut self, begin: S, end: S) -> (i64, Cnfa) {
        let parent = (self.nfa.bos, self.nfa.eos);
        let Some(mut nfa) = Nfa::new(self.budget.clone(), &self.cm, Some(parent)) else {
            return (0, Cnfa::default());
        };
        let (init, final_) = (nfa.init, nfa.final_);
        dupnfa_into(&mut self.nfa, &mut nfa, begin, end, init, final_);
        let mut info = 0;
        if !self.iserr() {
            nfa.specialcolors(&mut self.cm);
            info = nfa.optimize();
        }
        let cnfa = if self.iserr() {
            Cnfa::default()
        } else {
            nfa.compact(&self.cm)
        };
        (info, cnfa)
    }

    /// `newlacon`: a lookahead constraint's number.
    fn newlacon(&mut self, begin: S, end: S, pos: usize) -> usize {
        if self.lacons.is_empty() {
            // Number 0 is skipped.
            self.lacons.push(Subre {
                op: b'=',
                flags: 0,
                id: 0,
                subno: 0,
                min: 1,
                max: 1,
                left: None,
                right: None,
                begin,
                end,
                cnfa: Cnfa::default(),
            });
        }
        self.lacons.push(Subre {
            op: b'=',
            flags: 0,
            id: 0,
            subno: pos,
            min: 1,
            max: 1,
            left: None,
            right: None,
            begin,
            end,
            cnfa: Cnfa::default(),
        });
        self.lacons.len() - 1
    }

    /// `makesearch`: turn the optimized main NFA into the search NFA, with
    /// an implicit `.*?` in front.
    fn makesearch(&mut self) {
        let pre = self.nfa.pre;
        let (bos0, bos1) = (self.nfa.bos[0], self.nfa.bos[1]);
        // No loops are needed if it's anchored.
        let anchored = self
            .nfa
            .outs_of(pre)
            .into_iter()
            .all(|a| self.nfa.arcs[a].co == bos0 || self.nfa.arcs[a].co == bos1);
        if !anchored {
            self.nfa.rainbow(&self.cm, PLAIN, COLORLESS, pre, pre);
            // And ^* and \A* too; not always necessary, but harmless.
            self.nfa.newarc(PLAIN, bos0, pre, pre);
            self.nfa.newarc(PLAIN, bos1, pre, pre);
        }
        // States after pre that progress can return to must be split into
        // progress and no-progress states. First list them, marking
        // listed states through tmp (the first one points at itself).
        let mut slist: Option<S> = None;
        for a in self.nfa.outs_of(pre) {
            let s = self.nfa.arcs[a].to;
            let mut other = false;
            let mut b = self.nfa.states[s].ins;
            while let Some(arc) = b {
                if self.nfa.arcs[arc].from != pre {
                    other = true;
                    break;
                }
                b = self.nfa.arcs[arc].inchain;
            }
            if other && self.nfa.states[s].tmp.is_none() {
                self.nfa.states[s].tmp = Some(slist.unwrap_or(s));
                slist = Some(s);
            }
        }
        // Do the splits.
        let mut s = slist;
        while let Some(st) = s {
            let s2 = self.nfa.newstate();
            if self.iserr() {
                return;
            }
            self.nfa.copyouts(st, s2);
            if self.iserr() {
                return;
            }
            let mut a = self.nfa.states[st].ins;
            while let Some(arc) = a {
                let b = self.nfa.arcs[arc].inchain;
                let from = self.nfa.arcs[arc].from;
                if from != pre {
                    self.nfa.cparc(arc, from, s2);
                    self.nfa.freearc(arc);
                }
                a = b;
            }
            let link = self.nfa.states[st].tmp;
            s = if link != Some(st) { link } else { None };
            self.nfa.states[st].tmp = None;
        }
    }
}
