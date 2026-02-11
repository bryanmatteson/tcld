"use client";

import type { Instruction, BasicBlock, DecompileLine, FileMetadata } from "@/types";

export interface LocationIndex {
  instIdxToPc: Int32Array;
  instIdxToGroup: Int32Array;
  instIdxToBlock: Int32Array;
  pcToInstIdx: Map<number, number>;
  groupToFirstInstIdx: Map<number, number>;
  blockToFirstInstIdx: Map<number, number>;
  decompileLineToGroup: Map<number, number>;
  sourceLineToFirstInstIdx: Map<number, number>;
  scopeId: string;
  instructionCount: number;
}

export function computeScopeId(meta: FileMetadata): string {
  return `${meta.filename}:${meta.instructionCount}:${meta.codeBytes}`;
}

export function buildLocationIndex(
  instructions: Instruction[],
  blocks: BasicBlock[],
  decompileLines: DecompileLine[],
  scopeId: string,
): LocationIndex {
  const count = instructions.length;
  const instIdxToPc = new Int32Array(count);
  const instIdxToGroup = new Int32Array(count);
  const instIdxToBlock = new Int32Array(count);
  instIdxToGroup.fill(-1);
  instIdxToBlock.fill(-1);

  const pcToInstIdx = new Map<number, number>();
  const groupToFirstInstIdx = new Map<number, number>();
  const blockToFirstInstIdx = new Map<number, number>();

  for (const block of blocks) {
    blockToFirstInstIdx.set(block.id, block.start);
    const start = Math.max(0, block.start);
    const end = Math.min(count - 1, block.end);
    for (let idx = start; idx <= end; idx += 1) {
      instIdxToBlock[idx] = block.id;
    }
  }

  for (const inst of instructions) {
    const idx = inst.idx;
    if (idx < 0 || idx >= count) continue;

    instIdxToPc[idx] = inst.pc;
    instIdxToGroup[idx] = inst.group ?? -1;
    pcToInstIdx.set(inst.pc, idx);

    if (inst.groupStart && inst.group != null && !groupToFirstInstIdx.has(inst.group)) {
      groupToFirstInstIdx.set(inst.group, idx);
    }
  }

  const decompileLineToGroup = new Map<number, number>();
  const sourceLineToFirstInstIdx = new Map<number, number>();

  for (let i = 0; i < decompileLines.length; i += 1) {
    const line = decompileLines[i];
    if (line.group == null) continue;

    decompileLineToGroup.set(i, line.group);

    const firstInst = groupToFirstInstIdx.get(line.group);
    if (
      firstInst != null
      && line.sourceLineStart != null
      && line.sourceLineEnd != null
    ) {
      const start = Math.min(line.sourceLineStart, line.sourceLineEnd);
      const end = Math.max(line.sourceLineStart, line.sourceLineEnd);
      for (let sourceLine = start; sourceLine <= end; sourceLine += 1) {
        if (!sourceLineToFirstInstIdx.has(sourceLine)) {
          sourceLineToFirstInstIdx.set(sourceLine, firstInst);
        }
      }
    }
  }

  return {
    instIdxToPc,
    instIdxToGroup,
    instIdxToBlock,
    pcToInstIdx,
    groupToFirstInstIdx,
    blockToFirstInstIdx,
    decompileLineToGroup,
    sourceLineToFirstInstIdx,
    scopeId,
    instructionCount: count,
  };
}
