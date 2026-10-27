use super::*;

/// The static forms accepted by Tcl's CompileEachloopCmd. Everything else
/// invokes the native command, including invalid variable lists.
pub(super) fn inline_lmap(text: &str, words: &[Range<usize>]) -> bool {
    if words.len() < 4 || !words.len().is_multiple_of(2) {
        return false;
    }
    let body = words.last().unwrap();
    let raw = &text[body.clone()];
    let simple_body = raw.starts_with('{')
        || bare_script_word(text, body)
        || (raw.starts_with('"')
            && word_literal(text, body.clone()).is_ok_and(|value| value == raw[1..raw.len() - 1]));
    simple_body
        && words[1..words.len() - 1].iter().step_by(2).all(|word| {
            word_literal(text, word.clone())
                .ok()
                .and_then(|spec| list_elements(&spec).ok())
                .is_some_and(|names| {
                    !names.is_empty()
                        && names
                            .iter()
                            .all(|name| valid_var_name(name) && !name.contains("::"))
                })
        })
}

/// Tcl compiles a static return of break/continue directly to loop control.
/// Validate its option dictionary first; malformed or computed options keep
/// their ordinary runtime validation and completion behavior.
pub(super) fn static_loop_return(text: &str, words: &[Range<usize>]) -> Option<Opcode> {
    use tcld_core::number::Number;
    // Tcl always uses returnStk for this dedicated form, even when literal.
    if words.len() == 4 && word_literal(text, words[1].clone()).ok().as_deref() == Some("-options")
    {
        return None;
    }
    let explicit_result = words.len().is_multiple_of(2);
    let end = words.len() - usize::from(explicit_result);
    let values = words[1..end]
        .iter()
        .map(|word| word_literal(text, word.clone()).ok())
        .collect::<Option<Vec<_>>>()?;
    let mut options = std::collections::BTreeMap::new();
    let mut pending = vec![values.into_iter()];
    while let Some(words) = pending.last_mut() {
        let Some(key) = words.next() else {
            pending.pop();
            continue;
        };
        let value = words.next()?;
        if key == "-options" {
            let nested = list_elements(&value).ok()?;
            if !nested.len().is_multiple_of(2) {
                return None;
            }
            pending.push(nested.into_iter());
        } else {
            options.insert(key, value);
        }
    }
    let integer = |text: &str| match Number::parse(text.as_bytes())? {
        Number::Int(value) => Some(value),
        _ => None,
    };
    if integer(options.get("-level")?)? != 0 {
        return None;
    }
    let code = options.get("-code")?;
    let opcode = match code.as_str() {
        "break" => Opcode::Break,
        "continue" => Opcode::Continue,
        _ => match integer(code)? {
            3 => Opcode::Break,
            4 => Opcode::Continue,
            _ => return None,
        },
    };
    for key in ["-errorcode", "-errorstack"] {
        if let Some(value) = options.get(key) {
            let list = list_elements(value).ok()?;
            if key == "-errorstack" && !list.len().is_multiple_of(2) {
                return None;
            }
        }
    }
    Some(opcode)
}

impl Compiler<'_> {
    pub(super) fn compile_script(
        &mut self,
        span: Range<usize>,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let commands = source::script_ranges(&self.source[span.clone()])
            .map_err(|e| diagnostic(self.path, self.source, span.clone(), "", e))?;
        if commands.is_empty() {
            self.push_literal("");
            return Ok(());
        }
        for (index, command) in commands.iter().enumerate() {
            self.compile_command(span.start + command.start..span.start + command.end, depth)?;
            if index + 1 < commands.len() {
                self.emit(Opcode::Pop);
                self.maps.last_mut().unwrap().pc_range.end = self.code.len() as u32;
            }
        }
        Ok(())
    }
    pub(super) fn body(
        &mut self,
        text: &str,
        word: &Range<usize>,
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        if bare_script_word(text, word) {
            return self.compile_script(base + word.start..base + word.end, depth);
        }
        if !text[word.clone()].starts_with('{') {
            self.compile_word(text, word.clone(), base, depth)?;
            self.emit(Opcode::EvalStk);
            return Ok(());
        }
        let range = self.body_range(text, word, base)?;
        self.compile_script(range, depth)
    }
    fn body_range(
        &self,
        text: &str,
        word: &Range<usize>,
        base: usize,
    ) -> Result<Range<usize>, CompileDiagnostic> {
        braced_literal(text, word.clone()).map_err(|e| {
            diagnostic(
                self.path,
                self.source,
                base + word.start..base + word.end,
                text,
                e,
            )
        })?;
        Ok(base + word.start + 1..base + word.end - 1)
    }
    fn condition(
        &mut self,
        text: &str,
        word: &Range<usize>,
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        if !text[word.clone()].starts_with('{') {
            self.compile_word(text, word.clone(), base, depth)?;
            self.emit(Opcode::ExprStk);
            return Ok(());
        }
        let range = self.body_range(text, word, base)?;
        self.compile_expression(range, depth)
    }
    pub(super) fn jump(&mut self, opcode: Opcode) -> usize {
        let pc = self.code.len();
        self.emit_arg(opcode, 0);
        pc
    }
    pub(super) fn patch_jump(&mut self, pc: usize, target: usize) {
        let offset = i32::try_from(target as i64 - pc as i64).expect("bytecode jump offset");
        self.code[pc + 1..pc + 5].copy_from_slice(&offset.to_be_bytes());
    }
    pub(super) fn loop_range(
        &mut self,
        start: usize,
        end: usize,
        exit: usize,
        next: usize,
        depth: u32,
    ) {
        self.exceptions.push(ExceptionRange {
            kind: ExceptionType::Loop,
            nesting_level: self.exception_level(depth),
            code_offset: start as i32,
            num_code_bytes: (end - start) as i32,
            break_offset: exit as i32,
            continue_offset: next as i32,
            catch_offset: -1,
            code_range: start as i32..end as i32,
        });
    }
    /// An unnamed compiler temporary (`VAR_TEMPORARY`), distinct from every
    /// variable and from every other temporary.
    pub(super) fn temporary(&mut self) -> usize {
        self.locals.push(String::new());
        self.locals.len() - 1
    }
    pub(super) fn local(&mut self, name: &str) -> usize {
        if let Some(index) = self.locals.iter().position(|n| n == name) {
            index
        } else {
            let index = self.locals.len();
            self.locals.push(name.into());
            index
        }
    }
    pub(super) fn store_local(&mut self, index: usize) {
        self.emit_arg(Opcode::StoreScalar4, index as u32);
        self.emit(Opcode::Pop);
    }
    pub(super) fn compile_control(
        &mut self,
        name: &str,
        text: &str,
        words: &[Range<usize>],
        span: Range<usize>,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let path = self.path;
        let source = self.source;
        let bad = |reason: &str| diagnostic(path, source, span.clone(), text, reason.into());
        match name {
            "if" => {
                let mut index = 1;
                let mut exits = Vec::new();
                loop {
                    let condition = words
                        .get(index)
                        .ok_or_else(|| bad("if requires a condition and braced body"))?;
                    index += 1;
                    self.condition(text, condition, span.start, depth + 1)?;
                    if words
                        .get(index)
                        .and_then(|w| word_literal(text, w.clone()).ok())
                        .as_deref()
                        == Some("then")
                    {
                        index += 1;
                    }
                    let body = words
                        .get(index)
                        .ok_or_else(|| bad("if requires a braced body"))?;
                    index += 1;
                    let otherwise = self.jump(Opcode::JumpFalse4);
                    self.body(text, body, span.start, depth + 1)?;
                    exits.push(self.jump(Opcode::Jump4));
                    self.patch_jump(otherwise, self.code.len());
                    match words
                        .get(index)
                        .and_then(|w| word_literal(text, w.clone()).ok())
                        .as_deref()
                    {
                        Some("elseif") => {
                            index += 1;
                            continue;
                        }
                        Some("else") => {
                            if index + 2 != words.len() {
                                return Err(bad("else requires one braced body"));
                            }
                            self.body(text, &words[index + 1], span.start, depth + 1)?;
                        }
                        None if index == words.len() => self.push_literal(""),
                        _ => return Err(bad("expected elseif or else")),
                    }
                    break;
                }
                // Keep every child range strictly inside the owning command's PC range.
                let end = self.code.len();
                for exit in exits {
                    self.patch_jump(exit, end);
                }
                self.emit(Opcode::Nop);
            }
            "while" | "for" => {
                if words.len() != if name == "for" { 5 } else { 3 } {
                    return Err(bad("expected static braced loop scripts and condition"));
                }
                let (condition, body) = if name == "for" {
                    self.body(text, &words[1], span.start, depth + 1)?;
                    self.emit(Opcode::Pop);
                    (&words[2], &words[4])
                } else {
                    (&words[1], &words[2])
                };
                let test = self.code.len();
                self.condition(text, condition, span.start, depth + 1)?;
                let exit_jump = self.jump(Opcode::JumpFalse4);
                let body_start = self.code.len();
                self.return_context.push(ExceptionType::Loop);
                self.body(text, body, span.start, depth + 1)?;
                self.return_context.pop();
                self.emit(Opcode::Pop);
                let body_end = self.code.len();
                let next = self.code.len();
                if name == "for" {
                    self.body(text, &words[3], span.start, depth + 1)?;
                    self.emit(Opcode::Pop);
                }
                let back = self.jump(Opcode::Jump4);
                self.patch_jump(back, test);
                let exit = self.code.len();
                self.patch_jump(exit_jump, exit);
                self.push_literal("");
                self.loop_range(
                    body_start,
                    body_end,
                    exit,
                    if name == "for" { next } else { test },
                    depth,
                );
            }
            "foreach" | "lmap" => {
                let collect = name == "lmap";
                if collect {
                    self.emit_arg(Opcode::List, 0);
                }
                if words.len() < 4 || !words.len().is_multiple_of(2) {
                    return Err(bad(
                        "foreach requires varList list pairs and one braced body",
                    ));
                }
                let mut lists = Vec::new();
                for pair in words[1..words.len() - 1].chunks(2) {
                    let spec = word_literal(text, pair[0].clone())
                        .map_err(|_| bad("foreach variable lists must be static"))?;
                    let names =
                        list_elements(&spec).map_err(|_| bad("invalid foreach variable list"))?;
                    if names.is_empty() {
                        return Err(bad("foreach variable list cannot be empty"));
                    }
                    let mut variables = Vec::new();
                    for name in names {
                        if !valid_var_name(&name) {
                            return Err(bad("foreach requires scalar variable names"));
                        }
                        variables.push(self.local(&name) as i32);
                    }
                    lists.push(variables);
                    self.compile_word(text, pair[1].clone(), span.start, depth)?;
                }
                let aux = self.auxiliaries.len();
                self.auxiliaries.push(AuxData {
                    kind: AuxDataType::NewForeach,
                    value: AuxDataValue::NewForeach(NewForeachInfo {
                        loop_counter: 0,
                        lists,
                    }),
                });
                self.emit_arg(Opcode::ForeachStart, aux as u32);
                let body_start = self.code.len();
                // Tcl invokes foreach at global scope. Its body cannot branch
                // directly to the loop, even though our frame-owned foreach
                // instructions can execute it without entering another frame.
                self.return_context.push(if self.procedure_context {
                    ExceptionType::Loop
                } else {
                    ExceptionType::Catch
                });
                let body = words.last().unwrap();
                if collect && text[body.clone()].starts_with('"') {
                    self.compile_script(
                        span.start + body.start + 1..span.start + body.end - 1,
                        depth + 1,
                    )?;
                } else {
                    self.body(text, body, span.start, depth + 1)?;
                }
                self.return_context.pop();
                self.emit(if collect {
                    Opcode::LmapCollect
                } else {
                    Opcode::Pop
                });
                let step = self.code.len();
                self.emit(Opcode::ForeachStep);
                let exit = self.code.len();
                self.emit(Opcode::ForeachEnd);
                if !collect {
                    self.push_literal("");
                    if !self.procedure_context {
                        // Native foreach ends with Tcl_ResetResult. Express
                        // that reset using a normal completion with no custom
                        // options, leaving the same empty result on the stack.
                        self.push_literal("");
                        self.emit_args(Opcode::ReturnImm, &[0, 0]);
                    }
                }
                let AuxDataValue::NewForeach(info) = &mut self.auxiliaries[aux].value else {
                    unreachable!()
                };
                info.loop_counter = body_start as i32 - step as i32;
                self.loop_range(body_start, step, exit, step, depth);
            }
            "catch" => {
                if !(2..=4).contains(&words.len()) {
                    return Err(bad(
                        "catch requires a braced script and optional result/options variable names",
                    ));
                }
                let mut vars = Vec::new();
                for word in &words[2..] {
                    let name = word_literal(text, word.clone())
                        .map_err(|_| bad("catch variable names must be static"))?;
                    if !valid_var_name(&name) {
                        return Err(bad("catch requires scalar result/options variables"));
                    }
                    vars.push(self.local(&name));
                }
                let index = self.exceptions.len();
                self.exceptions.push(ExceptionRange {
                    kind: ExceptionType::Catch,
                    nesting_level: self.exception_level(depth),
                    code_offset: 0,
                    num_code_bytes: 0,
                    break_offset: -1,
                    continue_offset: -1,
                    catch_offset: -1,
                    code_range: 0..0,
                });
                self.emit_arg(Opcode::BeginCatch4, index as u32);
                let start = self.code.len();
                self.return_context.push(ExceptionType::Catch);
                self.body(text, &words[1], span.start, depth + 1)?;
                self.return_context.pop();
                let end = self.code.len();
                if let Some(var) = vars.first() {
                    self.store_local(*var);
                } else {
                    self.emit(Opcode::Pop);
                }
                if let Some(var) = vars.get(1) {
                    self.emit(Opcode::PushReturnOptions);
                    self.store_local(*var);
                }
                self.push_literal("0");
                let normal = self.jump(Opcode::Jump4);
                let handler = self.code.len();
                if let Some(var) = vars.first() {
                    self.emit(Opcode::PushResult);
                    self.store_local(*var);
                }
                if let Some(var) = vars.get(1) {
                    self.emit(Opcode::PushReturnOptions);
                    self.store_local(*var);
                }
                self.emit(Opcode::PushReturnCode);
                self.patch_jump(normal, self.code.len());
                self.emit(Opcode::EndCatch);
                self.exceptions[index] = ExceptionRange {
                    kind: ExceptionType::Catch,
                    nesting_level: self.exception_level(depth),
                    code_offset: start as i32,
                    num_code_bytes: (end - start) as i32,
                    break_offset: -1,
                    continue_offset: -1,
                    catch_offset: handler as i32,
                    code_range: start as i32..end as i32,
                };
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
