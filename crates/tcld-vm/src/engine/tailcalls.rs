//! Native Tcl tailcall preparation and NRE scheduling. The mapped procedure
//! owns the pending native tail until its frame exits, including caught returns.
use super::*;
pub(crate) struct NativeTailcall(pub(crate) usize);
impl NativeTailcall {
    pub(super) fn into_raw(self) -> usize {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }
}
impl Drop for NativeTailcall {
    fn drop(&mut self) {
        #[cfg(feature = "host")]
        unsafe {
            tcld_tailcall_free(self.0 as *mut std::ffi::c_void);
        }
    }
}
#[cfg(feature = "host")]
unsafe extern "C" {
    fn tcld_tailcall_free(tail: *mut std::ffi::c_void);
}
pub(super) struct Tailcall {
    native: NativeTailcall,
    source: String,
    namespace: NamespaceName,
    site: CodeAddress,
}
impl HybridEngineInner {
    pub(super) fn invoke_tailcall(
        &mut self,
        words: &[VmObj],
        name: &str,
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        if name != "tailcall" {
            return Ok(None);
        }
        #[cfg(feature = "host")]
        {
            let scope = self.scope_index();
            let frame = self.frames[scope].id;
            let allowed = matches!(
                self.frames[scope].kind,
                FrameKind::VmProc | FrameKind::Lambda
            );
            let ns = self.frames[scope].namespace.clone();
            let (result, pending) = self.execution_host(|host| {
                host.prepare_tailcall(words, &ns.as_bytes().as_str_lossy(), allowed)
            })?;
            if result.code == 2 {
                self.tailcalls.remove(&frame.0);
                if let Some(native) = pending {
                    let source = words[1..]
                        .iter()
                        .map(|w| w.as_string().map(|s| tcld_compile::quote_word(&s)))
                        .collect::<Result<Vec<_>, _>>()?
                        .join(" ");
                    self.tailcalls.insert(
                        frame.0,
                        Tailcall {
                            native,
                            source,
                            namespace: ns,
                            site,
                        },
                    );
                }
            }
            self.map_bridge_result(result).map(Some)
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = (words, site);
            Err(VmError::Internal(
                "tailcall requires the real Tcl host".into(),
            ))
        }
    }
    pub(super) fn perform_tailcall(
        &mut self,
        tail: Tailcall,
        mut finished: CallFrame,
        result: VmObj,
        boundary: usize,
    ) -> Result<Option<LoopExit>, VmError> {
        // The command that entered the departed procedure leaves before its
        // native tail runs. Preserve Tcl's trace order without a second leave.
        #[cfg(feature = "host")]
        if let Some(command) = self.traced_commands.last_mut() {
            let native = std::mem::take(&mut command.native);
            for trace in native.iter().rev() {
                let old = std::mem::replace(&mut self.delivering_execution_trace, true);
                let traced = self.execution_host(|host| {
                    host.trace_hook(trace, true, &Completion::ok(result.clone()))
                });
                self.delivering_execution_trace = old;
                if let Some(c) = traced? {
                    if let Some(continuation) = &finished.continuation {
                        return self.route_completion(c, continuation.call_site.pc, boundary);
                    }
                    return Ok(Some(LoopExit::Completion(c)));
                }
            }
        }
        // Check the retained native namespace after cleanup, before parking
        // mapped callers. A deleted namespace must still reach their catches.
        #[cfg(feature = "host")]
        if let Some(c) = self.execution_host(|host| host.validate_tailcall(&tail.native))? {
            return if let Some(continuation) = &finished.continuation {
                self.route_completion(c, continuation.call_site.pc, boundary)
            } else {
                Ok(Some(LoopExit::Completion(c)))
            };
        }
        // A generated tail target is a one-command redirector. Its native
        // counterpart is gone before the next tail begins, so discard that
        // hidden mapped boundary as well instead of accumulating redirectors.
        while self.frames.last().is_some_and(|f| f.tail_root) {
            self.unwind_execution_commands(None, Completion::ok(result.clone()))?;
            self.cleanup_trace_scope();
            finished = self.frames.pop().unwrap();
        }
        if self.coroutine_can_handoff() {
            // Replace the departed mapped frame, then return to Tcl's NRE loop.
            // The native scheduler invokes this task again after the tailcall
            // boundary; no Rust evaluator recursion grows with a tail chain.
            self.enter_generated(
                "tailcall-target",
                tail.source,
                tail.site,
                tail.namespace,
                finished.tcl_parent,
                FrameKind::Dynamic,
                None,
            )?;
            let root = self.frames.last_mut().unwrap();
            root.hidden_scope = true;
            root.tail_root = true;
            root.tcl_parent = finished.tcl_parent;
            root.continuation = finished.continuation;
            self.coroutine_yield = Some(coroutines::Handoff {
                words: Vec::new(),
                created: None,
                tail: Some(tail.native),
            });
            return Ok(None);
        }
        #[cfg(feature = "host")]
        {
            let id = self.next_event;
            self.next_event += 1;
            self.coroutines.insert(
                id,
                coroutines::Coroutine {
                    script_kind: None,
                    repeat: false,
                    tail_start: true,
                    source: tail.source,
                    site: tail.site,
                    namespace: tail.namespace,
                    frames: Vec::new(),
                    traces: Vec::new(),
                    completion: Completion::ok(VmObj::empty()),
                    name: String::new(),
                    tail_scope: finished.tcl_parent,
                    parent: None,
                    owner: 0,
                    created: None,
                },
            );
            let args = [
                VmObj::from_str("::tcld::tail_transfer"),
                VmObj::from_int(id as i64),
                VmObj::from_int(tail.native.into_raw() as i64),
            ];
            let completion = self.execution_host(|host| host.invoke_objv(&args))?;
            self.coroutines.remove(&id);
            match self.map_bridge_result(completion)? {
                InvokeOutcome::Value(value) => {
                    self.complete_popped_frame(finished, value, boundary)
                }
                InvokeOutcome::Raise(c) => {
                    if let Some(continuation) = finished.continuation {
                        self.route_completion(c, continuation.call_site.pc, boundary)
                    } else {
                        Ok(Some(LoopExit::Completion(c)))
                    }
                }
                InvokeOutcome::FramePushed => Err(VmError::Internal(
                    "native tail returned an unfinished frame".into(),
                )),
            }
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = (tail, finished, boundary);
            Err(VmError::Internal(
                "tailcall requires the real Tcl host".into(),
            ))
        }
    }
}
