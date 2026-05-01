import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { CommandContext, CommandDef } from "@/lib/command-registry";
import type { Settings } from "@/lib/settings";
import type { TcldData } from "@/types";

function fixtureData(overrides: Partial<TcldData> = {}): TcldData {
  return {
    metadata: {
      filename: "fixture.tbc",
      path: "/tmp/fixture.tbc",
      tclVersion: "8.6",
      instructionCount: 5,
      literalCount: 2,
      commandCount: 3,
      exceptionCount: 0,
      auxCount: 0,
      codeBytes: 16,
      maxStackDepth: 2,
      syntaxEngine: "tree-sitter-tcl",
      format: 1,
      build: 0,
    },
    instructions: [
      inst(0, 0, "push1", "0", [0], 0, false, false, false, null, 2),
      inst(1, 2, "jumpFalse1", "6", [6], 0, true, true, false, 8, 5),
      inst(2, 5, "push1", "1", [1], 1, false, false, false, null, 7),
      inst(3, 7, "jump1", "3", [3], 1, true, false, true, 12, null),
      inst(4, 12, "done", "", [], 2, false, false, false, null, null),
    ],
    literals: [],
    exceptions: [],
    blocks: [
      { id: 0, start: 0, end: 1, startPc: 0, endPc: 5, group: "c000", edges: [1, 2], instructions: [{ pc: 0, name: "push1", operands: [0] }, { pc: 2, name: "jumpFalse1", operands: [6] }] },
      { id: 1, start: 2, end: 3, startPc: 5, endPc: 10, group: "c001", edges: [2], instructions: [{ pc: 5, name: "push1", operands: [1] }, { pc: 7, name: "jump1", operands: [3] }] },
      { id: 2, start: 4, end: 4, startPc: 12, endPc: 13, group: "c002", edges: [], instructions: [{ pc: 12, name: "done", operands: [] }] },
    ],
    traces: [],
    decompileLines: [
      { text: "proc demo {x} {", group: 0, lineIndex: 0, sourceLineStart: 0, sourceLineEnd: 1, sourceMapIndex: 0 },
      { text: "if {$x} { puts yes }", group: 1, lineIndex: 1, sourceLineStart: 1, sourceLineEnd: 2, sourceMapIndex: 1 },
      { text: "}", group: 2, lineIndex: 2, sourceLineStart: 2, sourceLineEnd: 3, sourceMapIndex: 2 },
      { text: "proc compact {} {puts ok}", group: 3, lineIndex: 3, sourceLineStart: 3, sourceLineEnd: 3, sourceMapIndex: 3 },
    ],
    decompileTokens: [],
    decompileFoldingRanges: [
      { kind: "region", startByte: 0, endByte: 39, startRow: 0, startColumn: 0, endRow: 2, endColumn: 1 },
      { kind: "region", startByte: 16, endByte: 39, startRow: 0, startColumn: 16, endRow: 2, endColumn: 1 },
      { kind: "region", startByte: 16, endByte: 38, startRow: 1, startColumn: 0, endRow: 1, endColumn: 22 },
      { kind: "region", startByte: 56, endByte: 88, startRow: 3, startColumn: 0, endRow: 3, endColumn: 25 },
      { kind: "comment", startByte: 40, endByte: 55, startRow: 4, startColumn: 0, endRow: 5, endColumn: 8 },
    ],
    decompileText: "proc demo {x} {\nif {$x} { puts yes }\n}\nproc compact {} {puts ok}\n",
    dumpText: "Instructions:\n",
    summaryText: "fixture",
    sourceMap: [
      sourceMapEntry([0, 5], [0, 1], "proc demo {x} {", 0, [0]),
      sourceMapEntry([5, 10], [1, 2], "if {$x} { puts yes }", 1, [0, 1], 0),
      sourceMapEntry([10, 13], [2, 3], "}", 2, [2]),
    ],
    sourceCommandAnchors: [
      { kind: "root", sourceMapIndex: 0, commandPath: [0], displayPath: "c000", parentRootGroup: 0, commandGroup: 0, sourceLines: [0, 1], snippet: "proc demo {x} {" },
      { kind: "nested", sourceMapIndex: 1, commandPath: [0, 1], displayPath: "c000/n001", parentRootGroup: 0, commandGroup: 1, sourceLines: [1, 2], snippet: "if {$x} { puts yes }" },
    ],
    diagnostics: {
      reconstruction: "Strict",
      recognizedConstructs: ["if"],
      unreducedOpcodes: [],
      approximations: [],
      suppressedFastPathRanges: [],
      passDecisions: [],
      unsupportedInstructions: [],
      suggestedNextWork: [],
    },
    reconstruction: "Strict",
    provenance: { kind: "Path", label: "/tmp/fixture.tbc", originalSourceAvailable: true },
    originalSourceText: "proc demo {x} {\nif {$x} { puts yes }\n}\n",
    ...overrides,
  };
}

function inst(
  idx: number,
  pc: number,
  op: string,
  args: string,
  operands: number[],
  group: number,
  isJump: boolean,
  isJumpConditional: boolean,
  isJumpUnconditional: boolean,
  jumpTarget: number | null,
  fallthrough: number | null,
) {
  return {
    idx,
    pc,
    op,
    args,
    operands,
    argc: operands.length,
    group,
    commandPcRange: [pc, pc + 2] as [number, number],
    groupStart: idx === 0 || idx === 2 || idx === 4,
    comment: "",
    size: 2,
    isJump,
    isJumpConditional,
    isJumpUnconditional,
    jumpTarget,
    fallthrough,
    primaryNextLocation: jumpTarget,
    secondaryNextLocation: fallthrough,
    nextLocations: [jumpTarget, fallthrough].filter((value): value is number => value != null),
  };
}

function sourceMapEntry(
  bytecodeRange: [number, number],
  sourceLines: [number, number],
  text: string,
  commandGroup: number,
  commandPath: number[],
  parentCommandGroup: number | null = null,
) {
  return {
    bytecodeRange,
    sourceLines,
    sourceChars: [0, text.length] as [number, number],
    text,
    commandGroup,
    commandPath,
    parentCommandGroup,
    originalStatus: "available" as const,
    originalSpan: [0, text.length] as [number, number],
  };
}

function fixtureSettings(overrides: Partial<Settings> = {}): Settings {
  return {
    theme: "tokyo-night",
    typeSystem: "mono-brutalist",
    fontFamily: "mono-brutalist",
    fontSize: 13,
    density: "regular",
    showSparklines: true,
    groupRails: true,
    panelColumns: {},
    debugger: {
      launch: {
        bytecodeHooks: {
          mode: "intercept",
          commandPreset: "tbcload",
          customCommands: [],
        },
      },
    },
    ...overrides,
  };
}

test("V1 graph view models expose basic-block and source-snippet modes with visual roles and cursor targets", async () => {
  const { buildGraphViewModel } = await import("@/lib/graph-model");
  const data = fixtureData();
  const basic = buildGraphViewModel(data, {
    mode: "basic-block",
    cursor: { blockId: 1, instIdx: 2, pc: 5, group: 1 },
    execution: { blockId: 0 },
  });
  const source = buildGraphViewModel(data, { mode: "source-snippet", cursor: null, execution: null });

  assert.equal(basic.mode, "basic-block");
  assert.equal(basic.nodes[0].kind, "basic-block");
  assert.equal(basic.nodes[0].flowRole, "flow-node");
  assert.deepEqual(new Set(basic.edges.map((edge) => edge.role)), new Set(["conditional-true", "conditional-false", "unconditional"]));
  assert.ok(basic.nodes.find((node) => node.blockId === 1)?.visualRoles.includes("selected"));
  assert.ok(basic.nodes.find((node) => node.blockId === 0)?.visualRoles.includes("execution"));
  assert.ok(basic.nodes.find((node) => node.blockId === 0)?.visualRoles.includes("predecessor"));
  assert.ok(basic.nodes.find((node) => node.blockId === 2)?.visualRoles.includes("successor"));
  assert.equal(source.mode, "source-snippet");
  assert.equal(source.nodes[0].kind, "source-snippet");
  assert.equal(source.nodes[0].commandPath, "c000");
  assert.equal(source.nodes[0].cursorTarget.sourceLine, 0);
});

test("V2 graph render hooks expose action ids, stable node/edge data attributes, and role classes", async () => {
  const { buildGraphViewModel, graphEdgeRenderAttrs, graphNodeRenderAttrs, getGraphActionIds } = await import("@/lib/graph-model");
  const graph = buildGraphViewModel(fixtureData(), {
    mode: "basic-block",
    cursor: { blockId: 1, instIdx: 2, pc: 5, group: 1 },
    execution: { blockId: 0 },
  });
  const selected = graph.nodes.find((node) => node.blockId === 1)!;
  const edge = graph.edges.find((candidate) => candidate.role === "conditional-true")!;

  assert.deepEqual(getGraphActionIds().sort(), [
    "graph:center-selection",
    "graph:copy-details",
    "graph:detail-compact",
    "graph:detail-full",
    "graph:detail-medium",
    "graph:expand-node",
    "graph:fit",
    "graph:mode-basic-block",
    "graph:mode-source-snippet",
    "graph:zoom-in",
    "graph:zoom-out",
  ].sort());
  assert.equal(graphNodeRenderAttrs(selected)["data-graph-mode"], "basic-block");
  assert.equal(graphNodeRenderAttrs(selected)["data-visual-selected"], "true");
  assert.match(graphNodeRenderAttrs(selected).className, /graph-role-selected/);
  assert.equal(graphEdgeRenderAttrs(edge)["data-edge-role"], "conditional-true");
  assert.match(graphEdgeRenderAttrs(edge).className, /graph-edge-conditional-true/);
});

test("V2 graph toolbar production controls execute stable graph action ids", () => {
  const source = readProductionFile("../src/components/panels/GraphPanel.tsx");

  for (const actionId of [
    "graph:mode-basic-block",
    "graph:mode-source-snippet",
    "graph:detail-compact",
    "graph:detail-medium",
    "graph:detail-full",
    "graph:fit",
    "graph:center-selection",
    "graph:zoom-in",
    "graph:zoom-out",
  ]) {
    assert.match(source, new RegExp(`actionId: "${actionId}"|commandRegistry\\.execute\\("${actionId}"\\)`), `toolbar must name ${actionId}`);
  }
  assert.match(source, /commandRegistry\.execute\(actionId\)/);
  assert.doesNotMatch(source, /onClick=\{\(\) => setGraphMode\(mode\)\}/);
  assert.doesNotMatch(source, /onClick=\{fitToView\}/);
});

test("V3 folding models preserve selected rows while folding source ranges and instruction commands", async () => {
  const { buildDecompileFoldBlocks, buildInstructionFoldModel, buildInstructionPanelRows, buildSourceFoldModel } = await import("@/lib/folding-model");
  const data = fixtureData();
  const source = buildSourceFoldModel(data, { foldedIds: new Set(["source:1"]), selectedSourceLine: 1 });
  const decompile = buildDecompileFoldBlocks(data);
  const disassembly = buildInstructionFoldModel(data, { foldedIds: new Set(["block:1", "group:1"]), selectedInstIdx: 3, filter: "all" });
  const productionRows = buildInstructionPanelRows(disassembly, data.instructions.map((inst) => inst.idx));

  assert.equal(decompile.get(0), 2);
  assert.equal(decompile.get(1), undefined);
  assert.equal(decompile.get(3), 3);
  assert.equal(decompile.get(4), undefined);
  assert.equal(source.foldRegions.find((region) => region.id === "source:1")?.commandPath, "c000/n001");
  const sourceFoldRow = source.visibleRows.find((row) => row.kind === "fold" && row.id === "source:1");
  assert.ok(sourceFoldRow);
  assert.equal(sourceFoldRow.kind, "fold");
  if (sourceFoldRow.kind !== "fold") throw new Error("expected folded source row");
  assert.equal(sourceFoldRow.hiddenLineCount, 1);
  assert.equal(source.selectedVisibleRowId, "source:1");
  assert.equal(disassembly.foldRegions.find((region) => region.id === "block:1"), undefined);
  assert.equal(disassembly.foldRegions.find((region) => region.id === "group:1")?.kind, "command-group");
  assert.equal(disassembly.visibleRows.find((row) => row.instIdx === 2)?.foldId, "group:1");
  assert.equal(disassembly.visibleRows.find((row) => row.instIdx === 2)?.hiddenInstructionCount, 2);
  assert.equal(disassembly.visibleRows.some((row) => row.instIdx === 3), false);
  assert.equal(disassembly.selectedVisibleRowId, "inst:2");
  assert.equal(productionRows.rows.find((row) => row.instIdx === 2)?.hiddenInstructionCount, 2);
  assert.equal(productionRows.visibleIndexByInstIdx.get(3), productionRows.rows.findIndex((row) => row.instIdx === 2));
});

test("V3 source and disassembly fold commands route through production panel fold state", async () => {
  const { dispatchPanelAction, subscribePanelAction } = await import("@/lib/panel-action-bus");
  const sourcePanel = readProductionFile("../src/components/panels/SourcePanel.tsx");
  const decompilePanel = readProductionFile("../src/components/panels/DecompilePanel.tsx");
  const instructionPanel = readProductionFile("../src/components/panels/InstructionPanel.tsx");
  const page = readProductionFile("../src/app/page.tsx");
  const calls: string[] = [];
  const unsubscribe = subscribePanelAction("source:fold-all", () => calls.push("fold"));
  const unsubscribeInstruction = subscribePanelAction("instructions:fold-all", () => calls.push("instruction-fold"));

  try {
    assert.equal(dispatchPanelAction("source:fold-all"), true);
    assert.equal(dispatchPanelAction("instructions:fold-all"), true);
    assert.deepEqual(calls, ["fold", "instruction-fold"]);
    assert.match(sourcePanel, /subscribePanelAction\("source:fold-all"/);
    assert.match(sourcePanel, /commandRegistry\.execute\("source:fold-all"\)/);
    assert.match(decompilePanel, /buildDecompileFoldBlocks\(data\)/);
    assert.doesNotMatch(decompilePanel, /function detectFoldableBlocks/);
    assert.match(instructionPanel, /subscribePanelAction\("instructions:fold-all"/);
    assert.match(instructionPanel, /commandRegistry\.execute\("instructions:fold-all"\)/);
    assert.match(instructionPanel, /commandRegistry\.execute\("instructions:unfold-all"\)/);
    assert.match(instructionPanel, /data-fold-disclosure="command"/);
    assert.match(instructionPanel, /data-fold-state=\{folded \? "collapsed" : "expanded"\}/);
    assert.match(instructionPanel, /columns\.breakpoint[\s\S]{0,900}data-fold-disclosure="command"/);
    assert.doesNotMatch(instructionPanel, /row\.kind === "fold"/);
    assert.match(page, /dispatchPanelAction\("source:fold-all"\)/);
    assert.match(page, /dispatchPanelAction\("instructions:fold-all"\)/);
    assert.match(page, /dispatchPanelAction\("instructions:unfold-all"\)/);
    assert.doesNotMatch(page, /id: "source:fold-all"[\s\S]{0,120}run: \(\) => setState/);
    assert.doesNotMatch(page, /id: "instructions:fold-all"[\s\S]{0,120}run: \(\) => setState/);
    assert.doesNotMatch(instructionPanel, /onClick=\{\(\) => setFoldedIds/);
  } finally {
    unsubscribe();
    unsubscribeInstruction();
  }
});

test("V4 syntax rendering emits deterministic source and instruction token roles tied to theme tokens", async () => {
  const { tokenizeInstructionRow, tokenizeSourceLine } = await import("@/lib/syntax-rendering");
  const data = fixtureData();
  const instruction = tokenizeInstructionRow(data.instructions[1], { groupLabel: "c000", comment: "jump if false", execution: true });
  const source = tokenizeSourceLine("if {$x} { puts yes } # branch", []);

  assert.deepEqual(instruction.map((token) => token.role), ["address", "opcode", "operand", "jump-target", "group-label", "comment", "runtime-marker"]);
  assert.equal(source.map((token) => token.text).join(""), "if {$x} { puts yes } # branch");
  for (const role of ["tcl-command", "tcl-variable", "tcl-brace", "tcl-string", "tcl-comment"]) {
    assert.ok(source.some((token) => token.role === role), `missing source token role ${role}`);
  }
  assert.ok(instruction.every((token) => token.themeToken.startsWith("--color-tcld-")));
  assert.ok(source.every((token) => token.className.startsWith("syntax-token-")));
});

test("V5 action inventory and registry metadata cover user-facing action classes, palette visibility, context, shortcut conflicts, and executable ids", async () => {
  const { commandRegistry } = await import("@/lib/command-registry");
  const { buildActionInventory, requiredActionClasses } = await import("@/lib/action-inventory");
  const { executeNativeMenuAction, nativeMenuActionId } = await import("@/lib/gui-workflows");
  const calls: string[] = [];
  const commands: CommandDef[] = [
    ...contractActionCommands(calls),
    { id: "test:shortcut-conflict", title: "Shortcut Conflict", category: "Graph", shortcut: ["f"], when: "focusedPanel:graph", run: () => calls.push("conflict") },
  ];
  const cleanup = commands.map((command) => commandRegistry.register(command));
  try {
    const incomplete = buildActionInventory([{ id: "graph:fit", title: "Graph: Fit", category: "Graph", run: () => {} }]);
    assert.equal(incomplete.classes.has("native-menu"), false, "inventory must not synthesize missing action classes");
    const inventory = buildActionInventory(commandRegistry.getAll());
    for (const actionClass of requiredActionClasses()) {
      assert.ok(inventory.classes.has(actionClass), `missing action class ${actionClass}`);
    }
    assert.ok(inventory.paletteVisibleIds.includes("graph:fit"));
    assert.ok(inventory.contexts.get("graph:fit")?.includes("focusedPanel:graph"));
    assert.ok(commandRegistry.execute("graph:fit"));
    assert.deepEqual(calls, ["graph:fit"]);
    assert.equal(nativeMenuActionId("debug_continue"), "debug:continue");
    assert.equal(executeNativeMenuAction("debug_continue", (id) => commandRegistry.execute(id)), true);
    assert.deepEqual(calls, ["graph:fit", "debug:continue"]);
    assert.ok(inventory.shortcutConflicts.some((conflict) => conflict.shortcut === "f"));
  } finally {
    cleanup.forEach((fn) => fn());
  }
});

test("V6 appearance preview and Cmd-click navigation update theme token surfaces and report unavailable targets without moving cursor", async () => {
  const { buildAppearancePreviewModel, updateAppearanceSettings } = await import("@/lib/appearance-workflows");
  const { resolveCmdClickNavigation } = await import("@/lib/source-navigation");
  const next = updateAppearanceSettings(fixtureSettings(), {
    theme: "dracula",
    fontFamily: "modern-sans",
    fontSize: 15,
    density: "compact",
  });
  const preview = buildAppearancePreviewModel(next);
  const hit = resolveCmdClickNavigation(fixtureData(), { tokenText: "if", sourceLine: 1, column: 1 });
  const miss = resolveCmdClickNavigation(fixtureData(), { tokenText: "unknown", sourceLine: 2, column: 1 });

  assert.equal(next.theme, "dracula");
  assert.equal(preview.sourceTokens.find((token) => token.role === "tcl-command")?.themeToken, "--color-tcld-keyword");
  assert.equal(preview.instructionTokens.find((token) => token.role === "opcode")?.themeToken, "--color-tcld-keyword");
  assert.equal(preview.graphTokens.edgeRoles["conditional-true"], "--color-tcld-success");
  assert.equal(hit.ok, true);
  assert.equal(hit.target.sourceLine, 1);
  assert.equal(miss.ok, false);
  assert.match(miss.reason, /No navigation target/);
});

test("V8 launch profiles persist complete debug launch options, feed launch requests, expose profile actions, and share main/profile-window messages", async () => {
  const { buildDebugLaunchRequestForProfile, buildSelectedLaunchProfileRequest, createLaunchProfileStore, launchProfileActionIds, reduceLaunchProfileWindowMessage } = await import("@/lib/launch-profiles");
  const store = createLaunchProfileStore({
    storage: memoryStorage(),
    settings: fixtureSettings(),
  });
  const profile = store.saveProfile({
    id: "profile:demo",
    name: "Demo",
    path: "/tmp/demo.tcl",
    args: "--flag two",
    env: "A=1 B=2",
    cwd: "/tmp",
    tclshPath: "/usr/bin/tclsh",
    bytecodeHooks: { mode: "intercept", commandPreset: "tbcload", customCommands: [] },
  });
  store.selectProfile(profile.id);
  const request = buildDebugLaunchRequestForProfile(profile, fixtureSettings());
  const selectedRequest = buildSelectedLaunchProfileRequest(store.snapshot(), fixtureSettings());
  const afterWindowSave = reduceLaunchProfileWindowMessage(store.snapshot(), { type: "save", profile: { ...profile, name: "Edited" } });

  assert.deepEqual(launchProfileActionIds().sort(), [
    "debug:launch-ad-hoc",
    "debug:launch-profile",
    "launch-profile:create",
    "launch-profile:delete",
    "launch-profile:edit",
    "launch-profile:open-window",
  ].sort());
  assert.deepEqual(request.args, ["--flag", "two"]);
  assert.deepEqual(selectedRequest?.args, request.args);
  assert.deepEqual(request.launchOptions.env, { A: "1", B: "2" });
  assert.equal(request.launchOptions.cwd, "/tmp");
  assert.equal(request.launchOptions.bytecodeHookMode, "intercept");
  assert.equal(afterWindowSave.profiles.find((candidate) => candidate.id === profile.id)?.name, "Edited");
});

test("V8 run-configuration Tauri window has a real frontend route that saves, cancels, deletes, imports, and exports through shared events", () => {
  const page = readProductionFile("../src/app/page.tsx");

  assert.match(page, /windowMode === "run-configuration"/);
  assert.match(page, /function RunConfigurationWindowApp\(/);
  assert.match(page, /createRunConfigurationStore\(\{ storage: browserStorage\(\), settings \}\)/);
  assert.match(page, /listen<RunConfigurationProfile \| null>\("run-configuration:edit"/);
  assert.match(page, /emitTauriEvent\("run-configuration:save"/);
  assert.match(page, /emitTauriEvent\("run-configuration:delete"/);
  assert.match(page, /emitTauriEvent\("run-configuration:cancel"/);
  assert.match(page, /emitTauriEvent\("run-configuration:import"/);
  assert.match(page, /emitTauriEvent\("run-configuration:export"/);
});

test("V8 detached window routing uses hydration-safe external location state", () => {
  const page = readProductionFile("../src/app/page.tsx");

  assert.doesNotMatch(page, /useState\(\s*currentWindowMode\s*\)/);
  assert.doesNotMatch(page, /useState\(\s*currentVisualParityState\s*\)/);
  assert.doesNotMatch(page, /setRouteState\(/);
  assert.match(page, /useSyncExternalStore\(/);
  assert.match(page, /serverWindowSearch/);
  assert.match(page, /readWindowRouteState\(search\)/);
});

test("command-level run to cursor preserves source offset and length", () => {
  const store = readProductionFile("../src/lib/debug-store.tsx");
  const workspace = readProductionFile("../src/components/MosaicWorkspace.tsx");

  assert.match(
    store,
    /runToCursor:\s*\([\s\S]*?command\?: \{ offset: number; len: number \}/,
  );
  assert.match(
    store,
    /await debugSetBreakpoint\([\s\S]*?command\?\.offset,[\s\S]*?command\?\.len/,
  );
  assert.match(
    store,
    /tempBreakpointKey\(file, line, command\?\.offset\)/,
  );
  assert.match(
    workspace,
    /debug\.runToCursor\(debug\.scriptPath, command\.line, command\)/,
  );
});

test("V9 keybinding persistence imports and exports action-id bindings with validation, reset, override, and conflict diagnostics", async () => {
  const { commandRegistry } = await import("@/lib/command-registry");
  const { createKeybindingStore } = await import("@/lib/keybindings");
  const calls: string[] = [];
  const cleanup = [
    commandRegistry.register({ id: "open", title: "Open", category: "File", shortcut: ["Mod+o"], run: () => calls.push("open") }),
    commandRegistry.register({ id: "graph:fit", title: "Graph Fit", category: "Graph", shortcut: ["f"], when: "focusedPanel:graph", run: () => calls.push("fit") }),
  ];
  const store = createKeybindingStore({
    storage: memoryStorage(),
    actions: [
      { id: "open", defaultShortcuts: ["Mod+o"] },
      { id: "graph:fit", defaultShortcuts: ["f"] },
      { id: "source:fold-all", defaultShortcuts: [] },
    ],
  });

  store.setBinding("graph:fit", ["Mod+g"]);
  assert.deepEqual(store.getEffectiveShortcuts("graph:fit"), ["Mod+g"]);
  const exported = store.exportJson();
  const imported = store.importJson(JSON.stringify({ bindings: { open: ["Mod+g"], "source:fold-all": ["Mod+Shift+["] } }));
  const unknown = store.importJson(JSON.stringify({ bindings: { "missing:action": ["Mod+x"] } }));
  const malformed = store.importJson(JSON.stringify({ bindings: { open: ["Mod+"] } }));

  assert.match(exported, /"graph:fit"/);
  assert.equal(imported.ok, true);
  commandRegistry.setShortcutOverrides(store.shortcutOverrides());
  const context: CommandContext = {
    overlayOpen: false,
    focusedPanel: "graph",
    debugState: "idle",
    debugBusy: false,
    isExceptionStop: false,
    hasFile: true,
    hasSelection: true,
    debuggerAvailable: true,
  };
  // `Mod` resolves from `navigator.platform` at dispatch time: Meta on
  // macOS, Ctrl everywhere else. Pin the platform both ways instead of
  // inheriting the host's, so the assertion is deterministic on any
  // machine and covers both resolutions (dispatching a bare metaKey event
  // passed on macOS but failed on Linux runners).
  withNavigatorPlatform("MacIntel", () => {
    assert.equal(commandRegistry.handleKeydown(keyEvent("g", { metaKey: true }), context), true, "Mod+g dispatches via Meta on macOS");
    assert.equal(commandRegistry.handleKeydown(keyEvent("g", { ctrlKey: true }), context), false, "Ctrl must not satisfy Mod on macOS");
  });
  withNavigatorPlatform("Linux x86_64", () => {
    assert.equal(commandRegistry.handleKeydown(keyEvent("g", { ctrlKey: true }), context), true, "Mod+g dispatches via Ctrl on Linux");
    assert.equal(commandRegistry.handleKeydown(keyEvent("g", { metaKey: true }), context), false, "Meta must not satisfy Mod on Linux");
  });
  assert.deepEqual(calls, ["fit", "fit"]);
  assert.ok(store.conflicts().some((conflict) => conflict.actionIds.includes("open") && conflict.actionIds.includes("graph:fit")));
  assert.equal(unknown.ok, false);
  assert.match(unknown.diagnostics[0].message, /unknown action/i);
  assert.equal(malformed.ok, false);
  assert.match(malformed.diagnostics[0].message, /malformed shortcut/i);
  store.resetToDefaults();
  commandRegistry.setShortcutOverrides(store.shortcutOverrides());
  assert.deepEqual(store.getEffectiveShortcuts("open"), ["Mod+o"]);
  cleanup.forEach((fn) => fn());
  commandRegistry.setShortcutOverrides({});
});

function contractActionCommands(calls: string[]): CommandDef[] {
  const command = (id: string, category: string, shortcut?: string[], when?: string): CommandDef => ({
    id,
    title: id,
    category,
    shortcut,
    when,
    run: () => calls.push(id.replace("native-menu:", "")),
  });
  return [
    command("open", "File", ["Mod+o"]),
    command("app:close-window", "App Shell"),
    command("toolbar:open-file", "Toolbar"),
    command("native-menu:file-open", "Native Menu"),
    command("native-menu:debug-continue", "Native Menu"),
    command("context-menu:copy-active-panel", "Context Menu"),
    command("panel:close", "Panels"),
    command("nav:cycle_focus", "Navigation", ["f"], "!overlayOpen"),
    command("layout:default", "Layout"),
    command("appearance:open", "Settings"),
    command("graph:fit", "Graph", ["f"], "focusedPanel:graph"),
    command("source:fold-all", "Source"),
    command("instructions:fold-all", "Disassembly"),
    command("decompile:copy", "Decompile"),
    command("provenance:copy", "Provenance"),
    command("diagnostics:copy", "Diagnostics"),
    command("detail:copy", "Detail"),
    command("debug:continue", "Debug"),
    command("breakpoints:toggle", "Breakpoint"),
    command("watchpoints:create", "Watch"),
    command("variables:copy", "Variable"),
    command("stack:copy", "Stack"),
    command("console:copy", "Console"),
    command("format:apply", "Format"),
    command("mark:set:a", "Mark"),
    command("pin:toggle_a", "Pin"),
    command("debug:launch-profile", "Launch Profiles"),
    command("keybindings:import", "Keybinding"),
    command("palette:open", "Command Palette"),
  ];
}

/**
 * Run `fn` with `navigator.platform` pinned to `platform`, restoring the
 * host's real navigator afterwards. The command registry resolves `Mod`
 * shortcuts from navigator.platform, so pinning makes Mod-dispatch
 * assertions deterministic instead of tracking the machine running the
 * suite (Node >= 21 exposes a real global navigator).
 */
function withNavigatorPlatform(platform: string, fn: () => void): void {
  const original = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { platform } });
  try {
    fn();
  } finally {
    if (original) Object.defineProperty(globalThis, "navigator", original);
    else Reflect.deleteProperty(globalThis, "navigator");
  }
}

function keyEvent(
  key: string,
  opts: Partial<Pick<KeyboardEvent, "ctrlKey" | "metaKey" | "altKey" | "shiftKey">> = {},
): KeyboardEvent {
  return {
    key,
    ctrlKey: opts.ctrlKey ?? false,
    metaKey: opts.metaKey ?? false,
    altKey: opts.altKey ?? false,
    shiftKey: opts.shiftKey ?? false,
  } as KeyboardEvent;
}

function memoryStorage() {
  const data = new Map<string, string>();
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
    removeItem: (key: string) => {
      data.delete(key);
    },
  };
}

function readProductionFile(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}
