//! Expression AST lowering. Lazy operands retain mapped command substitutions;
//! no expression is handed to an opaque script evaluator.
use super::*;
use tcld_core::inst_table::BuiltInMathFunction;

/// Tcl 9's builtin math functions (`BuiltinFuncTable` in `tclBasic.c`).
const MATH_FUNCTIONS: [&str; 37] = [
    "abs",
    "acos",
    "asin",
    "atan",
    "atan2",
    "bool",
    "ceil",
    "cos",
    "cosh",
    "double",
    "entier",
    "exp",
    "floor",
    "fmod",
    "hypot",
    "int",
    "isfinite",
    "isinf",
    "isnan",
    "isnormal",
    "isqrt",
    "issubnormal",
    "isunordered",
    "log",
    "log10",
    "max",
    "min",
    "pow",
    "rand",
    "round",
    "sin",
    "sinh",
    "sqrt",
    "srand",
    "tan",
    "tanh",
    "wide",
];

enum Expr {
    Literal(String),
    /// `$name`, or `${name}` with any name.
    Variable(String),
    /// `$name(index)`: the array, and the absolute span of the index, which
    /// substitutes like a word.
    Element(String, Range<usize>),
    /// A quoted string with substitutions: the absolute span of the word,
    /// quotes included.
    Word(Range<usize>),
    Script(Range<usize>),
    Unary(Opcode, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Call(usize, Vec<Expr>),
    /// Any other builtin math function, or a legacy builtin called with the
    /// wrong number of arguments: called by name, as Tcl invokes
    /// `tcl::mathfunc::name`, so arity errors surface at runtime.
    Function(String, Vec<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
}
struct Parser<'a> {
    text: &'a str,
    base: usize,
    cursor: usize,
}
impl Parser<'_> {
    fn whitespace(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
    }
    fn take(&mut self, s: &str) -> bool {
        self.whitespace();
        if self.text[self.cursor..].starts_with(s) {
            self.cursor += s.len();
            true
        } else {
            false
        }
    }
    fn parse(mut self) -> Result<Expr, String> {
        let result = self.expression(0)?;
        self.whitespace();
        if self.cursor != self.text.len() {
            return Err(format!(
                "unsupported expression near {:?}",
                &self.text[self.cursor..]
            ));
        }
        Ok(result)
    }
    fn expression(&mut self, min: u8) -> Result<Expr, String> {
        let mut left = self.atom()?;
        loop {
            self.whitespace();
            let remaining = &self.text[self.cursor..];
            if min == 0 && remaining.starts_with('?') {
                self.cursor += 1;
                let yes = self.expression(0)?;
                if !self.take(":") {
                    return Err("conditional expression requires ':'".into());
                }
                let no = self.expression(0)?;
                left = Expr::Conditional(Box::new(left), Box::new(yes), Box::new(no));
                continue;
            }
            let op = [
                "||", "&&", "eq", "ne", "in", "ni", "==", "!=", "<=", ">=", "<<", ">>", "**", "|",
                "^", "&", "<", ">", "+", "-", "*", "/", "%",
            ]
            .into_iter()
            .find(|op| remaining.starts_with(op));
            let Some(op) = op else { break };
            let priority = match op {
                "||" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "eq" | "ne" | "in" | "ni" | "==" | "!=" => 6,
                "<" | ">" | "<=" | ">=" => 7,
                "<<" | ">>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                "**" => 12,
                _ => unreachable!(),
            };
            if priority < min {
                break;
            }
            self.cursor += op.len();
            let right = self.expression(if op == "**" { priority } else { priority + 1 })?;
            left = Expr::Binary(op.into(), Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn atom(&mut self) -> Result<Expr, String> {
        self.whitespace();
        let start = self.cursor;
        let byte = *self
            .text
            .as_bytes()
            .get(start)
            .ok_or("missing expression operand")?;
        if matches!(byte, b'+' | b'-' | b'!' | b'~') {
            self.cursor += 1;
            return Ok(Expr::Unary(
                match byte {
                    b'+' => Opcode::Uplus,
                    b'-' => Opcode::Uminus,
                    b'!' => Opcode::Not,
                    _ => Opcode::Bitnot,
                },
                Box::new(self.expression(13)?),
            ));
        }
        if self.take("(") {
            let value = self.expression(0)?;
            if !self.take(")") {
                return Err("unclosed expression parenthesis".into());
            }
            return Ok(value);
        }
        if byte == b'[' {
            let end = matching_bracket(self.text, start)?;
            self.cursor = end + 1;
            return Ok(Expr::Script(self.base + start + 1..self.base + end));
        }
        if byte == b'$' {
            let reference = words::variable(self.text, start)?.ok_or("invalid character \"$\"")?;
            let name = self.text[reference.name.clone()].to_string();
            self.cursor = reference.end;
            return Ok(match reference.index {
                Some(index) => Expr::Element(name, self.base + index.start..self.base + index.end),
                None => Expr::Variable(name),
            });
        }
        // A quoted string substitutes like a word; one without substitutions
        // is a literal, as is any braced string.
        if byte == b'"' {
            let end = words::quoted_end(self.text.as_bytes(), start)?;
            self.cursor = end;
            return Ok(match word_literal(self.text, start..end) {
                Ok(value) => Expr::Literal(value),
                Err(_) => Expr::Word(self.base + start..self.base + end),
            });
        }
        if byte == b'{' {
            let end = words::braced_end(self.text.as_bytes(), start)?;
            self.cursor = end;
            return Ok(Expr::Literal(braced_literal_raw(&self.text[start..end])?));
        }
        if byte.is_ascii_digit() || byte == b'.' {
            self.cursor += 1;
            while let Some(&ch) = self.text.as_bytes().get(self.cursor) {
                if ch.is_ascii_alphanumeric()
                    || matches!(ch, b'.' | b'_')
                    || (matches!(ch, b'+' | b'-')
                        && matches!(self.text.as_bytes().get(self.cursor - 1), Some(b'e' | b'E')))
                {
                    self.cursor += 1;
                } else {
                    break;
                }
            }
            // Tcl stores a numeric literal in canonical form: `abs(0x10)` is 16.
            let raw = &self.text[start..self.cursor];
            let literal = tcld_core::number::Number::parse(raw.as_bytes())
                .map_or_else(|| raw.to_string(), |n| n.to_canonical_string());
            return Ok(Expr::Literal(literal));
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            self.cursor += 1;
            while self
                .text
                .as_bytes()
                .get(self.cursor)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                self.cursor += 1;
            }
            let name = &self.text[start..self.cursor];
            if self.take("(") {
                let mut args = Vec::new();
                if !self.take(")") {
                    loop {
                        args.push(self.expression(0)?);
                        if self.take(")") {
                            break;
                        }
                        if !self.take(",") {
                            return Err("expected ',' between math function arguments".into());
                        }
                    }
                }
                if let Some(index) = BuiltInMathFunction::ALL
                    .iter()
                    .position(|f| f.name() == name && f.argc() as usize == args.len())
                {
                    return Ok(Expr::Call(index, args));
                }
                if !MATH_FUNCTIONS.contains(&name) {
                    return Err(format!(
                        "math function {name:?} is not a compiler-owned builtin"
                    ));
                }
                return Ok(Expr::Function(name.into(), args));
            }
            return match name {
                "true" | "yes" | "on" | "false" | "no" | "off" | "Inf" | "NaN" => {
                    Ok(Expr::Literal(name.into()))
                }
                _ => Err(format!("invalid bareword in expression: {name}")),
            };
        }
        Err(format!(
            "unsupported expression operand near {:?}",
            &self.text[start..]
        ))
    }
}

impl Compiler<'_> {
    fn parse_expression(&self, span: Range<usize>) -> Result<Expr, CompileDiagnostic> {
        Parser {
            text: &self.source[span.clone()],
            base: span.start,
            cursor: 0,
        }
        .parse()
        .map_err(|e| diagnostic(self.path, self.source, span.clone(), &self.source[span], e))
    }
    pub(super) fn compile_expression(
        &mut self,
        span: Range<usize>,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let expr = self.parse_expression(span)?;
        self.emit_expression(&expr, depth)
    }
    pub(super) fn compile_expression_command(
        &mut self,
        text: &str,
        words: &[Range<usize>],
        span: Range<usize>,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        if words.len() < 2 {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                "expr requires an expression".into(),
            ));
        }
        let first = &words[1];
        let last = words.last().unwrap();
        let raw = &text[first.start..last.end];
        let grouped = words.len() == 2 && (raw.starts_with('{') || raw.starts_with('"'));
        let (start, end) = if grouped {
            (first.start + 1, last.end - 1)
        } else {
            (first.start, last.end)
        };
        // Unbraced Tcl performs substitutions before expression parsing. Admit only
        // numeric nested expr substitutions here; reject ambiguous dynamic injection
        // rather than silently changing its precedence or lazy evaluation semantics.
        if !raw.starts_with('{') {
            let mut i = start;
            let bytes = text.as_bytes();
            let mut has_substitution = false;
            while i < end {
                if bytes[i] == b'$' {
                    return Err(diagnostic(
                        self.path,
                        self.source,
                        span,
                        text,
                        "brace expressions containing variable substitutions".into(),
                    ));
                }
                if bytes[i] == b'[' {
                    has_substitution = true;
                    let close = matching_bracket(text, i).map_err(|reason| {
                        diagnostic(self.path, self.source, span.clone(), text, reason)
                    })?;
                    let nested = &text[i + 1..close];
                    let nested_words = scan_words(nested)
                        .map_err(|e| diagnostic(self.path, self.source, span.clone(), text, e))?;
                    if nested_words
                        .first()
                        .and_then(|w| word_literal(nested, w.clone()).ok())
                        .as_deref()
                        != Some("expr")
                        || !numeric_script(nested)
                    {
                        return Err(diagnostic(self.path,self.source,span,text,"brace expressions containing command substitutions other than numeric expr".into()));
                    }
                    i = close + 1;
                } else {
                    i += 1;
                }
            }
            if has_substitution
                && ["&&", "||", "?"]
                    .iter()
                    .any(|operator| raw.contains(operator))
            {
                return Err(diagnostic(
                    self.path,
                    self.source,
                    span,
                    text,
                    "brace conditional expressions containing substitutions".into(),
                ));
            }
        }
        let expr = self.parse_expression(span.start + start..span.start + end)?;
        self.emit_expression(&expr, depth + 1)?;
        // As in Tcl, a value taken straight from an operand, a function or a
        // ?: branch comes out as the canonical number (`expr {0x10}` is 16);
        // operators already produce numbers, and operands inside an
        // expression keep their strings (`$x eq "0x10"`).
        if !matches!(expr, Expr::Unary(..) | Expr::Binary(..)) {
            self.emit(Opcode::TryCvtToNumeric);
        }
        Ok(())
    }

    /// Compile an operand's span of the source as the word it substitutes
    /// like: a quoted string, or an array index. Its command substitutions
    /// nest as deep as the expression's own, one level inside the command.
    fn compile_operand_word(
        &mut self,
        span: Range<usize>,
        depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let source = self.source;
        let word = 0..span.len();
        self.compile_word(
            &source[span.clone()],
            word,
            span.start,
            depth.saturating_sub(1),
        )
    }
    fn emit_expression(&mut self, expr: &Expr, depth: u32) -> Result<(), CompileDiagnostic> {
        match expr {
            Expr::Literal(value) => self.push_literal(value),
            Expr::Variable(name) => {
                self.push_literal(name);
                self.emit(Opcode::LoadScalarStk);
            }
            Expr::Element(name, index) => {
                self.push_literal(name);
                self.compile_operand_word(index.clone(), depth)?;
                self.emit(Opcode::LoadArrayStk);
            }
            Expr::Word(span) => self.compile_operand_word(span.clone(), depth)?,
            Expr::Script(span) => self.compile_script(span.clone(), depth)?,
            Expr::Unary(op, value) => {
                self.emit_expression(value, depth)?;
                self.emit(*op);
            }
            Expr::Call(index, args) => {
                for arg in args {
                    self.emit_expression(arg, depth)?;
                }
                self.emit_arg(Opcode::CallBuiltinFunc1, *index as u32);
            }
            Expr::Function(name, args) => {
                self.push_literal(name);
                for arg in args {
                    self.emit_expression(arg, depth)?;
                }
                self.emit_arg(Opcode::CallFunc1, args.len() as u32 + 1);
            }
            Expr::Conditional(condition, yes, no) => {
                self.emit_expression(condition, depth)?;
                let otherwise = self.jump(Opcode::JumpFalse4);
                self.emit_expression(yes, depth)?;
                let end = self.jump(Opcode::Jump4);
                self.patch_jump(otherwise, self.code.len());
                self.emit_expression(no, depth)?;
                self.patch_jump(end, self.code.len());
            }
            Expr::Binary(op, left, right) if op == "&&" || op == "||" => {
                self.emit_expression(left, depth)?;
                let short = self.jump(if op == "&&" {
                    Opcode::JumpFalse4
                } else {
                    Opcode::JumpTrue4
                });
                self.emit_expression(right, depth)?;
                self.emit(Opcode::Not);
                self.emit(Opcode::Not);
                let end = self.jump(Opcode::Jump4);
                self.patch_jump(short, self.code.len());
                self.push_literal(if op == "&&" { "0" } else { "1" });
                self.patch_jump(end, self.code.len());
            }
            Expr::Binary(op, left, right) => {
                self.emit_expression(left, depth)?;
                self.emit_expression(right, depth)?;
                let opcode = match op.as_str() {
                    "+" => Opcode::Add,
                    "-" => Opcode::Sub,
                    "*" => Opcode::Mult,
                    "/" => Opcode::Div,
                    "%" => Opcode::Mod,
                    "**" => Opcode::Expon,
                    "==" => Opcode::Eq,
                    "!=" => Opcode::Neq,
                    "<" => Opcode::Lt,
                    ">" => Opcode::Gt,
                    "<=" => Opcode::Le,
                    ">=" => Opcode::Ge,
                    "<<" => Opcode::Lshift,
                    ">>" => Opcode::Rshift,
                    "&" => Opcode::Bitand,
                    "|" => Opcode::Bitor,
                    "^" => Opcode::Bitxor,
                    "eq" => Opcode::StrEq,
                    "ne" => {
                        self.emit(Opcode::StrEq);
                        Opcode::Not
                    }
                    "in" => Opcode::ListIn,
                    "ni" => Opcode::ListNotIn,
                    _ => unreachable!(),
                };
                self.emit(opcode);
            }
        }
        Ok(())
    }
}

// In an unbraced expression Tcl reparses substitution results as syntax. Only
// statically numeric results preserve the AST boundary used by this compiler.
fn numeric_script(text: &str) -> bool {
    let Ok(commands) = source::script_ranges(text) else {
        return false;
    };
    if commands.len() != 1 {
        return false;
    }
    let text = &text[commands[0].clone()];
    let Ok(words) = scan_words(text) else {
        return false;
    };
    if words.len() < 2 || word_literal(text, words[0].clone()).ok().as_deref() != Some("expr") {
        return false;
    }
    let mut raw = &text[words[1].start..words.last().unwrap().end];
    if words.len() == 2 && (raw.starts_with('{') || raw.starts_with('"')) {
        raw = &raw[1..raw.len() - 1];
    }
    fn numeric(expr: &Expr, text: &str) -> bool {
        match expr {
            Expr::Literal(s) => {
                s.parse::<f64>().is_ok()
                    || ["0x", "0o", "0b"].iter().any(|prefix| {
                        s.starts_with(prefix)
                            && i64::from_str_radix(
                                &s[2..],
                                match *prefix {
                                    "0x" => 16,
                                    "0o" => 8,
                                    _ => 2,
                                },
                            )
                            .is_ok()
                    })
            }
            Expr::Variable(_) | Expr::Element(..) | Expr::Word(_) => false,
            Expr::Script(span) => numeric_script(&text[span.clone()]),
            Expr::Unary(..) | Expr::Binary(..) | Expr::Call(..) | Expr::Function(..) => true,
            Expr::Conditional(_, yes, no) => numeric(yes, text) && numeric(no, text),
        }
    }
    Parser {
        text: raw,
        base: 0,
        cursor: 0,
    }
    .parse()
    .is_ok_and(|expr| numeric(&expr, raw))
}
