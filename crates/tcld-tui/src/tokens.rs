//! Semantic tokens and style attributes for the explorer's own surfaces.
//!
//! Content styling — Tcl source, decompiled output, instruction operands —
//! reuses Termosaic's `syntax.*` vocabulary, so a lexical highlighter and a
//! Tree-sitter one can produce the same tokens interchangeably. Everything here
//! is chrome that Termosaic has no opinion about: panels, gutters, the
//! inspector, the palette.
//!
//! Ratatui expresses two attributes Termosaic's core style fields do not, so
//! they travel as backend-owned attributes and are applied by
//! [`crate::theme::TcldConverter`].

use termosaic::{StyleAttr, TokenId};

/// Reverse video. Ratatui `Modifier::REVERSED`.
pub const ATTR_REVERSED: StyleAttr = StyleAttr::from_static("ratatui.reversed");
/// Italic text. Ratatui `Modifier::ITALIC`.
pub const ATTR_ITALIC: StyleAttr = StyleAttr::from_static("ratatui.italic");

macro_rules! ui_tokens {
    ($($konst:ident => $name:literal),+ $(,)?) => {
        $(pub const $konst: TokenId = TokenId::from_static($name);)+

        /// Every token the explorer resolves, so a theme can be checked for
        /// completeness rather than silently falling back to unstyled text.
        pub const ALL: &[TokenId] = &[$($konst),+];
    };
}

ui_tokens! {
    // Base text
    FG => "ui.text",
    FG_DIM => "ui.text.dim",
    FG_MUTED => "ui.text.muted",
    ACCENT => "ui.accent",
    ACCENT_BAR => "ui.accent.bar",
    ERROR => "ui.error",
    SUCCESS => "ui.success",
    EMPTY_STATE => "ui.empty",

    // Panels and borders
    BORDER => "ui.border",
    BORDER_DIM => "ui.border.dim",
    BORDER_FOCUSED => "ui.border.focused",
    BORDER_FOCUSED_ACCENT => "ui.border.focused.accent",
    PANEL_TITLE => "ui.panel.title",
    PANEL_TITLE_FOCUSED => "ui.panel.title.focused",
    PANEL_TITLE_LINE => "ui.panel.title.line",
    PANEL_TITLE_LINE_FOCUSED => "ui.panel.title.line.focused",
    PANEL_SUBTITLE => "ui.panel.subtitle",
    HEADER_BG => "ui.header",

    // Mode switcher
    TAB_ACTIVE => "ui.tab.active",
    TAB_INACTIVE => "ui.tab.inactive",

    // Selection
    SELECTION => "ui.selection",
    SELECTION_INDICATOR => "ui.selection.indicator",
    FOCUSED_LINE => "ui.selection.line",

    // Decompile gutter and command groups
    GROUP_LABEL => "ui.group.label",
    GROUP_DIM => "ui.group.dim",
    GROUP_START_MARKER => "ui.group.marker",
    GUTTER_DEFAULT => "ui.gutter",
    GUTTER_ANNOTATED => "ui.gutter.annotated",
    GUTTER_FOCUSED => "ui.gutter.focused",

    // Instruction listing
    INST_PC => "ui.instruction.pc",
    INST_OPCODE => "ui.instruction.opcode",
    INST_OPCODE_SELECTED => "ui.instruction.opcode.selected",
    INST_OPERAND => "ui.instruction.operand",

    // Inspector
    DETAIL_LABEL => "ui.detail.label",
    DETAIL_VALUE => "ui.detail.value",
    DETAIL_VALUE_ACCENT => "ui.detail.value.accent",
    DETAIL_GROUP_REF => "ui.detail.group",
    DETAIL_SECTION => "ui.detail.section",
    DETAIL_SEPARATOR => "ui.detail.separator",

    // Status strip and breadcrumbs
    STATUS_PILL_ACTIVE => "ui.pill.active",
    STATUS_PILL_INACTIVE => "ui.pill.inactive",
    BREADCRUMB_ARTIFACT => "ui.breadcrumb.artifact",
    BREADCRUMB_GROUP => "ui.breadcrumb.group",
    BREADCRUMB_DETAIL => "ui.breadcrumb.detail",

    // Command line and palette
    COMMAND_PROMPT => "ui.command.prompt",
    COMMAND_INPUT => "ui.command.input",
    PALETTE_SELECTED => "ui.palette.selected",
    PALETTE_HINT => "ui.palette.hint",

    // Unstyled text. Named so that "no styling" is a deliberate choice a theme
    // can still speak to, rather than a bare Style::default() at a call site.
    PLAIN => "ui.plain",
    SEARCH_MATCH => "ui.search.match",
    SELECTION_STRONG => "ui.selection.strong",

    // Debugger surfaces
    DEBUG_STATE_ERROR => "ui.debug.state.error",
    DEBUG_STATE_COMPLETED => "ui.debug.state.completed",
    DEBUG_TITLE => "ui.debug.title",
    DEBUG_BANNER_ERROR => "ui.debug.banner.error",
    DEBUG_BREAKPOINT => "ui.debug.breakpoint",
    DEBUG_COMMAND_SELECTED => "ui.debug.command.selected",
    DEBUG_COMMAND_BREAKPOINT => "ui.debug.command.breakpoint",
    DEBUG_CHANGED => "ui.debug.changed",

    // Help overlay
    HELP_SECTION_TITLE => "ui.help.section",
    HELP_KEY => "ui.help.key",
    HELP_DESC => "ui.help.desc",
}

/// Map a Tree-sitter capture name onto the semantic token that styles it.
///
/// This is the single place capture names are interpreted, so the terminal UI
/// and the batch CLI cannot drift into styling the same Tcl differently. Names
/// come from `tcld-syntax`'s `queries/tcl/highlights.scm`; the most specific
/// name wins, falling back to the dotted parent and finally to plain text, so a
/// query update that introduces `string.regexp` still renders as a string
/// instead of silently losing its styling.
pub fn token_for_capture(capture: &str) -> TokenId {
    use termosaic::tokens as semantic;

    match capture {
        "boolean" => semantic::SYNTAX_BOOLEAN,
        "comment" => semantic::SYNTAX_COMMENT,
        "conditional" => semantic::SYNTAX_CONDITIONAL,
        "function" => semantic::SYNTAX_FUNCTION,
        "function.builtin" => semantic::SYNTAX_FUNCTION_BUILTIN,
        "function.call" => semantic::SYNTAX_FUNCTION_CALL,
        "keyword" => semantic::SYNTAX_KEYWORD,
        "keyword.function" => semantic::SYNTAX_KEYWORD_FUNCTION,
        "keyword.operator" => semantic::SYNTAX_KEYWORD_OPERATOR,
        "namespace" => semantic::SYNTAX_NAMESPACE,
        "number" => semantic::SYNTAX_NUMBER,
        "float" => semantic::SYNTAX_FLOAT,
        "operator" => semantic::SYNTAX_OPERATOR,
        "punctuation.bracket" => semantic::SYNTAX_PUNCTUATION_BRACKET,
        "punctuation.delimiter" => semantic::SYNTAX_PUNCTUATION_DELIMITER,
        "repeat" => semantic::SYNTAX_REPEAT,
        "string" => semantic::SYNTAX_STRING,
        "string.escape" => semantic::SYNTAX_STRING_ESCAPE,
        "type" => semantic::SYNTAX_TYPE,
        "variable" => semantic::SYNTAX_VARIABLE,
        "variable.builtin" => semantic::SYNTAX_VARIABLE_BUILTIN,
        "variable.parameter" => semantic::SYNTAX_PARAMETER,
        // `spell` marks prose for a spell checker and carries no styling.
        "spell" => semantic::TEXT,
        other => match other.rsplit_once('.') {
            Some((parent, _)) => token_for_capture(parent),
            None => semantic::TEXT,
        },
    }
}
