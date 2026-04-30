use std::io;
use std::sync::{Arc, Mutex};

use tcld_tui::TokenId;
use tcld_tui::tokens;
use termosaic::{
    ColorChoice, ColorDepth, Console, ConsoleOptions, DestinationOptions, OutputClass, Span,
    Stream, TerminalCapabilities,
};

use crate::highlight::Segment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchColorPolicy {
    pub stdout_is_terminal: bool,
    pub term: Option<String>,
    pub no_colors: bool,
    pub no_color_env: Option<String>,
    pub colorterm: Option<String>,
}

impl BatchColorPolicy {
    pub fn from_environment(no_colors: bool, stdout_is_terminal: bool) -> Self {
        Self {
            stdout_is_terminal,
            term: std::env::var("TERM").ok(),
            no_colors,
            no_color_env: std::env::var("NO_COLOR").ok(),
            colorterm: std::env::var("COLORTERM").ok(),
        }
    }

    /// How much color the terminal can actually show.
    ///
    /// The theme is defined in RGB; Termosaic quantizes it down for terminals
    /// that cannot render truecolor, so a 256-color or 16-color terminal gets
    /// the nearest match rather than the wrong palette.
    fn color_depth(&self) -> ColorDepth {
        let colorterm = self.colorterm.as_deref().unwrap_or_default();
        if colorterm.eq_ignore_ascii_case("truecolor") || colorterm.eq_ignore_ascii_case("24bit") {
            return ColorDepth::TrueColor;
        }
        match self.term.as_deref() {
            Some(term) if term.contains("256") => ColorDepth::Ansi256,
            _ => ColorDepth::Ansi16,
        }
    }

    pub fn colors_enabled(&self) -> bool {
        self.stdout_is_terminal
            && !self.no_colors
            && self.term.as_deref() != Some("dumb")
            && self.no_color_env.as_deref().is_none_or(str::is_empty)
    }
}

pub fn render_decompile_output(output: &str, policy: BatchColorPolicy) -> String {
    if policy.colors_enabled() {
        highlight_decompile_output_at(output, policy.color_depth())
    } else {
        output.to_string()
    }
}

pub fn render_disassemble_output(output: &str, policy: BatchColorPolicy) -> String {
    if policy.colors_enabled() {
        highlight_disassemble_output_at(output, policy.color_depth())
    } else {
        output.to_string()
    }
}

/// Colour decompiled Tcl using the same Tree-sitter parse and the same semantic
/// theme the explorer uses, so identical source never renders two ways.
pub fn highlight_decompile_output(output: &str) -> String {
    highlight_decompile_output_at(output, ColorDepth::TrueColor)
}

fn highlight_decompile_output_at(output: &str, depth: ColorDepth) -> String {
    let lines: Vec<String> = output.lines().map(str::to_string).collect();
    render_segments(&crate::highlight::document_segments(&lines), output, depth)
}

pub fn highlight_disassemble_output(output: &str) -> String {
    highlight_disassemble_output_at(output, ColorDepth::TrueColor)
}

fn highlight_disassemble_output_at(output: &str, depth: ColorDepth) -> String {
    let segments: Vec<Vec<Segment>> = output.lines().map(disassemble_segments).collect();
    render_segments(&segments, output, depth)
}

// ── Termosaic rendering ─────────────────────────────────────────

/// An in-memory destination, so the batch commands keep returning a `String`
/// while Termosaic owns every escape sequence that reaches it.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("buffer lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Render segmented lines to an ANSI string.
///
/// `source` supplies the one thing segmentation cannot: whether the output ended
/// with a newline. Termosaic writes a line terminator after every line, so the
/// final one is removed when the input had none, keeping the coloured text
/// byte-identical to the plain text once escapes are stripped.
fn render_segments(lines: &[Vec<Segment>], source: &str, depth: ColorDepth) -> String {
    let buffer = Buffer::default();
    let destination = DestinationOptions {
        output_class: OutputClass::Human,
        color: ColorChoice::Always,
        terminal: TerminalCapabilities {
            is_terminal: true,
            color_depth: depth,
            // Batch output is piped as often as it is read; it must never be
            // rewrapped to a terminal width.
            width: None,
            interactive_cursor: false,
        },
    };
    let theme = tcld_tui::Theme::new(false).semantic().clone();
    let Ok(console) = Console::with_writers(
        buffer.clone(),
        io::sink(),
        ConsoleOptions {
            stdout: destination,
            stderr: destination,
            theme,
        },
    ) else {
        return source.to_string();
    };
    let Ok(mut human) = console.human(Stream::Stdout) else {
        return source.to_string();
    };

    for line in lines {
        let spans: Vec<Span> = line
            .iter()
            .map(|segment| Span::new(segment.token.clone(), &segment.text))
            .collect();
        if human.line(&spans).is_err() {
            return source.to_string();
        }
    }
    drop(human);

    let bytes = buffer.0.lock().expect("buffer lock").clone();
    let Ok(mut rendered) = String::from_utf8(bytes) else {
        return source.to_string();
    };
    if !source.ends_with('\n') {
        rendered.pop();
    }
    rendered
}

// ── Disassembly segmentation ────────────────────────────────────

fn segment(token: TokenId, text: &str) -> Segment {
    Segment {
        token,
        text: text.to_string(),
    }
}

/// Split one disassembly line into styled runs.
///
/// Bytecode listings have no grammar to parse, so this stays lexical — but it
/// emits the same tokens the instruction view uses, so the batch listing and the
/// explorer agree.
fn disassemble_segments(line: &str) -> Vec<Segment> {
    let plain = || vec![segment(tokens::FG, line)];
    let Some(close_paren) = line.find(')') else {
        return plain();
    };
    let opcode_start = line[close_paren + 1..]
        .char_indices()
        .find_map(|(offset, ch)| (!ch.is_whitespace()).then_some(close_paren + 1 + offset));
    let Some(opcode_start) = opcode_start else {
        return plain();
    };
    let opcode_end = line[opcode_start..]
        .char_indices()
        .find_map(|(offset, ch)| ch.is_whitespace().then_some(opcode_start + offset))
        .unwrap_or(line.len());
    let comment_start = line[opcode_end..]
        .find('#')
        .map(|offset| opcode_end + offset);
    let operands_end = comment_start.unwrap_or(line.len());

    let mut segments = vec![
        segment(tokens::INST_PC, &line[..opcode_start]),
        segment(tokens::INST_OPCODE, &line[opcode_start..opcode_end]),
    ];
    segments.extend(operand_segments(&line[opcode_end..operands_end]));
    if let Some(comment_start) = comment_start {
        segments.extend(comment_segments(&line[comment_start..]));
    }
    segments
}

fn operand_segments(operands: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    for (start, end) in token_spans(operands) {
        if start > cursor {
            segments.push(segment(tokens::FG, &operands[cursor..start]));
        }
        let text = &operands[start..end];
        let token = if text.parse::<isize>().is_ok() {
            tokens::INST_OPERAND
        } else {
            tokens::FG
        };
        segments.push(segment(token, text));
        cursor = end;
    }
    if cursor < operands.len() {
        segments.push(segment(tokens::FG, &operands[cursor..]));
    }
    segments
}

fn comment_segments(comment: &str) -> Vec<Segment> {
    let comment_token = termosaic::tokens::SYNTAX_COMMENT;
    let Some(target_start) = comment.find("pc ").map(|at| at + "pc ".len()) else {
        return vec![segment(comment_token, comment)];
    };
    let target_end = target_start
        + comment[target_start..]
            .find(|ch: char| !ch.is_ascii_digit() && ch != '-')
            .unwrap_or(comment[target_start..].len());

    let mut segments = vec![segment(comment_token.clone(), &comment[..target_start])];
    if target_start < target_end {
        // A jump target is a reference to another location, styled like the
        // group references the explorer uses for the same idea.
        segments.push(segment(
            tokens::DETAIL_GROUP_REF,
            &comment[target_start..target_end],
        ));
    }
    segments.push(segment(comment_token, &comment[target_end..]));
    segments
}

fn token_spans(input: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (idx, ch) in input.char_indices() {
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                spans.push((token_start, idx));
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(token_start) = start {
        spans.push((token_start, input.len()));
    }
    spans
}

#[cfg(test)]
pub(crate) mod tests {
    use tcld_decompile::{DecompileOptions, DisassembleOptions};

    fn fixture() -> String {
        format!("{}/tests/fixtures/expr.tbc", env!("CARGO_MANIFEST_DIR"))
    }

    fn strip_ansi(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' && chars.peek() == Some(&'[') {
                chars.next();
                for seq in chars.by_ref() {
                    if seq == 'm' {
                        break;
                    }
                }
                continue;
            }
            out.push(ch);
        }
        out
    }

    #[test]
    fn decompile_highlighting_color_policy_and_plain_text_are_stable() {
        let fixture = fixture();
        let plain = tcld_decompile::decompile_path_with_options(&fixture, DecompileOptions)
            .expect("fixture decompiles");
        let colored = super::highlight_decompile_output(&plain);

        assert!(
            colored.contains("\u{1b}["),
            "enabled policy should apply ANSI highlighting"
        );
        assert_eq!(strip_ansi(&colored), plain);

        let disabled_by_flag = super::render_decompile_output(
            &plain,
            super::BatchColorPolicy {
                stdout_is_terminal: true,
                term: Some("xterm-256color".to_string()),
                no_colors: true,
                no_color_env: None,
                colorterm: None,
            },
        );
        let disabled_by_env = super::render_decompile_output(
            &plain,
            super::BatchColorPolicy {
                stdout_is_terminal: true,
                term: Some("xterm-256color".to_string()),
                no_colors: false,
                no_color_env: Some("1".to_string()),
                colorterm: None,
            },
        );
        let disabled_by_pipe = super::render_decompile_output(
            &plain,
            super::BatchColorPolicy {
                stdout_is_terminal: false,
                term: Some("xterm-256color".to_string()),
                no_colors: false,
                no_color_env: None,
                colorterm: None,
            },
        );
        let disabled_by_empty_no_color_env = super::render_decompile_output(
            &plain,
            super::BatchColorPolicy {
                stdout_is_terminal: true,
                term: Some("xterm-256color".to_string()),
                no_colors: false,
                no_color_env: Some(String::new()),
                colorterm: None,
            },
        );

        assert!(!disabled_by_flag.contains("\u{1b}["));
        assert!(!disabled_by_env.contains("\u{1b}["));
        assert!(!disabled_by_pipe.contains("\u{1b}["));
        assert!(
            disabled_by_empty_no_color_env.contains("\u{1b}["),
            "empty NO_COLOR should not suppress color"
        );
        assert_eq!(disabled_by_flag, plain);
        assert_eq!(disabled_by_env, plain);
        assert_eq!(disabled_by_pipe, plain);
        assert_eq!(strip_ansi(&disabled_by_empty_no_color_env), plain);
    }

    #[test]
    fn disassemble_highlighting_color_policy_and_plain_text_are_stable() {
        let fixture = fixture();
        let plain = tcld_decompile::disassemble_path_with_options(
            &fixture,
            DisassembleOptions {
                text_truncate: None,
                line_width: Some(80),
            },
        )
        .expect("fixture disassembles");
        let colored = super::highlight_disassemble_output(&plain);

        assert!(
            colored.contains("\u{1b}["),
            "enabled policy should apply ANSI highlighting"
        );
        assert_eq!(strip_ansi(&colored), plain);

        let disabled_by_flag = super::render_disassemble_output(
            &plain,
            super::BatchColorPolicy {
                stdout_is_terminal: true,
                term: Some("xterm-256color".to_string()),
                no_colors: true,
                no_color_env: None,
                colorterm: None,
            },
        );
        let disabled_by_env = super::render_disassemble_output(
            &plain,
            super::BatchColorPolicy {
                stdout_is_terminal: true,
                term: Some("xterm-256color".to_string()),
                no_colors: false,
                no_color_env: Some("1".to_string()),
                colorterm: None,
            },
        );
        let disabled_by_dumb = super::render_disassemble_output(
            &plain,
            super::BatchColorPolicy {
                stdout_is_terminal: true,
                term: Some("dumb".to_string()),
                no_colors: false,
                no_color_env: None,
                colorterm: None,
            },
        );

        assert!(!disabled_by_flag.contains("\u{1b}["));
        assert!(!disabled_by_env.contains("\u{1b}["));
        assert!(!disabled_by_dumb.contains("\u{1b}["));
        assert_eq!(disabled_by_flag, plain);
        assert_eq!(disabled_by_env, plain);
        assert_eq!(disabled_by_dumb, plain);
    }

    /// The batch commands and the explorer must colour the same Tcl the same
    /// way. Both resolve the same semantic token against the same theme, so the
    /// escape the CLI writes is derivable from the theme itself rather than
    /// from a second table that can drift.
    #[test]
    fn batch_colours_come_from_the_same_theme_as_the_explorer() {
        use termosaic::Color;

        let theme = tcld_tui::Theme::new(false);
        let expected = |token: &termosaic::TokenId| match theme.semantic().resolve(token).foreground
        {
            Some(Color::Rgb {
                red, green, blue, ..
            }) => format!("38;2;{red};{green};{blue}"),
            other => panic!("token has no RGB foreground: {other:?}"),
        };

        let colored = super::highlight_decompile_output("proc demo {} {\n    set x \"hi\"\n}\n");

        for token in [
            termosaic::tokens::SYNTAX_KEYWORD,
            termosaic::tokens::SYNTAX_STRING,
        ] {
            let escape = expected(&token);
            let name = token.as_str().to_string();
            assert!(
                colored.contains(&escape),
                "{name} should render as {escape}\n{colored:?}"
            );
        }
    }

    #[test]
    fn colouring_preserves_the_text_and_its_final_newline() {
        for source in [
            "proc demo {} {\n    set x 1\n}\n",
            "proc demo {} {\n    set x 1\n}",
            "",
        ] {
            let colored = super::highlight_decompile_output(source);
            assert_eq!(
                strip_ansi(&colored),
                source,
                "colouring must not alter the bytes of {source:?}"
            );
        }
    }

    #[test]
    fn disassembly_colouring_preserves_its_text() {
        let source = "    (0) push1 0  # pc 12\n    (2) pop\n";
        let colored = super::highlight_disassemble_output(source);
        assert_eq!(strip_ansi(&colored), source);
    }

    #[test]
    fn colour_depth_follows_what_the_terminal_advertises() {
        use termosaic::ColorDepth;

        let policy = |term: &str, colorterm: Option<&str>| super::BatchColorPolicy {
            stdout_is_terminal: true,
            term: Some(term.to_string()),
            no_colors: false,
            no_color_env: None,
            colorterm: colorterm.map(str::to_string),
        };

        assert_eq!(
            policy("xterm-256color", Some("truecolor")).color_depth(),
            ColorDepth::TrueColor
        );
        assert_eq!(
            policy("xterm-256color", None).color_depth(),
            ColorDepth::Ansi256
        );
        assert_eq!(policy("xterm", None).color_depth(), ColorDepth::Ansi16);
    }

    /// A 16-colour terminal must receive 16-colour escapes, not truecolour ones.
    #[test]
    fn a_limited_terminal_gets_quantised_colour() {
        let policy = super::BatchColorPolicy {
            stdout_is_terminal: true,
            term: Some("xterm".to_string()),
            no_colors: false,
            no_color_env: None,
            colorterm: None,
        };
        let colored = super::render_decompile_output("proc demo {} {}\n", policy);

        assert!(colored.contains("\u{1b}["), "output is coloured");
        assert!(
            !colored.contains("38;2;"),
            "a 16-colour terminal must not receive truecolour escapes\n{colored:?}"
        );
    }
}
