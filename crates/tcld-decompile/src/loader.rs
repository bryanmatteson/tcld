use crate::bytecode::ByteCode;
use crate::error::{Result, TcldError};
use crate::init::ensure_initialized;
use std::path::{Path, PathBuf};

pub fn load_bytecode_from_path(path: &str) -> Result<ByteCode> {
    Ok(load_bytecode(LoadInput::path(path))?.bytecode)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadKind {
    Path,
    RawPayloadText,
    TclSourceText,
    RawBytes,
    DebuggerChunk,
    WasmBytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadProvenance {
    pub kind: LoadKind,
    pub label: String,
    pub normalized_path: Option<PathBuf>,
    pub original_source_available: bool,
    pub debugger_sequence: Option<u64>,
}

#[derive(Clone)]
pub struct LoadedBytecode {
    pub bytecode: ByteCode,
    pub provenance: LoadProvenance,
    pub payload_text: String,
    pub original_source: Option<String>,
    /// Inline-traced command regions when the bytecode came from tcld's own
    /// compiler; empty for Tcl-compiled bytecode.
    pub trace_captures: Vec<crate::decompiler::InlineTraceCapture>,
}

#[derive(Debug, Clone)]
pub enum LoadInput {
    Path(PathBuf),
    RawPayloadText {
        payload: String,
        label: String,
    },
    TclSourceText {
        source: String,
        label: String,
    },
    RawBytes {
        bytes: Vec<u8>,
        label: String,
    },
    DebuggerChunk {
        payload: String,
        label: String,
        sequence: u64,
    },
    WasmBytes {
        bytes: Vec<u8>,
        label: String,
    },
}

impl LoadInput {
    pub fn path(path: impl AsRef<Path>) -> Self {
        Self::Path(path.as_ref().to_path_buf())
    }

    pub fn raw_payload_text(payload: impl Into<String>, label: impl Into<String>) -> Self {
        Self::RawPayloadText {
            payload: payload.into(),
            label: label.into(),
        }
    }

    pub fn tcl_source_text(source: impl Into<String>, label: impl Into<String>) -> Self {
        Self::TclSourceText {
            source: source.into(),
            label: label.into(),
        }
    }

    pub fn raw_bytes(bytes: impl Into<Vec<u8>>, label: impl Into<String>) -> Self {
        Self::RawBytes {
            bytes: bytes.into(),
            label: label.into(),
        }
    }

    pub fn debugger_chunk(
        payload: impl Into<String>,
        label: impl Into<String>,
        sequence: u64,
    ) -> Self {
        Self::DebuggerChunk {
            payload: payload.into(),
            label: label.into(),
            sequence,
        }
    }

    pub fn wasm_bytes(bytes: impl Into<Vec<u8>>, label: impl Into<String>) -> Self {
        Self::WasmBytes {
            bytes: bytes.into(),
            label: label.into(),
        }
    }
}

pub fn load_bytecode(input: LoadInput) -> Result<LoadedBytecode> {
    ensure_initialized();
    match input {
        LoadInput::Path(path) => {
            let normalized_path = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            let source = std::fs::read_to_string(&path).map_err(TcldError::from)?;
            let payload = tcld_syntax::extract_from_contents(&source)?;
            let bytecode = ByteCode::from_compiled_payload(&payload)?;
            Ok(LoadedBytecode {
                bytecode,
                provenance: LoadProvenance {
                    kind: LoadKind::Path,
                    label: normalized_path.display().to_string(),
                    normalized_path: Some(normalized_path),
                    original_source_available: true,
                    debugger_sequence: None,
                },
                payload_text: payload,
                original_source: Some(source),
                trace_captures: Vec::new(),
            })
        }
        LoadInput::RawPayloadText { payload, label } => {
            let normalized = tcld_syntax::extract_from_contents(&payload)
                .unwrap_or_else(|_| payload.trim().to_string());
            let bytecode = ByteCode::from_compiled_payload(&normalized)?;
            Ok(LoadedBytecode {
                bytecode,
                provenance: LoadProvenance {
                    kind: LoadKind::RawPayloadText,
                    label,
                    normalized_path: None,
                    original_source_available: false,
                    debugger_sequence: None,
                },
                payload_text: normalized,
                original_source: None,
                trace_captures: Vec::new(),
            })
        }
        LoadInput::TclSourceText { source, label } => {
            let payload = tcld_syntax::extract_from_contents(&source)?;
            let bytecode = ByteCode::from_compiled_payload(&payload)?;
            Ok(LoadedBytecode {
                bytecode,
                provenance: LoadProvenance {
                    kind: LoadKind::TclSourceText,
                    label,
                    normalized_path: None,
                    original_source_available: true,
                    debugger_sequence: None,
                },
                payload_text: payload,
                original_source: Some(source),
                trace_captures: Vec::new(),
            })
        }
        LoadInput::RawBytes { bytes, label } => {
            let text = String::from_utf8(bytes)
                .map_err(|err| TcldError::Other(format!("raw bytes are not UTF-8: {err}")))?;
            let mut loaded = load_bytecode(LoadInput::raw_payload_text(text, label))?;
            loaded.provenance.kind = LoadKind::RawBytes;
            Ok(loaded)
        }
        LoadInput::DebuggerChunk {
            payload,
            label,
            sequence,
        } => {
            let mut loaded = load_bytecode(LoadInput::raw_payload_text(payload, label))?;
            loaded.provenance.kind = LoadKind::DebuggerChunk;
            loaded.provenance.debugger_sequence = Some(sequence);
            Ok(loaded)
        }
        LoadInput::WasmBytes { bytes, label } => {
            let mut loaded = load_bytecode(LoadInput::raw_bytes(bytes, label))?;
            loaded.provenance.kind = LoadKind::WasmBytes;
            Ok(loaded)
        }
    }
}
