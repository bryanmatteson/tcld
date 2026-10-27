//! `dict` subcommands compiled to Tcl's dictionary instructions, following
//! `tclCompCmds.c`. A subcommand that updates a variable compiles only when
//! the variable is a plain local name; every other form stays an ordinary
//! command invocation.

use super::*;
use tcld_core::bytecode::DictUpdateInfo;
use tcld_core::number::Number;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DictForm {
    /// `dict get` or `dict exists` with at least one key.
    Read(Opcode),
    /// `dict set var key ?key ...? value`
    Set(String),
    /// `dict unset var key ?key ...?`
    Unset(String),
    /// `dict incr var key ?literal-increment?`
    Incr(String, i32),
    /// `dict append var key value ?value ...?`
    Append(String),
    /// `dict lappend var key value`
    Lappend(String),
    /// `dict create` with distinct literal keys: the key/value list is
    /// already canonical.
    Create(Vec<String>),
    /// `dict for {key value} dictionary {body}`
    For(String, String),
    /// `dict map {key value} dictionary {body}`
    Map(String, String),
    /// `dict update var key name ?key name ...? {body}`
    Update(String, Vec<String>),
    /// `dict with dictVar ?key ...? {body}`, with the variable when it is a
    /// plain local name.
    With(Option<String>),
}

/// A variable `dict` can update in place: a plain local name.
fn local_scalar(text: &str, word: &Range<usize>) -> Option<String> {
    word_literal(text, word.clone())
        .ok()
        .filter(|name| valid_var_name(name) && !name.contains("::"))
}

pub(crate) fn dict_form(text: &str, words: &[Range<usize>]) -> Option<DictForm> {
    if words.iter().any(|word| expanded_word(&text[word.clone()])) {
        return None;
    }
    let subcommand = dict_subcommand(&static_subcommand(text, words)?)?;
    let args = &words[2..];
    let var = || local_scalar(text, &args[0]);
    Some(match (subcommand, args.len()) {
        ("get", 2..) => DictForm::Read(Opcode::DictGet),
        ("exists", 2..) => DictForm::Read(Opcode::DictExists),
        ("set", 3..) => DictForm::Set(var()?),
        ("unset", 2..) => DictForm::Unset(var()?),
        ("incr", 2 | 3) => {
            let amount = match args.get(2) {
                Some(word) => {
                    match Number::parse(word_literal(text, word.clone()).ok()?.as_bytes())? {
                        Number::Int(amount) => i32::try_from(amount).ok()?,
                        _ => return None,
                    }
                }
                None => 1,
            };
            DictForm::Incr(var()?, amount)
        }
        ("append", 3..=99) => DictForm::Append(var()?),
        ("lappend", 3) => DictForm::Lappend(var()?),
        ("for" | "map", 3) if text[args[2].clone()].starts_with('{') => {
            let names = list_elements(&word_literal(text, args[0].clone()).ok()?).ok()?;
            let [key, value] = <[String; 2]>::try_from(names).ok()?;
            let scalar = |name: &String| valid_var_name(name) && !name.contains("::");
            if !scalar(&key) || !scalar(&value) {
                return None;
            }
            if subcommand == "for" {
                DictForm::For(key, value)
            } else {
                DictForm::Map(key, value)
            }
        }
        ("with", 2..) if text[args.last().unwrap().clone()].starts_with('{') => {
            DictForm::With(local_scalar(text, &args[0]))
        }
        ("update", count) if count >= 4 && count % 2 == 0 => {
            let body = args.last().unwrap();
            if !text[body.clone()].starts_with('{') {
                return None;
            }
            let names = args[1..count - 1]
                .chunks(2)
                .map(|pair| local_scalar(text, &pair[1]))
                .collect::<Option<Vec<_>>>()?;
            DictForm::Update(var()?, names)
        }
        ("create", count) if count % 2 == 0 => {
            let keys = args
                .iter()
                .step_by(2)
                .map(|word| word_literal(text, word.clone()).ok())
                .collect::<Option<Vec<_>>>()?;
            let distinct = keys.iter().collect::<std::collections::BTreeSet<_>>().len();
            if distinct != keys.len() {
                return None;
            }
            DictForm::Create(keys)
        }
        _ => return None,
    })
}

impl Compiler<'_> {
    /// Emit `form` for a `dict` command whose words are already captured for
    /// its trace, so each operand compiles to a copy of its word.
    pub(super) fn compile_dict_form(
        &mut self,
        form: &DictForm,
        text: &str,
        words: &[Range<usize>],
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let args = &words[2..];
        match form {
            DictForm::For(key, value) => {
                return self.compile_dict_each(key, value, false, text, args, base, depth);
            }
            DictForm::Map(key, value) => {
                return self.compile_dict_each(key, value, true, text, args, base, depth);
            }
            DictForm::With(var) => {
                return self.compile_dict_with(var.as_deref(), text, args, base, depth);
            }
            DictForm::Update(var, names) => {
                return self.compile_dict_update(var, names, text, args, base, depth);
            }
            _ => {}
        }
        let operands = match form {
            DictForm::Read(_) | DictForm::Create(_) => args,
            DictForm::Incr(..) => &args[1..2],
            _ => &args[1..],
        };
        if let DictForm::Create(keys) = form {
            for (key, value) in keys.iter().zip(args.iter().skip(1).step_by(2)) {
                self.push_literal(key);
                self.compile_word(text, value.clone(), base, depth)?;
            }
            self.emit_arg(Opcode::List, args.len() as u32);
            return Ok(());
        }
        for word in operands {
            self.compile_word(text, word.clone(), base, depth)?;
        }
        match form {
            DictForm::Read(opcode) => self.emit_arg(*opcode, operands.len() as u32 - 1),
            DictForm::Set(var) => {
                let local = self.local(var) as u32;
                self.emit_args(Opcode::DictSet, &[operands.len() as u32 - 1, local]);
            }
            DictForm::Unset(var) => {
                let local = self.local(var) as u32;
                self.emit_args(Opcode::DictUnset, &[operands.len() as u32, local]);
            }
            DictForm::Incr(var, amount) => {
                let local = self.local(var) as u32;
                self.emit_args(Opcode::DictIncrImm, &[*amount as u32, local]);
            }
            DictForm::Append(var) => {
                if operands.len() > 2 {
                    self.emit_arg(Opcode::StrConcat1, operands.len() as u32 - 1);
                }
                let local = self.local(var) as u32;
                self.emit_arg(Opcode::DictAppend, local);
            }
            DictForm::Lappend(var) => {
                let local = self.local(var) as u32;
                self.emit_arg(Opcode::DictLappend, local);
            }
            DictForm::Create(_)
            | DictForm::For(..)
            | DictForm::Map(..)
            | DictForm::Update(..)
            | DictForm::With(_) => unreachable!(),
        }
        Ok(())
    }

    /// Reserve a catch range whose bounds are filled in once known.
    fn open_catch(&mut self, depth: u32) -> usize {
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
        index
    }

    fn close_catch(&mut self, index: usize, code: Range<usize>, handler: usize) {
        let range = &mut self.exceptions[index];
        range.code_offset = code.start as i32;
        range.num_code_bytes = (code.end - code.start) as i32;
        range.catch_offset = handler as i32;
        range.code_range = code.start as i32..code.end as i32;
    }

    /// `CompileDictEachCmd`: iterate under a catch so that an error ends the
    /// iteration before it propagates. A caught error also discards the
    /// iterator's runtime state, so only normal exits finish it explicitly.
    /// `dict map` (`collect`) gathers each body result under its key in a
    /// temporary dictionary, which is the command's result.
    #[allow(clippy::too_many_arguments)]
    fn compile_dict_each(
        &mut self,
        key: &str,
        value: &str,
        collect: bool,
        text: &str,
        args: &[Range<usize>],
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let collected = collect.then(|| self.temporary() as u32);
        let iterator = self.temporary() as u32;
        let key = self.local(key);
        let value = self.local(value);
        if let Some(collected) = collected {
            self.push_literal("");
            self.store_local(collected as usize);
        }
        self.compile_word(text, args[1].clone(), base, depth)?;
        let catch = self.open_catch(depth);
        let catch_start = self.code.len();
        self.emit_arg(Opcode::DictFirst, iterator);
        let empty = self.jump(Opcode::JumpTrue4);
        let assign = self.code.len();
        self.store_local(key);
        self.store_local(value);
        let body_start = self.code.len();
        // The body nests one source level down but two exception levels:
        // inside the iteration's catch and its loop.
        self.exception_bias += 1;
        self.return_context.push(ExceptionType::Loop);
        let body = self.body(text, &args[2], base, depth + 1);
        self.return_context.pop();
        self.exception_bias -= 1;
        body?;
        if let Some(collected) = collected {
            self.emit_arg(Opcode::LoadScalar4, key as u32);
            self.emit_arg(Opcode::Over, 1);
            self.emit_args(Opcode::DictSet, &[1, collected]);
            self.emit(Opcode::Pop);
        }
        self.emit(Opcode::Pop);
        let body_end = self.code.len();
        self.emit_arg(Opcode::DictNext, iterator);
        let again = self.jump(Opcode::JumpFalse4);
        self.patch_jump(again, assign);
        let finished = self.jump(Opcode::Jump4);
        let handler = self.code.len();
        self.emit(Opcode::PushReturnOptions);
        self.emit(Opcode::PushResult);
        self.emit(Opcode::EndCatch);
        self.emit(Opcode::ReturnStk);
        self.patch_jump(empty, self.code.len());
        self.patch_jump(finished, self.code.len());
        // Drop the empty key/value pair that ended the iteration.
        self.emit(Opcode::Pop);
        self.emit(Opcode::Pop);
        let mut exit = self.code.len();
        self.emit(Opcode::EndCatch);
        self.emit_arg(Opcode::DictDone, iterator);
        match collected {
            // Outside a procedure Tcl runs `dict map` as a command, whose
            // `break` discards what the iteration collected.
            Some(collected) if self.static_declarations => {
                self.emit_arg(Opcode::LoadScalar4, collected);
                let done = self.jump(Opcode::Jump4);
                exit = self.code.len();
                self.emit(Opcode::EndCatch);
                self.emit_arg(Opcode::DictDone, iterator);
                self.push_literal("");
                self.patch_jump(done, self.code.len());
            }
            Some(collected) => self.emit_arg(Opcode::LoadScalar4, collected),
            None => self.push_literal(""),
        }
        self.close_catch(catch, catch_start..body_end, handler);
        self.loop_range(body_start, body_end, exit, body_end, depth + 1);
        Ok(())
    }

    /// `TclCompileDictUpdateCmd`: bind the keys, run the body under a
    /// catch, and write the variables back however the body finished.
    fn compile_dict_update(
        &mut self,
        var: &str,
        names: &[String],
        text: &str,
        args: &[Range<usize>],
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        for pair in args[1..args.len() - 1].chunks(2) {
            self.compile_word(text, pair[0].clone(), base, depth)?;
        }
        self.emit_arg(Opcode::List, names.len() as u32);
        let dict = self.local(var) as u32;
        let indices = names.iter().map(|name| self.local(name) as i32).collect();
        let info = self.auxiliaries.len() as u32;
        self.auxiliaries.push(AuxData {
            kind: AuxDataType::DictUpdate,
            value: AuxDataValue::DictUpdate(DictUpdateInfo { indices }),
        });
        self.emit_args(Opcode::DictUpdateStart, &[dict, info]);
        let catch = self.open_catch(depth);
        let body_start = self.code.len();
        self.return_context.push(ExceptionType::Catch);
        self.body(text, args.last().unwrap(), base, depth + 1)?;
        self.return_context.pop();
        let body_end = self.code.len();
        self.emit(Opcode::EndCatch);
        self.emit_arg(Opcode::Reverse, 2);
        self.emit_args(Opcode::DictUpdateEnd, &[dict, info]);
        let done = self.jump(Opcode::Jump4);
        let handler = self.code.len();
        self.emit(Opcode::PushResult);
        self.emit(Opcode::PushReturnOptions);
        self.emit(Opcode::EndCatch);
        self.emit_arg(Opcode::Reverse, 3);
        self.emit_args(Opcode::DictUpdateEnd, &[dict, info]);
        self.emit(Opcode::ReturnStk);
        self.patch_jump(done, self.code.len());
        self.close_catch(catch, body_start..body_end, handler);
        Ok(())
    }

    /// `TclCompileDictWithCmd`: expand the dictionary (at the key path) into
    /// variables, run the body under a catch, and fold the variables back
    /// into the dictionary however the body finished. An empty body needs
    /// neither the catch nor temporaries.
    fn compile_dict_with(
        &mut self,
        var: Option<&str>,
        text: &str,
        args: &[Range<usize>],
        base: usize,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let (body, rest) = args.split_last().unwrap();
        let path = &rest[1..];
        let dict = var.map(|name| self.local(name) as u32);
        let script = &text[body.start + 1..body.end - 1];
        if script
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            if dict.is_none() {
                self.compile_word(text, args[0].clone(), base, depth)?;
            }
            for word in path {
                self.compile_word(text, word.clone(), base, depth)?;
            }
            match (dict, path.is_empty()) {
                (Some(dict), true) => {
                    self.push_literal("");
                    self.emit_arg(Opcode::LoadScalar4, dict);
                    self.push_literal("");
                }
                (Some(dict), false) => {
                    self.emit_arg(Opcode::List, path.len() as u32);
                    self.emit_arg(Opcode::LoadScalar4, dict);
                    self.emit_arg(Opcode::Over, 1);
                }
                (None, true) => {
                    self.emit(Opcode::Dup);
                    self.emit(Opcode::LoadStk);
                    self.push_literal("");
                }
                (None, false) => {
                    self.emit_arg(Opcode::List, path.len() as u32);
                    self.emit_arg(Opcode::Over, 1);
                    self.emit(Opcode::LoadStk);
                    self.emit_arg(Opcode::Over, 1);
                }
            }
            self.emit(Opcode::DictExpand);
            match dict {
                Some(dict) => self.emit_arg(Opcode::DictRecombineImm, dict),
                None => {
                    if path.is_empty() {
                        self.push_literal("");
                        self.emit_arg(Opcode::Reverse, 2);
                    }
                    self.emit(Opcode::DictRecombineStk);
                }
            }
            self.push_literal("");
            return Ok(());
        }
        let name = dict.is_none().then(|| self.temporary() as u32);
        let path_list = (!path.is_empty()).then(|| self.temporary() as u32);
        let keys = self.temporary() as u32;
        if let Some(name) = name {
            // The name stays on the stack to load the dictionary.
            self.compile_word(text, args[0].clone(), base, depth)?;
            self.emit_arg(Opcode::StoreScalar4, name);
        }
        if let Some(path_list) = path_list {
            for word in path {
                self.compile_word(text, word.clone(), base, depth)?;
            }
            self.emit_arg(Opcode::List, path.len() as u32);
            self.store_local(path_list as usize);
        }
        match dict {
            Some(dict) => self.emit_arg(Opcode::LoadScalar4, dict),
            None => self.emit(Opcode::LoadStk),
        }
        match path_list {
            Some(path_list) => self.emit_arg(Opcode::LoadScalar4, path_list),
            None => self.push_literal(""),
        }
        self.emit(Opcode::DictExpand);
        self.store_local(keys as usize);
        let catch = self.open_catch(depth);
        let body_start = self.code.len();
        self.return_context.push(ExceptionType::Catch);
        self.body(text, body, base, depth + 1)?;
        self.return_context.pop();
        let body_end = self.code.len();
        self.emit(Opcode::EndCatch);
        self.recombine_dict(dict, name, path_list, keys);
        let done = self.jump(Opcode::Jump4);
        let handler = self.code.len();
        self.emit(Opcode::PushReturnOptions);
        self.emit(Opcode::PushResult);
        self.emit(Opcode::EndCatch);
        self.recombine_dict(dict, name, path_list, keys);
        self.emit(Opcode::ReturnStk);
        self.patch_jump(done, self.code.len());
        self.close_catch(catch, body_start..body_end, handler);
        Ok(())
    }

    /// Fold the variables `dict with` made back into the dictionary in the
    /// local `dict`, or in the variable named in the temporary `name`.
    fn recombine_dict(
        &mut self,
        dict: Option<u32>,
        name: Option<u32>,
        path_list: Option<u32>,
        keys: u32,
    ) {
        if let Some(name) = name {
            self.emit_arg(Opcode::LoadScalar4, name);
        }
        match path_list {
            Some(path_list) => self.emit_arg(Opcode::LoadScalar4, path_list),
            None => self.push_literal(""),
        }
        self.emit_arg(Opcode::LoadScalar4, keys);
        match dict {
            Some(dict) => self.emit_arg(Opcode::DictRecombineImm, dict),
            None => self.emit(Opcode::DictRecombineStk),
        }
    }
}
