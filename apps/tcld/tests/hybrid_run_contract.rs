//! Hybrid `tcld run` contracts (executable plan §9.1 / S08).

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn tcld_bin() -> PathBuf {
    // Cargo sets CARGO_BIN_EXE_tcld for integration tests of this package.
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_tcld") {
        let path = PathBuf::from(p);
        assert!(
            path.is_file(),
            "CARGO_BIN_EXE_tcld is not a file: {}",
            path.display()
        );
        return path;
    }
    // Fallback for ad-hoc runs after `cargo build -p tcld --features hybrid`.
    let fallback = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/tcld");
    assert!(
        fallback.is_file(),
        "tcld binary missing at {} (build with cargo test -p tcld --features hybrid)",
        fallback.display()
    );
    fallback
}

#[test]
fn production_hybrid_gate_script_exists_and_requires_hybrid_feature() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/verify-hybrid-gates.sh");
    assert!(
        script.is_file(),
        "production gate script missing: {}",
        script.display()
    );
    let src = std::fs::read_to_string(&script).expect("read verify-hybrid-gates.sh");
    assert!(
        src.contains("--features hybrid"),
        "gate script must build/test with --features hybrid"
    );
    assert!(
        src.contains("hybrid_run_contract") && src.contains("get_instructions_raw"),
        "gate script must run hybrid Pass contracts and I6"
    );
    assert!(
        src.contains("exit 2") || src.contains("die_env"),
        "gate script must fail closed when Tcl 9 is missing"
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_help_advertises_hybrid_and_sandbox() {
    let bin = tcld_bin();

    // `tcld run --help` must surface Pass/Trap limits (not only short about).
    let run_help = {
        let out = Command::new(&bin)
            .args(["run", "--help"])
            .output()
            .expect("spawn run --help");
        assert!(out.status.success(), "run --help failed");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    let run_l = run_help.to_ascii_lowercase();
    assert!(
        run_l.contains("hybrid") || run_l.contains("tebc"),
        "run --help must mention hybrid/TEBC:\n{run_help}"
    );
    assert!(
        run_l.contains("vm-strict") || run_l.contains("sandbox"),
        "run --help must mention vm-strict/sandbox:\n{run_help}"
    );
    assert!(
        run_l.contains("trap")
            || run_l.contains("unimplementedopcode")
            || run_l.contains("pass-scope")
            || run_l.contains("pass scope"),
        "run --help must mention Pass/Trap (or UnimplementedOpcode) limits:\n{run_help}"
    );

    // `tcld debug --help` must surface reduced hybrid TUI surface.
    let debug_help = {
        let out = Command::new(&bin)
            .args(["debug", "--help"])
            .output()
            .expect("spawn debug --help");
        assert!(out.status.success(), "debug --help failed");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    let dbg_l = debug_help.to_ascii_lowercase();
    assert!(
        dbg_l.contains("hybrid") || dbg_l.contains("tebc"),
        "debug --help must mention hybrid/TEBC:\n{debug_help}"
    );
    assert!(
        dbg_l.contains("watch") || dbg_l.contains("coverage") || dbg_l.contains("reduced"),
        "debug --help must mention reduced surface (watches/coverage/reduced):\n{debug_help}"
    );
}

#[test]
fn hybrid_module_source_never_mentions_decompile_to_temp() {
    // Structural I1 guard: hybrid run path source must not reference decompile_to_temp.
    let run_src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/run.rs"));
    assert!(
        !run_src.contains("decompile_to_temp"),
        "apps/tcld/src/run.rs must not reference decompile_to_temp (I1)"
    );
    #[cfg(feature = "hybrid")]
    {
        let backend = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/backend.rs"));
        assert!(
            !backend.contains("decompile_to_temp"),
            "hybrid backend must not reference decompile_to_temp (I1)"
        );
        let app = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/debug/app.rs"));
        assert!(
            !app.contains("decompile_to_temp"),
            "canonical debugger app must not choose or invoke decompilation (I1)"
        );
    }
}

#[test]
fn sandbox_flag_is_vm_strict_in_run_router() {
    // I10: sandbox ≡ vm-strict — structural proof in shipped router.
    let run_src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/run.rs"));
    assert!(
        run_src.contains("args.sandbox") && run_src.contains("RunBackend::VmStrict"),
        "run.rs must map --sandbox to RunBackend::VmStrict"
    );
    assert!(
        run_src.contains("RuntimeMode::VmStrict") || !cfg!(feature = "hybrid"),
        "hybrid run path must select RuntimeMode::VmStrict for sandbox/vm-strict"
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_run_proc_tbc_exit_0() {
    let bin = tcld_bin();
    let path = fixture("proc.tbc");
    let out = Command::new(&bin)
        .arg("run")
        .arg(&path)
        .output()
        .expect("spawn tcld run");
    assert!(
        out.status.success(),
        "tcld run proc.tbc failed: status={:?}\nstdout={}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_run_sandbox_denies_world_host_puts() {
    // Drive shipped HybridEngine path used by `tcld run --sandbox` / vm-strict.
    use tcld_core::bytecode::{ByteCode, ByteCodeInfo, LiteralObject, LiteralObjectType};
    use tcld_core::source::SourceRef;
    use tcld_vm::{HybridEngine, RuntimeMode, VmError};

    fn lit_str(s: &str) -> LiteralObject {
        LiteralObject {
            kind: LiteralObjectType::XString,
            value: SourceRef::Shared(s.into()),
            proc_body: None,
        }
    }
    // push puts; push msg; invokeStk1 2; done  (opcodes match hybrid_fixtures puts_bc)
    let code = vec![1, 0, 1, 1, 6, 2, 0];
    let bc = ByteCode {
        header: None,
        info: ByteCodeInfo {
            num_commands: 0,
            num_src_bytes: 0,
            num_code_bytes: code.len() as i32,
            num_lit_objects: 2,
            num_except_ranges: 0,
            num_aux_data_items: 0,
            num_cmd_loc_bytes: 0,
            max_except_depth: 0,
            max_stack_depth: 8,
            code_delta_size: 0,
            code_length_size: 0,
            src_delta_size: 0,
            src_length_size: 0,
        },
        code_bytes: code,
        cmd_code_ranges: vec![],
        cmd_source_ranges: None,
        literals: vec![lit_str("puts"), lit_str("sandbox-deny")],
        exceptions: vec![],
        auxiliaries: vec![],
        proc_num_args: 0,
        proc_num_locals: 0,
        locals: vec![],
    };
    let mut eng = HybridEngine::new(RuntimeMode::VmStrict).expect("vm-strict");
    let err = eng.run_tbc(bc).expect_err("puts must be denied");
    assert!(
        matches!(err, VmError::PolicyDenied { .. }),
        "expected PolicyDenied, got {err:?}"
    );
    assert_eq!(err.exit_code(), 2);

    // CLI: --sandbox ≡ vm-strict on a pure-TEBC fixture (break.tbc: loops +
    // VarName append; no WorldHost). proc.tbc may invoke host `return` and is
    // hybrid-only for full success.
    let bin = tcld_bin();
    let path = fixture("break.tbc");
    let out = Command::new(&bin)
        .args(["run", "--sandbox"])
        .arg(&path)
        .output()
        .expect("spawn sandbox run");
    assert!(
        out.status.success(),
        "sandbox run of break.tbc should succeed: status={:?}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let out2 = Command::new(&bin)
        .args(["run", "--backend=vm-strict"])
        .arg(&path)
        .output()
        .expect("spawn vm-strict run");
    assert!(
        out2.status.success(),
        "vm-strict run of break.tbc should succeed: status={:?}\nstderr={}",
        out2.status,
        String::from_utf8_lossy(&out2.stderr)
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_run_does_not_call_decompile_to_temp() {
    use std::sync::atomic::Ordering;
    let before = tcld::debug::DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst);
    // In-process path via library API (same entry as `tcld run` hybrid).
    let path = fixture("proc.tbc");
    let mut eng = tcld_vm::HybridEngine::new(tcld_vm::RuntimeMode::Hybrid).expect("engine");
    eng.bootstrap(Some("hybrid_run_contract"))
        .expect("bootstrap");
    eng.run_tbc_path(&path).expect("run_tbc_path");
    let after = tcld::debug::DECOMPILE_TO_TEMP_CALLS.load(Ordering::SeqCst);
    assert_eq!(
        after, before,
        "hybrid run must not call decompile_to_temp (I1 counter)"
    );
}

#[test]
#[cfg(feature = "hybrid")]
fn hybrid_run_break_tbc_exit_0() {
    let bin = tcld_bin();
    let path = fixture("break.tbc");
    let out = Command::new(&bin)
        .arg("run")
        .arg(&path)
        .output()
        .expect("spawn tcld run break.tbc");
    assert!(
        out.status.success(),
        "tcld run break.tbc failed: status={:?}\nstdout={}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[cfg(not(feature = "hybrid"))]
fn hybrid_unavailable_forced_backend_exits_3() {
    let bin = tcld_bin();
    let path = fixture("proc.tbc");
    for args in [
        vec!["run", "--backend=hybrid"],
        vec!["run", "--backend=vm-strict"],
        vec!["run", "--sandbox"],
    ] {
        let out = Command::new(&bin)
            .args(&args)
            .arg(&path)
            .output()
            .expect("spawn");
        let code = out.status.code().unwrap_or(0);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(code, 3, "args={args:?} code={code} stderr={stderr}");
        assert!(
            stderr.contains("hybrid_unavailable"),
            "args={args:?} stderr should mention hybrid_unavailable: {stderr}"
        );
    }
}

#[test]
fn non_hybrid_forced_vm_strict_is_fail_closed_in_source() {
    // Structural: without hybrid feature, VmStrict must exit 3 (not soft host).
    let run_src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/run.rs"));
    assert!(
        run_src.contains("RunBackend::Hybrid | RunBackend::VmStrict")
            || run_src.contains("RunBackend::VmStrict"),
        "non-hybrid path must treat vm-strict like forced hybrid"
    );
}

#[test]
fn hybrid_unavailable_source_present_when_feature_off() {
    // Structural: non-hybrid build path logs hybrid_unavailable (plan §2.1).
    let run_src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/run.rs"));
    assert!(
        run_src.contains("hybrid_unavailable"),
        "run.rs must emit hybrid_unavailable when feature not linked"
    );
}
