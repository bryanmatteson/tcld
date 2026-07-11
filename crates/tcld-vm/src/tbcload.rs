//! tbcload façade (S06).

use std::sync::Arc;

use crate::cmd::{CommandRegistry, FacadeKind, VmProc};
use crate::error::VmError;
use crate::load::{LoadedCode, locals_meta_from_bc};
use crate::value::{ByteString, VmObj};

/// Handle `tbcload::bcproc name args body` — register VmProc. Never marshals body to host.
pub fn handle_bcproc(words: &[VmObj], registry: &mut CommandRegistry) -> Result<VmObj, VmError> {
    handle_bcproc_in_namespace(words, registry, &ByteString::from("::"))
}

pub fn handle_bcproc_in_namespace(
    words: &[VmObj],
    registry: &mut CommandRegistry,
    namespace: &ByteString,
) -> Result<VmObj, VmError> {
    // words[0]=cmd, [1]=name, [2]=args_spec, [3]=body
    if words.len() != 4 {
        return Err(VmError::ExtractBadBody(format!(
            "bcproc expects 3 arguments, got {}",
            words.len().saturating_sub(1)
        )));
    }
    let name = words[1].ensure_bytes()?;
    let args_spec = words[2].ensure_bytes()?;
    let code = match &words[3] {
        VmObj::ProcBody(code) => Arc::clone(code),
        _ => return Err(VmError::BcprocBodyNotBytecode),
    };

    let qname = qualify_proc_name(&name, namespace);
    let num_args = code.bytecode.proc_num_args;
    let num_locals = code.bytecode.proc_num_locals;
    // Validate locals meta can be decoded (defaults).
    let _ = locals_meta_from_bc(&code.bytecode.locals)?;

    registry.register_vm_proc(VmProc {
        qname,
        args_spec,
        code,
        num_args,
        num_locals,
    });
    Ok(VmObj::empty())
}

/// Handle `tbcload::bceval body` — run a ProcBody and return its result.
/// The engine supplies the run callback.
pub fn handle_bceval_body(words: &[VmObj]) -> Result<Arc<LoadedCode>, VmError> {
    if words.len() != 2 {
        return Err(VmError::ExtractBadBody(format!(
            "bceval expects 1 argument, got {}",
            words.len().saturating_sub(1)
        )));
    }
    match &words[1] {
        VmObj::ProcBody(code) => Ok(Arc::clone(code)),
        _ => Err(VmError::BcprocBodyNotBytecode),
    }
}

pub fn facade_kind_name(kind: FacadeKind) -> &'static str {
    match kind {
        FacadeKind::BcProc => "tbcload::bcproc",
        FacadeKind::BcEval => "tbcload::bceval",
    }
}

fn qualify_proc_name(name: &ByteString, namespace: &ByteString) -> ByteString {
    if name.as_bytes().starts_with(b"::") {
        name.clone()
    } else if name.as_bytes().contains(&b':') {
        // already ns-qualified without leading ::
        ByteString::from(format!("::{}", name.as_str_lossy()).as_str())
    } else {
        let ns = namespace.as_str_lossy();
        if ns.as_ref() == "::" {
            ByteString::from(format!("::{}", name.as_str_lossy()).as_str())
        } else {
            ByteString::from(format!("{ns}::{}", name.as_str_lossy()).as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcproc_rejects_non_body() {
        let mut reg = CommandRegistry::new();
        let words = [
            VmObj::from_str("tbcload::bcproc"),
            VmObj::from_str("p"),
            VmObj::from_str("x"),
            VmObj::from_str("not-a-body"),
        ];
        let err = handle_bcproc(&words, &mut reg).unwrap_err();
        assert!(matches!(err, VmError::BcprocBodyNotBytecode));
    }
}
