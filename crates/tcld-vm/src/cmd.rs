//! Command registry and resolution classes (executable plan §4).

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::VmError;
use crate::load::LoadedCode;
use crate::mode::RuntimeMode;
use crate::value::{ByteString, VmObj};

/// Closed VarNameBuiltin set (executable plan §4).
pub const VAR_NAME_BUILTINS: &[&str] = &[
    "set", "append", "lappend", "incr", "unset", "global", "variable",
];

/// Value commands the VM runs natively in every mode (`builtin::run_value_builtin`).
pub const VALUE_BUILTINS: &[&str] = &[
    "list", "llength", "lindex", "lrange", "concat", "join", "split", "error",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandClass {
    Facade,
    VmProc,
    VarNameBuiltin,
    WorldHost,
    Denied,
}

/// Normalize command table key: strip one leading `::` for lookup.
pub fn normalize_cmd_key(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"::").unwrap_or(name)
}

pub fn is_var_name_builtin(name: &[u8]) -> bool {
    let key = normalize_cmd_key(name);
    VAR_NAME_BUILTINS.iter().any(|b| key == b.as_bytes())
}

pub fn is_value_builtin(name: &[u8]) -> bool {
    let key = normalize_cmd_key(name);
    VALUE_BUILTINS.iter().any(|b| key == b.as_bytes())
}

pub fn is_facade_name(name: &[u8]) -> bool {
    let key = normalize_cmd_key(name);
    matches!(
        key,
        b"tbcload::bcproc" | b"tbcload::bceval" | b"tbcproc" | b"tbceval"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacadeKind {
    BcProc,
    BcEval,
}

pub fn facade_kind(name: &[u8]) -> Option<FacadeKind> {
    let key = normalize_cmd_key(name);
    match key {
        b"tbcload::bcproc" | b"tbcproc" => Some(FacadeKind::BcProc),
        b"tbcload::bceval" | b"tbceval" => Some(FacadeKind::BcEval),
        _ => None,
    }
}

/// Registered bytecode procedure.
#[derive(Debug, Clone)]
pub struct VmProc {
    pub qname: ByteString,
    pub args_spec: ByteString,
    /// The body as written, for a procedure compiled from source.
    pub body: Option<ByteString>,
    pub code: Arc<LoadedCode>,
    pub num_args: i32,
    pub num_locals: i32,
}

impl VmProc {
    /// `info body`: the body as written. A precompiled procedure reports the
    /// placeholder TclPro's loader gives it.
    pub fn body_text(&self) -> VmObj {
        match &self.body {
            Some(body) => VmObj::Bytes(body.clone()),
            None => VmObj::from_str(
                "# Compiled -- no source code available\nerror \"called a copy of a compiled script\"",
            ),
        }
    }

    /// `info args`: the formal argument names.
    pub fn argument_names(&self) -> Result<VmObj, VmError> {
        let mut names = Vec::new();
        for formal in VmObj::Bytes(self.args_spec.clone()).as_list_vec()? {
            names.extend(formal.as_list_vec()?.into_iter().next());
        }
        Ok(VmObj::List(names))
    }
}

#[derive(Debug, Clone)]
pub enum CommandRef {
    Facade(FacadeKind),
    VmProc(VmProc),
    VarNameBuiltin { name: ByteString },
    ValueBuiltin { name: ByteString },
    WorldHost { name: ByteString },
    Denied { name: ByteString },
}

#[derive(Debug, Default)]
pub struct CommandRegistry {
    procs: HashMap<ByteString, VmProc>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_vm_proc(&mut self, proc: VmProc) {
        self.procs.insert(proc.qname.clone(), proc);
    }

    pub fn rename_vm_proc(&mut self, old: &ByteString, new: Option<ByteString>) {
        if let Some(mut proc) = self.procs.remove(old)
            && let Some(new) = new
        {
            proc.qname = new.clone();
            self.procs.insert(new, proc);
        }
    }

    pub fn get_vm_proc(&self, name: &ByteString) -> Option<&VmProc> {
        self.procs.get(name).or_else(|| {
            let key = normalize_cmd_key(name.as_bytes());
            self.procs
                .iter()
                .find(|(k, _)| normalize_cmd_key(k.as_bytes()) == key)
                .map(|(_, v)| v)
        })
    }

    pub fn vm_proc_count(&self) -> usize {
        self.procs.len()
    }

    pub fn vm_proc_names(&self) -> Vec<String> {
        let mut names = self
            .procs
            .keys()
            .map(|name| name.as_str_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn resolve(
        &self,
        cmd_word: &ByteString,
        current_ns: &ByteString,
        mode: RuntimeMode,
    ) -> CommandRef {
        let raw = cmd_word.as_bytes();
        let key = normalize_cmd_key(raw);

        if let Some(kind) = facade_kind(key) {
            return CommandRef::Facade(kind);
        }

        // Exact VmProc
        if let Some(p) = self.lookup_proc(raw, current_ns) {
            return CommandRef::VmProc(p.clone());
        }

        if is_var_name_builtin(key) {
            return CommandRef::VarNameBuiltin {
                name: ByteString::from_bytes(key.to_vec()),
            };
        }

        if is_value_builtin(key) {
            return CommandRef::ValueBuiltin {
                name: ByteString::from_bytes(key.to_vec()),
            };
        }

        match mode {
            RuntimeMode::VmStrict => CommandRef::Denied {
                name: cmd_word.clone(),
            },
            RuntimeMode::Hybrid => CommandRef::WorldHost {
                name: cmd_word.clone(),
            },
        }
    }

    fn lookup_proc(&self, raw: &[u8], current_ns: &ByteString) -> Option<&VmProc> {
        if raw.starts_with(b"::") {
            return self.procs.get(&ByteString::from_bytes(raw.to_vec()));
        }
        let key = raw;

        let ns = current_ns.as_str_lossy();
        let namespaced = if ns.as_ref() == "::" {
            format!("::{}", String::from_utf8_lossy(key))
        } else {
            format!("{ns}::{}", String::from_utf8_lossy(key))
        };
        self.procs
            .get(&ByteString::from(namespaced.as_str()))
            .or_else(|| {
                self.procs.get(&ByteString::from(
                    format!("::{}", String::from_utf8_lossy(key)).as_str(),
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_name_set_is_closed() {
        assert!(is_var_name_builtin(b"append"));
        assert!(is_var_name_builtin(b"::set"));
        assert!(!is_var_name_builtin(b"puts"));
    }

    #[test]
    fn facade_names() {
        assert!(is_facade_name(b"tbcload::bcproc"));
        assert!(is_facade_name(b"tbcproc"));
        assert_eq!(facade_kind(b"tbceval"), Some(FacadeKind::BcEval));
    }

    #[test]
    fn resolve_vm_strict_denies_puts() {
        let reg = CommandRegistry::new();
        let r = reg.resolve(
            &ByteString::from("puts"),
            &ByteString::from("::"),
            RuntimeMode::VmStrict,
        );
        assert!(matches!(r, CommandRef::Denied { .. }));
    }
}
