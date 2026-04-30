import type {
  DebugBreakpoint,
  DebugStoppedEvent,
  LoadProvenance,
  SourceMapEntry,
  TcldData,
} from "@/types";
import type { OutputLine } from "@/lib/debug-store";
import type { SessionState } from "@/lib/store-data";
import { formatOutputForCopy } from "@/lib/output-search";

export interface ComparisonModel {
  original: {
    available: boolean;
    label: string;
    text: string | null;
  };
  reconstructed: {
    label: string;
    text: string;
  };
  links: Array<{
    sourceMapIndex: number;
    commandGroup: number | null;
    bytecodeRange: [number, number];
    originalSpan: [number, number] | null;
    reconstructedLines: [number, number];
  }>;
  rows: ComparisonRow[];
  legendCounts: ComparisonLegendCounts;
}

export type ComparisonRowStatus = "match" | "partial" | "stripped";

export interface ComparisonRow {
  sourceMapIndex: number;
  commandGroup: number | null;
  status: ComparisonRowStatus;
  sourceText: string;
  reconstructedText: string;
  disassemblyText: string;
  bytecodeRange: [number, number];
  originalSpan: [number, number] | null;
  reconstructedLines: [number, number];
  originalAvailable: boolean;
  metadata: string;
  unavailableReason: string | null;
}

export type ComparisonLegendCounts = Record<ComparisonRowStatus, number>;

export interface ProvenanceRow {
  index: number;
  bytecodeRange: [number, number];
  sourceLines: [number, number];
  sourceChars: [number, number];
  commandGroup: number | null;
  commandPath: string;
  originalStatus: SourceMapEntry["originalStatus"];
  originalSpan: [number, number] | null;
  provenance: LoadProvenance | null;
  text: string;
}

function normalizeComparisonText(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

function spanText(text: string | null, span: [number, number] | null): string | null {
  if (!text || !span) return null;
  const [start, end] = span;
  if (start < 0 || end <= start || start >= text.length) return null;
  return text.slice(start, Math.min(end, text.length));
}

function reconstructedTextForEntry(data: TcldData, entry: SourceMapEntry, sourceMapIndex: number): string {
  const lines = data.decompileLines.filter((line) => {
    if (line.sourceMapIndex != null) return line.sourceMapIndex === sourceMapIndex;
    return entry.sourceLines[0] <= line.lineIndex && line.lineIndex < entry.sourceLines[1];
  });
  return lines.length > 0 ? lines.map((line) => line.text).join("\n") : entry.text;
}

function disassemblyTextForEntry(data: TcldData, entry: SourceMapEntry): string {
  const ops = data.instructions
    .filter((inst) => entry.bytecodeRange[0] <= inst.pc && inst.pc < entry.bytecodeRange[1])
    .map((inst) => inst.args ? `${inst.op} ${inst.args}` : inst.op);
  return ops.length > 0 ? ops.join(" / ") : `bytecode ${entry.bytecodeRange[0]}..${entry.bytecodeRange[1]}`;
}

function classifyComparisonRow(args: {
  originalAvailable: boolean;
  entry: SourceMapEntry;
  sourceText: string | null;
  reconstructedText: string;
}): ComparisonRowStatus {
  if (!args.originalAvailable || args.entry.originalStatus !== "available" || !args.entry.originalSpan || !args.sourceText) {
    return "stripped";
  }
  return normalizeComparisonText(args.sourceText) === normalizeComparisonText(args.reconstructedText)
    ? "match"
    : "partial";
}

function countComparisonRows(rows: ComparisonRow[]): ComparisonLegendCounts {
  return rows.reduce<ComparisonLegendCounts>((counts, row) => {
    counts[row.status] += 1;
    return counts;
  }, { match: 0, partial: 0, stripped: 0 });
}

export function buildComparisonModel(data: TcldData, originalSourceText: string | null): ComparisonModel {
  const originalAvailable = data.provenance?.originalSourceAvailable === true && originalSourceText != null;
  const rows = (data.sourceMap ?? []).map((entry, sourceMapIndex) => {
    const sourceText = spanText(originalSourceText, entry.originalSpan);
    const reconstructedText = reconstructedTextForEntry(data, entry, sourceMapIndex);
    const disassemblyText = disassemblyTextForEntry(data, entry);
    const status = classifyComparisonRow({ originalAvailable, entry, sourceText, reconstructedText });
    const sourceDisplayText = status === "stripped"
      ? "Original source unavailable."
      : (sourceText ?? "Original source span unavailable.");
    return {
      sourceMapIndex,
      commandGroup: entry.commandGroup,
      status,
      sourceText: sourceDisplayText,
      reconstructedText,
      disassemblyText,
      bytecodeRange: entry.bytecodeRange,
      originalSpan: entry.originalSpan,
      reconstructedLines: entry.sourceLines,
      originalAvailable: status !== "stripped",
      metadata: `#${sourceMapIndex} bc ${entry.bytecodeRange[0]}..${entry.bytecodeRange[1]} source ${entry.sourceLines[0]}..${entry.sourceLines[1]}`,
      unavailableReason: status === "stripped" ? "Original source text/span unavailable for this source-map entry." : null,
    };
  });
  return {
    original: {
      available: originalAvailable,
      label: originalAvailable ? "Original Source" : "Original Source Unavailable",
      text: originalAvailable ? originalSourceText : null,
    },
    reconstructed: {
      label: "Reconstructed Source",
      text: data.decompileText,
    },
    links: (data.sourceMap ?? []).map((entry, sourceMapIndex) => ({
      sourceMapIndex,
      commandGroup: entry.commandGroup,
      bytecodeRange: entry.bytecodeRange,
      originalSpan: entry.originalSpan,
      reconstructedLines: entry.sourceLines,
    })),
    rows,
    legendCounts: countComparisonRows(rows),
  };
}

export function buildProvenanceRows(data: TcldData): ProvenanceRow[] {
  return (data.sourceMap ?? []).map((entry, index) => ({
    index,
    bytecodeRange: entry.bytecodeRange,
    sourceLines: entry.sourceLines,
    sourceChars: entry.sourceChars,
    commandGroup: entry.commandGroup,
    commandPath: entry.commandPath.join("."),
    originalStatus: entry.originalStatus,
    originalSpan: entry.originalSpan,
    provenance: data.provenance ?? null,
    text: entry.text,
  }));
}

export function toggleBreakpointEnabled(bp: DebugBreakpoint): DebugBreakpoint {
  return {
    ...bp,
    enabled: !bp.enabled,
  };
}

export function resolveBreakpointTarget(args: {
  data: TcldData | null;
  selectedInstructionIndex: number | null;
  stoppedAt: DebugStoppedEvent | null;
  scriptPath: string;
}): { file: string; line: number; source: "selection" | "stopped" } | null {
  const { data, selectedInstructionIndex, stoppedAt, scriptPath } = args;
  if (data && selectedInstructionIndex != null) {
    const inst = data.instructions[selectedInstructionIndex];
    if (inst?.group != null) {
      const line = data.decompileLines.find((candidate) => candidate.group === inst.group);
      const sourceLine = line?.sourceLineStart ?? (line ? line.lineIndex + 1 : null);
      if (sourceLine != null && scriptPath) {
        return { file: scriptPath, line: sourceLine, source: "selection" };
      }
    }
  }
  if (stoppedAt?.file && stoppedAt.line > 0) {
    return { file: stoppedAt.file, line: stoppedAt.line, source: "stopped" };
  }
  return null;
}

export function buildFullReport(args: {
  data: TcldData;
  session: SessionState;
  output: OutputLine[];
  stoppedAt?: DebugStoppedEvent | null;
}): string {
  const { data, session, output, stoppedAt } = args;
  const diagnostics = data.diagnostics;
  const provenance = data.provenance;
  const lines = [
    "# Tcld Full Report",
    "",
    "## Metadata",
    `File: ${data.metadata.filename}`,
    `Path: ${data.metadata.path}`,
    `Tcl Version: ${data.metadata.tclVersion}`,
    `Instructions: ${data.metadata.instructionCount}`,
    `Commands: ${data.metadata.commandCount}`,
    "",
    "## Provenance",
    `Kind: ${provenance?.kind ?? "unknown"}`,
    `Label: ${provenance?.label ?? "unknown"}`,
    `Original Source: ${provenance?.originalSourceAvailable ? "available" : "unavailable"}`,
    "",
    "## Reconstruction",
    `Reconstruction: ${data.reconstruction ?? diagnostics?.reconstruction ?? "unknown"}`,
    `Unsupported Instructions: ${diagnostics?.unsupportedInstructions.join(", ") || "none"}`,
    `Unreduced Opcodes: ${diagnostics?.unreducedOpcodes.join(", ") || "none"}`,
    `Approximations: ${diagnostics?.approximations.map((approximation) => `${approximation.kind} ${approximation.bytecodeRange ? `${approximation.bytecodeRange[0]}..${approximation.bytecodeRange[1]}` : "nested procedure"}: ${approximation.reason}`).join("; ") || "none"}`,
    `Suppressed Fast Paths: ${diagnostics?.suppressedFastPathRanges.map((range) => `${range[0]}..${range[1]}`).join(", ") || "none"}`,
    `Pass Decisions: ${diagnostics?.passDecisions.map((decision) => `${decision.name}:${decision.enabled ? "on" : "off"}:${decision.changed ? "changed" : "unchanged"}`).join(", ") || "none"}`,
    "",
    "## Source Map Summary",
    ...buildProvenanceRows(data).map((row) => `${row.index}: bc ${row.bytecodeRange[0]}..${row.bytecodeRange[1]} source ${row.sourceLines[0]}..${row.sourceLines[1]} original=${row.originalStatus} path=${row.commandPath}`),
    "",
    "## Decompiled Tcl",
    data.decompileText,
    "",
    "## Disassembly",
    data.dumpText,
    "",
    "## Dump Text",
    data.dumpText,
    "",
    "## Debugger Session",
    stoppedAt
      ? `Stopped: ${stoppedAt.reason} ${stoppedAt.file}:${stoppedAt.line}`
      : "No stopped debugger context.",
    stoppedAt?.watchpointName
      ? `Watchpoint: ${stoppedAt.watchpointName}${stoppedAt.watchpointElement ? `(${stoppedAt.watchpointElement})` : ""} ${stoppedAt.watchpointOperation ?? ""} ${stoppedAt.watchpointOldValue ?? ""} -> ${stoppedAt.watchpointNewValue ?? ""}`
      : "",
    "",
    "## Console Output",
    formatOutputForCopy(output),
    "",
    "## Open Documents",
    ...session.documents.map((doc) => `${doc.id} ${doc.role} ${doc.status} ${doc.label} provenance=${doc.provenance.kind}`),
  ];

  return lines.filter((line, index, all) => line !== "" || all[index - 1] !== "").join("\n");
}
