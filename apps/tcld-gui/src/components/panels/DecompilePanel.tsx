"use client";

import React, { useMemo, useState, useRef, useEffect, useCallback } from "react";
import { createPortal } from "react-dom";
import { useTcldStore } from "@/lib/store";
import { useDebugStore } from "@/lib/debug-store";
import { DEBUGGER_UNAVAILABLE_MESSAGE } from "@/lib/capabilities";
import { tokenizeTclLine, type TclToken } from "@/lib/tcl-tokenizer";
import type { SyntaxToken, DecompileLine, DebugBreakpoint } from "@/types";
import { Panel } from "./Panel";
import { clsx } from "clsx";
import { ArrowRight, ChevronDown, ChevronRight, Circle, Columns2, Copy, FileCode2, Link2, Palette, Play, Unlink2 } from "lucide-react";
import { T } from "@/lib/theme";
import { useContextMenu } from "@/hooks/use-context-menu";
import { ContextMenu, type ContextMenuItem } from "@/components/overlays/ContextMenu";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useCommands } from "@/hooks/use-commands";
import { getRowMetrics } from "@/lib/row-metrics";
import { useLocationStore } from "@/lib/location-store";
import { hoverBus } from "@/lib/hover-bus";
import { useHoverHighlight } from "@/hooks/use-hover-highlight";
import { getPanelColumns, savePanelColumns } from "@/lib/settings";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";
import { subscribePanelAction } from "@/lib/panel-action-bus";
import { commandRegistry } from "@/lib/command-registry";
import { useSourceSyncStore } from "@/lib/source-sync-store";
import { buildDecompileFoldBlocks } from "@/lib/folding-model";

// ─── Types ──────────────────────────────────────────────────────

interface DecompilePanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
  debugActive?: boolean;
  breakpoints?: DebugBreakpoint[];
  /** Single-click on the gutter — toggles a breakpoint between
   *  enabled / disabled (or creates an enabled one when none exists). */
  onToggleBreakpoint?: (line: number) => void;
  /** Right-click action for removing the breakpoint. */
  onRemoveBreakpoint?: (line: number) => void;
  onRunToCursor?: (line: number) => void;
}

type DecompileColumnId = "lineNumbers" | "groupLabels" | "foldMarkers" | "breakpointGutter";

const DEFAULT_DECOMPILE_COLUMNS: Record<DecompileColumnId, boolean> = {
  lineNumbers: true,
  groupLabels: false,
  foldMarkers: true,
  breakpointGutter: true,
};

type VisibleDecompileRow = { kind: "line"; line: DecompileLine; idx: number; isFolded: boolean; foldEnd?: number };

// ─── Source Folding ─────────────────────────────────────────────

function foldedHeaderText(lines: DecompileLine[], startIdx: number, endIdx: number): string {
  const parts: string[] = [];
  const lastHeaderCandidate = Math.min(endIdx, startIdx + 8);
  for (let idx = startIdx; idx <= lastHeaderCandidate; idx += 1) {
    const text = lines[idx]?.text.trim();
    if (!text) continue;
    parts.push(text);
    if (text.endsWith("{")) break;
  }
  return parts.join(" ").replace(/\s+/g, " ").trim();
}

function stripBodyOpenBrace(header: string): string {
  return header.trim().endsWith("{")
    ? header.trim().slice(0, -1).trimEnd()
    : header.trim();
}

function readTclHeaderWord(source: string, startIndex: number): { word: string; endIndex: number } | null {
  let index = startIndex;
  while (index < source.length && /\s/.test(source[index])) {
    index += 1;
  }
  if (index >= source.length) return null;

  if (source[index] === "{") {
    let depth = 0;
    for (let cursor = index; cursor < source.length; cursor += 1) {
      const ch = source[cursor];
      if (ch === "{") depth += 1;
      if (ch === "}") {
        depth -= 1;
        if (depth === 0) {
          return { word: source.slice(index, cursor + 1), endIndex: cursor + 1 };
        }
      }
    }
    return { word: source.slice(index), endIndex: source.length };
  }

  let endIndex = index;
  while (endIndex < source.length && !/\s/.test(source[endIndex])) {
    endIndex += 1;
  }
  return { word: source.slice(index, endIndex), endIndex };
}

function foldedProcSummary(header: string): string {
  const procWord = readTclHeaderWord(header, 0);
  if (!procWord || procWord.word !== "proc") return "proc { ... }";
  const name = readTclHeaderWord(header, procWord.endIndex);
  const args = name ? readTclHeaderWord(header, name.endIndex) : null;
  if (!name) return "proc { ... }";
  return args
    ? `proc ${name.word} ${args.word} { ... }`
    : `proc ${name.word} { ... }`;
}

function foldedTclSummary(lines: DecompileLine[], startIdx: number, endIdx: number): string {
  const header = foldedHeaderText(lines, startIdx, endIdx);
  const base = stripBodyOpenBrace(header);
  const command = base.split(/\s+/, 1)[0] ?? "";

  if (command === "proc") {
    return foldedProcSummary(base);
  }

  if (command === "if" || command === "elseif" || command === "foreach" || command === "else") {
    return `${base} { ... }`;
  }

  return base ? `${base} { ... }` : "{ ... }";
}

function splitFoldedSummary(summary: string): { command: string; rest: string } {
  const match = /^(\S+)(.*)$/.exec(summary);
  return {
    command: match?.[1] ?? summary,
    rest: match?.[2] ?? "",
  };
}

/**
 * Subtle background color for a single command (group). Hues are spaced
 * via the golden angle so adjacent groups never share a color, and the
 * lightness/alpha keep the tint quiet against the panel background.
 *
 * Returns `undefined` when group is null so lines without a group stay
 * unstyled (the panel default background shows through).
 */
function commandColorForGroup(group: number | null | undefined): string | undefined {
  if (group == null) return undefined;
  const hue = Math.round((group * 137.508) % 360);
  return `hsl(${hue} 60% 55% / 0.06)`;
}

// ─── Byte Offset Builder ────────────────────────────────────────

function buildLineByteOffsets(text: string): number[] {
  const encoder = new TextEncoder();
  const offsets: number[] = [];
  let byteOffset = 0;
  for (const line of text.split("\n")) {
    offsets.push(byteOffset);
    byteOffset += encoder.encode(line).length + 1;
  }
  return offsets;
}

interface EditorCommandSegment {
  startByte: number;
  endByte: number;
}

const EDITOR_OPERATOR_PATTERN = /(>=|<=|==|!=|&&|\|\||>|<|\beq\b|\bne\b|\bin\b|\bni\b)/g;

function stringIndexToByteOffset(text: string, index: number): number {
  if (index <= 0) return 0;
  const clamped = Math.max(0, Math.min(index, text.length));
  return TEXT_ENCODER.encode(text.slice(0, clamped)).length;
}

function detectEditorCommandSegments(lineText: string): EditorCommandSegment[] {
  if (!lineText.trim()) return [];

  const segmentsByChar: { start: number; end: number }[] = [];
  const subcommandRanges: { start: number; end: number }[] = [];

  const firstNonWs = lineText.search(/\S/);
  if (firstNonWs >= 0) {
    const leading = /[^\s{\[]+/.exec(lineText.slice(firstNonWs));
    if (leading) {
      const start = firstNonWs + leading.index;
      segmentsByChar.push({ start, end: start + leading[0].length });
    }
  }

  let bracketDepth = 0;
  let escaped = false;
  let subcommandStart = -1;
  for (let i = 0; i < lineText.length; i += 1) {
    const ch = lineText[i];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (ch === "\\") {
      escaped = true;
      continue;
    }
    if (ch === "[") {
      if (bracketDepth === 0) {
        subcommandStart = i;
      }
      bracketDepth += 1;
      continue;
    }
    if (ch === "]" && bracketDepth > 0) {
      bracketDepth -= 1;
      if (bracketDepth === 0 && subcommandStart >= 0) {
        const range = { start: subcommandStart, end: i + 1 };
        subcommandRanges.push(range);
        segmentsByChar.push(range);
        subcommandStart = -1;
      }
    }
  }

  EDITOR_OPERATOR_PATTERN.lastIndex = 0;
  let operatorMatch: RegExpExecArray | null;
  while ((operatorMatch = EDITOR_OPERATOR_PATTERN.exec(lineText)) !== null) {
    const start = operatorMatch.index;
    const end = start + operatorMatch[0].length;
    const overlapsSubcommand = subcommandRanges.some((range) => start < range.end && end > range.start);
    if (!overlapsSubcommand) {
      segmentsByChar.push({ start, end });
    }
  }

  const normalized = segmentsByChar
    .filter((segment) => segment.end > segment.start)
    .sort((a, b) => (a.start - b.start) || (a.end - b.end));

  const merged: { start: number; end: number }[] = [];
  for (const segment of normalized) {
    const prev = merged[merged.length - 1];
    if (!prev || segment.start > prev.end) {
      merged.push({ ...segment });
    } else if (segment.end > prev.end) {
      prev.end = segment.end;
    }
  }

  const byteSegments = merged.map((segment) => ({
    startByte: stringIndexToByteOffset(lineText, segment.start),
    endByte: stringIndexToByteOffset(lineText, segment.end),
  }));

  if (byteSegments.length > 0) {
    return byteSegments;
  }

  const trimmedStart = lineText.length - lineText.trimStart().length;
  const trimmedEnd = lineText.trimEnd().length;
  return [{
    startByte: stringIndexToByteOffset(lineText, trimmedStart),
    endByte: stringIndexToByteOffset(lineText, trimmedEnd),
  }];
}

function selectActiveSegmentForInstruction(
  segments: EditorCommandSegment[],
  instructionIndices: number[],
  executionInstructionIndex: number | null,
): EditorCommandSegment | null {
  if (segments.length === 0 || executionInstructionIndex == null) return null;
  if (instructionIndices.length === 0) return segments[0] ?? null;

  const position = instructionIndices.indexOf(executionInstructionIndex);
  if (position < 0) return segments[0] ?? null;

  if (segments.length === 1 || instructionIndices.length === 1) {
    return segments[0];
  }

  const progress = position / (instructionIndices.length - 1);
  const segmentIndex = Math.min(
    segments.length - 1,
    Math.round(progress * (segments.length - 1)),
  );
  return segments[segmentIndex] ?? null;
}

// ─── Interactive Token ──────────────────────────────────────────

interface InteractiveTclLineProps {
  text: string;
  row: number;
  allTokens: SyntaxToken[];
  lineStartByte: number;
  highlightedVariable: string | null;
  onTokenClick: (token: TclToken, capture: string | null) => void;
  lineGroup: number | null;
  activePcSegment: EditorCommandSegment | null;
  executionStyled: boolean;
  foldDisclosure?: React.ReactNode;
}

const TEXT_ENCODER = new TextEncoder();

const InteractiveTclLine = React.memo(function InteractiveTclLine({
  text,
  row,
  allTokens,
  lineStartByte,
  highlightedVariable,
  onTokenClick,
  lineGroup,
  activePcSegment,
  executionStyled,
  foldDisclosure,
}: InteractiveTclLineProps) {
  const tokens = tokenizeTclLine(row, text, allTokens, lineStartByte);

  // Build a capture lookup for each token position
  const encoder = TEXT_ENCODER;
  const lineBytes = encoder.encode(text);
  const lineEndByte = lineStartByte + lineBytes.length;

  // Collect captures for this line
  const captures: { start: number; end: number; capture: string }[] = [];
  for (const tok of allTokens) {
    if (tok.startByte >= lineEndByte) break;
    if (tok.endByte <= lineStartByte) continue;
    captures.push({
      start: Math.max(tok.startByte, lineStartByte) - lineStartByte,
      end: Math.min(tok.endByte, lineEndByte) - lineStartByte,
      capture: tok.capture,
    });
  }

  // Match tokens to captures
  let bytePos = 0;
  const tokenCaptures: (string | null)[] = [];
  const tokenStarts: number[] = [];
  const tokenEnds: number[] = [];
  for (const tok of tokens) {
    const tokBytes = encoder.encode(tok.text).length;
    const tokStart = bytePos;
    const tokEnd = bytePos + tokBytes;
    // Find the most specific capture overlapping this token
    let bestCapture: string | null = null;
    for (const cap of captures) {
      if (cap.start < tokEnd && cap.end > tokStart) {
        bestCapture = cap.capture;
      }
    }
    tokenCaptures.push(bestCapture);
    tokenStarts.push(tokStart);
    tokenEnds.push(tokEnd);
    bytePos = tokEnd;
  }

  const renderToken = (
    tok: TclToken,
    i: number,
    key: React.Key,
    text: string = tok.text,
    startByte: number = tokenStarts[i] ?? 0,
    endByte: number = tokenEnds[i] ?? startByte,
  ) => {
    const capture = tokenCaptures[i];
    const isVariable = capture?.includes("variable");
    const isHighlighted = isVariable && highlightedVariable !== null && tok.text === highlightedVariable;
    const isCommand = capture?.includes("keyword") || capture?.includes("function");
    const tokenRole = capture?.includes("comment")
      ? "tcl-comment"
      : capture?.includes("string")
        ? "tcl-string"
        : capture?.includes("variable")
          ? "tcl-variable"
          : isCommand
            ? "tcl-command"
            : capture?.includes("punctuation")
              ? "tcl-punctuation"
              : "tcl-text";
    const isPcActive = activePcSegment != null
      && startByte < activePcSegment.endByte
      && endByte > activePcSegment.startByte;
    const highlightedBg = isPcActive
      ? `color-mix(in srgb, ${T.group} 22%, transparent)`
      : undefined;
    const color = executionStyled
      ? (isPcActive ? T.group : T.fg)
      : tok.color;
    const fontWeight = isPcActive
      ? 700
      : executionStyled
        ? 500
        : (tok.bold ? 600 : 400);

    return (
      <span
        key={key}
        data-syntax-token="source"
        data-token-role={tokenRole}
        style={{
          color,
          fontWeight,
          backgroundColor: isHighlighted
            ? `color-mix(in srgb, ${T.variable} 20%, transparent)`
            : highlightedBg,
          borderRadius: (isHighlighted || isPcActive) ? 2 : undefined,
          boxShadow: isPcActive
            ? `inset 0 -1px 0 0 color-mix(in srgb, ${T.group} 65%, transparent)`
            : undefined,
          cursor: (isVariable || isCommand) ? "pointer" : undefined,
        }}
        onClick={
          isVariable || isCommand
            ? () => {
                onTokenClick({ ...tok, text }, capture);
              }
            : undefined
        }
        title={
          isVariable
            ? `Variable: ${tok.text}`
            : isCommand
            ? `Command · Group c${lineGroup !== null ? String(lineGroup).padStart(3, "0") : "---"}`
            : undefined
        }
      >
        {text}
      </span>
    );
  };

  const renderedTokens: React.ReactNode[] = [];
  let insertedFoldDisclosure = false;
  tokens.forEach((tok, i) => {
    if (foldDisclosure && !insertedFoldDisclosure) {
      const firstTextMatch = /\S/.exec(tok.text);
      if (firstTextMatch) {
        const splitAt = firstTextMatch.index;
        const tokenStartByte = tokenStarts[i] ?? 0;
        if (splitAt > 0) {
          const prefix = tok.text.slice(0, splitAt);
          const suffix = tok.text.slice(splitAt);
          const prefixEndByte = tokenStartByte + encoder.encode(prefix).length;
          renderedTokens.push(renderToken(tok, i, `${i}:prefix`, prefix, tokenStartByte, prefixEndByte));
          renderedTokens.push(<React.Fragment key={`fold:${i}`}>{foldDisclosure}</React.Fragment>);
          renderedTokens.push(renderToken(tok, i, `${i}:suffix`, suffix, prefixEndByte, tokenEnds[i] ?? prefixEndByte));
        } else {
          renderedTokens.push(<React.Fragment key={`fold:${i}`}>{foldDisclosure}</React.Fragment>);
          renderedTokens.push(renderToken(tok, i, i));
        }
        insertedFoldDisclosure = true;
        return;
      }
    }

    renderedTokens.push(renderToken(tok, i, i));
  });

  if (foldDisclosure && !insertedFoldDisclosure) {
    renderedTokens.push(<React.Fragment key="fold:end">{foldDisclosure}</React.Fragment>);
  }

  return (
    <span className="font-mono text-ui leading-[var(--code-row-height)] whitespace-pre select-text">
      {renderedTokens}
    </span>
  );
});

// ─── Main Component ─────────────────────────────────────────────

export function DecompilePanel({
  id,
  focused,
  onClose,
  onFocus,
  debugActive,
  breakpoints,
  onToggleBreakpoint,
  onRemoveBreakpoint,
  onRunToCursor,
}: DecompilePanelProps) {
  const { cursor, execution, jumpToGroup, setCursor, index, pinA, pinB } = useLocationStore();
  const { data } = useTcldStore();
  const debug = useDebugStore();
  const lines = useMemo(() => data?.decompileLines ?? [], [data?.decompileLines]);
  const tokens = useMemo(() => data?.decompileTokens ?? [], [data?.decompileTokens]);
  const decompileText = useMemo(() => data?.decompileText ?? "", [data?.decompileText]);
  const liveText = debug.liveDecompileText ?? "";

  const [highlightedVariable, setHighlightedVariable] = useState<string | null>(null);
  const [foldedLines, setFoldedLines] = useState<Set<number>>(new Set());
  // Toggleable per-command background tinting. Off by default — on, each
  // group gets a subtle hue derived from its group id (golden-angle hue
  // distribution so adjacent groups never share a color).
  const [commandColoringEnabled, setCommandColoringEnabled] = useState(false);
  const [selectedLine, setSelectedLine] = useState<number | null>(null);
  const [showColumnMenu, setShowColumnMenu] = useState(false);
  const { linked: sourceLinked, toggleLinked: toggleSourceLinked } = useSourceSyncStore();
  const [columns, setColumns] = useState<Record<DecompileColumnId, boolean>>(
    () => {
      const defaults = { ...DEFAULT_DECOMPILE_COLUMNS };
      return getPanelColumns("decompile", defaults);
    },
  );
  const { menuState, showMenu, closeMenu } = useContextMenu();
  const containerRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const columnMenuRef = useRef<HTMLDivElement>(null);
  const columnButtonRef = useRef<HTMLButtonElement>(null);
  const columnDropdownRef = useRef<HTMLDivElement>(null);
  const columnMenuPosition = useAnchoredFloatingPosition(showColumnMenu, columnButtonRef, columnDropdownRef, { placement: "bottom-end" });
  const [rowMetrics, setRowMetrics] = useState(() => getRowMetrics());
  const focusedGroup = cursor.group;
  const debugExecution = useMemo(() => {
    const instIdx = debug.stoppedAt?.instructionIndex;
    if (instIdx == null || !data) return null;
    const inst = data.instructions[instIdx];
    if (!inst) return null;
    return {
      instIdx,
      group: inst.group ?? null,
      sourceFile: debug.stoppedAt?.file ?? null,
      sourceLine: debug.stoppedAt?.line ?? null,
    };
  }, [data, debug.stoppedAt?.file, debug.stoppedAt?.instructionIndex, debug.stoppedAt?.line]);
  const activeExecution = execution ?? debugExecution;
  const executionGroup = activeExecution?.group ?? null;
  const executionInstIdx = activeExecution?.instIdx ?? null;
  const executionSourceLine = activeExecution?.sourceLine ?? null;
  const executionPc = useMemo(
    () => (executionInstIdx != null && data ? (data.instructions[executionInstIdx]?.pc ?? null) : null),
    [data, executionInstIdx],
  );
  const instructionIndicesByGroup = useMemo(() => {
    const map = new Map<number, number[]>();
    if (!data) return map;
    for (const inst of data.instructions) {
      if (inst.group == null) continue;
      const groupInsts = map.get(inst.group);
      if (groupInsts) {
        groupInsts.push(inst.idx);
      } else {
        map.set(inst.group, [inst.idx]);
      }
    }
    return map;
  }, [data]);

  // Map executionInstIdx to the specific decompile line index for precise highlighting.
  // When instruction-stepping within a group, multiple decompile lines share the same group;
  // we need to identify which exact line the PC is on.
  const executionLineIdx = useMemo(() => {
    if (executionInstIdx == null || !data) return -1;
    const inst = data.instructions[executionInstIdx];
    if (!inst || inst.group == null) return -1;
    const candidates = lines
      .map((line, idx) => ({ line, idx }))
      .filter(({ line }) => line.group === inst.group);
    if (candidates.length === 0) return -1;

    if (executionSourceLine != null) {
      const exact = candidates.find(({ line }) =>
        line.sourceLineStart != null
        && line.sourceLineEnd != null
        && executionSourceLine >= Math.min(line.sourceLineStart, line.sourceLineEnd)
        && executionSourceLine <= Math.max(line.sourceLineStart, line.sourceLineEnd),
      );
      if (exact) return exact.idx;
    }

    if (candidates.length === 1) return candidates[0].idx;
    const groupInstructionIndices = instructionIndicesByGroup.get(inst.group) ?? [];
    const pos = groupInstructionIndices.indexOf(executionInstIdx);
    if (pos < 0 || groupInstructionIndices.length <= 1) return candidates[0].idx;
    const progress = pos / (groupInstructionIndices.length - 1);
    const linePos = Math.min(candidates.length - 1, Math.round(progress * (candidates.length - 1)));
    return candidates[linePos]?.idx ?? candidates[0].idx;
  }, [executionInstIdx, data, executionSourceLine, instructionIndicesByGroup, lines]);

  const toggleColumn = useCallback((columnId: DecompileColumnId) => {
    setColumns((prev) => {
      const next = { ...prev, [columnId]: !prev[columnId] };
      savePanelColumns("decompile", next);
      return next;
    });
  }, []);

  useEffect(() => {
    if (!showColumnMenu) return;
    const handleClickOutside = (event: MouseEvent) => {
      if (columnMenuRef.current && !columnMenuRef.current.contains(event.target as Node)) {
        setShowColumnMenu(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [showColumnMenu]);

  useEffect(() => {
    const refresh = () => setRowMetrics(getRowMetrics());
    refresh();
    window.addEventListener("resize", refresh);
    const observer = new MutationObserver(refresh);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["style", "class"],
    });
    return () => {
      window.removeEventListener("resize", refresh);
      observer.disconnect();
    };
  }, []);

  const lineByteOffsets = useMemo(
    () => buildLineByteOffsets(decompileText),
    [decompileText],
  );
  const lineNumberDigits = useMemo(
    () => Math.max(2, String(Math.max(lines.length, 1)).length),
    [lines.length],
  );
  const editorSegmentsByLine = useMemo(
    () => lines.map((line) => detectEditorCommandSegments(line.text)),
    [lines],
  );

  // Breakpoint lines (1-indexed)
  // Map of line → enabled flag for every breakpoint at that line. We track
  // disabled breakpoints too so the gutter dot still renders (just dimmer)
  // — disabled breakpoints are still real breakpoints the user has set;
  // hiding them entirely was misleading.
  const breakpointStates = useMemo(() => {
    const map = new Map<number, boolean>();
    for (const bp of breakpoints ?? []) {
      if (bp.kind === "instruction" || bp.line == null) continue;
      // Enabled wins if a line has both states (shouldn't happen, but
      // safe). The map key is the source line (1-indexed).
      map.set(bp.line, (map.get(bp.line) ?? false) || bp.enabled);
    }
    return map;
  }, [breakpoints]);
  const breakpointLines = useMemo(
    () => new Set(breakpointStates.keys()),
    [breakpointStates],
  );

  // The decompiler view consumes folding ranges produced by tcld-syntax on
  // the Rust side. That keeps Tcl structure parsing out of the React layer.
  const foldableBlocks = useMemo(() => buildDecompileFoldBlocks(data), [data]);
  const foldableGroupStarts = useMemo(() => {
    return new Set(foldableBlocks.keys());
  }, [foldableBlocks]);
  const foldedFoldableLineCount = useMemo(
    () => [...foldableGroupStarts].filter((line) => foldedLines.has(line)).length,
    [foldableGroupStarts, foldedLines],
  );
  const foldAllSource = useCallback(() => {
    setFoldedLines(new Set(foldableGroupStarts));
  }, [foldableGroupStarts]);
  const unfoldAllSource = useCallback(() => {
    setFoldedLines(new Set());
  }, []);

  useEffect(() => {
    const unsubscribeFold = subscribePanelAction("source:fold-all", foldAllSource);
    const unsubscribeUnfold = subscribePanelAction("source:unfold-all", unfoldAllSource);
    const unsubscribeLinked = subscribePanelAction("source:linked-navigation", toggleSourceLinked);
    return () => {
      unsubscribeFold();
      unsubscribeUnfold();
      unsubscribeLinked();
    };
  }, [foldAllSource, toggleSourceLinked, unfoldAllSource]);

  const visibleLines = useMemo(() => {
    const out: VisibleDecompileRow[] = [];
    let skipUntil = -1;
    for (let i = 0; i < lines.length; i++) {
      if (i <= skipUntil) continue;
      const foldEnd = foldableBlocks.get(i) ?? i;
      if (foldedLines.has(i) && foldEnd >= i) {
        out.push({ kind: "line", line: lines[i], idx: i, isFolded: true, foldEnd });
        skipUntil = foldEnd;
      } else {
        out.push({ kind: "line", line: lines[i], idx: i, isFolded: false });
      }
    }
    return out;
  }, [foldableBlocks, foldedLines, lines]);

  const lineToVisibleIndex = useMemo(() => {
    const map = new Map<number, number>();
    visibleLines.forEach((entry, visibleIndex) => {
      map.set(entry.idx, visibleIndex);
      if (entry.isFolded && entry.foldEnd != null) {
        for (let line = entry.idx + 1; line <= entry.foldEnd; line++) {
          map.set(line, visibleIndex);
        }
      }
    });
    return map;
  }, [visibleLines]);

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: visibleLines.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => rowMetrics.rowHeight,
    overscan: rowMetrics.overscan,
  });

  const hoverGroup = useCallback((location: { group?: number | null; scopeId?: string | null }) => {
    if (location.scopeId != null && index?.scopeId != null && location.scopeId !== index.scopeId) {
      return null;
    }
    return location.group ?? null;
  }, [index]);

  useHoverHighlight({
    containerRef: listRef,
    rowSelector: ".decompile-row",
    keyAttribute: "data-group",
    getKey: hoverGroup,
  });

  // Auto-scroll on sync.
  //
  // These effects must only fire when their actual TRIGGER changes
  // (focusedGroup / executionLineIdx). The `lineToVisibleIndex` map is
  // looked up but kept in a ref so it isn't a dependency — otherwise every
  // fold/unfold would rebuild lineToVisibleIndex and re-scroll back to the
  // focused/execution row, causing a visible jump after each fold toggle.
  const lineToVisibleIndexRef = useRef(lineToVisibleIndex);
  useEffect(() => { lineToVisibleIndexRef.current = lineToVisibleIndex; }, [lineToVisibleIndex]);

  useEffect(() => {
    if (!sourceLinked || focusedGroup === null) return;
    const firstLineIdx = lines.findIndex((l) => l.group === focusedGroup);
    if (firstLineIdx < 0) return;
    const visibleIndex = lineToVisibleIndexRef.current.get(firstLineIdx);
    if (visibleIndex != null) {
      virtualizer.scrollToIndex(visibleIndex, { align: "center" });
    }
  }, [focusedGroup, lines, sourceLinked, virtualizer]);

  // Auto-scroll to execution line when debugger stops
  useEffect(() => {
    if (!sourceLinked) return;
    const targetLineIdx = executionLineIdx >= 0 ? executionLineIdx : (
      executionGroup != null ? lines.findIndex((l) => l.group === executionGroup) : -1
    );
    if (targetLineIdx < 0) return;
    const visibleIndex = lineToVisibleIndexRef.current.get(targetLineIdx);
    if (visibleIndex != null) {
      virtualizer.scrollToIndex(visibleIndex, { align: "center" });
    }
  }, [executionGroup, executionLineIdx, lines, virtualizer, sourceLinked]);

  // Token click handler
  const handleTokenClick = useCallback(
    (token: TclToken, capture: string | null) => {
      if (capture?.includes("variable")) {
        setHighlightedVariable((prev) => (prev === token.text ? null : token.text));
      }
    },
    [],
  );

  // Line click → sync to disassembly
  const handleLineClick = useCallback(
    (group: number | null, lineIdx: number) => {
      if (group !== null) {
        const instIdx = index?.groupToFirstInstIdx.get(group) ?? null;
        if (instIdx != null) {
          setCursor({ instIdx, group, decompileLine: lineIdx }, { source: "user" });
        } else {
          jumpToGroup(group);
        }
      }
    },
    [index, jumpToGroup, setCursor],
  );

  // Fold toggle.
  //
  // No scroll-preservation logic needed: the virtualizer naturally keeps
  // scrollTop on the container across `visibleLines.length` changes, and
  // the fold-target line stays at its same visible-index in the new
  // visibleLines (only the lines AFTER it shift). The previous "jump to
  // top" was caused by an unrelated auto-scroll useEffect re-firing on
  // every lineToVisibleIndex rebuild — that's now fixed by reading the
  // index map through a ref so it's no longer an effect dependency.
  const toggleFold = useCallback((lineIdx: number) => {
    setFoldedLines((prev) => {
      const next = new Set(prev);
      if (next.has(lineIdx)) next.delete(lineIdx);
      else next.add(lineIdx);
      return next;
    });
  }, []);

  // Clear variable highlight on click outside
  const handleContainerClick = useCallback(() => {
    setHighlightedVariable(null);
  }, []);

  const clearHover = useCallback(() => {
    hoverBus.emit(null);
  }, []);

  const moveSelectedLine = useCallback((direction: -1 | 1) => {
    if (visibleLines.length === 0) return;
    const currentVisible = selectedLine == null
      ? 0
      : (lineToVisibleIndex.get(selectedLine) ?? 0);
    const lineRows = visibleLines
      .map((entry, visibleIndex) => ({ entry, visibleIndex }));
    if (lineRows.length === 0) return;
    const currentLineRow = lineRows.findIndex((entry) => entry.visibleIndex >= currentVisible);
    const nextLineRow = Math.max(0, Math.min((currentLineRow >= 0 ? currentLineRow : 0) + direction, lineRows.length - 1));
    const { entry: next, visibleIndex: nextVisible } = lineRows[nextLineRow];
    if (!next) return;

    setSelectedLine(next.idx);
    virtualizer.scrollToIndex(nextVisible, { align: "auto" });

    if (next.line.group != null) {
      handleLineClick(next.line.group, next.idx);
    }
  }, [handleLineClick, lineToVisibleIndex, selectedLine, visibleLines, virtualizer]);

  const commands = useMemo(() => ([
    {
      id: `${id}:line_up`,
      title: "Previous Source Line",
      category: "Source View",
      shortcut: ["ArrowUp"],
      when: "focusedPanel:decompile,!overlayOpen",
      run: () => moveSelectedLine(-1),
    },
    {
      id: `${id}:line_down`,
      title: "Next Source Line",
      category: "Source View",
      shortcut: ["ArrowDown"],
      when: "focusedPanel:decompile,!overlayOpen",
      run: () => moveSelectedLine(1),
    },
  ]), [id, moveSelectedLine]);

  useCommands(commands);

  const pinAFirstLineIdx = useMemo(() => {
    if (!pinA || pinA.scopeId !== (index?.scopeId ?? cursor.scopeId) || pinA.group == null) return -1;
    return lines.findIndex((l) => l.group === pinA.group);
  }, [cursor.scopeId, index?.scopeId, lines, pinA]);

  const pinBFirstLineIdx = useMemo(() => {
    if (!pinB || pinB.scopeId !== (index?.scopeId ?? cursor.scopeId) || pinB.group == null) return -1;
    return lines.findIndex((l) => l.group === pinB.group);
  }, [cursor.scopeId, index?.scopeId, lines, pinB]);

  if (lines.length === 0) {
    if (liveText) {
      const liveLines = liveText.split("\n");
      const encoder = new TextEncoder();
      let byteOffset = 0;
      const liveByteOffsets: number[] = [];
      for (const line of liveLines) {
        liveByteOffsets.push(byteOffset);
        byteOffset += encoder.encode(line).length + 1; // +1 for \n
      }
      return (
        <Panel title="Source View" badge="live" focused={focused} accent id={id} designSurface="view-source" onClose={onClose} onFocus={onFocus}>
          <div className="px-3 py-2 font-mono text-ui leading-[var(--code-row-height)] whitespace-pre overflow-auto flex-1">
            {liveLines.map((line, i) => {
              const toks = tokens.length > 0
                ? tokenizeTclLine(i, line, tokens, liveByteOffsets[i] ?? 0)
                : [{ text: line, color: T.fg }];
              return (
                <div key={i} className="flex items-center h-[var(--code-row-height)]">
                  <span className="w-[36px] flex-shrink-0 select-none pr-2 text-right font-mono text-meta tabular-nums text-tcld-fg-dim/50">
                    {i + 1}
                  </span>
                  <span className="select-text">
                    {toks.map((tok, j) => (
                      <span key={j} style={{ color: tok.color, fontWeight: tok.bold ? 600 : 400 }}>
                        {tok.text}
                      </span>
                    ))}
                  </span>
                </div>
              );
            })}
          </div>
        </Panel>
      );
    }
    return (
      <Panel title="Source View" badge="—" focused={focused} accent id={id} designSurface="view-source" onClose={onClose} onFocus={onFocus}>
        <div className="px-6 py-6 font-mono text-xs text-center text-tcld-fg-dim">
          {debugActive ? "waiting for bytecode chunks..." : "no decompilation available"}
        </div>
      </Panel>
    );
  }

  return (
    <Panel title="Source View" badge={String(lines.length)} focused={focused} accent id={id} designSurface="view-source" onClose={onClose} onFocus={onFocus}>
      <div className="decompile-editor-shell flex min-h-0 flex-1 flex-col">
        <div className="decompile-editor-toolbar flex items-center gap-2 px-2 py-1 min-w-0 flex-nowrap">
          <FileCode2 size={12} className="shrink-0 text-tcld-accent" aria-hidden />
          <span className="shrink min-w-0 truncate font-mono text-meta text-tcld-fg" title={data?.metadata.filename ?? "source.tcl"}>
            {data?.metadata.filename ?? "source.tcl"}
          </span>
          {/*
           * Breadcrumb resolution chain (most specific first):
           *  1. Active execution location (running / paused at this point)
           *  2. Cursor location from useLocationStore (user clicked a line)
           *  3. Selected line in this panel (decompile selection)
           *  4. First decompile line that has a group
           *  5. Em-dash placeholders
           */}
          {(() => {
            const fallbackLine = selectedLine != null ? lines[selectedLine] : undefined;
            const groupCandidate =
              executionGroup
              ?? cursor.group
              ?? fallbackLine?.group
              ?? lines.find((l) => l.group != null)?.group
              ?? null;
            const pcCandidate = (() => {
              if (executionPc != null) return executionPc;
              if (cursor.pc != null) return cursor.pc;
              // Map selected/first decompile line → first instruction in its group.
              const groupForPc = groupCandidate;
              if (groupForPc == null || !data) return null;
              const instIdx = data.instructions.findIndex((i) => i.group === groupForPc);
              return instIdx >= 0 ? data.instructions[instIdx].pc : null;
            })();
            return (
              <span className="hidden md:inline-flex shrink-0 items-center gap-1 font-mono text-label text-tcld-text-inactive">
                <ChevronRight size={9} aria-hidden />
                <span data-source-crumb="group">
                  {groupCandidate != null ? `c${String(groupCandidate).padStart(3, "0")}` : "c —"}
                </span>
                <ChevronRight size={9} aria-hidden />
                <span data-source-crumb="pc">
                  {pcCandidate != null ? `pc 0x${pcCandidate.toString(16).padStart(4, "0")}` : "pc —"}
                </span>
              </span>
            );
          })()}
          {executionSourceLine != null && (
            <span className="shrink-0 font-mono text-label text-tcld-fg-dim/80">:{executionSourceLine}</span>
          )}
          {highlightedVariable && (
            <span className="decompile-editor-varpill hidden rounded px-1.5 py-0.5 font-mono text-label md:inline shrink-0">
              {highlightedVariable}
            </span>
          )}
          <div className="flex-1 min-w-2" />

          <button
            type="button"
            data-source-toolbar-action="command-coloring"
            data-source-command-coloring={commandColoringEnabled ? "true" : "false"}
            onClick={() => setCommandColoringEnabled((prev) => !prev)}
            className={clsx(
              "rounded p-1 transition-colors shrink-0",
              commandColoringEnabled ? "text-tcld-accent bg-tcld-accent/10" : "text-tcld-text-inactive",
              "hover:bg-tcld-bg-highlight/70 hover:text-tcld-fg",
            )}
            title={commandColoringEnabled ? "Disable per-command coloring" : "Color each command differently"}
            aria-label="Toggle per-command coloring"
            aria-pressed={commandColoringEnabled}
          >
            <Palette size={12} aria-hidden />
          </button>

          {foldableGroupStarts.size > 0 && foldedFoldableLineCount >= foldableGroupStarts.size ? (
            <button
              type="button"
              data-source-toolbar-action="unfold-all"
              data-fold-state="all-folded"
              onClick={() => commandRegistry.execute("source:unfold-all")}
              className="decompile-editor-toolbar-action shrink-0 is-active"
              title="Unfold all source command groups"
              aria-label="unfold all source command groups"
              aria-pressed="true"
            >
              <ChevronDown size={12} aria-hidden />
              <span className="sr-only">unfold all</span>
            </button>
          ) : (
            <button
              type="button"
              data-source-toolbar-action="fold-all"
              data-fold-state={foldedFoldableLineCount > 0 ? "partial" : "all-unfolded"}
              onClick={() => commandRegistry.execute("source:fold-all")}
              className={clsx(
                "decompile-editor-toolbar-action shrink-0",
                foldedFoldableLineCount > 0 && "is-active",
              )}
              title="Fold all source command groups"
              aria-label="fold all source command groups"
              aria-pressed={false}
            >
              <ChevronRight size={12} aria-hidden />
              <span className="sr-only">fold all</span>
            </button>
          )}
          <span
            className="sr-only"
            data-source-folded-count={`${foldedFoldableLineCount}/${foldableGroupStarts.size}`}
          >
            {foldedFoldableLineCount}/{foldableGroupStarts.size} folded
          </span>

          <button
            type="button"
            data-source-toolbar-action="linked"
            data-source-linked={sourceLinked ? "true" : "false"}
            onClick={toggleSourceLinked}
            className={clsx(
              "rounded p-1 transition-colors",
              sourceLinked ? "text-tcld-accent" : "text-tcld-fg-dim",
              "hover:bg-tcld-bg-highlight/70"
            )}
            title={sourceLinked ? "Linked navigation active (click to freeze)" : "Linked navigation frozen (click to resume)"}
          >
            {sourceLinked ? <Link2 size={12} /> : <Unlink2 size={12} />}
          </button>

          <div className="relative" ref={columnMenuRef}>
            <button
              ref={columnButtonRef}
              onClick={() => setShowColumnMenu((prev) => !prev)}
              className="rounded p-1 text-tcld-fg-dim transition-colors hover:bg-tcld-bg-highlight/70 hover:text-tcld-fg"
              title="Toggle visible columns"
            >
              <Columns2 size={12} />
            </button>
            {showColumnMenu && typeof document !== "undefined" && createPortal(
              <div
                ref={columnDropdownRef}
                className="fixed z-[var(--z-context-menu)] min-w-44 rounded border border-tcld-border bg-tcld-bg-1 p-2 shadow-[var(--shadow-popover)]"
                style={{
                  left: columnMenuPosition.left,
                  top: columnMenuPosition.top,
                  maxHeight: columnMenuPosition.maxHeight || undefined,
                  overflowY: "auto",
                  visibility: columnMenuPosition.ready ? "visible" : "hidden",
                }}
              >
                <ColumnToggleItem
                  label="Line Numbers"
                  enabled={columns.lineNumbers}
                  onToggle={() => toggleColumn("lineNumbers")}
                />
                <ColumnToggleItem
                  label="Group Labels"
                  enabled={columns.groupLabels}
                  onToggle={() => toggleColumn("groupLabels")}
                />
                <ColumnToggleItem
                  label="Fold Markers"
                  enabled={columns.foldMarkers}
                  onToggle={() => toggleColumn("foldMarkers")}
                />
                <ColumnToggleItem
                  label="Breakpoint Gutter"
                  enabled={columns.breakpointGutter}
                  onToggle={() => toggleColumn("breakpointGutter")}
                />
              </div>,
              document.body,
            )}
          </div>
        </div>

        <div
          ref={containerRef}
          className="decompile-editor-surface flex-1 overflow-auto"
          onClick={handleContainerClick}
          onMouseLeave={clearHover}
        >
          <div
            ref={listRef}
            style={{
              height: virtualizer.getTotalSize(),
              position: "relative",
              minWidth: "100%",
              width: "fit-content",
            }}
          >
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const entry = visibleLines[virtualRow.index];
              if (!entry) return null;

              const { line, idx, isFolded, foldEnd } = entry;
              // First visible line of each command group acts as that group's
              // header row (it carries the fold disclosure and reads as the
              // command headline). The data-source-group-header /
              // data-source-group markers are the Source View parity contract
              // for "reconstructed source renders command group headers".
              const isGroupHeader = line.group != null && (idx === 0 || lines[idx - 1]?.group !== line.group);
              const isFocused = line.group === focusedGroup && focusedGroup !== null;
              const isExecution = executionLineIdx >= 0 ? idx === executionLineIdx : (line.group === executionGroup && executionGroup != null);
              const isExecutionGroup = !isExecution && line.group === executionGroup && executionGroup != null;
              const isSelected = selectedLine === idx;
              const lineNum = idx + 1;
              const hasBreakpoint = breakpointLines.has(lineNum);
              const isFoldable = foldableBlocks.has(idx);
              const isFoldedState = foldedLines.has(idx);
              const foldedLineCount = Math.max(0, (foldEnd ?? idx) - idx);
              const isPinA = line.group != null && pinA?.scopeId === (index?.scopeId ?? cursor.scopeId) && pinA.group === line.group;
              const isPinB = line.group != null && pinB?.scopeId === (index?.scopeId ?? cursor.scopeId) && pinB.group === line.group;
              const showPinA = isPinA && idx === pinAFirstLineIdx;
              const showPinB = !showPinA && isPinB && idx === pinBFirstLineIdx;
              const lineSegments = editorSegmentsByLine[idx] ?? [];
              const groupInstructionIndices = line.group != null ? (instructionIndicesByGroup.get(line.group) ?? []) : [];
              const foldedSummary = isFolded
                ? splitFoldedSummary(foldedTclSummary(lines, idx, foldEnd ?? idx))
                : null;
              const activePcSegment = isExecution
                ? (
                  executionInstIdx != null
                    ? selectActiveSegmentForInstruction(lineSegments, groupInstructionIndices, executionInstIdx)
                    : (lineSegments[0] ?? null)
                )
                : null;
              const foldDisclosure = columns.foldMarkers && isFoldable ? (
                <span className="relative inline-block h-[var(--code-row-height)] w-0 overflow-visible align-middle">
                  <button
                    type="button"
                    data-source-fold-disclosure="command"
                    data-fold-state={isFoldedState ? "collapsed" : "expanded"}
                    className={clsx(
                      "absolute right-[2px] top-1/2 inline-flex h-[14px] w-[14px] -translate-y-1/2 items-center justify-center",
                      "rounded border-0 bg-transparent p-0",
                      "text-tcld-text-inactive hover:text-tcld-accent",
                      "transition-opacity",
                      isFoldedState
                        ? "opacity-100"
                        : "opacity-0 group-hover/row:opacity-80 focus:opacity-100",
                    )}
                    aria-label={`${isFoldedState ? "Unfold" : "Fold"} source block`}
                    title={`${isFoldedState ? "Unfold" : "Fold"} source block`}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleFold(idx);
                    }}
                  >
                    {isFoldedState ? <ChevronRight size={12} aria-hidden /> : <ChevronDown size={12} aria-hidden />}
                  </button>
                </span>
              ) : null;

              return (
                <div
                  key={idx}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    minWidth: "100%",
                    height: virtualRow.size,
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                >
                  <div
                    data-group={line.group ?? ""}
                    data-idx={idx}
                    data-source-line={idx}
                    data-source-reconstructed="true"
                    data-source-group-header={isGroupHeader ? "true" : undefined}
                    data-source-group={isGroupHeader && line.group != null ? line.group : undefined}
                    data-fold-state={isGroupHeader && isFoldable ? (isFoldedState ? "collapsed" : "expanded") : undefined}
                    data-selected={isSelected ? "true" : undefined}
                    data-execution={isExecution ? "true" : undefined}
                    data-execution-group={isExecutionGroup ? "true" : undefined}
                    data-command-tinted={commandColoringEnabled && line.group != null ? "true" : undefined}
                    className={clsx(
                      "decompile-row",
                      "decompile-editor-row group/row flex h-[var(--code-row-height)] cursor-pointer items-stretch transition-colors"
                    )}
                    style={
                      commandColoringEnabled && line.group != null
                        ? { background: commandColorForGroup(line.group) }
                        : undefined
                    }
                    onClick={(e) => {
                      e.stopPropagation();
                      setSelectedLine(idx);
                      handleLineClick(line.group, idx);
                    }}
                    onMouseMove={() => {
                      const instIdx = line.group != null ? (index?.groupToFirstInstIdx.get(line.group) ?? null) : null;
                      const block = (instIdx != null && index)
                        ? index.instIdxToBlock[instIdx]
                        : -1;
                      hoverBus.emit({
                        scopeId: index?.scopeId ?? cursor.scopeId,
                        decompileLine: idx,
                        group: line.group,
                        instIdx,
                        blockId: block >= 0 ? block : null,
                      });
                    }}
                    onContextMenu={(e) => {
                      const menuItems: ContextMenuItem[] = [
                        {
                          label: "Run To Cursor",
                          icon: <Play size={12} aria-hidden />,
                          shortcut: "Ctrl+F5",
                          disabled: !onRunToCursor || !debug.debuggerAvailable,
                          onClick: () => onRunToCursor?.(lineNum),
                        },
                        // Add Breakpoint shows when no breakpoint is set;
                        // single-click on the gutter dot is the path for
                        // enable/disable on existing breakpoints. All
                        // breakpoint entries disable when the backend was
                        // built without the debugger feature.
                        ...(hasBreakpoint
                          ? [
                              {
                                label: breakpointStates.get(lineNum) === false
                                  ? "Enable Breakpoint"
                                  : "Disable Breakpoint",
                                icon: <Circle size={11} fill="currentColor" aria-hidden />,
                                shortcut: "F9",
                                disabled: !debug.debuggerAvailable,
                                onClick: () => onToggleBreakpoint?.(lineNum),
                              },
                              {
                                label: "Remove Breakpoint",
                                icon: <Circle size={11} aria-hidden />,
                                disabled: !onRemoveBreakpoint || !debug.debuggerAvailable,
                                onClick: () => onRemoveBreakpoint?.(lineNum),
                              },
                            ]
                          : [
                              {
                                label: "Add Breakpoint",
                                icon: <Circle size={11} fill="currentColor" aria-hidden />,
                                shortcut: "F9",
                                disabled: !debug.debuggerAvailable,
                                onClick: () => onToggleBreakpoint?.(lineNum),
                              },
                            ]),
                        {
                          label: "Go to Group",
                          icon: <ArrowRight size={12} aria-hidden />,
                          disabled: line.group === null,
                          onClick: () => line.group !== null && jumpToGroup(line.group),
                        },
                        {
                          label: "Copy Line",
                          icon: <Copy size={12} aria-hidden />,
                          onClick: () => navigator.clipboard.writeText(line.text),
                        },
                        { label: "", separator: true },
                        {
                          label: isFoldedState ? "Unfold" : "Fold",
                          icon: isFoldedState ? <ChevronDown size={12} aria-hidden /> : <ChevronRight size={12} aria-hidden />,
                          disabled: !isFoldable,
                          onClick: () => isFoldable && toggleFold(idx),
                        },
                      ];
                      showMenu(e, menuItems);
                    }}
                  >
                    <div
                      className={clsx(
                        "decompile-editor-gutter flex h-[var(--code-row-height)] flex-shrink-0 items-center",
                        isFocused && "decompile-editor-gutter-focus"
                      )}
                    >
                      {/* VS Code-style breakpoint area (leftmost) */}
                      {columns.breakpointGutter && (
                        <span
                          className={clsx(
                            "flex w-[20px] flex-shrink-0 items-center justify-center select-none",
                            debug.debuggerAvailable ? "cursor-pointer" : "cursor-not-allowed",
                          )}
                          data-source-breakpoint-gutter="true"
                          data-source-breakpoint-active={hasBreakpoint ? "true" : "false"}
                          data-source-breakpoint-unavailable={debug.debuggerAvailable ? undefined : "true"}
                          title={
                            !debug.debuggerAvailable
                              ? DEBUGGER_UNAVAILABLE_MESSAGE
                              : hasBreakpoint
                                ? `Remove breakpoint at line ${lineNum}`
                                : `Add breakpoint at line ${lineNum}`
                          }
                          onClick={(e) => {
                            e.stopPropagation();
                            if (!debug.debuggerAvailable) return;
                            onToggleBreakpoint?.(lineNum);
                          }}
                        >
                          {isExecution ? (
                            <Play size={11} fill="currentColor" strokeWidth={0} style={{ color: T.success }} aria-hidden />
                          ) : showPinA ? (
                            <span style={{ color: T.warning, fontSize: 10, fontWeight: 700 }}>A</span>
                          ) : showPinB ? (
                            <span style={{ color: T.number, fontSize: 10, fontWeight: 700 }}>B</span>
                          ) : (
                            <span
                              className={clsx(
                                "decompile-bp-dot inline-block rounded-full transition-opacity duration-75",
                                hasBreakpoint ? "h-[10px] w-[10px]" : "h-[8px] w-[8px]"
                              )}
                              data-active={hasBreakpoint || undefined}
                              data-bp-disabled={hasBreakpoint && breakpointStates.get(lineNum) === false ? "true" : undefined}
                            />
                          )}
                        </span>
                      )}

                      {/* Line numbers */}
                      {columns.lineNumbers && (
                        <span
                          className="decompile-editor-line-number flex-shrink-0 select-none pr-2 text-right font-mono text-meta tabular-nums"
                          style={{ width: `calc(${lineNumberDigits}ch + 8px)` }}
                        >
                          {lineNum}
                        </span>
                      )}

                      {columns.foldMarkers && (
                        <span
                          className="w-[18px] flex-shrink-0 select-none"
                          data-source-fold-paint-lane="true"
                          aria-hidden
                        />
                      )}
                    </div>

                    <div
                      className="decompile-editor-code flex min-w-0 flex-1 items-center"
                      style={{ paddingLeft: columns.foldMarkers ? 0 : undefined }}
                    >
                      {isFolded ? (
                        <span className="font-mono text-ui leading-[var(--code-row-height)] whitespace-pre select-text">
                          {/* Preserve the original indent in the collapsed
                              summary so the folded line still reads at the
                              correct depth. */}
                          <span aria-hidden>{line.text.match(/^\s*/)?.[0] ?? ""}</span>
                          {foldDisclosure}
                          <span style={{ color: T.keyword, fontWeight: 600 }}>
                            {foldedSummary?.command ?? line.text.trimStart().split(/\s/)[0]}
                          </span>
                          <span style={{ color: T.fgDim }}>{foldedSummary?.rest ?? " {...}"}</span>
                          {foldedLineCount > 0 && (
                            <span className="ml-2 text-label" style={{ color: T.comment }}>
                              ({foldedLineCount} lines)
                            </span>
                          )}
                        </span>
                      ) : (
                        <InteractiveTclLine
                          text={line.text}
                          row={idx}
                          allTokens={tokens}
                          lineStartByte={lineByteOffsets[idx] ?? 0}
                          highlightedVariable={highlightedVariable}
                          onTokenClick={handleTokenClick}
                          lineGroup={line.group}
                          activePcSegment={activePcSegment}
                          executionStyled={isExecution}
                          foldDisclosure={foldDisclosure}
                        />
                      )}
                    </div>

                    {columns.groupLabels && line.group != null && (
                      <div className="decompile-editor-group-pill mr-2 my-1 self-center rounded px-1.5 py-0.5 font-mono text-label">
                        c{String(line.group).padStart(3, "0")}
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        <div className="decompile-editor-status flex items-center gap-3 px-2.5">
          <span className="font-mono text-meta">
            Ln {executionLineIdx >= 0 ? executionLineIdx + 1 : "—"}, Col 1
          </span>
          <span className="font-mono text-meta">
            {executionInstIdx != null ? `i${executionInstIdx}` : "i—"}
          </span>
          <div className="flex-1" />
          <span className="font-mono text-meta">
            {debugActive ? "Tcl Debug" : "Tcl"}
          </span>
        </div>
      </div>

      {/* Context menu */}
      {menuState && (
        <ContextMenu
          items={menuState.items}
          position={menuState.position}
          onClose={closeMenu}
        />
      )}
    </Panel>
  );
}

function ColumnToggleItem({
  label,
  enabled,
  onToggle,
}: {
  label: string;
  enabled: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className="w-full text-left px-2 py-1 rounded hover:bg-tcld-bg-highlight font-mono text-xs flex items-center justify-between gap-2"
    >
      <span className="text-tcld-fg">{label}</span>
      <span className={enabled ? "text-tcld-accent" : "text-tcld-fg-dim"}>{enabled ? "on" : "off"}</span>
    </button>
  );
}
