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
        "/workspace/dict-filter.tcl".into(),
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
fn filter_matches_native_tcl_predicates_bindings_and_completions() {
    let mut failures = Vec::new();
    for source in [
        r#"dict filter {a 1 b 2 c 3} script {k v} {expr {$v%2}}"#,
        r#"dict filter {a 1 b 2} script {k v} {set k changed; set v 99; expr {1}}"#,
        r#"set d {a 1 b 2}; set result [dict filter $d script {k v} {dict set d c 3; expr {1}}]; list $result $d $k $v"#,
        r#"list [dict filter {} script {k v} {error unreachable}] [info exists k] [info exists v]"#,
        r#"dict filter {a 1 b 2 c 3 d 4} script {k v} {if {$v==2} {continue}; if {$v==4} {break}; expr {true}}"#,
        r#"set d {a 1 b 2}; list [dict filter $d script {k v} {break}] $k $v"#,
        r#"list [catch {dict filter {a 1 b 2} script {k v} {set k}} m o] $m [dict get $o -errorcode]"#,
        r#"proc f {} {dict filter {a 1} script {k v} {return done}; error unreachable}; f"#,
        r#"proc f {} {set c [catch {dict filter {a 1} script {k v} {return -code return -level 2 done}} m o]; list $c $m [dict get $o -code] [dict get $o -level]}; f"#,
        r#"list [catch {dict filter {a 1} script {k v} {error bad {} {FILTER FAIL}}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter {a 1} script {k v} {return -code 7 custom}} m o] $m [dict get $o -code]"#,
        r#"list [catch {dict filter {a 1} script {k v} {set v invalid}} m o] $m [dict get $o -errorcode]"#,
        r#"proc work {} {set k old; set v 0; set r [dict filter {a 40 b 42} script {k v} {incr v; expr {$v>42}}]; list $r $k $v}; work"#,
        r#"proc outer {} {set x old; proc inner {} {upvar 1 x k; dict filter {a 1} script {k v} {expr {true}}}; list [inner] $x}; outer"#,
        r#"proc work {} {set a(k) 0; set a(v) 0; list [dict filter {a 40} script {a(k) a(v)} {incr a(v) 2; expr {true}}] [lsort [array get a]]}; work"#,
        r#"namespace eval n {set r [dict filter {a 1} script {k v} {list [namespace current]; expr {$v}}]; list $r $k $v}"#,
        r#"set sub fi; set mode s; set body [format {expr {$v > %d}} 1]; dict $sub {a 1 b 2} $mode {k v} $body"#,
        r#"::tcl::dict::filter {a 1 b 2} script {k v} {expr {$v>1}}"#,
        r#"dict filter {a 1 b 2} key a"#,
        r#"dict filter {a 1 b 2 c 3} k b a"#,
        r#"dict filter {a 1 b 2} value 2"#,
        r#"dict filter {a 1 b 2} v"#,
        r#"list [catch {dict filter} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter {a 1} bogus} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter {a 1} {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter {a 1} s} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter {a 1} script {k v} {} extra} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter {a 1} script {k} {error unreachable}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter {a 1} script {k v z} {error unreachable}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict filter invalid script {k v} {error unreachable}} m o] $m [dict get $o -errorcode]"#,
        r#"set out {}; proc traced {n e op} {lappend ::out [list $n $e $op]}; proc work {} {set k old; set v 0; trace add variable k write traced; trace add variable v write traced; dict filter {a 40 b 41} script {k v} {expr {$v>40}}}; list [work] [join $out |]"#,
        r#"proc traced {n e op} {upvar 1 $n x; incr x}; proc work {} {set v 0; trace add variable v write traced; list [dict filter {a 40} script {k v} {expr {$v==41}}] $v}; work"#,
        r#"proc traced {args} {error denied}; proc work {} {set v 0; trace add variable v write traced; list [catch {dict filter {a 1} script {k v} {error unreachable}} m o] $m [dict get $o -errorcode]}; work"#,
        r#"set k 0; list [catch {dict filter {a 1} script {k(x) v} {error unreachable}} m o] $m [dict get $o -errorcode]"#,
        r#"dict filter {a 1 b 2} script {k v} {dict filter {x 40 y 42} script {a b} {expr {$b==42}}; expr {$v==2}}"#,
        r#"proc target {} {return done}; proc work {} {dict filter {a 1} script {k v} {tailcall target}}; work"#,
    ] {
        if std::panic::catch_unwind(|| compare(source)).is_err() {
            failures.push(source);
        }
    }
    assert!(failures.is_empty(), "failed programs: {failures:#?}");
}
#[test]
fn filter_coroutines_suspend_resume_and_release_native_searches() {
    // Stock Tcl's synchronous filter cannot yield. These expectations describe
    // the NRE extension, while ordinary predicates are compared to unmodified Tcl.
    for (source, expected) in [
        (
            r#"proc work {} {dict filter {a 1 b 2} script {k v} {yield $k; expr {$v>1}}}; list [coroutine co work] [co] [co]"#,
            "a b {b 2}",
        ),
        (
            r#"proc work {} {dict filter {a 1 b 2} script {k v} {yieldto list $k; expr {$v>1}}}; list [coroutine co work] [co] [co]"#,
            "a b {b 2}",
        ),
        (
            r#"proc work {n} {dict filter [list a $n] script {k v} {yield $v; expr {$v>20}}}; list [coroutine a work 10] [coroutine b work 40] [a] [b]"#,
            "10 40 {} {a 40}",
        ),
        (
            r#"set d {a 1 b 2}; coroutine co dict filter $d script {k v} {yield ready; expr {true}}; set d {c 3}; rename co {}; list $d [info commands co]; dict filter {x 42} script {k v} {expr {$v==42}}"#,
            "x 42",
        ),
        (
            r#"proc work {} {try {dict filter {a 1} script {k v} {yield ready; error bad {} {FILTER FAIL}}} trap FILTER {m o} {list $m [dict get $o -errorcode]} finally {set ::cleaned yes}}; list [coroutine co work] [co] $cleaned"#,
            "ready {bad {FILTER FAIL}} yes",
        ),
        (
            r#"proc target {} {yield target; return done}; proc work {} {try {dict filter {a 1} script {k v} {yield ready; tailcall target}} finally {set ::cleaned yes}}; list [coroutine co work] [co] [co] $cleaned"#,
            "ready target done yes",
        ),
    ] {
        let image = compile_source(
            "/workspace/filter-yield.tcl".into(),
            source.into(),
            CompileMode::Hybrid,
        )
        .unwrap();
        let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
        e.bootstrap(Some("tcld")).unwrap();
        match e.debug_start_image(&image, false).unwrap().outcome {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), expected, "{source}")
            }
            o => panic!("{source}: {o:?}"),
        }
    }
}
#[test]
fn generated_filter_predicates_survive_yield_and_live_edits() {
    for command in [
        "dict filter {a 1} script {key value} $body",
        "dict fi {a 1} s {key value} $body",
        "::tcl::dict::filter {a 1} script {key value} $body",
    ] {
        let body = "yield ready\nset marker 40\nincr marker\nset ::result $marker";
        let source = format!(
            "proc work {{}} {{set d {{marker 40 a 40}}; set body {}; {}; if {{[dict exists $d marker]}} {{set ::saved [dict get $d marker]}} }}; coroutine co work; co; set ::result",
            tcld_compile::quote_word(body),
            command
        );
        let image = compile_source(
            "/workspace/generated-dict.tcl".into(),
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
                        && s.image.units[s.unit_index]
                            .commands
                            .iter()
                            .any(|m| m.start_line >= 3 && m.pc_range.contains(&a.pc))
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
                "dict body never paused"
            );
            state = e.step_instruction().unwrap();
        }
        let a =
            address.expect("exact generated dict body paused after native coroutine resumption");
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
                .any(|(n, v)| n == "marker" && v == "81"),
            "{command}: {:?}",
            e.ui_frame_locals(0)
        );
        e.remove_breakpoint(bp);
        match e.continue_run().unwrap() {
            DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "81"),
            o => panic!("{o:?}"),
        }
    }
}

#[test]
fn editing_filter_predicates_changes_selection_but_keeps_original_pairs() {
    let body = "yield ready\nexpr {$value > 1}";
    let source = format!(
        "proc work {{}} {{set body {}; dict filter {{a 1 b 2}} script {{key value}} $body}}; coroutine co work; co; co",
        tcld_compile::quote_word(body)
    );
    let image = compile_source(
        "/workspace/filter-selection.tcl".into(),
        source,
        CompileMode::Hybrid,
    )
    .unwrap();
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    let mut state = e.debug_start_image(&image, true).unwrap().outcome;
    let mut breakpoint = None;
    for _ in 0..500 {
        if let DebugOutcome::Paused(p) = &state
            && let Some(a) = p.address
            && e.debug_sources().iter().any(|s| {
                s.code_id == a.code_id
                    && s.origin.is_some()
                    && s.image.source.content.as_ref() == body
                    && s.image.units[s.unit_index]
                        .commands
                        .iter()
                        .any(|m| m.start_line == 2 && m.pc_range.contains(&a.pc))
            })
            && e.ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "value" && v == "1")
        {
            breakpoint = Some(a);
            break;
        }
        assert!(!matches!(state, DebugOutcome::Finished { .. }));
        state = e.step_instruction().unwrap();
    }
    let bp = breakpoint.expect("generated predicate pauses after native yield");
    e.add_breakpoint(bp);
    e.eval_in_frame(0, tcld_vm::VmObj::from_str("set value 42"))
        .unwrap();
    assert!(matches!(e.continue_run().unwrap(), DebugOutcome::Paused(_)));
    assert!(
        e.ui_frame_locals(0)
            .iter()
            .any(|(n, v)| n == "key" && v == "b")
    );
    assert!(
        e.ui_frame_locals(0)
            .iter()
            .any(|(n, v)| n == "value" && v == "2")
    );
    e.eval_in_frame(0, tcld_vm::VmObj::from_str("set value 0"))
        .unwrap();
    e.remove_breakpoint(bp);
    match e.continue_run().unwrap() {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "a 1"),
        o => panic!("{o:?}"),
    }
}
