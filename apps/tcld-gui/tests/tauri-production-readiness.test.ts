import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  buildComparisonPanelModel,
  buildDiagnosticsPanelModel,
  buildProvenancePanelModel,
  buildSourcePanelModel,
  sourceMapCrossLinkTargets,
  sourceMapSelectionTarget,
} from "@/lib/panel-models";
import { resolveBreakpointTarget } from "@/lib/workflow-contracts";
import {
  buildDebugLaunchRequest,
  buildExportRequests,
  dispatchNativeMenuAction,
  resolveDroppedFile,
  summarizeFormatPreview,
} from "@/lib/gui-workflows";
import { buildSessionDocument, createSessionState, upsertSessionDocument } from "@/lib/store-data";
import { createOutputSearchState, formatOutputForCopy, searchOutput } from "@/lib/output-search";
import { __setInvokeForTests, debugResolveCommand, debugSetBreakpointEnabled } from "@/lib/tauri-api";
import type { TcldData } from "@/types";

function fixtureData(overrides: Partial<TcldData> = {}): TcldData {
  return {
    metadata: {
      filename: "fixture.tbc",
      path: "/tmp/fixture.tbc",
      tclVersion: "8.6",
      instructionCount: 3,
      literalCount: 0,
      commandCount: 1,
      exceptionCount: 0,
      auxCount: 0,
      codeBytes: 8,
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
        op: "branchWithoutRegexName",
        args: "5",
        operands: [5],
        argc: 1,
        group: 0,
        groupStart: false,
        comment: "pc 5",
        size: 3,
        isJump: true,
        isJumpConditional: true,
        isJumpUnconditional: false,
        jumpTarget: 5,
        fallthrough: 8,
        primaryNextLocation: 5,
        secondaryNextLocation: 8,
        nextLocations: [5, 8],
        commandPcRange: [0, 5],
      },
      {
        idx: 2,
        pc: 8,
        op: "done",
        args: "",
        operands: [],
        argc: 0,
        group: 1,
        groupStart: true,
        comment: "",
        size: 1,
        isJump: false,
        isJumpConditional: false,
        isJumpUnconditional: false,
        jumpTarget: null,
        fallthrough: null,
        primaryNextLocation: null,
        secondaryNextLocation: null,
        nextLocations: [],
        commandPcRange: [8, 9],
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
        instructions: [{ pc: 2, name: "notRegexJump", operands: [5] }],
      },
      {
        id: 1,
        start: 2,
        end: 2,
        startPc: 5,
        endPc: 6,
        group: "c001",
        edges: [],
        instructions: [],
      },
      {
        id: 2,
        start: 2,
        end: 2,
        startPc: 8,
        endPc: 9,
        group: "c002",
        edges: [],
        instructions: [],
      },
    ],
    traces: [{ instructionIndex: 1, pc: 2, opcode: "branchWithoutRegexName", argc: 1, stackDelta: -1, beforeDepth: 1, afterDepth: 0, popped: ["x"], pushed: [] }],
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
    sourceCommandAnchors: [],
    diagnostics: {
      reconstruction: "Approximation",
      recognizedConstructs: ["if"],
      unreducedOpcodes: ["branchWithoutRegexName"],
      approximations: [{ kind: "UnresolvedInstruction", bytecodeRange: [0, 3], reason: "stack expression folded" }],
      suppressedFastPathRanges: [[0, 2]],
      passDecisions: [{ name: "conditionals", enabled: true, iteration: 0, changed: true }],
      unsupportedInstructions: ["mystery"],
      suggestedNextWork: ["inspect source map"],
    },
    reconstruction: "Approximation",
    provenance: { kind: "Path", label: "/tmp/fixture.tbc", originalSourceAvailable: true },
    originalSourceText: "set x 1\nif {$x} {puts ok}\n",
    ...overrides,
  };
}

test("registered production panels include source map, debugger, workflow, and resolver surfaces", () => {
  const registry = fs.readFileSync(path.resolve(process.cwd(), "src/lib/view-registry.ts"), "utf8");
  for (const id of [
    "source",
    "decompile",
    "instructions",
    "graph",
    "detail",
    "diagnostics",
    "provenance",
    "comparison",
    "project",
    "console",
    "breakpoints",
    "watchpoints",
    "commandResolver",
  ]) {
    assert.match(registry, new RegExp(`id: "${id}"`), `missing registered panel ${id}`);
  }

  const workspace = fs.readFileSync(path.resolve(process.cwd(), "src/components/MosaicWorkspace.tsx"), "utf8");
  for (const component of [
    "SourcePanel",
    "DecompilePanel",
    "InstructionPanel",
    "GraphPanel",
    "DetailPanel",
    "DiagnosticsPanel",
    "ProvenancePanel",
    "ComparisonPanel",
    "ProjectPanel",
    "ConsolePanel",
    "BreakpointsPanel",
    "WatchpointsPanel",
    "CommandResolverPanel",
  ]) {
    assert.match(workspace, new RegExp(`<${component}\\b`), `workspace does not render ${component}`);
  }
});

test("source map selection cross-links source, decompile, instruction, graph, detail, diagnostics, provenance, comparison, and debugger stop contexts", () => {
  const data = fixtureData();
  const cursor = { scopeId: "fixture", instIdx: 1, pc: 2, group: 0, blockId: 0, decompileLine: null, sourceFile: "/tmp/source.tcl", sourceLine: 7 };
  const source = buildSourcePanelModel(data, cursor);
  const comparison = buildComparisonPanelModel(data, cursor);
  const diagnostics = buildDiagnosticsPanelModel(data, cursor);
  const provenance = buildProvenancePanelModel(data, cursor);
  const target = sourceMapSelectionTarget(data, 0);
  const crossLinks = sourceMapCrossLinkTargets(data, 0, {
    stoppedAt: { reason: "watchpoint", file: "/tmp/source.tcl", line: 7, frameCount: 2, pc: 2, instructionIndex: 1, blockId: 0, watchpointName: "x", watchpointOperation: "write", watchpointOldValue: "1", watchpointNewValue: "2" },
  });

  assert.equal(source.available, true);
  assert.equal(source.sourceMapRows[0].selected, true);
  assert.equal(comparison?.selection.sourceMapIndex, 0);
  assert.equal(diagnostics.linkedRows.find((row) => row.kind === "approximation")?.selected, true);
  assert.equal(provenance.rows[0].selected, true);
  assert.equal(target?.instIdx, 0);
  assert.deepEqual(crossLinks.source.originalSpan, [11, 28]);
  assert.equal(crossLinks.decompile.lineIndex, 0);
  assert.equal(crossLinks.instruction.instIdx, 0);
  assert.equal(crossLinks.graph.blockId, 0);
  assert.equal(crossLinks.detail.instructionIndex, 0);
  assert.equal(crossLinks.diagnostics.linkedRows.length >= 2, true);
  assert.equal(crossLinks.provenance.rowIndex, 0);
  assert.equal(crossLinks.comparison.sourceMapIndex, 0);
  assert.equal(crossLinks.debuggerStop?.reason, "watchpoint");
});

test("source and comparison expose unavailable original source without substituting reconstructed text", () => {
  const data = fixtureData({
    provenance: { kind: "DebuggerChunk", label: "live#1", originalSourceAvailable: false },
    originalSourceText: null,
    sourceMap: [{ ...fixtureData().sourceMap![0], originalStatus: "unavailable", originalSpan: null }],
  });
  const source = buildSourcePanelModel(data, null);
  const comparison = buildComparisonPanelModel(data, null);
  const crossLinks = sourceMapCrossLinkTargets(data, 0);

  assert.equal(source.available, false);
  assert.equal(source.text, null);
  assert.equal(comparison?.original.available, false);
  assert.equal(comparison?.original.text, null);
  assert.equal(crossLinks.source.available, false);
  assert.equal(crossLinks.source.originalSpan, null);
});

test("debugger launch, breakpoint, watchpoint, stack, eval, command resolution, drag-drop, export, and console workflows stay wired to production IPC and active data", async () => {
  const data = fixtureData();
  const session = upsertSessionDocument(createSessionState(), buildSessionDocument(data, { role: "open", label: "fixture.tbc" }));
  const calls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
  __setInvokeForTests(async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ cmd, args });
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
    await debugSetBreakpointEnabled("/tmp/source.tcl", 7, false);
    const resolved = await debugResolveCommand("puts", "::");
    assert.equal(resolved.fullyQualifiedName, "::puts");
  } finally {
    __setInvokeForTests(null);
  }

  const launch = buildDebugLaunchRequest(
    "/tmp/app.tcl",
    { args: "--flag \"two words\"", env: "A=1 B=two", cwd: "/tmp", tclshPath: "/usr/bin/tclsh" },
    { bytecodeHookMode: "intercept", bytecodeHookCommands: ["expr"] },
  );
  const dropped = resolveDroppedFile({ name: "fixture.tcl", path: "/tmp/fixture.tcl" });
  const noPath = resolveDroppedFile({ name: "fixture.tcl" });
  const output = [
    { stream: "stdout" as const, text: "hello world", timestamp: 1 },
    { stream: "stderr" as const, text: "warning world", timestamp: 2 },
    { stream: "eval" as const, text: "> expr {$world}", timestamp: 3 },
    { stream: "error" as const, text: "error world", timestamp: 4 },
    { stream: "system" as const, text: "system world", timestamp: 5 },
  ];
  const search = searchOutput(output, createOutputSearchState("world"));
  const exports = buildExportRequests({
    data,
    session,
    output,
    stoppedAt: { reason: "watchpoint", file: "a.tcl", line: 7, frameCount: 2, pc: 2, instructionIndex: 1, blockId: 0, watchpointName: "x", watchpointOperation: "write", watchpointOldValue: "1", watchpointNewValue: "2" },
  });
  const breakpointTarget = resolveBreakpointTarget({
    data,
    selectedInstructionIndex: 1,
    stoppedAt: { reason: "breakpoint", file: "fallback.tcl", line: 99, frameCount: 1, pc: null, instructionIndex: null, blockId: null },
    scriptPath: "/tmp/source.tcl",
  });

  assert.deepEqual(launch.args, ["--flag", "two words"]);
  assert.deepEqual(launch.launchOptions.env, { A: "1", B: "two" });
  assert.equal(launch.launchOptions.cwd, "/tmp");
  assert.equal(launch.tclshPath, "/usr/bin/tclsh");
  assert.deepEqual(dropped, { supported: true, path: "/tmp/fixture.tcl", filename: "fixture.tcl", reason: "ready" });
  assert.equal(noPath.reason, "path-unavailable");
  assert.equal(search.visibleLines.length, 5);
  assert.equal(formatOutputForCopy(search.visibleLines), "[stdout] hello world\n[stderr] warning world\n[eval] > expr {$world}\n[error] error world\n[system] system world");
  assert.match(exports.fullReport.contents, /Watchpoint: x write 1 -> 2/);
  assert.match(exports.fullReport.contents, /Open Documents/);
  assert.deepEqual(breakpointTarget, { file: "/tmp/source.tcl", line: 7, source: "selection" });
  assert.equal(summarizeFormatPreview([{ startByte: 0, endByte: 1, replacement: "x" }]), "1 formatting edit");
  assert.deepEqual(calls.map((call) => call.cmd), ["debug_set_breakpoint_enabled", "debug_resolve_command"]);
  assert.deepEqual(calls[0].args, { file: "/tmp/source.tcl", line: 7, enabled: false });
});

test("native menu, toolbar source, and capabilities expose supported actions while omitting unsupported reverse controls", () => {
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

  const toolbar = fs.readFileSync(path.resolve(process.cwd(), "src/components/chrome/DebugToolbar.tsx"), "utf8");
  const page = fs.readFileSync(path.resolve(process.cwd(), "src/app/page.tsx"), "utf8");
  const main = fs.readFileSync(path.resolve(process.cwd(), "../tcld-tauri/src/app.rs"), "utf8");
  const tauriLib = fs.readFileSync(path.resolve(process.cwd(), "../tcld-tauri/src/lib.rs"), "utf8");
  const capabilities = JSON.parse(fs.readFileSync(path.resolve(process.cwd(), "../tcld-tauri/capabilities/default.json"), "utf8")) as { permissions: string[] };

  assert.deepEqual(calls, ["stepInstruction"]);
  assert.match(toolbar, /resolveBreakpointTarget/);
  assert.match(toolbar, /canStepInstruction &&/);
  assert.doesNotMatch(toolbar, /Step Previous|Previous Group|Reverse/);
  assert.match(page, /debug:toggle_breakpoint/);
  assert.match(main, /native_menu_run_configuration_contract/);
  assert.match(tauriLib, /debug_step_instruction/);
  assert.match(tauriLib, /debug_restart/);
  assert.doesNotMatch(main, /debug_step_previous|debug_reverse|debug_previous_group/);
  assert.ok(capabilities.permissions.includes("dialog:allow-open"));
  assert.ok(capabilities.permissions.includes("dialog:allow-save"));
});
