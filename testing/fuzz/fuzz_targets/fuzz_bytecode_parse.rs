#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Feed arbitrary strings to the bytecode parser.
    // We only care that it does not panic — errors are expected.
    let _ = tcld_core::bytecode::ByteCode::from_compiled_payload(data);
});
