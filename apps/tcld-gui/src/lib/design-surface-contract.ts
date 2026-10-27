import type { ViewId } from "@/types";

export interface DesignSurfaceGroup {
  designFiles: string[];
  productionFiles: string[];
  productionMarkers: string[];
}

export interface DesignViewMapping {
  designId: string;
  designSource: string;
  designSurface: string;
  designIconRole: string;
  productionViewId?: ViewId;
  productionFile?: string;
  productionPresentation?: string;
  deviationId?: string;
}

export interface DesignParityState {
  id: string;
  designSource: string;
  artboard: string;
  designArtboardId: string;
  productionRoute: string;
  productionSelector: string;
  deviations: string[];
  artifacts: {
    design: string;
    production: string;
  };
}

export interface DesignControlMapping {
  id: string;
  designSource: string;
  designSurface: string;
  productionFile: string;
  productionAction: string;
  productionModel: string;
  deviationId?: string;
}

export interface DesignPreferenceSectionMapping {
  label: string;
  slug: string;
  group: "Workspace" | "Tools" | "System";
  designSource: "tcld-prefs.jsx";
  productionRoute: string;
  productionSelector: string;
  productionBehavior: string;
}

export const DESIGN_SURFACES: Record<string, DesignSurfaceGroup> = {
  shell: {
    designFiles: ["tcld-chrome.jsx", "tcld-welcome.jsx"],
    productionFiles: [
      "app/page.tsx",
      "components/WelcomeScreen.tsx",
      "components/chrome/FileHeader.tsx",
      "components/chrome/MainToolbar.tsx",
      "components/chrome/DebugToolbar.tsx",
      "components/chrome/StatusBar.tsx",
    ],
    productionMarkers: [
      "data-design-surface=\"welcome-screen\"",
      "data-design-surface=\"top-tab-strip\"",
      "data-design-surface=\"action-bar\"",
      "data-design-surface=\"debug-paused-overlay\"",
      "data-design-surface=\"status-bar\"",
    ],
  },
  workspace: {
    designFiles: ["tcld-workspace.jsx", "tcld-panels.jsx", "tcld-app.jsx"],
    productionFiles: [
      "components/MosaicWorkspace.tsx",
      "components/panels/Panel.tsx",
      "lib/default-layouts.ts",
      "lib/layout-storage.ts",
    ],
    productionMarkers: [
      "data-design-surface=\"workspace-mosaic\"",
      "data-design-surface=\"singleton-tab\"",
      "data-design-surface=\"panel-header\"",
    ],
  },
  views: {
    designFiles: ["tcld-views.jsx", "tcld-flow.jsx", "tcld-trace.jsx", "tcld-console.jsx"],
    productionFiles: ["components/panels"],
    productionMarkers: [
      "data-design-surface=\"view-disassembly\"",
      "data-design-surface=\"view-instructions\"",
      "data-design-surface=\"view-symbols\"",
      "data-design-surface=\"view-source\"",
      "data-design-surface=\"view-compare\"",
      "data-design-surface=\"view-inspector\"",
      "data-design-surface=\"view-issues\"",
      "data-design-surface=\"view-flow\"",
      "data-design-surface=\"view-trace\"",
      "data-design-surface=\"view-console\"",
    ],
  },
  debug: {
    designFiles: ["tcld-debug.jsx", "tcld-chrome.jsx"],
    productionFiles: [
      "components/chrome/DebugToolbar.tsx",
      "components/panels/VariablesPanel.tsx",
      "components/panels/StackPanel.tsx",
      "components/panels/BreakpointsPanel.tsx",
      "components/panels/WatchpointsPanel.tsx",
    ],
    productionMarkers: [
      "data-design-surface=\"debug-paused-overlay\"",
      "data-design-surface=\"debug-variables\"",
      "data-design-surface=\"debug-call-stack\"",
      "data-design-surface=\"debug-breakpoints\"",
      "data-design-surface=\"debug-watchpoints\"",
    ],
  },
  "detached-windows": {
    designFiles: ["tcld-prefs.jsx"],
    productionFiles: ["app/page.tsx", "lib/tauri-api.ts", "../apps/tcld-tauri/src/main.rs"],
    productionMarkers: [
      "data-design-surface=\"preferences-window\"",
      "data-design-surface=\"run-configuration-window\"",
    ],
  },
};

export const DESIGN_VIEW_MAPPINGS: DesignViewMapping[] = [
  { designId: "disasm", designSource: "tcld-views.jsx", designSurface: "view-disassembly", designIconRole: "cpu", productionViewId: "instructions", productionFile: "components/panels/InstructionPanel.tsx", productionPresentation: "ir", deviationId: "views-disasm-id" },
  { designId: "symbols", designSource: "tcld-views.jsx", designSurface: "view-symbols", designIconRole: "list", productionViewId: "literals", productionFile: "components/panels/LiteralsPanel.tsx", deviationId: "views-symbols-id" },
  { designId: "source", designSource: "tcld-views.jsx", designSurface: "view-source", designIconRole: "file", productionViewId: "decompile", productionFile: "components/panels/DecompilePanel.tsx", productionPresentation: "reconstructed Tcl source from TcldData.decompileLines" },
  { designId: "inst", designSource: "tcld-views.jsx", designSurface: "view-instructions", designIconRole: "list", productionViewId: "instructions", productionFile: "components/panels/InstructionPanel.tsx", productionPresentation: "instructions", deviationId: "views-inst-id" },
  { designId: "compare", designSource: "tcld-views.jsx", designSurface: "view-compare", designIconRole: "cpu", productionViewId: "comparison", productionFile: "components/panels/ComparisonPanel.tsx" },
  { designId: "inspector", designSource: "tcld-views.jsx", designSurface: "view-inspector", designIconRole: "eye", productionViewId: "detail", productionFile: "components/panels/DetailPanel.tsx", deviationId: "views-inspector-id" },
  { designId: "issues", designSource: "tcld-views.jsx", designSurface: "view-issues", designIconRole: "alert", productionViewId: "diagnostics", productionFile: "components/panels/DiagnosticsPanel.tsx", deviationId: "views-issues-id" },
  { designId: "flow", designSource: "tcld-flow.jsx", designSurface: "view-flow", designIconRole: "flow", productionViewId: "graph", productionFile: "components/panels/GraphPanel.tsx", deviationId: "views-flow-id" },
  { designId: "trace", designSource: "tcld-trace.jsx", designSurface: "view-trace", designIconRole: "trace", productionViewId: "provenance", productionFile: "components/panels/ProvenancePanel.tsx", deviationId: "views-trace-id" },
  { designId: "console", designSource: "tcld-console.jsx", designSurface: "view-console", designIconRole: "terminal", productionViewId: "console", productionFile: "components/panels/ConsolePanel.tsx" },
  { designId: "variables", designSource: "tcld-debug.jsx", designSurface: "debug-variables", designIconRole: "stack", productionViewId: "variables", productionFile: "components/panels/VariablesPanel.tsx" },
  { designId: "stack", designSource: "tcld-debug.jsx", designSurface: "debug-call-stack", designIconRole: "layers", productionViewId: "stack", productionFile: "components/panels/StackPanel.tsx" },
  { designId: "breakpoints", designSource: "tcld-debug.jsx", designSurface: "debug-breakpoints", designIconRole: "circle", productionViewId: "breakpoints", productionFile: "components/panels/BreakpointsPanel.tsx" },
  { designId: "watchpoints", designSource: "tcld-debug.jsx", designSurface: "debug-watchpoints", designIconRole: "circle", productionViewId: "watchpoints", productionFile: "components/panels/WatchpointsPanel.tsx" },
];

export const DESIGN_CONTROL_MAPPINGS: DesignControlMapping[] = [
  { id: "welcome.open-file", designSource: "tcld-welcome.jsx", designSurface: "Open File CTA", productionFile: "components/WelcomeScreen.tsx", productionAction: "open", productionModel: "useTcldStore.loadFile" },
  { id: "welcome.file-drop", designSource: "tcld-welcome.jsx", designSurface: "Drop zone", productionFile: "app/page.tsx", productionAction: "drop:file", productionModel: "resolveDroppedFile" },
  { id: "welcome.recents-open", designSource: "tcld-welcome.jsx", designSurface: "Recent file row", productionFile: "components/WelcomeScreen.tsx", productionAction: "recent:open", productionModel: "recent-files" },
  { id: "welcome.recents-clear", designSource: "tcld-welcome.jsx", designSurface: "Clear recents", productionFile: "components/WelcomeScreen.tsx", productionAction: "recent:clear", productionModel: "recent-files" },
  { id: "workspace.layout-preset", designSource: "tcld-app.jsx", designSurface: "Layout preset picker", productionFile: "lib/default-layouts.ts", productionAction: "layout:<name>", productionModel: "LAYOUT_PRESETS" },
  { id: "workspace.save-layout", designSource: "tcld-welcome.jsx", designSurface: "Save layout", productionFile: "lib/layout-storage.ts", productionAction: "layout:save", productionModel: "saveCurrentLayout", deviationId: "workspace-layout-saved-menu" },
  { id: "workspace.add-singleton-view", designSource: "tcld-views.jsx", designSurface: "Add view", productionFile: "hooks/use-explorer.ts", productionAction: "panel:<view>", productionModel: "addView" },
  { id: "workspace.swap-singleton-view", designSource: "tcld-views.jsx", designSurface: "Swap view", productionFile: "components/chrome/PanelPicker.tsx", productionAction: "panel:swap", productionModel: "stack/split workspace helpers" },
  { id: "workspace.close-last-tab", designSource: "tcld-views.jsx", designSurface: "Close tab", productionFile: "hooks/use-explorer.ts", productionAction: "panel:close", productionModel: "removeView" },
  { id: "workspace.drag-reorder", designSource: "tcld-panels.jsx", designSurface: "Drag reorder", productionFile: "components/MosaicWorkspace.tsx", productionAction: "workspace:drag", productionModel: "react-mosaic+dnd-kit", deviationId: "workspace-drag-resize" },
  { id: "workspace.resize-min-width", designSource: "tcld-panels.jsx", designSurface: "Resize gutter", productionFile: "components/MosaicWorkspace.tsx", productionAction: "workspace:resize", productionModel: "Mosaic minSize 220", deviationId: "workspace-drag-resize" },
  { id: "debug.run", designSource: "tcld-welcome.jsx", designSurface: "Run button", productionFile: "app/page.tsx", productionAction: "debug:launch-profile", productionModel: "debug-store.launch" },
  { id: "debug.pause", designSource: "tcld-debug.jsx", designSurface: "Pause", productionFile: "components/chrome/DebugToolbar.tsx", productionAction: "debug:pause", productionModel: "debug-store.pause" },
  { id: "debug.step", designSource: "tcld-debug.jsx", designSurface: "Step", productionFile: "components/chrome/DebugToolbar.tsx", productionAction: "debug:step_into", productionModel: "debug-store.stepInto" },
  { id: "debug.continue", designSource: "tcld-debug.jsx", designSurface: "Continue", productionFile: "components/chrome/DebugToolbar.tsx", productionAction: "debug:continue", productionModel: "debug-store.resume" },
  { id: "debug.stop", designSource: "tcld-debug.jsx", designSurface: "Stop", productionFile: "components/chrome/DebugToolbar.tsx", productionAction: "debug:stop", productionModel: "debug-store.stop" },
  { id: "instructions.fold-filter", designSource: "tcld-workspace.jsx", designSurface: "Disassembly fold/filter", productionFile: "components/panels/InstructionPanel.tsx", productionAction: "instructions:fold-all", productionModel: "folding-model" },
  { id: "source.linked-navigation", designSource: "tcld-workspace.jsx", designSurface: "Source linked navigation", productionFile: "components/panels/DecompilePanel.tsx", productionAction: "source:linked-navigation", productionModel: "source-sync-store/location-store" },
  { id: "source.split", designSource: "tcld-views.jsx", designSurface: "Source split", productionFile: "components/MosaicWorkspace.tsx", productionAction: "source:split", productionModel: "splitViewIntoLeaf" },
  { id: "flow.detail-mode", designSource: "tcld-flow.jsx", designSurface: "Flow detail/mode", productionFile: "components/panels/GraphPanel.tsx", productionAction: "graph:detail-medium", productionModel: "graph-model", deviationId: "views-flow-id" },
  { id: "trace.timeline-select", designSource: "tcld-trace.jsx", designSurface: "Trace timeline", productionFile: "components/panels/ProvenancePanel.tsx", productionAction: "trace:select-pass", productionModel: "buildTracePanelModel", deviationId: "views-trace-id" },
  { id: "console.channel-filter", designSource: "tcld-console.jsx", designSurface: "Console channel chips", productionFile: "components/panels/ConsolePanel.tsx", productionAction: "console:filter-channel", productionModel: "debug-store.output" },
  { id: "console.repl-input", designSource: "tcld-console.jsx", designSurface: "REPL input", productionFile: "components/panels/ConsolePanel.tsx", productionAction: "debug:eval", productionModel: "debug-store.evalScript" },
  { id: "compare.selection", designSource: "tcld-workspace.jsx", designSurface: "Compare selection", productionFile: "components/panels/ComparisonPanel.tsx", productionAction: "comparison:select", productionModel: "buildComparisonModel" },
  { id: "inspector.selection", designSource: "tcld-workspace.jsx", designSurface: "Inspector selection", productionFile: "components/panels/DetailPanel.tsx", productionAction: "detail:select", productionModel: "TcldData.instructions", deviationId: "views-inspector-id" },
  { id: "issues.selection", designSource: "tcld-workspace.jsx", designSurface: "Issues selection", productionFile: "components/panels/DiagnosticsPanel.tsx", productionAction: "diagnostics:select", productionModel: "DiagnosticsReport", deviationId: "views-issues-id" },
  { id: "variables.expand-sparkline", designSource: "tcld-debug.jsx", designSurface: "Variable expansion/sparkline", productionFile: "components/panels/VariablesPanel.tsx", productionAction: "variables:expand", productionModel: "debug-store.variables", deviationId: "debug-sparklines" },
  { id: "stack.frame-selection", designSource: "tcld-debug.jsx", designSurface: "Call stack frame", productionFile: "components/panels/StackPanel.tsx", productionAction: "stack:select-frame", productionModel: "debug-store.stack" },
  { id: "breakpoints.toggle-condition-hits", designSource: "tcld-debug.jsx", designSurface: "Breakpoint toggle/condition/hits", productionFile: "components/panels/BreakpointsPanel.tsx", productionAction: "debug:toggle_breakpoint", productionModel: "debug-store.breakpoints", deviationId: "debug-breakpoint-controls" },
  { id: "watchpoints.production-backed", designSource: "tcld-debug.jsx", designSurface: "Watchpoints", productionFile: "components/panels/WatchpointsPanel.tsx", productionAction: "debug:watchpoints", productionModel: "debug-store.watchpoints" },
  { id: "preferences.theme", designSource: "tcld-prefs.jsx", designSurface: "Theme picker", productionFile: "app/page.tsx", productionAction: "appearance:theme", productionModel: "settings/theme tokens" },
  { id: "preferences.type-system", designSource: "tcld-prefs.jsx", designSurface: "Type system", productionFile: "app/page.tsx", productionAction: "appearance:type-system", productionModel: "settings/type systems" },
  { id: "preferences.density", designSource: "tcld-prefs.jsx", designSurface: "Density", productionFile: "app/page.tsx", productionAction: "appearance:density", productionModel: "settings/density scale" },
  { id: "preferences.sparklines", designSource: "tcld-prefs.jsx", designSurface: "Variable sparklines", productionFile: "app/page.tsx", productionAction: "appearance:sparklines", productionModel: "settings.showSparklines" },
  { id: "preferences.group-rails", designSource: "tcld-prefs.jsx", designSurface: "Disassembly group rails", productionFile: "app/page.tsx", productionAction: "appearance:group-rails", productionModel: "settings.groupRails" },
  { id: "run-config.save-cancel-delete", designSource: "tcld-prefs.jsx", designSurface: "Run configuration save/cancel/delete", productionFile: "app/page.tsx", productionAction: "run-configuration:save/delete/cancel", productionModel: "run-configuration-store", deviationId: "run-config-model" },
  { id: "run-config.import-export", designSource: "tcld-prefs.jsx", designSurface: "Run configuration import/export", productionFile: "app/page.tsx", productionAction: "run-configuration:import/export", productionModel: "run-configuration-store", deviationId: "run-config-model" },
];

export const DESIGN_PREFERENCE_SECTION_MAPPINGS: DesignPreferenceSectionMapping[] = [
  {
    label: "Appearance",
    slug: "appearance",
    group: "Workspace",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Appearance",
    productionSelector: "[data-parity-state=\"preferences-appearance\"]",
    productionBehavior: "Live settings for theme, type system, density, sparklines, and group rails.",
  },
  {
    label: "Editor",
    slug: "editor",
    group: "Workspace",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Editor",
    productionSelector: "[data-parity-state=\"preferences-editor\"]",
    productionBehavior: "Intentional disabled/readiness section until editor-specific settings exist.",
  },
  {
    label: "Keybindings",
    slug: "keybindings",
    group: "Workspace",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Keybindings",
    productionSelector: "[data-parity-state=\"preferences-keybindings\"]",
    productionBehavior: "Intentional disabled/readiness section; keybinding import/export remains in main command actions.",
  },
  {
    label: "Decompiler",
    slug: "decompiler",
    group: "Tools",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Decompiler",
    productionSelector: "[data-parity-state=\"preferences-decompiler\"]",
    productionBehavior: "Intentional disabled/readiness section until decompiler toggles graduate to settings.",
  },
  {
    label: "Debugger",
    slug: "debugger",
    group: "Tools",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Debugger",
    productionSelector: "[data-parity-state=\"preferences-debugger\"]",
    productionBehavior: "Intentional disabled/readiness section; launch behavior is edited in Run Configuration.",
  },
  {
    label: "Run profiles",
    slug: "run-profiles",
    group: "Tools",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Run%20profiles",
    productionSelector: "[data-parity-state=\"preferences-run-profiles\"]",
    productionBehavior: "Intentional disabled/readiness section pointing users to the detached Run Configuration window.",
  },
  {
    label: "Telemetry",
    slug: "telemetry",
    group: "System",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Telemetry",
    productionSelector: "[data-parity-state=\"preferences-telemetry\"]",
    productionBehavior: "Telemetry remains an informational section with no active controls.",
  },
  {
    label: "Advanced",
    slug: "advanced",
    group: "System",
    designSource: "tcld-prefs.jsx",
    productionRoute: "/?window=preferences&section=Advanced",
    productionSelector: "[data-parity-state=\"preferences-advanced\"]",
    productionBehavior: "Intentional disabled/readiness section for future low-level settings.",
  },
];

export const DESIGN_PARITY_STATES: DesignParityState[] = [
  {
    id: "welcome",
    designSource: "tcld-welcome.jsx",
    artboard: "WelcomeOnly",
    designArtboardId: "welcome",
    productionRoute: "/",
    productionSelector: "[data-parity-state=\"welcome\"]",
    deviations: ["shell-launch-debug-secondary-cta", "welcome-browse-examples-disabled", "shell-native-window-controls"],
    artifacts: { design: "welcome.design.png", production: "welcome.production.png" },
  },
  {
    id: "open-file-workspace",
    designSource: "tcld-workspace.jsx",
    artboard: "Workspace inspect",
    designArtboardId: "inspect",
    productionRoute: "/?parity=open-file",
    productionSelector: "[data-panel-id=\"instructions\"]",
    deviations: ["workspace-drag-resize", "workspace-layout-saved-menu", "top-tabs-readonly-dirty-state", "actionbar-detached-run-config"],
    artifacts: { design: "open-file-workspace.design.png", production: "open-file-workspace.production.png" },
  },
  {
    id: "debug-paused-workspace",
    designSource: "tcld-debug.jsx",
    artboard: "Workspace debug paused",
    designArtboardId: "debug",
    productionRoute: "/?parity=debug-paused",
    productionSelector: "[data-panel-id=\"variables\"]",
    deviations: ["debug-sparklines", "debug-breakpoint-controls"],
    artifacts: { design: "debug-paused-workspace.design.png", production: "debug-paused-workspace.production.png" },
  },
  {
    id: "flow",
    designSource: "tcld-flow.jsx",
    artboard: "Flow control graph",
    designArtboardId: "flow",
    productionRoute: "/?parity=flow",
    productionSelector: "[data-panel-id=\"graph\"]",
    deviations: ["views-flow-id", "flow-source-snippet-grid"],
    artifacts: { design: "flow.design.png", production: "flow.production.png" },
  },
  {
    id: "trace",
    designSource: "tcld-trace.jsx",
    artboard: "Trace pass timeline",
    designArtboardId: "trace",
    productionRoute: "/?parity=trace",
    productionSelector: "[data-panel-id=\"provenance\"]",
    deviations: ["views-trace-id"],
    artifacts: { design: "trace.design.png", production: "trace.production.png" },
  },
  {
    id: "console",
    designSource: "tcld-console.jsx",
    artboard: "Console channels",
    designArtboardId: "console",
    productionRoute: "/?parity=console",
    productionSelector: "[data-panel-id=\"console\"]",
    deviations: [],
    artifacts: { design: "console.design.png", production: "console.production.png" },
  },
  {
    id: "symbols",
    designSource: "tcld-workspace.jsx",
    artboard: "Symbols deep dive",
    designArtboardId: "symbols",
    productionRoute: "/?parity=symbols",
    productionSelector: "[data-panel-id=\"literals\"][data-design-surface=\"view-symbols\"]",
    deviations: ["views-symbols-id"],
    artifacts: { design: "symbols.design.png", production: "symbols.production.png" },
  },
  {
    id: "preferences-appearance",
    designSource: "tcld-prefs.jsx",
    artboard: "Preferences Appearance",
    designArtboardId: "prefs-appearance",
    productionRoute: "/?window=preferences&section=Appearance",
    productionSelector: "[data-parity-state=\"preferences-appearance\"]",
    deviations: ["preferences-editor-sections"],
    artifacts: { design: "preferences-appearance.design.png", production: "preferences-appearance.production.png" },
  },
  {
    id: "preferences-decompiler",
    designSource: "tcld-prefs.jsx",
    artboard: "Preferences Decompiler",
    designArtboardId: "prefs-decomp",
    productionRoute: "/?window=preferences&section=Decompiler",
    productionSelector: "[data-parity-state=\"preferences-decompiler\"]",
    deviations: ["preferences-editor-sections"],
    artifacts: { design: "preferences-decompiler.design.png", production: "preferences-decompiler.production.png" },
  },
  {
    id: "preferences-keybindings",
    designSource: "tcld-prefs.jsx",
    artboard: "Preferences Keybindings",
    designArtboardId: "prefs-keys",
    productionRoute: "/?window=preferences&section=Keybindings",
    productionSelector: "[data-parity-state=\"preferences-keybindings\"]",
    deviations: ["preferences-editor-sections"],
    artifacts: { design: "preferences-keybindings.design.png", production: "preferences-keybindings.production.png" },
  },
  {
    id: "run-configuration",
    designSource: "tcld-prefs.jsx",
    artboard: "Run Configuration",
    designArtboardId: "runcfg",
    productionRoute: "/?window=run-configuration",
    productionSelector: "[data-parity-state=\"run-configuration\"]",
    deviations: ["run-config-model"],
    artifacts: { design: "run-configuration.design.png", production: "run-configuration.production.png" },
  },
  {
    id: "variables-detail",
    designSource: "tcld-debug.jsx",
    artboard: "Variables detail",
    designArtboardId: "vars",
    productionRoute: "/?parity=variables",
    productionSelector: "[data-panel-id=\"variables\"]",
    deviations: ["debug-sparklines"],
    artifacts: { design: "variables-detail.design.png", production: "variables-detail.production.png" },
  },
  {
    id: "call-stack-detail",
    designSource: "tcld-debug.jsx",
    artboard: "Call Stack detail",
    designArtboardId: "stack",
    productionRoute: "/?parity=stack",
    productionSelector: "[data-panel-id=\"stack\"]",
    deviations: [],
    artifacts: { design: "call-stack-detail.design.png", production: "call-stack-detail.production.png" },
  },
  {
    id: "breakpoints-detail",
    designSource: "tcld-debug.jsx",
    artboard: "Breakpoints detail",
    designArtboardId: "bps",
    productionRoute: "/?parity=breakpoints",
    productionSelector: "[data-panel-id=\"breakpoints\"]",
    deviations: ["debug-breakpoint-controls"],
    artifacts: { design: "breakpoints-detail.design.png", production: "breakpoints-detail.production.png" },
  },
  ...(["graphite", "catppuccin", "dracula", "solarized", "tokyolight"] as const).map((themeId) => ({
    id: `theme-${themeId}-inspect`,
    designSource: "tcld-themes.jsx",
    artboard: `Theme inspect ${themeId}`,
    designArtboardId: `th-${themeId}`,
    productionRoute: `/?parity=theme-${themeId}`,
    productionSelector: `[data-panel-id="instructions"]`,
    deviations: ["workspace-drag-resize"],
    artifacts: {
      design: `theme-${themeId}-inspect.design.png`,
      production: `theme-${themeId}-inspect.production.png`,
    },
  })),
];

export function assertDesignSurfaceCoverage(group: keyof typeof DESIGN_SURFACES): void {
  const surface = DESIGN_SURFACES[group];
  if (!surface) throw new Error(`Unknown design surface group: ${String(group)}`);
  if (surface.designFiles.length === 0) throw new Error(`${String(group)} has no design files`);
  if (surface.productionFiles.length === 0) throw new Error(`${String(group)} has no production files`);
  if (surface.productionMarkers.length === 0) throw new Error(`${String(group)} has no markers`);
}
