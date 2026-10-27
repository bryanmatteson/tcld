/*
 * re_*exec and friends - match REs
 *
 * Copyright © 1998, 1999 Henry Spencer.  All rights reserved.
 *
 * Development of this software was funded, in part, by Cray Research Inc.,
 * UUNET Communications Services Inc., Sun Microsystems Inc., and Scriptics
 * Corporation, none of whom are responsible for the results.  The author
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
 * AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL
 * HENRY SPENCER BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 * EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 * PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS;
 * OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
 * WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
 * OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF
 * ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

// Ported from Tcl 9.0.2 generic/regexec.c. Modified for Tcld: native Rust
// data structures and VM integration. Tcl licensing terms: license.terms.

//! The matcher (`regexec.c` with the lazy DFAs of `rege_dfa.c`): a search
//! DFA finds where a match can start, the main NFA's DFA finds where it
//! ends, and the subexpression tree is dissected to place the submatches
//! and check back references.

use super::compile::Guts;
use super::locale::equal_nocase;
use super::subflags::SHORTER;
use super::{
    CNFA_NOPROGRESS, COLORLESS, Chr, Cnfa, Color, DUPINF, HASLACONS, REG_ASSERT, REG_NOMATCH,
    REG_NOSUB, REG_NOTBOL, REG_NOTEOL, REG_OKAY, REG_UBACKREF, REG_UIMPOSSIBLE, RegexError,
    Submatch, WHITE,
};

/// State-set flags.
const STARTER: u8 = 0o1;
const POSTSTATE: u8 = 0o2;
const LOCKED: u8 = 0o4;
const NOPROGRESS: u8 = 0o10;

/// Bits per word of a state-set bitvector.
const UBITS: usize = 32;

/// A "pointer" to a state set's outarc: the set and the color.
#[derive(Debug, Clone, Copy)]
struct ArcP {
    ss: Option<usize>,
    co: Color,
}

impl Default for ArcP {
    fn default() -> Self {
        ArcP {
            ss: None,
            co: WHITE,
        }
    }
}

/// A state set (`struct sset`); its bitvector, outarcs and inchain
/// pointers live in the DFA's areas at its index.
#[derive(Debug, Clone, Default)]
struct SSet {
    hash: u32,
    flags: u8,
    ins: ArcP,
    lastseen: Option<usize>,
}

/// A lazy DFA over a compacted NFA (`struct dfa`), caching up to twice as
/// many state sets as the NFA has states.
struct Dfa<'g> {
    nssets: usize,
    nssused: usize,
    nstates: usize,
    wordsper: usize,
    ncolors: usize,
    ssets: Vec<SSet>,
    states: Vec<u32>,
    work: Vec<u32>,
    outs: Vec<Option<usize>>,
    inchain: Vec<ArcP>,
    cnfa: &'g Cnfa,
    lastpost: Option<usize>,
    lastnopr: Option<usize>,
    search: usize,
}

impl<'g> Dfa<'g> {
    /// `newDFA`.
    fn new(cnfa: &'g Cnfa) -> Self {
        let nss = cnfa.nstates * 2;
        let wordsper = cnfa.nstates.div_ceil(UBITS);
        Dfa {
            nssets: nss,
            nssused: 0,
            nstates: cnfa.nstates,
            wordsper,
            ncolors: cnfa.ncolors,
            ssets: vec![SSet::default(); nss],
            states: vec![0; nss * wordsper],
            work: vec![0; wordsper],
            outs: vec![None; nss * cnfa.ncolors],
            inchain: vec![ArcP::default(); nss * cnfa.ncolors],
            cnfa,
            lastpost: None,
            lastnopr: None,
            search: 0,
        }
    }

    fn bitset(&self, ss: usize, st: usize) -> bool {
        self.states[ss * self.wordsper + st / UBITS] & (1 << (st % UBITS)) != 0
    }

    fn out(&self, ss: usize, co: Color) -> Option<usize> {
        self.outs[ss * self.ncolors + co as usize]
    }
}

/// `hash`: a state set's hash, its only word when it has one.
fn hash(words: &[u32]) -> u32 {
    words.iter().fold(0, |h, w| h ^ w)
}

/// The matcher's state (`struct vars`).
struct Vars<'g, 't> {
    g: &'g Guts,
    eflags: i32,
    nmatch: usize,
    pmatch: Vec<Submatch>,
    text: &'t [Chr],
    start: usize,
    stop: usize,
    err: i32,
    subdfas: Vec<Option<Dfa<'g>>>,
}

/// `exec`: match `text`, reporting `nmatch` subexpressions.
pub(super) fn exec(
    g: &Guts,
    text: &[Chr],
    nmatch: usize,
    eflags: i32,
) -> Result<Option<Vec<Submatch>>, RegexError> {
    if g.info & REG_UIMPOSSIBLE != 0 {
        return Ok(None);
    }
    let backref = g.info & REG_UBACKREF != 0;
    let nmatch = if g.cflags & REG_NOSUB != 0 { 0 } else { nmatch };
    // Back references need every subexpression, in a work area.
    let vnmatch = if backref { g.nsub + 1 } else { nmatch };
    let mut v = Vars {
        g,
        eflags,
        nmatch: vnmatch,
        pmatch: vec![None; vnmatch],
        text,
        start: 0,
        stop: text.len(),
        err: 0,
        subdfas: (0..g.ntree).map(|_| None).collect(),
    };
    let cnfa = &g.subres[g.tree].cnfa;
    let st = if backref {
        v.complicated_find(cnfa)
    } else {
        v.simple_find(cnfa)
    };
    match st {
        REG_OKAY => {
            let mut out = vec![None; nmatch];
            let n = nmatch.min(v.nmatch);
            out[..n].copy_from_slice(&v.pmatch[..n]);
            Ok(Some(out))
        }
        REG_NOMATCH => Ok(None),
        e => Err(RegexError(e)),
    }
}

impl<'g> Vars<'g, '_> {
    fn set_err(&mut self, e: i32) {
        if self.err == 0 {
            self.err = e;
        }
    }

    fn color_at(&self, cp: usize) -> Color {
        self.g.cmap.getcolor(self.text[cp])
    }

    /// The color before position `cp`: BOS (or BOL) at the start.
    fn color_before(&self, d: &Dfa, cp: usize) -> Color {
        if cp == self.start {
            d.cnfa.bos[usize::from(self.eflags & REG_NOTBOL == 0)]
        } else {
            self.color_at(cp - 1)
        }
    }

    fn eos_color(&self, d: &Dfa) -> Color {
        d.cnfa.eos[usize::from(self.eflags & REG_NOTEOL == 0)]
    }

    /// `simpleFind`: a match without back references.
    fn simple_find(&mut self, cnfa: &'g Cnfa) -> i32 {
        let g = self.g;
        let shorter = g.subres[g.tree].flags & SHORTER != 0;
        // First, a shot with the search RE.
        let mut s = Dfa::new(&g.search);
        let mut cold = None;
        let close = self.shortest(
            &mut s,
            self.start,
            self.start,
            self.stop,
            Some(&mut cold),
            None,
        );
        if self.err != 0 {
            return self.err;
        }
        let Some(close) = close else {
            return REG_NOMATCH;
        };
        if self.nmatch == 0 {
            // Found, and the location does not matter.
            return REG_OKAY;
        }
        // Find the starting point and match.
        let Some(open) = cold else {
            return REG_ASSERT;
        };
        let mut d = Dfa::new(cnfa);
        let mut begin = open;
        let mut end = None;
        while begin <= close {
            let mut hitend = false;
            end = if shorter {
                self.shortest(&mut d, begin, begin, self.stop, None, Some(&mut hitend))
            } else {
                self.longest(&mut d, begin, self.stop, Some(&mut hitend))
            };
            if self.err != 0 {
                return self.err;
            }
            if end.is_some() {
                break;
            }
            begin += 1;
        }
        let Some(end) = end else {
            // The search RE matched, so the loop should have.
            return REG_ASSERT;
        };
        // And pin down the details.
        self.pmatch[0] = Some((begin, end));
        if self.nmatch == 1 {
            return REG_OKAY;
        }
        self.zapallsubs();
        self.cdissect(g.tree, begin, end)
    }

    /// `complicatedFind`: a match with back references.
    fn complicated_find(&mut self, cnfa: &'g Cnfa) -> i32 {
        let g = self.g;
        let mut s = Dfa::new(&g.search);
        let mut d = Dfa::new(cnfa);
        let ret = self.complicated_find_loop(&mut d, &mut s);
        if self.err != 0 {
            return self.err;
        }
        ret
    }

    /// `complicatedFindLoop`: try each possible start and end until the
    /// dissection, which checks the back references, succeeds.
    fn complicated_find_loop(&mut self, d: &mut Dfa<'g>, s: &mut Dfa<'g>) -> i32 {
        let g = self.g;
        let shorter = g.subres[g.tree].flags & SHORTER != 0;
        let mut close = self.start;
        loop {
            let mut cold = None;
            let Some(found) = self.shortest(s, close, close, self.stop, Some(&mut cold), None)
            else {
                break;
            };
            close = found;
            let Some(open) = cold else {
                return REG_ASSERT;
            };
            let mut begin = open;
            while begin <= close {
                let mut estart = begin;
                let mut estop = self.stop;
                loop {
                    let end = if shorter {
                        self.shortest(d, begin, estart, estop, None, None)
                    } else {
                        self.longest(d, begin, estop, None)
                    };
                    let Some(end) = end else {
                        break;
                    };
                    self.zapallsubs();
                    let er = self.cdissect(g.tree, begin, end);
                    if er == REG_OKAY {
                        if self.nmatch > 0 {
                            self.pmatch[0] = Some((begin, end));
                        }
                        return REG_OKAY;
                    }
                    if er != REG_NOMATCH {
                        self.set_err(er);
                        return er;
                    }
                    if (shorter && end == estop) || (!shorter && end == begin) {
                        break;
                    }
                    // Go around and try again.
                    if shorter {
                        estart = end + 1;
                    } else {
                        estop = end - 1;
                    }
                }
                begin += 1;
            }
            if close >= self.stop {
                break;
            }
        }
        REG_NOMATCH
    }

    /// `zapallsubs`: mark every subexpression unmatched.
    fn zapallsubs(&mut self) {
        for m in self.pmatch.iter_mut().skip(1) {
            *m = None;
        }
    }

    /// `zaptreesubs`: mark a subtree's subexpressions unmatched.
    fn zaptreesubs(&mut self, t: usize) {
        let g = self.g;
        let node = &g.subres[t];
        if node.op == b'(' && node.subno < self.nmatch {
            self.pmatch[node.subno] = None;
        }
        if let Some(left) = node.left {
            self.zaptreesubs(left);
        }
        if let Some(right) = node.right {
            self.zaptreesubs(right);
        }
    }

    /// `subset`: record a capture's match.
    fn subset(&mut self, t: usize, begin: usize, end: usize) {
        let n = self.g.subres[t].subno;
        if n < self.nmatch {
            self.pmatch[n] = Some((begin, end));
        }
    }

    /// The DFA of a tree node, made on first use (`getsubdfa`), taken out
    /// while it runs.
    fn take_subdfa(&mut self, t: usize) -> Dfa<'g> {
        let g = self.g;
        let node = &g.subres[t];
        self.subdfas[node.id]
            .take()
            .unwrap_or_else(|| Dfa::new(&node.cnfa))
    }

    fn put_subdfa(&mut self, t: usize, d: Dfa<'g>) {
        let id = self.g.subres[t].id;
        self.subdfas[id] = Some(d);
    }

    fn sub_longest(&mut self, t: usize, start: usize, stop: usize) -> Option<usize> {
        let mut d = self.take_subdfa(t);
        let end = self.longest(&mut d, start, stop, None);
        self.put_subdfa(t, d);
        end
    }

    fn sub_shortest(&mut self, t: usize, start: usize, min: usize, max: usize) -> Option<usize> {
        let mut d = self.take_subdfa(t);
        let end = self.shortest(&mut d, start, min, max, None, None);
        self.put_subdfa(t, d);
        end
    }

    /// `cdissect`: check back references and place the submatches of a
    /// match from `begin` to `end` that the node's DFA has accepted.
    fn cdissect(&mut self, t: usize, begin: usize, end: usize) -> i32 {
        let g = self.g;
        let node = &g.subres[t];
        match node.op {
            // A terminal node; the parent did the work.
            b'=' => REG_OKAY,
            b'b' => self.cbrdissect(t, begin, end),
            b'.' => {
                let left = node.left.expect("concatenation");
                if g.subres[left].flags & SHORTER != 0 {
                    self.crevcondissect(t, begin, end)
                } else {
                    self.ccondissect(t, begin, end)
                }
            }
            b'|' => self.caltdissect(t, begin, end),
            b'*' => {
                let left = node.left.expect("iteration");
                if g.subres[left].flags & SHORTER != 0 {
                    self.creviterdissect(t, begin, end)
                } else {
                    self.citerdissect(t, begin, end)
                }
            }
            b'(' => {
                let left = node.left.expect("capture");
                let er = self.cdissect(left, begin, end);
                if er == REG_OKAY {
                    self.subset(t, begin, end);
                }
                er
            }
            _ => REG_ASSERT,
        }
    }

    /// `ccondissect`: a concatenation, longest left part first.
    fn ccondissect(&mut self, t: usize, begin: usize, end: usize) -> i32 {
        let g = self.g;
        let node = &g.subres[t];
        let (left, right) = (
            node.left.expect("concatenation"),
            node.right.expect("concatenation"),
        );
        // A tentative midpoint.
        let Some(mut mid) = self.sub_longest(left, begin, end) else {
            return REG_NOMATCH;
        };
        loop {
            // Try this midpoint on for size.
            if self.sub_longest(right, mid, end) == Some(end) {
                let mut er = self.cdissect(left, begin, mid);
                if er == REG_OKAY {
                    er = self.cdissect(right, mid, end);
                    if er == REG_OKAY {
                        return REG_OKAY;
                    }
                }
                if er != REG_NOMATCH {
                    return er;
                }
            }
            // That midpoint didn't work; find a new one.
            if mid == begin {
                return REG_NOMATCH;
            }
            match self.sub_longest(left, begin, mid - 1) {
                Some(next) => mid = next,
                None => return REG_NOMATCH,
            }
            self.zaptreesubs(left);
            self.zaptreesubs(right);
        }
    }

    /// `crevcondissect`: a concatenation, shortest left part first.
    fn crevcondissect(&mut self, t: usize, begin: usize, end: usize) -> i32 {
        let g = self.g;
        let node = &g.subres[t];
        let (left, right) = (
            node.left.expect("concatenation"),
            node.right.expect("concatenation"),
        );
        let Some(mut mid) = self.sub_shortest(left, begin, begin, end) else {
            return REG_NOMATCH;
        };
        loop {
            if self.sub_longest(right, mid, end) == Some(end) {
                let mut er = self.cdissect(left, begin, mid);
                if er == REG_OKAY {
                    er = self.cdissect(right, mid, end);
                    if er == REG_OKAY {
                        return REG_OKAY;
                    }
                }
                if er != REG_NOMATCH {
                    return er;
                }
            }
            if mid == end {
                return REG_NOMATCH;
            }
            match self.sub_shortest(left, begin, mid + 1, end) {
                Some(next) => mid = next,
                None => return REG_NOMATCH,
            }
            self.zaptreesubs(left);
            self.zaptreesubs(right);
        }
    }

    /// `cbrdissect`: a back reference, possibly repeated.
    fn cbrdissect(&mut self, t: usize, begin: usize, end: usize) -> i32 {
        let g = self.g;
        let node = &g.subres[t];
        let (n, min, max) = (node.subno, node.min, node.max);
        let Some(Some((so, eo))) = self.pmatch.get(n).copied() else {
            return REG_NOMATCH;
        };
        let brlen = eo - so;
        // Zero-length strings.
        if brlen == 0 {
            return if begin == end && min <= max {
                REG_OKAY
            } else {
                REG_NOMATCH
            };
        }
        if begin == end {
            return if min == 0 { REG_OKAY } else { REG_NOMATCH };
        }
        // Could the target be an allowed number of repetitions?
        let tlen = end - begin;
        if !tlen.is_multiple_of(brlen) {
            return REG_NOMATCH;
        }
        let numreps = tlen / brlen;
        if numreps < min as usize || (numreps > max as usize && max != DUPINF) {
            return REG_NOMATCH;
        }
        let referenced = &self.text[so..eo];
        for rep in self.text[begin..end].chunks(brlen) {
            let equal = if g.icase {
                equal_nocase(referenced, rep)
            } else {
                referenced == rep
            };
            if !equal {
                return REG_NOMATCH;
            }
        }
        REG_OKAY
    }

    /// `caltdissect`: an alternation, a chain of `|` nodes.
    fn caltdissect(&mut self, t: usize, begin: usize, end: usize) -> i32 {
        let mut t = Some(t);
        while let Some(node) = t {
            let left = self.g.subres[node].left.expect("alternative");
            if self.sub_longest(left, begin, end) == Some(end) {
                let er = self.cdissect(left, begin, end);
                if er != REG_NOMATCH {
                    return er;
                }
            }
            t = self.g.subres[node].right;
        }
        REG_NOMATCH
    }

    /// `citerdissect`: an iteration, longest sub-matches first.
    fn citerdissect(&mut self, t: usize, begin: usize, end: usize) -> i32 {
        let g = self.g;
        let node = &g.subres[t];
        let left = node.left.expect("iteration");
        // Zero matches work only for an empty target; otherwise at least
        // one is needed.
        let mut min_matches = node.min;
        if min_matches <= 0 {
            if begin == end {
                return REG_OKAY;
            }
            min_matches = 1;
        }
        let mut max_matches = end - begin;
        if max_matches > node.max as usize && node.max != DUPINF {
            max_matches = node.max as usize;
        }
        if max_matches < min_matches as usize {
            max_matches = min_matches as usize;
        }
        let min = min_matches as i64;
        // endpts[0] is begin; the sub-match endpoints follow.
        let mut endpts = vec![begin; max_matches + 1];
        let mut nverified = 0;
        let mut k = 1;
        let mut limit = end;
        'iterate: while k > 0 {
            // Find an endpoint for the k'th sub-match.
            'attempt: {
                let Some(ep) = self.sub_longest(left, endpts[k - 1], limit) else {
                    // No match: see if the previous one can be shorter.
                    k -= 1;
                    break 'attempt;
                };
                endpts[k] = ep;
                if nverified >= k {
                    nverified = k - 1;
                }
                if ep != end {
                    // Not at the end yet: another iteration, if allowed.
                    if k >= max_matches {
                        k -= 1;
                        break 'attempt;
                    }
                    // Reject a zero-length match unless needed for min.
                    if ep == endpts[k - 1]
                        && (k as i64 >= min || min - (k as i64) < (end - ep) as i64)
                    {
                        break 'attempt;
                    }
                    k += 1;
                    limit = end;
                    continue 'iterate;
                }
                // k sub-matches the DFA accepts: verify each, if k is an
                // allowed number of them.
                if (k as i64) < min {
                    break 'attempt;
                }
                let mut i = nverified + 1;
                while i <= k {
                    self.zaptreesubs(left);
                    let er = self.cdissect(left, endpts[i - 1], endpts[i]);
                    if er == REG_OKAY {
                        nverified = i;
                        i += 1;
                        continue;
                    }
                    if er == REG_NOMATCH {
                        break;
                    }
                    return er;
                }
                if i > k {
                    return REG_OKAY;
                }
            }
            // Backtrack: consider shorter versions of the current
            // sub-match, asking for a zero-length one only if necessary.
            while k > 0 {
                let prev_end = endpts[k - 1];
                if endpts[k] > prev_end {
                    limit = endpts[k] - 1;
                    if limit > prev_end
                        || ((k as i64) < min && min - (k as i64) >= (end - prev_end) as i64)
                    {
                        break;
                    }
                }
                k -= 1;
            }
        }
        REG_NOMATCH
    }

    /// `creviterdissect`: an iteration, shortest sub-matches first.
    fn creviterdissect(&mut self, t: usize, begin: usize, end: usize) -> i32 {
        let g = self.g;
        let node = &g.subres[t];
        let left = node.left.expect("iteration");
        let mut min_matches = node.min;
        if min_matches <= 0 {
            if begin == end {
                return REG_OKAY;
            }
            min_matches = 1;
        }
        let mut max_matches = end - begin;
        if max_matches > node.max as usize && node.max != DUPINF {
            max_matches = node.max as usize;
        }
        if max_matches < min_matches as usize {
            max_matches = min_matches as usize;
        }
        let min = min_matches as i64;
        let mut endpts = vec![begin; max_matches + 1];
        let mut nverified = 0;
        let mut k = 1;
        let mut limit = begin;
        'iterate: while k > 0 {
            'attempt: {
                // Disallow a zero-length match unless needed for min.
                if limit == endpts[k - 1]
                    && limit != end
                    && (k as i64 >= min || min - (k as i64) < (end - limit) as i64)
                {
                    limit += 1;
                }
                // The last allowed sub-match must reach the end.
                if k >= max_matches {
                    limit = end;
                }
                let Some(ep) = self.sub_shortest(left, endpts[k - 1], limit, end) else {
                    // No match: see if the previous one can be longer.
                    k -= 1;
                    break 'attempt;
                };
                endpts[k] = ep;
                if nverified >= k {
                    nverified = k - 1;
                }
                if ep != end {
                    if k >= max_matches {
                        k -= 1;
                        break 'attempt;
                    }
                    k += 1;
                    limit = endpts[k - 1];
                    continue 'iterate;
                }
                if (k as i64) < min {
                    break 'attempt;
                }
                let mut i = nverified + 1;
                while i <= k {
                    self.zaptreesubs(left);
                    let er = self.cdissect(left, endpts[i - 1], endpts[i]);
                    if er == REG_OKAY {
                        nverified = i;
                        i += 1;
                        continue;
                    }
                    if er == REG_NOMATCH {
                        break;
                    }
                    return er;
                }
                if i > k {
                    return REG_OKAY;
                }
            }
            // Backtrack: consider longer versions of the current sub-match.
            while k > 0 {
                if endpts[k] < end {
                    limit = endpts[k] + 1;
                    break;
                }
                k -= 1;
            }
        }
        REG_NOMATCH
    }

    // The lazy DFAs (rege_dfa.c).

    /// `longest`: the longest match from `start` ending at or before
    /// `stop`.
    fn longest(
        &mut self,
        d: &mut Dfa<'g>,
        start: usize,
        stop: usize,
        hitstopp: Option<&mut bool>,
    ) -> Option<usize> {
        let realstop = if stop == self.stop { stop } else { stop + 1 };
        let mut css = self.initialize(d, start);
        let mut cp = start;
        let mut hitstop = false;
        let found = 'run: {
            // Startup.
            let co = self.color_before(d, cp);
            let Some(first) = self.miss(d, css, co, cp, start) else {
                break 'run None;
            };
            css = first;
            d.ssets[css].lastseen = Some(cp);
            // The main loop.
            while cp < realstop {
                let co = self.color_at(cp);
                let ss = match d.out(css, co) {
                    Some(ss) => ss,
                    None => match self.miss(d, css, co, cp + 1, start) {
                        Some(ss) => ss,
                        None => break,
                    },
                };
                cp += 1;
                d.ssets[ss].lastseen = Some(cp);
                css = ss;
            }
            // Shutdown.
            if cp == self.stop && stop == self.stop {
                hitstop = true;
                let co = self.eos_color(d);
                if let Some(ss) = self.miss(d, css, co, cp, start) {
                    // A match ending at the end?
                    if d.ssets[ss].flags & POSTSTATE != 0 {
                        break 'run Some(cp);
                    }
                    d.ssets[ss].lastseen = Some(cp);
                }
            }
            // The last match, if any.
            let mut post = d.lastpost;
            for ss in &d.ssets[..d.nssused] {
                if ss.flags & POSTSTATE != 0
                    && post != ss.lastseen
                    && (post.is_none() || post < ss.lastseen)
                {
                    post = ss.lastseen;
                }
            }
            post.and_then(|post| post.checked_sub(1))
        };
        if let Some(h) = hitstopp {
            *h = hitstop;
        }
        found
    }

    /// `shortest`: the shortest match from `start` ending between `min`
    /// and `max`; `coldp` gets the last point no progress had been made.
    fn shortest(
        &mut self,
        d: &mut Dfa<'g>,
        start: usize,
        min: usize,
        max: usize,
        coldp: Option<&mut Option<usize>>,
        hitstopp: Option<&mut bool>,
    ) -> Option<usize> {
        let realmin = if min == self.stop { min } else { min + 1 };
        let realmax = if max == self.stop { max } else { max + 1 };
        let mut hitstop = false;
        let found = 'run: {
            let mut css = self.initialize(d, start);
            let mut cp = start;
            // Startup.
            let co = self.color_before(d, cp);
            let Some(first) = self.miss(d, css, co, cp, start) else {
                break 'run None;
            };
            css = first;
            d.ssets[css].lastseen = Some(cp);
            let mut ss = Some(css);
            // The main loop.
            while cp < realmax {
                let co = self.color_at(cp);
                ss = match d.out(css, co) {
                    Some(next) => Some(next),
                    None => self.miss(d, css, co, cp + 1, start),
                };
                let Some(next) = ss else {
                    break;
                };
                cp += 1;
                d.ssets[next].lastseen = Some(cp);
                css = next;
                if d.ssets[next].flags & POSTSTATE != 0 && cp >= realmin {
                    break;
                }
            }
            let Some(mut ss) = ss else {
                break 'run None;
            };
            if let Some(coldp) = coldp {
                // The last no-progress state set, if any.
                *coldp = Some(self.last_cold(d));
            }
            if d.ssets[ss].flags & POSTSTATE != 0 && cp > min {
                cp -= 1;
            } else if cp == self.stop && max == self.stop {
                let co = self.eos_color(d);
                let at_end = self.miss(d, css, co, cp, start);
                // The match might have ended at the end.
                if at_end.is_none_or(|end| d.ssets[end].flags & POSTSTATE == 0) {
                    hitstop = true;
                }
                match at_end {
                    Some(end) => ss = end,
                    None => break 'run None,
                }
            }
            if d.ssets[ss].flags & POSTSTATE == 0 {
                break 'run None;
            }
            Some(cp)
        };
        if let Some(h) = hitstopp {
            *h = hitstop;
        }
        found
    }

    /// `lastCold`: the last point at which no progress had been made.
    fn last_cold(&self, d: &Dfa) -> usize {
        let mut nopr = d.lastnopr.unwrap_or(self.start);
        for ss in &d.ssets[..d.nssused] {
            if ss.flags & NOPROGRESS != 0
                && let Some(seen) = ss.lastseen
                && nopr < seen
            {
                nopr = seen;
            }
        }
        nopr
    }

    /// `initialize`: the starting state set, made if necessary.
    fn initialize(&mut self, d: &mut Dfa<'g>, start: usize) -> usize {
        let ss = if d.nssused > 0 && d.ssets[0].flags & STARTER != 0 {
            0
        } else {
            let ss = self.get_vacant_ss(d, start, start);
            let wp = d.wordsper;
            let words = &mut d.states[ss * wp..(ss + 1) * wp];
            words.fill(0);
            let pre = d.cnfa.pre;
            words[pre / UBITS] |= 1 << (pre % UBITS);
            d.ssets[ss].hash = hash(words);
            d.ssets[ss].flags = STARTER | LOCKED | NOPROGRESS;
            ss
        };
        for set in &mut d.ssets[..d.nssused] {
            set.lastseen = None;
        }
        // Maybe untrue, but harmless.
        d.ssets[ss].lastseen = Some(start);
        d.lastpost = None;
        d.lastnopr = None;
        ss
    }

    /// `miss`: the state set `css` goes to on color `co`, made and cached
    /// on a cache miss; `None` for the empty set.
    fn miss(
        &mut self,
        d: &mut Dfa<'g>,
        css: usize,
        co: Color,
        cp: usize,
        start: usize,
    ) -> Option<usize> {
        if let Some(ss) = d.out(css, co) {
            return Some(ss);
        }
        let cnfa = d.cnfa;
        let wp = d.wordsper;
        // First, the set of states it leads to.
        d.work.fill(0);
        let mut is_post = false;
        let mut no_progress = true;
        let mut got_state = false;
        for i in 0..d.nstates {
            if !d.bitset(css, i) {
                continue;
            }
            let mut ca = cnfa.states[i];
            while cnfa.arcs[ca].co != COLORLESS {
                let arc = cnfa.arcs[ca];
                if arc.co == co {
                    d.work[arc.to / UBITS] |= 1 << (arc.to % UBITS);
                    got_state = true;
                    if arc.to == cnfa.post {
                        is_post = true;
                    }
                    if cnfa.stflags[arc.to] & CNFA_NOPROGRESS == 0 {
                        no_progress = false;
                    }
                }
                ca += 1;
            }
        }
        let mut do_lacons = got_state && cnfa.flags & HASLACONS != 0;
        let mut saw_lacons = false;
        while do_lacons {
            // The transitive closure over satisfied lookahead constraints.
            do_lacons = false;
            for i in 0..d.nstates {
                if d.work[i / UBITS] & (1 << (i % UBITS)) == 0 {
                    continue;
                }
                let mut ca = cnfa.states[i];
                while cnfa.arcs[ca].co != COLORLESS {
                    let arc = cnfa.arcs[ca];
                    ca += 1;
                    if arc.co < cnfa.ncolors as Color {
                        continue;
                    }
                    saw_lacons = true;
                    if d.work[arc.to / UBITS] & (1 << (arc.to % UBITS)) != 0 {
                        continue;
                    }
                    if !self.check_lacon(cnfa, cp, arc.co) {
                        continue;
                    }
                    d.work[arc.to / UBITS] |= 1 << (arc.to % UBITS);
                    do_lacons = true;
                    if arc.to == cnfa.post {
                        is_post = true;
                    }
                    if cnfa.stflags[arc.to] & CNFA_NOPROGRESS == 0 {
                        no_progress = false;
                    }
                }
            }
        }
        if !got_state {
            return None;
        }
        let h = hash(&d.work);
        // Next, is it in the cache?
        let cached = (0..d.nssused).find(|&p| {
            d.ssets[p].hash == h && (wp == 1 || d.states[p * wp..(p + 1) * wp] == d.work[..])
        });
        let p = match cached {
            Some(p) => p,
            None => {
                let p = self.get_vacant_ss(d, cp, start);
                let work = std::mem::take(&mut d.work);
                d.states[p * wp..(p + 1) * wp].copy_from_slice(&work);
                d.work = work;
                d.ssets[p].hash = h;
                d.ssets[p].flags = if is_post { POSTSTATE } else { 0 };
                if no_progress {
                    d.ssets[p].flags |= NOPROGRESS;
                }
                p
            }
        };
        // Lookahead conditions always miss the cache.
        if !saw_lacons {
            let slot = css * d.ncolors + co as usize;
            d.outs[slot] = Some(p);
            d.inchain[slot] = d.ssets[p].ins;
            d.ssets[p].ins = ArcP { ss: Some(css), co };
        }
        Some(p)
    }

    /// `checkLAConstraint`: whether lookahead constraint `co` holds at
    /// `cp`.
    fn check_lacon(&mut self, pcnfa: &Cnfa, cp: usize, co: Color) -> bool {
        let g = self.g;
        let n = co as usize - pcnfa.ncolors;
        let sub = &g.lacons[n];
        let mut d = Dfa::new(&sub.cnfa);
        let end = self.longest(&mut d, cp, self.stop, None);
        if sub.subno != 0 {
            end.is_some()
        } else {
            end.is_none()
        }
    }

    /// `getVacantSS`: a free state set, unlinked from the cache.
    fn get_vacant_ss(&mut self, d: &mut Dfa<'g>, cp: usize, start: usize) -> usize {
        let ss = self.pick_next_ss(d, cp, start);
        let nc = d.ncolors;
        // Clear out its inarcs, including self-referential ones.
        let mut ap = d.ssets[ss].ins;
        while let Some(p) = ap.ss {
            let slot = p * nc + ap.co as usize;
            d.outs[slot] = None;
            ap = d.inchain[slot];
            d.inchain[slot].ss = None;
        }
        d.ssets[ss].ins.ss = None;
        // Take it off the inarc chains of the sets its outarcs reach.
        for i in 0..nc {
            let slot = ss * nc + i;
            let Some(p) = d.outs[slot] else {
                continue;
            };
            if d.ssets[p].ins.ss == Some(ss) && d.ssets[p].ins.co == i as Color {
                d.ssets[p].ins = d.inchain[slot];
            } else {
                let mut ap = d.ssets[p].ins;
                let mut lastap = ArcP::default();
                while let Some(aps) = ap.ss {
                    if aps == ss && ap.co == i as Color {
                        break;
                    }
                    lastap = ap;
                    ap = d.inchain[aps * nc + ap.co as usize];
                }
                if let Some(last) = lastap.ss {
                    d.inchain[last * nc + lastap.co as usize] = d.inchain[slot];
                }
            }
            d.outs[slot] = None;
            d.inchain[slot].ss = None;
        }
        // A success state's or no-progress state's location is kept.
        let set = &d.ssets[ss];
        if set.flags & POSTSTATE != 0
            && set.lastseen != d.lastpost
            && (d.lastpost.is_none() || d.lastpost < set.lastseen)
        {
            d.lastpost = set.lastseen;
        }
        if set.flags & NOPROGRESS != 0
            && set.lastseen != d.lastnopr
            && (d.lastnopr.is_none() || d.lastnopr < set.lastseen)
        {
            d.lastnopr = set.lastseen;
        }
        ss
    }

    /// `pickNextSS`: the state set to use next.
    fn pick_next_ss(&mut self, d: &mut Dfa<'g>, cp: usize, start: usize) -> usize {
        // Shortcut while the cache isn't full.
        if d.nssused < d.nssets {
            let j = d.nssused;
            d.nssused += 1;
            d.ssets[j] = SSet::default();
            let nc = d.ncolors;
            d.outs[j * nc..(j + 1) * nc].fill(None);
            for chain in &mut d.inchain[j * nc..(j + 1) * nc] {
                chain.ss = None;
            }
            return j;
        }
        // The oldest, or old enough anyway.
        let ancient = if cp - start > d.nssets * 2 / 3 {
            cp - d.nssets * 2 / 3
        } else {
            start
        };
        let expendable =
            |set: &SSet| set.lastseen.is_none_or(|seen| seen < ancient) && set.flags & LOCKED == 0;
        let found = (d.search..d.nssets)
            .chain(0..d.search)
            .find(|&ss| expendable(&d.ssets[ss]));
        match found {
            Some(ss) => {
                d.search = ss + 1;
                ss
            }
            None => {
                // Nobody's old enough?!? Something's really wrong.
                self.set_err(REG_ASSERT);
                0
            }
        }
    }
}
