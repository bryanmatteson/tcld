#![cfg(all(feature = "host", feature = "debug"))]
use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{CodeAddress, DebugOutcome, HybridEngine, RuntimeMode, VmObj};
fn engine() -> HybridEngine {
    let mut e = HybridEngine::new(RuntimeMode::Hybrid).unwrap();
    e.bootstrap(Some("tcld")).unwrap();
    e
}
fn image(s: &str) -> tcld_compile::DebugImage {
    compile_source(
        "/workspace/callbacks.tcl".into(),
        s.into(),
        CompileMode::Hybrid,
    )
    .unwrap_or_else(|e| panic!("{s}: {e}"))
}
fn compare(s: &str) {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    let expected = tcld_interp::Interpreter::new()
        .unwrap()
        .eval(s)
        .unwrap_or_else(|e| panic!("oracle {s}: {e}"));
    let mut e = engine();
    let out = e
        .debug_start_image(&image(s), false)
        .unwrap_or_else(|e| panic!("{s}: {e}"));
    match out.outcome {
        DebugOutcome::Finished { result } => {
            let normalize = |v: &str| {
                tcld_interp::Interpreter::new()
                    .unwrap()
                    .eval(&format!("list {{*}}{}", tcld_compile::quote_word(v)))
                    .unwrap_or_else(|_| v.into())
            };
            assert_eq!(
                normalize(&result.as_string().unwrap()),
                normalize(&expected),
                "{s}"
            )
        }
        o => panic!("{s}: {o:?}"),
    }
}
#[test]
fn synchronous_traces_match_real_tcl() {
    for s in [
        "set out {}; proc cb {n e op} {lappend ::out [list $n $e $op]}; trace add variable x {write read unset} cb; set x 1; set y $x; unset x; set out",
        "proc cb {n e op} {uplevel 1 {set x 42}}; trace add variable x read cb; set x",
        "proc cb {n e op} {uplevel 1 {set x 42}}; trace add variable x write cb; set x 1; set x",
        "proc cb {n e op} {uplevel 1 {set x 42}}; trace add variable x write cb; set y [set x 1]; set y",
        "proc cb {n e op} {uplevel 1 {set x 42}}; set x 1; trace add variable x write cb; incr x",
        "proc cb {n e op} {uplevel 1 {set x changed}}; set x hi; trace add variable x write cb; append x there",
        "proc cb {n e op} {error denied}; trace add variable x write cb; list [catch {set x 1} message] $message",
        "proc cb {n e op} {error denied}; trace add variable x read cb; set x 1; list [catch {set x} message] $message",
        "set out {}; proc cb {tag n e op} {lappend ::out $tag}; trace add variable x write {cb first}; trace add variable x write {cb second}; set x 1; set out",
        "set out {}; proc one {n e op} {lappend ::out one}; proc two {n e op} {trace remove variable x write one; lappend ::out two}; trace add variable x write one; trace add variable x write two; set x 1; set out",
        "set out {}; proc cb {n e op} {lappend ::out [list $n $e $op]}; proc inner {} {upvar 1 a(k) x; trace add variable x write cb; set x 7}; proc outer {} {set a(k) 1; inner}; outer; set out",
        "set out {}; proc cb {n e op} {lappend ::out [list $n $e $op]}; proc inner {} {upvar 1 x alias; trace add variable alias write cb; set alias 7}; proc outer {} {set x 1; inner}; outer; set out",
        "proc cb {n e op} {upvar 1 $n x; set x 42}; proc f {} {set x 0; trace add variable x write cb; set x 1}; f",
        "proc inner {} {upvar x alias; incr alias}; proc outer {} {set x 1; inner; set x}; outer",
        "proc cb {args} {}; proc inner {} {upvar 1 a(k) x; trace add variable x write cb; set x 7}; proc outer {} {set a(k) 1; inner; set a(k)}; outer",
        "proc inner {} {upvar #0 x alias; set alias 7}; set x 1; inner; set x",
        "set out {}; proc cb {n e op} {lappend ::out [list $n $e $op]}; trace add variable a write cb; set a(k) 7; set out",
        "proc cb {n e op} {uplevel 1 {incr x}}; proc f {} {set x 1; trace add variable x write cb; set x 2; set x}; f",
        "set out {}; proc cb {n e op} {lappend ::out [list $n $e $op]}; proc f {} {set x 1; trace add variable x unset cb}; f; set out",
        "namespace eval n {set out {}; proc cb {n e op} {lappend ::n::out $op}; trace add variable x write cb; set x 2; set out}",
        "proc cb {args} {}; trace add variable x {write read} cb; trace add variable x {read write} cb; trace info variable x",
        "proc cb {args} {}; trace add variable x write cb; trace remove variable x write cb; trace info variable x",
        "set out {}; proc cb {cmd op} {lappend ::out $op}; trace add execution string enter cb; string length hi; set out",
        "set out {}; proc cb {cmd args} {lappend ::out [lindex $args end]}; proc f {x} {incr x}; trace add execution f {enter leave} cb; list [f 40] $out",
        "set out {}; proc cb {old new op} {lappend ::out $op}; proc f {} {list}; trace add command f rename cb; rename f g; g; set out",
    ] {
        compare(s)
    }
}
#[test]
fn real_notifier_handles_channel_callbacks_and_errors() {
    let p = std::env::temp_dir().join(format!("tcld-events-{}", std::process::id()));
    std::fs::write(&p, "hello\n").unwrap();
    let p = tcld_compile::quote_word(&p.display().to_string());
    for script in [
        "set out {}; proc ready {ch} {lappend ::out [gets $ch]; fileevent $ch readable {}}; set ch [open PATH r]; fileevent $ch readable [list ready $ch]; vwait out; close $ch; set out",
        "set done 0; proc ready {ch} {set ::done [gets $ch]; fileevent $ch readable {}}; set ch [open PATH r]; chan event $ch readable [list ready $ch]; vwait done; close $ch; set done",
        "set out {}; proc ready {ch} {lappend ::out ok; fileevent $ch readable {}; close $ch}; set ch [open PATH r]; fileevent $ch readable [list ready $ch]; vwait out; set out",
        "set out {}; proc bgerror {message} {lappend ::out $message}; set ch [open PATH r]; fileevent $ch readable {error channel_failure}; vwait out; set result [list $out [fileevent $ch readable]]; close $ch; set result",
        "set ch [open PATH r]; fileevent $ch readable {set done 1}; set result [chan event $ch readable]; fileevent $ch readable {}; close $ch; set result",
        "set out {}; set ch [open PATH r]; fileevent $ch readable {lappend ::out wrong}; fileevent $ch readable {}; update; close $ch; set out",
    ] {
        compare(&script.replace("PATH", &p));
    }
    // The temporary fixture is removed through its original path.
    std::fs::remove_file(std::env::temp_dir().join(format!("tcld-events-{}", std::process::id())))
        .unwrap();
}
fn pause_and_edit(source: &str, callback: &str) {
    let mut e = engine();
    let mut state = e.debug_start_image(&image(source), true).unwrap().outcome;
    let mut address = None;
    for _ in 0..500 {
        if let DebugOutcome::Paused(p) = &state
            && p.proc_qname.as_deref() == Some(&format!("::{callback}"))
            && e.ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "marker" && v == "40")
        {
            address = p.address;
            break;
        }
        if matches!(state, DebugOutcome::Finished { .. }) {
            panic!("callback never entered");
        }
        state = e.step_instruction().unwrap();
    }
    let a = address.expect("paused in actual callback");
    if source.contains("tailcall target") {
        assert!(
            !e.ui_stack_frames()
                .iter()
                .any(|f| f.proc_qname.as_deref() == Some("::start")),
            "a tail call removes the departed procedure from the debugger stack"
        );
    }
    let source = e
        .debug_sources()
        .into_iter()
        .find(|s| s.code_id == a.code_id)
        .unwrap();
    assert!(matches!(
        source.image.units[source.unit_index].kind,
        tcld_compile::DebugCodeUnitKind::Procedure { .. }
    ));
    assert!(e.debug_sources().iter().any(|s| s.origin.is_some()));
    let pc = source.image.units[source.unit_index]
        .commands
        .iter()
        .find(|m| source.image.source.content[m.source_span.clone()].starts_with("set ::result"))
        .unwrap()
        .anchor_pc;
    let bp = CodeAddress {
        code_id: a.code_id,
        pc,
    };
    e.add_breakpoint(bp);
    e.eval_in_frame(0, VmObj::from_str("set marker 80"))
        .unwrap();
    assert!(matches!(e.continue_run().unwrap(), DebugOutcome::Paused(_)));
    e.remove_breakpoint(bp);
    match e.continue_run().unwrap() {
        DebugOutcome::Finished { result } => assert_eq!(result.as_string().unwrap(), "81"),
        o => panic!("{o:?}"),
    }
}
#[test]
fn callbacks_pause_with_the_triggering_tcl_stack_intact() {
    let path = std::env::temp_dir().join(format!("tcld-debug-callback-{}", std::process::id()));
    std::fs::write(&path, "hello\n").unwrap();
    pause_and_edit(
        &format!(
            "proc ready {{ch}} {{set marker 40; incr marker; set ::result $marker; fileevent $ch readable {{}}}}; set ch [open {} r]; fileevent $ch readable [list ready $ch]; update; close $ch; set result",
            tcld_compile::quote_word(&path.display().to_string())
        ),
        "ready",
    );
    std::fs::remove_file(path).unwrap();
    pause_and_edit(
        "set x 0; proc traced {n e op} {set marker 40; incr marker; set ::result $marker}; trace add variable x write traced; set x 2; set result",
        "traced",
    );
}
#[test]
fn paused_engines_and_cancellation_keep_bridge_ownership() {
    let source = image(
        "proc cb {n e op} {set marker 1; incr marker}; trace add variable x write cb; set x 1",
    );
    let mut a = engine();
    let mut b = engine();
    a.debug_start_image(&source, true).unwrap();
    b.debug_start_image(&source, true).unwrap();
    for _ in 0..50 {
        if a.pause_snapshot()
            .is_some_and(|s| s.proc_qname.as_deref() == Some("::cb"))
        {
            break;
        }
        a.step_instruction().unwrap();
    }
    drop(a);
    assert!(matches!(
        b.continue_run().unwrap(),
        DebugOutcome::Finished { .. }
    ));
    b.debug_start_image(&image("set x 42; set x"), false)
        .unwrap();
}

#[test]
fn mapped_execution_traces_match_real_tcl() {
    for s in [
        "set out {}; proc cb {args} {lappend ::out [join $args |]}; proc f {x} {set y $x; incr y; expr {$y+1}}; trace add execution f {enter leave enterstep leavestep} cb; set answer [f 40]; trace remove execution f {enter leave enterstep leavestep} cb; list $answer [join $out \n]",
        "set out {}; proc cb {args} {lappend ::out [join $args |]}; trace add execution set {enter leave} cb; set x 3; trace remove execution set {enter leave} cb; join $out \n",
        "set out {}; proc cb {args} {lappend ::out [join $args |]}; proc f {} {if {1} {set x 1; incr x}; return $x}; trace add execution f {enterstep leavestep} cb; f; join $out \n",
        "set out {}; proc cb {args} {lappend ::out [join $args |]}; proc f {} {catch {error boom} m; set m}; trace add execution f {enterstep leavestep} cb; f; join $out \n",
        "set out {}; proc cb {args} {lappend ::out [join $args |]}; trace add execution expr {enter leave} cb; expr {2+3}; trace remove execution expr {enter leave} cb; join $out \n",
    ] {
        compare(s)
    }
    pause_and_edit(
        "proc cb {args} {set marker 40; incr marker; set ::result $marker}; proc f {} {set x 2}; trace add execution f enterstep cb; f; set result",
        "cb",
    );
    pause_and_edit(
        "proc cb {args} {set marker 40; incr marker; set ::result $marker}; proc f {} {set x 2}; trace add execution f leavestep cb; f; set result",
        "cb",
    );
    pause_and_edit(
        "proc cb {args} {set marker 40; incr marker; set ::result $marker}; trace add execution expr enter cb; expr {2+3}; set result",
        "cb",
    );
}

#[test]
fn execution_trace_order_errors_and_generated_bodies_match_tcl() {
    let prefix = "set out {}; proc cb {args} {lappend ::out [join $args |]}; ";
    for body in [
        "proc f {} {for {set i 0} {$i<3} {incr i} {if {$i==1} {continue}; if {$i==2} {break}; set x $i}; set i}; trace add execution f {enterstep leavestep} cb; f",
        "proc f {} {foreach x {1 2} {incr x}; catch {return -code error boom} m; return $m}; trace add execution f {enter leave enterstep leavestep} cb; catch {f}",
        "proc f {} {eval {set x 1; incr x}; namespace eval n {set x 2; incr x}; uplevel #0 {set x 4; incr x}}; trace add execution f {enterstep leavestep} cb; f",
        "proc f {} {set cmd {if {1} {set x 3}}; eval $cmd; apply {{x} {incr x}} 4}; trace add execution f {enterstep leavestep} cb; f",
        "proc f {} {set body {incr x}; proc g {x} $body; g 40}; trace add execution f {enterstep leavestep} cb; f",
        "proc f {} {lsort -command cmp {3 1 2}}; proc cmp {a b} {expr {$a-$b}}; trace add execution f {enterstep leavestep} cb; f",
        "proc f {} {set x 1; error boom}; trace add execution f {enter leave enterstep leavestep} cb; catch {f} m",
        "proc f {} {return -code error -errorcode {MY ERROR} boom}; trace add execution f {enter leave enterstep leavestep} cb; catch {f} m",
        "proc deny {args} {error denied}; trace add execution set enter deny; catch {set x 1}; trace remove execution set enter deny",
        "proc deny {args} {error denied}; trace add execution expr leave deny; catch {expr {2+3}}; trace remove execution expr leave deny",
        "proc one {args} {lappend ::out one}; proc two {args} {lappend ::out two}; trace add execution set {enter leave} one; trace add execution set {enter leave} two; set x 1; trace remove execution set {enter leave} one; trace remove execution set {enter leave} two",
        "proc one {args} {lappend ::out one}; proc two {args} {trace remove execution set enter one; lappend ::out two}; trace add execution set enter one; trace add execution set enter two; set x 1; trace remove execution set enter two",
        "proc edit {cmd op} {uplevel 1 {set x 40}}; proc f {} {set x 1; expr {$x+1}}; trace add execution expr enter edit; lappend out [f]; trace remove execution expr enter edit",
        "set n 0; proc bump {} {incr ::n; list 1 2}; proc f {} {foreach x [bump] {incr x}; return [bump]}; trace add execution f {enterstep leavestep} cb; f; lappend out count=$n",
        "proc f {n} {if {$n>0} {f [expr {$n-1}]}; set n}; trace add execution f {enter leave enterstep leavestep} cb; f 2",
        "trace add execution proc {enter leave} cb; proc f {} {set x 1}; trace remove execution proc {enter leave} cb",
        "trace add execution if {enter leave enterstep leavestep} cb; if {1} {set x 1; incr x}; trace remove execution if {enter leave enterstep leavestep} cb",
        "proc f {} {set cmd {expr {2+3}}; eval $cmd}; trace add execution expr {enter leave} cb; f; trace remove execution expr {enter leave} cb",
        "proc deny {args} {error denied}; trace add execution set enter deny; lappend out [catch {set x 1} m]; trace remove execution set enter deny; lappend out $m",
        "proc deny {args} {error denied}; trace add execution expr leave deny; lappend out [catch {expr {2+3}} m]; trace remove execution expr leave deny; lappend out $m",
        "proc f {} {set x 1}; proc one {args} {trace remove execution f enterstep one; lappend ::out one}; trace add execution f enterstep one; f; f",
        "proc f {} {set x 1}; proc one {args} {trace add execution f leave cb; lappend ::out one}; trace add execution f enter one; f; f",
        "proc f {} {set x 1}; proc one {args} {rename f g}; trace add execution f enter one; lappend out [catch {f} m] $m; catch {g}",
        "proc f {} {set x 1}; proc one {args} {rename f {}}; trace add execution f enter one; lappend out [catch {f} m] $m",
        "proc f {} {set x 1}; proc one {args} {rename f g; lappend ::out renamed}; trace add execution f enter one; trace add execution f leave cb; lappend out [catch {f} m] $m; trace remove execution g enter one; g",
        "proc change {args} {rename expr oldexpr; proc expr {args} {return replaced}}; trace add execution expr enter change; lappend out [expr {1+2}]",
        "proc change {args} {rename expr {}}; trace add execution expr enter change; lappend out [catch {expr {1+2}} m] $m",
        "proc f {} {after 0 {set ::done 1}; vwait ::done}; trace add execution f enterstep cb; f",
        "proc traced {args} {set ::seen 1}; proc f {} {trace add variable x write traced; set x 1}; trace add execution f {enterstep leavestep} cb; f",
        "proc f {} {set x [string length hello]; info level}; trace add execution f {enterstep leavestep} cb; f",
        "namespace eval n {proc f {} {set x 1; incr x}; trace add execution f {enterstep leavestep} ::cb; f}",
    ] {
        compare(&format!("{prefix}{body}; join $out \\n"));
    }
}

#[test]
fn variable_inspection_does_not_run_read_callbacks_but_console_evaluation_does() {
    let mut e = engine();
    let source = image(
        "set ::reads 0; proc cb {args} {incr ::reads}; set x 1; trace add variable x read cb; set answer $x; set reads",
    );
    e.debug_start_image(&source, true).unwrap();
    let source_code = e
        .debug_sources()
        .into_iter()
        .find(|s| s.unit_index == s.image.top_level_unit)
        .unwrap();
    let pc = source_code
        .image
        .top_level()
        .commands
        .iter()
        .find(|m| source_code.image.source.content[m.source_span.clone()].starts_with("set answer"))
        .unwrap()
        .anchor_pc;
    let address = CodeAddress {
        code_id: source_code.code_id,
        pc,
    };
    e.add_breakpoint(address);
    assert!(matches!(e.continue_run().unwrap(), DebugOutcome::Paused(_)));
    for _ in 0..3 {
        e.ui_frame_locals(0);
    }
    assert_eq!(
        e.eval_in_frame(0, VmObj::from_str("set ::reads"))
            .unwrap()
            .result
            .as_string()
            .unwrap(),
        "0"
    );
    e.eval_in_frame(0, VmObj::from_str("set x")).unwrap();
    assert_eq!(
        e.eval_in_frame(0, VmObj::from_str("set ::reads"))
            .unwrap()
            .result
            .as_string()
            .unwrap(),
        "1"
    );
    assert!(matches!(
        e.continue_run().unwrap(),
        DebugOutcome::Finished { .. }
    ));
}

#[test]
fn real_coroutines_match_tcl() {
    for s in [
        "proc work {} {set a [yield ready]; set b [yield $a]; list $a $b [info coroutine]}; list [coroutine co work] [co first] [co second] [info commands co]",
        "proc work {} {for {set i 0} {$i<3} {incr i} {yield $i}; return done}; list [coroutine co work] [co] [co] [co]",
        "proc work {} {set x [eval {yield evaled}]; list $x}; list [coroutine co work] [co back]",
        "proc work {} {set x [yieldto list one two]; list resumed $x}; list [coroutine co work] [co a b c]",
        "proc work {} {yieldto no_such_target; return resumed}; list [catch {coroutine co work} m] $m [co]",
        "proc work {} {set x [catch {yield 1 2} m]; list $x $m}; coroutine co work",
        "list [catch {yield hi} m] $m [catch {yieldto list x} n] $n",
        "proc work {} {yield ready; error broken}; list [coroutine co work] [catch {co} m] $m [info commands co]",
        "namespace eval n {proc target {args} {list called $args}; proc work {} {set x [yieldto target hi]; list $x [info coroutine]}; list [coroutine co work] [co a b]}",
        "proc inner {x} {yield $x; incr x}; proc work {} {set a [inner 40]; incr a}; list [coroutine co work] [co]",
        "proc work {} {yield ready; return done}; coroutine co work; rename co renamed; list [renamed] [info commands renamed]",
        "proc work {} {yield ready; error should_not_run}; coroutine co work; rename co {}; info commands co",
        "proc work {} {yield ready; return done}; coroutine co work; list [catch {co a b} m] $m [co]",
        "proc work {} {yield ready; return done}; coroutine co work; coroutine co work; co",
        "set out {}; proc work {tag} {lappend ::out $tag; yield $tag; lappend ::out $tag; return $tag}; list [coroutine a work A] [coroutine b work B] [a] [b] $out",
        "proc work {} {set x 40; yield ready; incr x}; coroutine co work; set x 9; list [co] $x",
        "coroutine co apply {{} {set x [yield ready]; list $x [info coroutine]}}; co back",
        "proc work {} {set x 1; yield ready; incr x}; proc resume {} {set ::done [co]}; coroutine co work; after 0 resume; vwait done; set done",
        "set out {}; proc cb {n e op} {lappend ::out $op}; proc work {} {set x 1; trace add variable x unset cb; yield ready}; coroutine co work; rename co {}; set out",
        "proc work {} {set a [catch {uplevel 0 {yield ready}} m]; list $a $m}; list [coroutine co work] [co back]",
        "proc work {} {set a [namespace eval n {yield ready}]; list $a}; list [coroutine co work] [co back]",
        "proc work {} {yield ready; return -code error -errorcode {MY CORO} broken}; coroutine co work; catch {co} m opts; list $m [dict get $opts -errorcode]",
        "proc co {} {error obsolete}; proc work {} {yield ready; return done}; list [coroutine co work] [co]",
        "proc work {} {yield ready; yieldto list next; return done}; set out {}; proc cb {args} {lappend ::out [join $args |]}; trace add execution work {enter leave enterstep leavestep} cb; set a [coroutine co work]; set b [co]; set c [co]; list $a $b $c [join $out \n]",
        "proc work {} {set x 40; yield ready; incr x}; coroutine co work; list [info coroutine] [co]",
        "proc child {} {yield ready; yieldto yield fromChild; return done}; proc parent {} {coroutine childco child; set x [childco]; list resumed $x}; list [coroutine parentco parent] [parentco back] [childco]",
        "proc child {} {yieldto set marker 41; yield second; return end}; proc parent {} {set marker 1; set x [coroutine childco child]; list $marker $x [childco] [childco]}; parent",
        "proc inner {} {yieldto yield nested; return childDone}; proc outer {} {set x [coroutine child inner]; list $x [child]}; list [coroutine co outer] [co back]",
        "proc work {} {yieldto return early; error unreachable}; proc f {} {coroutine co work; error unreachable}; f",
        "proc cb {args} {yield not_allowed}; proc work {} {trace add variable x write cb; catch {set x 1} m; set m}; coroutine co work",
        "proc work {} {yield ready; return done}; set out {}; proc cb {args} {lappend ::out [join $args |]}; trace add execution work {enter leave enterstep leavestep} cb; coroutine co work; rename co {}; join $out \n",
        "proc other {} {yield other; yieldto list transferred; return done}; proc work {} {yieldto otherco; return finished}; coroutine otherco other; list [coroutine co work] [co] [otherco]",
    ] {
        compare(s);
    }
}
#[test]
fn coroutine_frames_pause_edit_and_resume_after_yield() {
    pause_and_edit(
        "proc work {} {yield ready; set marker 40; incr marker; set ::result $marker}; coroutine co work; co; set result",
        "work",
    );
    pause_and_edit(
        "proc work {} {set marker 40; incr marker; set ::result $marker; yield ready}; coroutine co work; co; set result",
        "work",
    );
    let mut e = engine();
    e.debug_start_image(&image("proc work {} {\n set x 40\n yield ready\n incr x\n return $x\n}\ncoroutine co work\nco"), true).unwrap();
    let mut address = None;
    for _ in 0..150 {
        if let Some(p) = e.pause_snapshot()
            && p.proc_qname.as_deref() == Some("::work")
            && e.ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "x" && v == "40")
        {
            address = p.address;
            break;
        }
        e.step_instruction().unwrap();
    }
    let code_id = address.unwrap().code_id;
    assert!(matches!(e.step_out().unwrap(), DebugOutcome::Paused(p) if p.proc_qname.is_none()));
    let source = e
        .debug_sources()
        .into_iter()
        .find(|s| s.code_id == code_id)
        .unwrap();
    let pc = source.image.units[source.unit_index]
        .commands
        .iter()
        .find(|m| m.start_line == 4)
        .unwrap()
        .anchor_pc;
    e.add_breakpoint(CodeAddress { code_id, pc });
    assert!(
        matches!(e.continue_run().unwrap(), DebugOutcome::Paused(p) if p.proc_qname.as_deref()==Some("::work"))
    );
    e.eval_in_frame(0, VmObj::from_str("set x 80")).unwrap();
    e.remove_breakpoint(CodeAddress { code_id, pc });
    assert!(
        matches!(e.continue_run().unwrap(), DebugOutcome::Finished {result} if result.as_string().unwrap()=="81")
    );
}

#[test]
fn coroutine_reset_and_paused_engine_destruction_keep_ownership() {
    let mut e = engine();
    let source = image(
        "proc work {} {set x 1; trace add variable x unset {apply {{args} {list}}}; yield ready; incr x}; coroutine co work; rename co renamed; list",
    );
    e.debug_start_image(&source, false).unwrap();
    let result = e
        .debug_start_image(
            &image("list [info commands renamed] [namespace children ::tcld::locals]"),
            false,
        )
        .unwrap();
    assert!(
        matches!(result.outcome, DebugOutcome::Finished { result } if result.as_list_vec().unwrap().iter().all(|v| v.as_string().unwrap().is_empty()))
    );
    let source =
        image("proc work {} {yield ready; set marker 40; incr marker}; coroutine co work; co");
    e.debug_start_image(&source, true).unwrap();
    for _ in 0..150 {
        if e.pause_snapshot()
            .is_some_and(|s| s.proc_qname.as_deref() == Some("::work"))
        {
            break;
        }
        e.step_instruction().unwrap();
    }
    let mut other = engine();
    other.debug_start_image(&source, true).unwrap();
    drop(e);
    assert!(
        matches!(other.continue_run().unwrap(), DebugOutcome::Finished { result } if result.as_string().unwrap()=="41")
    );
    other
        .debug_start_image(&image("expr {6*7}"), false)
        .unwrap();
}
#[test]
fn coroutine_sourced_and_runtime_generated_bodies_keep_provenance() {
    let path = std::env::temp_dir().join(format!("tcld-coroutine-{}.tcl", std::process::id()));
    std::fs::write(&path, "set x [yield sourced]; incr x").unwrap();
    let script = format!(
        "proc work {{}} {{source {}}}; list [coroutine co work] [co 40]",
        tcld_compile::quote_word(&path.display().to_string())
    );
    compare(&script);
    std::fs::remove_file(path).unwrap();
    compare(
        "set body {set x [yield dynamic]; incr x}; proc work {} $body; list [coroutine co work] [co 40]",
    );
    compare(
        "proc work {} {for {set i 0} {$i<100} {incr i} {yield $i}; return done}; set sum [coroutine co work]; for {set i 0} {$i<99} {incr i} {incr sum [co]}; list $sum [co]",
    );
}

#[test]
fn tailcalls_match_the_real_tcl_scheduler() {
    for s in [
        "proc end {x} {list done $x [info level]}; proc start {x} {tailcall end $x; error unreachable}; start 42",
        "proc end {} {upvar 1 x v; incr v}; proc start {} {set x bad; tailcall end}; proc outer {} {set x 40; start; set x}; outer",
        "namespace eval n {proc end {x} {namespace current}; proc start {} {tailcall end 1}}; n::start",
        "proc end {args} {list {*}$args}; proc start {} {set target end; tailcall $target {*}[list {a b} c]}; start",
        "proc end {} {return target}; proc start {} {set r [catch {tailcall end} m o]; list $r $m}; start",
        "proc end {} {return target}; proc start {} {catch {tailcall end}; tailcall}; start",
        "proc end {} {error target}; proc start {} {tailcall end}; list [catch {start} m] $m",
        "proc end {} {return target}; proc start {} {catch {tailcall end}; error canceled}; list [catch {start} m] $m",
        "proc end {} {return target}; proc start {} {catch {tailcall end}; return -code error canceled}; list [catch {start} m] $m",
        "proc end {} {return target}; proc start {} {catch {tailcall missing}; catch {tailcall end}; return ignored}; start",
        "list [catch {tailcall list x} m] $m",
        "proc start {} {namespace eval n {tailcall list x}}; list [catch {start} m] $m",
        "proc end {} {return -level 2 bypass}; proc start {} {tailcall end}; proc outer {} {start; error unreachable}; outer",
        "proc start {} {tailcall break}; list [catch {start} m] $m",
        "proc end {} {return -code break}; proc start {} {tailcall end}; list [catch {start} m] $m",
        "proc start {} {eval {tailcall list from-eval}}; start",
        "proc start {} {uplevel 0 {tailcall list from-uplevel}}; start",
        "proc end {} {yield paused; return complete}; proc start {} {tailcall end}; list [coroutine co start] [co] [info commands co]",
        "proc body {} {yield ready; return resumed}; coroutine co body; proc start {} {tailcall co}; list [start] [info commands co]",
        "proc one {} {yield first; return one}; coroutine a one; proc two {} {tailcall a}; list [coroutine b two] [info commands a] [info commands b]",
        "proc body {} {yield ready; yieldto list outside; tailcall list done}; list [coroutine co body] [co] [co]",
        "proc end {} {yield ready; tailcall list complete}; proc start {} {tailcall end}; list [coroutine co start] [co]",
        "set out {}; proc cb {args} {lappend ::out [join $args |]}; proc end {} {lappend ::out target; return yes}; proc start {} {tailcall end}; trace add execution start {enter leave enterstep leavestep} cb; set r [start]; list $r [join $out \n]",
        "set out {}; proc cb {args} {lappend ::out unset}; proc end {} {lappend ::out target}; proc start {} {set x 1; trace add variable x unset cb; tailcall end}; start; set out",
        "proc cb {args} {tailcall list callback}; trace add variable x write cb; set x 1; set x",
        "proc cb {} {tailcall list callback}; after 0 cb; update; set ok yes",
        "proc end {} {return done}; proc start {} {catch {tailcall end}; yield ready; return ignored}; list [coroutine co start] [co]",
        "proc end {} {return done}; proc start {} {catch {tailcall end}; yield ready}; coroutine co start; rename co {}; info commands co",
        "proc body {} {yield ready; tailcall yieldto list outside}; list [coroutine co body] [co] [info commands co]",
        "proc end {} {set x [yield ready]; tailcall list $x}; proc parent {} {set a [coroutine child end]; list $a [child value]}; coroutine co parent",
        "proc childbody {} {yield ready; return done}; proc parent {} {coroutine child childbody; tailcall child}; coroutine co parent",
        "proc end {args} {list {*}$args}; proc start {} {set prefix [list tailcall end]; {*}$prefix a b}; start",
        "proc end {} {return yes}; proc start {} {tailcall eval {end}}; start",
        "proc start {} {tailcall apply {{} {list lambda [info level]}}}; start",
        "proc end {} {return done}; proc cb {args} {error canceled}; proc start {} {tailcall end}; trace add execution start leave cb; list [catch {start} m] $m",
        "set out {}; proc cb {args} {lappend ::out [join $args |]}; proc end {} {return yes}; proc start {} {tailcall end}; trace add execution tailcall {enter leave} cb; start; join $out \n",
        "proc start {} {tailcall missing}; list [catch {start} m] $m",
        "proc target {args} {return yes}; proc cb {args} {rename target {}}; trace add execution target enter cb; proc start {} {set words [list target a b c]; set code [catch {{*}$words} m]; tailcall list $code $m}; start",
        "namespace eval n {proc gone {} {catch {tailcall list done}; namespace delete ::n; return ignored}}; proc target {} {set c [catch {n::gone} m]; list caught $c $m}; proc start {} {tailcall target}; start",
        "proc start {} {set empty {}; {*}$empty; tailcall list done}; start",
        "proc start {} {set a [format %c 123]; tailcall list {*}$a}; list [catch {start} m] $m",
        "proc end {} {yieldto set x 41; return done}; proc target {} {set x 1; set r [coroutine child end]; list $x $r [child]}; proc start {} {tailcall target}; start",
        "proc end {} {return -code 201 custom}; proc start {} {tailcall end}; list [catch {start} m] $m",
        "proc end {} {return -code 200 custom}; proc start {} {tailcall end}; list [catch {coroutine co start} m] $m",
        "proc end {} {return -code 201 custom}; proc start {} {tailcall end}; list [catch {coroutine co start} m] $m",
        "proc target {x} {return $x}; proc start {} {tailcall target}; list [catch {start} m] $m",
        "namespace eval n {proc start {} {catch {tailcall list done}; namespace delete ::n; return ignored}}; list [catch {n::start} m] $m",
        "list [catch {::tcld::tail_transfer 0 1234} m] [string match {*tailcall token*} $m]",
    ] {
        // The final case addresses our private native adapter, not an oracle command.
        if s.contains("::tcld::tail_transfer") {
            let mut e = engine();
            assert!(matches!(
                e.debug_start_image(&image(s), false).unwrap().outcome,
                DebugOutcome::Finished { .. }
            ));
        } else {
            compare(s);
        }
    }
}
#[test]
fn tailcall_targets_remain_mapped_and_replace_the_callers_scope() {
    pause_and_edit(
        "proc target {} {set marker 40; incr marker; set ::result $marker}; proc start {} {tailcall target}; start; set result",
        "target",
    );
    pause_and_edit(
        "proc target {} {yield ready; set marker 40; incr marker; set ::result $marker}; proc start {} {tailcall target}; coroutine co start; co; set result",
        "target",
    );
    compare(
        "proc even {n} {if {$n == 0} {return [info level]}; tailcall odd [expr {$n-1}]}; proc odd {n} {tailcall even [expr {$n-1}]}; even 2000",
    );
}
