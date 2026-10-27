//! Native Tcl NRE control commands own matching, iteration and completions.
//! Only its script arguments are replaced with mapped source continuations.
use super::*;
pub(super) struct ControlGroup {
    pub(super) owner: FrameId,
    tasks: Vec<u64>,
    invocation: Vec<String>,
    started: bool,
}
impl HybridEngineInner {
    pub(super) fn invoke_native_control(
        &mut self,
        words: &[VmObj],
        name: &str,
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        if !matches!(name, "subst" | "time" | "dict" | "array" | "info") {
            return Ok(None);
        }
        if name == "dict"
            && !words.get(1).is_some_and(|s| {
                s.as_string()
                    .ok()
                    .as_deref()
                    .and_then(tcld_compile::dict_subcommand)
                    .is_some_and(|s| {
                        matches!(
                            s,
                            "for"
                                | "filter"
                                | "map"
                                | "update"
                                | "with"
                                | "set"
                                | "unset"
                                | "incr"
                                | "append"
                                | "lappend"
                        )
                    })
            })
        {
            return Ok(None);
        }
        if name == "info" && !(words.len() == 3 && words[1].as_string()? == "exists") {
            return Ok(None);
        }
        let scope = self.frames[self.scope_index()].id;
        let owner = self.frames[self.fi()].id;
        let namespace = self.frames[self.fi()].namespace.clone();
        let mut args = words.to_vec();
        // Identify script and binding positions only. Native Tcl still validates
        // its full grammar and chooses which mapped continuation to invoke.
        let mut scripts = Vec::new();
        let mut bindings = Vec::new();
        let mut substitutions = Vec::new();
        let mut template = None;
        let mut valid_shape = true;
        match name {
            "array" | "info" => {
                valid_shape = words.len() >= 3;
                if valid_shape {
                    bindings.push(words[2].clone());
                }
            }
            "dict" => {
                let subcommand = words[1].as_string()?;
                let subcommand = tcld_compile::dict_subcommand(&subcommand).unwrap();
                args[1] = VmObj::from_str(subcommand);
                match subcommand {
                    "for" | "map" => {
                        valid_shape = words.len() == 5;
                        if valid_shape {
                            if let Ok(vars) = words[2].as_list_vec() {
                                bindings.extend(vars);
                            }
                            scripts.push((
                                4,
                                if subcommand == "for" {
                                    "dict-for-body"
                                } else {
                                    "dict-map-body"
                                },
                            ));
                        }
                    }
                    "filter" => {
                        // Tcl's filterType uses unique nonempty prefixes. Only
                        // the script form has an executable argument; key/value
                        // patterns, invalid modes and malformed calls stay native.
                        if words.len() == 6 {
                            let mode = words[3].as_string()?;
                            if !mode.is_empty() && "script".starts_with(&mode) {
                                args[3] = VmObj::from_str("script");
                                if let Ok(vars) = words[4].as_list_vec() {
                                    bindings.extend(vars);
                                }
                                scripts.push((5, "dict-filter-body"));
                            }
                        }
                    }
                    "update" => {
                        valid_shape = words.len() >= 6 && words.len().is_multiple_of(2);
                        if valid_shape {
                            bindings.push(words[2].clone());
                            bindings
                                .extend((4..words.len() - 1).step_by(2).map(|i| words[i].clone()));
                            scripts.push((words.len() - 1, "dict-update-body"));
                        }
                    }
                    "with" => {
                        valid_shape = words.len() >= 4;
                        if valid_shape {
                            bindings.push(words[2].clone());
                            scripts.push((words.len() - 1, "dict-with-body"));
                        }
                    }
                    "set" | "unset" | "incr" | "append" | "lappend" => {
                        valid_shape = words.len() >= 3;
                        if valid_shape {
                            bindings.push(words[2].clone());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            "time" => {
                valid_shape = matches!(words.len(), 2 | 3);
                if valid_shape {
                    scripts.push((1, "time-body"));
                }
            }
            "subst" => {
                #[cfg(feature = "host")]
                let plan = self.execution_host(|host| host.subst_plan(words))?;
                #[cfg(not(feature = "host"))]
                let plan: Option<VmObj> = None;
                valid_shape = plan.is_some();
                if let Some(plan) = plan {
                    let parts = plan.as_list_vec()?;
                    template = Some(parts[0].ensure_bytes()?);
                    bindings.extend(parts[1].as_list_vec()?);
                    for command in parts[2].as_list_vec()? {
                        let command = command.as_list_vec()?;
                        substitutions
                            .push((command[0].as_i64()? as usize, command[1].as_i64()? as usize));
                        scripts.push((args.len(), "subst-command"));
                        args.push(command[2].clone());
                    }
                }
            }
            _ => unreachable!(),
        }
        let id = self.next_event;
        self.next_event += 1;
        let mut tasks = Vec::new();
        let mut aliases = Vec::new();
        if valid_shape {
            for var in bindings {
                // Native Tcl binds aliases to the caller's exact storage,
                // preserving arrays, upvar and lexical variable-trace names.
                let (base, _) = split_var_name(&var.ensure_bytes()?);
                if !base.as_str_lossy().contains("::") {
                    let target = self.trace_target(&base.as_str_lossy())?;
                    aliases.push(VmObj::Bytes(base));
                    aliases.push(VmObj::from_str(&target));
                }
            }
            for (index, kind) in scripts {
                let task = self.next_event;
                self.next_event += 1;
                self.coroutines.insert(
                    task,
                    coroutines::Coroutine {
                        script_kind: Some(kind),
                        repeat: name == "time"
                            || matches!(
                                kind,
                                "dict-for-body" | "dict-map-body" | "dict-filter-body"
                            ),
                        tail_start: false,
                        source: args[index].as_string()?,
                        site,
                        namespace: namespace.clone(),
                        frames: Vec::new(),
                        traces: Vec::new(),
                        completion: Completion::ok(VmObj::empty()),
                        name: String::new(),
                        tail_scope: Some(scope),
                        parent: self.coroutine_task,
                        owner: 0,
                        created: None,
                    },
                );
                tasks.push(task);
                args[index] = VmObj::List(vec![
                    VmObj::from_str("::tcld::coroutine_body"),
                    VmObj::from_int(task as i64),
                ]);
            }
        }
        if let Some(template) = template {
            let bytes = template.as_bytes();
            let mut rewritten = Vec::new();
            let mut cursor = 0;
            for (i, (start, length)) in substitutions.into_iter().enumerate() {
                // Native subst tokens include their surrounding delimiters.
                // The last delimiter can be a command terminator in a valid
                // prefix before a deferred parse error, so preserve it too.
                rewritten.extend_from_slice(&bytes[cursor..start + 1]);
                rewritten.extend_from_slice(args[words.len() + i].ensure_bytes()?.as_bytes());
                cursor = start + length - 1;
            }
            rewritten.extend_from_slice(&bytes[cursor..]);
            args.truncate(words.len());
            *args.last_mut().unwrap() = VmObj::from_bytes(rewritten);
        }
        let mut invocation = vec![
            VmObj::from_str("::tcld::control"),
            VmObj::from_int(id as i64),
            VmObj::Bytes(namespace.as_bytes().clone()),
            VmObj::List(aliases),
        ];
        invocation.extend(args);
        self.control_groups.insert(
            id,
            ControlGroup {
                owner,
                tasks,
                invocation: invocation[1..]
                    .iter()
                    .map(VmObj::as_string)
                    .collect::<Result<_, _>>()?,
                started: false,
            },
        );
        if self.coroutine_can_handoff() {
            self.coroutine_yield = Some(coroutines::Handoff {
                words: invocation,
                created: None,
                tail: None,
            });
            return Ok(Some(InvokeOutcome::Value(VmObj::empty())));
        }
        self.host_value_result(&invocation).map(|result| {
            Some(match result {
                Ok(v) => InvokeOutcome::Value(v),
                Err(c) => InvokeOutcome::Raise(c),
            })
        })
    }
    // Called only by Tcl's variable resolver in this group's native dict-with
    // frame. Resolve lazily: native Tcl reads the dictionary exactly once and
    // determines its keys, ordering, traces and nested-path diagnostics itself.
    #[cfg(feature = "host")]
    pub(super) fn control_variable(&mut self, words: &[VmObj]) -> Result<BridgeResult, VmError> {
        let id = words[1].as_i64()? as u64;
        let active = self.execution_host(|host| Ok(host.active_control()))?;
        let owner = self
            .control_groups
            .get(&id)
            .filter(|g| g.started && active == id)
            .map(|g| g.owner)
            .ok_or_else(|| VmError::Internal("inactive control variable resolver".into()))?;
        let parent = if self.frames.iter().any(|f| f.id == owner) {
            None
        } else {
            self.coroutines
                .iter()
                .find(|(_, c)| c.frames.iter().any(|f| f.id == owner))
                .map(|(id, _)| *id)
        };
        let borrowed = self.borrow_coroutine_contexts(parent);
        let target = self.trace_target(&words[2].as_string()?);
        self.return_coroutine_contexts(borrowed);
        Ok(BridgeResult {
            code: 0,
            result: VmObj::from_str(&target?),
            error_info: None,
            error_code: None,
            return_options: None,
        })
    }
    #[cfg(feature = "host")]
    pub(super) fn validate_control(&mut self, words: &[VmObj]) -> Result<BridgeResult, VmError> {
        let id = words.get(1).and_then(|v| v.as_i64().ok()).unwrap_or(-1) as u64;
        let arguments = words[1..]
            .iter()
            .map(VmObj::as_string)
            .collect::<Result<Vec<_>, _>>()?;
        let group = self
            .control_groups
            .get_mut(&id)
            .filter(|g| !g.started && g.invocation == arguments)
            .ok_or_else(|| VmError::TclError {
                message: "invalid mapped control continuation".into(),
                error_info: None,
                error_code: None,
            })?;
        group.started = true;
        Ok(BridgeResult {
            code: 0,
            result: VmObj::empty(),
            error_info: None,
            error_code: None,
            return_options: None,
        })
    }
    #[cfg(feature = "host")]
    pub(super) fn finish_control(&mut self, words: &[VmObj]) -> Result<BridgeResult, VmError> {
        let id = words[1].as_i64()? as u64;
        let code = words[2].as_i64()? as i32;
        if let Some(group) = self.control_groups.remove(&id) {
            for task in group.tasks {
                self.coroutines.remove(&task);
            }
            if code >= 2 {
                self.coroutine_completions.insert(
                    group.owner.0,
                    Completion {
                        code,
                        result: words[3].clone(),
                        options: ReturnOptions::from_vm_obj(words[4].clone())?,
                    },
                );
            }
        }
        Ok(BridgeResult {
            code: 0,
            result: VmObj::empty(),
            error_info: None,
            error_code: None,
            return_options: None,
        })
    }
}
