import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  DESIGN_DENSITY_SCALE,
  DESIGN_RADIUS_TOKENS,
  DESIGN_THEME_TOKENS,
  DESIGN_TOKEN_KEYS,
  DESIGN_TYPE_SCALE_TOKENS,
  DESIGN_TYPE_SYSTEMS,
  THEMES,
  THEME_ALIASES,
  buildDesignDensityVars,
  normalizeThemeId,
} from "@/lib/themes";
import {
  DEFAULT_SETTINGS,
  applySettings,
  loadSettingsWithTauriFallback,
  serializeSettingsForTauri,
  settingsConfigStorageContract,
} from "@/lib/settings";
import { __setInvokeForTests } from "@/lib/tauri-api";
import {
  DESIGN_DEVIATIONS,
  auditDesignDeviationInventory,
  findDeviation,
} from "@/lib/design-deviation-inventory";
import {
  DESIGN_PARITY_STATES,
  DESIGN_PREFERENCE_SECTION_MAPPINGS,
  DESIGN_SURFACES,
  DESIGN_VIEW_MAPPINGS,
  assertDesignSurfaceCoverage,
} from "@/lib/design-surface-contract";

const ROOT = path.resolve(import.meta.dirname, "../../..");
const GUI_SRC = path.join(ROOT, "apps/tcld-gui/src");

function readProductionFile(rel: string): string {
  return readFileSync(path.join(GUI_SRC, rel), "utf8");
}

function walkFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return walkFiles(full);
    return full;
  });
}

function fakeLocalStorage(initial: Record<string, string> = {}): Storage {
  const values = new Map(Object.entries(initial));
  return {
    get length() {
      return values.size;
    },
    clear: () => {
      values.clear();
    },
    key: (index: number) => [...values.keys()][index] ?? null,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}

async function withTauriWindow<T>(
  storage: Storage,
  invoke: Parameters<typeof __setInvokeForTests>[0],
  run: () => Promise<T>,
): Promise<T> {
  // Node has no `window`; the double cast gives us a mutable, optional slot
  // without widening the real DOM globals anywhere else.
  const globalRef = globalThis as unknown as { window?: unknown; localStorage?: unknown };
  const originalWindow = globalRef.window;
  const originalLocalStorage = globalRef.localStorage;
  globalRef.window = { __TAURI_INTERNALS__: {} };
  globalRef.localStorage = storage;
  __setInvokeForTests(invoke);
  try {
    return await run();
  } finally {
    __setInvokeForTests(null);
    if (originalWindow === undefined) {
      delete globalRef.window;
    } else {
      globalRef.window = originalWindow;
    }
    if (originalLocalStorage === undefined) {
      delete globalRef.localStorage;
    } else {
      globalRef.localStorage = originalLocalStorage;
    }
  }
}

// The design/ reference sources were deliberately removed from the repo
// (2026-05, "Remove design inventory/parity docs; relocate plans to
// top-level"). src/lib/themes.ts is now the single source of truth, so this
// contract asserts internal completeness and self-consistency of the theme /
// alias / type-system / density vocabulary rather than parity with a file
// that no longer exists.
test("V1 design themes, aliases, type systems, and density formulas are complete and self-consistent", () => {
  // Themes: the five canonical workbench themes, each exposing exactly the
  // shared token vocabulary with concrete color values.
  assert.deepEqual(
    Object.keys(DESIGN_THEME_TOKENS).sort(),
    ["catppuccin", "dracula", "graphite", "solarized", "tokyolight"],
  );
  for (const [themeId, theme] of Object.entries(DESIGN_THEME_TOKENS)) {
    assert.ok(theme.name.length > 0, `${themeId} has a display name`);
    assert.ok(theme.accentName.length > 0, `${themeId} names its accent`);
    assert.deepEqual(Object.keys(theme.tokens).sort(), [...DESIGN_TOKEN_KEYS].sort(), `${themeId} exposes every design token`);
    for (const [token, value] of Object.entries(theme.tokens)) {
      assert.match(
        value,
        /^(#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?|rgba?\([^)]+\))$/,
        `${themeId}.${token} must be a concrete color, got ${value}`,
      );
    }
  }

  // Aliases: every legacy id maps to a real design theme, resolves through
  // normalizeThemeId, and shares the target's definition in THEMES.
  assert.equal(THEME_ALIASES["tokyo-night"], "graphite");
  assert.equal(THEME_ALIASES["tokyo-night-light"], "tokyolight");
  assert.equal(THEME_ALIASES["catppuccin-mocha"], "catppuccin");
  assert.equal(THEME_ALIASES["solarized-dark"], "solarized");
  for (const [legacyId, designId] of Object.entries(THEME_ALIASES)) {
    assert.ok(designId in DESIGN_THEME_TOKENS, `${legacyId} maps to a real design theme`);
    assert.equal(normalizeThemeId(legacyId), designId, `${legacyId} normalizes to ${designId}`);
    assert.equal(
      THEMES[legacyId as keyof typeof THEMES],
      THEMES[designId],
      `${legacyId} shares the ${designId} ThemeDef`,
    );
  }
  assert.equal(normalizeThemeId("not-a-theme"), "graphite", "unknown themes fall back to graphite");

  // Type systems: all three are fully described with real font stacks.
  assert.deepEqual(Object.keys(DESIGN_TYPE_SYSTEMS).sort(), ["editorial", "modern-sans", "mono-brutalist"]);
  for (const [systemId, system] of Object.entries(DESIGN_TYPE_SYSTEMS)) {
    assert.ok(system.ui.includes(","), `${systemId}.ui declares a fallback stack`);
    assert.match(system.mono, /mono/i, `${systemId}.mono is a monospace stack`);
    assert.ok(system.scale > 0.5 && system.scale < 2, `${systemId}.scale stays near 1`);
    assert.ok(system.weight.length > 0, `${systemId} declares a weight`);
    assert.ok(system.tracking.length > 0, `${systemId} declares tracking`);
    assert.ok(system.label.length > 0, `${systemId} declares a label`);
  }

  // Density: compact < regular (= 1) < comfy, and the derived CSS variables
  // follow the shared rounding formulas so paired variables stay in sync.
  assert.deepEqual(Object.keys(DESIGN_DENSITY_SCALE).sort(), ["comfy", "compact", "regular"]);
  assert.equal(DESIGN_DENSITY_SCALE.regular, 1);
  assert.ok(DESIGN_DENSITY_SCALE.compact < DESIGN_DENSITY_SCALE.regular, "compact tightens rows");
  assert.ok(DESIGN_DENSITY_SCALE.comfy > DESIGN_DENSITY_SCALE.regular, "comfy loosens rows");
  for (const mode of ["compact", "regular", "comfy"] as const) {
    const scale = DESIGN_DENSITY_SCALE[mode];
    const vars = buildDesignDensityVars(mode);
    assert.equal(vars["--density"], String(scale), `${mode} exposes its scale`);
    assert.equal(vars["--row-py"], `${Math.round(5 * scale)}px`, `${mode} row padding follows the density formula`);
    assert.equal(vars["--row-px"], `${Math.round(10 * scale)}px`, `${mode} row inline padding follows the density formula`);
    assert.equal(vars["--header-h"], `${Math.round(34 * scale)}px`, `${mode} header height follows the density formula`);
    assert.equal(vars["--row-h"], `${Math.round(26 * scale)}px`, `${mode} row height follows the density formula`);
    assert.equal(vars["--panel-header-height"], vars["--header-h"], `${mode} keeps legacy header-height alias in sync`);
    assert.equal(vars["--code-row-height"], vars["--row-h"], `${mode} keeps legacy code-row alias in sync`);
  }
});

test("V2 appearance settings apply design CSS variables and preserve Tauri config persistence shape", () => {
  const root = {
    dataset: {} as Record<string, string>,
    style: {
      values: new Map<string, string>(),
      setProperty(name: string, value: string) {
        this.values.set(name, value);
      },
    },
  } as unknown as HTMLElement & { style: { values: Map<string, string>; setProperty(name: string, value: string): void } };

  applySettings({ ...DEFAULT_SETTINGS, theme: "graphite", typeSystem: "editorial", density: "comfy", showSparklines: false, groupRails: false }, root);

  assert.equal(root.style.values.get("--bg-canvas"), DESIGN_THEME_TOKENS.graphite.tokens["bg-canvas"]);
  assert.equal(root.style.values.get("--ui"), DESIGN_TYPE_SYSTEMS.editorial.ui);
  assert.equal(root.style.values.get("--row-py"), "6px");
  assert.equal(root.style.values.get("--rail-display"), "none");
  assert.equal(root.style.values.get("--variable-sparklines"), "none");
  assert.equal(settingsConfigStorageContract.tauriCommands.load, "load_appearance_settings");
  assert.equal(settingsConfigStorageContract.tauriCommands.save, "save_appearance_settings");
  assert.equal(settingsConfigStorageContract.tauriEvents.changed, "appearance:changed");
  assert.match(settingsConfigStorageContract.productionConfigPath, /tcld[/\\]appearance-settings\.json$/);
  assert.equal(serializeSettingsForTauri(DEFAULT_SETTINGS).theme, "graphite");
});

test("V2 Tauri appearance load preserves browser fallback only when config is missing", async () => {
  const browserSettings = { ...DEFAULT_SETTINGS, theme: "dracula" as const, density: "comfy" as const };
  const storage = fakeLocalStorage({
    [settingsConfigStorageContract.browserFallbackStorageKey]: JSON.stringify(browserSettings),
  });

  const missingConfig = await withTauriWindow(storage, async (cmd) => {
    assert.equal(cmd, "load_appearance_settings");
    return null as never;
  }, () => loadSettingsWithTauriFallback());

  assert.equal(missingConfig.theme, "dracula");
  assert.equal(missingConfig.density, "comfy");

  const explicitDefault = await withTauriWindow(storage, async (cmd) => {
    assert.equal(cmd, "load_appearance_settings");
    return serializeSettingsForTauri(DEFAULT_SETTINGS) as never;
  }, () => loadSettingsWithTauriFallback());

  assert.equal(explicitDefault.theme, DEFAULT_SETTINGS.theme);
  assert.equal(explicitDefault.density, DEFAULT_SETTINGS.density);
});

test("V2 hardcoded design palette audit keeps UI colors behind approved token definitions", () => {
  const approved = new Set([
    path.join(GUI_SRC, "lib/themes.ts"),
    path.join(GUI_SRC, "app/globals.css"),
    path.join(GUI_SRC, "lib/design-deviation-inventory.ts"),
    path.join(GUI_SRC, "lib/design-surface-contract.ts"),
  ]);
  const productionFiles = walkFiles(GUI_SRC).filter((file) => /\.(ts|tsx|css)$/.test(file));
  const paletteValues = new Set(
    Object.values(DESIGN_THEME_TOKENS).flatMap((theme) =>
      Object.values(theme.tokens).filter((value) => value.startsWith("#")),
    ),
  );
  const leaks = productionFiles.flatMap((file) => {
    if (approved.has(file)) return [];
    const text = readFileSync(file, "utf8");
    return [...paletteValues].filter((value) => text.includes(value)).map((value) => `${path.relative(ROOT, file)}:${value}`);
  });
  assert.deepEqual(leaks, []);
});

test("V3 shell surfaces expose design chrome/welcome markers and justified deviations", () => {
  assertDesignSurfaceCoverage("shell");
  const welcome = readProductionFile("components/WelcomeScreen.tsx");
  const page = readProductionFile("app/page.tsx");
  for (const marker of DESIGN_SURFACES.shell.productionMarkers) {
    assert.match(`${welcome}\n${page}`, new RegExp(marker), `missing shell marker ${marker}`);
  }
  assert.doesNotMatch(welcome, /<Capability|Decompile|Correlate/);
  assert.ok(findDeviation("shell", "tcld-welcome.jsx", "Launch Debug secondary CTA"));
  assert.ok(findDeviation("shell", "tcld-welcome.jsx", "tab dirty dot"));
});

test("V4 workspace surfaces expose design presets, singleton tab chrome, save state, and minimum widths", () => {
  assertDesignSurfaceCoverage("workspace");
  const layouts = readProductionFile("lib/default-layouts.ts");
  const workspace = readProductionFile("components/MosaicWorkspace.tsx");
  for (const label of [
    "Default - disasm / source / inspect",
    "Flow focus - graph / source / inspect",
    "Trace + console",
    "Console focus - source / console",
    "Symbols deep dive",
    "Cockpit - source / disasm / flow",
    "Runbook - source / debug rail / console",
  ]) {
    assert.match(layouts, new RegExp(label.replace(/[+/]/g, "\\$&")), `missing design preset ${label}`);
  }
  assert.match(workspace, /data-design-surface="workspace-mosaic"/);
  assert.match(workspace, /minSize=\{220\}/);
  assert.match(workspace, /layoutDirty/);
  assert.match(workspace, /saveLayout/);
  assert.ok(findDeviation("workspace", "tcld-panels.jsx", "drag-resize implementation"));
});

test("V5 every design-listed view maps to a production panel with model-backed surface markers", () => {
  assertDesignSurfaceCoverage("views");
  const expected = [
    "disasm",
    "symbols",
    "source",
    "inst",
    "compare",
    "inspector",
    "issues",
    "flow",
    "trace",
    "console",
    "variables",
    "stack",
    "breakpoints",
    "watchpoints",
  ];
  for (const designId of expected) {
    const mapping = DESIGN_VIEW_MAPPINGS.find((view) => view.designId === designId);
    assert.ok(mapping, `missing mapping for ${designId}`);
    assert.ok(mapping?.productionViewId || mapping?.deviationId, `${designId} has production route or deviation`);
    if (mapping?.productionFile) {
      assert.match(readProductionFile(mapping.productionFile), new RegExp(`(data-design-surface|designSurface)="${mapping.designSurface}"`));
    }
  }
});

test("V6 debug surfaces expose paused overlay, typed badges, sparklines preference, and breakpoint metadata", () => {
  assertDesignSurfaceCoverage("debug");
  const debugToolbar = readProductionFile("components/chrome/DebugToolbar.tsx");
  const variables = readProductionFile("components/panels/VariablesPanel.tsx");
  const stack = readProductionFile("components/panels/StackPanel.tsx");
  const breakpoints = readProductionFile("components/panels/BreakpointsPanel.tsx");
  assert.match(debugToolbar, /data-design-surface="debug-paused-overlay"/);
  assert.match(variables, /(data-design-surface|designSurface)="debug-variables"/);
  assert.match(variables, /--variable-sparklines/);
  assert.match(stack, /(data-design-surface|designSurface)="debug-call-stack"/);
  assert.match(breakpoints, /(data-design-surface|designSurface)="debug-breakpoints"/);
  assert.match(breakpoints, /hitCount|condition|enabled/);
});

test("V7 preferences and run configuration windows expose detached design routes", () => {
  assertDesignSurfaceCoverage("detached-windows");
  const page = readProductionFile("app/page.tsx");
  const api = readProductionFile("lib/tauri-api.ts");
  assert.match(page, /windowMode === "preferences"/);
  assert.match(page, /loadSettingsWithTauriFallback/);
  assert.match(page, /listen<SerializedAppearanceSettings>\(settingsConfigStorageContract\.tauriEvents\.changed/);
  assert.match(page, /emitTauriEvent\(settingsConfigStorageContract\.tauriEvents\.changed/);
  assert.match(page, /data-design-surface="preferences-window"/);
  assert.match(page, /Appearance|Editor|Keybindings|Decompiler|Debugger|Run profiles|Telemetry|Advanced/);
  assert.match(page, /data-design-surface="run-configuration-window"/);
  assert.match(api, /openPreferencesWindow/);
  assert.ok(
    findDeviation("shell", "tcld-welcome.jsx", "ActionBar inline run-config popover"),
    "action bar inline run-config omission must have a runtime deviation",
  );
  assert.ok(
    DESIGN_PARITY_STATES.find((state) => state.id === "open-file-workspace")?.deviations.includes("actionbar-detached-run-config"),
    "open workspace manifest must carry the detached run-config deviation",
  );
  assert.deepEqual(DESIGN_PREFERENCE_SECTION_MAPPINGS.map((section) => section.label), [
    "Appearance",
    "Editor",
    "Keybindings",
    "Decompiler",
    "Debugger",
    "Run profiles",
    "Telemetry",
    "Advanced",
  ]);
  for (const section of DESIGN_PREFERENCE_SECTION_MAPPINGS) {
    assert.match(section.productionSelector, new RegExp(`preferences-${section.slug}`));
    assert.match(section.productionRoute, /window=preferences/);
    assert.equal(section.designSource, "tcld-prefs.jsx");
    assert.ok(section.productionBehavior.length > 20, `${section.label} must name live behavior or disabled-state evidence`);
  }
  assert.deepEqual(
    DESIGN_PREFERENCE_SECTION_MAPPINGS.map((section) => `${section.group}:${section.label}`),
    [
      "Workspace:Appearance",
      "Workspace:Editor",
      "Workspace:Keybindings",
      "Tools:Decompiler",
      "Tools:Debugger",
      "Tools:Run profiles",
      "System:Telemetry",
      "System:Advanced",
    ],
  );
  assert.match(page, /data-preferences-group/);
  assert.match(page, /data-preferences-badge/);
});

test("V8 radius and type-scale variables cover the design inventory and are used by production surfaces", () => {
  assert.deepEqual(DESIGN_RADIUS_TOKENS, {
    window: "12px",
    card: "8px",
    panel: "6px",
    button: "5px",
    tabTop: "6px 6px 0 0",
    chip: "11px",
    badge: "3px",
  });
  for (const token of ["--radius-window", "--radius-card", "--radius-panel", "--radius-button", "--radius-tab-top", "--radius-chip", "--type-ui", "--type-label", "--type-status", "--type-code"]) {
    assert.match(readProductionFile("app/globals.css"), new RegExp(token), `${token} must be defined in globals.css`);
  }
  assert.equal(DESIGN_TYPE_SCALE_TOKENS.ui, "13px");
  assert.equal(DESIGN_TYPE_SCALE_TOKENS.code, "12px");
});

test("V10 deviation inventory is structured, specific, and rejects unimplemented enabled controls", () => {
  const audit = auditDesignDeviationInventory(DESIGN_DEVIATIONS, {
    mappedDesignSurfaces: DESIGN_VIEW_MAPPINGS.flatMap((mapping) => mapping.deviationId
      ? [{ deviationId: mapping.deviationId, designSurface: mapping.designSurface }]
      : []),
  });
  assert.deepEqual(audit.errors, []);
  assert.ok(DESIGN_DEVIATIONS.length >= 12, "the visual adaptation requires an explicit inventory");
  assert.ok(DESIGN_DEVIATIONS.every((entry) => entry.verificationIds.length > 0));
});

test("V11 parity state manifest maps production states to design source files and deviations", () => {
  const harness = readFileSync(path.join(ROOT, "apps/tcld-gui/scripts/design-parity.mjs"), "utf8");
  assert.doesNotMatch(harness, /function svgFor|svgFor\(/, "V11 must not hand-author screenshot stand-ins");
  assert.match(harness, /puppeteer|playwright/, "V11 harness must use a browser renderer");
  assert.doesNotMatch(harness, /nextBin,\s*"dev"/, "V11 production capture must not run next dev");
  assert.match(harness, /runNextCommand\(\["build"\]/, "V11 harness must build before production capture");
  assert.match(harness, /serveProductionExport/, "V11 harness must capture the built production export");
  assert.match(harness, /page\.goto\(/, "V11 harness must navigate rendered production/design routes");
  assert.match(harness, /waitForSelector\(/, "V11 harness must fail when required states are missing");
  assert.match(harness, /\.screenshot\(/, "V11 harness must capture rendered image artifacts");

  assert.deepEqual(DESIGN_PARITY_STATES.map((state) => state.id), [
    "welcome",
    "open-file-workspace",
    "debug-paused-workspace",
    "flow",
    "trace",
    "console",
    "symbols",
    "preferences-appearance",
    "preferences-decompiler",
    "preferences-keybindings",
    "run-configuration",
    "variables-detail",
    "call-stack-detail",
    "breakpoints-detail",
    "theme-graphite-inspect",
    "theme-catppuccin-inspect",
    "theme-dracula-inspect",
    "theme-solarized-inspect",
    "theme-tokyolight-inspect",
  ]);
  for (const state of DESIGN_PARITY_STATES) {
    assert.ok(state.designSource.endsWith(".jsx"));
    assert.ok(state.productionRoute.length > 0);
    assert.ok(state.designArtboardId?.length, `${state.id} must name a concrete design artboard id`);
    assert.ok(state.productionSelector?.length, `${state.id} must name the required production selector`);
    assert.ok(state.artifacts.design.endsWith(".png"));
    assert.ok(state.artifacts.production.endsWith(".png"));
  }
  // The removed design source named the flagship theme "Graphite Workbench";
  // the in-repo theme table remains the carrier of that identity.
  assert.equal(DESIGN_THEME_TOKENS.graphite.name, "Graphite Workbench");
});
