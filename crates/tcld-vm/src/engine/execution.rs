//! Mapped command boundaries feed the real Tcl extension execution hooks.
use super::*;
pub(super) struct ActiveCommand {
    frame: FrameId,
    anchor: CodeAddress,
    range: std::ops::Range<u32>,
    words: Vec<VmObj>,
    #[cfg(feature = "host")]
    pub(super) native: Vec<crate::bridge::TraceCommand>,
}
impl HybridEngineInner {
    #[cfg(feature = "host")]
    pub(super) fn execution_host<R>(
        &mut self,
        f: impl FnOnce(&mut HostBridge) -> Result<R, VmError>,
    ) -> Result<R, VmError> {
        if let Some(mut host) = self.host.take() {
            let engine = self as *mut Self;
            let result = host.with_stub_dispatch(
                Box::new(move |words| unsafe { (&mut *engine).dispatch_stub_reenter(words) }),
                f,
            );
            self.host = Some(host);
            result
        } else {
            self.inspection_bridge(f).ok_or_else(|| {
                VmError::HybridUnavailable(
                    "execution tracing needs an active Tcl interpreter".into(),
                )
            })?
        }
    }
    fn execution_leave(
        &mut self,
        command: ActiveCommand,
        mut completion: Completion,
    ) -> Result<Completion, VmError> {
        #[cfg(feature = "host")]
        for native in command.native.iter().rev() {
            let saved = self.delivering_execution_trace;
            self.delivering_execution_trace = true;
            let result = self.execution_host(|host| host.trace_hook(native, true, &completion));
            self.delivering_execution_trace = saved;
            if let Some(changed) = result? {
                completion = changed;
            }
        }
        let _ = command;
        Ok(completion)
    }
    #[cfg(feature = "host")]
    pub(super) fn cancel_execution_commands(&mut self) -> Result<(), VmError> {
        let frame = self.frames[self.fi()].id;
        while self
            .traced_commands
            .last()
            .is_some_and(|c| c.frame == frame)
        {
            let command = self.traced_commands.pop().unwrap();
            for native in command.native.iter().rev() {
                self.execution_host(|host| {
                    host.cancel_trace(native);
                    Ok(())
                })?;
            }
        }
        Ok(())
    }
    pub(super) fn unwind_execution_commands(
        &mut self,
        target: Option<u32>,
        mut completion: Completion,
    ) -> Result<Completion, VmError> {
        let frame = self.frames[self.fi()].id;
        while self
            .traced_commands
            .last()
            .is_some_and(|c| c.frame == frame && target.is_none_or(|pc| !c.range.contains(&pc)))
        {
            let command = self.traced_commands.pop().unwrap();
            completion = self.execution_leave(command, completion)?;
        }
        Ok(completion)
    }
    pub(super) fn execution_boundary(&mut self, pc: u32) -> Result<Option<Flow>, VmError> {
        let fi = self.fi();
        let frame = self.frames[fi].id;
        let code_id = self.frames[fi].code.id;
        let Some(source) = self.source_images.get(&code_id) else {
            return Ok(None);
        };
        let maps = &source.image.units[source.unit_index].commands;
        if let Some((anchor, index)) = maps.iter().find_map(|m| {
            m.trace_word_copies
                .iter()
                .find(|(copy, _)| *copy == pc)
                .map(|(_, index)| (m.anchor_pc, *index))
        }) {
            let value = self
                .traced_commands
                .iter()
                .rev()
                .find(|c| c.frame == frame && c.anchor.pc == anchor)
                .and_then(|c| c.words.get(index))
                .cloned()
                .ok_or_else(|| VmError::Internal("lost evaluated command argument".into()))?;
            self.frames[fi].stack.push(value);
            return Ok(Some(Flow::Continue));
        }
        if let Some(map) = maps.iter().find(|m| m.trace_end_pc == pc) {
            let anchor = map.anchor_pc;
            let index = self
                .traced_commands
                .iter()
                .rposition(|c| c.frame == frame && c.anchor.pc == anchor)
                .ok_or_else(|| VmError::Internal(format!("missing command completion at {pc}")))?;
            let command = self.traced_commands.remove(index);
            let result = self.frames[fi]
                .stack
                .last()
                .cloned()
                .unwrap_or_else(VmObj::empty);
            let completion = self.execution_leave(command, Completion::ok(result))?;
            return Ok(Some(if completion.code == 0 {
                Flow::Continue
            } else {
                Flow::Raise(completion)
            }));
        }
        let Some(map) = maps.iter().find(|m| m.trace_pc == pc).cloned() else {
            return Ok(None);
        };
        let expanded_start = if self.frames[fi]
            .code
            .instructions
            .get(self.frames[fi].pc_index)
            .is_some_and(|i| i.opcode == Opcode::InvokeExpanded)
        {
            self.frames[fi]
                .aux_states
                .iter()
                .rev()
                .find_map(|s| match s {
                    AuxRuntimeState::Expand(mark) => Some(mark.stack_depth),
                    _ => None,
                })
        } else {
            None
        };
        let words = if map.trace_inline {
            self.frames[fi]
                .stack
                .pop()
                .ok_or_else(|| VmError::Internal("missing inline command arguments".into()))?
                .as_list_vec()?
        } else if let Some(start) = expanded_start {
            self.frames[fi].stack[start..].to_vec()
        } else {
            let stack = &self.frames[fi].stack;
            stack
                .get(
                    stack
                        .len()
                        .checked_sub(map.trace_argv_count)
                        .ok_or_else(|| VmError::Internal("missing command arguments".into()))?..,
                )
                .unwrap()
                .to_vec()
        };
        let anchor = CodeAddress {
            code_id,
            pc: map.anchor_pc,
        };
        let hidden = self.hidden_trace_commands.contains(&anchor);
        #[cfg(feature = "host")]
        let native = if self.mode == RuntimeMode::Hybrid && !hidden {
            let ns = self.frames[fi]
                .namespace
                .as_bytes()
                .as_str_lossy()
                .into_owned();
            let level = self.tcl_scope_chain().len().saturating_sub(1);
            let depth = self.traced_commands.len() + 1;
            self.execution_host(|host| host.trace_command(&ns, &words, level, depth))?
        } else {
            None
        };
        #[cfg(not(feature = "host"))]
        let _ = hidden;
        let mut command = ActiveCommand {
            frame,
            anchor,
            range: map.pc_range,
            words,
            #[cfg(feature = "host")]
            native: Vec::new(),
        };
        #[cfg(feature = "host")]
        {
            let mut next = native;
            // Built-in ensembles rewrite once; support nested user ensembles
            // with the same structural guard as mapped call frames.
            let mut rewrites = 0;
            while let Some(native) = next {
                if rewrites >= MAX_CALL_DEPTH {
                    return Err(VmError::Internal(
                        "ensemble rewrite nesting exceeded".into(),
                    ));
                }
                rewrites += 1;
                let saved = self.delivering_execution_trace;
                self.delivering_execution_trace = true;
                let result = self.execution_host(|host| {
                    host.trace_hook(&native, false, &Completion::ok(VmObj::empty()))
                });
                self.delivering_execution_trace = saved;
                if let Some(completion) = result? {
                    // A failed enter has no leave; already-entered ensemble
                    // parents still complete with that error.
                    if !command.native.is_empty() {
                        self.traced_commands.push(command);
                    }
                    return Ok(Some(Flow::Raise(completion)));
                }
                let refreshed = native.refresh();
                if refreshed < 0 {
                    // Let Tcl run its real unknown-command resolver. A removed
                    // command has no leave event; entered ensemble parents do.
                    let words = command.words.clone();
                    self.traced_commands.push(command);
                    self.frames[fi].pc_index =
                        self.frames[fi].code.index_for_pc(map.trace_end_pc)?;
                    if let Some(start) = expanded_start {
                        self.frames[fi].aux_states.pop();
                        self.frames[fi].stack.truncate(start);
                    } else if !map.trace_inline {
                        let len = self.frames[fi].stack.len();
                        self.frames[fi].stack.truncate(len - map.trace_argv_count);
                    }
                    let flow = match self.invoke_world_host(&words, &words[0].ensure_bytes()?)? {
                        InvokeOutcome::Value(value) => {
                            self.frames[fi].stack.push(value);
                            Flow::Continue
                        }
                        InvokeOutcome::Raise(completion) => Flow::Raise(completion),
                        InvokeOutcome::FramePushed => {
                            return Err(VmError::Internal(
                                "unknown resolver escaped native call".into(),
                            ));
                        }
                    };
                    return Ok(Some(flow));
                }
                next = native.ensemble();
                command.native.push(native);
            }
        }
        let inline_proc = if map.trace_inline {
            match self.registry.resolve(
                &command.words[0].ensure_bytes()?,
                self.frames[fi].namespace.as_bytes(),
                self.mode,
            ) {
                CommandRef::VmProc(proc) => Some(proc),
                _ => None,
            }
        } else {
            None
        };
        self.traced_commands.push(command);
        if let Some(proc) = inline_proc {
            let words = self.traced_commands.last().unwrap().words.clone();
            self.frames[fi].pc_index = self.frames[fi].code.index_for_pc(map.trace_end_pc)?;
            self.push_call_frame(
                Arc::clone(&proc.code),
                FrameKind::VmProc,
                Some(proc.qname.clone()),
                &words[1..],
                NamespaceName::from_proc_qname(&proc.qname),
                CodeAddress { code_id, pc },
            )?;
            self.frames.last_mut().unwrap().invocation = Some(VmObj::List(words));
        }
        Ok(Some(Flow::Continue))
    }
}
