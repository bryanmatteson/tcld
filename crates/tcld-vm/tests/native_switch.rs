#![cfg(all(feature = "host", feature = "debug"))]
//! Native switch matching, completion, and mapped callback bodies against Tcl.
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
    let image = compile_source("/workspace/switch.tcl".into(), source.into(), mode(runtime))
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
fn modes_options_dynamic_arms_and_fallthrough_match_tcl() {
    compare(&[
        "switch b {a {list wrong} b {expr {6*7}} default {list fallback}}",
        "set key b; set body {expr {6*7}}; switch $key a {error wrong} b $body default {error wrong}",
        "set cmd ::switch; set arms [list a - b {expr {6*7}}]; $cmd a $arms",
        "set words {switch -n -g ABC a* {list matched}}; {*}$words",
        "switch -- -x {-x {list yes} default {error wrong}}",
        "switch -glob abc {a* {list matched} default {error wrong}}",
        "switch -nocase -exact ABC abc {list matched}",
        "switch -nocase -nocase ABC {abc {list matched}}",
        "switch -e 01 {1 {error numeric} 01 {list string}}",
        "switch absent {a {error wrong}}",
        "switch default {default {list first} a {error wrong} default {error wrong}}",
        "switch absent {default {error wrong} a {error wrong} default {list last}}",
        "switch absent {a {error wrong} default {list fallback}}",
        "switch a {a - b - c {list chained} default {error wrong}}",
        "switch -n İ {i {list simplefold} default {error wrong}}",
        "switch -n ẞ {ß {list simplefold} default {error wrong}}",
        "switch -n ẞ {ss {error expandedfold} default {list simplefold}}",
        "switch -n -g ÄBC {ä* {list unicode}}",
        "switch -r -n ÄBC {^äbc$ {list unicode}}",
        "switch -g {a*b} {a\\*b {list escaped}}",
        "switch a {a {list selected} unused {interp eval {} {error unreachable}}}",
        "namespace eval n {variable x 40; switch a {a {incr x 2}}; list $x [info level]}",
        "proc f {x} {set y 40; switch $x {a {incr y 2} default {error wrong}}; list $y [info level]}; f a",
        "proc f {} {set x 40; switch a {a {upvar 0 x alias; incr alias 2}}; set x}; f",
        "proc f {} {set body {incr x 2}; set x 40; uplevel 0 [list switch a a $body]; set x}; f",
    ]);
}

#[test]
fn regexp_captures_defaults_and_assignment_order_match_tcl() {
    compare(&[
        "switch -regexp -matchvar m -indexvar i abc123 {([a-z]+)([0-9]+)} {list $m $i}",
        "switch -r -m m -i i ab {(a)(x)?(b)} {list $m $i}",
        "switch -r -m m -i i {} {()} {list $m $i}",
        "switch -r -m m -i i abc {^()} {list $m $i}",
        "switch -r -m m -i i abc {$()} {list $m $i}",
        "switch -r -m m -i i abc {(?=b)} {list $m $i}",
        "switch -r -m m -i i é😀x {(😀)(x)} {list $m $i}",
        "switch -r -n -m m -i i ÄBC {(ä)(bc)} {list $m $i}",
        "switch -r -m m -i i abc {(a)(b)(c)()()()()()()()()} {list $m $i}",
        "switch -r -m m -i i abc {(a)} - b {list $m $i}",
        "set m saved; set i saved; set result [switch -r -m m -i i absent {^a$ {error wrong}}]; list $result $m $i",
        "switch -r -m m -i i absent {^a$ {error wrong} default {list $m $i}}",
        "switch -r -m m -i m abc {(a)} {set m}",
        "switch -r -m old -m m -i i abc {(a)} {list [info exists old] $m $i}",
        "proc f {} {set storage(index) old; upvar 0 storage(index) alias; switch -r -m alias -i storage(i) abc {(a)} {list $alias $storage(index) $storage(i)}}; f",
        "namespace eval n {switch -r -m m -i i abc {(a)} {list $m $i}}; list $n::m $n::i",
        "set a(k) old; set c [catch {switch -r -i a -m m abc {(a)} {set ::ran yes}} m o]; list $c $m [dict get $o -errorcode] [info exists ::ran]",
        "set a(k) old; set c [catch {switch -r -i i -m a abc {(a)} {set ::ran yes}} m o]; list $c $m [dict get $o -errorcode] $i [info exists ::ran]",
    ]);
}

#[test]
fn argument_errors_and_validation_order_match_tcl() {
    let commands = [
        "switch",
        "switch a",
        "switch a {}",
        "::switch a {}",
        "switch a {one}",
        "switch a {a -}",
        "switch a {a {list yes} unused -}",
        "switch a {# comment a {list} extra}",
        "switch a # comment a {list} extra",
        "switch -bad a a {list}",
        "switch - a a {list}",
        "switch -exact -glob a a {list}",
        "switch -r -r a a {list}",
        "switch -matchvar m a a {list}",
        "switch -indexvar i -matchvar m a {a {list}}",
        "switch -regexp -indexvar a b",
        "switch -regexp -matchvar a b",
        "switch -indexvar x a",
        "switch -matchvar x a",
        "switch -regexp a {[} {list}",
        "switch -r a {a {list yes} [ {error wrong}}",
        r#"switch a "bad \{list""#,
        "switch -n a {}",
        "switch -m m a {# comment a}",
        "switch -r -m m a {a {list yes} unused -}",
    ];
    let sources: Vec<_> = commands.iter().map(|command| format!(
        "set c [catch {{{command}}} m o]; list $c $m [expr {{[dict exists $o -errorcode] ? [dict get $o -errorcode] : {{}}}}]"
    )).collect();
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn completion_options_caller_loops_and_callback_use_match_tcl() {
    let mut sources = vec![
        "proc f {} {switch a {a {return found}}; error wrong}; f".to_owned(),
        "set out {}; foreach x {a b c} {switch $x {a {continue} b {lappend out $x} c {break}}; lappend out after}; set out".into(),
        "set out {}; foreach x {a b} {switch $x {a {catch {error inner}; lappend out $x} b {lappend out $x}}}; set out".into(),
        "proc f {} {switch a {a {return -level 0 -note retained result}}}; set c [catch {f} m o]; list $c $m [dict get $o -note]".into(),
        "lsort -command {apply {{a b} {switch [string compare $a $b] {-1 {return -1} 0 {return 0} default {return 1}}}}} {c b a}".into(),
        "regsub -all -command . abc {apply {{m} {switch $m {a {return X} default {string toupper $m}}}}}".into(),
        "lsort -command {apply {{a b} {switch -r -m m -i i $a {(.)} {expr {$a-$b}}}}} {3 1 2}".into(),
        "set c [catch {set arms [list a - b {set x 1\nerror failed {} {ARM FAILURE}}]; switch a $arms} m o]; list $c $m [dict get $o -errorcode] [string match {*\"a\" arm line 2*} [dict get $o -errorinfo]]".into(),
        "set c [catch {lsort -command {apply {{a b} {set arms [list a {error failed {} {ARM SORT}}]; switch a $arms}}} {b a}} m o]; list $c $m [dict get $o -errorcode] [string match {*\"a\" arm line 1*lambda term*(-compare command)*} [dict get $o -errorinfo]]".into(),
    ];
    for body in [
        "break",
        "continue",
        "return found",
        "return -level 0 found",
        "return -code break found",
        "return -code continue found",
        "return -code error -errorcode {ARM FAIL} failed",
        "return -code 7 -note custom found",
        "return -level 2 -note outer found",
        "return -level 0 -note retained found",
        "error failed {} {ARM FAIL}",
        "catch {error inner}; return -code 7 found",
    ] {
        for wrap in [false, true] {
            let branch = format!("switch a [list a {}]", tcld_compile::quote_word(body));
            let invocation = if wrap {
                format!("proc f {{}} {{{branch}}}; f")
            } else {
                branch
            };
            sources.push(format!(
                "set c [catch {{{invocation}}} m o]; list $c $m [dict get $o -code] [dict get $o -level] [expr {{[dict exists $o -errorcode] ? [dict get $o -errorcode] : {{}}}}] [expr {{[dict exists $o -note] ? [dict get $o -note] : {{}}}}]"
            ));
        }
    }
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn only_the_selected_body_is_admitted_before_any_of_its_commands_run() {
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let source = "set c [catch {switch a {a {set ::before yes; interp eval {} {set ::escaped yes}}}} m]; list $c [info exists ::before] [info exists ::escaped]";
        assert_eq!(run(source, runtime).unwrap(), "1 0 0", "{runtime:?}");
    }
    compare(&["switch b {a {interp eval {} {error unreachable}} b {list selected}}"]);
}

#[test]
fn nested_callback_branches_keep_breakpoints_locals_and_repeated_calls() {
    let body = "set result [string compare $a $b]\nset marker [list $a $b]\nreturn $result";
    let source = format!(
        "lsort -command [list apply [list {{a b}} [list switch selected [list selected {}]]]] {{c b a}}",
        tcld_compile::quote_word(body),
    );
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let image = compile_source(
            "/workspace/switch-sort.tcl".into(),
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
                        && source.image.source.content.as_ref() == body
                })
                && engine
                    .ui_frame_locals(0)
                    .iter()
                    .any(|(name, value)| name == "marker" && value == "c b")
            {
                address = Some(at);
                break;
            }
            assert!(
                !matches!(state, DebugOutcome::Finished { .. }),
                "{runtime:?}: branch never paused"
            );
            state = engine.step_instruction().unwrap();
        }
        let address =
            address.expect("mapped branch in comparator lambda paused with caller locals");
        assert!(
            engine
                .ui_frame_locals(0)
                .iter()
                .any(|(name, value)| name == "a" && value == "c")
        );
        let mapped = engine
            .debug_sources()
            .into_iter()
            .find(|source| source.code_id == address.code_id)
            .unwrap();
        let breakpoint = CodeAddress {
            code_id: address.code_id,
            pc: mapped.image.units[mapped.unit_index]
                .commands
                .iter()
                .find(|command| {
                    mapped.image.source.content[command.source_span.clone()].starts_with("return")
                })
                .unwrap()
                .anchor_pc,
        };
        engine.add_breakpoint(breakpoint);
        if runtime == RuntimeMode::Hybrid {
            engine
                .eval_in_frame(0, VmObj::from_str("set result -1"))
                .unwrap();
        }
        assert!(matches!(
            engine.continue_run().unwrap(),
            DebugOutcome::Paused(_)
        ));
        let repeated = engine.continue_run().unwrap();
        assert!(
            matches!(repeated, DebugOutcome::Paused(_)),
            "{runtime:?}: {repeated:?}"
        );
        assert_eq!(
            engine.pause_snapshot().unwrap().address.unwrap().code_id,
            address.code_id
        );
        engine.remove_breakpoint(breakpoint);
        match engine.continue_run().unwrap() {
            DebugOutcome::Finished { result } => assert_eq!(
                result.as_string().unwrap(),
                if runtime == RuntimeMode::Hybrid {
                    "a c b"
                } else {
                    "a b c"
                }
            ),
            other => panic!("{runtime:?}: {other:?}"),
        }
        if runtime == RuntimeMode::VmStrict {
            assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
        }
    }
}

#[test]
fn substitution_branch_breakpoints_survive_changing_capture_arguments() {
    let body = "set replacement [string toupper $m]\nset marker $m\nset replacement";
    let source = format!(
        "regsub -all -command . abc [list apply [list m [list switch selected [list selected {}]]]]",
        tcld_compile::quote_word(body),
    );
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let image = compile_source(
            "/workspace/switch-replace.tcl".into(),
            source.clone(),
            mode(runtime),
        )
        .unwrap();
        let mut engine = engine(runtime);
        let mut state = engine.debug_start_image(&image, true).unwrap().outcome;
        let mut address = None;
        for _ in 0..300 {
            if let DebugOutcome::Paused(pause) = &state
                && let Some(at) = pause.address
                && engine.debug_sources().iter().any(|source| {
                    source.code_id == at.code_id
                        && source.origin.is_some()
                        && source.image.source.content.as_ref() == body
                })
                && engine
                    .ui_frame_locals(0)
                    .iter()
                    .any(|(name, value)| name == "marker" && value == "a")
            {
                address = Some(at);
                break;
            }
            assert!(
                !matches!(state, DebugOutcome::Finished { .. }),
                "{runtime:?}: branch never paused"
            );
            state = engine.step_instruction().unwrap();
        }
        let at = address.expect("mapped substitution branch with caller locals");
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
                    &mapped.image.source.content[command.source_span.clone()] == "set replacement"
                })
                .unwrap()
                .anchor_pc,
        };
        engine.add_breakpoint(breakpoint);
        if runtime == RuntimeMode::Hybrid {
            engine
                .eval_in_frame(0, VmObj::from_str("set replacement X"))
                .unwrap();
        }
        for capture in ["a", "b", "c"] {
            let state = engine.continue_run().unwrap();
            assert!(
                matches!(state, DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint)),
                "{runtime:?}: {state:?}"
            );
            assert!(
                engine
                    .ui_frame_locals(0)
                    .iter()
                    .any(|(name, value)| name == "m" && value == capture)
            );
        }
        engine.remove_breakpoint(breakpoint);
        match engine.continue_run().unwrap() {
            DebugOutcome::Finished { result } => assert_eq!(
                result.as_string().unwrap(),
                if runtime == RuntimeMode::Hybrid {
                    "XBC"
                } else {
                    "ABC"
                }
            ),
            other => panic!("{runtime:?}: {other:?}"),
        }
        if runtime == RuntimeMode::VmStrict {
            assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
        }
    }
}
