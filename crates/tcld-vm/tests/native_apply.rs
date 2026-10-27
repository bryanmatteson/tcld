#![cfg(all(feature = "host", feature = "debug"))]
//! Lambda semantics and mapped callback debugging, compared with embedded Tcl.
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
    let image = compile_source("/workspace/apply.tcl".into(), source.into(), mode(runtime))
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
fn bindings_defaults_variadics_duplicate_and_literal_names_match_tcl() {
    compare(&[
        "apply {{} {expr {6*7}}}",
        "::apply {{x y} {expr {$x+$y}}} 20 22",
        "set command apply; set lambda {{x {y 2}} {expr {$x+$y}}}; $command $lambda 40",
        "set words {apply {{x {y 2}} {expr {$x+$y}}} 40 3}; {*}$words",
        "apply {{args} {list [llength $args] $args}} a {b c} d",
        "apply {{x args} {list $x $args}} 40",
        "apply {{x args} {list $x $args}} 40 1 2",
        "apply {{{x default} y} {list $x $y}} one two",
        "apply {{{args default}} {list $args}}",
        "apply {{{args default}} {list $args}} a b",
        "apply {{args x} {list $args $x}} one two",
        "apply {{x x} {set x}} first last",
        "apply {{x {x default}} {incr x; set x}} 40",
        "apply {{args args} {set args}} first last tail",
        "apply {{{{two words}}} {set {two words}}} value",
        "apply {{{a;b}} {set {a;b}}} {[error injected]$x}",
        "apply {{{a(b}} {set {a(b}}} value",
        "apply {{{a)b}} {set {a)b}}} value",
        "apply {{{λ 🙂}} {set λ}}",
        "set body [string map {@ 2} {expr {$x * @}}]; apply [list x $body] 21",
        "set body [string map {@ 世界} {list @ $x}]; apply [list x $body] λ",
        "proc closure {base} {list apply [list {base value} {expr {$base+$value}}] $base}; set add [closure 40]; {*}$add 2",
        "proc closure {base} {list apply [list {base args} {list $base $args}] $base}; set add [closure {a;[error injected]}]; {*}$add {b c}",
        "set f {apply {{x} {expr {$x+1}}}}; list [{*}$f 1] [{*}$f 40] [info commands ::__tcld_lambda_*]",
        "apply {{x} {set x 40; incr x; set x}} 0",
        "apply {{} {set a(k) 40; incr a(k) 2; set a(k)}}",
        "apply {{x} {apply {{y} {expr {$y+2}}} $x}} 40",
        "proc recursive {n} {if {$n == 0} {return done}; apply {{n} {recursive [expr {$n-1}]}} $n}; recursive 20",
    ]);
}

#[test]
fn namespaces_scope_links_and_invocation_levels_match_tcl() {
    compare(&[
        "proc outer {} {set secret hidden; apply {{} {info exists secret}}}; outer",
        "set x global; namespace eval n {set x local; apply {{} {global x; set x}}}",
        "namespace eval n {variable x 40; proc plus {v} {variable x; incr x $v}}; apply {{} {plus 2} ::n}",
        "namespace eval n {variable x 40}; apply {{} {variable x; incr x 2; list [namespace current] $x} n}",
        "namespace eval n {namespace eval child {}}; namespace eval child {variable x 42}; namespace eval n {apply {{} {variable x; list [namespace current] $x} child}}",
        "namespace eval n {apply {{} {namespace current}}}",
        "apply {{} {namespace current} {}}",
        "namespace eval n {}; apply {{} {namespace current} ::::n::::}",
        "proc outer {} {set x 40; apply {{} {upvar 1 x alias; incr alias 2}}; set x}; outer",
        "proc outer {} {set x 40; apply {{} {uplevel 1 {incr x 2}}}; set x}; outer",
        "apply {{x} {proc child {} {upvar 1 x alias; incr alias 2}; child; set x}} 40",
        "apply {{x} {list [info level] [info level 0]}} 42",
        "proc outer {} {apply {{x} {list [info level] [info level 0] [info level 1]}} 40}; outer",
        "namespace eval n {}; apply {{x} {info level 0} ::n} {two words}",
        "list [info commands apply] [info procs *lambda*]",
    ]);
}

#[test]
fn lambda_shapes_formals_namespaces_and_arity_errors_match_tcl() {
    let mut sources = Vec::new();
    for command in [
        "apply",
        "apply bad",
        "apply {a b c d}",
        "apply \"bad \\{list\"",
        "apply {{} {}} extra",
        "::apply {{} {}} extra",
        "apply {{x} {set x}}",
        "apply {{x} {set x}} a b",
        "apply {{x args} {}}",
        "apply {{{x default} y} {}} one",
        "apply {{{args default} y} {}}",
        "apply {{{{two words}}} {}}",
        "apply {{{a b c}} {}}",
        "apply {{{}} {}}",
        "apply {{{a(k)}} {}} 1",
        "apply {{{n::x}} {}} 1",
        "apply {{{a::b(k)}} {}} 1",
        "apply {{{a(b::c)}} {}} 1",
        "apply {{} {} ::missing}",
        "apply {{} {} {bad ns}}",
        "apply {{x} {} ::missing}",
        "apply {{{}} {} ::missing}",
        "apply [list \"\\{\" {}]",
        "apply [list [list \"\\{\"] {}]",
    ] {
        sources.push(format!("set code [catch {{{command}}} result options]; list $code $result [dict get $options -errorcode]"));
    }
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn returns_loop_boundaries_and_custom_options_match_tcl() {
    compare(&[
        "apply {{} {return found; error unreachable}}",
        "proc outer {} {apply {{} {return -level 2 found}}; error unreachable}; outer",
        "set c [catch {apply {{} {return -level 2 found}}} m o]; list $c $m [dict get $o -code] [dict get $o -level]",
        "set c [catch {apply {{} {error bad {} {APP FAIL}}}} m o]; list $c $m [dict get $o -errorcode] [string match {*lambda term*} [dict get $o -errorinfo]]",
        "set c [catch {apply {{{}} {}}} m o]; list $c $m [dict get $o -errorcode] [string match {*parsing lambda expression*} [dict get $o -errorinfo]]",
        "set c [catch {apply {{} {break}}} m o]; list $c $m [dict get $o -errorcode]",
        "set c [catch {apply {{} {continue}}} m o]; list $c $m [dict get $o -errorcode]",
        "set c [catch {foreach x {1 2} {apply {{} {break}}}} m o]; list $c $m [dict get $o -errorcode]",
        "set c [catch {apply {{} {return -level 0 -code break stopped}}} m o]; list $c $m [dict get $o -errorcode]",
        "set out {}; foreach x {1 2} {lappend out $x; apply {{} {return -code break stopped}}; lappend out bad}; set out",
        "set out {}; foreach x {1 2} {lappend out $x; apply {{} {return -code continue next}}; lappend out bad}; set out",
        "set c [catch {apply {{} {return -code 7 custom}}} m o]; list $c $m [dict get $o -code] [dict get $o -level]",
        "set c [catch {apply {{} {return -code 7 -note kept custom}}} m o]; list $c $m [dict get $o -code] [dict get $o -level] [dict get $o -note]",
        "set c [catch {apply {{} {return -code error -errorcode {APP FAIL} -note kept bad}}} m o]; list $c $m [dict get $o -errorcode] [dict get $o -note]",
        "set c [catch {apply {{} {return -options {-code 1 -level 0 -errorcode {APP RAW} -errorinfo detail -note kept} bad}}} m o]; list $c $m [dict get $o -errorcode] [dict get $o -note] [string match {*lambda term*} [dict get $o -errorinfo]]",
        "set c [catch {apply {{} {return -note kept result}}} m o]; list $c $m [dict get $o -code] [dict get $o -level] [dict get $o -note]",
        "set c [catch {apply {{} {return -level 0 -note kept result}}} m o]; list $c $m [dict get $o -code] [dict get $o -level] [dict get $o -note]",
        "set c [catch {apply {{} {return -level 0 -note kept result; set x later}}} m o]; list $c $m [dict get $o -note]",
        "set c [catch {apply {{} {set command return; $command -level 0 -note kept result}}} m o]; list $c $m [dict get $o -note]",
        "set c [catch {apply {{} {return -level 0 -note kept result; catch {set x reset}; set x}}} m o]; list $c $m [dict exists $o -note]",
        "set c [catch {catch {apply {{} {return -note kept result}}} m o; set m} outer options]; list $c $outer [dict exists $options -note]",
    ]);
}

#[test]
fn return_option_expansion_and_diagnostics_match_tcl() {
    let mut sources = Vec::new();
    for body in [
        "return -code 7 -options {-code 8 -note inner} -code 9 -note outer result",
        "return -options {-code 7 -options {-note nested -level 1}} result",
        "set flag -code; return $flag 7 -note computed result",
        "set words {-code 7 -note expanded}; return {*}$words result",
        "set command return; $command -code 7 -note computed-command result",
        "set command ::return; $command -options {-code 7 -note computed-options} result",
        "return -code return -level 0 found",
        "return -code return found",
        "return x y",
        "return -- result",
        "return -code bad result",
        "return -code e result",
        "return -code 2147483648 result",
        "return -code 4294967295 result",
        "return -code 4294967296 result",
        "return -level bad result",
        "return -level -1 result",
        "return -level 2147483648 result",
        "return -options odd result",
        r#"return -options "bad \{list" result"#,
        "return -options {-options odd} result",
        r#"return -errorcode "bad \{list" result"#,
        r#"return -errorstack "bad \{list" result"#,
        "return -errorstack odd result",
        "return -code 7 -errorstack {INNER value} result",
    ] {
        sources.push(format!(
            "set c [catch {{apply [list {{}} {}]}} m o]; list $c $m [dict get $o -code] [dict get $o -level] [expr {{[dict exists $o -errorcode] ? [dict get $o -errorcode] : {{}}}}] [expr {{[dict exists $o -note] ? [dict get $o -note] : {{}}}}]",
            tcld_compile::quote_word(body),
        ));
    }
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn anonymous_sort_and_regsub_callbacks_match_tcl() {
    compare(&[
        "lsort -command {apply {{a b} {expr {$a-$b}}}} {3 1 2}",
        "set calls {}; lsort -unique -decreasing -command {apply {{a b} {lappend ::calls [list $a $b]; expr {$a-$b}}}} {3 1 2 1 3}; set calls",
        "set body {string compare $a $b}; set prefix [list apply [list {a b} $body]]; lsort -indices -command $prefix {c a b}",
        "namespace eval n {variable direction -1}; lsort -command {apply {{a b} {variable direction; expr {($a-$b)*$direction}} ::n}} {3 1 2}",
        "lsort -stride 2 -index 1 -command {apply {{a b} {string compare $a $b}}} {X c Y a Z b}",
        "regsub -all -command . abc {apply {{m} {string toupper $m}}}",
        "regsub -command {(a)(b)} abc {apply {{all a b} {list $all $a $b}}}",
        "set prefix [list apply {{tag m} {string cat $tag $m}} {[$not_script]}]; regsub -all -command . ab $prefix",
        "lsort -command {apply {{a b} {regsub -command . x {apply {{m} {string toupper $m}}}; string compare $a $b}}} {c a b}",
        "set c [catch {lsort -command {apply {{a b} {error failed {} {LAMBDA SORT}}}} {b a}} m o]; list $c $m [dict get $o -errorcode] [string match {*lambda term*(-compare command)*} [dict get $o -errorinfo]]",
        "set c [catch {regsub -command . x {apply {{m} {error failed {} {LAMBDA REPLACE}}}}} m o]; list $c $m [dict get $o -errorcode] [string match {*lambda term*(-command substitution computation script)*} [dict get $o -errorinfo]]",
    ]);
}

#[test]
fn lambda_source_admission_precedes_execution() {
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let source = "set c [catch {apply {{} {set ::before yes; interp eval {} {set ::escaped yes}}}} m]; list $c [info exists ::before] [info exists ::escaped]";
        assert_eq!(run(source, runtime).unwrap(), "1 0 0", "{runtime:?}");
    }
    compare(&["if {0} {apply {{} {interp eval {} {error unreachable}}}}; set result ok"]);
}

#[test]
fn lambda_callback_breakpoints_locals_and_edits_survive_repeated_calls() {
    let body = "set result [string compare $a $b]\nset marker [list $a $b]\nreturn $result";
    let source = format!(
        "lsort -command [list apply [list {{a b}} {}]] {{c b a}}",
        tcld_compile::quote_word(body)
    );
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let image = compile_source(
            "/workspace/lambda-sort.tcl".into(),
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
                    .any(|(name, value)| name == "marker" && value == "c b")
            {
                address = Some(at);
                break;
            }
            assert!(
                !matches!(state, DebugOutcome::Finished { .. }),
                "lambda never paused: {runtime:?}"
            );
            state = engine.step_instruction().unwrap();
        }
        let address = address.expect("mapped comparator lambda paused with bound arguments");
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
        let pc = mapped.image.units[mapped.unit_index]
            .commands
            .iter()
            .find(|command| {
                mapped.image.source.content[command.source_span.clone()].starts_with("return")
            })
            .unwrap()
            .anchor_pc;
        let breakpoint = CodeAddress {
            code_id: address.code_id,
            pc,
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
        assert!(matches!(
            engine.continue_run().unwrap(),
            DebugOutcome::Paused(_)
        ));
        assert_eq!(engine.pause_snapshot().unwrap().address, Some(breakpoint),);
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
