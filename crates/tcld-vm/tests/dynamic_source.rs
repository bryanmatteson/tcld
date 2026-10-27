#![cfg(all(feature = "host", feature = "debug"))]
use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{CodeAddress, DebugOutcome, HybridEngine, RuntimeMode, VmObj};
fn image(source: &str) -> tcld_compile::DebugImage {
    compile_source(
        "/workspace/dynamic.tcl".into(),
        source.into(),
        CompileMode::Hybrid,
    )
    .unwrap_or_else(|e| panic!("{e}\n{source}"))
}
fn engine() -> HybridEngine {
    let mut engine = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    engine.bootstrap(Some("tcld")).unwrap();
    engine
}
fn oracle(source: &str) -> String {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    tcld_interp::Interpreter::new()
        .unwrap()
        .eval(source)
        .unwrap()
}
#[test]
fn generated_scripts_match_real_tcl_completion_and_scope() {
    for source in [
        "set x 1; set body {incr x 2; set x}; eval $body",
        "eval {set x} {hello}; set x",
        "set command eval; $command {expr {6*7}}",
        "proc f {x} {set body {incr x; set y 8; list $x $y}; eval $body}; f 3",
        "proc f {x} {set body {unset x; set y 8}; eval $body; set y}; f 3",
        "proc f {x} {set body {array set a {k 7}; incr x $a(k); set x}; eval $body}; f 3",
        "proc f {x} {set body {string toupper $x}; eval $body}; f hello",
        "set x 0; set body {incr x}; if {1} $body; set x",
        "set x 0; set test {$x<3}; set body {incr x}; while $test $body; set x",
        "set x 0; set body {incr x; set body {error must_not_run}}; while {$x<3} $body; set x",
        "set body {incr n $i}; set n 0; for {set i 0} {$i<4} {incr i} $body; set n",
        "set body {lappend out $x}; set out {}; foreach x {a b c} $body; set out",
        "set names {a b}; set out {}; foreach $names {1 2 3 4} {lappend out $a $b}; set out",
        "set body {error generated}; catch $body result options; list $result [dict get $options -code]",
        "set x {3+2}; expr 2 * $x",
        "set x {3+2}; expr {2 * [expr $x]}",
        "set body {return done}; proc f {body} {eval $body; error unreachable}; f $body",
        "set body {return -level 2 done}; proc inner {body} {eval $body; error bad}; proc outer {body} {inner $body; error bad}; outer $body",
        "set body {break}; set x 0; while {1} {incr x; eval $body}; set x",
        "set body {continue}; set n {}; foreach i {1 2 3} {if {$i==2} {eval $body}; lappend n $i}; set n",
        "set body {return done}; catch {eval $body} result options; list $result [dict get $options -code] [dict get $options -level]",
        "set name greet; set args {x}; set body {return [string toupper $x]}; proc $name $args $body; greet hi",
        "proc factory {name} {proc $name {n} {expr {$n+1}}}; factory next; next 4",
        "set x first; proc f {} {return ok}; f",
        "set body {eval {incr x}}; set x 0; eval $body; set x",
        "set body {set x \"héllo\\nworld\"}; eval $body; set x",
        "set body {error bad}; catch {eval $body} r; set r",
        "set c :::eval; $c {expr {6*7}}",
        "proc f {x} {set b {set a(k) 7; set i k; incr x $a($i); set x}; eval $b}; f 3",
        "set b {set x [list a\\b c]}; eval $b; set x",
        "set b {expr {6*7}}; eval $b",
    ] {
        let mut e = engine();
        let outcome = e
            .debug_start_image(&image(source), false)
            .unwrap_or_else(|e| panic!("{source}\n{e}"));
        match outcome.outcome {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), oracle(source), "{source}")
            }
            other => panic!("{source}: {other:?}"),
        }
    }
}
#[test]
fn procedure_defaults_match_real_tcl() {
    for source in [
        "proc greet {{name world}} {return \"hi $name\"}; list [greet] [greet you]",
        "proc f {a {b 2} {c {x y}}} {list $a $b $c}; list [f 1] [f 1 3] [f 1 3 4]",
        "proc g {a {b 2} args} {list $a $b $args}; list [g 1] [g 1 5] [g 1 5 6 7]",
        "proc h {{v {}}} {string length $v}; h",
        "proc k {{v $x}} {set v}; k",
        "proc m {{a 1} b} {list $a $b}; m x y",
        // Generated procedures compile through the same declaration path.
        "set name gen; proc $name {{n 5}} {expr {$n*2}}; list [gen] [gen 3]",
        "eval {proc e2 {{a 1} {b 2}} {expr {$a+$b}}}; list [e2] [e2 5] [e2 5 5]",
        // Calls the compiler cannot check get Tcl's usage message at runtime.
        "proc greet {{name world}} {}; catch {eval greet a b} r; set r",
        "proc tail {a args} {}; catch {eval tail} r; set r",
        "proc none {} {}; catch {eval none extra} r; set r",
    ] {
        let mut e = engine();
        let outcome = e
            .debug_start_image(&image(source), false)
            .unwrap_or_else(|e| panic!("{source}\n{e}"));
        match outcome.outcome {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), oracle(source), "{source}")
            }
            other => panic!("{source}: {other:?}"),
        }
    }
}

#[test]
fn generated_images_have_mapped_steps_qualified_breakpoints_scope_edits_and_stable_cache() {
    let source = "proc f {x} {set body {incr x\nset x}; eval $body; eval $body}\nf 1";
    let mut e = engine();
    let mut outcome = e.debug_start_image(&image(source), true).unwrap().outcome;
    let mut generated = None;
    for _ in 0..200 {
        if let DebugOutcome::Paused(p) = &outcome {
            let a = p.address.unwrap();
            if e.debug_sources()
                .iter()
                .any(|s| s.code_id == a.code_id && s.origin.is_some())
            {
                generated = Some(a);
                break;
            }
        }
        outcome = e.step_instruction().unwrap();
    }
    let a = generated.expect("step into generated body");
    let sources = e.debug_sources();
    let s = sources.iter().find(|s| s.code_id == a.code_id).unwrap();
    assert_eq!(&*s.image.source.content, "incr x\nset x");
    assert!(s.origin.is_some());
    s.image.verify().unwrap();
    assert_eq!(
        e.eval_in_frame(0, VmObj::from_str("set x"))
            .unwrap()
            .result
            .as_string()
            .unwrap(),
        "1"
    );
    e.eval_in_frame(0, VmObj::from_str("set x 40")).unwrap();
    let pc = s.image.units[s.unit_index]
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
    match e.continue_run().unwrap() {
        DebugOutcome::Paused(p) => assert_eq!(p.address, Some(bp)),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        e.eval_in_frame(0, VmObj::from_str("set x"))
            .unwrap()
            .result
            .as_string()
            .unwrap(),
        "41"
    );
    e.remove_breakpoint(bp);
    match e.continue_run().unwrap() {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "42"),
        other => panic!("{other:?}"),
    }
    // The same text at two different call sites has distinct provenance.
    assert_eq!(
        e.debug_sources()
            .iter()
            .filter(|s| s.origin.is_some())
            .count(),
        2
    );
    e.debug_start_image(&image("set x 0"), true).unwrap();
    assert!(e.debug_sources().iter().all(|s| s.origin.is_none()));
}
#[test]
fn same_callsite_reuses_identity_but_changed_text_gets_a_new_image() {
    for (source, count) in [
        (
            "set body {incr x}; set x 0; foreach i {1 2 3} {eval $body}",
            1,
        ),
        (
            "set x 0; foreach body {{incr x} {incr x 2} {incr x}} {eval $body}",
            2,
        ),
    ] {
        let mut e = engine();
        e.debug_start_image(&image(source), false).unwrap();
        assert_eq!(
            e.debug_sources()
                .iter()
                .filter(|s| s.origin.is_some())
                .count(),
            count
        );
    }
}
#[test]
fn malformed_and_unsupported_generated_scripts_are_catchable() {
    for source in [
        "set b {interp eval {} {set x 1}}; catch {eval $b} result; string match *outside* $result",
        "set command interp; set sub eval; catch {$command $sub {} {set x 1}} result; string match *outside* $result",
        "set b \"set x \\\"\"; catch {eval $b} result; string match *malformed* $result",
    ] {
        let mut e = engine();
        match e.debug_start_image(&image(source), false).unwrap().outcome {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), "1", "{source}")
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn captured_words_round_trip_through_real_tcl() {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    let interp = tcld_interp::Interpreter::new().unwrap();
    for value in [
        "",
        "a b",
        "héllo",
        "$x [error no]",
        "a}b",
        "{a",
        "\\",
        "a\\\n b",
        "set x \"value\"",
        "a\n b",
        "a\\{b",
    ] {
        assert_eq!(
            interp
                .eval(&format!("set value {}", tcld_compile::quote_word(value)))
                .unwrap(),
            value
        );
    }
}

#[test]
fn bounded_browser_telemetry_preserves_execution_and_exact_coverage_counts() {
    let mut e = engine();
    e.set_observation_limit(Some(256));
    let outcome = e
        .debug_start_image(
            &image(
                "set n 0; set body {incr n}; for {set i 0} {$i<10000} {incr i} {eval $body}; set n",
            ),
            false,
        )
        .unwrap()
        .outcome;
    match outcome {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "10000"),
        other => panic!("{other:?}"),
    }
    assert!(e.observations().len() <= 256);
    assert!(e.coverage_snapshot().executed_instruction_total > 10000);
    assert_eq!(
        e.debug_sources()
            .iter()
            .filter(|s| s.origin.is_some())
            .count(),
        1
    );
}
