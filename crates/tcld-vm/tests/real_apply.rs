#![cfg(all(feature = "host", feature = "debug"))]
use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{DebugOutcome, HybridEngine, RuntimeMode};
fn compare(source: &str) {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    let oracle = tcld_interp::Interpreter::new().unwrap();
    let expected = oracle
        .eval(source)
        .unwrap_or_else(|e| panic!("oracle {source}: {e}"));
    let image = compile_source(
        "/workspace/apply.tcl".into(),
        source.into(),
        CompileMode::Hybrid,
    )
    .unwrap_or_else(|e| panic!("{source}: {e}"));
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    match e
        .debug_start_image(&image, false)
        .unwrap_or_else(|e| panic!("{source}: {e}"))
        .outcome
    {
        DebugOutcome::Finished { result } => {
            assert_eq!(result.as_string().unwrap(), expected, "{source}")
        }
        o => panic!("{source}: {o:?}"),
    }
}
#[test]
fn native_apply_bindings_namespaces_closures_completions_and_coroutines() {
    let mut failures = Vec::new();
    for source in [
        r#"apply {{} {expr {6*7}}}"#,
        r#"apply {{x y} {expr {$x+$y}}} 20 22"#,
        r#"apply {{x {y 2}} {expr {$x+$y}}} 40"#,
        r#"apply {{x {y 2}} {expr {$x+$y}}} 40 3"#,
        r#"apply {{args} {list [llength $args] $args}} a {b c} d"#,
        r#"apply {{x args} {list $x $args}} 40"#,
        r#"apply {{x args} {list $x $args}} 40 1 2"#,
        r#"set body [format {expr {$x * %d}} 2]; set lambda [list x $body]; apply $lambda 21"#,
        r#"set body [format {list %s $x} 世界]; apply [list x $body] λ"#,
        r#"proc closure {base} {list apply [list {base value} {expr {$base+$value}}] $base}; set add [closure 40]; {*}$add 2"#,
        r#"proc closure {base} {list apply [list {base {value 2}} {expr {$base+$value}}] $base}; set add [closure 40]; {*}$add"#,
        r#"proc closure {base} {list apply [list {base args} {list $base $args}] $base}; set f [closure {captured value}]; {*}$f a b"#,
        r#"proc outer {} {set secret hidden; apply {{} {info exists secret}}}; outer"#,
        r#"set x global; namespace eval n {set x local; apply {{} {global x; set x}}}"#,
        r#"namespace eval n {variable x 40; proc plus {v} {variable x; incr x $v}}; apply {{} {plus 2} ::n}"#,
        r#"namespace eval n {variable x 40}; apply {{} {variable x; incr x 2; list [namespace current] $x} n}"#,
        r#"namespace eval n {variable x 40; set f [namespace code [list apply [list v {variable x; incr x $v} ::n]]]}; {*}$n::f 2"#,
        r#"namespace eval n {set f [list apply [list x {list [namespace current] $x} ::n]]}; {*}$n::f 42"#,
        r#"set f [list apply [list x {expr {$x+1}}]]; list [{*}$f 1] [{*}$f 40] [info commands ::__tcld_lambda_*]"#,
        r#"apply {{x} {set x 40; incr x; set x}} 0"#,
        r#"apply {{} {set a(k) 40; incr a(k) 2; set a(k)}}"#,
        r#"proc outer {} {set x 40; apply {{} {upvar 1 x alias; incr alias 2}}; set x}; outer"#,
        r#"proc outer {} {set x 40; apply {{} {uplevel 1 {incr x 2}}}; set x}; outer"#,
        r#"apply {{x} {proc child {} {upvar 1 x alias; incr alias 2}; child; set x}} 40"#,
        r#"apply {{} {list [info level] [info locals]}}"#,
        r#"proc outer {} {apply {{x} {list [info level] [info locals]}} 40}; outer"#,
        r#"apply {{} {return found; error unreachable}}"#,
        r#"proc outer {} {apply {{} {return -level 2 found}}; error unreachable}; outer"#,
        r#"set c [catch {apply {{} {error bad {} {APP FAIL}}}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {{} {break}}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {{} {continue}}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set out {}; proc traced {n e op} {upvar 1 $n v; incr v; lappend ::out $n}; set r [apply {{x} {trace add variable x write traced; incr x; set x}} 40]; list $r $out"#,
        r#"set out {}; proc traced {n e op} {lappend ::out $op}; apply {{x} {trace add variable x unset traced; return done}} 40; set out"#,
        r#"set out {}; proc traced {n e op} {lappend ::out $op}; coroutine co apply {{x} {trace add variable x unset traced; yield ready; set x}} 40; rename co {}; set out"#,
        r#"list [coroutine co apply {{x} {set v [yield ready]; list $x $v}} 40] [co resumed] [info commands co]"#,
        r#"set body [format {yield ready; incr x %d; return $x} 2]; list [coroutine co apply [list x $body] 40] [co]"#,
        r#"namespace eval n {variable x 40}; list [coroutine co apply {{} {variable x; yield [namespace current]; incr x 2} ::n}] [co] $n::x"#,
        r#"list [coroutine a apply {{x} {yield $x; incr x}} 10] [coroutine b apply {{x} {yield $x; incr x}} 40] [a] [b]"#,
        r#"proc target {x} {expr {$x+2}}; apply {{x} {tailcall target $x}} 40"#,
        r#"proc target {x} {yield target; expr {$x+2}}; list [coroutine co apply {{x} {tailcall target $x}} 40] [co]"#,
        r#"proc outer {x} {tailcall apply {{x} {yield ready; incr x 2}} $x}; list [coroutine co outer 40] [co]"#,
        r#"proc target {x} {yield target; expr {$x+2}}; list [coroutine co apply {{x} {try {tailcall target $x} finally {yield cleanup}}} 40] [co] [co]"#,
        r#"proc target {x} {list $x [info coroutine]}; list [coroutine co apply {{x} {yieldto target $x; incr x 2}} 40] [co]"#,
        r#"apply {{x} {apply {{y} {expr {$y+2}}} $x}} 40"#,
        r#"set out {}; proc cb {cmd args} {lappend ::out [join $args |]}; proc f {} {apply {{x} {incr x}} 40}; trace add execution f {enterstep leavestep} cb; list [f] [llength $out]"#,
        r#"apply {{} {subst {answer[expr {6*7}]}}}"#,
        r#"apply {{x} {lmap y {1 2} {expr {$x+$y}}}} 40"#,
        r#"apply {{x} {set t [time {incr x} 2]; set x}} 40"#,
        r#"apply {{} {set x 1; eval {incr x}; set x}}"#,
        r#"list [coroutine co apply {{} {try {yield body} finally {yield cleanup}; return done}}] [co] [co]"#,
        r#"set c [catch {apply} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply bad} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {a b c d}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {x {set x}}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {{} {}} extra} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {{{a b c}} {}}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {{} {} ::missing}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {{} {} {bad ns}}} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"set c [catch {apply {{{a(k)}} {}} 1} m o]; list $c $m [dict get $o -errorcode]"#,
        r#"apply {{x} {info level 0}} 42"#,
        r#"namespace eval n {}; apply {{x} {info level 0} ::n} {two words}"#,
        r#"proc target {} {return wrong}; apply {{} {try {tailcall target} finally {return override}}}"#,
        r#"proc target {} {return wrong}; set c [catch {apply {{} {try {tailcall target} finally {error cleanup}}}} m]; list $c $m"#,
        r#"namespace eval n {}; coroutine co apply {{x} {yield ready; list [namespace current] $x} ::n} 42; namespace delete ::n; co"#,
        r#"proc f {n} {if {$n == 0} {return done}; tailcall apply {{n} {tailcall f [expr {$n-1}]}} $n}; f 1000"#,
        r#"list [coroutine a apply {{} {yield A; yield A2; return A3}}] [coroutine b apply {{} {yield B; yield B2; return B3}}] [a] [b] [a] [b]"#,
        r#"apply {{} {set f [list apply [list x {expr {$x*2}}]]; {*}$f 21}}"#,
    ] {
        if std::panic::catch_unwind(|| compare(source)).is_err() {
            failures.push(source);
        }
    }
    assert!(failures.is_empty(), "failed programs: {failures:#?}");
}
#[test]
fn generated_lambda_source_survives_yield_and_live_edits() {
    let body = "yield ready\nset marker $captured\nincr marker\nset ::result $marker";
    let source = format!(
        "namespace eval n {{variable unused 0}}; proc closure {{captured}} {{list apply [list captured {} ::n] $captured}}; set f [closure 40]; coroutine co {{*}}$f; co; set ::result",
        tcld_compile::quote_word(body)
    );
    let image = compile_source(
        "/workspace/generated-apply.tcl".into(),
        source,
        CompileMode::Hybrid,
    )
    .unwrap();
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    let mut state = e.debug_start_image(&image, true).unwrap().outcome;
    let mut address = None;
    for _ in 0..500 {
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
            "lambda never paused"
        );
        state = e.step_instruction().unwrap();
    }
    let a = address.expect("exact generated lambda body paused after native coroutine resumption");
    assert!(
        e.ui_frame_locals(0)
            .iter()
            .any(|(n, v)| n == "captured" && v == "40")
    );
    let src = e
        .debug_sources()
        .into_iter()
        .find(|s| s.code_id == a.code_id)
        .unwrap();
    let pc = src.image.units[src.unit_index]
        .commands
        .iter()
        .find(|m| src.image.source.content[m.source_span.clone()].starts_with("set ::result"))
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
#[test]
fn selected_lambda_keeps_source_admission_guard() {
    let source =
        "list [catch {apply {{} {interp eval {} {set ::escaped yes}}}} m] [info exists ::escaped]";
    let image = compile_source(
        "/workspace/admission.tcl".into(),
        source.into(),
        CompileMode::Hybrid,
    )
    .unwrap();
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    match e.debug_start_image(&image, false).unwrap().outcome {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "1 0"),
        o => panic!("{o:?}"),
    }
}
