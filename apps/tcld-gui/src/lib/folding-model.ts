import type { TcldData } from "@/types";

export interface FoldRegion {
  id: string;
  kind: "source-command" | "source-map" | "command-group";
  start: number;
  end: number;
  commandPath: string | null;
  label: string;
  snippet: string;
  count?: number;
  pcRange?: [number, number];
  sourceLines?: [number, number];
}

export type SourceVisibleRow =
  | { kind: "line"; id: string; lineIndex: number; text: string }
  | { kind: "fold"; id: string; lineIndex: number; hiddenLineCount: number; commandPath: string | null; snippet: string };

export interface SourceFoldModel {
  foldRegions: FoldRegion[];
  visibleRows: SourceVisibleRow[];
  selectedVisibleRowId: string | null;
}

export type InstructionVisibleRow =
  | { kind: "instruction"; id: string; instIdx: number; foldId: string | null; hiddenInstructionCount: number };

export interface InstructionFoldModel {
  foldRegions: FoldRegion[];
  visibleRows: InstructionVisibleRow[];
  selectedVisibleRowId: string | null;
}

export interface InstructionPanelRows {
  rows: InstructionVisibleRow[];
  visibleIndexByInstIdx: Map<number, number>;
}

export function buildDecompileFoldBlocks(data: TcldData | null): Map<number, number> {
  const blocks = new Map<number, number>();
  if (!data) return blocks;

  const lineCount = data.decompileLines.length;
  const byStart = new Map<number, { end: number; span: number }>();
  for (const range of data.decompileFoldingRanges ?? []) {
    const start = range.startRow;
    const end = Math.min(range.endRow, lineCount - 1);
    if (start < 0 || start >= lineCount || end < start) continue;
    if (end === start && !isSameLineDecompileFoldable(data.decompileLines[start]?.text ?? "")) {
      continue;
    }

    const span = end - start;
    const incumbent = byStart.get(start);
    if (!incumbent || span > incumbent.span) {
      byStart.set(start, { end, span });
    }
  }

  for (const [start, range] of byStart) {
    blocks.set(start, range.end);
  }
  return blocks;
}

function isSameLineDecompileFoldable(text: string): boolean {
  return /^proc(?:\s|$)/.test(text.trimStart());
}

export function buildSourceFoldModel(
  data: TcldData,
  options: { foldedIds: Set<string>; selectedSourceLine: number | null },
): SourceFoldModel {
  const lines = (data.originalSourceText ?? data.decompileText).split("\n");
  // Only `sourceCommandAnchors` are user-foldable. Each anchor represents a
  // semantic command block (`namespace eval`, `proc`, `if`, `for`, …) with
  // a meaningful commandPath label and snippet — exactly the units a user
  // wants to collapse.
  //
  // We intentionally do NOT include `sourceMap` regions here. Those exist
  // to cross-link instructions ↔ source lines and overlap heavily with
  // source-command anchors (often identical start, slightly different end),
  // which previously caused a visible duplication: a fold-row "header"
  // followed by the regular line-row of the same block.
  //
  // Even an aggressive range-key dedup wasn't enough — source-map ranges
  // for individual statements inside a command body would still emit fold
  // controls that the user perceived as redundant noise on top of the
  // outer command's chevron. Dropping them entirely is the right call.
  const rawRegions = (data.sourceCommandAnchors ?? []).map((anchor, index) => ({
    id: `source:${index}`,
    kind: "source-command" as const,
    start: anchor.sourceLines[0],
    end: anchor.sourceLines[1],
    commandPath: anchor.displayPath,
    label: anchor.displayPath,
    snippet: anchor.snippet,
  }));

  // Dedup by starting line. A single source line can only host one fold
  // chevron, so when multiple anchors start at the same line (e.g. an
  // outer `namespace eval` and a nested anchor that also starts at the
  // first body line) we keep just one — the OUTERMOST (largest range).
  // This eliminates the visible "header + duplicate line" pattern where
  // a folded inner anchor's header appeared right above the outer anchor's
  // line-row with its own chevron.
  const byStart = new Map<number, typeof rawRegions[number]>();
  for (const region of rawRegions) {
    if (region.end <= region.start) continue;
    const incumbent = byStart.get(region.start);
    if (!incumbent) {
      byStart.set(region.start, region);
      continue;
    }
    const incumbentSpan = incumbent.end - incumbent.start;
    const candidateSpan = region.end - region.start;
    if (candidateSpan > incumbentSpan) {
      byStart.set(region.start, region);
    }
  }
  const foldRegions = [...byStart.values()].sort((a, b) => a.start - b.start);
  const folded = foldRegions
    .filter((region) => options.foldedIds.has(region.id))
    .sort((a, b) => a.start - b.start || b.end - a.end);
  const visibleRows: SourceVisibleRow[] = [];
  let line = 0;
  while (line < lines.length) {
    const region = folded.find((candidate) => candidate.start === line && candidate.end > candidate.start);
    if (region) {
      visibleRows.push({
        kind: "fold",
        id: region.id,
        lineIndex: region.start,
        hiddenLineCount: Math.max(1, region.end - region.start),
        commandPath: region.commandPath,
        snippet: region.snippet,
      });
      line = Math.max(line + 1, region.end);
      continue;
    }
    visibleRows.push({ kind: "line", id: `line:${line}`, lineIndex: line, text: lines[line] ?? "" });
    line += 1;
  }

  return {
    foldRegions,
    visibleRows,
    selectedVisibleRowId: selectedSourceRowId(visibleRows, options.selectedSourceLine),
  };
}

export function buildInstructionFoldModel(
  data: TcldData,
  options: { foldedIds: Set<string>; selectedInstIdx: number | null; filter: string },
): InstructionFoldModel {
  const groupIds = [...new Set(data.instructions.map((inst) => inst.group).filter((group): group is number => group != null))];
  const foldRegions: FoldRegion[] = groupIds.map((group) => {
    const members = data.instructions.filter((inst) => inst.group === group);
    const sourceMapEntry = data.sourceMap?.find((entry) => entry.commandGroup === group);
    const decompileLine = data.decompileLines?.find((line) => line.group === group);
    const pcStart = members[0]?.pc ?? 0;
    const pcEnd = members[members.length - 1]?.pc ?? pcStart;
    const commandPath = sourceMapEntry ? displayCommandPath(sourceMapEntry.commandPath) : `c${String(group).padStart(3, "0")}`;
    const label = commandPath || `c${String(group).padStart(3, "0")}`;
    return {
      id: `group:${group}`,
      kind: "command-group" as const,
      start: members[0]?.idx ?? 0,
      end: (members[members.length - 1]?.idx ?? 0) + 1,
      commandPath: label,
      label,
      snippet: firstLine(sourceMapEntry?.text ?? decompileLine?.text ?? members[0]?.op ?? ""),
      count: members.length,
      pcRange: [pcStart, pcEnd],
      sourceLines: sourceMapEntry?.sourceLines ?? (
        decompileLine?.sourceLineStart != null && decompileLine?.sourceLineEnd != null
          ? [decompileLine.sourceLineStart, decompileLine.sourceLineEnd]
          : undefined
      ),
    };
  });
  const folded = foldRegions
    .filter((region) => options.foldedIds.has(region.id) && region.end - region.start > 1)
    .sort((a, b) => a.start - b.start);
  const visibleRows: InstructionVisibleRow[] = [];
  let idx = 0;
  while (idx < data.instructions.length) {
    const region = folded.find((candidate) => candidate.start === idx && candidate.end > candidate.start);
    if (region) {
      visibleRows.push({
        kind: "instruction",
        id: `inst:${region.start}`,
        instIdx: region.start,
        foldId: region.id,
        hiddenInstructionCount: Math.max(1, region.end - region.start),
      });
      idx = Math.max(idx + 1, region.end);
      continue;
    }
    visibleRows.push({ kind: "instruction", id: `inst:${idx}`, instIdx: idx, foldId: null, hiddenInstructionCount: 0 });
    idx += 1;
  }
  return {
    foldRegions,
    visibleRows,
    selectedVisibleRowId: selectedInstructionRowId(visibleRows, options.selectedInstIdx),
  };
}

export function buildInstructionPanelRows(
  foldModel: InstructionFoldModel,
  filteredInstructionIndices: number[],
): InstructionPanelRows {
  const allowed = new Set(filteredInstructionIndices);
  const rows = foldModel.visibleRows.filter((row) => {
    if (allowed.has(row.instIdx)) return true;
    for (let idx = row.instIdx; idx < row.instIdx + row.hiddenInstructionCount; idx += 1) {
      if (allowed.has(idx)) return true;
    }
    return false;
  });
  const visibleIndexByInstIdx = new Map<number, number>();
  rows.forEach((row, visibleIdx) => {
    visibleIndexByInstIdx.set(row.instIdx, visibleIdx);
    for (let idx = row.instIdx; idx < row.instIdx + row.hiddenInstructionCount; idx += 1) {
      if (allowed.has(idx)) {
        visibleIndexByInstIdx.set(idx, visibleIdx);
      }
    }
  });
  return { rows, visibleIndexByInstIdx };
}

function selectedSourceRowId(rows: SourceVisibleRow[], selectedLine: number | null): string | null {
  if (selectedLine == null) return null;
  const direct = rows.find((row) => row.kind === "line" && row.lineIndex === selectedLine);
  if (direct) return direct.id;
  const folded = rows.find((row) => row.kind === "fold" && row.lineIndex <= selectedLine && selectedLine < row.lineIndex + row.hiddenLineCount);
  return folded?.id ?? null;
}

function selectedInstructionRowId(rows: InstructionVisibleRow[], selectedIdx: number | null): string | null {
  if (selectedIdx == null) return null;
  const direct = rows.find((row) => row.instIdx === selectedIdx);
  if (direct) return direct.id;
  const folded = rows.find((row) => row.hiddenInstructionCount > 0 && row.instIdx <= selectedIdx && selectedIdx < row.instIdx + row.hiddenInstructionCount);
  return folded?.id ?? null;
}

function displayCommandPath(commandPath: number[]): string {
  const [root, ...nested] = commandPath;
  if (root == null) return "";
  return [`c${String(root).padStart(3, "0")}`, ...nested.map((part) => `n${String(part).padStart(3, "0")}`)].join("/");
}

function firstLine(text: string): string {
  return text.split("\n").map((line) => line.trim()).find(Boolean) ?? "";
}
