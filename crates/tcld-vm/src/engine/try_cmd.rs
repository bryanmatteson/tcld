/*
 * Ported from Tcl 9.0.2's generic/tclCmdMZ.c (TclNRTryObjCmd,
 * TryPostBody, TryPostHandler, TryPostFinal and During).
 * Copyright © 1987-1993 The Regents of the University of California.
 * Copyright © 1994-1997 Sun Microsystems, Inc.
 * Copyright © 1998-2000 Scriptics Corporation.
 * Copyright © 2002 ActiveState Corporation.
 * Copyright © 2003-2009 Donal K. Fellows.
 * See ../regex/license.terms for usage, redistribution and warranty terms.
 */

//! Try owns a completion across mapped body, handler and cleanup frames. No
//! Rust evaluator recursion or host control command spans those scripts, so
//! breakpoints and coroutine suspension retain the same continuation state.
use super::*;
use crate::args::{lookup, tcl_error, wrong_args};
use crate::dict::Dict;

struct Handler {
    kind: String,
    code: i32,
    prefix: Vec<VmObj>,
    variables: Vec<ByteString>,
    script: VmObj,
    fallthrough: bool,
}

enum Phase {
    Body,
    Handler(String),
    Finally,
}

pub(super) struct TryState {
    invoked: String,
    handlers: Vec<Handler>,
    finally: Option<VmObj>,
    pending: Option<Completion>,
    phase: Phase,
    site: CodeAddress,
    namespace: NamespaceName,
    scope: FrameId,
}

enum Action {
    Script(VmObj),
    Done(Completion),
}

impl HybridEngineInner {
    pub(super) fn invoke_try(
        &mut self,
        words: &[VmObj],
        site: CodeAddress,
    ) -> Result<InvokeOutcome, VmError> {
        let invoked = words[0].as_string()?;
        if words.len() < 2 {
            return Err(wrong_args(
                &[&invoked],
                "body ?handler ...? ?finally script?",
            ));
        }
        let mut state = TryState {
            invoked,
            handlers: Vec::new(),
            finally: None,
            pending: None,
            phase: Phase::Body,
            site,
            namespace: self.frames[self.fi()].namespace.clone(),
            scope: self.frames[self.scope_index()].id,
        };
        let mut index = 2;
        while index < words.len() {
            let keyword = words[index].as_string()?;
            let kind = lookup(&["finally", "on", "trap"], "handler type", &keyword)?;
            if kind == "finally" {
                if index + 2 < words.len() {
                    return Err(tcl_error(
                        "finally clause must be last",
                        &["TCL", "OPERATION", "TRY", "FINALLY", "NONTERMINAL"],
                    ));
                }
                if index + 1 == words.len() {
                    return Err(clause_arity(kind));
                }
                state.finally = Some(words[index + 1].clone());
                break;
            }
            if index + 4 > words.len() {
                return Err(clause_arity(kind));
            }
            let (code, prefix) = if kind == "on" {
                (
                    crate::completion::parse_completion_code(&words[index + 1])?,
                    Vec::new(),
                )
            } else {
                let prefix = words[index + 1].as_list_vec().map_err(|_| {
                    tcl_error(
                        format!(
                            "bad prefix '{}': must be a list",
                            words[index + 1].as_string().unwrap_or_default()
                        ),
                        &["TCL", "OPERATION", "TRY", "TRAP", "EXNFORMAT"],
                    )
                })?;
                (1, prefix)
            };
            let variables = words[index + 2]
                .as_list_vec()?
                .iter()
                .take(2)
                .map(VmObj::ensure_bytes)
                .collect::<Result<Vec<_>, _>>()?;
            let script = words[index + 3].clone();
            let fallthrough = script.ensure_bytes()?.as_bytes() == b"-";
            state.handlers.push(Handler {
                kind: keyword,
                code,
                prefix,
                variables,
                script,
                fallthrough,
            });
            index += 4;
        }
        if state
            .handlers
            .last()
            .is_some_and(|handler| handler.fallthrough)
        {
            return Err(tcl_error(
                "last non-finally clause must not have a body of \"-\"",
                &["TCL", "OPERATION", "TRY", "BADFALLTHROUGH"],
            ));
        }
        match self.schedule_try(state, Action::Script(words[1].clone()), false)? {
            None => Ok(InvokeOutcome::FramePushed),
            Some(completion) => Ok(if completion.code == 0 {
                self.last_completion = completion.clone();
                InvokeOutcome::Value(completion.result)
            } else {
                InvokeOutcome::Raise(completion)
            }),
        }
    }

    /// Schedule the next exact script. Admission failures are completions of
    /// that script and must reach its handler/finally, just like runtime errors.
    fn schedule_try(
        &mut self,
        mut state: TryState,
        mut action: Action,
        replacing: bool,
    ) -> Result<Option<Completion>, VmError> {
        loop {
            let script = match action {
                Action::Done(completion) => return Ok(Some(completion)),
                Action::Script(script) => script,
            };
            let name = match &state.phase {
                Phase::Body => "try-body",
                Phase::Handler(_) => "try-handler",
                Phase::Finally => "try-finally",
            };
            match self.enter_generated(
                name,
                script.as_string()?,
                state.site,
                state.namespace.clone(),
                Some(state.scope),
                FrameKind::Dynamic,
                None,
            ) {
                Ok(_) => {
                    if replacing {
                        let mut next = self.frames.pop().unwrap();
                        self.cleanup_trace_scope();
                        let previous = self.frames.pop().unwrap();
                        next.continuation = previous.continuation;
                        self.frames.push(next);
                    }
                    self.frames.last_mut().unwrap().try_state = Some(state);
                    return Ok(None);
                }
                Err(error) => {
                    action = self.advance_try(&mut state, script_error(error)?, 1)?;
                }
            }
        }
    }

    /// Body-local catch and loop ranges have already handled their own
    /// completions. The try frame is transparent to return levels.
    pub(super) fn resume_try(
        &mut self,
        mut completion: Completion,
        producing_pc: u32,
    ) -> Result<Option<Completion>, VmError> {
        let line = self.mapped_error_line(producing_pc);
        let mut state = self.frames.last_mut().unwrap().try_state.take().unwrap();
        if completion.code == 0 && completion.options.raw.is_none() {
            completion.options = self.last_completion.options.clone();
            completion.options.code = 0;
            completion.options.level = 0;
            if completion.options.raw.is_some() {
                completion.options.sync_raw_code_level()?;
            }
        }
        let action = self.advance_try(&mut state, completion, line)?;
        self.schedule_try(state, action, true)
    }

    fn advance_try(
        &mut self,
        state: &mut TryState,
        mut completion: Completion,
        line: usize,
    ) -> Result<Action, VmError> {
        match &state.phase {
            Phase::Body => {
                if completion.code == 1 {
                    completion.append_error_context(&format!(
                        "\n    (\"{}\" body line {line})",
                        state.invoked
                    ))?;
                }
                // Tcl snapshots the original options then resets the result
                // before selecting and binding a handler, even when none match.
                self.last_completion = Completion::ok(VmObj::empty());
                let mut matched = false;
                let mut selected = None;
                for (index, handler) in state.handlers.iter().enumerate() {
                    if !matched {
                        if handler.code != completion.code {
                            continue;
                        }
                        if completion.code == 1 {
                            let options = Dict::from_value(&completion.options.as_vm_obj())?;
                            let error = options.get(&VmObj::from_str("-errorcode"))?.unwrap();
                            let Ok(parts) = error.as_list_vec() else {
                                continue;
                            };
                            if parts.len() < handler.prefix.len() {
                                continue;
                            }
                            let equal = handler.prefix.iter().zip(parts).try_fold(
                                true,
                                |equal, (a, b)| {
                                    Ok::<_, VmError>(
                                        equal && a.ensure_bytes()? == b.ensure_bytes()?,
                                    )
                                },
                            )?;
                            if !equal {
                                continue;
                            }
                        }
                        matched = true;
                    }
                    if !handler.fallthrough {
                        selected = Some(index);
                        break;
                    }
                }
                state.pending = Some(completion);
                if let Some(index) = selected {
                    let handler = &state.handlers[index];
                    let previous = state.pending.as_ref().unwrap();
                    for (name, value) in handler
                        .variables
                        .iter()
                        .zip([previous.result.clone(), previous.options.as_vm_obj()])
                    {
                        let (base, element) = split_var_name(name);
                        if let Err(error) = self.var_set(&base, element.as_ref(), value) {
                            let mut failure = script_error(error)?;
                            during(&mut failure, previous)?;
                            state.pending = Some(failure);
                            return Ok(after_handler(state));
                        }
                    }
                    state.phase = Phase::Handler(handler.kind.clone());
                    return Ok(Action::Script(handler.script.clone()));
                }
                Ok(after_handler(state))
            }
            Phase::Handler(kind) => {
                if completion.code == 1 {
                    completion.append_error_context(&format!(
                        "\n    (\"{} ... {kind}\" handler line {line})",
                        state.invoked
                    ))?;
                    during(&mut completion, state.pending.as_ref().unwrap())?;
                }
                self.last_completion = completion.clone();
                state.pending = Some(completion);
                Ok(after_handler(state))
            }
            Phase::Finally => {
                let previous = state.pending.take().unwrap();
                if completion.code == 0 {
                    return Ok(Action::Done(previous));
                }
                if completion.code == 1 {
                    completion.append_error_context(&format!(
                        "\n    (\"{} ... finally\" body line {line})",
                        state.invoked
                    ))?;
                    during(&mut completion, &previous)?;
                }
                Ok(Action::Done(completion))
            }
        }
    }
}

fn after_handler(state: &mut TryState) -> Action {
    match state.finally.take() {
        Some(script) => {
            state.phase = Phase::Finally;
            Action::Script(script)
        }
        None => Action::Done(state.pending.take().unwrap()),
    }
}

fn during(completion: &mut Completion, previous: &Completion) -> Result<(), VmError> {
    let mut options = Dict::from_value(&completion.options.as_vm_obj())?;
    options.put(VmObj::from_str("-during"), previous.options.as_vm_obj())?;
    completion.options = ReturnOptions::from_vm_obj(options.into_value())?;
    Ok(())
}

fn script_error(error: VmError) -> Result<Completion, VmError> {
    match error {
        VmError::TclError {
            message,
            error_info,
            error_code,
        } => Ok(Completion::error(
            VmObj::from_str(&message),
            error_info.map(ByteString::from),
            error_code.map(|code| VmObj::from_str(&code)),
        )),
        other => Err(other),
    }
}

fn clause_arity(kind: &str) -> VmError {
    let usage = match kind {
        "finally" => "finally script",
        "on" => "on code variableList script",
        "trap" => "trap pattern variableList script",
        _ => unreachable!(),
    };
    tcl_error(
        format!("wrong # args to {kind} clause: must be \"... {usage}\""),
        &[
            "TCL",
            "OPERATION",
            "TRY",
            &kind.to_ascii_uppercase(),
            "ARGUMENT",
        ],
    )
}
