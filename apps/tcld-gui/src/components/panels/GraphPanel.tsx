"use client";

import React, { useMemo, useRef, useState, useCallback, useEffect } from "react";
import dagre from "@dagrejs/dagre";
import { useTcldData } from "@/lib/store";
import { T } from "@/lib/theme";
import { Panel } from "./Panel";
import clsx from "clsx";
import { ArrowRight, ChevronDown, ChevronRight, Crosshair, Maximize, Play, Workflow, ZoomIn, ZoomOut } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import type { BasicBlock, Instruction } from "@/types";
import { useContextMenu } from "@/hooks/use-context-menu";
import { ContextMenu, type ContextMenuItem } from "@/components/overlays/ContextMenu";
import { useCommands } from "@/hooks/use-commands";
import { commandRegistry } from "@/lib/command-registry";
import { useLocationStore } from "@/lib/location-store";
import { useShallow } from "zustand/react/shallow";
import { hoverBus } from "@/lib/hover-bus";
import { useHoverHighlight } from "@/hooks/use-hover-highlight";
import { useDebugStore } from "@/lib/debug-store";
import {
  buildGraphViewModel,
  classifyGraphEdge,
  graphEdgeRenderAttrs,
  graphNodeRenderAttrs,
  deriveGraphBlockRole,
  type GraphEdgeType,
  type GraphMode,
  type GraphBlockRole,
} from "@/lib/graph-model";

// ─── Constants ───────────────────────────────────────────────────

type DetailMode = "compact" | "medium" | "full";

const NODE_W = 232;
const NODE_PAD_X = 12;
const NODE_PAD_TOP = 6;
const LINE_H = 17;
const HEADER_H = 22;
const SOURCE_LINE_H = 13;
const SOURCE_GAP = 6;
const FOOTER_H = 16;
const NODE_SEP = 30;
const RANK_SEP = 50;
const GRAPH_PAD = 40;
const ZOOM_MIN = 0.25;
// Opening zoom never drops below this: a whole-graph fit of a large proc is
// unreadable, so the first view favors legible blocks near the entry.
const READABLE_ZOOM = 0.8;
const ZOOM_MAX = 2.5;
const ZOOM_STEP = 0.1;

// ─── Layout Helpers ──────────────────────────────────────────────

function visibleSourceLineCount(mode: DetailMode, sourceLines: string[]): number {
  if (mode === "compact") return 0;
  return Math.min(sourceLines.length, mode === "medium" ? 1 : 3);
}

function visibleInstructionLineCount(block: BasicBlock, mode: DetailMode): number {
  if (mode === "compact") return 1; // terminal opcode
  if (mode === "medium") return Math.min(block.instructions.length, 4); // first 2 + ... + last 1
  return block.instructions.length;
}

// Vertical stack inside a node: header, instruction rows, optional source
// preview, then a footer row for the edge-count marker. Rendering reads the
// same offsets so rows never overlap.
function sourceTop(instructionLines: number): number {
  return NODE_PAD_TOP + HEADER_H + instructionLines * LINE_H + SOURCE_GAP;
}

function nodeHeight(block: BasicBlock, mode: DetailMode, sourceLines: string[] = []): number {
  const instructionLines = visibleInstructionLineCount(block, mode);
  const sourceCount = visibleSourceLineCount(mode, sourceLines);
  const body = sourceCount > 0
    ? sourceTop(instructionLines) + sourceCount * SOURCE_LINE_H
    : NODE_PAD_TOP + HEADER_H + instructionLines * LINE_H;
  return body + FOOTER_H;
}

interface LayoutNode {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
  block: BasicBlock;
}

interface LayoutEdge {
  source: number;
  target: number;
  points: { x: number; y: number }[];
  edgeType: GraphEdgeType;
}

interface GraphLayout {
  nodes: LayoutNode[];
  edges: LayoutEdge[];
  width: number;
  height: number;
}

function computeLayout(
  blocks: BasicBlock[],
  mode: DetailMode,
  expandedBlocks: Set<number>,
  instructions: Instruction[],
  sourceSummaries: Map<number, string[]>,
): GraphLayout {
  const g = new dagre.graphlib.Graph();
  g.setGraph({
    rankdir: "TB",
    nodesep: NODE_SEP,
    ranksep: RANK_SEP,
    marginx: GRAPH_PAD,
    marginy: GRAPH_PAD,
  });
  g.setDefaultEdgeLabel(() => ({}));

  for (const block of blocks) {
    const effectiveMode = expandedBlocks.has(block.id)
      ? (mode === "compact" ? "full" : mode === "medium" ? "full" : "compact")
      : mode;
    const h = nodeHeight(block, effectiveMode, sourceSummaries.get(block.id));
    g.setNode(String(block.id), { width: NODE_W, height: h });
  }

  for (const block of blocks) {
    for (const target of block.edges) {
      g.setEdge(String(block.id), String(target));
    }
  }

  dagre.layout(g);

  const graphInfo = g.graph();
  const nodes: LayoutNode[] = blocks.map((block) => {
    const n = g.node(String(block.id));
    return {
      id: block.id,
      x: n.x - n.width / 2,
      y: n.y - n.height / 2,
      w: n.width,
      h: n.height,
      block,
    };
  });

  const edges: LayoutEdge[] = [];
  for (const block of blocks) {
    for (const target of block.edges) {
      const e = g.edge(String(block.id), String(target));
      if (e) {
        edges.push({
          source: block.id,
          target,
          points: e.points,
          edgeType: classifyGraphEdge(block, target, instructions, blocks),
        });
      }
    }
  }

  return {
    nodes,
    edges,
    width: (graphInfo.width ?? 300) + GRAPH_PAD,
    height: (graphInfo.height ?? 200) + GRAPH_PAD,
  };
}

// ─── Predecessor Map ─────────────────────────────────────────────

function buildPredecessorMap(blocks: BasicBlock[]): Map<number, number[]> {
  const map = new Map<number, number[]>();
  for (const block of blocks) {
    for (const target of block.edges) {
      const list = map.get(target) ?? [];
      list.push(block.id);
      map.set(target, list);
    }
  }
  return map;
}

// ─── Edge Path Builder ───────────────────────────────────────────

function edgePath(points: { x: number; y: number }[]): string {
  if (points.length < 2) return "";
  const [start, ...rest] = points;
  let d = `M ${start.x} ${start.y}`;
  if (rest.length === 1) {
    d += ` L ${rest[0].x} ${rest[0].y}`;
  } else if (rest.length === 2) {
    d += ` Q ${rest[0].x} ${rest[0].y} ${rest[1].x} ${rest[1].y}`;
  } else {
    // Use cubic bezier through control points
    for (let i = 0; i < rest.length - 1; i += 2) {
      if (i + 1 < rest.length) {
        const cp = rest[i];
        const end = rest[i + 1] ?? rest[i];
        d += ` Q ${cp.x} ${cp.y} ${end.x} ${end.y}`;
      }
    }
    // If odd number of remaining points, line to last
    if (rest.length % 2 === 1) {
      const last = rest[rest.length - 1];
      d += ` L ${last.x} ${last.y}`;
    }
  }
  return d;
}

function parseBlockGroup(group: string): number | null {
  const match = /^c(\d+)$/.exec(group);
  return match ? Number(match[1]) : null;
}

const EDGE_COLORS: Record<LayoutEdge["edgeType"], string> = {
  fallthrough: T.fgDim,
  unconditional: T.accent,
  "conditional-true": T.success,
  "conditional-false": T.error,
};

const EDGE_OPACITY: Record<LayoutEdge["edgeType"], number> = {
  fallthrough: 0.5,
  unconditional: 0.8,
  "conditional-true": 0.85,
  "conditional-false": 0.85,
};

const EDGE_LEGEND: Array<{ role: GraphEdgeType; label: string }> = [
  { role: "fallthrough", label: "fallthrough" },
  { role: "conditional-true", label: "true" },
  { role: "conditional-false", label: "false" },
  { role: "unconditional", label: "jump" },
];

const BLOCK_ROLE_COLORS: Record<GraphBlockRole, string> = {
  entry: T.group,
  branch: T.warning,
  loop: T.accent,
  exit: T.error,
  linear: T.borderDim,
};

const BLOCK_ROLE_LABELS: Record<GraphBlockRole, string> = {
  entry: "entry",
  branch: "branch",
  loop: "loop",
  exit: "exit",
  linear: "linear",
};

// ─── Block Node SVG ──────────────────────────────────────────────

interface BlockNodeProps {
  node: LayoutNode;
  mode: DetailMode;
  blockRole: GraphBlockRole;
  sourceLines: string[];
  isSelected: boolean;
  isPathHighlighted: boolean;
  isDimmed: boolean;
  isExecution: boolean;
  expandedBlocks: Set<number>;
  onBlockClick: (blockId: number) => void;
  onBlockDoubleClick: (blockId: number) => void;
  onBlockHover: (blockId: number) => void;
  onBlockHoverEnd: () => void;
  onBlockContextMenu?: (e: React.MouseEvent, blockId: number) => void;
}

const BlockNode = React.memo(function BlockNode({
  node,
  mode,
  blockRole,
  sourceLines,
  isSelected,
  isPathHighlighted,
  isDimmed,
  isExecution,
  expandedBlocks,
  onBlockClick,
  onBlockDoubleClick,
  onBlockHover,
  onBlockHoverEnd,
  onBlockContextMenu,
}: BlockNodeProps) {
  const { block, x, y, w, h } = node;
  const effectiveMode = expandedBlocks.has(block.id)
    ? (mode === "compact" ? "full" : mode === "medium" ? "full" : "compact")
    : mode;

  const lastInst = block.instructions[block.instructions.length - 1];
  const terminalOp = lastInst?.name ?? "—";
  const roleColor = BLOCK_ROLE_COLORS[blockRole];

  // Determine visible instructions based on mode
  let visibleInstructions: { pc: number; name: string; operands: number[] }[];
  if (effectiveMode === "compact") {
    visibleInstructions = [];
  } else if (effectiveMode === "medium") {
    const insts = block.instructions;
    if (insts.length <= 4) {
      visibleInstructions = insts;
    } else {
      visibleInstructions = [...insts.slice(0, 2), { pc: -1, name: "···", operands: [] }, insts[insts.length - 1]];
    }
  } else {
    visibleInstructions = block.instructions;
  }

  const borderColor = isExecution
    ? T.success
    : isSelected
      ? roleColor
      : isPathHighlighted
        ? T.group
        : T.border;

  const opacity = isDimmed && !isExecution ? 0.6 : 1;

  return (
    <g
      className={clsx(
        "graph-node graph-node-basic-block",
        `graph-block-role-${blockRole}`,
        isSelected && "graph-role-selected",
        isExecution && "graph-role-execution",
        isPathHighlighted && !isSelected && "graph-role-successor graph-role-predecessor",
      )}
      data-block={block.id}
      data-block-id={block.id}
      data-flow-block-role={blockRole}
      data-graph-mode="basic-block"
      data-node-kind="basic-block"
      data-flow-role="flow-node"
      data-visual-selected={isSelected ? "true" : undefined}
      data-visual-execution={isExecution ? "true" : undefined}
      data-visual-successor={isPathHighlighted && !isSelected ? "true" : undefined}
      transform={`translate(${x}, ${y})`}
      style={{ cursor: "pointer", opacity, transition: "opacity 150ms" }}
      onClick={(e) => { e.stopPropagation(); onBlockClick(block.id); }}
      onDoubleClick={(e) => { e.stopPropagation(); onBlockDoubleClick(block.id); }}
      onMouseEnter={() => onBlockHover(block.id)}
      onMouseLeave={onBlockHoverEnd}
      onContextMenu={(e) => { e.stopPropagation(); e.preventDefault(); onBlockContextMenu?.(e as unknown as React.MouseEvent, block.id); }}
    >
      {/* Background */}
      <rect
        width={w}
        height={h}
        rx={6}
        fill={T.bg3}
        stroke={borderColor}
        strokeWidth={isSelected ? 2 : 1}
        style={{ transition: "stroke 150ms, stroke-width 150ms" }}
      />
      {/* Role rail, inset so it stays inside the rounded corners. */}
      <rect
        x={3}
        y={6}
        width={3}
        height={Math.max(0, h - 12)}
        rx={1.5}
        fill={roleColor}
        opacity={isSelected ? 0.95 : 0.72}
      />
      {/* Execution glow (pulsing green) */}
      {isExecution && (
        <rect
          width={w}
          height={h}
          rx={6}
          fill="none"
          stroke={T.success}
          strokeWidth={1}
          strokeOpacity={0.5}
          filter="url(#glow)"
        >
          <animate attributeName="stroke-opacity" values="0.5;0.2;0.5" dur="2s" repeatCount="indefinite" />
        </rect>
      )}
      {/* Selected glow */}
      {isSelected && !isExecution && (
        <rect
          width={w}
          height={h}
          rx={6}
          fill="none"
          stroke={T.accent}
          strokeWidth={0.5}
          strokeOpacity={0.3}
          filter="url(#glow)"
        />
      )}

      {/* Header line */}
      <text
        x={NODE_PAD_X}
        y={NODE_PAD_TOP + HEADER_H - 4}
        fontFamily="var(--font-mono)"
        fontSize={12}
        fontWeight={600}
        fill={isSelected ? T.accent : T.fg}
      >
        <tspan>b{String(block.id).padStart(3, "0")}</tspan>
        <tspan fill={T.fgDim} fontWeight={400}>
          {" "}({block.instructions.length})
        </tspan>
      </text>

      {/* Group label on right */}
      {block.group && (
        <text
          x={w - NODE_PAD_X}
          y={NODE_PAD_TOP + HEADER_H - 4}
          fontFamily="var(--font-ui)"
          fontSize={10.5}
          fontWeight={500}
          fill={roleColor}
          textAnchor="end"
        >
          {BLOCK_ROLE_LABELS[blockRole]} · {block.group}
        </text>
      )}

      {/* Compact: just terminal opcode */}
      {effectiveMode === "compact" && (
        <text
          x={NODE_PAD_X}
          y={NODE_PAD_TOP + HEADER_H + LINE_H - 4}
          fontFamily="var(--font-mono)"
          fontSize={11}
          fill={T.keyword}
        >
          {terminalOp}
        </text>
      )}

      {/* Medium/Full: instruction list */}
      {effectiveMode !== "compact" && visibleInstructions.map((inst, i) => (
        <text
          key={i}
          x={NODE_PAD_X}
          y={NODE_PAD_TOP + HEADER_H + (i + 1) * LINE_H - 4}
          fontFamily="var(--font-mono)"
          fontSize={11}
          fill={inst.pc === -1 ? T.fgDim : T.keyword}
        >
          {inst.pc === -1 ? inst.name : (
            <>
              <tspan fill={T.fgDim} fontSize={10}>
                {inst.pc.toString(16).padStart(4, "0")}
              </tspan>
              <tspan fill={T.keyword}>{" "}{inst.name}</tspan>
            </>
          )}
        </text>
      ))}

      {sourceLines.slice(0, visibleSourceLineCount(effectiveMode, sourceLines)).map((line, i) => (
        <text
          key={`source-${i}`}
          x={NODE_PAD_X}
          y={sourceTop(visibleInstructionLineCount(block, effectiveMode)) + (i + 1) * SOURCE_LINE_H - 3}
          fontFamily="var(--font-mono)"
          fontSize={10}
          fill={T.fgDim}
        >
          {line.slice(0, 32)}
        </text>
      ))}

      {/* Edge count indicator (bottom right) */}
      {block.edges.length > 0 && (
        <text
          x={w - NODE_PAD_X}
          y={h - 5}
          fontFamily="var(--font-mono)"
          fontSize={9.5}
          fill={T.fgDim}
          textAnchor="end"
          opacity={0.6}
        >
          →{block.edges.length}
        </text>
      )}
      {block.edges.length === 0 && (
        <text
          x={w - NODE_PAD_X}
          y={h - 5}
          fontFamily="var(--font-ui)"
          fontSize={9.5}
          fill={T.error}
          textAnchor="end"
          opacity={0.75}
        >
          exit
        </text>
      )}
    </g>
  );
});

// ─── Edge SVG ────────────────────────────────────────────────────

interface EdgePathProps {
  edge: LayoutEdge;
  isHighlighted: boolean;
  isDimmed: boolean;
}

const EdgePath = React.memo(function EdgePath({ edge, isHighlighted, isDimmed }: EdgePathProps) {
  const d = edgePath(edge.points);
  const color = EDGE_COLORS[edge.edgeType];
  const baseOpacity = EDGE_OPACITY[edge.edgeType];
  const opacity = isDimmed ? 0.2 : isHighlighted ? 1 : baseOpacity;
  const strokeWidth = isHighlighted ? 2 : 1.5;
  const isDashed = edge.edgeType === "conditional-true" || edge.edgeType === "conditional-false";
  const markerId = isHighlighted ? "arrow-highlighted" : `arrow-${edge.edgeType}`;

  const edgeAttrs = graphEdgeRenderAttrs({
    id: `edge:${edge.source}:${edge.target}`,
    sourceId: `block:${edge.source}`,
    targetId: `block:${edge.target}`,
    sourceBlockId: edge.source,
    targetBlockId: edge.target,
    role: edge.edgeType,
    label: edge.edgeType,
  });

  return (
    <path
      className={edgeAttrs.className}
      data-edge-id={edgeAttrs["data-edge-id"]}
      data-edge-role={edgeAttrs["data-edge-role"]}
      d={d}
      fill="none"
      stroke={isHighlighted ? T.group : color}
      strokeWidth={strokeWidth}
      strokeDasharray={isDashed ? "5 3" : undefined}
      opacity={opacity}
      markerEnd={`url(#${markerId})`}
      style={{ transition: "opacity 150ms, stroke 150ms" }}
    />
  );
});

// ─── Main Component ──────────────────────────────────────────────

interface GraphPanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
}

export function GraphPanel({
  id,
  focused,
  onClose,
  onFocus,
}: GraphPanelProps) {
  const { cursor, execution, setCursor, jumpToBlock, index } = useLocationStore(useShallow((s) => ({ cursor: s.cursor, execution: s.execution, setCursor: s.setCursor, jumpToBlock: s.jumpToBlock, index: s.index })));
  const { data } = useTcldData();
  const debug = useDebugStore();
  const blocks = useMemo(() => data?.blocks ?? [], [data?.blocks]);
  const instructions = useMemo(() => data?.instructions ?? [], [data?.instructions]);
  const executionBlockId = execution?.blockId ?? null;

  const [detailMode, setDetailMode] = useState<DetailMode>("medium");
  const [graphMode, setGraphMode] = useState<GraphMode>("basic-block");
  const [expandedBlocks, setExpandedBlocks] = useState<Set<number>>(new Set());
  const { menuState, showMenu, closeMenu } = useContextMenu();

  // Pan/zoom state in ref for perf (no re-renders during drag)
  const viewportRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const transformRef = useRef({ panX: 0, panY: 0, zoom: 1 });
  const gRef = useRef<SVGGElement>(null);
  const [, forceRender] = useState(0);
  const [zoom, setZoom] = useState(1);
  const [isPanningState, setIsPanningState] = useState(false);

  const hoverBlockId = useCallback((location: { blockId?: number | null; scopeId?: string | null }) => {
    if (location.scopeId != null && index?.scopeId != null && location.scopeId !== index.scopeId) {
      return null;
    }
    return location.blockId ?? null;
  }, [index]);

  useHoverHighlight({
    containerRef: gRef,
    rowSelector: ".graph-node",
    keyAttribute: "data-block",
    getKey: hoverBlockId,
  });

  // ─── Computed Data ───────────────────────────────────────────

  const sourceSummaries = useMemo(() => {
    const byBlock = new Map<number, string[]>();
    if (!data) return byBlock;
    for (const block of data.blocks) {
      const group = parseBlockGroup(block.group);
      const lines = data.decompileLines
        .filter((line) => group != null && line.group === group)
        .map((line) => line.text.trim())
        .filter(Boolean);
      if (lines.length > 0) {
        byBlock.set(block.id, lines);
        continue;
      }
      const sourceLines = data.sourceMap
        ?.filter((entry) => group != null && entry.commandGroup === group)
        .flatMap((entry) => entry.text.split("\n").map((line) => line.trim()).filter(Boolean)) ?? [];
      if (sourceLines.length > 0) byBlock.set(block.id, sourceLines);
    }
    return byBlock;
  }, [data]);

  const layout = useMemo(
    () => computeLayout(blocks, detailMode, expandedBlocks, instructions, sourceSummaries),
    [blocks, detailMode, expandedBlocks, instructions, sourceSummaries],
  );
  const graphModel = useMemo(
    () => data ? buildGraphViewModel(data, { mode: graphMode, cursor, execution: { blockId: executionBlockId } }) : null,
    [cursor, data, executionBlockId, graphMode],
  );

  const predecessorMap = useMemo(
    () => buildPredecessorMap(blocks),
    [blocks],
  );

  // Find which block the selected instruction belongs to
  const selectedBlockId = cursor.blockId ?? null;

  // Path highlight: predecessors + successors of selected block
  const { highlightedBlockIds, highlightedEdges } = useMemo(() => {
    if (selectedBlockId === null) return { highlightedBlockIds: new Set<number>(), highlightedEdges: new Set<string>() };

    const ids = new Set<number>();
    const edgeKeys = new Set<string>();
    ids.add(selectedBlockId);

    // Successors
    const selectedBlock = blocks.find((b) => b.id === selectedBlockId);
    if (selectedBlock) {
      for (const target of selectedBlock.edges) {
        ids.add(target);
        edgeKeys.add(`${selectedBlockId}->${target}`);
      }
    }

    // Predecessors
    const preds = predecessorMap.get(selectedBlockId) ?? [];
    for (const pred of preds) {
      ids.add(pred);
      edgeKeys.add(`${pred}->${selectedBlockId}`);
    }

    return { highlightedBlockIds: ids, highlightedEdges: edgeKeys };
  }, [selectedBlockId, blocks, predecessorMap]);

  const hasSelection = selectedBlockId !== null;

  // ─── Transform Helpers ─────────────────────────────────────

  const applyTransform = useCallback(() => {
    if (gRef.current) {
      const { panX, panY, zoom } = transformRef.current;
      gRef.current.setAttribute("transform", `translate(${panX}, ${panY}) scale(${zoom})`);
    }
  }, []);

  const fitToView = useCallback(() => {
    if (!viewportRef.current) return;
    const vp = viewportRef.current.getBoundingClientRect();
    if (layout.width === 0 || layout.height === 0) return;

    const scaleX = (vp.width - 20) / layout.width;
    const scaleY = (vp.height - 20) / layout.height;
    const zoom = Math.min(scaleX, scaleY, 1.5);
    const panX = (vp.width - layout.width * zoom) / 2;
    const panY = (vp.height - layout.height * zoom) / 2;

    transformRef.current = { panX, panY, zoom };
    setZoom(zoom);
    applyTransform();
    forceRender((n) => n + 1);
  }, [layout, applyTransform]);

  // First view: fit when that stays legible, otherwise keep a readable zoom
  // and frame the selected (or entry) block at the top of the viewport.
  const showInitialView = useCallback(() => {
    if (!viewportRef.current) return;
    const vp = viewportRef.current.getBoundingClientRect();
    if (layout.width === 0 || layout.height === 0) return;
    const fitZoom = Math.min((vp.width - 20) / layout.width, (vp.height - 20) / layout.height, 1.5);
    if (fitZoom >= READABLE_ZOOM) {
      fitToView();
      return;
    }
    const anchor = layout.nodes.find((n) => n.id === cursor.blockId)
      ?? layout.nodes.reduce<LayoutNode | null>((top, n) => (top == null || n.y < top.y ? n : top), null);
    const zoom = READABLE_ZOOM;
    const panX = anchor ? vp.width / 2 - (anchor.x + anchor.w / 2) * zoom : (vp.width - layout.width * zoom) / 2;
    const panY = anchor ? 24 - anchor.y * zoom : 24;
    transformRef.current = { panX, panY, zoom };
    setZoom(zoom);
    applyTransform();
    forceRender((n) => n + 1);
  }, [layout, cursor.blockId, fitToView, applyTransform]);

  const centerOnBlock = useCallback(
    (blockId: number) => {
      if (!viewportRef.current) return;
      const node = layout.nodes.find((n) => n.id === blockId);
      if (!node) return;

      const vp = viewportRef.current.getBoundingClientRect();
      const { zoom } = transformRef.current;
      const cx = node.x + node.w / 2;
      const cy = node.y + node.h / 2;
      const panX = vp.width / 2 - cx * zoom;
      const panY = vp.height / 2 - cy * zoom;

      transformRef.current = { ...transformRef.current, panX, panY };
      applyTransform();
      forceRender((n) => n + 1);
    },
    [layout, applyTransform],
  );

  const zoomBy = useCallback((delta: number) => {
    transformRef.current.zoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, transformRef.current.zoom + delta));
    setZoom(transformRef.current.zoom);
    applyTransform();
    forceRender((n) => n + 1);
  }, [applyTransform]);

  // ─── Initial fit + re-fit on layout change ─────────────────

  const hasFittedRef = useRef(false);
  useEffect(() => {
    if (blocks.length > 0 && !hasFittedRef.current) {
      // Small delay for viewport to have dimensions
      requestAnimationFrame(() => {
        showInitialView();
        hasFittedRef.current = true;
      });
    }
  }, [blocks.length, showInitialView]);

  // Re-fit when detail mode changes
  useEffect(() => {
    if (hasFittedRef.current) {
      requestAnimationFrame(fitToView);
    }
  }, [detailMode, fitToView]);

  // Auto-center on execution block when debugger stops (only on value change)
  const prevExecBlockRef = useRef<number | null | undefined>(undefined);
  useEffect(() => {
    if (executionBlockId != null && executionBlockId !== prevExecBlockRef.current && hasFittedRef.current) {
      requestAnimationFrame(() => centerOnBlock(executionBlockId));
    }
    prevExecBlockRef.current = executionBlockId;
  }, [executionBlockId, centerOnBlock]);

  // ─── Pan & Zoom Handlers ───────────────────────────────────

  const isPanning = useRef(false);
  const panStart = useRef({ x: 0, y: 0 });

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    // Only pan on left click on empty SVG area
    if (e.button !== 0) return;
    const target = e.target as SVGElement;
    if (target.tagName !== "svg" && target.tagName !== "rect" && target.closest("g[data-block]")) return;

    isPanning.current = true;
    setIsPanningState(true);
    panStart.current = { x: e.clientX - transformRef.current.panX, y: e.clientY - transformRef.current.panY };
    (e.target as Element).setPointerCapture(e.pointerId);
    e.preventDefault();
  }, []);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!isPanning.current) return;
    transformRef.current.panX = e.clientX - panStart.current.x;
    transformRef.current.panY = e.clientY - panStart.current.y;
    applyTransform();
  }, [applyTransform]);

  const handlePointerUp = useCallback(() => {
    if (isPanning.current) {
      isPanning.current = false;
      setIsPanningState(false);
      forceRender((n) => n + 1);
    }
  }, []);

  // Attach wheel listener natively with { passive: false } so preventDefault works
  // (React's onWheel is passive and ignores preventDefault)
  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = vp.getBoundingClientRect();
      const mouseX = e.clientX - rect.left;
      const mouseY = e.clientY - rect.top;

      const { panX, panY, zoom: oldZoom } = transformRef.current;
      const delta = e.deltaY > 0 ? -ZOOM_STEP : ZOOM_STEP;
      const newZoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, oldZoom + delta));

      const scale = newZoom / oldZoom;
      transformRef.current = {
        panX: mouseX - (mouseX - panX) * scale,
        panY: mouseY - (mouseY - panY) * scale,
        zoom: newZoom,
      };
      setZoom(newZoom);
      applyTransform();
      forceRender((n) => n + 1);
    };

    vp.addEventListener("wheel", onWheel, { passive: false });
    return () => vp.removeEventListener("wheel", onWheel);
  }, [applyTransform]);

  // ─── Block Interaction ─────────────────────────────────────

  const handleBlockClick = useCallback(
    (blockId: number) => {
      jumpToBlock(blockId);
    },
    [jumpToBlock],
  );

  const handleBlockDoubleClick = useCallback((blockId: number) => {
    setExpandedBlocks((prev) => {
      const next = new Set(prev);
      if (next.has(blockId)) next.delete(blockId);
      else next.add(blockId);
      return next;
    });
  }, []);

  const handleBlockHover = useCallback((blockId: number) => {
    const block = blocks.find((candidate) => candidate.id === blockId);
    if (!block) return;
    const group = index ? index.instIdxToGroup[block.start] : -1;
    const blockIdx = index ? index.instIdxToBlock[block.start] : -1;
    hoverBus.emit({
      scopeId: index?.scopeId ?? cursor.scopeId,
      blockId: blockIdx >= 0 ? blockIdx : blockId,
      instIdx: block.start,
      group: group >= 0 ? group : null,
    });
  }, [blocks, cursor.scopeId, index]);

  const clearHover = useCallback(() => {
    hoverBus.emit(null);
  }, []);

  const handleBlockContextMenu = useCallback(
    (e: React.MouseEvent, blockId: number) => {
      const block = blocks.find((b) => b.id === blockId);
      if (!block) return;
      const isExpanded = expandedBlocks.has(blockId);
      const group = index ? index.instIdxToGroup[block.start] : -1;
      const lineIdx = data?.decompileLines.findIndex((line) => line.group === (group >= 0 ? group : null)) ?? -1;
      const sourceLine = lineIdx >= 0
        ? (data?.decompileLines[lineIdx]?.sourceLineStart ?? (lineIdx + 1))
        : null;
      const sourceSpan = data?.sourceMap?.find(
        (entry) => entry.commandGroup === group && entry.originalSpan != null,
      )?.originalSpan;
      const commandRange = sourceSpan
        ? { offset: sourceSpan[0], len: sourceSpan[1] - sourceSpan[0] }
        : undefined;
      const canRunToBlockStart = Boolean(debug.scriptPath && sourceLine != null);

      const items: ContextMenuItem[] = [
        {
          label: "Go to Block",
          icon: <ArrowRight size={12} aria-hidden />,
          onClick: () => jumpToBlock(blockId),
        },
        {
          label: "Run to Block Start",
          icon: <Play size={12} aria-hidden />,
          shortcut: "Ctrl+F5",
          disabled: !canRunToBlockStart,
          onClick: () => {
            if (!debug.scriptPath || sourceLine == null) return;
            void debug.runToCursor(debug.scriptPath, sourceLine, commandRange);
          },
        },
        {
          label: "Center on Block",
          icon: <Crosshair size={12} aria-hidden />,
          shortcut: "c",
          onClick: () => centerOnBlock(blockId),
        },
        {
          label: "Fit to View",
          icon: <Maximize size={12} aria-hidden />,
          shortcut: "f",
          onClick: () => fitToView(),
        },
        {
          label: isExpanded ? "Collapse Detail" : "Expand Detail",
          icon: isExpanded ? <ChevronRight size={12} aria-hidden /> : <ChevronDown size={12} aria-hidden />,
          onClick: () => handleBlockDoubleClick(blockId),
        },
      ];
      showMenu(e, items);
    },
    [blocks, expandedBlocks, index, data?.decompileLines, data?.sourceMap, debug, jumpToBlock, centerOnBlock, fitToView, handleBlockDoubleClick, showMenu],
  );

  const commands = useMemo(() => ([
    {
      id: "graph:mode-basic-block",
      title: "Graph Mode: Basic Block",
      category: "Graph",
      when: "focusedPanel:graph,!overlayOpen",
      run: () => setGraphMode("basic-block"),
    },
    {
      id: "graph:mode-source-snippet",
      title: "Graph Mode: Source Snippet",
      category: "Graph",
      when: "focusedPanel:graph,!overlayOpen",
      run: () => setGraphMode("source-snippet"),
    },
    {
      id: "graph:fit",
      title: "Fit Graph To View",
      category: "Graph",
      shortcut: ["f"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => fitToView(),
    },
    {
      id: "graph:center-selection",
      title: "Center Graph Selection",
      category: "Graph",
      shortcut: ["c"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => {
        if (selectedBlockId !== null) centerOnBlock(selectedBlockId);
      },
    },
    {
      id: "graph:expand-node",
      title: "Expand Graph Node",
      category: "Graph",
      when: "focusedPanel:graph,!overlayOpen",
      run: () => {
        if (selectedBlockId !== null) handleBlockDoubleClick(selectedBlockId);
      },
    },
    {
      id: "graph:copy-details",
      title: "Copy Graph Details",
      category: "Graph",
      when: "focusedPanel:graph,!overlayOpen",
      run: () => {
        const node = graphModel?.nodes.find((candidate) => candidate.blockId === selectedBlockId) ?? graphModel?.nodes[0];
        if (node) void navigator.clipboard.writeText(`${node.label} ${node.detail}`);
      },
    },
    {
      id: "graph:detail-compact",
      title: "Graph Detail: Compact",
      category: "Graph",
      shortcut: ["1"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => setDetailMode("compact"),
    },
    {
      id: "graph:detail-medium",
      title: "Graph Detail: Medium",
      category: "Graph",
      shortcut: ["2"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => setDetailMode("medium"),
    },
    {
      id: "graph:detail-full",
      title: "Graph Detail: Full",
      category: "Graph",
      shortcut: ["3"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => setDetailMode("full"),
    },
    {
      id: "graph:zoom-in",
      title: "Graph Zoom In",
      category: "Graph",
      when: "focusedPanel:graph,!overlayOpen",
      run: () => zoomBy(ZOOM_STEP),
    },
    {
      id: "graph:zoom-out",
      title: "Graph Zoom Out",
      category: "Graph",
      when: "focusedPanel:graph,!overlayOpen",
      run: () => zoomBy(-ZOOM_STEP),
    },
    {
      id: `${id}:next_block`,
      title: "Graph Next Block",
      category: "Graph",
      shortcut: ["Tab"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => {
        if (selectedBlockId === null) return;
        const selectedBlock = blocks.find((block) => block.id === selectedBlockId);
        if (!selectedBlock || selectedBlock.edges.length === 0) return;
        const nextBlockId = selectedBlock.edges[0];
        const nextBlock = blocks.find((block) => block.id === nextBlockId);
        if (!nextBlock) return;
        jumpToBlock(nextBlockId);
        centerOnBlock(nextBlockId);
      },
    },
    {
      id: `${id}:prev_block`,
      title: "Graph Previous Block",
      category: "Graph",
      shortcut: ["Shift+Tab"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => {
        if (selectedBlockId === null) return;
        const preds = predecessorMap.get(selectedBlockId);
        if (!preds || preds.length === 0) return;
        const prevId = preds[0];
        const prevBlock = blocks.find((block) => block.id === prevId);
        if (!prevBlock) return;
        jumpToBlock(prevId);
        centerOnBlock(prevId);
      },
    },
  ]), [blocks, centerOnBlock, fitToView, graphModel?.nodes, handleBlockDoubleClick, id, jumpToBlock, predecessorMap, selectedBlockId, zoomBy]);

  useCommands(commands);

  // ─── Zoom display ──────────────────────────────────────────

  const zoomPercent = Math.round(zoom * 100);
  const metrics = graphModel?.metrics ?? {
    blockCount: blocks.length,
    edgeCount: layout.edges.length,
    loopCount: 0,
    branchCount: 0,
    cyclomatic: 0,
    depth: 0,
  };
  const selectedStatus = selectedBlockId != null ? `selected b${String(selectedBlockId).padStart(3, "0")}` : "selected none";

  // ─── Render ────────────────────────────────────────────────

  if (blocks.length === 0) {
    return (
      <Panel id={id} title="Graph" badge="0 blocks" focused={focused} designSurface="view-flow" onClose={onClose} onFocus={onFocus}>
        <EmptyState
          icon={<Workflow aria-hidden />}
          title="No control-flow graph"
          message="Open a file with bytecode to see its basic blocks and branches."
        />
      </Panel>
    );
  }

  return (
    <Panel
      designSurface="view-flow"
      id={id}
      title="Graph"
      badge={`${blocks.length} blocks`}
      focused={focused}
      onClose={onClose}
      onFocus={onFocus}
    >
      {/* Toolbar */}
      <div className="tcld-flow-toolbar">
        <div className="tcld-toolbar-group" role="tablist" aria-label="Block contents">
          {([
            { mode: "basic-block", actionId: "graph:mode-basic-block" },
            { mode: "source-snippet", actionId: "graph:mode-source-snippet" },
          ] as const).map(({ mode, actionId }) => (
            <button
              key={mode}
              type="button"
              role="tab"
              aria-selected={graphMode === mode}
              onClick={() => commandRegistry.execute(actionId)}
              className={clsx("tcld-toolbar-pill", graphMode === mode && "is-active")}
              title={mode === "basic-block" ? "Basic Block Mode" : "Source Snippet Mode"}
            >
              {mode === "basic-block" ? "Disasm" : "Source"}
            </button>
          ))}
        </div>

        {/* Detail mode toggle */}
        <div className="tcld-toolbar-group" role="tablist" aria-label="Block detail">
          {([
            { mode: "compact", actionId: "graph:detail-compact" },
            { mode: "medium", actionId: "graph:detail-medium" },
            { mode: "full", actionId: "graph:detail-full" },
          ] as const).map(({ mode, actionId }) => (
            <button
              key={mode}
              type="button"
              role="tab"
              aria-selected={detailMode === mode}
              onClick={() => commandRegistry.execute(actionId)}
              className={clsx("tcld-toolbar-pill", detailMode === mode && "is-active")}
              title={`${mode} detail (${mode === "compact" ? "1" : mode === "medium" ? "2" : "3"})`}
            >
              {mode[0].toUpperCase() + mode.slice(1)}
            </button>
          ))}
        </div>

        <span className="tcld-toolbar-divider" aria-hidden />
        <span className="tcld-toolbar-meta">{metrics.blockCount} blocks · {metrics.edgeCount} edges</span>
        <span className="tcld-flow-chip tcld-flow-chip-accent" title="Back edges that close a loop">{metrics.loopCount} loops</span>
        <span className="tcld-flow-chip tcld-flow-chip-warn" title="Blocks that end in a conditional jump">{metrics.branchCount} branches</span>

        <div className="flex-1" />

        {/* Viewport: fit, center, zoom */}
        <div className="tcld-toolbar-group" role="group" aria-label="Zoom">
          <button
            type="button"
            onClick={() => commandRegistry.execute("graph:fit")}
            className="tcld-toolbar-icon"
            title="Fit to view (f)"
            aria-label="Fit to view"
          >
            <Maximize size={14} aria-hidden />
          </button>
          <button
            type="button"
            onClick={() => commandRegistry.execute("graph:center-selection")}
            className="tcld-toolbar-icon"
            disabled={selectedBlockId === null}
            title="Center on selected (c)"
            aria-label="Center on selected block"
          >
            <Crosshair size={14} aria-hidden />
          </button>
          <button
            type="button"
            onClick={() => commandRegistry.execute("graph:zoom-out")}
            className="tcld-toolbar-icon"
            title="Zoom out"
            aria-label="Zoom out"
          >
            <ZoomOut size={14} aria-hidden />
          </button>
          <span className="tcld-flow-zoom" aria-live="polite">
            {zoomPercent}%
          </span>
          <button
            type="button"
            onClick={() => commandRegistry.execute("graph:zoom-in")}
            className="tcld-toolbar-icon"
            title="Zoom in"
            aria-label="Zoom in"
          >
            <ZoomIn size={14} aria-hidden />
          </button>
        </div>

        <div className="tcld-flow-inline-legend" data-flow-edge-legend="toolbar">
          {EDGE_LEGEND.map((item) => (
            <span key={item.role} className="tcld-flow-legend-item" data-edge-role={item.role}>
              <svg width={14} height={6} aria-hidden>
                <line
                  x1={0}
                  y1={3}
                  x2={14}
                  y2={3}
                  stroke={EDGE_COLORS[item.role]}
                  strokeWidth={1.2}
                  strokeDasharray={item.role === "conditional-true" || item.role === "conditional-false" ? "3 2" : undefined}
                />
              </svg>
              <span style={{ color: EDGE_COLORS[item.role] }}>{item.label}</span>
            </span>
          ))}
        </div>
      </div>

      {/* Graph viewport */}
      <div
        ref={viewportRef}
        className="tcld-flow-canvas"
        style={{ cursor: isPanningState ? "grabbing" : "grab" }}
      >
        {graphMode === "source-snippet" && graphModel ? (
          <div className="tcld-flow-source-board">
            <div className="tcld-flow-source-grid">
              {graphModel.nodes.map((node, index) => {
                const attrs = graphNodeRenderAttrs(node);
                const lineRange = node.sourceLineRange
                  ? `${node.sourceLineRange[0]}..${node.sourceLineRange[1]}`
                  : "-";
                return (
                  <button
                    key={node.id}
                    type="button"
                    className={clsx(attrs.className, "tcld-flow-source-node")}
                    data-graph-mode={attrs["data-graph-mode"]}
                    data-node-kind={attrs["data-node-kind"]}
                    data-flow-role={attrs["data-flow-role"]}
                    data-flow-block-role={attrs["data-flow-block-role"]}
                    data-node-id={attrs["data-node-id"]}
                    data-command-path={attrs["data-command-path"]}
                    style={{ ["--flow-node-index" as string]: index }}
                    onClick={() => {
                      setCursor(node.cursorTarget, { source: "user" });
                    }}
                    onMouseEnter={() => {
                      hoverBus.emit({
                        scopeId: cursor.scopeId,
                        blockId: node.blockId,
                        group: node.cursorTarget.group ?? null,
                        instIdx: node.cursorTarget.instIdx ?? null,
                        sourceLine: node.cursorTarget.sourceLine ?? null,
                      });
                    }}
                    onMouseLeave={clearHover}
                  >
                    <span
                      className="tcld-flow-source-node-rail"
                      aria-hidden
                      style={{ background: BLOCK_ROLE_COLORS[node.blockRole] }}
                    />
                    <span className="tcld-flow-source-node-header">
                      <span>{node.commandPath ?? node.label}</span>
                      <span>{BLOCK_ROLE_LABELS[node.blockRole]} · Ln {lineRange}</span>
                    </span>
                    <span className="tcld-flow-source-node-snippet">{node.snippet}</span>
                    <span className="tcld-flow-source-node-detail">
                      {node.commandGroup ?? "source"} · {node.pcRange ? `pc ${node.pcRange[0]}..${node.pcRange[1]}` : "pending bytecode"}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>
        ) : (
        <svg
          ref={svgRef}
          width="100%"
          height="100%"
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          style={{ display: "block" }}
        >
          <defs>
            {/* Arrow markers for each edge type */}
            {Object.entries(EDGE_COLORS).map(([type, color]) => (
              <marker
                key={type}
                id={`arrow-${type}`}
                markerWidth={7}
                markerHeight={5}
                refX={6}
                refY={2.5}
                orient="auto"
              >
                <path d="M0,0 L7,2.5 L0,5" fill={color} />
              </marker>
            ))}
            {/* Highlighted edge arrow marker */}
            <marker
              id="arrow-highlighted"
              markerWidth={7}
              markerHeight={5}
              refX={6}
              refY={2.5}
              orient="auto"
            >
              <path d="M0,0 L7,2.5 L0,5" fill={T.group} />
            </marker>
            {/* Glow filter for selected node */}
            <filter id="glow" x="-20%" y="-20%" width="140%" height="140%">
              <feGaussianBlur stdDeviation="3" result="blur" />
              <feMerge>
                <feMergeNode in="blur" />
                <feMergeNode in="SourceGraphic" />
              </feMerge>
            </filter>
          </defs>

          <g ref={gRef}>
            {/* Edges (behind nodes) */}
            {layout.edges.map((edge) => {
              const key = `${edge.source}->${edge.target}`;
              const isHighlighted = highlightedEdges.has(key);
              const isDimmed = hasSelection && !isHighlighted;
              return (
                <EdgePath
                  key={key}
                  edge={edge}
                  isHighlighted={isHighlighted}
                  isDimmed={isDimmed}
                />
              );
            })}

            {/* Nodes */}
            {layout.nodes.map((node) => {
              const isSelected = node.id === selectedBlockId;
              const isPathHighlighted = highlightedBlockIds.has(node.id);
              const isDimmed = hasSelection && !isPathHighlighted;
              const isExecution = node.id === executionBlockId;
              return (
                <BlockNode
                  key={node.id}
                  node={node}
                  mode={detailMode}
                  blockRole={deriveGraphBlockRole(node.block, blocks, instructions)}
                  sourceLines={sourceSummaries.get(node.id) ?? []}
                  isSelected={isSelected}
                  isPathHighlighted={isPathHighlighted}
                  isDimmed={isDimmed}
                  isExecution={isExecution}
                  expandedBlocks={expandedBlocks}
                  onBlockClick={handleBlockClick}
                  onBlockDoubleClick={handleBlockDoubleClick}
                  onBlockHover={handleBlockHover}
                  onBlockHoverEnd={clearHover}
                  onBlockContextMenu={handleBlockContextMenu}
                />
              );
            })}
          </g>
        </svg>
        )}
      </div>

      <div className="tcld-flow-status" data-flow-status="true">
        <span data-flow-selected-block={selectedBlockId != null ? String(selectedBlockId) : "none"}>{selectedStatus}</span>
        <span>cyclomatic {metrics.cyclomatic}</span>
        <span>depth {metrics.depth}</span>
        <span>{metrics.blockCount} blocks</span>
        <span>{metrics.edgeCount} edges</span>
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
