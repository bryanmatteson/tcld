import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  DESIGN_CONTROL_MAPPINGS,
  DESIGN_PARITY_STATES,
  DESIGN_SURFACES,
  DESIGN_VIEW_MAPPINGS,
} from "@/lib/design-surface-contract";
import {
  LAYOUT_PRESETS,
  getOpenViews,
  workspaceFromViewTree,
} from "@/lib/default-layouts";
import {
  VIEW_DEFINITIONS,
  VIEW_ICON_ROLES,
} from "@/lib/view-registry";
import {
  DESIGN_DEVIATIONS,
  auditDesignDeviationInventory,
} from "@/lib/design-deviation-inventory";
import {
  adaptiveTabsHardMinimum,
  estimateAdaptiveTabWidth,
  fitAdaptiveTabs,
  renderAdaptiveTabLabel,
} from "@/lib/adaptive-tabs";
import { T } from "@/lib/theme";
import { DESIGN_DENSITY_SCALE, DESIGN_THEME_TOKENS, DESIGN_TYPE_SYSTEMS, THEMES } from "@/lib/themes";
import type { ViewId, WorkspaceLayoutSnapshot, WorkspaceLeafId, WorkspaceTree } from "@/types";

const ROOT = path.resolve(import.meta.dirname, "../../..");
const GUI_SRC = path.join(ROOT, "apps/tcld-gui/src");

const REQUIRED_ARTBOARDS = [
  "welcome",
  "inspect",
  "debug",
  "flow",
  "trace",
  "console",
  "symbols",
  "prefs-appearance",
  "prefs-decomp",
  "prefs-keys",
  "runcfg",
  "vars",
  "stack",
  "bps",
] as const;

const REQUIRED_VIEW_IDS = [
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
] as const;

const REQUIRED_DESIGN_VIEW_ICON_ROLES: Record<(typeof REQUIRED_VIEW_IDS)[number], string> = {
  disasm: "cpu",
  symbols: "list",
  source: "file",
  inst: "list",
  compare: "cpu",
  inspector: "eye",
  issues: "alert",
  flow: "flow",
  trace: "trace",
  console: "terminal",
  variables: "stack",
  stack: "layers",
  breakpoints: "circle",
  watchpoints: "circle",
};

const REQUIRED_PRODUCTION_VIEW_ICON_ROLES = {
  instructions: "cpu",
  literals: "list",
  decompile: "file",
  comparison: "compare",
  detail: "inspector",
  diagnostics: "alert",
  graph: "flow",
  provenance: "trace",
  console: "console",
  variables: "variables",
  stack: "stack",
  breakpoints: "breakpoint",
  watchpoints: "watchpoint",
} as const;

const REQUIRED_CONTROL_IDS = [
  "welcome.open-file",
  "welcome.file-drop",
  "welcome.recents-open",
  "welcome.recents-clear",
  "workspace.layout-preset",
  "workspace.save-layout",
  "workspace.add-singleton-view",
  "workspace.swap-singleton-view",
  "workspace.close-last-tab",
  "workspace.drag-reorder",
  "workspace.resize-min-width",
  "debug.run",
  "debug.pause",
  "debug.step",
  "debug.continue",
  "debug.stop",
  "instructions.fold-filter",
  "source.linked-navigation",
  "source.split",
  "flow.detail-mode",
  "trace.timeline-select",
  "console.channel-filter",
  "console.repl-input",
  "compare.selection",
  "inspector.selection",
  "issues.selection",
  "variables.expand-sparkline",
  "stack.frame-selection",
  "breakpoints.toggle-condition-hits",
  "watchpoints.production-backed",
  "preferences.theme",
  "preferences.type-system",
  "preferences.density",
  "preferences.sparklines",
  "preferences.group-rails",
  "run-config.save-cancel-delete",
  "run-config.import-export",
] as const;

const DESIGN_PRESET_STACKS = {
  default: [["disasm", "symbols"], ["source", "inst", "compare"], ["inspector", "issues"]],
  flow: [["flow"], ["source"], ["inspector", "issues"]],
  trace: [["trace"], ["source"], ["console"]],
  console: [["source"], ["console"]],
  symbols: [["symbols", "disasm"], ["source"], ["inspector", "issues"]],
  cockpit: [["disasm", "flow"], ["source", "compare"], ["inspector", "issues"]],
  runbook: [["source"], ["disasm"], ["variables", "stack", "breakpoints", "watchpoints"], ["console"]],
} as const;

function mappedDesignViews(designIds: readonly string[]): ViewId[] {
  const views: ViewId[] = [];
  for (const designId of designIds) {
    const viewId = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === designId)?.productionViewId;
    assert.ok(viewId, `missing production mapping for design view ${designId}`);
    if (!views.includes(viewId)) {
      views.push(viewId);
    }
  }
  return views;
}

function mappedDesignStacks(designStacks: readonly (readonly string[])[]): ViewId[][] {
  const seen = new Set<ViewId>();
  return designStacks.map((stack) => {
    const views: ViewId[] = [];
    for (const view of mappedDesignViews(stack)) {
      if (seen.has(view)) continue;
      seen.add(view);
      views.push(view);
    }
    return views;
  });
}

function presetWorkspace(name: string): WorkspaceLayoutSnapshot {
  const preset = LAYOUT_PRESETS.find((candidate) => candidate.name === name);
  assert.ok(preset, `missing layout preset ${name}`);
  return preset.workspace ?? workspaceFromViewTree(preset.tree);
}

function orderedLeafIds(tree: WorkspaceTree | null): WorkspaceLeafId[] {
  if (tree === null) return [];
  if (typeof tree === "string") return [tree];
  return [
    ...orderedLeafIds(tree.first as WorkspaceTree),
    ...orderedLeafIds(tree.second as WorkspaceTree),
  ];
}

function leafAreaPercentages(tree: WorkspaceTree | null, area = 1): Record<WorkspaceLeafId, number> {
  if (tree === null) return {};
  if (typeof tree === "string") return { [tree]: area };
  const firstShare = (tree.splitPercentage ?? 50) / 100;
  return {
    ...leafAreaPercentages(tree.first as WorkspaceTree, area * firstShare),
    ...leafAreaPercentages(tree.second as WorkspaceTree, area * (1 - firstShare)),
  };
}

function viewArea(workspace: WorkspaceLayoutSnapshot, viewId: ViewId): number {
  const areas = leafAreaPercentages(workspace.tree);
  return Object.values(workspace.stacks)
    .filter((stack) => stack.views.includes(viewId))
    .reduce((sum, stack) => sum + (areas[stack.leafId] ?? 0), 0);
}

function readProductionFile(rel: string): string {
  return readFileSync(path.join(GUI_SRC, rel), "utf8");
}

function listProductionFiles(relDir: string): string[] {
  const absDir = path.join(GUI_SRC, relDir);
  return readdirSync(absDir).flatMap((entry) => {
    const rel = path.join(relDir, entry);
    const abs = path.join(GUI_SRC, rel);
    return statSync(abs).isDirectory() ? listProductionFiles(rel) : [rel];
  });
}

const RETAINED_SHORTCUT_GLYPH_CONTRACT = "`⌘` in Search & commands / command palette hints";
const CONSOLE_STREAM_GLYPH_CONTRACT = "console stream-role markers in ConsolePanel STREAM_PRESENTATION";

const RAW_ICON_SOURCE_ALLOWLIST = [
  { file: "components/chrome/FileHeader.tsx", glyph: "⌘", contract: RETAINED_SHORTCUT_GLYPH_CONTRACT, context: /⌘[KP]/u },
  { file: "components/chrome/TabBar.tsx", glyph: "⌘", contract: RETAINED_SHORTCUT_GLYPH_CONTRACT, context: /⌘P/u },
  { file: "components/chrome/DebugToolbar.tsx", glyph: "⌘", contract: RETAINED_SHORTCUT_GLYPH_CONTRACT, context: /shortcut="⌘⇧F5"/u },
  { file: "components/overlays/CommandPalette.tsx", glyph: "⌘", contract: RETAINED_SHORTCUT_GLYPH_CONTRACT, context: /^\s*⌘\s*$/u },
  { file: "components/panels/InstructionPanel.tsx", glyph: "⌘", contract: RETAINED_SHORTCUT_GLYPH_CONTRACT, context: /⌘F/u },
  { file: "components/panels/Panel.tsx", glyph: "⌘", contract: RETAINED_SHORTCUT_GLYPH_CONTRACT, context: /⌘W/u },
  { file: "components/panels/SummaryPanel.tsx", glyph: "⌘", contract: RETAINED_SHORTCUT_GLYPH_CONTRACT, context: /⌘O/u },
  { file: "components/panels/ConsolePanel.tsx", glyph: "▶", contract: CONSOLE_STREAM_GLYPH_CONTRACT, context: /eval:\s+\{ glyph: "▶"/u },
] as const;

// design/components was deliberately removed from the repo (2026-05);
// src/lib/themes.ts and the in-repo contracts are the source of truth. The
// enumeration below asserts internal completeness of that vocabulary instead
// of parity with the deleted design files.
test("V1 enumerates every relevant design file, artboard, token group, view, and visible control", () => {
  const actualArtboards = new Set(DESIGN_PARITY_STATES.map((state) => state.designArtboardId));
  const themeArtboards = Object.keys(DESIGN_THEME_TOKENS).map((themeId) => `th-${themeId}`);

  for (const artboard of [...REQUIRED_ARTBOARDS, ...themeArtboards]) {
    assert.ok(actualArtboards.has(artboard), `missing production parity state for design artboard ${artboard}`);
  }

  for (const viewId of REQUIRED_VIEW_IDS) {
    assert.ok(
      DESIGN_VIEW_MAPPINGS.some((mapping) => mapping.designId === viewId),
      `missing design view mapping for ${viewId}`,
    );
    assert.equal(
      DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === viewId)?.designIconRole,
      REQUIRED_DESIGN_VIEW_ICON_ROLES[viewId],
      `design view ${viewId} must preserve its icon role`,
    );
  }

  const controlIds = new Set(DESIGN_CONTROL_MAPPINGS.map((mapping) => mapping.id));
  for (const controlId of REQUIRED_CONTROL_IDS) {
    assert.ok(controlIds.has(controlId), `missing visible interaction mapping for ${controlId}`);
  }

  // Token-group completeness: the five canonical themes, the three type
  // systems, and the three density modes stay fully enumerated.
  assert.deepEqual(
    Object.keys(DESIGN_THEME_TOKENS).sort(),
    ["catppuccin", "dracula", "graphite", "solarized", "tokyolight"],
  );
  assert.deepEqual(Object.keys(DESIGN_TYPE_SYSTEMS).sort(), ["editorial", "modern-sans", "mono-brutalist"]);
  assert.deepEqual(Object.keys(DESIGN_DENSITY_SCALE).sort(), ["comfy", "compact", "regular"]);
  assert.ok(DESIGN_SURFACES.views.designFiles.includes("tcld-flow.jsx"));
  assert.ok(DESIGN_SURFACES.debug.designFiles.includes("tcld-debug.jsx"));
});

test("V1 icon inventory maps required design roles to registry-backed production view icons", () => {
  for (const [viewId, role] of Object.entries(REQUIRED_PRODUCTION_VIEW_ICON_ROLES)) {
    assert.equal(
      VIEW_ICON_ROLES[viewId as keyof typeof REQUIRED_PRODUCTION_VIEW_ICON_ROLES],
      role,
      `production view ${viewId} must keep the expected lucide icon role`,
    );
  }

  const symbols = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === "symbols");
  assert.equal(symbols?.productionViewId, "literals");
  assert.equal(symbols?.deviationId, "views-symbols-id");
  assert.notEqual(symbols?.deviationId, "views-inst-id");

  const requiredViews = new Set(Object.keys(REQUIRED_PRODUCTION_VIEW_ICON_ROLES));
  for (const view of VIEW_DEFINITIONS.filter((definition) => requiredViews.has(definition.id))) {
    assert.ok(view.iconRole, `${view.id} must expose an icon role`);
  }
});

test("V1 Symbols surface keeps transitional id mapping but renders Symbols table contract", () => {
  const panel = readFileSync(path.join(GUI_SRC, "components/panels/LiteralsPanel.tsx"), "utf8");
  const model = readFileSync(path.join(GUI_SRC, "lib/panel-models.ts"), "utf8");
  const surfaceContract = readFileSync(path.join(GUI_SRC, "lib/design-surface-contract.ts"), "utf8");
  const deviations = readFileSync(path.join(GUI_SRC, "lib/design-deviation-inventory.ts"), "utf8");
  const symbolsMappingLine = surfaceContract.split("\n").find((line) => line.includes('designId: "symbols"')) ?? "";

  assert.match(symbolsMappingLine, /productionViewId: "literals"/);
  assert.match(symbolsMappingLine, /deviationId: "views-symbols-id"/);
  assert.doesNotMatch(symbolsMappingLine, /views-inst-id/);
  assert.match(deviations, /id: "views-symbols-id"[\s\S]*Symbols unavailable/);
  assert.match(panel, /data-symbol-filter="true"/);
  assert.match(panel, /data-symbol-count="true"/);
  assert.match(panel, /data-symbol-kind-pill=\{row\.kind\}/);
  for (const column of ["kind", "name", "addr", "size", "refs"]) {
    assert.match(panel, new RegExp(`data-symbol-column="${column}"`), `missing Symbols column ${column}`);
  }
  assert.match(model, /buildSymbolsPanelModel/);
  assert.match(model, /unavailableTitle: "Symbols unavailable"/);
  assert.match(model, /unavailableExplanation: "This payload does not expose namespace\/proc\/var symbol facts yet\."/);
  assert.doesNotMatch(panel, /fixture::stoplog|fixture::setLevel|debug_types|debug_level/, "production Symbols panel must not copy prototype symbol rows");
});

test("V4 design-named layout presets materialize mapped production view stacks as singletons", () => {
  for (const [name, designStacks] of Object.entries(DESIGN_PRESET_STACKS)) {
    const workspace = presetWorkspace(name);
    const openViews = getOpenViews(workspace.tree, workspace.stacks);
    assert.equal(new Set(openViews).size, openViews.length, `${name} must not duplicate singleton view IDs`);

    const expectedOpenViews = mappedDesignViews(designStacks.flat());
    assert.deepEqual(
      openViews.slice().sort(),
      expectedOpenViews.slice().sort(),
      `${name} must resolve design view IDs through DESIGN_VIEW_MAPPINGS`,
    );

    const actualStacks = orderedLeafIds(workspace.tree).map((leafId) => workspace.stacks[leafId]?.views ?? []);
    const expectedStacks = mappedDesignStacks(designStacks);
    assert.deepEqual(actualStacks, expectedStacks, `${name} stack composition must match the design slot groups`);
  }
});

test("V4 design-named layout presets preserve relative design emphasis in Mosaic percentages", () => {
  const flow = presetWorkspace("flow");
  assert.ok(viewArea(flow, "graph") > viewArea(flow, "decompile"), "flow gives graph more area than source");
  assert.ok(viewArea(flow, "graph") > viewArea(flow, "detail"), "flow gives graph more area than inspector");

  const trace = presetWorkspace("trace");
  assert.ok(viewArea(trace, "provenance") > viewArea(trace, "decompile"), "trace gives provenance a primary pane");
  assert.ok(viewArea(trace, "provenance") > viewArea(trace, "console"), "trace keeps trace/provenance dominant beside source/console");

  const consolePreset = presetWorkspace("console");
  assert.ok(viewArea(consolePreset, "console") > viewArea(consolePreset, "decompile"), "console gives console more area than source");

  const symbols = presetWorkspace("symbols");
  assert.equal(orderedLeafIds(symbols.tree)[0], "leaf-1", "symbols keeps the symbols stack in the leading pane");
  assert.ok(viewArea(symbols, "literals") >= viewArea(symbols, "detail"), "symbols keeps Symbols/Literals at primary emphasis");

  const cockpit = presetWorkspace("cockpit");
  assert.ok(viewArea(cockpit, "decompile") > viewArea(cockpit, "detail"), "cockpit keeps source larger than inspector");
  assert.ok(viewArea(cockpit, "graph") > viewArea(cockpit, "detail"), "cockpit keeps flow reachable beside disassembly");

  const runbook = presetWorkspace("runbook");
  assert.ok(viewArea(runbook, "decompile") > viewArea(runbook, "console"), "runbook keeps source primary over console");
  assert.ok(viewArea(runbook, "variables") > viewArea(runbook, "console"), "runbook keeps debug rail dominant over console");
  assert.ok(viewArea(runbook, "watchpoints") > 0, "runbook includes watchpoints in the debug rail");
});

test("V1 mapped design view deviations stay on their declared design surfaces", () => {
  const deviationsById = new Map(DESIGN_DEVIATIONS.map((entry) => [entry.id, entry]));
  for (const mapping of DESIGN_VIEW_MAPPINGS) {
    if (!mapping.deviationId) continue;
    const deviation = deviationsById.get(mapping.deviationId);
    assert.ok(deviation, `${mapping.designId} deviation ${mapping.deviationId} must exist`);
    assert.match(
      deviation.designSurface,
      new RegExp(`\\b${mapping.designSurface}\\b`),
      `${mapping.designId} deviation ${mapping.deviationId} must name ${mapping.designSurface}`,
    );
  }

  const symbols = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === "symbols");
  assert.equal(symbols?.productionViewId, "literals");
  assert.equal(symbols?.deviationId, "views-symbols-id");
});

// design/parity/icon-inventory.md was removed with the design tree; the
// retained-glyph contract now lives fail-closed in this test: the exact set
// of registry views allowed to keep a raw glyph is enumerated, so any new
// raw-glyph icon (or a glyph appearing on an unexpected view) fails here.
test("V1 retained registry glyphs are explicit contracts and rendered view icons use lucide", () => {
  const allowedGlyphViews = new Map([
    ["instructions", "views-disasm-id"],
    ["literals", "views-symbols-id"],
    ["decompile", "source design view"],
    ["diagnostics", "views-issues-id"],
    ["provenance", "views-trace-id"],
    ["console", "console design view"],
    ["commandResolver", "platform shortcut glyph"],
  ]);

  const rawGlyphPattern = /[^\w\s#?]/u;
  const glyphViews = VIEW_DEFINITIONS.filter((view) => rawGlyphPattern.test(view.icon));
  assert.deepEqual(
    glyphViews.map((view) => view.id).sort(),
    [...allowedGlyphViews.keys()].sort(),
    "raw registry glyphs are limited to the explicitly contracted views",
  );
  for (const view of glyphViews) {
    assert.ok(
      allowedGlyphViews.has(view.id) || !Object.keys(REQUIRED_PRODUCTION_VIEW_ICON_ROLES).includes(view.id),
      `${view.id} uses retained glyph ${view.icon} without an icon-inventory contract`,
    );
    assert.ok(view.iconRole, `${view.id} must still expose a lucide icon role for rendered icons`);
  }

  const viewIcon = readProductionFile("components/ViewIcon.tsx");
  const mosaic = readProductionFile("components/MosaicWorkspace.tsx");
  const panelPicker = readProductionFile("components/chrome/PanelPicker.tsx");
  assert.match(viewIcon, /strokeWidth=\{1\.75\}/);
  assert.match(viewIcon, /size = 13/);
  assert.match(mosaic, /<ViewIcon viewId=\{activeView\} size=\{13\}/);
  assert.match(mosaic, /<ViewIcon viewId=\{view\} size=\{12\}/);
  assert.match(panelPicker, /<ViewIcon viewId=\{view\.id\} size=\{13\}/);
});

test("V1 production panel, chrome, and overlay sources do not render unapproved raw icon glyphs", () => {
  // The allowlist itself is the in-repo contract (design/parity docs were
  // removed). Every entry must still be live — a stale entry means the glyph
  // moved or disappeared and the allowlist no longer describes reality.
  for (const entry of RAW_ICON_SOURCE_ALLOWLIST) {
    const source = readProductionFile(entry.file);
    assert.ok(entry.contract.length > 0, `${entry.file} allowlist entry names a contract`);
    assert.ok(
      source.split("\n").some((line) => line.includes(entry.glyph) && entry.context.test(line)),
      `stale allowlist entry: ${entry.file} no longer contains ${entry.glyph} in context ${entry.context}`,
    );
  }

  const scannedFiles = [
    ...listProductionFiles("components/chrome"),
    ...listProductionFiles("components/panels"),
    ...listProductionFiles("components/overlays"),
    "lib/omnibox.ts",
  ].filter((file) => file.endsWith(".tsx") || file === "lib/omnibox.ts");
  const rawIconGlyphs = /[⌘●▶◆▸▾⎘◇◈⌁▤⊞⇄◫▥Σ⊟⊙◉✓•↵]/gu;
  const violations: string[] = [];

  for (const file of scannedFiles) {
    const source = readProductionFile(file);
    if (source.includes("GLYPH.")) {
      violations.push(`${file}: contains GLYPH.* raw icon proxy`);
    }

    for (const match of source.matchAll(rawIconGlyphs)) {
      const before = source.slice(0, match.index);
      const line = before.split("\n").length;
      const lineText = source.split("\n")[line - 1] ?? "";
      const glyph = match[0];
      // Comments never render — the contract is about user-visible glyphs.
      if (/^\s*(\/\/|\*|\/\*)/.test(lineText)) continue;
      const allowed = RAW_ICON_SOURCE_ALLOWLIST.some((entry) => (
        entry.file === file
        && entry.glyph === glyph
        && entry.context.test(lineText)
      ));
      if (!allowed) {
        violations.push(`${file}:${line}: ${glyph}`);
      }
    }
  }

  assert.deepEqual(violations, []);
});

test("V1 representative icon controls expose accessible names or titles", () => {
  const fileHeader = readProductionFile("components/chrome/FileHeader.tsx");
  const panel = readProductionFile("components/panels/Panel.tsx");
  const panelHeaderMenu = readProductionFile("components/panels/PanelHeaderMenu.tsx");
  const mosaic = readProductionFile("components/MosaicWorkspace.tsx");
  const debugToolbar = readProductionFile("components/chrome/DebugToolbar.tsx");
  const app = readProductionFile("app/page.tsx");

  assert.match(fileHeader, /className="tcld-command-icon"[\s\S]*aria-label="Command Palette"/);
  assert.match(panel, /aria-label=\{tab\.label\}/);
  assert.match(panel, /aria-label=\{`Close \$\{tab\.label\}`\}/);
  // Maximize/restore moved from a standalone icon button into the unified
  // panel-header dropdown: the Panel builds the labelled menu item and
  // PanelHeaderMenu renders it as a role="menuitem" whose accessible name is
  // that label text.
  assert.match(panel, /label: expanded \? "Restore panel" : "Maximize panel"/);
  assert.match(panelHeaderMenu, /role="menuitem"[\s\S]*\{item\.label\}/);
  assert.match(panelHeaderMenu, /aria-label="Panel actions"/);
  // MosaicWorkspace no longer renders inline header-action buttons (the
  // unified PanelHeaderMenu owns panel-header chrome); its remaining icon
  // controls are the workspace flyout toggles, which stay labelled.
  assert.doesNotMatch(mosaic, /function PanelHeaderAction\(/);
  assert.match(mosaic, /aria-label="Open flyout"/);
  assert.match(mosaic, /aria-label="Close flyout"/);
  assert.match(debugToolbar, /title=\{shortcut \? `\$\{label\} \(\$\{shortcut\}\)` : label\}/);
  assert.match(app, /preferenceSectionIcons/);
  assert.match(app, /<Icon size=\{13\} aria-hidden/);
  assert.match(app, /data-run-config-action="new"[\s\S]*<Plus size=\{13\} aria-hidden/);
});

test("V3 top bar open controls and read-only dirty-state deviation stay wired to production paths", () => {
  const fileHeader = readProductionFile("components/chrome/FileHeader.tsx");
  const storeData = readProductionFile("lib/store-data.ts");
  const deviations = new Map(DESIGN_DEVIATIONS.map((entry) => [entry.id, entry]));
  const openFileWorkspace = DESIGN_PARITY_STATES.find((state) => state.id === "open-file-workspace");

  assert.match(fileHeader, /data-testid="topbar-empty-open-file"[\s\S]*onClick=\{onOpenFile\}/);
  assert.match(fileHeader, /No file loaded/);
  assert.match(fileHeader, /Open a \.tbc to begin/);
  assert.match(fileHeader, /data-testid="topbar-open-another-file"[\s\S]*onClick=\{onOpenFile\}/);
  assert.match(fileHeader, /<FilePlus2 size=\{13\} aria-hidden \/>/);
  assert.match(fileHeader, /data-document-dirty="false"/);
  assert.doesNotMatch(fileHeader, /tcld-tab-dirty-dot/);
  assert.match(storeData, /readOnly:\s*true/);

  const deviation = deviations.get("top-tabs-readonly-dirty-state");
  assert.equal(deviation?.designSurface, "tab dirty dot");
  assert.equal(deviation?.productionSurface, "read-only loaded document tabs");
  assert.ok(openFileWorkspace?.deviations.includes("top-tabs-readonly-dirty-state"));
});

test("V1 maps design ok semantics to production success tokens without undefined CSS variables", () => {
  const globals = readProductionFile("app/globals.css");
  const definedGlobals = new Set([...globals.matchAll(/--([A-Za-z0-9_-]+)\s*:/g)].map((match) => match[1]));
  const referencedGlobals = new Set([...globals.matchAll(/var\(--([A-Za-z0-9_-]+)/g)].map((match) => match[1]));
  const missingGlobals = [...referencedGlobals].filter((token) => !definedGlobals.has(token)).sort();

  assert.deepEqual(missingGlobals, []);
  assert.match(globals, /--ok:\s*var\(--color-tcld-success\)/);
  assert.match(globals, /--success:\s*var\(--color-tcld-success\)/);
  assert.equal(T.success, "var(--color-tcld-success)");
  assert.equal(T.ok, "var(--ok)");

  for (const themeId of Object.keys(DESIGN_THEME_TOKENS)) {
    const colors = THEMES[themeId as keyof typeof DESIGN_THEME_TOKENS].colors;
    assert.equal(colors["--color-tcld-success"], DESIGN_THEME_TOKENS[themeId as keyof typeof DESIGN_THEME_TOKENS].tokens.accent);
    assert.equal(colors["--ok"], "var(--color-tcld-success)");
    assert.equal(colors["--success"], "var(--color-tcld-success)");
    assert.equal(colors["--border-focus"], "var(--color-tcld-border-focus)");
    assert.equal(colors["--bg-highlight"], "var(--color-tcld-bg-highlight)");
    assert.equal(colors["--group"], "var(--color-tcld-group)");
  }
});

test("V9 deviation inventory is exact, non-vague, and covers every mapped deviation once", () => {
  const audit = auditDesignDeviationInventory(DESIGN_DEVIATIONS, {
    requiredDeviationIds: [...new Set(DESIGN_PARITY_STATES.flatMap((state) => state.deviations))],
    mappedDeviationIds: [
      ...DESIGN_VIEW_MAPPINGS.flatMap((mapping) => mapping.deviationId ? [mapping.deviationId] : []),
      ...DESIGN_CONTROL_MAPPINGS.flatMap((mapping) => mapping.deviationId ? [mapping.deviationId] : []),
    ],
    mappedDesignSurfaces: DESIGN_VIEW_MAPPINGS.flatMap((mapping) => mapping.deviationId
      ? [{ deviationId: mapping.deviationId, designSurface: mapping.designSurface }]
      : []),
  });

  assert.deepEqual(audit.errors, []);
  assert.equal(new Set(DESIGN_DEVIATIONS.map((entry) => entry.id)).size, DESIGN_DEVIATIONS.length);
  assert.ok(
    DESIGN_DEVIATIONS.every((entry) => entry.verificationIds.every((id) => /^V(?:[1-9]|10)$/.test(id))),
    "every deviation must name locked verification ids",
  );
});

test("V4 adaptive tab fitting follows the design demotion order", () => {
  const tabs = [
    { label: "Disassembly", active: true, badge: "i" },
    { label: "Symbols", active: false, badge: "l" },
    { label: "Source View", active: false, badge: "d" },
    { label: "Inspector", active: false, badge: null },
  ];
  // Derive the strip's reserved header width from the production module
  // (adaptiveTabsHardMinimum over zero tabs is exactly that constant), so
  // this test tracks the real actions-cluster reservation instead of
  // hardcoding a stale copy of it.
  const headerPadding = adaptiveTabsHardMinimum([]);
  const fullWidth = tabs.reduce((sum, tab) => sum + estimateAdaptiveTabWidth(tab, "full"), headerPadding);
  assert.deepEqual(fitAdaptiveTabs(tabs, fullWidth), ["full", "full", "full", "full"]);

  const oneInactiveDemotion = fullWidth
    - estimateAdaptiveTabWidth(tabs[1], "full")
    + estimateAdaptiveTabWidth(tabs[1], "iconCount");
  assert.deepEqual(fitAdaptiveTabs(tabs, oneInactiveDemotion), ["full", "iconCount", "full", "full"]);

  const inactiveIconOnlyWidth = headerPadding
    + estimateAdaptiveTabWidth(tabs[0], "full")
    + tabs.slice(1).reduce((sum, tab) => sum + estimateAdaptiveTabWidth(tab, "iconOnly"), 0);
  assert.deepEqual(fitAdaptiveTabs(tabs, inactiveIconOnlyWidth), ["full", "iconOnly", "iconOnly", "iconCount"]);

  const activeEllipsisWidth = headerPadding
    + estimateAdaptiveTabWidth(tabs[0], "ellipsis")
    + tabs.slice(1).reduce((sum, tab) => sum + estimateAdaptiveTabWidth(tab, "iconOnly"), 0);
  assert.deepEqual(fitAdaptiveTabs(tabs, activeEllipsisWidth), ["ellipsis", "iconOnly", "iconOnly", "iconOnly"]);
  assert.equal(renderAdaptiveTabLabel("Disassembly", "ellipsis"), "Disass…");
  assert.deepEqual(fitAdaptiveTabs(tabs, 1), ["iconOnly", "iconOnly", "iconOnly", "iconOnly"]);
});

test("V4 adaptive tab badges are production badges, not shortcut key hints", () => {
  const panel = readProductionFile("components/panels/Panel.tsx");
  const tabBadge = readProductionFile("components/primitives/TabBadge.tsx");
  const mosaic = readProductionFile("components/MosaicWorkspace.tsx");

  // The badge markup moved into the TabBadge primitive: Panel feeds it
  // tab.badge (never the key hint), and TabBadge renders the value inside
  // the data-tab-badge marker element.
  assert.match(panel, /<TabBadge value=\{tab\.badge\}/);
  assert.match(tabBadge, /data-tab-badge="true"[\s\S]*\{value\}/);
  assert.doesNotMatch(tabBadge, /keyHint/);
  assert.match(panel, /data-tab-key-hint=\{tab\.keyHint/);
  assert.match(panel, /badge: tab\.badge \?\? null/);
  assert.match(panel, /Math\.max\(0, tabStripWidth - 4\)/);
  assert.doesNotMatch(panel, /badge: tab\.keyHint/);
  assert.doesNotMatch(panel, /<TabBadge value=\{tab\.keyHint/);
  assert.match(mosaic, /function tabBadgeForView/);
  assert.match(mosaic, /tabBadgeForView\(view, store\.data, debug\)/);
  assert.match(mosaic, /badge: tabBadgeMap\[view\]/);
});
