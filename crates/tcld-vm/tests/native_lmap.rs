#![cfg(all(feature = "host", feature = "debug"))]
//! Native lmap iteration, completion, and mapped callback bodies against Tcl.
use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{CodeAddress, DebugOutcome, HybridEngine, RuntimeMode, VmObj};

fn mode(runtime: RuntimeMode) -> CompileMode {
    match runtime {
        RuntimeMode::VmStrict => CompileMode::VmStrict,
        RuntimeMode::Hybrid => CompileMode::Hybrid,
    }
}

fn engine(runtime: RuntimeMode) -> HybridEngine {
    let mut engine = HybridEngine::new(runtime).unwrap();
    if runtime == RuntimeMode::Hybrid {
        engine.bootstrap(Some("tcld")).unwrap();
    }
    engine
}

fn run(source: &str, runtime: RuntimeMode) -> Result<String, String> {
    let image = compile_source("/workspace/lmap.tcl".into(), source.into(), mode(runtime))
        .map_err(|error| error.to_string())?;
    let mut engine = engine(runtime);
    match engine
        .debug_start_image(&image, false)
        .map_err(|error| error.to_string())?
        .outcome
    {
        DebugOutcome::Finished { result } => {
            if runtime == RuntimeMode::VmStrict {
                assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
            }
            result.as_string().map_err(|error| error.to_string())
        }
        other => Err(format!("{other:?}")),
    }
}

fn compare(sources: &[&str]) {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    let mut failures = Vec::new();
    for source in sources {
        let expected = tcld_interp::Interpreter::new()
            .unwrap()
            .eval(source)
            .unwrap_or_else(|error| panic!("Tcl: {source}: {error}"));
        for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            let actual = run(source, runtime);
            if actual.as_ref() != Ok(&expected) {
                failures.push(format!(
                    "{runtime:?}: {source}\nTcl: {expected}\nVM: {actual:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn iteration_lists_padding_dynamic_commands_and_scope_match_tcl() {
    compare(&[
        "lmap x {1 2 3} {expr {$x*2}}",
        "::lmap {x y} {1 2 3} {list $x $y}",
        "set cmd lmap; set vars {x y}; set values {1 2 3}; set body {list $x $y}; $cmd $vars $values $body",
        "set words {lmap x {1 2} {incr x}}; {*}$words",
        "lmap {x y} {1 2 3} z {a b c} {list $x $y $z}",
        "lmap x {} {error unreachable}",
        "set x old; list [lmap x {} {set x}] $x",
        "list [lmap x {} y {1 2} {list $x $y}] $x $y",
        "lmap {x x} {1 2 3 4 5} {set x}",
        "lmap x {1 2} x {a b c} {set x}",
        "lmap x {1 2} {list}",
        "lmap x {{a b} {[literal]} {$not_script} {;literal} é😀} {set x}",
        "set values {1 2 3}; list [lmap x $values {set values changed; set x}] $values",
        "set vars x; list [lmap $vars {1 2 3} {set vars changed; set x}] $vars",
        "proc f {} {set prefix 40; list [lmap x {1 2} {expr {$prefix+$x}}] $x}; f",
        "proc f {} {lmap a(k) {1 2} {set a(k)}}; f",
        "proc f {} {::lmap x {1 2} {incr x}}; f",
        "proc f {} {lmap x {1 2} \"incr x\"}; f",
        "proc f {} {set x old; proc child {} {upvar 1 x alias; lmap alias {1 2} {incr alias}}; list [child] $x}; f",
        "namespace eval n {variable offset 40; list [lmap x {1 2} {incr offset; list $x $offset [info level]}] $x}",
        "lmap ::x {1 2} {incr ::x}; set ::x",
        "namespace eval n {}; lmap ::n:: {1 2} {set ::n::}; list [set ::n::] [info exists ::n]",
        "lmap {{}} {a b} {set {}}",
        "lmap {{space name}} {a b} {set {space name}}",
    ]);
}

#[test]
fn loop_completion_return_options_and_nested_callbacks_match_tcl() {
    let mut sources = vec![
        "lmap x {1 2 3 4 5} {if {$x == 2} {continue}; if {$x == 4} {break}; expr {$x*10}}".to_owned(),
        "lmap x {1 2} {foreach y {1 2 3} {if {$y == 2} {break}}; set x}".into(),
        "lmap x {1 2} {set c [catch {continue} m o]; list $x $c $m [dict get $o -code]}".into(),
        "lmap x {1 2} {set result [list before $x]; continue; error wrong}".into(),
        "proc f {} {lmap x {1 2 3} {if {$x == 2} {return found}; set x}; error wrong}; f".into(),
        "lmap x {1 2} {lmap y {3 4} {expr {$x*$y}}}".into(),
        "set result {}; foreach x {1 2} {lappend result [lmap y {a b} {if {$y eq {a}} {continue}; list $x $y}]}; set result".into(),
        "set body {switch $x {1 {continue} 3 {break} default {expr {$x*2}}}}; lmap x {1 2 3 4} $body".into(),
        "lsort -command {apply {{a b} {lindex [lmap x [list $a] y [list $b] {expr {$x-$y}}] 0}}} {3 1 2}".into(),
        "regsub -all -command . abc {apply {{m} {join [lmap x [list $m] {string toupper $x}] {}}}}".into(),
        "lmap x {c b a} {lsort -command {apply {{a b} {expr {$a-$b}}}} {3 1 2}}".into(),
        "set body {set x 1\nerror failed {} {LMAP BODY}}; set c [catch {lmap item {a} $body} m o]; list $c $m [dict get $o -errorcode] [string match {*\"lmap\" body line 2*} [dict get $o -errorinfo]]".into(),
    ];
    for body in [
        "break",
        "continue",
        "return found",
        "return -level 0 found",
        "return -code break found",
        "return -code continue found",
        "return -level 0 -code break -note stopped found",
        "return -level 0 -code continue -note skipped found",
        "return -code error -errorcode {LMAP FAIL} failed",
        "return -code 7 -note custom found",
        "return -level 2 -note outer found",
        "return -level 0 -note retained found",
        "error failed {} {LMAP FAIL}",
        "catch {error inner}; return -level 0 -note nested found",
        "return -level 0 -note retained found; break",
        "return -level 0 -note retained found; continue",
        "return -options {-level 0 -code break -note nested} found",
        "return -options {-level 0 -code continue -note nested} found",
        "return -level 0 -code break -errorcode \"bad \\{list\" found",
        "return -level 0 -code continue -errorstack odd found",
        "catch {return -level 0 -code break -note caught found} m o; list $m [dict get $o -note]",
        "foreach inner {a} {return -level 0 -code break -note inner found}; set x",
        "foreach inner {a} {return -level 0 -code continue -note inner found}; set x",
    ] {
        for procedure in 0..5 {
            let command = format!("lmap x {{1 2}} {}", tcld_compile::quote_word(body));
            let invocation = match procedure {
                0 => command,
                1 => format!("proc f {{}} {{{command}}}; f"),
                2 => format!("apply {{{{}} {{{command}}}}}"),
                3 => format!(
                    "proc f {{}} {{set body {}; lmap x {{1 2}} $body}}; f",
                    tcld_compile::quote_word(body)
                ),
                _ => format!(
                    "apply {{{{}} {{set body {}; lmap x {{1 2}} $body}}}}",
                    tcld_compile::quote_word(body)
                ),
            };
            sources.push(format!(
                "set c [catch {{{invocation}}} m o]; list $c $m [dict get $o -code] [dict get $o -level] [expr {{[dict exists $o -errorcode] ? [dict get $o -errorcode] : {{}}}}] [expr {{[dict exists $o -note] ? [dict get $o -note] : {{}}}}]",
            ));
        }
    }
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn validation_order_partial_assignments_and_errors_match_tcl() {
    let mut sources = Vec::new();
    for command in [
        "lmap",
        "::lmap",
        "lmap x",
        "lmap x {}",
        "lmap x {} y {}",
        "lmap {} {1} {list}",
        "lmap {} {} {list}",
        r#"lmap "bad \{list" {} {list}"#,
        r#"lmap x "bad \{list" {list}"#,
        r#"lmap {} "bad \{list" {list}"#,
        r#"lmap x "bad \{list" {} {1} {list}"#,
        r#"lmap x {} "bad \{list" {1} {list}"#,
        r#"lmap x {} y "bad \{list" {list}"#,
        "set x scalar; lmap x(k) {1} {error wrong}",
        "set a(k) array; lmap a {1} {error wrong}",
        "lmap ::absent::x {1} {error wrong}",
        "lmap ::absent::x(k) {1} {error wrong}",
        "lmap ::absent:: {1} {error wrong}",
        "namespace eval n {}; lmap ::n::absent::x {1} {error wrong}",
        "set y scalar; lmap x {a} y(k) {b} {error wrong}",
        "set y scalar; lmap x {} y(k) {} {error wrong}",
    ] {
        sources.push(format!(
            "set c [catch {{{command}}} m o]; list $c $m [expr {{[dict exists $o -errorcode] ? [dict get $o -errorcode] : {{}}}}] [info exists x] [expr {{[info exists x] ? $x : {{}}}}]",
        ));
    }
    sources.push("set y scalar; set c [catch {lmap x {a} y(k) {b} {error wrong}} m o]; list $c $m [dict get $o -errorcode] $x [string match {*setting lmap loop variable \"y(k)\"*} [dict get $o -errorinfo]]".into());
    // An assignment on a later iteration must fail through the caller's catch.
    sources.push("set c [catch {lmap x {1 2} {unset x; set x(k) array; list done}} m o]; list $c $m [dict get $o -errorcode] [string match {*setting lmap loop variable \"x\"*} [dict get $o -errorinfo]] [string match {*\"lmap\" body*} [dict get $o -errorinfo]]".into());
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn unexecuted_bodies_do_not_compile_and_executed_bodies_keep_admission() {
    compare(&[
        "list [lmap x {} {interp eval {} {error unreachable}}] [info exists x]",
        r#"lmap x {} "bad \{script""#,
    ]);
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        assert_eq!(run(
            "set c [catch {lmap x {1} {set ::before yes; interp eval {} {set ::escaped yes}}} m]; list $c $x [info exists ::before] [info exists ::escaped]",
            runtime,
        ).unwrap(), "1 1 0 0", "{runtime:?}");
    }
}

#[test]
fn repeated_bodies_keep_source_breakpoints_caller_locals_and_live_edits() {
    let body = "set marker [expr {40+$x}]\nincr marker\nset marker";
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        for computed in [false, true] {
            let command = if computed {
                format!(
                    "set body {}; lmap x {{0 1 2}} $body",
                    tcld_compile::quote_word(body)
                )
            } else {
                format!("lmap x {{0 1 2}} {}", tcld_compile::quote_word(body))
            };
            let source = format!("proc work {{}} {{{command}}}; work",);
            let image =
                compile_source("/workspace/lmap-debug.tcl".into(), source, mode(runtime)).unwrap();
            let mut engine = engine(runtime);
            let mut state = engine.debug_start_image(&image, true).unwrap().outcome;
            let mut address = None;
            for _ in 0..300 {
                if let DebugOutcome::Paused(pause) = &state
                    && let Some(at) = pause.address
                    && engine.debug_sources().iter().any(|source| {
                        source.code_id == at.code_id && source.image.source.content.contains(body)
                    })
                    && engine
                        .ui_frame_locals(0)
                        .iter()
                        .any(|(name, value)| name == "marker" && value == "40")
                {
                    address = Some(at);
                    break;
                }
                assert!(
                    !matches!(state, DebugOutcome::Finished { .. }),
                    "{runtime:?}: body never paused"
                );
                state = engine.step_instruction().unwrap();
            }
            let at = address.expect("mapped lmap body paused with caller locals");
            let mapped = engine
                .debug_sources()
                .into_iter()
                .find(|source| source.code_id == at.code_id)
                .unwrap();
            let breakpoint = CodeAddress {
                code_id: at.code_id,
                pc: mapped.image.units[mapped.unit_index]
                    .commands
                    .iter()
                    .find(|command| {
                        &mapped.image.source.content[command.source_span.clone()] == "set marker"
                    })
                    .unwrap()
                    .anchor_pc,
            };
            engine.add_breakpoint(breakpoint);
            for iteration in 0..3 {
                assert!(
                    matches!(engine.continue_run().unwrap(), DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint))
                );
                assert!(
                    engine
                        .ui_frame_locals(0)
                        .iter()
                        .any(|(name, value)| name == "x" && value == &iteration.to_string())
                );
                if runtime == RuntimeMode::Hybrid {
                    engine
                        .eval_in_frame(
                            0,
                            VmObj::from_str(&format!("set marker {}", 80 + iteration)),
                        )
                        .unwrap();
                }
            }
            engine.remove_breakpoint(breakpoint);
            match engine.continue_run().unwrap() {
                DebugOutcome::Finished { result } => assert_eq!(
                    result.as_string().unwrap(),
                    if runtime == RuntimeMode::Hybrid {
                        "80 81 82"
                    } else {
                        "41 42 43"
                    }
                ),
                other => panic!("{runtime:?}: {other:?}"),
            }
            if runtime == RuntimeMode::VmStrict {
                assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
            }
        }
    }
}

#[test]
fn anonymous_callback_bodies_keep_breakpoints_across_changing_arguments() {
    for (command, parameters, input, body, first, second, expected) in [
        (
            "lsort -command",
            "a b",
            "{3 2 1}",
            "set result [expr {$a-$b}]\nset marker [list $a $b]\nset result",
            "3 2",
            "2 1",
            "1 2 3",
        ),
        (
            "regsub -all -command . abc",
            "m",
            "",
            "set result [string toupper $m]\nset marker $m\nset result",
            "a",
            "b",
            "ABC",
        ),
    ] {
        let callback = format!(
            "lindex [lmap once {{go}} {}] 0",
            tcld_compile::quote_word(body)
        );
        let prefix = format!(
            "[list apply [list {{{parameters}}} {}]]",
            tcld_compile::quote_word(&callback)
        );
        let source = format!("{command} {prefix} {input}");
        for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            let image = compile_source(
                "/workspace/lmap-callback.tcl".into(),
                source.clone(),
                mode(runtime),
            )
            .unwrap();
            let mut engine = engine(runtime);
            let mut state = engine.debug_start_image(&image, true).unwrap().outcome;
            let mut address = None;
            for _ in 0..400 {
                if let DebugOutcome::Paused(pause) = &state
                    && let Some(at) = pause.address
                    && engine.debug_sources().iter().any(|source| {
                        source.code_id == at.code_id
                            && source.origin.is_some()
                            && source.image.source.content.contains(body)
                    })
                    && engine
                        .ui_frame_locals(0)
                        .iter()
                        .any(|(name, value)| name == "marker" && value == first)
                {
                    address = Some(at);
                    break;
                }
                assert!(
                    !matches!(state, DebugOutcome::Finished { .. }),
                    "{runtime:?}: {command} body never paused"
                );
                state = engine.step_instruction().unwrap();
            }
            let at = address.expect("mapped lmap body inside callback lambda");
            let mapped = engine
                .debug_sources()
                .into_iter()
                .find(|source| source.code_id == at.code_id)
                .unwrap();
            let breakpoint = CodeAddress {
                code_id: at.code_id,
                pc: mapped.image.units[mapped.unit_index]
                    .commands
                    .iter()
                    .find(|command| {
                        &mapped.image.source.content[command.source_span.clone()] == "set result"
                    })
                    .unwrap()
                    .anchor_pc,
            };
            engine.add_breakpoint(breakpoint);
            for marker in [first, second] {
                let state = engine.continue_run().unwrap();
                assert!(
                    matches!(state, DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint)),
                    "{runtime:?}: {command}: {state:?}"
                );
                assert!(
                    engine
                        .ui_frame_locals(0)
                        .iter()
                        .any(|(name, value)| name == "marker" && value == marker)
                );
            }
            engine.remove_breakpoint(breakpoint);
            match engine.continue_run().unwrap() {
                DebugOutcome::Finished { result } => {
                    assert_eq!(result.as_string().unwrap(), expected)
                }
                other => panic!("{runtime:?}: {command}: {other:?}"),
            }
            if runtime == RuntimeMode::VmStrict {
                assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
            }
        }
    }
}

#[test]
fn hybrid_assignment_traces_coroutines_tailcalls_and_cancellation_match_tcl() {
    for source in [
        "proc traced {n e op} {upvar 1 $n v; incr v 10}; proc f {} {trace add variable x write traced; lmap x {1 2} {set x}}; f",
        "set out {}; proc traced {n e op} {upvar 1 $n v; lappend ::out $v; if {$v == 2} {error denied}}; proc f {} {trace add variable x write traced; set c [catch {lmap x {1 2 3} {list $x}} m o]; list $c $m [dict get $o -errorcode] $::out}; f",
        "proc work {} {lmap x {1 2} {set resumed [yield $x]; list $x $resumed}}; list [coroutine co work] [co one] [co two]",
        "proc work {} {lmap x {1 2} {try {yield $x; expr {$x*2}} finally {yield cleanup}}}; list [coroutine co work] [co] [co] [co] [co]",
        "set out {}; proc work {} {lmap x {1 2} {try {yield $x} finally {lappend ::out cleanup}}}; coroutine co work; rename co {}; set out",
        "proc target {x} {return $x}; proc work {} {lmap x {1 2} {tailcall target $x}}; work",
        "proc traced {n e op} {upvar 1 $n v; incr v 10}; proc work {} {trace add variable x write traced; lmap x {1 2} {yield $x; set x}}; list [coroutine co work] [co] [co]",
    ] {
        tcld_interp::Interpreter::find_executable(Some("tcld"));
        let expected = tcld_interp::Interpreter::new()
            .unwrap()
            .eval(source)
            .unwrap();
        assert_eq!(run(source, RuntimeMode::Hybrid), Ok(expected), "{source}");
    }
}
