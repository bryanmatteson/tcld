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
        "/workspace/dict.tcl".into(),
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
fn native_dict_iteration_binding_writeback_traces_completions_and_coroutines() {
    let mut failures = Vec::new();
    for source in [
        r#"dict for {k v} {a 1 b 2} {lappend out $k $v}; set out"#,
        r#"dict map {k v} {a 1 b 2} {expr {$v*2}}"#,
        r#"dict map {k v} {a 1 b 2} {set k new$k; expr {$v*2}}"#,
        r#"set out {}; dict for {k v} {} {error unreachable}; list $out [info exists k]"#,
        r#"dict map {k v} {} {error unreachable}"#,
        r#"set out {}; dict for {k v} {a 1 b 2 c 3} {if {$v==2} {continue}; lappend out $k}; set out"#,
        r#"dict map {k v} {a 1 b 2 c 3} {if {$v==2} {continue}; expr {$v*2}}"#,
        r#"set out {}; dict for {k v} {a 1 b 2} {lappend out $k; break}; set out"#,
        r#"dict map {k v} {a 1 b 2} {if {$v==2} {break}; set v}"#,
        r#"set d {a 1 b 2}; dict for {k v} $d {dict set d c 3}; set d"#,
        r#"proc work {} {set k old; set v old; dict for {k v} {a 40} {incr v 2}; list $k $v}; work"#,
        r#"proc work {} {set d {a 40}; dict update d a value {incr value 2}; list $d $value}; work"#,
        r#"set d {a 40 b 2}; dict update d a x b y {incr x $y}; list $d $x $y"#,
        r#"set d {a 40}; dict update d missing x {set x 42}; set d"#,
        r#"set x before; set d {a 40}; dict update d missing x {info exists x}; list $d [info exists x]"#,
        r#"set d {a 40}; dict update d a x {unset x}; set d"#,
        r#"set d {a 40}; dict update d a x {unset d; incr x}; list [info exists d] $x"#,
        r#"set d {a 40}; dict update d a x {dict set d b 2; incr x}; set d"#,
        r#"set d {a 40}; set c [catch {dict update d a x {incr x; error bad {} {DICT FAIL}}} m o]; list $c $m [dict get $o -errorcode] $d"#,
        r#"proc f {} {set d {a 40}; set r [catch {dict update d a x {incr x; return hello}} m o]; list $r $m [dict get $o -level] $d}; f"#,
        r#"set d {a 1}; set c [catch {dict update d a x {break}} m]; list $c $m $d"#,
        r#"set d {a 1}; set c [catch {dict update d a x {continue}} m]; list $c $m $d"#,
        r#"proc f {} {set d {a 40}; dict with d {incr a 2}; list $d $a}; f"#,
        r#"set d {nested {a 40 b 2} other 3}; dict with d nested {incr a $b}; list $d $a $b"#,
        r#"set d {a 40 b 2}; dict with d {unset a; set c 99; incr b}; list $d [info exists a] $b $c"#,
        r#"set d {a 40}; dict with d {unset d; incr a}; list [info exists d] $a"#,
        r#"set d {a 40}; dict with d {dict set d b 2; incr a}; set d"#,
        r#"set d {nested {a 40}}; dict with d nested {dict unset d nested; incr a}; set d"#,
        r#"set d {a 40}; set c [catch {dict with d {incr a; error bad {} {DICT FAIL}}} m o]; list $c $m [dict get $o -errorcode] $d"#,
        r#"proc f {} {set d {a 40}; set c [catch {dict with d {incr a; return done}} m o]; list $c $m [dict get $o -level] $d}; f"#,
        r#"set d {a 1}; list [catch {dict with d {break}} m] $m $d"#,
        r#"set d {a 1}; list [catch {dict with d {continue}} m] $m $d"#,
        r#"proc work {} {set d {a 40}; set a 99; dict with d {incr a 2}; list $a $d}; work"#,
        r#"proc work {} {set d {a 40}; set a 99; dict with d {info locals}}; lsort [work]"#,
        r#"proc work {} {set a 99; set d {}; dict with d {incr a}; list $a $d}; work"#,
        r#"proc work {} {set a 40; set d {a 1}; dict with d {incr a}; list $a $d}; work"#,
        r#"proc work {} {set d {a 40}; set b [format {incr a %d} 2]; dict with d $b; set d}; work"#,
        r#"set sub for; set body [format {lappend out $k [expr {$v+%d}]} 2]; set out {}; dict $sub {k v} {a 40} $body; set out"#,
        r#"set sub map; set body [format {expr {$v+%d}} 2]; dict $sub {k v} {a 40} $body"#,
        r#"set sub update; set d {a 40}; set body [format {incr a %d} 2]; dict $sub d a a $body; set d"#,
        r#"set sub with; set d {a 40}; set body [format {incr a %d} 2]; dict $sub d $body; set d"#,
        r#"proc work {} {dict for {k v} {a 40 b 41} {yield $v; incr v}; list $k $v}; list [coroutine co work] [co] [co]"#,
        r#"proc work {} {dict map {k v} {a 40 b 41} {yield $v; incr v}}; list [coroutine co work] [co] [co]"#,
        r#"proc work {} {set d {a 40}; dict update d a v {yield $v; incr v 2}; set d}; list [coroutine co work] [co]"#,
        r#"proc work {} {set d {a 40}; dict with d {yield $a; incr a 2}; set d}; list [coroutine co work] [co]"#,
        r#"proc work {} {set d {nested {a 40}}; dict with d nested {yield $a; incr a 2}; set d}; list [coroutine co work] [co]"#,
        r#"set d {a 40}; set body {yield ready; incr a 2}; coroutine co dict with d $body; co; set d"#,
        r#"set d {a 40}; coroutine co dict update d a v {yield ready; incr v 2}; set d {a 90 b 2}; co; set d"#,
        r#"set d {a 40}; coroutine co dict with d {yield ready; incr a 2}; set d {a 90 b 2}; co; set d"#,
        r#"proc work {n} {set d [list a $n]; dict with d {yield $a; incr a}; set d}; list [coroutine a work 10] [coroutine b work 40] [a] [b]"#,
        r#"set d {a 40}; coroutine co dict with d {yield ready; incr a}; rename co {}; list $d $a"#,
        r#"set d {a 40}; coroutine co dict update d a v {yield ready; incr v}; rename co {}; list $d $v"#,
        r#"namespace eval n {set d {a 40}; dict with d {incr a 2}; list $d $a}"#,
        r#"namespace eval n {proc work {} {set d {a 40}; dict with d {incr a 2}; list [namespace current] $d}}; n::work"#,
        r#"set d {a(k) 40}; proc work {} {global d; dict with d {incr a(k) 2}; list $d [array get a]}; work"#,
        r#"proc work {} {set a(k) old; set d {a 40}; list [catch {dict with d {error unreachable}} m] $m [array get a]}; work"#,
        r#"proc work {} {set x 0; set d {x(k) 40}; list [catch {dict with d {error unreachable}} m] $m $x}; work"#,
        r#"proc work {} {set a(k) {old 40}; dict update a(k) missing v {set v 42}; set a(k)}; work"#,
        r#"proc outer {} {set d {a 40}; proc inner {} {upvar 1 d alias; dict with alias {incr a 2}}; inner; set d}; outer"#,
        r#"proc outer {} {set x 40; proc inner {} {upvar 1 x a; set d {a 41}; dict with d {incr a}; list $a $d}; list [inner] $x}; outer"#,
        r#"proc outer {} {set a(k) 40; proc inner {} {upvar 1 a(k) x; set d {x 41}; dict with d {incr x}; list $x $d}; list [inner] [array get a]}; outer"#,
        r#"set out {}; proc traced {n e op} {lappend ::out [list $n $e $op]}; proc work {} {set a 0; set d {a 40}; trace add variable a write traced; [format %s dict] with d {incr a 2}; list $a $d}; list [work] [join $out |]"#,
        r#"set out {}; proc traced {n e op} {lappend ::out $op}; proc work {} {set d {a 40}; trace add variable d {read write} traced; [format %s dict] with d {incr a}; set d}; list [work] [join $out |]"#,
        r#"set out {}; proc traced {n e op} {upvar 1 $n d; set d {a 40}; lappend ::out $op}; proc work {} {set d {old 0}; trace add variable d read traced; [format %s dict] with d {incr a 2}; set d}; list [work] [join $out |]"#,
        r#"set out {}; proc traced {n e op} {upvar 1 $n v; incr v; lappend ::out $op}; proc work {} {set x 0; set d {a 40}; trace add variable x write traced; [format %s dict] update d a x {incr x}; list $d $x}; list [work] [join $out |]"#,
        r#"proc target {} {return done}; set d {a 40}; list [catch {dict with d {incr a; tailcall target}} m] $m $d"#,
        r#"proc target {} {yield target; return done}; proc work {} {set d {a 40}; try {dict update d a v {incr v; tailcall target}} finally {set ::saved $d}}; list [coroutine co work] [co] $saved"#,
        r#"set d {a 40}; dict for {k v} $d {dict with d {incr a 2}}; set d"#,
        r#"set d {a 40}; dict with d {dict update d a v {incr v}; incr a 2}; set d"#,
        r#"proc work {} {set d {a 40}; set c [catch {dict with d {set d invalid; incr a}} m]; list $c $m}; work"#,
        r#"list [catch {dict for} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict for {k} {a 1} {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict for {k v z} {a 1} {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict for {k v} bad {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict map {k} {a 1} {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict map {k v} bad {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict update} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {[format %s dict] update missing a v {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {set d bad; dict update d a v {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {set d {a 1}; dict update d a v extra {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {dict with} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {[format %s dict] with missing {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {set d bad; dict with d {}} m o] $m [dict get $o -errorcode]"#,
        r#"list [catch {set d {a 1}; dict with d missing {}} m o] $m [dict get $o -errorcode]"#,
        r#"dict fo {k v} {a 40} {incr v 2}; set v"#,
        r#"dict ma {k v} {a 40} {incr v 2}"#,
        r#"set sub up; set d {a 40}; dict $sub d a x {incr x 2}; set d"#,
        r#"set sub w; set d {a 40}; dict $sub d {incr a 2}; set d"#,
        r#"set d {a 40}; dict for {k v} {a 1} {dict se d a 42}; set d"#,
        r#"proc work {} {set d {a 40}; dict with d {list [info exists a] [info exists missing]}}; work"#,
        r#"set d {a 40}; ::tcl::dict::with d {incr a 2}; set d"#,
        r#"::tcl::dict::for {k v} {a 40} {incr v 2}; set v"#,
        r#"proc work {} {dict set d a 40; dict incr d a 2; set d}; work"#,
        r#"list [catch {::tcl::dict::fo {k v} {a 1} {set ::escaped yes}} m] [info exists ::escaped]"#,
    ] {
        if std::panic::catch_unwind(|| compare(source)).is_err() {
            failures.push(source);
        }
    }
    assert!(failures.is_empty(), "failed programs: {failures:#?}");
}
#[test]
fn generated_dict_bodies_survive_yield_and_live_edits() {
    for command in [
        "dict for {key value} {a 1} $body",
        "dict map {key value} {a 1} $body",
        "dict update d a marker $body",
        "dict with d $body",
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
fn selected_dict_bodies_cannot_bypass_admission() {
    for command in [
        "dict for {k v} {a 1} $body",
        "dict map {k v} {a 1} $body",
        "dict update d a v $body",
        "dict with d $body",
    ] {
        let source = format!(
            "set body {{interp eval {{}} {{set ::escaped yes}}}}; set d {{a 1}}; list [catch {{{command}}} m] [info exists ::escaped]"
        );
        let image = compile_source(
            "/workspace/admission.tcl".into(),
            source,
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
}

#[test]
fn abbreviated_filter_still_cannot_bypass_script_admission() {
    let source = "set sub fi; list [catch {dict $sub {a 1} script {k v} {interp eval {} {set ::escaped yes}}} m] [info exists ::escaped]";
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
