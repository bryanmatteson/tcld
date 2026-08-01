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
}

#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub action: DebugAction,
    pub section: &'static str,
    pub key_label: &'static str,
    pub description: &'static str,
    pub compact: Option<&'static str>,
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
            compact: None,
        }
    };
    ($contexts:expr, $code:expr, $mods:expr, $action:expr, $section:expr, $keys:expr, $desc:expr, $compact:expr) => {
        Binding {
            contexts: $contexts,
            code: $code,
            modifiers: $mods,
            action: $action,
            section: $section,
            key_label: $keys,
            description: $desc,
            compact: Some($compact),
        }
    };
}

pub static DEBUG_BINDINGS: &[Binding] = &[
    binding!(
        ALL,
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
        DebugAction::Quit,
        "Global",
        "Ctrl-C",
        "quit from any context",
        "Ctrl-C quit"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('q'),
        KeyModifiers::NONE,
        DebugAction::Quit,
        "Global",
        "q",
        "quit",
        "q quit"
    ),
    binding!(
        NORMAL | HELP,
        KeyCode::Char('?'),
        KeyModifiers::NONE,
        DebugAction::ToggleHelp,
        "Global",
        "?",
        "open or dismiss help",
        "? help"
    ),
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
        "j / Down",
        "scroll down"
    ),
    binding!(
        HELP,
        KeyCode::Down,
        KeyModifiers::NONE,
        DebugAction::HelpDown,
        "Help",
        "Down",
        "scroll down"
    ),
    binding!(
        HELP,
        KeyCode::Char('k'),
        KeyModifiers::NONE,
        DebugAction::HelpUp,
        "Help",
        "k / Up",
        "scroll up"
    ),
    binding!(
        HELP,
        KeyCode::Up,
        KeyModifiers::NONE,
        DebugAction::HelpUp,
        "Help",
        "Up",
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
        "Up",
        "previous history or selection"
    ),
    binding!(
        PROMPT | PROC_PICKER,
        KeyCode::Down,
        KeyModifiers::NONE,
        DebugAction::PromptDown,
        "Prompt",
        "Down",
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
    binding!(
        NORMAL,
        KeyCode::Char('!'),
        KeyModifiers::NONE,
        DebugAction::StatusInspect,
        "Inspection",
        "!",
        "inspect full status"
    ),
    binding!(
        NORMAL,
        KeyCode::Tab,
        KeyModifiers::NONE,
        DebugAction::FocusNext,
        "Navigation",
        "Tab",
        "next pane"
    ),
    binding!(
        NORMAL,
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
        DebugAction::FocusPrev,
        "Navigation",
        "Shift-Tab",
        "previous pane"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('1'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Source),
        "Navigation",
        "1",
        "focus Source"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('2'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Stack),
        "Navigation",
        "2",
        "focus Stack"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('3'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Locals),
        "Navigation",
        "3",
        "focus Locals"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('4'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Breakpoints),
        "Navigation",
        "4",
        "focus Breakpoints"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('5'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Watches),
        "Navigation",
        "5",
        "focus Watches"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('6'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Events),
        "Navigation",
        "6",
        "focus Events"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('7'),
        KeyModifiers::NONE,
        DebugAction::FocusPane(Pane::Coverage),
        "Navigation",
        "7",
        "focus Coverage"
    ),
    binding!(
        NORMAL,
        KeyCode::F(5),
        KeyModifiers::NONE,
        DebugAction::Continue,
        "Execution",
        "F5",
        "continue",
        "F5 cont"
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
        "step over",
        "F10 over"
    ),
    binding!(
        NORMAL,
        KeyCode::F(11),
        KeyModifiers::NONE,
        DebugAction::StepInto,
        "Execution",
        "F11",
        "step into",
        "F11 into"
    ),
    binding!(
        NORMAL,
        KeyCode::F(11),
        KeyModifiers::SHIFT,
        DebugAction::StepOut,
        "Execution",
        "Shift-F11",
        "step out",
        "S-F11 out"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('n'),
        KeyModifiers::NONE,
        DebugAction::NextOrStepOver,
        "Execution",
        "n",
        "next search hit or step over"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('N'),
        KeyModifiers::SHIFT,
        DebugAction::PreviousSearch,
        "Navigation",
        "N",
        "previous search hit"
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
        KeyCode::Char('S'),
        KeyModifiers::SHIFT,
        DebugAction::StepOut,
        "Execution",
        "S",
        "step out"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('p'),
        KeyModifiers::NONE,
        DebugAction::Pause,
        "Execution",
        "p",
        "pause"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('r'),
        KeyModifiers::NONE,
        DebugAction::Restart,
        "Execution",
        "r",
        "restart"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('R'),
        KeyModifiers::SHIFT,
        DebugAction::Refresh,
        "Execution",
        "R",
        "refresh state"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('u'),
        KeyModifiers::NONE,
        DebugAction::UserSource,
        "Navigation",
        "u",
        "show target source"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('.'),
        KeyModifiers::NONE,
        DebugAction::Repeat,
        "Execution",
        ".",
        "repeat last action"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('t'),
        KeyModifiers::CONTROL,
        DebugAction::CycleTheme,
        "Global",
        "Ctrl-T",
        "cycle theme"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL,
        DebugAction::Search,
        "Navigation",
        "Ctrl-F",
        "search source"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('I'),
        KeyModifiers::SHIFT,
        DebugAction::IgnoreError,
        "Execution",
        "I",
        "ignore next error"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('C'),
        KeyModifiers::SHIFT,
        DebugAction::ToggleCoverage,
        "Coverage",
        "C",
        "toggle coverage"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('L'),
        KeyModifiers::SHIFT,
        DebugAction::ListSlaves,
        "Inspection",
        "L",
        "list interpreters"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('P'),
        KeyModifiers::SHIFT,
        DebugAction::PreInstrument,
        "Inspection",
        "P",
        "pre-instrument proc"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('['),
        KeyModifiers::NONE,
        DebugAction::PreviousCommand,
        "Navigation",
        "[",
        "previous command on line"
    ),
    binding!(
        NORMAL,
        KeyCode::Char(']'),
        KeyModifiers::NONE,
        DebugAction::NextCommand,
        "Navigation",
        "]",
        "next command on line"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('b'),
        KeyModifiers::NONE,
        DebugAction::ToggleBreakpoint,
        "Breakpoints",
        "b",
        "toggle breakpoint at cursor",
        "b bp"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('B'),
        KeyModifiers::SHIFT,
        DebugAction::AddBreakpoint,
        "Breakpoints",
        "B",
        "add breakpoint by name or address"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('g'),
        KeyModifiers::NONE,
        DebugAction::GotoProc,
        "Navigation",
        "g",
        "go to proc",
        "g goto"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
        DebugAction::RunToCursor,
        "Navigation",
        "Ctrl-G",
        "run to executable cursor location",
        "Ctrl-G run-to"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('G'),
        KeyModifiers::SHIFT,
        DebugAction::ProcPicker,
        "Navigation",
        "G",
        "open proc picker"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('w'),
        KeyModifiers::NONE,
        DebugAction::AddWatch,
        "Inspection",
        "w",
        "add watch",
        "w watch"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('W'),
        KeyModifiers::SHIFT,
        DebugAction::AddWatchpoint,
        "Breakpoints",
        "W",
        "add watchpoint"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('e'),
        KeyModifiers::NONE,
        DebugAction::Eval,
        "Inspection",
        "e",
        "evaluate expression",
        "e eval"
    ),
    binding!(
        NORMAL,
        KeyCode::Char(':'),
        KeyModifiers::NONE,
        DebugAction::Command,
        "Inspection",
        ":",
        "debugger command",
        ": cmd"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('/'),
        KeyModifiers::NONE,
        DebugAction::EventFilter,
        "Inspection",
        "/",
        "filter Events"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
        DebugAction::ExportBreakpoints,
        "Breakpoints",
        "Ctrl-S",
        "export breakpoints"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('o'),
        KeyModifiers::CONTROL,
        DebugAction::ImportBreakpoints,
        "Breakpoints",
        "Ctrl-O",
        "import breakpoints"
    ),
    binding!(
        BREAKPOINTS,
        KeyCode::Char(' '),
        KeyModifiers::NONE,
        DebugAction::ToggleBreakpointEnabled,
        "Breakpoints",
        "Space",
        "enable or disable selected"
    ),
    binding!(
        BREAKPOINTS,
        KeyCode::Char('i'),
        KeyModifiers::NONE,
        DebugAction::SetIgnore,
        "Breakpoints",
        "i",
        "set ignore count"
    ),
    binding!(
        BREAKPOINTS,
        KeyCode::Char('c'),
        KeyModifiers::NONE,
        DebugAction::EditCondition,
        "Breakpoints",
        "c",
        "edit condition"
    ),
    binding!(
        BREAKPOINTS,
        KeyCode::Char('d'),
        KeyModifiers::NONE,
        DebugAction::DeleteBreakpoint,
        "Breakpoints",
        "d",
        "delete selected"
    ),
    binding!(
        WATCHES,
        KeyCode::Char('d'),
        KeyModifiers::NONE,
        DebugAction::DeleteWatch,
        "Inspection",
        "d",
        "delete selected watch"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('k'),
        KeyModifiers::NONE,
        DebugAction::MoveUp,
        "Navigation",
        "k / Up",
        "move up"
    ),
    binding!(
        NORMAL,
        KeyCode::Up,
        KeyModifiers::NONE,
        DebugAction::MoveUp,
        "Navigation",
        "Up",
        "move up"
    ),
    binding!(
        NORMAL,
        KeyCode::Char('j'),
        KeyModifiers::NONE,
        DebugAction::MoveDown,
        "Navigation",
        "j / Down",
        "move down"
    ),
    binding!(
        NORMAL,
        KeyCode::Down,
        KeyModifiers::NONE,
        DebugAction::MoveDown,
        "Navigation",
        "Down",
        "move down"
    ),
    binding!(
        NORMAL,
        KeyCode::PageUp,
        KeyModifiers::NONE,
        DebugAction::PageUp,
        "Navigation",
        "PgUp",
        "page up"
    ),
    binding!(
        NORMAL,
        KeyCode::PageDown,
        KeyModifiers::NONE,
        DebugAction::PageDown,
        "Navigation",
        "PgDn",
        "page down"
    ),
    binding!(
        NORMAL,
        KeyCode::Home,
        KeyModifiers::NONE,
        DebugAction::Home,
        "Navigation",
        "Home",
        "first row"
    ),
    binding!(
        NORMAL,
        KeyCode::End,
        KeyModifiers::NONE,
        DebugAction::End,
        "Navigation",
        "End",
        "last row"
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

pub fn help_lines() -> Vec<String> {
    let mut output = Vec::new();
    let mut section = "";
    for binding in DEBUG_BINDINGS {
        if matches!(binding.section, "Prompt" | "Help") {
            continue;
        }
        if binding.section != section {
            if !output.is_empty() {
                output.push(String::new());
            }
            section = binding.section;
            output.push(format!("  {section}"));
        }
        output.push(format!(
            "    {:<20} {}",
            binding.key_label, binding.description
        ));
    }
    output
}

pub fn compact_hints() -> String {
    let mut seen = std::collections::BTreeSet::new();
    let hints = DEBUG_BINDINGS
        .iter()
        .filter_map(|binding| binding.compact)
        .filter(|hint| seen.insert(*hint))
        .collect::<Vec<_>>()
        .join(" · ");
    format!(" {hints} ")
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
            Some(DebugAction::Quit)
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
