/*
 * tclCmdMZ.c --
 *
 *	This file contains the top-level command routines for most of the Tcl
 *	built-in commands whose names begin with the letters M to Z. It
 *	contains only commands in the generic core (i.e. those that don't
 *	depend much upon UNIX facilities).
 *
 * Copyright © 1987-1993 The Regents of the University of California.
 * Copyright © 1994-1997 Sun Microsystems, Inc.
 * Copyright © 1998-2000 Scriptics Corporation.
 * Copyright © 2002 ActiveState Corporation.
 * Copyright © 2003-2009 Donal K. Fellows.
 *
 * See the file "license.terms" for information on usage and redistribution of
 * this file, and for a DISCLAIMER OF ALL WARRANTIES.
 */

/*
 * tclRegexp.c --
 *
 *	This file contains the public interfaces to the Tcl regular expression
 *	mechanism.
 *
 * Copyright © 1998 Sun Microsystems, Inc.
 * Copyright © 1998-1999 Scriptics Corporation.
 *
 * See the file "license.terms" for information on usage and redistribution of
 * this file, and for a DISCLAIMER OF ALL WARRANTIES.
 */

// Ported from Tcl 9.0.2 for Tcld's native VM. The command layer uses
// Rust-owned values and variables; see regex/license.terms.

//! `regexp` and `regsub` (`Tcl_RegexpObjCmd` and `Tcl_RegsubObjCmd` in
//! `tclCmdMZ.c`) over the native engine, with the layer `tclRegexp.c` puts
//! on it: the cache of compiled expressions, compile and match errors,
//! `-about`, and the glob shortcut for expressions `TclReToGlob` can turn
//! into a glob pattern.

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use crate::args::{exact_lookup, tcl_error, wrong_args};
use crate::builtin::VarNameCtx;
use crate::completion::Completion;
use crate::error::VmError;
use crate::regex::{
    Chr, INFO_NAMES, REG_ADVANCED, REG_EXPANDED, REG_ICASE, REG_NEWLINE, REG_NLANCH, REG_NLSTOP,
    REG_NOSUB, REG_NOTBOL, Regex, RegexError, Submatch,
};
use crate::strings;
use crate::value::{VmObj, glob_match};
use crate::var::split_var_name;

/// How many compiled expressions a thread keeps (`NUM_REGEXPS`).
const NUM_REGEXPS: usize = 30;

/// A compiled expression with the flags it was compiled with and its glob
/// equivalent, if it has one (`struct TclRegexp`).
pub struct TclRegexp {
    flags: i32,
    re: Regex,
    glob: Option<String>,
}

thread_local! {
    /// The most recently used expressions first, as `CompileRegexp` keeps
    /// them.
    static CACHE: RefCell<Vec<(String, Rc<TclRegexp>)>> = const { RefCell::new(Vec::new()) };
}

/// A string to match, with its characters found once.
pub struct Subject<'s> {
    text: &'s str,
    chars: OnceCell<Vec<Chr>>,
}

impl<'s> Subject<'s> {
    pub fn new(text: &'s str) -> Self {
        Subject {
            text,
            chars: OnceCell::new(),
        }
    }

    pub fn chars(&self) -> &[Chr] {
        self.chars
            .get_or_init(|| self.text.chars().map(Chr::from).collect())
    }
}

/// `TclRegError`: Tcl's message and `REGEXP` error code for an engine
/// failure.
fn reg_error(prefix: &str, error: RegexError) -> VmError {
    let explain = error.explain();
    tcl_error(
        format!("{prefix}{explain}"),
        &["REGEXP", &error.name(), &explain],
    )
}

/// `Tcl_GetRegExpFromObj`: the expression `pattern` compiles to with
/// `flags`, from the cache when it is there.
pub fn compiled(pattern: &str, flags: i32) -> Result<Rc<TclRegexp>, VmError> {
    let cached = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let at = cache
            .iter()
            .position(|(text, re)| re.flags == flags && text == pattern)?;
        let entry = cache.remove(at);
        let re = entry.1.clone();
        cache.insert(0, entry);
        Some(re)
    });
    if let Some(re) = cached {
        return Ok(re);
    }
    let chars: Vec<Chr> = pattern.chars().map(Chr::from).collect();
    let re = Regex::compile(&chars, flags)
        .map_err(|error| reg_error("cannot compile regular expression pattern: ", error))?;
    let compiled = Rc::new(TclRegexp {
        flags,
        re,
        glob: re_to_glob(pattern),
    });
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.insert(0, (pattern.to_string(), compiled.clone()));
        cache.truncate(NUM_REGEXPS);
    });
    Ok(compiled)
}

impl TclRegexp {
    pub fn nsub(&self) -> usize {
        self.re.nsub()
    }

    /// `Tcl_RegExpExecObj`: match from character `offset`, keeping
    /// `nmatches` submatches (`None` for all of them), whose positions are
    /// relative to `offset`. A match needing no submatches may take the
    /// glob shortcut.
    pub fn exec(
        &self,
        subject: &Subject,
        offset: usize,
        nmatches: Option<usize>,
        eflags: i32,
    ) -> Result<Option<Vec<Submatch>>, VmError> {
        const GLOB_OK: i32 = REG_ADVANCED | REG_NOSUB | REG_ICASE;
        if offset == 0
            && nmatches == Some(0)
            && eflags == 0
            && self.flags & !GLOB_OK == 0
            && let Some(glob) = &self.glob
        {
            let nocase = self.flags & REG_ICASE != 0;
            return Ok(glob_match(glob, subject.text, nocase).then(Vec::new));
        }
        let chars = subject.chars();
        let offset = offset.min(chars.len());
        let last = self.re.nsub() + 1;
        let nmatch = nmatches.map_or(last, |n| n.min(last));
        self.re
            .exec(&chars[offset..], nmatch, eflags)
            .map_err(|error| reg_error("error while matching regular expression: ", error))
    }

    /// `TclRegAbout`: the subexpression count and the facts noted.
    pub fn about(&self) -> VmObj {
        let info = self.re.info();
        let names = INFO_NAMES
            .iter()
            .filter(|(bit, _)| info & bit != 0)
            .map(|(_, name)| VmObj::from_str(name))
            .collect();
        VmObj::List(vec![VmObj::Int(self.nsub() as i64), VmObj::List(names)])
    }
}

/// `TclReToGlob`: the glob pattern equivalent to `re`, when it is a plain
/// string with only `.`, `.*`, `.+`, escapes and anchors.
pub fn re_to_glob(re: &str) -> Option<String> {
    let bytes = re.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + 2);
    // ***=xxx is *xxx*, with glob-sensitive characters escaped.
    if let Some(literal) = bytes.strip_prefix(b"***=") {
        out.push(b'*');
        for &b in literal {
            if matches!(b, b'\\' | b'*' | b'[' | b']' | b'?') {
                out.push(b'\\');
            }
            out.push(b);
        }
        out.push(b'*');
        return String::from_utf8(out).ok();
    }
    let mut p = 0;
    let mut anchor_right = false;
    let mut last_is_star;
    let mut num_stars = 0;
    if bytes.first() == Some(&b'^') {
        p += 1;
        last_is_star = false;
    } else {
        out.push(b'*');
        last_is_star = true;
    }
    while p < bytes.len() {
        match bytes[p] {
            b'\\' => {
                p += 1;
                match bytes.get(p).copied() {
                    Some(b'a') => out.push(0x07),
                    Some(b'b') => out.push(0x08),
                    Some(b'f') => out.push(0x0C),
                    Some(b'n') => out.push(b'\n'),
                    Some(b'r') => out.push(b'\r'),
                    Some(b't') => out.push(b'\t'),
                    Some(b'v') => out.push(0x0B),
                    Some(b'B' | b'\\') => out.extend_from_slice(b"\\\\"),
                    Some(c @ (b'*' | b'[' | b']' | b'?')) => {
                        out.push(b'\\');
                        out.push(c);
                    }
                    Some(c @ (b'{' | b'}' | b'(' | b')' | b'+' | b'.' | b'|' | b'^' | b'$')) => {
                        out.push(c)
                    }
                    // An invalid escape sequence.
                    _ => return None,
                }
            }
            b'.' => match bytes.get(p + 1) {
                Some(b'*') => {
                    p += 2;
                    if !last_is_star {
                        out.push(b'*');
                        last_is_star = true;
                        num_stars += 1;
                    }
                    continue;
                }
                Some(b'+') => {
                    p += 2;
                    out.extend_from_slice(b"?*");
                    last_is_star = true;
                    num_stars += 1;
                    continue;
                }
                _ => out.push(b'?'),
            },
            b'$' => {
                if p + 1 != bytes.len() {
                    // $ not an anchor.
                    return None;
                }
                anchor_right = true;
            }
            // An RE special character a glob cannot express.
            b'*' | b'+' | b'?' | b'|' | b'^' | b'{' | b'}' | b'(' | b')' | b'[' | b']' => {
                return None;
            }
            b => out.push(b),
        }
        last_is_star = false;
        p += 1;
    }
    if num_stars > 1 {
        // Excessive recursive glob backtrack potential.
        return None;
    }
    if !anchor_right && !last_is_star {
        out.push(b'*');
    }
    String::from_utf8(out).ok()
}

/// `Tcl_GetRange`: characters `first` to `last`, empty when `last` comes
/// first.
fn range(chars: &[Chr], first: i64, last: i64) -> VmObj {
    let first = first.max(0);
    let last = last.min(chars.len() as i64 - 1);
    if last < first {
        return VmObj::empty();
    }
    VmObj::from_str(&text(&chars[first as usize..=last as usize]))
}

fn text(chars: &[Chr]) -> String {
    chars.iter().filter_map(|&c| char::from_u32(c)).collect()
}

fn set_variable(vars: &mut dyn VarNameCtx, name: &VmObj, value: VmObj) -> Result<(), VmError> {
    let name = name.ensure_bytes()?;
    let (name1, name2) = split_var_name(&name);
    vars.set_var(&name1, name2.as_ref(), value)
}

/// Validate a `-start` value the way the option loop does
/// (`TclGetIntForIndexM` against `TCL_SIZE_MAX - 1`).
fn check_start(value: &VmObj) -> Result<(), VmError> {
    strings::index(value, i64::MAX - 1).map(|_| ())
}

/// `-start` against a string of `length` characters, never negative.
fn start_offset(value: Option<&VmObj>, length: usize) -> Result<usize, VmError> {
    let Some(value) = value else {
        return Ok(0);
    };
    let offset = strings::index(value, length as i64)?;
    Ok(usize::try_from(offset).unwrap_or(0))
}

/// Run `regexp ?-option ...? exp string ?matchVar? ?subMatchVar ...?`;
/// `words` include the command word.
pub fn regexp(words: &[VmObj], vars: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    const OPTIONS: [&str; 11] = [
        "-all",
        "-about",
        "-indices",
        "-inline",
        "-expanded",
        "-line",
        "-linestop",
        "-lineanchor",
        "-nocase",
        "-start",
        "--",
    ];
    let (mut indices, mut about, mut doinline) = (false, false, false);
    let mut all = 0;
    let mut cflags = REG_ADVANCED;
    let mut start_index = None;
    let mut i = 1;
    while i < words.len() {
        let name = words[i].as_string()?;
        if !name.starts_with('-') {
            break;
        }
        match exact_lookup(&OPTIONS, "option", &name)? {
            "-all" => all = 1,
            "-indices" => indices = true,
            "-inline" => doinline = true,
            "-nocase" => cflags |= REG_ICASE,
            "-about" => about = true,
            "-expanded" => cflags |= REG_EXPANDED,
            "-line" => cflags |= REG_NEWLINE,
            "-linestop" => cflags |= REG_NLSTOP,
            "-lineanchor" => cflags |= REG_NLANCH,
            "-start" => {
                i += 1;
                if i >= words.len() {
                    break;
                }
                check_start(&words[i])?;
                start_index = Some(&words[i]);
            }
            _ => {
                i += 1;
                break;
            }
        }
        i += 1;
    }
    let args = &words[i.min(words.len())..];
    if args.len() < 2 - usize::from(about) {
        return Err(wrong_args(
            &[&words[0].as_string()?],
            "?-option ...? exp string ?matchVar? ?subMatchVar ...?",
        ));
    }
    if doinline && args.len() != 2 {
        return Err(tcl_error(
            "regexp match variables not allowed when using -inline",
            &["TCL", "OPERATION", "REGEXP", "MIX_VAR_INLINE"],
        ));
    }
    if about {
        return Ok(compiled(&args[0].as_string()?, cflags)?.about());
    }
    // The string's length first, as Tcl gets it before the expression.
    let string = args[1].as_string()?;
    let subject = Subject::new(&string);
    let length = subject.chars().len();
    let mut offset = start_offset(start_index, length)?;
    let re = compiled(&args[0].as_string()?, cflags)?;
    let var_names = &args[2..];
    // Keep only the submatches wanted, but with -all at least the match's
    // own, to know where to go on.
    let saved = if doinline {
        None
    } else if var_names.is_empty() {
        Some(all)
    } else {
        Some(var_names.len())
    };
    let mut result = Vec::new();
    loop {
        // The character at the offset begins a line only after a newline.
        let eflags = if offset == 0 {
            0
        } else if offset > length {
            REG_NOTBOL
        } else if subject.chars()[offset - 1] == Chr::from(b'\n') {
            0
        } else {
            REG_NOTBOL
        };
        let Some(matches) = re.exec(&subject, offset, saved, eflags)? else {
            if all <= 1 {
                return Ok(if doinline {
                    VmObj::empty()
                } else {
                    VmObj::Int(0)
                });
            }
            break;
        };
        let nsubs = re.nsub();
        let count = if doinline { nsubs + 1 } else { var_names.len() };
        if doinline && all <= 1 {
            result.clear();
        }
        // Inline results have no output variable names; variable results
        // pair each requested submatch with the variable receiving it.
        let outputs = var_names
            .iter()
            .map(Some)
            .chain(std::iter::repeat(None))
            .take(count);
        for (i, name) in outputs.enumerate() {
            let found = if i <= nsubs {
                matches.get(i).copied().flatten()
            } else {
                None
            };
            let value = if indices {
                let (start, end) = match found {
                    Some((start, end)) => ((offset + start) as i64, (offset + end) as i64 - 1),
                    None => (-1, -1),
                };
                VmObj::List(vec![VmObj::Int(start), VmObj::Int(end)])
            } else {
                match found {
                    Some((start, end)) if end > 0 => range(
                        subject.chars(),
                        (offset + start) as i64,
                        (offset + end) as i64 - 1,
                    ),
                    _ => VmObj::empty(),
                }
            };
            if let Some(name) = name {
                set_variable(vars, name, value)?;
            } else {
                result.push(value);
            }
        }
        if all == 0 {
            break;
        }
        // Go on after the match, by at least one character so that an
        // empty match does not repeat.
        let (start, end) = matches.first().copied().flatten().unwrap_or((0, 0));
        offset += end;
        if end == start {
            offset += 1;
        }
        all += 1;
        if offset >= length {
            break;
        }
    }
    Ok(if doinline {
        VmObj::List(result)
    } else {
        VmObj::Int(if all > 0 { all as i64 - 1 } else { 1 })
    })
}

/// What `regsub` came to.
pub enum Regsub {
    Value(VmObj),
    /// The substitutions come from `-command`, a command prefix to call.
    Command(CommandSubstitution),
}

/// Owned match inputs survive nested regular expressions, variable edits,
/// and debugger suspension while a replacement command executes.
pub struct CommandSubstitution {
    re: Rc<TclRegexp>,
    original: VmObj,
    prefix: Vec<VmObj>,
    offset: usize,
    all: bool,
    variable: Option<VmObj>,
}

impl CommandSubstitution {
    /// A non-OK callback completion escapes unchanged. In particular, an
    /// output variable is never assigned a partially substituted string.
    pub fn substitute(
        &self,
        mut callback: impl FnMut(&[VmObj]) -> Result<Result<VmObj, Completion>, VmError>,
    ) -> Result<Result<(usize, VmObj), Completion>, VmError> {
        let string = self.original.as_string()?;
        let subject = Subject::new(&string);
        let chars = subject.chars();
        let mut offset = self.offset;
        let mut count = 0;
        let mut out = String::new();
        while offset <= chars.len() {
            let eflags = if offset > 0 && chars[offset - 1] != Chr::from(b'\n') {
                REG_NOTBOL
            } else {
                0
            };
            // A command receives every capture, rather than just the ten
            // matches needed by replacement text's & and \0 through \9.
            let Some(info) = self.re.exec(&subject, offset, None, eflags)? else {
                break;
            };
            if count == 0 {
                out.push_str(&text(&chars[..offset]));
            }
            let (start, end) = info.first().copied().flatten().unwrap_or((0, 0));
            out.push_str(&text(&chars[offset..offset + start]));
            let mut words = self.prefix.clone();
            words.extend((0..=self.re.nsub()).map(|index| match info.get(index) {
                Some(Some((start, end))) => {
                    VmObj::from_str(&text(&chars[offset + start..offset + end]))
                }
                _ => VmObj::empty(),
            }));
            match callback(&words)? {
                Ok(value) => out.push_str(&value.as_string()?),
                Err(completion) => return Ok(Err(completion)),
            }
            count += 1;
            offset += end;
            if end == 0 || start == end {
                if offset < chars.len() {
                    out.extend(char::from_u32(chars[offset]));
                }
                offset += 1;
            }
            if !self.all {
                break;
            }
        }
        if count == 0 {
            return Ok(Ok((0, self.original.clone())));
        }
        if offset < chars.len() {
            out.push_str(&text(&chars[offset..]));
        }
        Ok(Ok((count, VmObj::from_str(&out))))
    }

    pub fn finish(
        &self,
        count: usize,
        value: VmObj,
        vars: &mut dyn VarNameCtx,
    ) -> Result<VmObj, VmError> {
        match &self.variable {
            Some(name) => {
                set_variable(vars, name, value)?;
                Ok(VmObj::Int(count as i64))
            }
            None => Ok(value),
        }
    }
}

/// Run `regsub ?-option ...? exp string subSpec ?varName?`; `words`
/// include the command word.
pub fn regsub(words: &[VmObj], vars: &mut dyn VarNameCtx) -> Result<Regsub, VmError> {
    const OPTIONS: [&str; 9] = [
        "-all",
        "-command",
        "-expanded",
        "-line",
        "-linestop",
        "-lineanchor",
        "-nocase",
        "-start",
        "--",
    ];
    let (mut all, mut command) = (false, false);
    let mut cflags = REG_ADVANCED;
    let mut start_index = None;
    let mut idx = 1;
    while idx < words.len() {
        let name = words[idx].as_string()?;
        if !name.starts_with('-') {
            break;
        }
        match exact_lookup(&OPTIONS, "option", &name)? {
            "-all" => all = true,
            "-nocase" => cflags |= REG_ICASE,
            "-command" => command = true,
            "-expanded" => cflags |= REG_EXPANDED,
            "-line" => cflags |= REG_NEWLINE,
            "-linestop" => cflags |= REG_NLSTOP,
            "-lineanchor" => cflags |= REG_NLANCH,
            "-start" => {
                idx += 1;
                if idx >= words.len() {
                    break;
                }
                check_start(&words[idx])?;
                start_index = Some(&words[idx]);
            }
            _ => {
                idx += 1;
                break;
            }
        }
        idx += 1;
    }
    if words.len() < idx + 3 || words.len() > idx + 4 {
        return Err(wrong_args(
            &[&words[0].as_string()?],
            "?-option ...? exp string subSpec ?varName?",
        ));
    }
    let args = &words[idx..];
    let string = args[1].as_string()?;
    let subject = Subject::new(&string);
    let offset = match start_index {
        Some(_) => start_offset(start_index, subject.chars().len())?,
        None => 0,
    };
    let pattern = args[0].as_string()?;
    let subspec = args[2].as_string()?;
    let (matches, result) = if all
        && offset == 0
        && !command
        && !subspec.contains(['&', '\\'])
        && !pattern.contains(|c| "*+?{}()[].\\|^$".contains(c))
    {
        // A simple one-pair string map.
        literal_substitute(&pattern, subject.chars(), &subspec, cflags & REG_ICASE != 0)
    } else {
        let re = compiled(&pattern, cflags)?;
        if command {
            // The substitutions come from a command prefix, which must be a
            // list of at least one word.
            let prefix = args[2].as_list_vec()?;
            if prefix.is_empty() {
                return Err(tcl_error(
                    "command prefix must be a list of at least one element",
                    &["TCL", "OPERATION", "REGSUB", "CMDEMPTY"],
                ));
            }
            return Ok(Regsub::Command(CommandSubstitution {
                re,
                original: args[1].clone(),
                prefix,
                offset,
                all,
                variable: args.get(3).cloned(),
            }));
        }
        substitute(&re, &subject, offset, &subspec, all)?
    };
    // With no match the string comes back as it was.
    let result = match result {
        Some(result) if matches > 0 => VmObj::from_str(&result),
        _ => args[1].clone(),
    };
    Ok(Regsub::Value(match args.get(3) {
        Some(name) => {
            set_variable(vars, name, result)?;
            VmObj::Int(matches as i64)
        }
        None => result,
    }))
}

/// The `regsub -all` shortcut for a pattern without metacharacters and a
/// substitution without `&` or `\`: the number of matches and the result.
fn literal_substitute(
    pattern: &str,
    string: &[Chr],
    subspec: &str,
    nocase: bool,
) -> (usize, Option<String>) {
    let pattern: Vec<Chr> = pattern.chars().map(Chr::from).collect();
    let mut matches = 0;
    let mut out = String::new();
    if pattern.is_empty() {
        // An empty pattern matches between every two characters, but not
        // in an empty string.
        for &c in string {
            out.push_str(subspec);
            out.extend(char::from_u32(c));
            matches += 1;
        }
        return (matches, (matches > 0).then_some(out));
    }
    let lower = |c: Chr| char::from_u32(c).map_or(c, |ch| Chr::from(crate::unicode::to_lower(ch)));
    let first = lower(pattern[0]);
    let slen = pattern.len();
    let mut p = 0;
    let mut w = 0;
    while w + slen <= string.len() {
        let c = string[w];
        let starts = c == pattern[0] || (nocase && lower(c) == first);
        let rest = slen == 1
            || string[w..w + slen]
                .iter()
                .zip(&pattern)
                .all(|(&a, &b)| a == b || (nocase && lower(a) == lower(b)));
        if starts && rest {
            out.push_str(&text(&string[p..w]));
            p = w + slen;
            out.push_str(subspec);
            matches += 1;
            w = p;
            continue;
        }
        w += 1;
    }
    if matches > 0 {
        out.push_str(&text(&string[p..]));
    }
    (matches, (matches > 0).then_some(out))
}

/// The general `regsub` loop: the number of matches and the result.
fn substitute(
    re: &TclRegexp,
    subject: &Subject,
    mut offset: usize,
    subspec: &str,
    all: bool,
) -> Result<(usize, Option<String>), VmError> {
    let chars = subject.chars();
    let wlen = chars.len();
    let subspec: Vec<Chr> = subspec.chars().map(Chr::from).collect();
    let mut matches = 0;
    let mut out = String::new();
    while offset <= wlen {
        // Part of a larger string, so ^ does not match there.
        let eflags = if offset > 0 && chars[offset - 1] != Chr::from(b'\n') {
            REG_NOTBOL
        } else {
            0
        };
        let Some(info) = re.exec(subject, offset, Some(10), eflags)? else {
            break;
        };
        if matches == 0 && offset > 0 {
            // The part before the starting offset.
            out.push_str(&text(&chars[..offset]));
        }
        matches += 1;
        let (start, end) = info.first().copied().flatten().unwrap_or((0, 0));
        out.push_str(&text(&chars[offset..offset + start]));
        // The substitution, with & and \0 to \9 for the match and its
        // subexpressions, and \\ and \& for themselves.
        let submatch = |out: &mut String, idx: usize| {
            if idx <= re.nsub()
                && let Some(Some((s, e))) = info.get(idx)
            {
                out.push_str(&text(&chars[offset + s..offset + e]));
            }
        };
        let mut first = 0;
        let mut i = 0;
        while i < subspec.len() {
            let ch = subspec[i];
            let idx = if ch == Chr::from(b'&') {
                0
            } else if ch == Chr::from(b'\\') {
                let next = subspec.get(i + 1).copied().unwrap_or(0);
                if (Chr::from(b'0')..=Chr::from(b'9')).contains(&next) {
                    (next - Chr::from(b'0')) as usize
                } else if next == Chr::from(b'\\') || next == Chr::from(b'&') {
                    out.push_str(&text(&subspec[first..i]));
                    out.extend(char::from_u32(next));
                    first = i + 2;
                    i += 2;
                    continue;
                } else {
                    i += 1;
                    continue;
                }
            } else {
                i += 1;
                continue;
            };
            out.push_str(&text(&subspec[first..i]));
            submatch(&mut out, idx);
            if subspec[i] == Chr::from(b'\\') {
                i += 1;
            }
            first = i + 1;
            i += 1;
        }
        out.push_str(&text(&subspec[first.min(subspec.len())..]));
        // Consume at least one character, so as not to match the empty
        // string at the same place again.
        if end == 0 {
            if offset < wlen {
                out.extend(char::from_u32(chars[offset]));
            }
            offset += 1;
        } else {
            offset += end;
            if start == end {
                if offset < wlen {
                    out.extend(char::from_u32(chars[offset]));
                }
                offset += 1;
            }
        }
        if !all {
            break;
        }
    }
    if matches > 0 && offset < wlen {
        out.push_str(&text(&chars[offset..]));
    }
    Ok((matches, (matches > 0).then_some(out)))
}

/// Match an `lsearch -regexp` pattern (`Tcl_GetRegExpFromObj` without
/// subexpressions first, then with them, for back references).
pub fn search_pattern(pattern: &str, nocase: bool) -> Result<Rc<TclRegexp>, VmError> {
    let flags = REG_ADVANCED | if nocase { REG_ICASE } else { 0 };
    compiled(pattern, flags | REG_NOSUB).or_else(|_| compiled(pattern, flags))
}

/// Whether `text` matches, as `lsearch -regexp` and the `regexp`
/// instruction ask (`Tcl_RegExpExecObj` from the start, no submatches).
pub fn matches(re: &TclRegexp, text: &str) -> Result<bool, VmError> {
    Ok(re.exec(&Subject::new(text), 0, Some(0), 0)?.is_some())
}
