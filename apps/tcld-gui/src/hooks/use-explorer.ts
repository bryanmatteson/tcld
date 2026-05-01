"use client";

import { useState, useCallback, useEffect, useRef, useMemo } from "react";
import type {
  ViewId,
  ViewStacks,
  WorkspaceLayoutSnapshot,
  WorkspaceLeafId,
  WorkspaceTree,
} from "@/types";
import { useTcldStore } from "@/lib/store";
import { openFileDialog } from "@/lib/tauri-api";
import {
  cloneViewStacks,
  createLeafId,
  findLeafForView,
  findPathToLeaf,
  getDefaultWorkspace,
  addToTree,
  insertLeafAtPathEdge,
  removeFromTree,
  getOpenViews,
  pruneStacksToTree,
  type SplitZone,
  wrapTreeAtEdge,
  workspaceFromViewTree,
  LAYOUT_PRESETS,
} from "@/lib/default-layouts";
import { saveCurrentLayout, loadCurrentLayout, saveNamedLayout, loadNamedLayouts } from "@/lib/layout-storage";
import { addRecentFile } from "@/lib/recent-files";
import type { Annotations } from "@/lib/annotations";
import { loadAnnotations, saveAnnotations, toggleBookmark, setComment, EMPTY_ANNOTATIONS } from "@/lib/annotations";
import { useLocationStore } from "@/lib/location-store";
import { buildLocationIndex, computeScopeId } from "@/lib/location-index";
import { parseOmniboxQuery, type OmniboxResult } from "@/lib/omnibox";
import { isViewId } from "@/lib/view-registry";

// ─── State ───────────────────────────────────────────────────────

export interface ExplorerState {
  // Mosaic tree layout
  mosaicNode: WorkspaceTree | null;
  viewStacks: ViewStacks;
  focusedViewId: ViewId | null;

  // Expand / maximize
  expandedViewId: WorkspaceLeafId | null;
  savedWorkspaceBeforeExpand: WorkspaceLayoutSnapshot | null;

  // Instruction selection (shared across panels)
  selectedIndex: number;

  // Feature toggles
  trace: boolean;
  showJumpArrows: boolean;

  // Annotations
  annotations: Annotations;

  // Overlays
  showPalette: boolean;
  showHelp: boolean;
  showCommand: boolean;
  commandValue: string;

  // Status
  statusMessage: string;
  layoutDirty: boolean;

  // Layout preset tracking — id of the most recently loaded preset / saved
  // layout (`null` for the default workspace or after the user has dirtied
  // a preset enough that we no longer want to claim a preset is "active").
  activePresetId: string | null;
  activePresetLabel: string | null;

  /**
   * Flyout state. When the workspace narrows past WORKSPACE_COLLAPSE_AT_PX,
   * one leaf is extracted from `mosaicNode` and parked here. The leaf's
   * stack stays alive in `viewStacks` (prune passes preserve it via
   * `flyoutLeafId`), so the panel renderer still resolves it. The drawer is
   * rendered as an overlay on the right edge.
   *
   *   - `flyoutLeafId`     — null when no leaf is extracted; the leaf id
   *                          when the leaf is parked in the drawer.
   *   - `flyoutSavedTree`  — the mosaic tree at extract time; replayed when
   *                          the workspace widens past the restore threshold.
   *   - `flyoutOpen`       — drawer visibility. The user can close the
   *                          drawer manually; the leaf stays extracted and
   *                          the edge re-open tab appears.
   *   - `flyoutDismissed`  — set when the user explicitly closes the drawer.
   *                          Stops the auto-collapse from re-promoting on
   *                          the next narrow tick (until they widen and the
   *                          flyout state is cleared, or they manually open).
   */
  flyoutLeafId: WorkspaceLeafId | null;
  flyoutSavedTree: WorkspaceTree | null;
  flyoutOpen: boolean;
  flyoutDismissed: boolean;
}

const defaultWorkspace = getDefaultWorkspace();
const defaultOpenViews = getOpenViews(defaultWorkspace.tree, defaultWorkspace.stacks);

const INITIAL_STATE: ExplorerState = {
  mosaicNode: defaultWorkspace.tree,
  viewStacks: defaultWorkspace.stacks,
  focusedViewId: defaultOpenViews[0] ?? null,
  expandedViewId: null,
  savedWorkspaceBeforeExpand: null,
  selectedIndex: 0,
  trace: false,
  showJumpArrows: true,
  annotations: EMPTY_ANNOTATIONS,
  showPalette: false,
  showHelp: false,
  showCommand: false,
  commandValue: "",
  statusMessage: "Open a file to begin (\u2318O)",
  layoutDirty: false,
  activePresetId: null,
  activePresetLabel: null,
  flyoutLeafId: null,
  flyoutSavedTree: null,
  flyoutOpen: false,
  flyoutDismissed: false,
};

function normalizeDesignWorkspace(snapshot: WorkspaceLayoutSnapshot): WorkspaceLayoutSnapshot {
  const stacks = cloneViewStacks(snapshot.stacks);
  let changed = false;
  for (const [leafId, stack] of Object.entries(stacks)) {
    if (!stack.views.includes("source") || stack.views.length <= 1) {
      continue;
    }
    const views = stack.views.filter((view) => view !== "source");
    stacks[leafId] = {
      ...stack,
      views,
      activeView: stack.activeView === "source"
        ? (views.includes("decompile") ? "decompile" : views[0])
        : stack.activeView,
    };
    changed = true;
  }
  if (!changed) return snapshot;
  return {
    tree: snapshot.tree,
    stacks: pruneStacksToTree(snapshot.tree, stacks),
  };
}

// ─── Hook ────────────────────────────────────────────────────────

export function useExplorer() {
  const store = useTcldStore();
  const {
    setIndex: setLocationIndex,
    index: locationIndex,
    jumpToInstruction: jumpLocationInstruction,
    cursor,
    goBack: goBackLocation,
    goForward: goForwardLocation,
  } = useLocationStore();

  const [state, setState] = useState<ExplorerState>(INITIAL_STATE);

  // Restore saved layout from localStorage after hydration
  const layoutRestoredRef = useRef(false);
  useEffect(() => {
    if (layoutRestoredRef.current) return;
    layoutRestoredRef.current = true;
    const saved = loadCurrentLayout();
    if (saved !== null) {
      const normalized = normalizeDesignWorkspace(saved);
      const views = getOpenViews(normalized.tree, normalized.stacks);
      const timer = window.setTimeout(() => {
        setState((s) => ({
          ...s,
          mosaicNode: normalized.tree,
          viewStacks: normalized.stacks,
          focusedViewId: views[0] ?? null,
          layoutDirty: false,
        }));
      }, 0);
      return () => window.clearTimeout(timer);
    }
  }, []);

  const instructionsRef = useRef(store.data?.instructions ?? []);
  useEffect(() => {
    instructionsRef.current = store.data?.instructions ?? [];
  }, [store.data?.instructions]);

  useEffect(() => {
    if (cursor.instIdx == null) return;
    const insts = instructionsRef.current;
    const max = Math.max(0, insts.length - 1);
    const clamped = Math.max(0, Math.min(cursor.instIdx, max));

    setState((s) => {
      if (s.selectedIndex === clamped) return s;
      return { ...s, selectedIndex: clamped };
    });
  }, [cursor.instIdx]);

  // Location index is republished into the global location store every time
  // store.data identity changes. The useEffect dependency on `store.data`
  // already guards against per-render recomputes — but a reload of the same
  // file (or any code path that hands us a freshly-cloned data object with
  // identical content) would still rebuild O(n) Int32Arrays for nothing.
  // The scope-id ref short-circuits the rebuild when the addressable shape
  // of the file (filename + instruction count + code bytes) hasn't changed.
  const lastIndexedScopeIdRef = useRef<string | null>(null);
  useEffect(() => {
    if (!store.data) {
      setLocationIndex(null);
      lastIndexedScopeIdRef.current = null;
      return;
    }
    const scopeId = computeScopeId(store.data.metadata);
    if (lastIndexedScopeIdRef.current === scopeId) return;
    const index = buildLocationIndex(
      store.data.instructions,
      store.data.blocks,
      store.data.decompileLines,
      scopeId,
    );
    setLocationIndex(index);
    lastIndexedScopeIdRef.current = scopeId;
  }, [setLocationIndex, store.data]);

  // ─── Persist layout on changes ─────────────────────────────────

  useEffect(() => {
    saveCurrentLayout({
      tree: state.mosaicNode,
      stacks: state.viewStacks,
    });
  }, [state.mosaicNode, state.viewStacks]);

  // ─── Persist annotations on changes ──────────────────────────────

  useEffect(() => {
    if (store.data) {
      const meta = store.data.metadata;
      saveAnnotations(meta.filename, meta.instructionCount, meta.codeBytes, state.annotations);
    }
  }, [state.annotations, store.data]);

  // ─── Update status when file loads ─────────────────────────────

  /*
   * \u2500\u2500\u2500 File-load hydration \u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500
   *
   * When the active store file changes we need to rehydrate three pieces
   * of explorer state from the new file's metadata:
   *   - `selectedIndex` \u2014 restored from the session document's saved
   *     selection (or 0 for a fresh document).
   *   - `annotations`   \u2014 bookmarks + comments live in localStorage keyed
   *     by `(filename, instructionCount, codeBytes)`.
   *   - `statusMessage` \u2014 "loaded foo.tbc \u2014 8421 instructions".
   *
   * The previous implementation did this in a `useEffect` that called
   * `setState` after the file-load render committed \u2014 which (a) cascaded
   * a second render the user could see flash through, and (b) tripped
   * react-hooks/exhaustive-deps' setState-in-effect guard. The proper
   * pattern is React's "adjust state during render" \u2014 we keep a snapshot
   * of the most recent file-load key in `useState`; when it diverges from
   * the current key we call `setState` *during render*. React discards
   * the in-flight render output and restarts with the new state, so there
   * is no visible second render and no effect\u2192setState\u2192effect cascade.
   *
   * Loading/error transitions only update statusMessage and don't
   * participate in the file-load key dance, so they're handled separately.
   */
  type LoadKey = string;
  const computeLoadKey = (): LoadKey | null => {
    if (!store.loaded || !store.data) return null;
    const meta = store.data.metadata;
    const docId = store.session.activeDocumentId ?? "single";
    return `${docId}:${meta.filename}:${meta.instructionCount}:${meta.codeBytes}`;
  };
  const currentLoadKey = computeLoadKey();
  const [hydratedLoadKey, setHydratedLoadKey] = useState<LoadKey | null>(null);
  if (currentLoadKey !== null && currentLoadKey !== hydratedLoadKey) {
    // Resolve hydration values from synchronous sources (localStorage +
    // the session document snapshot) at the same render-time tick that
    // we'll commit them in. `loadAnnotations` is a deterministic localStorage
    // read; `selectedInst` is a property lookup. Doing this work here lets
    // the second render React schedules carry the new state directly.
    const meta = store.data!.metadata;
    const loaded = loadAnnotations(meta.filename, meta.instructionCount, meta.codeBytes);
    const activeDocument = store.session.documents.find((doc) => doc.id === store.session.activeDocumentId);
    const selectedInst = activeDocument?.workspaceState.selectedInst ?? 0;
    setHydratedLoadKey(currentLoadKey);
    setState((s) => ({
      ...s,
      selectedIndex: selectedInst,
      annotations: loaded,
      statusMessage: `loaded ${meta.filename} \u2014 ${meta.instructionCount} instructions`,
    }));
    // jumpLocationInstruction touches an *external* store (location-store),
    // so we shouldn't dispatch into it during render. Microtask defer
    // sequences it after the current render commits \u2014 by then, the state
    // updates above have also committed, so the location store sees a
    // consistent world.
    void Promise.resolve().then(() => {
      jumpLocationInstruction(selectedInst, { source: "history", pushHistory: false });
    });
  } else if (currentLoadKey === null && hydratedLoadKey !== null) {
    // The store cleared (file closed). Reset the hydration snapshot so
    // the next file load triggers a fresh hydration even if it produces
    // the same key (rare but possible \u2014 e.g. close-then-reopen of the
    // same file).
    setHydratedLoadKey(null);
  }

  /*
   * Loading / error status transitions. These only affect statusMessage
   * (no file-load hydration), so the cascading-render concern is much
   * smaller \u2014 but we apply the same render-time adjustment pattern for
   * consistency. The status snapshot tracks "what state did we last
   * announce?" so we don't re-announce on every render.
   */
  type StatusKey = string;
  const currentStatusKey: StatusKey | null = store.error
    ? `error:${store.error}`
    : store.loading
      ? "loading"
      : null;
  const [announcedStatusKey, setAnnouncedStatusKey] = useState<StatusKey | null>(null);
  if (currentStatusKey !== announcedStatusKey) {
    setAnnouncedStatusKey(currentStatusKey);
    if (currentStatusKey !== null) {
      const message = store.error ? `error: ${store.error}` : "loading\u2026";
      setState((s) => ({ ...s, statusMessage: message }));
    }
  }

  // ─── Mosaic Tree Management ────────────────────────────────────

  const setMosaicNode = useCallback((node: WorkspaceTree | null) => {
    setState((s) => {
      // Preserve the flyout leaf's stack across pruning. The flyout
      // intentionally extracts its leaf from the mosaic tree but the stack
      // must survive — otherwise the renderer can't resolve the leaf when
      // the drawer asks for it (renders "Missing panel stack: …" instead).
      const preserveLeafIds = s.flyoutLeafId ? [s.flyoutLeafId] : undefined;
      const prunedStacks = pruneStacksToTree(node, s.viewStacks, { preserveLeafIds });
      const openViews = getOpenViews(node, prunedStacks);
      const focusedViewId = s.focusedViewId && openViews.includes(s.focusedViewId)
        ? s.focusedViewId
        : (openViews[0] ?? null);

      if (focusedViewId) {
        const leafId = findLeafForView(prunedStacks, focusedViewId);
        if (leafId) {
          prunedStacks[leafId] = {
            ...prunedStacks[leafId],
            activeView: focusedViewId,
          };
        }
      }

      return {
        ...s,
        mosaicNode: node,
        viewStacks: prunedStacks,
        focusedViewId,
        layoutDirty: true,
      };
    });
  }, []);

  const addView = useCallback((type: ViewId) => {
    setState((s) => {
      const base = s.expandedViewId && s.savedWorkspaceBeforeExpand
        ? s.savedWorkspaceBeforeExpand
        : { tree: s.mosaicNode, stacks: s.viewStacks };
      const openViews = getOpenViews(base.tree, base.stacks);
      const stacks = cloneViewStacks(base.stacks);

      // If already open, just focus it
      if (openViews.includes(type)) {
        const leafId = findLeafForView(stacks, type);
        if (leafId) {
          stacks[leafId].activeView = type;
        }
        return {
          ...s,
          mosaicNode: base.tree,
          viewStacks: stacks,
          focusedViewId: type,
          expandedViewId: null,
          savedWorkspaceBeforeExpand: null,
          statusMessage: `focused ${type}`,
          layoutDirty: s.layoutDirty,
        };
      }

      // If the requested view lives in the parked flyout leaf, surface the
      // drawer instead of duplicating the view into the mosaic. The flyout
      // leaf's stack stays in viewStacks (preserved across pruning), so we
      // can detect the case by walking that one stack alone — `openViews`
      // won't see it because the leaf isn't in `base.tree`.
      if (s.flyoutLeafId) {
        const flyoutStack = stacks[s.flyoutLeafId];
        if (flyoutStack && flyoutStack.views.includes(type)) {
          flyoutStack.activeView = type;
          return {
            ...s,
            viewStacks: stacks,
            flyoutOpen: true,
            flyoutDismissed: false,
            focusedViewId: type,
            statusMessage: `focused ${type} (flyout)`,
          };
        }
      }

      const targetLeafId = s.focusedViewId
        ? findLeafForView(stacks, s.focusedViewId)
        : null;
      const newLeafId = createLeafId(stacks);
      stacks[newLeafId] = {
        leafId: newLeafId,
        views: [type],
        activeView: type,
      };
      const newTree = addToTree(base.tree, newLeafId, targetLeafId);
      const preserveLeafIds = s.flyoutLeafId ? [s.flyoutLeafId] : undefined;

      return {
        ...s,
        mosaicNode: newTree,
        viewStacks: pruneStacksToTree(newTree, stacks, { preserveLeafIds }),
        focusedViewId: type,
        expandedViewId: null,
        savedWorkspaceBeforeExpand: null,
        statusMessage: `added ${type}`,
        layoutDirty: true,
      };
    });
  }, []);

  /**
   * ─── Flyout ────────────────────────────────────────────────────
   *
   * Helpers consumed by MosaicWorkspace's resize-observer effect. The
   * flyout owns one parked leaf at a time:
   *   - `extractLeafToFlyout` pulls the leaf out of the mosaic tree and
   *     records the previous tree shape so we can replay it on restore.
   *     The leaf's view stack is intentionally NOT pruned — `setMosaicNode`
   *     and other state writers honor `flyoutLeafId` via the
   *     `preserveLeafIds` option to `pruneStacksToTree`.
   *   - `restoreFlyoutLeaf` undoes the extraction. If the saved tree
   *     diverges from the current tree (the user restructured the mosaic
   *     during narrow mode), we fall back to the current tree — the parked
   *     leaf stays out and the user can drag it back manually.
   *   - `setFlyoutOpen` / `setFlyoutDismissed` are the manual close/reopen
   *     controls. Closing remembers the user's intent so we don't auto-
   *     re-promote on the next narrow tick.
   */
  const extractLeafToFlyout = useCallback((leafId: WorkspaceLeafId) => {
    setState((s) => {
      if (s.flyoutLeafId) return s; // already extracted
      if (!s.viewStacks[leafId]) return s; // nothing to extract
      const nextTree = removeFromTree(s.mosaicNode, leafId);
      // Don't prune — explicit preserveLeafIds keeps the flyout leaf alive.
      const prunedStacks = pruneStacksToTree(nextTree, s.viewStacks, {
        preserveLeafIds: [leafId],
      });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: prunedStacks,
        flyoutLeafId: leafId,
        flyoutSavedTree: s.mosaicNode,
        flyoutOpen: true,
        flyoutDismissed: false,
        // Keep layoutDirty unchanged: extracting to flyout is a viewport-
        // driven adjustment, not a user-authored layout change. We don't
        // want to mark the preset as dirty just because someone resized.
        layoutDirty: s.layoutDirty,
      };
    });
  }, []);

  const restoreFlyoutLeaf = useCallback(() => {
    setState((s) => {
      if (!s.flyoutLeafId) return s;
      // Replay the saved tree if its leaf set still matches what we know;
      // otherwise we can't safely reattach (the user restructured during
      // narrow mode). In the divergent case, leave the leaf out and let
      // the user drag it back.
      const tree = s.flyoutSavedTree ?? s.mosaicNode;
      return {
        ...s,
        mosaicNode: tree,
        flyoutLeafId: null,
        flyoutSavedTree: null,
        flyoutOpen: false,
        flyoutDismissed: false,
        layoutDirty: s.layoutDirty,
      };
    });
  }, []);

  const setFlyoutOpen = useCallback((open: boolean) => {
    setState((s) => {
      if (!s.flyoutLeafId) return s;
      // Closing remembers the user's intent so the auto-collapse effect
      // doesn't immediately re-promote on the next render. Re-opening
      // clears the dismiss flag so we resume normal auto behavior.
      return {
        ...s,
        flyoutOpen: open,
        flyoutDismissed: !open,
      };
    });
  }, []);

  const removeView = useCallback((viewId: ViewId) => {
    setState((s) => {
      const openViews = getOpenViews(s.mosaicNode, s.viewStacks);
      if (openViews.length <= 1) return s; // Keep at least one panel

      const leafId = findLeafForView(s.viewStacks, viewId);
      if (!leafId) {
        return s;
      }

      let newTree = s.mosaicNode;
      const stacks = cloneViewStacks(s.viewStacks);
      const stack = stacks[leafId];
      if (!stack) {
        return s;
      }

      const removeIdx = stack.views.indexOf(viewId);
      stack.views = stack.views.filter((view) => view !== viewId);
      if (stack.views.length === 0) {
        delete stacks[leafId];
        newTree = removeFromTree(newTree, leafId);
      } else if (stack.activeView === viewId) {
        const nextIdx = Math.max(0, Math.min(removeIdx, stack.views.length - 1));
        stack.activeView = stack.views[nextIdx];
      }

      const prunedStacks = pruneStacksToTree(newTree, stacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      const newViews = getOpenViews(newTree, prunedStacks);
      const fallbackFocused = newViews[0] ?? null;
      const focusedViewId = s.focusedViewId === viewId
        ? fallbackFocused
        : (s.focusedViewId && newViews.includes(s.focusedViewId) ? s.focusedViewId : fallbackFocused);

      if (focusedViewId) {
        const focusedLeaf = findLeafForView(prunedStacks, focusedViewId);
        if (focusedLeaf) {
          prunedStacks[focusedLeaf] = {
            ...prunedStacks[focusedLeaf],
            activeView: focusedViewId,
          };
        }
      }

      return {
        ...s,
        mosaicNode: newTree,
        viewStacks: prunedStacks,
        focusedViewId,
        statusMessage: `closed ${viewId}`,
        layoutDirty: true,
      };
    });
  }, []);

  const setFocusedView = useCallback((viewId: ViewId) => {
    setState((s) => {
      const leafId = findLeafForView(s.viewStacks, viewId);
      if (!leafId) {
        return s;
      }
      const stacks = cloneViewStacks(s.viewStacks);
      stacks[leafId].activeView = viewId;
      return { ...s, focusedViewId: viewId, viewStacks: stacks };
    });
  }, []);

  const stackLeafIntoTabs = useCallback((sourceLeafId: WorkspaceLeafId, targetLeafId: WorkspaceLeafId) => {
    setState((s) => {
      if (!s.mosaicNode || sourceLeafId === targetLeafId) {
        return s;
      }
      const source = s.viewStacks[sourceLeafId];
      const target = s.viewStacks[targetLeafId];
      if (!source || !target) {
        return s;
      }

      const stacks = cloneViewStacks(s.viewStacks);
      const nextSource = stacks[sourceLeafId];
      const nextTarget = stacks[targetLeafId];
      if (!nextSource || !nextTarget) {
        return s;
      }

      const merged = [...nextTarget.views];
      for (const view of nextSource.views) {
        if (!merged.includes(view)) {
          merged.push(view);
        }
      }
      nextTarget.views = merged;
      nextTarget.activeView = nextSource.activeView;
      delete stacks[sourceLeafId];

      const newTree = removeFromTree(s.mosaicNode, sourceLeafId);
      const prunedStacks = pruneStacksToTree(newTree, stacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });

      return {
        ...s,
        mosaicNode: newTree,
        viewStacks: prunedStacks,
        focusedViewId: nextTarget.activeView,
        statusMessage: `stacked into ${nextTarget.activeView}`,
        layoutDirty: true,
      };
    });
  }, []);

  const moveLeafToSplit = useCallback((
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
    zone: SplitZone,
  ) => {
    setState((s) => {
      if (!s.mosaicNode || sourceLeafId === targetLeafId) {
        return s;
      }
      if (!s.viewStacks[sourceLeafId] || !s.viewStacks[targetLeafId]) {
        return s;
      }

      const treeWithoutSource = removeFromTree(s.mosaicNode, sourceLeafId);
      if (!treeWithoutSource) {
        return s;
      }
      const targetPath = findPathToLeaf(treeWithoutSource, targetLeafId);
      if (!targetPath) {
        return s;
      }
      const nextTree = insertLeafAtPathEdge(
        treeWithoutSource,
        targetPath,
        sourceLeafId,
        zone,
      );
      const nextStacks = pruneStacksToTree(nextTree, s.viewStacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: nextStacks,
        statusMessage: "Split moved",
        layoutDirty: true,
      };
    });
  }, []);

  const moveLeafToWindowEdge = useCallback((sourceLeafId: WorkspaceLeafId, edge: SplitZone) => {
    setState((s) => {
      if (!s.mosaicNode || !s.viewStacks[sourceLeafId]) {
        return s;
      }

      const remainder = removeFromTree(s.mosaicNode, sourceLeafId);
      const nextTree = wrapTreeAtEdge(remainder, sourceLeafId, edge);
      const nextStacks = pruneStacksToTree(nextTree, s.viewStacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      const focusedViewId = nextStacks[sourceLeafId]?.activeView ?? s.focusedViewId;
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: nextStacks,
        focusedViewId,
        statusMessage: "Panel moved to the window edge",
        layoutDirty: true,
      };
    });
  }, []);

  /**
   * Move a tab to a specific position inside any leaf (same or different).
   * Used for in-strip reorder via drag-and-drop. `targetIndex` is the
   * insertion point in the *resulting* views array (so dropping at the end
   * uses targetIndex === stack.views.length).
   */
  const reorderTabInLeaf = useCallback((
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
    targetIndex: number,
  ) => {
    setState((s) => {
      const source = s.viewStacks[sourceLeafId];
      const target = s.viewStacks[targetLeafId];
      if (!source || !target || !source.views.includes(viewId)) return s;

      let nextTree = s.mosaicNode;
      const nextStacks = cloneViewStacks(s.viewStacks);
      const nextSource = nextStacks[sourceLeafId];
      const nextTarget = nextStacks[targetLeafId];
      if (!nextSource || !nextTarget) return s;

      // Same-leaf reorder: pull the view out of its current position, then
      // splice it back in at targetIndex (clamped). Adjust the index when
      // the original position was before the target so we don't off-by-one.
      if (sourceLeafId === targetLeafId) {
        const currentIndex = nextSource.views.indexOf(viewId);
        if (currentIndex === -1) return s;
        const adjustedIndex = currentIndex < targetIndex ? targetIndex - 1 : targetIndex;
        const clamped = Math.max(0, Math.min(adjustedIndex, nextSource.views.length - 1));
        if (clamped === currentIndex) return s;
        nextSource.views = nextSource.views.filter((view) => view !== viewId);
        nextSource.views.splice(clamped, 0, viewId);
        nextSource.activeView = viewId;
        return {
          ...s,
          viewStacks: nextStacks,
          focusedViewId: viewId,
          statusMessage: `reordered ${viewId}`,
          layoutDirty: true,
        };
      }

      // Cross-leaf insert at a specific position.
      nextSource.views = nextSource.views.filter((view) => view !== viewId);
      const clamped = Math.max(0, Math.min(targetIndex, nextTarget.views.length));
      if (!nextTarget.views.includes(viewId)) {
        nextTarget.views.splice(clamped, 0, viewId);
      }
      nextTarget.activeView = viewId;

      if (nextSource.views.length === 0) {
        delete nextStacks[sourceLeafId];
        nextTree = removeFromTree(nextTree, sourceLeafId);
      } else if (nextSource.activeView === viewId) {
        nextSource.activeView = nextSource.views[0];
      }

      const prunedStacks = pruneStacksToTree(nextTree, nextStacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: prunedStacks,
        focusedViewId: viewId,
        statusMessage: `moved ${viewId}`,
        layoutDirty: true,
      };
    });
  }, []);

  const stackViewIntoLeaf = useCallback((
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
  ) => {
    setState((s) => {
      const source = s.viewStacks[sourceLeafId];
      const target = s.viewStacks[targetLeafId];
      if (!source || !target || !source.views.includes(viewId)) {
        return s;
      }

      if (sourceLeafId === targetLeafId) {
        const nextStacks = cloneViewStacks(s.viewStacks);
        nextStacks[targetLeafId].activeView = viewId;
        return {
          ...s,
          viewStacks: nextStacks,
          focusedViewId: viewId,
          statusMessage: `focused ${viewId}`,
          layoutDirty: s.layoutDirty,
        };
      }

      let nextTree = s.mosaicNode;
      const nextStacks = cloneViewStacks(s.viewStacks);
      const nextSource = nextStacks[sourceLeafId];
      const nextTarget = nextStacks[targetLeafId];
      if (!nextSource || !nextTarget) {
        return s;
      }

      nextSource.views = nextSource.views.filter((view) => view !== viewId);
      if (!nextTarget.views.includes(viewId)) {
        nextTarget.views.push(viewId);
      }
      nextTarget.activeView = viewId;

      if (nextSource.views.length === 0) {
        delete nextStacks[sourceLeafId];
        nextTree = removeFromTree(nextTree, sourceLeafId);
      } else if (nextSource.activeView === viewId) {
        nextSource.activeView = nextSource.views[0];
      }

      const prunedStacks = pruneStacksToTree(nextTree, nextStacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: prunedStacks,
        focusedViewId: viewId,
        statusMessage: `stacked ${viewId}`,
        layoutDirty: true,
      };
    });
  }, []);

  const addViewToLeaf = useCallback((viewId: ViewId, targetLeafId: WorkspaceLeafId) => {
    setState((s) => {
      const target = s.viewStacks[targetLeafId];
      if (!target) {
        return s;
      }

      const existingLeafId = findLeafForView(s.viewStacks, viewId);
      if (existingLeafId === targetLeafId) {
        const nextStacks = cloneViewStacks(s.viewStacks);
        nextStacks[targetLeafId].activeView = viewId;
        return {
          ...s,
          viewStacks: nextStacks,
          focusedViewId: viewId,
          statusMessage: `focused ${viewId}`,
          layoutDirty: s.layoutDirty,
        };
      }

      let nextTree = s.mosaicNode;
      const nextStacks = cloneViewStacks(s.viewStacks);
      const nextTarget = nextStacks[targetLeafId];
      if (!nextTarget) {
        return s;
      }

      // Track whether the source leaf is the flyout. If we drain the
      // flyout's last view, we must also clear flyoutLeafId so the
      // resize-observer effect doesn't try to render a now-empty drawer
      // and so subsequent renders don't preserve a stack that no longer
      // exists. The rest of the move proceeds as usual.
      const sourceIsFlyout = existingLeafId != null && existingLeafId === s.flyoutLeafId;
      let flyoutCleared = false;

      if (existingLeafId) {
        const source = nextStacks[existingLeafId];
        if (source) {
          source.views = source.views.filter((view) => view !== viewId);
          if (source.views.length === 0) {
            delete nextStacks[existingLeafId];
            nextTree = removeFromTree(nextTree, existingLeafId);
            if (sourceIsFlyout) flyoutCleared = true;
          } else if (source.activeView === viewId) {
            source.activeView = source.views[0];
          }
        }
      }

      if (!nextTarget.views.includes(viewId)) {
        nextTarget.views.push(viewId);
      }
      nextTarget.activeView = viewId;

      // Preserve the flyout leaf's stack on prune unless we just drained
      // and cleared it.
      const preserveLeafIds = !flyoutCleared && s.flyoutLeafId
        ? [s.flyoutLeafId]
        : undefined;
      const prunedStacks = pruneStacksToTree(nextTree, nextStacks, { preserveLeafIds });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: prunedStacks,
        focusedViewId: viewId,
        statusMessage: `added ${viewId} to panel`,
        layoutDirty: true,
        ...(flyoutCleared
          ? {
              flyoutLeafId: null,
              flyoutSavedTree: null,
              flyoutOpen: false,
              flyoutDismissed: false,
            }
          : {}),
      };
    });
  }, []);

  const swapViewInLeaf = useCallback((
    oldViewId: ViewId,
    newViewId: ViewId,
    targetLeafId: WorkspaceLeafId,
  ) => {
    setState((s) => {
      const target = s.viewStacks[targetLeafId];
      if (!target || !target.views.includes(oldViewId)) {
        return s;
      }
      if (oldViewId === newViewId) {
        const nextStacks = cloneViewStacks(s.viewStacks);
        nextStacks[targetLeafId].activeView = newViewId;
        return {
          ...s,
          viewStacks: nextStacks,
          focusedViewId: newViewId,
          statusMessage: `focused ${newViewId}`,
          layoutDirty: s.layoutDirty,
        };
      }

      let nextTree = s.mosaicNode;
      const nextStacks = cloneViewStacks(s.viewStacks);
      const nextTarget = nextStacks[targetLeafId];
      if (!nextTarget) {
        return s;
      }

      const existingLeafId = findLeafForView(nextStacks, newViewId);
      if (existingLeafId && existingLeafId !== targetLeafId) {
        const source = nextStacks[existingLeafId];
        if (source) {
          source.views = source.views.filter((view) => view !== newViewId);
          if (source.views.length === 0) {
            delete nextStacks[existingLeafId];
            nextTree = removeFromTree(nextTree, existingLeafId);
          } else if (source.activeView === newViewId) {
            source.activeView = source.views[0];
          }
        }
      }

      if (nextTarget.views.includes(newViewId)) {
        nextTarget.views = nextTarget.views.filter((view) => view !== oldViewId);
      } else {
        const index = nextTarget.views.indexOf(oldViewId);
        nextTarget.views[index] = newViewId;
      }
      nextTarget.activeView = newViewId;

      const prunedStacks = pruneStacksToTree(nextTree, nextStacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: prunedStacks,
        focusedViewId: newViewId,
        statusMessage: `swapped ${oldViewId} for ${newViewId}`,
        layoutDirty: true,
      };
    });
  }, []);

  const splitViewIntoLeaf = useCallback((
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    targetLeafId: WorkspaceLeafId,
    zone: SplitZone,
  ) => {
    setState((s) => {
      let nextTree = s.mosaicNode;
      const nextStacks = cloneViewStacks(s.viewStacks);
      const source = nextStacks[sourceLeafId];
      const target = nextStacks[targetLeafId];
      if (!source || !target || !source.views.includes(viewId) || !nextTree) {
        return s;
      }

      if (sourceLeafId === targetLeafId && source.views.length <= 1) {
        return s;
      }

      const removeIdx = source.views.indexOf(viewId);
      source.views = source.views.filter((view) => view !== viewId);

      let newLeafId: WorkspaceLeafId;
      if (source.views.length === 0) {
        delete nextStacks[sourceLeafId];
        nextTree = removeFromTree(nextTree, sourceLeafId);
        newLeafId = sourceLeafId;
      } else {
        if (source.activeView === viewId) {
          const nextIdx = Math.max(0, Math.min(removeIdx, source.views.length - 1));
          source.activeView = source.views[nextIdx];
        }
        newLeafId = createLeafId(nextStacks);
      }

      if (!nextTree) {
        nextStacks[newLeafId] = {
          leafId: newLeafId,
          views: [viewId],
          activeView: viewId,
        };
        return {
          ...s,
          mosaicNode: newLeafId,
          viewStacks: nextStacks,
          focusedViewId: viewId,
          statusMessage: `split ${viewId}`,
          layoutDirty: true,
        };
      }

      const targetPath = findPathToLeaf(nextTree, targetLeafId);
      if (!targetPath) {
        return s;
      }

      nextStacks[newLeafId] = {
        leafId: newLeafId,
        views: [viewId],
        activeView: viewId,
      };
      nextTree = insertLeafAtPathEdge(nextTree, targetPath, newLeafId, zone);

      const prunedStacks = pruneStacksToTree(nextTree, nextStacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: prunedStacks,
        focusedViewId: viewId,
        statusMessage: `split ${viewId}`,
        layoutDirty: true,
      };
    });
  }, []);

  const splitViewToWindowEdge = useCallback((
    viewId: ViewId,
    sourceLeafId: WorkspaceLeafId,
    edge: SplitZone,
  ) => {
    setState((s) => {
      let nextTree = s.mosaicNode;
      const nextStacks = cloneViewStacks(s.viewStacks);
      const source = nextStacks[sourceLeafId];
      if (!source || !source.views.includes(viewId)) {
        return s;
      }

      if (source.views.length <= 1 && nextTree && typeof nextTree === "string" && nextTree === sourceLeafId) {
        return s;
      }

      const removeIdx = source.views.indexOf(viewId);
      source.views = source.views.filter((view) => view !== viewId);

      let newLeafId: WorkspaceLeafId;
      if (source.views.length === 0) {
        delete nextStacks[sourceLeafId];
        nextTree = removeFromTree(nextTree, sourceLeafId);
        newLeafId = sourceLeafId;
      } else {
        if (source.activeView === viewId) {
          const nextIdx = Math.max(0, Math.min(removeIdx, source.views.length - 1));
          source.activeView = source.views[nextIdx];
        }
        newLeafId = createLeafId(nextStacks);
      }

      nextStacks[newLeafId] = {
        leafId: newLeafId,
        views: [viewId],
        activeView: viewId,
      };
      nextTree = wrapTreeAtEdge(nextTree, newLeafId, edge);

      const prunedStacks = pruneStacksToTree(nextTree, nextStacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
      return {
        ...s,
        mosaicNode: nextTree,
        viewStacks: prunedStacks,
        focusedViewId: viewId,
        statusMessage: `split ${viewId} to edge`,
        layoutDirty: true,
      };
    });
  }, []);

  const loadPreset = useCallback((name: string) => {
    const preset = LAYOUT_PRESETS.find((p) => p.name === name);
    if (preset) {
      const workspace = preset.workspace ?? workspaceFromViewTree(preset.tree);
      const leaves = getOpenViews(workspace.tree, workspace.stacks);
      setState((s) => ({
        ...s,
        mosaicNode: workspace.tree,
        viewStacks: workspace.stacks,
        focusedViewId: leaves[0] ?? null,
        expandedViewId: null,
        savedWorkspaceBeforeExpand: null,
        statusMessage: `layout: ${preset.label}`,
        layoutDirty: false,
        activePresetId: preset.name,
        activePresetLabel: preset.label,
      }));
      return;
    }

    const saved = loadNamedLayouts().find((layout) => layout.id === name);
    if (saved) {
      const workspace = normalizeDesignWorkspace(saved.layout);
      const leaves = getOpenViews(workspace.tree, workspace.stacks);
      setState((s) => ({
        ...s,
        mosaicNode: workspace.tree,
        viewStacks: workspace.stacks,
        focusedViewId: leaves[0] ?? null,
        expandedViewId: null,
        savedWorkspaceBeforeExpand: null,
        statusMessage: `layout: ${saved.name}`,
        layoutDirty: false,
        activePresetId: saved.id,
        activePresetLabel: saved.name,
      }));
    }
  }, []);

  // ─── Selection ─────────────────────────────────────────────────

  const setSelected = useCallback((idx: number, opts?: { source?: "user" | "history"; pushHistory?: boolean }) => {
    const insts = instructionsRef.current;
    const max = Math.max(0, insts.length - 1);
    const clamped = Math.max(0, Math.min(idx, max));
    jumpLocationInstruction(clamped, opts);
    setState((s) => ({ ...s, selectedIndex: clamped }));
  }, [jumpLocationInstruction]);

  const goBack = useCallback(() => {
    const moved = goBackLocation();
    if (!moved) {
      setState((s) => ({ ...s, statusMessage: "No previous location" }));
    }
  }, [goBackLocation]);

  const goForward = useCallback(() => {
    const moved = goForwardLocation();
    if (!moved) {
      setState((s) => ({ ...s, statusMessage: "No next location" }));
    }
  }, [goForwardLocation]);

  /**
   * Cycle to the next layout preset in `LAYOUT_PRESETS` order. If no preset
   * is currently active (or the active id no longer matches any preset), we
   * start from the first preset. Declared after loadPreset so its dependency
   * is real and not a stale-closure ref.
   */
  const cycleLayoutPreset = useCallback(() => {
    const ids = LAYOUT_PRESETS.map((p) => p.name);
    if (ids.length === 0) return;
    setState((s) => {
      const idx = s.activePresetId ? ids.indexOf(s.activePresetId) : -1;
      const next = ids[(idx + 1) % ids.length];
      Promise.resolve().then(() => loadPreset(next));
      return s;
    });
  }, [loadPreset]);

  // ─── Panel Visibility Toggles ──────────────────────────────────

  const togglePanel = useCallback((type: ViewId) => {
    setState((s) => {
      const base = s.expandedViewId && s.savedWorkspaceBeforeExpand
        ? s.savedWorkspaceBeforeExpand
        : { tree: s.mosaicNode, stacks: s.viewStacks };
      const openViews = getOpenViews(base.tree, base.stacks);

      if (openViews.includes(type)) {
        const stacks = cloneViewStacks(base.stacks);
        const leafId = findLeafForView(stacks, type);
        if (leafId) {
          stacks[leafId].activeView = type;
        }

        if (s.focusedViewId === type) {
          if (openViews.length <= 1) {
            return s;
          }
          if (!leafId) {
            return s;
          }

          let newTree = base.tree;
          const targetStack = stacks[leafId];
          const removeIdx = targetStack.views.indexOf(type);
          targetStack.views = targetStack.views.filter((view) => view !== type);
          if (targetStack.views.length === 0) {
            delete stacks[leafId];
            newTree = removeFromTree(newTree, leafId);
          } else if (targetStack.activeView === type) {
            const nextIdx = Math.max(0, Math.min(removeIdx, targetStack.views.length - 1));
            targetStack.activeView = targetStack.views[nextIdx];
          }

          const prunedStacks = pruneStacksToTree(newTree, stacks, { preserveLeafIds: s.flyoutLeafId ? [s.flyoutLeafId] : undefined });
          const newViews = getOpenViews(newTree, prunedStacks);
          const focusedViewId = newViews[0] ?? null;
          if (focusedViewId) {
            const focusedLeaf = findLeafForView(prunedStacks, focusedViewId);
            if (focusedLeaf) {
              prunedStacks[focusedLeaf].activeView = focusedViewId;
            }
          }

          return {
            ...s,
            mosaicNode: newTree,
            viewStacks: prunedStacks,
            focusedViewId,
            expandedViewId: null,
            savedWorkspaceBeforeExpand: null,
            statusMessage: `closed ${type}`,
            layoutDirty: true,
          };
        }

        return {
          ...s,
          mosaicNode: base.tree,
          viewStacks: stacks,
          focusedViewId: type,
          expandedViewId: null,
          savedWorkspaceBeforeExpand: null,
          statusMessage: `focused ${type}`,
          layoutDirty: s.layoutDirty,
        };
      }

      const stacks = cloneViewStacks(base.stacks);

      // Same-as-addView duplication guard: if the requested view lives in
      // the parked flyout leaf, surface the drawer rather than spawning a
      // second copy of the panel in the mosaic. The flyout leaf's stack
      // is preserved (it isn't in `base.tree` so `openViews` doesn't list
      // it, but it remains in `viewStacks`), so we detect the case by
      // walking that one stack directly.
      if (s.flyoutLeafId) {
        const flyoutStack = stacks[s.flyoutLeafId];
        if (flyoutStack && flyoutStack.views.includes(type)) {
          flyoutStack.activeView = type;
          return {
            ...s,
            viewStacks: stacks,
            flyoutOpen: true,
            flyoutDismissed: false,
            focusedViewId: type,
            statusMessage: `focused ${type} (flyout)`,
          };
        }
      }

      const targetLeafId = s.focusedViewId
        ? findLeafForView(stacks, s.focusedViewId)
        : null;
      const newLeafId = createLeafId(stacks);
      stacks[newLeafId] = {
        leafId: newLeafId,
        views: [type],
        activeView: type,
      };
      const newTree = addToTree(base.tree, newLeafId, targetLeafId);
      const preserveLeafIds = s.flyoutLeafId ? [s.flyoutLeafId] : undefined;

      return {
        ...s,
        mosaicNode: newTree,
        viewStacks: pruneStacksToTree(newTree, stacks, { preserveLeafIds }),
        focusedViewId: type,
        expandedViewId: null,
        savedWorkspaceBeforeExpand: null,
        statusMessage: `added ${type}`,
        layoutDirty: true,
      };
    });
  }, []);

  const saveLayout = useCallback((name = "Design parity layout") => {
    setState((s) => {
      saveNamedLayout(name, {
        tree: s.mosaicNode,
        stacks: s.viewStacks,
      });
      return {
        ...s,
        layoutDirty: false,
        statusMessage: `saved layout: ${name}`,
      };
    });
  }, []);

  // ─── Focus Cycling ─────────────────────────────────────────────

  const cycleFocus = useCallback((direction: 1 | -1 = 1) => {
    setState((s) => {
      const openViews = getOpenViews(s.mosaicNode, s.viewStacks);
      if (openViews.length === 0) return s;
      const idx = openViews.indexOf(s.focusedViewId!);
      const nextIdx = idx < 0 ? 0 : (idx + direction + openViews.length) % openViews.length;
      const next = openViews[nextIdx];
      const stacks = cloneViewStacks(s.viewStacks);
      const leafId = findLeafForView(stacks, next);
      if (leafId) {
        stacks[leafId].activeView = next;
      }
      return { ...s, focusedViewId: next, viewStacks: stacks, statusMessage: `focus: ${next}` };
    });
  }, []);

  /**
   * Cycle the active tab inside the *focused leaf*. Distinct from cycleFocus,
   * which cycles across every open view in every leaf — this stays inside one
   * panel's tab strip, matching what `Ctrl+Tab` / `Ctrl+Shift+Tab` typically
   * do in IDE workflows.
   */
  const cycleTabInLeaf = useCallback((direction: 1 | -1 = 1) => {
    setState((s) => {
      if (!s.focusedViewId) return s;
      const leafId = findLeafForView(s.viewStacks, s.focusedViewId);
      if (!leafId) return s;
      const stack = s.viewStacks[leafId];
      if (!stack || stack.views.length <= 1) return s;
      const idx = stack.views.indexOf(s.focusedViewId);
      const nextIdx = idx < 0 ? 0 : (idx + direction + stack.views.length) % stack.views.length;
      const next = stack.views[nextIdx];
      const stacks = cloneViewStacks(s.viewStacks);
      stacks[leafId].activeView = next;
      return { ...s, focusedViewId: next, viewStacks: stacks, statusMessage: `tab: ${next}` };
    });
  }, []);

  /**
   * Close the currently-focused tab. Routes through the standard removeView
   * path so the leaf collapses to its remaining tabs / leaves cleanly. The
   * read of focusedViewId happens inside setState's snapshot so we always
   * see the latest value, even when the user fires the shortcut twice in
   * quick succession.
   */
  const closeFocusedTab = useCallback(() => {
    setState((s) => {
      if (s.focusedViewId) {
        // Schedule the removal asynchronously so we don't mutate state from
        // inside another setState callback.
        Promise.resolve().then(() => removeView(s.focusedViewId!));
      }
      return s;
    });
  }, [removeView]);

  /**
   * Close every tab in the focused panel — Mod+Shift+W contract. Sequenced
   * via Promise.resolve so each removal sees the prior state update; otherwise
   * a synchronous loop would race against React's batching and only the first
   * removeView would land. The leaf collapses naturally once its last view
   * is removed.
   */
  const closeFocusedLeaf = useCallback(() => {
    setState((s) => {
      const focused = s.focusedViewId;
      if (!focused) return s;
      const leafId = findLeafForView(s.viewStacks, focused);
      if (!leafId) return s;
      const stack = s.viewStacks[leafId];
      if (!stack) return s;
      const views = [...stack.views];
      void views.reduce<Promise<void>>(
        (chain, view) => chain.then(() => { removeView(view); }),
        Promise.resolve(),
      );
      return s;
    });
  }, [removeView]);

  /**
   * Split the focused view to the named edge of the workspace. Wrapper
   * around `splitViewToWindowEdge` that resolves the focused leaf for the
   * caller, so keyboard shortcuts ("Mod+\" / "Mod+Shift+\") don't have to
   * thread the leaf id through the command registry.
   */
  const splitFocusedToEdge = useCallback((edge: SplitZone) => {
    setState((s) => {
      const v = s.focusedViewId;
      if (!v) return s;
      const leafId = findLeafForView(s.viewStacks, v);
      if (!leafId) return s;
      Promise.resolve().then(() => splitViewToWindowEdge(v, leafId, edge));
      return s;
    });
  }, [splitViewToWindowEdge]);

  // cycleLayoutPreset is defined further below — it depends on loadPreset
  // (which lives later in the file), so we declare it after loadPreset to
  // keep the dependency direction natural and avoid a stale-closure pin via
  // refs.

  // ─── Group Navigation ──────────────────────────────────────────

  const navigateGroup = useCallback((direction: 1 | -1) => {
    const insts = instructionsRef.current;
    let target: number | null = null;
    setState((s) => {
      const cur = insts[s.selectedIndex]?.group;
      if (direction > 0) {
        const next = insts.findIndex(
          (inst) => inst.group !== null && inst.group > (cur ?? -1) && inst.groupStart,
        );
        if (next >= 0) {
          target = next;
          return {
            ...s,
            selectedIndex: next,
            statusMessage: `group c${String(insts[next].group).padStart(3, "0")}`,
          };
        }
        return s;
      } else {
        for (let i = s.selectedIndex - 1; i >= 0; i--) {
          if (insts[i].groupStart && insts[i].group !== cur) {
            target = i;
            return {
              ...s,
              selectedIndex: i,
              statusMessage: `group c${String(insts[i].group).padStart(3, "0")}`,
            };
          }
        }
        return s;
      }
    });
    if (target != null) {
      jumpLocationInstruction(target);
    }
  }, [jumpLocationInstruction]);

  // ─── Toggles ───────────────────────────────────────────────────

  const toggleTrace = useCallback(() => {
    setState((s) => ({ ...s, trace: !s.trace, statusMessage: s.trace ? "trace off" : "trace on" }));
  }, []);

  // ─── Annotations & Sync ────────────────────────────────────────

  const handleToggleBookmark = useCallback((idx: number) => {
    setState((s) => {
      const next = toggleBookmark(s.annotations, idx);
      return { ...s, annotations: next };
    });
  }, []);

  const handleSetComment = useCallback((idx: number, text: string) => {
    setState((s) => {
      const next = setComment(s.annotations, idx, text);
      return { ...s, annotations: next };
    });
  }, []);

  const toggleJumpArrows = useCallback(() => {
    setState((s) => ({ ...s, showJumpArrows: !s.showJumpArrows, statusMessage: s.showJumpArrows ? "jump arrows off" : "jump arrows on" }));
  }, []);

  // ─── Expand / Maximize ────────────────────────────────────────

  const toggleExpand = useCallback(() => {
    setState((s) => {
      if (s.expandedViewId) {
        if (!s.savedWorkspaceBeforeExpand) {
          return {
            ...s,
            expandedViewId: null,
            statusMessage: "Layout restored",
          };
        }
        // Restore previous layout
        return {
          ...s,
          mosaicNode: s.savedWorkspaceBeforeExpand.tree,
          viewStacks: s.savedWorkspaceBeforeExpand.stacks,
          expandedViewId: null,
          savedWorkspaceBeforeExpand: null,
          statusMessage: `restored layout`,
        };
      }
      // Expand focused panel to fill the whole workspace
      if (!s.focusedViewId) return s;
      const focusedLeafId = findLeafForView(s.viewStacks, s.focusedViewId);
      if (!focusedLeafId) {
        return s;
      }
      const focusedStack = s.viewStacks[focusedLeafId];
      if (!focusedStack) {
        return s;
      }
      return {
        ...s,
        savedWorkspaceBeforeExpand: {
          tree: s.mosaicNode,
          stacks: s.viewStacks,
        },
        mosaicNode: focusedLeafId,
        viewStacks: {
          [focusedLeafId]: {
            leafId: focusedLeafId,
            views: [...focusedStack.views],
            activeView: focusedStack.activeView,
          },
        },
        expandedViewId: focusedLeafId,
        statusMessage: `maximized ${s.focusedViewId}`,
      };
    });
  }, []);

  // ─── Overlay Controls ──────────────────────────────────────────

  const openPalette = useCallback(() => setState((s) => ({ ...s, showPalette: true })), []);
  const closePalette = useCallback(() => setState((s) => ({ ...s, showPalette: false })), []);
  const openHelp = useCallback(() => setState((s) => ({ ...s, showHelp: true })), []);
  const closeHelp = useCallback(() => setState((s) => ({ ...s, showHelp: false })), []);
  const openCommand = useCallback(() => setState((s) => ({ ...s, showCommand: true, commandValue: "" })), []);
  const closeCommand = useCallback(() => setState((s) => ({ ...s, showCommand: false })), []);

  // ─── File Operations ───────────────────────────────────────────

  const handleOpenFile = useCallback(async () => {
    const path = await openFileDialog();
    if (path) {
      const data = await store.loadFile(path);
      if (data) {
        addRecentFile({
          path,
          filename: data.metadata.filename || path.split("/").pop() || path,
          mode: "open",
          instructionCount: data.metadata.instructionCount,
          tclVersion: data.metadata.tclVersion,
        });
      }
    }
  }, [store]);

  const omniboxResults = useMemo(() => {
    return parseOmniboxQuery({
      query: state.commandValue,
      instructions: store.data?.instructions ?? [],
      blocks: store.data?.blocks ?? [],
      locationIndex,
      annotations: state.annotations,
      literals: store.data?.literals ?? [],
      decompileLines: store.data?.decompileLines ?? [],
      limit: 12,
    });
  }, [
    state.commandValue,
    locationIndex,
    state.annotations,
    store.data?.blocks,
    store.data?.decompileLines,
    store.data?.instructions,
    store.data?.literals,
  ]);

  const executeOmniboxResult = useCallback((result: OmniboxResult) => {
    if (result.instIdx != null) {
      setSelected(result.instIdx);
      setState((s) => ({
        ...s,
        showCommand: false,
        statusMessage: `${result.type}: ${result.label}`,
      }));
      return;
    }

    if (result.type === "literal") {
      addView("literals");
      setState((s) => ({
        ...s,
        showCommand: false,
        statusMessage: `literal: ${result.label}`,
      }));
      return;
    }

    setState((s) => ({
      ...s,
      showCommand: false,
      statusMessage: `no jump target: ${result.label}`,
    }));
  }, [addView, setSelected]);

  // ─── Command Execution ─────────────────────────────────────────

  const executeCommand = useCallback((cmd: string) => {
    const trimmed = cmd.trim();

    if (trimmed === "q" || trimmed === "quit") {
      setState((s) => ({
        ...s,
        showCommand: false,
        statusMessage: "Close is available in the desktop application.",
      }));
      return;
    }

    if (trimmed.startsWith("panel add ") || trimmed.startsWith("panel ")) {
      const type = trimmed.replace(/^panel\s+(add\s+)?/, "").trim();
      if (!isViewId(type)) {
        setState((s) => ({
          ...s,
          showCommand: false,
          statusMessage: `unknown panel: ${type}`,
        }));
        return;
      }
      addView(type);
      setState((s) => ({ ...s, showCommand: false }));
      return;
    }

    if (trimmed.startsWith("layout ")) {
      const name = trimmed.slice(7).trim();
      loadPreset(name);
      setState((s) => ({ ...s, showCommand: false }));
      return;
    }

    const matches = parseOmniboxQuery({
      query: trimmed,
      instructions: store.data?.instructions ?? [],
      blocks: store.data?.blocks ?? [],
      locationIndex,
      annotations: state.annotations,
      literals: store.data?.literals ?? [],
      decompileLines: store.data?.decompileLines ?? [],
      limit: 1,
    });
    if (matches[0]) {
      executeOmniboxResult(matches[0]);
      return;
    }

    setState((s) => ({ ...s, showCommand: false, statusMessage: `unknown: ${trimmed}` }));
  }, [
    addView,
    executeOmniboxResult,
    locationIndex,
    loadPreset,
    state.annotations,
    store.data?.blocks,
    store.data?.decompileLines,
    store.data?.instructions,
    store.data?.literals,
  ]);

  return {
    state,
    setState,
    setSelected,
    addView,
    removeView,
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
    loadPreset,
    saveLayout,
    togglePanel,
    toggleTrace,
    toggleExpand,
    cycleFocus,
    cycleTabInLeaf,
    closeFocusedTab,
    closeFocusedLeaf,
    splitFocusedToEdge,
    cycleLayoutPreset,
    extractLeafToFlyout,
    restoreFlyoutLeaf,
    setFlyoutOpen,
    goBack,
    goForward,
    navigateGroup,
    openPalette,
    closePalette,
    openHelp,
    closeHelp,
    openCommand,
    closeCommand,
    handleOpenFile,
    executeCommand,
    omniboxResults,
    executeOmniboxResult,
    handleToggleBookmark,
    handleSetComment,
    toggleJumpArrows,
  };
}
