/*
 * Ported from Tcl 9.0.2's generic/tclCmdAH.c (EachloopCmd,
 * ForeachLoopStep and ForeachAssignments).
 * Copyright © 1987-1993 The Regents of the University of California.
 * Copyright © 1994-1997 Sun Microsystems, Inc.
 * See ../regex/license.terms for usage, redistribution and warranty terms.
 */

//! List mapping retains its iterator and results in the mapped body frame.
//! The scheduler can park that frame for a breakpoint or coroutine suspension.
use super::*;

struct Input {
    variables: Vec<ByteString>,
    values: Vec<VmObj>,
}

pub(super) struct LmapState {
    inputs: Vec<Input>,
    iteration: usize,
    iterations: usize,
    results: Vec<VmObj>,
}

impl HybridEngineInner {
    pub(super) fn invoke_lmap(
        &mut self,
        words: &[VmObj],
        site: CodeAddress,
    ) -> Result<InvokeOutcome, VmError> {
        if words.len() < 4 || !words.len().is_multiple_of(2) {
            return Err(crate::args::wrong_args(
                &[&words[0].as_string()?],
                "varList list ?varList list ...? command",
            ));
        }
        let mut state = LmapState {
            inputs: Vec::new(),
            iteration: 0,
            iterations: 0,
            results: Vec::new(),
        };
        // Validate and snapshot each pair in Tcl's order, before any store or
        // body admission. Later writes to an input cannot change its iteration.
        for pair in words[1..words.len() - 1].chunks_exact(2) {
            let variables = pair[0].as_list_vec()?;
            if variables.is_empty() {
                return Err(crate::args::tcl_error(
                    "lmap varlist is empty",
                    &["TCL", "OPERATION", "LMAP", "NEEDVARS"],
                ));
            }
            let variables = variables
                .iter()
                .map(VmObj::ensure_bytes)
                .collect::<Result<Vec<_>, _>>()?;
            let values = pair[1].as_list_vec()?;
            state.iterations = state.iterations.max(values.len().div_ceil(variables.len()));
            state.inputs.push(Input { variables, values });
        }
        if state.iterations == 0 {
            return Ok(InvokeOutcome::Value(VmObj::List(Vec::new())));
        }
        self.lmap_assignments(&state)?;
        let namespace = self.frames[self.fi()].namespace.clone();
        let scope = self.frames[self.scope_index()].id;
        self.enter_generated(
            "lmap-body",
            words.last().unwrap().as_string()?,
            site,
            namespace,
            Some(scope),
            FrameKind::Dynamic,
            None,
        )?;
        let frame = self.frames.last_mut().unwrap();
        frame.body_error_context = Some("\"lmap\" body".into());
        frame.lmap = Some(state);
        Ok(InvokeOutcome::FramePushed)
    }

    fn lmap_assignments(&mut self, state: &LmapState) -> Result<(), VmError> {
        for input in &state.inputs {
            for (offset, name) in input.variables.iter().enumerate() {
                let value = state
                    .iteration
                    .checked_mul(input.variables.len())
                    .and_then(|start| start.checked_add(offset))
                    .and_then(|index| input.values.get(index))
                    .cloned()
                    .unwrap_or_else(VmObj::empty);
                let (base, element) = split_var_name(name);
                self.var_set(&base, element.as_ref(), value)
                    .map_err(|error| match error {
                        VmError::TclError {
                            message,
                            error_info,
                            error_code,
                        } => VmError::TclError {
                            error_info: Some(format!(
                                "{}\n    (setting lmap loop variable \"{}\")",
                                error_info.as_deref().unwrap_or(&message),
                                name.as_str_lossy(),
                            )),
                            message,
                            error_code,
                        },
                        other => other,
                    })?;
            }
        }
        Ok(())
    }

    /// Consume a body result, break or continue. `None` schedules the next
    /// iteration; a completion finishes the lmap invocation. Catch/loop ranges
    /// inside the body have already had the first opportunity to handle it.
    pub(super) fn resume_lmap(
        &mut self,
        completion: Completion,
    ) -> Result<Option<Completion>, VmError> {
        let mut state = self.frames.last_mut().unwrap().lmap.take().unwrap();
        if matches!(completion.code, 3 | 4) {
            self.consume_loop_completion(&completion)?;
        }
        if completion.code == 0 {
            state.results.push(completion.result);
        }
        state.iteration += 1;
        if completion.code == 3 || state.iteration >= state.iterations {
            return Ok(Some(Completion {
                code: 0,
                result: VmObj::List(state.results),
                options: self.last_completion.options.clone(),
            }));
        }
        // Discard operands and nested iteration/catch state left by continue.
        // Bindings run outside the next body, so assignment errors get only
        // Tcl's setting-variable context, not a misleading body line.
        let frame = self.frames.last_mut().unwrap();
        frame.stack.clear();
        frame.catch_stack.clear();
        frame.aux_states.clear();
        frame.loop_marks.clear();
        frame.body_error_context = None;
        match self.lmap_assignments(&state) {
            Ok(()) => {}
            Err(VmError::TclError {
                message,
                error_info,
                error_code,
            }) => {
                return Ok(Some(Completion::error(
                    VmObj::from_str(&message),
                    error_info.map(ByteString::from),
                    error_code.map(|code| VmObj::from_str(&code)),
                )));
            }
            Err(error) => return Err(error),
        }
        let frame = self.frames.last_mut().unwrap();
        frame.pc_index = 0;
        frame.body_error_context = Some("\"lmap\" body".into());
        frame.lmap = Some(state);
        Ok(None)
    }
}
