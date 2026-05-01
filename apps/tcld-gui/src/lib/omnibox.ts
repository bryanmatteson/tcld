"use client";

import type { Instruction, BasicBlock, DecompileLine, LiteralEntry } from "@/types";
import type { Annotations } from "@/lib/annotations";
import type { LocationIndex } from "@/lib/location-index";

export type OmniboxResultType =
  | "instruction"
  | "pc"
  | "group"
  | "block"
  | "line"
  | "bookmark"
  | "comment"
  | "opcode"
  | "literal";

export interface OmniboxResult {
  id: string;
  type: OmniboxResultType;
  label: string;
  detail?: string;
  icon?: string;
  instIdx: number | null;
}

interface OmniboxParams {
  query: string;
  instructions: Instruction[];
  blocks: BasicBlock[];
  locationIndex?: LocationIndex | null;
  annotations: Annotations;
  literals: LiteralEntry[];
  decompileLines: DecompileLine[];
  limit?: number;
}

function formatInstructionLabel(inst: Instruction): string {
  const pc = `0x${inst.pc.toString(16).padStart(4, "0")}`;
  const group = inst.group != null ? `c${String(inst.group).padStart(3, "0")}` : "c---";
  return `${pc} · i${inst.idx} · ${group} · ${inst.op}`;
}

function parsePc(raw: string): number | null {
  const value = raw.trim().toLowerCase();
  if (!value) return null;
  if (value.startsWith("0x")) {
    const parsed = Number.parseInt(value.slice(2), 16);
    return Number.isFinite(parsed) ? parsed : null;
  }
  if (/^[0-9]+$/.test(value)) {
    return Number.parseInt(value, 10);
  }
  if (/^[0-9a-f]+$/.test(value)) {
    const parsed = Number.parseInt(value, 16);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function getInstructionByIdx(instructions: Instruction[], idx: number): Instruction | null {
  if (idx < 0) return null;
  const direct = instructions[idx];
  if (direct && direct.idx === idx) return direct;
  return instructions.find((inst) => inst.idx === idx) ?? null;
}

function findNearestInstructionAtOrAfterPc(
  instructions: Instruction[],
  pc: number,
): Instruction | null {
  if (instructions.length === 0) return null;

  let lo = 0;
  let hi = instructions.length - 1;
  let found = -1;

  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const candidatePc = instructions[mid]?.pc ?? Number.POSITIVE_INFINITY;
    if (candidatePc >= pc) {
      found = mid;
      hi = mid - 1;
    } else {
      lo = mid + 1;
    }
  }

  if (found < 0) return null;
  return instructions[found] ?? null;
}

export function parseOmniboxQuery({
  query,
  instructions,
  blocks,
  locationIndex,
  annotations,
  literals,
  decompileLines,
  limit = 20,
}: OmniboxParams): OmniboxResult[] {
  const trimmed = query.trim();
  if (!trimmed) return [];
  const lower = trimmed.toLowerCase();
  const results: OmniboxResult[] = [];
  const seen = new Set<string>();

  const add = (result: OmniboxResult) => {
    if (seen.has(result.id)) return;
    seen.add(result.id);
    results.push(result);
  };

  const addInstruction = (inst: Instruction, type: OmniboxResultType, icon: string, detail?: string) => {
    add({
      id: `${type}:${inst.idx}`,
      type,
      label: formatInstructionLabel(inst),
      detail,
      icon,
      instIdx: inst.idx,
    });
  };

  const instByIndex = /^i(\d+)$/i.exec(trimmed);
  if (instByIndex) {
    const idx = Number.parseInt(instByIndex[1], 10);
    const inst = instructions[idx];
    if (inst) {
      addInstruction(inst, "instruction", "i", "instruction");
      return results;
    }
    return [];
  }

  const groupMatch = /^c(\d+)$/i.exec(trimmed);
  if (groupMatch) {
    const group = Number.parseInt(groupMatch[1], 10);
    const indexedIdx = locationIndex?.groupToFirstInstIdx.get(group);
    const inst = indexedIdx != null
      ? getInstructionByIdx(instructions, indexedIdx)
      : instructions.find((candidate) => candidate.group === group && candidate.groupStart) ?? null;
    if (inst) {
      addInstruction(inst, "group", "c", `group c${String(group).padStart(3, "0")}`);
      return results;
    }
  }

  const blockMatch = /^b(\d+)$/i.exec(trimmed);
  if (blockMatch) {
    const blockId = Number.parseInt(blockMatch[1], 10);
    const indexedIdx = locationIndex?.blockToFirstInstIdx.get(blockId);
    const indexedInst = indexedIdx != null ? getInstructionByIdx(instructions, indexedIdx) : null;
    if (indexedInst) {
      addInstruction(indexedInst, "block", "b", `block b${String(blockId).padStart(3, "0")}`);
      return results;
    }

    const block = blocks.find((candidate) => candidate.id === blockId);
    if (block) {
      const inst = getInstructionByIdx(instructions, block.start);
      if (inst) {
        addInstruction(inst, "block", "b", `block b${String(blockId).padStart(3, "0")}`);
        return results;
      }
    }
  }

  const lineMatch = /^line:(\d+)$/i.exec(trimmed);
  if (lineMatch) {
    const lineNumber = Number.parseInt(lineMatch[1], 10);
    const lineIdx = lineNumber - 1;
    const indexedGroup = locationIndex?.decompileLineToGroup.get(lineIdx);
    const indexedInstIdx = indexedGroup != null ? locationIndex?.groupToFirstInstIdx.get(indexedGroup) : null;
    const indexedInst = indexedInstIdx != null ? getInstructionByIdx(instructions, indexedInstIdx) : null;
    if (indexedInst) {
      addInstruction(indexedInst, "line", "L", `line ${lineNumber}`);
      return results;
    }

    const line = decompileLines[lineIdx];
    if (line?.group != null) {
      const inst = instructions.find((candidate) => candidate.group === line.group && candidate.groupStart);
      if (inst) {
        addInstruction(inst, "line", "L", `line ${lineNumber}`);
        return results;
      }
    }
  }

  if (lower.startsWith("pc:") || lower.startsWith("0x")) {
    const token = lower.startsWith("pc:") ? lower.slice(3).trim() : lower;
    const pc = parsePc(token);
    if (pc != null) {
      const indexedIdx = locationIndex?.pcToInstIdx.get(pc);
      const exact = indexedIdx != null
        ? getInstructionByIdx(instructions, indexedIdx)
        : instructions.find((candidate) => candidate.pc === pc) ?? null;
      if (exact) {
        addInstruction(exact, "pc", "pc", "exact pc match");
      } else {
        const nearest = findNearestInstructionAtOrAfterPc(instructions, pc);
        if (nearest) {
          addInstruction(nearest, "pc", "pc", `nearest pc >= ${pc}`);
        }
      }
      return results;
    }
  }

  if (lower.startsWith("@bookmark") || lower.startsWith("@bm")) {
    const sorted = [...annotations.bookmarks].sort((a, b) => a - b).slice(0, limit);
    for (const idx of sorted) {
      const inst = instructions[idx];
      if (!inst) continue;
      addInstruction(inst, "bookmark", "bookmark", "bookmark");
    }
    return results;
  }

  if (lower.startsWith("@comment") || lower.startsWith("@cm")) {
    const sorted = [...annotations.comments.entries()]
      .sort(([a], [b]) => a - b)
      .slice(0, limit);
    for (const [idx, comment] of sorted) {
      const inst = instructions[idx];
      if (!inst) continue;
      addInstruction(inst, "comment", ";", comment);
    }
    return results;
  }

  const literalMatch = /^#(\d+)$/i.exec(trimmed);
  if (literalMatch) {
    const literalIdx = Number.parseInt(literalMatch[1], 10);
    const literal = literals.find((candidate) => candidate.idx === literalIdx);
    if (literal) {
      add({
        id: `literal:${literal.idx}`,
        type: "literal",
        label: `#${literal.idx} · ${literal.kind}`,
        detail: literal.value,
        icon: "#",
        instIdx: null,
      });
      return results;
    }
  }

  const searchNeedle = lower.startsWith("/") ? lower.slice(1) : lower.startsWith("search ") ? lower.slice(7) : lower;
  const needle = searchNeedle.trim();
  if (!needle) return results;

  for (const inst of instructions) {
    const haystack = `${inst.op} ${inst.args} ${inst.comment}`.toLowerCase();
    if (!haystack.includes(needle)) continue;
    addInstruction(inst, "opcode", "/", `${inst.op} ${inst.args}`.trim());
    if (results.length >= limit) break;
  }

  if (results.length === 0 && /^[0-9]+$/.test(needle)) {
    const pc = Number.parseInt(needle, 10);
    const candidate = findNearestInstructionAtOrAfterPc(instructions, pc);
    if (candidate) {
      addInstruction(candidate, "pc", "pc", `nearest pc >= ${pc}`);
    }
  }

  return results.slice(0, limit);
}
