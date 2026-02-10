#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Feed arbitrary strings to the syntax extraction pipeline.
    let _ = tcld_syntax::extract_from_contents(data);
});
