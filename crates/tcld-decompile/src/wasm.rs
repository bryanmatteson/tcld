//! WASM-compatible API for running the decompiler in the browser.
//!
//! When compiled with `--target wasm32-unknown-unknown`, this module
//! provides JavaScript-friendly entry points via wasm-bindgen.
//!
//! # Usage (from JavaScript)
//!
//! ```js
//! import init, { decompile, disassemble } from './tcld_wasm.js';
//! await init();
//! const source = decompile(bytecodeUint8Array);
//! ```

/// Decompile a bytecode payload (raw bytes) to Tcl source.
///
/// This is the primary entry point for WASM consumers. Pass the raw
/// compiled Tcl bytecode (the payload portion, not the full .tbc file)
/// and receive the decompiled source as a string.
///
/// # Errors
///
/// Returns an error string if parsing or decompilation fails.
#[cfg(target_arch = "wasm32")]
pub fn decompile_bytes(payload: &[u8]) -> Result<String, String> {
    let loaded = crate::load_bytecode(crate::LoadInput::wasm_bytes(payload.to_vec(), "wasm"))
        .map_err(|e| e.to_string())?;
    crate::decompile_loaded(&loaded).map_err(|e| e.to_string())
}

/// Disassemble a bytecode payload to human-readable instruction listing.
#[cfg(target_arch = "wasm32")]
pub fn disassemble_bytes(payload: &[u8]) -> Result<String, String> {
    let loaded = crate::load_bytecode(crate::LoadInput::wasm_bytes(payload.to_vec(), "wasm"))
        .map_err(|e| e.to_string())?;
    crate::disassemble_loaded(&loaded).map_err(|e| e.to_string())
}

// Non-WASM stub so the module compiles on all targets
#[cfg(not(target_arch = "wasm32"))]
pub fn decompile_bytes(payload: &[u8]) -> Result<String, String> {
    let loaded = crate::load_bytecode(crate::LoadInput::wasm_bytes(payload.to_vec(), "wasm"))
        .map_err(|e| e.to_string())?;
    crate::decompile_loaded(&loaded).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn disassemble_bytes(payload: &[u8]) -> Result<String, String> {
    let loaded = crate::load_bytecode(crate::LoadInput::wasm_bytes(payload.to_vec(), "wasm"))
        .map_err(|e| e.to_string())?;
    crate::disassemble_loaded(&loaded).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompile_empty_payload_returns_error() {
        let result = decompile_bytes(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn decompile_invalid_payload_returns_error() {
        let result = decompile_bytes(&[0xFF, 0xFE, 0xFD]);
        assert!(result.is_err());
    }

    #[test]
    fn native_wasm_facade_decompiles_and_disassembles_valid_bytes() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/tcld/tests/fixtures/expr.tbc");
        let bytes = std::fs::read(fixture).expect("fixture bytes");

        let source = decompile_bytes(&bytes).expect("WASM decompile facade");
        assert!(!source.is_empty());
        let instructions = disassemble_bytes(&bytes).expect("WASM disassemble facade");
        assert!(instructions.contains("push"));
        assert!(disassemble_bytes(b"invalid").is_err());
    }
}
