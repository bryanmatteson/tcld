#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Full pipeline: parse + decompile. Must not panic.
    if let Ok(bc) = tcld_core::bytecode::ByteCode::from_compiled_payload(data) {
        if let Ok(decompiler) = tcld_decompile::decompiler::Decompiler::new(bc) {
            let _ = decompiler.get_source();
        }
    }
});
