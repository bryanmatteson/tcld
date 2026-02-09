use std::fs;
use std::path::{Path, PathBuf};

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalize_tcl(s: &str) -> String {
    let logical = join_line_continuations(&normalize_newlines(s));
    let filtered = strip_comments(&logical);
    let collapsed = collapse_single_command_proc_bodies(&filtered);
    let mut lines = Vec::new();
    for line in collapsed {
        if let Some(normalized) = normalize_line(&line) {
            lines.push(normalized);
        }
    }
    lines.join("\n")
}

fn join_line_continuations(s: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut pending: Option<String> = None;
    for raw in s.lines() {
        let mut line = raw.to_string();
        if line.ends_with('\\') {
            line.pop();
            if let Some(prev) = pending.take() {
                if line.trim().is_empty() {
                    line = prev;
                } else {
                    line = format!("{} {}", prev, line.trim_start());
                }
            }
            pending = Some(line);
            continue;
        }
        line = line.trim_end().to_string();
        if let Some(prev) = pending.take() {
            if line.trim().is_empty() {
                line = prev;
            } else {
                line = format!("{} {}", prev, line.trim_start());
            }
        }
        lines.push(line);
    }
    if let Some(rest) = pending {
        lines.push(rest.trim_end().to_string());
    }
    lines
}

fn strip_comments(lines: &[String]) -> Vec<String> {
    let mut filtered = Vec::new();
    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut line = line.clone();
        if let Some(idx) = line.find(";#") {
            line.truncate(idx);
            line = line.trim_end().to_string();
            if line.is_empty() {
                continue;
            }
        }
        filtered.push(line);
    }
    filtered
}

fn collapse_single_command_proc_bodies(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = &lines[i];
        if is_proc_header(line) {
            let mut depth = 1;
            let mut j = i + 1;
            let mut body_lines = Vec::new();
            let mut found_end = false;
            while j < lines.len() {
                let body_line = &lines[j];
                depth += brace_delta(body_line);
                if depth <= 0 && body_line.trim() == "}" {
                    found_end = true;
                    break;
                }
                if !body_line.trim().is_empty() {
                    body_lines.push(body_line.clone());
                }
                j += 1;
            }
            if found_end && body_lines.len() == 1 {
                let cmd = body_lines[0].trim();
                let base = line.trim_end();
                if let Some(trimmed) = base.strip_suffix('{') {
                    out.push(format!("{}{{{}}}", trimmed, cmd));
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(line.clone());
        i += 1;
    }
    out
}

fn is_proc_header(line: &str) -> bool {
    let trimmed = line.trim_end();
    if !trimmed.ends_with('{') {
        return false;
    }
    let mut parts = trimmed.split_whitespace();
    matches!(parts.next(), Some("proc"))
}

fn brace_delta(line: &str) -> i32 {
    let mut delta = 0;
    let mut in_quote = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            chars.next();
            continue;
        }
        if ch == '"' {
            in_quote = !in_quote;
            continue;
        }
        if in_quote {
            continue;
        }
        if ch == '{' {
            delta += 1;
        } else if ch == '}' {
            delta -= 1;
        }
    }
    delta
}

fn normalize_line(line: &str) -> Option<String> {
    let mut s = line.to_string();
    if s.contains("bad option") && s.contains("must be") {
        s = normalize_bad_option(&s);
    }
    if s.contains("cannot use namespace qualifiers") {
        s = s.replace("as hidden commandtoken", "in hidden command token");
    }
    let mut s = normalize_segment(&s);
    s = strip_space_before_bracket(&s);
    s = s.trim_end().to_string();
    s = s.trim_start().to_string();
    s = strip_trailing_semicolon(&s);
    if s.is_empty() { None } else { Some(s) }
}

fn normalize_bad_option(line: &str) -> String {
    let Some(idx) = line.find("must be") else {
        return line.to_string();
    };
    let (before, after) = line.split_at(idx + "must be".len());
    let rest = after.trim_start();
    let mut parts = Vec::new();
    for part in rest.split(',') {
        let item = part.trim();
        if item.is_empty() {
            continue;
        }
        if matches!(item, "bgerror" | "limit" | "recursionlimit") {
            continue;
        }
        parts.push(item);
    }
    if parts.is_empty() {
        return line.to_string();
    }
    format!("{} {}", before, parts.join(", "))
}

fn normalize_segment(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_whitespace() {
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '{' {
            let mut depth = 1;
            let mut j = i + 1;
            while j < bytes.len() && depth > 0 {
                let c = bytes[j] as char;
                if c == '\\' {
                    j = (j + 2).min(bytes.len());
                    continue;
                }
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if depth != 0 {
                out.push('{');
                i += 1;
                continue;
            }
            let inner = &s[i + 1..j];
            let mut inner_norm = normalize_segment(inner);
            if !inner_norm.contains('\n') {
                inner_norm = inner_norm.trim().to_string();
            }
            if inner_norm.is_empty() {
                out.push_str("__EMPTY__");
            } else if is_simple_word(&inner_norm) {
                out.push_str(&inner_norm);
            } else {
                out.push('{');
                out.push_str(&inner_norm);
                out.push('}');
            }
            i = j + 1;
            continue;
        }
        if ch == '"' {
            let mut j = i + 1;
            while j < bytes.len() {
                let c = bytes[j] as char;
                if c == '\\' {
                    j = (j + 2).min(bytes.len());
                    continue;
                }
                if c == '"' {
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                out.push('"');
                out.push_str(&s[i + 1..]);
                break;
            }
            let inner = &s[i + 1..j];
            if inner.is_empty() {
                out.push_str("__EMPTY__");
            } else if is_simple_word(inner) {
                out.push_str(inner);
            } else if is_brace_safe(inner) {
                out.push('{');
                out.push_str(inner);
                out.push('}');
            } else {
                out.push('"');
                out.push_str(inner);
                out.push('"');
            }
            i = j + 1;
            continue;
        }
        let mut j = i;
        while j < bytes.len() {
            let c = bytes[j] as char;
            if c.is_whitespace() || c == '{' || c == '"' {
                break;
            }
            j += 1;
        }
        out.push_str(&s[i..j]);
        i = j;
    }
    out
}

fn is_simple_word(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-' | '.' | '/' | '+'))
}

fn is_brace_safe(s: &str) -> bool {
    if s.contains('\\') || s.contains('$') || s.contains('[') || s.contains(']') || s.contains('\r')
    {
        return false;
    }
    let mut depth = 0i32;
    for c in s.chars() {
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        }
    }
    depth == 0
}

fn strip_space_before_bracket(s: &str) -> String {
    let mut out = String::new();
    let mut pending = String::new();
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending.push(ch);
            continue;
        }
        if ch == ']' {
            pending.clear();
            out.push(ch);
            continue;
        }
        if !pending.is_empty() {
            out.push_str(&pending);
            pending.clear();
        }
        out.push(ch);
    }
    if !pending.is_empty() {
        out.push_str(&pending);
    }
    out
}

fn strip_trailing_semicolon(s: &str) -> String {
    let mut out = s.trim_end().to_string();
    if out.ends_with(';') {
        out.pop();
        return out.trim_end().to_string();
    }
    if out.ends_with('}') {
        let mut chars: Vec<char> = out.chars().collect();
        let mut idx = chars.len();
        if idx == 0 {
            return out;
        }
        // skip trailing whitespace (should be none after trim_end)
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        if idx == 0 || chars[idx - 1] != '}' {
            return out;
        }
        let mut j = idx - 1;
        while j > 0 && chars[j - 1].is_whitespace() {
            j -= 1;
        }
        if j > 0 && chars[j - 1] == ';' {
            chars.remove(j - 1);
            out = chars.into_iter().collect::<String>();
            out = out.trim_end().to_string();
        }
    }
    out
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn raw_bytecode_payload_matches() {
    let fixtures = fixtures_dir();
    let sample = fixtures.join("break.tbc");
    assert!(sample.exists(), "missing sample fixture {:?}", sample);
    let expected = sample.with_extension("tcl");
    let compiled = tcld::bytecode::extract_tbc_payload_from_file(sample.to_str().unwrap())
        .unwrap_or_else(|e| panic!("extract payload failed for {:?}: {e}", sample));
    let temp = std::env::temp_dir().join(format!("tcld_raw_payload_{}.tbc", std::process::id()));
    fs::write(&temp, compiled).expect("write raw payload");
    let output = tcld::decompile_path(temp.to_str().unwrap())
        .unwrap_or_else(|e| panic!("decompile failed for raw payload {:?}: {e}", temp));
    let expected_src = fs::read_to_string(&expected)
        .unwrap_or_else(|e| panic!("read failed for {:?}: {e}", expected));

    let got = normalize_tcl(&output);
    let want = normalize_tcl(&expected_src);
    assert_eq!(
        want, got,
        "raw payload mismatch for {:?} (expected from {:?})",
        temp, expected
    );
}

#[test]
fn golden_fixtures_match() {
    let fixtures = fixtures_dir();
    let entries = fs::read_dir(&fixtures)
        .unwrap_or_else(|e| panic!("failed to read fixtures dir {:?}: {e}", fixtures));

    let mut tbc_files = Vec::new();
    for entry in entries {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("tbc") {
            tbc_files.push(path);
        }
    }

    tbc_files.sort();
    assert!(!tbc_files.is_empty(), "no .tbc fixtures found");

    for tbc in tbc_files {
        let tcl = tbc.with_extension("tcl");
        assert!(tcl.exists(), "missing golden .tcl for {:?}", tbc);

        let output = tcld::decompile_path(tbc.to_str().unwrap())
            .unwrap_or_else(|e| panic!("decompile failed for {:?}: {e}", tbc));
        let expected =
            fs::read_to_string(&tcl).unwrap_or_else(|e| panic!("read failed for {:?}: {e}", tcl));

        let got = normalize_tcl(&output);
        let want = normalize_tcl(&expected);

        assert_eq!(
            want, got,
            "golden mismatch for {:?} (expected from {:?})",
            tbc, tcl
        );
    }
}
