//! Rendering functions for individual CommandKind variants.
//!
//! This module extracts the rendering logic from `Command::write_source_repr()` into
//! focused, reusable functions. Each function handles the rendering of a specific
//! command kind, reducing the complexity of the main dispatch method.

use crate::code::{CmdBody, CmdJump, CmdJumpType, CodeRef};
use crate::source::SourceRef;
use std::borrow::Cow;
use std::fmt::Write;

const INDENT: &str = "    ";

/// Push a word to the output string, adding a space if needed.
pub(crate) fn push_word(out: &mut String, word: &str) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(word);
}

/// Push a SourceRef as a word, adding a space if needed.
pub(crate) fn push_word_source(out: &mut String, value: &SourceRef) {
    if !out.is_empty() {
        out.push(' ');
    }
    value.write_repr(out);
}

/// Wrap a string in braces, creating a Tcl literal block.
pub(crate) fn wrap_in_braces(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('{');
    out.push_str(input);
    out.push('}');
    out
}

/// Escape special characters for Tcl quoted literals (double-quoted strings).
pub(crate) fn escape_tcl_quoted_literal(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '}' => out.push_str("\\}"),
            '{' => out.push_str("\\{"),
            '"' => out.push_str("\\\""),
            '[' => out.push_str("\\["),
            ']' => out.push_str("\\]"),
            '$' => out.push_str("\\$"),
            _ => out.push(ch),
        }
    }
    out
}

/// Normalize a procedure body, handling quoting and bracing conventions.
pub(crate) fn normalize_proc_body(body: &str) -> Cow<'_, str> {
    let body_trim = body.trim();
    let is_subst = body_trim.starts_with('$') || body_trim.starts_with('[');
    let is_quoted = body_trim.starts_with('"') && body_trim.ends_with('"');
    if is_subst || is_quoted {
        return Cow::Borrowed(body);
    }
    if body.contains('\n') {
        if body.starts_with('{') && body.ends_with('}') {
            return Cow::Borrowed(body);
        }
        return Cow::Owned(wrap_in_braces(body));
    }
    if body.starts_with('{') && body.ends_with('}') && body.len() >= 2 {
        let inner = body[1..body.len() - 1].trim();
        if !inner.contains('\n') {
            return Cow::Owned(wrap_in_braces(inner));
        }
        return Cow::Borrowed(body);
    }
    if !body_trim.contains('\n') {
        return Cow::Owned(wrap_in_braces(body_trim));
    }
    if !body.starts_with('{') {
        return Cow::Owned(wrap_in_braces(body_trim));
    }
    Cow::Borrowed(body)
}

/// Check if a string is a simple expression literal (e.g., an integer).
pub(crate) fn is_simple_expr_literal(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if first == '+' || first == '-' {
        return chars.all(|c| c.is_ascii_digit());
    }
    if !first.is_ascii_digit() {
        return false;
    }
    chars.all(|c| c.is_ascii_digit())
}

/// Check if a string is a bare expression word (unquoted identifier).
pub(crate) fn is_bare_expr_word(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    if s.starts_with('$') || s.starts_with('[') || s.starts_with('{') || s.starts_with('"') {
        return false;
    }
    if is_simple_expr_literal(s) {
        return false;
    }
    if s.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    true
}

/// Write an operator expression (e.g., "a + b" or "a == b").
pub(crate) fn write_op_expr(out: &mut String, op: &str, args: &[SourceRef]) {
    if op != "==" && op != "!=" && op != "eq" && op != "ne" {
        if let Some(second) = args.get(1) {
            if let Some(first) = args.first() {
                first.write_repr(out);
            }
            out.push(' ');
            out.push_str(op);
            out.push(' ');
            second.write_repr(out);
        } else {
            out.push_str(op);
            out.push(' ');
            if let Some(first) = args.first() {
                first.write_repr(out);
            }
        }
        return;
    }

    let mut first = args
        .first()
        .map(|a| a.repr_cow())
        .unwrap_or(Cow::Borrowed(""));
    let mut second = args.get(1).map(|a| a.repr_cow());
    let has_bare =
        is_bare_expr_word(&first) || second.as_ref().is_some_and(|s| is_bare_expr_word(s));
    if has_bare {
        if !first.starts_with('"') && !first.starts_with('{') && !first.starts_with('[') {
            let mut quoted = String::with_capacity(first.len() + 2);
            quoted.push('"');
            quoted.push_str(&first);
            quoted.push('"');
            first = Cow::Owned(quoted);
        }
        if let Some(s) = &mut second
            && !s.starts_with('"')
            && !s.starts_with('{')
            && !s.starts_with('[')
        {
            let mut quoted = String::with_capacity(s.len() + 2);
            quoted.push('"');
            quoted.push_str(s);
            quoted.push('"');
            *s = Cow::Owned(quoted);
        }
    }
    if let Some(second) = second {
        out.push_str(&first);
        out.push(' ');
        out.push_str(op);
        out.push(' ');
        out.push_str(&second);
    } else {
        out.push_str(op);
        out.push(' ');
        out.push_str(&first);
    }
}

/// Write a function call expression (e.g., "sin(x)").
pub(crate) fn write_expr_func(out: &mut String, name: &str, args: &[SourceRef]) {
    if args.is_empty() {
        out.push_str(name);
        out.push_str("()");
        return;
    }
    out.push_str(name);
    out.push('(');
    for (idx, arg) in args.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        arg.write_repr(out);
    }
    out.push(')');
}

/// Write a procedure call in expression context.
pub(crate) fn write_expr_proc_call(out: &mut String, args: &[SourceRef], stackn: i32) {
    if args.is_empty() {
        return;
    }
    if args[0].repr_eq_str("string equal") && args.len() >= 3 {
        write_op_expr(out, "eq", &args[1..3]);
        return;
    }
    if stackn > 0 {
        out.push('[');
    }
    if args[0].repr_eq_str("::tcl::array::set") {
        out.push_str("array set");
    } else if args[0].repr_eq_str("tbcload::bcproc") {
        out.push_str("proc");
    } else {
        args[0].write_repr(out);
    }
    for arg in args.iter().skip(1) {
        out.push(' ');
        arg.write_repr(out);
    }
    if stackn > 0 {
        out.push(']');
    }
}

/// Append a string concatenation part, handling escaping.
pub(crate) fn append_strcat_part(out: &mut String, part: &str) {
    if part.is_empty() {
        return;
    }
    if part.starts_with('$') || part.starts_with('[') {
        out.push_str(part);
        return;
    }
    if part.starts_with('"') && part.ends_with('"') && part.len() >= 2 {
        out.push_str(&part[1..part.len() - 1]);
        return;
    }
    let literal = if part.starts_with('{') && part.ends_with('}') && part.len() >= 2 {
        &part[1..part.len() - 1]
    } else {
        part
    };
    for ch in literal.chars() {
        // Braces must be escaped too: this fragment sits inside a "..."
        // word, but that word may itself sit inside a braced proc body,
        // where Tcl counts braces even within double quotes. An
        // unescaped `{` (e.g. the fragment `{` in `"{$key\}"`) leaves the
        // body unbalanced. `\{` / `\}` re-parse to the same characters.
        if matches!(ch, '\\' | '"' | '$' | '[' | ']' | '{' | '}') {
            out.push('\\');
        }
        out.push(ch);
    }
}

use crate::util::push_indented_lines;

/// Render a Literal command.
pub(crate) fn render_literal(out: &mut String, value: &SourceRef, stackn: i32) {
    if stackn == 0 {
        return;
    }
    let mut rep = String::new();
    value.write_repr(&mut rep);
    if rep.is_empty() {
        out.push_str("{}");
        return;
    }

    let special = |c: char| {
        matches!(
            c,
            '$' | '[' | ']' | '{' | '}' | '"' | '\r' | '\n' | '\t' | ' '
        )
    };
    if rep.chars().all(|c| !special(c)) {
        out.push_str(&rep);
        return;
    }

    // A value can be brace-wrapped only if its braces are balanced *and*
    // never dip negative — otherwise the wrapping braces don't re-parse.
    // `bracket_level < 0` catches a `}` before its `{`; the final
    // `bracket_level != 0` check catches a trailing-unbalanced `{` (the
    // value `{` must become `"\{"`, never the malformed `{{}`).
    let mut matching_brackets = true;
    let mut bracket_level = 0i32;
    for c in rep.chars() {
        if c == '{' {
            bracket_level += 1;
        } else if c == '}' {
            bracket_level -= 1;
        }
        if bracket_level < 0 {
            matching_brackets = false;
            break;
        }
    }
    if bracket_level != 0 {
        matching_brackets = false;
    }

    if rep.contains('\r') || !matching_brackets {
        out.push('"');
        out.push_str(&escape_tcl_quoted_literal(&rep));
        out.push('"');
    } else {
        out.push('{');
        out.push_str(&rep);
        out.push('}');
    }
}

/// Render a ProcCall command.
pub(crate) fn render_proc_call(out: &mut String, args: &[SourceRef], stackn: i32) {
    if args.is_empty() {
        return;
    }
    let mut rep_buf = String::new();
    let head = &args[0];
    let is_array_set = head.repr_eq_str("::tcl::array::set");
    let is_proc = head.repr_eq_str("proc") || head.repr_eq_str("tbcload::bcproc");
    if is_array_set {
        push_word(&mut rep_buf, "array");
        push_word(&mut rep_buf, "set");
    } else if head.repr_eq_str("tbcload::bcproc") {
        push_word(&mut rep_buf, "proc");
    } else {
        push_word_source(&mut rep_buf, head);
    }
    for (idx, arg) in args.iter().enumerate().skip(1) {
        let arg_rep = arg.repr_cow();
        if is_proc && idx == 2 {
            if arg_rep.starts_with('{') {
                push_word(&mut rep_buf, &arg_rep);
            } else {
                let wrapped = wrap_in_braces(&arg_rep);
                push_word(&mut rep_buf, &wrapped);
            }
            continue;
        }
        if is_proc && idx == 3 {
            let normalized = normalize_proc_body(&arg_rep);
            push_word(&mut rep_buf, normalized.as_ref());
            continue;
        }
        push_word_source(&mut rep_buf, arg);
    }
    if stackn > 0 {
        out.push('[');
        out.push_str(&rep_buf);
        out.push(']');
    } else {
        out.push_str(&rep_buf);
    }
}

/// Render an ArrayRef command.
pub(crate) fn render_array_ref(out: &mut String, args: &[SourceRef]) {
    out.push('$');
    if let Some(first) = args.first() {
        first.write_repr(out);
    }
    if let Some(second) = args.get(1) {
        out.push('(');
        second.write_repr(out);
        out.push(')');
    }
}

/// Render a Variable command.
pub(crate) fn render_variable(out: &mut String, value: &SourceRef) {
    out.push('$');
    value.write_repr(out);
}

/// Render a Set command.
pub(crate) fn render_set(out: &mut String, var_name: &SourceRef, value: &SourceRef, stackn: i32) {
    if var_name.repr_is_empty() {
        value.write_repr(out);
        return;
    }
    if stackn > 0 {
        out.push('[');
    }
    out.push_str("set ");
    var_name.write_repr(out);
    out.push(' ');
    value.write_repr(out);
    if stackn > 0 {
        out.push(']');
    }
}

/// Render an Op (operator expression) command.
pub(crate) fn render_op(out: &mut String, op: &str, args: &[SourceRef], stackn: i32) {
    if stackn > 0 {
        out.push_str("[expr {");
        write_op_expr(out, op, args);
        out.push_str("}]");
    } else {
        out.push_str("expr {");
        write_op_expr(out, op, args);
        out.push('}');
    }
}

/// Render an ExprFunc (function expression) command.
pub(crate) fn render_expr_func(out: &mut String, name: &str, args: &[SourceRef], stackn: i32) {
    if stackn > 0 {
        out.push_str("[expr {");
        write_expr_func(out, name, args);
        out.push_str("}]");
    } else {
        out.push_str("expr {");
        write_expr_func(out, name, args);
        out.push('}');
    }
}

/// Render an Expr command.
pub(crate) fn render_expr(out: &mut String, expr: &SourceRef, stackn: i32) {
    let mut expr_rep = String::new();
    expr.write_repr(&mut expr_rep);
    let cleaned = if expr_rep.starts_with('"') && expr_rep.ends_with('"') && expr_rep.len() >= 2 {
        let mut inner = expr_rep[1..expr_rep.len() - 1].to_string();
        if inner.contains("{ }") {
            inner = inner.replace("{ }", " ");
        }
        if inner.contains("\"[") {
            inner = inner.replace("\"[", "[");
        }
        if inner.contains("]\"") {
            inner = inner.replace("]\"", "]");
        }
        if inner.contains(")\"") {
            inner = inner.replace(")\"", ")");
        }
        if inner.contains("\"(") {
            inner = inner.replace("\"(", "(");
        }
        Some(inner)
    } else {
        None
    };
    let simple_candidate = cleaned.as_deref().unwrap_or(&expr_rep);
    if is_simple_expr_literal(simple_candidate) {
        out.push_str(simple_candidate);
        return;
    }
    if stackn > 0 {
        out.push('[');
    }
    if let Some(inner) = cleaned {
        out.push_str("expr ");
        out.push_str(&inner);
    } else if expr_rep.chars().any(|c| c.is_whitespace()) {
        out.push_str("expr {");
        out.push_str(&expr_rep);
        out.push('}');
    } else {
        out.push_str("expr ");
        out.push_str(&expr_rep);
    }
    if stackn > 0 {
        out.push(']');
    }
}

/// Render a StrCat (string concatenation) command.
pub(crate) fn render_strcat(out: &mut String, args: &[SourceRef]) {
    let mut rep = String::new();
    for arg in args {
        let part = arg.repr_cow();
        append_strcat_part(&mut rep, &part);
    }
    out.push('"');
    out.push_str(&rep);
    out.push('"');
}

/// Render an ExpandedArg command.
pub(crate) fn render_expanded_arg(out: &mut String, value: &SourceRef) {
    out.push('*');
    value.write_repr(out);
}

/// Render a Catch command.
pub(crate) fn render_catch(out: &mut String, body: &CmdBody, stackn: i32) {
    let body_rep = body.source_repr();
    if stackn > 0 {
        out.push('[');
    }
    if body_rep.contains('\n') {
        out.push_str("catch {\n");
        push_indented_lines(out, &body_rep, INDENT);
        out.push_str("\n}");
    } else {
        out.push_str("catch {");
        out.push_str(&body_rep);
        out.push('}');
    }
    if stackn > 0 {
        out.push(']');
    }
}

/// Render a Foreach command.
pub(crate) fn render_foreach(
    out: &mut String,
    body: &CmdBody,
    lists: &[Vec<SourceRef>],
    values: &[SourceRef],
) {
    let mut head = String::new();
    for (idx, list) in lists.iter().enumerate() {
        let list_rep = if list.len() == 1 {
            let item = list[0].repr_cow();
            if item.contains(' ') {
                format!("{{{}}}", item)
            } else {
                item.into_owned()
            }
        } else {
            let mut items = String::new();
            for (item_idx, item) in list.iter().enumerate() {
                if item_idx > 0 {
                    items.push(' ');
                }
                let rep = item.repr_cow();
                items.push_str(&rep);
            }
            format!("{{{}}}", items)
        };
        if !head.is_empty() {
            head.push(' ');
        }
        head.push_str(&list_rep);
        if let Some(val) = values.get(idx) {
            head.push(' ');
            val.write_repr(&mut head);
        }
    }
    out.push_str("foreach ");
    out.push_str(&head);
    out.push_str(" {\n");
    let body_rep = body.source_repr();
    push_indented_lines(out, &body_rep, INDENT);
    out.push_str("\n}");
}

/// Render a DictUpdate command.
pub(crate) fn render_dict_update(
    out: &mut String,
    dict_var: &SourceRef,
    pairs: &[(SourceRef, SourceRef)],
    body: &CmdBody,
) {
    out.push_str("dict update ");
    dict_var.write_repr(out);
    for (key, var) in pairs {
        out.push(' ');
        key.write_repr(out);
        out.push(' ');
        var.write_repr(out);
    }
    out.push_str(" {\n");
    let body_rep = body.source_repr();
    push_indented_lines(out, &body_rep, INDENT);
    out.push_str("\n}");
}

/// Render a Switch command.
pub(crate) fn render_switch(
    out: &mut String,
    value: &SourceRef,
    arms: &[(Vec<SourceRef>, CmdBody)],
    default: Option<&CmdBody>,
    nocase: bool,
    integer: bool,
) {
    out.push_str("switch");
    if integer {
        out.push_str(" -integer");
    } else if nocase {
        out.push_str(" -nocase");
    }
    out.push_str(" -- ");
    value.write_repr(out);
    out.push_str(" {");
    for (patterns, body) in arms {
        if patterns.is_empty() {
            continue;
        }
        let body_rep = body.source_repr();
        for (pidx, pat) in patterns.iter().enumerate() {
            out.push('\n');
            out.push_str(INDENT);
            pat.write_repr(out);
            if pidx + 1 < patterns.len() {
                out.push_str(" -");
            } else {
                out.push_str(" {\n");
                push_indented_lines(out, &body_rep, INDENT);
                out.push('\n');
                out.push_str(INDENT);
                out.push('}');
            }
        }
    }
    if let Some(body) = default {
        let body_rep = body.source_repr();
        out.push('\n');
        out.push_str(INDENT);
        out.push_str("default {\n");
        push_indented_lines(out, &body_rep, INDENT);
        out.push('\n');
        out.push_str(INDENT);
        out.push('}');
    }
    out.push('\n');
    out.push('}');
}

/// Render an IfElse command.
pub(crate) fn render_if_else(
    out: &mut String,
    test: CodeRef,
    if_body: &CmdBody,
    else_body: &CmdBody,
) {
    out.push_str("if ");
    let test_rep = test.borrow().expr_repr();
    if is_simple_expr_literal(&test_rep) {
        out.push_str(&test_rep);
    } else {
        out.push('{');
        out.push_str(&test_rep);
        out.push('}');
    }
    out.push_str(" {\n");
    let if_rep = if_body.source_repr();
    push_indented_lines(out, &if_rep, INDENT);
    out.push_str("\n}");
    let else_rep = else_body.source_repr();
    if !else_rep.is_empty() && else_rep != "{}" {
        out.push_str(" else {\n");
        push_indented_lines(out, &else_rep, INDENT);
        out.push_str("\n}");
    }
}

/// Render a WhileLoop command.
pub(crate) fn render_while_loop(out: &mut String, body: &CmdBody, condition: &SourceRef) {
    let body_rep = body.source_repr();
    out.push_str("while {");
    condition.write_repr(out);
    out.push_str("} ");
    if body_rep.contains('\n') {
        out.push_str("{\n");
        push_indented_lines(out, &body_rep, INDENT);
        out.push_str("\n}");
    } else {
        out.push('{');
        out.push_str(&body_rep);
        out.push('}');
    }
}

/// Render a ForLoop command.
pub(crate) fn render_for_loop(
    out: &mut String,
    body: &CmdBody,
    setter: CodeRef,
    condition: CodeRef,
    incr: CodeRef,
) {
    let body_rep = body.source_repr();
    out.push_str("for {");
    setter.borrow().write_source_repr(out);
    out.push_str("} {");
    condition.borrow().write_expr_repr(out);
    out.push_str("} {");
    incr.borrow().write_source_repr(out);
    out.push_str("} ");
    if body_rep.contains('\n') {
        out.push_str("{\n");
        push_indented_lines(out, &body_rep, INDENT);
        out.push_str("\n}");
    } else {
        out.push('{');
        out.push_str(&body_rep);
        out.push('}');
    }
}

/// Render a Jump command.
pub(crate) fn render_jump(out: &mut String, info: &CmdJump) {
    match info.jump_type {
        CmdJumpType::Jump => out.push_str("Jump"),
        CmdJumpType::Break => out.push_str("Break"),
        CmdJumpType::Continue => out.push_str("Continue"),
    }
    out.push_str(" -> ");
    let _ = write!(out, "{}", info.target);
}

#[cfg(test)]
mod brace_balance_tests {
    //! Always-on guard for the two renderers that emit word literals
    //! (`render_literal`, `append_strcat_part`). A rendered word placed
    //! inside a braced proc body must keep the body's braces balanced, or
    //! the decompiled source won't re-parse (the public_complex_fixture.tbc `missing
    //! close-brace` class). These assert brace balance directly, without a
    //! Tcl interpreter; the fixture-wide `info complete` round-trip lives
    //! in `apps/tcld/tests/decompile_roundtrip.rs`.
    use super::*;
    use std::rc::Rc;

    /// True if `word`'s braces are balanced the way Tcl scans a braced
    /// word: `\{` / `\}` are escaped and do not count; the running depth
    /// must never go negative and must end at zero.
    fn braces_balanced(word: &str) -> bool {
        let mut depth = 0i32;
        let mut chars = word.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    chars.next(); // skip the escaped char, whatever it is
                }
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        depth == 0
    }

    fn literal(value: &str) -> String {
        let mut out = String::new();
        render_literal(&mut out, &SourceRef::Shared(Rc::from(value)), 1);
        out
    }

    fn strcat_part(part: &str) -> String {
        let mut out = String::new();
        append_strcat_part(&mut out, part);
        out
    }

    #[test]
    fn render_literal_keeps_braces_balanced_for_unbalanced_values() {
        // Each value is a legal bytecode literal but has unbalanced braces;
        // the rendered word must still be brace-balanced. `{` used to emit
        // the malformed `{{}`.
        for value in ["{", "}", "{a", "a}", "}{", "{{", "}}", "a{b", "a}b"] {
            let rendered = literal(value);
            assert!(
                braces_balanced(&rendered),
                "render_literal({value:?}) = {rendered:?} is not brace-balanced",
            );
        }
    }

    #[test]
    fn render_literal_preserves_already_balanced_values() {
        for value in ["{}", "{a}", "a{b}c", "plain"] {
            assert!(braces_balanced(&literal(value)));
        }
    }

    #[test]
    fn strcat_part_escapes_unbalanced_literal_fragments() {
        // The `"{$x\}"` class: a literal fragment `{` inside an interpolated
        // word must be escaped so the enclosing braced body stays balanced.
        for part in ["{", "}", "{a", "a}", "{{}"] {
            let rendered = strcat_part(part);
            assert!(
                braces_balanced(&rendered),
                "append_strcat_part({part:?}) = {rendered:?} is not brace-balanced",
            );
        }
    }

    #[test]
    fn strcat_part_passes_substitutions_through_unescaped() {
        // Substitutions must not be escaped — they carry no literal braces.
        let mut out = String::new();
        append_strcat_part(&mut out, "$fixture::key");
        assert_eq!(out, "$fixture::key");
    }
}
