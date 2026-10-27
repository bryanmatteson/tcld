#![cfg(all(feature = "host", feature = "debug"))]
//! Tcl's regular expressions in the VM, checked against embedded Tcl: the
//! cases of Tcl's own `tests/reg.test` run through `regexp`, `regsub`,
//! `regexp -about` and `lsearch -regexp` in vm-strict and hybrid mode, then
//! the commands' options, quirks and errors.

use tcld_compile::{CompileMode, compile_source};
use tcld_vm::list_codec::TclListCodec;
use tcld_vm::{DebugOutcome, HybridEngine, RuntimeMode};

fn run(source: &str, mode: RuntimeMode) -> Result<String, String> {
    let compile_mode = match mode {
        RuntimeMode::VmStrict => CompileMode::VmStrict,
        _ => CompileMode::Hybrid,
    };
    let image = compile_source("/workspace/regexp.tcl".into(), source.into(), compile_mode)
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

fn assert_matches_tcl(sources: &[&str]) {
    let mut mismatches = Vec::new();
    for source in sources {
        let expected = oracle(source);
        for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            let actual = run(source, mode);
            if actual.as_ref() != Ok(&expected) {
                mismatches.push(format!(
                    "{mode:?}: {source}\n  Tcl: {expected}\n  VM: {actual:?}"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

fn list(words: &[&str]) -> String {
    String::from_utf8(TclListCodec::format(
        words.iter().map(|word| word.as_bytes()),
    ))
    .unwrap()
}

fn parse(text: &str) -> Vec<String> {
    TclListCodec::parse(text.as_bytes())
        .unwrap()
        .into_iter()
        .map(|item| String::from_utf8_lossy(&item).into_owned())
        .collect()
}

/// Run each command under `catch`, as one script, and compare every
/// command's outcome (code, result and error code) with Tcl's.
fn assert_commands_match_tcl(commands: &[String]) {
    let script = format!(
        "set out {{}}\nforeach cmd {{{}}} {{\n    set code [catch {{{{*}}$cmd}} r o]\n    \
         if {{$code}} {{lappend out [list $code $r [dict get $o -errorcode]]}} \
         else {{lappend out [list 0 $r]}}\n}}\nset out",
        list(&commands.iter().map(String::as_str).collect::<Vec<_>>())
    );
    let expected = parse(&oracle(&script));
    assert_eq!(expected.len(), commands.len());
    for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let actual = parse(&run(&script, mode).unwrap_or_else(|error| panic!("{mode:?}: {error}")));
        assert_eq!(actual.len(), commands.len(), "{mode:?}");
        let mismatches: Vec<String> = commands
            .iter()
            .zip(expected.iter().zip(&actual))
            .filter(|(_, (want, got))| want != got)
            .map(|(command, (want, got))| format!("{command}\n  Tcl: {want}\n  VM:  {got}"))
            .collect();
        assert!(
            mismatches.is_empty(),
            "{mode:?}: {} of {} commands differ:\n{}",
            mismatches.len(),
            commands.len(),
            mismatches
                .iter()
                .take(25)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

const REG_TEST: &str = include_str!("../../../third_party/tcl/src/tcl-core-9-0-2/tests/reg.test");

/// The `expect*` cases of reg.test, as Tcl reads them: kind, test id,
/// flags, expression, then the target and expectations.
fn reg_test_cases() -> Vec<Vec<String>> {
    let start = REG_TEST
        .find("######## the tests themselves ########")
        .expect("reg.test tests");
    let end = REG_TEST
        .find("namespace import -force ::tcltest::test")
        .expect("reg.test custom tests");
    let script = format!(
        "namespace eval ::tcltest {{proc test args {{}}}}\n\
         set ::cases {{}}\n\
         proc doing args {{}}\n\
         proc knownBug args {{}}\n\
         foreach kind {{Match Nomatch Indices Partial Error}} {{\n    \
             proc expect$kind args \"lappend ::cases \\[list [string tolower $kind] {{*}}\\$args\\]\"\n\
         }}\n\
         {}\nset ::cases",
        &REG_TEST[start..end]
    );
    parse(&oracle(&script))
        .into_iter()
        .map(|case| parse(&case))
        .collect()
}

/// The commands that exercise one reg.test case: its flags become
/// `regexp` options and embedded options (`(?b)`, `(?e)`, `***=`), and a
/// string that is not the start of a line becomes `-start 1` after an
/// extra character.
fn case_commands(case: &[String], out: &mut Vec<String>) {
    let (kind, flags, re) = (&case[0], &case[2], &case[3]);
    if kind == "partial" || flags.contains(['+', '$', 't']) {
        return;
    }
    if kind == "error" && case.get(4).is_some_and(|code| code == "INVARG") {
        return;
    }
    let variants: Vec<String> = if flags.contains('&') {
        let base = flags.replace('&', "");
        vec![base.clone(), format!("{base}b")]
    } else {
        vec![flags.clone()]
    };
    for flags in variants {
        let prefix = if flags.contains('q') {
            "***="
        } else if flags.contains('b') {
            "(?b)"
        } else if flags.contains('e') && !flags.contains('a') {
            "(?e)"
        } else {
            ""
        };
        if !prefix.is_empty() && re.starts_with("***") {
            continue;
        }
        let pattern = format!("{prefix}{re}");
        let pattern = pattern.as_str();
        let mut options: Vec<&str> = Vec::new();
        for (flag, option) in [
            ('i', "-nocase"),
            ('x', "-expanded"),
            ('n', "-line"),
            ('p', "-linestop"),
            ('w', "-lineanchor"),
        ] {
            if flags.contains(flag) {
                options.push(option);
            }
        }
        let command = |head: &[&str], tail: &[&str]| {
            let mut words: Vec<&str> = head.to_vec();
            words.extend(&options);
            words.push("--");
            words.extend(tail);
            list(&words)
        };
        out.push(command(&["regexp", "-about"], &[pattern]));
        let Some(target) = case.get(4).filter(|_| kind != "error") else {
            out.push(command(&["regexp"], &[pattern, ""]));
            continue;
        };
        let notbol = flags.contains('^');
        let target = if notbol {
            format!("x{target}")
        } else {
            target.clone()
        };
        let target = target.as_str();
        let start: &[&str] = if notbol { &["-start", "1"] } else { &[] };
        let head = |words: &[&'static str]| [words, start].concat();
        out.push(command(
            &head(&["regexp", "-indices", "-inline"]),
            &[pattern, target],
        ));
        out.push(command(&head(&["regexp", "-inline"]), &[pattern, target]));
        out.push(command(
            &head(&["regexp", "-all", "-indices", "-inline"]),
            &[pattern, target],
        ));
        out.push(command(&head(&["regexp", "-all"]), &[pattern, target]));
        out.push(command(
            &head(&["regsub", "-all"]),
            &[pattern, target, "<&|\\1|\\2>"],
        ));
        if !notbol && options.iter().all(|option| *option == "-nocase") {
            let mut words = vec!["lsearch", "-all", "-regexp"];
            words.extend(&options);
            let items = list(&[target, "x", ""]);
            words.extend([items.as_str(), pattern]);
            out.push(list(&words));
        }
    }
}

#[test]
fn reg_test_cases_match_tcl() {
    let cases = reg_test_cases();
    assert!(cases.len() > 450, "{} reg.test cases", cases.len());
    let mut commands = Vec::new();
    for case in &cases {
        case_commands(case, &mut commands);
    }
    assert!(commands.len() > 3000, "{} commands", commands.len());
    assert_commands_match_tcl(&commands);
}

#[test]
fn regexp_command_matches_tcl() {
    assert_matches_tcl(&[
        "regexp {a(b)c} xabcx m s; list $m $s",
        "regexp -indices {a(b)?c} ac m s; list $m $s",
        "regexp -inline -all {\\d+} {a1b22c333}",
        "regexp -all {a} banana",
        "regexp -all {} abc",
        "regexp -all -inline {a*} baaab",
        "regexp -all -inline -indices {a*} baaab",
        "regexp -all -inline {(a)(b)?} {ab a}",
        "regexp -start 2 -inline {a.} xxabab",
        "regexp -start end-1 -indices -inline {.} abcdef",
        "regexp -start end+5 {$} abc",
        "list [regexp -start -3 a abc] [regexp -start 99 {$} abc]",
        "regexp -start 1 {^b} ab",
        "regexp -start 2 {^b} \"a\\nb\"",
        "regexp -about {(a)(b)?}",
        "regexp -about -nocase {a}",
        "regexp -about {a} b c d",
        "regexp -nocase -inline {AB+} xabbb",
        "regexp -expanded -inline { a # comment\n b } ab",
        "regexp -line -inline -all {^\\w+$} \"one\\ntwo\"",
        "regexp -linestop -inline {a.*} \"ab\\ncd\"",
        "regexp -lineanchor -inline -all {^.} \"ab\\ncd\"",
        "regexp -- -a- x-a-y",
        "regexp {(a)|(b)} b m x y; list $m $x $y",
        "regexp {(a)} a m s extra; list $m $s $extra",
        "set a(1) z; regexp {(.)(.)} xy a(1) a(2); list $a(1) $a(2)",
        "regexp {} {}",
        "regexp -inline {} abc",
        "regexp -indices -inline {} abc",
        "regexp {\\y\\w+\\y} {  hello  } m; set m",
        "regexp {(?i)HELLO} hello",
        "proc f {s} {regexp {^(\\w+)@(\\w+)\\.com$} $s -> u d; list $u $d}; f joe@example.com",
        "regexp -inline {(\\w)\\1} abccd",
        "regexp -indices {(a)(b)?} a m s1 s2; list $m $s1 $s2",
        "regexp -inline {[[:alpha:]]+} {12 héllo wörld}",
        "regexp -inline {a{2,3}} aaaa",
        "set re {(\\d+)-(\\d+)}; regexp -inline $re {10-20}",
        "set opts {-nocase -inline}; regexp {*}$opts AB ab",
        "regexp -inline -all -indices {\\m\\w} {one two three}",
        "regexp -inline {(?=a)\\w+} {xyz abc}",
        "regexp -inline {x(?!y)} {xy xz}",
        "regexp -inline {a*?b} aaab",
        "regexp -inline {(a+?)(a*)} aaa",
        "regexp -inline {\\u00e9+} {caf\\u00e9\\u00e9}",
        "regexp -inline {***=a.b} {a.b axb}",
        "regexp -inline {[^[:space:]]+} {  x y  }",
        "regexp -inline {\\Bfoo} {a\\foo}",
        "regexp -inline -all {.} {aé中}",
        "proc f {} {regexp -all -inline {[0-9]+} {a1b23}}; f",
        "proc f {} {regexp {(x)(y)?} x all sub1 sub2; list $all $sub1 $sub2}; f",
        "regexp -all -inline -indices {.} {é😀𐐀}",
        "regexp -nocase -inline {𐐀+} {𐐨𐐨}",
        "regexp -inline {[[:alpha:]]+} {1𐐀2}",
        "regexp -inline {(?i)(𐐀)\\1} {𐐨𐐀}",
    ]);
}

#[test]
fn regexp_errors_match_tcl() {
    let catch =
        |command: &str| format!("list [catch {{{command}}} m o] $m [dict get $o -errorcode]");
    let sources: Vec<String> = [
        "regexp -foo a b",
        "regexp -ind a b",
        "regexp - a b",
        "regexp a",
        "regexp",
        "regexp -start",
        "regexp -start 1 a",
        "regexp -start foo a b",
        "regexp -start 1.5 a b",
        "regexp -inline a b c",
        "regexp -inline -about a",
        "regexp {(} x",
        "regexp a\\{1 x",
        "regexp {\\q} x",
        "regexp {(?z)a} x",
        "regexp {***?} x",
        "regexp {a**} x",
        "regexp {x{300}} x",
        "regexp {[[:foo:]]} x",
        "regexp {[z-a]} x",
        "regexp {\\3} x",
        "regexp {(a)\\2} x",
        "regexp {[[.foo.]]} x",
        "regexp -about {(}",
        "set a 1; regexp {(.)} x a(1)",
        "set a(1) 1; regexp {(.)} x a",
        "proc f {} {set a 1; regexp {(.)} x a(1)}; f",
        "proc f {} {set a(1) 1; regexp {(.)} x a}; f",
        "namespace eval n {set a 1; regexp {(.)} x a(1)}",
        "namespace eval n {set a(1) 1; regexp {(.)} x a}",
    ]
    .iter()
    .map(|command| catch(command))
    .collect();
    assert_matches_tcl(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn regsub_matches_tcl() {
    assert_matches_tcl(&[
        "regsub {a} banana A",
        "regsub -all {a} banana A",
        "regsub -all {(a)(n)} banana {<\\2\\1>}",
        "regsub -all {a|n} banana {[&]}",
        "regsub -all -nocase {A} bAnana x",
        "regsub -all -nocase {AN} bANana x",
        "regsub {x} banana y",
        "regsub -all {} abc -",
        "regsub -all {} {} -",
        "regsub {} abc -",
        "regsub {} {} -",
        "regsub -all {^} abc >",
        "regsub -all {$} abc <",
        "regsub -all {b*} abc X",
        "regsub -all {x*} abc -",
        "regsub -start 3 -all a banana A",
        "regsub -start 1 {^b} abab X",
        "regsub -start 9 a abc X",
        "regsub -start end a aba X",
        "regsub -all {a} banana {\\\\&}",
        "regsub -all {a} banana \"x\\\\\"",
        "regsub -all {(b)(a)} banana {\\0|\\1|\\2|\\3|\\9}",
        "regsub -all {a} banana {\\x}",
        "list [regsub {a} banana A v] $v",
        "list [regsub -all a banana A v] $v",
        "list [regsub x banana A v] $v",
        "regsub -all -expanded {a n} banana X",
        "regsub -all -expanded {a\\ n} banana X",
        "regsub -all -line {^} \"a\\nb\" >",
        "regsub -all {(?n)$} \"a\\nb\" <",
        "regsub -all {\\w+} {hello big world} {(&)}",
        "regsub -all {é} {café é} e",
        "regsub -- -a- x-a-y _",
        "proc f {s} {regsub -all {\\s+} $s { }}; f {a   b \t c}",
        "set s banana; regsub -all {(an)} $s {[\\1]} s; set s",
        "regsub -- -command {a-commandb} x",
        "regsub a abc -command",
        "set re a; regsub $re abc -command",
        "set opts {--}; regsub {*}$opts -command {a-commandb} x",
        "regsub -all {😀} {😀a😀} 𐐀",
        "regsub -all -nocase {𐐀} {𐐨𐐀} x",
    ]);
    let catch =
        |command: &str| format!("list [catch {{{command}}} m o] $m [dict get $o -errorcode]");
    let sources: Vec<String> = [
        "regsub a b",
        "regsub a b c d e",
        "regsub -foo a b c",
        "regsub -al a b c",
        "regsub -all {(} a b",
        "regsub -start x a b c",
        "regsub -start",
        "set a(1) 1; regsub a abc x a",
        "set a 1; regsub a abc x a(1)",
        "proc f {} {set a(1) 1; regsub a abc x a}; f",
        "proc f {} {set a 1; regsub a abc x a(1)}; f",
        "namespace eval n {set a(1) 1; regsub a abc x a}",
        "regsub -start -command a abc x",
    ]
    .iter()
    .map(|command| catch(command))
    .collect();
    assert_matches_tcl(&sources.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn lsearch_regexp_matches_tcl() {
    assert_matches_tcl(&[
        "lsearch -regexp {abc ab a} {^ab}",
        "lsearch -regexp -all {abc ab a} {^ab}",
        "lsearch -regexp -inline {abc ab a} {b$}",
        "lsearch -regexp -all -not {abc ab a} b",
        "lsearch -regexp -nocase -all {ABC ab} {^ab}",
        "lsearch -regexp -all {aa ab bb} {(.)\\1}",
        "lsearch -regexp -start 1 {ab ab} ab",
        "lsearch -regexp -index 1 -all {{x ab} {y cd}} c",
        "lsearch -regexp {} a",
        "lsearch -regexp {a b} {}",
        "lsearch -regexp -inline -all {1 22 333} {^\\d{2,}$}",
        "proc f {l} {lsearch -all -regexp $l {^[a-c]}}; f {apple dog cat}",
        "list [catch {lsearch -regexp {a} {(}} m o] $m [dict get $o -errorcode]",
        "list [catch {lsearch -regexp {a} {(a)\\2}} m o] $m [dict get $o -errorcode]",
    ]);
}

#[test]
fn regsub_command_callbacks_match_tcl() {
    assert_matches_tcl(&[
        r#"regsub -command {.x.} abcxdef {string length}"#,
        r#"regsub -command {.x.} abcxdefxghi {string length}"#,
        r#"regsub -command {(.)(.)} abcdef {list ,}"#,
        r#"regsub -command -all {(.)(.)} abcdef {list ,}"#,
        r#"set c -command; regsub $c {[a-z]} abc {string toupper}"#,
        r#"set opts {-all -command}; regsub {*}$opts {[a-z]} abc {string toupper}"#,
        r#"::regsub -all -command . abc {::string toupper}"#,
        r#"regsub -command {(a)?b} bc list"#,
        r#"regsub -command {(a)(b)(c)(d)(e)(f)(g)(h)(i)} abcdefghi list"#,
        r#"regsub -command -all {(?=.)} abcde {string length}"#,
        r#"regsub -command -all {} abc {string length}"#,
        r#"regsub -command -all {} {} {string length}"#,
        r#"regsub -command -all {$} abc {string length}"#,
        r#"regsub -command -all -start 2 . abcde {string toupper}"#,
        r#"regsub -command -all -start end+1 {} abc {string length}"#,
        r#"regsub -command -start 99 . abc error"#,
        r#"regsub -command z abc nonexistent"#,
        r#"regsub -command {.+} {set x 42} eval"#,
        r#"proc f {} {set x 1; set r [regsub -command {.+} {incr x 2} eval]; list $r $x}; f"#,
        r#"regsub -all -command {\d+} {1 20 300} {expr 2*}"#,
        r#"regsub -command -all {(.)} {é🙂𝄞} list"#,
        r#"regsub -command -all -nocase {é} {aÉé} {string toupper}"#,
        r#"regsub -command -all -line {^a} "a\na\nb" {string toupper}"#,
        r#"regsub -command -expanded {( a )} abc list"#,
        r#"regsub -command -- {-command} a-commandb {string toupper}"#,
        r#"regsub -command {(.*)} {a;[error injected]$x\q} list"#,
        r#"set prefix [list list {a;[error injected]$x\q}]; regsub -command . xyz $prefix"#,
        r#"proc cb {m} {return {&\1}}; regsub -command -all . ab cb"#,
        r#"set out seed; set n [regsub -command -all . abc {string toupper} out]; list $n $out"#,
        r#"set out seed; set n [regsub -command z abc error out]; list $n $out"#,
        r#"proc f {} {set out {}; set r [regsub -all -command . abc {append out}]; list $r $out}; f"#,
        r#"proc cb {m} {upvar 1 x x; incr x; return $x}; proc f {} {set x 0; set r [regsub -all -command {(?=.)} abc cb]; list $r $x}; f"#,
        r#"namespace eval n {proc cb {m} {string toupper $m}; proc f {} {regsub -all -command . abc cb}; f}"#,
        r#"namespace eval n {variable x 0; proc cb {m} {variable x; incr x}; regsub -all -command . abc cb}"#,
        r#"proc cb {m} {list [info level] [info level 0]}; proc f {} {regsub -command . a cb}; f"#,
        r#"proc cb {m} {upvar 1 original s; set s changed; return $m}; proc f {} {set original abc; set r [regsub -all -command . $original cb]; list $r $original}; f"#,
        r#"proc cb {m} {set ::prefix error; set ::pattern z; return $m}; set prefix cb; set pattern .; list [regsub -all -command $pattern abc $prefix] $prefix $pattern"#,
        r#"proc cb {m} {regsub -all -command . $m {string toupper}}; regsub -all -command {\w+} {a b cd} cb"#,
        r#"set c [catch {regsub -command . abc return} r o]; list $c $r [dict get $o -code] [dict get $o -level]"#,
        r#"proc cb {m} {return -level 2 found}; proc f {} {regsub -command . abc cb; error unreachable}; f"#,
        r#"proc cb {m} {return -code 7 custom}; set c [catch {regsub -command . abc cb} r o]; list $c $r [dict get $o -code] [dict get $o -level]"#,
        r#"proc cb {m} {return -code break stop}; set out {}; foreach v {1 2} {lappend out $v; regsub -command . abc cb; lappend out bad}; set out"#,
        r#"proc cb {m} {return -code continue next}; set out {}; foreach v {1 2} {lappend out $v; regsub -command . abc cb; lappend out bad}; set out"#,
        r#"set out original; proc cb {m} {incr ::calls; if {$::calls == 2} {error failed {} {REGSUB CALLBACK}}; return X}; set calls 0; set c [catch {regsub -all -command . abc cb out} r o]; list $c $r $calls $out [dict get $o -errorcode] [string match {*(-command substitution computation script)*} [dict get $o -errorinfo]]"#,
        r#"set c [catch {regsub -command . abc error} r o]; list $c $r [dict get $o -errorcode] [string match {*(-command substitution computation script)*} [dict get $o -errorinfo]]"#,
        r#"set c [catch {regsub -command . abc {}} r o]; list $c $r [dict get $o -errorcode]"#,
        r#"set c [catch {regsub -command . abc "def \{ghi"} r o]; list $c $r [dict get $o -errorcode]"#,
        r#"set c [catch {regsub -command {(} abc {}} r o]; list $c $r [dict get $o -errorcode]"#,
        r#"proc f {} {set a 1; set c [catch {regsub -command . abc {string toupper} a(k)} r o]; list $c $r [dict get $o -errorcode]}; f"#,
        r#"proc f {} {set a(k) 1; set c [catch {regsub -command . abc {string toupper} a} r o]; list $c $r [dict get $o -errorcode]}; f"#,
    ]);
}

#[test]
fn regsub_command_hybrid_lambdas_traces_and_synchronous_yield_match_tcl() {
    for source in [
        r#"set x 0; regsub -all -command {(?=.)} abcde {apply {args {incr ::x}}}"#,
        r#"set ::s {123=456 789}; regsub -command -all {\d+} $::s {apply {n {expr {[llength $::s] + $n}}}}"#,
        r#"set ::t {apply {n {expr {[llength [lindex $::t 1 1 1]] + $n}}}}; regsub -command -all {\d+} {123=456 789} $::t"#,
        r#"set s {list (.+)}; regsub -command $s {list list} $s"#,
        r#"proc cb {m} {yield $m}; set c [catch {coroutine co regsub -all -command . abc cb} r o]; list $c $r [dict get $o -errorcode] [info commands co]"#,
        r#"set events {}; proc tracecb {args} {lappend ::events $args}; proc cb {m} {string toupper $m}; trace add execution cb {enter leave} tracecb; set r [regsub -all -command . ab cb]; list $r $events"#,
        r#"set events {}; proc tracecb {n e op} {upvar 1 $n v; lappend ::events $v}; proc f {} {set out seed; trace add variable out write tracecb; regsub -all -command . ab {string toupper} out; return $out}; list [f] $events"#,
    ] {
        assert_eq!(
            run(source, RuntimeMode::Hybrid).unwrap(),
            oracle(source),
            "{source}"
        );
    }
}

#[test]
fn regsub_callbacks_keep_runtime_source_admission() {
    for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let source = "set code [catch {regsub -command {.+} {interp eval {} {set ::escaped yes}} eval} message]; list $code [info exists ::escaped]";
        assert_eq!(run(source, mode).unwrap(), "1 0", "{mode:?}");
    }
}

#[test]
fn regsub_command_supplies_every_capture_beyond_nine() {
    // Tcl 9's Tcl_RegsubObjCmd asks its matcher for only ten entries, then
    // reads nsubs + 1 entries. Beyond nine captures the installed C oracle
    // can read uninitialized ranges and crash in Tcl_NewUnicodeObj. Verify
    // the complete argument contract directly, without calling that oracle.
    for (pattern, subject, captures) in [
        (
            "(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)".to_owned(),
            "abcdefghij".to_owned(),
            vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"],
        ),
        (
            "(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)?(k)?(l)?".to_owned(),
            "abcdefghi".to_owned(),
            vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "", "", ""],
        ),
        ("(a)".repeat(30), "a".repeat(30), vec!["a"; 30]),
    ] {
        let source = list(&["regsub", "-command", &pattern, &subject, "list capture"]);
        let expected = list(&[&["capture", subject.as_str()][..], &captures].concat());
        for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
            assert_eq!(run(&source, mode).unwrap(), expected, "{mode:?}: {source}");
        }
    }
}

#[test]
fn regsub_callbacks_preserve_debug_breakpoints_locals_and_continuation() {
    let source = "proc replacement {digits} {\n\
                      set marker $digits\n\
                      incr marker\n\
                      return <$marker>\n\
                  }\n\
                  set output [regsub -all -command {\\d+} {a1 b2} replacement]\n\
                  set output\n";
    for mode in [RuntimeMode::VmStrict, RuntimeMode::Hybrid] {
        let image = compile_source(
            "/workspace/regsub-debug.tcl".into(),
            source.into(),
            if mode == RuntimeMode::VmStrict {
                CompileMode::VmStrict
            } else {
                CompileMode::Hybrid
            },
        )
        .unwrap();
        let mut engine = HybridEngine::new(mode).unwrap();
        if mode == RuntimeMode::Hybrid {
            engine.bootstrap(Some("tcld")).unwrap();
        }
        let mut state = engine.debug_start_image(&image, true).unwrap().outcome;
        let mut address = None;
        for _ in 0..500 {
            if let DebugOutcome::Paused(pause) = &state
                && let Some(at) = pause.address
                && engine.debug_sources().iter().any(|s| {
                    s.code_id == at.code_id
                        && s.image.units[s.unit_index].logical_id == "proc:replacement"
                })
                && engine
                    .ui_frame_locals(0)
                    .iter()
                    .any(|(n, v)| n == "marker" && v == "1")
            {
                address = Some(at);
                break;
            }
            assert!(
                !matches!(state, DebugOutcome::Finished { .. }),
                "{mode:?}: callback never paused"
            );
            state = engine.step_instruction().unwrap();
        }
        let at = address.expect("paused in mapped callback body with its locals intact");
        let sources = engine.debug_sources();
        let callback = sources.iter().find(|s| s.code_id == at.code_id).unwrap();
        let pc = callback.image.units[callback.unit_index]
            .commands
            .iter()
            .find(|m| callback.image.source.content[m.source_span.clone()].starts_with("return <"))
            .unwrap()
            .anchor_pc;
        assert!(
            sources
                .iter()
                .any(|s| s.origin.is_some() && s.image.source.content.as_ref() == "replacement 1")
        );
        let breakpoint = tcld_vm::CodeAddress {
            code_id: at.code_id,
            pc,
        };
        engine.add_breakpoint(breakpoint);
        if mode == RuntimeMode::Hybrid {
            engine
                .eval_in_frame(0, tcld_vm::VmObj::from_str("set marker 40"))
                .unwrap();
        }
        let state = engine.continue_run().unwrap();
        assert!(
            matches!(state, DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint)),
            "{mode:?}: {state:?}"
        );
        let first = if mode == RuntimeMode::Hybrid {
            "41"
        } else {
            "2"
        };
        assert!(
            engine
                .ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "marker" && v == first)
        );
        let state = engine.continue_run().unwrap();
        assert!(
            matches!(state, DebugOutcome::Paused(ref pause) if pause.address == Some(breakpoint)),
            "{mode:?}: {state:?}"
        );
        assert!(
            engine
                .ui_frame_locals(0)
                .iter()
                .any(|(n, v)| n == "marker" && v == "3")
        );
        engine.remove_breakpoint(breakpoint);
        match engine.continue_run().unwrap() {
            DebugOutcome::Finished { result } => {
                assert_eq!(result.as_string().unwrap(), format!("a<{first}> b<3>"))
            }
            other => panic!("{mode:?}: {other:?}"),
        }
        if mode == RuntimeMode::VmStrict {
            assert_eq!(engine.coverage_snapshot().host_command_invocations, 0);
        }
    }
}
