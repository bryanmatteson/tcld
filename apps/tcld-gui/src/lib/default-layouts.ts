// Default Layout Presets
// Pre-configured workspace layouts using react-mosaic binary tree model.

import type { MosaicBranch, MosaicNode } from "react-mosaic-component";
import type {
  LayoutPreset,
  ViewId,
  ViewLayoutTree,
  ViewStackLeaf,
  ViewStacks,
  WorkspaceLeafId,
  WorkspaceLayoutSnapshot,
  WorkspaceTree,
} from "@/types";

// Default Tree

export function getDefaultTree(): ViewLayoutTree {
  return {
    direction: "row",
    first: "instructions",
    second: {
      direction: "row",
      first: "decompile",
      second: "detail",
      splitPercentage: 60,
    },
    splitPercentage: 30,
  };
}

export function getDefaultWorkspace(): WorkspaceLayoutSnapshot {
  return {
    tree: {
      direction: "row",
      first: "leaf-1",
      second: {
        direction: "row",
        first: "leaf-2",
        second: "leaf-3",
        splitPercentage: 60,
      },
      splitPercentage: 30,
    },
    stacks: {
      "leaf-1": {
        leafId: "leaf-1",
        views: ["instructions", "literals"],
        activeView: "instructions",
      },
      "leaf-2": {
        leafId: "leaf-2",
        views: ["decompile", "comparison"],
        activeView: "decompile",
      },
      "leaf-3": {
        leafId: "leaf-3",
        views: ["detail", "diagnostics"],
        activeView: "detail",
      },
    },
  };
}

function designWorkspace(stacks: ViewStacks, tree: WorkspaceTree): WorkspaceLayoutSnapshot {
  return { tree, stacks };
}

// Presets

export const LAYOUT_PRESETS: LayoutPreset[] = [
  {
    name: "default",
    label: "Default - disasm / source / inspect",
    tree: getDefaultTree(),
    workspace: getDefaultWorkspace(),
  },
  {
    name: "flow",
    label: "Flow focus - graph / source / inspect",
    tree: {
      direction: "row",
      first: "graph",
      second: {
        direction: "row",
        first: "decompile",
        second: "detail",
        splitPercentage: 55,
      },
      splitPercentage: 41,
    },
    workspace: designWorkspace(
      {
        "leaf-1": {
          leafId: "leaf-1",
          views: ["graph"],
          activeView: "graph",
        },
        "leaf-2": {
          leafId: "leaf-2",
          views: ["decompile"],
          activeView: "decompile",
        },
        "leaf-3": {
          leafId: "leaf-3",
          views: ["detail", "diagnostics"],
          activeView: "detail",
        },
      },
      {
        direction: "row",
        first: "leaf-1",
        second: {
          direction: "row",
          first: "leaf-2",
          second: "leaf-3",
          splitPercentage: 55,
        },
        splitPercentage: 41,
      },
    ),
  },
  {
    name: "trace",
    label: "Trace + console",
    tree: {
      direction: "row",
      first: "provenance",
      second: {
        direction: "column",
        first: "decompile",
        second: "console",
        splitPercentage: 58,
      },
      splitPercentage: 56,
    },
    workspace: designWorkspace(
      {
        "leaf-1": {
          leafId: "leaf-1",
          views: ["provenance"],
          activeView: "provenance",
        },
        "leaf-2": {
          leafId: "leaf-2",
          views: ["decompile"],
          activeView: "decompile",
        },
        "leaf-3": {
          leafId: "leaf-3",
          views: ["console"],
          activeView: "console",
        },
      },
      {
        direction: "row",
        first: "leaf-1",
        second: {
          direction: "column",
          first: "leaf-2",
          second: "leaf-3",
          splitPercentage: 58,
        },
        splitPercentage: 56,
      },
    ),
  },
  {
    name: "console",
    label: "Console focus - source / console",
    tree: {
      direction: "row",
      first: "decompile",
      second: "console",
      splitPercentage: 47,
    },
    workspace: designWorkspace(
      {
        "leaf-1": {
          leafId: "leaf-1",
          views: ["decompile"],
          activeView: "decompile",
        },
        "leaf-2": {
          leafId: "leaf-2",
          views: ["console"],
          activeView: "console",
        },
      },
      {
        direction: "row",
        first: "leaf-1",
        second: "leaf-2",
        splitPercentage: 47,
      },
    ),
  },
  {
    name: "symbols",
    label: "Symbols deep dive",
    tree: {
      direction: "row",
      first: "literals",
      second: {
        direction: "row",
        first: "decompile",
        second: "detail",
        splitPercentage: 58,
      },
      splitPercentage: 33,
    },
    workspace: designWorkspace(
      {
        "leaf-1": {
          leafId: "leaf-1",
          views: ["literals", "instructions"],
          activeView: "literals",
        },
        "leaf-2": {
          leafId: "leaf-2",
          views: ["decompile"],
          activeView: "decompile",
        },
        "leaf-3": {
          leafId: "leaf-3",
          views: ["detail", "diagnostics"],
          activeView: "detail",
        },
      },
      {
        direction: "row",
        first: "leaf-1",
        second: {
          direction: "row",
          first: "leaf-2",
          second: "leaf-3",
          splitPercentage: 58,
        },
        splitPercentage: 33,
      },
    ),
  },
  {
    name: "cockpit",
    label: "Cockpit - source / disasm / flow",
    tree: {
      direction: "row",
      first: {
        direction: "row",
        first: "instructions",
        second: "decompile",
        splitPercentage: 45,
      },
      second: "detail",
      splitPercentage: 78,
    },
    workspace: designWorkspace(
      {
        "leaf-1": {
          leafId: "leaf-1",
          views: ["instructions", "graph"],
          activeView: "instructions",
        },
        "leaf-2": {
          leafId: "leaf-2",
          views: ["decompile", "comparison"],
          activeView: "decompile",
        },
        "leaf-3": {
          leafId: "leaf-3",
          views: ["detail", "diagnostics"],
          activeView: "detail",
        },
      },
      {
        direction: "row",
        first: {
          direction: "row",
          first: "leaf-1",
          second: "leaf-2",
          splitPercentage: 45,
        },
        second: "leaf-3",
        splitPercentage: 78,
      },
    ),
  },
  {
    name: "runbook",
    label: "Runbook - source / debug rail / console",
    tree: {
      direction: "row",
      first: {
        direction: "row",
        first: "decompile",
        second: "instructions",
        splitPercentage: 58,
      },
      second: {
        direction: "column",
        first: "variables",
        second: "console",
        splitPercentage: 68,
      },
      splitPercentage: 68,
    },
    workspace: designWorkspace(
      {
        "leaf-1": {
          leafId: "leaf-1",
          views: ["decompile"],
          activeView: "decompile",
        },
        "leaf-2": {
          leafId: "leaf-2",
          views: ["instructions"],
          activeView: "instructions",
        },
        "leaf-3": {
          leafId: "leaf-3",
          views: ["variables", "stack", "breakpoints", "watchpoints"],
          activeView: "variables",
        },
        "leaf-4": {
          leafId: "leaf-4",
          views: ["console"],
          activeView: "console",
        },
      },
      {
        direction: "row",
        first: {
          direction: "row",
          first: "leaf-1",
          second: "leaf-2",
          splitPercentage: 58,
        },
        second: {
          direction: "column",
          first: "leaf-3",
          second: "leaf-4",
          splitPercentage: 68,
        },
        splitPercentage: 68,
      },
    ),
  },
  {
    name: "analysis",
    label: "Analysis",
    tree: {
      direction: "row",
      first: "instructions",
      second: {
        direction: "row",
        first: {
          direction: "column",
          first: "decompile",
          second: "graph",
          splitPercentage: 67,
        },
        second: {
          direction: "column",
          first: "detail",
          second: "literals",
          splitPercentage: 50,
        },
        splitPercentage: 50,
      },
      splitPercentage: 33,
    },
  },
  {
    name: "wide",
    label: "Wide",
    tree: {
      direction: "column",
      first: "instructions",
      second: {
        direction: "row",
        first: "decompile",
        second: "detail",
        splitPercentage: 50,
      },
      splitPercentage: 58,
    },
  },
  {
    name: "decompile",
    label: "Decompile Focus",
    tree: {
      direction: "row",
      first: "decompile",
      second: {
        direction: "column",
        first: "detail",
        second: "literals",
        splitPercentage: 50,
      },
      splitPercentage: 67,
    },
  },
  {
    name: "debug",
    label: "Debug",
    // tree is kept as a minimal fallback (workspaceFromViewTree ignores stacks)
    tree: {
      direction: "row",
      first: "decompile",
      second: "variables",
      splitPercentage: 65,
    },
    workspace: {
      tree: {
        direction: "row",
        first: "leaf-1",
        second: {
          direction: "column",
          first: "leaf-2",
          second: "leaf-3",
          splitPercentage: 55,
        },
        splitPercentage: 65,
      },
      stacks: {
        "leaf-1": {
          leafId: "leaf-1",
          views: ["decompile", "instructions"],
          activeView: "decompile",
        },
        "leaf-2": {
          leafId: "leaf-2",
          views: ["variables", "stack", "breakpoints"],
          activeView: "variables",
        },
        "leaf-3": {
          leafId: "leaf-3",
          views: ["console"],
          activeView: "console",
        },
      },
    },
  },
  {
    name: "overview",
    label: "Overview",
    tree: {
      direction: "column",
      first: {
        direction: "row",
        first: "instructions",
        second: "decompile",
        splitPercentage: 50,
      },
      second: {
        direction: "row",
        first: "literals",
        second: {
          direction: "row",
          first: "exceptions",
          second: "summary",
          splitPercentage: 50,
        },
        splitPercentage: 33,
      },
      splitPercentage: 50,
    },
  },
];

// Workspace Materialization

export function workspaceFromViewTree(tree: ViewLayoutTree | null): WorkspaceLayoutSnapshot {
  if (tree === null) {
    return { tree: null, stacks: {} };
  }

  let counter = 1;
  const stacks: ViewStacks = {};

  const convert = (node: MosaicNode<ViewId>): WorkspaceTree => {
    if (typeof node === "string") {
      const leafId = `leaf-${counter++}`;
      stacks[leafId] = {
        leafId,
        views: [node],
        activeView: node,
      };
      return leafId;
    }

    return {
      ...node,
      first: convert(node.first as MosaicNode<ViewId>),
      second: convert(node.second as MosaicNode<ViewId>),
    };
  };

  return {
    tree: convert(tree),
    stacks,
  };
}

export function createLeafId(stacks: ViewStacks): WorkspaceLeafId {
  let next = Object.keys(stacks).length + 1;
  while (stacks[`leaf-${next}`]) {
    next += 1;
  }
  return `leaf-${next}`;
}

export function cloneViewStacks(stacks: ViewStacks): ViewStacks {
  const cloned: ViewStacks = {};
  for (const [leafId, stack] of Object.entries(stacks)) {
    cloned[leafId] = {
      leafId,
      views: [...stack.views],
      activeView: stack.activeView,
    };
  }
  return cloned;
}

export function getLeafIds(tree: WorkspaceTree | null): WorkspaceLeafId[] {
  if (!tree) {
    return [];
  }
  if (typeof tree === "string") {
    return [tree];
  }
  return [
    ...getLeafIds(tree.first as WorkspaceTree),
    ...getLeafIds(tree.second as WorkspaceTree),
  ];
}

/**
 * Return the leaf that sits on the rightmost edge of the tree, or null for
 * a single-leaf / empty tree. Used by the flyout-collapse machinery to pick
 * a sensible default panel to promote when the workspace narrows: the
 * rightmost leaf is what the user is most likely to consider "secondary"
 * in a left-to-right reading order, and it matches the canonical inspector
 * placement in the default preset.
 *
 * Returning null when the tree is a bare leaf id lets the caller fall back
 * to "no flyout" instead of collapsing the only panel.
 */
export function findRightmostLeaf(tree: WorkspaceTree | null): WorkspaceLeafId | null {
  if (!tree) return null;
  if (typeof tree === "string") return null;
  const second = tree.second as WorkspaceTree;
  if (typeof second === "string") return second;
  return findRightmostLeaf(second);
}

/**
 * Locate the leaf in `stacks` that hosts `viewId`, or null if the view isn't
 * present anywhere. Distinct from `findLeafForView` (also defined in this
 * module further down) only in that it scans the *stacks* directly rather
 * than walking the tree — useful when the caller already knows the view
 * exists in the workspace and just wants to know which leaf owns it.
 */
export function findLeafHostingView(stacks: ViewStacks, viewId: ViewId): WorkspaceLeafId | null {
  for (const [leafId, stack] of Object.entries(stacks)) {
    if (stack.views.includes(viewId)) return leafId;
  }
  return null;
}

export function pruneStacksToTree(
  tree: WorkspaceTree | null,
  stacks: ViewStacks,
  options: { preserveLeafIds?: Iterable<WorkspaceLeafId> } = {},
): ViewStacks {
  const leaves = new Set(getLeafIds(tree));
  // Caller-supplied preserve set: leaves that aren't in the tree but should
  // keep their stacks alive anyway. The flyout uses this — its parked leaf
  // is intentionally absent from `mosaicNode` but must keep its view stack
  // around so the drawer can render the panel and so we can restore the
  // leaf to the mosaic when the workspace widens. Without this, every
  // setMosaicNode call would orphan the flyout leaf and the renderer would
  // fall back to "Missing panel stack: …".
  if (options.preserveLeafIds) {
    for (const id of options.preserveLeafIds) leaves.add(id);
  }
  const pruned: ViewStacks = {};
  for (const leafId of leaves) {
    const stack = stacks[leafId];
    if (!stack || stack.views.length === 0) {
      continue;
    }
    const activeView = stack.views.includes(stack.activeView)
      ? stack.activeView
      : stack.views[0];
    pruned[leafId] = {
      leafId,
      views: [...stack.views],
      activeView,
    };
  }
  return pruned;
}

export function findLeafForView(stacks: ViewStacks, viewId: ViewId): WorkspaceLeafId | null {
  for (const [leafId, stack] of Object.entries(stacks)) {
    if (stack.views.includes(viewId)) {
      return leafId;
    }
  }
  return null;
}

export function getOpenViews(tree: WorkspaceTree | null, stacks: ViewStacks): ViewId[] {
  const seen = new Set<ViewId>();
  const ordered: ViewId[] = [];
  for (const leafId of getLeafIds(tree)) {
    const stack = stacks[leafId];
    if (!stack) {
      continue;
    }
    for (const view of stack.views) {
      if (seen.has(view)) {
        continue;
      }
      seen.add(view);
      ordered.push(view);
    }
  }
  return ordered;
}

// Tree Manipulation Helpers

/**
 * Smart add: split the target leaf to insert a new leaf beside it.
 * Preserves every existing split percentage in the tree.
 */
export function addToTree(
  currentTree: WorkspaceTree | null,
  newLeafId: WorkspaceLeafId,
  targetLeafId?: WorkspaceLeafId | null,
): WorkspaceTree {
  if (currentTree === null) {
    return newLeafId;
  }

  const leaves = getLeafIds(currentTree);
  const target = targetLeafId && leaves.includes(targetLeafId)
    ? targetLeafId
    : leaves[0];

  return splitAtLeaf(currentTree, target, newLeafId);
}

/**
 * Smart remove: prune the leaf and promote its sibling.
 * Preserves all split percentages except the removed node's parent.
 */
export function removeFromTree(
  tree: WorkspaceTree | null,
  leafId: WorkspaceLeafId,
): WorkspaceTree | null {
  if (tree === null) {
    return null;
  }
  return pruneLeaf(tree, leafId);
}

export type SplitZone = "left" | "right" | "top" | "bottom";

export function findPathToLeaf(
  tree: WorkspaceTree | null,
  target: WorkspaceLeafId,
): MosaicBranch[] | null {
  if (tree === null || tree === undefined) {
    return null;
  }
  if (typeof tree === "string") {
    return tree === target ? [] : null;
  }
  const firstPath = findPathToLeaf(tree.first as WorkspaceTree, target);
  if (firstPath !== null) {
    return ["first", ...firstPath];
  }
  const secondPath = findPathToLeaf(tree.second as WorkspaceTree, target);
  if (secondPath !== null) {
    return ["second", ...secondPath];
  }
  return null;
}

export function getNodeAtPath(
  tree: WorkspaceTree,
  path: MosaicBranch[],
): WorkspaceTree {
  let node: WorkspaceTree = tree;
  for (const branch of path) {
    node = (node as { first: WorkspaceTree; second: WorkspaceTree })[branch];
  }
  return node;
}

export function setNodeAtPath(
  tree: WorkspaceTree,
  path: MosaicBranch[],
  value: WorkspaceTree,
): WorkspaceTree {
  if (path.length === 0) {
    return value;
  }
  const [head, ...rest] = path;
  const parent = { ...(tree as { first: WorkspaceTree; second: WorkspaceTree }) };
  parent[head] = setNodeAtPath(parent[head], rest, value);
  return parent as WorkspaceTree;
}

export function insertLeafAtPathEdge(
  tree: WorkspaceTree,
  targetPath: MosaicBranch[],
  newLeafId: WorkspaceLeafId,
  zone: SplitZone,
): WorkspaceTree {
  const targetNode = getNodeAtPath(tree, targetPath);
  const splits: Record<SplitZone, WorkspaceTree> = {
    left: {
      direction: "row",
      first: newLeafId,
      second: targetNode,
      splitPercentage: 50,
    },
    right: {
      direction: "row",
      first: targetNode,
      second: newLeafId,
      splitPercentage: 50,
    },
    top: {
      direction: "column",
      first: newLeafId,
      second: targetNode,
      splitPercentage: 50,
    },
    bottom: {
      direction: "column",
      first: targetNode,
      second: newLeafId,
      splitPercentage: 50,
    },
  };
  return setNodeAtPath(tree, targetPath, splits[zone]);
}

export function wrapTreeAtEdge(
  tree: WorkspaceTree | null,
  newLeafId: WorkspaceLeafId,
  zone: SplitZone,
): WorkspaceTree {
  if (tree === null) {
    return newLeafId;
  }
  switch (zone) {
    case "left":
      return {
        direction: "row",
        first: newLeafId,
        second: tree,
        splitPercentage: 30,
      };
    case "right":
      return {
        direction: "row",
        first: tree,
        second: newLeafId,
        splitPercentage: 70,
      };
    case "top":
      return {
        direction: "column",
        first: newLeafId,
        second: tree,
        splitPercentage: 30,
      };
    case "bottom":
      return {
        direction: "column",
        first: tree,
        second: newLeafId,
        splitPercentage: 70,
      };
    default:
      return tree;
  }
}

// Internal Tree Surgery

/** Replace `targetLeaf` with a split containing [targetLeaf, newLeaf]. */
function splitAtLeaf(
  tree: WorkspaceTree,
  targetLeaf: WorkspaceLeafId,
  newLeaf: WorkspaceLeafId,
): WorkspaceTree {
  if (typeof tree === "string") {
    if (tree === targetLeaf) {
      return {
        direction: "row",
        first: tree,
        second: newLeaf,
        splitPercentage: 50,
      };
    }
    return tree;
  }
  return {
    ...tree,
    first: splitAtLeaf(tree.first as WorkspaceTree, targetLeaf, newLeaf),
    second: splitAtLeaf(tree.second as WorkspaceTree, targetLeaf, newLeaf),
  };
}

/** Remove `leafId` from the tree, promoting its sibling in place. */
function pruneLeaf(
  tree: WorkspaceTree,
  leafId: WorkspaceLeafId,
): WorkspaceTree | null {
  if (typeof tree === "string") {
    return tree === leafId ? null : tree;
  }
  const first = pruneLeaf(tree.first as WorkspaceTree, leafId);
  const second = pruneLeaf(tree.second as WorkspaceTree, leafId);
  if (first === null) {
    return second;
  }
  if (second === null) {
    return first;
  }
  return { ...tree, first, second };
}

export function normalizeLeafStack(stack: ViewStackLeaf): ViewStackLeaf | null {
  if (stack.views.length === 0) {
    return null;
  }
  const deduped: ViewId[] = [];
  const seen = new Set<ViewId>();
  for (const view of stack.views) {
    if (seen.has(view)) {
      continue;
    }
    seen.add(view);
    deduped.push(view);
  }
  if (deduped.length === 0) {
    return null;
  }
  return {
    leafId: stack.leafId,
    views: deduped,
    activeView: deduped.includes(stack.activeView) ? stack.activeView : deduped[0],
  };
}
