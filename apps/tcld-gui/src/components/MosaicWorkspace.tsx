"use client";

import React, { useCallback, useEffect, useEffectEvent, useMemo, useRef, useState } from "react";
import { Mosaic } from "react-mosaic-component";
import "react-mosaic-component/react-mosaic-component.css";
import {
  DndContext,
  DragOverlay,
  useDraggable,
  useDroppable,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import type { DragStartEvent, DragEndEvent, DragOverEvent } from "@dnd-kit/core";
import { Plus, Replace } from "lucide-react";
import { ViewIcon } from "./ViewIcon";
import { Panel } from "./panels/Panel";
import { PanelMetaProvider } from "./panels/PanelContext";
import { useTcldData } from "@/lib/store";
import { useDebugStore } from "@/lib/debug-store";
import { useLocationStore } from "@/lib/location-store";
import { computeScopeId } from "@/lib/location-index";
import { T, FONT_MONO } from "@/lib/theme";
import { ALL_VIEW_IDS, VIEW_DEFINITIONS, VIEW_KEYS, VIEW_TITLES } from "@/lib/view-registry";
import { compactCount } from "@/lib/format";
import type { PanelActionItem } from "@/components/panels/PanelContext";
import type { ViewId, ViewStacks, WorkspaceLeafId, WorkspaceTree } from "@/types";
import {
  findLeafHostingView,
  findRightmostLeaf,
  getLeafIds,
  getOpenViews,
  type SplitZone,
} from "@/lib/default-layouts";
import {
  leafDropHandle,
  parseLeafDropHandle,
  parseTabSlotHandle,
} from "@/lib/ids";
import {
  honorsLeafMinSizes,
  PREFERRED_FLYOUT_VIEW,
  WORKSPACE_COLLAPSE_AT_PX,
  WORKSPACE_RESTORE_AT_PX,
  leafMinWidthPx,
} from "@/lib/layout-constraints";
import { X, PanelRightOpen } from "lucide-react";

// ─── Panel metadata ──────────────────────────────────────────────

type DropZone = "left" | "right" | "top" | "bottom" | "center";
type WindowEdge = Exclude<DropZone, "center">;

type DragDescriptor =
  | { kind: "leaf"; leafId: WorkspaceLeafId; activeViewId: ViewId | null }
  | { kind: "tab"; leafId: WorkspaceLeafId; viewId: ViewId };

const LEAF_DRAG_PREFIX = "leaf:";
const WINDOW_EDGE_PREFIX = "window-edge:";

interface TabSlotTarget {
  leafId: WorkspaceLeafId;
  index: number;
}

// Wire-format helpers for tab-slot / leaf-drop ids live in lib/ids.ts so
// every constructor and parser shares one source of truth. The functions
// below are thin local adapters that keep the existing call sites in this
// file readable while delegating the actual string layout to the helpers.
function parseTabSlotId(id: string): TabSlotTarget | null {
  return parseTabSlotHandle(id);
}

function parseLeafDropId(id: string): WorkspaceLeafId | null {
  return parseLeafDropHandle(id);
}

function parseWindowEdgeId(id: string): WindowEdge | null {
  if (!id.startsWith(WINDOW_EDGE_PREFIX)) {
    return null;
  }
  const edge = id.slice(WINDOW_EDGE_PREFIX.length);
  if (edge === "left" || edge === "right" || edge === "top" || edge === "bottom") {
    return edge;
  }
  return null;
}

// ─── Drop zone overlay config (GPU-accelerated clip-path) ───────

function computeZone(clientX: number, clientY: number, rect: DOMRect): DropZone {
  const relX = (clientX - rect.left) / rect.width;
  const relY = (clientY - rect.top) / rect.height;
  const dL = relX, dR = 1 - relX, dT = relY, dB = 1 - relY;
  const minD = Math.min(dL, dR, dT, dB);
  if (minD > 0.25) return "center";
  if (minD === dL) return "left";
  if (minD === dR) return "right";
  if (minD === dT) return "top";
  return "bottom";
}

const ZONE_CLIP: Record<DropZone, string> = {
  left:   "inset(0 50% 0 0)",
  right:  "inset(0 0 0 50%)",
  top:    "inset(0 0 50% 0)",
  bottom: "inset(50% 0 0 0)",
  center: "inset(0 0 0 0)",
};

const ZONE_CONTENT_POS: Record<DropZone, { left: string; top: string }> = {
  left:   { left: "25%", top: "50%" },
  right:  { left: "75%", top: "50%" },
  top:    { left: "50%", top: "25%" },
  bottom: { left: "50%", top: "75%" },
  center: { left: "50%", top: "50%" },
};

const TRANSITION_ZONE = "clip-path 0.18s cubic-bezier(0.22, 1, 0.36, 1), opacity 0.15s ease";
const TRANSITION_CONTENT = "left 0.18s cubic-bezier(0.22, 1, 0.36, 1), top 0.18s cubic-bezier(0.22, 1, 0.36, 1), opacity 0.12s ease";

// ─── DndTile: draggable + droppable tile wrapper ─────────────────

interface DndTileProps {
  leafId: WorkspaceLeafId;
  activeDrag: DragDescriptor | null;
  dropZoneRef: React.MutableRefObject<DropZone>;
  /** Pixel floor for this tile's width. Computed from `leafMinWidthPx` over
   *  the tile's view stack. Drives the `min-width` style so the tile never
   *  visually crushes below the per-view contract, even if the parent
   *  splitter is dragged past it. */
  minWidthPx: number;
  meta: {
    leafId: WorkspaceLeafId;
    icon?: React.ReactNode;
    keyHint?: string;
    expanded: boolean;
    onExpand: () => void;
    tabs?: Array<{
      id: ViewId;
      label: string;
      icon?: React.ReactNode;
      keyHint?: string;
    }>;
    activeTabId?: ViewId;
    onSelectTab?: (tabId: ViewId) => void;
    onCloseTab?: (tabId: ViewId) => void;
    actions?: React.ReactNode;
  };
  children: React.ReactNode;
}

const DndTile = React.memo(function DndTile({
  leafId,
  activeDrag,
  dropZoneRef,
  minWidthPx,
  meta,
  children,
}: DndTileProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const {
    attributes,
    listeners,
    setNodeRef: setDragRef,
    isDragging,
  } = useDraggable({
    id: `${LEAF_DRAG_PREFIX}${leafId}`,
    data: {
      kind: "leaf",
      leafId,
      activeViewId: meta.activeTabId ?? null,
    } satisfies DragDescriptor,
  });
  const { isOver, setNodeRef: setDropRef } = useDroppable({
    id: leafDropHandle(leafId),
  });

  const isDropTarget = isOver && !isDragging;
  const [zone, setZone] = useState<DropZone>("center");

  // Compute zone locally from mousemove — only runs on the hovered tile
  useEffect(() => {
    if (!isDropTarget) {
      if (zone !== "center") {
        const timer = window.setTimeout(() => setZone("center"), 0);
        return () => window.clearTimeout(timer);
      }
      return;
    }
    const el = containerRef.current;
    if (!el) return;

    // Cache rect once (tile doesn't move during drag)
    const rect = el.getBoundingClientRect();

    const onMove = (e: MouseEvent) => {
      const z = computeZone(e.clientX, e.clientY, rect);
      setZone((prev) => {
        if (prev !== z) {
          dropZoneRef.current = z;
          return z;
        }
        return prev;
      });
    };

    window.addEventListener("mousemove", onMove);
    return () => window.removeEventListener("mousemove", onMove);
  }, [dropZoneRef, isDropTarget, zone]);

  const isAnyTabDragging = activeDrag?.kind === "tab";
  const fullMeta = useMemo(() => ({
    ...meta,
    dragHandleProps: { ...listeners, ...attributes } as Record<string, unknown>,
    isDropTarget,
    isAnyTabDragging,
  }), [meta, listeners, attributes, isDropTarget, isAnyTabDragging]);

  const showOverlay = isDropTarget && activeDrag !== null;
  const activeViewId = activeDrag?.kind === "leaf"
    ? activeDrag.activeViewId
    : (activeDrag?.kind === "tab" ? activeDrag.viewId : null);
  const pos = ZONE_CONTENT_POS[zone];
  const isDebugRail = (meta.tabs ?? []).some((tab) =>
    tab.id === "variables" || tab.id === "stack" || tab.id === "breakpoints" || tab.id === "watchpoints"
  );

  return (
    <div
      ref={(node) => {
        containerRef.current = node;
        setDragRef(node);
        setDropRef(node);
      }}
      data-design-surface="workspace-mosaic"
      data-leaf-id={leafId}
      data-debug-rail={isDebugRail ? "true" : undefined}
      data-leaf-min-width={minWidthPx}
      style={{ height: "100%", minWidth: minWidthPx, position: "relative" }}
    >
      {/* Panel content */}
      <div
        style={{
          height: "100%",
          opacity: isDragging ? 0.3 : 1,
          transform: isDropTarget ? "scale(0.97)" : "scale(1)",
          transition: "opacity 0.15s ease, transform 0.2s cubic-bezier(0.22, 1, 0.36, 1)",
          transformOrigin: "center center",
        }}
      >
        <PanelMetaProvider value={fullMeta}>{children}</PanelMetaProvider>
      </div>

      {/* Drop zone overlay — always mounted, driven by CSS transitions */}
      <div
        style={{
          position: "absolute",
          inset: 0,
          clipPath: showOverlay ? ZONE_CLIP[zone] : "inset(50% 50% 50% 50%)",
          opacity: showOverlay ? 1 : 0,
          transition: TRANSITION_ZONE,
          pointerEvents: "none",
          zIndex: 10,
          borderRadius: 3,
        }}
      >
        {/* Frosted background */}
        <div
          style={{
            position: "absolute",
            inset: 0,
            background: `color-mix(in srgb, ${T.bg} 90%, transparent)`,
            borderRadius: 3,
            border: `2px dashed ${T.accent}`,
            boxShadow: `inset 0 0 30px color-mix(in srgb, ${T.accent} 8%, transparent)`,
          }}
        />
        {/* Content — positioned in the center of the visible zone half */}
        <div
          style={{
            position: "absolute",
            left: pos.left,
            top: pos.top,
            transform: "translate(-50%, -50%)",
            transition: TRANSITION_CONTENT,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: 6,
            opacity: showOverlay ? 1 : 0,
          }}
        >
          {activeViewId && (
            <>
              <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                <span style={{ color: T.accent, opacity: 0.7 }}>
                  <ViewIcon viewId={activeViewId} size={18} />
                </span>
                <span
                  style={{
                    fontFamily: FONT_MONO,
                    fontSize: 13,
                    fontWeight: 600,
                    color: T.accent,
                    letterSpacing: "0.02em",
                  }}
                >
                  {VIEW_TITLES[activeViewId]}
                </span>
              </div>
              <span
                style={{
                  fontFamily: FONT_MONO,
                  fontSize: 10,
                  color: T.fgDim,
                  letterSpacing: "0.05em",
                  textTransform: "uppercase",
                  opacity: 0.7,
                }}
              >
                {/* Direction-specific drop labels — turns the previously
                    ambiguous "split" word into an actionable preview the
                    user can read mid-drag. */}
                {zone === "center" && "Add to tabs"}
                {zone === "left" && "Split ← left"}
                {zone === "right" && "Split right →"}
                {zone === "top" && "Split ↑ top"}
                {zone === "bottom" && "Split ↓ bottom"}
              </span>
            </>
          )}
        </div>
      </div>
    </div>
  );
});

interface WindowEdgeDropProps {
  edge: WindowEdge;
  activeDrag: DragDescriptor | null;
}

const WindowEdgeDrop = React.memo(function WindowEdgeDrop({ edge, activeDrag }: WindowEdgeDropProps) {
  const { isOver, setNodeRef } = useDroppable({
    id: `${WINDOW_EDGE_PREFIX}${edge}`,
  });
  const visible = activeDrag !== null;
  const active = visible && isOver;

  const styleByEdge: Record<WindowEdge, React.CSSProperties> = {
    left: { left: 0, top: 0, bottom: 0, width: 24 },
    right: { right: 0, top: 0, bottom: 0, width: 24 },
    top: { left: 0, top: 0, right: 0, height: 24 },
    bottom: { left: 0, bottom: 0, right: 0, height: 24 },
  };

  return (
    <div
      ref={setNodeRef}
      style={{
        position: "absolute",
        ...styleByEdge[edge],
        pointerEvents: visible ? "auto" : "none",
        opacity: visible ? 1 : 0,
        transition: "opacity 0.14s ease",
        zIndex: 30,
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: 0,
          background: active
            ? `color-mix(in srgb, ${T.accent} 24%, transparent)`
            : `color-mix(in srgb, ${T.bgHighlight} 35%, transparent)`,
          boxShadow: active
            ? `inset 0 0 0 1px ${T.accent}, 0 0 20px color-mix(in srgb, ${T.accent} 22%, transparent)`
            : `inset 0 0 0 1px color-mix(in srgb, ${T.border} 80%, transparent)`,
          transition: "background 0.14s ease, box-shadow 0.14s ease",
        }}
      />
    </div>
  );
});

// ─── Lazy imports for panel content ──────────────────────────────

import { InstructionPanel, type InstructionPresentationMode } from "@/components/panels/InstructionPanel";
import { DecompilePanel } from "@/components/panels/DecompilePanel";
import { DetailPanel } from "@/components/panels/DetailPanel";
import { LiteralsPanel } from "@/components/panels/LiteralsPanel";
import { ExceptionsPanel } from "@/components/panels/ExceptionsPanel";
import { GraphPanel } from "@/components/panels/GraphPanel";

import { SummaryPanel } from "@/components/panels/SummaryPanel";
import { VariablesPanel } from "@/components/panels/VariablesPanel";
import { StackPanel } from "@/components/panels/StackPanel";
import { ConsolePanel } from "@/components/panels/ConsolePanel";
import { BreakpointsPanel } from "@/components/panels/BreakpointsPanel";
import { SourcePanel } from "@/components/panels/SourcePanel";
import { ComparisonPanel } from "@/components/panels/ComparisonPanel";
import { DiagnosticsPanel } from "@/components/panels/DiagnosticsPanel";
import { ProvenancePanel } from "@/components/panels/ProvenancePanel";
import { ProjectPanel } from "@/components/panels/ProjectPanel";
import { WatchpointsPanel } from "@/components/panels/WatchpointsPanel";
import { CommandResolverPanel } from "@/components/panels/CommandResolverPanel";

// ─── Component ───────────────────────────────────────────────────

import type { Annotations } from "@/lib/annotations";
import type { TcldData } from "@/lib/store-data";

// PanelHeaderAction was the inline header-action button used by the now
// removed LeafViewMenu / ViewHeaderActions. The unified PanelHeaderMenu
// owns all panel-header chrome — nothing in MosaicWorkspace renders inline
// header buttons anymore. The corresponding `.tcld-panel-header-action*`
// CSS in globals.css is also dead and should be cleaned up in a follow-up.

function sourceLineCount(data: TcldData | null): number | undefined {
  const text = data?.originalSourceText ?? null;
  if (!text) return undefined;
  return text.endsWith("\n") ? text.split("\n").length - 1 : text.split("\n").length;
}

function lineForUtf8ByteOffset(source: string, offset: number): number {
  const encoder = new TextEncoder();
  let consumed = 0;
  const lines = source.split(/(?<=\n)/);
  for (let index = 0; index < lines.length; index++) {
    const next = consumed + encoder.encode(lines[index]).length;
    if (offset < next) return index + 1;
    consumed = next;
  }
  return Math.max(1, lines.length);
}

/**
 * Total issue count for the diagnostics tab badge. This mirrors the rows
 * produced by buildDiagnosticsPanelModel.
 */
function diagnosticsIssueCount(data: TcldData | null): number | undefined {
  const diagnostics = data?.diagnostics;
  if (!diagnostics) return undefined;
  const approximations = diagnostics.approximations?.length ?? 0;
  const unsupported = diagnostics.unsupportedInstructions?.length ?? 0;
  const unreduced = diagnostics.unreducedOpcodes?.length ?? 0;
  const suppressed = diagnostics.suppressedFastPathRanges?.length ?? 0;
  const passDisabled = (diagnostics.passDecisions ?? []).filter((decision) => !decision.enabled).length;
  return approximations + unsupported + unreduced + suppressed + passDisabled;
}

function tabBadgeForView(
  view: ViewId,
  data: TcldData | null,
  debug: ReturnType<typeof useDebugStore>,
): string | number | null {
  switch (view) {
    case "instructions":
      return compactCount(data?.instructions.length) ?? null;
    case "literals":
      return compactCount(data?.literals.length) ?? null;
    case "exceptions":
      return compactCount(data?.exceptions.length) ?? null;
    case "graph":
      return compactCount(data?.blocks.length) ?? null;
    case "decompile":
      return compactCount(data?.decompileLines.length) ?? null;
    case "source":
      return compactCount(sourceLineCount(data)) ?? null;
    case "comparison":
    case "provenance":
      return compactCount(data?.sourceMap?.length) ?? null;
    case "diagnostics":
      return compactCount(diagnosticsIssueCount(data)) ?? null;
    case "variables":
      return compactCount(debug.variables.length) ?? null;
    case "stack":
      return compactCount(debug.stack.length) ?? null;
    case "console":
      return compactCount(debug.output.length) ?? null;
    case "breakpoints":
      return compactCount(debug.breakpoints.length) ?? null;
    case "watchpoints":
      return compactCount(debug.watchpoints.length) ?? null;
    default:
      return null;
  }
}

// LeafViewMenu / ViewHeaderActions previously rendered the swap-view and
// add-view popovers as inline header buttons. They were removed when the
// panel-header collapsed into a single ⋯ menu — those actions are now
// surfaced as typed `actionItems` with submenus on the leaf meta below.

interface MosaicWorkspaceProps {
  mosaicNode: WorkspaceTree | null;
  viewStacks: ViewStacks;
  focusedViewId: ViewId | null;
  expandedViewId: WorkspaceLeafId | null;
  setMosaicNode: (node: WorkspaceTree | null) => void;
  stackLeafIntoTabs: (sourceLeafId: WorkspaceLeafId, targetLeafId: WorkspaceLeafId) => void;
  moveLeafToSplit: (
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
    zone: SplitZone,
  ) => void;
  moveLeafToWindowEdge: (sourceLeafId: WorkspaceLeafId, edge: SplitZone) => void;
  stackViewIntoLeaf: (
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
  ) => void;
  /**
   * Reorder a tab to a specific position (drop-on-tab-slot). targetIndex
   * is the insertion point in the resulting target stack's views array.
   */
  reorderTabInLeaf: (
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
    targetIndex: number,
  ) => void;
  addViewToLeaf: (viewId: ViewId, leafId: WorkspaceLeafId) => void;
  swapViewInLeaf: (oldViewId: ViewId, newViewId: ViewId, leafId: WorkspaceLeafId) => void;
  splitViewIntoLeaf: (
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
    zone: SplitZone,
  ) => void;
  splitViewToWindowEdge: (
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    edge: SplitZone,
  ) => void;
  setFocusedView: (viewId: ViewId) => void;
  removeView: (viewId: ViewId) => void;
  setSelected: (idx: number) => void;
  toggleExpand: () => void;
  annotations: Annotations;
  showJumpArrows: boolean;
  onToggleJumpArrows: () => void;
  instructionPresentationMode?: InstructionPresentationMode;
  onInstructionPresentationModeChange?: (mode: InstructionPresentationMode) => void;
  onToggleBookmark: (idx: number) => void;
  onSetComment: (idx: number, text: string) => void;
  /**
   * Flyout state — managed by useExplorer so addView / togglePanel can
   * route flyout-resident views to the drawer instead of duplicating them
   * into the mosaic. The four fields below are the entire state surface;
   * the helpers (extractLeafToFlyout / restoreFlyoutLeaf / setFlyoutOpen)
   * mutate them.
   */
  flyoutLeafId: WorkspaceLeafId | null;
  flyoutOpen: boolean;
  flyoutDismissed: boolean;
  extractLeafToFlyout: (leafId: WorkspaceLeafId) => void;
  restoreFlyoutLeaf: () => void;
  setFlyoutOpen: (open: boolean) => void;
}

export const MosaicWorkspace = React.memo(function MosaicWorkspace({
  mosaicNode,
  viewStacks,
  focusedViewId,
  expandedViewId,
  setMosaicNode,
  stackLeafIntoTabs,
  moveLeafToSplit,
  moveLeafToWindowEdge,
  stackViewIntoLeaf,
  reorderTabInLeaf,
  addViewToLeaf,
  swapViewInLeaf,
  splitViewIntoLeaf,
  splitViewToWindowEdge,
  setFocusedView,
  removeView,
  setSelected,
  toggleExpand,
  annotations,
  showJumpArrows,
  onToggleJumpArrows,
  instructionPresentationMode = "ir",
  onInstructionPresentationModeChange,
  onToggleBookmark,
  onSetComment,
  flyoutLeafId,
  flyoutOpen,
  flyoutDismissed,
  extractLeafToFlyout,
  restoreFlyoutLeaf,
  setFlyoutOpen,
}: MosaicWorkspaceProps) {
  const store = useTcldData();
  const debug = useDebugStore();
  const setExecution = useLocationStore((s) => s.setExecution);
  const instructions = useMemo(() => store.data?.instructions ?? [], [store.data?.instructions]);

  // Debug execution state — correlate from stoppedAt
  const decompileLines = useMemo(() => store.data?.decompileLines ?? [], [store.data?.decompileLines]);
  const blocks = useMemo(() => store.data?.blocks ?? [], [store.data?.blocks]);

  const filesLikelyMatch = useCallback((a: string, b: string): boolean => {
    if (!a || !b) return false;
    const norm = (s: string) => s.replace(/\\/g, "/").toLowerCase();
    const na = norm(a);
    const nb = norm(b);
    if (na === nb) return true;
    const ta = na.split("/").pop();
    const tb = nb.split("/").pop();
    return !!ta && ta === tb;
  }, []);

  const executionIndex = useMemo(() => {
    if (!debug.stoppedAt) return null;
    if (
      debug.stoppedAt.file
      && debug.scriptPath
      && debug.stoppedAt.pc == null
      && !filesLikelyMatch(debug.stoppedAt.file, debug.scriptPath)
    ) {
      return null;
    }
    // Prefer bridge-provided value
    if (debug.stoppedAt.instructionIndex != null) return debug.stoppedAt.instructionIndex;
    if (debug.stoppedAt.pc != null) {
      const pcMatch = instructions.find((inst) => inst.pc === debug.stoppedAt?.pc);
      if (pcMatch) return pcMatch.idx;
    }
    // Line 0 means "at the very start" — find first instruction with a command group
    if (debug.stoppedAt.line === 0 && instructions.length > 0) {
      const firstGrouped = instructions.findIndex((i) => i.group != null);
      return firstGrouped >= 0 ? firstGrouped : 0;
    }
    // Fallback: correlate source line → decompile line → group → instruction
    if (debug.stoppedAt.line != null && debug.stoppedAt.line > 0 && decompileLines.length > 0) {
      const sourceLine = debug.stoppedAt.line;
      // Prefer command source-line span mapping from disassembly src offsets.
      for (const dl of decompileLines) {
        if (
          dl.group != null
          && dl.sourceLineStart != null
          && dl.sourceLineEnd != null
          && sourceLine >= dl.sourceLineStart
          && sourceLine <= dl.sourceLineEnd
        ) {
          const inst = instructions.find((i) => i.group === dl.group && i.groupStart);
          if (inst) return inst.idx;
        }
      }
      // Try matching decompile line by line number (1-indexed source → 0-indexed decompile)
      const matchingLine = decompileLines[sourceLine - 1];
      if (matchingLine?.group != null) {
        const inst = instructions.find((i) => i.group === matchingLine.group && i.groupStart);
        if (inst) return inst.idx;
      }
      // Broader search: find any decompile line with closest line number that has a group
      const searchRadius = Math.min(decompileLines.length, 20);
      for (let offset = 0; offset < searchRadius; offset++) {
        for (const dir of [0, -1, 1]) {
          const idx = sourceLine - 1 + dir * offset;
          if (idx >= 0 && idx < decompileLines.length && decompileLines[idx].group != null) {
            const inst = instructions.find((i) => i.group === decompileLines[idx].group && i.groupStart);
            if (inst) return inst.idx;
          }
        }
      }
    }
    // Last resort: first instruction with a command group
    const firstGrouped = instructions.findIndex((i) => i.group != null);
    if (firstGrouped >= 0) return firstGrouped;
    return null;
  }, [debug.stoppedAt, debug.scriptPath, decompileLines, instructions, filesLikelyMatch]);

  const executionBlockId = useMemo(() => {
    const stoppedBlockId = debug.stoppedAt?.blockId;
    if (stoppedBlockId != null) {
      const candidate = blocks.find((b) => b.id === stoppedBlockId);
      if (
        candidate
        && (
          executionIndex == null
          || (executionIndex >= candidate.start && executionIndex <= candidate.end)
        )
      ) {
        return candidate.id;
      }
    }
    if (executionIndex != null) {
      const block = blocks.find((b) => executionIndex >= b.start && executionIndex <= b.end);
      return block?.id ?? null;
    }
    return null;
  }, [debug.stoppedAt, executionIndex, blocks]);

  const executionGroup = useMemo(
    () => executionIndex != null ? (instructions[executionIndex]?.group ?? null) : null,
    [executionIndex, instructions],
  );
  const debugActive = debug.sessionState !== "idle" && debug.sessionState !== "terminated";

  useEffect(() => {
    if (!store.data || executionIndex == null) {
      setExecution(null);
      return;
    }

    const scopeId = computeScopeId(store.data.metadata);
    setExecution({
      scopeId,
      instIdx: executionIndex,
      group: executionGroup,
      blockId: executionBlockId,
      sourceFile: debug.stoppedAt?.file ?? null,
      sourceLine: debug.stoppedAt?.line ?? null,
    });
  }, [
    debug.stoppedAt?.file,
    debug.stoppedAt?.line,
    executionBlockId,
    executionGroup,
    executionIndex,
    setExecution,
    store.data,
  ]);

  // Map breakpoint lines → command groups (for InstructionPanel gutter)
  const breakpointGroups = useMemo(() => {
    if (!debug.breakpoints.length || !decompileLines.length) return new Set<number>();
    const groups = new Set<number>();
    for (const bp of debug.breakpoints) {
      if (bp.kind === "instruction" || bp.line == null) continue;
      if (!bp.enabled) continue;
      if (bp.offset != null) {
        for (const entry of store.data?.sourceMap ?? []) {
          if (
            entry.commandGroup != null
            && entry.originalSpan?.[0] === bp.offset
          ) {
            groups.add(entry.commandGroup);
          }
        }
        continue;
      }
      for (const dl of decompileLines) {
        if (
          dl.group != null
          && dl.sourceLineStart != null
          && dl.sourceLineEnd != null
          && bp.line >= dl.sourceLineStart
          && bp.line <= dl.sourceLineEnd
        ) {
          groups.add(dl.group);
        }
      }
      // bp.line is 1-indexed source line → decompileLines is 0-indexed
      const dl = decompileLines[bp.line - 1];
      if (dl?.group != null) groups.add(dl.group);
    }
    return groups;
  }, [debug.breakpoints, decompileLines, store.data?.sourceMap]);

  // Reverse-map: group → first decompile line number (1-indexed) for toggling
  const groupToLine = useMemo(() => {
    const map = new Map<number, number>();
    for (let i = 0; i < decompileLines.length; i++) {
      const g = decompileLines[i].group;
      if (g != null && !map.has(g)) map.set(g, i + 1); // 1-indexed
    }
    return map;
  }, [decompileLines]);

  const groupToCommand = useMemo(() => {
    const map = new Map<number, { line: number; offset: number; len: number }>();
    const source = store.data?.originalSourceText;
    if (!source) return map;
    for (const entry of store.data?.sourceMap ?? []) {
      const group = entry.commandGroup;
      const span = entry.originalSpan;
      if (group == null || span == null || map.has(group)) continue;
      map.set(group, {
        line: lineForUtf8ByteOffset(source, span[0]),
        offset: span[0],
        len: span[1] - span[0],
      });
    }
    return map;
  }, [store.data?.originalSourceText, store.data?.sourceMap]);

  const [activeDrag, setActiveDrag] = useState<DragDescriptor | null>(null);
  const dropZoneRef = useRef<DropZone>("center");

  const activeViewForLeaf = useCallback((leafId: WorkspaceLeafId | null): ViewId | null => {
    if (!leafId) {
      return null;
    }
    const stack = viewStacks[leafId];
    if (!stack || stack.views.length === 0) {
      return null;
    }
    if (stack.views.includes(stack.activeView)) {
      return stack.activeView;
    }
    return stack.views[0];
  }, [viewStacks]);

  // @dnd-kit: require 8px drag distance before activation (so header buttons still work)
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
  );

  const handleDragStart = useCallback((event: DragStartEvent) => {
    const data = event.active.data.current as Partial<DragDescriptor> | undefined;
    if (data?.kind === "leaf" && data.leafId) {
      setActiveDrag({
        kind: "leaf",
        leafId: data.leafId,
        activeViewId: data.activeViewId ?? activeViewForLeaf(data.leafId),
      });
    } else if (data?.kind === "tab" && data.leafId && data.viewId) {
      setActiveDrag({
        kind: "tab",
        leafId: data.leafId,
        viewId: data.viewId,
      });
    } else {
      const id = String(event.active.id);
      if (id.startsWith(LEAF_DRAG_PREFIX)) {
        const leafId = id.slice(LEAF_DRAG_PREFIX.length);
        setActiveDrag({
          kind: "leaf",
          leafId,
          activeViewId: activeViewForLeaf(leafId),
        });
      } else {
        setActiveDrag(null);
      }
    }
    dropZoneRef.current = "center";
  }, [activeViewForLeaf]);

  /**
   * Live reorder while a tab is being dragged over a sortable sibling.
   * dnd-kit/sortable triggers DragOverEvents whenever the dragged item
   * crosses the midpoint of another sortable item — we apply the reorder
   * immediately so the strip's tabs slide into their new resting places
   * (the FLIP animation comes from `transform/transition` on each tab).
   *
   * Cross-leaf moves are NOT handled here — those go through the normal
   * leaf-drop / tab-slot drop-target wiring in handleDragEnd. This handler
   * only acts when `over` is itself a sortable tab in the same leaf.
   */
  const handleDragOver = useCallback((event: DragOverEvent) => {
    const drag = activeDrag;
    if (!drag || drag.kind !== "tab") return;
    const overData = event.over?.data?.current as
      | { kind?: string; leafId?: WorkspaceLeafId; viewId?: ViewId }
      | undefined;
    if (overData?.kind !== "tab") return;
    if (!overData.leafId || !overData.viewId) return;
    if (overData.leafId !== drag.leafId) return; // cross-leaf handled in dragEnd
    if (overData.viewId === drag.viewId) return;

    // Find the target index in the live viewStacks order.
    const stack = viewStacks[drag.leafId];
    if (!stack) return;
    const overIndex = stack.views.indexOf(overData.viewId as ViewId);
    if (overIndex === -1) return;
    reorderTabInLeaf(drag.viewId, drag.leafId, drag.leafId, overIndex);
  }, [activeDrag, reorderTabInLeaf, viewStacks]);

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const zone = dropZoneRef.current;
      const drag = activeDrag;
      setActiveDrag(null);
      dropZoneRef.current = "center";

      const { over } = event;
      if (!over || !drag) {
        return;
      }

      const overId = String(over.id);
      const tabSlot = parseTabSlotId(overId);
      const targetLeafId = parseLeafDropId(overId);
      const windowEdge = parseWindowEdgeId(overId);

      // Tab-slot drops take precedence — they preserve insertion order
      // (used for in-strip reorder and cross-strip insert at a specific
      // position).
      if (tabSlot && drag.kind === "tab") {
        reorderTabInLeaf(drag.viewId, drag.leafId, tabSlot.leafId, tabSlot.index);
        return;
      }

      if (targetLeafId) {
        if (drag.kind === "leaf") {
          if (drag.leafId === targetLeafId) {
            return;
          }
          if (zone === "center") {
            stackLeafIntoTabs(drag.leafId, targetLeafId);
          } else {
            moveLeafToSplit(drag.leafId, targetLeafId, zone);
          }
        } else if (zone === "center") {
          stackViewIntoLeaf(drag.viewId, drag.leafId, targetLeafId);
        } else {
          splitViewIntoLeaf(drag.viewId, drag.leafId, targetLeafId, zone);
        }
        return;
      }

      if (windowEdge) {
        if (drag.kind === "leaf") {
          moveLeafToWindowEdge(drag.leafId, windowEdge);
        } else {
          splitViewToWindowEdge(drag.viewId, drag.leafId, windowEdge);
        }
      }
    },
    [
      activeDrag,
      moveLeafToSplit,
      moveLeafToWindowEdge,
      reorderTabInLeaf,
      splitViewIntoLeaf,
      splitViewToWindowEdge,
      stackLeafIntoTabs,
      stackViewIntoLeaf,
    ],
  );

  // Per-view badge map memoized on the precise count fields each badge reads.
  // Without this, every workspace render recomputed badges for every tab in
  // every leaf — counts like decompileLines.length and breakpoints.length
  // were called once per visible tab and once per render, even when nothing
  // had changed. This map is rebuilt only when one of the underlying counts
  // actually shifts.
  const tabBadgeMap = useMemo<Record<ViewId, string | number | null>>(() => {
    const map = {} as Record<ViewId, string | number | null>;
    for (const view of ALL_VIEW_IDS) {
      map[view] = tabBadgeForView(view, store.data, debug);
    }
    return map;
    // tabBadgeForView reads through `store.data` and `debug` for every view.
    // We could enumerate every count field separately, but those references
    // already turn over whenever the parent stores' identities change (Zustand
    // gives a new debug object on each update, and store.data is replaced
    // wholesale on file load). Keeping the deps coarse here is correct and
    // also lints clean.
  }, [store.data, debug]);

  const renderTile = (leafId: WorkspaceLeafId) => {
      const stack = viewStacks[leafId];
      const activeView = stack && stack.views.length > 0
        ? (stack.views.includes(stack.activeView) ? stack.activeView : stack.views[0])
        : null;

      if (!activeView) {
        return (
          <Panel
            id={leafId}
            title="Missing Panel"
            focused={false}
            onClose={() => undefined}
            onFocus={() => undefined}
          >
            <div className="flex items-center justify-center h-full text-tcld-fg-muted">
              Missing panel stack: {leafId}
            </div>
          </Panel>
        );
      }

      const focused = focusedViewId === activeView;
      const expanded = expandedViewId === leafId;
      const openViews = new Set(getOpenViews(mosaicNode, viewStacks));

      // Typed panel-header menu items. Add View / Replace View are submenus
      // listing every view that is currently eligible (not already open for
      // Add; not the active view and not open elsewhere for Replace).
      const addCandidates = VIEW_DEFINITIONS.filter((view) => view.id !== "source" && !openViews.has(view.id));
      const swapCandidates = VIEW_DEFINITIONS.filter((view) => view.id !== "source" && view.id !== activeView && !openViews.has(view.id));
      const buildViewItem = (
        viewId: ViewId,
        label: string,
        keyId: "add" | "swap",
        onPick: (id: ViewId) => void,
      ): PanelActionItem => ({
        id: `${keyId}-view-${viewId}`,
        label,
        icon: <ViewIcon viewId={viewId} size={12} />,
        onSelect: () => onPick(viewId),
      });
      const actionItems: PanelActionItem[] = [
        {
          id: "add-view",
          label: "Add view to panel",
          icon: <Plus size={12} />,
          submenu: addCandidates.map((view) =>
            buildViewItem(view.id, view.label, "add", (id) => addViewToLeaf(id, leafId))),
          submenuEmptyLabel: "All views are already open in this workspace.",
        },
        {
          id: "swap-view",
          label: "Replace this view",
          icon: <Replace size={12} />,
          submenu: swapCandidates.map((view) =>
            buildViewItem(view.id, view.label, "swap", (id) => swapViewInLeaf(activeView, id, leafId))),
          submenuEmptyLabel: "No other views available to swap in.",
        },
      ];

      const meta = {
        leafId,
        icon: <ViewIcon viewId={activeView} size={13} />,
        keyHint: VIEW_KEYS[activeView] || undefined,
        expanded,
        onExpand: toggleExpand,
        tabs: (stack?.views ?? []).map((view) => ({
          id: view,
          label: VIEW_TITLES[view],
          icon: <ViewIcon viewId={view} size={12} />,
          badge: tabBadgeMap[view],
          keyHint: VIEW_KEYS[view] || undefined,
        })),
        activeTabId: activeView,
        onSelectTab: (view: ViewId) => {
          setFocusedView(view);
        },
        onCloseTab: (view: ViewId) => {
          removeView(view);
        },
        actionItems,
      };

      // Per-view renderer table. Each entry takes the standard panel props
      // (id, focused, onClose, onFocus — applied uniformly below) and any
      // view-specific props the panel needs. Replaces the 200-line switch
      // statement that lived here previously; adding a new view is now one
      // entry, and the type signature of `panelRenderers` ensures every
      // ViewId has a renderer (Record<ViewId, ...>).
      const close = () => removeView(activeView);
      const focus = () => setFocusedView(activeView);
      const baseProps = { id: activeView, focused, onClose: close, onFocus: focus };
      const panelRenderers: Record<ViewId, () => React.ReactElement> = {
        instructions: () => (
          <InstructionPanel
            {...baseProps}
            onSelect={setSelected}
            presentationMode={instructionPresentationMode}
            onPresentationModeChange={onInstructionPresentationModeChange}
            annotations={annotations}
            showJumpArrows={showJumpArrows}
            onToggleBookmark={onToggleBookmark}
            onSetComment={onSetComment}
            onToggleJumpArrows={onToggleJumpArrows}
            debugActive={debugActive}
            breakpointGroups={breakpointGroups}
            onToggleBreakpointAtGroup={(group) => {
              const command = groupToCommand.get(group);
              if (command && debug.scriptPath) {
                debug.toggleBreakpoint(debug.scriptPath, command.line, command);
                return;
              }
              const line = groupToLine.get(group);
              if (line != null && debug.scriptPath) {
                debug.toggleBreakpoint(debug.scriptPath, line);
              }
            }}
            onRunToCursorAtGroup={(group) => {
              const command = groupToCommand.get(group);
              if (command && debug.scriptPath) {
                void debug.runToCursor(debug.scriptPath, command.line, command);
                return;
              }
              const line = groupToLine.get(group);
              if (line != null && debug.scriptPath) {
                void debug.runToCursor(debug.scriptPath, line);
              }
            }}
          />
        ),
        decompile: () => (
          <DecompilePanel
            {...baseProps}
            debugActive={debugActive}
            breakpoints={debug.breakpoints}
            onToggleBreakpoint={(line) => {
              // Prefer the active debug session's scriptPath. When no debug
              // session is running, fall back to the active document's
              // sourcePath so users can set breakpoints BEFORE launch —
              // they'll be replayed when the session starts.
              const file = debug.scriptPath || store.activeSourcePath;
              if (file) debug.toggleBreakpoint(file, line);
            }}
            onRemoveBreakpoint={(line) => {
              const file = debug.scriptPath || store.activeSourcePath;
              if (file) debug.clearBreakpoint(file, line);
            }}
            onRunToCursor={(line) => {
              if (debug.scriptPath) {
                void debug.runToCursor(debug.scriptPath, line);
              }
            }}
          />
        ),
        source: () => <SourcePanel {...baseProps} />,
        comparison: () => <ComparisonPanel {...baseProps} />,
        diagnostics: () => <DiagnosticsPanel {...baseProps} />,
        provenance: () => <ProvenancePanel {...baseProps} />,
        project: () => <ProjectPanel {...baseProps} />,
        detail: () => (
          <DetailPanel {...baseProps} />
        ),
        literals: () => <LiteralsPanel {...baseProps} />,
        exceptions: () => <ExceptionsPanel {...baseProps} />,
        graph: () => <GraphPanel {...baseProps} />,
        summary: () => <SummaryPanel {...baseProps} />,
        variables: () => <VariablesPanel {...baseProps} />,
        stack: () => <StackPanel {...baseProps} />,
        console: () => <ConsolePanel {...baseProps} />,
        breakpoints: () => <BreakpointsPanel {...baseProps} />,
        watchpoints: () => <WatchpointsPanel {...baseProps} />,
        commandResolver: () => <CommandResolverPanel {...baseProps} />,
      };
      const renderer = panelRenderers[activeView];
      const panelContent = renderer
        ? renderer()
        : (
          <Panel {...baseProps} title={activeView}>
            <div className="flex items-center justify-center h-full text-tcld-fg-muted">
              Unknown view: {activeView}
            </div>
          </Panel>
        );

      return (
        <DndTile
          leafId={leafId}
          activeDrag={activeDrag}
          dropZoneRef={dropZoneRef}
          minWidthPx={leafMinWidthPx(leafId, viewStacks)}
          meta={meta}
        >
          {panelContent}
        </DndTile>
      );
    };

  /**
   * Workspace container size, observed via ResizeObserver. Drives:
   *   - The minPx clamp in `handleChange` (a proposed tree must honor every
   *     leaf's minimum width given the current container size).
   *   - The `data-narrow="true"` attribute on the workspace root, which
   *     downstream CSS / future drawer logic can opt into when the viewport
   *     drops below `WORKSPACE_COLLAPSE_AT_PX`.
   */
  const workspaceRootRef = useRef<HTMLDivElement | null>(null);
  const [workspaceSize, setWorkspaceSize] = useState<{ width: number; height: number }>({
    width: 0,
    height: 0,
  });
  // Latest workspace size, mirrored into a ref so handleChange can read it
  // without re-creating its callback on every resize tick (which would force
  // Mosaic to re-mount its onChange listener and lose its drag state). The
  // mirror lives in a layout effect rather than a render-time assignment so
  // we don't violate React's "no ref writes during render" rule under
  // concurrent / strict mode.
  const workspaceSizeRef = useRef(workspaceSize);
  useEffect(() => {
    workspaceSizeRef.current = workspaceSize;
  }, [workspaceSize]);

  /*
   * Flyout auto-collapse / auto-restore is driven by the ResizeObserver
   * callback below (an external event source), not by a React effect that
   * reacts to `workspaceSize.width` changes. Driving from the observer
   * directly avoids the setState-in-effect cascade pattern: the callback
   * is one event, fires one round of state updates, React batches those
   * into one render. Hysteresis between WORKSPACE_COLLAPSE_AT_PX and
   * WORKSPACE_RESTORE_AT_PX (60px gap) stops the layout from flapping at
   * the threshold.
   *
   * `useEffectEvent` lets the observer's mount-only effect call the
   * decision function without listing `flyoutLeafId / mosaicNode /
   * viewStacks / …` as deps (which would re-bind the observer on every
   * panel resize). The event reads the latest render's closure, so it
   * always sees current state — no stale-closure bugs, no ref hack.
   */
  const onWorkspaceResize = useEffectEvent((width: number, height: number) => {
    setWorkspaceSize((prev) =>
      prev.width === width && prev.height === height
        ? prev
        : { width, height });
    if (width <= 0) return;
    if (
      width < WORKSPACE_COLLAPSE_AT_PX
      && !flyoutLeafId
      && !flyoutDismissed
    ) {
      if (!mosaicNode) return;
      const leaves = getLeafIds(mosaicNode);
      if (leaves.length <= 1) return;
      const preferred = findLeafHostingView(viewStacks, PREFERRED_FLYOUT_VIEW);
      const candidate = preferred && leaves.includes(preferred)
        ? preferred
        : findRightmostLeaf(mosaicNode);
      if (candidate) extractLeafToFlyout(candidate);
    } else if (
      width >= WORKSPACE_RESTORE_AT_PX
      && flyoutLeafId
    ) {
      restoreFlyoutLeaf();
    }
  });

  useEffect(() => {
    const node = workspaceRootRef.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        onWorkspaceResize(width, height);
      }
    });
    observer.observe(node);
    return () => observer.disconnect();
    // The observer is genuinely mount-only: `onWorkspaceResize` is a
    // useEffectEvent (stable identity, hook-injected by the React lint
    // plugin) so the exhaustive-deps rule does not require it in the
    // dep list.
  }, []);

  const isNarrow = workspaceSize.width > 0 && workspaceSize.width < WORKSPACE_COLLAPSE_AT_PX;

  // Manual flyout controls — thin wrappers so the drawer's close X and the
  // edge re-open tab can call setFlyoutOpen with the right argument.
  const closeFlyout = useCallback(() => setFlyoutOpen(false), [setFlyoutOpen]);
  const openFlyout = useCallback(() => setFlyoutOpen(true), [setFlyoutOpen]);

  /**
   * Mosaic onChange clamp. The Mosaic library only enforces a percentage
   * floor (`minimumPaneSizePercentage`) which is wrong for "the inspector
   * needs 280px to be useful." We re-validate every proposed tree against
   * per-leaf pixel floors and reject changes that would crush a panel below
   * its contract — the splitter feels like it stops at the floor.
   *
   * Two intentional escape hatches:
   *
   *   1. If the container itself is too narrow to honor every floor (sum of
   *      minPx > container width), we accept the change anyway. There's no
   *      "valid" tree to fall back to in that regime; better to let the
   *      panel render at its `min-width` floor and overflow horizontally
   *      than refuse all input.
   *
   *   2. Restructure changes (drag-to-split, tab moves) bypass the clamp.
   *      Those rewrite topology and we don't want to reject them just because
   *      the proposed split percentage produced an undersized leaf — the
   *      user can resize after. Detected via leaf-set difference: same
   *      leaves → resize change → clamp; different leaves → restructure →
   *      let it through.
   */
  const handleChange = useCallback(
    (newNode: WorkspaceTree | null) => {
      const { width, height } = workspaceSizeRef.current;
      // Restructure (different leaf set) bypasses the clamp; resize honors it.
      const sameLeafSet = (() => {
        const collect = (node: WorkspaceTree | null, out: Set<string>) => {
          if (!node) return;
          if (typeof node === "string") { out.add(node); return; }
          collect(node.first as WorkspaceTree, out);
          collect(node.second as WorkspaceTree, out);
        };
        const prev = new Set<string>();
        const next = new Set<string>();
        collect(mosaicNode, prev);
        collect(newNode, next);
        if (prev.size !== next.size) return false;
        for (const id of prev) if (!next.has(id)) return false;
        return true;
      })();
      if (sameLeafSet && width > 0) {
        if (!honorsLeafMinSizes(newNode, viewStacks, width, height)) {
          // Clamp violation — keep the previous tree.
          return;
        }
      }
      setMosaicNode(newNode);
    },
    [mosaicNode, setMosaicNode, viewStacks],
  );

  const activeDragViewId = useMemo(
    () => {
      if (!activeDrag) {
        return null;
      }
      if (activeDrag.kind === "tab") {
        return activeDrag.viewId;
      }
      return activeDrag.activeViewId ?? activeViewForLeaf(activeDrag.leafId);
    },
    [activeDrag, activeViewForLeaf],
  );

  const activeDragLeafId = useMemo(
    () => activeDrag?.leafId ?? null,
    [activeDrag],
  );

  return (
    <DndContext
      sensors={sensors}
      onDragStart={handleDragStart}
      onDragOver={handleDragOver}
      onDragEnd={handleDragEnd}
      onDragCancel={() => { setActiveDrag(null); dropZoneRef.current = "center"; }}
    >
      <div
        ref={workspaceRootRef}
        data-design-surface="workspace-mosaic"
        data-design-min-size="minSize={220}"
        data-layout-dirty="layoutDirty"
        data-save-layout="saveLayout"
        data-mosaic-resize-minimum-pane-size-percentage="20"
        data-narrow={isNarrow ? "true" : undefined}
        data-workspace-collapse-at={WORKSPACE_COLLAPSE_AT_PX}
        data-flyout-state={flyoutLeafId ? (flyoutOpen ? "open" : "closed") : undefined}
        style={{ position: "relative", height: "100%", minWidth: 220 }}
      >
        <Mosaic<WorkspaceLeafId>
          renderTile={renderTile}
          value={mosaicNode}
          onChange={handleChange}
          className="mosaic-tcld-theme"
          resize={{ minimumPaneSizePercentage: 20 }}
        />
        <WindowEdgeDrop edge="left" activeDrag={activeDrag} />
        <WindowEdgeDrop edge="right" activeDrag={activeDrag} />
        <WindowEdgeDrop edge="top" activeDrag={activeDrag} />
        <WindowEdgeDrop edge="bottom" activeDrag={activeDrag} />

        {/*
         * ─── Flyout drawer ────────────────────────────────────────
         * Pinned to the right edge, slides in when a leaf has been
         * extracted from the mosaic via the auto-collapse machinery.
         * Reuses the same renderTile function the mosaic uses, so the
         * panel inside the drawer behaves identically (including DnD —
         * the user can drag the panel header back into the mosaic to
         * un-collapse manually).
         *
         * Width: clamped to min(420, 90% of workspace width) so on very
         * narrow windows it doesn't fully cover the underlying mosaic.
         * Height: full workspace height minus 0 — the drawer sits inside
         * the workspace container, not over the title bar / status bar.
         *
         * The "reopen" affordance lives in the same overlay layer as a
         * small chevron tab on the right edge whenever the leaf is
         * extracted but the drawer is closed (user dismissed). Click to
         * re-open without having to find a separate menu entry.
         */}
        {flyoutLeafId && flyoutOpen && (
          <div
            role="complementary"
            aria-label="Workspace flyout"
            data-flyout-drawer="true"
            style={{
              position: "absolute",
              top: 0,
              right: 0,
              bottom: 0,
              width: `min(420px, 90%)`,
              zIndex: 30,
              background: T.bg2,
              borderLeft: `1px solid ${T.borderFocus}`,
              boxShadow: "var(--shadow-popover)",
              display: "flex",
              flexDirection: "column",
              animation: "flyout-slide-in 0.18s cubic-bezier(0.22, 1, 0.36, 1)",
            }}
          >
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "6px 10px",
                borderBottom: `1px solid ${T.borderDim}`,
                background: T.bg1,
                fontFamily: FONT_MONO,
                fontSize: 11,
                color: T.fgMuted,
                flexShrink: 0,
              }}
            >
              <PanelRightOpen size={12} aria-hidden style={{ color: T.accent }} />
              <span style={{ flex: 1, letterSpacing: "0.04em", textTransform: "uppercase" }}>
                Flyout
              </span>
              <button
                type="button"
                onClick={closeFlyout}
                aria-label="Close flyout"
                title="Close flyout (workspace stays narrow; click ▸ to reopen)"
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  justifyContent: "center",
                  width: 22,
                  height: 22,
                  border: 0,
                  borderRadius: 4,
                  background: "transparent",
                  color: T.fgMuted,
                  cursor: "pointer",
                }}
              >
                <X size={13} aria-hidden />
              </button>
            </div>
            <div style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
              {renderTile(flyoutLeafId)}
            </div>
          </div>
        )}

        {/* Edge re-open tab: surfaces only when a leaf has been extracted
         * but the user has closed the drawer. Provides a one-click path
         * back into the panel without needing the StatusBar toggle. */}
        {flyoutLeafId && !flyoutOpen && (
          <button
            type="button"
            onClick={openFlyout}
            aria-label="Open flyout"
            title="Open flyout"
            data-flyout-reopen="true"
            style={{
              position: "absolute",
              top: "50%",
              right: 0,
              transform: "translateY(-50%)",
              width: 18,
              height: 56,
              border: `1px solid ${T.borderDim}`,
              borderRight: 0,
              borderRadius: "4px 0 0 4px",
              background: T.bg1,
              color: T.accent,
              cursor: "pointer",
              zIndex: 30,
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              padding: 0,
              // Token: --shadow-panel mirrored as a left-edge drop. Avoids
              // hardcoding rgba so the flyout matches the panel elevation
              // used everywhere else in the workbench.
              boxShadow: "var(--shadow-panel)",
            }}
          >
            <PanelRightOpen size={13} aria-hidden />
          </button>
        )}
      </div>
      <DragOverlay dropAnimation={null}>
        {activeDragLeafId && activeDragViewId ? (
          <div
            style={{
              background: T.bg2,
              border: `1px solid ${T.accent}`,
              borderRadius: 4,
              padding: "6px 14px",
              fontFamily: FONT_MONO,
              fontSize: 12,
              color: T.accent,
              display: "flex",
              alignItems: "center",
              gap: 8,
              // Token: --shadow-overlay carries the canonical scrim shadow
              // for floating, full-bleed surfaces (drag overlay, command
              // palette). The accent ring is appended via the second box
              // so we keep parity with the canonical shadow string.
              boxShadow: `var(--shadow-overlay), 0 0 0 1px ${T.accent}`,
              whiteSpace: "nowrap",
              pointerEvents: "none",
            }}
          >
            <span style={{ opacity: 0.7 }}><ViewIcon viewId={activeDragViewId} size={13} /></span>
            <span style={{ fontWeight: 600 }}>{VIEW_TITLES[activeDragViewId]}</span>
            {(viewStacks[activeDragLeafId]?.views.length ?? 0) > 1 && (
              <span style={{ opacity: 0.7, fontSize: 11 }}>
                ({viewStacks[activeDragLeafId]!.views.length} tabs)
              </span>
            )}
          </div>
        ) : null}
      </DragOverlay>
    </DndContext>
  );
});
