//! Runtime-created source is compiled into the same verified images and enters
//! the scheduler as an execution frame sharing its caller's Tcl variable scope.
use super::*;
use tcld_compile::{CompileMode, DebugCodeUnitKind, DebugImage, compile_source_in_context};

pub(super) struct PendingEvent {
    script: String,
    site: CodeAddress,
    due: std::time::Instant,
    idle: bool,
}
impl HybridEngineInner {
    /// The line in the current mapped body that produced an escaping error.
    pub(super) fn mapped_error_line(&self, producing_pc: u32) -> usize {
        self.source_images
            .get(&self.frames[self.fi()].code.id)
            .and_then(|source| {
                source.image.units[source.unit_index]
                    .commands
                    .iter()
                    .filter(|map| map.pc_range.contains(&producing_pc))
                    .max_by_key(|map| map.nesting_depth)
                    .map(|map| {
                        source.image.source.content[..map.source_span.start]
                            .bytes()
                            .filter(|byte| *byte == b'\n')
                            .count()
                            + 1
                    })
            })
            .unwrap_or(1)
    }

    pub(crate) fn debug_sources(&self) -> Vec<crate::DebugSource> {
        self.source_images.values().cloned().collect()
    }

    pub(super) fn register_source_image(
        &mut self,
        image: Arc<DebugImage>,
        origin: Option<CodeAddress>,
    ) -> Result<Arc<LoadedCode>, VmError> {
        self.register_source_image_in_namespace(image, origin, &NamespaceName::root())
    }
    fn register_source_image_in_namespace(
        &mut self,
        image: Arc<DebugImage>,
        origin: Option<CodeAddress>,
        namespace: &NamespaceName,
    ) -> Result<Arc<LoadedCode>, VmError> {
        image.verify().map_err(VmError::Decode)?;
        let codes = image
            .units
            .iter()
            .map(|unit| LoadedCode::from_bytecode(unit.bytecode.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let top = Arc::clone(&codes[image.top_level_unit]);
        for (index, (unit, code)) in image.units.iter().zip(codes).enumerate() {
            self.source_images.insert(
                code.id,
                crate::DebugSource {
                    code_id: code.id,
                    image: Arc::clone(&image),
                    unit_index: index,
                    origin,
                },
            );
            if let DebugCodeUnitKind::Procedure {
                qname: _,
                args_spec,
                declaration_span,
            } = &unit.kind
            {
                let declaration = image
                    .top_level()
                    .commands
                    .iter()
                    .find(|map| map.source_span == *declaration_span)
                    .ok_or_else(|| VmError::Decode("missing generated proc activation".into()))?;
                self.pending_image_procs.insert(
                    CodeAddress {
                        code_id: top.id,
                        pc: declaration.trace_pc + 1,
                    },
                    crate::cmd::VmProc {
                        qname: namespace.qualify(&ByteString::from(
                            tcld_compile::procedure_declaration_name(
                                &image.source.content,
                                declaration_span.clone(),
                            )
                            .map_err(VmError::Decode)?
                            .as_str(),
                        )),
                        args_spec: ByteString::from(args_spec.as_str()),
                        body: Some(ByteString::from(
                            tcld_compile::procedure_declaration_body(
                                &image.source.content,
                                declaration_span.clone(),
                            )
                            .map_err(VmError::Decode)?
                            .as_str(),
                        )),
                        num_args: code.bytecode.proc_num_args,
                        num_locals: code.bytecode.proc_num_locals,
                        code,
                    },
                );
            }
        }
        Ok(top)
    }

    pub(super) fn invoke_generated(
        &mut self,
        words: &[VmObj],
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        let values = words
            .iter()
            .map(VmObj::as_string)
            .collect::<Result<Vec<_>, _>>()?;
        let name = if values[0].starts_with("::") {
            values[0].trim_start_matches(':')
        } else {
            &values[0]
        };
        if let Some(subcommand) = name
            .strip_prefix("tcl::dict::")
            .filter(|subcommand| tcld_compile::dict_subcommand(subcommand) == Some(*subcommand))
        {
            let mut canonical = vec![VmObj::from_str("dict"), VmObj::from_str(subcommand)];
            canonical.extend_from_slice(&words[1..]);
            return self.invoke_generated(&canonical, site);
        }
        tcld_compile::validate_runtime_invocation(&values, CompileMode::Hybrid)
            .map_err(dynamic_error)?;
        if let Some(outcome) = self.invoke_context(&values, words, name, site)? {
            return Ok(Some(outcome));
        }
        if !matches!(
            name,
            "eval" | "expr" | "if" | "while" | "for" | "foreach" | "catch" | "proc"
        ) {
            return Ok(None);
        }
        if !self.policy.allow_runtime_compile {
            return Err(VmError::PolicyDenied {
                action: "runtime_compile",
                detail: "runtime source compilation is disabled".into(),
            });
        }
        let namespace = self.frames[self.fi()].namespace.clone();
        let source = match name {
            "eval" => {
                if words.len() < 2 {
                    return Err(dynamic_error("wrong # args: should be eval arg ?arg ...?"));
                }
                self.generated_concat(&words[1..])?
            }
            "expr" => {
                if words.len() < 2 {
                    return Err(dynamic_error("wrong # args: should be expr arg ?arg ...?"));
                }
                let expression = values[1..].join(" ");
                let quoted = tcld_compile::quote_word(&expression);
                if !quoted.starts_with('{') {
                    return Err(dynamic_error(
                        "generated expression has unsupported grouping",
                    ));
                }
                format!("expr {quoted}")
            }
            "proc" => {
                if words.len() != 4 {
                    return Err(dynamic_error("wrong # args: should be proc name args body"));
                }
                if !tcld_compile::quote_word(&values[2]).starts_with('{')
                    || !tcld_compile::quote_word(&values[3]).starts_with('{')
                {
                    return Err(dynamic_error(
                        "generated procedure argument list and body require balanced Tcl grouping",
                    ));
                }
                let qname = namespace.qualify(&ByteString::from(values[1].as_str()));
                format!(
                    "proc {} {} {}",
                    tcld_compile::quote_word(&qname.as_str_lossy()),
                    tcld_compile::quote_word(&values[2]),
                    tcld_compile::quote_word(&values[3])
                )
            }
            _ => values
                .iter()
                .map(|value| tcld_compile::quote_word(value))
                .collect::<Vec<_>>()
                .join(" "),
        };
        let scope = self.frames[self.scope_index()].id;
        self.enter_generated(
            name,
            source,
            site,
            namespace,
            Some(scope),
            FrameKind::Dynamic,
            None,
        )
        .map(Some)
    }
    /// The script of `eval`-style arguments: one argument as is, several
    /// joined as `concat` joins them.
    fn generated_concat(&mut self, words: &[VmObj]) -> Result<String, VmError> {
        if words.len() == 1 {
            return words[0].as_string();
        }
        crate::lists::concat(words)?.as_string()
    }

    pub(super) fn generated_leaf(&mut self, words: &[VmObj]) -> Result<VmObj, VmError> {
        match self.host_value_result(words)? {
            Ok(value) => Ok(value),
            Err(completion) => Err(completion_to_error(completion)),
        }
    }

    // Generated images hold Rc-backed bytecode; Arc is the VM's sharing
    // handle, not a promise of cross-thread use.
    #[allow(clippy::arc_with_non_send_sync, clippy::too_many_arguments)]
    pub(super) fn enter_generated(
        &mut self,
        name: &str,
        source: String,
        site: CodeAddress,
        namespace: NamespaceName,
        scope: Option<FrameId>,
        kind: FrameKind,
        path: Option<std::path::PathBuf>,
    ) -> Result<InvokeOutcome, VmError> {
        if !self.policy.allow_runtime_compile {
            return Err(dynamic_error("runtime source compilation is disabled"));
        }
        let procedure_context = kind == FrameKind::Lambda
            || scope.is_some_and(|id| {
                self.frames.iter().any(|frame| {
                    frame.id == id && matches!(frame.kind, FrameKind::VmProc | FrameKind::Lambda)
                })
            });
        let key = (
            self.command_callback_sites
                .get(&site.code_id)
                .copied()
                .unwrap_or(site),
            format!(
                "{name}@{}@{}@{procedure_context}",
                namespace.as_bytes().as_str_lossy(),
                path.as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
            source.clone(),
        );
        let code = if let Some(code) = self.generated_cache.get(&key) {
            Arc::clone(code)
        } else {
            let parent = self.source_images.get(&site.code_id);
            let mode = parent
                .map(|source| source.image.mode)
                .unwrap_or(match self.mode {
                    RuntimeMode::VmStrict => CompileMode::VmStrict,
                    RuntimeMode::Hybrid => CompileMode::Hybrid,
                });
            let label = path.unwrap_or_else(|| {
                parent
                    .and_then(|source| source.image.source.path.parent())
                    .unwrap_or_else(|| Path::new("."))
                    .join(".tcld-generated")
                    .join(format!(
                        "{}-{}.tcl",
                        self.generated_cache.len() + 1,
                        name.replace(':', "-")
                    ))
            });
            let image = compile_source_in_context(label, source, mode, procedure_context).map_err(
                |error| {
                    dynamic_error(format!(
                        "generated {name} source at code {} pc {}: {error}",
                        site.code_id.0, site.pc
                    ))
                },
            )?;
            let code =
                self.register_source_image_in_namespace(Arc::new(image), Some(site), &namespace)?;
            self.generated_cache.insert(key, Arc::clone(&code));
            code
        };
        let source = &self.source_images[&code.id];
        let helper = matches!(name, "update" | "vwait" | "callback")
            || (matches!(name, "apply" | "apply-body")
                && self
                    .frames
                    .last()
                    .is_some_and(|f| self.helper_trace_units.contains(&f.code.id)));
        let root_only = matches!(
            name,
            "expr" | "if" | "while" | "for" | "foreach" | "catch" | "proc" | "apply"
        );
        for source in self
            .source_images
            .values()
            .filter(|s| Arc::ptr_eq(&s.image, &source.image))
        {
            let unit = &source.image.units[source.unit_index];
            if helper {
                self.helper_trace_units.insert(source.code_id);
            }
            for map in &unit.commands {
                if helper || (root_only && source.code_id == code.id && map.nesting_depth == 0) {
                    self.hidden_trace_commands.insert(CodeAddress {
                        code_id: source.code_id,
                        pc: map.anchor_pc,
                    });
                }
            }
        }
        self.push_call_frame(code, kind, None, &[], namespace, site)?;
        if let Some(scope) = scope {
            self.frames.last_mut().unwrap().scope_parent = Some(scope);
        }
        Ok(InvokeOutcome::FramePushed)
    }

    fn clear_wait_trace(&mut self, watch: &[VmObj]) {
        if watch.len() != 3 {
            return;
        }
        let _ = self.generated_leaf(&[
            VmObj::from_str("trace"),
            VmObj::from_str("remove"),
            VmObj::from_str("variable"),
            watch[0].clone(),
            VmObj::from_str("write unset"),
            watch[2].clone(),
        ]);
        let _ = self.generated_leaf(&[VmObj::from_str("unset"), watch[1].clone()]);
    }

    fn invoke_context(
        &mut self,
        values: &[String],
        words: &[VmObj],
        name: &str,
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        let ns = self.frames[self.fi()].namespace.clone();
        let scope = self.frames[self.scope_index()].id;
        let q = tcld_compile::quote_word;
        let value = |s: &str| Ok(Some(InvokeOutcome::Value(VmObj::from_str(s))));
        if self.mode == RuntimeMode::VmStrict {
            // Without a host, only the commands the VM runs on its own
            // scopes, namespaces and generated images remain; the rest
            // resolve as ordinary commands and are denied.
            if !matches!(name, "global" | "namespace" | "upvar" | "uplevel") {
                return Ok(None);
            }
        } else {
            if let Some(outcome) =
                self.generated_host_callback_command(values, words, name, site)?
            {
                return Ok(Some(outcome));
            }
            if let Some(outcome) = self.invoke_native_control(words, name, site)? {
                return Ok(Some(outcome));
            }
            if let Some(outcome) = self.invoke_tailcall(words, name, site)? {
                return Ok(Some(outcome));
            }
            if let Some(outcome) = self.invoke_coroutine(words, name, site)? {
                return Ok(Some(outcome));
            }
        }
        match name {
            "global"
                if values.len() > 1
                    && matches!(
                        self.frames[self.scope_index()].kind,
                        FrameKind::Toplevel | FrameKind::Namespace
                    ) =>
            {
                value("")
            }
            "info"
                if values.get(1).map(String::as_str) == Some("locals")
                    && matches!(values.len(), 2 | 3) =>
            {
                let fi = self.scope_index();
                if matches!(
                    self.frames[fi].kind,
                    FrameKind::Toplevel | FrameKind::Namespace
                ) {
                    return value("");
                }
                let mut candidates = std::collections::HashMap::new();
                for local in &self.frames[fi].compiled_locals {
                    candidates.insert(local.name.clone(), local.link.clone());
                }
                for (name, slot) in self.frames[fi].local_vars.iter_slots() {
                    candidates.insert(
                        name.clone(),
                        match slot {
                            VarSlot::Link(link) => Some(link.clone()),
                            _ => None,
                        },
                    );
                }
                let own_storage = self.trace_scopes.get(&self.frames[fi].id.0).cloned();
                let mut names = Vec::new();
                for (name, link) in candidates {
                    if values
                        .get(2)
                        .is_some_and(|pattern| !glob_match(pattern, &name.as_str_lossy(), false))
                    {
                        continue;
                    }
                    let exists = match link {
                        Some(VarLink::HostVar { name1, name2: None })
                            if own_storage.as_ref().is_some_and(|ns| {
                                name1.as_str_lossy().starts_with(format!("{ns}::").as_str())
                            }) =>
                        {
                            #[cfg(feature = "host")]
                            {
                                self.execution_host(|host| {
                                    Ok(host.variable_defined(&name1.as_str_lossy()))
                                })?
                            }
                            #[cfg(not(feature = "host"))]
                            {
                                false
                            }
                        }
                        Some(_) => false,
                        None => {
                            self.frames[fi].local_vars.get_slot(&name).is_some_and(|s| {
                                matches!(s, VarSlot::Scalar(_) | VarSlot::Array(_))
                            }) || self.frames[fi]
                                .compiled_locals
                                .iter()
                                .any(|s| s.is_named(&name) && s.value.is_some())
                        }
                    };
                    if exists {
                        names.push(VmObj::Bytes(name));
                    }
                }
                Ok(Some(InvokeOutcome::Value(VmObj::List(names))))
            }
            "info"
                if values.get(1).map(String::as_str) == Some("level")
                    && values.get(2).map(String::as_str) == Some("0")
                    && values.len() == 3
                    && self.frames[self.scope_index()].invocation.is_some() =>
            {
                Ok(Some(InvokeOutcome::Value(
                    self.frames[self.scope_index()].invocation.clone().unwrap(),
                )))
            }
            "namespace"
                if values.get(1).map(String::as_str) == Some("current") && values.len() == 2 =>
            {
                value(&ns.as_bytes().as_str_lossy())
            }
            "info" if values.get(1).map(String::as_str) == Some("level") && values.len() == 2 => {
                value(&self.tcl_scope_chain().len().saturating_sub(1).to_string())
            }
            "info" if values.get(1).map(String::as_str) == Some("script") && values.len() == 2 => {
                let path = self
                    .frames
                    .iter()
                    .rev()
                    .find_map(|f| f.script_path.clone())
                    .unwrap_or_else(|| {
                        self.source_images[&site.code_id]
                            .image
                            .source
                            .path
                            .display()
                            .to_string()
                    });
                value(&path)
            }
            "source" => {
                let (filename, encoding) = match values.len() {
                    2 => (values[1].clone(), None),
                    4 if values[1] == "-encoding" => (values[3].clone(), Some(values[2].clone())),
                    _ => {
                        return Err(dynamic_error(
                            "wrong # args: should be source ?-encoding name? fileName",
                        ));
                    }
                };
                let path = self
                    .generated_leaf(&[
                        VmObj::from_str("file"),
                        VmObj::from_str("normalize"),
                        VmObj::from_str(&filename),
                    ])?
                    .as_string()?;
                let channel = self.generated_leaf(&[
                    VmObj::from_str("open"),
                    VmObj::from_str(&path),
                    VmObj::from_str("r"),
                ])?;
                let contents = (|| {
                    if let Some(encoding) = encoding {
                        self.generated_leaf(&[
                            VmObj::from_str("fconfigure"),
                            channel.clone(),
                            VmObj::from_str("-encoding"),
                            VmObj::from_str(&encoding),
                        ])?;
                    }
                    self.generated_leaf(&[VmObj::from_str("read"), channel.clone()])?
                        .as_string()
                })();
                let closed = self.generated_leaf(&[VmObj::from_str("close"), channel]);
                let text = contents?;
                closed?;
                // Tcl source ignores a BOM and treats control-Z as end of file.
                let text = text
                    .trim_start_matches('\u{feff}')
                    .split('\u{1a}')
                    .next()
                    .unwrap_or("")
                    .to_string();
                let outcome = self.enter_generated(
                    "source",
                    text,
                    site,
                    ns,
                    Some(scope),
                    FrameKind::Source,
                    Some(path.into()),
                )?;
                self.frames.last_mut().unwrap().script_path = Some(filename);
                Ok(Some(outcome))
            }
            "upvar" => {
                let explicit = values.get(1).is_some_and(|s| {
                    s.starts_with('#') || s.chars().next().is_some_and(|c| c.is_ascii_digit())
                });
                let start = if explicit { 2 } else { 1 };
                if values.len() <= start || !(values.len() - start).is_multiple_of(2) {
                    return Err(dynamic_error(
                        "wrong # args: should be \"upvar ?level? otherVar localVar ?otherVar localVar ...?\"",
                    ));
                }
                let target = self.resolve_vm_level(
                    &ByteString::from(if explicit { values[1].as_str() } else { "1" }),
                    true,
                )?;
                let current = self.scope_index();
                for pair in values[start..].chunks_exact(2) {
                    let (other, element) = split_var_name(&ByteString::from(pair[0].as_str()));
                    let local = ByteString::from(pair[1].as_str());
                    if pair[1].ends_with(')') && pair[1].contains('(') {
                        return Err(dynamic_error(
                            "bad variable name: can't create a scalar variable that looks like an array element",
                        ));
                    }
                    if target == current && other == local {
                        return Err(dynamic_error("can't upvar from variable to itself"));
                    }
                    let occupied = self.frames[current]
                        .compiled_locals
                        .iter()
                        .find(|s| s.is_named(&local))
                        .is_some_and(|s| s.value.is_some())
                        || matches!(
                            self.frames[current].local_vars.get_slot(&local),
                            Some(VarSlot::Scalar(_) | VarSlot::Array(_))
                        );
                    if occupied {
                        return Err(dynamic_error(format!(
                            "variable \"{}\" already exists",
                            pair[1]
                        )));
                    }
                    let mut cursor = Some(VarLink::Vm {
                        target_frame: self.frames[target].id,
                        target_name: other.clone(),
                        part2: element.clone(),
                    });
                    let mut visited = std::collections::HashSet::new();
                    while let Some(VarLink::Vm {
                        target_frame,
                        target_name,
                        ..
                    }) = cursor
                    {
                        if (target_frame == self.frames[current].id && target_name == local)
                            || !visited.insert((target_frame.0, target_name.clone()))
                        {
                            return Err(dynamic_error("can't upvar from variable to itself"));
                        }
                        let frame = self
                            .frames
                            .iter()
                            .find(|f| f.id == target_frame)
                            .ok_or(VmError::CrossRuntimeUpvar)?;
                        cursor = frame
                            .compiled_locals
                            .iter()
                            .find(|s| s.is_named(&target_name))
                            .and_then(|s| s.link.clone())
                            .or_else(|| match frame.local_vars.get_slot(&target_name) {
                                Some(VarSlot::Link(l)) => Some(l.clone()),
                                _ => None,
                            });
                    }
                    let link = if other.as_str_lossy().contains("::")
                        || matches!(
                            self.frames[target].kind,
                            FrameKind::Toplevel | FrameKind::Namespace
                        ) {
                        VarLink::HostVar {
                            name1: self.frames[target].namespace.qualify(&other),
                            name2: element,
                        }
                    } else {
                        VarLink::Vm {
                            target_frame: self.frames[target].id,
                            target_name: other,
                            part2: element,
                        }
                    };
                    if let Some(slot) = self.frames[current]
                        .compiled_locals
                        .iter_mut()
                        .find(|s| s.is_named(&local))
                    {
                        slot.value = None;
                        slot.link = Some(link.clone());
                    }
                    self.frames[current].local_vars.install_link(local, link);
                }
                value("")
            }
            "uplevel" => {
                if values.len() < 2 {
                    return Err(dynamic_error(
                        "wrong # args: should be uplevel ?level? command ?arg ...?",
                    ));
                }
                let explicit = values[1].starts_with('#')
                    || values[1].chars().next().is_some_and(|c| c.is_ascii_digit());
                let (level, start) = if explicit {
                    (values[1].as_str(), 2)
                } else {
                    ("1", 1)
                };
                if start == values.len() {
                    return Err(dynamic_error(
                        "wrong # args: should be uplevel ?level? command ?arg ...?",
                    ));
                }
                let target = self.resolve_vm_level(&ByteString::from(level), false)?;
                let text = self.generated_concat(&words[start..])?;
                self.enter_generated(
                    "uplevel",
                    text,
                    site,
                    self.frames[target].namespace.clone(),
                    Some(self.frames[target].id),
                    FrameKind::Dynamic,
                    None,
                )
                .map(Some)
            }
            "namespace" => {
                match values.get(1).map(String::as_str) {
                    Some("current") if values.len() == 2 => value(&ns.as_bytes().as_str_lossy()),
                    Some("eval" | "inscope") => {
                        if values.len() < 4 {
                            return Err(dynamic_error(
                                "wrong # args: should be namespace eval|inscope name arg ?arg ...?",
                            ));
                        }
                        let namespace =
                            NamespaceName(ns.qualify(&ByteString::from(values[2].as_str())));
                        let namespace = if self.mode == RuntimeMode::VmStrict {
                            let canonical = crate::world::canonical(&namespace.0);
                            if values[1] == "eval" {
                                self.world.create_namespace(&canonical);
                            } else if !self.world.namespace_exists(&canonical) {
                                return Err(dynamic_error(format!(
                                    "namespace {} not found",
                                    values[2]
                                )));
                            }
                            NamespaceName(canonical)
                        } else {
                            if values[1] == "eval" {
                                self.generated_leaf(&[
                                    VmObj::from_str("namespace"),
                                    VmObj::from_str("eval"),
                                    VmObj::Bytes(namespace.0.clone()),
                                    VmObj::empty(),
                                ])?;
                            } else {
                                let exists = self
                                    .generated_leaf(&[
                                        VmObj::from_str("namespace"),
                                        VmObj::from_str("exists"),
                                        VmObj::Bytes(namespace.0.clone()),
                                    ])?
                                    .as_string()?;
                                if exists == "0" {
                                    return Err(dynamic_error(format!(
                                        "namespace {} not found",
                                        values[2]
                                    )));
                                }
                            }
                            NamespaceName(
                                self.generated_leaf(&[
                                    VmObj::from_str("namespace"),
                                    VmObj::from_str("eval"),
                                    VmObj::Bytes(namespace.0),
                                    VmObj::from_str("namespace current"),
                                ])?
                                .ensure_bytes()?,
                            )
                        };
                        let text = if values[1] == "inscope" && values.len() > 4 {
                            // inscope appends arguments as list elements, without concat flattening.
                            let suffix = crate::value::list_to_string(&words[4..])?;
                            format!("{} {suffix}", values[3])
                        } else {
                            self.generated_concat(&words[3..])?
                        };
                        let outcome = self.enter_generated(
                            "namespace",
                            text,
                            site,
                            namespace,
                            None,
                            FrameKind::Namespace,
                            None,
                        )?;
                        self.frames.last_mut().unwrap().invocation =
                            Some(VmObj::List(words.to_vec()));
                        Ok(Some(outcome))
                    }
                    Some("code") => {
                        if values.len() != 3 {
                            return Err(dynamic_error(
                                "wrong # args: should be namespace code arg",
                            ));
                        }
                        if values[2].starts_with("::namespace inscope ") {
                            return value(&values[2]);
                        }
                        value(&format!(
                            "::namespace inscope {} {}",
                            q(&ns.as_bytes().as_str_lossy()),
                            q(&values[2])
                        ))
                    }
                    _ => Ok(None),
                }
            }
            "after" => self.generated_after(values, words, site),
            "update" | "vwait" => {
                let mut mode = "all".to_string();
                let mut watch = String::new();
                if name == "update" {
                    if values.len() == 2 && values[1] == "idletasks" {
                        mode = "idle".into();
                    } else if values.len() != 1 {
                        return Err(dynamic_error("wrong # args: should be update ?idletasks?"));
                    }
                } else {
                    if values.len() != 2 {
                        return Err(dynamic_error("wrong # args: should be vwait name"));
                    }
                    mode = "wait".into();
                    let target =
                        NamespaceName::root().qualify(&ByteString::from(values[1].as_str()));
                    let counter = format!("::tcld::__wait_{}", self.next_event);
                    self.next_event += 1;
                    let prefix = crate::value::list_to_string(&[
                        VmObj::from_str("apply"),
                        VmObj::from_str("{counter name element op} {incr $counter}"),
                        VmObj::from_str(&counter),
                    ])?;
                    self.host_set(
                        &ByteString::from(counter.as_str()),
                        None,
                        VmObj::from_int(0),
                    )?;
                    self.generated_leaf(&[
                        VmObj::from_str("trace"),
                        VmObj::from_str("add"),
                        VmObj::from_str("variable"),
                        VmObj::Bytes(target.clone()),
                        VmObj::from_str("write unset"),
                        VmObj::from_str(&prefix),
                    ])?;
                    watch = crate::value::list_to_string(&[
                        VmObj::Bytes(target),
                        VmObj::from_str(&counter),
                        VmObj::from_str(&prefix),
                    ])?;
                }
                let text = format!(
                    "while {{[::tcld::debug_event_ready {} {}]}} {{::tcld::debug_event_dispatch {}}}\nlist",
                    q(&mode),
                    q(&watch),
                    q(&mode)
                );
                self.enter_generated(name, text, site, ns, Some(scope), FrameKind::Dynamic, None)
                    .map(Some)
            }
            "tcld::debug_event_ready" => {
                if values.len() != 3 {
                    return Err(dynamic_error("invalid event continuation"));
                }
                let mode = &values[1];
                let watch = if mode == "wait" {
                    words[2].as_list_vec()?
                } else {
                    vec![]
                };
                if mode == "wait" {
                    if watch.len() != 3 {
                        return Err(dynamic_error("invalid wait continuation"));
                    }
                    if self.host_get(&watch[1].ensure_bytes()?, None)?.is_true()? {
                        self.clear_wait_trace(&watch);
                        return value("0");
                    }
                }
                loop {
                    let handled = self.pump_tcl_event(mode == "idle")?;
                    if mode == "wait"
                        && self.host_get(&watch[1].ensure_bytes()?, None)?.is_true()?
                    {
                        self.clear_wait_trace(&watch);
                        return value("0");
                    }
                    if handled {
                        return value("1");
                    }
                    let now = std::time::Instant::now();
                    if self
                        .events
                        .values()
                        .any(|e| (mode != "idle" || e.idle) && e.due <= now)
                    {
                        return value("1");
                    }
                    if mode != "wait" {
                        return value("0");
                    }
                    let due = self.events.values().map(|e| e.due).min();
                    if due.is_none() && !self.has_file_events() {
                        self.clear_wait_trace(&watch);
                        return Err(dynamic_error(
                            "can't wait for variable: would wait forever; no pending browser events",
                        ));
                    }
                    let millis = due
                        .unwrap_or(now + std::time::Duration::from_millis(10))
                        .saturating_duration_since(now)
                        .as_millis()
                        .clamp(1, 50)
                        .to_string();
                    self.generated_leaf(&[VmObj::from_str("after"), VmObj::from_str(&millis)])?;
                }
            }
            "tcld::debug_event_dispatch" => {
                if values.len() != 2 {
                    return Err(dynamic_error("invalid event continuation"));
                }
                let now = std::time::Instant::now();
                let id = self
                    .events
                    .iter()
                    .filter(|(_, e)| (values[1] != "idle" || e.idle) && e.due <= now)
                    .min_by_key(|(id, e)| (e.idle, e.due, **id))
                    .map(|(id, _)| *id);
                let Some(id) = id else { return value("") };
                let event = self.events.remove(&id).unwrap();
                // A procedure gives the event handler private completion bookkeeping;
                // the actual callback runs in the global scope, including namespace code.
                let text = format!(
                    "apply {{{{script}} {{set code [catch {{uplevel #0 $script}} result options]; if {{$code != 0}} {{if {{[llength [info commands ::bgerror]]}} {{::bgerror $result}} else {{puts stderr $result}}}}; return {{}}}} ::}} {}",
                    q(&event.script)
                );
                let outcome = self.enter_generated(
                    "callback",
                    text,
                    event.site,
                    NamespaceName::root(),
                    Some(self.frames[0].id),
                    FrameKind::Dynamic,
                    None,
                )?;
                // Source provenance points to registration; completion routing
                // must instead point at the currently suspended event dispatcher.
                if let Some(continuation) = self.frames.last_mut().unwrap().continuation.as_mut() {
                    continuation.call_site = site;
                }
                Ok(Some(outcome))
            }
            _ => Ok(None),
        }
    }

    fn generated_after(
        &mut self,
        values: &[String],
        words: &[VmObj],
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        if values.len() < 2 {
            return Err(dynamic_error(
                "wrong # args: should be after option ?arg ...?",
            ));
        }
        let sub = &values[1];
        if sub == "info" {
            if values.len() == 2 {
                return Ok(Some(InvokeOutcome::Value(VmObj::from_str(
                    &self
                        .events
                        .keys()
                        .rev()
                        .map(|id| format!("after#tcld{id}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                ))));
            }
            if values.len() != 3 {
                return Err(dynamic_error("wrong # args: should be after info ?id?"));
            }
            let id = values[2]
                .strip_prefix("after#tcld")
                .and_then(|s| s.parse().ok())
                .and_then(|id| self.events.get(&id))
                .ok_or_else(|| dynamic_error("event doesn't exist"))?;
            return Ok(Some(InvokeOutcome::Value(VmObj::from_str(
                &crate::value::list_to_string(&[
                    VmObj::from_str(&id.script),
                    VmObj::from_str(if id.idle { "idle" } else { "timer" }),
                ])?,
            ))));
        }
        if sub == "cancel" {
            if values.len() < 3 {
                return Err(dynamic_error(
                    "wrong # args: should be after cancel id|command ?arg ...?",
                ));
            }
            let text = self.generated_concat(&words[2..])?;
            let id = text
                .strip_prefix("after#tcld")
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| {
                    self.events
                        .iter()
                        .find(|(_, e)| e.script == text)
                        .map(|(id, _)| *id)
                });
            if let Some(id) = id {
                self.events.remove(&id);
            }
            return Ok(Some(InvokeOutcome::Value(VmObj::empty())));
        }
        let idle = sub == "idle";
        let delay = if idle {
            0
        } else {
            sub.parse::<i64>()
                .map_err(|_| {
                    dynamic_error("bad after argument: must be cancel, idle, info, or milliseconds")
                })?
                .max(0) as u64
        };
        if values.len() == 2 && !idle {
            return Ok(None);
        }
        if values.len() < 3 {
            return Err(dynamic_error(
                "wrong # args: should be after idle script ?script ...?",
            ));
        }
        let script = self.generated_concat(&words[2..])?;
        let due = std::time::Instant::now()
            .checked_add(std::time::Duration::from_millis(delay))
            .ok_or_else(|| dynamic_error("after interval out of range"))?;
        let id = self.next_event;
        self.next_event += 1;
        self.events.insert(
            id,
            PendingEvent {
                script,
                site,
                due,
                idle,
            },
        );
        Ok(Some(InvokeOutcome::Value(VmObj::from_str(&format!(
            "after#tcld{id}"
        )))))
    }
}
pub(super) fn dynamic_error(message: impl Into<String>) -> VmError {
    VmError::TclError {
        message: message.into(),
        error_info: None,
        error_code: Some("TCLD GENERATED_SOURCE".into()),
    }
}
