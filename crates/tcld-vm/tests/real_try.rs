#![cfg(all(feature = "host", feature = "debug"))]
use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{DebugOutcome, HybridEngine, RuntimeMode};
fn compare(s: &str) {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    let oracle = tcld_interp::Interpreter::new().unwrap();
    let expected = oracle.eval(s).unwrap_or_else(|e| panic!("oracle {s}: {e}"));
    let image = compile_source("/workspace/try.tcl".into(), s.into(), CompileMode::Hybrid)
        .unwrap_or_else(|e| panic!("{s}: {e}"));
    let mut engine = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    engine.bootstrap(Some("tcld")).unwrap();
    let result = engine
        .debug_start_image(&image, false)
        .unwrap_or_else(|e| panic!("{s}: {e}"));
    match result.outcome {
        DebugOutcome::Finished { result } => {
            assert_eq!(result.as_string().unwrap(), expected, "{s}")
        }
        other => panic!("{s}: {other:?}"),
    }
}
#[test]
fn native_try_handlers_and_cleanup() {
    for s in [
        "try {expr {40+2}} finally {set done 1}",
        "set out {}; try {lappend out body; error failure} on error {message opts} {lappend out $message [dict get $opts -code]} finally {lappend out finally}; join $out ,",
        "proc f {} {set x 10; try {error failure} on error {msg opts} {incr x; list $x $msg [dict get $opts -code]} finally {incr x}}; f",
        "try {return -level 0 -code error -errorcode {POSIX ENOENT detail} missing} trap {POSIX ENOENT} {m o} {list $m [dict get $o -errorcode]}",
        "try {error bad {} {A B C}} trap {A X} {} {error wrong} trap A {m o} {list $m [dict get $o -errorcode]}",
        "try {error bad} on error {} - on ok {m o} {list shared $m [dict get $o -code]}",
        "proc f {} {try {return body} on return {m o} {list handled $m [dict get $o -level]} finally {set x 1}}; f",
        "set out {}; foreach x {1 2 3} {try {if {$x == 2} {continue}; if {$x == 3} {break}; lappend out $x} finally {lappend out cleanup}}; join $out ,",
        "set code [catch {try {error first} finally {error second}} m o]; list $code $m [dict get $o -during -errorcode]",
        "set code [catch {try {error first} on error {} {error second} finally {error third}} m o]; list $code $m [dict get $o -during -during -errorcode]",
        "proc f {} {try {return body} finally {return cleanup}}; f",
        "proc target {} {return target}; proc start {} {try {tailcall target} finally {set ::clean yes}}; list [start] $clean",
        "proc target {} {return target}; proc start {} {try {tailcall target} finally {error cleanup}}; list [catch {start} m] $m",
        "proc target {} {return target}; proc start {} {try {tailcall target} finally {return cleanup}}; start",
        "set out {}; proc work {} {try {lappend ::out [yield body]; error failure} on error {m o} {lappend ::out [yield handler]} finally {lappend ::out [yield cleanup]}; return done}; list [coroutine co work] [co B] [co H] [co F] $out",
        "set out {}; proc target {} {lappend ::out target; return done}; proc work {} {try {tailcall target} finally {lappend ::out [yield cleanup]}}; list [coroutine co work] [co resumed] $out",
        "set out {}; proc work {} {try {yield paused} finally {lappend ::out cleanup}}; coroutine co work; rename co {}; set out",
        "proc f {} {try {error bad} on error {a(k) opts ignored} {list $a(k) [dict get $opts -code]}}; f",
        "namespace eval n {variable x 0; try {incr x} on ok {m o} {list $m [dict get $o -code]} finally {incr x}}",
        "set script {expr {21*2}}; try $script on ok {m} {set m}",
        "try {expr 42} f {set cleanup yes}",
    ] {
        compare(s)
    }
}
#[test]
fn nested_completions_bindings_and_invalid_forms_match_tcl() {
    for s in [
        "proc f {} {try {return -level 2 outside} finally {set ::clean yes}}; proc g {} {f; error unreachable}; list [g] $clean",
        "try {return -level 0 -code 200 custom} on 200 {m o} {list $m [dict get $o -code]}",
        "try {return -level 0 -code 201 custom} on 201 {m o} {list $m [dict get $o -code]}",
        "set out {}; try {try {error one} finally {lappend out inner}} on error {m o} {lappend out $m} finally {lappend out outer}; join $out ,",
        "proc target {} {return target}; proc f {} {try {tailcall target} on return {m o} {return handled} finally {set ::clean yes}}; list [f] $clean",
        "set out {}; proc target {} {lappend ::out target; return yes}; proc f {} {try {tailcall target} finally {try {yield cleanup} finally {lappend ::out inner}; lappend ::out outer}}; list [coroutine co f] [co] [join $out ,]",
        "proc target {} {yield target; return done}; proc f {} {try {tailcall target} finally {yield cleanup}}; list [coroutine co f] [co] [co]",
        "set out {}; proc traced {n e op} {lappend ::out $n}; proc f {} {set m old; trace add variable m write traced; try {error bad} on error {m o} {set m}}; list [f] $out",
        "set out {}; proc traced {n e op} {lappend ::out $n; upvar 1 $n alias; append alias !}; proc f {} {set m old; trace add variable m write traced; try {yield body; error bad} on error {m o} {set m}}; list [coroutine co f] [co] $out",
        "proc traced {n e op} {error denied}; proc f {} {set m old; trace add variable m write traced; try {error bad} on error {m o} {error unreachable} finally {set ::clean yes}}; list [catch {f} m] $m $clean",
        "proc f {} {set m old; proc child {} {upvar 1 m alias; try {error bad} on error {alias o} {set alias}}; list [child] $m}; f",
        "set out {}; proc cb {cmd args} {lappend ::out [join $args |]}; proc f {} {try {set x 1} finally {incr x}}; trace add execution f {enterstep leavestep} cb; set r [f]; list $r [join $out \n]",
        "proc f {n} {if {$n == 0} {return done}; try {tailcall f [expr {$n-1}]} finally {incr ::count}}; set count 0; list [f 1000] $count",
        "set body {error bad}; set handler {list $m [dict get $o -code]}; try $body on error {m o} $handler",
    ] {
        compare(s);
    }
    compare(
        "set bad [format %c 123]; set c [catch {try {} trap $bad {} {}} m o]; list $c $m [dict get $o -errorcode]",
    );
    compare(
        "set bad [format %c 123]; set c [catch {try {} on error $bad {}} m o]; list $c $m [dict get $o -errorcode]",
    );
    for form in [
        "try",
        "try {} finally",
        "try {} finally {} on error {} {}",
        "try {} on error {} -",
        "try {} on nonsense {} {}",
        "try {} bogus {}",
    ] {
        compare(&format!(
            "set c [catch {{{form}}} m o]; list $c $m [dict get $o -errorcode]"
        ));
    }
}
#[test]
fn mapped_try_bodies_pause_edit_and_resume() {
    for (clause, body, suffix) in [
        ("", "set marker 40\nincr marker\nset ::result $marker", ""),
        (
            "on error {message options}",
            "set marker 40\nincr marker\nset ::result $marker",
            "",
        ),
        (
            "trap {APP FAIL} {message options}",
            "set marker 40\nincr marker\nset ::result $marker",
            "",
        ),
        (
            "finally",
            "yield cleanup\nset marker 40\nincr marker\nset ::result $marker",
            "coroutine co work; co",
        ),
    ] {
        let start = if clause.is_empty() {
            format!("try {{{body}}} finally {{set cleaned yes}}")
        } else if clause == "finally" {
            format!("try {{tailcall list done}} finally {{{body}}}")
        } else {
            format!(
                "try {{error failure {{}} {{APP FAIL}}}} {clause} {{{body}}} finally {{set cleaned yes}}"
            )
        };
        let source = format!(
            "proc work {{}} {{{start}}}\n{}\nset result",
            if suffix.is_empty() { "work" } else { suffix }
        );
        let image = compile_source(
            "/workspace/inspect-try.tcl".into(),
            source,
            CompileMode::Hybrid,
        )
        .unwrap();
        let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
        e.bootstrap(Some("tcld")).unwrap();
        let mut state = e.debug_start_image(&image, true).unwrap().outcome;
        let mut address = None;
        for _ in 0..400 {
            if let DebugOutcome::Paused(p) = &state
                && let Some(a) = p.address
                && e.debug_sources().iter().any(|s| {
                    s.code_id == a.code_id
                        && s.origin.is_some()
                        && s.image.source.content.as_ref() == body
                })
                && e.ui_frame_locals(0)
                    .iter()
                    .any(|(n, v)| n == "marker" && v == "40")
            {
                address = Some(a);
                break;
            }
            assert!(
                !matches!(state, DebugOutcome::Finished { .. }),
                "body never paused: {clause}"
            );
            state = e.step_instruction().unwrap();
        }
        let a = address.expect("mapped try body with caller locals");
        let unit = e
            .debug_sources()
            .into_iter()
            .find(|s| s.code_id == a.code_id)
            .unwrap();
        let pc = unit.image.units[unit.unit_index]
            .commands
            .iter()
            .find(|m| unit.image.source.content[m.source_span.clone()].starts_with("set ::result"))
            .unwrap()
            .anchor_pc;
        let bp = tcld_vm::CodeAddress {
            code_id: a.code_id,
            pc,
        };
        e.add_breakpoint(bp);
        e.eval_in_frame(0, tcld_vm::VmObj::from_str("set marker 80"))
            .unwrap();
        assert!(matches!(e.continue_run().unwrap(), DebugOutcome::Paused(_)));
        assert!(
            e.ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "marker" && v == "81")
        );
        e.remove_breakpoint(bp);
        match e.continue_run().unwrap() {
            DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "81"),
            o => panic!("{o:?}"),
        }
    }
}

#[test]
fn internal_try_entry_cannot_execute_unmapped_bodies() {
    let source =
        "list [catch {::tcld::control 0 :: {} try {set ::escaped yes}} m] [info exists ::escaped]";
    let image = compile_source(
        "/workspace/private-try.tcl".into(),
        source.into(),
        CompileMode::Hybrid,
    )
    .unwrap();
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    match e.debug_start_image(&image, false).unwrap().outcome {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "1 0"),
        other => panic!("{other:?}"),
    }
}
