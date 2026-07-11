//! ByteCode → LoadedCode (S03).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tcld_core::bytecode::{
    ByteCode, CompiledLocal, Header, LiteralObject, LiteralObjectType, ProcBody,
};
use tcld_core::inst_table::{InstructionDesc, Opcode};
use tcld_core::source::SourceRef;
use tcld_decompile::{LoadInput, load_bytecode};

use crate::error::VmError;
use crate::value::{ByteString, VmObj, parse_tcl_int};

/// Unique id for a loaded code object (stable for a process session).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeId(pub u64);

/// One decoded instruction with resolved push literal (if any).
#[derive(Debug, Clone)]
pub struct DecodedInst {
    pub pc: u32,
    pub opcode: Opcode,
    pub operands: Vec<i32>,
    pub desc: InstructionDesc,
    /// Resolved push / default literal when applicable.
    pub lit: Option<VmObj>,
}

/// Execution-ready bytecode (raw Break/Continue operands).
#[derive(Clone)]
pub struct LoadedCode {
    pub id: CodeId,
    pub header: Option<Header>,
    pub bytecode: ByteCode,
    pub instructions: Arc<[DecodedInst]>,
    /// Absolute PC (instruction `location`) → index into `instructions`.
    pub pc_to_index: HashMap<u32, usize>,
}

impl std::fmt::Debug for LoadedCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedCode")
            .field("id", &self.id)
            .field("n_inst", &self.instructions.len())
            .field("code_bytes", &self.bytecode.code_bytes.len())
            .finish()
    }
}

static NEXT_CODE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl LoadedCode {
    // Loaded bytecode contains Rc-backed source objects and the VM is
    // intentionally single-threaded. Arc remains the public sharing handle
    // used by frames and ProcBody values; it does not imply cross-thread use.
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn from_bytecode(bc: ByteCode) -> Result<Arc<Self>, VmError> {
        let raw = bc
            .get_instructions_raw()
            .map_err(|e| VmError::Decode(e.to_string()))?;

        let mut instructions = Vec::with_capacity(raw.len());
        let mut pc_to_index = HashMap::with_capacity(raw.len());

        for (idx, ins) in raw.into_iter().enumerate() {
            let pc = ins.location as u32;
            pc_to_index.insert(pc, idx);
            let lit = resolve_push_literal(&bc, ins.opcode, &ins.operands)?;
            instructions.push(DecodedInst {
                pc,
                opcode: ins.opcode,
                operands: ins.operands,
                desc: ins.desc,
                lit,
            });
        }

        let id = CodeId(NEXT_CODE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        Ok(Arc::new(Self {
            id,
            header: bc.header.clone(),
            bytecode: bc,
            instructions: instructions.into(),
            pc_to_index,
        }))
    }

    pub fn from_path(path: &Path) -> Result<Arc<Self>, VmError> {
        let path_str = path.to_string_lossy();
        let loaded = load_bytecode(LoadInput::path(path_str.as_ref())).map_err(|e| {
            if e.to_string().contains("No such file") || e.to_string().contains("os error") {
                VmError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::new(std::io::ErrorKind::NotFound, e.to_string()),
                }
            } else {
                VmError::Decode(e.to_string())
            }
        })?;
        Self::from_bytecode(loaded.bytecode)
    }

    pub fn index_for_pc(&self, pc: u32) -> Result<usize, VmError> {
        self.pc_to_index
            .get(&pc)
            .copied()
            .ok_or_else(|| VmError::Internal(format!("no instruction at pc {pc}")))
    }
}

fn resolve_push_literal(
    bc: &ByteCode,
    opcode: Opcode,
    operands: &[i32],
) -> Result<Option<VmObj>, VmError> {
    match opcode {
        Opcode::Push1 | Opcode::Push4 => {
            let idx = *operands.first().unwrap_or(&0) as usize;
            let lit = bc
                .literals
                .get(idx)
                .ok_or_else(|| VmError::Decode(format!("literal index {idx} out of range")))?;
            Ok(Some(literal_to_vmobj(lit)?))
        }
        _ => Ok(None),
    }
}

/// Convert a core `LiteralObject` into a `VmObj` (including nested ProcBody).
pub fn literal_to_vmobj(lit: &LiteralObject) -> Result<VmObj, VmError> {
    if let Some(pb) = lit.as_proc_body() {
        return proc_body_to_vmobj(pb);
    }
    match lit.kind {
        LiteralObjectType::Int => {
            let s = lit.value.source_repr();
            if let Some(i) = parse_tcl_int(&s) {
                Ok(VmObj::Int(i))
            } else {
                Ok(VmObj::from_str(&s))
            }
        }
        LiteralObjectType::Boolean => match &lit.value {
            SourceRef::Bool(b) => Ok(VmObj::Int(if *b { 1 } else { 0 })),
            _ => {
                let s = lit.value.source_repr();
                Ok(VmObj::from_str(&s))
            }
        },
        LiteralObjectType::Double => Ok(VmObj::from_str(&lit.value.source_repr())),
        LiteralObjectType::String | LiteralObjectType::XString => {
            Ok(VmObj::from_str(&lit.value.source_repr()))
        }
        LiteralObjectType::ProcBody => {
            // Prefer structured field; fall back to error if missing.
            Err(VmError::BcprocBodyNotBytecode)
        }
        LiteralObjectType::Bytecode => {
            // Nested bare bytecode as ProcBody-like load without args meta.
            Err(VmError::Internal(
                "bare Bytecode literal push not supported as value".into(),
            ))
        }
    }
}

pub fn proc_body_to_vmobj(pb: &ProcBody) -> Result<VmObj, VmError> {
    let loaded = LoadedCode::from_bytecode(pb.bytecode.clone())?;
    Ok(VmObj::ProcBody(loaded))
}

/// Compiled-local metadata view used when building frames.
#[derive(Debug, Clone)]
pub struct LocalMeta {
    pub name: ByteString,
    pub flag_mask: i32,
    pub frame_index: i32,
    pub default_value: Option<VmObj>,
}

pub fn locals_meta_from_bc(locals: &[CompiledLocal]) -> Result<Vec<LocalMeta>, VmError> {
    let mut out = Vec::with_capacity(locals.len());
    for loc in locals {
        let default_value = match &loc.default_value {
            Some(lit) => Some(literal_to_vmobj(lit)?),
            None => None,
        };
        out.push(LocalMeta {
            name: ByteString::from(loc.name.as_str()),
            flag_mask: loc.flag_mask,
            frame_index: loc.frame_index,
            default_value,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tcld_core::inst_table::Opcode;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/tcld/tests/fixtures")
            .join(name)
    }

    #[test]
    fn raw_break_operands_empty_on_break_tbc() {
        let path = fixture("break.tbc");
        assert!(
            path.exists(),
            "required fixture missing: {}",
            path.display()
        );
        let loaded = LoadedCode::from_path(&path).expect("load break.tbc");
        let raw = loaded.bytecode.get_instructions_raw().expect("raw");
        let rew = loaded.bytecode.get_instructions().expect("rewritten");
        let mut saw = false;
        for (r, w) in raw.iter().zip(rew.iter()) {
            if r.opcode == Opcode::Break {
                saw = true;
                assert!(
                    r.operands.is_empty(),
                    "raw Break must have empty operands, got {:?}",
                    r.operands
                );
                assert!(
                    !w.operands.is_empty(),
                    "rewritten Break should have synthetic operand"
                );
                assert_ne!(r.operands, w.operands);
            }
        }
        assert!(saw, "expected Break in break.tbc");
    }

    #[test]
    fn proc_body_literals_convert() {
        let path = fixture("proc.tbc");
        assert!(
            path.exists(),
            "required fixture missing: {}",
            path.display()
        );
        let loaded = LoadedCode::from_path(&path).expect("load proc.tbc");
        let mut found = 0;
        for lit in &loaded.bytecode.literals {
            if lit.as_proc_body().is_some() {
                let v = literal_to_vmobj(lit).expect("proc body vmobj");
                assert!(matches!(v, VmObj::ProcBody(_)));
                found += 1;
            }
        }
        assert_eq!(found, 2, "proc.tbc has two ProcBody literals");
    }
}
