import type { BasicBlock, DecompileLine, Instruction } from "@/types";

export interface LocationFixture {
  scopeId: string;
  instructions: Instruction[];
  blocks: BasicBlock[];
  decompileLines: DecompileLine[];
}

export function createLocationFixture(): LocationFixture {
  const instructions: Instruction[] = [
    {
      idx: 0,
      pc: 10,
      op: "loadScalar1",
      args: "a",
      operands: [],
      argc: 0,
      group: 1,
      commandPcRange: [10, 13],
      groupStart: true,
      comment: "",
      size: 1,
      isJump: false,
      isJumpConditional: false,
      isJumpUnconditional: false,
      jumpTarget: null,
      fallthrough: 11,
      primaryNextLocation: 11,
      secondaryNextLocation: null,
      nextLocations: [11],
    },
    {
      idx: 1,
      pc: 12,
      op: "jumpFalse1",
      args: "24",
      operands: [24],
      argc: 1,
      group: 1,
      commandPcRange: [10, 13],
      groupStart: false,
      comment: "",
      size: 1,
      isJump: true,
      isJumpConditional: true,
      isJumpUnconditional: false,
      jumpTarget: 24,
      fallthrough: 13,
      primaryNextLocation: 24,
      secondaryNextLocation: 13,
      nextLocations: [24, 13],
    },
    {
      idx: 2,
      pc: 14,
      op: "loadScalar1",
      args: "b",
      operands: [],
      argc: 0,
      group: 2,
      commandPcRange: [14, 24],
      groupStart: true,
      comment: "",
      size: 1,
      isJump: false,
      isJumpConditional: false,
      isJumpUnconditional: false,
      jumpTarget: null,
      fallthrough: 15,
      primaryNextLocation: 15,
      secondaryNextLocation: null,
      nextLocations: [15],
    },
    {
      idx: 3,
      pc: 24,
      op: "done",
      args: "",
      operands: [],
      argc: 0,
      group: 3,
      commandPcRange: [24, 25],
      groupStart: true,
      comment: "",
      size: 1,
      isJump: false,
      isJumpConditional: false,
      isJumpUnconditional: false,
      jumpTarget: null,
      fallthrough: 25,
      primaryNextLocation: 25,
      secondaryNextLocation: null,
      nextLocations: [25],
    },
  ];

  const blocks: BasicBlock[] = [
    {
      id: 7,
      start: 0,
      end: 1,
      startPc: 10,
      endPc: 12,
      group: "1",
      edges: [8],
      instructions: [
        { pc: 10, name: "loadScalar1", operands: [] },
        { pc: 12, name: "jumpFalse1", operands: [24] },
      ],
    },
    {
      id: 8,
      start: 2,
      end: 3,
      startPc: 14,
      endPc: 24,
      group: "2",
      edges: [],
      instructions: [
        { pc: 14, name: "loadScalar1", operands: [] },
        { pc: 24, name: "done", operands: [] },
      ],
    },
  ];

  const decompileLines: DecompileLine[] = [
    { text: "set a 1", group: 1, lineIndex: 0, sourceLineStart: 20, sourceLineEnd: 20 },
    { text: "if {$a} {...}", group: 2, lineIndex: 1, sourceLineStart: 21, sourceLineEnd: 22 },
    { text: "return", group: 3, lineIndex: 2, sourceLineStart: 23, sourceLineEnd: 23 },
  ];

  return {
    scopeId: "fixture-scope",
    instructions,
    blocks,
    decompileLines,
  };
}
