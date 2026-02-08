use crate::code::{Code, CodeRef, code_ref};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::Range;

#[derive(Clone)]
pub struct CodeGraph {
    code_map: BTreeMap<i32, CodeRef>,
    source_map: BTreeMap<i32, BTreeSet<i32>>,
}

impl CodeGraph {
    pub fn new(instructions: Vec<crate::bytecode::Instruction>) -> Self {
        let mut code_map: BTreeMap<i32, CodeRef> = BTreeMap::new();
        for inst in instructions {
            let loc = inst.location;
            code_map.insert(loc, code_ref(Code::Instruction(inst)));
        }

        let mut source_map: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
        if let Some((&first, _)) = code_map.iter().next() {
            source_map.insert(first, BTreeSet::new());
        }

        for (loc, code) in &code_map {
            code.borrow().for_each_next_location(&mut |dest| {
                let entry = source_map.entry(dest).or_default();
                entry.insert(*loc);
            });
        }

        CodeGraph {
            code_map,
            source_map,
        }
    }

    fn connect(&mut self, source: i32, destination: i32) {
        let entry = self.source_map.entry(destination).or_default();
        entry.insert(source);
    }

    pub fn sources_of(&self, code: &CodeRef) -> Vec<CodeRef> {
        let loc = code.borrow().location();
        self.source_map
            .get(&loc)
            .map(|locs| {
                locs.iter()
                    .filter_map(|l| self.code_map.get(l).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn single_source_of(&self, code: &CodeRef) -> Option<CodeRef> {
        let loc = code.borrow().location();
        let locs = self.source_map.get(&loc)?;
        let mut only: Option<CodeRef> = None;
        for src_loc in locs {
            let Some(src_ref) = self.code_map.get(src_loc).cloned() else {
                continue;
            };
            if only.is_some() {
                return None;
            }
            only = Some(src_ref);
        }
        only
    }

    pub fn code_before(&self, location: i32) -> Option<CodeRef> {
        self.code_map
            .range(..location)
            .rev()
            .find(|(_, code)| code.borrow().location() + code.borrow().size() == location)
            .map(|(_, code)| code.clone())
    }

    pub fn code_range(&self) -> Range<i32> {
        let Some(first) = self.code_map.iter().next() else {
            return 0..0;
        };
        let Some(last) = self.code_map.iter().next_back() else {
            return 0..0;
        };
        *first.0..(last.0 + last.1.borrow().size())
    }

    pub fn code_in_range(&self, range: Range<i32>) -> Vec<CodeRef> {
        if range.start >= range.end {
            return Vec::new();
        }
        self.code_map
            .range(range)
            .map(|(_, code)| code.clone())
            .collect()
    }

    pub fn code_count_in_range(&self, range: Range<i32>) -> usize {
        if range.start >= range.end {
            return 0;
        }
        self.code_map.range(range).count()
    }

    pub fn code_at_location(&self, location: i32) -> Option<CodeRef> {
        self.code_map.get(&location).cloned()
    }

    pub fn replace_code(&mut self, code: CodeRef) {
        let (location, range, next_inline, next_len, next_overflow) = {
            let c = code.borrow();
            let mut inline = [0i32; 2];
            let mut len = 0usize;
            let mut overflow = Vec::new();
            c.for_each_next_location(&mut |dest| {
                if len < inline.len() {
                    inline[len] = dest;
                    len += 1;
                } else {
                    overflow.push(dest);
                }
            });
            (c.location(), c.range(), inline, len, overflow)
        };
        let sources = self.source_map.get(&location).cloned();

        let mut cursor = range.start;
        while let Some((&loc, old_code)) = self.code_map.range(cursor..range.end).next() {
            let mut old_next_inline = [0i32; 2];
            let mut old_next_len = 0usize;
            let mut old_next_overflow = Vec::new();
            old_code.borrow().for_each_next_location(&mut |dest| {
                if old_next_len < old_next_inline.len() {
                    old_next_inline[old_next_len] = dest;
                    old_next_len += 1;
                } else {
                    old_next_overflow.push(dest);
                }
            });
            for dest in old_next_inline.iter().take(old_next_len) {
                let mut remove_entry = false;
                if let Some(incoming) = self.source_map.get_mut(dest) {
                    incoming.remove(&loc);
                    remove_entry = incoming.is_empty();
                }
                if remove_entry {
                    self.source_map.remove(dest);
                }
            }
            for dest in old_next_overflow {
                let mut remove_entry = false;
                if let Some(incoming) = self.source_map.get_mut(&dest) {
                    incoming.remove(&loc);
                    remove_entry = incoming.is_empty();
                }
                if remove_entry {
                    self.source_map.remove(&dest);
                }
            }

            self.code_map.remove(&loc);
            if loc != location {
                self.source_map.remove(&loc);
            }
            cursor = loc.saturating_add(1);
        }

        for dest in next_inline.iter().take(next_len).copied() {
            self.connect(location, dest);
        }
        for dest in next_overflow {
            self.connect(location, dest);
        }

        self.code_map.insert(location, code);
        match sources {
            Some(s) => {
                self.source_map.insert(location, s);
            }
            None => {
                self.source_map.remove(&location);
            }
        }
    }
}
