//! The explorer's visual vocabulary, resolved through Termosaic.
//!
//! Every style is a semantic token resolved against a Termosaic theme, one per
//! palette. The `Theme` methods below are thin accessors over that resolution,
//! so a palette is a complete description of the interface rather than a
//! partial overlay on hard-coded colors.
//!
//! Ratatui expresses reverse video and italics, which Termosaic's core style
//! fields do not. Those travel as backend-owned style attributes and are applied
//! by [`TcldConverter`].

use ratatui::style::{Color, Style};
use termosaic::{
    AnsiColor, AttrSet, AttrValue, ColorChoice, ColorDepth, TerminalColorPolicy,
    Theme as SemanticTheme, TokenId, TokenStyle,
};
use termosaic_ratatui::StyleConverter;

use crate::tokens;

/// Obsidian — a Tokyo Night Stormcore palette designed for
/// information-dense disassembler / decompiler interfaces.
///
/// Design principles:
///   1. Cool blue-violet base with warm accents for focus
///   2. High contrast where the eye needs it (selection, groups)
///   3. Dim chrome so content dominates
///   4. Three-level hierarchy: bg < bg1 < bg2 for depth
pub struct Theme {
    pub no_colors: bool,
    palette: PaletteName,
    color_depth: ColorDepth,
    semantic: SemanticTheme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteName {
    Obsidian,
    Forest,
    Sunset,
}

impl PaletteName {
    fn name(self) -> &'static str {
        match self {
            Self::Obsidian => "obsidian",
            Self::Forest => "forest",
            Self::Sunset => "sunset",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "obsidian" | "default" => Some(Self::Obsidian),
            "forest" => Some(Self::Forest),
            "sunset" => Some(Self::Sunset),
            _ => None,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Obsidian => Self::Forest,
            Self::Forest => Self::Sunset,
            Self::Sunset => Self::Obsidian,
        }
    }
}

// ── Palette ─────────────────────────────────────────────────────

type SemanticColor = termosaic::Color;

const fn rgb(red: u8, green: u8, blue: u8, fallback: AnsiColor) -> SemanticColor {
    SemanticColor::Rgb {
        red,
        green,
        blue,
        fallback,
    }
}

/// Every color a palette must name. A role is referenced by token, never by
/// literal, so selecting a palette repaints the whole interface.
#[derive(Debug, Clone, Copy)]
struct PaletteColors {
    fg: SemanticColor,
    fg_dim: SemanticColor,
    fg_muted: SemanticColor,
    accent: SemanticColor,
    keyword: SemanticColor,
    string: SemanticColor,
    number: SemanticColor,
    comment: SemanticColor,
    variable: SemanticColor,
    group: SemanticColor,
    error: SemanticColor,
    success: SemanticColor,
    border: SemanticColor,
    border_dim: SemanticColor,
    /// Selection fill.
    bg_highlight: SemanticColor,
    /// Panel and header fill.
    bg_panel: SemanticColor,
    /// Fill for lines inside the focused command group.
    line_focus: SemanticColor,
    pill_active_bg: SemanticColor,
    crumb_artifact_bg: SemanticColor,
    crumb_group_bg: SemanticColor,
    crumb_detail_bg: SemanticColor,
    /// Text drawn on top of a saturated fill.
    on_fill: SemanticColor,
}

const OBSIDIAN: PaletteColors = PaletteColors {
    fg: rgb(192, 202, 245, AnsiColor::BrightWhite),
    fg_dim: rgb(86, 95, 137, AnsiColor::BrightBlack),
    fg_muted: rgb(115, 122, 162, AnsiColor::White),
    accent: rgb(122, 162, 247, AnsiColor::BrightBlue),
    keyword: rgb(125, 207, 255, AnsiColor::BrightCyan),
    string: rgb(158, 206, 106, AnsiColor::BrightGreen),
    number: rgb(255, 158, 100, AnsiColor::BrightYellow),
    comment: rgb(86, 95, 137, AnsiColor::BrightBlack),
    variable: rgb(224, 175, 104, AnsiColor::Yellow),
    group: rgb(187, 154, 247, AnsiColor::BrightMagenta),
    error: rgb(247, 118, 142, AnsiColor::BrightRed),
    success: rgb(158, 206, 106, AnsiColor::BrightGreen),
    border: rgb(59, 66, 97, AnsiColor::BrightBlack),
    border_dim: rgb(41, 46, 66, AnsiColor::Black),
    bg_highlight: rgb(47, 53, 73, AnsiColor::BrightBlack),
    bg_panel: rgb(36, 40, 59, AnsiColor::Black),
    line_focus: rgb(32, 32, 48, AnsiColor::Black),
    pill_active_bg: rgb(36, 54, 44, AnsiColor::Black),
    crumb_artifact_bg: rgb(34, 38, 58, AnsiColor::Black),
    crumb_group_bg: rgb(44, 39, 66, AnsiColor::Black),
    crumb_detail_bg: rgb(36, 47, 74, AnsiColor::Black),
    on_fill: rgb(0, 0, 0, AnsiColor::Black),
};

const FOREST: PaletteColors = PaletteColors {
    fg: rgb(218, 232, 216, AnsiColor::BrightWhite),
    fg_dim: rgb(119, 151, 124, AnsiColor::BrightBlack),
    fg_muted: rgb(151, 177, 146, AnsiColor::White),
    accent: rgb(135, 194, 126, AnsiColor::BrightGreen),
    keyword: rgb(112, 199, 186, AnsiColor::BrightCyan),
    string: rgb(190, 214, 116, AnsiColor::BrightGreen),
    number: rgb(230, 178, 103, AnsiColor::BrightYellow),
    comment: rgb(107, 132, 112, AnsiColor::BrightBlack),
    variable: rgb(221, 198, 117, AnsiColor::Yellow),
    group: rgb(157, 176, 232, AnsiColor::BrightBlue),
    error: rgb(231, 108, 104, AnsiColor::BrightRed),
    success: rgb(135, 194, 126, AnsiColor::BrightGreen),
    border: rgb(79, 112, 90, AnsiColor::BrightBlack),
    border_dim: rgb(43, 65, 52, AnsiColor::Black),
    bg_highlight: rgb(42, 61, 50, AnsiColor::BrightBlack),
    bg_panel: rgb(32, 48, 38, AnsiColor::Black),
    line_focus: rgb(28, 42, 33, AnsiColor::Black),
    pill_active_bg: rgb(34, 58, 42, AnsiColor::Black),
    crumb_artifact_bg: rgb(32, 46, 37, AnsiColor::Black),
    crumb_group_bg: rgb(38, 52, 44, AnsiColor::Black),
    crumb_detail_bg: rgb(35, 55, 45, AnsiColor::Black),
    on_fill: rgb(0, 0, 0, AnsiColor::Black),
};

const SUNSET: PaletteColors = PaletteColors {
    fg: rgb(248, 220, 202, AnsiColor::BrightWhite),
    fg_dim: rgb(159, 119, 122, AnsiColor::BrightBlack),
    fg_muted: rgb(190, 145, 134, AnsiColor::White),
    accent: rgb(255, 172, 112, AnsiColor::BrightYellow),
    keyword: rgb(255, 132, 123, AnsiColor::BrightRed),
    string: rgb(247, 196, 121, AnsiColor::Yellow),
    number: rgb(142, 205, 190, AnsiColor::BrightCyan),
    comment: rgb(150, 106, 113, AnsiColor::BrightBlack),
    variable: rgb(246, 213, 135, AnsiColor::Yellow),
    group: rgb(205, 159, 244, AnsiColor::BrightMagenta),
    error: rgb(255, 103, 112, AnsiColor::BrightRed),
    success: rgb(160, 207, 138, AnsiColor::BrightGreen),
    border: rgb(126, 82, 94, AnsiColor::BrightBlack),
    border_dim: rgb(78, 52, 64, AnsiColor::Black),
    bg_highlight: rgb(75, 49, 59, AnsiColor::BrightBlack),
    bg_panel: rgb(58, 38, 46, AnsiColor::Black),
    line_focus: rgb(52, 34, 41, AnsiColor::Black),
    pill_active_bg: rgb(44, 58, 44, AnsiColor::Black),
    crumb_artifact_bg: rgb(56, 37, 45, AnsiColor::Black),
    crumb_group_bg: rgb(60, 42, 54, AnsiColor::Black),
    crumb_detail_bg: rgb(62, 44, 48, AnsiColor::Black),
    on_fill: rgb(0, 0, 0, AnsiColor::Black),
};

// ── Token specification ─────────────────────────────────────────

/// Which palette role a token draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Fg,
    FgDim,
    FgMuted,
    Accent,
    Keyword,
    String,
    Number,
    Comment,
    Variable,
    Group,
    Error,
    Success,
    Border,
    BorderDim,
    BgHighlight,
    BgPanel,
    LineFocus,
    PillActiveBg,
    CrumbArtifactBg,
    CrumbGroupBg,
    CrumbDetailBg,
    OnFill,
}

impl Role {
    const fn pick(self, palette: &PaletteColors) -> SemanticColor {
        match self {
            Self::Fg => palette.fg,
            Self::FgDim => palette.fg_dim,
            Self::FgMuted => palette.fg_muted,
            Self::Accent => palette.accent,
            Self::Keyword => palette.keyword,
            Self::String => palette.string,
            Self::Number => palette.number,
            Self::Comment => palette.comment,
            Self::Variable => palette.variable,
            Self::Group => palette.group,
            Self::Error => palette.error,
            Self::Success => palette.success,
            Self::Border => palette.border,
            Self::BorderDim => palette.border_dim,
            Self::BgHighlight => palette.bg_highlight,
            Self::BgPanel => palette.bg_panel,
            Self::LineFocus => palette.line_focus,
            Self::PillActiveBg => palette.pill_active_bg,
            Self::CrumbArtifactBg => palette.crumb_artifact_bg,
            Self::CrumbGroupBg => palette.crumb_group_bg,
            Self::CrumbDetailBg => palette.crumb_detail_bg,
            Self::OnFill => palette.on_fill,
        }
    }
}

/// Attributes a token carries regardless of whether color is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Attrs {
    bold: bool,
    dim: bool,
    underline: bool,
    reversed: bool,
    italic: bool,
}

const NONE: Attrs = Attrs {
    bold: false,
    dim: false,
    underline: false,
    reversed: false,
    italic: false,
};
const BOLD: Attrs = Attrs { bold: true, ..NONE };
const REVERSED: Attrs = Attrs {
    reversed: true,
    ..NONE
};
const BOLD_REVERSED: Attrs = Attrs {
    bold: true,
    reversed: true,
    ..NONE
};
const BOLD_UNDERLINED: Attrs = Attrs {
    bold: true,
    underline: true,
    ..NONE
};
const UNDERLINED: Attrs = Attrs {
    underline: true,
    ..NONE
};
const DIM_ITALIC: Attrs = Attrs {
    dim: true,
    italic: true,
    ..NONE
};

struct TokenSpec {
    token: TokenId,
    foreground: Option<Role>,
    background: Option<Role>,
    attrs: Attrs,
}

const fn spec(
    token: TokenId,
    foreground: Option<Role>,
    background: Option<Role>,
    attrs: Attrs,
) -> TokenSpec {
    TokenSpec {
        token,
        foreground,
        background,
        attrs,
    }
}

fn token_specs() -> Vec<TokenSpec> {
    use Role::*;
    vec![
        // Base text
        spec(tokens::FG, Some(Fg), None, NONE),
        spec(tokens::FG_DIM, Some(FgDim), None, NONE),
        spec(tokens::FG_MUTED, Some(FgMuted), None, NONE),
        spec(tokens::ACCENT, Some(Accent), None, BOLD),
        spec(tokens::ACCENT_BAR, Some(Accent), None, BOLD),
        spec(tokens::ERROR, Some(Error), None, NONE),
        spec(tokens::SUCCESS, Some(Success), None, NONE),
        spec(tokens::EMPTY_STATE, Some(FgDim), None, DIM_ITALIC),
        // Panels and borders
        spec(tokens::BORDER, Some(Border), None, NONE),
        spec(tokens::BORDER_DIM, Some(BorderDim), None, NONE),
        spec(tokens::BORDER_FOCUSED, Some(Accent), None, BOLD),
        spec(tokens::BORDER_FOCUSED_ACCENT, Some(Group), None, BOLD),
        spec(tokens::PANEL_TITLE, Some(FgMuted), None, NONE),
        spec(tokens::PANEL_TITLE_FOCUSED, Some(Accent), None, BOLD),
        spec(tokens::PANEL_TITLE_LINE, Some(FgMuted), None, NONE),
        spec(tokens::PANEL_TITLE_LINE_FOCUSED, Some(Fg), None, BOLD),
        spec(tokens::PANEL_SUBTITLE, Some(Group), None, NONE),
        spec(tokens::HEADER_BG, None, Some(BgPanel), NONE),
        // Mode switcher
        spec(tokens::TAB_ACTIVE, Some(Accent), None, BOLD_UNDERLINED),
        spec(tokens::TAB_INACTIVE, Some(FgDim), None, NONE),
        // Selection
        spec(tokens::SELECTION, None, Some(BgHighlight), REVERSED),
        spec(tokens::SELECTION_INDICATOR, Some(Accent), None, NONE),
        spec(tokens::FOCUSED_LINE, None, Some(LineFocus), NONE),
        // Groups and the decompile gutter
        spec(tokens::GROUP_LABEL, Some(Group), None, BOLD),
        spec(tokens::GROUP_DIM, Some(FgDim), None, NONE),
        spec(tokens::GROUP_START_MARKER, Some(Group), None, BOLD),
        spec(tokens::GUTTER_DEFAULT, Some(FgDim), None, NONE),
        spec(tokens::GUTTER_ANNOTATED, Some(Group), None, BOLD),
        spec(
            tokens::GUTTER_FOCUSED,
            Some(OnFill),
            Some(Variable),
            BOLD_REVERSED,
        ),
        // Instruction listing
        spec(tokens::INST_PC, Some(FgDim), None, NONE),
        spec(tokens::INST_OPCODE, Some(Keyword), None, BOLD),
        spec(tokens::INST_OPCODE_SELECTED, None, None, BOLD_REVERSED),
        spec(tokens::INST_OPERAND, Some(Number), None, NONE),
        // Inspector
        spec(tokens::DETAIL_LABEL, Some(FgMuted), None, NONE),
        spec(tokens::DETAIL_VALUE, Some(Fg), None, NONE),
        spec(tokens::DETAIL_VALUE_ACCENT, Some(Keyword), None, BOLD),
        spec(tokens::DETAIL_GROUP_REF, Some(Group), None, BOLD),
        spec(tokens::DETAIL_SECTION, Some(Accent), None, BOLD),
        spec(tokens::DETAIL_SEPARATOR, Some(BorderDim), None, NONE),
        // Status strip and breadcrumbs
        spec(
            tokens::STATUS_PILL_ACTIVE,
            Some(Success),
            Some(PillActiveBg),
            BOLD,
        ),
        spec(
            tokens::STATUS_PILL_INACTIVE,
            Some(FgDim),
            Some(BgPanel),
            NONE,
        ),
        spec(
            tokens::BREADCRUMB_ARTIFACT,
            Some(FgMuted),
            Some(CrumbArtifactBg),
            NONE,
        ),
        spec(
            tokens::BREADCRUMB_GROUP,
            Some(Group),
            Some(CrumbGroupBg),
            NONE,
        ),
        spec(
            tokens::BREADCRUMB_DETAIL,
            Some(Accent),
            Some(CrumbDetailBg),
            NONE,
        ),
        // Command line and palette
        spec(tokens::COMMAND_PROMPT, Some(Accent), None, BOLD),
        spec(tokens::COMMAND_INPUT, Some(Variable), None, NONE),
        spec(
            tokens::PALETTE_SELECTED,
            Some(Fg),
            Some(BgHighlight),
            REVERSED,
        ),
        spec(tokens::PALETTE_HINT, Some(FgDim), None, NONE),
        // Unstyled text, and surfaces that decorate another style
        spec(tokens::PLAIN, None, None, NONE),
        spec(tokens::SEARCH_MATCH, None, None, UNDERLINED),
        spec(
            tokens::SELECTION_STRONG,
            None,
            Some(BgHighlight),
            BOLD_REVERSED,
        ),
        // Debugger surfaces
        spec(tokens::DEBUG_STATE_ERROR, Some(Error), None, BOLD),
        spec(tokens::DEBUG_STATE_COMPLETED, Some(Success), None, BOLD),
        spec(tokens::DEBUG_TITLE, Some(Success), Some(PillActiveBg), BOLD),
        spec(tokens::DEBUG_BANNER_ERROR, Some(Error), None, BOLD_REVERSED),
        spec(tokens::DEBUG_BREAKPOINT, Some(Error), None, BOLD),
        spec(
            tokens::DEBUG_COMMAND_SELECTED,
            Some(Accent),
            None,
            BOLD_UNDERLINED,
        ),
        spec(
            tokens::DEBUG_COMMAND_BREAKPOINT,
            Some(Error),
            None,
            BOLD_UNDERLINED,
        ),
        spec(tokens::DEBUG_CHANGED, Some(Success), None, BOLD),
        // Help overlay
        spec(
            tokens::HELP_SECTION_TITLE,
            Some(Accent),
            None,
            BOLD_UNDERLINED,
        ),
        spec(tokens::HELP_KEY, Some(Variable), None, NONE),
        spec(tokens::HELP_DESC, Some(FgMuted), None, NONE),
        // Content styling reuses Termosaic's syntax vocabulary so a lexical and
        // a Tree-sitter highlighter can produce the same tokens.
        spec(termosaic::tokens::SYNTAX_KEYWORD, Some(Keyword), None, BOLD),
        spec(
            termosaic::tokens::SYNTAX_KEYWORD_FUNCTION,
            Some(Keyword),
            None,
            BOLD,
        ),
        spec(
            termosaic::tokens::SYNTAX_KEYWORD_OPERATOR,
            Some(Keyword),
            None,
            NONE,
        ),
        spec(
            termosaic::tokens::SYNTAX_FUNCTION,
            Some(Keyword),
            None,
            BOLD,
        ),
        spec(
            termosaic::tokens::SYNTAX_FUNCTION_BUILTIN,
            Some(Keyword),
            None,
            BOLD,
        ),
        spec(
            termosaic::tokens::SYNTAX_FUNCTION_CALL,
            Some(Keyword),
            None,
            NONE,
        ),
        spec(termosaic::tokens::SYNTAX_STRING, Some(String), None, NONE),
        spec(
            termosaic::tokens::SYNTAX_STRING_ESCAPE,
            Some(Number),
            None,
            NONE,
        ),
        spec(termosaic::tokens::SYNTAX_NUMBER, Some(Number), None, NONE),
        spec(termosaic::tokens::SYNTAX_FLOAT, Some(Number), None, NONE),
        spec(termosaic::tokens::SYNTAX_BOOLEAN, Some(Number), None, NONE),
        spec(termosaic::tokens::SYNTAX_COMMENT, Some(Comment), None, NONE),
        spec(
            termosaic::tokens::SYNTAX_VARIABLE,
            Some(Variable),
            None,
            NONE,
        ),
        spec(
            termosaic::tokens::SYNTAX_VARIABLE_BUILTIN,
            Some(Variable),
            None,
            BOLD,
        ),
        spec(
            termosaic::tokens::SYNTAX_PARAMETER,
            Some(Variable),
            None,
            NONE,
        ),
        spec(
            termosaic::tokens::SYNTAX_OPERATOR,
            Some(FgMuted),
            None,
            NONE,
        ),
        spec(
            termosaic::tokens::SYNTAX_PUNCTUATION,
            Some(FgDim),
            None,
            NONE,
        ),
        spec(
            termosaic::tokens::SYNTAX_PUNCTUATION_BRACKET,
            Some(FgDim),
            None,
            NONE,
        ),
        spec(
            termosaic::tokens::SYNTAX_PUNCTUATION_DELIMITER,
            Some(FgDim),
            None,
            NONE,
        ),
        spec(
            termosaic::tokens::SYNTAX_CONDITIONAL,
            Some(Keyword),
            None,
            BOLD,
        ),
        spec(termosaic::tokens::SYNTAX_REPEAT, Some(Keyword), None, BOLD),
        spec(termosaic::tokens::SYNTAX_NAMESPACE, Some(Group), None, NONE),
        spec(termosaic::tokens::SYNTAX_TYPE, Some(Group), None, NONE),
        spec(termosaic::tokens::TEXT, Some(Fg), None, NONE),
    ]
}

fn build_semantic_theme(name: &str, palette: &PaletteColors, colors: bool) -> SemanticTheme {
    let mut builder = SemanticTheme::builder(name);
    for spec in token_specs() {
        let mut style = TokenStyle::default();
        if colors {
            if let Some(role) = spec.foreground {
                style.foreground = Some(role.pick(palette));
            }
            if let Some(role) = spec.background {
                style.background = Some(role.pick(palette));
            }
        }
        style.bold = spec.attrs.bold;
        style.dim = spec.attrs.dim;
        style.underline = spec.attrs.underline;
        if spec.attrs.reversed {
            style = style.with_attr(tokens::ATTR_REVERSED, AttrValue::Flag(true));
        }
        if spec.attrs.italic {
            style = style.with_attr(tokens::ATTR_ITALIC, AttrValue::Flag(true));
        }
        builder = builder
            .rule(spec.token, style)
            .expect("explorer token vocabulary is valid");
    }
    builder.build().expect("explorer theme builds")
}

// ── Ratatui conversion ──────────────────────────────────────────

/// Applies the two ratatui modifiers Termosaic's core style fields cannot carry.
#[derive(Clone, Copy, Debug)]
pub struct TcldConverter {
    depth: ColorDepth,
}

impl TcldConverter {
    const fn new(depth: ColorDepth) -> Self {
        Self { depth }
    }
}

impl StyleConverter for TcldConverter {
    fn color(&self, color: SemanticColor) -> Color {
        termosaic_ratatui::to_color_at_depth(color, self.depth)
    }

    fn attrs(&self, attrs: AttrSet, style: Style) -> Style {
        use ratatui::style::Modifier;
        let mut style = style;
        if attrs
            .get(tokens::ATTR_REVERSED)
            .and_then(AttrValue::as_flag)
            == Some(true)
        {
            style = style.add_modifier(Modifier::REVERSED);
        }
        if attrs.get(tokens::ATTR_ITALIC).and_then(AttrValue::as_flag) == Some(true) {
            style = style.add_modifier(Modifier::ITALIC);
        }
        style
    }
}

impl Theme {
    pub fn new(no_colors: bool) -> Self {
        Self::from_palette(no_colors, PaletteName::Obsidian, ColorDepth::TrueColor)
    }

    /// Construct a theme for an attached terminal.
    pub fn for_terminal(no_colors: bool, name: Option<&str>) -> Result<Self, String> {
        let choice = if no_colors {
            ColorChoice::Never
        } else {
            ColorChoice::Auto
        };
        let policy = TerminalColorPolicy::detect_stdout(choice);
        let palette = match name {
            Some(name) => PaletteName::from_name(name).ok_or_else(|| {
                format!(
                    "unknown theme '{name}' (available: {})",
                    Self::palette_names().join(", ")
                )
            })?,
            None => PaletteName::Obsidian,
        };
        Ok(Self::from_palette(!policy.enabled, palette, policy.depth))
    }

    fn from_palette(no_colors: bool, palette: PaletteName, color_depth: ColorDepth) -> Self {
        let semantic = build_semantic_theme(palette.name(), &palette_colors(palette), !no_colors);
        Self {
            no_colors,
            palette,
            color_depth,
            semantic,
        }
    }

    pub fn with_palette(no_colors: bool, name: &str) -> Result<Self, String> {
        let palette = PaletteName::from_name(name).ok_or_else(|| {
            format!(
                "unknown theme '{name}' (available: {})",
                Self::palette_names().join(", ")
            )
        })?;
        Ok(Self::from_palette(
            no_colors,
            palette,
            ColorDepth::TrueColor,
        ))
    }

    pub fn name(&self) -> &'static str {
        self.palette.name()
    }

    pub fn palette_names() -> Vec<&'static str> {
        vec!["obsidian", "forest", "sunset"]
    }

    pub fn set_palette(&mut self, name: &str) -> Result<(), String> {
        let palette = PaletteName::from_name(name).ok_or_else(|| {
            format!(
                "unknown theme '{name}' (available: {})",
                Self::palette_names().join(", ")
            )
        })?;
        *self = Self::from_palette(self.no_colors, palette, self.color_depth);
        Ok(())
    }

    pub fn next_palette(&mut self) {
        *self = Self::from_palette(self.no_colors, self.palette.next(), self.color_depth);
    }

    /// The Termosaic theme backing this palette, for callers that render
    /// Termosaic documents directly.
    pub fn semantic(&self) -> &SemanticTheme {
        &self.semantic
    }

    /// Resolve one semantic token into a ratatui style.
    pub fn style(&self, token: &TokenId) -> Style {
        termosaic_ratatui::to_style_with(
            self.semantic.resolve(token),
            &TcldConverter::new(self.color_depth),
        )
    }

    // ── Base styles ─────────────────────────────────────────────

    pub fn fg(&self) -> Style {
        self.style(&tokens::FG)
    }

    pub fn fg_dim(&self) -> Style {
        self.style(&tokens::FG_DIM)
    }

    pub fn fg_muted(&self) -> Style {
        self.style(&tokens::FG_MUTED)
    }

    // ── Semantic styles ─────────────────────────────────────────

    pub fn accent(&self) -> Style {
        self.style(&tokens::ACCENT)
    }

    pub fn keyword(&self) -> Style {
        self.style(&termosaic::tokens::SYNTAX_KEYWORD)
    }

    pub fn string(&self) -> Style {
        self.style(&termosaic::tokens::SYNTAX_STRING)
    }

    pub fn number(&self) -> Style {
        self.style(&termosaic::tokens::SYNTAX_NUMBER)
    }

    pub fn comment(&self) -> Style {
        self.style(&termosaic::tokens::SYNTAX_COMMENT)
    }

    pub fn variable(&self) -> Style {
        self.style(&termosaic::tokens::SYNTAX_VARIABLE)
    }

    pub fn group_label(&self) -> Style {
        self.style(&tokens::GROUP_LABEL)
    }

    pub fn group_dim(&self) -> Style {
        self.style(&tokens::GROUP_DIM)
    }

    pub fn error(&self) -> Style {
        self.style(&tokens::ERROR)
    }

    pub fn success(&self) -> Style {
        self.style(&tokens::SUCCESS)
    }

    // ── Chrome: borders, panels, tab bar ────────────────────────

    pub fn border(&self) -> Style {
        self.style(&tokens::BORDER)
    }

    pub fn border_dim(&self) -> Style {
        self.style(&tokens::BORDER_DIM)
    }

    pub fn border_focused(&self) -> Style {
        self.style(&tokens::BORDER_FOCUSED)
    }

    pub fn border_focused_accent(&self) -> Style {
        self.style(&tokens::BORDER_FOCUSED_ACCENT)
    }

    pub fn panel_title(&self) -> Style {
        self.style(&tokens::PANEL_TITLE)
    }

    pub fn panel_title_focused(&self) -> Style {
        self.style(&tokens::PANEL_TITLE_FOCUSED)
    }

    pub fn tab_active(&self) -> Style {
        self.style(&tokens::TAB_ACTIVE)
    }

    pub fn tab_inactive(&self) -> Style {
        self.style(&tokens::TAB_INACTIVE)
    }

    // ── Selection ───────────────────────────────────────────────

    pub fn selection(&self) -> Style {
        self.style(&tokens::SELECTION)
    }

    pub fn selection_indicator(&self) -> Style {
        self.style(&tokens::SELECTION_INDICATOR)
    }

    // ── Decompile gutter ────────────────────────────────────────

    pub fn gutter_focused(&self) -> Style {
        self.style(&tokens::GUTTER_FOCUSED)
    }

    pub fn gutter_annotated(&self) -> Style {
        self.style(&tokens::GUTTER_ANNOTATED)
    }

    pub fn gutter_default(&self) -> Style {
        self.style(&tokens::GUTTER_DEFAULT)
    }

    pub fn focused_line_bg(&self) -> Style {
        self.style(&tokens::FOCUSED_LINE)
    }

    // ── Status bar ──────────────────────────────────────────────

    pub fn status_pill_active(&self) -> Style {
        self.style(&tokens::STATUS_PILL_ACTIVE)
    }

    pub fn status_pill_inactive(&self) -> Style {
        self.style(&tokens::STATUS_PILL_INACTIVE)
    }

    pub fn breadcrumb(&self, level: BreadcrumbLevel) -> Style {
        match level {
            BreadcrumbLevel::Block => self.style(&tokens::BREADCRUMB_ARTIFACT),
            BreadcrumbLevel::Group => self.style(&tokens::BREADCRUMB_GROUP),
            BreadcrumbLevel::Instruction => self.style(&tokens::BREADCRUMB_DETAIL),
            BreadcrumbLevel::Separator => self.style(&tokens::FG_DIM),
        }
    }

    // ── Command line / palette ──────────────────────────────────

    pub fn command_prompt(&self) -> Style {
        self.style(&tokens::COMMAND_PROMPT)
    }

    pub fn command_input(&self) -> Style {
        self.style(&tokens::COMMAND_INPUT)
    }

    pub fn palette_selected(&self) -> Style {
        self.style(&tokens::PALETTE_SELECTED)
    }

    pub fn palette_hint(&self) -> Style {
        self.style(&tokens::PALETTE_HINT)
    }

    // ── Inspector ───────────────────────────────────────────────

    pub fn detail_label(&self) -> Style {
        self.style(&tokens::DETAIL_LABEL)
    }

    pub fn detail_value(&self) -> Style {
        self.style(&tokens::DETAIL_VALUE)
    }

    pub fn detail_value_accent(&self) -> Style {
        self.style(&tokens::DETAIL_VALUE_ACCENT)
    }

    pub fn detail_group_ref(&self) -> Style {
        self.style(&tokens::DETAIL_GROUP_REF)
    }

    pub fn detail_section(&self) -> Style {
        self.style(&tokens::DETAIL_SECTION)
    }

    pub fn detail_separator(&self) -> Style {
        self.style(&tokens::DETAIL_SEPARATOR)
    }

    pub fn empty_state(&self) -> Style {
        self.style(&tokens::EMPTY_STATE)
    }

    pub fn header_bg(&self) -> Style {
        self.style(&tokens::HEADER_BG)
    }

    pub fn panel_title_line(&self, focused: bool) -> Style {
        if focused {
            self.style(&tokens::PANEL_TITLE_LINE_FOCUSED)
        } else {
            self.style(&tokens::PANEL_TITLE_LINE)
        }
    }

    pub fn panel_subtitle(&self) -> Style {
        self.style(&tokens::PANEL_SUBTITLE)
    }

    pub fn accent_bar(&self) -> Style {
        self.style(&tokens::ACCENT_BAR)
    }

    // ── Instruction listing ─────────────────────────────────────

    pub fn inst_opcode(&self) -> Style {
        self.style(&tokens::INST_OPCODE)
    }

    pub fn inst_opcode_selected(&self) -> Style {
        self.style(&tokens::INST_OPCODE_SELECTED)
    }

    pub fn inst_pc(&self) -> Style {
        self.style(&tokens::INST_PC)
    }

    pub fn inst_operand(&self) -> Style {
        self.style(&tokens::INST_OPERAND)
    }

    pub fn group_start_marker(&self) -> Style {
        self.style(&tokens::GROUP_START_MARKER)
    }

    // ── Help overlay ────────────────────────────────────────────

    pub fn help_section_title(&self) -> Style {
        self.style(&tokens::HELP_SECTION_TITLE)
    }

    pub fn help_key(&self) -> Style {
        self.style(&tokens::HELP_KEY)
    }

    pub fn help_desc(&self) -> Style {
        self.style(&tokens::HELP_DESC)
    }

    // ── Tcl source highlighting ─────────────────────────────────

    pub fn tcl_command(&self) -> Style {
        self.keyword()
    }
    pub fn tcl_variable(&self) -> Style {
        self.variable()
    }
    pub fn tcl_string(&self) -> Style {
        self.string()
    }
    pub fn tcl_number(&self) -> Style {
        self.number()
    }
    pub fn tcl_comment(&self) -> Style {
        self.comment()
    }
    pub fn tcl_brace(&self) -> Style {
        self.style(&termosaic::tokens::SYNTAX_PUNCTUATION_BRACKET)
    }
}

fn palette_colors(palette: PaletteName) -> PaletteColors {
    match palette {
        PaletteName::Obsidian => OBSIDIAN,
        PaletteName::Forest => FOREST,
        PaletteName::Sunset => SUNSET,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreadcrumbLevel {
    Block,
    Group,
    Instruction,
    Separator,
}

// ── Unicode glyphs for refined chrome ───────────────────────────

/// Group start marker in instruction list.
pub const GROUP_MARKER: &str = "▸";

/// Selection indicator (right side of selected row).
pub const SELECTION_INDICATOR: &str = "◂";

/// Left accent bar for selected row.
pub const ACCENT_BAR: &str = "▎";

/// Status pill indicators.
pub const PILL_ACTIVE: &str = "●";
pub const PILL_INACTIVE: &str = "○";

/// Breadcrumb separator.
pub const BREADCRUMB_SEP: &str = " › ";

/// Detail section divider.
pub const DETAIL_DIVIDER: &str = "─";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_no_colors_returns_plain_styles() {
        let theme = Theme::new(true);
        let style = theme.accent();
        assert!(
            style.fg.is_none() || matches!(style.fg, Some(Color::Reset)),
            "no-colors mode should not set foreground color"
        );
        assert!(style.bg.is_none(), "no-colors mode should not fill cells");
    }

    #[test]
    fn theme_with_colors_returns_colored_styles() {
        let theme = Theme::new(false);
        let style = theme.accent();
        assert!(style.fg.is_some(), "colors mode should set foreground");
    }

    #[test]
    fn theme_palette_selection_preserves_no_color_behavior() {
        let mut theme = Theme::with_palette(false, "forest").expect("forest palette");
        assert_eq!(theme.name(), "forest");
        let forest = theme.accent().fg;
        theme.set_palette("sunset").expect("sunset palette");
        assert_eq!(theme.name(), "sunset");
        assert_ne!(theme.accent().fg, forest);
        theme.next_palette();
        assert_eq!(theme.name(), "obsidian");

        let plain = Theme::with_palette(true, "sunset").expect("no-color sunset");
        assert!(plain.accent().fg.is_none());
    }

    #[test]
    fn terminal_depth_controls_rgb_conversion() {
        let ansi16 = Theme::from_palette(false, PaletteName::Obsidian, ColorDepth::Ansi16);
        let ansi256 = Theme::from_palette(false, PaletteName::Obsidian, ColorDepth::Ansi256);
        let truecolor = Theme::from_palette(false, PaletteName::Obsidian, ColorDepth::TrueColor);

        assert_eq!(ansi16.accent().fg, Some(Color::LightBlue));
        assert!(matches!(ansi256.accent().fg, Some(Color::Indexed(_))));
        assert_eq!(truecolor.accent().fg, Some(Color::Rgb(122, 162, 247)));
    }

    /// Every style the interface can draw must move when the palette moves.
    /// Before the token vocabulary existed, 32 of these were pinned to Obsidian
    /// and a non-default palette repainted only part of the screen.
    #[test]
    fn every_colored_token_follows_the_selected_palette() {
        let obsidian = Theme::with_palette(false, "obsidian").expect("obsidian");
        let sunset = Theme::with_palette(false, "sunset").expect("sunset");

        let mut shared = Vec::new();
        for token in tokens::ALL {
            let left = obsidian.style(token);
            let right = sunset.style(token);
            if (left.fg.is_some() || left.bg.is_some()) && left == right {
                shared.push(token.as_str());
            }
        }
        assert!(
            shared.is_empty(),
            "these tokens ignore the palette: {shared:?}"
        );
    }

    #[test]
    fn reverse_video_and_italics_survive_as_ratatui_modifiers() {
        use ratatui::style::Modifier;
        let theme = Theme::new(false);
        assert!(
            theme.selection().add_modifier.contains(Modifier::REVERSED),
            "selection is reverse video"
        );
        assert!(
            theme.empty_state().add_modifier.contains(Modifier::ITALIC),
            "empty states are italic"
        );
    }

    #[test]
    fn attributes_survive_when_color_is_unavailable() {
        use ratatui::style::Modifier;
        let theme = Theme::new(true);
        assert!(theme.selection().add_modifier.contains(Modifier::REVERSED));
        assert!(theme.accent().add_modifier.contains(Modifier::BOLD));
    }

    /// The default palette must be byte-identical to the hand-written styles the
    /// token vocabulary replaced. Values are transcribed from the constants that
    /// used to live at the top of this file.
    #[test]
    fn obsidian_reproduces_the_styles_it_replaced() {
        use ratatui::style::Modifier;
        let theme = Theme::new(false);

        for (label, actual, expected_fg, expected_bg) in [
            ("fg", theme.fg(), Some((192, 202, 245)), None),
            ("fg_dim", theme.fg_dim(), Some((86, 95, 137)), None),
            ("fg_muted", theme.fg_muted(), Some((115, 122, 162)), None),
            ("accent", theme.accent(), Some((122, 162, 247)), None),
            ("keyword", theme.keyword(), Some((125, 207, 255)), None),
            ("string", theme.string(), Some((158, 206, 106)), None),
            ("number", theme.number(), Some((255, 158, 100)), None),
            ("comment", theme.comment(), Some((86, 95, 137)), None),
            ("variable", theme.variable(), Some((224, 175, 104)), None),
            (
                "group_label",
                theme.group_label(),
                Some((187, 154, 247)),
                None,
            ),
            ("error", theme.error(), Some((247, 118, 142)), None),
            ("border", theme.border(), Some((59, 66, 97)), None),
            ("border_dim", theme.border_dim(), Some((41, 46, 66)), None),
            (
                "gutter_annotated",
                theme.gutter_annotated(),
                Some((187, 154, 247)),
                None,
            ),
            (
                "gutter_default",
                theme.gutter_default(),
                Some((86, 95, 137)),
                None,
            ),
            (
                "inst_opcode",
                theme.inst_opcode(),
                Some((125, 207, 255)),
                None,
            ),
            ("inst_pc", theme.inst_pc(), Some((86, 95, 137)), None),
            (
                "inst_operand",
                theme.inst_operand(),
                Some((255, 158, 100)),
                None,
            ),
            (
                "detail_label",
                theme.detail_label(),
                Some((115, 122, 162)),
                None,
            ),
            (
                "detail_value",
                theme.detail_value(),
                Some((192, 202, 245)),
                None,
            ),
            ("help_key", theme.help_key(), Some((224, 175, 104)), None),
            (
                "command_input",
                theme.command_input(),
                Some((224, 175, 104)),
                None,
            ),
            ("header_bg", theme.header_bg(), None, Some((36, 40, 59))),
            (
                "focused_line_bg",
                theme.focused_line_bg(),
                None,
                Some((32, 32, 48)),
            ),
            (
                "status_pill_active",
                theme.status_pill_active(),
                Some((158, 206, 106)),
                Some((36, 54, 44)),
            ),
            (
                "palette_selected",
                theme.palette_selected(),
                Some((192, 202, 245)),
                Some((47, 53, 73)),
            ),
        ] {
            assert_eq!(
                actual.fg,
                expected_fg.map(|(r, g, b)| Color::Rgb(r, g, b)),
                "{label} foreground"
            );
            assert_eq!(
                actual.bg,
                expected_bg.map(|(r, g, b)| Color::Rgb(r, g, b)),
                "{label} background"
            );
        }

        assert!(theme.accent().add_modifier.contains(Modifier::BOLD));
        assert!(theme.string().add_modifier.is_empty());
        assert!(
            theme
                .tab_active()
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
    }

    /// Styles are theme data, not call-site literals. Every visual decision must
    /// name a token so a palette can speak to it; a bare `Style::default()` or an
    /// `.add_modifier()` at a render site is invisible to the theme and silently
    /// ignores the selected palette. This module is the only place allowed to
    /// build a style.
    #[test]
    fn no_style_is_constructed_outside_this_module() {
        const FORBIDDEN: [&str; 6] = [
            "Style::default()",
            "Style::new()",
            ".add_modifier(",
            ".remove_modifier(",
            "Color::Rgb(",
            "Color::Indexed(",
        ];

        fn walk(dir: &std::path::Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("readable source directory") {
                let path = entry.expect("directory entry").path();
                if path.is_dir() {
                    walk(&path, found);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs")
                    || path.file_name().is_some_and(|name| name == "theme.rs")
                {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("readable source file");
                for (number, line) in source.lines().enumerate() {
                    let code = line.trim_start();
                    if code.starts_with("//") || code.starts_with("*") {
                        continue;
                    }
                    for pattern in FORBIDDEN {
                        if code.contains(pattern) {
                            found.push(format!("{}:{}: {pattern}", path.display(), number + 1));
                        }
                    }
                }
            }
        }

        let mut found = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut found,
        );
        assert!(
            found.is_empty(),
            "styles must come from a token, not a literal:\n  {}",
            found.join("\n  ")
        );
    }

    #[test]
    fn semantic_theme_boundary_stays_parser_provider_neutral() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in ["termosaic-tree-sitter", "tree-sitter ="] {
            assert!(
                !manifest.contains(forbidden),
                "tcld-tui must consume semantic tokens without importing parser provider {forbidden:?}"
            );
        }
    }

    /// A token with no rule resolves to an unstyled run, which looks like a
    /// deliberate choice and is invisible to the palette test above. Every token
    /// the interface can name must actually be defined.
    #[test]
    fn every_declared_token_has_a_rule() {
        let specs = token_specs();
        let specified: std::collections::HashSet<&str> =
            specs.iter().map(|spec| spec.token.as_str()).collect();
        let undefined: Vec<&str> = tokens::ALL
            .iter()
            .map(TokenId::as_str)
            .filter(|name| !specified.contains(name))
            .collect();
        assert!(
            undefined.is_empty(),
            "declared but never styled: {undefined:?}"
        );
    }

    /// Losing color must not lose meaning. Emphasis is carried by attributes the
    /// theme still resolves when color is unavailable, so highlighters no longer
    /// short-circuit to unstyled text and monochrome terminals keep the same
    /// structural cues as color ones.
    #[test]
    fn emphasis_survives_when_color_does_not() {
        use ratatui::style::Modifier;
        let plain = Theme::new(true);

        for (label, style, modifier) in [
            ("accent", plain.accent(), Modifier::BOLD),
            ("keyword", plain.keyword(), Modifier::BOLD),
            ("inst_opcode", plain.inst_opcode(), Modifier::BOLD),
            ("gutter_annotated", plain.gutter_annotated(), Modifier::BOLD),
            ("detail_section", plain.detail_section(), Modifier::BOLD),
            ("selection", plain.selection(), Modifier::REVERSED),
            ("tab_active", plain.tab_active(), Modifier::UNDERLINED),
            ("empty_state", plain.empty_state(), Modifier::ITALIC),
        ] {
            assert!(
                style.add_modifier.contains(modifier),
                "{label} must keep {modifier:?} without color"
            );
            assert_eq!(style.fg, None, "{label} must not set a color");
            assert_eq!(style.bg, None, "{label} must not fill cells");
        }

        // Styles whose only signal was color stay genuinely plain.
        assert_eq!(plain.string().add_modifier, Modifier::empty());
        assert_eq!(plain.string().fg, None);
    }
}
