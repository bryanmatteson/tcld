//! First-use opcode capability checks must observe one fully initialized cache.
#![cfg(feature = "tcl9")]

use std::sync::{Arc, Barrier};
use tcld_dbg as _;

unsafe extern "C" {
    fn tcld_opcode_runtime_supported(interp: *mut tcl_sys::Tcl_Interp) -> std::ffi::c_int;
}

#[test]
fn concurrent_first_use_observes_complete_opcode_runtime_symbols() {
    // This integration test has its own process, so no other test initializes
    // the shim before all workers enter its first-use path together.
    unsafe { tcl_sys::Tcl_FindExecutable(std::ptr::null()) };
    let barrier = Arc::new(Barrier::new(32));
    let workers: Vec<_> = (0..32)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || unsafe {
                let interp = tcl_sys::Tcl_CreateInterp();
                assert!(!interp.is_null());
                barrier.wait();
                let supported = tcld_opcode_runtime_supported(interp);
                tcl_sys::Tcl_DeleteInterp(interp);
                supported
            })
        })
        .collect();
    let observed: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().expect("opcode probe worker"))
        .collect();
    let expected = unsafe {
        let interp = tcl_sys::Tcl_CreateInterp();
        assert!(!interp.is_null());
        let supported = tcld_opcode_runtime_supported(interp);
        tcl_sys::Tcl_DeleteInterp(interp);
        supported
    };
    // A stock runtime may lack the optional hooks. Every first-use caller must
    // agree with the stable result on both stock and patched Tcl runtimes.
    assert!(
        observed.iter().all(|value| *value == expected),
        "partially initialized opcode capabilities: {observed:?}; stable={expected}"
    );
}
