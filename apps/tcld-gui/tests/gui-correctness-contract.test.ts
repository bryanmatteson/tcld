import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  buildInstructionDetailFromData,
  buildSessionDocument,
  createSessionState,
  selectActiveDocument,
  upsertSessionDocument,
} from "@/lib/store-data";
import { createOutputSearchState, formatOutputForCopy, searchOutput } from "@/lib/output-search";
import {
  buildFullReport,
  resolveBreakpointTarget,
} from "@/lib/workflow-contracts";
import {
  applyFormattedTextToData,
  buildDebugLaunchRequest,
  buildExportRequests,
  dispatchNativeMenuAction,
  resolveDroppedFile,
  summarizeFormatPreview,
} from "@/lib/gui-workflows";
import {
  buildComparisonPanelModel,
  buildDiagnosticsPanelModel,
  buildDiagnosticsSummaryModel,
  buildSymbolsPanelModel,
  buildProvenancePanelModel,
  buildSourcePanelModel,
  buildTracePanelModel,
  sourceMapSelectionTarget,
} from "@/lib/panel-models";
import { buildGraphViewModel, classifyGraphEdge, computeGraphMetrics, deriveGraphBlockRole } from "@/lib/graph-model";
import { ingestLiveLoadSnapshot, nextBreakpointEnabledState } from "@/lib/debug-workflows";
import {
  __setInvokeForTests,
  debugEval,
  debugGetStack,
  debugGetVariables,
  debugLaunch,
  debugResolveCommand,
  debugSetBreakpointEnabled,
  debugSetWatchpoint,
} from "@/lib/tauri-api";
import type { DebugBreakpoint, TcldData } from "@/types";

function fixtureData(overrides: Partial<TcldData> = {}): TcldData {
  return {
    metadata: {
      filename: "fixture.tbc",
      path: "/tmp/fixture.tbc",
      tclVersion: "8.6",
      instructionCount: 2,
      literalCount: 0,
      commandCount: 1,
      exceptionCount: 0,
      auxCount: 0,
      codeBytes: 5,
      maxStackDepth: 2,
      syntaxEngine: "tree-sitter-tcl",
      format: 1,
      build: 0,
    },
    instructions: [
      {
        idx: 0,
        pc: 0,
        op: "push1",
        args: "0",
        operands: [0],
        argc: 0,
        group: 0,
        groupStart: true,
        comment: "x",
        size: 2,
        isJump: false,
        isJumpConditional: false,
        isJumpUnconditional: false,
        jumpTarget: null,
        fallthrough: 2,
        primaryNextLocation: 2,
        secondaryNextLocation: null,
        nextLocations: [2],
        commandPcRange: [0, 5],
      },
      {
        idx: 1,
        pc: 2,
        op: "jumpFalse1",
        args: "3",
        operands: [3],
        argc: 1,
        group: 0,
        groupStart: false,
        comment: "pc 5",
        size: 3,
        isJump: true,
        isJumpConditional: true,
        isJumpUnconditional: false,
        jumpTarget: 5,
        fallthrough: 5,
        primaryNextLocation: 5,
        secondaryNextLocation: 5,
        nextLocations: [5, 5],
        commandPcRange: [0, 5],
      },
    ],
    literals: [],
    exceptions: [],
    blocks: [
      {
        id: 0,
        start: 0,
        end: 1,
        startPc: 0,
        endPc: 5,
        group: "c000",
        edges: [1, 2],
        instructions: [{ pc: 0, name: "push1", operands: [0] }],
      },
      {
        id: 1,
        start: 1,
        end: 1,
        startPc: 5,
        endPc: 6,
        group: "c001",
        edges: [],
        instructions: [{ pc: 5, name: "done", operands: [] }],
      },
      {
        id: 2,
        start: 1,
        end: 1,
        startPc: 8,
        endPc: 9,
        group: "c002",
        edges: [],
        instructions: [{ pc: 8, name: "fallthrough", operands: [] }],
      },
    ],
    traces: [
      { instructionIndex: 0, pc: 0, opcode: "push1", argc: 0, stackDelta: 1, beforeDepth: 0, afterDepth: 1, popped: [], pushed: ["x"] },
      { instructionIndex: 1, pc: 2, opcode: "jumpFalse1", argc: 1, stackDelta: -1, beforeDepth: 1, afterDepth: 0, popped: ["x"], pushed: [] },
    ],
    decompileLines: [{ text: "if {$x} {puts ok}", group: 0, lineIndex: 0, sourceLineStart: 7, sourceLineEnd: 7, sourceMapIndex: 0 }],
    decompileTokens: [],
    decompileText: "if {$x} {puts ok}\n",
    dumpText: "Instructions:\n",
    summaryText: "snapshot: canonical-domain\n",
    sourceMap: [
      {
        bytecodeRange: [0, 5],
        sourceLines: [0, 1],
        sourceChars: [0, 17],
        text: "if {$x} {puts ok}",
        commandGroup: 0,
        commandPath: [0],
        parentCommandGroup: null,
        originalStatus: "available",
        originalSpan: [11, 28],
      },
    ],
    diagnostics: {
      reconstruction: "Strict",
      recognizedConstructs: ["if"],
      unreducedOpcodes: [],
      approximations: [],
      suppressedFastPathRanges: [],
      passDecisions: [{ name: "conditionals", enabled: true, iteration: 0, changed: true }],
      unsupportedInstructions: [],
      suggestedNextWork: [],
    },
    reconstruction: "Strict",
    provenance: { kind: "Path", label: "/tmp/fixture.tbc", originalSourceAvailable: true },
    originalSourceText: "set x 1\nif {$x} {puts ok}\n",
    ...overrides,
  };
}

test("active snapshot detail is derived from TcldData without backend global state", () => {
  const detail = buildInstructionDetailFromData(fixtureData(), 1);
  assert.equal(detail?.commandPcRange, "0..5");
  assert.deepEqual(detail?.operands, [3]);
  assert.deepEqual(detail?.jumps, { conditional: true, unconditional: false });
  assert.equal(detail?.fallthrough, "5");
  assert.match(detail?.value ?? "", /popped: \[x\]/);
});

test("diagnostics summary is derived from loaded TcldData", () => {
  const data = fixtureData({
    diagnostics: {
      ...fixtureData().diagnostics!,
      approximations: [{ kind: "UnresolvedInstruction", bytecodeRange: [0, 3], reason: "stack expression folded" }],
      unsupportedInstructions: ["unsupportedDemo"],
      unreducedOpcodes: ["unknownOpcode"],
      suppressedFastPathRanges: [[1, 3]],
      passDecisions: [{ name: "reduce", enabled: false, iteration: 1, changed: true }],
    },
  });

  const diagnostics = buildDiagnosticsSummaryModel(data);
  assert.equal(diagnostics.available, true);
  assert.equal(diagnostics.status, "issues");
  assert.equal(diagnostics.rows.find((row) => row.kind === "approximation")?.count, 1);
  assert.equal(diagnostics.rows.find((row) => row.kind === "unsupported")?.status, "unsupportedDemo");
  assert.equal(diagnostics.rows.find((row) => row.kind === "unreduced")?.status, "unknownOpcode");
  assert.equal(diagnostics.rows.find((row) => row.kind === "suppressed")?.status, "1 suppressed");
  assert.equal(diagnostics.rows.find((row) => row.kind === "pass")?.status, "1 changed · 1 disabled");

  const cleanDiagnostics = buildDiagnosticsSummaryModel(fixtureData({
    diagnostics: {
      reconstruction: "Strict",
      recognizedConstructs: [],
      unreducedOpcodes: [],
      approximations: [],
      suppressedFastPathRanges: [],
      passDecisions: [],
      unsupportedInstructions: [],
      suggestedNextWork: [],
    },
  }));
  assert.equal(cleanDiagnostics.status, "ok");
  assert.equal(cleanDiagnostics.message, "no approximations · all opcodes reduced · all fast paths enabled");

  const unavailableDiagnostics = buildDiagnosticsSummaryModel(fixtureData({ diagnostics: undefined }));
  assert.equal(unavailableDiagnostics.status, "unavailable");
  assert.equal(unavailableDiagnostics.message, "diagnostics unavailable for this load");
});

test("symbols model derives namespace, proc, and variable rows from loaded TcldData", () => {
  const data = fixtureData({
    sourceMap: [
      {
        bytecodeRange: [0, 12],
        sourceLines: [0, 4],
        sourceChars: [0, 48],
        text: "namespace eval fixture {\n    set debug_level 1\n}",
        commandGroup: 0,
        commandPath: [0],
        parentCommandGroup: null,
        originalStatus: "available",
        originalSpan: [0, 48],
      },
      {
        bytecodeRange: [12, 30],
        sourceLines: [4, 8],
        sourceChars: [49, 120],
        text: "proc fixture::startlog { _text } {\n    variable baseDir\n}",
        commandGroup: 1,
        commandPath: [1],
        parentCommandGroup: null,
        originalStatus: "available",
        originalSpan: [49, 120],
      },
      {
        bytecodeRange: [18, 24],
        sourceLines: [5, 6],
        sourceChars: [80, 96],
        text: "variable baseDir",
        commandGroup: 2,
        commandPath: [1, 0],
        parentCommandGroup: 1,
        originalStatus: "available",
        originalSpan: [80, 96],
      },
    ],
  });

  const model = buildSymbolsPanelModel(data);
  assert.equal(model.status, "available");
  assert.deepEqual(
    model.rows.map((row) => [row.kind, row.name]),
    [
      ["ns", "fixture"],
      ["proc", "fixture::startlog"],
      ["var", "baseDir"],
    ],
  );
  assert.equal(model.rows.find((row) => row.kind === "proc")?.addressLabel, "000c");
  assert.equal(model.rows.find((row) => row.kind === "proc")?.size, 18);
  assert.equal(model.rows.find((row) => row.kind === "proc")?.refs, null);

  const filtered = buildSymbolsPanelModel(data, "START");
  assert.equal(filtered.visibleCount, 1);
  assert.equal(filtered.visibleRows[0]?.name, "fixture::startlog");
});

test("symbols model reports deterministic unavailable state without symbol facts", () => {
  const model = buildSymbolsPanelModel(fixtureData({ sourceMap: [], sourceCommandAnchors: [] }), "anything");
  assert.equal(model.status, "unavailable");
  assert.equal(model.unavailableTitle, "Symbols unavailable");
  assert.equal(model.unavailableExplanation, "This payload does not expose namespace/proc/var symbol facts yet.");
  assert.equal(model.visibleCount, 0);
  assert.equal(model.noMatchText, null);
});

test("session model preserves static and live documents with active document selection", () => {
  let session = createSessionState();
  const staticDoc = buildSessionDocument(fixtureData(), { role: "open", label: "fixture.tbc" });
  const liveDoc = buildSessionDocument(
    fixtureData({
      metadata: { ...fixtureData().metadata, filename: "live#1", path: "live#1" },
      provenance: { kind: "DebuggerChunk", label: "live#1", originalSourceAvailable: false },
    }),
    { role: "live", label: "live#1" },
  );

  session = upsertSessionDocument(session, staticDoc);
  session = upsertSessionDocument(session, liveDoc);
  session = selectActiveDocument(session, staticDoc.id);

  assert.equal(session.documents.length, 2);
  assert.equal(session.activeDocumentId, staticDoc.id);
  assert.deepEqual(session.documents.map((doc) => doc.role), ["open", "live"]);
  assert.equal(session.documents[1].provenance.originalSourceAvailable, false);
});

test("debugger live_load ingestion updates active live snapshot detail through the production ingestion path", () => {
  const active: { value: TcldData | null } = { value: null };
  const live = fixtureData({
    metadata: { ...fixtureData().metadata, filename: "live#1", path: "live#1" },
    instructions: fixtureData().instructions.map((inst, index) => index === 1 ? { ...inst, operands: [9], commandPcRange: [2, 9] } : inst),
    provenance: { kind: "DebuggerChunk", label: "live#1", originalSourceAvailable: false },
    originalSourceText: null,
  });

  ingestLiveLoadSnapshot((data) => { active.value = data; }, live);

  assert.equal(active.value?.provenance?.kind, "DebuggerChunk");
  const detail = active.value ? buildInstructionDetailFromData(active.value, 1) : null;
  assert.deepEqual(detail?.operands, [9]);
  assert.equal(detail?.commandPcRange, "2..9");
});

test("source, comparison, diagnostics, and provenance models use original text and linked source-map selection", () => {
  const data = fixtureData({
    diagnostics: {
      ...fixtureData().diagnostics!,
      approximations: [{ kind: "UnresolvedInstruction", bytecodeRange: [0, 3], reason: "stack expression folded" }],
    },
  });
  const cursor = { scopeId: "fixture", instIdx: 1, pc: 2, group: 0, blockId: 0, decompileLine: null, sourceFile: "/tmp/source.tcl", sourceLine: 7 };
  const source = buildSourcePanelModel(data, cursor);
  const comparison = buildComparisonPanelModel(data, cursor);
  const diagnostics = buildDiagnosticsPanelModel(data, cursor);
  const provenance = buildProvenancePanelModel(data, cursor);
  const target = sourceMapSelectionTarget(data, 0);

  assert.equal(source.available, true);
  assert.equal(source.text, data.originalSourceText);
  assert.notEqual(source.text, data.decompileText);
  assert.equal(source.sourceMapRows[0].selected, true);
  assert.equal(comparison?.original.available, true);
  assert.equal(comparison?.original.text, data.originalSourceText);
  assert.equal(comparison?.selection.sourceMapIndex, 0);
  assert.equal(diagnostics.linkedRows.find((row) => row.kind === "approximation")?.selected, true);
  assert.equal(provenance.rows[0].selected, true);
  assert.equal(target?.instIdx, 0);
});

test("source and comparison models expose unavailable original source without substituting reconstructed lines", () => {
  const data = fixtureData({
    provenance: { kind: "DebuggerChunk", label: "live#1", originalSourceAvailable: false },
    originalSourceText: null,
    sourceMap: [
      {
        ...fixtureData().sourceMap![0],
        originalStatus: "unavailable",
        originalSpan: null,
      },
    ],
  });
  const source = buildSourcePanelModel(data, null);
  const comparison = buildComparisonPanelModel(data, null);
  const provenance = buildProvenancePanelModel(data, null);
  const report = buildFullReport({
    data,
    session: upsertSessionDocument(createSessionState(), buildSessionDocument(data, { role: "live", label: "live#1" })),
    output: [{ stream: "system", text: "paused", timestamp: 1 }],
    stoppedAt: { reason: "watchpoint", file: "a.tcl", line: 7, frameCount: 2, pc: 0, instructionIndex: 0, blockId: 0, watchpointName: "x", watchpointOperation: "write", watchpointOldValue: "1", watchpointNewValue: "2" },
  });

  assert.equal(source.available, false);
  assert.equal(source.text, null);
  assert.equal(comparison?.original.available, false);
  assert.equal(comparison?.original.text, null);
  assert.equal(provenance.rows[0].originalStatus, "unavailable");
  assert.match(report, /Original Source: unavailable/);
  assert.match(report, /Watchpoint: x write 1 -> 2/);
  assert.match(report, /Open Documents/);
});

test("console search and copy preserve stream labels across filtered output", () => {
  const output = [
    { stream: "stdout" as const, text: "hello world", timestamp: 1 },
    { stream: "stderr" as const, text: "warning world", timestamp: 2 },
    { stream: "eval" as const, text: "> expr {$world}", timestamp: 3 },
  ];
  const search = searchOutput(output, createOutputSearchState("world"));
  assert.deepEqual(search.matches.map((match) => match.lineIndex), [0, 1, 2]);
  assert.equal(search.visibleLines.length, 3);
  assert.equal(formatOutputForCopy(search.visibleLines), "[stdout] hello world\n[stderr] warning world\n[eval] > expr {$world}");
});

test("file, project, format, export, copy, drag-drop, and native menu workflows use active document data", () => {
  const data = fixtureData();
  const session = upsertSessionDocument(createSessionState(), buildSessionDocument(data, { role: "open", label: "fixture.tbc" }));
  const formatted = applyFormattedTextToData(data, "if {$x} {\n    puts ok\n}\n");
  const exports = buildExportRequests({
    data,
    session,
    output: [{ stream: "stdout", text: "hello", timestamp: 1 }],
    stoppedAt: null,
  });
  const dropped = resolveDroppedFile({ name: "fixture.tcl", path: "/tmp/fixture.tcl" });
  const noPath = resolveDroppedFile({ name: "fixture.tcl" });
  const launch = buildDebugLaunchRequest(
    "/tmp/app.tcl",
    { args: "--flag \"two words\"", env: "A=1 B=two", cwd: "/tmp", tclshPath: "/usr/bin/tclsh" },
    { bytecodeHookMode: "intercept", bytecodeHookCommands: ["expr"] },
  );
  const calls: string[] = [];
  dispatchNativeMenuAction(
    "debug_step_instruction",
    {
      openFile: () => calls.push("open"),
      closeWindow: () => calls.push("close"),
      resume: () => calls.push("resume"),
      stop: () => calls.push("stop"),
      stepInto: () => calls.push("stepInto"),
      stepInstruction: () => calls.push("stepInstruction"),
      stepOver: () => calls.push("stepOver"),
      stepOut: () => calls.push("stepOut"),
      toggleBreakpoint: () => calls.push("breakpoint"),
      restart: () => calls.push("restart"),
      clearAllBreakpoints: () => calls.push("clear"),
      launch: () => calls.push("launch"),
      openPalette: () => calls.push("palette"),
      addView: (viewId) => calls.push(`view:${viewId}`),
    },
    { isBusy: false, isExceptionStop: false, canStepInstruction: true },
  );
  const capabilityPath = path.resolve(process.cwd(), "../tcld-tauri/capabilities/default.json");
  const capabilities = JSON.parse(fs.readFileSync(capabilityPath, "utf8")) as { permissions: string[] };

  assert.equal(formatted.decompileLines[0].group, 0);
  assert.equal(formatted.decompileLines[0].sourceMapIndex, 0);
  assert.match(exports.comparison.contents, /Original Source/);
  assert.match(exports.comparison.contents, /set x 1/);
  assert.match(exports.fullReport.contents, /Open Documents/);
  assert.equal(exports.console.contents, "[stdout] hello");
  assert.deepEqual(dropped, { supported: true, path: "/tmp/fixture.tcl", filename: "fixture.tcl", reason: "ready" });
  assert.equal(noPath.reason, "path-unavailable");
  assert.deepEqual(launch.args, ["--flag", "two words"]);
  assert.deepEqual(launch.launchOptions.env, { A: "1", B: "two" });
  assert.equal(launch.launchOptions.cwd, "/tmp");
  assert.equal(launch.tclshPath, "/usr/bin/tclsh");
  assert.deepEqual(calls, ["stepInstruction"]);
  assert.equal(summarizeFormatPreview([{ startByte: 0, endByte: 1, replacement: "x" }]), "1 formatting edit");
  assert.ok(capabilities.permissions.includes("dialog:allow-save"));
});

test("debugger IPC workflows cover launch config, menu/toolbar breakpoint target, condition-preserving toggles, watchpoints, stack, variables, eval, and command resolution", async () => {
  const data = fixtureData();
  const target = resolveBreakpointTarget({
    data,
    selectedInstructionIndex: 1,
    stoppedAt: { reason: "breakpoint", file: "fallback.tcl", line: 99, frameCount: 1, pc: null, instructionIndex: null, blockId: null },
    scriptPath: "/tmp/source.tcl",
  });
  assert.deepEqual(target, { file: "/tmp/source.tcl", line: 7, source: "selection" });

  const bp = { file: "/tmp/source.tcl", line: 7, condition: "$x > 1", enabled: true } satisfies DebugBreakpoint;
  const next = nextBreakpointEnabledState([bp], bp.file, bp.line);
  assert.deepEqual(next, { ...bp, enabled: false });
  assert.equal(next?.condition, "$x > 1");

  const invocations: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
  __setInvokeForTests(async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    invocations.push({ cmd, args });
    if (cmd === "debug_resolve_command") {
      return {
        originalName: "puts",
        resolvedName: "puts",
        fullyQualifiedName: "::puts",
        namespace: "::",
        resolvedInNamespace: "::",
        isImported: false,
        importChain: [],
        isEnsemble: false,
        isAlias: false,
        commandType: "native",
        exists: true,
        resolutionPath: [{ step: "global", result: "::puts" }],
      } as T;
    }
    return undefined as T;
  });

  try {
    await debugLaunch("/tmp/app.tcl", ["arg"], "/usr/bin/tclsh", { cwd: "/tmp", env: { A: "1" }, bytecodeHookMode: "intercept" });
    await debugSetBreakpointEnabled(bp.file, bp.line, false);
    await debugSetWatchpoint("x", "local", 1, "$x ne 0");
    await debugGetStack();
    await debugGetVariables(2);
    await debugEval("expr {$x}", 1);
    const resolved = await debugResolveCommand("puts", "::");
    assert.equal(resolved.fullyQualifiedName, "::puts");
  } finally {
    __setInvokeForTests(null);
  }

  assert.deepEqual(invocations.map((call) => call.cmd), [
    "debug_launch",
    "debug_set_breakpoint_enabled",
    "debug_set_watchpoint",
    "debug_get_stack",
    "debug_get_variables",
    "debug_eval",
    "debug_resolve_command",
  ]);
  assert.deepEqual(invocations[0].args, {
    path: "/tmp/app.tcl",
    args: ["arg"],
    tclshPath: "/usr/bin/tclsh",
    launchOptions: { cwd: "/tmp", env: { A: "1" }, bytecodeHookMode: "intercept" },
  });
  assert.deepEqual(invocations[1].args, { file: bp.file, line: bp.line, enabled: false });
  assert.deepEqual(invocations[2].args, { name: "x", scope: "local", level: 1, condition: "$x ne 0" });
});

test("graph edge classification uses backend jump facts rather than opcode-name regexes", () => {
  const data = fixtureData({
    instructions: fixtureData().instructions.map((inst, index) => index === 1 ? {
      ...inst,
      op: "mysteryBranchWithoutJumpName",
      isJump: true,
      isJumpConditional: true,
      isJumpUnconditional: false,
      jumpTarget: 5,
      fallthrough: 8,
      nextLocations: [5, 8],
    } : inst),
    blocks: [
      {
        id: 0,
        start: 1,
        end: 1,
        startPc: 2,
        endPc: 5,
        group: "c000",
        edges: [1, 2],
        instructions: [{ pc: 2, name: "notARegexJump", operands: [] }],
      },
      {
        id: 1,
        start: 1,
        end: 1,
        startPc: 5,
        endPc: 6,
        group: "c001",
        edges: [],
        instructions: [],
      },
      {
        id: 2,
        start: 1,
        end: 1,
        startPc: 8,
        endPc: 9,
        group: "c002",
        edges: [],
        instructions: [],
      },
    ],
  });

  assert.equal(classifyGraphEdge(data.blocks[0], 1, data.instructions, data.blocks), "conditional-true");
  assert.equal(classifyGraphEdge(data.blocks[0], 2, data.instructions, data.blocks), "conditional-false");
});

test("graph roles and metrics are derived from real block and edge facts", () => {
  const base = fixtureData();
  const data = fixtureData({
    instructions: [
      { ...base.instructions[0], idx: 0, pc: 0, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 10, nextLocations: [10] },
      { ...base.instructions[1], idx: 1, pc: 10, isJump: true, isJumpConditional: true, isJumpUnconditional: false, jumpTarget: 30, fallthrough: 20, nextLocations: [30, 20] },
      { ...base.instructions[0], idx: 2, pc: 20, isJump: true, isJumpConditional: false, isJumpUnconditional: true, jumpTarget: 10, fallthrough: null, nextLocations: [10] },
      { ...base.instructions[0], idx: 3, pc: 30, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: null, nextLocations: [] },
    ],
    blocks: [
      { id: 0, start: 0, end: 0, startPc: 0, endPc: 9, group: "c000", edges: [1], instructions: [{ pc: 0, name: "push1", operands: [] }] },
      { id: 1, start: 1, end: 1, startPc: 10, endPc: 19, group: "c001", edges: [2, 3], instructions: [{ pc: 10, name: "jumpFalse1", operands: [] }] },
      { id: 2, start: 2, end: 2, startPc: 20, endPc: 29, group: "c002", edges: [1], instructions: [{ pc: 20, name: "jump1", operands: [] }] },
      { id: 3, start: 3, end: 3, startPc: 30, endPc: 39, group: "c003", edges: [], instructions: [{ pc: 30, name: "done", operands: [] }] },
    ],
  });

  assert.deepEqual(
    data.blocks.map((block) => deriveGraphBlockRole(block, data.blocks, data.instructions)),
    ["entry", "branch", "loop", "exit"],
  );

  const metrics = computeGraphMetrics(data.blocks, data.instructions);
  assert.equal(metrics.blockCount, 4);
  assert.equal(metrics.edgeCount, 4);
  assert.equal(metrics.branchCount, 1);
  assert.equal(metrics.loopCount, 1);
  assert.equal(metrics.cyclomatic, 2);
  assert.equal(metrics.depth, 3);

  const model = buildGraphViewModel(data, { mode: "basic-block", cursor: { blockId: 1 }, execution: { blockId: null } });
  assert.equal(model.metrics.branchCount, 1);
  assert.equal(model.nodes.find((node) => node.blockId === 1)?.blockRole, "branch");
});

test("trace panel model derives commands, filter, and pass-chain detail from real diagnostics facts", () => {
  const data = fixtureData({
    sourceMap: [
      {
        bytecodeRange: [0, 14],
        sourceLines: [0, 1],
        sourceChars: [0, 23],
        text: "package provide fixture 1.1",
        commandGroup: 0,
        commandPath: [0],
        parentCommandGroup: null,
        originalStatus: "available",
        originalSpan: [0, 23],
      },
      {
        bytecodeRange: [14, 23],
        sourceLines: [1, 2],
        sourceChars: [24, 60],
        text: "if {$level >= 4} { set tag debug+ } else { set tag info }",
        commandGroup: 14,
        commandPath: [1],
        parentCommandGroup: null,
        originalStatus: "available",
        originalSpan: [24, 60],
      },
      {
        bytecodeRange: [23, 41],
        sourceLines: [2, 3],
        sourceChars: [61, 100],
        text: "foreach s $sources { fixture::log scan $s }",
        commandGroup: 23,
        commandPath: [2],
        parentCommandGroup: null,
        originalStatus: "available",
        originalSpan: [61, 100],
      },
    ],
    diagnostics: {
      reconstruction: "Strict",
      recognizedConstructs: ["loop:foreach", "conditional:if"],
      unreducedOpcodes: [],
      approximations: [{ kind: "UnresolvedInstruction", bytecodeRange: [23, 41], reason: "loop body omitted" }],
      suppressedFastPathRanges: [],
      passDecisions: [
        { name: "instructions", enabled: true, iteration: 0, changed: false },
        { name: "foreach", enabled: true, iteration: 0, changed: true },
        { name: "if_else", enabled: false, iteration: 0, changed: false },
      ],
      unsupportedInstructions: [],
      suggestedNextWork: [],
    },
  });

  const unfiltered = buildTracePanelModel(data, null, "");
  assert.equal(unfiltered.status, "available");
  assert.equal(unfiltered.totals.commands, 3);
  assert.equal(unfiltered.commands[0].id, "c000");
  assert.equal(unfiltered.commands[2].id, "c023");
  assert.equal(unfiltered.commands[2].pcRangeText, "23..41");
  assert.equal(unfiltered.commands[2].hot, true);
  assert.equal(unfiltered.commands[2].changed, true);
  assert.equal(unfiltered.commands[2].matched, "loop:foreach");

  const filtered = buildTracePanelModel(data, null, "foreach");
  assert.equal(filtered.visibleCommands.length, 1);
  assert.equal(filtered.visibleCommands[0].id, "c023");

  const empty = buildTracePanelModel(data, null, "no-match");
  assert.equal(empty.visibleCommands.length, 0);
  assert.match(empty.noMatchText ?? "", /no commands match/i);

  // Cursor selection drives detail — a full Location with only the group set.
  const detailModel = buildTracePanelModel(data, {
    scopeId: null,
    instIdx: null,
    pc: null,
    group: 23,
    blockId: null,
    decompileLine: null,
    sourceFile: null,
    sourceLine: null,
  }, "");
  const detail = detailModel.detail;
  assert.equal(detail.available, true);
  assert.equal(detail.command.id, "c023");
  assert.equal(detail.passChain.length, 3);
  const matchedRow = detail.passChain.find((row) => row.name === "foreach");
  assert.ok(matchedRow);
  assert.equal(matchedRow!.status, "matched");
  assert.equal(matchedRow!.changed, true);
  const skipped = detail.passChain.find((row) => row.name === "if_else");
  assert.equal(skipped?.status, "skipped");
  assert.ok(detail.evidence.length > 0, "expected evidence rows from real instructions and source-map");
  assert.ok(detail.decisions.some((row) => row.outcome === "yes"));
});

test("trace panel model rejects null data and prototype TRACE_DETAIL", () => {
  const empty = buildTracePanelModel(null, null, "");
  assert.equal(empty.status, "unavailable");
  assert.equal(empty.commands.length, 0);
  assert.equal(empty.detail.available, false);

  const productionSource = fs.readFileSync(
    path.join(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), "..", "src", "components", "panels", "ProvenancePanel.tsx"),
    "utf8",
  );
  assert.doesNotMatch(productionSource, /TRACE_DETAIL|TRACE_COMMANDS/);
  const modelSource = fs.readFileSync(
    path.join(import.meta.dirname ?? path.dirname(new URL(import.meta.url).pathname), "..", "src", "lib", "panel-models.ts"),
    "utf8",
  );
  assert.doesNotMatch(modelSource, /TRACE_DETAIL|TRACE_COMMANDS/);
});
