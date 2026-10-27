//! Real Tcl traces and notifier callbacks. Only the visitor body crosses into
//! the mapped VM; Tcl owns registration, triggering, order, guards and errors.
use super::*;
pub(super) struct VariableAccess {
    frame: FrameId,
    depth: u32,
    name: ByteString,
    element: Option<ByteString>,
}
pub(super) struct AccessGuard {
    stack: *mut Vec<VariableAccess>,
    pushed: bool,
}
impl Drop for AccessGuard {
    fn drop(&mut self) {
        if self.pushed {
            unsafe {
                (&mut *self.stack).pop();
            }
        }
    }
}
#[derive(Clone)]
pub(super) struct HostCallback {
    pub prefix: String,
    pub site: CodeAddress,
    pub kind: String,
    pub target: String,
    pub operations: String,
}
impl HybridEngineInner {
    pub(super) fn variable_access(
        &mut self,
        name: ByteString,
        element: Option<ByteString>,
    ) -> AccessGuard {
        let frame = self.frames[self.scope_index()].id;
        let pushed = !self
            .variable_accesses
            .last()
            .is_some_and(|a| a.frame == frame && a.depth == self.host_callback_depth);
        if pushed {
            self.variable_accesses.push(VariableAccess {
                frame,
                depth: self.host_callback_depth,
                name,
                element,
            });
        }
        AccessGuard {
            stack: &mut self.variable_accesses,
            pushed,
        }
    }
    fn callback_prefix(id: u64) -> VmObj {
        VmObj::List(vec![
            VmObj::from_str("::tcld::callback"),
            VmObj::from_int(id as i64),
        ])
    }
    fn original_callback(&self, prefix: &VmObj) -> VmObj {
        let Ok(words) = prefix.as_list_vec() else {
            return prefix.clone();
        };
        if words.len() == 2
            && words[0].as_string().ok().as_deref() == Some("::tcld::callback")
            && let Some(cb) = words[1]
                .as_string()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .and_then(|id| self.host_callbacks.get(&id))
        {
            return VmObj::from_str(&cb.prefix);
        }
        prefix.clone()
    }
    fn callback_id(
        &mut self,
        prefix: String,
        site: CodeAddress,
        kind: String,
        target: String,
        operations: String,
    ) -> u64 {
        if let Some((id, _)) = self.host_callbacks.iter().find(|(_, cb)| {
            cb.prefix == prefix
                && cb.kind == kind
                && cb.target == target
                && cb.operations == operations
        }) {
            return *id;
        }
        let id = self.next_event;
        self.next_event += 1;
        self.host_callbacks.insert(
            id,
            HostCallback {
                prefix,
                site,
                kind,
                target,
                operations,
            },
        );
        id
    }
    // Give traced VM locals actual Tcl variable storage. Existing aliases keep
    // pointing to their owner. Unset traces also run when that owner returns.
    pub(super) fn trace_target(&mut self, name: &str) -> Result<String, VmError> {
        let (base, mut element) = split_var_name(&ByteString::from(name));
        let mut fi = self.scope_index();
        let mut base = base;
        loop {
            let link = self.frames[fi]
                .compiled_locals
                .iter()
                .find(|s| s.is_named(&base))
                .and_then(|s| s.link.clone())
                .or_else(|| match self.frames[fi].local_vars.get_slot(&base) {
                    Some(VarSlot::Link(l)) => Some(l.clone()),
                    _ => None,
                });
            match link {
                Some(VarLink::HostVar { name1, name2 }) => {
                    let part = element.as_ref().or(name2.as_ref());
                    return Ok(if let Some(part) = part {
                        format!("{}({})", name1.as_str_lossy(), part.as_str_lossy())
                    } else {
                        name1.as_str_lossy().into_owned()
                    });
                }
                Some(VarLink::Vm {
                    target_frame,
                    target_name,
                    part2,
                }) => {
                    if let Some(part) = part2 {
                        if element.is_some() {
                            return Err(VmError::HostFrameMarshalUnsupported(
                                "array element on a scalar alias".into(),
                            ));
                        }
                        element = Some(part);
                    }
                    fi = self
                        .frames
                        .iter()
                        .position(|f| f.id == target_frame)
                        .ok_or(VmError::CrossRuntimeUpvar)?;
                    base = target_name;
                }
                None => break,
            }
        }
        if base.as_str_lossy().contains("::")
            || matches!(
                self.frames[fi].kind,
                FrameKind::Namespace | FrameKind::Toplevel
            )
        {
            let target = self.frames[fi]
                .namespace
                .qualify(&base)
                .as_str_lossy()
                .into_owned();
            return Ok(if let Some(e) = element {
                format!("{target}({})", e.as_str_lossy())
            } else {
                target
            });
        }
        let namespace = format!("::tcld::locals::f{}", self.frames[fi].id.0);
        self.generated_leaf(&[
            VmObj::from_str("namespace"),
            VmObj::from_str("eval"),
            VmObj::from_str(&namespace),
            VmObj::empty(),
        ])?;
        let target = format!("{namespace}::{}", base.as_str_lossy());
        let old = self.frames[fi]
            .compiled_locals
            .iter()
            .find(|s| s.is_named(&base))
            .and_then(|s| s.value.clone());
        let slot = self.frames[fi].local_vars.get_slot(&base).cloned();
        match slot {
            Some(VarSlot::Array(map)) => {
                let pairs = map
                    .into_iter()
                    .flat_map(|(k, v)| [VmObj::Bytes(k), v])
                    .collect();
                self.generated_leaf(&[
                    VmObj::from_str("array"),
                    VmObj::from_str("set"),
                    VmObj::from_str(&target),
                    VmObj::List(pairs),
                ])?;
            }
            Some(VarSlot::Scalar(v)) => {
                self.host_set(&ByteString::from(target.as_str()), None, v)?;
            }
            _ => {
                if let Some(v) = old {
                    self.host_set(&ByteString::from(target.as_str()), None, v)?;
                }
            }
        }
        let link = VarLink::HostVar {
            name1: ByteString::from(target.as_str()),
            name2: None,
        };
        if let Some(slot) = self.frames[fi]
            .compiled_locals
            .iter_mut()
            .find(|s| s.is_named(&base))
        {
            slot.value = None;
            slot.link = Some(link.clone());
        }
        self.frames[fi].local_vars.install_link(base, link);
        self.trace_scopes.insert(self.frames[fi].id.0, namespace);
        Ok(if let Some(e) = element {
            format!("{target}({})", e.as_str_lossy())
        } else {
            target
        })
    }
    pub(super) fn cleanup_trace_scope(&mut self) {
        let id = self.frames[self.fi()].id.0;
        if let Some(namespace) = self.trace_scopes.remove(&id) {
            let _ = self.generated_leaf(&[
                VmObj::from_str("namespace"),
                VmObj::from_str("delete"),
                VmObj::from_str(&namespace),
            ]);
        }
    }
    pub(super) fn invoke_host_callback(
        &mut self,
        words: &[VmObj],
    ) -> Result<InvokeOutcome, VmError> {
        // A native control command may bind a variable between mapped bodies.
        // Its coroutine's caller frames are parked on Tcl's NRE stack; lend
        // that exact owner context to traces before entering their mapped body.
        #[cfg(feature = "host")]
        let borrowed = {
            let group = self.execution_host(|host| Ok(host.active_control()))?;
            let owner = self.control_groups.get(&group).map(|g| g.owner);
            let parent = owner
                .filter(|owner| !self.frames.iter().any(|f| f.id == *owner))
                .and_then(|owner| {
                    self.coroutines
                        .iter()
                        .find(|(_, c)| c.frames.iter().any(|f| f.id == owner))
                        .map(|(id, _)| *id)
                });
            self.borrow_coroutine_contexts(parent)
        };
        let result = self.invoke_host_callback_in_context(words);
        #[cfg(feature = "host")]
        self.return_coroutine_contexts(borrowed);
        result
    }
    fn invoke_host_callback_in_context(
        &mut self,
        words: &[VmObj],
    ) -> Result<InvokeOutcome, VmError> {
        let id = words
            .get(1)
            .ok_or_else(|| VmError::Internal("missing callback ID".into()))?
            .as_string()?
            .parse::<u64>()
            .map_err(|_| VmError::Internal("invalid callback ID".into()))?;
        let cb = self
            .host_callbacks
            .get(&id)
            .cloned()
            .ok_or_else(|| VmError::Internal("expired callback".into()))?;
        // Inspecting variables must not execute user read traces. Explicit
        // console evaluations still execute callbacks, without nested pauses.
        if self.inspecting.get() || (!self.callbacks_can_pause() && !self.debug_frame_eval_active) {
            return Ok(InvokeOutcome::Value(VmObj::empty()));
        }
        #[cfg(feature = "host")]
        if self.debug_frame_eval_active && !crate::bridge::in_frame_eval_visitor() {
            return Ok(InvokeOutcome::Value(VmObj::empty()));
        }
        if cb.kind == "execution"
            && !self.delivering_execution_trace
            && !(self.debug_frame_eval_active && {
                #[cfg(feature = "host")]
                {
                    crate::bridge::in_frame_eval_visitor()
                }
                #[cfg(not(feature = "host"))]
                {
                    false
                }
            })
        {
            return Ok(InvokeOutcome::Value(VmObj::empty()));
        }
        let saved_delivery = self.delivering_execution_trace;
        self.delivering_execution_trace = false;
        let depth = self.frames.len();
        let (ns, scope, script) = if cb.kind == "fileevent" {
            (NamespaceName::root(), self.frames[0].id, cb.prefix)
        } else {
            let fi = self.scope_index();
            let mut args = words[2..].to_vec();
            if cb.kind == "variable" && args.len() == 3 {
                if let Some(access) = self
                    .variable_accesses
                    .last()
                    .filter(|a| a.depth == self.host_callback_depth)
                {
                    args[0] = VmObj::Bytes(access.name.clone());
                    if let Some(element) = &access.element {
                        args[1] = VmObj::Bytes(element.clone());
                    }
                } else {
                    let actual = args[0].as_string()?;
                    // The host variable is qualified (or promoted local storage),
                    // while Tcl source accesses it through a VM-local spelling.
                    if let Some(local) = self.frames[fi].compiled_locals.iter().find(|s| matches!(&s.link, Some(VarLink::HostVar {name1,..}) if name1.as_str_lossy() == actual)) {
                    args[0] = VmObj::Bytes(local.name.clone());
                } else if let Some((name, _)) = self.frames[fi].local_vars.iter_slots().find(|(_, s)| matches!(s, VarSlot::Link(VarLink::HostVar {name1,..}) if name1.as_str_lossy() == actual)) {
                    args[0] = VmObj::Bytes(name.clone());
                }
                }
            }
            let script = format!("{} {}", cb.prefix, crate::value::list_to_string(&args)?);
            (
                self.frames[fi].namespace.clone(),
                self.frames[fi].id,
                script,
            )
        };
        self.enter_generated(
            "host-callback",
            script,
            cb.site,
            ns,
            Some(scope),
            FrameKind::Dynamic,
            None,
        )?;
        self.frames.last_mut().unwrap().continuation = None;
        let allow_pause = self.callbacks_can_pause();
        self.host_callback_depth += 1;
        let outcome = self.eval_loop(depth, allow_pause);
        self.host_callback_depth -= 1;
        self.delivering_execution_trace = saved_delivery;
        if outcome.is_err() {
            self.frames.truncate(depth);
        }
        match outcome? {
            LoopExit::Done(v) => Ok(InvokeOutcome::Value(v)),
            LoopExit::Completion(c) => Ok(InvokeOutcome::Raise(c)),
            LoopExit::Paused => Err(VmError::Internal("callback escaped continuation".into())),
        }
    }
    fn callback_command_name(&mut self, name: &str) -> Result<String, VmError> {
        let ns = self.frames[self.fi()].namespace.clone();
        if let CommandRef::VmProc(proc) =
            self.registry
                .resolve(&ByteString::from(name), ns.as_bytes(), self.mode)
        {
            return Ok(proc.qname.as_str_lossy().into_owned());
        }
        self.generated_leaf(&[
            VmObj::from_str("namespace"),
            VmObj::from_str("eval"),
            VmObj::Bytes(ns.as_bytes().clone()),
            VmObj::from_str(&crate::value::list_to_string(&[
                VmObj::from_str("namespace"),
                VmObj::from_str("which"),
                VmObj::from_str("-command"),
                VmObj::from_str(name),
            ])?),
        ])?
        .as_string()
    }
    pub(super) fn generated_host_callback_command(
        &mut self,
        values: &[String],
        words: &[VmObj],
        name: &str,
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        let result = match name {
            "rename" if values.len() == 3 => {
                let old = self.callback_command_name(&values[1])?;
                let new = if values[2].is_empty() {
                    None
                } else {
                    Some(
                        self.frames[self.fi()]
                            .namespace
                            .qualify(&ByteString::from(values[2].as_str())),
                    )
                };
                let real = [
                    VmObj::from_str("rename"),
                    VmObj::from_str(if old.is_empty() { &values[1] } else { &old }),
                    new.clone().map(VmObj::Bytes).unwrap_or_else(VmObj::empty),
                ];
                let result = self
                    .generated_leaf(&real)
                    .map_err(|error| match (error, &new) {
                        // Name the target as the program wrote it, not qualified.
                        (
                            VmError::TclError {
                                message,
                                error_info,
                                error_code,
                            },
                            Some(qualified),
                        ) => VmError::TclError {
                            message: message.replace(
                                &format!("\"{}\"", qualified.as_str_lossy()),
                                &format!("\"{}\"", values[2]),
                            ),
                            error_info,
                            error_code,
                        },
                        (error, _) => error,
                    })?;
                for callback in self.host_callbacks.values_mut().filter(|cb| {
                    cb.kind != "variable" && cb.kind != "fileevent" && cb.target == old
                }) {
                    if let Some(new) = &new {
                        callback.target = new.as_str_lossy().into_owned();
                    }
                }
                for task in self.coroutines.values_mut().filter(|task| task.name == old) {
                    if let Some(new) = &new {
                        task.name = new.as_str_lossy().into_owned();
                    }
                }
                self.registry
                    .rename_vm_proc(&ByteString::from(old.as_str()), new.clone());
                self.image_stub_commands.remove(&old);
                if let Some(new) = new {
                    self.image_stub_commands
                        .insert(new.as_str_lossy().into_owned());
                }
                result
            }
            "fileevent" | "chan"
                if name == "fileevent" || values.get(1).map(String::as_str) == Some("event") =>
            {
                let offset = if name == "chan" { 2 } else { 1 };
                if !(values.len() == offset + 2 || values.len() == offset + 3) {
                    return self
                        .generated_leaf(words)
                        .map(|v| Some(InvokeOutcome::Value(v)));
                }
                let channel = &values[offset];
                let event = &values[offset + 1];
                let mut real = vec![
                    VmObj::from_str("fileevent"),
                    VmObj::from_str(channel),
                    VmObj::from_str(event),
                ];
                if let Some(script) = values.get(offset + 2) {
                    let prefix = if script.is_empty() {
                        VmObj::empty()
                    } else {
                        let id = self.callback_id(
                            script.clone(),
                            site,
                            "fileevent".into(),
                            channel.clone(),
                            event.clone(),
                        );
                        Self::callback_prefix(id)
                    };
                    real.push(prefix);
                }
                let out = self.generated_leaf(&real)?;
                self.original_callback(&out)
            }
            "trace" => {
                let mut real = words.to_vec();
                let (_action, kind, target, ops, prefix): (
                    &str,
                    &str,
                    usize,
                    Option<usize>,
                    Option<usize>,
                ) = match values.get(1).map(String::as_str) {
                    Some("add" | "remove") if values.len() == 6 => {
                        (&values[1], &values[2], 3, Some(4), Some(5))
                    }
                    Some("info") if values.len() == 4 => (&values[1], &values[2], 3, None, None),
                    _ => {
                        return self
                            .generated_leaf(words)
                            .map(|v| Some(InvokeOutcome::Value(v)));
                    }
                };
                let target_name = if kind == "variable" {
                    self.trace_target(&values[target])?
                } else {
                    let resolved = self.callback_command_name(&values[target])?;
                    if resolved.is_empty() {
                        values[target].clone()
                    } else {
                        resolved
                    }
                };
                real[target] = VmObj::from_str(&target_name);
                if let (Some(ops), Some(prefix)) = (ops, prefix) {
                    // Reuse the exact registration prefix for duplicate add and
                    // remove; real Tcl retains its native matching semantics.
                    let operations = {
                        let mut list = words[ops]
                            .as_list_vec()?
                            .into_iter()
                            .map(|v| v.as_string())
                            .collect::<Result<Vec<_>, _>>()?;
                        list.sort();
                        list.join(" ")
                    };
                    let id = self.callback_id(
                        values[prefix].clone(),
                        site,
                        kind.into(),
                        target_name,
                        operations,
                    );
                    real[prefix] = Self::callback_prefix(id);
                    let result = self.generated_leaf(&real)?;
                    #[cfg(feature = "host")]
                    if kind == "execution" {
                        self.inspection_bridge(|host| {
                            host.track_execution_command(&values[target])
                        });
                    }
                    result
                } else {
                    let info = self.generated_leaf(&real)?.as_list_vec()?;
                    let mut out = Vec::new();
                    for item in info {
                        let mut pair = item.as_list_vec()?;
                        if pair.len() == 2 {
                            pair[1] = self.original_callback(&pair[1]);
                        }
                        let mut args = vec![VmObj::from_str("list")];
                        args.extend(pair);
                        out.push(self.generated_leaf(&args)?);
                    }
                    let mut args = vec![VmObj::from_str("list")];
                    args.extend(out);
                    self.generated_leaf(&args)?
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(InvokeOutcome::Value(result)))
    }
    pub(super) fn clear_host_callbacks(&mut self) {
        let callbacks = self
            .host_callbacks
            .iter()
            .map(|(id, cb)| (*id, cb.clone()))
            .collect::<Vec<_>>();
        for (id, cb) in callbacks {
            let args = if cb.kind == "fileevent" {
                vec![
                    VmObj::from_str("fileevent"),
                    VmObj::from_str(&cb.target),
                    VmObj::from_str(&cb.operations),
                    VmObj::empty(),
                ]
            } else {
                let ops = if cb.operations.chars().all(|c| "rwu".contains(c)) {
                    cb.operations
                        .chars()
                        .map(|c| match c {
                            'r' => "read",
                            'w' => "write",
                            _ => "unset",
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    cb.operations
                };
                vec![
                    VmObj::from_str("trace"),
                    VmObj::from_str("remove"),
                    VmObj::from_str(&cb.kind),
                    VmObj::from_str(&cb.target),
                    VmObj::from_str(&ops),
                    Self::callback_prefix(id),
                ]
            };
            let _ = self.generated_leaf(&args);
        }
        self.host_callbacks.clear();
        for namespace in std::mem::take(&mut self.trace_scopes).into_values() {
            let _ = self.generated_leaf(&[
                VmObj::from_str("namespace"),
                VmObj::from_str("delete"),
                VmObj::from_str(&namespace),
            ]);
        }
    }
    pub(super) fn pump_tcl_event(&mut self, idle: bool) -> Result<bool, VmError> {
        #[cfg(feature = "host")]
        {
            self.inspection_bridge(|_| unsafe {
                tcl_sys::Tcl_DoOneEvent(tcl_sys::TCL_DONT_WAIT | if idle { 32 } else { 0 }) != 0
            })
            .ok_or_else(|| VmError::HybridUnavailable("no Tcl notifier".into()))
        }
        #[cfg(not(feature = "host"))]
        {
            let _ = idle;
            Ok(false)
        }
    }
    pub(super) fn has_file_events(&mut self) -> bool {
        let events = self
            .host_callbacks
            .values()
            .filter(|cb| cb.kind == "fileevent")
            .map(|cb| (cb.target.clone(), cb.operations.clone()))
            .collect::<Vec<_>>();
        events.into_iter().any(|(channel, event)| {
            self.generated_leaf(&[
                VmObj::from_str("fileevent"),
                VmObj::from_str(&channel),
                VmObj::from_str(&event),
            ])
            .is_ok_and(|v| v.as_string().is_ok_and(|s| !s.is_empty()))
        })
    }
}
