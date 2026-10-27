use super::*;

fn chrs(text: &str) -> Vec<Chr> {
    text.chars().map(Chr::from).collect()
}

/// Every submatch of the first match, as Tcl's `regexp -indices` reports
/// them (an end index is the last character's).
fn indices(pattern: &str, text: &str) -> Option<Vec<(isize, isize)>> {
    let re = Regex::compile(&chrs(pattern), REG_ADVANCED).expect("pattern compiles");
    let matched = re
        .exec(&chrs(text), re.nsub() + 1, 0)
        .expect("match runs")?;
    Some(
        matched
            .into_iter()
            .map(|m| match m {
                Some((start, end)) => (start as isize, end as isize - 1),
                None => (-1, -1),
            })
            .collect(),
    )
}

fn compile_error(pattern: &str) -> String {
    Regex::compile(&chrs(pattern), REG_ADVANCED)
        .expect_err("pattern fails")
        .name()
}

#[test]
fn matches_place_submatches_as_tcl_does() {
    assert_eq!(indices("abc", "xabcx"), Some(vec![(1, 3)]));
    assert_eq!(indices("a(b)c", "abc"), Some(vec![(0, 2), (1, 1)]));
    assert_eq!(indices("(a)|b", "b"), Some(vec![(0, 0), (-1, -1)]));
    assert_eq!(indices("a*", "baaab"), Some(vec![(0, -1)]));
    assert_eq!(
        indices("(a|ab)(c|bcd)(d*)", "abcd"),
        Some(vec![(0, 3), (0, 1), (2, 2), (3, 3)])
    );
    assert_eq!(
        indices("(a*?)(a*)", "aaa"),
        Some(vec![(0, -1), (0, -1), (0, -1)])
    );
    assert_eq!(indices("x+?y", "xxy"), Some(vec![(0, 2)]));
    assert_eq!(indices("^b", "ab"), None);
    assert_eq!(indices("[[:digit:]]+", "ab123c"), Some(vec![(2, 4)]));
    assert_eq!(indices("\\w+", "  héllo_1 "), Some(vec![(2, 8)]));
    assert_eq!(indices("(a)\\1", "xaa"), Some(vec![(1, 2), (1, 1)]));
    assert_eq!(indices("a(?=b)", "acab"), Some(vec![(2, 2)]));
    assert_eq!(indices("a(?!b)", "abac"), Some(vec![(2, 2)]));
    assert_eq!(indices("\\mfoo\\M", "xfoo foo"), Some(vec![(5, 7)]));
    assert_eq!(indices("a{2,3}", "aaaa"), Some(vec![(0, 2)]));
    assert_eq!(indices("(?i)ABC", "xabc"), Some(vec![(1, 3)]));
    assert_eq!(indices("***=a.b", "a.b axb"), Some(vec![(0, 2)]));
}

#[test]
fn compile_errors_name_tcl_codes() {
    assert_eq!(compile_error("["), "REG_EBRACK");
    assert_eq!(compile_error("a("), "REG_EPAREN");
    assert_eq!(compile_error("a{1"), "REG_EBRACE");
    assert_eq!(compile_error("\\q"), "REG_EESCAPE");
    assert_eq!(compile_error("(?z)a"), "REG_BADOPT");
    assert_eq!(compile_error("***?"), "REG_BADPAT");
    assert_eq!(compile_error("a**"), "REG_BADRPT");
    assert_eq!(compile_error("x{300}"), "REG_BADBR");
    assert_eq!(compile_error("[[:foo:]]"), "REG_ECTYPE");
    assert_eq!(compile_error("[z-a]"), "REG_ERANGE");
    assert_eq!(compile_error("\\3"), "REG_ESUBREG");
}

#[test]
fn about_reports_tcl_facts() {
    let info = |pattern: &str| {
        let re = Regex::compile(&chrs(pattern), REG_ADVANCED).unwrap();
        let names: Vec<&str> = INFO_NAMES
            .iter()
            .filter(|(bit, _)| re.info() & bit != 0)
            .map(|(_, name)| *name)
            .collect();
        (re.nsub(), names)
    };
    assert_eq!(
        info("(a)(?=b)\\1"),
        (1, vec!["REG_UBACKREF", "REG_ULOOKAHEAD", "REG_UNONPOSIX"])
    );
    assert_eq!(
        info("a{3}b*\\w"),
        (0, vec!["REG_UBOUNDS", "REG_UNONPOSIX", "REG_ULOCALE"])
    );
}
