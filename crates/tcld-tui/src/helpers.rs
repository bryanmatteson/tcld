use crate::data::ViewId;
use crate::views::{self, ViewSpec};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn fuzzy_score(label: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let label_lower = label.to_ascii_lowercase();
    if label_lower.contains(query) {
        return Some(label.len().saturating_sub(query.len()));
    }
    let mut q = query.chars();
    let mut next = q.next()?;
    let mut gaps = 0usize;
    for ch in label_lower.chars() {
        if ch == next {
            if let Some(n) = q.next() {
                next = n;
            } else {
                return Some(gaps);
            }
        } else {
            gaps += 1;
        }
    }
    None
}

pub fn preview_text(text: &str, max_lines: usize) -> String {
    if max_lines == 0 {
        return String::new();
    }
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    let mut out = lines[..max_lines].join("\n");
    out.push_str("\n... (press Enter to expand)");
    out
}

pub fn usize_to_u16(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

pub fn centered_rect(percent_x: u16, percent_y: u16, outer: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(outer);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

pub fn point_in_rect(x: u16, y: u16, rect: Rect) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

pub fn session_file_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".tcld")
            .join("explore-session.txt"),
    )
}

pub fn normalize_session_path(path: &str) -> String {
    Path::new(path)
        .canonicalize()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| path.to_string())
}

pub fn encode_view_map(map: &HashMap<ViewId, usize>) -> String {
    let mut pairs = map
        .iter()
        .map(|(view, idx)| format!("{}:{idx}", view.0))
        .collect::<Vec<_>>();
    pairs.sort();
    pairs.join(",")
}

pub fn decode_view_map(raw: &str, specs: &[ViewSpec]) -> HashMap<ViewId, usize> {
    let mut out = HashMap::new();
    for pair in raw.split(',') {
        let Some((view_raw, idx_raw)) = pair.split_once(':') else {
            continue;
        };
        let Some(view) = views::parse_token(specs, view_raw) else {
            continue;
        };
        let Ok(idx) = idx_raw.parse::<usize>() else {
            continue;
        };
        out.insert(view, idx);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_exact_match() {
        assert_eq!(fuzzy_score("decompile", "decompile"), Some(0));
    }

    #[test]
    fn fuzzy_prefix_match() {
        assert_eq!(fuzzy_score("decompile", "dec"), Some(6));
    }

    #[test]
    fn fuzzy_no_match() {
        assert_eq!(fuzzy_score("decompile", "xyz"), None);
    }

    #[test]
    fn centered_rect_reasonable() {
        let outer = Rect::new(0, 0, 100, 50);
        let inner = centered_rect(50, 50, outer);
        assert!(inner.width > 0 && inner.height > 0);
        assert!(inner.x > 0 && inner.y > 0);
    }

    #[test]
    fn point_in_rect_works() {
        let r = Rect::new(10, 10, 20, 20);
        assert!(point_in_rect(15, 15, r));
        assert!(!point_in_rect(5, 5, r));
    }

    #[test]
    fn encode_decode_roundtrip() {
        use crate::views::{ViewOpenBehavior, ViewSpec};

        let specs = &[
            ViewSpec {
                view: ViewId("instructions"),
                token: "instructions",
                title: "Instructions",
                aliases: &[],
                keybind: None,
                icon: "·",
                show_in_tab_bar: true,
                show_in_palette: true,
                is_list_view: true,
                open_behavior: ViewOpenBehavior::ReplaceMain,
            },
            ViewSpec {
                view: ViewId("decompile"),
                token: "decompile",
                title: "Decompile",
                aliases: &[],
                keybind: None,
                icon: "·",
                show_in_tab_bar: true,
                show_in_palette: true,
                is_list_view: false,
                open_behavior: ViewOpenBehavior::ReplaceMain,
            },
        ];
        let mut map = HashMap::new();
        map.insert(ViewId("instructions"), 42);
        map.insert(ViewId("decompile"), 7);
        let encoded = encode_view_map(&map);
        let decoded = decode_view_map(&encoded, specs);
        assert_eq!(decoded.get(&ViewId("instructions")), Some(&42));
        assert_eq!(decoded.get(&ViewId("decompile")), Some(&7));
    }
}
