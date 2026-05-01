// ─── Graphite Workbench Design System ─────────────────────────────
// A refined, dark terminal aesthetic inspired by Ghostty/Warp/Zed.
// Every color earns its place. Aqua = focus. Violet = groups. Amber = values.
//
// Colors reference CSS custom properties so they follow the active theme.
// These work as inline style values: style={{ color: T.keyword }}

export const T = {
  // Background ramp
  bg: "var(--color-tcld-bg)",
  bg1: "var(--color-tcld-bg-1)",
  bg2: "var(--color-tcld-bg-2)",
  bg3: "var(--color-tcld-bg-3)",
  bgHighlight: "var(--color-tcld-bg-highlight)",

  // Border ramp
  border: "var(--color-tcld-border)",
  borderFocus: "var(--color-tcld-border-focus)",
  borderDim: "var(--color-tcld-border-dim)",

  // Foreground ramp
  fg: "var(--color-tcld-fg)",
  fgDim: "var(--color-tcld-fg-dim)",
  fgMuted: "var(--color-tcld-fg-muted)",
  fgBright: "var(--color-tcld-fg-bright)",

  // Semantic colors
  accent: "var(--color-tcld-accent)",
  accentDim: "var(--color-tcld-accent-dim)",
  keyword: "var(--color-tcld-keyword)",
  string: "var(--color-tcld-string)",
  number: "var(--color-tcld-number)",
  comment: "var(--color-tcld-comment)",
  variable: "var(--color-tcld-variable)",
  group: "var(--color-tcld-group)",
  groupDim: "var(--color-tcld-group-dim)",
  error: "var(--color-tcld-error)",
  warning: "var(--color-tcld-warning)",
  success: "var(--color-tcld-success)",
  ok: "var(--ok)",

  // UI-specific (derived from the main ramp)
  selection: "var(--color-tcld-bg-highlight)",
  selectionFg: "var(--color-tcld-variable)",
  tabActive: "var(--color-tcld-accent)",
  tabInactive: "var(--color-tcld-fg-dim)",
  gutter: "var(--color-tcld-border)",
  gutterActive: "var(--color-tcld-group)",
  railBg: "var(--color-tcld-bg-3)",
  statusBg: "var(--color-tcld-bg-1)",
  commandBg: "var(--color-tcld-bg-2)",
  modalBg: "var(--color-tcld-bg-1)",
  modalBorder: "var(--color-tcld-accent)",
} as const;

export const FONT_MONO = "var(--font-mono)";

// Unicode glyphs used throughout the UI
export const GLYPH = {
  groupStart: "▸",
  selectionIndicator: "◂",
  pillActive: "●",
  pillInactive: "○",
  breadcrumbSep: "›",
  boxVertical: "│",
  tabUnderline: "━",
  iconInstructions: "⊞",
  iconLiterals: "◈",
  iconExceptions: "⚡",
  iconGraph: "◎",
  iconDecompile: "◇",
  iconDump: "▤",
  iconSource: "◉",
  iconSummary: "◆",
} as const;
