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
        "/workspace/subst-time.tcl".into(),
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
        other => panic!("{source}: {other:?}"),
    }
}
#[test]
fn subst_matches_native_parsing_variables_and_completions() {
    for s in [
        "subst {plain text}",
        "set x 40; subst {answer [incr x] / $x / [expr {$x+1}]}",
        "proc f {} {set x 40; set template {[incr x]:$x}; list [subst $template] $x}; f",
        "set script [format {expr {%d * %d}} 6 7]; set template [format {value [%s]} $script]; subst $template",
        "subst {nested [subst {inner [expr {6*7}]}]}",
        "set a(k) 40; set key k; subst {$a($key):[incr a(k)]}",
        "proc f {} {set a(k) 40; set key k; subst {$a([set key]):$a($key)}}; f",
        "proc f {} {set x 40; proc g {} {upvar 1 x y; subst {[incr y]:$y}}; list [g] $x}; f",
        "namespace eval n {set x 40; subst {$x:[incr x]}}",
        "set x 40; subst -nocommands {$x:[incr x]}",
        "set x 40; subst -novariables {$x:[incr x]}",
        "set x 40; subst -nobackslashes {\\n$x:[incr x]}",
        "set x 40; subst -nob -noc -nov {\\n$x:[incr x]}",
        "set x 40; subst -nocommands -nocommands {$x:[incr x]}",
        "subst -novariables -nocommands {a\\nb\\t\\u03bb}",
        "set a(k) 42; subst -nocommands {$a([set index k])}",
        "set a(k) 42; subst -novariables {$a([set index k])}",
        "subst {before[return returned]after}",
        "subst {before[break]after[error unreachable]}",
        "subst {before[continue]after[expr {6*7}]}",
        "subst {a[return -level 0 -code break stop]b}",
        "subst {a[return -level 0 -code continue skip]b}",
        "proc f {} {subst {a[return found]b}; return completed}; f",
        "set c [catch {subst {before[error bad {} {APP FAIL}]after}} m o]; list $c $m [dict get $o -errorcode]",
        "set out {}; proc traced {n e op} {upvar 1 $n v; set v changed; lappend ::out $op}; proc f {} {set x old; trace add variable x read traced; list [subst {$x}] $x}; list [f] $out",
        "proc traced {n e op} {error denied}; proc f {} {trace add variable x read traced; subst {$x}}; list [catch {f} m] $m",
        "proc work {} {set x 40; subst {before[yield paused]:[incr x]:$x}}; list [coroutine co work] [co resumed]",
        "proc work {} {set x 40; yield ready; subst {$x:[incr x]}}; list [coroutine co work] [co]",
        "proc target {} {return done}; proc f {} {subst {a[tailcall target]b}; return unreachable}; f",
        "subst {héllo [list 世界] λ}",
        "set x 40; subst {\\[literal\\] \\$x [incr x]}",
        "subst -nocommands {unmatched [ and $}",
    ] {
        compare(s)
    }
}
#[test]
fn subst_preserves_deferred_parse_errors_and_prefix_effects() {
    for suffix in [
        "[",
        "[set later 2",
        "[set first 1; set later 2",
        "${bad",
        "$a(",
        "$a([set later 2",
        "[set first 1; error {bad",
    ] {
        // Construct arbitrary template text with quote_word; its malformed
        // substitution syntax is runtime data, never the outer script grammar.
        let template = format!("prefix[incr x]{suffix}");
        let source = format!(
            "set x 0; set template {}; set c [catch {{subst $template}} m o]; list $c $m $x [info exists first] [info exists later]",
            tcld_compile::quote_word(&template)
        );
        compare(&source);
        for completion in ["break", "continue", "return done"] {
            let template = format!("prefix[{completion}]{suffix}");
            compare(&format!(
                "set template {}; set c [catch {{subst $template}} m o]; list $c $m [info exists first] [info exists later]",
                tcld_compile::quote_word(&template)
            ));
        }
    }
}
#[test]
fn time_uses_native_repetition_completion_and_timing_shape() {
    for s in [
        "set x 0; set t [time {incr x}]; list $x [llength $t] [string is double -strict [lindex $t 0]] [lrange $t 1 end]",
        "set x 0; set t [time {incr x} 3]; list $x [llength $t] [string is double -strict [lindex $t 0]] [lrange $t 1 end]",
        "set x 0; list [time {incr x} 0] $x",
        "set x 0; list [time {incr x} -2] $x",
        "set x 0; list [time {incr x} 2147483648] $x",
        "set body [format {incr x %d} 2]; set x 0; set t [time $body 3]; list $x [lrange $t 1 end]",
        "proc f {} {set x 0; time {incr x} 3; set x}; f",
        "set body {lmap x {1 2} {subst {[expr {$x*2}]}}}; set t [time $body 3]; lrange $t 1 end",
        "proc target {} {return done}; proc f {} {time {tailcall target} 3; error unreachable}; f",
        "proc f {} {time {return found} 3; error unreachable}; f",
        "set out {}; foreach x {1 2 3} {time {if {$x == 2} {continue}; if {$x == 3} {break}; lappend out $x} 2}; set out",
        "set x 0; set c [catch {time {incr x; error bad {} {APP FAIL}} 3} m o]; list $c $m $x [dict get $o -errorcode]",
        "proc work {} {time {yield pause} 2}; list [catch {coroutine co work} m] $m",
        "set x 0; proc traced {n e op} {upvar 1 $n v; incr v}; trace add variable x write traced; time {incr x} 2; set x",
        "namespace eval n {set x 0; time {incr x} 3; set x}",
        "set out {}; proc cb {cmd args} {lappend ::out [join $args |]}; proc f {} {time {set x 1} 2; set x}; trace add execution f {enterstep leavestep} cb; set result [f]; list $result [llength $out]",
    ] {
        compare(s)
    }
}
#[test]
fn invalid_forms_keep_native_diagnostics_and_private_entry_is_guarded() {
    for form in [
        "subst",
        "subst -bad {abc}",
        "subst -- {abc}",
        "subst -no {abc}",
        "subst a b",
        "time",
        "time {} bad",
        "time {} 1.5",
        "time {} 9223372036854775808",
        "time {} 1 extra",
    ] {
        compare(&format!(
            "set c [catch {{{form}}} m o]; list $c $m [dict get $o -errorcode]"
        ));
    }
    compare(
        "list [catch {::tcld::control 0 :: {} subst {[set ::escaped yes]}} m] [info exists ::escaped]",
    );
    let source = "list [catch {subst {before[interp eval {} {set ::escaped yes}]}} m] [info exists ::escaped]";
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
        other => panic!("{other:?}"),
    }
}
#[test]
fn generated_substitutions_and_timing_iterations_have_live_source_breakpoints() {
    for name in ["subst", "time"] {
        let body = "set marker 40\nincr marker\nset ::result $marker";
        let command = if name == "subst" {
            "set template [format {before[%s]after} $body]; subst $template"
        } else {
            "time $body 2; set ::result"
        };
        let source = format!(
            "proc work {{}} {{set body {}; {command}}}\nwork",
            tcld_compile::quote_word(body)
        );
        let image = compile_source(
            "/workspace/generated-subst-time.tcl".into(),
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
        let a = address.expect("generated body has source and actual caller locals");
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
        if name == "time" {
            assert!(matches!(e.continue_run().unwrap(), DebugOutcome::Paused(_)));
            assert_eq!(
                e.pause_snapshot().unwrap().address.unwrap().code_id,
                a.code_id
            );
            e.eval_in_frame(0, tcld_vm::VmObj::from_str("set marker 100"))
                .unwrap();
        }
        e.remove_breakpoint(bp);
        match e.continue_run().unwrap() {
            DebugOutcome::Finished { result } => assert_eq!(
                result.as_string().unwrap(),
                if name == "time" {
                    "100"
                } else {
                    "before81after"
                }
            ),
            o => panic!("{o:?}"),
        }
    }
}
