"use client";

import React, { useRef, useEffect, useState, useCallback, useMemo } from "react";
import { createPortal } from "react-dom";
import { T } from "@/lib/theme";
import { useTcldStore } from "@/lib/store";
import { useDebugStore } from "@/lib/debug-store";
import { Panel } from "./Panel";
import { clsx } from "clsx";
import { ArrowRight, Bookmark, ChevronDown, ChevronLeft, ChevronRight, Circle, Columns, Copy, Filter, GitBranch, Link2, MessageSquare, Play, Unlink2 } from "lucide-react";
import type { Instruction, SourceMapEntry } from "@/types";
import type { Annotations } from "@/lib/annotations";

import { getOpcodeCategory, opcodeCategoryColor } from "@/lib/opcode-categories";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useContextMenu } from "@/hooks/use-context-menu";
import { ContextMenu, type ContextMenuItem } from "@/components/overlays/ContextMenu";
import { getDisassemblyRowMetrics } from "@/lib/row-metrics";
import { useCommands } from "@/hooks/use-commands";
import { commandRegistry } from "@/lib/command-registry";
import { useLocationStore } from "@/lib/location-store";
import { hoverBus } from "@/lib/hover-bus";
import { useHoverHighlight } from "@/hooks/use-hover-highlight";
import { getPanelColumns, loadSettings, savePanelColumns, withDisassemblyParityColumns } from "@/lib/settings";
import { tokenizeInstructionRow } from "@/lib/syntax-rendering";
import { buildInstructionFoldModel, buildInstructionPanelRows } from "@/lib/folding-model";
import { subscribePanelAction } from "@/lib/panel-action-bus";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";

// ─── Types ──────────────────────────────────────────────────────

interface JumpArrow {
  fromIdx: number;
  toIdx: number;
  fromPc: number;
  toPc: number;
  conditional: boolean;
  forward: boolean;
  lane: number;
}

export type InstructionPresentationMode = "instructions" | "ir" | "source";

type InstructionColumnId =
  | "breakpoint"
  | "marker"
  | "group"
  | "address"
  | "opcode"
  | "args"
  | "size"
  | "comment";

const DEFAULT_INSTRUCTION_COLUMNS: Record<InstructionColumnId, boolean> = {
  breakpoint: true,
  marker: true,
  group: true,
  address: true,
  opcode: true,
  args: true,
  size: false,
  comment: true,
};

const GROUP_COLORS = ["var(--accent)", "var(--info)", "var(--violet)", "var(--pink)", "var(--warn)"] as const;

interface CommandGroupMeta {
  label: string;
  detail: string;
  count: number;
  pcStart: number;
  pcEnd: number;
  sourceLabel: string;
  color: string;
}

type InstructionFilterMode =
  | "all"
  | "jumps"
  | "bookmarked"
  | "group"
  | "opcode-category"
  | "neighborhood";

interface InstructionPanelProps {
  id: string;
  presentationMode?: InstructionPresentationMode;
  onPresentationModeChange?: (mode: InstructionPresentationMode) => void;
  onSelect: (idx: number) => void;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
  annotations: Annotations;
  showJumpArrows: boolean;
  onToggleBookmark: (idx: number) => void;
  onSetComment: (idx: number, text: string) => void;
  onToggleJumpArrows: () => void;
  debugActive?: boolean;
  breakpointGroups?: Set<number>;
  onToggleBreakpointAtGroup?: (group: number) => void;
  onRunToCursorAtGroup?: (group: number) => void;
}

// ─── Jump Arrow Computation ─────────────────────────────────────

function computeJumpArrows(instructions: Instruction[]): JumpArrow[] {
  const arrows: JumpArrow[] = [];
  const pcToIdx = new Map<number, number>();
  for (const inst of instructions) {
    pcToIdx.set(inst.pc, inst.idx);
  }

  for (const inst of instructions) {
    if (inst.isJump && inst.jumpTarget !== null) {
      const targetIdx = pcToIdx.get(inst.jumpTarget);
      if (targetIdx === undefined) continue;
      const forward = targetIdx > inst.idx;
      const conditional = inst.op.includes("True") || inst.op.includes("False");
      arrows.push({
        fromIdx: inst.idx,
        toIdx: targetIdx,
        fromPc: inst.pc,
        toPc: inst.jumpTarget,
        conditional,
        forward,
        lane: 0,
      });
    }
  }

  // Assign lanes to avoid overlapping arrows
  // Sort by span length (shorter spans get inner lanes)
  arrows.sort((a, b) => {
    const spanA = Math.abs(a.toIdx - a.fromIdx);
    const spanB = Math.abs(b.toIdx - b.fromIdx);
    return spanA - spanB;
  });

  const lanes: { min: number; max: number }[] = [];
  for (const arrow of arrows) {
    const min = Math.min(arrow.fromIdx, arrow.toIdx);
    const max = Math.max(arrow.fromIdx, arrow.toIdx);
    let lane = 0;
    while (lane < lanes.length) {
      const existing = lanes[lane];
      // Check if this arrow overlaps with the lane
      if (min <= existing.max && max >= existing.min) {
        lane++;
      } else {
        break;
      }
    }
    arrow.lane = lane;
    if (lane >= lanes.length) {
      lanes.push({ min, max });
    } else {
      lanes[lane] = {
        min: Math.min(lanes[lane].min, min),
        max: Math.max(lanes[lane].max, max),
      };
    }
  }

  return arrows;
}

function pcLabel(pc: number): string {
  return `0x${pc.toString(16).padStart(4, "0")}`;
}

function commandGroupLabel(group: number | null): string {
  return group != null ? `c${String(group).padStart(3, "0")}` : "c---";
}

function groupColor(group: number | null): string {
  if (group == null) return "var(--fg-4)";
  return GROUP_COLORS[Math.abs(group) % GROUP_COLORS.length];
}

function compactSourceText(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

// ─── Jump Gutter SVG ────────────────────────────────────────────

function JumpGutterSVG({
  arrows,
  scrollTop,
  viewportHeight,
  rowHeight,
}: {
  arrows: JumpArrow[];
  scrollTop: number;
  viewportHeight: number;
  rowHeight: number;
}) {
  const maxLane = arrows.reduce((m, a) => Math.max(m, a.lane), 0);
  const gutterWidth = Math.max(16, (maxLane + 1) * 6 + 6);

  // Only render arrows visible in a generous viewport
  const visibleMin = scrollTop - 200;
  const visibleMax = scrollTop + viewportHeight + 200;

  return (
    <svg
      width={gutterWidth}
      height={viewportHeight}
      className="pointer-events-none sticky left-0 top-0"
      style={{ zIndex: 10, overflow: "visible" }}
    >
      <defs>
        <marker id="arrow-green" markerWidth="6" markerHeight="4" refX="5" refY="2" orient="auto">
          <path d="M0,0 L6,2 L0,4" fill={T.success} />
        </marker>
        <marker id="arrow-red" markerWidth="6" markerHeight="4" refX="5" refY="2" orient="auto">
          <path d="M0,0 L6,2 L0,4" fill={T.error} />
        </marker>
      </defs>
      <g transform={`translate(0, ${-scrollTop})`}>
        {arrows.map((arrow, i) => {
          const fromY = arrow.fromIdx * rowHeight + rowHeight / 2;
          const toY = arrow.toIdx * rowHeight + rowHeight / 2;
          const minY = Math.min(fromY, toY);
          const maxY = Math.max(fromY, toY);

          if (maxY < visibleMin || minY > visibleMax) return null;

          const x = gutterWidth - 3 - arrow.lane * 6;
          const color = arrow.forward ? T.success : T.error;
          const markerId = arrow.forward ? "arrow-green" : "arrow-red";
          const d = `M ${gutterWidth - 2} ${fromY} H ${x} V ${toY} H ${gutterWidth - 2}`;

          return (
            <path
              key={i}
              d={d}
              fill="none"
              stroke={color}
              strokeWidth={1.5}
              strokeDasharray={arrow.conditional ? "4 2" : undefined}
              markerEnd={`url(#${markerId})`}
              opacity={0.8}
            />
          );
        })}
      </g>
    </svg>
  );
}

// ─── Inline Comment Editor ──────────────────────────────────────

function CommentEditor({
  initialValue,
  onSave,
  onCancel,
}: {
  initialValue: string;
  onSave: (text: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initialValue);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  return (
    <input
      ref={inputRef}
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") onSave(value);
        if (e.key === "Escape") onCancel();
      }}
      onBlur={() => onSave(value)}
      className="ml-1 px-1 py-0 h-4 text-xs font-mono bg-tcld-bg-3 border border-tcld-accent rounded outline-none"
      style={{ color: T.warning, width: "200px" }}
      placeholder="comment..."
    />
  );
}

// ─── Instruction Row ────────────────────────────────────────────

interface InstructionRowProps {
  inst: Instruction;
  selected: boolean;
  isBookmarked: boolean;
  isExecutionPointer: boolean;
  isExecutionGroup: boolean;
  isPinA: boolean;
  isPinB: boolean;
  hasBreakpoint: boolean;
  foldable: boolean;
  folded: boolean;
  hiddenInstructionCount: number;
  groupMeta: CommandGroupMeta | null;
  selectedGroup: boolean;
  onToggleFold: () => void;
  userComment: string | undefined;
  isEditing: boolean;
  onJumpClick: (targetPc: number) => void;
  onStartEdit: () => void;
  onSaveComment: (text: string) => void;
  onCancelEdit: () => void;
  flashIdx: number | null;
  gutterPadding: number;
  columns: Record<InstructionColumnId, boolean>;
  /**
   * PC → instruction-index lookup, used to render jump-target labels as
   * `→ i145` instead of raw `→ 0xADDR`. Stable across renders (memoized in
   * the parent) so InstructionRow's React.memo equality holds; only changes
   * when the instruction set itself changes.
   */
  pcToInstIdx?: ReadonlyMap<number, number>;
  /**
   * Command-group nesting depth (0 = root, +1 per level). Used to render
   * a left indent of `depth * 12px` in addition to the jump-arrow gutter,
   * so nested commands read as nested in the IR view. Derived from
   * sourceMap commandPath; supplied by the parent so the row's memo
   * equality survives cursor updates.
   */
  depth?: number;
}

const InstructionRow = React.memo(function InstructionRow({
  inst,
  selected,
  isBookmarked,
  isExecutionPointer,
  isExecutionGroup,
  isPinA,
  isPinB,
  hasBreakpoint,
  foldable,
  folded,
  hiddenInstructionCount,
  groupMeta,
  selectedGroup,
  onToggleFold,
  userComment,
  isEditing,
  onJumpClick,
  onStartEdit,
  onSaveComment,
  onCancelEdit,
  flashIdx,
  gutterPadding,
  columns,
  pcToInstIdx,
  depth = 0,
}: InstructionRowProps) {
  const isEven = inst.idx % 2 === 0;
  const opcodeColor = opcodeCategoryColor(getOpcodeCategory(inst.op));
  const pcHex = pcLabel(inst.pc);
  const isFlashing = flashIdx === inst.idx;
  const pinColor = isPinA ? T.warning : (isPinB ? T.number : null);
  const isGroupHeader = inst.groupStart && groupMeta != null;
  const rowGroupColor = groupMeta?.color ?? groupColor(inst.group);
  const selectedVisual = selected || (isGroupHeader && selectedGroup);
  const tokenRoles = tokenizeInstructionRow(inst, {
    groupLabel: inst.group != null ? `c${String(inst.group).padStart(3, "0")}` : null,
    comment: inst.comment,
    execution: isExecutionPointer || isExecutionGroup,
  });

  const handleCopyPc = () => {
    navigator.clipboard.writeText(pcHex);
  };

  return (
    <div
      data-idx={inst.idx}
      data-command-group={inst.group ?? undefined}
      data-group-start={inst.groupStart ? "true" : undefined}
      data-selected={selected ? "true" : undefined}
      data-selected-group={selectedGroup ? "true" : undefined}
      data-foldable={foldable ? "true" : undefined}
      data-fold-state={foldable ? (folded ? "collapsed" : "expanded") : undefined}
      data-group-count={groupMeta?.count}
      data-group-range={groupMeta ? `${pcLabel(groupMeta.pcStart)}-${pcLabel(groupMeta.pcEnd)}` : undefined}
      className={clsx(
        "instruction-row",
        "flex items-center py-0 h-[var(--disassembly-row-height)] cursor-pointer transition-colors duration-75 font-mono text-xs leading-[var(--disassembly-row-height)]",
        "group/row",
        inst.groupStart && "border-t border-tcld-border-dim",
        isFlashing && "flash-row",
        isExecutionPointer
          ? "border-l-[3px] border-tcld-success"
          : selectedVisual
            ? "instruction-row-selected border-l-[3px]"
            : isExecutionGroup
              ? "border-l-[3px] border-tcld-success/40"
            : pinColor
              ? "border-l-[3px] border-dashed"
            : clsx(
                "border-l-[3px] border-transparent",
                isEven ? "bg-tcld-bg-2/40" : "bg-transparent",
                "hover:bg-tcld-bg-highlight/50"
              )
      )}
      data-nesting-depth={depth || undefined}
      style={{
        paddingLeft: gutterPadding + 2 + depth * 12,
        ...(isExecutionPointer
          ? { background: `color-mix(in srgb, ${T.success} 10%, transparent)` }
          : isExecutionGroup
            ? { background: `color-mix(in srgb, ${T.success} 5%, transparent)` }
            : selectedVisual
              ? { borderLeftColor: rowGroupColor }
            : pinColor
              ? {
                  background: `color-mix(in srgb, ${pinColor} 4%, transparent)`,
                  borderLeftColor: pinColor,
                  borderLeftStyle: "dashed",
                }
            : {}),
      }}
      title={`${inst.op} ${inst.args} | size:${inst.size} | group:${inst.group !== null ? inst.group : "none"}`}
    >
      <span
        className="tcld-disasm-group-rail"
        data-group-start={inst.groupStart ? "true" : undefined}
        style={{ color: rowGroupColor }}
        aria-hidden
      />

      {columns.breakpoint && (
        <span className="flex w-6 flex-shrink-0 items-center justify-center gap-0.5 text-center">
          {foldable ? (
            <button
              type="button"
              data-fold-disclosure="command"
              data-fold-state={folded ? "collapsed" : "expanded"}
              className="tcld-disasm-fold-control h-4 w-3 rounded text-label leading-4 text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-accent"
              onClick={(event) => {
                event.stopPropagation();
                onToggleFold();
              }}
              aria-label={`${folded ? "Unfold" : "Fold"} command${hiddenInstructionCount > 1 ? ` (${hiddenInstructionCount} instructions)` : ""}`}
              title={`${folded ? "Unfold" : "Fold"} command${hiddenInstructionCount > 1 ? ` (${hiddenInstructionCount} instructions)` : ""}`}
            >
              {folded ? <ChevronRight size={12} aria-hidden /> : <ChevronDown size={12} aria-hidden />}
            </button>
          ) : (
            <span className="h-4 w-3" />
          )}
          {hasBreakpoint ? (
            <Circle size={10} fill="currentColor" strokeWidth={0} style={{ color: T.error }} aria-hidden />
          ) : null}
        </span>
      )}

      {/* Execution pointer / Bookmark marker */}
      {columns.marker && (
        <span className="w-3 flex-shrink-0 text-center">
          {isExecutionPointer ? (
            <Play size={10} fill="currentColor" strokeWidth={0} style={{ color: T.success }} aria-hidden />
          ) : isBookmarked ? (
            <Bookmark size={10} fill="currentColor" strokeWidth={1.5} style={{ color: T.accent }} aria-hidden />
          ) : isPinA ? (
            <span style={{ color: T.warning, fontSize: 9, fontWeight: 700 }}>A</span>
          ) : isPinB ? (
            <span style={{ color: T.number, fontSize: 9, fontWeight: 700 }}>B</span>
          ) : null}
        </span>
      )}

      {/* Group label */}
      {columns.group && (
        <span
          className={clsx(
            "tcld-disasm-source-group-column w-[104px] text-xs flex-shrink-0 tabular-nums",
            inst.groupStart ? "font-semibold" : "text-tcld-fg-dim font-normal",
            inst.group !== null ? "opacity-100" : "opacity-30"
          )}
          style={isGroupHeader ? { color: rowGroupColor } : undefined}
          data-disasm-column="source-group"
        >
          {isGroupHeader && groupMeta ? (
            <span className="tcld-disasm-group-summary">
              <span className="tcld-disasm-group-dot" style={{ background: rowGroupColor }} aria-hidden />
              <span className="tcld-disasm-group-label">{groupMeta.label}</span>
              <span className="tcld-disasm-group-source">{groupMeta.sourceLabel}</span>
            </span>
          ) : (
            inst.group !== null ? commandGroupLabel(inst.group) : "·"
          )}
        </span>
      )}

      {/* Address gutter */}
      {columns.address && (
        <span
          className="w-[72px] text-xs text-tcld-fg-dim text-right pr-2 flex-shrink-0 tabular-nums cursor-copy hover:text-tcld-accent"
          data-token-role="address"
          data-disasm-column="address"
          onClick={handleCopyPc}
          title={`Click to copy ${pcHex}`}
        >
          {pcHex}
        </span>
      )}

      {/* Opcode */}
      {columns.opcode && (
        <span
          className="w-30 flex-shrink-0 font-semibold"
          data-token-role="opcode"
          data-disasm-column="opcode"
          style={{ color: opcodeColor }}
        >
          {inst.op}
        </span>
      )}

      {/* Operands */}
      {columns.args && (
        <span className="w-[52px] flex-shrink-0 tabular-nums text-right pr-2" style={{ color: T.number }} data-token-role="operand" data-disasm-column="operand">
          {tokenRoles.find((token) => token.role === "operand")?.text ?? inst.args}
        </span>
      )}

      {columns.size && (
        <span className="w-8 flex-shrink-0 tabular-nums text-right pr-1 text-tcld-fg-dim">
          {inst.size}
        </span>
      )}

      {/* Comment + jump xref */}
      {columns.comment ? (
        <span className="flex-1 flex items-center overflow-hidden whitespace-nowrap gap-1" data-disasm-column="hint">
          {isGroupHeader && groupMeta && (
            <span className="tcld-disasm-group-detail" title={groupMeta.detail}>
              {groupMeta.detail}
            </span>
          )}
          {isGroupHeader && groupMeta && (
            <span className="tcld-disasm-group-range">
              {groupMeta.count} inst · {pcLabel(groupMeta.pcStart)}-{pcLabel(groupMeta.pcEnd)}
            </span>
          )}
          {folded && (
            <span className="tcld-disasm-folded-count" data-folded-hidden-count={hiddenInstructionCount}>
              {hiddenInstructionCount} hidden
            </span>
          )}
          {inst.isJump && inst.jumpTarget !== null && (
            <span
              className="cursor-pointer hover:underline flex-shrink-0"
              data-token-role="jump-target"
              style={{ color: inst.op.includes("True") || inst.op.includes("False") ? T.warning : T.error }}
              onClick={(e) => {
                e.stopPropagation();
                onJumpClick(inst.jumpTarget!);
              }}
              title={`Jump target: 0x${inst.jumpTarget.toString(16).padStart(4, "0")}`}
            >
              {/* Address-relative jumps render as i{idx} when we can resolve
               * the target through the location index — easier to scan than
               * raw PC addresses, and matches the iN vocabulary used by the
               * cursor / breadcrumbs / panel badges. The hex PC is preserved
               * in the title attribute as a fallback for users who navigate
               * by address. Falls back to 0x{hex} if the index isn't ready
               * (cold load, missing source map). */}
              {(() => {
                const idx = pcToInstIdx?.get(inst.jumpTarget!);
                return idx != null
                  ? `→ i${idx}`
                  : `→ 0x${inst.jumpTarget!.toString(16).padStart(4, "0")}`;
              })()}
            </span>
          )}
          <span
            className="overflow-hidden text-ellipsis"
            data-token-role="comment"
            style={{ color: T.comment }}
            title={inst.comment || undefined}
          >
            {inst.comment && `; ${inst.comment}`}
          </span>
          {userComment && !isEditing && (
            <span
              className="flex-shrink-0 cursor-pointer hover:underline"
              style={{ color: T.warning }}
              onClick={(e) => {
                e.stopPropagation();
                onStartEdit();
              }}
              title="Click to edit comment"
            >
              ; {userComment}
            </span>
          )}
          {isEditing && (
            <CommentEditor
              initialValue={userComment ?? ""}
              onSave={onSaveComment}
              onCancel={onCancelEdit}
            />
          )}
        </span>
      ) : (
        <span className="flex-1" />
      )}

      {/* Selection indicator */}
      {selected && (
        <span className="text-tcld-accent text-xs flex-shrink-0 ml-1">
          <ChevronLeft size={12} aria-hidden />
        </span>
      )}
    </div>
  );
});

interface FlatInstructionRowProps {
  inst: Instruction;
  selected: boolean;
  isExecutionPointer: boolean;
  isBookmarked: boolean;
  userComment: string | undefined;
}

const FlatInstructionRow = React.memo(function FlatInstructionRow({
  inst,
  selected,
  isExecutionPointer,
  isBookmarked,
  userComment,
}: FlatInstructionRowProps) {
  const opcodeColor = inst.op.startsWith("invoke")
    ? "var(--info)"
    : opcodeCategoryColor(getOpcodeCategory(inst.op));
  const hint = userComment || inst.comment || (inst.isJump && inst.jumpTarget !== null
    ? `jump ${pcLabel(inst.jumpTarget)}`
    : inst.group != null
      ? commandGroupLabel(inst.group)
      : "");

  return (
    <div
      data-idx={inst.idx}
      data-instruction-row-mode="flat"
      data-selected={selected ? "true" : undefined}
      data-execution-pointer={isExecutionPointer ? "true" : undefined}
      className={clsx(
        "instruction-row grid h-[var(--disassembly-row-height)] cursor-pointer items-center gap-1 px-3 py-0 font-mono text-xs leading-[var(--disassembly-row-height)] transition-colors duration-75",
        "hover:bg-tcld-bg-highlight/50",
        selected ? "instruction-row-selected border-l-[3px] border-tcld-accent bg-tcld-bg-highlight/60" : "border-l-[3px] border-transparent",
        isExecutionPointer && "border-l-[3px] border-tcld-success",
      )}
      style={{ gridTemplateColumns: "40px 70px 92px 80px minmax(0, 1fr)" }}
      title={`${inst.op} ${inst.args}`.trim()}
    >
      <span className="tabular-nums text-label text-tcld-fg-dim" data-instructions-column="index">
        {inst.idx}
      </span>
      <span className="tabular-nums text-tcld-fg-dim" data-token-role="address" data-instructions-column="address">
        {pcLabel(inst.pc)}
      </span>
      <span className="truncate font-semibold" style={{ color: opcodeColor }} data-token-role="opcode" data-instructions-column="opcode">
        {inst.op}
      </span>
      <span className="truncate text-right tabular-nums text-tcld-accent" data-token-role="operand" data-instructions-column="operand">
        {inst.args}
      </span>
      <span className="flex min-w-0 items-center gap-1 overflow-hidden text-ellipsis whitespace-nowrap text-label text-tcld-fg-dim" data-token-role="comment" data-instructions-column="hint">
        {isBookmarked && <Bookmark size={10} fill="currentColor" strokeWidth={1.5} className="shrink-0 text-tcld-accent" aria-hidden />}
        <span className="truncate">{hint}</span>
      </span>
    </div>
  );
});

// ─── Bookmark Dropdown ──────────────────────────────────────────

function BookmarkDropdown({
  annotations,
  instructions,
  anchorRef,
  onSelect,
  onClose,
}: {
  annotations: Annotations;
  instructions: Instruction[];
  anchorRef: React.RefObject<HTMLElement | null>;
  onSelect: (idx: number) => void;
  onClose: () => void;
}) {
  const dropdownRef = useRef<HTMLDivElement>(null);
  const position = useAnchoredFloatingPosition(true, anchorRef, dropdownRef, { placement: "bottom-start" });

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [onClose]);

  // Portalize so the dropdown escapes the InstructionPanel's transform-induced
  // containing block (DndTile applies `transform: scale(...)` which makes
  // `position: fixed` anchor to the tile instead of the viewport, and traps
  // the dropdown's stacking context inside the tile so it can be obscured by
  // neighboring panels).
  const renderPortal = (content: React.ReactElement) =>
    typeof document !== "undefined" ? createPortal(content, document.body) : null;

  const bookmarkList = useMemo(() => {
    return [...annotations.bookmarks]
      .sort((a, b) => a - b)
      .map((idx) => {
        const inst = instructions[idx];
        if (!inst) return null;
        const comment = annotations.comments.get(idx) ?? inst.comment;
        return { idx, inst, comment };
      })
      .filter(Boolean) as { idx: number; inst: Instruction; comment: string }[];
  }, [annotations, instructions]);

  if (bookmarkList.length === 0) {
    return renderPortal(
      <div
        ref={dropdownRef}
        className="fixed z-[var(--z-context-menu)] bg-tcld-bg-1 border border-tcld-border rounded shadow-[var(--shadow-popover)] p-3"
        style={{
          left: position.left,
          top: position.top,
          maxHeight: position.maxHeight || undefined,
          minWidth: 200,
          visibility: position.ready ? "visible" : "hidden",
        }}
      >
        <span className="text-xs font-mono text-tcld-fg-dim">no bookmarks (press B to add)</span>
      </div>,
    );
  }

  return renderPortal(
    <div
      ref={dropdownRef}
      className="fixed z-[var(--z-context-menu)] bg-tcld-bg-1 border border-tcld-border rounded shadow-[var(--shadow-popover)] overflow-y-auto"
      style={{
        left: position.left,
        top: position.top,
        maxHeight: position.maxHeight || 300,
        minWidth: 280,
        visibility: position.ready ? "visible" : "hidden",
      }}
    >
      {bookmarkList.map(({ idx, inst, comment }) => (
        <div
          key={idx}
          onClick={() => {
            onSelect(idx);
            onClose();
          }}
          className="flex items-center gap-2 px-3 py-1.5 cursor-pointer hover:bg-tcld-bg-highlight font-mono text-xs"
        >
          <Bookmark size={11} fill="currentColor" strokeWidth={1.5} style={{ color: T.accent }} aria-hidden />
          <span className="tabular-nums" style={{ color: T.fgDim }}>
            0x{inst.pc.toString(16).padStart(4, "0")}
          </span>
          <span style={{ color: opcodeCategoryColor(getOpcodeCategory(inst.op)) }}>
            {inst.op}
          </span>
          <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap" style={{ color: T.comment }}>
            {comment ? comment.slice(0, 30) : ""}
          </span>
        </div>
      ))}
    </div>,
  );
}

// ─── Source-mode body ───────────────────────────────────────────
// Renders one row per source-map entry. The row text prefers the original
// source span when `originalStatus === "available"` (sliced from
// `originalSourceText`); otherwise it falls back to `entry.text`, which is
// the decompiled span. Indent comes from the command-path nesting depth, so
// nested commands read as nested in the same vocabulary the IR view uses.
//
// Click → select the first instruction whose pc lands inside the entry's
// bytecode range (cursor sync). Keyboard activation (Enter/Space) follows
// the same path; the rows are role="button" tabbable for screen readers.
//
// Not virtualized: source maps are one entry per command, not one per
// opcode, so the list size is O(file commands).
interface SourceModeBodyProps {
  sourceMap: ReadonlyArray<SourceMapEntry>;
  originalSourceText: string | null;
  instructions: ReadonlyArray<Instruction>;
  pcToInstIdx: ReadonlyMap<number, number>;
  groupDepthByGroup: ReadonlyMap<number, number>;
  selectedPc: number | null;
  executionPc: number | null;
  onSelect: (idx: number) => void;
}

function SourceModeBody({
  sourceMap,
  originalSourceText,
  instructions,
  pcToInstIdx,
  groupDepthByGroup,
  selectedPc,
  executionPc,
  onSelect,
}: SourceModeBodyProps) {
  if (sourceMap.length === 0) {
    return (
      <div
        className="flex flex-1 items-center justify-center px-4 py-6 font-mono text-xs text-tcld-fg-dim"
        data-source-mode-empty="true"
      >
        no source-map entries for this file
      </div>
    );
  }

  return (
    <div
      className="flex-1 overflow-y-auto overflow-x-hidden min-h-0"
      data-source-mode-rows="true"
    >
      <div className="py-1">
        {sourceMap.map((entry, sourceMapIndex) => {
          const depth = entry.commandGroup != null
            ? (groupDepthByGroup.get(entry.commandGroup) ?? Math.max(0, entry.commandPath.length - 1))
            : Math.max(0, entry.commandPath.length - 1);
          const [pcStart, pcEnd] = entry.bytecodeRange;
          const fromOriginal =
            originalSourceText != null &&
            entry.originalStatus === "available" &&
            entry.originalSpan != null;
          const sourceText = fromOriginal
            ? originalSourceText!.slice(entry.originalSpan![0], entry.originalSpan![1])
            : entry.text;
          const isSelected = selectedPc != null && selectedPc >= pcStart && selectedPc <= pcEnd;
          const isExecuting = executionPc != null && executionPc >= pcStart && executionPc <= pcEnd;

          const handleSelect = () => {
            const exact = pcToInstIdx.get(pcStart);
            if (exact != null) {
              onSelect(exact);
              return;
            }
            const found = instructions.find((inst) => inst.pc >= pcStart && inst.pc <= pcEnd);
            if (found) onSelect(found.idx);
          };

          return (
            <div
              key={`${sourceMapIndex}-${pcStart}`}
              role="button"
              tabIndex={0}
              data-source-row-index={sourceMapIndex}
              data-source-pc-range={`${pcStart}-${pcEnd}`}
              data-source-depth={depth || undefined}
              data-source-selected={isSelected ? "true" : undefined}
              data-source-executing={isExecuting ? "true" : undefined}
              data-source-origin={fromOriginal ? "original" : "decompile"}
              onClick={handleSelect}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  handleSelect();
                }
              }}
              className={clsx(
                "grid items-center cursor-pointer font-mono text-xs leading-5 border-l-[3px] transition-colors duration-75",
                "hover:bg-tcld-bg-highlight/50",
                isExecuting
                  ? "border-tcld-success bg-tcld-success/10"
                  : isSelected
                    ? "border-tcld-accent bg-tcld-accent/10"
                    : "border-transparent",
              )}
              style={{
                paddingLeft: 8 + depth * 12,
                paddingRight: 12,
                gridTemplateColumns: "72px minmax(0, 1fr) 64px",
                columnGap: 8,
                minHeight: "var(--code-row-height)",
              }}
              title={fromOriginal ? "original source span" : "decompiled span"}
            >
              <span className="tabular-nums text-tcld-fg-dim">
                i{pcStart.toString(16).padStart(4, "0")}
              </span>
              <span
                className="truncate"
                style={{
                  color: fromOriginal ? "var(--fg)" : "var(--fg-2)",
                  whiteSpace: "pre",
                }}
              >
                {sourceText}
              </span>
              <span className="text-right text-label text-tcld-fg-dim">
                {entry.sourceLines[0] === entry.sourceLines[1]
                  ? `L${entry.sourceLines[0] + 1}`
                  : `L${entry.sourceLines[0] + 1}-${entry.sourceLines[1] + 1}`}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ─── Main Component ─────────────────────────────────────────────

export function InstructionPanel({
  id,
  presentationMode = "ir",
  onPresentationModeChange,
  onSelect,
  focused,
  onClose,
  onFocus,
  annotations,
  showJumpArrows,
  onToggleBookmark,
  onSetComment,
  onToggleJumpArrows,
  debugActive,
  breakpointGroups,
  onToggleBreakpointAtGroup,
  onRunToCursorAtGroup,
}: InstructionPanelProps) {
  const { data } = useTcldStore();
  const { cursor, execution, index, pinA, pinB } = useLocationStore();
  const debug = useDebugStore();
  const instructions = useMemo(() => data?.instructions ?? [], [data?.instructions]);
  const selectedIdx = cursor.instIdx ?? 0;
  const executionIndex = execution?.instIdx ?? null;
  const executionGroup = execution?.group ?? null;
  const liveDisassembly = debug.disassemblyHeader
    ? `${debug.disassemblyHeader}\n\n${debug.disassemblyText ?? ""}`
    : (debug.disassemblyText ?? "");
  const scrollRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const { menuState, showMenu, closeMenu } = useContextMenu();
  const [editingIdx, setEditingIdx] = useState<number | null>(null);
  const [flashIdx, setFlashIdx] = useState<number | null>(null);
  const [showBookmarks, setShowBookmarks] = useState(false);
  const [showColumnMenu, setShowColumnMenu] = useState(false);
  const [showFilterBar, setShowFilterBar] = useState(false);
  const [filterMode, setFilterMode] = useState<InstructionFilterMode>("all");
  const [filterValue, setFilterValue] = useState("");
  const [panelMode, setPanelMode] = useState<InstructionPresentationMode>(presentationMode);
  const [foldedIds, setFoldedIds] = useState<Set<string>>(new Set());
  const [freezeAutoScroll, setFreezeAutoScroll] = useState(false);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [rowMetrics, setRowMetrics] = useState(() => getDisassemblyRowMetrics());
  const [columns, setColumns] = useState<Record<InstructionColumnId, boolean>>(
    () => {
      const settings = loadSettings();
      const defaults = withDisassemblyParityColumns({ ...DEFAULT_INSTRUCTION_COLUMNS });
      if (!settings.panelColumns?.instructions && settings.density !== "regular") {
        defaults.size = false;
      }
      return getPanelColumns("instructions", defaults);
    },
  );
  const columnMenuRef = useRef<HTMLDivElement>(null);
  const columnButtonRef = useRef<HTMLButtonElement>(null);
  const columnDropdownRef = useRef<HTMLDivElement>(null);
  const bookmarkButtonRef = useRef<HTMLButtonElement>(null);
  const filterInputRef = useRef<HTMLInputElement>(null);
  const columnMenuPosition = useAnchoredFloatingPosition(showColumnMenu, columnButtonRef, columnDropdownRef, { placement: "bottom-end" });
  const flatMode = panelMode === "instructions";
  const irMode = panelMode === "ir";
  const sourceMode = panelMode === "source";

  useEffect(() => {
    setPanelMode(presentationMode);
  }, [presentationMode]);

  const toggleColumn = useCallback((columnId: InstructionColumnId) => {
    setColumns((prev) => {
      const next = { ...prev, [columnId]: !prev[columnId] };
      savePanelColumns("instructions", next);
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
    const refreshMetrics = () => setRowMetrics(getDisassemblyRowMetrics());
    refreshMetrics();
    window.addEventListener("resize", refreshMetrics);
    const observer = new MutationObserver(refreshMetrics);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["style", "class"],
    });
    return () => {
      window.removeEventListener("resize", refreshMetrics);
      observer.disconnect();
    };
  }, []);

  useEffect(() => {
    if (!showFilterBar) return;
    const timer = window.setTimeout(() => filterInputRef.current?.focus(), 0);
    return () => window.clearTimeout(timer);
  }, [showFilterBar]);

  const filteredInstructionIndices = useMemo(() => {
    if (filterMode === "all") {
      return instructions.map((inst) => inst.idx);
    }
    if (filterMode === "jumps") {
      return instructions.filter((inst) => inst.isJump).map((inst) => inst.idx);
    }
    if (filterMode === "bookmarked") {
      return instructions.filter((inst) => annotations.bookmarks.has(inst.idx)).map((inst) => inst.idx);
    }
    if (filterMode === "group") {
      const group = Number.parseInt(filterValue.trim(), 10);
      if (!Number.isFinite(group)) return [];
      return instructions.filter((inst) => inst.group === group).map((inst) => inst.idx);
    }
    if (filterMode === "opcode-category") {
      const category = filterValue.trim().toLowerCase();
      if (!category) return [];
      return instructions
        .filter((inst) => getOpcodeCategory(inst.op).toLowerCase().includes(category))
        .map((inst) => inst.idx);
    }
    if (filterMode === "neighborhood") {
      const radius = Math.max(1, Number.parseInt(filterValue.trim(), 10) || 20);
      const anchor = executionIndex ?? selectedIdx;
      return instructions
        .filter((inst) => Math.abs(inst.idx - anchor) <= radius)
        .map((inst) => inst.idx);
    }
    return instructions.map((inst) => inst.idx);
  }, [
    annotations.bookmarks,
    executionIndex,
    filterMode,
    filterValue,
    instructions,
    selectedIdx,
  ]);

  const filterActive = filterMode !== "all";

  const foldModel = useMemo(
    () => data
      ? buildInstructionFoldModel(data, { foldedIds, selectedInstIdx: selectedIdx, filter: filterMode })
      : { foldRegions: [], visibleRows: [], selectedVisibleRowId: null },
    [data, foldedIds, filterMode, selectedIdx],
  );

  const panelRows = useMemo(
    () => buildInstructionPanelRows(foldModel, filteredInstructionIndices),
    [filteredInstructionIndices, foldModel],
  );

  const flatVisibleIndexByInstIdx = useMemo(
    () => new Map(filteredInstructionIndices.map((instIdx, visibleIndex) => [instIdx, visibleIndex])),
    [filteredInstructionIndices],
  );

  const foldRegionByStart = useMemo(
    () => new Map(foldModel.foldRegions.map((region) => [region.start, region])),
    [foldModel.foldRegions],
  );

  const groupMetaByGroup = useMemo(() => {
    const map = new Map<number, CommandGroupMeta>();
    for (const region of foldModel.foldRegions) {
      const group = data?.instructions[region.start]?.group ?? null;
      if (group == null) continue;
      const sourceStart = region.sourceLines?.[0];
      const sourceEnd = region.sourceLines?.[1];
      const sourceLabel = sourceStart == null
        ? "src -"
        : sourceEnd != null && sourceEnd > sourceStart + 1
          ? `L${sourceStart + 1}-${sourceEnd}`
          : `L${sourceStart + 1}`;
      map.set(group, {
        label: region.label,
        detail: compactSourceText(region.snippet),
        count: region.count ?? Math.max(1, region.end - region.start),
        pcStart: region.pcRange?.[0] ?? data?.instructions[region.start]?.pc ?? 0,
        pcEnd: region.pcRange?.[1] ?? data?.instructions[Math.max(region.start, region.end - 1)]?.pc ?? 0,
        sourceLabel,
        color: groupColor(group),
      });
    }
    return map;
  }, [data?.instructions, foldModel.foldRegions]);

  const visibleIndexByInstIdx = flatMode ? flatVisibleIndexByInstIdx : panelRows.visibleIndexByInstIdx;
  const renderedRowCount = flatMode ? filteredInstructionIndices.length : panelRows.rows.length;

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: renderedRowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowMetrics.rowHeight,
    overscan: rowMetrics.overscan,
  });

  const hoverInstIdx = useCallback((location: { instIdx?: number | null; scopeId?: string | null }) => {
    if (location.scopeId != null && index?.scopeId != null && location.scopeId !== index.scopeId) {
      return null;
    }
    return location.instIdx ?? null;
  }, [index]);

  useHoverHighlight({
    containerRef: listRef,
    rowSelector: ".instruction-row",
    keyAttribute: "data-idx",
    getKey: hoverInstIdx,
  });

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const update = () => setViewportHeight(el.clientHeight);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Scroll selected row into view (skip if execution pointer is active — it takes priority)
  useEffect(() => {
    if (freezeAutoScroll) return;
    if (executionIndex != null) return;
    const visibleIdx = visibleIndexByInstIdx.get(selectedIdx);
    if (visibleIdx != null) {
      virtualizer.scrollToIndex(visibleIdx, { align: "auto" });
    }
  }, [selectedIdx, virtualizer, executionIndex, freezeAutoScroll, visibleIndexByInstIdx]);

  // Auto-scroll to execution pointer when debugger stops (takes priority over selection)
  useEffect(() => {
    if (freezeAutoScroll) return;
    if (executionIndex != null) {
      const visibleIdx = visibleIndexByInstIdx.get(executionIndex);
      if (visibleIdx != null) {
        virtualizer.scrollToIndex(visibleIdx, { align: "center" });
      }
    }
  }, [executionIndex, virtualizer, freezeAutoScroll, visibleIndexByInstIdx]);

  // Track scroll position for SVG culling
  const handleScroll = useCallback((e: React.UIEvent) => {
    setScrollTop((e.target as HTMLElement).scrollTop);
  }, []);

  const handleRowMouseMove = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-idx]");
    if (!row) return;
    const idx = Number(row.dataset.idx);
    if (!Number.isFinite(idx)) return;
    const inst = instructions[idx];
    if (!inst) return;
    const block = index ? index.instIdxToBlock[idx] : -1;
    hoverBus.emit({
      scopeId: index?.scopeId ?? cursor.scopeId,
      instIdx: idx,
      pc: inst.pc,
      group: inst.group,
      blockId: block >= 0 ? block : null,
    });
  }, [cursor.scopeId, index, instructions]);

  const clearHover = useCallback(() => {
    hoverBus.emit(null);
  }, []);

  // PC → instIdx lookup for inline jump labels. Built once per instruction
  // set and threaded into InstructionRow so the row's React.memo equality
  // doesn't break when the cursor (or any other store slice) updates.
  const pcToInstIdx = useMemo(() => {
    const map = new Map<number, number>();
    for (const inst of instructions) map.set(inst.pc, inst.idx);
    return map;
  }, [instructions]);

  // Command-group nesting depth, derived from sourceMap entries' commandPath.
  // Root commands have commandPath.length === 1 → depth 0; each nested level
  // adds 1. Used by IR rows for visual indent and by Source-mode rows for the
  // same purpose, keeping nesting visualization consistent across views.
  const groupDepthByGroup = useMemo(() => {
    const map = new Map<number, number>();
    const sourceMap = data?.sourceMap;
    if (!sourceMap) return map;
    for (const entry of sourceMap) {
      if (entry.commandGroup != null) {
        map.set(entry.commandGroup, Math.max(0, entry.commandPath.length - 1));
      }
    }
    return map;
  }, [data?.sourceMap]);

  // Compute jump arrows
  const jumpArrows = useMemo(() => computeJumpArrows(instructions), [instructions]);
  const visibleJumpArrows = useMemo(() => jumpArrows
    .map((arrow) => {
      const fromIdx = visibleIndexByInstIdx.get(arrow.fromIdx);
      const toIdx = visibleIndexByInstIdx.get(arrow.toIdx);
      if (fromIdx == null || toIdx == null) return null;
      return { ...arrow, fromIdx, toIdx };
    })
    .filter((arrow): arrow is JumpArrow => arrow !== null), [jumpArrows, visibleIndexByInstIdx]);
  const maxLane = jumpArrows.reduce((m, a) => Math.max(m, a.lane), 0);
  const gutterPadding = showJumpArrows && !filterActive && irMode ? Math.max(24, (maxLane + 1) * 8 + 8) : 0;

  const foldAllInstructions = useCallback(() => {
    setFoldedIds(new Set(foldModel.foldRegions.map((region) => region.id)));
  }, [foldModel.foldRegions]);

  const unfoldAllInstructions = useCallback(() => {
    setFoldedIds(new Set());
  }, []);

  const toggleInstructionFold = useCallback((foldId: string) => {
    setFoldedIds((prev) => {
      const next = new Set(prev);
      if (next.has(foldId)) {
        next.delete(foldId);
      } else {
        next.add(foldId);
      }
      return next;
    });
  }, []);

  useEffect(() => {
    const unsubscribeFold = subscribePanelAction("instructions:fold-all", foldAllInstructions);
    const unsubscribeUnfold = subscribePanelAction("instructions:unfold-all", unfoldAllInstructions);
    return () => {
      unsubscribeFold();
      unsubscribeUnfold();
    };
  }, [foldAllInstructions, unfoldAllInstructions]);

  // Clear flash after animation completes
  useEffect(() => {
    if (flashIdx === null) return;
    const timer = setTimeout(() => setFlashIdx(null), 650);
    return () => clearTimeout(timer);
  }, [flashIdx]);

  // Jump target navigation
  const handleJumpClick = useCallback(
    (targetPc: number) => {
      const targetInst = instructions.find((i) => i.pc === targetPc);
      if (targetInst) {
        onSelect(targetInst.idx);
        setFlashIdx(targetInst.idx);
      }
    },
    [instructions, onSelect],
  );

  // Inline comment editing
  const handleStartEdit = useCallback((idx: number) => {
    setEditingIdx(idx);
  }, []);

  const handleSaveComment = useCallback(
    (text: string) => {
      if (editingIdx !== null) {
        onSetComment(editingIdx, text);
        setEditingIdx(null);
      }
    },
    [editingIdx, onSetComment],
  );

  const handleCancelEdit = useCallback(() => {
    setEditingIdx(null);
  }, []);

  const commands = useMemo(() => ([
    {
      id: "instructions:fold-all",
      title: "Disassembly: Fold All",
      category: "Disassembly",
      when: "focusedPanel:instructions,!overlayOpen",
      run: foldAllInstructions,
    },
    {
      id: "instructions:unfold-all",
      title: "Disassembly: Unfold All",
      category: "Disassembly",
      when: "focusedPanel:instructions,!overlayOpen",
      run: unfoldAllInstructions,
    },
    {
      id: `${id}:edit_comment`,
      title: "Edit Comment",
      category: "Instructions",
      shortcut: [";"],
      when: "focusedPanel:instructions,!overlayOpen",
      run: () => setEditingIdx(selectedIdx),
    },
    {
      id: `${id}:follow_jump`,
      title: "Follow Jump Target",
      category: "Instructions",
      shortcut: ["Enter"],
      when: "focusedPanel:instructions,!overlayOpen",
      run: () => {
        const inst = instructions[selectedIdx];
        if (inst?.isJump && inst.jumpTarget !== null) {
          handleJumpClick(inst.jumpTarget);
        }
      },
    },
    {
      id: `${id}:toggle_bookmark`,
      title: "Toggle Bookmark",
      category: "Instructions",
      shortcut: ["B"],
      when: "focusedPanel:instructions,!overlayOpen",
      run: () => onToggleBookmark(selectedIdx),
    },
    {
      id: `${id}:filter_toggle`,
      title: "Toggle Instruction Filter",
      category: "Instructions",
      shortcut: ["Mod+f", "/"],
      when: "focusedPanel:instructions,!overlayOpen",
      run: () => setShowFilterBar((prev) => !prev),
    },
    {
      id: `${id}:filter_clear`,
      title: "Clear Instruction Filter",
      category: "Instructions",
      shortcut: ["Escape"],
      when: "focusedPanel:instructions,!overlayOpen",
      run: () => {
        if (!showFilterBar && !filterActive) return;
        setFilterMode("all");
        setFilterValue("");
        setShowFilterBar(false);
      },
    },
  ]), [filterActive, foldAllInstructions, handleJumpClick, id, instructions, onToggleBookmark, selectedIdx, showFilterBar, unfoldAllInstructions]);

  useCommands(commands);

  // Event delegation for row clicks
  const handleRowClick = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>("[data-idx]");
      if (row) onSelect(Number(row.dataset.idx));
    },
    [onSelect],
  );

  // Context menu for rows
  const handleRowContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>("[data-idx]");
      if (!row) return;
      const idx = Number(row.dataset.idx);
      const inst = instructions[idx];
      if (!inst) return;

      const pcHex = `0x${inst.pc.toString(16).padStart(4, "0")}`;
      const isBookmarked = annotations.bookmarks.has(idx);
      const hasComment = annotations.comments.has(idx);

      const bpActive = inst.group != null && (breakpointGroups?.has(inst.group) ?? false);

      const items: ContextMenuItem[] = [
        {
          label: "Run To Cursor",
          icon: <Play size={12} aria-hidden />,
          shortcut: "Ctrl+F5",
          disabled: inst.group == null || !onRunToCursorAtGroup || !debug.scriptPath || !debug.debuggerAvailable,
          onClick: () => inst.group != null && onRunToCursorAtGroup?.(inst.group),
        },
        {
          label: bpActive ? "Remove Breakpoint" : "Set Breakpoint",
          icon: <Circle size={11} fill="currentColor" aria-hidden />,
          disabled: inst.group == null || !onToggleBreakpointAtGroup || !debug.debuggerAvailable,
          onClick: () => inst.group != null && onToggleBreakpointAtGroup?.(inst.group),
        },
        {
          label: "Copy Address",
          icon: <Copy size={12} aria-hidden />,
          onClick: () => navigator.clipboard.writeText(pcHex),
        },
        {
          label: "Follow Jump",
          icon: <ArrowRight size={12} aria-hidden />,
          disabled: !inst.isJump || inst.jumpTarget === null,
          onClick: () => inst.jumpTarget !== null && handleJumpClick(inst.jumpTarget),
        },
        {
          label: isBookmarked ? "Remove Bookmark" : "Toggle Bookmark",
          icon: <Bookmark size={12} aria-hidden />,
          shortcut: "B",
          onClick: () => onToggleBookmark(idx),
        },
        {
          label: hasComment ? "Edit Comment" : "Add Comment",
          icon: <MessageSquare size={12} aria-hidden />,
          shortcut: ";",
          onClick: () => setEditingIdx(idx),
        },
        { label: "", separator: true },
        {
          label: "Copy Instruction",
          icon: <Copy size={12} aria-hidden />,
          onClick: () => navigator.clipboard.writeText(`${pcHex} ${inst.op} ${inst.args}`.trim()),
        },
      ];
      showMenu(e, items);
    },
    [instructions, annotations, showMenu, handleJumpClick, onToggleBookmark, breakpointGroups, onToggleBreakpointAtGroup, onRunToCursorAtGroup, debug.scriptPath, debug.debuggerAvailable],
  );

  const selectedInstruction = instructions[selectedIdx] ?? null;
  const selectedGroup = selectedInstruction?.group ?? null;
  const groupLabel = commandGroupLabel(selectedGroup);
  const selectedPcLabel = selectedInstruction ? pcLabel(selectedInstruction.pc) : "0x----";
  const foldableGroupCount = foldModel.foldRegions.filter((region) => region.end - region.start > 1).length;
  const foldedGroupCount = foldModel.foldRegions.filter((region) => region.end - region.start > 1 && foldedIds.has(region.id)).length;
  // Static design audit markers for the two mapped presentations:
  // designSurface="view-disassembly"
  // designSurface="view-instructions"
  const panelTitle = flatMode ? "Instructions" : sourceMode ? "Source" : "Disassembly";
  const panelDesignSurface = flatMode ? "view-instructions" : sourceMode ? "view-source" : "view-disassembly";

  const bookmarkCount = annotations.bookmarks.size;

  // Subtitle = scope breadcrumb + position counter ("::clock::scan · i142 / 8421").
  // The scope half identifies the containing proc (when symbols are loaded
  // and the cursor lives inside one); the counter half is the cursor's
  // address. Together they replicate the deleted NavBreadcrumbs row inside
  // the panel that already owns the cursor — closer to the data, no global
  // chrome row, and only renders for the focused / inspected panel.
  const containingProc = useMemo(() => {
    const symbols = data?.symbols;
    if (!symbols || !selectedInstruction) return null;
    const pc = selectedInstruction.pc;
    for (const sym of symbols) {
      if (sym.kind !== "proc" || !sym.bytecodeRange) continue;
      const [lo, hi] = sym.bytecodeRange;
      if (pc >= lo && pc <= hi) return sym;
    }
    return null;
  }, [data?.symbols, selectedInstruction]);
  const positionSubtitle = (() => {
    if (instructions.length === 0) return "";
    const counter = `i${selectedIdx} / ${instructions.length}`;
    return containingProc ? `${containingProc.name} · ${counter}` : counter;
  })();

  if (instructions.length === 0) {
    return (
      <Panel title={panelTitle} badge="0" focused={focused} subtitle="" id={id} designSurface={panelDesignSurface} onClose={onClose} onFocus={onFocus}>
        {debugActive && liveDisassembly.trim().length > 0 ? (
          <pre className="m-0 px-4 py-3 font-mono text-disasm leading-5 text-tcld-fg whitespace-pre-wrap break-words">
            {liveDisassembly}
          </pre>
        ) : (
          <div className="px-6 py-6 font-mono text-xs text-center text-tcld-fg-dim">
            {debugActive
              ? "no structured instructions yet (continue/step to stream chunks)"
              : "no instructions loaded"}
          </div>
        )}
      </Panel>
    );
  }

  return (
    <Panel
      title={panelTitle}
      designSurface={panelDesignSurface}
      badge={String(instructions.length)}
      subtitle={positionSubtitle}
      focused={focused}
      id={id}
      onClose={onClose}
      onFocus={onFocus}
    >
      {/* Toolbar */}
      <div
        className="tcld-disasm-toolbar flex h-6 min-w-0 flex-nowrap items-center gap-1 overflow-x-auto px-2 py-0 border-b border-tcld-border-dim/60 bg-tcld-bg-1/35 flex-shrink-0"
        data-instruction-presentation={panelMode}
      >
        {/* Three-mode segment — instructions / ir / source. Active pill carries
            accent wash; clicking switches modes. The data-instructions-mode-switch
            attribute carries the *target* mode for the click — parity probes
            and tests use it to navigate. */}
        <div
          className="tcld-toolbar-group"
          role="tablist"
          aria-label="Disassembly view mode"
        >
          {(["instructions", "ir", "source"] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              role="tab"
              aria-selected={panelMode === mode}
              data-instructions-mode-switch={mode}
              data-mode-active={panelMode === mode ? "true" : undefined}
              onClick={() => {
                setPanelMode(mode);
                onPresentationModeChange?.(mode);
              }}
              className={clsx("tcld-toolbar-pill", panelMode === mode && "is-active")}
              title={
                mode === "instructions"
                  ? "Raw instruction stream"
                  : mode === "ir"
                    ? "Grouped IR view with command rails"
                    : "Source-mapped view (decompile / original spans)"
              }
            >
              {mode}
            </button>
          ))}
        </div>

        {flatMode && (
          <>
            <span className="tcld-toolbar-meta" data-instructions-flat-count={`${filteredInstructionIndices.length}/${instructions.length}`}>
              {filteredInstructionIndices.length}/{instructions.length}
            </span>
            <span className="sr-only" data-instructions-flat-label>flat instruction stream</span>
            <div className="flex-1" />
            <button
              type="button"
              disabled
              aria-disabled="true"
              data-instructions-byte-view="disabled"
              className="tcld-toolbar-pill is-disabled"
              title="Byte view requires backend raw byte ranges and bytes for each loaded instruction."
            >
              byte view
            </button>
          </>
        )}

        {sourceMode && (
          <>
            <span className="sr-only" data-source-mode-label>source-mapped view</span>
            <div className="flex-1" />
            <span className="tcld-toolbar-meta" data-source-span-count={`${data?.sourceMap?.length ?? 0}`}>
              {data?.sourceMap?.length ?? 0} spans
            </span>
          </>
        )}

        {irMode && (
          <>
            <span className="sr-only" data-disassembly-grouped-label>grouped disassembly</span>
            <span className="tcld-toolbar-divider" aria-hidden />

            {/* Group: visual aids (sync, jumps, bookmarks) */}
            <div className="tcld-toolbar-group" role="group" aria-label="View aids">
              <button
                onClick={() => setFreezeAutoScroll((prev) => !prev)}
                className={clsx(
                  "tcld-toolbar-icon",
                  !freezeAutoScroll && "is-active",
                )}
                title={freezeAutoScroll ? "Auto-scroll frozen (click to unfreeze)" : "Auto-scroll active (click to freeze)"}
                aria-pressed={!freezeAutoScroll}
              >
                {freezeAutoScroll ? <Unlink2 size={12} /> : <Link2 size={12} />}
              </button>
              <button
                onClick={onToggleJumpArrows}
                className={clsx(
                  "tcld-toolbar-icon",
                  showJumpArrows && "is-active",
                )}
                title={showJumpArrows ? "Hide jump arrows (a)" : "Show jump arrows (a)"}
                aria-pressed={showJumpArrows}
              >
                <GitBranch size={12} />
              </button>
              <div className="relative">
                <button
                  ref={bookmarkButtonRef}
                  onClick={() => setShowBookmarks(!showBookmarks)}
                  className={clsx(
                    "tcld-toolbar-icon",
                    bookmarkCount > 0 && "is-active",
                  )}
                  title={`Bookmarks (${bookmarkCount})`}
                  aria-pressed={bookmarkCount > 0}
                >
                  <Bookmark size={12} fill={bookmarkCount > 0 ? "currentColor" : "none"} strokeWidth={1.5} aria-hidden />
                  {bookmarkCount > 0 && (
                    <span className="tcld-toolbar-icon-badge">{bookmarkCount}</span>
                  )}
                </button>
                {showBookmarks && (
                  <BookmarkDropdown
                    annotations={annotations}
                    instructions={instructions}
                    anchorRef={bookmarkButtonRef}
                    onSelect={onSelect}
                    onClose={() => setShowBookmarks(false)}
                  />
                )}
              </div>
            </div>

            <span className="tcld-toolbar-divider" aria-hidden />

            {/* Group: configuration (columns, filter) */}
            <div className="tcld-toolbar-group" role="group" aria-label="Configuration">
              <div className="relative" ref={columnMenuRef}>
                <button
                  ref={columnButtonRef}
                  onClick={() => setShowColumnMenu((prev) => !prev)}
                  className="tcld-toolbar-icon"
                  title="Toggle visible columns"
                  aria-label="Toggle columns"
                >
                  <Columns size={11} aria-hidden />
                </button>
                {showColumnMenu && typeof document !== "undefined" && createPortal(
                  <div
                    ref={columnDropdownRef}
                    className="fixed z-[var(--z-context-menu)] bg-tcld-bg-1 border border-tcld-border rounded shadow-[var(--shadow-popover)] p-2 min-w-44"
                    style={{
                      left: columnMenuPosition.left,
                      top: columnMenuPosition.top,
                      maxHeight: columnMenuPosition.maxHeight || undefined,
                      overflowY: "auto",
                      visibility: columnMenuPosition.ready ? "visible" : "hidden",
                    }}
                  >
                    <ColumnToggleItem
                      label="Breakpoint Gutter"
                      enabled={columns.breakpoint}
                      onToggle={() => toggleColumn("breakpoint")}
                    />
                    <ColumnToggleItem
                      label="Marker Gutter"
                      enabled={columns.marker}
                      onToggle={() => toggleColumn("marker")}
                    />
                    <ColumnToggleItem
                      label="Group Label"
                      enabled={columns.group}
                      onToggle={() => toggleColumn("group")}
                    />
                    <ColumnToggleItem
                      label="Address"
                      enabled={columns.address}
                      onToggle={() => toggleColumn("address")}
                    />
                    <ColumnToggleItem
                      label="Opcode"
                      enabled={columns.opcode}
                      onToggle={() => toggleColumn("opcode")}
                    />
                    <ColumnToggleItem
                      label="Operands"
                      enabled={columns.args}
                      onToggle={() => toggleColumn("args")}
                    />
                    <ColumnToggleItem
                      label="Size"
                      enabled={columns.size}
                      onToggle={() => toggleColumn("size")}
                    />
                    <ColumnToggleItem
                      label="Comment / Jump"
                      enabled={columns.comment}
                      onToggle={() => toggleColumn("comment")}
                    />
                  </div>,
                  document.body,
                )}
              </div>
              <button
                onClick={() => setShowFilterBar((prev) => !prev)}
                className={clsx(
                  "tcld-toolbar-icon",
                  (showFilterBar || filterActive) && "is-active",
                )}
                title="Filter instructions (⌘F /)"
                aria-label="Filter instructions"
                aria-pressed={showFilterBar || filterActive}
              >
                <Filter size={11} aria-hidden />
              </button>
            </div>

            <span className="tcld-toolbar-divider" aria-hidden />

            {/* Group: folding (single toggle — fold-all when expanded, unfold-all when collapsed) */}
            <div className="tcld-toolbar-group" role="group" aria-label="Folding">
              {foldableGroupCount > 0 && foldedGroupCount >= foldableGroupCount ? (
                <button
                  onClick={() => commandRegistry.execute("instructions:unfold-all")}
                  data-disassembly-toolbar-action="unfold-all"
                  data-fold-state="all-folded"
                  className="tcld-toolbar-icon is-active"
                  title="Unfold all commands"
                  aria-label="unfold all commands"
                  aria-pressed="true"
                >
                  <ChevronDown size={11} aria-hidden />
                  <span className="sr-only">unfold all</span>
                </button>
              ) : (
                <button
                  onClick={() => commandRegistry.execute("instructions:fold-all")}
                  data-disassembly-toolbar-action="fold-all"
                  data-fold-state={foldedGroupCount > 0 ? "partial" : "all-unfolded"}
                  className={clsx("tcld-toolbar-icon", foldedGroupCount > 0 && "is-active")}
                  title="Fold all commands"
                  aria-label="fold all commands"
                  aria-pressed={false}
                >
                  <ChevronRight size={11} aria-hidden />
                  <span className="sr-only">fold all</span>
                </button>
              )}
              <span className="sr-only" data-disassembly-folded-count={`${foldedGroupCount}/${foldableGroupCount}`}>
                {foldedGroupCount}/{foldableGroupCount} folded
              </span>
            </div>

            <div className="flex-1" />

            <span className="sr-only" data-disassembly-selected-group={groupLabel} data-disassembly-selected-inst={selectedPcLabel}>
              sel: {groupLabel}
            </span>

            {filterActive && (
              <span className="tcld-toolbar-meta">
                {filteredInstructionIndices.length}/{instructions.length} filt
              </span>
            )}

            {!filterActive && jumpArrows.length > 0 && (
              <span className="tcld-toolbar-meta hidden md:inline">
                {jumpArrows.length} jmp
              </span>
            )}
          </>
        )}
      </div>

      {showFilterBar && irMode && (
        <div className="flex items-center gap-2 px-2 py-1 border-b border-tcld-border-dim bg-tcld-bg-1/40">
          <span className="text-label font-mono text-tcld-fg-dim">mode</span>
          <select
            value={filterMode}
            onChange={(e) => {
              setFilterMode(e.target.value as InstructionFilterMode);
              if (e.target.value === "all" || e.target.value === "jumps" || e.target.value === "bookmarked") {
                setFilterValue("");
              }
            }}
            className="bg-tcld-bg-2 border border-tcld-border-dim rounded px-1.5 py-0.5 font-mono text-xs text-tcld-fg"
          >
            <option value="all">all</option>
            <option value="jumps">jumps</option>
            <option value="bookmarked">bookmarked</option>
            <option value="group">group</option>
            <option value="opcode-category">opcode-category</option>
            <option value="neighborhood">neighborhood</option>
          </select>
          {(filterMode === "group" || filterMode === "opcode-category" || filterMode === "neighborhood") && (
            <input
              ref={filterInputRef}
              value={filterValue}
              onChange={(e) => setFilterValue(e.target.value)}
              placeholder={
                filterMode === "group"
                  ? "group #"
                  : filterMode === "opcode-category"
                    ? "jump/load/store…"
                    : "radius"
              }
              className="min-w-28 bg-tcld-bg-2 border border-tcld-border-dim rounded px-2 py-0.5 font-mono text-xs text-tcld-fg placeholder-tcld-fg-dim/60 outline-none"
            />
          )}
          <button
            type="button"
            onClick={() => {
              setFilterMode("all");
              setFilterValue("");
            }}
            className="ml-auto px-2 py-0.5 rounded border border-tcld-border-dim text-label font-mono text-tcld-fg-dim hover:text-tcld-fg hover:bg-tcld-bg-highlight"
          >
            clear
          </button>
        </div>
      )}

      {flatMode && (
        <div
          className="grid flex-shrink-0 gap-1 border-b border-tcld-border-dim px-3 py-1 font-mono text-label uppercase text-tcld-fg-dim"
          style={{ gridTemplateColumns: "40px 70px 92px 80px minmax(0, 1fr)" }}
          data-instructions-flat-header
        >
          <span data-instructions-column-header="index">#</span>
          <span data-instructions-column-header="address">addr</span>
          <span data-instructions-column-header="opcode">opcode</span>
          <span className="text-right" data-instructions-column-header="operand">operand</span>
          <span data-instructions-column-header="hint">hint/comment</span>
        </div>
      )}

      {sourceMode && (
        <SourceModeBody
          sourceMap={data?.sourceMap ?? []}
          originalSourceText={data?.originalSourceText ?? null}
          instructions={instructions}
          pcToInstIdx={pcToInstIdx}
          groupDepthByGroup={groupDepthByGroup}
          selectedPc={selectedInstruction?.pc ?? null}
          executionPc={executionIndex != null ? instructions[executionIndex]?.pc ?? null : null}
          onSelect={onSelect}
        />
      )}

      {/* Instruction list with jump arrow overlay (instructions / ir modes) */}
      {!sourceMode && (
      <div
        ref={scrollRef}
        className="relative flex-1 overflow-y-auto overflow-x-hidden min-h-0"
        onScroll={handleScroll}
        onMouseLeave={clearHover}
      >
        <div
          ref={listRef}
          onClick={handleRowClick}
          onMouseMove={handleRowMouseMove}
          onContextMenu={handleRowContextMenu}
          style={{
            height: virtualizer.getTotalSize(),
            position: "relative",
            width: "100%",
          }}
        >
          {/* Virtualized instruction rows */}
          {renderedRowCount === 0 && (
            <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
              <span className="font-mono text-xs text-tcld-fg-dim">no instructions match current filter</span>
            </div>
          )}
          {virtualizer.getVirtualItems().map((virtualRow) => {
            if (flatMode) {
              const instIdx = filteredInstructionIndices[virtualRow.index];
              const inst = instIdx == null ? null : instructions[instIdx];
              if (!inst) return null;
              return (
                <div
                  key={inst.idx}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    height: virtualRow.size,
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                >
                  <FlatInstructionRow
                    inst={inst}
                    selected={inst.idx === selectedIdx}
                    isExecutionPointer={inst.idx === executionIndex}
                    isBookmarked={annotations.bookmarks.has(inst.idx)}
                    userComment={annotations.comments.get(inst.idx)}
                  />
                </div>
              );
            }
            const row = panelRows.rows[virtualRow.index];
            if (!row) return null;
            const inst = instructions[row.instIdx];
            if (!inst) return null;
            const foldRegion = foldRegionByStart.get(inst.idx);
            const foldable = Boolean(foldRegion && foldRegion.end - foldRegion.start > 1);
            const folded = row.hiddenInstructionCount > 0;
            const rowGroupMeta = inst.group != null ? (groupMetaByGroup.get(inst.group) ?? null) : null;
            return (
              <div
                key={inst.idx}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  height: virtualRow.size,
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                <InstructionRow
                  inst={inst}
                  selected={inst.idx === selectedIdx}
                  isBookmarked={annotations.bookmarks.has(inst.idx)}
                  isExecutionPointer={inst.idx === executionIndex}
                  isExecutionGroup={executionGroup != null && inst.group === executionGroup}
                  isPinA={pinA?.scopeId === cursor.scopeId && pinA?.instIdx === inst.idx}
                  isPinB={pinB?.scopeId === cursor.scopeId && pinB?.instIdx === inst.idx}
                  hasBreakpoint={inst.groupStart && inst.group != null && (breakpointGroups?.has(inst.group) ?? false)}
                  foldable={foldable}
                  folded={folded}
                  hiddenInstructionCount={folded ? row.hiddenInstructionCount : Math.max(0, (foldRegion?.end ?? inst.idx + 1) - inst.idx)}
                  groupMeta={rowGroupMeta}
                  selectedGroup={selectedGroup != null && inst.group === selectedGroup}
                  onToggleFold={() => {
                    if (foldRegion) toggleInstructionFold(foldRegion.id);
                  }}
                  userComment={annotations.comments.get(inst.idx)}
                  isEditing={editingIdx === inst.idx}
                  onJumpClick={handleJumpClick}
                  onStartEdit={() => handleStartEdit(inst.idx)}
                  onSaveComment={handleSaveComment}
                  onCancelEdit={handleCancelEdit}
                  flashIdx={flashIdx}
                  gutterPadding={gutterPadding}
                  columns={columns}
                  pcToInstIdx={pcToInstIdx}
                  depth={inst.group != null ? (groupDepthByGroup.get(inst.group) ?? 0) : 0}
                />
              </div>
            );
          })}

          {/* Jump arrows SVG overlay — rendered AFTER rows so it paints on top */}
          {showJumpArrows && !filterActive && irMode && visibleJumpArrows.length > 0 && (
            <JumpGutterSVG
              arrows={visibleJumpArrows}
              scrollTop={scrollTop}
              viewportHeight={viewportHeight}
              rowHeight={rowMetrics.rowHeight}
            />
          )}
        </div>
      </div>
      )}

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
