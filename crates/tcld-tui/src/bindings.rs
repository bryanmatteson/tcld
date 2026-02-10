use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::TuiApp;
use crate::data::{DataSource, ViewId};
use crate::state::ModeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingContext {
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingAction {
    QuitImmediate,
    QuitConfirm,
    Help,
    Command,
    Palette,
    Search,
    CloseDiff,
    ScrollDetail(i8),
    ToggleMap,
    CycleMap,
    AdjustMapContrast(i8),
    ToggleTrace,
    RevealDecompile,
    NavigateGroup(i8),
    ToggleInspector,
    FocusNext,
    FocusPrevious,
    Navigate(i8),
    First,
    Last,
    Page(i8),
    SearchHit(i8),
    Escape,
    Mode(ModeId),
    View(ViewId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    pub fn matches(self, event: KeyEvent) -> bool {
        self.code == event.code && self.modifiers == event.modifiers
    }

    pub fn display(self) -> String {
        let key = match self.code {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "Shift-Tab".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            other => format!("{other:?}"),
        };
        if self.modifiers == KeyModifiers::CONTROL {
            format!("Ctrl-{key}")
        } else {
            key
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub context: BindingContext,
    pub chord: KeyChord,
    pub action: BindingAction,
}

pub fn normal_bindings<D: DataSource>(app: &TuiApp<D>) -> Vec<Binding> {
    let mut bindings = vec![
        binding(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            BindingAction::QuitImmediate,
        ),
        binding(
            KeyCode::Char('q'),
            KeyModifiers::empty(),
            BindingAction::QuitConfirm,
        ),
        binding(
            KeyCode::Char('?'),
            KeyModifiers::empty(),
            BindingAction::Help,
        ),
        binding(
            KeyCode::Char(':'),
            KeyModifiers::empty(),
            BindingAction::Command,
        ),
        binding(
            KeyCode::Char('/'),
            KeyModifiers::empty(),
            BindingAction::Search,
        ),
        binding(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
            BindingAction::Palette,
        ),
        binding(
            KeyCode::Char('Q'),
            KeyModifiers::SHIFT,
            BindingAction::CloseDiff,
        ),
        binding(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL,
            BindingAction::ScrollDetail(1),
        ),
        binding(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
            BindingAction::ScrollDetail(-1),
        ),
        binding(
            KeyCode::Char('m'),
            KeyModifiers::empty(),
            BindingAction::ToggleMap,
        ),
        binding(
            KeyCode::Char('M'),
            KeyModifiers::SHIFT,
            BindingAction::CycleMap,
        ),
        binding(
            KeyCode::Char('='),
            KeyModifiers::empty(),
            BindingAction::AdjustMapContrast(1),
        ),
        binding(
            KeyCode::Char('+'),
            KeyModifiers::SHIFT,
            BindingAction::AdjustMapContrast(1),
        ),
        binding(
            KeyCode::Char('-'),
            KeyModifiers::empty(),
            BindingAction::AdjustMapContrast(-1),
        ),
        binding(
            KeyCode::Char('_'),
            KeyModifiers::SHIFT,
            BindingAction::AdjustMapContrast(-1),
        ),
        binding(
            KeyCode::Char('t'),
            KeyModifiers::empty(),
            BindingAction::ToggleTrace,
        ),
        binding(
            KeyCode::Char('v'),
            KeyModifiers::empty(),
            BindingAction::RevealDecompile,
        ),
        binding(
            KeyCode::Char('['),
            KeyModifiers::empty(),
            BindingAction::NavigateGroup(-1),
        ),
        binding(
            KeyCode::Char(']'),
            KeyModifiers::empty(),
            BindingAction::NavigateGroup(1),
        ),
        binding(
            KeyCode::Enter,
            KeyModifiers::empty(),
            BindingAction::ToggleInspector,
        ),
        binding(
            KeyCode::Tab,
            KeyModifiers::empty(),
            BindingAction::FocusNext,
        ),
        binding(
            KeyCode::BackTab,
            KeyModifiers::empty(),
            BindingAction::FocusPrevious,
        ),
        binding(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
            BindingAction::FocusPrevious,
        ),
        binding(
            KeyCode::Char('j'),
            KeyModifiers::empty(),
            BindingAction::Navigate(1),
        ),
        binding(
            KeyCode::Down,
            KeyModifiers::empty(),
            BindingAction::Navigate(1),
        ),
        binding(
            KeyCode::Char('k'),
            KeyModifiers::empty(),
            BindingAction::Navigate(-1),
        ),
        binding(
            KeyCode::Up,
            KeyModifiers::empty(),
            BindingAction::Navigate(-1),
        ),
        binding(
            KeyCode::Char('g'),
            KeyModifiers::empty(),
            BindingAction::First,
        ),
        binding(KeyCode::Char('G'), KeyModifiers::SHIFT, BindingAction::Last),
        binding(
            KeyCode::PageDown,
            KeyModifiers::empty(),
            BindingAction::Page(1),
        ),
        binding(
            KeyCode::PageUp,
            KeyModifiers::empty(),
            BindingAction::Page(-1),
        ),
        binding(
            KeyCode::Char('n'),
            KeyModifiers::empty(),
            BindingAction::SearchHit(1),
        ),
        binding(
            KeyCode::Char('N'),
            KeyModifiers::SHIFT,
            BindingAction::SearchHit(-1),
        ),
        binding(KeyCode::Esc, KeyModifiers::empty(), BindingAction::Escape),
    ];

    bindings.extend(app.mode_specs().iter().filter_map(|mode| {
        let key = mode.keybind?;
        Some(binding(
            KeyCode::Char(key),
            modifiers_for_char(key),
            BindingAction::Mode(mode.id),
        ))
    }));
    bindings.extend(
        app.data
            .available_views()
            .iter()
            .filter_map(|view| crate::views::spec(app.specs(), *view))
            .filter_map(|spec| {
                let key = spec.keybind?;
                Some(binding(
                    KeyCode::Char(key),
                    modifiers_for_char(key),
                    BindingAction::View(spec.view),
                ))
            }),
    );
    bindings
}

pub fn resolve_normal<D: DataSource>(app: &TuiApp<D>, event: KeyEvent) -> Option<BindingAction> {
    normal_bindings(app)
        .into_iter()
        .find(|binding| binding.chord.matches(event))
        .map(|binding| binding.action)
}

pub fn chord_for_action<D: DataSource>(
    app: &TuiApp<D>,
    action: &BindingAction,
) -> Option<KeyChord> {
    normal_bindings(app)
        .into_iter()
        .find(|binding| &binding.action == action)
        .map(|binding| binding.chord)
}

pub fn verify_no_conflicts(bindings: &[Binding]) -> Result<(), String> {
    let mut seen: HashMap<(BindingContext, KeyChord), &BindingAction> = HashMap::new();
    for binding in bindings {
        let key = (binding.context, binding.chord);
        if let Some(previous) = seen.insert(key, &binding.action)
            && previous != &binding.action
        {
            return Err(format!(
                "conflicting binding {} in {:?}: {previous:?} vs {:?}",
                binding.chord.display(),
                binding.context,
                binding.action
            ));
        }
    }
    Ok(())
}

fn binding(code: KeyCode, modifiers: KeyModifiers, action: BindingAction) -> Binding {
    Binding {
        context: BindingContext::Normal,
        chord: KeyChord::new(code, modifiers),
        action,
    }
}

fn modifiers_for_char(key: char) -> KeyModifiers {
    if key.is_ascii_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::ListEntry;
    use crate::state::{ModeLayoutProfile, ModeSpec};
    use crate::views::{ViewOpenBehavior, ViewSpec};

    const SOURCE: ViewId = ViewId("source");
    const DECOMPILE: ViewId = ViewId("decompile");
    const BROWSE: ModeId = ModeId("browse");
    const COMPARE: ModeId = ModeId("compare");
    const VIEWS: &[ViewSpec] = &[
        ViewSpec {
            view: SOURCE,
            token: "source",
            title: "Source",
            aliases: &["source"],
            keybind: Some('c'),
            icon: "S",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
        ViewSpec {
            view: DECOMPILE,
            token: "decompile",
            title: "Decompile",
            aliases: &["decompile"],
            keybind: Some('d'),
            icon: "D",
            show_in_tab_bar: true,
            show_in_palette: true,
            is_list_view: false,
            open_behavior: ViewOpenBehavior::ReplaceMain,
        },
    ];
    const MODES: &[ModeSpec] = &[
        ModeSpec {
            id: BROWSE,
            token: "browse",
            title: "Browse",
            icon: "B",
            keybind: Some('b'),
            layout: ModeLayoutProfile::Browse,
            status_affordances: &[],
            unavailable_message: None,
        },
        ModeSpec {
            id: COMPARE,
            token: "compare",
            title: "Compare",
            icon: "C",
            keybind: Some('C'),
            layout: ModeLayoutProfile::Compare,
            status_affordances: &[],
            unavailable_message: None,
        },
    ];

    struct Data;

    impl DataSource for Data {
        fn path(&self) -> &str {
            "test.tcl"
        }
        fn available_views(&self) -> &[ViewId] {
            &[SOURCE, DECOMPILE]
        }
        fn default_view(&self) -> ViewId {
            SOURCE
        }
        fn no_colors(&self) -> bool {
            true
        }
        fn page_size(&self) -> usize {
            10
        }
        fn summary_text(&self) -> String {
            String::new()
        }
        fn view_specs(&self) -> &[ViewSpec] {
            VIEWS
        }
        fn mode_specs(&self) -> &[ModeSpec] {
            MODES
        }
        fn list_len(&self, _view: ViewId) -> usize {
            0
        }
        fn list_entry(&self, _view: ViewId, _index: usize) -> Option<ListEntry> {
            None
        }
        fn list_detail(&self, _view: ViewId, _index: usize) -> Option<String> {
            None
        }
        fn jump_index(&self, _view: ViewId, _value: i32) -> Option<usize> {
            None
        }
        fn document_text(&mut self, _view: ViewId) -> Result<String, String> {
            Ok(String::new())
        }
        fn command_group_count(&self) -> usize {
            0
        }
        fn group_for_index(&self, _index: usize) -> Option<usize> {
            None
        }
        fn first_index_for_group(&self, _group: usize) -> Option<usize> {
            None
        }
    }

    #[test]
    fn shifted_modes_and_lowercase_views_are_distinct() {
        let app = TuiApp::new(Data, false);
        let bindings = normal_bindings(&app);
        verify_no_conflicts(&bindings).expect("binding registry is conflict-free");
        assert_eq!(
            resolve_normal(
                &app,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty())
            ),
            Some(BindingAction::View(SOURCE))
        );
        assert_eq!(
            resolve_normal(&app, KeyEvent::new(KeyCode::Char('C'), KeyModifiers::SHIFT)),
            Some(BindingAction::Mode(COMPARE))
        );
        assert_eq!(
            resolve_normal(
                &app,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(BindingAction::QuitImmediate)
        );
        assert_eq!(
            resolve_normal(
                &app,
                KeyEvent::new(KeyCode::Char('m'), KeyModifiers::empty())
            ),
            Some(BindingAction::ToggleMap)
        );
        assert_eq!(
            resolve_normal(&app, KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT)),
            Some(BindingAction::CycleMap)
        );
        assert_eq!(
            resolve_normal(
                &app,
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
            ),
            None,
            "unregistered modifier variants must not alias a view or mode"
        );
    }

    #[test]
    fn conflict_verifier_rejects_duplicate_chords() {
        let chord = KeyChord::new(KeyCode::Char('x'), KeyModifiers::empty());
        let bindings = vec![
            Binding {
                context: BindingContext::Normal,
                chord,
                action: BindingAction::Help,
            },
            Binding {
                context: BindingContext::Normal,
                chord,
                action: BindingAction::Search,
            },
        ];
        assert!(verify_no_conflicts(&bindings).is_err());
    }
}
