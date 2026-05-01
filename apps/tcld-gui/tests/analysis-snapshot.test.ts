import test from "node:test";
import assert from "node:assert/strict";
import { consumeLoadResult, type LoadResult } from "@/lib/tauri-api";
import { tcldDataFromLoadResult } from "@/lib/store-data";
import type { SourceMapEntry } from "@/types";

test("frontend API and store consume snapshot-backed DTO and correlate source map fields", () => {
  const sourceMap: SourceMapEntry[] = [
    {
      bytecodeRange: [0, 8],
      sourceLines: [0, 1],
      sourceChars: [0, 8],
      text: "set x 1",
      commandGroup: 0,
      commandPath: [0],
      parentCommandGroup: null,
      originalStatus: "available",
      originalSpan: [20, 27],
    },
  ];
  const raw: LoadResult = {
    metadata: {
      filename: "fixture.tbc",
      path: "/tmp/fixture.tbc",
      tclVersion: "8.6",
      instructionCount: 1,
      literalCount: 0,
      commandCount: 1,
      exceptionCount: 0,
      auxCount: 0,
      codeBytes: 8,
      maxStackDepth: 1,
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
        commandPcRange: [0, 8],
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
      },
    ],
    literals: [],
    exceptions: [],
    blocks: [],
    traces: [],
    decompileLines: [{ text: "set x 1", group: 0, lineIndex: 0 }],
    decompileTokens: [],
    decompileText: "set x 1\n",
    dumpText: "Instructions:\n",
    summaryText: "snapshot: canonical-domain\n",
    sourceMap,
    diagnostics: {
      reconstruction: "Strict",
      recognizedConstructs: ["command:set"],
      unreducedOpcodes: [],
      approximations: [],
      suppressedFastPathRanges: [],
      passDecisions: [{ name: "instructions", enabled: true, iteration: 0, changed: true }],
      unsupportedInstructions: [],
      suggestedNextWork: [],
    },
    reconstruction: "Strict",
    provenance: { kind: "Path", label: "/tmp/fixture.tbc", originalSourceAvailable: true },
  };

  const loaded = consumeLoadResult(raw);
  const stored = tcldDataFromLoadResult(loaded);
  const correlated = stored.sourceMap?.find(
    (entry) => entry.commandGroup === stored.decompileLines[0].group,
  );
  assert.equal(stored.decompileLines[0].sourceMapIndex, 0);
  assert.equal(correlated?.bytecodeRange[0], stored.instructions[0].pc);
  assert.equal(stored.diagnostics?.reconstruction, stored.reconstruction);
  assert.equal(stored.provenance?.originalSourceAvailable, true);
});
