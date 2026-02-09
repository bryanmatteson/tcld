use super::*;

fn command_needs_following_blank_line(kind: &CommandKind) -> bool {
    match kind {
        CommandKind::ProcCall { args } => args
            .first()
            .is_some_and(|head| head.repr_eq_str("proc") || head.repr_eq_str("tbcload::bcproc")),
        CommandKind::IfElse { .. }
        | CommandKind::Switch { .. }
        | CommandKind::WhileLoop { .. }
        | CommandKind::ForLoop { .. }
        | CommandKind::Foreach { .. }
        | CommandKind::Catch { .. }
        | CommandKind::DictUpdate { .. } => true,
        _ => false,
    }
}

fn rendered_command_body_children(code: CodeRef) -> Vec<CodeRef> {
    let borrowed = code.borrow();
    let Code::Command(cmd) = &*borrowed else {
        return Vec::new();
    };
    let mut children = Vec::new();
    match &cmd.kind {
        CommandKind::Catch { body }
        | CommandKind::Foreach { body, .. }
        | CommandKind::DictUpdate { body, .. }
        | CommandKind::WhileLoop { body, .. }
        | CommandKind::ForLoop { body, .. } => {
            children.extend(body.block.children.values().cloned());
        }
        CommandKind::Switch { arms, default, .. } => {
            for (_, body) in arms {
                children.extend(body.block.children.values().cloned());
            }
            if let Some(body) = default {
                children.extend(body.block.children.values().cloned());
            }
        }
        CommandKind::IfElse {
            if_body, else_body, ..
        } => {
            children.extend(if_body.block.children.values().cloned());
            children.extend(else_body.block.children.values().cloned());
        }
        _ => {}
    }
    children
}

fn find_rendered_child_command(
    rendered: &str,
    child_text: &str,
    cursor: usize,
) -> Option<(usize, usize)> {
    find_rendered_child_candidate(rendered, child_text, cursor).or_else(|| {
        let indented = indent_rendered_child(child_text);
        find_rendered_child_candidate(rendered, &indented, cursor)
    })
}

fn find_rendered_child_candidate(
    rendered: &str,
    candidate: &str,
    cursor: usize,
) -> Option<(usize, usize)> {
    rendered
        .get(cursor..)
        .and_then(|tail| tail.find(candidate))
        .map(|offset| (cursor + offset, candidate.len()))
}

fn indent_rendered_child(child_text: &str) -> String {
    child_text
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_looks_like_rendered_command(trimmed: &str) -> bool {
    !trimmed.is_empty()
        && trimmed != "{"
        && trimmed != "}"
        && !trimmed.starts_with('}')
        && !trimmed.ends_with(" -")
}

fn rendered_command_already_covered(existing: &str, candidate: &str) -> bool {
    let existing = existing.trim();
    let candidate = candidate.trim();
    existing == candidate || existing.starts_with(candidate) || candidate.starts_with(existing)
}

fn synthetic_identity(span: &tcld_syntax::TclCommandSpan) -> CommandIdentity {
    CommandIdentity::Synthetic {
        proc_qname: None,
        body_byte_offset: span.start_byte,
        body_byte_len: span.end_byte.saturating_sub(span.start_byte),
        kind: span.kind,
        name: span.name.as_deref().map(Arc::from),
    }
}

fn synthetic_first_word(text: &str) -> Option<Arc<str>> {
    text.split_whitespace().next().map(Arc::from)
}

fn proc_body_span_byte_offset(body: &str, span: &DecompiledCommandSpan) -> usize {
    let line_start = body
        .split_inclusive('\n')
        .take(span.start_line)
        .map(str::len)
        .sum::<usize>();
    let line = body[line_start..].split('\n').next().unwrap_or_default();
    let needle = span.text.lines().next().unwrap_or_default().trim_start();
    line_start
        + line.find(needle).unwrap_or_else(|| {
            line.char_indices()
                .find(|(_, ch)| !ch.is_whitespace())
                .map(|(offset, _)| offset)
                .unwrap_or(0)
        })
}

impl Decompiler {
    pub fn new(bytecode: ByteCode) -> Result<Self> {
        Self::new_with_mode(bytecode, false)
    }

    pub fn new_with_config(bytecode: ByteCode, config: DecompilePipelineConfig) -> Result<Self> {
        Self::new_with_mode_and_config(bytecode, false, config)
    }

    pub fn new_proc_body(bytecode: ByteCode) -> Result<Self> {
        Self::new_with_mode(bytecode, true)
    }

    fn new_with_mode(bytecode: ByteCode, is_proc_body: bool) -> Result<Self> {
        Self::new_with_mode_and_config(bytecode, is_proc_body, DecompilePipelineConfig::default())
    }

    fn new_with_mode_and_config(
        bytecode: ByteCode,
        is_proc_body: bool,
        pipeline_config: DecompilePipelineConfig,
    ) -> Result<Self> {
        crate::init::ensure_initialized();
        let instructions = bytecode.get_instructions()?;
        let fastpath_suppress = expr_fastpath_ranges(&instructions);
        let graph = CodeGraph::new(instructions);
        Ok(Decompiler {
            bytecode,
            graph,
            fastpath_suppress,
            is_proc_body,
            pipeline_config,
            pass_trace: Vec::new(),
            pipeline_error: None,
            nested_render_issues: Vec::new(),
        })
    }

    pub(super) fn stitch_ranges(&self, starts: &[i32], ends: &[i32]) -> Vec<Range<i32>> {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum EventKind {
            Start,
            End,
        }

        let mut ranges = Vec::new();
        let mut open_starts = Vec::new();
        let mut events: Vec<(i32, EventKind)> = starts
            .iter()
            .cloned()
            .map(|v| (v, EventKind::Start))
            .chain(ends.iter().cloned().map(|v| (v, EventKind::End)))
            .collect();
        events.sort();
        for (event, kind) in events {
            match kind {
                EventKind::Start => open_starts.push(event),
                EventKind::End => {
                    let Some(start) = open_starts.pop() else {
                        return Vec::new();
                    };
                    let Some(end) = event.checked_add(1) else {
                        return Vec::new();
                    };
                    if start >= end {
                        return Vec::new();
                    }
                    ranges.push(start..end);
                }
            }
        }
        if open_starts.is_empty() {
            ranges
        } else {
            Vec::new()
        }
    }

    pub(super) fn process_instructions(&mut self, range: Range<i32>) {
        while let Some(codes) = self.reduce_instructions(self.graph.code_in_range(range.clone())) {
            for code in codes {
                self.graph.replace_code(code);
            }
        }
        self.process_stack(range);
    }

    pub(super) fn process_stack(&mut self, range: Range<i32>) -> bool {
        let pops: Vec<CodeRef> = self
            .graph
            .code_in_range(range)
            .into_iter()
            .filter(|c| {
                if let Code::Instruction(inst) = &*c.borrow() {
                    inst.opcode == Opcode::Pop || inst.opcode == Opcode::Done
                } else {
                    false
                }
            })
            .collect();
        let mut reduced = true;
        for pop in pops {
            if let Some(cmd_ref) = self.graph.single_source_of(&pop) {
                let replaced = {
                    let mut borrowed = cmd_ref.borrow_mut();
                    if let Code::Command(cmd) = &mut *borrowed {
                        cmd.merge_one(pop);
                        cmd.destack();
                        true
                    } else {
                        false
                    }
                };
                if replaced {
                    self.graph.replace_code(cmd_ref);
                } else {
                    reduced = false;
                }
            } else {
                reduced = false;
            }
        }
        reduced
    }

    pub(super) fn process_catches(&mut self, range: Range<i32>) {
        passes::catches::run(self, range);
    }

    pub(super) fn process_foreach(&mut self, range: Range<i32>) {
        passes::foreach::run(self, range);
    }

    pub(super) fn process_dict_update(&mut self, range: Range<i32>) {
        passes::dict_update::run(self, range);
    }

    pub(super) fn process_switch_tables(&mut self, range: Range<i32>) {
        passes::switch_tables::run(self, range);
    }

    pub(super) fn process_chained_logicals(&mut self, range: Range<i32>) {
        passes::conditionals::run_chained_logicals(self, range);
    }

    pub(super) fn process_loops(&mut self, range: Range<i32>) -> bool {
        passes::loops::run(self, range)
    }

    pub(super) fn process_if_else(&mut self, range: Range<i32>) -> bool {
        passes::conditionals::run_if_else(self, range)
    }

    pub(super) fn run_pipeline_pass(&mut self, pass: PipelinePass, range: Range<i32>) {
        match pass {
            PipelinePass::Instructions => self.process_instructions(range),
            PipelinePass::DictUpdate => self.process_dict_update(range),
            PipelinePass::SwitchTables => self.process_switch_tables(range),
            PipelinePass::Catches => self.process_catches(range),
            PipelinePass::ChainedLogicals => self.process_chained_logicals(range),
            PipelinePass::Foreach => self.process_foreach(range),
            PipelinePass::IfElseFixpoint => while self.process_if_else(range.clone()) {},
            PipelinePass::LoopsFixpoint => while self.process_loops(range.clone()) {},
        }
    }

    pub(super) fn process(&mut self, range: Range<i32>) {
        const MAX_FIXPOINT_ITERATIONS: usize = 1_024;

        if self.pipeline_error.is_some() {
            return;
        }
        if self.pipeline_config.include_global_registry
            && pass_registry::global_registry().lock().is_err()
        {
            self.pipeline_error = Some("global decompiler pass registry is poisoned".to_string());
            return;
        }
        for entry in self.pipeline_entries() {
            let name = entry.name().to_string();
            if !self.pipeline_config.is_enabled(&name) {
                self.pass_trace.push(crate::diagnostics::PassDecision {
                    name,
                    enabled: false,
                    iteration: 0,
                    changed: false,
                });
                continue;
            }
            match entry {
                PipelineEntry::BuiltIn {
                    pass,
                    name,
                    fixpoint,
                    ..
                } => {
                    if fixpoint {
                        let mut iteration = 0usize;
                        loop {
                            let changed = match pass {
                                PipelinePass::IfElseFixpoint => self.process_if_else(range.clone()),
                                PipelinePass::LoopsFixpoint => self.process_loops(range.clone()),
                                _ => {
                                    self.run_pipeline_pass(pass, range.clone());
                                    true
                                }
                            };
                            self.pass_trace.push(crate::diagnostics::PassDecision {
                                name: name.to_string(),
                                enabled: true,
                                iteration,
                                changed,
                            });
                            if !changed {
                                break;
                            }
                            iteration += 1;
                            if iteration >= MAX_FIXPOINT_ITERATIONS {
                                self.pipeline_error = Some(format!(
                                    "decompiler pass {name:?} exceeded {MAX_FIXPOINT_ITERATIONS} fixpoint iterations"
                                ));
                                return;
                            }
                        }
                    } else {
                        self.run_pipeline_pass(pass, range.clone());
                        self.pass_trace.push(crate::diagnostics::PassDecision {
                            name: name.to_string(),
                            enabled: true,
                            iteration: 0,
                            changed: true,
                        });
                    }
                }
                PipelineEntry::Custom(pass) => {
                    if pass.is_fixpoint() {
                        let mut iteration = 0usize;
                        loop {
                            let changed =
                                match pass.run(&mut self.graph, &self.bytecode, range.clone()) {
                                    Ok(changed) => changed,
                                    Err(error) => {
                                        self.pipeline_error = Some(format!(
                                            "custom decompiler pass {:?} failed: {error}",
                                            pass.name()
                                        ));
                                        return;
                                    }
                                };
                            self.pass_trace.push(crate::diagnostics::PassDecision {
                                name: pass.name().to_string(),
                                enabled: true,
                                iteration,
                                changed,
                            });
                            if !changed {
                                break;
                            }
                            iteration += 1;
                            if iteration >= MAX_FIXPOINT_ITERATIONS {
                                self.pipeline_error = Some(format!(
                                    "custom decompiler pass {:?} exceeded {MAX_FIXPOINT_ITERATIONS} fixpoint iterations",
                                    pass.name()
                                ));
                                return;
                            }
                        }
                    } else {
                        let changed = match pass.run(&mut self.graph, &self.bytecode, range.clone())
                        {
                            Ok(changed) => changed,
                            Err(error) => {
                                self.pipeline_error = Some(format!(
                                    "custom decompiler pass {:?} failed: {error}",
                                    pass.name()
                                ));
                                return;
                            }
                        };
                        self.pass_trace.push(crate::diagnostics::PassDecision {
                            name: pass.name().to_string(),
                            enabled: true,
                            iteration: 0,
                            changed,
                        });
                    }
                }
            }
        }
    }

    pub fn get_source(self) -> Result<String> {
        Ok(self.get_output()?.text)
    }

    pub fn get_source_approximate(self) -> Result<String> {
        Ok(self.get_output_approximate()?.text)
    }

    pub fn get_output(mut self) -> Result<DecompiledOutput> {
        let codes = self.finalized_codes()?;
        let (output, issues) = crate::init::with_proc_body_render_context(
            crate::init::ProcBodyRenderMode::Strict,
            || self.render_output_from_codes(&codes),
        );
        self.nested_render_issues = issues;
        if !self.nested_render_issues.is_empty() {
            return Err(TcldError::Other(format!(
                "strict decompilation failed: {}",
                self.nested_render_issues.join("; ")
            )));
        }
        output
    }

    pub fn get_output_approximate(mut self) -> Result<DecompiledOutput> {
        let codes = self.finalized_codes_approximate()?;
        let (output, issues) = crate::init::with_proc_body_render_context(
            crate::init::ProcBodyRenderMode::Approximation,
            || self.render_output_from_codes(&codes),
        );
        self.nested_render_issues = issues;
        output
    }

    // StrictDecompileError intentionally carries the full DiagnosticsReport
    // gathered before failure (documented contract); boxing it is a future
    // API change, not a release-prep edit.
    #[allow(clippy::result_large_err)]
    pub fn get_output_with_diagnostics(
        mut self,
    ) -> std::result::Result<
        crate::diagnostics::DecompileWithDiagnostics,
        crate::diagnostics::StrictDecompileError,
    > {
        match self.finalized_codes() {
            Ok(codes) => {
                let (output, issues) = crate::init::with_proc_body_render_context(
                    crate::init::ProcBodyRenderMode::Strict,
                    || self.render_output_from_codes(&codes),
                );
                self.nested_render_issues = issues;
                if !self.nested_render_issues.is_empty() {
                    let source = TcldError::Other(format!(
                        "strict decompilation failed: {}",
                        self.nested_render_issues.join("; ")
                    ));
                    let diagnostics = self.build_diagnostics(None, true, Some(source.to_string()));
                    return Err(crate::diagnostics::StrictDecompileError {
                        source,
                        diagnostics,
                    });
                }
                let output = output.map_err(|source| {
                    let diagnostics = self.build_diagnostics(None, true, None);
                    crate::diagnostics::StrictDecompileError {
                        source,
                        diagnostics,
                    }
                })?;
                let diagnostics = self.build_diagnostics(Some(&output), true, None);
                Ok(crate::diagnostics::DecompileWithDiagnostics {
                    output,
                    diagnostics,
                })
            }
            Err(source) => {
                let err_text = source.to_string();
                let diagnostics = self.build_diagnostics(None, true, Some(err_text));
                Err(crate::diagnostics::StrictDecompileError {
                    source,
                    diagnostics,
                })
            }
        }
    }

    pub fn get_output_approximate_with_diagnostics(
        mut self,
    ) -> Result<crate::diagnostics::DecompileWithDiagnostics> {
        let codes = self.finalized_codes_approximate()?;
        let (output, issues) = crate::init::with_proc_body_render_context(
            crate::init::ProcBodyRenderMode::Approximation,
            || self.render_output_from_codes(&codes),
        );
        self.nested_render_issues = issues;
        let output = output?;
        let diagnostics = self.build_diagnostics(Some(&output), false, None);
        Ok(crate::diagnostics::DecompileWithDiagnostics {
            output,
            diagnostics,
        })
    }

    fn build_diagnostics(
        &self,
        output: Option<&DecompiledOutput>,
        strict: bool,
        failure: Option<String>,
    ) -> crate::diagnostics::DiagnosticsReport {
        let mut report = crate::diagnostics::DiagnosticsReport {
            suppressed_fast_path_ranges: self.fastpath_suppress.clone(),
            pass_decisions: self.pass_trace.clone(),
            reconstruction: crate::diagnostics::ReconstructionStatus::Strict,
            ..Default::default()
        };
        let mut unresolved_ranges = Vec::new();
        for code in self.graph.code_in_range(self.graph.code_range()) {
            let borrowed = code.borrow();
            let Code::Instruction(instruction) = &*borrowed else {
                continue;
            };
            if is_ir_ignorable_inst(instruction.opcode) || self.is_suppressed(&code) {
                continue;
            }
            let description = instruction.debug_description();
            if !report.unreduced_opcodes.contains(&description) {
                report.unreduced_opcodes.push(description.clone());
                report.unsupported_instructions.push(description);
            }
            unresolved_ranges
                .push(instruction.location..(instruction.location + instruction.size()));
        }
        if let Some(output) = output {
            report.recognized_constructs = recognized_constructs(output);
        }
        if !strict {
            for issue in &self.nested_render_issues {
                report
                    .approximations
                    .push(crate::diagnostics::Approximation {
                        kind: crate::diagnostics::ApproximationKind::NestedProcedure,
                        bytecode_range: None,
                        reason: issue.clone(),
                    });
            }
            for range in unresolved_ranges {
                report
                    .approximations
                    .push(crate::diagnostics::Approximation {
                        kind: crate::diagnostics::ApproximationKind::UnresolvedInstruction,
                        bytecode_range: Some(range),
                        reason:
                            "unresolved semantic instruction was omitted from reconstructed source"
                                .to_string(),
                    });
            }
            if !report.approximations.is_empty() {
                report.reconstruction = crate::diagnostics::ReconstructionStatus::Approximation;
            }
        }
        if let Some(failure) = failure {
            report.reconstruction = crate::diagnostics::ReconstructionStatus::Failed;
            if let Some(rest) = failure.split("unreduced instructions remain:").nth(1) {
                for item in rest.split(',') {
                    let trimmed = item.trim();
                    if !trimmed.is_empty() {
                        report.unreduced_opcodes.push(trimmed.to_string());
                        report.unsupported_instructions.push(trimmed.to_string());
                        report
                            .suggested_next_work
                            .push(format!("add opcode fixture/handler coverage for {trimmed}"));
                    }
                }
            }
        }
        if !self.nested_render_issues.is_empty() {
            report.suggested_next_work.extend(
                self.nested_render_issues
                    .iter()
                    .map(|issue| format!("repair nested decompilation: {issue}")),
            );
        }
        report
    }

    fn pipeline_entries(&self) -> Vec<PipelineEntry> {
        let mut entries = vec![
            PipelineEntry::BuiltIn {
                pass: PipelinePass::Instructions,
                name: "instructions",
                priority: 100,
                fixpoint: false,
            },
            PipelineEntry::BuiltIn {
                pass: PipelinePass::DictUpdate,
                name: "dict_update",
                priority: 200,
                fixpoint: false,
            },
            PipelineEntry::BuiltIn {
                pass: PipelinePass::SwitchTables,
                name: "switch_tables",
                priority: 300,
                fixpoint: false,
            },
            PipelineEntry::BuiltIn {
                pass: PipelinePass::Catches,
                name: "catches",
                priority: 400,
                fixpoint: false,
            },
            PipelineEntry::BuiltIn {
                pass: PipelinePass::ChainedLogicals,
                name: "chained_logicals",
                priority: 500,
                fixpoint: false,
            },
            PipelineEntry::BuiltIn {
                pass: PipelinePass::Foreach,
                name: "foreach",
                priority: 600,
                fixpoint: false,
            },
            PipelineEntry::BuiltIn {
                pass: PipelinePass::IfElseFixpoint,
                name: "if_else",
                priority: 700,
                fixpoint: true,
            },
            PipelineEntry::BuiltIn {
                pass: PipelinePass::LoopsFixpoint,
                name: "loops",
                priority: 800,
                fixpoint: true,
            },
        ];
        entries.extend(
            self.pipeline_config
                .custom_passes
                .iter()
                .cloned()
                .map(PipelineEntry::Custom),
        );
        if self.pipeline_config.include_global_registry
            && let Ok(registry) = pass_registry::global_registry().lock()
        {
            entries.extend(registry.passes().iter().cloned().map(PipelineEntry::Custom));
        }
        entries.sort_by(|left, right| {
            left.priority()
                .cmp(&right.priority())
                .then_with(|| left.name().cmp(right.name()))
        });
        entries
    }

    fn render_output_from_codes(&mut self, codes: &[CodeRef]) -> Result<DecompiledOutput> {
        let mut out = String::new();
        let mut commands = Vec::new();
        let mut first = true;
        let mut prev_break_after = false;
        let mut next_line = 0usize;

        for code in codes {
            crate::init::clear_inner_output();
            let (rendered, break_after) = self.render_command(code.clone());
            if rendered.is_empty() {
                continue;
            }
            if !first {
                out.push('\n');
                // No next_line increment here: this \n terminates the previous
                // line. next_line already points to the correct line after the
                // previous span (end_line + 1).
                if prev_break_after {
                    out.push('\n');
                    next_line += 1;
                }
            }
            first = false;

            let start_line = next_line;
            let line_count = rendered.lines().count().max(1);
            let rendered_is_multiline = rendered.contains('\n');
            out.push_str(&rendered);
            let end_line = start_line + line_count.saturating_sub(1);
            next_line = end_line + 1;

            let code_range = {
                let borrowed = code.borrow();
                borrowed.range()
            };
            // Look up which bytecode command group this code belongs to
            let command_group = self.command_group_for_range(&code_range);
            let identity = match command_group {
                Some(group) => CommandIdentity::Bytecode {
                    command_group: group,
                },
                None => CommandIdentity::Unknown,
            };
            commands.push(DecompiledCommandSpan {
                code_range: code_range.clone(),
                text: rendered,
                start_line,
                end_line,
                identity,
                parent_command_group: None,
                nested_command_index: None,
            });
            let nested_spans = self.collect_nested_rendered_command_spans(
                code.clone(),
                &commands.last().unwrap().text,
                start_line,
            );
            commands.extend(nested_spans);

            // Check if this span is a proc definition — if so, retrieve
            // stashed inner commands and remap them into the outer line space.
            let proc_qname: Option<Arc<str>> = {
                let borrowed = code.borrow();
                if let Code::Command(cmd) = &*borrowed {
                    if let CommandKind::ProcCall { args } = &cmd.kind
                        && args.first().is_some_and(|a| {
                            a.repr_eq_str("proc") || a.repr_eq_str("tbcload::bcproc")
                        })
                    {
                        args.get(1)
                            .map(|name| Arc::from(name.repr_cow().into_owned()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(qname) = proc_qname.as_ref()
                && let Some(inner) = crate::init::take_inner_output()
            {
                // Multiline proc rendering puts the body after the header;
                // compact single-command bodies remain on the proc's line.
                let body_line_offset = start_line + usize::from(rendered_is_multiline);
                for (nested_index, mut inner_span) in inner.commands.into_iter().enumerate() {
                    let inferred_offset = proc_body_span_byte_offset(&inner.text, &inner_span);
                    inner_span.start_line += body_line_offset;
                    inner_span.end_line += body_line_offset;
                    // Nested bytecode offsets are relative to a distinct proc
                    // body and cannot safely share the root bytecode namespace.
                    // Key every nested span by proc name and body-source offset.
                    inner_span.identity = match inner_span.identity {
                        CommandIdentity::Synthetic {
                            proc_qname: existing,
                            body_byte_offset,
                            body_byte_len,
                            kind,
                            name,
                        } => CommandIdentity::Synthetic {
                            proc_qname: existing.or_else(|| Some(qname.clone())),
                            body_byte_offset,
                            body_byte_len,
                            kind,
                            name,
                        },
                        CommandIdentity::Bytecode { .. } | CommandIdentity::Unknown => {
                            CommandIdentity::Synthetic {
                                proc_qname: Some(qname.clone()),
                                body_byte_offset: inferred_offset,
                                body_byte_len: inner_span.text.len(),
                                kind: tcld_syntax::TclCommandKind::Command,
                                name: synthetic_first_word(&inner_span.text),
                            }
                        }
                    };
                    inner_span.parent_command_group = command_group;
                    inner_span.nested_command_index = Some(nested_index);
                    commands.push(inner_span);
                }
            }

            prev_break_after = break_after;
        }

        if self.is_proc_body {
            self.add_syntax_backed_proc_body_spans(&out, &mut commands);
        }

        Ok(DecompiledOutput {
            text: out,
            commands,
        })
    }

    fn command_group_for_range(&self, code_range: &Range<i32>) -> Option<usize> {
        self.bytecode.cmd_code_ranges.iter().position(|cmd_range| {
            code_range.start >= cmd_range.start && code_range.start < cmd_range.end
        })
    }

    fn collect_nested_rendered_command_spans(
        &self,
        code: CodeRef,
        rendered: &str,
        start_line: usize,
    ) -> Vec<DecompiledCommandSpan> {
        let mut spans = Vec::new();
        self.collect_nested_rendered_command_spans_into(code, rendered, start_line, &mut spans);
        spans
    }

    fn collect_nested_rendered_command_spans_into(
        &self,
        code: CodeRef,
        rendered: &str,
        start_line: usize,
        spans: &mut Vec<DecompiledCommandSpan>,
    ) {
        let mut cursor = 0usize;
        for child in rendered_command_body_children(code) {
            crate::init::clear_inner_output();
            let (child_text, _) = self.render_command(child.clone());
            if child_text.is_empty() {
                continue;
            }
            let Some((match_start, match_len)) =
                find_rendered_child_command(rendered, &child_text, cursor)
                    .or_else(|| find_rendered_child_command(rendered, &child_text, 0))
            else {
                continue;
            };
            cursor = match_start.saturating_add(match_len);

            let child_start_line = start_line + rendered[..match_start].matches('\n').count();
            let line_count = child_text.lines().count().max(1);
            let child_end_line = child_start_line + line_count.saturating_sub(1);
            let child_range = {
                let borrowed = child.borrow();
                borrowed.range()
            };
            let identity = match self.command_group_for_range(&child_range) {
                Some(group) => CommandIdentity::Bytecode {
                    command_group: group,
                },
                None => CommandIdentity::Unknown,
            };
            spans.push(DecompiledCommandSpan {
                code_range: child_range.clone(),
                text: child_text.clone(),
                start_line: child_start_line,
                end_line: child_end_line,
                identity,
                parent_command_group: None,
                nested_command_index: None,
            });
            // Rendering a nested proc definition uses the same core callback
            // as a root proc. Do not let an unclaimed side-channel value leak
            // into a later, unrelated command.
            crate::init::clear_inner_output();
            self.collect_nested_rendered_command_spans_into(
                child,
                &child_text,
                child_start_line,
                spans,
            );
        }
    }

    fn add_syntax_backed_proc_body_spans(
        &self,
        rendered: &str,
        commands: &mut Vec<DecompiledCommandSpan>,
    ) {
        // Line-backed pass runs first: it produces correct full-line text
        // even on inputs that confuse the tree-sitter-tcl grammar (which
        // notably truncates commands with nested `$var($idx)` substitutions).
        self.add_line_backed_proc_body_spans(rendered, commands);

        // Syntax pass runs second to enrich identity. For each parsed
        // command, we either upgrade an existing line-backed span's identity
        // in place (preserving the longer line-derived text) or, if no
        // covering span exists yet, add a fresh synthetic span.
        if let Ok(syntax_spans) = tcld_syntax::command_spans(rendered) {
            for syntax_span in syntax_spans {
                let existing_idx = commands.iter().position(|existing| {
                    existing.start_line == syntax_span.start_row
                        && rendered_command_already_covered(&existing.text, &syntax_span.text)
                });
                if let Some(idx) = existing_idx {
                    // Upgrade only if the existing identity is weaker (line-
                    // backed Command stub or Unknown). Don't clobber a real
                    // bytecode-backed span with a synthetic guess.
                    let upgrade = matches!(
                        commands[idx].identity,
                        CommandIdentity::Unknown
                            | CommandIdentity::Synthetic {
                                kind: tcld_syntax::TclCommandKind::Command,
                                ..
                            }
                    );
                    if upgrade {
                        commands[idx].identity = synthetic_identity(&syntax_span);
                    }
                    continue;
                }
                let containing = commands
                    .iter()
                    .filter(|existing| {
                        existing.start_line <= syntax_span.start_row
                            && existing.end_line >= syntax_span.end_row
                    })
                    .min_by_key(|existing| existing.end_line.saturating_sub(existing.start_line));
                let code_range = containing
                    .map(|existing| existing.code_range.clone())
                    .unwrap_or_else(|| 0..0);
                let identity = synthetic_identity(&syntax_span);
                commands.push(DecompiledCommandSpan {
                    code_range,
                    text: syntax_span.text,
                    start_line: syntax_span.start_row,
                    end_line: syntax_span.end_row,
                    identity,
                    parent_command_group: None,
                    nested_command_index: None,
                });
            }
        }

        commands.sort_by(|left, right| {
            left.start_line
                .cmp(&right.start_line)
                .then_with(|| left.end_line.cmp(&right.end_line))
                .then_with(|| left.code_range.start.cmp(&right.code_range.start))
                .then_with(|| left.text.cmp(&right.text))
        });
    }

    fn add_line_backed_proc_body_spans(
        &self,
        rendered: &str,
        commands: &mut Vec<DecompiledCommandSpan>,
    ) {
        let mut line_byte_offset = 0usize;
        for (line_idx, line) in rendered.split('\n').enumerate() {
            let line_start = line_byte_offset;
            // Advance past this line's bytes plus the trailing '\n' (the one
            // `split` removed) for the next iteration. The final iteration's
            // line has no trailing newline; we increment anyway, which is fine
            // because the loop ends right after.
            line_byte_offset += line.len() + 1;

            let trimmed_with_offset = line
                .char_indices()
                .find(|(_, ch)| !ch.is_whitespace())
                .map(|(off, _)| off)
                .unwrap_or(line.len());
            let trimmed = line.trim();
            if !line_looks_like_rendered_command(trimmed) {
                continue;
            }
            if commands.iter().any(|existing| {
                existing.start_line == line_idx
                    && rendered_command_already_covered(&existing.text, trimmed)
            }) {
                continue;
            }
            let containing = commands
                .iter()
                .filter(|existing| existing.start_line <= line_idx && existing.end_line >= line_idx)
                .min_by_key(|existing| existing.end_line.saturating_sub(existing.start_line));
            let code_range = containing
                .map(|existing| existing.code_range.clone())
                .unwrap_or_else(|| 0..0);
            let identity = CommandIdentity::Synthetic {
                proc_qname: None,
                body_byte_offset: line_start + trimmed_with_offset,
                body_byte_len: trimmed.len(),
                kind: tcld_syntax::TclCommandKind::Command,
                name: synthetic_first_word(trimmed),
            };
            commands.push(DecompiledCommandSpan {
                code_range,
                text: trimmed.to_string(),
                start_line: line_idx,
                end_line: line_idx,
                identity,
                parent_command_group: None,
                nested_command_index: None,
            });
        }
    }

    fn finalized_codes(&mut self) -> Result<Vec<CodeRef>> {
        let range = self.graph.code_range();
        if range.start >= range.end {
            return Ok(Vec::new());
        }
        if std::env::var("TCLD_DEBUG_FASTPATH").is_ok() {
            eprintln!("fastpath ranges: {:?}", self.fastpath_suppress);
        }
        self.process(range.clone());
        if let Some(error) = self.pipeline_error.take() {
            return Err(TcldError::Other(error));
        }
        let codes: Vec<CodeRef> = self
            .graph
            .code_in_range(range)
            .into_iter()
            .filter(|c| !self.is_suppressed(c))
            .collect();
        let mut unreduced = String::new();
        for c in &codes {
            let borrowed = c.borrow();
            let Code::Instruction(inst) = &*borrowed else {
                continue;
            };
            if is_ir_ignorable_inst(inst.opcode) {
                continue;
            }
            if !unreduced.is_empty() {
                unreduced.push_str(", ");
            }
            unreduced.push_str(&inst.debug_description());
        }
        if !unreduced.is_empty() {
            return Err(TcldError::Other(format!(
                "unreduced instructions remain: {}",
                unreduced
            )));
        }
        if self.is_proc_body {
            for c in &codes {
                self.rewrite_proc_body_literal_returns_in_code(c.clone());
            }
        }
        Ok(codes)
    }

    fn finalized_codes_approximate(&mut self) -> Result<Vec<CodeRef>> {
        let range = self.graph.code_range();
        if range.start >= range.end {
            return Ok(Vec::new());
        }
        self.process(range.clone());
        if let Some(error) = self.pipeline_error.take() {
            return Err(TcldError::Other(error));
        }
        let codes: Vec<CodeRef> = self
            .graph
            .code_in_range(range)
            .into_iter()
            .filter(|c| !self.is_suppressed(c))
            .collect();
        if self.is_proc_body {
            for c in &codes {
                self.rewrite_proc_body_literal_returns_in_code(c.clone());
            }
        }
        Ok(codes)
    }

    fn render_command(&self, code: CodeRef) -> (String, bool) {
        let mut break_after = false;
        let rendered = {
            let borrowed = code.borrow();
            let mut text = String::new();
            if let Code::Command(cmd) = &*borrowed {
                break_after = command_needs_following_blank_line(&cmd.kind);
                if let CommandKind::Variable { value } = &cmd.kind {
                    text.push_str("set ");
                    value.write_repr(&mut text);
                } else {
                    borrowed.write_source_repr(&mut text);
                }
            } else {
                borrowed.write_source_repr(&mut text);
            }
            text
        };
        (rendered, break_after)
    }

    fn rewrite_proc_body_literal_returns_in_code(&self, code: CodeRef) {
        let nested_children: Vec<CodeRef> = {
            let borrowed = code.borrow();
            let Code::Command(cmd) = &*borrowed else {
                return;
            };
            let mut nested = Vec::new();
            match &cmd.kind {
                CommandKind::Catch { body } => {
                    nested.extend(body.block.children.values().cloned());
                }
                CommandKind::Foreach { body, .. } => {
                    nested.extend(body.block.children.values().cloned());
                }
                CommandKind::DictUpdate { body, .. } => {
                    nested.extend(body.block.children.values().cloned());
                }
                CommandKind::Switch { arms, default, .. } => {
                    for (_, body) in arms {
                        nested.extend(body.block.children.values().cloned());
                    }
                    if let Some(body) = default {
                        nested.extend(body.block.children.values().cloned());
                    }
                }
                CommandKind::IfElse {
                    if_body, else_body, ..
                } => {
                    nested.extend(if_body.block.children.values().cloned());
                    nested.extend(else_body.block.children.values().cloned());
                }
                CommandKind::WhileLoop { body, .. } => {
                    nested.extend(body.block.children.values().cloned());
                }
                CommandKind::ForLoop { body, .. } => {
                    nested.extend(body.block.children.values().cloned());
                }
                _ => {}
            }
            nested
        };
        for child in nested_children {
            self.rewrite_proc_body_literal_returns_in_code(child);
        }

        let return_arg = {
            let borrowed = code.borrow();
            let Code::Command(cmd) = &*borrowed else {
                return;
            };

            if let CommandKind::ProcCall { args } = &cmd.kind
                && args.first().is_some_and(|a| a.repr_eq_str("return"))
            {
                None
            } else if cmd.source_repr() == "{}" {
                Some(sr("{}"))
            } else if let CommandKind::ProcCall { args } = &cmd.kind {
                if args.len() == 1 && args.first().is_some_and(|a| a.repr_eq_str("{}")) {
                    Some(sr("{}"))
                } else {
                    None
                }
            } else {
                match &cmd.kind {
                    CommandKind::Literal { value } => {
                        if cmd.base.stackn > 0 {
                            if value.repr_is_empty() {
                                Some(sr("{}"))
                            } else {
                                Some(value.clone())
                            }
                        } else if cmd.base.stackn == 0 && value.repr_eq_str("{}") {
                            Some(sr("{}"))
                        } else if cmd.base.stackn < 0 {
                            let token = value.repr_cow();
                            let token = token.trim();
                            if token.parse::<i64>().is_ok() || token.parse::<f64>().is_ok() {
                                Some(sr(token.to_string()))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    CommandKind::Expr { expr } => {
                        let token = expr.repr_cow();
                        let token = token.trim();
                        if token.parse::<i64>().is_ok() || token.parse::<f64>().is_ok() {
                            Some(sr(token.to_string()))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
        };
        if let Some(return_arg) = return_arg
            && let Code::Command(cmd) = &mut *code.borrow_mut()
        {
            cmd.kind = CommandKind::ProcCall {
                args: vec![sr("return"), return_arg],
            };
            cmd.base.stackn = 0;
        }
    }

    pub(super) fn is_suppressed(&self, code: &CodeRef) -> bool {
        if self.fastpath_suppress.is_empty() {
            return false;
        }
        let loc = code.borrow().location();
        self.fastpath_suppress.iter().any(|r| r.contains(&loc))
    }

    pub(super) fn filter_codes(&self, codes: Vec<CodeRef>) -> Vec<CodeRef> {
        if self.fastpath_suppress.is_empty() {
            return codes;
        }
        codes
            .into_iter()
            .filter(|c| !self.is_suppressed(c))
            .collect()
    }

    pub(super) fn collect_args_skipping(
        &self,
        codes: &[CodeRef],
        idx: usize,
        argc: usize,
    ) -> Option<(usize, Vec<CodeRef>)> {
        let mut args = Vec::with_capacity(argc);
        let mut start_idx = None;
        let mut i = idx;
        while i > 0 && args.len() < argc {
            i -= 1;
            let code = &codes[i];
            if self.is_suppressed(code) {
                continue;
            }
            if code.borrow().is_command() {
                args.push(code.clone());
                start_idx = Some(i);
                continue;
            }
            if let Code::Instruction(inst) = &*code.borrow()
                && is_reduction_separator_inst(inst.opcode)
            {
                continue;
            }
            return None;
        }
        if args.len() == argc {
            args.reverse();
            Some((start_idx?, args))
        } else {
            None
        }
    }

    pub(super) fn command_at_depth(
        &self,
        codes: &[CodeRef],
        idx: usize,
        depth: usize,
    ) -> Option<CodeRef> {
        let mut i = idx;
        let mut count = 0usize;
        while i > 0 {
            i -= 1;
            let code = &codes[i];
            if self.is_suppressed(code) {
                continue;
            }
            if code.borrow().is_command() {
                if count == depth {
                    return Some(code.clone());
                }
                count += 1;
                continue;
            }
            if let Code::Instruction(inst) = &*code.borrow()
                && is_reduction_separator_inst(inst.opcode)
            {
                continue;
            }
            break;
        }
        None
    }

    pub(super) fn lset_var_name(&self, code: &CodeRef) -> Option<SourceRef> {
        let borrowed = code.borrow();
        let Code::Command(cmd) = &*borrowed else {
            return None;
        };
        match &cmd.kind {
            CommandKind::Variable { value } => Some(value.clone()),
            CommandKind::ArrayRef { args } => {
                if args.len() >= 2 {
                    Some(sr_dyn(ArrayElem::new(vec![
                        args[0].clone(),
                        args[1].clone(),
                    ])))
                } else if args.len() == 1 {
                    Some(args[0].clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(super) fn lvt_value(&self, idx: i32) -> Option<SourceRef> {
        if idx < 0 {
            return None;
        }
        self.bytecode
            .locals
            .get(idx as usize)
            .map(|v| sr_dyn(v.clone()))
    }

    pub(super) fn lset_var_from_value(&self, code: &CodeRef) -> Option<SourceRef> {
        let borrowed = code.borrow();
        let Code::Command(cmd) = &*borrowed else {
            return None;
        };
        let CommandKind::ProcCall { args } = &cmd.kind else {
            return None;
        };
        if args.is_empty() {
            return None;
        }
        if !args[0].repr_eq_str("lset") {
            return None;
        }
        args.get(1).cloned()
    }
}

fn expr_fastpath_ranges(insts: &[Instruction]) -> Vec<Range<i32>> {
    let mut ranges = Vec::new();
    for (idx, inst) in insts.iter().enumerate() {
        if inst.opcode != Opcode::BeginCatch4 {
            continue;
        }
        let mut end1 = None;
        for (j, next) in insts.iter().enumerate().skip(idx + 1) {
            if next.opcode == Opcode::EndCatch {
                end1 = Some(j);
                break;
            }
        }
        let end1 = match end1 {
            Some(v) => v,
            None => continue,
        };
        if end1 + 1 >= insts.len() {
            continue;
        }
        let jump = &insts[end1 + 1];
        if !jump.is_jump_unconditional() {
            continue;
        }
        let Some(target) = jump.jump_target() else {
            continue;
        };
        let mut end2 = None;
        for (j, next) in insts.iter().enumerate().skip(end1 + 2) {
            if next.location >= target {
                break;
            }
            if next.opcode == Opcode::EndCatch {
                end2 = Some(j);
                break;
            }
        }
        let search_start = end2.unwrap_or(end1);
        let mut has_expr = false;
        for next in insts.iter().skip(search_start + 1) {
            if next.location >= target {
                break;
            }
            if next.opcode == Opcode::ExprStk {
                has_expr = true;
                break;
            }
        }
        if !has_expr {
            continue;
        }
        ranges.push(inst.location..(jump.location + jump.size()));
    }
    ranges
}

fn recognized_constructs(output: &DecompiledOutput) -> Vec<String> {
    let mut constructs = std::collections::BTreeSet::new();
    for span in &output.commands {
        let text = span.text.trim_start();
        if text.starts_with("if ") {
            constructs.insert("conditional:if".to_string());
        } else if text.starts_with("foreach ") {
            constructs.insert("loop:foreach".to_string());
        } else if text.starts_with("while ") {
            constructs.insert("loop:while".to_string());
        } else if text.starts_with("catch ") {
            constructs.insert("exception:catch".to_string());
        } else if text.contains("expr") {
            constructs.insert("expr".to_string());
        } else if !text.is_empty() {
            constructs.insert("command".to_string());
        }
    }
    constructs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{ByteCodeInfo, CompiledLocal, Instruction};
    use crate::code::code_ref;
    use tcld_core::inst_table::DEFAULT_INSTRUCTION_SET;

    fn empty_bytecode() -> ByteCode {
        ByteCode {
            header: None,
            info: ByteCodeInfo {
                num_commands: 0,
                num_src_bytes: 0,
                num_code_bytes: 0,
                num_lit_objects: 0,
                num_except_ranges: 0,
                num_aux_data_items: 0,
                num_cmd_loc_bytes: 0,
                max_except_depth: 0,
                max_stack_depth: 0,
                code_delta_size: 0,
                code_length_size: 0,
                src_delta_size: 0,
                src_length_size: 0,
            },
            code_bytes: Vec::new(),
            cmd_code_ranges: Vec::new(),
            cmd_source_ranges: None,
            literals: Vec::new(),
            exceptions: Vec::new(),
            auxiliaries: Vec::new(),
            proc_num_args: 0,
            proc_num_locals: 0,
            locals: Vec::new(),
        }
    }

    fn dummy_inst(location: i32) -> CodeRef {
        let opcode = DEFAULT_INSTRUCTION_SET
            .opcode_from_i32(0)
            .expect("opcode exists");
        let desc = *DEFAULT_INSTRUCTION_SET
            .desc_from_i32(0)
            .expect("instruction desc exists");
        code_ref(Code::Instruction(Instruction::new(
            location,
            opcode,
            desc,
            vec![],
            None,
        )))
    }

    fn opcode_inst(location: i32, opcode: Opcode, operands: Vec<i32>) -> CodeRef {
        let desc = *DEFAULT_INSTRUCTION_SET
            .instruction_table
            .iter()
            .find(|desc| DEFAULT_INSTRUCTION_SET.opcode_from_i32(desc.opcode) == Some(opcode))
            .expect("instruction desc exists");
        code_ref(Code::Instruction(Instruction::new(
            location, opcode, desc, operands, None,
        )))
    }

    #[test]
    fn over_reduction_duplicates_the_requested_stack_command() {
        let mut decompiler = Decompiler::new_with_mode(empty_bytecode(), true).expect("decompiler");
        let first = cmd_literal(vec![dummy_inst(0)], sr("first"));
        let second = cmd_literal(vec![dummy_inst(10)], sr("second"));
        let over = opcode_inst(20, Opcode::Over, vec![1]);

        let replacements = decompiler
            .reduce_instructions(vec![first, second, over])
            .expect("over reduces");
        assert_eq!(replacements.len(), 1);
        let borrowed = replacements[0].borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("over replacement must be a command");
        };
        let CommandKind::Literal { value } = &command.kind else {
            panic!("over replacement must preserve the duplicated command kind");
        };
        assert!(value.repr_eq_str("first"));
    }

    #[test]
    fn expand_stack_top_marks_the_top_command_as_expanded() {
        let mut decompiler = Decompiler::new_with_mode(empty_bytecode(), true).expect("decompiler");
        let value = cmd_literal(vec![dummy_inst(0)], sr("items"));
        let expand = opcode_inst(10, Opcode::ExpandStktop, vec![0]);

        let replacements = decompiler
            .reduce_instructions(vec![value, expand])
            .expect("expandStkTop reduces");
        assert_eq!(replacements.len(), 1);
        let borrowed = replacements[0].borrow();
        let Code::Command(command) = &*borrowed else {
            panic!("expandStkTop replacement must be a command");
        };
        let CommandKind::ExpandedArg { value } = &command.kind else {
            panic!("expandStkTop replacement must mark an expanded argument");
        };
        assert!(value.repr_eq_str("items"));
    }

    #[test]
    fn rewrites_empty_brace_proc_call_to_empty_return_in_proc_body() {
        let decompiler = Decompiler::new_with_mode(empty_bytecode(), true).expect("decompiler");
        let code = cmd_proc_call(vec![dummy_inst(10)], vec![sr("{}")]);
        if let Code::Command(cmd) = &mut *code.borrow_mut() {
            cmd.destack();
        }

        decompiler.rewrite_proc_body_literal_returns_in_code(code.clone());

        let borrowed = code.borrow();
        let Code::Command(cmd) = &*borrowed else {
            panic!("expected command");
        };
        let CommandKind::ProcCall { args } = &cmd.kind else {
            panic!("expected proc call");
        };
        assert_eq!(args.len(), 2);
        assert!(args[0].repr_eq_str("return"));
        assert!(args[1].repr_eq_str("{}"));
        assert_eq!(cmd.base.stackn, 0);
    }

    #[test]
    fn rewrites_destacked_empty_literal_to_empty_return_in_proc_body() {
        let decompiler = Decompiler::new_with_mode(empty_bytecode(), true).expect("decompiler");
        let code = cmd_literal(vec![dummy_inst(20)], sr("{}"));
        if let Code::Command(cmd) = &mut *code.borrow_mut() {
            cmd.destack();
        }

        decompiler.rewrite_proc_body_literal_returns_in_code(code.clone());

        let borrowed = code.borrow();
        let Code::Command(cmd) = &*borrowed else {
            panic!("expected command");
        };
        let CommandKind::ProcCall { args } = &cmd.kind else {
            panic!("expected proc call");
        };
        assert_eq!(args.len(), 2);
        assert!(args[0].repr_eq_str("return"));
        assert!(args[1].repr_eq_str("{}"));
        assert_eq!(cmd.base.stackn, 0);
    }

    #[test]
    fn render_output_span_lines_match_actual_text_lines() {
        let mut bc = empty_bytecode();
        // Three command groups covering PCs 0..5, 5..10, 10..15
        bc.cmd_code_ranges = vec![0..5, 5..10, 10..15];

        let mut decompiler = Decompiler::new_with_mode(bc, false).expect("decompiler");

        // Create three simple commands at distinct PC locations
        let code_a = cmd_proc_call(vec![dummy_inst(0)], vec![sr("set"), sr("x"), sr("1")]);
        let code_b = cmd_proc_call(vec![dummy_inst(5)], vec![sr("set"), sr("y"), sr("2")]);
        let code_c = cmd_proc_call(vec![dummy_inst(10)], vec![sr("set"), sr("z"), sr("3")]);

        // Destack so they render as top-level commands
        for code in [code_a.clone(), code_b.clone(), code_c.clone()] {
            if let Code::Command(cmd) = &mut *code.borrow_mut() {
                cmd.destack();
            }
        }

        let output = decompiler
            .render_output_from_codes(&[code_a, code_b, code_c])
            .expect("render succeeds");

        let lines: Vec<&str> = output.text.lines().collect();
        // Should produce 3 lines with no blanks between simple commands
        assert_eq!(lines.len(), 3, "text: {:?}", output.text);

        // Each span's start/end should match the actual line index
        assert_eq!(output.commands.len(), 3);
        for (i, span) in output.commands.iter().enumerate() {
            assert_eq!(
                span.start_line, i,
                "span {} start_line should be {} but was {}",
                i, i, span.start_line
            );
            assert_eq!(
                span.end_line, i,
                "span {} end_line should be {} but was {}",
                i, i, span.end_line
            );
            // Each span should have the correct command group
            assert_eq!(
                span.command_group(),
                Some(i),
                "span {} command_group should be Some({})",
                i,
                i
            );
        }
    }

    #[test]
    fn stack_lookup_helpers_skip_separators_and_suppressed_codes() {
        let mut decompiler = Decompiler::new_with_mode(empty_bytecode(), true).expect("decompiler");
        let first = cmd_literal(vec![dummy_inst(0)], sr("first"));
        let separator = opcode_inst(10, Opcode::Nop, Vec::new());
        let second = cmd_literal(vec![dummy_inst(20)], sr("second"));
        let consumer = opcode_inst(30, Opcode::ExprStk, Vec::new());
        let codes = vec![first.clone(), separator, second.clone(), consumer.clone()];

        let (start, args) = decompiler
            .collect_args_skipping(&codes, 3, 2)
            .expect("two stack arguments");
        assert_eq!(start, 0);
        assert_eq!(args, vec![first.clone(), second.clone()]);
        assert_eq!(
            decompiler.command_at_depth(&codes, 3, 1),
            Some(first.clone())
        );

        decompiler.fastpath_suppress.push(20..21);
        let (_, args) = decompiler
            .collect_args_skipping(&codes, 3, 1)
            .expect("suppressed second argument");
        assert_eq!(args, vec![first.clone()]);
        assert_eq!(decompiler.filter_codes(codes.clone()).len(), 3);

        let barrier = opcode_inst(15, Opcode::Push1, vec![0]);
        assert!(
            decompiler
                .collect_args_skipping(&[first, barrier, consumer], 2, 2)
                .is_none()
        );
    }

    #[test]
    fn lset_and_local_lookup_helpers_cover_all_shapes() {
        let mut bytecode = empty_bytecode();
        bytecode.locals.push(CompiledLocal {
            name: "local".to_string(),
            default_value: None,
            frame_index: 0,
            flag_mask: 0,
        });
        let decompiler = Decompiler::new(bytecode).expect("decompiler");

        assert!(decompiler.lvt_value(-1).is_none());
        assert!(decompiler.lvt_value(9).is_none());
        assert!(decompiler.lvt_value(0).unwrap().repr_eq_str("local"));

        let variable = cmd_variable(vec![dummy_inst(0)], sr("scalar"));
        assert!(
            decompiler
                .lset_var_name(&variable)
                .unwrap()
                .repr_eq_str("scalar")
        );
        let array = cmd_array_ref(
            vec![dummy_inst(10)],
            vec![sr("items"), sr("key"), sr("ignored")],
        );
        assert!(
            decompiler
                .lset_var_name(&array)
                .unwrap()
                .repr_eq_str("items(key)")
        );
        let one_arg_array = cmd_array_ref(vec![dummy_inst(20)], vec![sr("items")]);
        assert!(
            decompiler
                .lset_var_name(&one_arg_array)
                .unwrap()
                .repr_eq_str("items")
        );
        let empty_array = cmd_array_ref(vec![dummy_inst(30)], Vec::new());
        assert!(decompiler.lset_var_name(&empty_array).is_none());
        assert!(
            decompiler
                .lset_var_name(&cmd_literal(vec![dummy_inst(40)], sr("value")))
                .is_none()
        );
        assert!(decompiler.lset_var_name(&dummy_inst(50)).is_none());

        let lset = cmd_proc_call(
            vec![dummy_inst(60)],
            vec![sr("lset"), sr("target"), sr("index"), sr("value")],
        );
        assert!(
            decompiler
                .lset_var_from_value(&lset)
                .unwrap()
                .repr_eq_str("target")
        );
        for not_lset in [
            cmd_proc_call(vec![dummy_inst(70)], Vec::new()),
            cmd_proc_call(vec![dummy_inst(80)], vec![sr("set"), sr("target")]),
            cmd_literal(vec![dummy_inst(90)], sr("value")),
            dummy_inst(100),
        ] {
            assert!(decompiler.lset_var_from_value(&not_lset).is_none());
        }
    }

    #[test]
    fn proc_body_numeric_results_and_span_fallbacks_are_normalized() {
        let decompiler = Decompiler::new_with_mode(empty_bytecode(), true).expect("decompiler");
        let integer = cmd_literal(vec![dummy_inst(0)], sr(" 42 "));
        if let Code::Command(command) = &mut *integer.borrow_mut() {
            command.base.stackn = -1;
        }
        decompiler.rewrite_proc_body_literal_returns_in_code(integer.clone());
        assert!(integer.borrow().source_repr().contains("return 42"));

        let expression = cmd_expr(vec![dummy_inst(10)], sr("3.5"));
        decompiler.rewrite_proc_body_literal_returns_in_code(expression.clone());
        assert!(expression.borrow().source_repr().contains("return 3.5"));

        let nonnumeric = cmd_expr(vec![dummy_inst(20)], sr("value + 1"));
        decompiler.rewrite_proc_body_literal_returns_in_code(nonnumeric.clone());
        assert!(matches!(
            &*nonnumeric.borrow(),
            Code::Command(command) if matches!(command.kind, CommandKind::Expr { .. })
        ));

        let span = DecompiledCommandSpan {
            code_range: 0..1,
            text: "missing".to_string(),
            start_line: 0,
            end_line: 0,
            identity: CommandIdentity::Unknown,
            parent_command_group: None,
            nested_command_index: None,
        };
        assert_eq!(proc_body_span_byte_offset("   actual\n", &span), 3);
    }

    #[test]
    fn configured_and_approximate_empty_decompilation_entrypoints_succeed() {
        let configured = Decompiler::new_with_config(
            empty_bytecode(),
            DecompilePipelineConfig::default().disable_pass("foreach"),
        )
        .expect("configured decompiler");
        assert_eq!(configured.get_source_approximate().unwrap(), "");
    }

    #[test]
    fn strict_mode_rejects_unrecovered_semantic_instructions() {
        let desc = *DEFAULT_INSTRUCTION_SET
            .instruction_table
            .iter()
            .find(|desc| {
                DEFAULT_INSTRUCTION_SET.opcode_from_i32(desc.opcode) == Some(Opcode::DictFirst)
            })
            .expect("dictFirst descriptor");
        let instruction = Instruction::new(0, Opcode::DictFirst, desc, vec![], None);
        let mut decompiler = Decompiler::new(empty_bytecode()).expect("decompiler");
        decompiler.graph = CodeGraph::new(vec![instruction]);

        let error = decompiler
            .get_output()
            .expect_err("unrecovered dictFirst must fail strict decompilation");
        assert!(error.to_string().contains("dictFirst"));
    }
}
