#![cfg(all(feature = "host", feature = "debug"))]
//! Native value operations checked against embedded Tcl: every script must
//! compile for vm-strict and give Tcl's result there and in hybrid mode.

use tcld_compile::{CompileMode, compile_source};
use tcld_vm::{DebugOutcome, HybridEngine, RuntimeMode};

fn run(source: &str, mode: RuntimeMode) -> Result<String, String> {
    let compile_mode = match mode {
        RuntimeMode::VmStrict => CompileMode::VmStrict,
        _ => CompileMode::Hybrid,
    };
    let image = compile_source("/workspace/native.tcl".into(), source.into(), compile_mode)
        .map_err(|error| format!("compile: {}", error.reason))?;
    let mut engine = HybridEngine::new(mode).map_err(|error| error.to_string())?;
    if mode != RuntimeMode::VmStrict {
        engine
            .bootstrap(Some("tcld"))
            .map_err(|error| error.to_string())?;
    }
    match engine
        .debug_start_image(&image, false)
        .map_err(|error| error.to_string())?
        .outcome
    {
        DebugOutcome::Finished { result } => result.as_string().map_err(|e| e.to_string()),
        other => Err(format!("{other:?}")),
    }
}

fn oracle(source: &str) -> String {
    tcld_interp::Interpreter::find_executable(Some("tcld"));
    tcld_interp::Interpreter::new()
        .unwrap()
        .eval(source)
        .unwrap_or_else(|error| panic!("{source}: Tcl raised {error}"))
}

/// Like `assert_matches_tcl`, for results Tcl lists in hash-table order.
fn assert_matches_tcl_unordered(sources: &[&str]) {
    let sorted = |text: &str| {
        let mut items = tcld_vm::list_codec::TclListCodec::parse(text.as_bytes()).unwrap();
        items.sort();
        items
    };
    for source in sources {
        let expected = sorted(&oracle(source));
        for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            let actual =
                run(source, mode).unwrap_or_else(|error| panic!("{mode:?}: {source}: {error}"));
            assert_eq!(sorted(&actual), expected, "{mode:?}: {source}");
        }
    }
}

fn assert_matches_tcl(sources: &[&str]) {
    for source in sources {
        let expected = oracle(source);
        for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            assert_eq!(
                run(source, mode),
                Ok(expected.clone()),
                "{mode:?}: {source}"
            );
        }
    }
}

#[test]
fn numbers_and_math_functions_match_tcl() {
    assert_matches_tcl(&[
        "expr {7 / 2}",
        "expr {-7 / 2}",
        "expr {-7 % 2}",
        "expr {2 ** 70}",
        "expr {9223372036854775807 + 1}",
        "expr {0x10 + 010 + 0b11 + 0o7}",
        "expr {1e16}",
        "expr {1.0 / 3}",
        "expr {1 << 3}",
        "expr {~5 & 0xff}",
        "expr {\"10\" == 10.0}",
        "expr {\"abc\" < \"abd\"}",
        "expr {!0 && 1 || 0}",
        "expr {abs(-3)}",
        "expr {abs(0x10)}",
        "expr {abs(-0)}",
        "expr {abs(-2.5)}",
        "expr {abs(-9223372036854775808)}",
        "expr {sqrt(2)}",
        "expr {sqrt(16)}",
        "expr {sin(0.5)}",
        "expr {cos(1)}",
        "expr {tan(0.7)}",
        "expr {exp(1)}",
        "expr {log(10)}",
        "expr {log10(1000)}",
        "expr {pow(2, 10)}",
        "expr {fmod(7.5, 2)}",
        "expr {hypot(3, 4)}",
        "expr {atan2(1, 1)}",
        "expr {acos(0.5) + asin(0.5) + atan(1)}",
        "expr {sinh(1) + cosh(1) + tanh(0.5)}",
        "expr {ceil(1.2)}",
        "expr {floor(-1.2)}",
        "expr {ceil(5)}",
        "expr {double(3)}",
        "expr {double(2**70)}",
        "expr {int(3.9)}",
        "expr {int(-3.9)}",
        "expr {int(1e20)}",
        "expr {wide(2**64+5)}",
        "expr {round(2.5)}",
        "expr {round(-2.5)}",
        "expr {round(1e20+0.5)}",
        "expr {isqrt(10**20 + 1)}",
        "expr {max(1, 2.5, -3)}",
        "expr {min(4, 2)}",
        "expr {bool(\"yes\")}",
        "expr {entier(1e20)}",
        "expr {exp(1000)}",
        "expr {log(0)}",
        "expr {isnan(0.0) + isinf(1e999) + isfinite(1.0) + isnormal(1.0)}",
        "expr {srand(42)}",
        "expr {srand(42)}; expr {rand()}",
        "set x -1; catch {expr {sqrt($x)}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set x abc; catch {expr {abs($x)}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set x abc; catch {expr {sin($x)}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set x {1 2}; catch {expr {sin($x)}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "catch {expr {int(Inf)}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "catch {expr {sqrt(NaN)}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "catch {expr {1 / 0}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set s x; catch {expr {$s + 1}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "catch {expr {1 << -1}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "catch {expr {abs(1, 2)}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "proc f {a b} {expr {$a * $b + 1}}; f 6 7",
    ]);
}

#[test]
fn string_subcommands_match_tcl() {
    assert_matches_tcl(&[
        "string length héllo",
        "set s héllo; string length $s",
        "string len abc",
        "string index abcdef 2",
        "set s abcdef; string index $s end",
        "string index abcdef end-1",
        "string index abc 5",
        "string index abc -1",
        "string index abc 1+1",
        "set code [catch {string index abc foo} msg]; set _ \"$code $msg\"",
        "string range abcdef 1 3",
        "string range abcdef 2 end",
        "string range abcdef -5 1",
        "string range abcdef end-2 end+5",
        "string range abc 2 1",
        "set code [catch {string range abc 0 end-x} msg]; set _ \"$code $msg\"",
        "string first b abcabc",
        "string last b abcabc",
        "string first {} abc",
        "string first x abc",
        "string first é héllo",
        "string equal abc abc",
        "string equal abc abd",
        "string compare abc abd",
        "string compare b a",
        "string compare abc ab",
        "string match a*c abbbc",
        "string match {a[b-d]c} acc",
        "string match {a[d-b]c} acc",
        "string match {a\\*} {a*}",
        "string match {a\\*} {ab}",
        "string match -nocase ABC abc",
        "string match -no É* éa",
        "string match ?? é1",
        "string match {a[} a",
        "string match {*[} x",
        "string match {**a} bba",
        "string map {a b} banana",
        "set m {ab {}}; string map {ab {}} abab",
        "string map {{} x} abc",
        "string trim {  hi  }",
        "string trimleft xxhixx x",
        "string trimright xxhixx x",
        "string trim \"\\u3000 hi \\u00a0\\t\"",
        "string trimleft \"\\ufeff\\u200bhi\"",
        "string toupper héllo",
        "string tolower HÉLLO",
        "string totitle hELLO",
        "string totitle ǆemal",
        "string toupper ß",
        "string replace abcdef 1 2 XY",
        "string replace abcdef 1 2",
        "string replace abc 5 6 X",
        "string replace abc -1 0 X",
        "string replace abc 0 end X",
        "string replace {} 0 0 X",
        "string cat a b c",
        "string cat",
        "set x 1; string cat x $x y",
        "proc f {s i} {string index $s $i}; f héllo end",
        "proc f {s} {string toupper [string trim $s]}; f {  abc }",
    ]);
}

#[test]
fn string_is_classes_match_tcl() {
    assert_matches_tcl(&[
        "string is integer 42",
        "string is int 5",
        "string is integer -strict {}",
        "string is integer {}",
        "string is integer 0x1F",
        "string is integer 1.5",
        "string is integer { 42 }",
        "string is wideinteger 99999999999999999999",
        "string is entier 99999999999999999999",
        "string is integer 99999999999999999999",
        "string is double 1e5",
        "string is double abc",
        "string is double {}",
        "string is double -strict {}",
        "string is double nan",
        "string is double 0x10",
        "string is boolean yes",
        "string is boolean maybe",
        "string is boolean {}",
        "string is boolean -strict {}",
        "string is true on",
        "string is true off",
        "string is false off",
        "string is true {}",
        "string is true -strict {}",
        "string is false -s maybe",
        "string is alpha abc",
        "string is alpha ab1",
        "string is alpha héllo",
        "string is digit 123",
        "string is digit {}",
        "string is digit -strict {}",
        "string is space { }",
        "string is upper ABC",
        "string is lower abC",
        "string is xdigit 0fA",
        "string is wordchar a_1",
        "string is punct !",
        "string is control \\x01",
        "string is print { }",
        "string is graph { }",
        "string is ascii é",
        "string is alnum a1",
        "string is list {a b}",
        "string is list \"a {b\"",
        "string is list {}",
        "proc f {v} {string is integer -strict $v}; set _ \"[f 12] [f x] [f {}]\"",
    ]);
}

#[test]
fn dict_subcommands_match_tcl() {
    assert_matches_tcl(&[
        "dict get {a 1 b 2} b",
        "set d {a 1 b {c 3}}; dict get $d b c",
        "set code [catch {dict get {a 1} x} msg]; set _ \"$code $msg\"",
        "set code [catch {dict get {a 1 b} a} msg]; set _ \"$code $msg\"",
        "set v \"a \\{b\"; set code [catch {dict get $v a} msg]; set _ \"$code $msg\"",
        "set code [catch {dict get {a {b}c} a} msg]; set _ \"$code $msg\"",
        "set code [catch {dict get {a 1} a b} msg]; set _ \"$code $msg\"",
        "dict get {a 1 a 2} a",
        "dict exists {a 1} a",
        "dict exists {a 1} b",
        "dict exists {a {b 1}} a b",
        "set v \"a \\{b\"; dict exists $v a",
        "dict exists {a 1 b} a",
        "dict exists {a 1} a b",
        "set d {}; dict set d a 1; dict set d b c 2; set d",
        "dict set fresh x {two words}",
        "set d {a 1 b 2 a 3}; dict set d c 4",
        "set d {a {x 1}}; dict set d a y 2",
        "set d {a  1}; dict set d a 1",
        "set d {a 1}; set code [catch {dict set d a b 2} msg]; set _ \"$code $msg\"",
        "set d {a 1 b 2}; dict unset d a; set d",
        "set d {a {b 1 c 2}}; dict unset d a b; set d",
        "set d {a 1}; dict unset d zz",
        "set d {a 1}; set code [catch {dict unset d q x} msg]; set _ \"$code $msg\"",
        "set d {}; dict incr d n; dict incr d n 5; dict incr d m -2; set d",
        "set d {n 1.5}; set code [catch {dict incr d n} msg]; set _ \"$code $msg\"",
        "set d {n abc}; set code [catch {dict incr d n} msg]; set _ \"$code $msg\"",
        "set d {n 9223372036854775807}; dict incr d n",
        "set d {n 0x10}; dict incr d n 0x10",
        "set d {}; dict append d s ab; dict append d s c d e; set d",
        "set d {}; dict lappend d l {x y}; dict lappend d l z; set d",
        "set d {l \\\"a\\ \\{b}; set code [catch {dict lappend d l z} msg]; set _ \"$code $msg\"",
        "dict create a 1 b {two words}",
        "set x 5; dict create a $x b [expr {$x*2}]",
        "dict create",
        "dict create # x",
        "string is dict {a 1}",
        "string is dict {a}",
        "set v \"a \\{b\"; string is dict $v",
        "catch {dict get {a 1} x} m o; dict get $o -errorcode",
        "set v \"a \\{b\"; catch {dict get $v x} m o; dict get $o -errorcode",
        "catch {dict get {a b c} x} m o; dict get $o -errorcode",
        "catch {string index abc foo} m o; dict get $o -errorcode",
        "proc f {} {set d {}; dict set d k v; dict incr d n; dict lappend d l 1; dict append d k 2; return $d}; f",
        "proc f {d} {dict get $d k}; f {k {v w}}",
        "proc f {} {dict set d a b c 1; dict unset d a b c; set d}; f",
    ]);
}

#[test]
fn dict_iteration_and_update_match_tcl() {
    assert_matches_tcl(&[
        "set out {}; dict for {k v} {a 1 b 2} {append out $k=$v,}; set out",
        "set n 0; dict for {k v} {a 1 b 2 c 3} {if {$k eq \"b\"} break; incr n}; set n",
        "set n 0; dict for {k v} {a 1 b 2 c 3} {if {$k eq \"b\"} continue; incr n $v}; set n",
        "set out none; dict for {k v} {} {set out ran}; set out",
        "set code [catch {dict for {k v} {a 1 b} {}} msg]; set _ \"$code $msg\"",
        "set code [catch {dict for {k v} {a 1 b 2} {expr {1/0}}} msg]; set _ \"$code $msg $k\"",
        "proc f {d} {dict for {k v} $d {if {$v > 1} {return $k}}; return none}; f {a 1 b 2 c 3}",
        "proc f {d} {set r {}; dict for {k v} $d {dict for {k2 v2} $v {append r $k.$k2=$v2 }}; set r}; f {x {a 1 b 2} y {c 3}}",
        "set d {a 1 b 2}; dict for {k v} $d {dict set d $k [expr {$v * 10}]}; set d",
        "set d {a 1 b 2}; dict update d a x b y {incr x; set y 20; set z 3}; set d",
        "set d {a 1}; dict update d a x b y {unset x; set y new}; set d",
        "set d {a 1}; set code [catch {dict update d a x {set x 5; expr {1/0}}} msg]; set _ \"$code $msg $d\"",
        "proc f {} {set d {a 1}; dict update d a x {return [incr x]}}; f",
        "set d {a 1}; set r [dict update d a x {set x 9; expr {$x + 1}}]; set _ \"$r $d\"",
        "set n 0; foreach i {1 2 3} {set d {a 1}; dict update d a x {if {$i == 2} continue; incr n}}; set n",
        "set code [catch {dict update nosuch a x {}} msg]; set _ \"$code $msg\"",
        "proc f {} {set d {k {a 1}}; dict update d k inner {dict set inner b 2}; set d}; f",
    ]);
}

#[test]
fn clock_readings_are_native_and_consistent() {
    for source in [
        "expr {[clock seconds] > 1700000000}",
        "expr {abs([clock milliseconds] / 1000 - [clock seconds]) <= 1}",
        "expr {abs([clock microseconds] / 1000000 - [clock seconds]) <= 1}",
        "expr {[clock clicks] > 0 && [clock clicks -milliseconds] > 0 && [clock clicks -micro] > 0}",
    ] {
        for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            assert_eq!(run(source, mode), Ok("1".into()), "{mode:?}: {source}");
        }
    }
}

#[test]
fn list_commands_and_error_match_tcl() {
    assert_matches_tcl(&[
        "list a {b c} \"d e\"",
        "list",
        "list {} {#x} \\$y {a\\\\}",
        "set x {1 2}; list $x [list $x]",
        "llength {a b {c d}}",
        "llength {}",
        "llength {  }",
        "set v \"a \\{b\"; set code [catch {llength $v} msg]; set _ \"$code $msg\"",
        "set code [catch {llength a b} msg]; set _ \"$code $msg\"",
        "lindex {a b c} 1",
        "lindex {a b c} end",
        "lindex {a b c} end-1",
        "lindex {a b c} 5",
        "lindex {a b c} -1",
        "lindex {a {b {c d}}} 1 1 0",
        "lindex {a {b {c d}}} {1 1 1}",
        "lindex {a b c}",
        "lindex {a b c} {}",
        "set i 1; lindex {a b c} $i+1",
        "set code [catch {lindex {a b c} x} msg]; set _ \"$code $msg\"",
        "set code [catch {lindex {a b c} 9 x} msg]; set _ \"$code $msg\"",
        "set code [catch {lindex} msg]; set _ \"$code $msg\"",
        "set v \"a \\{b\"; lindex $v",
        "lrange {a b c d e} 1 3",
        "lrange {a b c d e} end-2 end",
        "lrange {a b c d e} 3 1",
        "lrange {a  b   c} 0 end",
        "set f 1; set l 2; lrange {a b c d} $f $l",
        "set f end-1; lrange {a b c d} $f end",
        "lrange {a b c d} -5 1",
        "lrange {a b c d} 2 99",
        "lrange {} 0 end",
        "set code [catch {lrange {a b} 0} msg]; set _ \"$code $msg\"",
        "set code [catch {lrange {a b} x 1} msg]; set _ \"$code $msg\"",
        "concat a {b c} {  d  }",
        "concat \"a  b\" c",
        "concat",
        "concat {} {}",
        "concat [list x] [list #a]",
        "concat [list a b] {  c  }",
        "set code [catch {error boom} msg]; set _ \"$code $msg\"",
        "catch {error boom} m o; dict get $o -errorcode",
        "catch {error boom info {MY CODE}} m o; dict get $o -errorcode",
        "catch {error boom info {MY CODE}} m o; string range [dict get $o -errorinfo] 0 3",
        "set code [catch {error} msg]; set _ \"$code $msg\"",
        "proc f {} {error inner}; set code [catch f msg]; set _ \"$code $msg\"",
        "set args {b c}; list a {*}$args d",
        "set parts {{x y} z}; concat {*}$parts",
        "set path {1 0}; lindex {a {b c}} {*}$path",
        "set range {1 2}; lrange {a b c d} {*}$range",
        "proc f {args} {llength $args}; f 1 2 3",
        "proc f {l} {set r {}; foreach i [lrange $l 1 end] {lappend r [lindex $i 0]}; set r}; f {{a 1} {b 2} {c 3}}",
    ]);
}

#[test]
fn runtime_source_matches_tcl() {
    assert_matches_tcl(&[
        // Unbraced expressions substitute before parsing.
        "set x 5; expr $x % 2",
        "set a 3; set b 4; expr $a * $b + 1",
        "set op +; expr 2 $op 3",
        "set e {1 + 2}; expr $e * 2",
        "proc f {n} {expr $n * $n}; f 7",
        // eval
        "eval set y 7; set y",
        "eval {set z [expr {2*3}]}; set z",
        "set cmd {llength {a b c}}; eval $cmd",
        "eval [list set w {a b}]; set w",
        "eval lappend l a b; eval lappend l {c d}; set l",
        "set script {expr {1/0}}; set code [catch $script msg]; set _ \"$code $msg\"",
        // Procedures created at runtime.
        "set body {return [expr {$x * 2}]}; proc dbl {x} $body; dbl 21",
        "proc mk {name} {proc $name {} [list return $name]}; mk hello; hello",
        "set formals {a {b 2}}; proc f $formals {list $a $b}; f 1",
        "proc outer {} {proc inner {} {return in}; inner}; outer",
        "foreach n {one two} {proc $n {} [list return $n]}; list [one] [two]",
        // Namespaces.
        "namespace eval ns {variable v 3; proc get {} {variable v; return $v}}; ns::get",
        "namespace eval ns {set x 1}; set ns::x",
        "namespace eval ::a::b {proc f {} {namespace current}}; a::b::f",
        "namespace eval ns {namespace current}",
        "namespace eval ns {variable v 1}; namespace inscope ::ns {set v}",
        "namespace eval ns {namespace eval inner {variable d 4}}; set ns::inner::d",
        "namespace eval ns {variable c 0; proc bump {} {variable c; incr c}}; ns::bump; ns::bump; set ns::c",
        "namespace eval ns {proc a {} {b}; proc b {} {return b-in-ns}}; ns::a",
        "namespace eval ns [list set val 5]; set ns::val",
        "namespace eval ns {variable items {a b}}; llength $ns::items",
        "set code [catch {namespace inscope ::nope {set x}} msg]; set code",
        // Global and caller scopes.
        "set g 1; proc f {} {global g; incr g}; f; set g",
        "proc f {} {upvar #0 h hh; set hh 9}; f; set h",
        "proc f {} {set ::q 4}; f; set q",
        "set a(1) x; proc f {} {global a; set a(1)}; f",
        "proc f {} {uplevel 1 {set u 5}}; f; set u",
        "proc g {} {set loc 1; h; set loc}; proc h {} {uplevel 1 {incr loc}}; g",
        "proc f {} {uplevel #0 {set top 1}}; f; set top",
        "proc f {} {global missing; set missing}; set code [catch f msg]; set _ \"$code $msg\"",
        // A procedure does not see globals it has not linked.
        "set g 1; proc f {} {set g}; set code [catch f msg]; set _ \"$code $msg\"",
        "set g 1; proc f {} {unset g}; set code [catch f msg]; set _ \"$code $msg $g\"",
        // Control commands with computed bodies or conditions.
        "set body {incr n}; set n 0; for {set i 0} {$i < 3} {incr i} $body; set n",
        "set c {$i < 3}; set i 0; while $c {incr i}; set i",
        "set cond {$n > 2}; set n 5; if $cond {set r big} else {set r small}",
        "set body {lappend out $x}; set out {}; foreach x {a b c} $body; set out",
        // A backslash-newline between words is a line continuation.
        "set out {}; for {set i 0} \\\n    {$i < 3} {incr i} {lappend out $i}; set out",
        "set x [list a \\\n b]; llength $x",
        // Computed command names.
        "set cmd llength; $cmd {a b}",
        "set c set; $c v 3; set v",
        "proc p {} {return pp}; set c p; $c",
        "set code [catch {nosuchcmd} msg]; set _ \"$code $msg\"",
        "catch {nosuchcmd} m o; dict get $o -errorcode",
    ]);
}

#[test]
fn vm_strict_generated_code_cannot_reach_the_host() {
    for (source, expected) in [
        (
            "catch {eval {open x}} msg; set msg",
            "\"open\" requires WorldHost",
        ),
        (
            "catch {namespace eval ns {exec ls}} msg; set msg",
            "\"exec\" requires WorldHost",
        ),
        // A generated procedure is compiled, and refused, when it is defined.
        (
            "set body {file delete x}; catch {proc f {} $body} msg; set msg",
            "\"file\" requires WorldHost",
        ),
    ] {
        let result = run(source, RuntimeMode::VmStrict).unwrap();
        assert!(result.contains(expected), "{source}: {result}");
    }
    let computed = run("set c open; $c x", RuntimeMode::VmStrict).unwrap_err();
    assert!(computed.contains("denied"), "{computed}");
}

#[test]
fn info_and_rename_match_tcl() {
    assert_matches_tcl(&[
        // info exists
        "set x 1; info exists x",
        "info exists nosuch",
        "proc f {} {info exists y}; f",
        "proc f {} {set y 1; info exists y}; f",
        "set a(1) 2; list [info exists a] [info exists a(1)] [info exists a(2)]",
        "set x 1; namespace eval ns {info exists x}",
        "namespace eval ns {variable v 1}; namespace eval ns {info exists v}",
        "proc f {} {global g; info exists g}; f",
        "set g 1; proc f {} {global g; info exists g}; f",
        "set code [catch {info exists} msg]; set _ \"$code $msg\"",
        "set x 1; namespace eval ns {catch {set x} r; set r}",
        // info args and info body
        "proc foo {a {b 2} args} {return x}; info args foo",
        "proc foo {} {return x}; info body foo",
        "proc foo {a {b 2}} {expr {$a + $b}}; list [info args foo] [info body foo]",
        "namespace eval ns {proc p {x} {return $x}}; list [info args ns::p] [info body ::ns::p]",
        "set body {return [list $x]}; proc gen {x} $body; info body gen",
        "set code [catch {info body nope} msg]; set _ \"$code $msg\"",
        "catch {info args nope} m o; dict get $o -errorcode",
        // info level
        "info level",
        "proc foo {} {info level}; foo",
        "proc foo {a} {info level 0}; foo 1",
        "proc foo {a} {bar $a}; proc bar {b} {info level 1}; foo 9",
        "proc foo {a} {bar $a}; proc bar {b} {info level -1}; foo 9",
        "proc foo {a} {bar $a}; proc bar {b} {info level 0}; foo 9",
        "namespace eval ns {info level}",
        "namespace eval ns {info level 0}",
        "set code [catch {info level 0} msg]; set _ \"$code $msg\"",
        "set code [catch {info level 5} msg]; set _ \"$code $msg\"",
        "set code [catch {info level x} msg]; set _ \"$code $msg\"",
        "catch {info level 5} m o; dict get $o -errorcode",
        // info commands and info procs with one match
        "proc foo {} {}; info commands foo",
        "proc foo {} {}; info procs foo",
        "info commands set",
        "info procs set",
        "info commands nosuch*",
        "namespace eval ns {proc p {} {}}; info commands ns::*",
        "namespace eval ns {proc p {} {}}; info procs ::ns::p",
        "namespace eval ns {proc p {} {}}; namespace eval ns {info procs}",
        "namespace eval ns {proc p {} {}}; namespace eval ns {info commands p}",
        "namespace eval ns {proc p {} {}}; namespace eval ns {info commands llength}",
        "info commands nope::*",
        // rename
        "proc foo {} {return 1}; rename foo bar; bar",
        "proc foo {} {return 1}; rename foo {}; info commands foo",
        "proc foo {} {return 1}; rename foo bar; set code [catch foo msg]; set _ \"$code $msg\"",
        "proc foo {} {return f}; rename foo ::ns3::bar; ns3::bar",
        "namespace eval ns {proc p {} {return p}}; rename ns::p ns::q; ns::q",
        "set code [catch {rename nope x} msg]; set _ \"$code $msg\"",
        "set code [catch {rename nope {}} msg]; set _ \"$code $msg\"",
        "proc a {} {}; proc b {} {}; set code [catch {rename a b} msg]; set _ \"$code $msg\"",
        "catch {rename nope x} m o; dict get $o -errorcode",
    ]);
}

#[test]
fn info_listings_match_tcl() {
    assert_matches_tcl_unordered(&[
        "proc my1 {} {}; proc my2 {} {}; info procs my*",
        "proc my1 {} {}; proc my2 {} {}; info commands my?",
        "namespace eval ns {proc x {} {}; proc y {} {}}; info commands ::ns::*",
        "namespace eval ns {proc x {} {}; proc y {} {}}; namespace eval ns {info procs}",
        "namespace eval ns {proc lindex2 {} {}}; namespace eval ns {info commands lindex*}",
        "namespace eval ns {proc x {} {}}; namespace eval ns::inner {proc y {} {}}; info procs ::ns::*",
    ]);
}

#[test]
fn lsearch_matches_tcl() {
    assert_matches_tcl(&[
        "lsearch {a b c} b",
        "lsearch {a b c} z",
        "lsearch {apple banana cherry} *an*",
        "lsearch -all {a b a c a} a",
        "lsearch -all -inline {a1 b2 a3} a*",
        "lsearch -inline {a1 b2 a3} b*",
        "lsearch -inline {a1 b2} z*",
        "lsearch -exact {a* b} a*",
        "lsearch -glob {a* b} a*",
        "lsearch -exact -nocase {Ab aB ab} AB",
        "lsearch -nocase {HELLO world} hello",
        "lsearch -not {a a b} a",
        "lsearch -all -not {a b a c} a",
        "lsearch -start 2 {a b a b} a",
        "lsearch -start end {a b a b} a",
        "lsearch -start 9 {a b} a",
        "lsearch -all -start 9 {a b} a",
        "lsearch -exact -integer {1 02 3} 2",
        "lsearch -exact -real {1.0 2.5 3} 2.50",
        "lsearch -sorted {a c e g} e",
        "lsearch -sorted {a c e g} d",
        "lsearch -sorted -integer {1 3 5 7} 5",
        "lsearch -sorted -decreasing -integer {9 7 5 3} 5",
        "lsearch -bisect -integer {1 3 5 7} 4",
        "lsearch -bisect -integer {1 3 5 7} 0",
        "lsearch -bisect {a c e} e",
        "lsearch -dictionary -sorted {a2 a10 b1} a10",
        "lsearch -exact -dictionary {X1 x01 y} x1",
        "lsearch -sorted -nocase {apple Banana cherry} BANANA",
        "lsearch -index 1 -all {{a 1} {b 2} {c 1}} 1",
        "lsearch -index end {{a 1} {b 2}} 2",
        "lsearch -index 0 -inline {{a 1} {b 2}} b",
        "lsearch -subindices -index 1 {{a 1} {b 2}} 2",
        "lsearch -subindices -index 1 -inline {{a 1} {b 2}} 2",
        "lsearch -subindices -index {1 0} -all {{x {y z}} {p {y q}}} y",
        "lsearch -stride 2 {a 1 b 2} b",
        "lsearch -stride 2 -index 1 {a 1 b 2} 2",
        "lsearch -stride 2 -index 1 -inline {a 1 b 2} 2",
        "lsearch -stride 2 -all -inline {a 1 b 2 a 3} a",
        "lsearch -ex {a b} b",
        "set code [catch {lsearch -bogus {a} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -i {a} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch {} {a} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch {a}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -start {a} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -stride 0 {a b} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -stride 2 {a b c} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -subindices {a} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -bisect -all {a} a} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -exact -integer {1 x} 2} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -index 1 {{a} {b 2}} 2} msg]; set _ \"$code $msg\"",
        "set code [catch {lsearch -index -1 {a} a} msg]; set _ \"$code $msg\"",
        "set v \"a \\{b\"; set code [catch {lsearch $v a} msg]; set _ \"$code $msg\"",
        "proc find {l x} {lsearch -exact $l $x}; find {p q r} r",
    ]);
}

#[test]
fn vm_strict_denies_renaming_tcl_commands() {
    let error = run("set old set; rename $old myset", RuntimeMode::VmStrict).unwrap_err();
    assert!(error.contains("denied"), "{error}");
}

#[test]
fn join_and_split_match_tcl() {
    assert_matches_tcl(&[
        "join {a b c}",
        "join {a b c} -",
        "join {a {b c} d} {, }",
        "join {}",
        "join {{a b}}",
        "join [list x y] {}",
        "join {1 2 3} [list + ]",
        "split {a b  c}",
        "split a,b,,c ,",
        "split {a,b;c} {,;}",
        "split abc {}",
        "split {a b} {}",
        "split {} ,",
        "split \"a\\nb\\tc\\rd e\"",
        "set e \\u00e9; split \"xa${e}z$e\" $e",
        "llength [split {,,} ,]",
        "set code [catch {join} msg]; set _ \"$code $msg\"",
        "set code [catch {join a b c} msg]; set _ \"$code $msg\"",
        "set code [catch {split} msg]; set _ \"$code $msg\"",
        "catch {join \"a \\{b\"} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "proc words {s} {join [split $s] _}; words {a b c}",
    ]);
}

#[test]
fn string_runtime_subcommands_match_tcl() {
    assert_matches_tcl(&[
        // Subcommands Tcl never compiles.
        "string repeat ab 3",
        "string repeat ab 0",
        "string repeat ab -2",
        "string repeat {} 5",
        "string repeat abc 1",
        "catch {string repeat a b} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "catch {string repeat a 99999999999999999999} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "string reverse abc",
        "string reverse {}",
        "set e \\u00e9; string reverse \"a${e}b\"",
        "string wordend {hello world} 2",
        "string wordend {hello world} 5",
        "string wordend {hello world} 100",
        "string wordend {hello world} -3",
        "string wordend {} 0",
        "string wordend {a_b c} 0",
        "string wordstart {hello world} 8",
        "string wordstart {hello world} 5",
        "string wordstart {hello world} 3",
        "string wordstart {hello world} 0",
        "string wordstart {hello world} end",
        "string wordstart {hello world} 100",
        "string wordstart {} 3",
        "string wordstart {a_b c} 2",
        "string insert abc 1 X",
        "string insert abc end X",
        "string insert abc end-1 X",
        "string insert abc -5 X",
        "string insert abc 10 X",
        "string insert {} 0 X",
        // Forms Tcl compiles only for some arguments.
        "string first an banana 2",
        "string first na banana end-2",
        "string first a banana 10",
        "string first a banana -5",
        "string last an banana 2",
        "string last a banana end-3",
        "string last a banana -1",
        "string last a banana 100",
        "string compare -nocase ABC abc",
        "string compare -length 2 abx aby",
        "string compare -length 3 ab abc",
        "string compare -length -1 ab abc",
        "string compare -nocase -length 3 ABCd abcE",
        "string compare -length 0 a b",
        "string equal -nocase Hello hello",
        "string equal -length 3 abcx abcy",
        "string equal -n -l 2 ABx aby",
        "string map {a 1 b 2} abcab",
        "string map {ab X a Y} aabab",
        "string map -nocase {A x} aAbA",
        "string map -nocase {AB 1 b 2} abAbB",
        "string map {} abc",
        "string map {{} x a y} aa",
        "string map {a b} {}",
        "string map [dict create a 1 a 2] aa",
        "set m {a b}; string map $m a",
        "string match -nocase A* abc",
        "string match -noc {[A-C]?} bx",
        "string tolower ABCDEF 1 3",
        "string toupper abcdef 2",
        "string toupper abcdef end-1 end",
        "string toupper abc 5 6",
        "string tolower {} 0 1",
        "string totitle {hello WORLD} 6 end",
        "string totitle {hello WORLD} 0",
        "string totitle abc 2 0",
        "string trim xxabcxx x",
        "string trimleft xxabc xy",
        "string trimright abcyy y",
        // Errors.
        "set code [catch {string} msg]; set _ \"$code $msg\"",
        "catch {string foo a} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string t abc} msg]; set _ \"$code $msg\"",
        "set code [catch {string fir a} msg]; set _ \"$code $msg\"",
        "set code [catch {string first a b c d} msg]; set _ \"$code $msg\"",
        "set code [catch {string last a} msg]; set _ \"$code $msg\"",
        "set code [catch {string reverse} msg]; set _ \"$code $msg\"",
        "set code [catch {string repeat a} msg]; set _ \"$code $msg\"",
        "set code [catch {string wordend abc} msg]; set _ \"$code $msg\"",
        "set code [catch {string insert abc 1} msg]; set _ \"$code $msg\"",
        "set code [catch {string compare a} msg]; set _ \"$code $msg\"",
        "catch {string compare -foo a b} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string compare -length a b} msg]; set _ \"$code $msg\"",
        "set code [catch {string equal -length x a b} msg]; set _ \"$code $msg\"",
        "catch {string map {a} abc} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "catch {string map -foo {a b} abc} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string map \"a \\{\" abc} msg]; set _ \"$code $msg\"",
        "catch {string match -x a b} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string tolower a b c d} msg]; set _ \"$code $msg\"",
        "catch {string index abc foo} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string toupper abc x} msg]; set _ \"$code $msg\"",
        "set code [catch {string trim a b c} msg]; set _ \"$code $msg\"",
        "set code [catch {::string reverse} msg]; set _ \"$code $msg\"",
        // Every subcommand through a computed subcommand, which Tcl and the
        // compiler leave to runtime.
        "set r {}
         foreach {sub arguments} {
            cat {a b c} compare {abc abd} equal {a a} first {b abc} index {abc 1}
            insert {abc 1 X} is {digit 12} last {b abcb} length {abc} map {{a b} aa}
            match {a* abc} range {abcdef 1 3} repeat {ab 2} replace {abc 1 1 X}
            reverse {abc} tolower {ABC} totitle {aBC} toupper {abc} trim {{ a }}
            trimleft {{ a }} trimright {{ a }} wordend {{ab cd} 0} wordstart {{ab cd} 4}
         } {lappend r [string $sub {*}$arguments]}
         set r",
        "set sub len; string $sub abcd",
        "string index abc 0x1",
        "string last a abc -99999999999999999999",
        "string map {{a b} X} {a b c}",
        "string trim \"\\u3000 \\u00a0x\\t\"",
        "string compare -nocase \\u00c9 \\u00e9",
        "string totitle \\u00e9T\\u00c9 1 end",
        "catch {string repeat a [string repeat x 60]} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string equal -length 99999999999999999999 a a} msg]; set _ \"$code $msg\"",
        "proc rev {s} {string reverse $s}; rev hello",
    ]);
}

#[test]
fn string_is_matches_tcl() {
    assert_matches_tcl(&[
        "string is digit -failindex i 12a4; set i",
        "set i untouched; string is digit -failindex i 123; set i",
        "string is alpha -strict {}",
        "string is alpha {}",
        "string is alpha -strict -failindex i {}; set i",
        "string is integer -failindex i { 12x}; set i",
        "string is integer -failindex i {x12}; set i",
        "string is integer 99999999999999999999",
        "string is integer { 12 }",
        "string is integer 1.0",
        "string is int -strict 7",
        "string is entier 12345678901234567890",
        "string is wideinteger 99999999999999999999",
        "string is wideinteger 9223372036854775807",
        "string is wideinteger -failindex i 99999999999999999999; set i",
        "string is wideinteger -failindex i 12x; set i",
        "string is wideinteger -failindex i x; set i",
        "string is double -failindex i 1.5e3x; set i",
        "string is double -failindex i {1e+}; set i",
        "string is double 1e5",
        "string is double 0x1F",
        "string is double -strict {}",
        "string is double inf",
        "string is boolean yes",
        "string is boolean 2",
        "string is boolean {}",
        "string is boolean -strict {}",
        "string is true -strict no",
        "string is true on",
        "string is false off",
        "string is false 1",
        "string is boolean -failindex i maybe; set i",
        "string is list {a b {c d}}",
        "string is list -strict {}",
        "string is list -failindex i {a {b c}x d}; set i",
        "string is list -failindex i \"a \\{b\"; set i",
        "string is dict {a 1 b 2}",
        "string is dict {}",
        "string is dict -failindex i {a b c}; set i",
        "string is dict -failindex i {a {b}x c}; set i",
        "string is xdigit -failindex i 0fAz; set i",
        "string is space \" \\t\\n\"",
        "string is upper ABC",
        "string is lower abC",
        "string is wordchar a_1",
        "string is control \"\\x01\\x02\"",
        "string is ascii abc",
        "set e \\u00e9; string is ascii -failindex i a$e; set i",
        "set e \\u00e9; string is alpha a$e",
        "string is graph -failindex i {ab c}; set i",
        "string is print {ab c}",
        "string is punct {.,;}",
        "string is alnum a1",
        "string is digit -failindex a(x) 1z; set a(x)",
        "set code [catch {string is} msg]; set _ \"$code $msg\"",
        "set code [catch {string is alpha} msg]; set _ \"$code $msg\"",
        "catch {string is foo x} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string is al x} msg]; set _ \"$code $msg\"",
        "set code [catch {string is {} x} msg]; set _ \"$code $msg\"",
        "string is alpha -strict",
        "catch {string is alpha -bogus x} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {string is alpha -failindex x} msg]; set _ \"$code $msg\"",
        "set code [catch {string is upp -failindex x} msg]; set _ \"$code $msg\"",
        "set code [catch {string is alpha -strict -strict -strict x} msg]; set _ \"$code $msg\"",
        "set c digit; string is $c -failindex i 1x; set i",
        "string is integer 1_000",
        "list [string is integer 1__0] [string is integer _1] [string is integer 0x_1]",
        "string is integer -failindex i 1_; set i",
        "string is double -failindex i {  }; set i",
        "string is double {1.5 }",
        "string is double -failindex i .; set i",
        "string is integer -failindex i 0x; set i",
        "string is integer -failindex i -; set i",
        "string is integer -failindex i +5x; set i",
        "string is integer -failindex i {12 34}; set i",
        "string is wideinteger -failindex i 0x8000000000000000; set i",
        "string is double -failindex i 0b102; set i",
        "string is double -failindex i 1e5e5; set i",
        "string is double -failindex i infx; set i",
        "string is double nan",
        "string is boolean -strict 0x1",
        "list [string is true 1] [string is false 0] [string is true TRUE]",
        "string is list {a \"b c\" d}",
        "string is list -failindex i {a \"b\"c d}; set i",
        "set v \"a 1 b \\{\"; string is dict -failindex i $v; set i",
        "string is digit \\u0663",
    ]);
}

#[test]
fn dict_runtime_subcommands_match_tcl() {
    assert_matches_tcl(&[
        "dict create",
        "dict create a 1 a 2",
        "dict get {a 1 a 2}",
        "dict get {}",
        "dict getdef {a 1} a 0",
        "dict getdef {a 1} b 0",
        "dict getwithdefault {a {b 1}} a b 0",
        "dict getdef {a {b 1}} x b 0",
        "dict getdef {a {b 1}} a c 0",
        "dict keys {a 1 b 2 c 3}",
        "dict keys {a 1 b 2 c 3} b",
        "dict keys {a 1 b 2 c 3} x",
        "dict keys {a 1 ab 2 c 3} a*",
        "dict values {a 1 b 2 c 13}",
        "dict values {a 1 b 2 c 13} 1*",
        "dict size {a 1 b 2}",
        "dict size {}",
        "dict replace {a 1 b 2} b 3 c 4",
        "dict replace {a  1}",
        "dict remove {a 1 b 2 c 3} b x",
        "dict remove {a  1}",
        "dict merge",
        "dict merge {a  1}",
        "dict merge {a 1 b 2} {b 3 c 4} {a 5}",
        "dict filter {a 1 b 2 ab 3} key a*",
        "dict filter {a 1 b 2 ab 3} key b",
        "dict filter {a 1 b 2 ab 3} key a* b",
        "dict filter {a 1 b 2 ab 3} key",
        "dict filter {a 1 b 2 ab 3} value 2",
        "dict filter {a 1 b 2 ab 3} value {[13]}",
        "dict filter {a 1 b 2 ab 3} value",
        "dict filter {a 1 b 2} k a",
        "dict cr a 1",
        // Variables the compiler does not update in place.
        "set ::g {a 1}; dict set ::g b c 2",
        "dict set ::nv a b 1; set ::nv",
        "set ::g {a 1}; dict unset ::g a; set ::g",
        "set ::g {a {b 1 c 2}}; dict unset ::g a b",
        "set ::g {a 1}; dict unset ::g zz",
        "set ::g {a 1}; dict incr ::g a; dict incr ::g b 0x10; set ::g",
        "set ::g {a 1}; dict incr ::g a 9223372036854775807",
        "set ::g {a 1}; dict incr ::g a -3",
        "dict incr ::fresh k",
        "set ::g {a 1}; dict append ::g a x y; dict append ::g b z; dict append ::g c; set ::g",
        "set ::g {a  1}; dict append ::g a",
        "set ::g {a {x  y}}; dict lappend ::g a",
        "set ::g {a {x  y}}; dict lappend ::g a z",
        "set ::g {}; dict lappend ::g k; set ::g",
        "set ::g {}; dict lappend ::g k a {b c}",
        "set ::a(x) {k 1}; dict set ::a(x) k 2; set ::a(x)",
        "namespace eval ns {variable d {a 1}; dict incr d a; set d}",
        // Computed subcommands reach the forms Tcl compiles.
        "set op get; dict $op {a {b 1}} a b",
        "set op get; dict $op {a 1 b 2}",
        "set op exists; dict $op {a {b 1}} a b",
        "set op exists; dict $op {a {b 1}} a c",
        "set op exists; dict $op {a {b} x} a",
        "set op set; set d {}; dict $op d a b 1; set d",
        "set op incr; dict $op ::cnt x; dict $op ::cnt x 5; set ::cnt",
        "set op lappend; dict $op ::l k a b; dict $op ::l k c; set ::l",
        "set op append; dict $op ::s k a b; set ::s",
        "set op unset; set ::u {a 1 b 2}; dict $op ::u a; set ::u",
        "set op create; dict $op a 1 b 2",
        "set path {a b}; set d {}; dict set d {*}$path 1; set d",
        "set keys {x y}; dict get {x {y 7}} {*}$keys",
        "set op get; catch {dict $op {a 1} b} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "dict merge {a {1 2}} {b {3 4}}",
        "dict keys {{a b} 1 c 2} {a b}",
        "dict filter {{a b} 1 c 2} key {a b}",
        "dict values {a {x y} b z} {x*}",
        "proc f {} {set d {a 1}; dict set d b 2; dict keys $d}; f",
        "proc f {} {set d {a 1 b 2}; list [dict size $d] [dict values $d]}; f",
        // Errors.
        "set code [catch {dict} msg]; set _ \"$code $msg\"",
        "catch {dict bogus} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {dict g {a 1}} msg]; set _ \"$code $msg\"",
        "set code [catch {dict create a} msg]; set _ \"$code $msg\"",
        "set code [catch {dict get} msg]; set _ \"$code $msg\"",
        "set code [catch {dict getdef {a 1} a} msg]; set _ \"$code $msg\"",
        "catch {dict getdef {a x} a b 0} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {dict keys} msg]; set _ \"$code $msg\"",
        "catch {dict keys {a}} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {dict size {a 1} x} msg]; set _ \"$code $msg\"",
        "set code [catch {dict replace {a 1} b} msg]; set _ \"$code $msg\"",
        "set code [catch {dict replace} msg]; set _ \"$code $msg\"",
        "set code [catch {dict remove} msg]; set _ \"$code $msg\"",
        "set code [catch {dict merge {a 1} {b}} msg]; set _ \"$code $msg\"",
        "set code [catch {dict merge {a}} msg]; set _ \"$code $msg\"",
        "catch {dict filter {a 1} bogus x} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {dict filter {a 1}} msg]; set _ \"$code $msg\"",
        "set code [catch {dict filter {a} key x} msg]; set _ \"$code $msg\"",
        "set code [catch {dict exists {a 1}} msg]; set _ \"$code $msg\"",
        "set code [catch {set ::g {a 1}; dict unset ::g x y} msg]; set _ \"$code $msg\"",
        "catch {set ::g {a x}; dict incr ::g a} r o; set _ \"$r|[dict get $o -errorcode]\"",
        "set code [catch {set ::g {a 1.5}; dict incr ::g a} msg]; set _ \"$code $msg\"",
        "set code [catch {set ::g {a 1}; dict incr ::g a y} msg]; set _ \"$code $msg\"",
        "set code [catch {set ::g {a 1}; dict incr ::g b 1.5} msg]; set _ \"$code $msg\"",
        "catch {set ::g {a 1}; dict incr ::g b 1.5} r o; string match {*(reading increment)*} [dict get $o -errorinfo]",
        "set code [catch {set ::g {a 1}; dict incr ::g} msg]; set _ \"$code $msg\"",
        "set code [catch {set ::g {a}; dict append ::g a x} msg]; set _ \"$code $msg\"",
        "set code [catch {set ::g \"a \\{\"; dict lappend ::g a z} msg]; set _ \"$code $msg\"",
        "set code [catch {set ::g {a 1}; dict set ::g a b c} msg]; set _ \"$code $msg\"",
        "set code [catch {set ::arr(x) 1; dict set ::arr k v} msg]; set _ \"$code $msg\"",
        "set code [catch {dict set ::g} msg]; set _ \"$code $msg\"",
        "set code [catch {dict lappend ::g} msg]; set _ \"$code $msg\"",
    ]);
}

#[test]
fn dict_map_and_with_match_tcl() {
    assert_matches_tcl(&[
        "dict map {k v} {a 1 b 2} {expr {$v * 2}}",
        "dict map {k v} {} {set v}",
        "dict map {k v} {a 1 a 2} {set v}",
        "dict map {k v} {a 1 b 2 c 3} {if {$k eq {b}} continue; set v}",
        "dict map {k v} {a 1 b 2 c 3} {if {$k eq {b}} break; set v}",
        "proc f {} {dict map {k v} {a 1 b 2 c 3} {if {$k eq {b}} break; set v}}; f",
        "namespace eval ns {dict map {k v} {a 1 b 2} {if {$k eq {b}} break; set v}}",
        "dict map {k v} {a 1 b 2} {set k x$k; set v}",
        "proc f {d} {dict map {k v} $d {string toupper $v}}; f {a x b y}",
        "proc f {} {set n 0; dict map {k v} {a 1 b 2} {incr n}}; f",
        "set code [catch {dict map {k v} {a 1} {error boom}} msg]; set _ \"$code $msg\"",
        "set code [catch {dict map {k v} {a} {set v}} msg]; set _ \"$code $msg\"",
        "set r {}; foreach d {{a 1} {b 2}} {lappend r [dict map {k v} $d {incr v}]}; set r",
        "set d {a 1 b 2}; dict with d {set c [expr {$a + $b}]}; list $c $d",
        "set d {a 1 b 2}; dict with d {set a 10; unset b}; set d",
        "set d {x {a 1}}; dict with d x {set a 2}; set d",
        "set d {a 1}; dict with d {set a}",
        "set d {a 1}; set r [dict with d {expr {$a + 41}}]",
        "proc f {} {set d {a 1}; dict with d {incr a}; set d}; f",
        "proc f {} {set d {p {q 1}}; dict with d p {incr q; set new 5}; set d}; f",
        "proc f {} {set d {a 1}; dict with d {return [expr {$a + 1}]}}; f",
        "proc f {} {set d {a 1}; catch {dict with d {set a 3; error oops}} r; list $r $d}; f",
        "set ::g {a 1}; dict with ::g {set a 5}; set ::g",
        "set ::g {k {a 1}}; dict with ::g k {set a 6}; set ::g",
        "set name d; set d {a 1}; dict with $name {set a 7}; set d",
        "set d {a 1}; dict with d {}; set d",
        "set d {x {a 1}}; dict with d x {}; set d",
        "set ::g {a  1}; dict with ::g {}; set ::g",
        "set ::g {k {a 1}}; dict with ::g k {}",
        "set d {a 1}; dict with d {unset d}; info exists d",
        "set code [catch {set d {a}; dict with d {}} msg]; set _ \"$code $msg\"",
        "set code [catch {dict with nosuch {set x}} msg]; set _ \"$code $msg\"",
        "set code [catch {set d {x 1}; dict with d y {}} msg]; set _ \"$code $msg\"",
        "set d {a 1}; for {set i 0} {$i < 3} {incr i} {dict with d {incr a}}; set d",
        // A key that names an array cannot become a scalar.
        "proc work {} {set a(k) old; set d {a 40}; list [catch {dict with d {error unreachable}} m] $m $a(k)}; work",
        "proc f {} {set a(k) 1; catch {set a 5} m o; list $m [dict get $o -errorcode] $a(k)}; f",
        "proc f {} {set a(k) 1; set n a; catch {set $n 5} m; list $m $a(k)}; f",
        "set a(k) 1; catch {set a 5} m; set m",
        // Compiler temporaries are not variables, even ones named "".
        "set d {x {a 1}}; dict with d x {set a 2}; info exists {}",
        "set d [dict create {} 1 a 2]; dict with d {set {} 5; set a 6}; set d",
        "proc f {} {set d [dict create {} 1 x {a 2}]; dict with d x {set {} 5}; set d}; f",
        "dict map {k v} {a 1} {set {} 3; set v}; set {}",
        "dict map {k v} {a 1 b 2} {dict map {k2 v2} {x 1} {set v2}}",
    ]);
}

#[test]
fn vm_strict_denies_dict_script_forms() {
    for source in [
        "set body {set v}; dict for {k v} {a 1} $body",
        "set body {set v}; dict map {k v} {a 1} $body",
        "set d {a 1}; set body {set a}; dict with d $body",
        "dict filter {a 1} script {k v} {expr 1}",
    ] {
        let error = run(source, RuntimeMode::VmStrict).unwrap_err();
        assert!(
            error.contains("runs script through the Tcl host"),
            "{source}: {error}"
        );
    }
    assert!(
        run("dict info {a 1}", RuntimeMode::VmStrict)
            .unwrap_err()
            .contains("\"dict\" requires WorldHost")
    );
    // A computed subcommand is refused when it runs.
    for source in [
        "set op map; dict $op {k v} {a 1} {set v}",
        "set f script; dict filter {a 1} $f {k v} {expr 1}",
    ] {
        let error = run(source, RuntimeMode::VmStrict).unwrap_err();
        assert!(error.contains("denied"), "{source}: {error}");
        assert_eq!(run(source, RuntimeMode::Hybrid), Ok(oracle(source)));
    }
}

#[test]
fn unquoted_word_syntax_matches_tcl() {
    assert_matches_tcl(&[
        // Braced variable names inside unquoted words.
        "set a 1; set x ${a}b",
        "set a 1; set x pre${a}post",
        "set {a b} 2; set x ${a b}",
        "set {a{b}c} 3; set x ${a{b}c}",
        "set {a\\}b} 4; set x ${a\\}b}",
        "set e \\u00e9; string reverse a${e}b",
        "proc f {} {set a 1; return ${a}x}; f",
        "set a 5; expr {${a} + 1}",
        // Braces, quotes and close brackets are ordinary inside a word.
        "set x a{b}c",
        "set x {a}; set y a{$x}c",
        "set x a\"b",
        "list a\"b c\"",
        "list }",
        "list a]b",
        "list a{b; list c}",
        "set x [list a{]",
        "set x [list a\\{b]",
        "set x \"a\\{b\"",
        "if {1} {set a 2; set x a{$a}b}",
        // Variable references follow Tcl's name and index rules.
        "set a(x\\ y) 5; set z $a(x y)",
        "set a(k) 7; set i k; set x $a($i)",
        "set a(k) 8; set x [list $a([set i k])]",
        "set (k) 9; set x $(k)",
        "namespace eval ns {variable v 10}; set x $ns::v:y",
        "set x $",
        "set x a$",
        "set x $$",
        "set x {$}",
        "set x \"cost: $ 5\"",
        "proc f {} {set a(1) z; return $a(1)}; f",
        // Command substitutions end at their own close bracket.
        "set x [list \"a]b\"]",
        "set x [list {a]b}]",
        "set x [set y 1; set z 2]",
        "set x [\n  set y 3\n]",
        "set c [list a]; set x \"[lindex $c 0]\"",
        "set x [list {*}{a b} c]",
        "set y {a b}; set x [list {*}$y c]",
        "set x a; set y $x[list b]$x",
    ]);
}

#[test]
fn expression_operands_match_tcl() {
    assert_matches_tcl(&[
        // Array elements.
        "set a(k) 7; expr {$a(k) * 2}",
        "set a(b) 7; set i b; expr {$a($i) + 1}",
        "set a(b) 7; expr {$a([set i b]) + 2}",
        "set {a(x y)} 3; expr {$a(x y) + 1}",
        "set a() 4; expr {$a() + 1}",
        "set (k) 5; expr {$(k) * 3}",
        "set a(k) 0x10; expr {$a(k)}",
        "proc f {} {set a(1) 5; expr {$a(1) ** 2}}; f",
        "proc f {k} {set t(x) 10; set t(y) 20; expr {$t($k) / 2}}; list [f x] [f y]",
        "set a(k) 2; if {$a(k) > 1} {set r big} else {set r small}",
        "set a(k) 1; set n 0; while {$a(k) < 4} {incr a(k); incr n}; set n",
        "catch {expr {$nosuch(k)}} m; set m",
        "set a(k) 1; catch {expr {$a + 1}} m; set m",
        // Braced variable names.
        "set {x y} 3; expr {${x y} * 2}",
        "set a(b) 7; expr {${a(b)} + 1}",
        "set x 2; expr {${x}**3}",
        "set ::g 4; expr {$::g + ${::g}}",
        // Quoted strings with substitutions.
        "set v abc; expr {\"$v\" eq \"abc\"}",
        "set x 0x10; expr {\"$x\" eq \"0x10\"}",
        "set x 0x10; expr {\"$x\"}",
        "set x 0x10; expr {\"v=$x\"}",
        "set x 5; expr {\"[set x]\" + 1}",
        "set a(k) 3; expr {\"$a(k)\" * 2}",
        "expr {\"a\\tb\" eq \"a\\tb\"}",
        "set s {}; expr {\"$s\" eq \"\"}",
        "set x 3; expr {\"$x$x\" + 1}",
        "set x 2; set y [expr {\"1$x\" * 2}]",
        "set x 1; expr {\"a{$x}b\"}",
        "set a(k) 1; set r [if {\"$a(k)\" eq \"1\"} {list yes} else {list no}]",
        // Operands keep their strings inside an expression; only its value
        // becomes a number.
        "set x 0x10; expr {$x eq \"0x10\"}",
        "set x 0x10; expr {$x in {0x10 a}}",
        "set x 0x10; set y 16; expr {$x eq $y}",
        "set x 0x10; expr {$x == 16}",
        "set x 0x10; expr {$x}",
        "set x 0x10; expr {1 ? $x : 2}",
        "set x 0x10; expr {max($x, 1)}",
        "set x 0x10; expr {[set x]}",
        "set x 0x10; string length [expr {$x}]",
        // Braced strings are literals.
        "set x 1; expr {{$x} eq \"\\$x\"}",
        "expr {{a{b}c}}",
        "expr {{0x10}}",
        "expr {{a [b] c} eq \"a \\[b\\] c\"}",
        "expr {{a\\\n   b}}",
        "set y {a\\\n   b}; string length $y",
        "proc f {} {set x 1;\\\n    set y 2}; info body f",
    ]);
}

/// A script's result and what it wrote to stdout and stderr, captured: the
/// host's output in hybrid mode, the VM's own in vm-strict.
fn run_capturing(source: &str, mode: RuntimeMode) -> Result<(String, String, String), String> {
    let compile_mode = match mode {
        RuntimeMode::VmStrict => CompileMode::VmStrict,
        _ => CompileMode::Hybrid,
    };
    let image = compile_source("/workspace/native.tcl".into(), source.into(), compile_mode)
        .map_err(|error| format!("compile: {}", error.reason))?;
    let mut engine = HybridEngine::new(mode).map_err(|error| error.to_string())?;
    if mode != RuntimeMode::VmStrict {
        engine
            .bootstrap(Some("tcld"))
            .map_err(|error| error.to_string())?;
    }
    engine
        .begin_output_capture()
        .map_err(|error| error.to_string())?;
    let result = match engine
        .debug_start_image(&image, false)
        .map_err(|error| error.to_string())?
        .outcome
    {
        DebugOutcome::Finished { result } => result.as_string().map_err(|e| e.to_string())?,
        other => return Err(format!("{other:?}")),
    };
    let (stdout, stderr) = engine
        .captured_output()
        .map_err(|error| error.to_string())?;
    Ok((
        result,
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    ))
}

/// Tcl's result and output for a script, captured beneath `puts` by
/// transforms stacked on stdout and stderr.
fn oracle_capturing(source: &str) -> (String, String, String) {
    let wrapper = format!(
        r#"namespace eval ::capture {{
    variable stdout {{}} stderr {{}}
    proc transform {{channel command handle args}} {{
        if {{$command eq "initialize"}} {{return {{initialize finalize write}}}}
        if {{$command eq "write"}} {{append ::capture::$channel [lindex $args 0]}}
        return
    }}
}}
foreach channel {{stdout stderr}} {{
    chan configure $channel -encoding utf-8 -translation lf
    chan push $channel [list ::capture::transform $channel]
}}
set ::capture::code [catch {{uplevel #0 {script}}} ::capture::result]
foreach channel {{stdout stderr}} {{
    flush $channel
    chan pop $channel
}}
list $::capture::code $::capture::result \
    [encoding convertfrom utf-8 $::capture::stdout] \
    [encoding convertfrom utf-8 $::capture::stderr]"#,
        script = tcld_compile::quote_word(source)
    );
    let fields = tcld_vm::list_codec::TclListCodec::parse(oracle(&wrapper).as_bytes()).unwrap();
    let field = |index: usize| String::from_utf8(fields[index].clone()).unwrap();
    assert_eq!(field(0), "0", "{source}: Tcl raised {}", field(1));
    (field(1), field(2), field(3))
}

fn assert_output_matches_tcl(sources: &[&str]) {
    for source in sources {
        let expected = oracle_capturing(source);
        for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            assert_eq!(
                run_capturing(source, mode),
                Ok(expected.clone()),
                "{mode:?}: {source}"
            );
        }
    }
}

#[test]
fn puts_output_matches_tcl() {
    assert_output_matches_tcl(&[
        "puts hello",
        "puts -nonewline hello; puts \" world\"",
        "puts stdout out; puts stderr err",
        "puts -nonewline stderr e1; puts stderr e2",
        "puts -nonewline stdout {}; puts stdout end",
        "puts {}",
        "puts \"a\\nb\"",
        "puts \"\\u00e9t\\u00e9 \\u263a\"",
        "set x [list a {b c}]; puts $x",
        "puts [expr {1 + 2}]",
        "proc greet {n} {puts \"hi $n\"}; greet bob; greet amy",
        "for {set i 0} {$i < 3} {incr i} {puts -nonewline $i}; puts {}",
        "set r [puts x]; puts \"<$r>\"",
        "puts -nonewline",
        "set ch stdout; puts $ch dynamic",
        "set ch stderr; puts -nonewline $ch dyn; puts stderr {}",
        "set w -nonewline; puts $w stdout flag; puts {}",
        "puts [string repeat - 3]; puts stderr [string toupper done]",
        "catch {puts} m; puts stderr $m",
        "catch {puts a b c d} m; puts $m",
        "catch {puts stdout x nonewline} m; puts $m",
        "namespace eval ns {proc say {} {puts [namespace current]}}; ns::say",
    ]);
}

#[test]
fn vm_strict_denies_puts_to_other_channels() {
    for source in ["puts stdin x", "puts -nonewline file3 x"] {
        let error = run(source, RuntimeMode::VmStrict).unwrap_err();
        assert!(
            error.contains("\"puts\" requires WorldHost"),
            "{source}: {error}"
        );
    }
    let error = run("set ch stdin; puts $ch x", RuntimeMode::VmStrict).unwrap_err();
    assert!(error.contains("denied"), "{error}");
    // Hybrid mode writes through Tcl's own channels, with Tcl's errors.
    let source = "catch {puts stdin x} m; set m";
    assert_eq!(run(source, RuntimeMode::Hybrid), Ok(oracle(source)));
}

#[test]
fn lsort_matches_tcl() {
    assert_matches_tcl(&[
        "lsort {c a b}",
        "lsort {}",
        "lsort {a}",
        "lsort -decreasing {c a b}",
        "lsort -increasing -decreasing -increasing {c a b}",
        "lsort {B a C b}",
        "lsort -nocase {B a C b}",
        "lsort -ascii -nocase {Ab aB ab}",
        "lsort -decreasing -nocase {Ab b aB B}",
        "lsort -dictionary {a10 a2 A1 a1 b}",
        "lsort -dictionary {x10y x9y x010y}",
        "lsort -dictionary -nocase {B a}",
        "lsort -integer {10 9 -1 0x10 2}",
        "lsort -integer -decreasing {10 9 -1 2}",
        "lsort -real {1.5 -2 3e1 0.25}",
        "lsort -real {1 1.0 0x1}",
        "lsort -real -decreasing {1 1.0 2 0x1}",
        "lsort -unique {c a b a c}",
        "lsort -unique -nocase {a A b B}",
        "lsort -unique -integer {1 01 0x1 2}",
        "lsort -unique -decreasing {a b a}",
        "lsort -indices {c a b}",
        "lsort -indices -decreasing {c a b}",
        "lsort -indices -unique {b a b}",
        "lsort -index 1 {{a 3} {b 1} {c 2}}",
        "lsort -index end {{a 3} {b 1} {c 2}}",
        "lsort -index end-1 {{a 3 x} {b 1 y}}",
        "lsort -index {1 0} {{x {b y}} {z {a w}}}",
        "lsort -index {} {b a}",
        "lsort -index 1 -integer -decreasing {{a 3} {b 10} {c 2}}",
        "lsort -index 0 -unique {{a 1} {b 2} {a 3}}",
        "lsort -stride 2 {b 2 a 1 c 3}",
        "lsort -stride 2 -index 1 {b 2 a 1 c 3}",
        "lsort -stride 2 -index end -decreasing {b 2 a 1 c 3}",
        "lsort -stride 2 -indices {b 2 a 1}",
        "lsort -stride 3 -index {1 0} {x {b q} 1 y {a r} 2}",
        "lsort -stride 2 -unique {a 1 b 2 a 3}",
        "lsort -stride 2 {}",
        "lsort -stride 2 -index 1 -integer {x 10 y 9}",
        "set l {b a c}; lsort $l",
        "proc f {l} {lsort -dictionary $l}; f {item10 item9}",
        "lsort [list b {a c} a]",
        "lsort {\\u00e9 e f}",
        "lsort -nocase {\\u00c9 e \\u00e9}",
        "lsort -dictionary {\\u00c9 e \\u00e9}",
        "set o -decreasing; lsort $o {a c b}",
        // Errors.
        "set code [catch {lsort} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -foo {a}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -in {a}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -index {a b}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -stride {a b}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -index x {a b}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -index -1 {a b}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -index 1 {{a} {b c}}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -index end-3 {{a b} {c d}}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -stride 1 {a b}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -stride 2 {a b c}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -stride x {a b}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -stride 2 -index 5 {a b c d}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -integer {1 x}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -integer {1 99999999999999999999}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -real {1 x}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort -real {1 NaN}} msg]; set _ \"$code $msg\"",
        "set code [catch {lsort \"a \\{b\"} msg]; set _ \"$code $msg\"",
        "catch {lsort -index 1 {{a} {b c}}} r o; dict get $o -errorcode",
        "catch {lsort -stride 1 {a b}} r o; dict get $o -errorcode",
        "catch {lsort -index -1 {a}} r o; dict get $o -errorcode",
    ]);
}

#[test]
fn lsort_command_runs_natively_in_both_modes() {
    assert_matches_tcl(&[
        "proc cmp {a b} {string compare $b $a}; lsort -command cmp {b c a}",
        "proc cmp {a b} {string compare $a $b}; lsort -c cmp {b c a}",
        "proc cmp {a b} {string compare $a $b}; set o -command; lsort $o cmp {b c a}",
        "lsort -command dummy -ascii {b a}",
    ]);
}
