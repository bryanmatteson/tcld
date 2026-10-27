/*
 * colorings of characters
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
 * Note that there are some incestuous relationships between this code and NFA
 * arc maintenance, which perhaps ought to be cleaned up sometime.
 */

// Ported from Tcl 9.0.2 generic/regc_color.c. Modified for Tcld: native Rust
// data structures and VM integration. Tcl licensing terms: license.terms.

//! The color map (`regc_color.c`): characters are partitioned into "colors",
//! the equivalence classes the expression can tell apart, through a
//! four-level tree indexed by the bytes of a character. Untouched parts of
//! the tree share fill blocks, and blocks of one color share that color's
//! solid block.
//!
//! The color chains that `struct colordesc` heads in C live with the main
//! NFA here (see [`super::nfa::Nfa`]), since only its arcs are on them.

use std::rc::Rc;

use super::nfa::{Nfa, S};
use super::token::PLAIN;
use super::{Budget, COLORLESS, Chr, Color, REG_ECOLORS, WHITE};

const BYTBITS: u32 = 8;
const BYTTAB: usize = 1 << BYTBITS;
const BYTMASK: u32 = (1 << BYTBITS) - 1;
const NBYTS: u32 = 4;
const MAX_COLOR: usize = i16::MAX as usize;
const CHR_MIN: u32 = 0;
const CHR_MAX: u32 = 0x10FFFF;

/// A color's open subcolor, or the free-list link of a free color.
pub(super) const NOSUB: Color = COLORLESS;
/// The color is free.
pub(super) const FREECOL: u8 = 0o1;
/// A pseudocolor, which no character has (BOS, EOS and friends).
pub(super) const PSEUDO: u8 = 0o2;

#[derive(Debug, Clone)]
pub(super) struct ColorDesc {
    pub nchrs: u32,
    pub sub: Color,
    pub flags: u8,
    /// The solid block of this color, if any.
    block: Option<usize>,
}

impl ColorDesc {
    fn new() -> Self {
        ColorDesc {
            nchrs: 0,
            sub: NOSUB,
            flags: 0,
            block: None,
        }
    }

    pub fn unused(&self) -> bool {
        self.flags & FREECOL != 0
    }
}

pub(crate) struct ColorMap {
    /// Color descriptors; entries past `max` are stale, as in C.
    pub(super) cd: Vec<ColorDesc>,
    /// The highest color in use.
    pub(super) max: usize,
    /// The head of the free-color chain, or 0.
    free: Color,
    /// Pointer blocks: the tree top (0) and the fill blocks of the two
    /// middle levels (1 and 2), then blocks made by `setcolor`. Entries of
    /// levels 0 and 1 index pointer blocks; entries of level 2 index color
    /// blocks.
    #[allow(
        clippy::vec_box,
        reason = "Grow sparse tree pages without copying existing pages"
    )]
    ptrs: Vec<Box<[u32; BYTTAB]>>,
    /// Color blocks: the bottom fill block (0, solid WHITE), then the rest.
    #[allow(
        clippy::vec_box,
        reason = "Grow sparse tree pages without copying existing pages"
    )]
    colors: Vec<Box<[i16; BYTTAB]>>,
    budget: Rc<Budget>,
}

impl ColorMap {
    /// `initcm`.
    pub(super) fn new(budget: Rc<Budget>) -> Self {
        let mut white = ColorDesc::new();
        white.nchrs = CHR_MAX - CHR_MIN + 1;
        white.block = Some(0);
        ColorMap {
            cd: vec![white],
            max: 0,
            free: 0,
            ptrs: vec![
                Box::new([1; BYTTAB]),
                Box::new([2; BYTTAB]),
                Box::new([0; BYTTAB]),
            ],
            colors: vec![Box::new([WHITE as i16; BYTTAB])],
            budget,
        }
    }

    fn cd(&self, co: Color) -> &ColorDesc {
        &self.cd[co as usize]
    }

    fn cd_mut(&mut self, co: Color) -> &mut ColorDesc {
        &mut self.cd[co as usize]
    }

    /// `GETCOLOR`.
    pub fn getcolor(&self, c: Chr) -> Color {
        let b = |shift: u32| ((c >> shift) & BYTMASK) as usize;
        let l1 = self.ptrs[0][b(24)] as usize;
        let l2 = self.ptrs[l1][b(16)] as usize;
        let block = self.ptrs[l2][b(8)] as usize;
        Color::from(self.colors[block][b(0)])
    }

    /// `setcolor`: set the color of a character, returning its old color.
    fn setcolor(&mut self, c: Chr, co: Color) -> Color {
        if self.budget.is_err() || co == COLORLESS {
            return COLORLESS;
        }
        let mut t = 0;
        let mut shift = BYTBITS * (NBYTS - 1);
        let mut level = 0;
        while shift > 0 {
            let b = ((c >> shift) & BYTMASK) as usize;
            let lastt = t;
            let child = self.ptrs[lastt][b] as usize;
            if shift > BYTBITS {
                // A pointer block below; the fill block must not change.
                if child == level + 1 {
                    let copy = self.ptrs[child].clone();
                    self.ptrs.push(copy);
                    t = self.ptrs.len() - 1;
                    self.ptrs[lastt][b] = t as u32;
                } else {
                    t = child;
                }
            } else {
                // A color block below; fill and solid blocks are shared.
                let solid = self.cd(Color::from(self.colors[child][0])).block;
                if child == 0 || solid == Some(child) {
                    let copy = self.colors[child].clone();
                    self.colors.push(copy);
                    t = self.colors.len() - 1;
                    self.ptrs[lastt][b] = t as u32;
                } else {
                    t = child;
                }
            }
            level += 1;
            shift -= BYTBITS;
        }
        let b = (c & BYTMASK) as usize;
        let prev = Color::from(self.colors[t][b]);
        self.colors[t][b] = co as i16;
        prev
    }

    /// `maxcolor`.
    pub(super) fn maxcolor(&self) -> Color {
        if self.budget.is_err() {
            return COLORLESS;
        }
        self.max as Color
    }

    /// `newcolor`: a new color, which must at once become some
    /// character's color.
    fn newcolor(&mut self) -> Color {
        if self.budget.is_err() {
            return COLORLESS;
        }
        let co = if self.free != 0 {
            let co = self.free as usize;
            self.free = self.cd[co].sub;
            co
        } else if self.max < MAX_COLOR {
            self.max += 1;
            if self.max == self.cd.len() {
                self.cd.push(ColorDesc::new());
            }
            self.max
        } else {
            self.budget.set_err(REG_ECOLORS);
            return COLORLESS;
        };
        self.cd[co] = ColorDesc::new();
        co as Color
    }

    /// `freecolor`: free a color that has no arcs or subcolor.
    fn freecolor(&mut self, co: Color) {
        if co == WHITE {
            return;
        }
        let c = co as usize;
        self.cd[c].flags = FREECOL;
        self.cd[c].block = None;
        if c == self.max {
            while self.max > WHITE as usize && self.cd[self.max].unused() {
                self.max -= 1;
            }
            while self.free as usize > self.max {
                self.free = self.cd[self.free as usize].sub;
            }
            if self.free > 0 {
                let mut pco = self.free as usize;
                let mut nco = self.cd[pco].sub;
                while nco > 0 {
                    if nco as usize > self.max {
                        // Take this one out of the free list.
                        nco = self.cd[nco as usize].sub;
                        self.cd[pco].sub = nco;
                    } else {
                        pco = nco as usize;
                        nco = self.cd[pco].sub;
                    }
                }
            }
        } else {
            self.cd[c].sub = self.free;
            self.free = co;
        }
    }

    /// `pseudocolor`: a color no character has.
    pub(super) fn pseudocolor(&mut self) -> Color {
        let co = self.newcolor();
        if self.budget.is_err() {
            return COLORLESS;
        }
        let cd = self.cd_mut(co);
        cd.nchrs = 1;
        cd.flags = PSEUDO;
        co
    }

    /// `subcolor`: give a character its color's open subcolor.
    pub(super) fn subcolor(&mut self, c: Chr) -> Color {
        let co = self.getcolor(c);
        let sco = self.newsub(co);
        if self.budget.is_err() {
            return COLORLESS;
        }
        if co == sco {
            return co;
        }
        self.cd_mut(co).nchrs -= 1;
        self.cd_mut(sco).nchrs += 1;
        self.setcolor(c, sco);
        sco
    }

    /// `newsub`: a color's open subcolor, made if necessary.
    fn newsub(&mut self, co: Color) -> Color {
        let mut sco = self.cd(co).sub;
        if sco == NOSUB {
            if self.cd(co).nchrs == 1 {
                return co;
            }
            sco = self.newcolor();
            if sco == COLORLESS {
                return COLORLESS;
            }
            self.cd_mut(co).sub = sco;
            self.cd_mut(sco).sub = sco;
        }
        sco
    }

    /// `subrange`: give a range of characters subcolors, with arcs for them.
    pub(super) fn subrange(&mut self, nfa: &mut Nfa, from: Chr, to: Chr, lp: S, rp: S) {
        let mut from = from;
        // First, align "from" on a tree-block boundary.
        let aligned = (from.wrapping_add(BYTTAB as u32 - 1) & !BYTMASK).wrapping_sub(from);
        let mut i = aligned as i64;
        while from <= to && i > 0 {
            let co = self.subcolor(from);
            nfa.newarc(PLAIN, co, lp, rp);
            i -= 1;
            from += 1;
        }
        if from > to {
            return;
        }
        // Whole blocks.
        while to - from >= BYTTAB as u32 {
            self.subblock(nfa, from, lp, rp);
            from += BYTTAB as u32;
        }
        // Any remaining partial block.
        while from <= to {
            let co = self.subcolor(from);
            nfa.newarc(PLAIN, co, lp, rp);
            from += 1;
        }
    }

    /// `subblock`: subcolors for one tree block of characters.
    fn subblock(&mut self, nfa: &mut Nfa, start: Chr, lp: S, rp: S) {
        let mut t = 0;
        let mut lastt = 0;
        let mut b = 0;
        let mut level = 0;
        let mut shift = BYTBITS * (NBYTS - 1);
        while shift > 0 {
            b = ((start >> shift) & BYTMASK) as usize;
            lastt = t;
            let child = self.ptrs[lastt][b] as usize;
            if shift > BYTBITS && child == level + 1 {
                let copy = self.ptrs[child].clone();
                self.ptrs.push(copy);
                t = self.ptrs.len() - 1;
                self.ptrs[lastt][b] = t as u32;
            } else {
                t = child;
            }
            level += 1;
            shift -= BYTBITS;
        }
        // Special cases: fill block or solid block.
        let co = Color::from(self.colors[t][0]);
        if t == 0 || self.cd(co).block == Some(t) {
            let sco = self.newsub(co);
            if sco == COLORLESS {
                return;
            }
            let solid = match self.cd(sco).block {
                Some(solid) => solid,
                None => {
                    self.colors.push(Box::new([sco as i16; BYTTAB]));
                    let solid = self.colors.len() - 1;
                    self.cd_mut(sco).block = Some(solid);
                    solid
                }
            };
            self.ptrs[lastt][b] = solid as u32;
            nfa.newarc(PLAIN, sco, lp, rp);
            self.cd_mut(co).nchrs -= BYTTAB as u32;
            self.cd_mut(sco).nchrs += BYTTAB as u32;
            return;
        }
        // General case, a mixed block to be altered.
        let mut i = 0;
        while i < BYTTAB {
            let co = Color::from(self.colors[t][i]);
            let sco = self.newsub(co);
            if sco == COLORLESS {
                return;
            }
            nfa.newarc(PLAIN, sco, lp, rp);
            let previ = i;
            loop {
                self.colors[t][i] = sco as i16;
                i += 1;
                if !(i < BYTTAB && Color::from(self.colors[t][i]) == co) {
                    break;
                }
            }
            let ndone = (i - previ) as u32;
            self.cd_mut(co).nchrs -= ndone;
            self.cd_mut(sco).nchrs += ndone;
        }
    }

    /// `okcolors`: promote subcolors to full colors.
    pub(super) fn okcolors(&mut self, nfa: &mut Nfa) {
        let end = self.max + 1;
        for c in 0..end {
            let co = c as Color;
            let sco = self.cd[c].sub;
            if self.cd[c].unused() || sco == NOSUB || sco == co {
                // No subcolor, or a subcolor its parent deals with.
                continue;
            }
            self.cd[c].sub = NOSUB;
            self.cd_mut(sco).sub = NOSUB;
            if self.cd[c].nchrs == 0 {
                // The parent is empty: its arcs change color.
                while let Some(a) = nfa.colorchain_head(co) {
                    nfa.recolor(a, sco);
                }
                self.freecolor(co);
            } else {
                // The parent's arcs gain parallel subcolor arcs.
                let mut a = nfa.colorchain_head(co);
                while let Some(arc) = a {
                    let (typ, from, to) = nfa.arc_ends(arc);
                    nfa.newarc(typ, sco, from, to);
                    a = nfa.colorchain_next(arc);
                }
            }
        }
    }

    /// The colors `rainbow` and `colorcomplement` consider: every full
    /// color in use, with whether it is an open subcolor.
    pub(super) fn colors(&self) -> impl Iterator<Item = (Color, &ColorDesc)> {
        self.cd[..=self.max]
            .iter()
            .enumerate()
            .map(|(co, cd)| (co as Color, cd))
    }
}
