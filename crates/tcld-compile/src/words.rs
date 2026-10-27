//! Tcl's word syntax as `tclParse.c` scans it: where commands, words,
//! variable references and command substitutions begin and end. Braces and
//! quotes group only at the start of a word; inside an unquoted word they are
//! ordinary characters, as are `]` outside a command substitution and a `$`
//! that starts no variable name.

use std::ops::Range;

/// `CHAR_TYPE(c) & TYPE_SPACE`: the white space between words. A newline
/// ends a command instead.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

/// Whether a command substitution's `]` ends commands, as it does in a
/// script nested in `[...]`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Nesting {
    Script,
    Substitution,
}

impl Nesting {
    /// `TYPE_COMMAND_END`, plus `TYPE_CLOSE_BRACK` in a substitution.
    fn ends_command(self, byte: u8) -> bool {
        byte == b'\n' || byte == b';' || (self == Nesting::Substitution && byte == b']')
    }
}

/// `ParseWhiteSpace`: spaces and backslash-newlines; the offset after them.
fn white_space(bytes: &[u8], mut i: usize) -> usize {
    loop {
        while bytes.get(i).is_some_and(|b| is_space(*b)) {
            i += 1;
        }
        if bytes.get(i) == Some(&b'\\') && bytes.get(i + 1) == Some(&b'\n') {
            i += 2;
        } else {
            return i;
        }
    }
}

/// `ParseComment`: the white space, newlines and comments before a command.
fn comments(bytes: &[u8], mut i: usize) -> usize {
    loop {
        loop {
            i = white_space(bytes, i);
            if bytes.get(i) == Some(&b'\n') {
                i += 1;
            } else {
                break;
            }
        }
        if bytes.get(i) != Some(&b'#') {
            return i;
        }
        // A comment runs to a newline that no backslash escapes.
        i += 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    i += 1;
                    break;
                }
                b'\\' => i += 2,
                _ => i += 1,
            }
        }
        i = i.min(bytes.len());
    }
}

/// `Tcl_ParseCommand` from the first word of a command: its words, and the
/// offset of the byte that ended it (its terminator, or the end).
fn command(
    bytes: &[u8],
    mut i: usize,
    nesting: Nesting,
) -> Result<(Vec<Range<usize>>, usize), String> {
    let mut words = Vec::new();
    loop {
        i = white_space(bytes, i);
        match bytes.get(i) {
            None => return Ok((words, i)),
            Some(&byte) if nesting.ends_command(byte) => return Ok((words, i)),
            Some(_)
                if words
                    .last()
                    .is_some_and(|word: &Range<usize>| word.end == i) =>
            {
                // A braced or quoted word must end where the word does.
                return Err(if bytes[i - 1] == b'"' {
                    "extra characters after close-quote"
                } else {
                    "extra characters after close-brace"
                }
                .into());
            }
            Some(_) => {}
        }
        let end = word(bytes, i, nesting)?;
        words.push(i..end);
        i = end;
    }
}

/// The offset after the word that starts at `i`.
fn word(bytes: &[u8], i: usize, nesting: Nesting) -> Result<usize, String> {
    // `{*}` expands the word that follows it with no white space between.
    let rest = i + 3;
    let expands = bytes[i..].starts_with(b"{*}")
        && bytes
            .get(rest)
            .is_some_and(|&byte| white_space(bytes, rest) == rest && byte != b'\n' && byte != b';');
    let start = if expands { rest } else { i };
    match bytes.get(start) {
        Some(b'{') => braced_end(bytes, start),
        Some(b'"') => quoted_end(bytes, start),
        _ => tokens(
            bytes,
            start,
            |byte| is_space(byte) || nesting.ends_command(byte),
            true,
        ),
    }
}

/// `Tcl_ParseBraces`: the offset after the brace that closes the one at
/// `open`. An escaped brace neither opens nor closes.
pub(crate) fn braced_end(bytes: &[u8], open: usize) -> Result<usize, String> {
    let mut level = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => level += 1,
            b'}' => {
                level -= 1;
                if level == 0 {
                    return Ok(i + 1);
                }
            }
            b'\\' => i += 1,
            _ => {}
        }
        i += 1;
    }
    Err("missing close-brace".into())
}

/// `Tcl_ParseQuotedString`: the offset after the quote that closes the one
/// at `open`.
pub(crate) fn quoted_end(bytes: &[u8], open: usize) -> Result<usize, String> {
    let close = tokens(bytes, open + 1, |byte| byte == b'"', false)?;
    if close == bytes.len() {
        return Err("missing \"".into());
    }
    Ok(close + 1)
}

/// `ParseTokens`: scan text, variable references, command substitutions and
/// backslash sequences from `i` to the first byte outside them that `stop`
/// accepts; its offset, or the end. In an unquoted word (`word`) a
/// backslash-newline ends the text as white space would.
fn tokens(
    bytes: &[u8],
    mut i: usize,
    stop: impl Fn(u8) -> bool,
    word: bool,
) -> Result<usize, String> {
    while i < bytes.len() {
        let byte = bytes[i];
        if stop(byte) {
            return Ok(i);
        }
        match byte {
            b'$' => i = variable_at(bytes, i)?.map_or(i + 1, |variable| variable.end),
            b'[' => i = substitution_end(bytes, i)? + 1,
            b'\\' if word && bytes.get(i + 1) == Some(&b'\n') => return Ok(i),
            b'\\' => i = (i + 2).min(bytes.len()),
            _ => i += 1,
        }
    }
    Ok(bytes.len())
}

/// A variable reference: `$name`, `${name}` or `$name(index)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Variable {
    /// The name as written; braces enclose it in `${name}`.
    pub name: Range<usize>,
    pub braced: bool,
    /// The element between an array reference's parentheses.
    pub index: Option<Range<usize>>,
    /// The offset after the reference.
    pub end: usize,
}

/// `Tcl_ParseVarName` at the `$` at `dollar`, or `None` when no variable
/// name follows and the `$` is an ordinary character. A braced name runs to
/// the brace that closes the first, so it may hold balanced braces, and
/// escapes no characters but skips over them; any other name is letters,
/// digits, underscores and `::` separators, optionally followed by an array
/// index (which may be empty, as may the name of an array).
pub(crate) fn variable(text: &str, dollar: usize) -> Result<Option<Variable>, String> {
    variable_at(text.as_bytes(), dollar)
}

fn variable_at(bytes: &[u8], dollar: usize) -> Result<Option<Variable>, String> {
    let mut i = dollar + 1;
    if bytes.get(i) == Some(&b'{') {
        let start = i + 1;
        let mut depth = 0usize;
        i = start;
        loop {
            match bytes.get(i) {
                None => return Err("missing close-brace for variable name".into()),
                Some(b'}') if depth == 0 => break,
                Some(b'{') => depth += 1,
                Some(b'}') => depth -= 1,
                Some(b'\\') if i + 1 < bytes.len() => i += 1,
                Some(_) => {}
            }
            i += 1;
        }
        return Ok(Some(Variable {
            name: start..i,
            braced: true,
            index: None,
            end: i + 1,
        }));
    }
    let start = i;
    loop {
        match bytes.get(i) {
            Some(byte) if byte.is_ascii_alphanumeric() || *byte == b'_' => i += 1,
            Some(b':') if bytes.get(i + 1) == Some(&b':') => {
                while bytes.get(i) == Some(&b':') {
                    i += 1;
                }
            }
            _ => break,
        }
    }
    let name = start..i;
    if bytes.get(i) != Some(&b'(') {
        return Ok((!name.is_empty()).then_some(Variable {
            name,
            braced: false,
            index: None,
            end: i,
        }));
    }
    let index = i + 1;
    let close = tokens(
        bytes,
        index,
        |byte| matches!(byte, b'(' | b')' | b'"' | b'{' | b'}'),
        false,
    )?;
    match bytes.get(close) {
        Some(b')') => Ok(Some(Variable {
            name,
            braced: false,
            index: Some(index..close),
            end: close + 1,
        })),
        Some(_) => Err("invalid character in array index".into()),
        None => Err("missing )".into()),
    }
}

/// The offset of the `]` that closes the command substitution opening at
/// `open`: the first that ends a command of the nested script.
pub(crate) fn substitution_end(bytes: &[u8], open: usize) -> Result<usize, String> {
    let mut i = open + 1;
    loop {
        let (_, end) = command(bytes, comments(bytes, i), Nesting::Substitution)?;
        match bytes.get(end) {
            Some(b']') => return Ok(end),
            Some(_) => i = end + 1,
            None => return Err("missing close-bracket".into()),
        }
    }
}

/// The commands of a script, each from its first word to its last, leaving
/// out comments, white space and terminators.
pub(crate) fn script_ranges(source: &str) -> Result<Vec<Range<usize>>, String> {
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let (words, end) = command(bytes, comments(bytes, i), Nesting::Script)
            .map_err(|reason| format!("Tcl syntax is incomplete or malformed: {reason}"))?;
        if let (Some(first), Some(last)) = (words.first(), words.last()) {
            ranges.push(first.start..last.end);
        }
        i = end + 1;
    }
    Ok(ranges)
}

/// The words of a single command, each with its `{*}` prefix, braces or
/// quotes.
pub(crate) fn scan_words(text: &str) -> Result<Vec<Range<usize>>, String> {
    let bytes = text.as_bytes();
    let (words, end) = command(bytes, 0, Nesting::Script)?;
    if end < bytes.len() && comments(bytes, end + 1) < bytes.len() {
        return Err("text holds more than one command".into());
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<&str> {
        scan_words(text)
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .into_iter()
            .map(|word| &text[word])
            .collect()
    }

    #[test]
    fn braces_and_quotes_group_only_at_the_start_of_a_word() {
        assert_eq!(words("set x a{b}c"), ["set", "x", "a{b}c"]);
        assert_eq!(words("set x a\"b c\""), ["set", "x", "a\"b", "c\""]);
        assert_eq!(words("puts }"), ["puts", "}"]);
        assert_eq!(words("puts a]b"), ["puts", "a]b"]);
        assert_eq!(
            words("set x {a b} \"c d\""),
            ["set", "x", "{a b}", "\"c d\""]
        );
        assert_eq!(words("list {*}$a {*}{b c}"), ["list", "{*}$a", "{*}{b c}"]);
        assert_eq!(words("list {*} x"), ["list", "{*}", "x"]);
        for bad in ["set x {a}b", "set x \"a\"b", "set x {a", "set x \"a"] {
            assert!(scan_words(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn variable_references_follow_tcl() {
        assert_eq!(words("set x ${a}b"), ["set", "x", "${a}b"]);
        assert_eq!(words("set x ${a b}"), ["set", "x", "${a b}"]);
        assert_eq!(words("puts $a(x y)"), ["puts", "$a(x y)"]);
        assert_eq!(words("puts $a([f ])"), ["puts", "$a([f ])"]);
        let reference = |text: &str| variable(text, 0).unwrap();
        assert_eq!(reference("${a{b}c}x").unwrap().name, 2..7);
        assert_eq!(reference("${a\\}b}").unwrap().name, 2..6);
        assert_eq!(reference("$ns::x:y").unwrap().name, 1..6);
        assert_eq!(reference("$(k)").unwrap().index, Some(2..3));
        assert_eq!(reference("$ x"), None);
        assert_eq!(reference("$"), None);
        assert!(variable("$a(b", 0).is_err());
        assert!(variable("$a(b{c)", 0).is_err());
        assert!(variable("${a", 0).is_err());
    }

    #[test]
    fn command_substitutions_end_at_their_own_bracket() {
        let end = |text: &str| substitution_end(text.as_bytes(), 0).unwrap();
        assert_eq!(end("[list a]b"), 7);
        assert_eq!(end("[list {]} \"]\" $a(])]"), 19);
        assert_eq!(end("[set x 1; list ]"), 15);
        assert_eq!(end("[list a{]"), 8);
        assert!(substitution_end(b"[list a", 0).is_err());
        assert!(substitution_end(b"[# comment ]", 0).is_err());
    }

    #[test]
    fn scripts_split_into_commands() {
        let script = "set a 1;# note\n  # comment {\nputs \"x;y\" ; set b {\n}\n";
        let commands = script_ranges(script)
            .unwrap()
            .into_iter()
            .map(|range| &script[range])
            .collect::<Vec<_>>();
        assert_eq!(commands, ["set a 1", "puts \"x;y\"", "set b {\n}"]);
        assert!(script_ranges("set x {").is_err());
        assert!(script_ranges("set x a{b; set y c}").is_ok());
    }
}
