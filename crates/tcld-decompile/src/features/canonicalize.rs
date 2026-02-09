use pretty::RcDoc;

pub fn canonicalize_tcl(source: &str) -> String {
    let normalized_newlines = source.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized_newlines.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut start = 0usize;
    let mut end = lines.len();
    while start < end && lines[start].trim_end().is_empty() {
        start += 1;
    }
    while end > start && lines[end - 1].trim_end().is_empty() {
        end -= 1;
    }
    if start == end {
        return String::new();
    }

    let doc = RcDoc::intersperse(
        lines[start..end]
            .iter()
            .map(|line| RcDoc::<()>::text(line.trim_end())),
        RcDoc::<()>::hardline(),
    );
    let mut out = Vec::new();
    if doc.render(100, &mut out).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_normalizes_newlines_and_trailing_whitespace() {
        let input = "\r\nset a 1  \r\nset b 2\r\n\r\n";
        assert_eq!(canonicalize_tcl(input), "set a 1\nset b 2");
    }
}
