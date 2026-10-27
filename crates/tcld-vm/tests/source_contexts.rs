#![cfg(all(feature = "host", feature = "debug"))]
use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{CodeAddress, DebugOutcome, HybridEngine, RuntimeMode, VmObj};
fn engine() -> HybridEngine {
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    e
}
fn image(s: &str) -> tcld_compile::DebugImage {
    compile_source(
        "/workspace/contexts.tcl".into(),
        s.into(),
        CompileMode::Hybrid,
    )
    .unwrap_or_else(|e| panic!("{s}\n{e}"))
}
fn oracle(s: &str) -> String {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    tcld_interp::Interpreter::new().unwrap().eval(s).unwrap()
}
fn compare(s: &str) {
    let mut e = engine();
    let out = e
        .debug_start_image(&image(s), false)
        .unwrap_or_else(|e| panic!("{s}\n{e}"));
    match out.outcome {
        DebugOutcome::Finished { result } => {
            assert_eq!(result.as_string().unwrap(), oracle(s), "{s}")
        }
        other => panic!("{s}\n{other:?}"),
    }
}
#[test]
fn contexts_callbacks_and_comparators_match_real_tcl() {
    for s in [
        "namespace eval n {set x 40; incr x}; namespace eval n {incr x}; set ::n::x",
        "namespace eval n {set x 7; proc f {} {variable x; incr x}}; n::f; set ::n::x",
        "namespace eval n {namespace eval child {set x 7}}; set ::n::child::x",
        "namespace eval n {namespace current}",
        "set x 1; namespace eval :: {incr x}; set x",
        "namespace eval n {set a(k) 7; incr a(k)}; set ::n::a(k)",
        "proc f {} {namespace eval n {return good}; error bad}; f",
        "proc outer {x} {inner; set x}; proc inner {} {uplevel 1 {incr x}}; outer 40",
        "proc f {} {set x 1; uplevel 0 {incr x}; set x}; f",
        "set x 1; proc f {} {uplevel #0 {incr x}}; f; set x",
        "proc f {} {uplevel 0 {return good}; error bad}; f",
        "proc f {} {uplevel 0 {uplevel 0 {set x 7}}; set x}; f",
        "proc f {x} {eval {uplevel 0 {incr x}}; set x}; f 40",
        "proc f {x} {uplevel 0 {eval {incr x}}; set x}; f 40",
        "proc outer {} {set x 7; inner; set x}; proc inner {} {uplevel 1 {uplevel 0 {incr x}}}; outer",
        "namespace eval n {set x 7; proc f {} {uplevel 1 {incr x}}; f; set x}",
        "apply {{a b} {expr {$a+$b}}} 20 22",
        "namespace eval n {set x 40}; apply {{} {variable x; incr x} ::n}",
        "set out {}; after 0 {lappend ::out timer}; after idle {lappend ::out idle}; update; set out",
        "set out {}; after idle {lappend ::out idle}; after 0 {lappend ::out timer}; update idletasks; set out",
        "set out {}; set id [after 0 {lappend ::out bad}]; after cancel $id; update; set out",
        "set out {}; after 0 {lappend ::out bad}; after cancel {lappend ::out bad}; update; set out",
        "set x 0; after 1 {set ::x 42}; vwait x; set x",
        "set x 0; after 0 {set ::x 0}; vwait x; set x",
        "set out {}; after 0 {lappend ::out first; after 0 {lappend ::out second}}; update; set out",
        "namespace eval n {set x 40; set callback [namespace code {incr x}]}; after 0 $n::callback; update; set n::x",
        "namespace eval n {proc f {x} {expr {$x+1}}; set callback [namespace code f]}; eval $n::callback 41",
        "namespace eval n {namespace inscope ::n {namespace current}}",
        "proc cb {a b} {expr {$a-$b}}; lsort -command cb {3 1 2}",
        "proc cb {a b} {expr {$a-$b}}; lsort -decreasing -command cb {3 1 2}",
        "proc cb {a b} {expr {$a-$b}}; lsort -unique -command cb -index 0 {{2 a} {1 b} {2 c}}",
        "proc cb {a b} {expr {$a-$b}}; lsort -indices -command cb {3 1 2}",
        "proc cb {a b} {expr {$a-$b}}; lsort -stride 2 -command cb {3 a 1 b 2 c}",
        "proc cb {a b} {expr {$a-$b}}; lsort -stride 2 -index 1 -command cb {a 3 b 1 c 2}",
        "namespace eval n {proc cb {a b} {expr {$b-$a}}; lsort -command cb {3 1 2}}",
        "set callback [list apply {{a b} {expr {$a-$b}}}]; lsort -command $callback {3 1 2}",
        "proc cb {a b} {error comparator}; catch {lsort -command cb {3 1 2}} r; set r",
        "set x 40; namespace eval n {global x; incr x}; set x",
        "namespace eval n {array set a {k 7}}; namespace eval n {incr a(k)}; set n::a(k)",
        "namespace eval n {info level}",
        "proc f {} {namespace eval n {info level}}; f",
        "proc cb {} {set ::done 42}; set done 0; after 0 cb; vwait done; set done",
        "set seen {}; proc bgerror {message} {lappend ::seen $message}; after 0 {error callback_error}; update; set seen",
        "set done {a}; after 0 {lset ::done 0 b}; vwait done; set done",
        "set done {a}; after 0 {unset ::done}; vwait done; info exists done",
        "namespace eval n {set x 1}; after 0 {set ::n::x 2}; vwait n::x; set n::x",
        "proc compare {a b} {uplevel 1 {incr calls}; expr {$a-$b}}; proc f {} {set calls 0; set result [lsort -command compare {3 1 2}]; list $result $calls}; f",
        "proc compare {a b} {uplevel 1 {set marker caller}; expr {$a-$b}}; set marker root; lsort -command compare {3 1 2}; set marker",
        "proc cb {} {uplevel 1 {set done 42}}; set done 0; after 0 cb; vwait done; set done",
        "set id [after idle {set x 1}]; lindex [after info $id] 1",
    ] {
        compare(s);
    }
}
#[test]
fn sourced_files_capture_revisions_encoding_scope_and_completion() {
    let path = std::env::temp_dir().join(format!("tcld-source-{}.tcl", std::process::id()));
    let path = path
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(path.file_name().unwrap());
    let p = tcld_compile::quote_word(&path.display().to_string());
    std::fs::write(&path, "incr x\nset x").unwrap();
    compare(&format!("proc f {{x}} {{source {p}}}; f 40"));
    compare(&format!("set x 40; source -encoding utf-8 {p}; set x"));
    compare(&format!(
        "namespace eval n {{set x 40; source {p}}}; set n::x"
    ));
    std::fs::write(&path, "return from_source\nerror unreachable").unwrap();
    compare(&format!(
        "proc f {{}} {{set result [source {p}]; list $result kept_going}}; f"
    ));
    std::fs::write(&path, "info script").unwrap();
    compare(&format!("source {p}"));
    std::fs::write(&path, "incr x\nset x").unwrap();
    let mut e = engine();
    let mut out = e
        .debug_start_image(&image(&format!("set x 40; source {p}; source {p}")), true)
        .unwrap()
        .outcome;
    let mut found = None;
    for _ in 0..150 {
        if let DebugOutcome::Paused(pause) = &out {
            let a = pause.address.unwrap();
            if e.debug_sources()
                .iter()
                .any(|s| s.code_id == a.code_id && s.image.source.path == path)
            {
                found = Some(a);
                break;
            }
        }
        out = e.step_instruction().unwrap();
    }
    let a = found.expect("entered source file");
    let source = e
        .debug_sources()
        .into_iter()
        .find(|s| s.code_id == a.code_id)
        .unwrap();
    assert_eq!(&*source.image.source.content, "incr x\nset x");
    let pc = source
        .image
        .top_level()
        .commands
        .iter()
        .find(|m| m.start_line == 2)
        .unwrap()
        .anchor_pc;
    e.add_breakpoint(CodeAddress {
        code_id: a.code_id,
        pc,
    });
    e.eval_in_frame(0, VmObj::from_str("set x 80")).unwrap();
    assert!(matches!(e.continue_run().unwrap(), DebugOutcome::Paused(_)));
    e.remove_breakpoint(CodeAddress {
        code_id: a.code_id,
        pc,
    });
    std::fs::write(&path, "incr x 10\nset x").unwrap();
    match e.continue_run().unwrap() {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "91"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        e.debug_sources()
            .iter()
            .filter(|s| s.image.source.path == path)
            .count(),
        2
    );
    std::fs::remove_file(path).unwrap();
}
#[test]
fn each_context_can_pause_edit_and_resume_inside_real_source() {
    for (source, body, expected) in [
        (
            "namespace eval n {set x 40; eval {incr x\nset x}}",
            "incr x\nset x",
            "81",
        ),
        (
            "proc f {x} {uplevel 0 {incr x\nset x}}; f 40",
            "incr x\nset x",
            "81",
        ),
        (
            "set x 40; after 0 {incr x\nset x}; update; set x",
            "incr x\nset x",
            "81",
        ),
        (
            "apply {{x} {eval {incr x\nset x}}} 40",
            "incr x\nset x",
            "81",
        ),
    ] {
        let mut e = engine();
        let mut out = e.debug_start_image(&image(source), true).unwrap().outcome;
        let mut found = None;
        for _ in 0..500 {
            if let DebugOutcome::Paused(p) = &out {
                let a = p.address.unwrap();
                if e.debug_sources()
                    .iter()
                    .any(|s| s.code_id == a.code_id && s.image.source.content.as_ref() == body)
                {
                    found = Some(a);
                    break;
                }
            }
            out = e
                .step_instruction()
                .unwrap_or_else(|e| panic!("{source}: {e}"));
        }
        let a = found.unwrap_or_else(|| panic!("did not pause inside {source}"));
        let s = e
            .debug_sources()
            .into_iter()
            .find(|s| s.code_id == a.code_id)
            .unwrap();
        let pc = s
            .image
            .top_level()
            .commands
            .iter()
            .find(|m| m.start_line == 2)
            .unwrap()
            .anchor_pc;
        let bp = CodeAddress {
            code_id: a.code_id,
            pc,
        };
        e.add_breakpoint(bp);
        let edited = e.eval_in_frame(0, VmObj::from_str("set x 80")).unwrap();
        assert_eq!(edited.code, 0);
        assert!(
            e.ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "x" && v == "80")
        );
        match e.continue_run().unwrap() {
            DebugOutcome::Paused(p) => assert_eq!(p.address, Some(bp)),
            o => panic!("{o:?}"),
        }
        e.remove_breakpoint(bp);
        match e.continue_run().unwrap() {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), expected, "{source}")
            }
            o => panic!("{o:?}"),
        }
    }
}
