"use client";

import type { LocationIndex } from "@/lib/location-index";

export interface Location {
  scopeId: string | null;
  instIdx: number | null;
  pc: number | null;
  group: number | null;
  blockId: number | null;
  decompileLine: number | null;
  sourceFile: string | null;
  sourceLine: number | null;
}

export function emptyLocation(scopeId: string | null = null): Location {
  return {
    scopeId,
    instIdx: null,
    pc: null,
    group: null,
    blockId: null,
    decompileLine: null,
    sourceFile: null,
    sourceLine: null,
  };
}

export function resolveLocation(
  partial: Partial<Location>,
  index: LocationIndex,
): Location {
  if (partial.scopeId != null && partial.scopeId !== index.scopeId) {
    return {
      ...emptyLocation(partial.scopeId),
      decompileLine: partial.decompileLine ?? null,
      sourceFile: partial.sourceFile ?? null,
      sourceLine: partial.sourceLine ?? null,
    };
  }

  let instIdx = partial.instIdx ?? null;

  if (instIdx == null && partial.pc != null) {
    instIdx = index.pcToInstIdx.get(partial.pc) ?? null;
  }

  if (instIdx == null && partial.group != null) {
    instIdx = index.groupToFirstInstIdx.get(partial.group) ?? null;
  }

  if (instIdx == null && partial.blockId != null) {
    instIdx = index.blockToFirstInstIdx.get(partial.blockId) ?? null;
  }

  if (instIdx == null && partial.decompileLine != null) {
    const group = index.decompileLineToGroup.get(partial.decompileLine);
    if (group != null) {
      instIdx = index.groupToFirstInstIdx.get(group) ?? null;
    }
  }

  if (instIdx == null && partial.sourceLine != null) {
    instIdx = index.sourceLineToFirstInstIdx.get(partial.sourceLine) ?? null;
  }

  if (instIdx != null && instIdx >= 0 && instIdx < index.instructionCount) {
    const group = index.instIdxToGroup[instIdx] === -1 ? null : index.instIdxToGroup[instIdx];
    const blockId = index.instIdxToBlock[instIdx] === -1 ? null : index.instIdxToBlock[instIdx];
    return {
      scopeId: index.scopeId,
      instIdx,
      pc: index.instIdxToPc[instIdx],
      group,
      blockId,
      decompileLine: partial.decompileLine ?? null,
      sourceFile: partial.sourceFile ?? null,
      sourceLine: partial.sourceLine ?? null,
    };
  }

  return {
    ...emptyLocation(partial.scopeId ?? index.scopeId),
    decompileLine: partial.decompileLine ?? null,
    sourceFile: partial.sourceFile ?? null,
    sourceLine: partial.sourceLine ?? null,
  };
}
