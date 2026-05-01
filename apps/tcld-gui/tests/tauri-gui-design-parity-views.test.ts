import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { commandRegistry } from "@/lib/command-registry";
import { dispatchPanelAction, subscribePanelAction } from "@/lib/panel-action-bus";
import { DESIGN_VIEW_MAPPINGS } from "@/lib/design-surface-contract";
import { DESIGN_DEVIATIONS } from "@/lib/design-deviation-inventory";
import { DISASSEMBLY_PARITY_COLUMN_IDS } from "@/lib/settings";
import {
  buildComparisonPanelModel,
  buildDiagnosticsPanelModel,
  buildProvenancePanelModel,
  buildSourcePanelModel,
  sourceMapSelectionTarget,
} from "@/lib/panel-models";
import { searchOutput, createOutputSearchState } from "@/lib/output-search";
import { tcldDataFromLoadResult } from "@/lib/store-data";
import {
  closeProductionPage,
  createLoadResult,
  pressModShortcut,
  waitForText,
  waitForVisibleText,
  withProductionPage,
} from "./browser-production-harness.ts";

function readProductionSource(relativePath: string): string {
  return readFileSync(new URL(`../src/${relativePath}`, import.meta.url), "utf8");
}

test("V6 stable command registry and panel action bus invoke production view actions", () => {
  let foldCount = 0;
  let unfoldCount = 0;
  const unsubFold = subscribePanelAction("instructions:fold-all", () => {
    foldCount += 1;
  });
  const unsubUnfold = subscribePanelAction("source:unfold-all", () => {
    unfoldCount += 1;
  });
  try {
    assert.equal(dispatchPanelAction("instructions:fold-all"), true);
    assert.equal(dispatchPanelAction("source:unfold-all"), true);
    assert.equal(foldCount, 1);
    assert.equal(unfoldCount, 1);
    assert.equal(dispatchPanelAction("missing:action"), false);
  } finally {
    unsubFold();
    unsubUnfold();
  }

  const executed: string[] = [];
  const unregister = [
    commandRegistry.register({ id: "instructions:filter", title: "Filter Instructions", category: "Views", run: () => executed.push("instructions:filter") }),
    commandRegistry.register({ id: "source:linked-navigation", title: "Source Linked Navigation", category: "Views", run: () => executed.push("source:linked-navigation") }),
    commandRegistry.register({ id: "trace:select-pass", title: "Trace Select Pass", category: "Views", run: () => executed.push("trace:select-pass") }),
    commandRegistry.register({ id: "console:submit-repl", title: "Console Submit", category: "Views", run: () => executed.push("console:submit-repl") }),
  ];
  try {
    for (const id of ["instructions:filter", "source:linked-navigation", "trace:select-pass", "console:submit-repl"]) {
      assert.equal(commandRegistry.execute(id), true, `${id} should be callable through command-registry.ts`);
    }
    assert.deepEqual(executed, ["instructions:filter", "source:linked-navigation", "trace:select-pass", "console:submit-repl"]);
  } finally {
    for (const dispose of unregister) dispose();
  }
});

test("V6 Disassembly and Instructions designs map to shared panel with distinct grouped and flat presentations", () => {
  const disasm = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === "disasm");
  const inst = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === "inst");
  assert.equal(disasm?.productionViewId, "instructions");
  assert.equal(inst?.productionViewId, "instructions");
  assert.equal(disasm?.designSurface, "view-disassembly");
  assert.equal(inst?.designSurface, "view-instructions");
  assert.equal(disasm?.productionPresentation, "ir");
  assert.equal(inst?.productionPresentation, "instructions");

  const panel = readProductionSource("components/panels/InstructionPanel.tsx");
  const css = readProductionSource("app/globals.css");
  const rowMetrics = readProductionSource("lib/row-metrics.ts");
  const settings = readProductionSource("lib/settings.ts");

  for (const selector of [
    "data-command-group",
    "data-group-start",
    "data-selected-group",
    "data-fold-state",
    "data-folded-hidden-count",
    "data-disassembly-toolbar-action=\"fold-all\"",
    "data-disassembly-toolbar-action=\"unfold-all\"",
    "data-disassembly-folded-count",
    "data-disasm-column=\"address\"",
    "data-disasm-column=\"opcode\"",
    "data-disasm-column=\"operand\"",
    "data-disasm-column=\"hint\"",
    "data-instruction-presentation={panelMode}",
    "data-instructions-flat-header",
    "data-instructions-column-header=\"index\"",
    "data-instructions-column-header=\"address\"",
    "data-instructions-column-header=\"opcode\"",
    "data-instructions-column-header=\"operand\"",
    "data-instructions-column-header=\"hint\"",
    "data-instruction-row-mode=\"flat\"",
    "data-instructions-byte-view=\"disabled\"",
  ]) {
    assert.ok(panel.includes(selector), `missing instructions selector ${selector}`);
  }

  const byteViewDeviation = DESIGN_DEVIATIONS.find((entry) => entry.id === "instructions-byte-view-backend-facts");
  assert.equal(byteViewDeviation?.designSurface, "view-instructions / byte view affordance");
  assert.match(byteViewDeviation?.implementationEvidence ?? "", /disabled/);
  assert.doesNotMatch(panel, /data-instructions-byte-view="enabled"|hex dump|raw bytes from mock|mock byte/i);

  assert.match(css, /\.tcld-disasm-group-rail/);
  assert.match(css, /\.tcld-disasm-group-dot/);
  assert.match(css, /\.instruction-row-selected/);
  assert.match(css, /display: var\(--rail-display\)/);
  assert.match(rowMetrics, /getDisassemblyRowMetrics/);
  assert.match(settings, /DISASSEMBLY_PARITY_COLUMN_IDS/);
  assert.deepEqual([...DISASSEMBLY_PARITY_COLUMN_IDS], ["group", "address", "opcode", "args", "comment"]);
});

test("V6 Source, Trace, Compare, Inspector, and Issues selections are backed by production panel models", () => {
  const data = tcldDataFromLoadResult(createLoadResult("/tmp/model.tbc", "model.tbc") as never);
  const cursor = { scopeId: null, instIdx: 1, pc: null, group: null, blockId: null, decompileLine: null, sourceFile: null, sourceLine: 0 };

  const source = buildSourcePanelModel(data, cursor);
  assert.equal(source.available, true);
  assert.equal(source.sourceMapRows[0].selected, true);
  assert.deepEqual(sourceMapSelectionTarget(data, 1), { instIdx: 3, pc: 3, group: 1, decompileLine: 1, sourceLine: 1 });

  const provenance = buildProvenancePanelModel(data, cursor);
  assert.equal(provenance.rows[0].selected, true);
  assert.equal(provenance.rows.some((row) => row.index === 1), true);

  const comparison = buildComparisonPanelModel(data, cursor);
  assert.equal(comparison?.selection.sourceMapIndex, 0);
  assert.equal(comparison?.links[0].sourceMapIndex, 0);
  assert.equal(comparison?.rows[0].sourceMapIndex, 0);
  assert.equal(comparison?.rows[0].status, "match");
  assert.equal(comparison?.rows[0].sourceText, "set x 1");
  assert.match(comparison?.rows[0].disassemblyText ?? "", /push1/);
  assert.deepEqual(comparison?.legendCounts, { match: 2, partial: 0, stripped: 0 });
  assert.match(comparison?.reconstructed.text ?? "", /set x 1/);

  const diagnostics = buildDiagnosticsPanelModel(data, cursor);
  assert.equal(diagnostics.linkedRows.some((row) => row.kind === "approximation" && row.selected), true);
  assert.equal(diagnostics.linkedRows.some((row) => row.kind === "unsupported"), true);
  assert.equal(diagnostics.status, "issues");
  assert.deepEqual(diagnostics.issueCounts, { total: 4, err: 1, warn: 2, info: 1 });
  assert.deepEqual(
    diagnostics.issueRows.map((row) => [row.kind, row.severity, row.code, row.sourceMapIndex]),
    [
      ["approximation", "warn", "TBCD-APPROXIMATION", 0],
      ["unsupported", "err", "TBCD-UNSUPPORTED", null],
      ["unreduced", "warn", "TBCD-UNREDUCED", null],
      ["suppressed", "info", "TBCD-FASTPATH", 0],
    ],
  );
  assert.match(diagnostics.issueRows[0].location, /model\.tbc:1/);

  const cleanDiagnostics = buildDiagnosticsPanelModel({
    ...data,
    diagnostics: {
      ...data.diagnostics!,
      approximations: [],
      unsupportedInstructions: [],
      unreducedOpcodes: [],
      suppressedFastPathRanges: [],
      passDecisions: [{ name: "reduce", enabled: true, iteration: 1, changed: true }],
    },
  }, cursor);
  assert.equal(cleanDiagnostics.status, "clean");
  assert.deepEqual(cleanDiagnostics.issueCounts, { total: 0, err: 0, warn: 0, info: 0 });

  const unavailableDiagnostics = buildDiagnosticsPanelModel({ ...data, diagnostics: undefined }, cursor);
  assert.equal(unavailableDiagnostics.status, "unavailable");
  assert.equal(unavailableDiagnostics.issueRows.length, 0);

  const console = searchOutput(
    [
      { stream: "stdout", text: "ok", timestamp: 1 },
      { stream: "stderr", text: "bad", timestamp: 2 },
      { stream: "eval", text: "expr 1+1", timestamp: 3 },
    ],
    createOutputSearchState("bad", { filterToMatches: true }),
  );
  assert.deepEqual(console.visibleLines.map((line) => line.stream), ["stderr"]);
});

test("V6 Compare row classification is deterministic from source-map production facts", () => {
  const base = tcldDataFromLoadResult(createLoadResult("/tmp/compare-model.tbc", "compare-model.tbc") as never);
  const available = buildComparisonPanelModel(base, null);
  assert.deepEqual(available?.rows.map((row) => row.status), ["match", "match"]);
  assert.equal(available?.rows[0].sourceText, "set x 1");
  assert.match(available?.rows[0].metadata ?? "", /#0 bc 0\.\.2 source 0\.\.1/);
  assert.deepEqual(available?.legendCounts, { match: 2, partial: 0, stripped: 0 });

  const partial = buildComparisonPanelModel({
    ...base,
    originalSourceText: "set x 2\nputs $x\n",
  }, null);
  assert.equal(partial?.rows[0].status, "partial");
  assert.equal(partial?.rows[1].status, "match");
  assert.deepEqual(partial?.legendCounts, { match: 1, partial: 1, stripped: 0 });

  const unavailable = buildComparisonPanelModel({
    ...base,
    provenance: { kind: "DebuggerChunk", label: "live#1", originalSourceAvailable: false },
    originalSourceText: null,
    sourceMap: base.sourceMap?.map((entry) => ({ ...entry, originalStatus: "unavailable" as const, originalSpan: null })),
  }, null);
  assert.deepEqual(unavailable?.rows.map((row) => row.status), ["stripped", "stripped"]);
  assert.equal(unavailable?.rows[0].sourceText, "Original source unavailable.");
  assert.deepEqual(unavailable?.legendCounts, { match: 0, partial: 0, stripped: 2 });
  assert.equal(unavailable?.original.available, false);
});

test("V6 Compare view is selector-backed without prototype pair rows or enabled filter controls", () => {
  const panel = readProductionSource("components/panels/ComparisonPanel.tsx");
  const model = readProductionSource("lib/workflow-contracts.ts");
  const css = readProductionSource("app/globals.css");
  const surfaceContract = readProductionSource("lib/design-surface-contract.ts");

  for (const selector of [
    'data-comparison-mode="pair-rows"',
    'data-comparison-toolbar="true"',
    "data-comparison-legend-status={status}",
    "data-comparison-legend-count={model?.legendCounts[status] ?? 0}",
    "data-comparison-row={row.status}",
    "data-comparison-status={row.status}",
    "data-original-available={row.originalAvailable ? \"true\" : \"false\"}",
    "data-comparison-empty-state={model?.original.available ? \"empty\" : \"no-source\"}",
  ]) {
    assert.ok(panel.includes(selector), `missing compare selector ${selector}`);
  }

  for (const marker of [
    ".comparison-toolbar",
    ".comparison-legend",
    ".comparison-row",
    ".comparison-row-rail-match",
    ".comparison-row-rail-partial",
    ".comparison-row-rail-stripped",
    "grid-template-columns: 8px minmax(0, 1fr) minmax(0, 1fr)",
    "background: var(--accent)",
    "background: var(--warn)",
    "background: var(--fg-4)",
  ]) {
    assert.ok(css.includes(marker), `missing compare CSS marker ${marker}`);
  }

  assert.match(surfaceContract, /designId: "compare"[\s\S]*productionFile: "components\/panels\/ComparisonPanel\.tsx"/);
  assert.match(surfaceContract, /id: "compare\.selection"[\s\S]*productionAction: "comparison:select"[\s\S]*productionModel: "buildComparisonModel"/);
  assert.match(model, /type ComparisonRowStatus = "match" \| "partial" \| "stripped"/);
  assert.match(model, /metadata: `#\$\{sourceMapIndex\} bc/);
  assert.doesNotMatch(panel, /set debug_types \[list main ctrl|foreach_start1|package provide fixture 1\.1|data-comparison-filter|data-compare-filter|comparison:filter/i);
});

test("V6 Inspector summary layout is selector-backed and preserves canonical panel navigation", () => {
  const detailPanel = readProductionSource("components/panels/DetailPanel.tsx");
  const surfaceContract = readProductionSource("lib/design-surface-contract.ts");

  for (const selector of [
    "data-inspector-summary=\"true\"",
    // Section root + head toggle markers ride through the Subpanel
    // primitive's sectionAttrs/headAttrs so the root marker stays in the
    // DOM (with data-open) even while the section is collapsed.
    "sectionAttrs={{ \"data-inspector-section\": id }}",
    "headAttrs={{ \"data-inspector-section-toggle\": id }}",
    "data-inspector-section-content={sectionId}",
    "data-inspector-literals-summary=\"true\"",
    "data-inspector-diagnostics-summary=\"true\"",
    "data-inspector-action=\"show-all-literals\"",
    "data-inspector-action=\"show-diagnostics\"",
    "data-inspector-debug-summary=\"variables\"",
    "data-inspector-debug-summary=\"stack\"",
    "buildInspectorLiteralSummaryModel",
    "buildInspectorDiagnosticsSummaryModel",
  ]) {
    assert.ok(detailPanel.includes(selector), `missing inspector selector or model marker ${selector}`);
  }

  // The disclosure caret affordance moved into the Subpanel primitive head.
  const subpanel = readProductionSource("components/primitives/Subpanel.tsx");
  assert.match(subpanel, /\{open \? "▾" : "▸"\}/);
  assert.match(subpanel, /aria-expanded=\{open\}/);
  assert.match(subpanel, /\{\.\.\.sectionAttrs\}/);
  assert.match(subpanel, /\{\.\.\.headAttrs\}/);

  assert.doesNotMatch(detailPanel, /package provide fixture Revision: 1\.1/, "inspector must not copy prototype literal sample data");
  assert.match(detailPanel, /debugActive && debug\.variables\.length > 0/);
  assert.match(detailPanel, /debugActive && debug\.stack\.length > 0/);
  assert.match(surfaceContract, /id: "inspector\.literals-summary"[\s\S]*productionAction: "panel:literals"/);
  assert.match(surfaceContract, /id: "inspector\.diagnostics-summary"[\s\S]*productionAction: "panel:diagnostics"/);
});

test("V6 Issues View renders Diagnostics as selector-backed issue rows without prototype rows", () => {
  const panel = readProductionSource("components/panels/DiagnosticsPanel.tsx");
  const css = readProductionSource("app/globals.css");
  const surfaceContract = readProductionSource("lib/design-surface-contract.ts");

  for (const selector of [
    "designSurface=\"view-issues\"",
    "data-diagnostics-state={model.status}",
    "data-diagnostics-summary=\"severity-counts\"",
    "data-diagnostics-count=\"total\"",
    "data-diagnostics-count=\"err\"",
    "data-diagnostics-count=\"warn\"",
    "data-diagnostics-count=\"info\"",
    "data-diagnostics-issue-list=\"true\"",
    "data-diagnostics-issue-row={row.kind}",
    "data-issue-severity={row.severity}",
    "data-diagnostics-severity-pill={row.severity}",
    "data-diagnostics-issue-meta=\"true\"",
    "data-diagnostics-empty-state=\"clean\"",
    "data-diagnostics-empty-state=\"unavailable\"",
  ]) {
    assert.ok(panel.includes(selector), `missing issues selector ${selector}`);
  }

  assert.match(css, /\.diagnostics-summary-strip/);
  assert.match(css, /\.diagnostics-issue-row/);
  assert.match(css, /\.diagnostics-severity-pill-warn/);
  assert.match(surfaceContract, /designId: "issues"[\s\S]*productionViewId: "diagnostics"/);
  assert.match(surfaceContract, /id: "issues\.selection"[\s\S]*productionAction: "diagnostics:select"/);

  for (const prototypeText of [
    "unsupported tcl::mathfunc",
    "literal table reduced 12%",
    "fast path enabled for foreach over list",
    "TBC-031",
    "TBC-008",
    "TBC-014",
  ]) {
    assert.doesNotMatch(panel, new RegExp(prototypeText.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")), `prototype issue row leaked: ${prototypeText}`);
  }
});

test("V6 Source View contract maps design source to reconstructed decompile source", () => {
  const sourceMapping = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === "source");
  assert.equal(sourceMapping?.productionViewId, "decompile");
  assert.equal(sourceMapping?.productionFile, "components/panels/DecompilePanel.tsx");
  assert.match(sourceMapping?.productionPresentation ?? "", /reconstructed Tcl source/);

  const sourceControl = readProductionSource("lib/design-surface-contract.ts");
  const registry = readProductionSource("lib/view-registry.ts");
  const decompilePanel = readProductionSource("components/panels/DecompilePanel.tsx");
  const sourcePanel = readProductionSource("components/panels/SourcePanel.tsx");
  const css = readProductionSource("app/globals.css");

  assert.match(registry, /\{ id: "decompile", label: "Source View"/);
  assert.match(registry, /\{ id: "source", label: "Source Map"/);
  assert.match(sourceControl, /productionViewId: "decompile"[\s\S]*productionPresentation: "reconstructed Tcl source from TcldData\.decompileLines"/);
  assert.match(sourceControl, /id: "source\.linked-navigation"[\s\S]*productionFile: "components\/panels\/DecompilePanel\.tsx"/);
  assert.match(decompilePanel, /data-source-reconstructed="true"/);
  // Group headers are the first row of each command group since the folding
  // refactor; the marker is conditional on that row role.
  assert.match(decompilePanel, /data-source-group-header=\{isGroupHeader \? "true" : undefined\}/);
  assert.match(decompilePanel, /data-source-breakpoint-gutter="true"/);
  assert.match(decompilePanel, /data-source-toolbar-action="fold-all"/);
  assert.match(decompilePanel, /data-token-role=\{tokenRole\}/);
  assert.match(decompilePanel, /data-execution=\{isExecution \? "true" : undefined\}/);
  assert.match(decompilePanel, /decompile-editor-status/);
  assert.match(sourcePanel, /title="Source Map"/);
  assert.match(sourcePanel, /designSurface="view-source-map"/);
  assert.match(css, /\.decompile-source-group-header/);
  assert.match(css, /\.decompile-source-group-rail/);
});

test("V6 Flow view maps GraphPanel and audits Source mode grid deviation", () => {
  const flowMapping = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === "flow");
  const flowDeviation = DESIGN_DEVIATIONS.find((entry) => entry.id === "flow-source-snippet-grid");
  const graphPanel = readProductionSource("components/panels/GraphPanel.tsx");
  const graphModel = readProductionSource("lib/graph-model.ts");
  const css = readProductionSource("app/globals.css");

  assert.equal(flowMapping?.deviationId, "views-flow-id");
  assert.equal(flowDeviation?.designSurface, "view-flow / Source mode");
  assert.match(flowDeviation?.implementationEvidence ?? "", /buildGraphViewModel/);
  assert.match(graphModel, /computeGraphMetrics/);
  assert.match(graphModel, /deriveGraphBlockRole/);
  assert.match(graphPanel, /data-flow-block-role/);
  assert.match(graphPanel, /flow-source-node-rail/);
  assert.match(graphPanel, /data-flow-status="true"/);
  assert.match(graphPanel, /tcld-flow-chip-warn/);
  assert.match(graphPanel, /fallthrough/);
  assert.match(graphPanel, /conditional-true/);
  assert.match(graphPanel, /conditional-false/);
  assert.match(graphPanel, /unconditional/);
  assert.doesNotMatch(graphPanel, /var\(--ok\)/);
  assert.match(css, /\.tcld-flow-status/);
});

test("V6 Trace view maps ProvenancePanel two-pane layout and audits trace deviation", () => {
  const traceMapping = DESIGN_VIEW_MAPPINGS.find((mapping) => mapping.designId === "trace");
  const traceDeviation = DESIGN_DEVIATIONS.find((entry) => entry.id === "views-trace-id");
  const provenancePanel = readProductionSource("components/panels/ProvenancePanel.tsx");
  const panelModels = readProductionSource("lib/panel-models.ts");
  const css = readProductionSource("app/globals.css");

  assert.equal(traceMapping?.deviationId, "views-trace-id");
  assert.match(traceDeviation?.implementationEvidence ?? "", /buildTracePanelModel/);
  assert.match(traceDeviation?.justification ?? "", /passDecisions|recognizedConstructs/);

  assert.match(provenancePanel, /buildTracePanelModel/);
  assert.match(provenancePanel, /data-trace-shell="true"/);
  assert.match(provenancePanel, /data-trace-pane="left"/);
  assert.match(provenancePanel, /data-trace-pane="right"/);
  assert.match(provenancePanel, /data-trace-row="true"/);
  assert.match(provenancePanel, /data-trace-pass-status/);
  assert.match(provenancePanel, /data-trace-evidence-kind/);
  assert.match(provenancePanel, /data-trace-decision-outcome/);
  assert.match(provenancePanel, /data-trace-filter-input="true"/);
  assert.doesNotMatch(provenancePanel, /TRACE_DETAIL|TRACE_COMMANDS/);
  assert.doesNotMatch(provenancePanel, /var\(--ok\)/);

  assert.match(panelModels, /export function buildTracePanelModel/);
  assert.match(panelModels, /TraceCommandRow/);
  assert.match(panelModels, /TracePassRow/);
  assert.match(panelModels, /TraceEvidenceRow/);
  assert.doesNotMatch(panelModels, /TRACE_DETAIL/);

  assert.match(css, /\.tcld-trace-shell/);
  assert.match(css, /\.tcld-trace-chain/);
  assert.match(css, /\.tcld-trace-evidence-badge-source/);
});

test("V6 rendered Instructions, Source, Flow, Trace, and Console actions mutate production panels", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/views.tbc" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /views\.tbc/);

    await app.page.click('[title="Filter instructions (⌘F /)"]');
    await app.page.select('[data-panel-id="instructions"] select', "group");
    await app.page.type('input[placeholder="group #"]', "0");
    // Filter status reads "n/m filt" in the toolbar meta since the panel
    // toolbar refresh.
    await waitForText(app.page, '[data-panel-id="instructions"]', /\d+\/\d+ filt/);

    await app.page.click('[title="Fold all commands"]');
    await app.page.waitForFunction(() => [...document.querySelectorAll("[data-fold-disclosure='command']")].some((node) => node.getAttribute("data-fold-state") === "collapsed"));
    await app.page.waitForFunction(() =>
      /^[1-9]\d*\/[1-9]\d*$/.test(document.querySelector('[data-panel-id="instructions"] [data-disassembly-folded-count]')?.getAttribute("data-disassembly-folded-count") ?? ""),
    );
    await app.page.click('[title="Unfold all commands"]');
    await app.page.waitForFunction(() => [...document.querySelectorAll("[data-fold-disclosure='command']")].every((node) => node.getAttribute("data-fold-state") !== "collapsed"));
    await app.page.waitForFunction(() =>
      /^0\/[1-9]\d*$/.test(document.querySelector('[data-panel-id="instructions"] [data-disassembly-folded-count]')?.getAttribute("data-disassembly-folded-count") ?? ""),
    );
    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-layout-menu-kind="built-in"][data-layout-menu-id="flow"]');
    await app.page.click('[data-layout-menu-kind="built-in"][data-layout-menu-id="flow"]');
    await app.page.waitForSelector('[data-panel-id="graph"]');
    await app.page.click('[title="Source Snippet Mode"]');
    await app.page.waitForSelector('[data-graph-mode="source-snippet"]');
    await app.page.click('[title="full detail (3)"]');
    await waitForText(app.page, '[data-panel-id="graph"]', /Source/);
    const flowState = await app.page.evaluate(() => {
      const panel = document.querySelector('[data-panel-id="graph"]') as HTMLElement | null;
      const toolbarText = panel?.querySelector(".tcld-flow-toolbar")?.textContent ?? "";
      const status = panel?.querySelector('[data-flow-status="true"]') as HTMLElement | null;
      const edgeRoles = [...(panel?.querySelectorAll('[data-flow-edge-legend="toolbar"] [data-edge-role]') ?? [])]
        .map((node) => node.getAttribute("data-edge-role"));
      const sourceNodes = [...(panel?.querySelectorAll('[data-graph-mode="source-snippet"][data-flow-role="source-anchor"]') ?? [])] as HTMLElement[];
      const sourceRoleSet = new Set(sourceNodes.map((node) => node.getAttribute("data-flow-block-role")));
      const graphText = panel?.textContent ?? "";
      return {
        toolbarText,
        statusText: status?.textContent ?? "",
        selectedBlock: status?.querySelector("[data-flow-selected-block]")?.getAttribute("data-flow-selected-block"),
        edgeRoles,
        sourceNodeCount: sourceNodes.length,
        sourceRoles: [...sourceRoleSet],
        sourceHasTargets: sourceNodes.every((node) => node.getAttribute("data-command-path") || /pc \d+\.\.\d+/.test(node.textContent ?? "")),
        sourceHasBlockRoles: sourceNodes.every((node) => Boolean(node.getAttribute("data-flow-block-role"))),
        undefinedOkToken: /var\(--ok\)/.test(panel?.outerHTML ?? ""),
        hasPrototypeCopy: /fixture::startlog|foreach · sources|debug\+/.test(graphText),
      };
    });
    assert.match(flowState.toolbarText, /Disasm/);
    assert.match(flowState.toolbarText, /Source/);
    assert.match(flowState.toolbarText, /\d+ blocks/);
    assert.match(flowState.toolbarText, /\d+ edges/);
    assert.match(flowState.toolbarText, /\d+ loops/);
    assert.match(flowState.toolbarText, /\d+ branches/);
    assert.deepEqual(flowState.edgeRoles, ["fallthrough", "conditional-true", "conditional-false", "unconditional"]);
    assert.match(flowState.statusText, /selected/);
    assert.match(flowState.statusText, /cyclomatic/);
    assert.equal(flowState.sourceNodeCount >= 1, true);
    assert.equal(flowState.sourceHasTargets, true);
    assert.equal(flowState.sourceHasBlockRoles, true);
    assert.equal(flowState.undefinedOkToken, false);
    assert.equal(flowState.hasPrototypeCopy, false);
    assert.ok(flowState.sourceRoles.includes("entry") || flowState.sourceRoles.includes("exit"));

    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-layout-menu-kind="built-in"][data-layout-menu-id="trace"]');
    await app.page.click('[data-layout-menu-kind="built-in"][data-layout-menu-id="trace"]');
    await app.page.waitForSelector('[data-panel-id="provenance"]');
    await app.page.waitForSelector('[data-panel-id="console"]');
    // Trace detail labels the span as "lines a..b" in the two-pane layout.
    await waitForText(app.page, '[data-panel-id="provenance"]', /lines 0\.\.1/);
    await waitForVisibleText(app.page, '[data-panel-id="console"]', /loaded views\.tbc|reconstruction=/);
    await app.page.click('[data-panel-id="console"] button:nth-of-type(2)');
  } finally {
    await closeProductionPage(app);
  }
});

test("V6 rendered Source View groups, folds, breakpoint gutter, and header actions use production state", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/source-view.tbc", debugPath: "/tmp/source-view.tcl" });
  try {
    await app.page.evaluate(() => {
      const previous = window.__TCLD_TEST_CREATE_LOAD_RESULT__;
      window.__TCLD_TEST_CREATE_LOAD_RESULT__ = (pathname: string) => {
        const result = previous?.(pathname) as Record<string, unknown>;
        result.metadata = {
          ...(result.metadata as Record<string, unknown>),
          filename: String(pathname).split("/").pop() || "source-view.tbc",
          instructionCount: 5,
          commandCount: 2,
        };
        result.instructions = [
          { idx: 0, pc: 0, op: "push1", args: "namespace", operands: [0], argc: 0, group: 0, commandPcRange: [0, 3], groupStart: true, comment: "namespace", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 1, primaryNextLocation: 1, secondaryNextLocation: null, nextLocations: [1] },
          { idx: 1, pc: 1, op: "push1", args: "eval", operands: [1], argc: 0, group: 0, commandPcRange: [0, 3], groupStart: false, comment: "eval", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 2, primaryNextLocation: 2, secondaryNextLocation: null, nextLocations: [2] },
          { idx: 2, pc: 2, op: "invokeStk1", args: "argc 2", operands: [2], argc: 2, group: 0, commandPcRange: [0, 3], groupStart: false, comment: "invoke", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 3, primaryNextLocation: 3, secondaryNextLocation: null, nextLocations: [3] },
          { idx: 3, pc: 3, op: "push1", args: "puts", operands: [3], argc: 0, group: 1, commandPcRange: [3, 4], groupStart: true, comment: "puts", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 4, primaryNextLocation: 4, secondaryNextLocation: null, nextLocations: [4] },
          { idx: 4, pc: 4, op: "done", args: "", operands: [], argc: 0, group: 1, commandPcRange: [3, 4], groupStart: false, comment: "done", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: null, primaryNextLocation: null, secondaryNextLocation: null, nextLocations: [] },
        ];
        result.decompileLines = [
          { lineIndex: 0, text: "namespace eval fixture {", indent: 0, group: 0, sourceMapIndex: 0, sourceLineStart: 0, sourceLineEnd: 2 },
          { lineIndex: 1, text: "    set x 1", indent: 4, group: 0, sourceMapIndex: 0, sourceLineStart: 1, sourceLineEnd: 1 },
          { lineIndex: 2, text: "}", indent: 0, group: 0, sourceMapIndex: 0, sourceLineStart: 2, sourceLineEnd: 2 },
          { lineIndex: 3, text: "puts $x", indent: 0, group: 1, sourceMapIndex: 1, sourceLineStart: 3, sourceLineEnd: 3 },
        ];
        result.decompileText = "namespace eval fixture {\n    set x 1\n}\nputs $x\n";
        // The namespace block (lines 0..2) is the foldable region the
        // backend's tcld-syntax folding pass would report for this text.
        result.decompileFoldingRanges = [
          { kind: "region", startByte: 0, endByte: 34, startRow: 0, startColumn: 0, endRow: 2, endColumn: 1 },
        ];
        result.decompileTokens = [
          { capture: "keyword", startByte: 0, endByte: 9, startRow: 0, startColumn: 0, endRow: 0, endColumn: 9 },
          { capture: "function", startByte: 10, endByte: 14, startRow: 0, startColumn: 10, endRow: 0, endColumn: 14 },
          { capture: "keyword", startByte: 25, endByte: 28, startRow: 1, startColumn: 4, endRow: 1, endColumn: 7 },
          { capture: "variable", startByte: 40, endByte: 42, startRow: 3, startColumn: 5, endRow: 3, endColumn: 7 },
        ];
        result.sourceMap = [
          { commandGroup: 0, commandPath: [0], parentCommandGroup: null, bytecodeRange: [0, 3], sourceLines: [0, 3], sourceChars: [0, 34], text: "namespace eval fixture {\n    set x 1\n}", originalStatus: "available", originalSpan: [0, 34] },
          { commandGroup: 1, commandPath: [1], parentCommandGroup: null, bytecodeRange: [3, 4], sourceLines: [3, 4], sourceChars: [35, 42], text: "puts $x", originalStatus: "available", originalSpan: [35, 42] },
        ];
        result.originalSourceText = "ORIGINAL SOURCE MAP ONLY\n";
        return result;
      };
    });

    await app.page.click('[data-testid="welcome-open-file"]');
    await app.page.waitForSelector('[data-panel-id="decompile"] [data-source-group-header="true"][data-source-group="0"]');
    await app.page.click('[data-panel-id="decompile"] [data-source-line="3"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-panel-id="instructions"] [data-command-group="1"]')?.getAttribute("data-selected-group") === "true",
    );
    // Breakpoints are frontend-owned: set one in the gutter BEFORE launch
    // (the IDE workflow), and the store replays it to the backend on Run.
    await app.page.click('[data-panel-id="decompile"] [data-source-line="1"] [data-source-breakpoint-gutter="true"]');
    await app.page.waitForSelector('[data-panel-id="decompile"] [data-source-breakpoint-active="true"]');
    await app.page.click('[title="Run / Debug (F5)"]');
    await app.page.waitForFunction(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).some((call) =>
        call.cmd === "debug_set_breakpoint" && call.args?.file === "/tmp/source-view.tbc" && call.args?.line === 2),
    );
    await app.page.waitForSelector('[data-panel-id="decompile"] [data-source-breakpoint-active="true"]');
    await waitForVisibleText(app.page, '[data-panel-id="decompile"]', /namespace eval/);
    const initial = await app.page.evaluate(() => ({
      text: document.querySelector('[data-panel-id="decompile"]')?.textContent ?? "",
      groupHeaders: document.querySelectorAll('[data-panel-id="decompile"] [data-source-group-header="true"]').length,
      sourceRows: document.querySelectorAll('[data-panel-id="decompile"] [data-source-reconstructed="true"]').length,
      commandTokens: document.querySelectorAll('[data-panel-id="decompile"] [data-token-role="tcl-command"]').length,
      variableTokens: document.querySelectorAll('[data-panel-id="decompile"] [data-token-role="tcl-variable"]').length,
      activeBreakpoints: document.querySelectorAll('[data-panel-id="decompile"] [data-source-breakpoint-active="true"]').length,
      linkedState: document.querySelector('[data-panel-id="decompile"] [data-source-toolbar-action="linked"]')?.getAttribute("data-source-linked"),
    }));
    assert.equal(initial.text.includes("ORIGINAL SOURCE MAP ONLY"), false, "Source View must render reconstructed decompile text, not original-source text");
    assert.ok(initial.groupHeaders >= 2, "reconstructed source renders command group headers");
    assert.ok(initial.sourceRows >= 4, "reconstructed source rows render from TcldData.decompileLines");
    assert.ok(initial.commandTokens >= 1, "syntax command tokens are selector-visible");
    assert.ok(initial.variableTokens >= 1, "syntax variable tokens are selector-visible");
    assert.ok(initial.activeBreakpoints >= 1, "debug breakpoint data renders in the source gutter");
    assert.equal(initial.linkedState, "true");

    await app.page.evaluate(() => {
      if (window.__TCLD_TEST_INVOKES__) window.__TCLD_TEST_INVOKES__.length = 0;
    });
    // Gutter clicks cycle enabled/disabled on the existing breakpoint. The
    // marker semantics: data-source-breakpoint-active tracks existence, and
    // the dot's data-bp-disabled distinguishes the disabled state.
    const lineTwoBreakpointGutter = '[data-panel-id="decompile"] [data-source-line="1"] [data-source-breakpoint-gutter="true"]';
    await app.page.click(lineTwoBreakpointGutter);
    await app.page.waitForFunction(
      (selector) => document.querySelector(`${selector} .decompile-bp-dot`)?.getAttribute("data-bp-disabled") === "true",
      {},
      lineTwoBreakpointGutter,
    );
    await app.page.click(lineTwoBreakpointGutter);
    await app.page.waitForFunction(
      (selector) => {
        const dot = document.querySelector(`${selector} .decompile-bp-dot`);
        return Boolean(dot && dot.getAttribute("data-active") != null && dot.getAttribute("data-bp-disabled") == null);
      },
      {},
      lineTwoBreakpointGutter,
    );
    const breakpointToggleInvokes = await app.page.evaluate(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).filter((entry) => entry.cmd === "debug_set_breakpoint_enabled"),
    );
    assert.deepEqual(
      breakpointToggleInvokes.map((entry) => entry.args),
      [
        // Run debugs the loaded document, so breakpoints key off its path.
        { file: "/tmp/source-view.tbc", line: 2, enabled: false },
        { file: "/tmp/source-view.tbc", line: 2, enabled: true },
      ],
      "clicking the Source View breakpoint gutter must toggle production breakpoint state",
    );

    await app.page.click('[data-panel-id="decompile"] [data-source-toolbar-action="fold-all"]');
    await app.page.waitForSelector('[data-panel-id="decompile"] [data-source-group-header="true"][data-fold-state="collapsed"]');
    const folded = await app.page.evaluate(() => ({
      foldedCount: document.querySelector('[data-panel-id="decompile"] [data-source-folded-count]')?.getAttribute("data-source-folded-count"),
      visibleRows: document.querySelectorAll('[data-panel-id="decompile"] [data-source-reconstructed="true"]').length,
    }));
    assert.match(folded.foldedCount ?? "", /^[1-9]\d*\/[1-9]\d*$/);
    assert.ok(folded.visibleRows < initial.sourceRows, "fold all hides grouped source body rows");

    await app.page.click('[data-panel-id="decompile"] [data-source-toolbar-action="unfold-all"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-panel-id="decompile"] [data-source-folded-count]')?.getAttribute("data-source-folded-count")?.startsWith("0/"),
    );

    await app.page.click('[data-panel-id="decompile"] [data-source-toolbar-action="linked"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-panel-id="decompile"] [data-source-toolbar-action="linked"]')?.getAttribute("data-source-linked") === "false",
    );

    // Split moved from an inline header button to the workspace command:
    // Mod+\ splits the focused Source View to the right edge and keeps it a
    // singleton. Mod is the platform-primary modifier — Meta on macOS,
    // Ctrl elsewhere — so dispatch through the harness helper instead of
    // hardcoding Meta.
    const leavesBeforeSplit = await app.page.$$eval("[data-leaf-id]", (nodes) => nodes.length);
    await app.page.click('[data-panel-id="decompile"] [data-source-line="0"]');
    await pressModShortcut(app.page, "\\");
    await app.page.waitForFunction((before) => document.querySelectorAll("[data-leaf-id]").length > before, {}, leavesBeforeSplit);
    const decompileCount = await app.page.$$eval('[data-panel-id="decompile"]', (nodes) => nodes.length);
    assert.equal(decompileCount, 1, "split keeps the reconstructed Source View a singleton");
  } finally {
    await closeProductionPage(app);
  }
});

test("V6 rendered Instructions flat stream has headers, disabled byte view, and selection sync", async () => {
  const app = await withProductionPage("/?parity=instructions", { dialogPath: "/tmp/instructions-flat.tbc" });
  try {
    await app.page.waitForSelector('[data-panel-id="instructions"][data-design-surface="view-instructions"]');
    await app.page.waitForSelector('[data-instruction-presentation="instructions"]');
    await waitForVisibleText(app.page, '[data-panel-id="instructions"]', /flat instruction stream/);

    const flatState = await app.page.evaluate(() => {
      const panel = document.querySelector('[data-panel-id="instructions"]') as HTMLElement | null;
      const headers = [...document.querySelectorAll('[data-instructions-flat-header] [data-instructions-column-header]')]
        .map((node) => node.textContent?.trim());
      const rows = [...document.querySelectorAll('[data-panel-id="instructions"] [data-instruction-row-mode="flat"]')];
      const byteView = document.querySelector('[data-instructions-byte-view]') as HTMLButtonElement | null;
      return {
        designSurface: panel?.getAttribute("data-design-surface"),
        presentation: document.querySelector('[data-instruction-presentation]')?.getAttribute("data-instruction-presentation"),
        headers,
        count: document.querySelector('[data-instructions-flat-count]')?.getAttribute("data-instructions-flat-count"),
        rowCount: rows.length,
        hasFoldControls: Boolean(document.querySelector('[data-fold-disclosure="command"], [data-disassembly-toolbar-action="fold-all"]')),
        byteViewState: byteView?.getAttribute("data-instructions-byte-view"),
        byteViewDisabled: Boolean(byteView?.disabled || byteView?.getAttribute("aria-disabled") === "true"),
        byteViewTitle: byteView?.getAttribute("title") ?? "",
        fakeByteText: /hex dump|raw bytes from mock|mock byte/i.test(document.body.textContent ?? ""),
      };
    });

    assert.equal(flatState.designSurface, "view-instructions");
    assert.equal(flatState.presentation, "instructions");
    assert.deepEqual(flatState.headers, ["#", "addr", "opcode", "operand", "hint/comment"]);
    assert.equal(flatState.count, "4/4");
    assert.ok(flatState.rowCount >= 4, "flat stream should render loaded instruction rows");
    assert.equal(flatState.hasFoldControls, false, "flat instructions should not expose grouped fold controls");
    assert.equal(flatState.byteViewState, "disabled");
    assert.equal(flatState.byteViewDisabled, true);
    assert.match(flatState.byteViewTitle, /raw byte ranges and bytes/);
    assert.equal(flatState.fakeByteText, false);

    await app.page.click('[data-panel-id="instructions"] [data-idx="3"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="tcld-design-parity-sample.tbc"]')?.getAttribute("data-selected-inst") === "3"
      && document.querySelector('[data-panel-id="instructions"] [data-idx="3"]')?.getAttribute("data-selected") === "true",
    );
  } finally {
    await closeProductionPage(app);
  }
});

test("V6 rendered Disassembly IR mode remains reachable separately from flat Instructions", async () => {
  const app = await withProductionPage("/?parity=open-file", { dialogPath: "/tmp/instructions-grouped.tbc" });
  try {
    await app.page.waitForSelector('[data-panel-id="instructions"][data-design-surface="view-disassembly"]');
    await app.page.waitForSelector('[data-instruction-presentation="ir"]');
    await waitForVisibleText(app.page, '[data-panel-id="instructions"]', /grouped disassembly/);

    const groupedState = await app.page.evaluate(() => ({
      designSurface: document.querySelector('[data-panel-id="instructions"]')?.getAttribute("data-design-surface"),
      presentation: document.querySelector('[data-instruction-presentation]')?.getAttribute("data-instruction-presentation"),
      hasGroupStarts: Boolean(document.querySelector('[data-panel-id="instructions"] [data-group-start="true"]')),
      hasFoldControls: Boolean(document.querySelector('[data-panel-id="instructions"] [data-disassembly-toolbar-action="fold-all"]')),
      hasFlatHeader: Boolean(document.querySelector('[data-panel-id="instructions"] [data-instructions-flat-header]')),
      hasByteView: Boolean(document.querySelector('[data-panel-id="instructions"] [data-instructions-byte-view]')),
    }));

    assert.equal(groupedState.designSurface, "view-disassembly");
    assert.equal(groupedState.presentation, "ir");
    assert.equal(groupedState.hasGroupStarts, true);
    assert.equal(groupedState.hasFoldControls, true);
    assert.equal(groupedState.hasFlatHeader, false);
    assert.equal(groupedState.hasByteView, false);

    await app.page.click('[data-instructions-mode-switch="instructions"]');
    await app.page.waitForSelector('[data-instruction-presentation="instructions"]');
    await app.page.click('[data-instructions-mode-switch="ir"]');
    await app.page.waitForSelector('[data-instruction-presentation="ir"]');
  } finally {
    await closeProductionPage(app);
  }
});

test("V6 rendered Symbols and Watchpoints views have production-backed interaction evidence", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/views-debug.tbc", debugPath: "/tmp/views-debug.tcl" });
  try {
    await app.page.evaluate(() => {
      const previous = window.__TCLD_TEST_CREATE_LOAD_RESULT__;
      window.__TCLD_TEST_CREATE_LOAD_RESULT__ = (pathname: string) => {
        const result = previous?.(pathname) as Record<string, unknown>;
        result.metadata = {
          ...(result.metadata as Record<string, unknown>),
          filename: String(pathname).split("/").pop() || "views-debug.tbc",
          instructionCount: 6,
          commandCount: 3,
        };
        result.instructions = [
          { idx: 0, pc: 0, op: "push1", args: "namespace", operands: [0], argc: 0, group: 0, commandPcRange: [0, 8], groupStart: true, comment: "namespace", size: 4, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 4, primaryNextLocation: 4, secondaryNextLocation: null, nextLocations: [4] },
          { idx: 1, pc: 4, op: "invokeStk1", args: "argc 3", operands: [3], argc: 3, group: 0, commandPcRange: [0, 8], groupStart: false, comment: "invoke", size: 4, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 8, primaryNextLocation: 8, secondaryNextLocation: null, nextLocations: [8] },
          { idx: 2, pc: 8, op: "push1", args: "proc", operands: [1], argc: 0, group: 1, commandPcRange: [8, 24], groupStart: true, comment: "proc", size: 4, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 12, primaryNextLocation: 12, secondaryNextLocation: null, nextLocations: [12] },
          { idx: 3, pc: 12, op: "invokeStk1", args: "argc 4", operands: [4], argc: 4, group: 1, commandPcRange: [8, 24], groupStart: false, comment: "invoke", size: 12, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 24, primaryNextLocation: 24, secondaryNextLocation: null, nextLocations: [24] },
          { idx: 4, pc: 24, op: "storeScalar1", args: "local 0", operands: [0], argc: 0, group: 2, commandPcRange: [24, 30], groupStart: true, comment: "baseDir", size: 6, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 30, primaryNextLocation: 30, secondaryNextLocation: null, nextLocations: [30] },
          { idx: 5, pc: 30, op: "done", args: "", operands: [], argc: 0, group: 2, commandPcRange: [24, 30], groupStart: false, comment: "done", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: null, primaryNextLocation: null, secondaryNextLocation: null, nextLocations: [] },
        ];
        result.sourceMap = [
          { commandGroup: 0, commandPath: [0], parentCommandGroup: null, bytecodeRange: [0, 8], sourceLines: [0, 1], sourceChars: [0, 20], text: "namespace eval fixture {}", originalStatus: "available", originalSpan: [0, 20] },
          { commandGroup: 1, commandPath: [1], parentCommandGroup: null, bytecodeRange: [8, 24], sourceLines: [1, 4], sourceChars: [21, 78], text: "proc fixture::startlog { _text } {\n    variable baseDir\n}", originalStatus: "available", originalSpan: [21, 78] },
          { commandGroup: 2, commandPath: [1, 0], parentCommandGroup: 1, bytecodeRange: [24, 30], sourceLines: [2, 3], sourceChars: [54, 70], text: "variable baseDir", originalStatus: "available", originalSpan: [54, 70] },
        ];
        result.decompileLines = [
          { lineIndex: 0, text: "namespace eval fixture {}", indent: 0, group: 0, sourceMapIndex: 0, sourceLineStart: 0, sourceLineEnd: 0 },
          { lineIndex: 1, text: "proc fixture::startlog { _text } {", indent: 0, group: 1, sourceMapIndex: 1, sourceLineStart: 1, sourceLineEnd: 3 },
          { lineIndex: 2, text: "    variable baseDir", indent: 4, group: 2, sourceMapIndex: 2, sourceLineStart: 2, sourceLineEnd: 2 },
          { lineIndex: 3, text: "}", indent: 0, group: 1, sourceMapIndex: 1, sourceLineStart: 3, sourceLineEnd: 3 },
        ];
        result.decompileText = "namespace eval fixture {}\nproc fixture::startlog { _text } {\n    variable baseDir\n}\n";
        result.originalSourceText = result.decompileText;
        return result;
      };
    });

    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /views-debug\.tbc/);

    // The Symbols tab title now carries its key hint ("Symbols (l)");
    // target the stable tab hook instead of the title text.
    await app.page.click('[data-tab-view-id="literals"]');
    await app.page.waitForSelector('[data-panel-id="literals"][data-design-surface="view-symbols"]');
    await waitForVisibleText(app.page, '[data-panel-id="literals"]', /fixture::startlog/);
    const symbolsState = await app.page.evaluate(() => ({
      rows: document.querySelectorAll('[data-panel-id="literals"] [data-symbol-row]').length,
      count: document.querySelector('[data-panel-id="literals"] [data-symbol-count="true"]')?.textContent,
      pills: [...document.querySelectorAll('[data-panel-id="literals"] [data-symbol-kind-pill]')].map((node) => node.getAttribute("data-symbol-kind-pill")),
      columns: [...document.querySelectorAll('[data-panel-id="literals"] [data-symbol-column]')].map((node) => node.getAttribute("data-symbol-column")),
      clickableRows: document.querySelectorAll('[data-panel-id="literals"] [data-symbol-row][data-symbol-clickable="true"]').length,
      prototypeCopied: /fixture::stoplog|fixture::setLevel|debug_types|debug_level/.test(document.querySelector('[data-panel-id="literals"]')?.textContent ?? ""),
    }));
    assert.equal(symbolsState.rows, 3);
    assert.equal(symbolsState.count, "3/3");
    assert.deepEqual(symbolsState.pills.sort(), ["ns", "proc", "var"]);
    assert.deepEqual(symbolsState.columns, ["kind", "name", "addr", "size", "refs"]);
    assert.equal(symbolsState.clickableRows, 3);
    assert.equal(symbolsState.prototypeCopied, false);

    await app.page.type('[data-panel-id="literals"] [data-symbol-filter="true"]', "start");
    await app.page.waitForFunction(() => document.querySelector('[data-panel-id="literals"] [data-symbol-count="true"]')?.textContent === "1/3");
    await app.page.click('[data-panel-id="literals"] [data-symbol-row][data-symbol-name="fixture::startlog"]');
    await app.page.waitForSelector('[data-panel-id="literals"] [data-symbol-row][data-symbol-name="fixture::startlog"][data-symbol-selected="true"]');
    await app.page.$eval('[data-panel-id="literals"] [data-symbol-filter="true"]', (node) => {
      const input = node as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await app.page.waitForFunction(() => document.querySelector('[data-panel-id="literals"] [data-symbol-count="true"]')?.textContent === "3/3");
    await app.page.type('[data-panel-id="literals"] [data-symbol-filter="true"]', "missing");
    await app.page.waitForSelector('[data-panel-id="literals"] [data-symbol-empty-state="true"]');
    await waitForVisibleText(app.page, '[data-panel-id="literals"]', /No symbols match "missing"/);

    await app.page.click('[title="Run / Debug (F5)"]');
    await app.page.waitForSelector('.relative[data-design-surface="debug-paused-overlay"]');
    await app.page.click('[data-testid="panel-picker-trigger"]');
    await app.page.click('[data-panel-picker-option="watchpoints"]');
    await app.page.waitForSelector('[data-panel-id="watchpoints"][data-design-surface="debug-watchpoints"]');
    await waitForVisibleText(app.page, '[data-panel-id="watchpoints"]', /watchVar/);
    await waitForVisibleText(app.page, '[data-panel-id="watchpoints"]', /0.*1|write/);
  } finally {
    await closeProductionPage(app);
  }
});

test("V6 Symbols view preserves its unavailable state while filtering", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/no-symbols.tbc" });
  try {
    await app.page.evaluate(() => {
      const previous = window.__TCLD_TEST_CREATE_LOAD_RESULT__;
      window.__TCLD_TEST_CREATE_LOAD_RESULT__ = (pathname: string) => {
        const result = previous?.(pathname) as Record<string, unknown>;
        result.sourceMap = [
          { commandGroup: 0, commandPath: [0], parentCommandGroup: null, bytecodeRange: [0, 2], sourceLines: [0, 1], sourceChars: [0, 10], text: "puts hello", originalStatus: "available", originalSpan: [0, 10] },
        ];
        result.decompileLines = [
          { lineIndex: 0, text: "puts hello", indent: 0, group: 0, sourceMapIndex: 0, sourceLineStart: 0, sourceLineEnd: 0 },
        ];
        result.decompileText = "puts hello\n";
        return result;
      };
    });

    await app.page.click('[data-testid="welcome-open-file"]');
    // The Symbols tab title now carries its key hint ("Symbols (l)");
    // target the stable tab hook instead of the title text.
    await app.page.click('[data-tab-view-id="literals"]');
    await app.page.waitForSelector('[data-panel-id="literals"] [data-symbols-unavailable="true"]');
    await waitForVisibleText(app.page, '[data-panel-id="literals"]', /Symbols unavailable/);
    await waitForVisibleText(app.page, '[data-panel-id="literals"]', /This payload does not expose namespace\/proc\/var symbol facts yet\./);
    await app.page.type('[data-panel-id="literals"] [data-symbol-filter="true"]', "fixture");
    await app.page.waitForSelector('[data-panel-id="literals"] [data-symbols-unavailable="true"]');
    const state = await app.page.evaluate(() => ({
      rows: document.querySelectorAll('[data-panel-id="literals"] [data-symbol-row]').length,
      count: document.querySelector('[data-panel-id="literals"] [data-symbol-count="true"]')?.textContent,
      unavailableVisible: Boolean(document.querySelector('[data-panel-id="literals"] [data-symbols-unavailable="true"]')),
    }));
    assert.equal(state.rows, 0);
    assert.equal(state.count, "0/0");
    assert.equal(state.unavailableVisible, true);
  } finally {
    await closeProductionPage(app);
  }
});
