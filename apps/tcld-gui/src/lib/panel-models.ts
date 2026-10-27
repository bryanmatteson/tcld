import type { DebugStoppedEvent, DiagnosticsReport, SourceCommandAnchor, SourceMapEntry, SymbolEntry, SymbolKind, TcldData } from "@/types";
import type { Location } from "@/lib/location-resolve";
import { buildComparisonModel, buildProvenanceRows, type ComparisonModel, type ProvenanceRow } from "@/lib/workflow-contracts";

export interface SourceMapSelection {
  sourceMapIndex: number | null;
  commandGroup: number | null;
  instructionIndex: number | null;
  sourceLine: number | null;
  unavailableReason: string | null;
}

export interface SourcePanelModel {
  available: boolean;
  text: string | null;
  lines: string[];
  selection: SourceMapSelection;
  sourceMapRows: Array<{
    index: number;
    selected: boolean;
    entry: SourceMapEntry;
  }>;
  sourceCommandRows: Array<{
    index: number;
    selected: boolean;
    anchor: SourceCommandAnchor;
  }>;
  unavailableMessage: string;
}

export interface ComparisonPanelModel extends ComparisonModel {
  selection: SourceMapSelection;
}

export interface DiagnosticsPanelModel {
  reconstruction: string;
  diagnostics: DiagnosticsReport | null;
  status: "unavailable" | "clean" | "issues";
  issueRows: DiagnosticsIssueRow[];
  issueCounts: DiagnosticsIssueCounts;
  linkedRows: Array<{
    kind: "approximation" | "unsupported" | "unreduced" | "suppressed" | "pass";
    label: string;
    sourceMapIndex: number | null;
    selected: boolean;
  }>;
  selection: SourceMapSelection;
}

export type DiagnosticsIssueSeverity = "err" | "warn" | "info";

export interface DiagnosticsIssueRow {
  id: string;
  kind: "approximation" | "unsupported" | "unreduced" | "suppressed" | "pass";
  severity: DiagnosticsIssueSeverity;
  severityLabel: "ERR" | "WARN" | "INFO";
  code: string;
  message: string;
  location: string;
  sourceMapIndex: number | null;
  selected: boolean;
}

export interface DiagnosticsIssueCounts {
  total: number;
  err: number;
  warn: number;
  info: number;
}

export interface SymbolRow {
  id: string;
  kind: SymbolKind;
  name: string;
  location: string | null;
  address: number | string | null;
  addressLabel: string;
  size: number | null;
  refs: number | null;
  sourceMapIndex: number | null;
  target: Partial<Location> | null;
}

export interface SymbolsPanelModel {
  status: "available" | "unavailable";
  filter: string;
  rows: SymbolRow[];
  visibleRows: SymbolRow[];
  totalCount: number;
  visibleCount: number;
  unavailableTitle: "Symbols unavailable";
  unavailableExplanation: "This payload does not expose namespace/proc/var symbol facts yet.";
  substituteLiteralCount: number;
  noMatchText: string | null;
}

export interface DiagnosticsSummaryRow {
  kind: "approximation" | "unsupported" | "unreduced" | "suppressed" | "pass";
  label: string;
  count: number;
  status: string;
}

export interface DiagnosticsSummaryModel {
  available: boolean;
  status: "unavailable" | "ok" | "issues";
  badge: string;
  message: string;
  rows: DiagnosticsSummaryRow[];
}

export interface ProvenancePanelModel {
  rows: Array<ProvenanceRow & { selected: boolean }>;
  selection: SourceMapSelection;
}

export type TracePassStatus = "matched" | "fixed" | "unmapped" | "skipped";

export type TraceEvidenceKind = "opcode" | "literal" | "source";

export interface TraceCommandRow {
  id: string;
  commandGroup: number | null;
  sourceMapIndex: number;
  label: string;
  pcRangeText: string;
  bytecodeRange: [number, number];
  passes: number;
  matched: string;
  changed: boolean;
  hot: boolean;
  selected: boolean;
}

export interface TracePassRow {
  index: number;
  name: string;
  iteration: number;
  status: TracePassStatus;
  changed: boolean;
  enabled: boolean;
  note: string;
}

export interface TraceEvidenceRow {
  kind: TraceEvidenceKind;
  badge: "OP" | "LIT" | "SRC";
  label: string;
  detail: string;
  target: Partial<Location> | null;
}

export interface TraceDecisionRow {
  outcome: "yes" | "no";
  text: string;
}

export interface TraceDetailModel {
  available: boolean;
  command: { id: string; label: string };
  construct: string;
  reconstruction: string;
  passesRan: number;
  changedPassCount: number;
  sourceSpan: string;
  sourcePreview: string;
  passChain: TracePassRow[];
  evidence: TraceEvidenceRow[];
  decisions: TraceDecisionRow[];
  unavailableReason: string | null;
}

export interface TracePanelModel {
  status: "available" | "unavailable";
  filter: string;
  commands: TraceCommandRow[];
  visibleCommands: TraceCommandRow[];
  totals: {
    commands: number;
    visible: number;
    changed: number;
    hot: number;
  };
  selection: SourceMapSelection;
  detail: TraceDetailModel;
  noMatchText: string | null;
  unavailableMessage: string | null;
}

export interface SourceMapCrossLinkTargets {
  source: {
    available: boolean;
    sourceLine: number | null;
    originalSpan: [number, number] | null;
    unavailableReason: string | null;
  };
  decompile: {
    lineIndex: number | null;
    commandGroup: number | null;
  };
  instruction: {
    instIdx: number | null;
    pc: number | null;
  };
  graph: {
    blockId: number | null;
    edgeTargets: number[];
  };
  detail: {
    instructionIndex: number | null;
  };
  diagnostics: {
    linkedRows: DiagnosticsPanelModel["linkedRows"];
  };
  provenance: {
    rowIndex: number | null;
  };
  comparison: {
    sourceMapIndex: number;
    originalSpan: [number, number] | null;
    reconstructedLines: [number, number];
  };
  debuggerStop: DebugStoppedEvent | null;
}

function sourceMapContainsPc(entry: SourceMapEntry, pc: number): boolean {
  return entry.bytecodeRange[0] <= pc && pc < entry.bytecodeRange[1];
}

export function resolveSourceMapSelection(data: TcldData, cursor: Location | null): SourceMapSelection {
  if (!cursor) {
    return {
      sourceMapIndex: null,
      commandGroup: null,
      instructionIndex: null,
      sourceLine: null,
      unavailableReason: "No cursor location selected.",
    };
  }

  const sourceMap = data.sourceMap ?? [];
  const instruction = cursor.instIdx != null ? data.instructions[cursor.instIdx] : undefined;
  const group = cursor.group ?? instruction?.group ?? null;
  const pc = cursor.pc ?? instruction?.pc ?? null;
  const decompileLine = cursor.decompileLine != null ? data.decompileLines[cursor.decompileLine] : undefined;

  let sourceMapIndex = decompileLine?.sourceMapIndex ?? null;
  if (sourceMapIndex == null && group != null) {
    sourceMapIndex = sourceMap.findIndex((entry) => entry.commandGroup === group);
  }
  if (sourceMapIndex == null && pc != null) {
    sourceMapIndex = sourceMap.findIndex((entry) => sourceMapContainsPc(entry, pc));
  }
  if (sourceMapIndex == null && cursor.sourceLine != null) {
    sourceMapIndex = sourceMap.findIndex(
      (entry) => entry.sourceLines[0] <= cursor.sourceLine! && cursor.sourceLine! < entry.sourceLines[1],
    );
  }
  if (sourceMapIndex != null && sourceMapIndex < 0) {
    sourceMapIndex = null;
  }

  const entry = sourceMapIndex != null ? sourceMap[sourceMapIndex] : null;
  return {
    sourceMapIndex,
    commandGroup: entry?.commandGroup ?? group,
    instructionIndex: instruction?.idx ?? null,
    sourceLine: cursor.sourceLine ?? data.decompileLines.find((line) => line.group === (entry?.commandGroup ?? group))?.sourceLineStart ?? null,
    unavailableReason: sourceMapIndex == null ? "Selected location has no source-map entry." : null,
  };
}

export function sourceMapSelectionTarget(
  data: TcldData,
  sourceMapIndex: number,
): Partial<Location> | null {
  const entry = data.sourceMap?.[sourceMapIndex];
  if (!entry) return null;

  const instruction = data.instructions.find((inst) => {
    if (entry.commandGroup != null && inst.group === entry.commandGroup && inst.groupStart) {
      return true;
    }
    return sourceMapContainsPc(entry, inst.pc);
  });

  const lineIndex = data.decompileLines.findIndex((line) => line.sourceMapIndex === sourceMapIndex);
  return {
    instIdx: instruction?.idx ?? null,
    pc: instruction?.pc ?? entry.bytecodeRange[0],
    group: entry.commandGroup,
    decompileLine: lineIndex >= 0 ? lineIndex : null,
    sourceLine: entry.sourceLines[0],
  };
}

export function sourceMapCrossLinkTargets(
  data: TcldData,
  sourceMapIndex: number,
  context: { stoppedAt?: DebugStoppedEvent | null } = {},
): SourceMapCrossLinkTargets {
  const entry = data.sourceMap?.[sourceMapIndex];
  if (!entry) {
    return {
      source: {
        available: false,
        sourceLine: null,
        originalSpan: null,
        unavailableReason: "Selected source-map entry does not exist.",
      },
      decompile: { lineIndex: null, commandGroup: null },
      instruction: { instIdx: null, pc: null },
      graph: { blockId: null, edgeTargets: [] },
      detail: { instructionIndex: null },
      diagnostics: { linkedRows: [] },
      provenance: { rowIndex: null },
      comparison: {
        sourceMapIndex,
        originalSpan: null,
        reconstructedLines: [0, 0],
      },
      debuggerStop: context.stoppedAt ?? null,
    };
  }

  const target = sourceMapSelectionTarget(data, sourceMapIndex);
  const instIdx = target?.instIdx ?? null;
  const instruction = instIdx != null ? data.instructions[instIdx] : undefined;
  const block = data.blocks.find((candidate) => {
    if (target?.blockId != null) return candidate.id === target.blockId;
    if (!instruction) return false;
    return candidate.start <= instruction.idx && instruction.idx <= candidate.end;
  });
  const diagnostics = buildDiagnosticsPanelModel(data, {
    scopeId: null,
    instIdx,
    pc: target?.pc ?? null,
    group: target?.group ?? entry.commandGroup,
    blockId: block?.id ?? null,
    decompileLine: target?.decompileLine ?? null,
    sourceFile: null,
    sourceLine: entry.sourceLines[0],
  });

  return {
    source: {
      available: data.provenance?.originalSourceAvailable === true && data.originalSourceText != null && entry.originalStatus === "available",
      sourceLine: entry.sourceLines[0],
      originalSpan: entry.originalSpan,
      unavailableReason: entry.originalStatus === "available" ? null : "Original source text unavailable for this source-map entry.",
    },
    decompile: {
      lineIndex: target?.decompileLine ?? null,
      commandGroup: entry.commandGroup,
    },
    instruction: {
      instIdx,
      pc: target?.pc ?? entry.bytecodeRange[0],
    },
    graph: {
      blockId: block?.id ?? null,
      edgeTargets: block?.edges ?? [],
    },
    detail: {
      instructionIndex: instIdx,
    },
    diagnostics: {
      linkedRows: diagnostics.linkedRows.filter((row) => row.sourceMapIndex === sourceMapIndex),
    },
    provenance: {
      rowIndex: sourceMapIndex,
    },
    comparison: {
      sourceMapIndex,
      originalSpan: entry.originalSpan,
      reconstructedLines: entry.sourceLines,
    },
    debuggerStop: context.stoppedAt ?? null,
  };
}

export function sourceCommandAnchorSelectionTarget(
  data: TcldData,
  anchorIndex: number,
): Partial<Location> | null {
  const anchor = data.sourceCommandAnchors?.[anchorIndex];
  if (!anchor) return null;
  const entry = data.sourceMap?.[anchor.sourceMapIndex];
  const rootGroup = anchor.parentRootGroup ?? entry?.parentCommandGroup ?? entry?.commandPath[0] ?? entry?.commandGroup ?? null;
  const instruction = data.instructions.find((inst) => {
    if (rootGroup != null && inst.group === rootGroup && inst.groupStart) {
      return true;
    }
    return entry ? sourceMapContainsPc(entry, inst.pc) : false;
  });
  const lineIndex = data.decompileLines.findIndex((line) => line.sourceMapIndex === anchor.sourceMapIndex)
    ?? -1;
  const fallbackLine = data.decompileLines.findIndex((line) => line.lineIndex === anchor.sourceLines[0]);
  return {
    instIdx: instruction?.idx ?? null,
    pc: instruction?.pc ?? entry?.bytecodeRange[0] ?? null,
    group: rootGroup,
    decompileLine: lineIndex >= 0 ? lineIndex : (fallbackLine >= 0 ? fallbackLine : null),
    sourceLine: anchor.sourceLines[0],
  };
}

export function buildSourcePanelModel(data: TcldData | null, cursor: Location | null): SourcePanelModel {
  const selection = data ? resolveSourceMapSelection(data, cursor) : {
    sourceMapIndex: null,
    commandGroup: null,
    instructionIndex: null,
    sourceLine: null,
    unavailableReason: "No document loaded.",
  };
  const text = data?.originalSourceText ?? null;
  const available = data?.provenance?.originalSourceAvailable === true && text != null;
  return {
    available,
    text: available ? text : null,
    lines: available ? text.split("\n") : [],
    selection,
    sourceMapRows: (data?.sourceMap ?? []).map((entry, index) => ({
      index,
      selected: index === selection.sourceMapIndex,
      entry,
    })),
    sourceCommandRows: (data?.sourceCommandAnchors ?? []).map((anchor, index) => ({
      index,
      selected: anchor.sourceMapIndex === selection.sourceMapIndex,
      anchor,
    })),
    unavailableMessage: data
      ? "Original source text unavailable for this document."
      : "No document loaded.",
  };
}

export function buildComparisonPanelModel(data: TcldData | null, cursor: Location | null): ComparisonPanelModel | null {
  if (!data) return null;
  return {
    ...buildComparisonModel(data, data.originalSourceText ?? null),
    selection: resolveSourceMapSelection(data, cursor),
  };
}

function sourceMapIndexForRange(sourceMap: SourceMapEntry[], range: [number, number]): number | null {
  const index = sourceMap.findIndex(
    (entry) => entry.bytecodeRange[0] < range[1] && range[0] < entry.bytecodeRange[1],
  );
  return index >= 0 ? index : null;
}

function sourceMapIndexForOpcode(data: TcldData, opcode: string): number | null {
  const instruction = data.instructions.find((inst) => inst.op === opcode);
  if (!instruction) return null;
  return sourceMapIndexForRange(data.sourceMap ?? [], [instruction.pc, instruction.pc + Math.max(1, instruction.size)]);
}

function sourceMapLocation(data: TcldData, sourceMapIndex: number | null, fallback: string): string {
  if (sourceMapIndex == null) return fallback;
  const entry = data.sourceMap?.[sourceMapIndex];
  if (!entry) return fallback;
  const filename = data.metadata.filename || data.metadata.path || "source";
  return `${filename}:${entry.sourceLines[0] + 1}`;
}

function firstTclWords(text: string, limit = 4): string[] {
  const words: string[] = [];
  let index = 0;
  const source = text.trimStart();
  while (index < source.length && words.length < limit) {
    while (index < source.length && /\s/.test(source[index]!)) index += 1;
    if (index >= source.length) break;

    let word = "";
    if (source[index] === "{") {
      let depth = 0;
      index += 1;
      while (index < source.length) {
        const ch = source[index]!;
        if (ch === "{") {
          depth += 1;
          word += ch;
        } else if (ch === "}") {
          if (depth === 0) {
            index += 1;
            break;
          }
          depth -= 1;
          word += ch;
        } else {
          word += ch;
        }
        index += 1;
      }
    } else if (source[index] === "\"") {
      index += 1;
      while (index < source.length) {
        const ch = source[index]!;
        if (ch === "\"") {
          index += 1;
          break;
        }
        word += ch;
        index += 1;
      }
    } else {
      while (index < source.length && !/\s/.test(source[index]!) && source[index] !== "{") {
        word += source[index]!;
        index += 1;
      }
    }

    if (word) words.push(word);
  }
  return words;
}

function normalizeSymbolName(name: string): string {
  return name.trim().replace(/^\$/, "");
}

function bytecodeLocation(range: [number, number] | null): string | null {
  if (!range) return null;
  return `bytecode ${range[0]}..${range[1]}`;
}

function formatSymbolAddress(address: number | string | null): string {
  if (address == null) return "-";
  if (typeof address === "number") return address.toString(16).padStart(4, "0");
  return address;
}

function symbolTargetFromSourceMap(data: TcldData, sourceMapIndex: number | null): Partial<Location> | null {
  if (sourceMapIndex == null) return null;
  return sourceMapSelectionTarget(data, sourceMapIndex);
}

function sourceMapSymbolRow(
  data: TcldData,
  kind: SymbolKind,
  name: string,
  sourceMapIndex: number,
): SymbolRow | null {
  const entry = data.sourceMap?.[sourceMapIndex];
  const normalized = normalizeSymbolName(name);
  if (!entry || !normalized) return null;
  const range: [number, number] = [entry.bytecodeRange[0], entry.bytecodeRange[1]];
  return {
    id: `${kind}:${normalized}:${sourceMapIndex}`,
    kind,
    name: normalized,
    location: bytecodeLocation(range),
    address: range[0],
    addressLabel: formatSymbolAddress(range[0]),
    size: Math.max(0, range[1] - range[0]),
    refs: null,
    sourceMapIndex,
    target: symbolTargetFromSourceMap(data, sourceMapIndex),
  };
}

function dtoSymbolRow(data: TcldData, symbol: SymbolEntry, index: number): SymbolRow | null {
  const name = normalizeSymbolName(symbol.name);
  if (!name) return null;
  const sourceMapIndex = symbol.sourceMapIndex ?? null;
  const bytecodeRange = symbol.bytecodeRange ?? null;
  const address = symbol.address ?? bytecodeRange?.[0] ?? null;
  return {
    id: `${symbol.kind}:${name}:dto:${index}`,
    kind: symbol.kind,
    name,
    location: symbol.location ?? bytecodeLocation(bytecodeRange),
    address,
    addressLabel: formatSymbolAddress(address),
    size: symbol.size ?? (bytecodeRange ? Math.max(0, bytecodeRange[1] - bytecodeRange[0]) : null),
    refs: symbol.refs ?? null,
    sourceMapIndex,
    target: symbolTargetFromSourceMap(data, sourceMapIndex),
  };
}

function dedupeSymbolRows(rows: SymbolRow[]): SymbolRow[] {
  const seen = new Set<string>();
  const deduped: SymbolRow[] = [];
  for (const row of rows) {
    const key = `${row.kind}:${row.name}`;
    if (seen.has(key)) continue;
    seen.add(key);
    deduped.push(row);
  }
  return deduped;
}

function rowsFromSourceMap(data: TcldData): SymbolRow[] {
  const rows: SymbolRow[] = [];
  for (const [sourceMapIndex, entry] of (data.sourceMap ?? []).entries()) {
    const words = firstTclWords(entry.text, 6);
    if (words[0] === "namespace" && words[1] === "eval" && words[2]) {
      const row = sourceMapSymbolRow(data, "ns", words[2], sourceMapIndex);
      if (row) rows.push(row);
      continue;
    }

    if (words[0] === "proc" && words[1]) {
      const procRow = sourceMapSymbolRow(data, "proc", words[1], sourceMapIndex);
      if (procRow) rows.push(procRow);
      const namespaceName = words[1].includes("::")
        ? words[1].split("::").filter(Boolean).slice(0, -1).join("::")
        : "";
      if (namespaceName) {
        const nsRow = sourceMapSymbolRow(data, "ns", namespaceName, sourceMapIndex);
        if (nsRow) rows.push(nsRow);
      }
      continue;
    }

    if ((words[0] === "variable" || words[0] === "global") && words[1]) {
      const names = words[0] === "global" ? words.slice(1) : [words[1]];
      for (const name of names) {
        const row = sourceMapSymbolRow(data, "var", name, sourceMapIndex);
        if (row) rows.push(row);
      }
      continue;
    }

    if (words[0] === "set" && words[1]) {
      const row = sourceMapSymbolRow(data, "var", words[1], sourceMapIndex);
      if (row) rows.push(row);
    }
  }
  return rows;
}

export function buildSymbolsPanelModel(data: TcldData | null, filter = ""): SymbolsPanelModel {
  const normalizedFilter = filter.trim().toLowerCase();
  const rows = data
    ? dedupeSymbolRows([
      ...(data.symbols ?? []).map((symbol, index) => dtoSymbolRow(data, symbol, index)).filter((row): row is SymbolRow => row !== null),
      ...rowsFromSourceMap(data),
    ])
    : [];
  const visibleRows = normalizedFilter
    ? rows.filter((row) => row.name.toLowerCase().includes(normalizedFilter))
    : rows;
  return {
    status: rows.length > 0 ? "available" : "unavailable",
    filter,
    rows,
    visibleRows,
    totalCount: rows.length,
    visibleCount: visibleRows.length,
    unavailableTitle: "Symbols unavailable",
    unavailableExplanation: "This payload does not expose namespace/proc/var symbol facts yet.",
    substituteLiteralCount: data?.literals.length ?? 0,
    noMatchText: rows.length > 0 && visibleRows.length === 0
      ? `No symbols match "${filter.trim()}".`
      : null,
  };
}

function rangeLocation(data: TcldData, range: [number, number], sourceMapIndex: number | null): string {
  return sourceMapLocation(data, sourceMapIndex, `bytecode ${range[0]}..${range[1]}`);
}

function opcodeLocation(data: TcldData, opcode: string, sourceMapIndex: number | null): string {
  const instruction = data.instructions.find((inst) => inst.op === opcode);
  const fallback = instruction ? `pc ${instruction.pc}` : `opcode ${opcode}`;
  return sourceMapLocation(data, sourceMapIndex, fallback);
}

function issueSeverityLabel(severity: DiagnosticsIssueSeverity): DiagnosticsIssueRow["severityLabel"] {
  if (severity === "err") return "ERR";
  if (severity === "warn") return "WARN";
  return "INFO";
}

function buildIssueRow(
  data: TcldData,
  selection: SourceMapSelection,
  row: Omit<DiagnosticsIssueRow, "severityLabel" | "selected">,
): DiagnosticsIssueRow {
  return {
    ...row,
    severityLabel: issueSeverityLabel(row.severity),
    selected: row.sourceMapIndex != null && row.sourceMapIndex === selection.sourceMapIndex,
  };
}

function countIssueRows(rows: DiagnosticsIssueRow[]): DiagnosticsIssueCounts {
  return rows.reduce<DiagnosticsIssueCounts>((counts, row) => {
    counts.total += 1;
    counts[row.severity] += 1;
    return counts;
  }, { total: 0, err: 0, warn: 0, info: 0 });
}

export function buildDiagnosticsPanelModel(data: TcldData | null, cursor: Location | null): DiagnosticsPanelModel {
  const diagnostics = data?.diagnostics ?? null;
  const selection = data ? resolveSourceMapSelection(data, cursor) : {
    sourceMapIndex: null,
    commandGroup: null,
    instructionIndex: null,
    sourceLine: null,
    unavailableReason: "No document loaded.",
  };
  const sourceMap = data?.sourceMap ?? [];
  const issueRows = data && diagnostics
    ? [
      ...diagnostics.approximations.map((approximation, index) => {
        const sourceMapIndex = approximation.bytecodeRange
          ? sourceMapIndexForRange(sourceMap, approximation.bytecodeRange)
          : null;
        return buildIssueRow(data, selection, {
          id: `approximation-${index}`,
          kind: "approximation",
          severity: "warn",
          code: "TBCD-APPROXIMATION",
          message: `${approximation.kind}: ${approximation.reason}`,
          location: approximation.bytecodeRange
            ? rangeLocation(data, approximation.bytecodeRange, sourceMapIndex)
            : "nested procedure",
          sourceMapIndex,
        });
      }),
      ...diagnostics.unsupportedInstructions.map((opcode, index) => {
        const sourceMapIndex = sourceMapIndexForOpcode(data, opcode);
        return buildIssueRow(data, selection, {
          id: `unsupported-${index}`,
          kind: "unsupported",
          severity: "err",
          code: "TBCD-UNSUPPORTED",
          message: `unsupported instruction ${opcode}`,
          location: opcodeLocation(data, opcode, sourceMapIndex),
          sourceMapIndex,
        });
      }),
      ...diagnostics.unreducedOpcodes.map((opcode, index) => {
        const sourceMapIndex = sourceMapIndexForOpcode(data, opcode);
        return buildIssueRow(data, selection, {
          id: `unreduced-${index}`,
          kind: "unreduced",
          severity: "warn",
          code: "TBCD-UNREDUCED",
          message: `unreduced opcode ${opcode}`,
          location: opcodeLocation(data, opcode, sourceMapIndex),
          sourceMapIndex,
        });
      }),
      ...diagnostics.suppressedFastPathRanges.map((range, index) => {
        const sourceMapIndex = sourceMapIndexForRange(sourceMap, range);
        return buildIssueRow(data, selection, {
          id: `suppressed-${index}`,
          kind: "suppressed",
          severity: "info",
          code: "TBCD-FASTPATH",
          message: `suppressed fast path for bytecode ${range[0]}..${range[1]}`,
          location: rangeLocation(data, range, sourceMapIndex),
          sourceMapIndex,
        });
      }),
      ...diagnostics.passDecisions
        .filter((decision) => !decision.enabled)
        .map((decision, index) => buildIssueRow(data, selection, {
          id: `pass-${index}`,
          kind: "pass",
          severity: "warn",
          code: "TBCD-PASS",
          message: `${decision.name} pass disabled at iteration ${decision.iteration}`,
          location: "pass decision",
          sourceMapIndex: null,
        })),
    ]
    : [];
  const issueCounts = countIssueRows(issueRows);
  const linkedRows = diagnostics
    ? [
      ...diagnostics.approximations.map((approximation) => ({
        kind: "approximation" as const,
        label: `${approximation.kind} ${approximation.bytecodeRange ? `${approximation.bytecodeRange[0]}..${approximation.bytecodeRange[1]}` : "nested procedure"}: ${approximation.reason}`,
        sourceMapIndex: approximation.bytecodeRange
          ? sourceMapIndexForRange(sourceMap, approximation.bytecodeRange)
          : null,
      })),
      ...diagnostics.unsupportedInstructions.map((opcode) => ({
        kind: "unsupported" as const,
        label: opcode,
        sourceMapIndex: null,
      })),
      ...diagnostics.unreducedOpcodes.map((opcode) => ({
        kind: "unreduced" as const,
        label: opcode,
        sourceMapIndex: null,
      })),
      ...diagnostics.suppressedFastPathRanges.map((range) => ({
        kind: "suppressed" as const,
        label: `${range[0]}..${range[1]}`,
        sourceMapIndex: sourceMapIndexForRange(sourceMap, range),
      })),
      ...diagnostics.passDecisions.map((decision) => ({
        kind: "pass" as const,
        label: `${decision.name} iteration=${decision.iteration} enabled=${decision.enabled} changed=${decision.changed}`,
        sourceMapIndex: null,
      })),
    ].map((row) => ({
      ...row,
      selected: row.sourceMapIndex != null && row.sourceMapIndex === selection.sourceMapIndex,
    }))
    : [];
  return {
    reconstruction: diagnostics?.reconstruction ?? "unknown",
    diagnostics,
    status: diagnostics ? (issueRows.length > 0 ? "issues" : "clean") : "unavailable",
    issueRows,
    issueCounts,
    linkedRows,
    selection,
  };
}

export function buildDiagnosticsSummaryModel(data: TcldData | null): DiagnosticsSummaryModel {
  const diagnostics = data?.diagnostics ?? null;
  if (!diagnostics) {
    return {
      available: false,
      status: "unavailable",
      badge: "unavailable",
      message: data ? "diagnostics unavailable for this load" : "no diagnostics loaded",
      rows: [],
    };
  }

  const approximationCount = diagnostics.approximations.length;
  const unsupportedCount = diagnostics.unsupportedInstructions.length;
  const unreducedCount = diagnostics.unreducedOpcodes.length;
  const suppressedCount = diagnostics.suppressedFastPathRanges.length;
  const passCount = diagnostics.passDecisions.length;
  const changedPassCount = diagnostics.passDecisions.filter((decision) => decision.changed).length;
  const disabledPassCount = diagnostics.passDecisions.filter((decision) => !decision.enabled).length;
  const issueCount = approximationCount + unsupportedCount + unreducedCount + suppressedCount + disabledPassCount;

  const rows: DiagnosticsSummaryRow[] = [
    {
      kind: "approximation",
      label: "Approximations",
      count: approximationCount,
      status: approximationCount > 0 ? `${approximationCount} record${approximationCount === 1 ? "" : "s"}` : "none",
    },
    {
      kind: "unsupported",
      label: "Unsupported",
      count: unsupportedCount,
      status: unsupportedCount > 0 ? diagnostics.unsupportedInstructions.slice(0, 2).join(", ") : "none",
    },
    {
      kind: "unreduced",
      label: "Unreduced",
      count: unreducedCount,
      status: unreducedCount > 0 ? diagnostics.unreducedOpcodes.slice(0, 2).join(", ") : "all opcodes reduced",
    },
    {
      kind: "suppressed",
      label: "Fast paths",
      count: suppressedCount,
      status: suppressedCount > 0 ? `${suppressedCount} suppressed` : "all enabled",
    },
    {
      kind: "pass",
      label: "Pass decisions",
      count: passCount,
      status: passCount > 0 ? `${changedPassCount} changed · ${disabledPassCount} disabled` : "none recorded",
    },
  ];

  return {
    available: true,
    status: issueCount > 0 ? "issues" : "ok",
    badge: issueCount > 0 ? `${issueCount}` : "ok",
    message: issueCount > 0
      ? `${issueCount} diagnostic signal${issueCount === 1 ? "" : "s"}`
      : "no approximations · all opcodes reduced · all fast paths enabled",
    rows,
  };
}

export function buildProvenancePanelModel(data: TcldData | null, cursor: Location | null): ProvenancePanelModel {
  const selection = data ? resolveSourceMapSelection(data, cursor) : {
    sourceMapIndex: null,
    commandGroup: null,
    instructionIndex: null,
    sourceLine: null,
    unavailableReason: "No document loaded.",
  };
  return {
    rows: data
      ? buildProvenanceRows(data).map((row) => ({
        ...row,
        selected: row.index === selection.sourceMapIndex,
      }))
      : [],
    selection,
  };
}

const TRACE_HOT_PASS_THRESHOLD = 3;

function passRangeOverlapsCommand(
  decision: { name: string },
  commandRange: [number, number],
  data: TcldData,
): boolean {
  const opcodeReducedNames = new Set(data.diagnostics?.unreducedOpcodes ?? []);
  const overlapsApproximation = (data.diagnostics?.approximations ?? []).some((approximation) =>
    approximation.bytecodeRange ? rangesOverlap(approximation.bytecodeRange, commandRange) : false);
  const overlapsSuppressed = (data.diagnostics?.suppressedFastPathRanges ?? []).some((range) => rangesOverlap(range, commandRange));
  if (overlapsApproximation || overlapsSuppressed) return true;
  if (opcodeReducedNames.size === 0) return decision.name.length > 0;
  const opcodesInRange = data.instructions
    .filter((inst) => commandRange[0] <= inst.pc && inst.pc < commandRange[1])
    .map((inst) => inst.op);
  return opcodesInRange.some((op) => opcodeReducedNames.has(op));
}

function rangesOverlap(a: [number, number], b: [number, number]): boolean {
  return a[0] < b[1] && b[0] < a[1];
}

function commandIdForGroup(commandGroup: number | null, sourceMapIndex: number): string {
  return commandGroup != null
    ? `c${String(commandGroup).padStart(3, "0")}`
    : `s${String(sourceMapIndex).padStart(3, "0")}`;
}

function commandLabelFor(entry: SourceMapEntry): string {
  const cleaned = (entry.text || "").trim().replace(/\s+/g, " ");
  if (!cleaned) return entry.commandPath.join(".") || "command";
  return cleaned.length > 80 ? `${cleaned.slice(0, 79)}…` : cleaned;
}

function classifyPassStatus(decision: { enabled: boolean; changed: boolean }, overlapsCommand: boolean): TracePassStatus {
  if (!decision.enabled) return "skipped";
  if (decision.changed && overlapsCommand) return "matched";
  if (decision.changed && !overlapsCommand) return "fixed";
  return "unmapped";
}

function passNoteFor(
  decision: { name: string; iteration: number; enabled: boolean; changed: boolean },
  status: TracePassStatus,
  range: [number, number],
): string {
  if (!decision.enabled) return `${decision.name} pass disabled at iteration ${decision.iteration}`;
  if (status === "matched") return `${decision.name} affected bytecode range ${range[0]}..${range[1]}`;
  if (status === "fixed") return `${decision.name} affected outside this command's range`;
  return `${decision.name} ran without changes in range ${range[0]}..${range[1]}`;
}

function buildTraceCommandRows(data: TcldData): TraceCommandRow[] {
  const sourceMap = data.sourceMap ?? [];
  return sourceMap.map((entry, sourceMapIndex) => {
    const passesAffecting = data.diagnostics
      ? data.diagnostics.passDecisions.filter((decision) => passRangeOverlapsCommand(decision, entry.bytecodeRange, data)).length
      : 0;
    const changed = data.diagnostics
      ? data.diagnostics.passDecisions.some((decision) => decision.changed && passRangeOverlapsCommand(decision, entry.bytecodeRange, data))
      : false;
    const approximationOverlap = (data.diagnostics?.approximations ?? []).some((approximation) =>
      approximation.bytecodeRange ? rangesOverlap(approximation.bytecodeRange, entry.bytecodeRange) : false);
    return {
      id: commandIdForGroup(entry.commandGroup, sourceMapIndex),
      commandGroup: entry.commandGroup,
      sourceMapIndex,
      label: commandLabelFor(entry),
      pcRangeText: `${entry.bytecodeRange[0]}..${entry.bytecodeRange[1]}`,
      bytecodeRange: entry.bytecodeRange,
      passes: passesAffecting,
      matched: matchedKindFor(entry, data),
      changed,
      hot: approximationOverlap || passesAffecting >= TRACE_HOT_PASS_THRESHOLD,
      selected: false,
    };
  });
}

function matchedKindFor(entry: SourceMapEntry, data: TcldData): string {
  const constructs = data.diagnostics?.recognizedConstructs ?? [];
  const path = entry.commandPath.join(".").toLowerCase();
  const text = (entry.text ?? "").toLowerCase();
  const matched = constructs.find((construct) => {
    const probe = construct.toLowerCase();
    if (path.includes(probe) || text.includes(probe)) return true;
    const tail = probe.includes(":") ? probe.slice(probe.lastIndexOf(":") + 1) : probe;
    if (!tail) return false;
    return text.includes(tail);
  });
  if (matched) return matched;
  const head = (entry.text || "").trim().split(/\s+/)[0] ?? "";
  if (!head) return "command";
  if (/^(if|while|for|foreach|switch|catch|try)$/i.test(head)) return `keyword:${head.toLowerCase()}`;
  return "command";
}

function buildTraceDetailModel(
  data: TcldData,
  selection: SourceMapSelection,
  command: TraceCommandRow | null,
): TraceDetailModel {
  if (!command || selection.sourceMapIndex == null) {
    return {
      available: false,
      command: { id: "", label: "" },
      construct: "",
      reconstruction: data.diagnostics?.reconstruction ?? "unknown",
      passesRan: 0,
      changedPassCount: 0,
      sourceSpan: "",
      sourcePreview: "",
      passChain: [],
      evidence: [],
      decisions: [],
      unavailableReason: "No trace command selected.",
    };
  }
  const entry = (data.sourceMap ?? [])[selection.sourceMapIndex];
  if (!entry) {
    return {
      available: false,
      command: { id: command.id, label: command.label },
      construct: command.matched,
      reconstruction: data.diagnostics?.reconstruction ?? "unknown",
      passesRan: 0,
      changedPassCount: 0,
      sourceSpan: "",
      sourcePreview: "",
      passChain: [],
      evidence: [],
      decisions: [],
      unavailableReason: "Source-map entry unavailable for selection.",
    };
  }

  const decisions = data.diagnostics?.passDecisions ?? [];
  const passChain: TracePassRow[] = decisions.map((decision, index) => {
    const overlaps = passRangeOverlapsCommand(decision, entry.bytecodeRange, data);
    const status = classifyPassStatus(decision, overlaps);
    return {
      index,
      name: decision.name,
      iteration: decision.iteration,
      status,
      changed: decision.changed,
      enabled: decision.enabled,
      note: passNoteFor(decision, status, entry.bytecodeRange),
    };
  });

  const evidence: TraceEvidenceRow[] = [];
  data.instructions
    .filter((inst) => entry.bytecodeRange[0] <= inst.pc && inst.pc < entry.bytecodeRange[1])
    .slice(0, 6)
    .forEach((inst) => {
      evidence.push({
        kind: "opcode",
        badge: "OP",
        label: `${inst.op} @ pc ${inst.pc}`,
        detail: inst.args ?? "",
        target: { instIdx: inst.idx, pc: inst.pc },
      });
    });
  const literalCount = data.literals?.length ?? 0;
  const literalIndices = new Set<number>();
  if (literalCount > 0) {
    data.instructions
      .filter((inst) => entry.bytecodeRange[0] <= inst.pc && inst.pc < entry.bytecodeRange[1])
      .forEach((inst) => {
        for (const operand of inst.operands) {
          if (operand >= 0 && operand < literalCount) literalIndices.add(operand);
        }
      });
  }
  Array.from(literalIndices).slice(0, 4).forEach((litIdx) => {
    const literal = data.literals?.[litIdx];
    evidence.push({
      kind: "literal",
      badge: "LIT",
      label: `literal #${litIdx}`,
      detail: literal ? String(literal.value ?? "") : "",
      target: null,
    });
  });
  if (entry.sourceLines[0] >= 0) {
    evidence.push({
      kind: "source",
      badge: "SRC",
      label: `source line ${entry.sourceLines[0]}..${entry.sourceLines[1]}`,
      detail: commandLabelFor(entry),
      target: { sourceLine: entry.sourceLines[0] },
    });
  }

  const changedPassCount = passChain.filter((row) => row.changed).length;
  const passesRan = passChain.length;

  const constructs = data.diagnostics?.recognizedConstructs ?? [];
  const decisionsRows: TraceDecisionRow[] = [
    ...constructs.slice(0, 3).map<TraceDecisionRow>((construct) => ({
      outcome: "yes",
      text: `recognized as ${construct}`,
    })),
    ...(data.diagnostics?.unreducedOpcodes ?? []).slice(0, 2).map<TraceDecisionRow>((opcode) => ({
      outcome: "no",
      text: `declined to reduce ${opcode}`,
    })),
    ...(data.diagnostics?.suppressedFastPathRanges ?? []).slice(0, 2).map<TraceDecisionRow>((range) => ({
      outcome: "no",
      text: `suppressed fast path ${range[0]}..${range[1]}`,
    })),
  ];

  return {
    available: true,
    command: { id: command.id, label: command.label },
    construct: command.matched,
    reconstruction: data.diagnostics?.reconstruction ?? "unknown",
    passesRan,
    changedPassCount,
    sourceSpan: `lines ${entry.sourceLines[0]}..${entry.sourceLines[1]} chars ${entry.sourceChars[0]}..${entry.sourceChars[1]}`,
    sourcePreview: entry.text || "(no source preview available)",
    passChain,
    evidence,
    decisions: decisionsRows,
    unavailableReason: null,
  };
}

export function buildTracePanelModel(
  data: TcldData | null,
  cursor: Location | null,
  filter = "",
): TracePanelModel {
  if (!data) {
    return {
      status: "unavailable",
      filter: "",
      commands: [],
      visibleCommands: [],
      totals: { commands: 0, visible: 0, changed: 0, hot: 0 },
      selection: {
        sourceMapIndex: null,
        commandGroup: null,
        instructionIndex: null,
        sourceLine: null,
        unavailableReason: "No document loaded.",
      },
      detail: {
        available: false,
        command: { id: "", label: "" },
        construct: "",
        reconstruction: "unknown",
        passesRan: 0,
        changedPassCount: 0,
        sourceSpan: "",
        sourcePreview: "",
        passChain: [],
        evidence: [],
        decisions: [],
        unavailableReason: "No document loaded.",
      },
      noMatchText: null,
      unavailableMessage: "Load a .tbc payload to populate trace facts.",
    };
  }

  const selection = resolveSourceMapSelection(data, cursor);
  const commands = buildTraceCommandRows(data);
  const trimmed = filter.trim().toLowerCase();
  const visibleCommands = trimmed
    ? commands.filter((row) =>
        row.id.toLowerCase().includes(trimmed)
        || row.label.toLowerCase().includes(trimmed)
        || row.matched.toLowerCase().includes(trimmed),
      )
    : commands.slice();
  const selectedCommand = commands.find((row) => row.sourceMapIndex === selection.sourceMapIndex)
    ?? visibleCommands[0]
    ?? null;
  const annotated = commands.map((row) => ({
    ...row,
    selected: selectedCommand != null && row.sourceMapIndex === selectedCommand.sourceMapIndex,
  }));
  const annotatedVisible = visibleCommands.map((row) => ({
    ...row,
    selected: selectedCommand != null && row.sourceMapIndex === selectedCommand.sourceMapIndex,
  }));

  const detail = buildTraceDetailModel(data, {
    ...selection,
    sourceMapIndex: selectedCommand ? selectedCommand.sourceMapIndex : selection.sourceMapIndex,
    commandGroup: selectedCommand ? selectedCommand.commandGroup : selection.commandGroup,
  }, selectedCommand);

  return {
    status: commands.length > 0 ? "available" : "unavailable",
    filter,
    commands: annotated,
    visibleCommands: annotatedVisible,
    totals: {
      commands: commands.length,
      visible: annotatedVisible.length,
      changed: commands.filter((row) => row.changed).length,
      hot: commands.filter((row) => row.hot).length,
    },
    selection,
    detail,
    noMatchText: trimmed && annotatedVisible.length === 0 ? `No commands match "${filter}"` : null,
    unavailableMessage: commands.length === 0 ? "Load a .tbc payload to populate trace facts." : null,
  };
}
