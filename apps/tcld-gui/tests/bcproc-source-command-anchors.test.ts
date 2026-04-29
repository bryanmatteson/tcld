import test from "node:test";
import assert from "node:assert/strict";
import { consumeLoadResult, type LoadResult } from "@/lib/tauri-api";
import { tcldDataFromLoadResult } from "@/lib/store-data";
import { buildSourcePanelModel, sourceCommandAnchorSelectionTarget } from "@/lib/panel-models";

function loadResult(): LoadResult {
  return {
    metadata: {
      filename: "public_complex_fixture.tbc",
      path: "/tmp/public_complex_fixture.tbc",
      tclVersion: "8.6",
      instructionCount: 2,
      literalCount: 0,
      commandCount: 8,
      exceptionCount: 0,
      auxCount: 0,
      codeBytes: 40,
      maxStackDepth: 3,
      syntaxEngine: "tree-sitter-tcl",
      format: 1,
      build: 0,
    },
    instructions: [
      {
        idx: 0,
        pc: 30,
        op: "push1",
        args: "fixture::startlog",
        operands: [0],
        argc: 0,
        group: 7,
        commandPcRange: [30, 40],
        groupStart: true,
        comment: "fixture::startlog",
        size: 2,
        isJump: false,
        isJumpConditional: false,
        isJumpUnconditional: false,
        jumpTarget: null,
        fallthrough: 32,
        primaryNextLocation: 32,
        secondaryNextLocation: null,
        nextLocations: [32],
      },
      {
        idx: 1,
        pc: 32,
        op: "invokeStk1",
        args: "3",
        operands: [3],
        argc: 1,
        group: 7,
        commandPcRange: [30, 40],
        groupStart: false,
        comment: "",
        size: 2,
        isJump: false,
        isJumpConditional: false,
        isJumpUnconditional: false,
        jumpTarget: null,
        fallthrough: 34,
        primaryNextLocation: 34,
        secondaryNextLocation: null,
        nextLocations: [34],
      },
    ],
    literals: [],
    exceptions: [],
    blocks: [],
    traces: [],
    decompileLines: [
      { text: "proc fixture::startlog { _text } {", group: 7, lineIndex: 0, sourceMapIndex: 0 },
      { text: "    variable baseDir", group: 100, lineIndex: 1, sourceMapIndex: 1 },
      { text: "    global _startlog", group: 101, lineIndex: 2, sourceMapIndex: 2 },
      { text: "}", group: 7, lineIndex: 3, sourceMapIndex: 0 },
    ],
    decompileTokens: [],
    decompileText: "proc fixture::startlog { _text } {\n    variable baseDir\n    global _startlog\n}\n",
    dumpText: "Instructions:\n",
    summaryText: "fixture snapshot\n",
    sourceMap: [
      {
        bytecodeRange: [30, 40],
        sourceLines: [0, 4],
        sourceChars: [0, 78],
        text: "proc fixture::startlog { _text } {\n    variable baseDir\n    global _startlog\n}",
        commandGroup: 7,
        commandPath: [7],
        parentCommandGroup: null,
        originalStatus: "unavailable",
        originalSpan: null,
      },
      {
        bytecodeRange: [0, 4],
        sourceLines: [1, 2],
        sourceChars: [31, 51],
        text: "variable baseDir",
        commandGroup: 100,
        commandPath: [7, 0],
        parentCommandGroup: 7,
        originalStatus: "unavailable",
        originalSpan: null,
      },
      {
        bytecodeRange: [4, 8],
        sourceLines: [2, 3],
        sourceChars: [56, 72],
        text: "global _startlog",
        commandGroup: 101,
        commandPath: [7, 1],
        parentCommandGroup: 7,
        originalStatus: "unavailable",
        originalSpan: null,
      },
    ],
    sourceCommandAnchors: [
      {
        kind: "root",
        sourceMapIndex: 0,
        commandPath: [7],
        displayPath: "c007",
        parentRootGroup: 7,
        sourceLines: [0, 4],
        snippet: "proc fixture::startlog { _text } {",
      },
      {
        kind: "nested",
        sourceMapIndex: 1,
        commandPath: [7, 0],
        displayPath: "c007/n000",
        parentRootGroup: 7,
        sourceLines: [1, 2],
        snippet: "variable baseDir",
      },
      {
        kind: "nested",
        sourceMapIndex: 2,
        commandPath: [7, 1],
        displayPath: "c007/n001",
        parentRootGroup: 7,
        sourceLines: [2, 3],
        snippet: "global _startlog",
      },
    ],
    diagnostics: {
      reconstruction: "Strict",
      recognizedConstructs: ["proc"],
      unreducedOpcodes: [],
      approximations: [],
      suppressedFastPathRanges: [],
      passDecisions: [{ name: "instructions", enabled: true, iteration: 0, changed: true }],
      unsupportedInstructions: [],
      suggestedNextWork: [],
    },
    reconstruction: "Strict",
    provenance: { kind: "Path", label: "/tmp/public_complex_fixture.tbc", originalSourceAvailable: false },
    originalSourceText: null,
  };
}

test("GUI consumes bcproc nested source-command anchors without collapsing them to root groups", () => {
  const data = tcldDataFromLoadResult(consumeLoadResult(loadResult()));
  const source = buildSourcePanelModel(data, { scopeId: "fixture", instIdx: 0, pc: 30, group: 7, blockId: null, decompileLine: 1, sourceFile: null, sourceLine: 1 });

  const nested = source.sourceCommandRows.find((row) => row.anchor.displayPath === "c007/n000");
  assert.equal(data.sourceCommandAnchors?.length, 3);
  assert.equal(nested?.anchor.kind, "nested");
  assert.equal(nested?.anchor.parentRootGroup, 7);
  assert.equal(nested?.anchor.snippet, "variable baseDir");
  assert.notEqual(nested?.anchor.displayPath, "c007");

  const target = sourceCommandAnchorSelectionTarget(data, nested!.index);
  assert.equal(target?.group, 7);
  assert.equal(target?.instIdx, 0);
  assert.equal(target?.decompileLine, 1);
  assert.equal(target?.sourceLine, 1);
});
