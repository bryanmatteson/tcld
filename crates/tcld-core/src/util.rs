use std::fmt::Write;

pub fn right_justified(s: &str, width: usize, truncate: bool) -> String {
    let len = s.chars().count();
    if width <= len {
        return if truncate {
            s.chars().skip(len - width).collect()
        } else {
            s.to_string()
        };
    }
    let mut out = String::with_capacity(width);
    out.extend(std::iter::repeat_n(' ', width - len));
    out.push_str(s);
    out
}

pub fn left_justified(s: &str, width: usize, truncate: bool) -> String {
    let len = s.chars().count();
    if width <= len {
        return if truncate {
            s.chars().take(width).collect()
        } else {
            s.to_string()
        };
    }
    let mut out = String::with_capacity(width);
    out.push_str(s);
    out.extend(std::iter::repeat_n(' ', width - len));
    out
}

pub fn truncate(s: &str, length: usize, trailing: &str) -> String {
    let len = s.chars().count();
    if len <= length {
        return s.to_string();
    }
    let mut out: String = s.chars().take(length).collect();
    out.push_str(trailing);
    out
}

pub fn hex_description(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for b in bytes {
        let _ = write!(out, "{:02x} ", b);
    }
    out
}

pub fn indent_lines(s: &str, indent: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let line_count = s.lines().count();
    let mut out = String::with_capacity(s.len() + line_count * indent.len());
    for (idx, line) in s.lines().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        if !line.is_empty() {
            out.push_str(indent);
        }
        out.push_str(line);
    }
    out
}

pub fn push_indented_lines(out: &mut String, s: &str, indent: &str) {
    if s.is_empty() {
        return;
    }
    for (idx, line) in s.lines().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        if !line.is_empty() {
            out.push_str(indent);
        }
        out.push_str(line);
    }
}
