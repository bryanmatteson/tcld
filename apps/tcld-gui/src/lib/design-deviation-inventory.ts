export type DesignParityStatus = "exact" | "adapted" | "omitted";

export interface DesignDeviationEntry {
  id: string;
  category: "shell" | "workspace" | "views" | "debug" | "detached-windows";
  designFile: string;
  designSurface: string;
  productionFile: string;
  productionSurface: string;
  parityStatus: DesignParityStatus;
  justification: string;
  implementationEvidence: string;
  verificationIds: string[];
}

export const DESIGN_DEVIATIONS: DesignDeviationEntry[] = [
  {
    id: "shell-launch-debug-secondary-cta",
    category: "shell",
    designFile: "tcld-welcome.jsx",
    designSurface: "Launch Debug secondary CTA",
    productionFile: "components/WelcomeScreen.tsx",
    productionSurface: "welcome primary action set",
    parityStatus: "omitted",
    justification: "Debug launch remains in the open-file toolbar and native Run menu, so the welcome screen does not repeat it.",
    implementationEvidence: "WelcomeScreen renders file, example, drop, and recent-file actions. MainToolbar and the native menu own debug launch.",
    verificationIds: ["V3", "V10"],
  },
  {
    // Historical id (the CTA shipped disabled before lib/examples.ts landed);
    // kept stable because the parity manifest and welcome parity state
    // reference it. The remaining adaptation is the example source: bundled
    // inline programs instead of the design's filesystem example browser.
    id: "welcome-browse-examples-disabled",
    category: "shell",
    designFile: "tcld-welcome.jsx",
    designSurface: "Browse examples secondary CTA",
    productionFile: "components/WelcomeScreen.tsx",
    productionSurface: "Browse examples dialog backed by bundled inline examples",
    parityStatus: "adapted",
    justification: "Examples are bundled inline and loaded through load_text, avoiding a filesystem plugin and separate resource bundle.",
    implementationEvidence: "BrowseExamplesDialog passes the selected source through store.loadText and load_text.",
    verificationIds: ["V3", "V9"],
  },
  {
    id: "top-tabs-readonly-dirty-state",
    category: "shell",
    designFile: "tcld-welcome.jsx",
    designSurface: "tab dirty dot",
    productionFile: "components/chrome/FileHeader.tsx",
    productionSurface: "read-only loaded document tabs",
    parityStatus: "omitted",
    justification: "Loaded bytecode documents are analysis inputs and do not expose an editable dirty state in the current session model.",
    implementationEvidence: "SessionDocument marks loaded documents read-only, and FileHeader renders data-document-dirty=false without a tab dirty-dot element.",
    verificationIds: ["V3", "V9"],
  },
  {
    id: "shell-native-window-controls",
    category: "shell",
    designFile: "tcld-chrome.jsx",
    designSurface: "reference window controls",
    productionFile: "apps/tcld-tauri/src/app.rs",
    productionSurface: "native Tauri window chrome",
    parityStatus: "omitted",
    justification: "The packaged application uses the native window frame.",
    implementationEvidence: "Application components do not render a simulated operating-system frame.",
    verificationIds: ["V3", "V10"],
  },
  {
    id: "workspace-drag-resize",
    category: "workspace",
    designFile: "tcld-panels.jsx",
    designSurface: "drag-resize implementation",
    productionFile: "components/MosaicWorkspace.tsx",
    productionSurface: "react-mosaic split and drag behavior",
    parityStatus: "adapted",
    justification: "The visual treatment uses the existing Mosaic rules for panel splitting, dragging, and closing the final tab.",
    implementationEvidence: "MosaicWorkspace uses Mosaic, DndContext, stackViewIntoLeaf, splitViewIntoLeaf, and minSize={220}.",
    verificationIds: ["V4", "V10"],
  },
  {
    id: "workspace-layout-saved-menu",
    category: "workspace",
    designFile: "tcld-welcome.jsx",
    designSurface: "Saved layout popover",
    productionFile: "lib/layout-storage.ts",
    productionSurface: "local saved layout persistence",
    parityStatus: "adapted",
    justification: "Saved layouts use the existing local persistence contract.",
    implementationEvidence: "MosaicWorkspace exposes layoutDirty/saveLayout, while layout-storage owns serialized layout state.",
    verificationIds: ["V4", "V10"],
  },
  {
    id: "actionbar-detached-run-config",
    category: "shell",
    designFile: "tcld-welcome.jsx",
    designSurface: "ActionBar inline run-config popover",
    productionFile: "app/page.tsx",
    productionSurface: "detached Run Configuration window",
    parityStatus: "adapted",
    justification: "The product contract keeps full run-configuration editing in a separate native Tauri window instead of embedding an editor in the main workspace action bar.",
    implementationEvidence: "MainToolbar renders Configure as a compact action-bar control that executes run-configuration:open-window, and ExplorerApp routes editing through openRunConfigurationWindow plus run-configuration:edit.",
    verificationIds: ["V3", "V7", "V10"],
  },
  {
    id: "views-disasm-id",
    category: "views",
    designFile: "tcld-views.jsx",
    designSurface: "view-disassembly / disasm view id",
    productionFile: "components/panels/InstructionPanel.tsx",
    productionSurface: "instructions view id",
    parityStatus: "adapted",
    justification: "The disasm design id maps to the existing instructions view without changing bytecode data.",
    implementationEvidence: "DESIGN_VIEW_MAPPINGS maps disasm to instructions and the panel consumes TcldData.instructions.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "views-inst-id",
    category: "views",
    designFile: "tcld-views.jsx",
    designSurface: "view-instructions / inst view id",
    productionFile: "components/panels/InstructionPanel.tsx",
    productionSurface: "instructions view id with flat presentation",
    parityStatus: "adapted",
    justification: "The disasm and inst design ids share one instructions panel with distinct presentations.",
    implementationEvidence: "DESIGN_VIEW_MAPPINGS maps inst to instructions with productionPresentation flat, and InstructionPanel renders data-instruction-presentation=flat from loaded TcldData.instructions.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "instructions-byte-view-backend-facts",
    category: "views",
    designFile: "tcld-workspace.jsx",
    designSurface: "view-instructions / byte view affordance",
    productionFile: "components/panels/InstructionPanel.tsx",
    productionSurface: "disabled byte view affordance",
    parityStatus: "adapted",
    justification: "Instruction data does not include raw byte ranges or bytes, so byte view remains disabled.",
    implementationEvidence: "InstructionPanel renders byte view only as disabled with aria-disabled and a title explaining that raw byte facts are unavailable.",
    verificationIds: ["V5", "V9", "V10"],
  },
  {
    id: "views-symbols-id",
    category: "views",
    designFile: "tcld-views.jsx",
    designSurface: "view-symbols / symbols view id",
    productionFile: "components/panels/LiteralsPanel.tsx",
    productionSurface: "literals view id with Symbols presentation",
    parityStatus: "adapted",
    justification: "Symbols uses the existing literals view id and derives namespace, procedure, and variable rows from loaded facts.",
    implementationEvidence: "DESIGN_VIEW_MAPPINGS maps symbols to literals with views-symbols-id, buildSymbolsPanelModel derives symbol rows from TcldData source-map facts, and the panel renders a deterministic Symbols unavailable state with a secondary Literals substitute when symbol facts are absent.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "views-inspector-id",
    category: "views",
    designFile: "tcld-views.jsx",
    designSurface: "view-inspector / inspector view id",
    productionFile: "components/panels/DetailPanel.tsx",
    productionSurface: "detail view id",
    parityStatus: "adapted",
    justification: "The Detail panel owns instruction inspection, so Inspector maps to the existing route.",
    implementationEvidence: "DESIGN_VIEW_MAPPINGS maps inspector to detail and the detail panel reads selected TcldData instruction facts.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "views-issues-id",
    category: "views",
    designFile: "tcld-views.jsx",
    designSurface: "view-issues / issues view id",
    productionFile: "components/panels/DiagnosticsPanel.tsx",
    productionSurface: "diagnostics view id",
    parityStatus: "adapted",
    justification: "Issues maps to the Diagnostics panel, which owns diagnostic facts.",
    implementationEvidence: "DESIGN_VIEW_MAPPINGS maps issues to diagnostics and DiagnosticsPanel consumes diagnostics/source-map model rows.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "views-flow-id",
    category: "views",
    designFile: "tcld-flow.jsx",
    designSurface: "view-flow / flow view",
    productionFile: "components/panels/GraphPanel.tsx",
    productionSurface: "graph view id",
    parityStatus: "adapted",
    justification: "Flow maps to the graph panel, where graph-model owns control-flow facts.",
    implementationEvidence: "DESIGN_VIEW_MAPPINGS maps flow to graph and GraphPanel consumes buildGraphViewModel.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "flow-source-snippet-grid",
    category: "views",
    designFile: "tcld-flow.jsx",
    designSurface: "view-flow / Source mode",
    productionFile: "components/panels/GraphPanel.tsx",
    productionSurface: "flow source-snippet grid",
    parityStatus: "adapted",
    justification: "Source mode renders a flow-scoped snippet grid from source maps and command anchors.",
    implementationEvidence: "GraphPanel renders source-snippet nodes from buildGraphViewModel with block roles and cursor targets.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "views-trace-id",
    category: "views",
    designFile: "tcld-trace.jsx",
    designSurface: "view-trace / trace view",
    productionFile: "components/panels/ProvenancePanel.tsx",
    productionSurface: "trace command list and pass-chain detail",
    parityStatus: "adapted",
    justification: "Trace uses a command list and detail view backed by source maps and diagnostics. Per-command pass chains derive from DiagnosticsReport.passDecisions.",
    implementationEvidence: "DESIGN_VIEW_MAPPINGS maps trace to provenance, and ProvenancePanel consumes buildTracePanelModel.",
    verificationIds: ["V5", "V10"],
  },
  {
    id: "debug-sparklines",
    category: "debug",
    designFile: "tcld-debug.jsx",
    designSurface: "variable sparklines",
    productionFile: "components/panels/VariablesPanel.tsx",
    productionSurface: "optional visual sparklines preference",
    parityStatus: "adapted",
    justification: "Sparklines follow the user preference; variable values remain sourced from debug-store.",
    implementationEvidence: "VariablesPanel uses --variable-sparklines and renders debug-store variable rows.",
    verificationIds: ["V6", "V10"],
  },
  {
    id: "debug-breakpoint-controls",
    category: "debug",
    designFile: "tcld-debug.jsx",
    designSurface: "breakpoint toggle plus condition plus hits",
    productionFile: "components/panels/BreakpointsPanel.tsx",
    productionSurface: "breakpoint cards",
    parityStatus: "adapted",
    justification: "Condition, enabled state, and hit count appear when the backend provides them.",
    implementationEvidence: "BreakpointsPanel renders breakpoint.enabled, breakpoint.condition, and breakpoint.hitCount from debug-store.",
    verificationIds: ["V6", "V10"],
  },
  {
    id: "preferences-editor-sections",
    category: "detached-windows",
    designFile: "tcld-prefs.jsx",
    designSurface: "Editor/Keybindings/Decompiler/Debugger/Telemetry/Advanced sections",
    productionFile: "app/page.tsx",
    productionSurface: "PreferencesWindowApp sidebar",
    parityStatus: "adapted",
    justification: "Sections without implemented settings remain informational or disabled.",
    implementationEvidence: "PreferencesWindowApp renders section names and wires Appearance settings through saveSettingsWithTauriFallback.",
    verificationIds: ["V7", "V10"],
  },
  {
    id: "run-config-model",
    category: "detached-windows",
    designFile: "tcld-prefs.jsx",
    designSurface: "Run Configuration reference fields",
    productionFile: "app/page.tsx",
    productionSurface: "RunConfigurationWindowApp",
    parityStatus: "adapted",
    justification: "The window adopts design layout/chrome while preserving the existing run profile model and event contract.",
    implementationEvidence: "RunConfigurationWindowApp continues using createRunConfigurationStore and run-configuration save/cancel/delete/import/export events.",
    verificationIds: ["V7", "V10"],
  },
];

export function findDeviation(
  category: DesignDeviationEntry["category"],
  designFile: string,
  designSurface: string,
): DesignDeviationEntry | undefined {
  return DESIGN_DEVIATIONS.find((entry) =>
    entry.category === category
    && entry.designFile === designFile
    && entry.designSurface === designSurface
  );
}

export function auditDesignDeviationInventory(
  entries: DesignDeviationEntry[],
  options: {
    requiredDeviationIds?: string[];
    mappedDeviationIds?: string[];
    mappedDesignSurfaces?: Array<{ deviationId: string; designSurface: string }>;
  } = {},
): { errors: string[] } {
  const errors: string[] = [];
  const vague = /\b(tbd|todo|later|maybe|unknown|n\/a)\b/i;
  const ids = new Set<string>();
  for (const entry of entries) {
    if (ids.has(entry.id)) {
      errors.push(`${entry.id} is duplicated`);
    }
    ids.add(entry.id);
    for (const field of [
      "designFile",
      "designSurface",
      "productionFile",
      "productionSurface",
      "justification",
      "implementationEvidence",
    ] as const) {
      if (!entry[field] || vague.test(entry[field])) {
        errors.push(`${entry.id}.${field} is missing or vague`);
      }
    }
    if (!["exact", "adapted", "omitted"].includes(entry.parityStatus)) {
      errors.push(`${entry.id}.parityStatus is invalid`);
    }
    if (entry.verificationIds.length === 0) {
      errors.push(`${entry.id}.verificationIds is empty`);
    }
    if (/enabled without implementation/i.test(entry.justification) || /enabled without implementation/i.test(entry.implementationEvidence)) {
      errors.push(`${entry.id} describes controls as enabled without implementation`);
    }
  }
  for (const id of options.requiredDeviationIds ?? []) {
    if (!ids.has(id)) {
      errors.push(`${id} is required by a parity state but missing from the deviation inventory`);
    }
  }
  for (const id of options.mappedDeviationIds ?? []) {
    if (!ids.has(id)) {
      errors.push(`${id} is required by a mapped design surface but missing from the deviation inventory`);
    }
  }
  for (const { deviationId, designSurface } of options.mappedDesignSurfaces ?? []) {
    const entry = entries.find((candidate) => candidate.id === deviationId);
    if (!entry) {
      continue;
    }
    if (!entry.designSurface.includes(designSurface)) {
      errors.push(`${deviationId}.designSurface must name mapped design surface ${designSurface}`);
    }
  }
  return { errors };
}
