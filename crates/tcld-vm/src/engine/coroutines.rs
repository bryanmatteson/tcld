//! Real Tcl NRE coroutine scheduling with retained, mapped VM source frames.
use super::*;
#[cfg_attr(not(feature = "host"), allow(dead_code))]
pub(super) struct Handoff {
    pub words: Vec<VmObj>,
    pub created: Option<u64>,
    pub tail: Option<super::tailcalls::NativeTailcall>,
}
#[cfg_attr(not(feature = "host"), allow(dead_code))]
pub(super) struct Coroutine {
    pub(super) script_kind: Option<&'static str>,
    pub(super) repeat: bool,
    pub(super) tail_start: bool,
    pub(super) source: String,
    pub(super) site: CodeAddress,
    pub(super) namespace: NamespaceName,
    pub(super) frames: Vec<CallFrame>,
    pub(super) traces: Vec<execution::ActiveCommand>,
    pub(super) completion: Completion,
    pub(super) name: String,
    pub(super) tail_scope: Option<FrameId>,
    pub(super) parent: Option<u64>,
    pub(super) owner: u64,
    pub(super) created: Option<u64>,
}
impl HybridEngineInner {
    pub(super) fn invoke_coroutine(
        &mut self,
        words: &[VmObj],
        name: &str,
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        if name == "coroutine" && words.len() >= 3 {
            let namespace = self.frames[self.fi()].namespace.clone();
            let qname = namespace
                .qualify(&words[1].ensure_bytes()?)
                .as_str_lossy()
                .into_owned();
            self.registry
                .rename_vm_proc(&ByteString::from(qname.as_str()), None);
            self.image_stub_commands.remove(&qname);
            let id = self.next_event;
            self.next_event += 1;
            let source = words[2..]
                .iter()
                .map(|w| w.as_string().map(|s| tcld_compile::quote_word(&s)))
                .collect::<Result<Vec<_>, _>>()?
                .join(" ");
            self.coroutines.insert(
                id,
                Coroutine {
                    script_kind: None,
                    repeat: false,
                    tail_start: false,
                    source,
                    site,
                    namespace,
                    frames: Vec::new(),
                    traces: Vec::new(),
                    completion: Completion::ok(VmObj::empty()),
                    name: qname.clone(),
                    tail_scope: None,
                    parent: None,
                    owner: 0,
                    created: None,
                },
            );
            let args = vec![
                VmObj::from_str("coroutine"),
                VmObj::from_str(&qname),
                VmObj::from_str("::tcld::coroutine_body"),
                VmObj::from_int(id as i64),
            ];
            if self.coroutine_can_handoff() {
                self.coroutine_yield = Some(Handoff {
                    words: args,
                    tail: None,
                    created: Some(id),
                });
                return Ok(Some(InvokeOutcome::Value(VmObj::empty())));
            }
            let outcome = self.host_value_result(&args);
            // Failed creation never owns a suspended continuation.
            if self
                .coroutines
                .get(&id)
                .is_some_and(|c| c.frames.is_empty())
            {
                self.coroutines.remove(&id);
            }
            return outcome.map(|v| {
                Some(match v {
                    Ok(v) => InvokeOutcome::Value(v),
                    Err(c) => InvokeOutcome::Raise(c),
                })
            });
        }
        if matches!(name, "yield" | "yieldto") && self.coroutine_can_handoff() {
            // Tcl validates arity and coroutine context, and chooses the resume
            // convention (one optional value for yield, all arguments for yieldto).
            let mut args = words.to_vec();
            args[0] = VmObj::from_str(name);
            if name == "yieldto" && args.len() >= 2 {
                let base = self.coroutine_boundary.unwrap();
                let parent = self
                    .coroutines
                    .iter()
                    .rev()
                    .find(|(_, c)| c.owner == self.coroutine_identity.1 && !c.frames.is_empty())
                    .map(|(id, _)| *id);
                let caller = parent
                    .and_then(|id| self.coroutines[&id].frames.last())
                    .unwrap_or(&self.frames[base - 1]);
                let id = self.next_event;
                self.next_event += 1;
                let source = args[1..]
                    .iter()
                    .map(|w| w.as_string().map(|s| tcld_compile::quote_word(&s)))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(" ");
                self.coroutines.insert(
                    id,
                    Coroutine {
                        script_kind: None,
                        repeat: false,
                        tail_start: false,
                        source,
                        site,
                        namespace: self.frames[self.fi()].namespace.clone(),
                        frames: Vec::new(),
                        traces: Vec::new(),
                        completion: Completion::ok(VmObj::empty()),
                        name: String::new(),
                        tail_scope: Some(caller.id),
                        parent,
                        owner: 0,
                        created: None,
                    },
                );
                args = vec![
                    VmObj::from_str("yieldto"),
                    VmObj::from_str("::tcld::coroutine_tail"),
                    VmObj::from_int(id as i64),
                ];
            }
            self.coroutine_yield = Some(Handoff {
                words: args,
                tail: None,
                created: None,
            });
            return Ok(Some(InvokeOutcome::Value(VmObj::empty())));
        }
        #[cfg(feature = "host")]
        if self.coroutine_can_handoff() {
            let ns = self.frames[self.fi()]
                .namespace
                .as_bytes()
                .as_str_lossy()
                .into_owned();
            if self.execution_host(|host| Ok(host.is_coroutine_command(name, &ns)))? {
                let mut args = words.to_vec();
                let q = self.frames[self.fi()]
                    .namespace
                    .qualify(&words[0].ensure_bytes()?);
                if !self
                    .generated_leaf(&[
                        VmObj::from_str("namespace"),
                        VmObj::from_str("which"),
                        VmObj::from_str("-command"),
                        VmObj::Bytes(q.clone()),
                    ])?
                    .as_string()?
                    .is_empty()
                {
                    args[0] = VmObj::Bytes(q);
                }
                self.coroutine_yield = Some(Handoff {
                    words: args,
                    tail: None,
                    created: None,
                });
                return Ok(Some(InvokeOutcome::Value(VmObj::empty())));
            }
        }
        // Coroutine commands are native Tcl commands, so resolve them in the
        // mapped caller namespace before entering the host's global evaluator.
        let ns = self.frames[self.fi()].namespace.clone();
        if ns != NamespaceName::root() && !matches!(name, "coroutine" | "yield" | "yieldto") {
            let q = ns.qualify(&words[0].ensure_bytes()?);
            let exists = self
                .generated_leaf(&[
                    VmObj::from_str("namespace"),
                    VmObj::from_str("which"),
                    VmObj::from_str("-command"),
                    VmObj::Bytes(q.clone()),
                ])?
                .as_string()?;
            if !exists.is_empty() {
                let mut resolved = words.to_vec();
                resolved[0] = VmObj::Bytes(q);
                return self.host_value_result(&resolved).map(|v| {
                    Some(match v {
                        Ok(v) => InvokeOutcome::Value(v),
                        Err(c) => InvokeOutcome::Raise(c),
                    })
                });
            }
        }
        Ok(None)
    }
    pub(super) fn coroutine_can_handoff(&self) -> bool {
        self.coroutine_boundary.is_some()
            && self.host_callback_depth == 0
            && !self.debug_frame_eval_active
    }
    #[cfg(feature = "host")]
    pub(super) fn resume_coroutine(&mut self, words: &[VmObj]) -> Result<BridgeResult, VmError> {
        let id = words[1].as_i64()? as u64;
        let op = words[2].as_string()?;
        let caller_identity = words[7].as_i64()? as u64;
        let return_frame = self
            .coroutines
            .iter()
            .rev()
            .find(|(_, c)| c.owner == caller_identity && !c.frames.is_empty())
            .and_then(|(_, c)| c.frames.last().map(|f| f.id))
            .or_else(|| self.frames.last().map(|f| f.id));
        let mut task = self
            .coroutines
            .remove(&id)
            .ok_or_else(|| VmError::Internal("coroutine continuation is unavailable".into()))?;
        if let Some(created) = task.created.take()
            && self
                .coroutines
                .get(&created)
                .is_some_and(|c| c.frames.is_empty())
        {
            self.coroutines.remove(&created);
        }
        // Tail targets run in the native caller's environment. Its mapped
        // continuation may be parked on Tcl's NRE stack, so lend those frames
        // to this target, then return them with any real variable edits intact.
        let borrowed = self.borrow_coroutine_contexts(task.parent);
        let base = self.frames.len();
        let trace_base = self.traced_commands.len();
        let old_identity = std::mem::replace(
            &mut self.coroutine_identity,
            (words[6].as_i64()? as u64, words[7].as_i64()? as u64),
        );
        task.owner = self.coroutine_identity.0;
        let old_task = self.coroutine_task.replace(id);
        let old_boundary = self.coroutine_boundary.replace(base);
        let old_callback_depth = std::mem::replace(&mut self.host_callback_depth, 0);
        let old_completion = std::mem::replace(&mut self.last_completion, task.completion);
        let outcome = (|| {
            if op == "start" || (op == "tail" && task.tail_start) {
                if task.script_kind.is_none() && task.tail_scope.is_some() && !task.tail_start {
                    // Native yieldto runs this target after the caller command's
                    // leave boundary, before its resulting tail invocation.
                    #[cfg(feature = "host")]
                    if let Some(command) = self.traced_commands.last_mut() {
                        let native = std::mem::take(&mut command.native);
                        for trace in native.iter().rev() {
                            let old = std::mem::replace(&mut self.delivering_execution_trace, true);
                            let result = self.execution_host(|host| {
                                host.trace_hook(trace, true, &Completion::ok(VmObj::empty()))
                            });
                            self.delivering_execution_trace = old;
                            if let Some(c) = result? {
                                return Ok(LoopExit::Completion(c));
                            }
                        }
                    }
                }
                self.enter_generated(
                    if let Some(kind) = task.script_kind {
                        kind
                    } else if task.tail_start {
                        "tailcall-target"
                    } else if task.tail_scope.is_some() {
                        "yieldto-target"
                    } else {
                        "coroutine"
                    },
                    task.source.clone(),
                    task.site,
                    task.namespace.clone(),
                    task.tail_scope,
                    if task.tail_scope.is_some() {
                        FrameKind::Dynamic
                    } else {
                        FrameKind::Namespace
                    },
                    None,
                )?;
                let global = self.frames.first().map(|f| f.id);
                let root = self.frames.last_mut().unwrap();
                root.continuation = None;
                root.hidden_scope = true;
                root.tail_root = task.tail_start;
                root.tcl_parent = if task.tail_start {
                    task.tail_scope
                } else {
                    global
                };
                task.tail_start = false;
            } else {
                self.frames.append(&mut task.frames);
                self.traced_commands.append(&mut task.traces);
                if op == "cancel" {
                    while self.frames.len() > base {
                        self.cancel_execution_commands()?;
                        self.cleanup_trace_scope();
                        self.frames.pop();
                    }
                    self.traced_commands.truncate(trace_base);
                    return Ok(LoopExit::Done(VmObj::empty()));
                }
                if op == "tail" {
                    return self.eval_loop(base, true);
                }
                let fi = self.fi();
                self.frames[fi].stack.pop(); // placeholder result of the suspended invoke
                let transferred = self.coroutine_completions.remove(&self.frames[fi].id.0);
                let code = transferred
                    .as_ref()
                    .map(|c| c.code)
                    .unwrap_or(words[3].as_i64()? as i32);
                if code == 0 {
                    self.push(words[4].clone());
                } else {
                    let pc = self.frames[fi].code.instructions
                        [self.frames[fi].pc_index.saturating_sub(1)]
                    .pc;
                    let completion = transferred.unwrap_or(Completion {
                        code,
                        result: words[4].clone(),
                        options: ReturnOptions::from_vm_obj(words[5].clone())?,
                    });
                    if let Some(exit) = self.route_completion(completion, pc, base)? {
                        return Ok(exit);
                    }
                }
            }
            self.eval_loop(base, true)
        })();
        self.coroutine_task = old_task;
        self.coroutine_boundary = old_boundary;
        self.coroutine_identity = old_identity;
        self.host_callback_depth = old_callback_depth;
        task.completion = std::mem::replace(&mut self.last_completion, old_completion);
        let yielded = self.coroutine_yield.take();
        let result = match (outcome, yielded) {
            (Ok(LoopExit::Paused), Some(handoff)) => {
                task.created = handoff.created;
                task.frames = self.frames.split_off(base);
                task.traces = self.traced_commands.split_off(trace_base);
                self.coroutines.insert(id, task);
                self.return_coroutine_contexts(borrowed);
                let (code, result) = if let Some(tail) = handoff.tail {
                    (201, VmObj::from_int(tail.into_raw() as i64))
                } else {
                    (200, VmObj::List(handoff.words))
                };
                crate::bridge::set_native_handoff(code);
                return Ok(BridgeResult {
                    code,
                    result,
                    error_info: None,
                    error_code: None,
                    return_options: None,
                });
            }
            (Ok(LoopExit::Done(v)), _) => Ok(Completion::ok(v)),
            (Ok(LoopExit::Completion(c)), _) => Ok(c),
            (Ok(LoopExit::Paused), _) => Err(VmError::Internal(
                "coroutine paused without an NRE handoff".into(),
            )),
            (Err(e), _) => Err(e),
        };
        // Errors/cancellation cannot leave stale frames or trace-command handles.
        while self.frames.len() > base {
            self.cleanup_trace_scope();
            self.frames.pop();
        }
        self.traced_commands.truncate(trace_base);
        self.return_coroutine_contexts(borrowed);
        let c = result?;
        // Tcl's outer host Eval boundary can consume TCL_RETURN or convert
        // break/continue. Preserve the completion for its actual mapped caller.
        if c.code >= 2
            && task.script_kind.is_none()
            && let Some(caller) = task.tail_scope.or(return_frame)
        {
            self.coroutine_completions.insert(caller.0, c.clone());
        }
        if task.repeat && op != "cancel" {
            // Tcl invokes repeated control bodies for time and dict. Retain the
            // verified source template, but start with fresh execution state.
            task.completion = Completion::ok(VmObj::empty());
            self.coroutines.insert(id, task);
        }
        Ok(BridgeResult {
            code: c.code,
            result: c.result,
            error_info: None,
            error_code: None,
            return_options: if c.code == 0 {
                None
            } else {
                Some(c.options.as_vm_obj())
            },
        })
    }
    pub(super) fn borrow_coroutine_contexts(
        &mut self,
        parent: Option<u64>,
    ) -> Vec<(u64, usize, usize)> {
        let mut ancestors = Vec::new();
        let mut parent = parent;
        while let Some(id) = parent {
            ancestors.push(id);
            parent = self.coroutines.get(&id).and_then(|c| c.parent);
        }
        let mut borrowed = Vec::new();
        for id in ancestors.into_iter().rev() {
            if let Some(c) = self.coroutines.get_mut(&id) {
                borrowed.push((id, self.frames.len(), self.traced_commands.len()));
                self.frames.append(&mut c.frames);
                self.traced_commands.append(&mut c.traces);
            }
        }
        borrowed
    }
    #[cfg(feature = "host")]
    pub(super) fn return_coroutine_contexts(&mut self, borrowed: Vec<(u64, usize, usize)>) {
        for (id, base, traces) in borrowed.into_iter().rev() {
            if let Some(c) = self.coroutines.get_mut(&id) {
                c.frames = self.frames.split_off(base);
                c.traces = self.traced_commands.split_off(traces);
            }
        }
    }
    pub(super) fn clear_coroutines(&mut self) {
        #[cfg(feature = "host")]
        {
            let names = self
                .coroutines
                .values()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>();
            for name in names.into_iter().filter(|n| !n.is_empty()) {
                let _ = self.execution_host(|host| host.uninstall_stub_command(&name));
            }
        }
        self.coroutines.clear();
        self.control_groups.clear();
        self.coroutine_task = None;
        self.coroutine_completions.clear();
        self.coroutine_yield = None;
        self.coroutine_boundary = None;
    }
}
