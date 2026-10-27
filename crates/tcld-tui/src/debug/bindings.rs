use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::state::Pane;

const SOURCE: u16 = 1 << 0;
const STACK: u16 = 1 << 1;
const LOCALS: u16 = 1 << 2;
const BREAKPOINTS: u16 = 1 << 3;
const WATCHES: u16 = 1 << 4;
const EVENTS: u16 = 1 << 5;
const COVERAGE: u16 = 1 << 6;
const HELP: u16 = 1 << 7;
const PROMPT: u16 = 1 << 8;
const PROC_PICKER: u16 = 1 << 9;
const NORMAL: u16 = SOURCE | STACK | LOCALS | BREAKPOINTS | WATCHES | EVENTS | COVERAGE;
const ALL: u16 = NORMAL | HELP | PROMPT | PROC_PICKER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingContext {
    Pane(Pane),
    Help,
    Prompt,
    ProcPicker,
}

impl BindingContext {
    fn mask(self) -> u16 {
        match self {
            Self::Pane(Pane::Source) => SOURCE,
            Self::Pane(Pane::Stack) => STACK,
            Self::Pane(Pane::Locals) => LOCALS,
            Self::Pane(Pane::Breakpoints) => BREAKPOINTS,
            Self::Pane(Pane::Watches) => WATCHES,
            Self::Pane(Pane::Events) => EVENTS,
            Self::Pane(Pane::Coverage) => COVERAGE,
            Self::Help => HELP,
            Self::Prompt => PROMPT,
            Self::ProcPicker => PROC_PICKER,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugAction {
    Quit,
    ForceQuit,
    ToggleHelp,
    HelpDown,
    HelpUp,
    HelpPageDown,
    HelpPageUp,
    CancelPrompt,
    SubmitPrompt,
    PromptUp,
    PromptDown,
    PromptComplete,
    StatusInspect,
    FocusNext,
    FocusPrev,
    FocusPane(Pane),
    Continue,
    StepOver,
    StepInto,
    StepOut,
    NextOrStepOver,
    PreviousSearch,
    Pause,
    Restart,
    Refresh,
    UserSource,
    Repeat,
    CycleTheme,
    Search,
    IgnoreError,
    ToggleCoverage,
    ListSlaves,
    PreInstrument,
    PreviousCommand,
    NextCommand,
    ToggleBreakpoint,
    AddBreakpoint,
    GotoProc,
    RunToCursor,
    ProcPicker,
    AddWatch,
    AddWatchpoint,
    Eval,
    Command,
    EventFilter,
    ExportBreakpoints,
    ImportBreakpoints,
    ToggleBreakpointEnabled,
    SetIgnore,
    EditCondition,
    DeleteBreakpoint,
    DeleteWatch,
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    Home,
    End,
    ScrollLeft,
    ScrollRight,
    InspectValue,
}

#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub action: DebugAction,
    pub section: &'static str,
    pub key_label: &'static str,
    pub description: &'static str,
    contexts: u16,
}

macro_rules! binding {
    ($contexts:expr, $code:expr, $mods:expr, $action:expr, $section:expr, $keys:expr, $desc:expr) => {
        Binding {
            contexts: $contexts,
            code: $code,
            modifiers: $mods,
            action: $action,
            section: $section,
            key_label: $keys,
            description: $desc,
        }
    };
}

pub static DEBUG_BINDINGS: &[Binding] = &[
    // ── Execution ──
    binding!(
        NORMAL,
        KeyCode::F(5),
        KeyModifiers::NONE,
        DebugAction::Continue,
        "Execution",
        "F5",
        "continue"
    ),
    binding!(
        NORMAL & !BREAKPOINTS,
        KeyCode::Char('c'),
        KeyModifiers::NONE,
        DebugAction::Continue,
        "Execution",
        "c",
        "continue"
    ),
    binding!(
        NORMAL,
        KeyCode::F(10),
        KeyModifiers::NONE,
        DebugAction::StepOver,
        "Execution",
        "F10",
        "step over"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('n'),
        KeyModifiers::NONE,
        DebugAction::NextOrStepOver,
        "Execution",
        "n",
        "step over / next search match"
    ),
    binding!(
        NORMAL,
        KeyCode::F(11),
        KeyModifiers::NONE,
        DebugAction::StepInto,
        "Execution",
        "F11",
        "step into"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('s'),
        KeyModifiers::NONE,
        DebugAction::StepInto,
        "Execution",
        "s",
        "step into"
    ),
    binding!(
        NORMAL,
        KeyCode::F(11),
        KeyModifiers::SHIFT,
        DebugAction::StepOut,
        "Execution",
        "Shift-F11",
        "step out"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('S'),
        KeyModifiers::SHIFT,
        DebugAction::StepOut,
        "Execution",
        "S",
        "step out"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
        DebugAction::RunToCursor,
        "Execution",
        "Ctrl-G",
        "run to the cursor line"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('p'),
        KeyModifiers::NONE,
        DebugAction::Pause,
        "Execution",
        "p",
        "pause a running program"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('r'),
        KeyModifiers::NONE,
        DebugAction::Restart,
        "Execution",
        "r",
        "restart from the beginning"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('.'),
        KeyModifiers::NONE,
        DebugAction::Repeat,
        "Execution",
        ".",
        "repeat the last action"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('I'),
        KeyModifiers::SHIFT,
        DebugAction::IgnoreError,
        "Execution",
        "I",
        "let the current error propagate"
    ),
    // ── Breakpoints & watches ──
    binding!(
        NORMAL,
        KeyCode::Char('b'),
        KeyModifiers::NONE,
        DebugAction::ToggleBreakpoint,
        "Breakpoints & watches",
        "b",
        "toggle breakpoint at the cursor"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('B'),
        KeyModifiers::SHIFT,
        DebugAction::AddBreakpoint,
        "Breakpoints & watches",
        "B",
        "breakpoint by proc and offset"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('w'),
        KeyModifiers::NONE,
        DebugAction::AddWatch,
        "Breakpoints & watches",
        "w",
        "watch a variable or expression"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('W'),
        KeyModifiers::SHIFT,
        DebugAction::AddWatchpoint,
        "Breakpoints & watches",
        "W",
        "break when a variable changes"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
        DebugAction::ExportBreakpoints,
        "Breakpoints & watches",
        "Ctrl-S",
        "save breakpoints & watches"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('o'),
        KeyModifiers::CONTROL,
        DebugAction::ImportBreakpoints,
        "Breakpoints & watches",
        "Ctrl-O",
        "load breakpoints & watches"
    ),
    // ── Inspection ──
    binding!(
        NORMAL,
        KeyCode::Char('e'),
        KeyModifiers::NONE,
        DebugAction::Eval,
        "Inspection",
        "e",
        "evaluate Tcl in the frame"
    ),
    binding!(
        LOCALS | WATCHES,
        KeyCode::Enter,
        KeyModifiers::NONE,
        DebugAction::InspectValue,
        "Inspection",
        "Enter",
        "show the selected value in full"
    ),
    binding!(
        NORMAL,
        KeyCode::Char(':'),
        KeyModifiers::NONE,
        DebugAction::Command,
        "Inspection",
        ":",
        "debugger command (try :help)"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('!'),
        KeyModifiers::NONE,
        DebugAction::StatusInspect,
        "Inspection",
        "!",
        "show the last message in full"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('R'),
        KeyModifiers::SHIFT,
        DebugAction::Refresh,
        "Inspection",
        "R",
        "refresh stack and variables"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('C'),
        KeyModifiers::SHIFT,
        DebugAction::ToggleCoverage,
        "Inspection",
        "C",
        "toggle command coverage"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('L'),
        KeyModifiers::SHIFT,
        DebugAction::ListSlaves,
        "Inspection",
        "L",
        "list child interpreters"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('P'),
        KeyModifiers::SHIFT,
        DebugAction::PreInstrument,
        "Inspection",
        "P",
        "pre-instrument a proc body"
    ),
    // ── Source ──
    binding!(
        NORMAL,
        KeyCode::Char('k'),
        KeyModifiers::NONE,
        DebugAction::MoveUp,
        "Source",
        "k",
        "move up"
    ),
    binding!(
        NORMAL,
        KeyCode::Up,
        KeyModifiers::NONE,
        DebugAction::MoveUp,
        "Source",
        "↑",
        "move up"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('j'),
        KeyModifiers::NONE,
        DebugAction::MoveDown,
        "Source",
        "j",
        "move down"
    ),
    binding!(
        NORMAL,
        KeyCode::Down,
        KeyModifiers::NONE,
        DebugAction::MoveDown,
        "Source",
        "↓",
        "move down"
    ),
    binding!(
        NORMAL,
        KeyCode::PageUp,
        KeyModifiers::NONE,
        DebugAction::PageUp,
        "Source",
        "PgUp",
        "page up"
    ),
    binding!(
        NORMAL,
        KeyCode::PageDown,
        KeyModifiers::NONE,
        DebugAction::PageDown,
        "Source",
        "PgDn",
        "page down"
    ),
    binding!(
        NORMAL,
        KeyCode::Home,
        KeyModifiers::NONE,
        DebugAction::Home,
        "Source",
        "Home",
        "first line"
    ),
    binding!(
        NORMAL,
        KeyCode::End,
        KeyModifiers::NONE,
        DebugAction::End,
        "Source",
        "End",
        "last line"
    ),
    binding!(
        SOURCE,
        KeyCode::Left,
        KeyModifiers::NONE,
        DebugAction::ScrollLeft,
        "Source",
        "←",
        "scroll long lines left"
    ),
    binding!(
        SOURCE,
        KeyCode::Right,
        KeyModifiers::NONE,
        DebugAction::ScrollRight,
        "Source",
        "→",
        "scroll long lines right"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('['),
        KeyModifiers::NONE,
        DebugAction::PreviousCommand,
        "Source",
        "[",
        "previous command on the line"
    ),
    binding!(
        NORMAL,
        KeyCode::Char(']'),
        KeyModifiers::NONE,
        DebugAction::NextCommand,
        "Source",
        "]",
        "next command on the line"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL,
        DebugAction::Search,
        "Source",
        "Ctrl-F",
        "search the source"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('N'),
        KeyModifiers::SHIFT,
        DebugAction::PreviousSearch,
        "Source",
        "N",
        "previous search match"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('g'),
        KeyModifiers::NONE,
        DebugAction::GotoProc,
        "Source",
        "g",
        "go to a proc by name"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('G'),
        KeyModifiers::SHIFT,
        DebugAction::ProcPicker,
        "Source",
        "G",
        "pick a proc from a list"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('u'),
        KeyModifiers::NONE,
        DebugAction::UserSource,
        "Source",
        "u",
        "show the program's main file"
    ),
    // ── Panes ──
    binding!(
        NORMAL,
        KeyCode::Tab,
        KeyModifiers::NONE,
        DebugAction::FocusNext,
        "Panes",
        "Tab",
        "next pane"
    ),
    binding!(
        NORMAL,
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
        DebugAction::FocusPrev,
        "Panes",
        "Shift-Tab",
        "previous pane"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('1'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Source),
        "Panes",
        "1",
        "Source"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('2'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Stack),
        "Panes",
        "2",
        "Stack"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('3'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Locals),
        "Panes",
        "3",
        "Locals"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('4'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Breakpoints),
        "Panes",
        "4",
        "Breakpoints"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('5'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Watches),
        "Panes",
        "5",
        "Watches"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('6'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Events),
        "Panes",
        "6",
        "Console"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('7'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Coverage),
        "Panes",
        "7",
        "Coverage"
    ),
    // ── Pane-specific ──
    binding!(
        BREAKPOINTS,
        KeyCode::Char(' '),
        KeyModifiers::NONE,
        DebugAction::ToggleBreakpointEnabled,
        "Breakpoints pane",
        "Space",
        "enable or disable"
    ),
    binding!(
        BREAKPOINTS,
        KeyCode::Char('c'),
        KeyModifiers::NONE,
        DebugAction::EditCondition,
        "Breakpoints pane",
        "c",
        "edit the condition"
    ),
    binding!(
        BREAKPOINTS,
        KeyCode::Char('i'),
        KeyModifiers::NONE,
        DebugAction::SetIgnore,
        "Breakpoints pane",
        "i",
        "skip the next N hits"
    ),
    binding!(
        BREAKPOINTS,
        KeyCode::Char('d'),
        KeyModifiers::NONE,
        DebugAction::DeleteBreakpoint,
        "Breakpoints pane",
        "d",
        "delete"
    ),
    binding!(
        WATCHES,
        KeyCode::Char('d'),
        KeyModifiers::NONE,
        DebugAction::DeleteWatch,
        "Watches pane",
        "d",
        "delete"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('/'),
        KeyModifiers::NONE,
        DebugAction::EventFilter,
        "Console",
        "/",
        "filter console lines"
    ),
    // ── General ──
    binding!(
        NORMAL | HELP,
        KeyCode::Char('?'),
        KeyModifiers::NONE,
        DebugAction::ToggleHelp,
        "General",
        "?",
        "show or hide this help"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('q'),
        KeyModifiers::NONE,
        DebugAction::Quit,
        "General",
        "q",
        "quit (confirms while running)"
    ),
    binding!(
        ALL,
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
        DebugAction::ForceQuit,
        "General",
        "Ctrl-C",
        "quit immediately, from anywhere"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('t'),
        KeyModifiers::CONTROL,
        DebugAction::CycleTheme,
        "General",
        "Ctrl-T",
        "next color theme"
    ),
    // ── Overlays and prompts (not listed in help) ──
    binding!(
        HELP,
        KeyCode::Esc,
        KeyModifiers::NONE,
        DebugAction::ToggleHelp,
        "Help",
        "Esc",
        "dismiss help"
    ),
    binding!(
        HELP,
        KeyCode::Char('q'),
        KeyModifiers::NONE,
        DebugAction::ToggleHelp,
        "Help",
        "q",
        "dismiss help"
    ),
    binding!(
        HELP,
        KeyCode::Char('j'),
        KeyModifiers::NONE,
        DebugAction::HelpDown,
        "Help",
        "j",
        "scroll down"
    ),
    binding!(
        HELP,
        KeyCode::Down,
        KeyModifiers::NONE,
        DebugAction::HelpDown,
        "Help",
        "↓",
        "scroll down"
    ),
    binding!(
        HELP,
        KeyCode::Char('k'),
        KeyModifiers::NONE,
        DebugAction::HelpUp,
        "Help",
        "k",
        "scroll up"
    ),
    binding!(
        HELP,
        KeyCode::Up,
        KeyModifiers::NONE,
        DebugAction::HelpUp,
        "Help",
        "↑",
        "scroll up"
    ),
    binding!(
        HELP,
        KeyCode::PageDown,
        KeyModifiers::NONE,
        DebugAction::HelpPageDown,
        "Help",
        "PgDn",
        "page down"
    ),
    binding!(
        HELP,
        KeyCode::PageUp,
        KeyModifiers::NONE,
        DebugAction::HelpPageUp,
        "Help",
        "PgUp",
        "page up"
    ),
    binding!(
        PROMPT | PROC_PICKER,
        KeyCode::Esc,
        KeyModifiers::NONE,
        DebugAction::CancelPrompt,
        "Prompt",
        "Esc",
        "cancel prompt"
    ),
    binding!(
        PROMPT | PROC_PICKER,
        KeyCode::Enter,
        KeyModifiers::NONE,
        DebugAction::SubmitPrompt,
        "Prompt",
        "Enter",
        "submit prompt"
    ),
    binding!(
        PROMPT | PROC_PICKER,
        KeyCode::Up,
        KeyModifiers::NONE,
        DebugAction::PromptUp,
        "Prompt",
        "↑",
        "previous history or selection"
    ),
    binding!(
        PROMPT | PROC_PICKER,
        KeyCode::Down,
        KeyModifiers::NONE,
        DebugAction::PromptDown,
        "Prompt",
        "↓",
        "next history or selection"
    ),
    binding!(
        PROMPT,
        KeyCode::Tab,
        KeyModifiers::NONE,
        DebugAction::PromptComplete,
        "Prompt",
        "Tab",
        "complete command"
    ),
];

pub fn resolve_binding(context: BindingContext, key: KeyEvent) -> Option<DebugAction> {
    DEBUG_BINDINGS
        .iter()
        .find(|binding| {
            binding.contexts & context.mask() != 0
                && binding.code == key.code
                && binding.modifiers == key.modifiers
        })
        .map(|binding| binding.action)
}

/// One titled group of the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpSection {
    pub title: &'static str,
    /// `(keys, description)`; keys bound to the same action are merged
    /// (`F5 / c`).
    pub rows: Vec<(String, &'static str)>,
}

/// The help overlay's content, in reading order.
///
/// Generated from [`DEBUG_BINDINGS`] so help cannot drift from what the keys
/// actually do. Bindings for the same action within a section share a row.
pub fn help_sections() -> Vec<HelpSection> {
    let mut sections: Vec<HelpSection> = Vec::new();
    let mut actions: Vec<Vec<DebugAction>> = Vec::new();
    for binding in DEBUG_BINDINGS {
        if matches!(binding.section, "Prompt" | "Help") {
            continue;
        }
        let index = match sections
            .iter()
            .position(|section| section.title == binding.section)
        {
            Some(index) => index,
            None => {
                sections.push(HelpSection {
                    title: binding.section,
                    rows: Vec::new(),
                });
                actions.push(Vec::new());
                sections.len() - 1
            }
        };
        match actions[index]
            .iter()
            .position(|action| *action == binding.action)
        {
            Some(row) => {
                let keys = &mut sections[index].rows[row].0;
                keys.push_str(" / ");
                keys.push_str(binding.key_label);
            }
            None => {
                actions[index].push(binding.action);
                sections[index]
                    .rows
                    .push((binding.key_label.to_string(), binding.description));
            }
        }
    }
    sections
}

/// Plain-text help, one line per row, for transcripts and narrow fallbacks.
pub fn help_lines() -> Vec<String> {
    let mut output = Vec::new();
    for section in help_sections() {
        if !output.is_empty() {
            output.push(String::new());
        }
        output.push(format!("  {}", section.title));
        for (keys, description) in section.rows {
            output.push(format!("    {keys:<20} {description}"));
        }
    }
    output
}

pub fn verify_bindings() -> Result<(), String> {
    for (index, left) in DEBUG_BINDINGS.iter().enumerate() {
        for right in &DEBUG_BINDINGS[index + 1..] {
            if left.contexts & right.contexts != 0
                && left.code == right.code
                && left.modifiers == right.modifiers
                && left.action != right.action
            {
                return Err(format!(
                    "conflicting debugger binding {}: {:?} vs {:?}",
                    left.key_label, left.action, right.action
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_merges_keys_that_share_an_action() {
        let sections = help_sections();
        let execution = sections
            .iter()
            .find(|section| section.title == "Execution")
            .expect("execution section");
        assert!(
            execution
                .rows
                .iter()
                .any(|(keys, description)| keys == "F5 / c" && *description == "continue"),
            "{execution:?}"
        );
        assert!(
            execution
                .rows
                .iter()
                .any(|(keys, _)| keys == "Shift-F11 / S"),
            "{execution:?}"
        );
        assert!(
            sections
                .iter()
                .all(|section| !matches!(section.title, "Prompt" | "Help")),
            "overlay-internal bindings stay out of help"
        );
        let source = sections
            .iter()
            .find(|section| section.title == "Source")
            .expect("source section");
        assert!(source.rows.iter().any(|(keys, _)| keys == "j / ↓"));
    }

    #[test]
    fn advertised_binding_registry_has_no_context_conflicts() {
        verify_bindings().unwrap();
    }

    #[test]
    fn modifiers_and_context_reuse_are_exact() {
        assert_eq!(
            resolve_binding(
                BindingContext::Pane(Pane::Source),
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)
            ),
            Some(DebugAction::Continue)
        );
        assert_eq!(
            resolve_binding(
                BindingContext::Pane(Pane::Breakpoints),
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)
            ),
            Some(DebugAction::EditCondition)
        );
        assert_eq!(
            resolve_binding(
                BindingContext::Help,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(DebugAction::ForceQuit)
        );
        assert_eq!(
            resolve_binding(
                BindingContext::Pane(Pane::Source),
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT)
            ),
            None
        );
        assert_eq!(
            resolve_binding(
                BindingContext::Pane(Pane::Source),
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)
            ),
            Some(DebugAction::ToggleHelp)
        );
        assert_eq!(
            resolve_binding(
                BindingContext::Pane(Pane::Source),
                KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)
            ),
            Some(DebugAction::Refresh)
        );
    }
}
