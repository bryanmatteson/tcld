#![cfg(all(feature = "host", feature = "debug"))]
//! `tcld_vm::unicode` against the embedded Tcl.
//!
//! The exhaustive test covers every Unicode scalar value twice: through Tcl's
//! commands (`string toupper`/`tolower`/`totitle` and `string is <class>`) and
//! through the C functions behind them (`Tcl_UniCharToUpper` and friends).
//! The commands keep a character whose mapping needs more UTF-8 bytes, so only
//! the C functions reveal the per-character mapping for those characters.
//!
//! Each Tcl release regenerates its tables for a newer Unicode version, so the
//! exhaustive test only runs when the embedded Tcl is the release the tables
//! were taken from ([`unicode::TCL_PATCH_LEVEL`]) and is skipped, with a note,
//! otherwise. The readable spot checks run against any Tcl 9.

use std::ffi::c_int;
use std::sync::{Mutex, PoisonError};

use tcld_interp::Interpreter;
use tcld_vm::unicode::{self, CharClass};

// Pure table lookups that accept any `int` (tclUtf.c).
unsafe extern "C" {
    safe fn Tcl_UniCharToUpper(ch: c_int) -> c_int;
    safe fn Tcl_UniCharToLower(ch: c_int) -> c_int;
    safe fn Tcl_UniCharToTitle(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsAlnum(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsAlpha(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsControl(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsDigit(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsGraph(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsLower(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsPrint(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsPunct(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsSpace(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsUpper(ch: c_int) -> c_int;
    safe fn Tcl_UniCharIsWordChar(ch: c_int) -> c_int;
}

/// The C predicate behind `string is <class>`; `ascii` and `xdigit` are
/// static helpers in tclCmdMZ.c and only reachable through the command.
fn tcl_predicate(class: CharClass) -> Option<extern "C" fn(c_int) -> c_int> {
    Some(match class {
        CharClass::Alnum => Tcl_UniCharIsAlnum,
        CharClass::Alpha => Tcl_UniCharIsAlpha,
        CharClass::Control => Tcl_UniCharIsControl,
        CharClass::Digit => Tcl_UniCharIsDigit,
        CharClass::Graph => Tcl_UniCharIsGraph,
        CharClass::Lower => Tcl_UniCharIsLower,
        CharClass::Print => Tcl_UniCharIsPrint,
        CharClass::Punct => Tcl_UniCharIsPunct,
        CharClass::Space => Tcl_UniCharIsSpace,
        CharClass::Upper => Tcl_UniCharIsUpper,
        CharClass::Wordchar => Tcl_UniCharIsWordChar,
        CharClass::Ascii | CharClass::Xdigit => return None,
    })
}

const ORACLE_PROCS: &str = r#"
namespace eval ::oracle {
    proc str {codes} {
        set s ""
        foreach code $codes {append s [format %c $code]}
        return $s
    }
    proc codes {s} {
        set codes {}
        foreach ch [split $s ""] {scan $ch %c code; lappend codes $code}
        return $codes
    }
    # One line per scalar value in lo..hi: the code point; what string
    # toupper, tolower and totitle make of it; what string totitle makes of it
    # after a first character; and its string is bits in strclass order.
    proc scan_range {lo hi} {
        set rows ""
        for {set cp $lo} {$cp <= $hi} {incr cp} {
            if {$cp >= 0xD800 && $cp <= 0xDFFF} continue
            set c [format %c $cp]
            scan [string toupper $c] %c upper
            scan [string tolower $c] %c lower
            scan [string totitle $c] %c title
            scan [string index [string totitle "x$c"] 1] %c rest
            append rows $cp " " $upper " " $lower " " $title " " $rest " " \
                [string is alnum $c] [string is alpha $c] [string is ascii $c] \
                [string is control $c] [string is digit $c] [string is graph $c] \
                [string is lower $c] [string is print $c] [string is punct $c] \
                [string is space $c] [string is upper $c] [string is wordchar $c] \
                [string is xdigit $c] \n
        }
        return $rows
    }
    # The full class name string is selects for name, or "" if it rejects it.
    proc class_of {name classes} {
        if {[catch {string is $name {}}]} {return ""}
        return [::tcl::prefix match $classes $name]
    }
}
"#;

fn tcl() -> Interpreter {
    // Tcl 9 builds that keep their script library in zipfs (such as
    // third_party/tcl/runtime) can fail Tcl_Init when test threads
    // bootstrap concurrently, so bootstrap one interpreter at a time.
    static BOOTSTRAP: Mutex<()> = Mutex::new(());
    let _guard = BOOTSTRAP.lock().unwrap_or_else(PoisonError::into_inner);
    Interpreter::find_executable(Some("tcld"));
    let interp = Interpreter::new().unwrap();
    interp.eval(ORACLE_PROCS).unwrap();
    interp
}

fn eval(tcl: &Interpreter, script: &str) -> String {
    tcl.eval(script)
        .unwrap_or_else(|error| panic!("{error}\n{script}"))
}

fn tcl_list(codes: &[u32]) -> String {
    let words: Vec<String> = codes.iter().map(u32::to_string).collect();
    format!("{{{}}}", words.join(" "))
}

fn code_points(s: &str) -> Vec<u32> {
    s.chars().map(u32::from).collect()
}

fn parse_codes(text: &str) -> Vec<u32> {
    text.split_whitespace()
        .map(|word| word.parse().unwrap())
        .collect()
}

fn single(s: &str) -> Option<u32> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(u32::from(c)),
        _ => None,
    }
}

fn embedded_tcl_has_these_tables(tcl: &Interpreter) -> bool {
    let patch_level = eval(tcl, "info patchlevel");
    if patch_level == unicode::TCL_PATCH_LEVEL {
        return true;
    }
    eprintln!(
        "skipping the exhaustive Unicode oracle: the embedded Tcl is {patch_level}, but \
         tcld_vm::unicode mirrors Tcl {}. Build with TCL9_PREFIX set to a Tcl {} \
         installation (such as third_party/tcl/runtime/tcl-core-9-0-2) to run it.",
        unicode::TCL_PATCH_LEVEL,
        unicode::TCL_PATCH_LEVEL,
    );
    false
}

#[derive(Default)]
struct Mismatches {
    count: usize,
    shown: Vec<String>,
}

impl Mismatches {
    const SHOWN: usize = 60;

    fn compare<T: PartialEq + std::fmt::Debug>(&mut self, cp: u32, what: &str, tcl: T, rust: T) {
        if tcl != rust {
            self.count += 1;
            if self.shown.len() < Self::SHOWN {
                self.shown
                    .push(format!("U+{cp:04X} {what}: Tcl {tcl:X?}, Rust {rust:X?}"));
            }
        }
    }

    fn assert_none(&self, checked: usize) {
        assert!(
            self.count == 0,
            "{} mismatches over {checked} code points (first {}):\n{}",
            self.count,
            self.shown.len(),
            self.shown.join("\n")
        );
    }
}

#[test]
fn every_scalar_value_matches_tcl() {
    const BATCH: u32 = 0x1_0000;
    let tcl = tcl();
    if !embedded_tcl_has_these_tables(&tcl) {
        return;
    }
    let classes: Vec<_> = CharClass::ALL
        .into_iter()
        .map(|class| {
            let label = format!("string is {}", class.name());
            let predicate =
                tcl_predicate(class).map(|predicate| (predicate, format!("{label} (C)")));
            (class, label, predicate)
        })
        .collect();
    let mut mismatches = Mismatches::default();
    let mut expected = (0..=u32::from(char::MAX)).filter_map(char::from_u32);
    let mut checked = 0;
    for lo in (0..=u32::from(char::MAX)).step_by(BATCH as usize) {
        let rows = eval(
            &tcl,
            &format!("::oracle::scan_range {lo} {}", lo + BATCH - 1),
        );
        for row in rows.lines() {
            let fields: Vec<&str> = row.split(' ').collect();
            let [cp, upper, lower, title, rest, bits] = fields[..] else {
                panic!("malformed oracle row {row:?}");
            };
            let cp: u32 = cp.parse().unwrap();
            let c = expected.next().expect("Tcl reported too many code points");
            assert_eq!(cp, u32::from(c), "Tcl skipped or reordered a code point");
            let [upper, lower, title, rest] =
                [upper, lower, title, rest].map(|code| code.parse::<u32>().ok());
            let ch = c_int::try_from(cp).unwrap();

            // Per character: Tcl_UniCharToUpper and friends.
            let rust_code = |mapped: char| c_int::try_from(u32::from(mapped)).unwrap();
            mismatches.compare(
                cp,
                "to_upper",
                Tcl_UniCharToUpper(ch),
                rust_code(unicode::to_upper(c)),
            );
            mismatches.compare(
                cp,
                "to_lower",
                Tcl_UniCharToLower(ch),
                rust_code(unicode::to_lower(c)),
            );
            mismatches.compare(
                cp,
                "to_title",
                Tcl_UniCharToTitle(ch),
                rust_code(unicode::to_title(c)),
            );

            // Whole strings: string toupper, tolower and totitle.
            let text = c.to_string();
            mismatches.compare(cp, "string toupper", upper, single(&unicode::upper(&text)));
            mismatches.compare(cp, "string tolower", lower, single(&unicode::lower(&text)));
            mismatches.compare(cp, "string totitle", title, single(&unicode::title(&text)));
            let after_first = unicode::title(&format!("x{c}"));
            mismatches.compare(
                cp,
                "string totitle (after the first character)",
                rest,
                after_first.strip_prefix('X').and_then(single),
            );

            // Classes: the string is command and, where exported, its predicate.
            assert_eq!(bits.len(), classes.len(), "malformed oracle row {row:?}");
            for ((class, label, predicate), bit) in classes.iter().zip(bits.bytes()) {
                let rust = unicode::is_class(*class, c);
                mismatches.compare(cp, label, bit == b'1', rust);
                if let Some((predicate, label)) = predicate {
                    mismatches.compare(cp, label, predicate(ch) != 0, rust);
                }
            }
            checked += 1;
        }
    }
    assert!(
        expected.next().is_none(),
        "Tcl reported too few code points"
    );
    assert_eq!(checked, 0x11_0000 - 0x800);
    eprintln!(
        "compared {checked} scalar values with Tcl {}: {} mismatches",
        unicode::TCL_PATCH_LEVEL,
        mismatches.count
    );
    mismatches.assert_none(checked);
}

#[test]
fn readable_spot_checks_match_tcl() {
    let tcl = tcl();

    // (character, to_upper, to_lower, to_title, string is classes)
    let characters: &[(char, char, char, char, &str)] = &[
        // No simple uppercase for ß (Rust would give "SS"); ẞ lowercases to it.
        ('ß', 'ß', 'ß', 'ß', "alnum alpha graph lower print wordchar"),
        ('ẞ', 'ẞ', 'ß', 'ẞ', "alnum alpha graph print upper wordchar"),
        // Dotted capital I lowercases to plain i; dotless i uppercases to I.
        ('İ', 'İ', 'i', 'İ', "alnum alpha graph print upper wordchar"),
        ('ı', 'I', 'ı', 'I', "alnum alpha graph lower print wordchar"),
        // The DŽ digraph has distinct upper, title and lower forms; the titlecase
        // form is alpha but neither upper nor lower.
        ('Ǆ', 'Ǆ', 'ǆ', 'ǅ', "alnum alpha graph print upper wordchar"),
        ('ǅ', 'Ǆ', 'ǆ', 'ǅ', "alnum alpha graph print wordchar"),
        ('ǆ', 'Ǆ', 'ǆ', 'ǅ', "alnum alpha graph lower print wordchar"),
        // Final sigma uppercases to Σ, which always lowercases to σ.
        ('ς', 'Σ', 'ς', 'Σ', "alnum alpha graph lower print wordchar"),
        ('Σ', 'Σ', 'σ', 'Σ', "alnum alpha graph print upper wordchar"),
        // Simple mappings only: ᾀ uppercases to ᾈ, not to the full "ἈΙ".
        ('ᾀ', 'ᾈ', 'ᾀ', 'ᾈ', "alnum alpha graph lower print wordchar"),
        ('ᾈ', 'ᾈ', 'ᾀ', 'ᾈ', "alnum alpha graph print wordchar"),
        // A CJK ideograph and decimal digits from other scripts.
        ('中', '中', '中', '中', "alnum alpha graph print wordchar"),
        ('३', '३', '३', '३', "alnum digit graph print wordchar"),
        ('٣', '٣', '٣', '٣', "alnum digit graph print wordchar"),
        ('\0', '\0', '\0', '\0', "ascii control"),
        // ɐ maps to Ɐ per character, which string toupper does not apply.
        ('ɐ', 'Ɐ', 'ɐ', 'Ɐ', "alnum alpha graph lower print wordchar"),
        // Zero width space: a format character that Tcl also counts as space.
        (
            '\u{200B}',
            '\u{200B}',
            '\u{200B}',
            '\u{200B}',
            "control space",
        ),
    ];
    for &(c, upper, lower, title, classes) in characters {
        let ch = c_int::try_from(u32::from(c)).unwrap();
        let mapped = (
            unicode::to_upper(c),
            unicode::to_lower(c),
            unicode::to_title(c),
        );
        assert_eq!(mapped, (upper, lower, title), "{c:?} (Rust)");
        let tcl_mapped = [
            Tcl_UniCharToUpper(ch),
            Tcl_UniCharToLower(ch),
            Tcl_UniCharToTitle(ch),
        ]
        .map(|code| char::from_u32(u32::try_from(code).unwrap()).unwrap());
        assert_eq!(tcl_mapped, [upper, lower, title], "{c:?} (Tcl)");

        let rust_classes: Vec<&str> = CharClass::ALL
            .into_iter()
            .filter(|class| unicode::is_class(*class, c))
            .map(CharClass::name)
            .collect();
        assert_eq!(rust_classes.join(" "), classes, "{c:?} classes (Rust)");
        let script = format!(
            "set c [::oracle::str {}]; lmap class {{{}}} {{if {{[string is $class $c]}} {{set class}} else continue}}",
            tcl_list(&[u32::from(c)]),
            CharClass::ALL.map(CharClass::name).join(" "),
        );
        assert_eq!(eval(&tcl, &script), classes, "{c:?} classes (Tcl)");
    }

    // (string, string toupper, string tolower, string totitle)
    let strings: &[(&str, &str, &str, &str)] = &[
        ("", "", "", ""),
        ("hELLO wORLD", "HELLO WORLD", "hello world", "Hello world"),
        ("straße", "STRAßE", "straße", "Straße"),
        ("İstanbul", "İSTANBUL", "istanbul", "İstanbul"),
        ("ǆemal", "ǄEMAL", "ǆemal", "ǅemal"),
        ("ǄEMAL", "ǄEMAL", "ǆemal", "ǅemal"),
        ("ΟΔΟΣ", "ΟΔΟΣ", "οδοσ", "Οδοσ"),
        (
            "\u{1F80}\u{1F80}",
            "\u{1F88}\u{1F88}",
            "\u{1F80}\u{1F80}",
            "\u{1F88}\u{1F80}",
        ),
        // Mappings that need more UTF-8 bytes are not applied.
        ("ɐbc", "ɐBC", "ɐbc", "ɐbc"),
        ("ȺB", "ȺB", "Ⱥb", "Ⱥb"),
        // NUL survives (Tcl stores it as C0 80).
        ("a\0b", "A\0B", "a\0b", "A\0b"),
        // Georgian Mtavruli is not lowered after the first character.
        (
            "\u{1C90}\u{1C91}",
            "\u{1C90}\u{1C91}",
            "\u{10D0}\u{10D1}",
            "\u{1C90}\u{1C91}",
        ),
        (
            "\u{10D0}\u{1C91}",
            "\u{1C90}\u{1C91}",
            "\u{10D0}\u{10D1}",
            "\u{10D0}\u{1C91}",
        ),
        ("中文", "中文", "中文", "中文"),
    ];
    for &(input, upper, lower, title) in strings {
        let rust = (
            unicode::upper(input),
            unicode::lower(input),
            unicode::title(input),
        );
        assert_eq!(
            rust,
            (upper.into(), lower.into(), title.into()),
            "{input:?} (Rust)"
        );
        for (command, expected) in [("toupper", upper), ("tolower", lower), ("totitle", title)] {
            let script = format!(
                "::oracle::codes [string {command} [::oracle::str {}]]",
                tcl_list(&code_points(input))
            );
            assert_eq!(
                parse_codes(&eval(&tcl, &script)),
                code_points(expected),
                "string {command} {input:?} (Tcl)"
            );
        }
    }
}

#[test]
fn class_names_resolve_like_string_is() {
    let tcl = tcl();
    // Tcl's own list of classes, from the error for an unknown one.
    let error = tcl.eval("string is bogus x").unwrap_err().message;
    let listed = error
        .split_once("must be ")
        .unwrap_or_else(|| panic!("unexpected error {error:?}"))
        .1
        .replace(", or ", ", ");
    let classes: Vec<&str> = listed.split(", ").collect();
    for class in CharClass::ALL {
        assert!(
            classes.contains(&class.name()),
            "{} not in {classes:?}",
            class.name()
        );
    }

    let mut names: Vec<String> = ["", "WORD", "Alpha", "alnumx", "wordchars", "é"]
        .map(String::from)
        .into();
    for class in &classes {
        names.extend((1..=class.len()).map(|end| class[..end].to_owned()));
    }
    let class_list = format!("{{{}}}", classes.join(" "));
    for name in &names {
        let selected = eval(&tcl, &format!("::oracle::class_of {{{name}}} {class_list}"));
        let expected = CharClass::ALL
            .into_iter()
            .find(|class| class.name() == selected);
        assert_eq!(
            CharClass::from_name(name),
            expected,
            "{name:?} (Tcl selects {selected:?})"
        );
    }
}
