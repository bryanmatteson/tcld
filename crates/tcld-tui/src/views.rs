use crate::data::ViewId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewOpenBehavior {
    /// Replace the main content area.
    ReplaceMain,
    /// Show as a centered modal overlay.
    SummaryModal,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewSpec {
    pub view: ViewId,
    pub token: &'static str,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
    pub keybind: Option<char>,
    pub icon: &'static str,
    pub show_in_tab_bar: bool,
    pub show_in_palette: bool,
    pub is_list_view: bool,
    pub open_behavior: ViewOpenBehavior,
}

// ── Registry queries (all take specs from the DataSource) ────

pub fn spec(specs: &[ViewSpec], view: ViewId) -> Option<&ViewSpec> {
    specs.iter().find(|s| s.view == view)
}

pub fn title(specs: &[ViewSpec], view: ViewId) -> &str {
    spec(specs, view).map(|s| s.title).unwrap_or("Unknown")
}

pub fn is_list_view(specs: &[ViewSpec], view: ViewId) -> bool {
    spec(specs, view).is_some_and(|s| s.is_list_view)
}

pub fn token(specs: &[ViewSpec], view: ViewId) -> &str {
    spec(specs, view).map(|s| s.token).unwrap_or("")
}

pub fn parse_token(specs: &[ViewSpec], value: &str) -> Option<ViewId> {
    specs.iter().find(|s| s.token == value).map(|s| s.view)
}

pub fn parse_alias(specs: &[ViewSpec], value: &str) -> Option<ViewId> {
    let raw = value.trim().to_ascii_lowercase();
    specs
        .iter()
        .find(|s| s.aliases.iter().any(|alias| *alias == raw))
        .map(|s| s.view)
}

pub fn spec_for_keybind<'a>(
    specs: &'a [ViewSpec],
    key: char,
    available: &[ViewId],
) -> Option<&'a ViewSpec> {
    specs
        .iter()
        .find(|s| s.keybind == Some(key) && available.contains(&s.view))
}

pub fn tab_specs<'a>(specs: &'a [ViewSpec], available: &[ViewId]) -> Vec<&'a ViewSpec> {
    available
        .iter()
        .filter_map(|view| {
            let s = spec(specs, *view)?;
            s.show_in_tab_bar.then_some(s)
        })
        .collect()
}

pub fn palette_specs<'a>(specs: &'a [ViewSpec], available: &[ViewId]) -> Vec<&'a ViewSpec> {
    available
        .iter()
        .filter_map(|view| {
            let s = spec(specs, *view)?;
            s.show_in_palette.then_some(s)
        })
        .collect()
}

pub fn help_specs<'a>(specs: &'a [ViewSpec], available: &[ViewId]) -> Vec<&'a ViewSpec> {
    available
        .iter()
        .filter_map(|view| {
            let s = spec(specs, *view)?;
            s.keybind.is_some().then_some(s)
        })
        .collect()
}
