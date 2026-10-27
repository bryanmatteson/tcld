/*
 * Ported from Tcl 9.0.2's generic/tclCmdMZ.c (TclNRSwitchObjCmd).
 * Copyright © 1987-1993 The Regents of the University of California.
 * Copyright © 1994-1997 Sun Microsystems, Inc.
 * Copyright © 1998-2000 Scriptics Corporation.
 * Copyright © 2002 ActiveState Corporation.
 * Copyright © 2003-2009 Donal K. Fellows.
 * See ../regex/license.terms for usage, redistribution and warranty terms.
 */

//! Native matching and captures; only the selected arm becomes mapped source.
use super::*;
use crate::args::{lookup, tcl_error, wrong_args};
use crate::regex::{REG_ADVANCED, REG_ICASE};
use crate::regexp_command::{Subject, compiled};

impl HybridEngineInner {
    pub(super) fn invoke_switch(
        &mut self,
        words: &[VmObj],
        site: CodeAddress,
    ) -> Result<InvokeOutcome, VmError> {
        const OPTIONS: [&str; 7] = [
            "-exact",
            "-glob",
            "-indexvar",
            "-matchvar",
            "-nocase",
            "-regexp",
            "--",
        ];
        let invoked = words[0].as_string()?;
        let mut mode = "-exact";
        let mut found_mode = false;
        let mut nocase = false;
        let (mut index_var, mut match_var) = (None, None);
        let mut i = 1;
        // Tcl leaves at least a string and one arm argument after options.
        // In particular a dash-prefixed string can be an ordinary argument.
        while i + 2 < words.len() {
            let option = words[i].as_string()?;
            if !option.starts_with('-') {
                break;
            }
            let option_name = lookup(&OPTIONS, "option", &option)?;
            i += 1;
            match option_name {
                "--" => break,
                "-nocase" => nocase = true,
                "-indexvar" | "-matchvar" => {
                    if i + 2 >= words.len() {
                        return Err(tcl_error(
                            format!("missing variable name argument to {option_name} option"),
                            &["TCL", "OPERATION", "SWITCH", "NOVAR"],
                        ));
                    }
                    if option_name == "-indexvar" {
                        index_var = Some(&words[i]);
                    } else {
                        match_var = Some(&words[i]);
                    }
                    i += 1;
                }
                _ => {
                    if found_mode {
                        return Err(tcl_error(
                            format!("bad option \"{option}\": {mode} option already found"),
                            &["TCL", "OPERATION", "SWITCH", "DOUBLEOPT"],
                        ));
                    }
                    found_mode = true;
                    mode = option_name;
                }
            }
        }
        if words.len().saturating_sub(i) < 2 {
            return Err(wrong_args(
                &[&invoked],
                "?-option ...? string ?pattern body ...? ?default body?",
            ));
        }
        for (name, variable) in [("-indexvar", index_var), ("-matchvar", match_var)] {
            if variable.is_some() && mode != "-regexp" {
                return Err(tcl_error(
                    format!("{name} option requires -regexp option"),
                    &["TCL", "OPERATION", "SWITCH", "MODERESTRICTION"],
                ));
            }
        }
        let string = words[i].as_string()?;
        let subject = Subject::new(&string);
        let split = words.len() == i + 2;
        let arms = if split {
            let arms = words[i + 1].as_list_vec()?;
            if arms.is_empty() {
                return Err(wrong_args(
                    &[&invoked],
                    "?-option ...? string {?pattern body ...? ?default body?}",
                ));
            }
            arms
        } else {
            words[i + 1..].to_vec()
        };
        if !arms.len().is_multiple_of(2) {
            let comment = split
                && arms
                    .iter()
                    .step_by(2)
                    .any(|pattern| pattern.as_string().is_ok_and(|text| text.starts_with('#')));
            let mut code = vec!["TCL", "OPERATION", "SWITCH", "BADARM"];
            let mut message = "extra switch pattern with no body".to_owned();
            if comment {
                message.push_str(", this may be due to a comment incorrectly placed outside of a switch body - see the \"switch\" documentation");
                code.push("COMMENT?");
            }
            return Err(tcl_error(message, &code));
        }
        if arms.last().unwrap().as_string()? == "-" {
            return Err(tcl_error(
                format!(
                    "no body specified for pattern \"{}\"",
                    arms[arms.len() - 2].as_string()?
                ),
                &["TCL", "OPERATION", "SWITCH", "BADARM", "FALLTHROUGH"],
            ));
        }
        for i in (0..arms.len()).step_by(2) {
            let pattern = arms[i].as_string()?;
            let default = i + 2 == arms.len() && pattern == "default";
            let mut indices = VmObj::empty();
            let mut matches = VmObj::empty();
            let matched = if default {
                true
            } else {
                match mode {
                    "-exact" if nocase => string
                        .chars()
                        .map(crate::unicode::to_lower)
                        .eq(pattern.chars().map(crate::unicode::to_lower)),
                    "-exact" => string == pattern,
                    "-glob" => glob_match(&pattern, &string, nocase),
                    "-regexp" => {
                        let regexp =
                            compiled(&pattern, REG_ADVANCED | if nocase { REG_ICASE } else { 0 })?;
                        let save = index_var.is_some() || match_var.is_some();
                        match regexp.exec(&subject, 0, if save { None } else { Some(0) }, 0)? {
                            None => false,
                            Some(captures) => {
                                if save {
                                    let mut ranges = Vec::new();
                                    let mut texts = Vec::new();
                                    for capture in captures {
                                        // Tcl reports an empty match at the start as
                                        // {-1 -1}, as well as unmatched groups.
                                        if let Some((start, end)) =
                                            capture.filter(|(_, end)| *end > 0)
                                        {
                                            ranges.push(VmObj::List(vec![
                                                VmObj::Int(start as i64),
                                                VmObj::Int(end as i64 - 1),
                                            ]));
                                            texts.push(VmObj::from_str(
                                                &subject.chars()[start..end]
                                                    .iter()
                                                    .filter_map(|c| char::from_u32(*c))
                                                    .collect::<String>(),
                                            ));
                                        } else {
                                            ranges.push(VmObj::from_str("-1 -1"));
                                            texts.push(VmObj::empty());
                                        }
                                    }
                                    indices = VmObj::List(ranges);
                                    matches = VmObj::List(texts);
                                }
                                true
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            };
            if !matched {
                continue;
            }
            // Capture stores precede body admission, in this order. A trace
            // can mutate the variables, fail, or suspend a hybrid coroutine.
            if default || mode == "-regexp" {
                for (name, value) in [(index_var, indices), (match_var, matches)] {
                    if let Some(name) = name {
                        let (base, element) = split_var_name(&name.ensure_bytes()?);
                        self.var_set(&base, element.as_ref(), value)?;
                    }
                }
            }
            let mut body = i + 1;
            while arms[body].as_string()? == "-" {
                body += 2;
            }
            let namespace = self.frames[self.fi()].namespace.clone();
            let scope = self.frames[self.scope_index()].id;
            self.enter_generated(
                "switch-body",
                arms[body].as_string()?,
                site,
                namespace,
                Some(scope),
                FrameKind::Dynamic,
                None,
            )?;
            let mut end = pattern.len().min(50);
            while !pattern.is_char_boundary(end) {
                end -= 1;
            }
            self.frames.last_mut().unwrap().body_error_context = Some(format!(
                "\"{}{}\" arm",
                &pattern[..end],
                if pattern.len() > end { "..." } else { "" },
            ));
            return Ok(InvokeOutcome::FramePushed);
        }
        Ok(InvokeOutcome::Value(VmObj::empty()))
    }
}
