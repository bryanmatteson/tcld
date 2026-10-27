#![cfg(all(feature = "host", feature = "debug"))]
use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{DebugOutcome, HybridEngine, RuntimeMode};
fn compare(s: &str) {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    let oracle = tcld_interp::Interpreter::new().unwrap();
    let expected = oracle.eval(s).unwrap_or_else(|e| panic!("oracle {s}: {e}"));
    let image = compile_source(
        "/workspace/switch-lmap.tcl".into(),
        s.into(),
        CompileMode::Hybrid,
    )
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
fn switch_matches_real_tcl_for_static_and_generated_branches() {
    for s in [
        "switch b {a {list wrong} b {expr {6*7}} default {list fallback}}",
        "set key b; set body {expr {6*7}}; switch $key a {error wrong} b $body default {error wrong}",
        "set arms [list a {error wrong} b {expr {6*7}}]; switch b $arms",
        "set arms [list a - b {expr {6*7}}]; switch a $arms",
        "switch -glob abc {a* {list matched} default {list wrong}}",
        "switch -nocase -exact ABC abc {list matched}",
        "switch -n -g ABC a* {list matched}",
        "switch -- -x {-x {list yes} default {error wrong}}",
        "switch absent {a {error wrong}}",
        "switch a {default {error wrong} a {list yes}}",
        "switch absent {a {error wrong} default {list fallback}}",
        "switch -regexp -matchvar matches -indexvar indices abc123 {([a-z]+)([0-9]+)} {list $matches $indices}",
        "proc f {} {switch -regexp -matchvar m -indexvar i abc123 {([a-z]+)([0-9]+)} {list $m $i}}; f",
        "proc f {} {switch -regexp -matchvar m -indexvar i absent {^[0-9]+$} {error wrong} default {list $m $i}}; f",
        "proc f {} {switch -regexp -matchvar a(k) abc {(b)} {set a(k)}}; f",
        "proc f {} {set body {return 42}; switch -glob abc a* $body; error unreachable}; f",
        "set out {}; foreach x {1 2 3} {switch $x 2 {continue} 3 {break} default {lappend out $x}}; set out",
        "set c [catch {switch a a {error bad {} {APP FAIL}}} m o]; list $c $m [dict get $o -errorcode]",
        "switch a a {list yes} b {interp eval {} {error unmapped}}",
        "set out {}; proc work {} {switch a a {lappend ::out [yield pause]; return done}}; list [coroutine co work] [co resumed] $out",
        "proc target {} {return done}; proc work {} {switch a a {tailcall target}}; work",
        "proc traced {n e op} {upvar 1 $n v; lappend v traced}; proc f {} {trace add variable m write traced; switch -regexp -matchvar m abc {(b)} {set m}}; f",
        "proc traced {n e op} {error denied}; proc f {} {trace add variable m write traced; switch -regexp -matchvar m abc b {error unreachable}}; list [catch {f} m] $m",
        "proc traced {n e op} {upvar 1 $n v; lappend v traced}; proc f {} {yield ready; trace add variable m write traced; switch -regexp -matchvar m abc b {set m}}; list [coroutine co f] [co]",
    ] {
        compare(s)
    }
}
#[test]
fn lmap_uses_native_iteration_bindings_and_completion_collection() {
    for s in [
        "lmap x {1 2 3} {expr {$x*2}}",
        "set body {expr {$x*2}}; lmap x {1 2 3} $body",
        "set vars {x y}; set values {1 2 3}; set body {list $x $y}; lmap $vars $values $body",
        "lmap {x y} {1 2 3} z {a b c} {list $x $y $z}",
        "lmap x {} {error unreachable}",
        "lmap x {1 2 3 4 5} {if {$x == 2} {continue}; if {$x == 4} {break}; expr {$x*10}}",
        "lmap x {1 2} {list}",
        "proc f {} {set prefix 40; list [lmap x {1 2} {expr {$prefix+$x}}] $x}; f",
        "proc f {} {lmap a(k) {1 2} {set a(k)}}; f",
        "proc f {} {set x old; proc child {} {upvar 1 x v; lmap v {1 2} {incr v}}; list [child] $x}; f",
        "proc f {} {lmap x {1 2 3} {if {$x == 2} {return found}; set x}; error unreachable}; f",
        "set c [catch {lmap x {1 2 3} {if {$x == 2} {error bad {} {APP FAIL}}; set x}} m o]; list $c $m [dict get $o -errorcode]",
        "proc traced {n e op} {upvar 1 $n v; incr v 10}; proc f {} {trace add variable x write traced; lmap x {1 2} {set x}}; f",
        "proc traced {n e op} {error denied}; proc f {} {trace add variable x write traced; lmap x {1 2} {error unreachable}}; list [catch {f} m] $m",
        "proc work {} {lmap x {1 2} {set resume [yield $x]; list $x $resume}}; list [coroutine co work] [co one] [co two]",
        "proc traced {n e op} {upvar 1 $n v; incr v 10}; proc work {} {trace add variable x write traced; lmap x {1 2} {yield $x; set x}}; list [coroutine co work] [co] [co]",
        "proc work {} {lmap x {1 2} {try {yield $x; expr {$x*2}} finally {yield cleanup}}}; list [coroutine co work] [co] [co] [co] [co]",
        "proc target {x} {return $x}; proc work {} {lmap x {1 2} {tailcall target $x}}; work",
        "lmap x {1 2} {lmap y {3 4} {expr {$x*$y}}}",
        "namespace eval n {lmap x {1 2} {incr x}; set x}",
        "set body {switch $x 1 {continue} 3 {break} default {expr {$x*2}}}; lmap x {1 2 3 4} $body",
        "set out {}; proc cb {cmd args} {lappend ::out [join $args |]}; proc f {} {lmap x {1 2} {incr x}}; trace add execution f {enterstep leavestep} cb; list [f] [join $out \n]",
    ] {
        compare(s)
    }
}
#[test]
fn invalid_native_forms_keep_their_diagnostics() {
    for form in [
        "switch",
        "switch a {}",
        "switch a {one}",
        "switch a {a -}",
        "switch a {# comment a {list} extra}",
        "switch -bad a a {list}",
        "switch -exact -glob a a {list}",
        "switch -matchvar m a a {list}",
        "switch -regexp a {[} {list}",
        "switch -indexvar x a",
        "switch - a a {list}",
        "lmap",
        "lmap x {}",
        "lmap {} {1} {list}",
        "lmap x {1} y {}",
    ] {
        compare(&format!(
            "set c [catch {{{form}}} m o]; list $c $m [dict get $o -errorcode]"
        ));
    }
    for form in ["switch a $bad", "lmap $bad {} {list}", "lmap x $bad {list}"] {
        compare(&format!(
            "set bad [format %c 123]; set c [catch {{{form}}} m o]; list $c $m [dict get $o -errorcode]"
        ));
    }
}
#[test]
fn generated_branches_and_repeated_bodies_keep_source_and_live_breakpoints() {
    for name in ["switch", "lmap"] {
        let body = "set marker 40\nincr marker\nset ::result $marker";
        let q = tcld_compile::quote_word;
        let command = if name == "switch" {
            "set branches [list chosen $body]; switch chosen $branches"
        } else {
            "lmap x {0 1} $body"
        };
        let source = format!("proc work {{}} {{set body {}; {command}}}\nwork", q(body));
        let image = compile_source(
            "/workspace/generated-control.tcl".into(),
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
                "{name} body never paused"
            );
            state = e.step_instruction().unwrap();
        }
        let a = address.expect("generated body has mapped source and caller variables");
        let source = e
            .debug_sources()
            .into_iter()
            .find(|s| s.code_id == a.code_id)
            .unwrap();
        let pc = source.image.units[source.unit_index]
            .commands
            .iter()
            .find(|m| {
                source.image.source.content[m.source_span.clone()].starts_with("set ::result")
            })
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
        if name == "lmap" {
            let pause = e.continue_run().unwrap();
            assert!(matches!(pause, DebugOutcome::Paused(_)));
            assert_eq!(
                e.pause_snapshot().unwrap().address.unwrap().code_id,
                a.code_id
            );
            assert!(
                e.ui_frame_locals(0)
                    .iter()
                    .any(|(n, v)| n == "x" && v == "1")
            );
            e.eval_in_frame(0, tcld_vm::VmObj::from_str("set marker 100"))
                .unwrap();
        }
        e.remove_breakpoint(bp);
        match e.continue_run().unwrap() {
            DebugOutcome::Finished { result } => assert_eq!(
                result.as_string().unwrap(),
                if name == "lmap" { "81 100" } else { "81" }
            ),
            o => panic!("{o:?}"),
        }
    }
}
#[test]
fn private_control_entry_rejects_unmapped_source() {
    let image=compile_source("/workspace/private.tcl".into(),"list [catch {::tcld::control 0 :: {} lmap x {1} {set ::escaped yes}} m] [info exists ::escaped]".into(),CompileMode::Hybrid).unwrap();
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    match e.debug_start_image(&image, false).unwrap().outcome {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "1 0"),
        o => panic!("{o:?}"),
    }
}
