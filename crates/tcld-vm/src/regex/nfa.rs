/*
 * NFA utilities.
 * This file is #included by regcomp.c.
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
 * One or two things that technically ought to be in here are actually in
 * color.c, thanks to some incestuous relationships in the color chains.
 */

// Ported from Tcl 9.0.2 generic/regc_nfa.c. Modified for Tcld: native Rust
// data structures and VM integration. Tcl licensing terms: license.terms.

//! NFAs (`regc_nfa.c`): states and arcs in arenas, linked into the same
//! chains the C code keeps, so every algorithm walks them in Tcl's order.
//!
//! The deep traversals (`deltraverse`, `duptraverse`, `markreachable`, ...)
//! keep explicit stacks rather than recursing, so long chains of states do
//! not exhaust a thread's stack; each visits states in the C order.

use std::rc::Rc;

use super::color::{ColorMap, PSEUDO};
use super::token::{AHEAD, BEHIND, EMPTY, LACON, PLAIN};
use super::{
    Budget, CNFA_NOPROGRESS, COLORLESS, Carc, Cnfa, Color, HASLACONS, REG_ASSERT, REG_ESPACE,
    REG_ETOOBIG, REG_UEMPTYMATCH, REG_UIMPOSSIBLE,
};

/// A state's index in its NFA's arena.
pub(super) type S = usize;
/// An arc's index in its NFA's arena.
pub(super) type A = usize;

/// The number of a freed state.
pub(super) const FREESTATE: usize = usize::MAX;
/// What `newstate` returns when the NFA has grown too complex.
pub(super) const NOSTATE: S = usize::MAX;

/// `sizeof(struct state)` and `sizeof(struct arcbatch)` on a 64-bit build,
/// which `REG_MAX_COMPILE_SPACE` counts in.
const STATE_SIZE: usize = 816;
const ARCBATCH_SIZE: usize = 728;
const ABSIZE: usize = 10;
const REG_MAX_COMPILE_SPACE: usize = 100_000 * STATE_SIZE + 100_000 * ARCBATCH_SIZE;
const DUPTRAVERSE_MAX_DEPTH: usize = 15000;

/// What a constraint does to an arc it lands on (`combine`).
const INCOMPATIBLE: u8 = 1;
const SATISFIED: u8 = 2;
const COMPATIBLE: u8 = 3;

#[derive(Debug, Clone)]
pub(super) struct State {
    pub no: usize,
    pub flag: u8,
    pub nins: usize,
    pub ins: Option<A>,
    pub nouts: usize,
    pub outs: Option<A>,
    pub tmp: Option<S>,
    pub next: Option<S>,
    pub prev: Option<S>,
    /// Arc storage accounting: arcs used from the inline batch, free arcs,
    /// and extra batches allocated (`noas`, `free`, `oas.next`).
    noas: usize,
    nfree: usize,
    nbatches: usize,
}

#[derive(Debug, Clone)]
pub(super) struct Arc {
    /// The arc type, 0 when the arc is free.
    pub typ: u8,
    pub co: Color,
    pub from: S,
    pub to: S,
    pub outchain: Option<A>,
    pub outchain_rev: Option<A>,
    pub inchain: Option<A>,
    pub inchain_rev: Option<A>,
    pub colorchain: Option<A>,
    pub colorchain_rev: Option<A>,
}

/// Whether an arc is colored, and hence on a color chain (`COLORED`).
fn colored(typ: u8) -> bool {
    typ == PLAIN || typ == AHEAD || typ == BEHIND
}

/// `isconstraintarc`.
fn is_constraint(typ: u8) -> bool {
    matches!(typ, b'^' | b'$' | BEHIND | AHEAD | LACON)
}

pub(crate) struct Nfa {
    pub states: Vec<State>,
    pub arcs: Vec<Arc>,
    free_arc_slots: Vec<A>,
    /// The state chain (`states` and `slast` in C).
    pub first: Option<S>,
    pub slast: Option<S>,
    /// Freed states, kept for reuse.
    free: Option<S>,
    /// The number the next state gets.
    pub nstates: usize,
    pub pre: S,
    pub init: S,
    pub final_: S,
    pub post: S,
    pub bos: [Color; 2],
    pub eos: [Color; 2],
    /// The parent NFA's BOS and EOS colors, for an NFA of a subexpression.
    parent: Option<([Color; 2], [Color; 2])>,
    /// The heads of the color chains, by color; only the main NFA keeps
    /// them.
    colorchains: Vec<Option<A>>,
    budget: Rc<Budget>,
}

impl Drop for Nfa {
    /// `freenfa`, as far as the compile-space budget sees it.
    fn drop(&mut self) {
        let used: usize = self
            .states
            .iter()
            .map(|state| STATE_SIZE + ARCBATCH_SIZE * state.nbatches)
            .sum();
        let space = self.budget.space.get();
        self.budget.space.set(space.saturating_sub(used));
    }
}

impl Nfa {
    /// `newnfa`; `parent` gives a subexpression NFA its parent's special
    /// colors.
    pub(super) fn new(
        budget: Rc<Budget>,
        cm: &ColorMap,
        parent: Option<([Color; 2], [Color; 2])>,
    ) -> Option<Nfa> {
        let mut nfa = Nfa {
            states: Vec::new(),
            arcs: Vec::new(),
            free_arc_slots: Vec::new(),
            first: None,
            slast: None,
            free: None,
            nstates: 0,
            pre: NOSTATE,
            init: NOSTATE,
            final_: NOSTATE,
            post: NOSTATE,
            bos: [COLORLESS; 2],
            eos: [COLORLESS; 2],
            parent,
            colorchains: Vec::new(),
            budget,
        };
        nfa.post = nfa.newfstate(b'@');
        nfa.pre = nfa.newfstate(b'>');
        nfa.init = nfa.newstate();
        nfa.final_ = nfa.newstate();
        if nfa.err() {
            return None;
        }
        let (pre, init, final_, post) = (nfa.pre, nfa.init, nfa.final_, nfa.post);
        nfa.rainbow(cm, PLAIN, COLORLESS, pre, init);
        nfa.newarc(b'^', 1, pre, init);
        nfa.newarc(b'^', 0, pre, init);
        nfa.rainbow(cm, PLAIN, COLORLESS, final_, post);
        nfa.newarc(b'$', 1, final_, post);
        nfa.newarc(b'$', 0, final_, post);
        if nfa.err() {
            return None;
        }
        Some(nfa)
    }

    fn err(&self) -> bool {
        self.budget.is_err()
    }

    fn set_err(&self, e: i32) {
        self.budget.set_err(e);
    }

    /// The states in chain order, as the chain stands now.
    fn chain(&self) -> Vec<S> {
        let mut out = Vec::new();
        let mut s = self.first;
        while let Some(st) = s {
            out.push(st);
            s = self.states[st].next;
        }
        out
    }

    /// The arcs out of `s`, in chain order.
    pub(super) fn outs_of(&self, s: S) -> Vec<A> {
        let mut out = Vec::new();
        let mut a = self.states[s].outs;
        while let Some(arc) = a {
            out.push(arc);
            a = self.arcs[arc].outchain;
        }
        out
    }

    /// `newstate`: a state with no flag, or `NOSTATE` when the NFA is too
    /// complex.
    pub(super) fn newstate(&mut self) -> S {
        let s = if let Some(s) = self.free {
            self.free = self.states[s].next;
            s
        } else {
            let space = self.budget.space.get();
            if space >= REG_MAX_COMPILE_SPACE {
                self.set_err(REG_ETOOBIG);
                return NOSTATE;
            }
            self.budget.space.set(space + STATE_SIZE);
            self.states.push(State {
                no: 0,
                flag: 0,
                nins: 0,
                ins: None,
                nouts: 0,
                outs: None,
                tmp: None,
                next: None,
                prev: None,
                noas: 0,
                nfree: 0,
                nbatches: 0,
            });
            self.states.len() - 1
        };
        let no = self.nstates;
        self.nstates += 1;
        let slast = self.slast;
        let state = &mut self.states[s];
        state.no = no;
        state.flag = 0;
        state.nins = 0;
        state.ins = None;
        state.nouts = 0;
        state.outs = None;
        state.tmp = None;
        state.next = None;
        state.prev = slast;
        if self.first.is_none() {
            self.first = Some(s);
        }
        if let Some(last) = slast {
            self.states[last].next = Some(s);
        }
        self.slast = Some(s);
        s
    }

    /// `newfstate`: a state with a flag.
    pub(super) fn newfstate(&mut self, flag: u8) -> S {
        let s = self.newstate();
        if s != NOSTATE {
            self.states[s].flag = flag;
        }
        s
    }

    /// `dropstate`: delete a state's arcs and free it.
    pub(super) fn dropstate(&mut self, s: S) {
        while let Some(a) = self.states[s].ins {
            self.freearc(a);
        }
        while let Some(a) = self.states[s].outs {
            self.freearc(a);
        }
        self.freestate(s);
    }

    /// `freestate`: free a state that has no arcs.
    pub(super) fn freestate(&mut self, s: S) {
        let (next, prev) = (self.states[s].next, self.states[s].prev);
        self.states[s].no = FREESTATE;
        self.states[s].flag = 0;
        match next {
            Some(n) => self.states[n].prev = prev,
            None => self.slast = prev,
        }
        match prev {
            Some(p) => self.states[p].next = next,
            None => self.first = next,
        }
        self.states[s].prev = None;
        self.states[s].next = self.free;
        self.free = Some(s);
    }

    /// `newarc`: a new arc, unless an identical one exists.
    pub(super) fn newarc(&mut self, t: u8, co: Color, from: S, to: S) {
        if self.err() {
            // createarc() would make nothing anyway.
            return;
        }
        if self.states[from].nouts <= self.states[to].nins {
            let mut a = self.states[from].outs;
            while let Some(arc) = a {
                let x = &self.arcs[arc];
                if x.to == to && x.co == co && x.typ == t {
                    return;
                }
                a = x.outchain;
            }
        } else {
            let mut a = self.states[to].ins;
            while let Some(arc) = a {
                let x = &self.arcs[arc];
                if x.from == from && x.co == co && x.typ == t {
                    return;
                }
                a = x.inchain;
            }
        }
        self.createarc(t, co, from, to);
    }

    /// `createarc`: a new arc, known not to duplicate one.
    fn createarc(&mut self, t: u8, co: Color, from: S, to: S) {
        self.allocarc(from);
        if self.err() {
            return;
        }
        let to_ins = self.states[to].ins;
        let from_outs = self.states[from].outs;
        let arc = Arc {
            typ: t,
            co,
            from,
            to,
            outchain: from_outs,
            outchain_rev: None,
            inchain: to_ins,
            inchain_rev: None,
            colorchain: None,
            colorchain_rev: None,
        };
        let a = match self.free_arc_slots.pop() {
            Some(a) => {
                self.arcs[a] = arc;
                a
            }
            None => {
                self.arcs.push(arc);
                self.arcs.len() - 1
            }
        };
        if let Some(x) = to_ins {
            self.arcs[x].inchain_rev = Some(a);
        }
        self.states[to].ins = Some(a);
        if let Some(x) = from_outs {
            self.arcs[x].outchain_rev = Some(a);
        }
        self.states[from].outs = Some(a);
        self.states[from].nouts += 1;
        self.states[to].nins += 1;
        if colored(t) && self.parent.is_none() {
            self.colorchain(a);
        }
    }

    /// `allocarc`, as far as the compile-space budget sees it.
    fn allocarc(&mut self, s: S) {
        let state = &mut self.states[s];
        if state.nfree == 0 && state.noas < ABSIZE {
            state.noas += 1;
            return;
        }
        if state.nfree == 0 {
            let space = self.budget.space.get();
            if space >= REG_MAX_COMPILE_SPACE {
                self.budget.set_err(REG_ETOOBIG);
                return;
            }
            self.budget.space.set(space + ARCBATCH_SIZE);
            state.nbatches += 1;
            state.nfree = ABSIZE;
        }
        state.nfree -= 1;
    }

    /// `freearc`.
    pub(super) fn freearc(&mut self, victim: A) {
        let Arc { typ, from, to, .. } = self.arcs[victim];
        if colored(typ) && self.parent.is_none() {
            self.uncolorchain(victim);
        }
        let (pred, next) = (self.arcs[victim].outchain_rev, self.arcs[victim].outchain);
        match pred {
            None => self.states[from].outs = next,
            Some(p) => self.arcs[p].outchain = next,
        }
        if let Some(n) = next {
            self.arcs[n].outchain_rev = pred;
        }
        self.states[from].nouts -= 1;
        let (pred, next) = (self.arcs[victim].inchain_rev, self.arcs[victim].inchain);
        match pred {
            None => self.states[to].ins = next,
            Some(p) => self.arcs[p].inchain = next,
        }
        if let Some(n) = next {
            self.arcs[n].inchain_rev = pred;
        }
        self.states[to].nins -= 1;
        let arc = &mut self.arcs[victim];
        arc.typ = 0;
        arc.outchain = None;
        arc.outchain_rev = None;
        arc.inchain = None;
        arc.inchain_rev = None;
        self.states[from].nfree += 1;
        self.free_arc_slots.push(victim);
    }

    /// `changearctarget`: point an arc at another state, which must not
    /// already have a duplicate of it.
    fn changearctarget(&mut self, a: A, newto: S) {
        let oldto = self.arcs[a].to;
        let (pred, next) = (self.arcs[a].inchain_rev, self.arcs[a].inchain);
        match pred {
            None => self.states[oldto].ins = next,
            Some(p) => self.arcs[p].inchain = next,
        }
        if let Some(n) = next {
            self.arcs[n].inchain_rev = pred;
        }
        self.states[oldto].nins -= 1;
        self.arcs[a].to = newto;
        let head = self.states[newto].ins;
        self.arcs[a].inchain = head;
        self.arcs[a].inchain_rev = None;
        if let Some(h) = head {
            self.arcs[h].inchain_rev = Some(a);
        }
        self.states[newto].ins = Some(a);
        self.states[newto].nins += 1;
    }

    /// `hasnonemptyout`.
    fn hasnonemptyout(&self, s: S) -> bool {
        let mut a = self.states[s].outs;
        while let Some(arc) = a {
            if self.arcs[arc].typ != EMPTY {
                return true;
            }
            a = self.arcs[arc].outchain;
        }
        false
    }

    /// `findarc`: an arc out of `s` with this type and color.
    pub(super) fn findarc(&self, s: S, typ: u8, co: Color) -> Option<A> {
        let mut a = self.states[s].outs;
        while let Some(arc) = a {
            if self.arcs[arc].typ == typ && self.arcs[arc].co == co {
                return Some(arc);
            }
            a = self.arcs[arc].outchain;
        }
        None
    }

    /// `cparc`: a new arc like `oa` between other states.
    pub(super) fn cparc(&mut self, oa: A, from: S, to: S) {
        let (typ, co) = (self.arcs[oa].typ, self.arcs[oa].co);
        self.newarc(typ, co, from, to);
    }

    pub(super) fn arc_ends(&self, a: A) -> (u8, S, S) {
        let arc = &self.arcs[a];
        (arc.typ, arc.from, arc.to)
    }

    fn ins_key(&self, a: A) -> (usize, Color, u8) {
        let arc = &self.arcs[a];
        (self.states[arc.from].no, arc.co, arc.typ)
    }

    fn outs_key(&self, a: A) -> (usize, Color, u8) {
        let arc = &self.arcs[a];
        (self.states[arc.to].no, arc.co, arc.typ)
    }

    /// `sortins`: sort a state's inarcs by from-state, color and type.
    fn sortins(&mut self, s: S) {
        let mut arcs = Vec::with_capacity(self.states[s].nins);
        let mut a = self.states[s].ins;
        while let Some(arc) = a {
            arcs.push(arc);
            a = self.arcs[arc].inchain;
        }
        if arcs.len() <= 1 {
            return;
        }
        arcs.sort_by_key(|&arc| self.ins_key(arc));
        self.states[s].ins = Some(arcs[0]);
        for (i, &arc) in arcs.iter().enumerate() {
            self.arcs[arc].inchain_rev = i.checked_sub(1).map(|j| arcs[j]);
            self.arcs[arc].inchain = arcs.get(i + 1).copied();
        }
    }

    /// `sortouts`: sort a state's outarcs by to-state, color and type.
    fn sortouts(&mut self, s: S) {
        let mut arcs = Vec::with_capacity(self.states[s].nouts);
        let mut a = self.states[s].outs;
        while let Some(arc) = a {
            arcs.push(arc);
            a = self.arcs[arc].outchain;
        }
        if arcs.len() <= 1 {
            return;
        }
        arcs.sort_by_key(|&arc| self.outs_key(arc));
        self.states[s].outs = Some(arcs[0]);
        for (i, &arc) in arcs.iter().enumerate() {
            self.arcs[arc].outchain_rev = i.checked_sub(1).map(|j| arcs[j]);
            self.arcs[arc].outchain = arcs.get(i + 1).copied();
        }
    }

    /// `moveins`: move all inarcs of a state to another state.
    pub(super) fn moveins(&mut self, old: S, new: S) {
        if !bulk_arc_op_use_sort(self.states[old].nins, self.states[new].nins) {
            while let Some(a) = self.states[old].ins {
                let from = self.arcs[a].from;
                self.cparc(a, from, new);
                self.freearc(a);
            }
            return;
        }
        self.sortins(old);
        self.sortins(new);
        let mut oa = self.states[old].ins;
        let mut na = self.states[new].ins;
        while let (Some(o), Some(n)) = (oa, na) {
            match self.ins_key(o).cmp(&self.ins_key(n)) {
                std::cmp::Ordering::Less => {
                    oa = self.arcs[o].inchain;
                    self.changearctarget(o, new);
                }
                std::cmp::Ordering::Equal => {
                    oa = self.arcs[o].inchain;
                    na = self.arcs[n].inchain;
                    self.freearc(o);
                }
                std::cmp::Ordering::Greater => na = self.arcs[n].inchain,
            }
        }
        while let Some(o) = oa {
            oa = self.arcs[o].inchain;
            self.changearctarget(o, new);
        }
    }

    /// `copyins`: copy the inarcs of a state to another state.
    pub(super) fn copyins(&mut self, old: S, new: S) {
        if !bulk_arc_op_use_sort(self.states[old].nins, self.states[new].nins) {
            let mut a = self.states[old].ins;
            while let Some(arc) = a {
                let from = self.arcs[arc].from;
                self.cparc(arc, from, new);
                a = self.arcs[arc].inchain;
            }
            return;
        }
        self.sortins(old);
        self.sortins(new);
        let mut oa = self.states[old].ins;
        let mut na = self.states[new].ins;
        while let (Some(o), Some(n)) = (oa, na) {
            match self.ins_key(o).cmp(&self.ins_key(n)) {
                std::cmp::Ordering::Less => {
                    oa = self.arcs[o].inchain;
                    let Arc { typ, co, from, .. } = self.arcs[o];
                    self.createarc(typ, co, from, new);
                }
                std::cmp::Ordering::Equal => {
                    oa = self.arcs[o].inchain;
                    na = self.arcs[n].inchain;
                }
                std::cmp::Ordering::Greater => na = self.arcs[n].inchain,
            }
        }
        while let Some(o) = oa {
            oa = self.arcs[o].inchain;
            let Arc { typ, co, from, .. } = self.arcs[o];
            self.createarc(typ, co, from, new);
        }
    }

    /// `mergeins`: merge a list of inarcs, possibly with duplicates, into
    /// a state.
    fn mergeins(&mut self, s: S, arcarray: &mut Vec<A>) {
        if arcarray.is_empty() {
            return;
        }
        self.sortins(s);
        arcarray.sort_by_key(|&arc| self.ins_key(arc));
        arcarray.dedup_by_key(|arc| self.ins_key(*arc));
        let mut i = 0;
        let mut na = self.states[s].ins;
        while i < arcarray.len()
            && let Some(n) = na
        {
            let a = arcarray[i];
            match self.ins_key(a).cmp(&self.ins_key(n)) {
                std::cmp::Ordering::Less => {
                    let Arc { typ, co, from, .. } = self.arcs[a];
                    self.createarc(typ, co, from, s);
                    i += 1;
                }
                std::cmp::Ordering::Equal => {
                    i += 1;
                    na = self.arcs[n].inchain;
                }
                std::cmp::Ordering::Greater => na = self.arcs[n].inchain,
            }
        }
        while i < arcarray.len() {
            let Arc { typ, co, from, .. } = self.arcs[arcarray[i]];
            self.createarc(typ, co, from, s);
            i += 1;
        }
    }

    /// `moveouts`: move all outarcs of a state to another state.
    pub(super) fn moveouts(&mut self, old: S, new: S) {
        if !bulk_arc_op_use_sort(self.states[old].nouts, self.states[new].nouts) {
            while let Some(a) = self.states[old].outs {
                let to = self.arcs[a].to;
                self.cparc(a, new, to);
                self.freearc(a);
            }
            return;
        }
        self.sortouts(old);
        self.sortouts(new);
        let mut oa = self.states[old].outs;
        let mut na = self.states[new].outs;
        while let (Some(o), Some(n)) = (oa, na) {
            match self.outs_key(o).cmp(&self.outs_key(n)) {
                std::cmp::Ordering::Less => {
                    oa = self.arcs[o].outchain;
                    let Arc { typ, co, to, .. } = self.arcs[o];
                    self.createarc(typ, co, new, to);
                    self.freearc(o);
                }
                std::cmp::Ordering::Equal => {
                    oa = self.arcs[o].outchain;
                    na = self.arcs[n].outchain;
                    self.freearc(o);
                }
                std::cmp::Ordering::Greater => na = self.arcs[n].outchain,
            }
        }
        while let Some(o) = oa {
            oa = self.arcs[o].outchain;
            let Arc { typ, co, to, .. } = self.arcs[o];
            self.createarc(typ, co, new, to);
            self.freearc(o);
        }
    }

    /// `copyouts`: copy the outarcs of a state to another state.
    pub(super) fn copyouts(&mut self, old: S, new: S) {
        if !bulk_arc_op_use_sort(self.states[old].nouts, self.states[new].nouts) {
            let mut a = self.states[old].outs;
            while let Some(arc) = a {
                let to = self.arcs[arc].to;
                self.cparc(arc, new, to);
                a = self.arcs[arc].outchain;
            }
            return;
        }
        self.sortouts(old);
        self.sortouts(new);
        let mut oa = self.states[old].outs;
        let mut na = self.states[new].outs;
        while let (Some(o), Some(n)) = (oa, na) {
            match self.outs_key(o).cmp(&self.outs_key(n)) {
                std::cmp::Ordering::Less => {
                    oa = self.arcs[o].outchain;
                    let Arc { typ, co, to, .. } = self.arcs[o];
                    self.createarc(typ, co, new, to);
                }
                std::cmp::Ordering::Equal => {
                    oa = self.arcs[o].outchain;
                    na = self.arcs[n].outchain;
                }
                std::cmp::Ordering::Greater => na = self.arcs[n].outchain,
            }
        }
        while let Some(o) = oa {
            oa = self.arcs[o].outchain;
            let Arc { typ, co, to, .. } = self.arcs[o];
            self.createarc(typ, co, new, to);
        }
    }

    /// `cloneouts`: copy the outarcs of a state to a state pair, with a
    /// different type.
    pub(super) fn cloneouts(&mut self, old: S, from: S, to: S, typ: u8) {
        let mut a = self.states[old].outs;
        while let Some(arc) = a {
            let co = self.arcs[arc].co;
            self.newarc(typ, co, from, to);
            a = self.arcs[arc].outchain;
        }
    }

    /// `delsub`: delete the sub-NFA from `lp` to `rp`, exclusive.
    pub(super) fn delsub(&mut self, lp: S, rp: S) {
        self.states[rp].tmp = Some(rp);
        self.deltraverse(lp, lp);
        self.states[rp].tmp = None;
        self.states[lp].tmp = None;
    }

    /// `deltraverse`: destroy the outarcs of `s0` and what they lead to.
    fn deltraverse(&mut self, _leftend: S, s0: S) {
        // Each frame is a state and the arc it is waiting on.
        let enter = |nfa: &mut Nfa, s: S| -> bool {
            if nfa.states[s].nouts == 0 || nfa.states[s].tmp.is_some() {
                return false;
            }
            nfa.states[s].tmp = Some(s);
            true
        };
        if !enter(self, s0) {
            return;
        }
        let mut stack: Vec<(S, Option<(A, S)>)> = vec![(s0, None)];
        while let Some(top) = stack.len().checked_sub(1) {
            let s = stack[top].0;
            if let Some((a, to)) = stack[top].1.take() {
                self.freearc(a);
                if self.states[to].nins == 0 && self.states[to].tmp.is_none() {
                    self.freestate(to);
                }
                continue;
            }
            match self.states[s].outs {
                Some(a) => {
                    let to = self.arcs[a].to;
                    stack[top].1 = Some((a, to));
                    if enter(self, to) {
                        stack.push((to, None));
                    }
                }
                None => {
                    self.states[s].tmp = None;
                    stack.pop();
                }
            }
        }
    }

    /// `dupnfa`: duplicate the sub-NFA from `start` to `stop` between
    /// `from` and `to`, within this NFA.
    pub(super) fn dupnfa(&mut self, start: S, stop: S, from: S, to: S) {
        if start == stop {
            self.newarc(EMPTY, 0, from, to);
            return;
        }
        self.states[stop].tmp = Some(to);
        self.duptraverse(start, from);
        self.states[stop].tmp = None;
        self.cleartraverse(start);
    }

    fn duptraverse(&mut self, s0: S, stmp: S) {
        // Frames: state, its next arc, whether that arc's target has been
        // visited, and the depth below it.
        struct Frame {
            s: S,
            a: Option<A>,
            visited: bool,
            depth: usize,
        }
        let mut stack = Vec::new();
        let enter = |nfa: &mut Nfa, stack: &mut Vec<Frame>, s: S, stmp: Option<S>, depth| {
            if nfa.states[s].tmp.is_some() {
                return;
            }
            let dup = stmp.unwrap_or_else(|| nfa.newstate());
            if dup == NOSTATE {
                return;
            }
            nfa.states[s].tmp = Some(dup);
            if depth > DUPTRAVERSE_MAX_DEPTH {
                nfa.set_err(REG_ESPACE);
            }
            let a = nfa.states[s].outs;
            stack.push(Frame {
                s,
                a,
                visited: false,
                depth: depth + 1,
            });
        };
        enter(self, &mut stack, s0, Some(stmp), 0);
        while let Some(frame) = stack.last_mut() {
            let Some(a) = frame.a else {
                stack.pop();
                continue;
            };
            if !frame.visited {
                if self.err() {
                    stack.pop();
                    continue;
                }
                frame.visited = true;
                let (to, depth) = (self.arcs[a].to, frame.depth);
                enter(self, &mut stack, to, None, depth);
                continue;
            }
            let s = frame.s;
            if self.err() {
                stack.pop();
                continue;
            }
            let to = self.arcs[a].to;
            let (from_dup, to_dup) = (self.states[s].tmp, self.states[to].tmp);
            if let (Some(fd), Some(td)) = (from_dup, to_dup) {
                self.cparc(a, fd, td);
            }
            let frame = stack.last_mut().expect("frame");
            frame.a = self.arcs[a].outchain;
            frame.visited = false;
        }
    }

    /// `cleartraverse`: clear the tmp marks a traversal left.
    pub(super) fn cleartraverse(&mut self, s0: S) {
        let mut stack = vec![s0];
        while let Some(s) = stack.pop() {
            if self.states[s].tmp.is_none() {
                continue;
            }
            self.states[s].tmp = None;
            let mut a = self.states[s].outs;
            while let Some(arc) = a {
                stack.push(self.arcs[arc].to);
                a = self.arcs[arc].outchain;
            }
        }
    }

    /// `specialcolors`: the false colors for BOS, BOL, EOS and EOL.
    pub(super) fn specialcolors(&mut self, cm: &mut ColorMap) {
        match self.parent {
            None => {
                self.bos[0] = cm.pseudocolor();
                self.bos[1] = cm.pseudocolor();
                self.eos[0] = cm.pseudocolor();
                self.eos[1] = cm.pseudocolor();
            }
            Some((bos, eos)) => {
                self.bos = bos;
                self.eos = eos;
            }
        }
    }

    /// `optimize`: reduce the NFA to PLAIN and LACON arcs, returning
    /// `re_info` bits.
    pub(super) fn optimize(&mut self) -> i64 {
        self.cleanup();
        self.fixempties();
        self.fixconstraintloops();
        self.pullback();
        self.pushfwd();
        self.cleanup();
        self.analyze()
    }

    /// `pullback`: pull back constraints backward to eliminate them.
    fn pullback(&mut self) {
        loop {
            let mut progress = false;
            let mut s = self.first;
            while let Some(st) = s {
                if self.err() {
                    break;
                }
                let nexts = self.states[st].next;
                let mut intermediates: Option<S> = None;
                let mut a = self.states[st].outs;
                while let Some(arc) = a {
                    if self.err() {
                        break;
                    }
                    let nexta = self.arcs[arc].outchain;
                    let typ = self.arcs[arc].typ;
                    if (typ == b'^' || typ == BEHIND) && self.pull(arc, &mut intermediates) {
                        progress = true;
                    }
                    a = nexta;
                }
                // Clear the tmp fields of intermediate states made here.
                while let Some(i) = intermediates {
                    intermediates = self.states[i].tmp;
                    self.states[i].tmp = None;
                }
                if (self.states[st].nins == 0 || self.states[st].nouts == 0)
                    && self.states[st].flag == 0
                {
                    self.dropstate(st);
                }
                s = nexts;
            }
            if !progress || self.err() {
                break;
            }
        }
        if self.err() {
            return;
        }
        // ^ constraints pulled to the start become BOS/BOL color arcs.
        let mut a = self.states[self.pre].outs;
        while let Some(arc) = a {
            let nexta = self.arcs[arc].outchain;
            if self.arcs[arc].typ == b'^' {
                let Arc { co, from, to, .. } = self.arcs[arc];
                self.newarc(PLAIN, self.bos[co as usize], from, to);
                self.freearc(arc);
            }
            a = nexta;
        }
    }

    /// `pull`: pull a back constraint backward past its source state.
    fn pull(&mut self, con: A, intermediates: &mut Option<S>) -> bool {
        let mut con = con;
        let mut from = self.arcs[con].from;
        let to = self.arcs[con].to;
        if self.states[from].flag != 0 {
            return false;
        }
        if self.states[from].nins == 0 {
            self.freearc(con);
            return true;
        }
        // Clone the from state first if it has other outarcs.
        if self.states[from].nouts > 1 {
            let s = self.newstate();
            if self.err() {
                return false;
            }
            self.copyins(from, s);
            self.cparc(con, s, to);
            self.freearc(con);
            if self.err() {
                return false;
            }
            from = s;
            con = self.states[from].outs.expect("cloned constraint");
        }
        // Propagate the constraint into the from state's inarcs.
        let mut a = self.states[from].ins;
        while let Some(arc) = a {
            if self.err() {
                break;
            }
            let nexta = self.arcs[arc].inchain;
            match self.combine(con, arc) {
                INCOMPATIBLE => self.freearc(arc),
                SATISFIED => {}
                _ => {
                    // Swap the two arcs, through an intermediate state.
                    let afrom = self.arcs[arc].from;
                    let mut s = *intermediates;
                    while let Some(i) = s {
                        let first_in = self.states[i].ins.expect("intermediate inarc");
                        let first_out = self.states[i].outs.expect("intermediate outarc");
                        if self.arcs[first_in].from == afrom && self.arcs[first_out].to == to {
                            break;
                        }
                        s = self.states[i].tmp;
                    }
                    let s = match s {
                        Some(s) => s,
                        None => {
                            let s = self.newstate();
                            if self.err() {
                                return false;
                            }
                            self.states[s].tmp = *intermediates;
                            *intermediates = Some(s);
                            s
                        }
                    };
                    self.cparc(con, afrom, s);
                    self.cparc(arc, s, to);
                    self.freearc(arc);
                }
            }
            a = nexta;
        }
        // Remaining inarcs incorporate the constraint.
        self.moveins(from, to);
        self.freearc(con);
        true
    }

    /// `pushfwd`: push forward constraints forward to eliminate them.
    fn pushfwd(&mut self) {
        loop {
            let mut progress = false;
            let mut s = self.first;
            while let Some(st) = s {
                if self.err() {
                    break;
                }
                let nexts = self.states[st].next;
                let mut intermediates: Option<S> = None;
                let mut a = self.states[st].ins;
                while let Some(arc) = a {
                    if self.err() {
                        break;
                    }
                    let nexta = self.arcs[arc].inchain;
                    let typ = self.arcs[arc].typ;
                    if (typ == b'$' || typ == AHEAD) && self.push(arc, &mut intermediates) {
                        progress = true;
                    }
                    a = nexta;
                }
                while let Some(i) = intermediates {
                    intermediates = self.states[i].tmp;
                    self.states[i].tmp = None;
                }
                if (self.states[st].nins == 0 || self.states[st].nouts == 0)
                    && self.states[st].flag == 0
                {
                    self.dropstate(st);
                }
                s = nexts;
            }
            if !progress || self.err() {
                break;
            }
        }
        if self.err() {
            return;
        }
        // $ constraints pushed to the end become EOS/EOL color arcs.
        let mut a = self.states[self.post].ins;
        while let Some(arc) = a {
            let nexta = self.arcs[arc].inchain;
            if self.arcs[arc].typ == b'$' {
                let Arc { co, from, to, .. } = self.arcs[arc];
                self.newarc(PLAIN, self.eos[co as usize], from, to);
                self.freearc(arc);
            }
            a = nexta;
        }
    }

    /// `push`: push a forward constraint forward past its destination.
    fn push(&mut self, con: A, intermediates: &mut Option<S>) -> bool {
        let mut con = con;
        let from = self.arcs[con].from;
        let mut to = self.arcs[con].to;
        if self.states[to].flag != 0 {
            return false;
        }
        if self.states[to].nouts == 0 {
            self.freearc(con);
            return true;
        }
        // Clone the to state first if it has other inarcs.
        if self.states[to].nins > 1 {
            let s = self.newstate();
            if self.err() {
                return false;
            }
            self.copyouts(to, s);
            self.cparc(con, from, s);
            self.freearc(con);
            if self.err() {
                return false;
            }
            to = s;
            con = self.states[to].ins.expect("cloned constraint");
        }
        // Propagate the constraint into the to state's outarcs.
        let mut a = self.states[to].outs;
        while let Some(arc) = a {
            if self.err() {
                break;
            }
            let nexta = self.arcs[arc].outchain;
            match self.combine(con, arc) {
                INCOMPATIBLE => self.freearc(arc),
                SATISFIED => {}
                _ => {
                    let ato = self.arcs[arc].to;
                    let mut s = *intermediates;
                    while let Some(i) = s {
                        let first_in = self.states[i].ins.expect("intermediate inarc");
                        let first_out = self.states[i].outs.expect("intermediate outarc");
                        if self.arcs[first_in].from == from && self.arcs[first_out].to == ato {
                            break;
                        }
                        s = self.states[i].tmp;
                    }
                    let s = match s {
                        Some(s) => s,
                        None => {
                            let s = self.newstate();
                            if self.err() {
                                return false;
                            }
                            self.states[s].tmp = *intermediates;
                            *intermediates = Some(s);
                            s
                        }
                    };
                    self.cparc(con, s, ato);
                    self.cparc(arc, from, s);
                    self.freearc(arc);
                }
            }
            a = nexta;
        }
        // Remaining outarcs incorporate the constraint.
        self.moveouts(to, from);
        self.freearc(con);
        true
    }

    /// `combine`: what a constraint landing on an arc does.
    fn combine(&self, con: A, a: A) -> u8 {
        let (ct, cco) = (self.arcs[con].typ, self.arcs[con].co);
        let (at, aco) = (self.arcs[a].typ, self.arcs[a].co);
        match (ct, at) {
            (b'^' | b'$', PLAIN) => INCOMPATIBLE,
            (AHEAD | BEHIND, PLAIN) => {
                if cco == aco {
                    SATISFIED
                } else {
                    INCOMPATIBLE
                }
            }
            (b'^', b'^') | (b'$', b'$') | (AHEAD, AHEAD) | (BEHIND, BEHIND) => {
                if cco == aco {
                    SATISFIED
                } else {
                    INCOMPATIBLE
                }
            }
            (b'^', BEHIND) | (BEHIND, b'^') | (b'$', AHEAD) | (AHEAD, b'$') => INCOMPATIBLE,
            (b'^', b'$')
            | (b'^', AHEAD)
            | (BEHIND, b'$')
            | (BEHIND, AHEAD)
            | (b'$', b'^')
            | (b'$', BEHIND)
            | (AHEAD, b'^')
            | (AHEAD, BEHIND)
            | (b'^', LACON)
            | (BEHIND, LACON)
            | (b'$', LACON)
            | (AHEAD, LACON) => COMPATIBLE,
            _ => INCOMPATIBLE,
        }
    }

    /// `fixempties`: get rid of EMPTY arcs.
    fn fixempties(&mut self) {
        // States whose sole outarc is an EMPTY are aliases for its target.
        let mut s = self.first;
        while let Some(st) = s {
            if self.err() {
                break;
            }
            let nexts = self.states[st].next;
            if self.states[st].flag == 0 && self.states[st].nouts == 1 {
                let a = self.states[st].outs.expect("sole outarc");
                if self.arcs[a].typ == EMPTY {
                    let to = self.arcs[a].to;
                    if st != to {
                        self.moveins(st, to);
                    }
                    self.dropstate(st);
                }
            }
            s = nexts;
        }
        // Likewise fold a state with a single EMPTY inarc into its source.
        let mut s = self.first;
        while let Some(st) = s {
            if self.err() {
                break;
            }
            let nexts = self.states[st].next;
            if self.states[st].flag == 0 && self.states[st].nins == 1 {
                let a = self.states[st].ins.expect("sole inarc");
                if self.arcs[a].typ == EMPTY {
                    let from = self.arcs[a].from;
                    if st != from {
                        self.moveouts(st, from);
                    }
                    self.dropstate(st);
                }
            }
            s = nexts;
        }
        if self.err() {
            return;
        }
        // Remember each state's first original inarc; arcs added below go
        // on the front of the chains, so the originals stay at the end.
        let mut inarcsorig: Vec<Option<A>> = vec![None; self.nstates];
        let mut totalinarcs = 0;
        for st in self.chain() {
            inarcsorig[self.states[st].no] = self.states[st].ins;
            totalinarcs += self.states[st].nins;
        }
        let mut arcarray: Vec<A> = Vec::with_capacity(totalinarcs);
        let mut s = self.first;
        while let Some(st) = s {
            if self.err() {
                break;
            }
            // States without non-EMPTY outarcs will be useless anyway.
            if self.states[st].flag != 0 || self.hasnonemptyout(st) {
                arcarray.clear();
                let mut s2 = self.emptyreachable(st, st, &inarcsorig);
                while s2 != st {
                    let mut a = inarcsorig[self.states[s2].no];
                    while let Some(arc) = a {
                        if self.arcs[arc].typ != EMPTY {
                            arcarray.push(arc);
                        }
                        a = self.arcs[arc].inchain;
                    }
                    let nexts = self.states[s2].tmp.expect("emptyreachable chain");
                    self.states[s2].tmp = None;
                    s2 = nexts;
                }
                self.states[st].tmp = None;
                let prevnins = self.states[st].nins;
                self.mergeins(st, &mut arcarray);
                // The original inarcs follow the ones just added.
                let mut nskip = self.states[st].nins - prevnins;
                let mut a = self.states[st].ins;
                while nskip > 0 {
                    a = a.and_then(|arc| self.arcs[arc].inchain);
                    nskip -= 1;
                }
                inarcsorig[self.states[st].no] = a;
            }
            s = self.states[st].next;
        }
        if self.err() {
            return;
        }
        // Remove all the EMPTY arcs.
        for st in self.chain() {
            let mut a = self.states[st].outs;
            while let Some(arc) = a {
                let nexta = self.arcs[arc].outchain;
                if self.arcs[arc].typ == EMPTY {
                    self.freearc(arc);
                }
                a = nexta;
            }
        }
        // And the states that became useless.
        for st in self.chain() {
            if (self.states[st].nins == 0 || self.states[st].nouts == 0)
                && self.states[st].flag == 0
            {
                self.dropstate(st);
            }
        }
    }

    /// `emptyreachable`: the states that reach `s0` by EMPTY arcs, chained
    /// back to `s0` through their tmp fields; returns the last found.
    fn emptyreachable(&mut self, s0: S, lastfound: S, inarcsorig: &[Option<A>]) -> S {
        let mut lastfound = lastfound;
        self.states[s0].tmp = Some(lastfound);
        lastfound = s0;
        let mut stack = vec![(s0, inarcsorig[self.states[s0].no])];
        while let Some(top) = stack.len().checked_sub(1) {
            let Some(a) = stack[top].1 else {
                stack.pop();
                continue;
            };
            stack[top].1 = self.arcs[a].inchain;
            let from = self.arcs[a].from;
            if self.arcs[a].typ == EMPTY && self.states[from].tmp.is_none() {
                self.states[from].tmp = Some(lastfound);
                lastfound = from;
                stack.push((from, inarcsorig[self.states[from].no]));
            }
        }
        lastfound
    }

    /// `hasconstraintout`.
    fn hasconstraintout(&self, s: S) -> bool {
        let mut a = self.states[s].outs;
        while let Some(arc) = a {
            if is_constraint(self.arcs[arc].typ) {
                return true;
            }
            a = self.arcs[arc].outchain;
        }
        false
    }

    /// `fixconstraintloops`: get rid of loops of constraint arcs.
    fn fixconstraintloops(&mut self) {
        // Single-state loops just lose their constraint arcs.
        let mut hasconstraints = false;
        let mut s = self.first;
        while let Some(st) = s {
            if self.err() {
                break;
            }
            let nexts = self.states[st].next;
            let mut a = self.states[st].outs;
            while let Some(arc) = a {
                if self.err() {
                    break;
                }
                let nexta = self.arcs[arc].outchain;
                if is_constraint(self.arcs[arc].typ) {
                    if self.arcs[arc].to == st {
                        self.freearc(arc);
                    } else {
                        hasconstraints = true;
                    }
                }
                a = nexta;
            }
            if self.states[st].nouts == 0 && self.states[st].flag == 0 {
                self.dropstate(st);
            }
            s = nexts;
        }
        if self.err() || !hasconstraints {
            return;
        }
        // Break multi-state loops one at a time, starting over each time.
        'restart: loop {
            let mut s = self.first;
            while let Some(st) = s {
                if self.err() {
                    break 'restart;
                }
                if self.findconstraintloop(st) {
                    continue 'restart;
                }
                s = self.states[st].next;
            }
            break;
        }
        if self.err() {
            return;
        }
        for st in self.chain() {
            self.states[st].tmp = None;
            if (self.states[st].nins == 0 || self.states[st].nouts == 0)
                && self.states[st].flag == 0
            {
                self.dropstate(st);
            }
        }
    }

    /// `findconstraintloop`: find and break a loop of constraint arcs
    /// reachable from `s0`. States proven loop-free are marked with
    /// `tmp == self`.
    fn findconstraintloop(&mut self, s0: S) -> bool {
        match self.states[s0].tmp {
            Some(t) if t == s0 => return false,
            Some(_) => {
                self.breakconstraintloop(s0);
                return true;
            }
            None => {}
        }
        let mut stack = vec![(s0, self.states[s0].outs)];
        while let Some(top) = stack.len().checked_sub(1) {
            let s = stack[top].0;
            let Some(a) = stack[top].1 else {
                self.states[s].tmp = Some(s);
                stack.pop();
                continue;
            };
            stack[top].1 = self.arcs[a].outchain;
            if !is_constraint(self.arcs[a].typ) {
                continue;
            }
            let sto = self.arcs[a].to;
            self.states[s].tmp = Some(sto);
            match self.states[sto].tmp {
                Some(t) if t == sto => {}
                Some(_) => {
                    self.breakconstraintloop(sto);
                    return true;
                }
                None => stack.push((sto, self.states[sto].outs)),
            }
        }
        false
    }

    /// `breakconstraintloop`: break the loop through `sinitial` whose
    /// members' tmp fields link each to its successor.
    fn breakconstraintloop(&mut self, sinitial: S) {
        // Prefer to break at a step with only one constraint arc.
        let mut refarc = None;
        let mut s = sinitial;
        loop {
            let nexts = self.states[s].tmp.expect("constraint loop link");
            if refarc.is_none() {
                let mut narcs = 0;
                let mut a = self.states[s].outs;
                while let Some(arc) = a {
                    if self.arcs[arc].to == nexts && is_constraint(self.arcs[arc].typ) {
                        refarc = Some(arc);
                        narcs += 1;
                    }
                    a = self.arcs[arc].outchain;
                }
                if narcs > 1 {
                    refarc = None;
                }
            }
            s = nexts;
            if s == sinitial {
                break;
            }
        }
        let (shead, stail) = match refarc {
            Some(arc) => (self.arcs[arc].from, self.arcs[arc].to),
            None => (
                sinitial,
                self.states[sinitial].tmp.expect("constraint loop link"),
            ),
        };
        for st in self.chain() {
            self.states[st].tmp = None;
        }
        let sclone = self.newstate();
        if sclone == NOSTATE {
            return;
        }
        let nstates = self.nstates;
        self.clonesuccessorstates(stail, sclone, shead, refarc, None, None, nstates);
        if self.err() {
            return;
        }
        let sclone = if self.states[sclone].nouts == 0 {
            self.freestate(sclone);
            None
        } else {
            Some(sclone)
        };
        // Move shead's loop arcs to the clone, or drop them.
        let mut a = self.states[shead].outs;
        while let Some(arc) = a {
            let nexta = self.arcs[arc].outchain;
            if self.arcs[arc].to == stail && is_constraint(self.arcs[arc].typ) {
                if let Some(clone) = sclone {
                    self.cparc(arc, shead, clone);
                }
                self.freearc(arc);
                if self.err() {
                    break;
                }
            }
            a = nexta;
        }
    }

    /// `clonesuccessorstates`: build the tree of constraint-arc successor
    /// states of `ssource` into `sclone`. `curdonemap` is set when merging
    /// into the caller's clone state; `outerdonemap` is the parent clone's
    /// map.
    #[allow(clippy::too_many_arguments)]
    fn clonesuccessorstates(
        &mut self,
        ssource: S,
        sclone: S,
        spredecessor: S,
        refarc: Option<A>,
        curdonemap: Option<&mut Vec<u8>>,
        outerdonemap: Option<&[u8]>,
        nstates: usize,
    ) {
        let outermost = curdonemap.is_none();
        let mut owned;
        let donemap: &mut Vec<u8> = match curdonemap {
            Some(map) => map,
            None => {
                owned = match outerdonemap {
                    Some(outer) => outer.to_vec(),
                    None => {
                        let mut map = vec![0; nstates];
                        map[self.states[spredecessor].no] = 1;
                        map
                    }
                };
                &mut owned
            }
        };
        donemap[self.states[ssource].no] = 1;
        let (refty, refco) = match refarc {
            Some(arc) => (Some(self.arcs[arc].typ), self.arcs[arc].co),
            None => (None, 0),
        };
        // Clone ssource's outarcs, making child clone states as needed;
        // child clones' tmp fields name the state they copy.
        let mut a = self.states[ssource].outs;
        while let Some(arc) = a {
            if self.err() {
                break;
            }
            let (typ, co, sto) = (self.arcs[arc].typ, self.arcs[arc].co, self.arcs[arc].to);
            if is_constraint(typ) && self.hasconstraintout(sto) {
                if donemap[self.states[sto].no] != 0 {
                    a = self.arcs[arc].outchain;
                    continue;
                }
                let mut prevclone = None;
                let mut a2 = self.states[sclone].outs;
                while let Some(arc2) = a2 {
                    let target = self.arcs[arc2].to;
                    if self.states[target].tmp == Some(sto) {
                        prevclone = Some(target);
                        break;
                    }
                    a2 = self.arcs[arc2].outchain;
                }
                let mut canmerge = refty == Some(typ) && co == refco;
                if !canmerge {
                    let mut s = sclone;
                    while let Some(first_in) = self.states[s].ins {
                        if self.states[s].nins == 1
                            && typ == self.arcs[first_in].typ
                            && co == self.arcs[first_in].co
                        {
                            canmerge = true;
                            break;
                        }
                        s = self.arcs[first_in].from;
                    }
                }
                if canmerge {
                    if let Some(prev) = prevclone {
                        self.dropstate(prev);
                    }
                    self.clonesuccessorstates(
                        sto,
                        sclone,
                        spredecessor,
                        refarc,
                        Some(donemap),
                        outerdonemap,
                        nstates,
                    );
                } else if let Some(prev) = prevclone {
                    self.cparc(arc, sclone, prev);
                } else {
                    let stoclone = self.newstate();
                    if stoclone == NOSTATE {
                        break;
                    }
                    self.states[stoclone].tmp = Some(sto);
                    self.cparc(arc, sclone, stoclone);
                }
            } else {
                self.cparc(arc, sclone, sto);
            }
            a = self.arcs[arc].outchain;
        }
        // At the outer level for this clone, recurse to its child clones.
        if outermost {
            let mut a = self.states[sclone].outs;
            while let Some(arc) = a {
                if self.err() {
                    break;
                }
                let stoclone = self.arcs[arc].to;
                if let Some(sto) = self.states[stoclone].tmp {
                    self.states[stoclone].tmp = None;
                    self.clonesuccessorstates(
                        sto,
                        stoclone,
                        spredecessor,
                        refarc,
                        None,
                        Some(donemap),
                        nstates,
                    );
                }
                a = self.arcs[arc].outchain;
            }
        }
    }

    /// `cleanup`: drop unreachable and dead-end states, and renumber.
    pub(super) fn cleanup(&mut self) {
        let (pre, post) = (self.pre, self.post);
        self.markreachable(pre, None, Some(pre));
        self.markcanreach(post, Some(pre), Some(post));
        for st in self.chain() {
            if self.states[st].tmp != Some(post) && self.states[st].flag == 0 {
                self.dropstate(st);
            }
        }
        self.cleartraverse(pre);
        let mut n = 0;
        let mut s = self.first;
        while let Some(st) = s {
            self.states[st].no = n;
            n += 1;
            s = self.states[st].next;
        }
        self.nstates = n;
    }

    /// `markreachable`: mark states reachable from `s0` that bear `okay`.
    fn markreachable(&mut self, s0: S, okay: Option<S>, mark: Option<S>) {
        let mut stack = vec![s0];
        while let Some(s) = stack.pop() {
            if self.states[s].tmp != okay {
                continue;
            }
            self.states[s].tmp = mark;
            let mut a = self.states[s].outs;
            while let Some(arc) = a {
                stack.push(self.arcs[arc].to);
                a = self.arcs[arc].outchain;
            }
        }
    }

    /// `markcanreach`: mark states that can reach `s0` and bear `okay`.
    fn markcanreach(&mut self, s0: S, okay: Option<S>, mark: Option<S>) {
        let mut stack = vec![s0];
        while let Some(s) = stack.pop() {
            if self.states[s].tmp != okay {
                continue;
            }
            self.states[s].tmp = mark;
            let mut a = self.states[s].ins;
            while let Some(arc) = a {
                stack.push(self.arcs[arc].from);
                a = self.arcs[arc].inchain;
            }
        }
    }

    /// `analyze`: facts about an optimized NFA.
    fn analyze(&self) -> i64 {
        let pre = self.pre;
        if self.states[pre].outs.is_none() {
            return REG_UIMPOSSIBLE;
        }
        let mut a = self.states[pre].outs;
        while let Some(arc) = a {
            let mut aa = self.states[self.arcs[arc].to].outs;
            while let Some(arc2) = aa {
                if self.arcs[arc2].to == self.post {
                    return REG_UEMPTYMATCH;
                }
                aa = self.arcs[arc2].outchain;
            }
            a = self.arcs[arc].outchain;
        }
        0
    }

    /// `compact`: the compacted form of an optimized NFA.
    pub(super) fn compact(&self, cm: &ColorMap) -> Cnfa {
        let chain = self.chain();
        let nstates = chain.len();
        let mut cnfa = Cnfa {
            nstates,
            ncolors: (cm.maxcolor() + 1) as usize,
            flags: 0,
            pre: self.states[self.pre].no,
            post: self.states[self.post].no,
            bos: self.bos,
            eos: self.eos,
            stflags: vec![0; nstates],
            states: vec![0; nstates],
            arcs: Vec::with_capacity(chain.iter().map(|&s| self.states[s].nouts + 1).sum()),
        };
        for &s in &chain {
            let no = self.states[s].no;
            cnfa.states[no] = cnfa.arcs.len();
            let first = cnfa.arcs.len();
            let mut a = self.states[s].outs;
            while let Some(arc) = a {
                let x = &self.arcs[arc];
                match x.typ {
                    PLAIN => cnfa.arcs.push(Carc {
                        co: x.co,
                        to: self.states[x.to].no,
                    }),
                    LACON => {
                        cnfa.arcs.push(Carc {
                            co: cnfa.ncolors as Color + x.co,
                            to: self.states[x.to].no,
                        });
                        cnfa.flags |= HASLACONS;
                    }
                    _ => self.set_err(REG_ASSERT),
                }
                a = x.outchain;
            }
            cnfa.arcs[first..].sort_by_key(|arc| (arc.co, arc.to));
            cnfa.arcs.push(Carc {
                co: COLORLESS,
                to: 0,
            });
        }
        // Mark the no-progress states.
        let mut a = self.states[self.pre].outs;
        while let Some(arc) = a {
            cnfa.stflags[self.states[self.arcs[arc].to].no] = CNFA_NOPROGRESS;
            a = self.arcs[arc].outchain;
        }
        cnfa.stflags[self.states[self.pre].no] = CNFA_NOPROGRESS;
        cnfa
    }

    // Color chains (`colorchain` and `uncolorchain` in regc_color.c).

    fn colorchain(&mut self, a: A) {
        let co = self.arcs[a].co;
        let Ok(c) = usize::try_from(co) else {
            return;
        };
        if self.colorchains.len() <= c {
            self.colorchains.resize(c + 1, None);
        }
        let head = self.colorchains[c];
        if let Some(h) = head {
            self.arcs[h].colorchain_rev = Some(a);
        }
        self.arcs[a].colorchain = head;
        self.arcs[a].colorchain_rev = None;
        self.colorchains[c] = Some(a);
    }

    fn uncolorchain(&mut self, a: A) {
        let Ok(c) = usize::try_from(self.arcs[a].co) else {
            return;
        };
        let (prev, next) = (self.arcs[a].colorchain_rev, self.arcs[a].colorchain);
        match prev {
            None => {
                if let Some(head) = self.colorchains.get_mut(c) {
                    *head = next;
                }
            }
            Some(p) => self.arcs[p].colorchain = next,
        }
        if let Some(n) = next {
            self.arcs[n].colorchain_rev = prev;
        }
        self.arcs[a].colorchain = None;
        self.arcs[a].colorchain_rev = None;
    }

    pub(super) fn colorchain_head(&self, co: Color) -> Option<A> {
        usize::try_from(co)
            .ok()
            .and_then(|c| self.colorchains.get(c).copied().flatten())
    }

    pub(super) fn colorchain_next(&self, a: A) -> Option<A> {
        self.arcs[a].colorchain
    }

    /// Move an arc to another color's chain.
    pub(super) fn recolor(&mut self, a: A, co: Color) {
        self.uncolorchain(a);
        self.arcs[a].co = co;
        self.colorchain(a);
    }

    /// `rainbow`: arcs of every full color but `but` between two states.
    pub(super) fn rainbow(&mut self, cm: &ColorMap, typ: u8, but: Color, from: S, to: S) {
        for (co, cd) in cm.colors() {
            if self.err() {
                break;
            }
            if !cd.unused() && cd.sub != co && co != but && cd.flags & PSEUDO == 0 {
                self.newarc(typ, co, from, to);
            }
        }
    }

    /// `colorcomplement`: arcs of the colors `of` has no PLAIN outarc of.
    pub(super) fn colorcomplement(&mut self, cm: &ColorMap, typ: u8, of: S, from: S, to: S) {
        for (co, cd) in cm.colors() {
            if self.err() {
                break;
            }
            if !cd.unused() && cd.flags & PSEUDO == 0 && self.findarc(of, PLAIN, co).is_none() {
                self.newarc(typ, co, from, to);
            }
        }
    }
}

/// `dupnfa` from one NFA into another: the NFA of a subexpression node is
/// a copy of its part of the main NFA. The source states' tmp fields point
/// at their copies while it runs.
pub(super) fn dupnfa_into(src: &mut Nfa, dst: &mut Nfa, start: S, stop: S, from: S, to: S) {
    if start == stop {
        dst.newarc(EMPTY, 0, from, to);
        return;
    }
    src.states[stop].tmp = Some(to);
    struct Frame {
        s: S,
        a: Option<A>,
        visited: bool,
        depth: usize,
    }
    let enter =
        |src: &mut Nfa, dst: &mut Nfa, stack: &mut Vec<Frame>, s: S, stmp: Option<S>, depth| {
            if src.states[s].tmp.is_some() {
                return;
            }
            let dup = stmp.unwrap_or_else(|| dst.newstate());
            if dup == NOSTATE {
                return;
            }
            src.states[s].tmp = Some(dup);
            if depth > DUPTRAVERSE_MAX_DEPTH {
                dst.set_err(REG_ESPACE);
            }
            let a = src.states[s].outs;
            stack.push(Frame {
                s,
                a,
                visited: false,
                depth: depth + 1,
            });
        };
    let mut stack = Vec::new();
    enter(src, dst, &mut stack, start, Some(from), 0);
    while let Some(frame) = stack.last_mut() {
        let Some(a) = frame.a else {
            stack.pop();
            continue;
        };
        if !frame.visited {
            if dst.err() {
                stack.pop();
                continue;
            }
            frame.visited = true;
            let (to_state, depth) = (src.arcs[a].to, frame.depth);
            enter(src, dst, &mut stack, to_state, None, depth);
            continue;
        }
        let s = frame.s;
        if dst.err() {
            stack.pop();
            continue;
        }
        let Arc {
            typ, co, to: ato, ..
        } = src.arcs[a];
        if let (Some(fd), Some(td)) = (src.states[s].tmp, src.states[ato].tmp) {
            dst.newarc(typ, co, fd, td);
        }
        let frame = stack.last_mut().expect("frame");
        frame.a = src.arcs[a].outchain;
        frame.visited = false;
    }
    src.states[stop].tmp = None;
    src.cleartraverse(start);
}

/// `BULK_ARC_OP_USE_SORT`: whether to sort-merge rather than copy arcs one
/// at a time.
fn bulk_arc_op_use_sort(nsrc: usize, ndest: usize) -> bool {
    nsrc >= 4 && (nsrc > 32 || ndest > 32)
}
