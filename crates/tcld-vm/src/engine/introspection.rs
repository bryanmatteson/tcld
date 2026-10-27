//! The commands the VM answers itself in source images and vm-strict:
//! `info`, `rename`, `lsearch` and `lsort` from the VM's own procedures,
//! levels and variables, the `string` and `dict` ensembles, `regexp` and
//! `regsub`, and in vm-strict `puts` to stdout and stderr. Hybrid mode adds
//! embedded Tcl's commands and procedures to the listings, runs the forms
//! that evaluate a script (`dict for`, ...), and writes through Tcl's channels.
//! `regsub -command` and `lsort -command` call mapped VM code in both modes.
use super::*;
use crate::args::{tcl_error, unique_prefix, wrong_args};
use crate::dict_command::Dispatch;
use crate::output::Stream;
use crate::regexp_command::Regsub;
use crate::value::match_is_trivial;

/// `info` subcommands (`defaultInfoMap`), for Tcl's prefix matching.
const INFO_SUBCOMMANDS: [&str; 27] = [
    "args",
    "body",
    "cmdcount",
    "cmdtype",
    "commands",
    "complete",
    "constant",
    "consts",
    "coroutine",
    "default",
    "errorstack",
    "exists",
    "frame",
    "functions",
    "globals",
    "hostname",
    "level",
    "library",
    "loaded",
    "locals",
    "nameofexecutable",
    "patchlevel",
    "procs",
    "script",
    "sharedlibextension",
    "tclversion",
    "vars",
];

/// Tcl's commands that vm-strict provides itself, which `info commands`
/// lists beside the program's procedures.
const VM_STRICT_COMMANDS: &[&str] = &[
    "append",
    #[cfg(feature = "debug")]
    "apply",
    "break",
    "catch",
    "clock",
    "concat",
    "continue",
    "dict",
    "error",
    "eval",
    "expr",
    "for",
    "foreach",
    "global",
    "if",
    "incr",
    "info",
    "join",
    "lappend",
    "lindex",
    "list",
    "llength",
    #[cfg(feature = "debug")]
    "lmap",
    "lrange",
    "lsearch",
    "lsort",
    "namespace",
    "proc",
    "puts",
    "regexp",
    "regsub",
    "rename",
    "return",
    "set",
    "split",
    "string",
    #[cfg(feature = "debug")]
    "switch",
    #[cfg(feature = "debug")]
    "try",
    "unset",
    "uplevel",
    "upvar",
    "variable",
    "while",
];

/// Procedures the hybrid host bootstrap defines to wrap Tcl commands; they
/// are not the program's procedures.
const HOST_WRAPPER_PROCS: [&str; 5] = ["namespace", "uplevel", "upvar", "yield", "yieldto"];

/// A qualified name's namespace and tail: `::a::b` is `::a` and `b`.
fn split_qualified(name: &[u8]) -> (ByteString, ByteString) {
    let at = name.windows(2).rposition(|pair| pair == b"::");
    match at {
        Some(at) => {
            let namespace = if at == 0 {
                b"::".as_slice()
            } else {
                &name[..at]
            };
            (
                ByteString::from_bytes(namespace.to_vec()),
                ByteString::from_bytes(name[at + 2..].to_vec()),
            )
        }
        None => (
            ByteString::from("::"),
            ByteString::from_bytes(name.to_vec()),
        ),
    }
}

fn join_qualified(namespace: &ByteString, tail: &ByteString) -> VmObj {
    let mut out = if namespace.as_bytes() == b"::" {
        Vec::new()
    } else {
        namespace.as_bytes().to_vec()
    };
    out.extend_from_slice(b"::");
    out.extend_from_slice(tail.as_bytes());
    VmObj::from_bytes(out)
}

impl HybridEngineInner {
    /// A command the VM answers itself, or `None` for a form left to the
    /// host or denied.
    pub(super) fn invoke_native(
        &mut self,
        words: &[VmObj],
        site: CodeAddress,
    ) -> Result<Option<InvokeOutcome>, VmError> {
        let name = words[0].as_string()?;
        let value = match name.strip_prefix("::").unwrap_or(&name) {
            #[cfg(feature = "debug")]
            "apply" => return self.invoke_apply(words, site).map(Some),
            #[cfg(feature = "debug")]
            "switch" => return self.invoke_switch(words, site).map(Some),
            #[cfg(feature = "debug")]
            "lmap" => return self.invoke_lmap(words, site).map(Some),
            #[cfg(feature = "debug")]
            "try" => return self.invoke_try(words, site).map(Some),
            "return" => {
                let completion = Completion::from_return_args(&words[1..])?;
                return Ok(Some(if completion.code == 0 {
                    self.last_completion = completion.clone();
                    InvokeOutcome::Value(completion.result)
                } else {
                    InvokeOutcome::Raise(completion)
                }));
            }
            "string" => crate::string_command::run(words, &mut EngineVarCtx { engine: self })?,
            "dict" => match crate::dict_command::run(words, &mut EngineVarCtx { engine: self })? {
                Dispatch::Value(value) => value,
                Dispatch::Host(_) if self.mode == RuntimeMode::Hybrid => return Ok(None),
                Dispatch::Host(reason) => {
                    return Err(VmError::PolicyDenied {
                        action: "world_host",
                        detail: format!("{reason} and requires hybrid mode"),
                    });
                }
            },
            "info" => match self.native_info(words)? {
                Some(value) => value,
                None => return Ok(None),
            },
            "rename" if self.mode == RuntimeMode::VmStrict => self.native_rename(words)?,
            "puts" if self.mode == RuntimeMode::VmStrict => self.native_puts(words)?,
            "lsearch" => crate::lsearch::search(&words[0], &words[1..])?,
            "regexp" => crate::regexp_command::regexp(words, &mut EngineVarCtx { engine: self })?,
            "regsub" => {
                match crate::regexp_command::regsub(words, &mut EngineVarCtx { engine: self })? {
                    Regsub::Value(value) => value,
                    Regsub::Command(command) => {
                        match command.substitute(|words| {
                            self.command_callback(
                                words,
                                site,
                                "regsub-command",
                                "\n    (-command substitution computation script)",
                            )
                        })? {
                            Ok((count, value)) => {
                                command.finish(count, value, &mut EngineVarCtx { engine: self })?
                            }
                            Err(completion) => return Ok(Some(InvokeOutcome::Raise(completion))),
                        }
                    }
                }
            }
            "lsort" => match crate::lsort::sort(&words[0], &words[1..])? {
                crate::lsort::Sort::Result(value) => value,
                crate::lsort::Sort::Command(command) => match command.sort(|words| {
                    self.command_callback(words, site, "lsort-command", "\n    (-compare command)")
                })? {
                    Ok(value) => value,
                    Err(completion) => return Ok(Some(InvokeOutcome::Raise(completion))),
                },
            },
            _ => return Ok(None),
        };
        Ok(Some(InvokeOutcome::Value(value)))
    }

    fn native_info(&mut self, words: &[VmObj]) -> Result<Option<VmObj>, VmError> {
        let Some(subcommand) = words.get(1) else {
            return Ok(None);
        };
        let Some(subcommand) = unique_prefix(&INFO_SUBCOMMANDS, &subcommand.as_string()?) else {
            return Ok(None);
        };
        let args = &words[2..];
        let invoked = words[0].as_string()?;
        let usage = |valid: bool, usage: &str| {
            if valid {
                Ok(())
            } else {
                Err(wrong_args(&[&invoked, subcommand], usage))
            }
        };
        Ok(Some(match subcommand {
            "exists" => {
                usage(args.len() == 1, "varName")?;
                VmObj::Int(i64::from(self.variable_exists(&args[0])?))
            }
            "commands" => {
                usage(args.len() <= 1, "?pattern?")?;
                self.info_listing(args.first(), false)?
            }
            "procs" => {
                usage(args.len() <= 1, "?pattern?")?;
                self.info_listing(args.first(), true)?
            }
            "body" | "args" => {
                usage(args.len() == 1, "procname")?;
                let namespace = self.frames[self.fi()].namespace.clone();
                match self.registry.resolve(
                    &args[0].ensure_bytes()?,
                    namespace.as_bytes(),
                    self.mode,
                ) {
                    CommandRef::VmProc(proc) if subcommand == "body" => proc.body_text(),
                    CommandRef::VmProc(proc) => proc.argument_names()?,
                    _ if self.mode == RuntimeMode::Hybrid => return Ok(None),
                    _ => {
                        let name = args[0].ensure_bytes()?;
                        return Err(tcl_error(
                            format!("\"{}\" isn't a procedure", name.as_str_lossy()),
                            &["TCL", "LOOKUP", "PROCEDURE", &name.as_str_lossy()],
                        ));
                    }
                }
            }
            "level" => {
                usage(args.len() <= 1, "?number?")?;
                self.info_level(args.first())?
            }
            _ => return Ok(None),
        }))
    }

    /// `info exists`: a variable, or array element, that holds a value.
    fn variable_exists(&mut self, name: &VmObj) -> Result<bool, VmError> {
        let name = name.ensure_bytes()?;
        let (name1, name2) = split_var_name(&name);
        Ok(match name2 {
            Some(element) => self.var_get(&name1, Some(&element)).is_ok(),
            None => self.var_get(&name1, None).is_ok() || self.array_exists_named(&name1)?,
        })
    }

    /// `info level ?number?`: the current level, or the words that created a
    /// level (relative when not positive).
    fn info_level(&self, number: Option<&VmObj>) -> Result<VmObj, VmError> {
        let chain = self.tcl_scope_chain();
        let current = chain.len().saturating_sub(1) as i64;
        let Some(number) = number else {
            return Ok(VmObj::Int(current));
        };
        let requested = match crate::number::of(number) {
            Some(crate::number::Number::Int(value)) if i32::try_from(value).is_ok() => value,
            _ => return Err(crate::number::expected("integer", number)),
        };
        // Compiled Tcl (`INST_INFO_LEVEL_ARGS`) reports a stack level.
        let text = number.ensure_bytes()?;
        let bad = || {
            tcl_error(
                format!("bad level \"{}\"", text.as_str_lossy()),
                &["TCL", "LOOKUP", "STACK_LEVEL", &text.as_str_lossy()],
            )
        };
        let level = if requested <= 0 {
            if current == 0 {
                return Err(bad());
            }
            current + requested
        } else {
            requested
        };
        if level < 1 || level > current {
            return Err(bad());
        }
        Ok(self.frames[chain[level as usize]]
            .invocation
            .clone()
            .unwrap_or_else(|| VmObj::List(Vec::new())))
    }

    /// The namespace a listing pattern names, its simple pattern, and whether
    /// the pattern named the namespace; `None` when that namespace is unknown.
    fn listing_scope(
        &mut self,
        pattern: Option<&VmObj>,
    ) -> Result<Option<(ByteString, Option<String>, bool)>, VmError> {
        let current = self.frames[self.fi()].namespace.as_bytes().clone();
        let Some(pattern) = pattern else {
            return Ok(Some((current, None, false)));
        };
        let text = pattern.as_string()?;
        let Some(split) = text.rfind("::") else {
            return Ok(Some((current, Some(text), false)));
        };
        let simple = text[split + 2..].to_string();
        let qualifier = text[..split].trim_end_matches(':');
        let candidates = if text.starts_with("::") {
            vec![crate::world::canonical(&ByteString::from(qualifier))]
        } else {
            vec![
                crate::world::canonical(
                    &NamespaceName(current).qualify(&ByteString::from(qualifier)),
                ),
                crate::world::canonical(&ByteString::from(qualifier)),
            ]
        };
        for namespace in candidates {
            if self.namespace_known(&namespace)? {
                return Ok(Some((namespace, Some(simple), true)));
            }
        }
        Ok(None)
    }

    fn namespace_known(&mut self, namespace: &ByteString) -> Result<bool, VmError> {
        if namespace.as_bytes() == b"::"
            || self
                .registry
                .vm_proc_names()
                .iter()
                .any(|name| split_qualified(name.as_bytes()).0 == *namespace)
        {
            return Ok(true);
        }
        if self.mode == RuntimeMode::VmStrict {
            return Ok(self.world.namespace_exists(namespace));
        }
        match self.host_value_result(&[
            VmObj::from_str("namespace"),
            VmObj::from_str("exists"),
            VmObj::Bytes(namespace.clone()),
        ])? {
            Ok(exists) => exists.is_true(),
            Err(completion) => Err(completion_to_error(completion)),
        }
    }

    /// The tails of the commands, or procedures, defined in a namespace.
    fn defined_in(
        &mut self,
        namespace: &ByteString,
        procs: bool,
    ) -> Result<Vec<ByteString>, VmError> {
        let mut names = self
            .registry
            .vm_proc_names()
            .iter()
            .map(|name| split_qualified(name.as_bytes()))
            .filter(|(owner, _)| owner == namespace)
            .map(|(_, tail)| tail)
            .collect::<Vec<_>>();
        let root = namespace.as_bytes() == b"::";
        if self.mode == RuntimeMode::VmStrict {
            if root && !procs {
                names.extend(
                    VM_STRICT_COMMANDS
                        .iter()
                        .map(|name| ByteString::from(*name)),
                );
            }
            return Ok(names);
        }
        let pattern = if root {
            "::*".to_string()
        } else {
            format!("{}::*", namespace.as_str_lossy())
        };
        let listed = match self.host_value_result(&[
            VmObj::from_str("info"),
            VmObj::from_str(if procs { "procs" } else { "commands" }),
            VmObj::from_str(&pattern),
        ])? {
            Ok(listed) => listed.as_list_vec()?,
            Err(completion) => return Err(completion_to_error(completion)),
        };
        for qualified in listed {
            let (_, tail) = split_qualified(qualified.ensure_bytes()?.as_bytes());
            let wrapper = procs && root && HOST_WRAPPER_PROCS.contains(&&*tail.as_str_lossy());
            if !wrapper && !names.contains(&tail) {
                names.push(tail);
            }
        }
        Ok(names)
    }

    /// `info commands` (`InfoCommandsCmd`) or `info procs` (`InfoProcsCmd`):
    /// names in the effective namespace matching the pattern; commands also
    /// include unshadowed global commands.
    fn info_listing(&mut self, pattern: Option<&VmObj>, procs: bool) -> Result<VmObj, VmError> {
        let Some((namespace, simple, specific)) = self.listing_scope(pattern)? else {
            return Ok(VmObj::List(Vec::new()));
        };
        let local = self.defined_in(&namespace, procs)?;
        let root = ByteString::from("::");
        let global = if !procs && namespace != root && !specific {
            self.defined_in(&root, false)?
        } else {
            Vec::new()
        };
        let shown = |tail: &ByteString| {
            if specific {
                join_qualified(&namespace, tail)
            } else {
                VmObj::Bytes(tail.clone())
            }
        };
        let matches = |tail: &ByteString| {
            simple
                .as_deref()
                .is_none_or(|pattern| glob_match(pattern, &tail.as_str_lossy(), false))
        };
        let mut out = Vec::new();
        match simple.as_deref() {
            Some(pattern) if match_is_trivial(pattern) => {
                let name = ByteString::from(pattern);
                if local.contains(&name) {
                    out.push(shown(&name));
                } else if global.contains(&name) {
                    out.push(VmObj::Bytes(name));
                }
            }
            _ => {
                out.extend(local.iter().filter(|tail| matches(tail)).map(shown));
                out.extend(
                    global
                        .iter()
                        .filter(|tail| matches(tail) && !local.contains(tail))
                        .map(|tail| VmObj::Bytes(tail.clone())),
                );
            }
        }
        Ok(VmObj::List(out))
    }

    /// `puts ?-nonewline? ?channel? string` (`Tcl_PutsObjCmd`) in vm-strict,
    /// whose only channels are stdout and stderr.
    fn native_puts(&mut self, words: &[VmObj]) -> Result<VmObj, VmError> {
        let flag = |word: &VmObj| word.as_string().is_ok_and(|word| word == "-nonewline");
        let (channel, text, newline) = match words {
            [_, text] => (None, text, true),
            [_, first, text] if flag(first) => (None, text, false),
            [_, channel, text] => (Some(channel), text, true),
            [_, first, channel, text] if flag(first) => (Some(channel), text, false),
            _ => {
                return Err(wrong_args(
                    &[&words[0].as_string()?],
                    "?-nonewline? ?channel? string",
                ));
            }
        };
        let name = channel.map_or_else(|| Ok("stdout".to_string()), VmObj::as_string)?;
        let Some(stream) = Stream::from_channel(&name) else {
            return Err(VmError::PolicyDenied {
                action: "world_host",
                detail: format!("puts to channel `{name}` requires hybrid mode"),
            });
        };
        let mut bytes = text.ensure_bytes()?.as_bytes().to_vec();
        if newline {
            bytes.push(b'\n');
        }
        self.output.write(stream, &bytes)?;
        Ok(VmObj::empty())
    }

    /// `rename` of the program's procedures in vm-strict. Tcl's own commands
    /// live in the host, which vm-strict does not have.
    fn native_rename(&mut self, words: &[VmObj]) -> Result<VmObj, VmError> {
        if words.len() != 3 {
            return Err(wrong_args(&[&words[0].as_string()?], "oldName newName"));
        }
        let old = words[1].ensure_bytes()?;
        let new = words[2].ensure_bytes()?;
        let namespace = self.frames[self.fi()].namespace.clone();
        let proc = match self.registry.resolve(&old, namespace.as_bytes(), self.mode) {
            CommandRef::VmProc(proc) => proc,
            _ if tcld_compile::is_tcl_builtin_command(&old.as_str_lossy()) => {
                return Err(VmError::PolicyDenied {
                    action: "world_host",
                    detail: format!(
                        "renaming Tcl command `{}` requires hybrid mode",
                        old.as_str_lossy()
                    ),
                });
            }
            _ => {
                return Err(tcl_error(
                    format!(
                        "can't {} \"{}\": command doesn't exist",
                        if new.is_empty() { "delete" } else { "rename" },
                        old.as_str_lossy()
                    ),
                    &["TCL", "LOOKUP", "COMMAND", &old.as_str_lossy()],
                ));
            }
        };
        if new.is_empty() {
            self.registry.rename_vm_proc(&proc.qname, None);
            return Ok(VmObj::empty());
        }
        let target = crate::world::canonical(&namespace.qualify(&new));
        let (target_namespace, tail) = split_qualified(target.as_bytes());
        let taken = self.registry.get_vm_proc(&target).is_some()
            || (target_namespace.as_bytes() == b"::"
                && tcld_compile::is_tcl_builtin_command(&tail.as_str_lossy()));
        if taken {
            return Err(tcl_error(
                format!(
                    "can't rename to \"{}\": command already exists",
                    new.as_str_lossy()
                ),
                &["TCL", "OPERATION", "RENAME", "TARGET_EXISTS"],
            ));
        }
        self.world.create_namespace(&target_namespace);
        self.registry.rename_vm_proc(&proc.qname, Some(target));
        Ok(VmObj::empty())
    }
}
