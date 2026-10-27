#![cfg(all(feature = "host", feature = "debug"))]
use std::path::PathBuf;
use tcld_compile::{CompileMode, DebugImage, compile_source};
use tcld_vm::{CodeAddress, DebugOutcome, HybridEngine, RuntimeMode, VmObj};

fn image(source: &str) -> DebugImage {
    compile_source(
        PathBuf::from("/workspace/edited.tcl"),
        source.into(),
        CompileMode::Hybrid,
    )
    .unwrap_or_else(|error| panic!("{error}\nsource: {source}"))
}
fn engine() -> HybridEngine {
    let mut engine = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    engine.bootstrap(Some("tcld")).unwrap();
    engine
}
fn finish(source: &str) -> String {
    let mut engine = engine();
    match engine
        .debug_start_image(&image(source), false)
        .unwrap()
        .outcome
    {
        DebugOutcome::Finished { result } => result.as_string().unwrap(),
        other => panic!("{other:?}"),
    }
}
fn oracle(source: &str) -> String {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    tcld_interp::Interpreter::new()
        .unwrap()
        .eval(source)
        .unwrap()
}

#[test]
fn edited_versions_of_all_six_examples_compile_and_step_real_commands() {
    let fixtures = [
        (
            include_str!("../../../apps/tcld/tests/fixtures/hello.tcl")
                .replace("hello, world", "edited greeting"),
            "edited greeting",
        ),
        (
            include_str!("../../../apps/tcld/tests/fixtures/proc.tcl").replace("TEST", "CHANGED"),
            "{CHANGED : CHANGED} {CHANGED > CHANGED}",
        ),
        (
            include_str!("../../../apps/tcld/tests/fixtures/break.tcl")
                .replace("set top 8", "set top 3")
                .replace("DONE", "EDITED"),
            "1.2..3...EDITED",
        ),
        (
            include_str!("../../../apps/tcld/tests/fixtures/expr.tcl")
                .replace("10 *", "12 *")
                .replace("sqrt(16)", "sqrt(25)"),
            "60",
        ),
        (
            include_str!("../../../apps/tcld/tests/fixtures/foreach.tcl")
                .replace("set top 8", "set top 3")
                .replace("DONE", "EDITED"),
            "1.2..3...EDITED",
        ),
        (
            include_str!("../../../apps/tcld/tests/fixtures/catch.tcl")
                .replace("error PASS", "error EDITED"),
            "EDITED",
        ),
    ];
    for (source, expected) in fixtures {
        let image = image(&source);
        image.verify().unwrap();
        assert_eq!(image.source.content.as_ref(), source);
        assert_eq!(oracle(&source), expected);
        assert_eq!(finish(&source), expected);
        let mut engine = engine();
        let started = engine.debug_start_image(&image, true).unwrap();

        let mut outcome = started.outcome;
        let mut steps = 0;
        let mut inner = 0;
        loop {
            match outcome {
                DebugOutcome::Finished { result } => {
                    assert_eq!(result.as_string().unwrap(), expected);
                    break;
                }
                DebugOutcome::Paused(ref pause) => {
                    if let Some(address) = pause.address {
                        let sources = engine.debug_sources();
                        let mapped = sources
                            .iter()
                            .find(|s| s.code_id == address.code_id)
                            .unwrap();
                        let unit = &mapped.image.units[mapped.unit_index];
                        if unit
                            .commands
                            .iter()
                            .any(|map| map.anchor_pc == address.pc && map.nesting_depth > 0)
                        {
                            inner += 1;
                        }
                    }
                }
            }
            steps += 1;
            assert!(steps < 5000, "step loop did not finish");
            outcome = engine.step_instruction().unwrap();
        }
        assert!(steps > 1);
        if image
            .top_level()
            .commands
            .iter()
            .any(|map| map.nesting_depth > 0)
        {
            assert!(inner > 0, "must step inside edited script bodies");
        }
    }
}

#[test]
fn structured_control_and_lazy_expressions_match_tcl() {
    for source in [
        "list [expr {true}] [expr {yes}] [expr {false}] [expr {true && !false}]",
        "set n 0; while {$n<5} {incr n; if {$n==2} {continue}; if {$n==4} {break}}; set n",
        "set n 0; for {set i 0} {$i<6} {incr i} {if {$i%2==0} {continue}; incr n $i}; set n",
        "set n {}; foreach {a b} {1 2 3} c {x y z} {lappend n $a $b $c}; set n",
        "set n {}; foreach x {1 2 3} {if {$x==2} {continue}; lappend n $x; if {$x==3} {break}}; set n",
        "set n 0; if {$n} {set n 1} elseif {2>1} then {set n 2} else {set n 3}; set n",
        "set x [expr {0 && [error unseen]}]; set y [expr {1 || [error unseen]}]; list $x $y [expr {1 ? 6*7 : [error unseen]}]",
        "catch {error bad} result options; list $result [dict get $options -code]",
        "catch {catch {error inner} result; error outer} result; set result",
        "set n 0; foreach x {1 2 3} {catch {break} result; incr n}; set n",
        "proc f {n} {foreach x {1 2 3} {if {$x==$n} {return $x}}; return 0}; f 2",
        "catch {return -code error boom} value options; list $value [dict get $options -code] [dict get $options -level]",
        "set n {}; foreach x {1 2 3} {list prefix [if {$x==2} {continue}]; lappend n $x}; set n",
        "set n 0; while {$n<3} {incr n; catch {while {1} {break}}}; set n",
        "set n [expr {2**3**2}]; list $n [expr {int(12*sqrt(25))}] [expr {-2**2}]",
    ] {
        assert_eq!(finish(source), oracle(source), "{source}");
    }
}

#[test]
fn breakpoint_and_frame_edits_inside_a_loop_change_its_real_result() {
    let source = "set total 0\nfor {set i 0} {$i<3} {incr i} {\n  incr total $i\n}\nset total\n";
    let image = image(source);
    let mut engine = engine();
    let started = engine.debug_start_image(&image, true).unwrap();
    let line = image
        .top_level()
        .commands
        .iter()
        .find(|map| map.start_line == 3)
        .unwrap();
    let address = CodeAddress {
        code_id: started
            .bindings
            .iter()
            .find(|binding| binding.logical_unit_id == image.top_level().logical_id)
            .unwrap()
            .code_id,
        pc: line.anchor_pc,
    };
    engine.add_breakpoint(address);
    assert!(matches!(
        engine.continue_run().unwrap(),
        DebugOutcome::Paused(_)
    ));
    assert_eq!(
        engine
            .eval_in_frame(0, VmObj::from_str("set i"))
            .unwrap()
            .result
            .as_string()
            .unwrap(),
        "0"
    );
    engine
        .eval_in_frame(0, VmObj::from_str("set total 40"))
        .unwrap();
    engine.remove_breakpoint(address);
    match engine.continue_run().unwrap() {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "43"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn malformed_word_grouping_remains_rejected() {
    // A brace or quote inside an unquoted word is an ordinary character, as
    // in Tcl; only a word that starts with one groups.
    for source in [
        "set x {a}trailing",
        "set x \"a\"trailing",
        "set x {a",
        "set x \"a",
        "set x [list a",
        "set x ${a",
        "set x $a(b",
        "set x $a(b{c)",
    ] {
        assert!(
            compile_source(
                PathBuf::from("rejected.tcl"),
                source.into(),
                CompileMode::Hybrid
            )
            .is_err(),
            "{source}"
        );
    }
}
