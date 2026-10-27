#![cfg(all(feature = "host", feature = "debug"))]
//! Comparator behavior, call order, completion routing, and debugger state
//! checked against embedded Tcl in both execution modes.

use tcld_compile::{CompileMode, compile_source};
use tcld_vm::list_codec::TclListCodec;
use tcld_vm::{CodeAddress, DebugOutcome, HybridEngine, RuntimeMode, VmObj};

fn mode(runtime: RuntimeMode) -> CompileMode {
    if runtime == RuntimeMode::VmStrict {
        CompileMode::VmStrict
    } else {
        CompileMode::Hybrid
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
    let image = compile_source("/workspace/lsort.tcl".into(), source.into(), mode(runtime))
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

fn oracle(source: &str) -> String {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    tcld_interp::Interpreter::new()
        .unwrap()
        .eval(source)
        .unwrap_or_else(|error| panic!("Tcl: {source}: {error}"))
}

fn compare(sources: &[&str]) {
    let mut failures = Vec::new();
    for source in sources {
        let expected = oracle(source);
        for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            let result = run(source, runtime);
            if result.as_ref() != Ok(&expected) {
                failures.push(format!(
                    "{runtime:?}: {source}\nTcl: {expected}\nVM: {result:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn list(words: &[&str]) -> String {
    String::from_utf8(TclListCodec::format(
        words.iter().map(|word| word.as_bytes()),
    ))
    .unwrap()
}

#[test]
fn command_options_scopes_indices_stride_and_input_snapshots_match_tcl() {
    compare(&[
        "lsort -command {string compare} {c a b}",
        "::lsort -c {::string compare} {c a b}",
        "set options {-c {string compare}}; lsort {*}$options {c a b}",
        "set command lsort; set option -command; $command $option {string compare} {c a b}",
        "lsort -decreasing -command {string compare} {c a b}",
        "lsort -decreasing -increasing -command {string compare} {c a b}",
        "lsort -command {string compare} -nocase {a B A b}",
        "lsort -command unused -ascii {c a b}",
        "lsort -command unused -dictionary {a10 a2 a1}",
        "lsort -integer -command {string compare} {2 10 1}",
        "lsort -command error -command {string compare} {c a b}",
        "lsort -command {} {}",
        "lsort -command {} single",
        "lsort -command error {}",
        "lsort -command error single",
        "lsort -stride 2 -index 99 -command error {}",
        "lsort -command {string compare} {é λ a 🙂 𝄞}",
        r#"lsort -command {string compare} {{a;[error injected]} {$x} {a\b} {a b}}"#,
        r#"proc compare {tag a b} {if {$tag ne {a;[error injected]$x}} {error prefix}; string compare $a $b}; lsort -command [list compare {a;[error injected]$x}] {c a b}"#,
        "lsort -index 1 -command {string compare} {{old b} {first a} {new b} {last a}}",
        "lsort -unique -index 1 -command {string compare} {{old b} {first a} {new b} {last a}}",
        "lsort -unique -indices -index 1 -command {string compare} {{old b} {first a} {new b} {last a}}",
        "lsort -index {1 end} -command {string compare} {{x {1 c}} {y {2 a}} {z {3 b}}}",
        "lsort -stride 2 -index 1 -command {string compare} {X c Y a Z b}",
        "lsort -stride 2 -index 1 -indices -command {string compare} {X c Y a Z b}",
        "lsort -stride 2 -index end -unique -command {string compare} {old b first a new b last a}",
        "lsort -stride 2 -index end -unique -indices -command {string compare} {old b first a new b last a}",
        "lsort -stride 3 -index {end 0} -command {string compare} {x X {c 1} y Y {a 2} z Z {b 3}}",
        "namespace eval n {proc compare {a b} {string compare $b $a}; lsort -command compare {c a b}}",
        "proc compare {a b} {upvar 1 calls calls; incr calls; expr {$a-$b}}; proc f {} {set calls 0; set sorted [lsort -command compare {3 1 2}]; list $sorted $calls}; f",
        "proc compare {a b} {uplevel 1 {incr calls}; expr {$a-$b}}; proc f {} {set calls 0; set sorted [lsort -command compare {3 1 2}]; list $sorted $calls}; f",
        "proc compare {a b} {list [info level] [info level 0]; string compare $a $b}; proc f {} {lsort -command compare {c a b}}; f",
        "proc compare {a b} {lappend ::levels [list [info level] [info level 0]]; string compare $a $b}; set levels {}; proc f {} {lsort -command compare {c a b}}; list [f] $levels",
        "proc compare {a b} {upvar 1 input input; set input changed; string compare $a $b}; proc f {} {set input {c a b}; set sorted [lsort -command compare $input]; list $sorted $input}; f",
        "proc compare {a b} {set ::prefix error; set ::input changed; string compare $a $b}; set prefix compare; set input {c a b}; list [lsort -command $prefix $input] $prefix $input",
        "proc compare {a b} {lsort -command {string compare} {z x y}; string compare $a $b}; lsort -command compare {c a b}",
        "proc compare {a b} {regsub -all -command . abc {string toupper}; string compare $a $b}; lsort -command compare {c a b}",
        "proc compare {a b} {string compare $a $b}; proc replacement {m} {join [lsort -command compare {c a b}] {}}; regsub -command . xyz replacement",
        "set calls 0; proc compare {a b} {incr ::calls; expr {$a-$b}}; list [lsort -command compare {9 3 7 1 8 2 6 4 5}] $calls",
    ]);
}

#[test]
fn every_comparison_and_unique_tie_follows_tcls_merge_order() {
    let mut sources = Vec::new();
    for count in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 31, 32, 33] {
        let items: Vec<String> = (0..count)
            .map(|i| list(&[&((i * 13 + 5) % 7).to_string(), &format!("item{i}")]))
            .collect();
        let input = list(&items.iter().map(String::as_str).collect::<Vec<_>>());
        for options in [
            "",
            "-unique",
            "-decreasing",
            "-decreasing -unique",
            "-unique -indices",
        ] {
            sources.push(format!(
                "set calls {{}}; proc compare {{a b}} {{lappend ::calls [list $a $b]; expr {{[lindex $a 0]-[lindex $b 0]}}}}; set sorted [lsort {options} -command compare {{{input}}}]; list $sorted $calls"
            ));
        }
    }
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn comparator_integer_conversion_and_error_codes_match_tcl() {
    let mut sources = Vec::new();
    for value in [
        "-2147483649",
        "-2147483648",
        "-1",
        "0",
        "1",
        "2147483647",
        "2147483648",
        "4294967295",
        "4294967296",
        "999999999999999999999",
        "0xffffffff",
        "0x100000000",
        " 1 ",
        "+1",
        "01",
        "0b1",
        "true",
        "1.0",
        "NaN",
        "",
        "bad",
    ] {
        for direction in ["", "-decreasing"] {
            sources.push(format!(
                "set calls 0; proc compare {{a b}} {{incr ::calls; return {}}}; set code [catch {{lsort {direction} -command compare {{b a c}}}} result options]; set errorcode {{}}; if {{$code == 1}} {{set errorcode [dict get $options -errorcode]}}; list $code $result $calls [dict get $options -code] [dict get $options -level] [dict exists $options -errorcode] $errorcode",
                tcld_compile::quote_word(value)
            ));
        }
    }
    for command in [
        "lsort -command {a b}",
        "lsort -command ignored -c {a b}",
        "lsort -command {string compare} -stride 1 {a b}",
        "lsort -command {string compare} -stride 2 {a b c}",
        "lsort -command {string compare} -stride 2 -index 2 {a b c d}",
        "lsort -command {string compare} -index -1 {}",
        "lsort -command {string compare} -index bad {a b}",
        "lsort -command {string compare} -index 1 {{a} {b c}}",
        "lsort -command \"bad \\{prefix\" {}",
        "lsort -command \"bad \\{prefix\" \"a \\{list\"",
        "lsort -command {string compare} \"a \\{list\"",
        "lsort -command {string compare} -i {a b}",
    ] {
        sources.push(format!("set code [catch {{{command}}} result options]; list $code $result [dict get $options -errorcode]"));
    }
    compare(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn callback_completions_and_validation_after_failure_match_tcl() {
    compare(&[
        "set calls 0; proc compare {a b} {incr ::calls; error failed {} {SORT CALLBACK}}; set code [catch {lsort -command compare {c b a}} result options]; list $code $result $calls [dict get $options -errorcode] [string match {*(-compare command)*} [dict get $options -errorinfo]]",
        "set calls 0; proc compare {a b} {incr ::calls; return bad}; set code [catch {lsort -command compare -index 1 {{a 1} {b 2} {c}}} result options]; list $code $result $calls [dict get $options -errorcode]",
        "set calls 0; proc compare {a b} {incr ::calls; error failed}; set code [catch {lsort -command compare -index 1 {{a 1} {b 2} {c}}} result options]; list $code $result $calls [dict get $options -errorcode]",
        "proc compare {a b} {return -level 2 found}; proc f {} {lsort -command compare {c b a}; error unreachable}; f",
        "proc compare {a b} {return -code 7 custom}; set code [catch {lsort -command compare {c b a}} result options]; list $code $result [dict get $options -code] [dict get $options -level]",
        "proc compare {a b} {return -code break stopped}; set out {}; foreach n {1 2} {lappend out $n; lsort -command compare {c b a}; lappend out bad}; set out",
        "proc compare {a b} {return -code continue next}; set out {}; foreach n {1 2} {lappend out $n; lsort -command compare {c b a}; lappend out bad}; set out",
        "set prefix {string compare}; lsort -command $prefix {c a b}",
    ]);
}

#[test]
fn comparator_lookup_arity_and_deletion_errors_match_tcl() {
    compare(&[
        "set code [catch {lsort -command nonexistent {b a}} result options]; list $code $result [dict get $options -errorcode] [string match {*(-compare command)*} [dict get $options -errorinfo]]",
        "set code [catch {lsort -command {} {b a}} result options]; list $code $result [dict get $options -errorcode]",
        "proc compare {a} {return 0}; set code [catch {lsort -command compare {b a}} result options]; list $code $result [dict get $options -errorcode]",
        "proc compare {a b c} {return 0}; set code [catch {lsort -command compare {b a}} result options]; list $code $result [dict get $options -errorcode]",
        "proc compare {} {return 0}; set code [catch {lsort -command compare {b a}} result options]; list $code $result [dict get $options -errorcode]",
        "proc compare {a b c args} {return 0}; set code [catch {lsort -command compare {b a}} result options]; list $code $result [dict get $options -errorcode]",
        "set calls 0; proc compare {a b} {incr ::calls; rename compare {}; return 0}; set code [catch {lsort -command compare {c b a}} result options]; list $code $result $calls [dict get $options -errorcode]",
    ]);
}

#[test]
fn hybrid_lambdas_execution_traces_and_synchronous_yield_match_tcl() {
    for source in [
        "lsort -command {apply {{a b} {expr {$a-$b}}}} {3 1 2}",
        "set events {}; proc traced {args} {lappend ::events $args}; proc compare {a b} {string compare $a $b}; trace add execution compare {enter leave} traced; set sorted [lsort -command compare {c a b}]; list $sorted $events",
        "proc compare {a b} {yield comparator}; set code [catch {coroutine co lsort -command compare {c a b}} result options]; list $code $result [dict get $options -errorcode] [info commands co]",
    ] {
        assert_eq!(
            run(source, RuntimeMode::Hybrid).unwrap(),
            oracle(source),
            "{source}"
        );
    }
}

#[test]
fn comparator_source_admission_cannot_escape_the_mapped_vm() {
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let source = "set code [catch {lsort -command {interp eval {} {set ::escaped yes}} {b a}} result]; list $code [info exists ::escaped]";
        assert_eq!(run(source, runtime).unwrap(), "1 0", "{runtime:?}");
    }
}

#[test]
fn comparator_breakpoints_locals_and_live_edits_survive_multiple_comparisons() {
    let source = "proc compare {a b} {\nset result [string compare $a $b]\nset marker [list $a $b]\nreturn $result\n}\nlsort -command compare {c b a}\n";
    for runtime in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let image = compile_source(
            "/workspace/sort-debug.tcl".into(),
            source.into(),
            mode(runtime),
        )
        .unwrap();
        let mut engine = engine(runtime);
        let mut state = engine.debug_start_image(&image, true).unwrap().outcome;
        let mut address = None;
        for _ in 0..500 {
            if let DebugOutcome::Paused(pause) = &state
                && let Some(at) = pause.address
                && engine.debug_sources().iter().any(|s| {
                    s.code_id == at.code_id
                        && s.image.units[s.unit_index].logical_id == "proc:compare"
                })
                && engine
                    .ui_frame_locals(0)
                    .iter()
                    .any(|(n, v)| n == "result" && v == "1")
            {
                address = Some(at);
                break;
            }
            assert!(
                !matches!(state, DebugOutcome::Finished { .. }),
                "{runtime:?}: never paused in comparator"
            );
            state = engine.step_instruction().unwrap();
        }
        let at = address.expect("mapped comparator paused with its local result");
        let sources = engine.debug_sources();
        let mapped = sources.iter().find(|s| s.code_id == at.code_id).unwrap();
        let pc = mapped.image.units[mapped.unit_index]
            .commands
            .iter()
            .find(|map| {
                mapped.image.source.content[map.source_span.clone()].starts_with("return $result")
            })
            .unwrap()
            .anchor_pc;
        assert!(
            sources
                .iter()
                .any(|s| s.origin.is_some() && s.image.source.content.as_ref() == "compare c b")
        );
        let breakpoint = CodeAddress {
            code_id: at.code_id,
            pc,
        };
        engine.add_breakpoint(breakpoint);
        if runtime == RuntimeMode::Hybrid {
            engine
                .eval_in_frame(0, VmObj::from_str("set result -1"))
                .unwrap();
        }
        let state = engine.continue_run().unwrap();
        assert!(
            matches!(state, DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint)),
            "{state:?}"
        );
        assert!(
            engine
                .ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "marker" && v == "c b")
        );
        let state = engine.continue_run().unwrap();
        assert!(
            matches!(state, DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint)),
            "{state:?}"
        );
        let second = if runtime == RuntimeMode::Hybrid {
            "c a"
        } else {
            "b a"
        };
        assert!(
            engine
                .ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "marker" && v == second)
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
    }
}
