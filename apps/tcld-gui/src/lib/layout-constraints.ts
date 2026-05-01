"use client";

import type {
  ViewId,
  ViewStacks,
  WorkspaceLeafId,
  WorkspaceTree,
} from "@/types";

/**
 * ─── Layout constraints ────────────────────────────────────────
 *
 * Per-view minimum widths. The mosaic library only enforces a percentage-
 * based floor (`minimumPaneSizePercentage`), which is the right primitive
 * for "no pane shall vanish" but the wrong one for "the inspector needs
 * 280px to be useful." A pane at 22% of a 700px window is 154px — visually
 * present, but every column is truncated. Pixel floors prevent that.
 *
 * The values are heuristic floors below which the panel's primary content
 * (column-rich tables, monospaced code rows, multi-tab strips) starts to
 * clip or wrap unrecognizably. They're enforced in `MosaicWorkspace`'s
 * `onChange`: a proposed tree that would shrink any leaf below its floor is
 * rejected, so user resize gestures feel like the splitter "stops" at the
 * minimum rather than allowing a degenerate layout.
 *
 * Heights are not constrained today: every panel scrolls vertically, so a
 * short panel is annoying but not broken. If we add fixed-height panels
 * later, extend the registry to carry both.
 */
export const VIEW_MIN_WIDTH_PX: Record<ViewId, number> = {
  // Code-dense panels with multi-column rows: address gutter + opcode
  // + operands + comments. Below 240px the comments column collapses and
  // the panel reads as "address noise" rather than disassembly.
  instructions: 240,
  // Source view rendered with line numbers + fold rail + breakpoint gutter
  // + the source text itself. 320px covers an 80-col-equivalent layout.
  decompile: 320,
  // Inspector tabs + sticky identity block + field grid: 280px keeps the
  // labels readable without truncation.
  detail: 280,
  // Source map listings: less column-rich than disassembly, smaller floor.
  source: 240,
  // Two-column compare with rails — one per side.
  comparison: 280,
  // Diagnostics issues: severity pill + message + meta line.
  diagnostics: 240,
  // Provenance pass list with status badges.
  provenance: 240,
  // Project tree: indented entries.
  project: 200,
  // Summary report; rich text, narrowest-friendly of the code panels.
  summary: 240,
  // Literals view (preview list).
  literals: 200,
  // Exception entries — narrow.
  exceptions: 240,
  // Control-flow graph: needs canvas room or labels become illegible.
  graph: 320,
  // Variables / Stack: scope filter chips + name/value rows.
  variables: 220,
  stack: 220,
  // Console output is monospaced; 280px is the floor for readable command
  // lines without aggressive wrap.
  console: 280,
  // Breakpoints / watchpoints — list panels.
  breakpoints: 220,
  watchpoints: 220,
  // Command resolver — single-column key/value.
  commandResolver: 240,
};

/** Fallback floor for any view not enumerated above (or for empty leaves
 *  during tree restructure). Matches the historical `minWidth: 220` value
 *  enforced inline in `DndTile`. */
export const DEFAULT_VIEW_MIN_WIDTH_PX = 220;

/**
 * Workspace-level breakpoint below which the layout is considered "narrow"
 * and the workspace root carries `data-narrow="true"`. Downstream CSS or
 * future drawer-collapse logic can opt into responsive behavior off this
 * single attribute. Threshold matches the review's 1280px target — wide
 * enough that a 3-pane default preset (240 + 320 + 280 + chrome ≈ 1100)
 * still has breathing room; narrower windows trigger the narrow contract.
 */
export const WORKSPACE_COLLAPSE_AT_PX = 1280;

/**
 * Hysteresis for flyout auto-collapse. We collapse a leaf out of the mosaic
 * when the workspace shrinks below WORKSPACE_COLLAPSE_AT_PX, and we restore
 * it only when the workspace grows past WORKSPACE_RESTORE_AT_PX. The 60px
 * gap stops the layout from flapping back and forth as the user drags the
 * window edge across the threshold.
 */
export const WORKSPACE_RESTORE_AT_PX = WORKSPACE_COLLAPSE_AT_PX + 60;

/**
 * The view that, when present in the workspace, is preferred for
 * promotion into the flyout drawer when the window goes narrow. Defaults
 * to the inspector — it's the typical "secondary reference" panel and the
 * one the user can most easily live without seeing continuously while the
 * source / disassembly remain in the mosaic.
 *
 * If `PREFERRED_FLYOUT_VIEW` isn't currently in the workspace, the flyout
 * falls back to the rightmost leaf in the tree.
 */
export const PREFERRED_FLYOUT_VIEW: ViewId = "detail";

/**
 * Effective minPx for a leaf = max of minPx across its open views. When
 * tabs in a leaf share the panel, the leaf must accommodate the widest of
 * them — the user shouldn't have to resize when they switch tabs.
 */
export function leafMinWidthPx(
  leafId: WorkspaceLeafId,
  stacks: ViewStacks,
): number {
  const stack = stacks[leafId];
  if (!stack) return DEFAULT_VIEW_MIN_WIDTH_PX;
  let max = DEFAULT_VIEW_MIN_WIDTH_PX;
  for (const view of stack.views) {
    const px = VIEW_MIN_WIDTH_PX[view] ?? DEFAULT_VIEW_MIN_WIDTH_PX;
    if (px > max) max = px;
  }
  return max;
}

/**
 * Walk the tree top-down, computing the available pixel size for each leaf
 * based on its parent's split direction + percentage. Returns a map keyed
 * by leafId. Used to validate proposed onChange trees against per-leaf
 * minimum widths before accepting them into state.
 *
 * Mosaic split percentage semantics:
 *   - `direction: "row"` → horizontal split, `first` on LEFT, `second` on
 *     RIGHT. `splitPercentage` is the LEFT pane's share of width.
 *   - `direction: "column"` → vertical split, `first` on TOP, `second` on
 *     BOTTOM. `splitPercentage` is the TOP pane's share of height.
 *   - Missing percentage defaults to 50 (Mosaic library convention).
 *
 * The helper does not subtract gutter / divider widths — the divider is a
 * few pixels and the constraint is a heuristic floor anyway, so the small
 * over-allocation just makes the floor slightly more lenient than ideal.
 */
export function computeLeafSizes(
  tree: WorkspaceTree | null,
  containerWidth: number,
  containerHeight: number,
): Map<WorkspaceLeafId, { width: number; height: number }> {
  const sizes = new Map<WorkspaceLeafId, { width: number; height: number }>();
  if (!tree) return sizes;
  walk(tree, containerWidth, containerHeight, sizes);
  return sizes;
}

function walk(
  node: WorkspaceTree,
  width: number,
  height: number,
  out: Map<WorkspaceLeafId, { width: number; height: number }>,
): void {
  if (typeof node === "string") {
    out.set(node, { width, height });
    return;
  }
  const pct = node.splitPercentage ?? 50;
  const firstFraction = Math.max(0, Math.min(100, pct)) / 100;
  if (node.direction === "row") {
    walk(node.first as WorkspaceTree, width * firstFraction, height, out);
    walk(node.second as WorkspaceTree, width * (1 - firstFraction), height, out);
  } else {
    walk(node.first as WorkspaceTree, width, height * firstFraction, out);
    walk(node.second as WorkspaceTree, width, height * (1 - firstFraction), out);
  }
}

/**
 * True if every leaf in `tree` has at least its `leafMinWidthPx` worth of
 * width, given the container size. Returns false if any leaf is under-
 * provisioned. Used as a "shall I accept this onChange" predicate in the
 * MosaicWorkspace resize handler.
 *
 * Edge cases:
 *   - `containerWidth <= 0`: returns true (no constraint can be enforced
 *     against a zero-sized container; let the change through and trust
 *     the next resize observation to bring the constraint back online).
 *   - A leaf with no entry in `stacks`: falls back to DEFAULT_VIEW_MIN_WIDTH_PX
 *     via `leafMinWidthPx`. Avoids accepting trees with orphaned leaves.
 */
export function honorsLeafMinSizes(
  tree: WorkspaceTree | null,
  stacks: ViewStacks,
  containerWidth: number,
  containerHeight: number,
): boolean {
  if (containerWidth <= 0) return true;
  const sizes = computeLeafSizes(tree, containerWidth, containerHeight);
  for (const [leafId, size] of sizes) {
    if (size.width < leafMinWidthPx(leafId, stacks)) return false;
  }
  return true;
}
