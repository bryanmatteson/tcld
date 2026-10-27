export const DESIGN_TOKEN_KEYS = [
  "bg-canvas",
  "bg-window",
  "bg-panel",
  "bg-panel-2",
  "bg-elev",
  "bg-input",
  "line",
  "line-2",
  "fg",
  "fg-2",
  "fg-3",
  "fg-4",
  // Named text-emphasis tokens (split out from fg-3/fg-4 so each has a single
  // semantic owner and can be tuned for WCAG AA contrast independently).
  "text-inactive",     // inactive labels: tab labels, panel titles when not focused
  "text-keyhint",      // keybinding hints / kbd-style chips
  "text-placeholder",  // input placeholder text (must clear SC 1.4.11 ≥ 3:1)
  "text-comment",      // syntax highlighting comments / metadata
  "accent",
  "accent-2",
  "accent-bg",
  "accent-bg-strong",
  "accent-fg-on",
  "warn",
  "err",
  "info",
  "violet",
  "pink",
  "kw",
  "str",
  "var",
  "num",
] as const;

export type DesignTokenKey = (typeof DESIGN_TOKEN_KEYS)[number];
export type DesignThemeId = "graphite" | "catppuccin" | "dracula" | "solarized" | "tokyolight";
export type LegacyThemeId = "tokyo-night" | "tokyo-night-light" | "catppuccin-mocha" | "solarized-dark";
export type ThemeId = DesignThemeId | LegacyThemeId;

export interface DesignThemeDef {
  name: string;
  accentName: string;
  tokens: Record<DesignTokenKey, string>;
}

export interface ThemeDef {
  label: string;
  colors: Record<string, string>;
  tokens: Record<DesignTokenKey, string>;
}

export const DESIGN_THEME_TOKENS: Record<DesignThemeId, DesignThemeDef> = {
  graphite: {
    name: "Graphite Workbench",
    accentName: "mint",
    tokens: {
      "bg-canvas": "#0b0d0c",
      "bg-window": "#15191a",
      "bg-panel": "#1a1f20",
      "bg-panel-2": "#20262a",
      "bg-elev": "#262d2f",
      "bg-input": "#11161a",
      // Lifted ~12% so panels read as separated cards even at low brightness.
      line: "#3a4347",
      "line-2": "#2c3437",
      fg: "#e6e8e6",
      "fg-2": "#a8b0ad",
      "fg-3": "#bcc3bf",   // alias of text-inactive — kept for legacy consumers
      "fg-4": "#7e8884",   // alias of text-comment / text-placeholder
      "text-inactive": "#bcc3bf",
      "text-keyhint": "#a4ada9",
      "text-placeholder": "#7e8884",
      "text-comment": "#7e8884",
      accent: "#7adfb6",
      "accent-2": "#5cc8a0",
      "accent-bg": "rgba(122,223,182,0.12)",
      "accent-bg-strong": "rgba(122,223,182,0.18)",
      "accent-fg-on": "#0c1411",
      warn: "#f3c469",
      err: "#f47174",
      info: "#88b8e6",
      violet: "#b59cf0",
      pink: "#e89bb8",
      kw: "#b59cf0",
      str: "#9fd285",
      var: "#88b8e6",
      num: "#f3c469",
    },
  },
  catppuccin: {
    name: "Catppuccin Mocha",
    accentName: "mauve",
    tokens: {
      "bg-canvas": "#11111b",
      "bg-window": "#1e1e2e",
      "bg-panel": "#181825",
      "bg-panel-2": "#1e1e2e",
      "bg-elev": "#313244",
      "bg-input": "#11111b",
      line: "#45475a",
      "line-2": "#3a3a4d",
      fg: "#cdd6f4",
      "fg-2": "#bac2de",
      "fg-3": "#cdd6f4",
      "fg-4": "#9399b2",
      "text-inactive": "#cdd6f4",
      "text-keyhint": "#a6adc8",
      "text-placeholder": "#9399b2",
      "text-comment": "#9399b2",
      accent: "#cba6f7",
      "accent-2": "#b4befe",
      "accent-bg": "rgba(203,166,247,0.14)",
      "accent-bg-strong": "rgba(203,166,247,0.22)",
      "accent-fg-on": "#11111b",
      warn: "#f9e2af",
      err: "#f38ba8",
      info: "#89b4fa",
      violet: "#cba6f7",
      pink: "#f5c2e7",
      kw: "#cba6f7",
      str: "#a6e3a1",
      var: "#89b4fa",
      num: "#fab387",
    },
  },
  dracula: {
    name: "Dracula",
    accentName: "pink",
    tokens: {
      "bg-canvas": "#191a21",
      "bg-window": "#282a36",
      "bg-panel": "#21222c",
      "bg-panel-2": "#282a36",
      "bg-elev": "#343746",
      "bg-input": "#191a21",
      line: "#4a4d63",
      "line-2": "#3d4053",
      fg: "#f8f8f2",
      "fg-2": "#c0c2d0",
      "fg-3": "#dfe1ee",
      "fg-4": "#888bab",
      "text-inactive": "#dfe1ee",
      "text-keyhint": "#9ea1bf",
      "text-placeholder": "#888bab",
      "text-comment": "#888bab",
      accent: "#ff79c6",
      "accent-2": "#bd93f9",
      "accent-bg": "rgba(255,121,198,0.12)",
      "accent-bg-strong": "rgba(255,121,198,0.20)",
      "accent-fg-on": "#191a21",
      warn: "#f1fa8c",
      err: "#ff5555",
      info: "#8be9fd",
      violet: "#bd93f9",
      pink: "#ff79c6",
      kw: "#ff79c6",
      str: "#f1fa8c",
      var: "#8be9fd",
      num: "#bd93f9",
    },
  },
  solarized: {
    name: "Solarized Dark",
    accentName: "cyan",
    tokens: {
      "bg-canvas": "#001f27",
      "bg-window": "#002b36",
      "bg-panel": "#00252e",
      "bg-panel-2": "#073642",
      "bg-elev": "#0a4252",
      "bg-input": "#001f27",
      line: "#125266",
      "line-2": "#0d4554",
      fg: "#eee8d5",
      "fg-2": "#93a1a1",
      "fg-3": "#cfd6c4",
      "fg-4": "#7d8e8b",
      "text-inactive": "#cfd6c4",
      "text-keyhint": "#93a1a1",
      "text-placeholder": "#7d8e8b",
      "text-comment": "#7d8e8b",
      accent: "#2aa198",
      "accent-2": "#268bd2",
      "accent-bg": "rgba(42,161,152,0.14)",
      "accent-bg-strong": "rgba(42,161,152,0.24)",
      "accent-fg-on": "#002b36",
      warn: "#b58900",
      err: "#dc322f",
      info: "#268bd2",
      violet: "#6c71c4",
      pink: "#d33682",
      kw: "#859900",
      str: "#2aa198",
      var: "#268bd2",
      num: "#cb4b16",
    },
  },
  tokyolight: {
    name: "Tokyo Night Light",
    accentName: "indigo",
    tokens: {
      "bg-canvas": "#e1e2e7",
      "bg-window": "#eaedf3",
      "bg-panel": "#f1f3f8",
      "bg-panel-2": "#e8ebf1",
      "bg-elev": "#dfe2ea",
      "bg-input": "#f6f8fc",
      line: "#b5bac6",
      "line-2": "#bdc1cc",
      fg: "#1f2335",
      "fg-2": "#3a4060",
      "fg-3": "#3a4060",
      "fg-4": "#5d6488",
      "text-inactive": "#3a4060",
      "text-keyhint": "#4f5680",
      "text-placeholder": "#5d6488",
      "text-comment": "#5d6488",
      accent: "#34548a",
      "accent-2": "#5a6fb0",
      "accent-bg": "rgba(52,84,138,0.10)",
      "accent-bg-strong": "rgba(52,84,138,0.18)",
      "accent-fg-on": "#ffffff",
      warn: "#7a4f12",
      err: "#8c4351",
      info: "#0f5b9b",
      violet: "#5a3e8e",
      pink: "#a4628b",
      kw: "#5a3e8e",
      str: "#385f0d",
      var: "#0f5b9b",
      num: "#965027",
    },
  },
};

export const THEME_ALIASES: Record<LegacyThemeId, DesignThemeId> = {
  "tokyo-night": "graphite",
  "tokyo-night-light": "tokyolight",
  "catppuccin-mocha": "catppuccin",
  "solarized-dark": "solarized",
};

export type TypeSystemId = "mono-brutalist" | "modern-sans" | "editorial";

export const DESIGN_TYPE_SYSTEMS: Record<TypeSystemId, {
  ui: string;
  mono: string;
  scale: number;
  tracking: string;
  weight: string;
  label: string;
}> = {
  "mono-brutalist": {
    ui: "'JetBrains Mono', ui-monospace, monospace",
    mono: "'JetBrains Mono', ui-monospace, monospace",
    scale: 1,
    tracking: "0",
    weight: "500",
    label: "Mono Brutalist",
  },
  "modern-sans": {
    ui: "'Geist', -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif",
    mono: "'JetBrains Mono', 'Geist Mono', ui-monospace, 'SF Mono', Menlo, monospace",
    scale: 1,
    tracking: "0",
    weight: "450",
    label: "Modern Sans",
  },
  editorial: {
    ui: "'IBM Plex Sans', sans-serif",
    mono: "'IBM Plex Mono', ui-monospace, monospace",
    scale: 1.07,
    tracking: "0.005em",
    weight: "500",
    label: "Editorial Plex",
  },
};

export type DensityMode = "compact" | "regular" | "comfy";

export const DESIGN_DENSITY_SCALE: Record<DensityMode, number> = {
  compact: 0.88,
  regular: 1,
  comfy: 1.14,
};

export const DESIGN_RADIUS_TOKENS = {
  window: "12px",
  card: "8px",
  panel: "6px",
  button: "5px",
  tabTop: "6px 6px 0 0",
  chip: "11px",
  badge: "3px",
} as const;

export const DESIGN_TYPE_SCALE_TOKENS = {
  ui: "13px",
  // Reduced from 56px — the previous value blew out the workbench scale.
  welcomeTitle: "40px",
  label: "10.5px",
  status: "11px",
  code: "12px",
  disassembly: "11.5px",
  panelTitle: "11.5px",
} as const;

export const FONTS = Object.fromEntries(
  Object.entries(DESIGN_TYPE_SYSTEMS).map(([id, system]) => [id, { label: system.label, stack: system.mono }]),
) as Record<TypeSystemId, { label: string; stack: string }>;

export type FontId = TypeSystemId;

function toCssColors(theme: DesignThemeDef): Record<string, string> {
  const t = theme.tokens;
  return {
    ...Object.fromEntries(DESIGN_TOKEN_KEYS.map((key) => [`--${key}`, t[key]])),
    "--color-tcld-bg": t["bg-canvas"],
    "--color-tcld-bg-1": t["bg-window"],
    "--color-tcld-bg-2": t["bg-panel"],
    "--color-tcld-bg-3": t["bg-panel-2"],
    "--color-tcld-bg-highlight": t["bg-elev"],
    "--color-tcld-border": t.line,
    "--color-tcld-border-focus": t.accent,
    "--color-tcld-border-dim": t["line-2"],
    "--color-tcld-fg": t.fg,
    // Legacy aliases — preserved so existing Tailwind utility classes
    // (text-tcld-fg-dim, text-tcld-fg-muted) continue to resolve, but they
    // now point at the new semantic tokens with AA-passing contrast.
    "--color-tcld-fg-dim": t["text-inactive"],
    "--color-tcld-fg-muted": t["fg-2"],
    "--color-tcld-fg-bright": t.fg,
    "--color-tcld-text-inactive": t["text-inactive"],
    "--color-tcld-text-keyhint": t["text-keyhint"],
    "--color-tcld-text-placeholder": t["text-placeholder"],
    "--color-tcld-text-comment": t["text-comment"],
    "--color-tcld-accent": t.accent,
    "--color-tcld-accent-dim": t["accent-2"],
    "--color-tcld-keyword": t.kw,
    "--color-tcld-string": t.str,
    "--color-tcld-number": t.num,
    "--color-tcld-comment": t["text-comment"],
    "--color-tcld-variable": t.var,
    "--color-tcld-group": t.violet,
    "--color-tcld-group-dim": t["accent-bg"],
    "--color-tcld-error": t.err,
    "--color-tcld-warning": t.warn,
    "--color-tcld-success": t.accent,
    "--ok": "var(--color-tcld-success)",
    "--success": "var(--color-tcld-success)",
    "--border-focus": "var(--color-tcld-border-focus)",
    "--bg-highlight": "var(--color-tcld-bg-highlight)",
    "--group": "var(--color-tcld-group)",
  };
}

function themeDef(id: DesignThemeId): ThemeDef {
  const theme = DESIGN_THEME_TOKENS[id];
  return {
    label: theme.name,
    tokens: theme.tokens,
    colors: toCssColors(theme),
  };
}

const DESIGN_THEMES: Record<DesignThemeId, ThemeDef> = {
  graphite: themeDef("graphite"),
  catppuccin: themeDef("catppuccin"),
  dracula: themeDef("dracula"),
  solarized: themeDef("solarized"),
  tokyolight: themeDef("tokyolight"),
};

export const THEMES: Record<ThemeId, ThemeDef> = {
  ...DESIGN_THEMES,
  "tokyo-night": DESIGN_THEMES.graphite,
  "tokyo-night-light": DESIGN_THEMES.tokyolight,
  "catppuccin-mocha": DESIGN_THEMES.catppuccin,
  "solarized-dark": DESIGN_THEMES.solarized,
};

export function normalizeThemeId(theme: ThemeId | string): DesignThemeId {
  if (theme in DESIGN_THEMES) return theme as DesignThemeId;
  if (theme in THEME_ALIASES) return THEME_ALIASES[theme as LegacyThemeId];
  return "graphite";
}

export function buildDesignDensityVars(density: DensityMode): Record<string, string> {
  const d = DESIGN_DENSITY_SCALE[density];
  return {
    "--density": String(d),
    "--row-py": `${Math.round(5 * d)}px`,
    "--row-px": `${Math.round(10 * d)}px`,
    "--row-gap": `${Math.round(8 * d)}px`,
    "--panel-pad": `${Math.round(10 * d)}px`,
    "--panel-gap": `${Math.round(10 * d)}px`,
    "--header-h": `${Math.round(34 * d)}px`,
    "--row-h": `${Math.round(26 * d)}px`,
    "--disasm-row-py": `${Math.max(1, Math.round(2 * d))}px`,
    "--panel-header-height": `${Math.round(34 * d)}px`,
    "--code-row-height": `${Math.round(26 * d)}px`,
  };
}
