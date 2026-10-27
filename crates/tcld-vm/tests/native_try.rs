#![cfg(all(feature = "host", feature = "debug"))]
//! Native try completions and exact mapped scripts compared with embedded Tcl.
use tcld_compile::{CompileMode, compile_source, quote_word};
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
    let image = compile_source("/workspace/try.tcl".into(), source.into(), mode(runtime))
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
fn handler_selection_fallthrough_bindings_and_scope_match_tcl() {
    compare(&[
        "try {expr {21*2}}",
        "::try {expr {21*2}} finally {set cleaned yes}",
        "set cmd try; set body {error failed {} {APP FAIL detail}}; set handler {list $m [dict get $o -errorcode]}; $cmd $body tr {APP FAIL} {m o} $handler f {set cleaned yes}",
        "set words {try {expr {21*2}} on ok {m o} {list $m [dict get $o -code]}}; {*}$words",
        "try {error missing {} {POSIX ENOENT file}} trap {POSIX WRONG} {} {error wrong} trap {POSIX ENOENT} {m o} {list $m [dict get $o -errorcode]}",
        "try {error missing {} {{APP CODE} detail}} trap {{APP CODE}} {m o} {dict get $o -errorcode}",
        "try {error missing {} {APP}} on error {} - on ok {m o} - trap {OTHER} {m o} {list shared $m [dict get $o -code]}",
        "try {list body} on ok {m o ignored} {list $m [dict get $o -level] [info exists ignored]}",
        "try {error failed} on error {same same} {dict get $same -code}",
        "try {error failed} on error {a(k) options} {list $a(k) [dict get $options -code]}",
        "try {error failed} on error {{space name} options} {list [set {space name}] [dict get $options -code]}",
        "try {error failed} on error {{} options} {list [set {}] [dict get $options -code]}",
        "namespace eval n {variable count 1; try {incr count} on ok {m o} {list $m [namespace current]} finally {incr count}}",
        "proc f {} {set prefix 40; try {expr {$prefix+2}} on ok {m o} {list $prefix $m [info level]} finally {incr prefix}}; f",
        "proc f {} {set m old; proc child {} {upvar 1 m alias; try {error failed} on error {alias o} {set alias}}; list [child] $m}; f",
        "set out {}; foreach x {1 2 3} {try {if {$x == 2} {continue}; if {$x == 3} {break}; lappend out $x} finally {lappend out cleanup}}; set out",
        "lmap x {1 2 3 4} {try {if {$x == 2} {continue}; if {$x == 4} {break}; expr {$x*10}} finally {set cleaned yes}}",
        "lsort -command {apply {{a b} {try {expr {$a-$b}} finally {set cleaned yes}}}} {3 1 2}",
        "regsub -all -command . abc {apply {{m} {try {string toupper $m} finally {set cleaned yes}}}}",
        "try {try {error inner {} {APP INNER}} finally {set inner yes}} trap APP {m o} {list $m [dict get $o -errorcode]} finally {set outer yes}",
    ]);
}

// Compare all completion fields that carry control or application state,
// recursively through -during. Debugger frames have their own stack text.
const OPTIONS: &str = "proc options {o} {set row {}; foreach k {-code -level -errorcode -note} {lappend row [expr {[dict exists $o $k] ? [dict get $o $k] : {}}]}; if {[dict exists $o -during]} {lappend row [options [dict get $o -during]]}; set row}; ";

#[test]
fn body_handler_finally_and_nested_during_completions_match_tcl() {
    let mut sources = Vec::new();
    for body in [
        "list body",
        "error body {} {APP BODY}",
        "return body",
        "return -level 0 -note body body",
        "return -level 0 -code break -note body body",
        "return -level 0 -code continue -note body body",
        "return -code error -errorcode {APP RETURN} -note body body",
        "return -level 2 -note body body",
        "return -level 0 -code 200 -note body body",
        "break",
        "continue",
    ] {
        for finally in [
            None,
            Some("list cleanup"),
            Some("error cleanup {} {APP CLEANUP}"),
            Some("return cleanup"),
            Some("return -level 0 -note cleanup cleanup"),
            Some("return -level 0 -code break -note cleanup cleanup"),
            Some("return -level 0 -code 201 -note cleanup cleanup"),
        ] {
            let command = format!(
                "try {}{}",
                quote_word(body),
                finally
                    .map(|body| format!(" finally {}", quote_word(body)))
                    .unwrap_or_default()
            );
            for procedure in [false, true] {
                let invocation = if procedure {
                    format!("proc f {{}} {{{command}}}; f")
                } else {
                    command.clone()
                };
                sources.push(format!(
                    "{OPTIONS}set c [catch {{{invocation}}} m o]; list $c $m [options $o]"
                ));
            }
        }
        for handler in [
            "on ok {m o} {list handler $m [options $o]}",
            "on error {m o} {error handler {} {APP HANDLER}}",
            "on return {m o} {list handler $m [options $o]}",
            "trap APP {} {return -level 0 -note handler handler}",
            "on break {} {return -level 0 -code continue -note handler handler}",
            "on 200 {m o} {list handler $m [options $o]}",
        ] {
            for finally in ["", " finally {error cleanup {} {APP CLEANUP}}"] {
                sources.push(format!("{OPTIONS}set c [catch {{try {} {handler}{finally}}} m o]; list $c $m [options $o]", quote_word(body)));
            }
        }
    }
    sources.push(format!("{OPTIONS}return -level 0 -note earlier earlier; set c [catch {{try {{list body}} on ok {{m o}} {{list $m [options $o]}}}} m o]; list $c $m [options $o]"));
    for control in ["break", "continue"] {
        let body = format!(
            "return -level 0 -code {control} -note kept -errorcode {{STALE CODE}} -errorinfo stale -errorstack {{INNER stale}} -errorline 99 result"
        );
        for invocation in [
            format!("proc f {{}} {}; f", quote_word(&body)),
            format!(
                "apply {}",
                quote_word(&format!("{{}} {}", quote_word(&body)))
            ),
        ] {
            sources.push(format!("{OPTIONS}set c [catch {{{invocation}}} m o]; list $c $m [options $o] [string match {{stale*}} [dict get $o -errorinfo]]"));
        }
    }
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn validation_order_binding_failures_and_error_context_match_tcl() {
    let mut sources = Vec::new();
    for form in [
        "try",
        "::try",
        "try {set ::entered yes} finally",
        "try {} finally {} on error {} {}",
        "try {} on",
        "try {} on error",
        "try {} on error {}",
        "try {} trap",
        "try {} trap APP",
        "try {} trap APP {}",
        "try {} on nonsense {} {}",
        "try {} on 4294967296 {} {}",
        "try {} on -2147483649 {} {}",
        "try {} on 2.0 {} {}",
        "try {} on {} {} {}",
        "try {} {} {}",
        "try {} unknown {}",
        "try {} on error {} -",
        "try {} on error {} - finally {}",
        r#"try {set ::entered yes} trap "bad \{list" {} {}"#,
        r#"try {set ::entered yes} on ok "bad \{list" {}"#,
        r#"try {set ::entered yes} on error {} {} on ok "bad \{list" {}"#,
        "set message(k) array; try {error body {} {APP BODY}} on error {message options} {error wrong} finally {set ::clean yes}",
        "set options(k) array; try {error body {} {APP BODY}} on error {message options} {error wrong} finally {set ::clean yes}",
        "try {error body} on error {::absent::m o} {error wrong} finally {set ::clean yes}",
    ] {
        sources.push(format!("{OPTIONS}set c [catch {{{form}}} m o]; list $c $m [options $o] [info exists ::entered] [info exists ::clean]"));
    }
    sources.extend([
        "set command try; set body {set x 1\nerror body {} {APP BODY}}; set c [catch {$command $body} m o]; list $c $m [dict get $o -errorcode] [string match {*\"try\" body line 2*} [dict get $o -errorinfo]]".into(),
        "set command ::try; set body {set x 1\nerror handler {} {APP HANDLER}}; set c [catch {$command {error body {} {APP BODY}} o error {} $body} m o]; list $c $m [dict get $o -errorcode] [dict get $o -during -errorcode] [string match {*\"::try ... o\" handler line 2*} [dict get $o -errorinfo]]".into(),
        "set command try; set body {set x 1\nerror cleanup {} {APP CLEANUP}}; set c [catch {$command {error body {} {APP BODY}} f $body} m o]; list $c $m [dict get $o -errorcode] [dict get $o -during -errorcode] [string match {*\"try ... finally\" body line 2*} [dict get $o -errorinfo]]".into(),
    ]);
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn only_executed_scripts_are_admitted_and_failures_still_run_cleanup() {
    compare(&[
        "try {list body} on error {} {interp eval {} {error wrong}} finally {set cleaned yes}",
        r"set invalid {bad \{script}; try {list body} on error {} $invalid",
    ]);
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        for source in [
            "try {set ::escaped yes; interp eval {} {error wrong}} on error {m o} {set ::handled yes} finally {set ::cleaned yes}; list [info exists ::escaped] $handled $cleaned",
            "set c [catch {try {error body} on error {} {set ::escaped yes; interp eval {} {error wrong}} finally {set ::cleaned yes}} m o]; list $c [info exists ::escaped] $cleaned [dict exists $o -during]",
        ] {
            let expected = if source.starts_with("try") {
                "0 yes yes"
            } else {
                "1 0 yes 1"
            };
            assert_eq!(
                run(source, runtime).unwrap(),
                expected,
                "{runtime:?}: {source}"
            );
        }
    }
}

#[test]
fn each_phase_retains_exact_source_breakpoints_and_caller_locals() {
    let body = "set marker 40\nincr marker\nset ::observed $marker";
    for clause in ["body", "on", "trap", "finally"] {
        let command = match clause {
            "body" => format!("try {} finally {{set cleaned yes}}", quote_word(body)),
            "on" => format!(
                "try {{error body}} on error {{m o}} {} finally {{set cleaned yes}}",
                quote_word(body)
            ),
            "trap" => format!(
                "try {{error body {{}} {{APP BODY}}}} trap APP {{m o}} {} finally {{set cleaned yes}}",
                quote_word(body)
            ),
            _ => format!("try {{list body}} finally {}", quote_word(body)),
        };
        for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            let source = format!("proc work {{}} {{set caller 42; {command}}}; work; set observed");
            let image =
                compile_source("/workspace/try-debug.tcl".into(), source, mode(runtime)).unwrap();
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
                        .any(|(name, value)| name == "marker" && value == "40")
                {
                    address = Some(at);
                    break;
                }
                assert!(
                    !matches!(state, DebugOutcome::Finished { .. }),
                    "{runtime:?} {clause}: phase never paused"
                );
                state = engine.step_instruction().unwrap();
            }
            let at = address.expect("mapped try phase");
            assert!(
                engine
                    .ui_frame_locals(0)
                    .iter()
                    .any(|(name, value)| name == "caller" && value == "42")
            );
            let source = engine
                .debug_sources()
                .into_iter()
                .find(|source| source.code_id == at.code_id)
                .unwrap();
            let breakpoint = CodeAddress {
                code_id: at.code_id,
                pc: source.image.units[source.unit_index]
                    .commands
                    .iter()
                    .find(|map| {
                        source.image.source.content[map.source_span.clone()]
                            .starts_with("set ::observed")
                    })
                    .unwrap()
                    .anchor_pc,
            };
            engine.add_breakpoint(breakpoint);
            if runtime == RuntimeMode::Hybrid {
                engine
                    .eval_in_frame(0, VmObj::from_str("set marker 80"))
                    .unwrap();
            }
            assert!(
                matches!(engine.continue_run().unwrap(), DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint))
            );
            engine.remove_breakpoint(breakpoint);
            match engine.continue_run().unwrap() {
                DebugOutcome::Finished { result } => assert_eq!(
                    result.as_string().unwrap(),
                    if runtime == RuntimeMode::Hybrid {
                        "81"
                    } else {
                        "41"
                    }
                ),
                other => panic!("{runtime:?} {clause}: {other:?}"),
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
            "try {} finally {{set cleaned yes}}",
            tcld_compile::quote_word(body)
        );
        let prefix = format!(
            "[list apply [list {{{parameters}}} {}]]",
            tcld_compile::quote_word(&callback)
        );
        let source = format!("{command} {prefix} {input}");
        for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            let image = compile_source(
                "/workspace/try-callback.tcl".into(),
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
            let at = address.expect("mapped try body inside callback lambda");
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
