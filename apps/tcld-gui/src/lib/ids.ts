/**
 * Typed identifier helpers for the workspace mosaic.
 *
 * Identifiers are strings on the wire (so they survive JSON serialization,
 * dnd-kit identity tests, and `react-mosaic-component` storage), but the
 * code that constructs and parses them should use these helpers so a typo
 * in `"tab:..."` or `"tab-slot:..."` is a compile-time error rather than a
 * silent string mismatch at runtime.
 *
 * Branded types: a `LeafId` is structurally a string but the `__leaf` brand
 * means a plain `string` cannot be assigned to a `LeafId` parameter — you
 * must go through `asLeafId(s)`. The runtime cost is zero.
 */

import type { ViewId, WorkspaceLeafId } from "@/types";

declare const LEAF_BRAND: unique symbol;

/**
 * A `LeafId` is the string identifier of a single leaf in the workspace
 * mosaic. Get one via `asLeafId(rawString)` when reading from storage or
 * a layout preset. New leaves are minted via `mintLeafId`.
 */
export type LeafId = string & { readonly [LEAF_BRAND]: "leaf" };

/** Coerce a raw string from storage / preset / DOM into a LeafId. No runtime check. */
export function asLeafId(value: string): LeafId {
  return value as LeafId;
}

/** Erase the brand for callers that still type things as `WorkspaceLeafId`. */
export function fromLeafId(id: LeafId): WorkspaceLeafId {
  return id;
}

let _nextLeafCounter = 0;
/**
 * Mint a fresh LeafId. The `${prefix}-${n}` shape keeps storage compatibility
 * with the existing `"leaf-1"` / `"leaf-2"` ids written by older builds.
 */
export function mintLeafId(prefix: string = "leaf"): LeafId {
  _nextLeafCounter += 1;
  return asLeafId(`${prefix}-${_nextLeafCounter}-${Date.now().toString(36)}`);
}

// ─── Tab handles ─────────────────────────────────────────────────

/** Identifier for a draggable tab inside a leaf's strip. */
export type TabHandle = `tab:${string}:${string}`;

export function tabHandle(leafId: WorkspaceLeafId, viewId: ViewId): TabHandle {
  return `tab:${leafId}:${viewId}` as TabHandle;
}

/**
 * Parse a tab handle. Returns `null` if the input doesn't have the right
 * shape — callers can use the result to discriminate "we own this id" from
 * "some other dnd-kit drag source".
 */
export function parseTabHandle(
  handle: string,
): { leafId: WorkspaceLeafId; viewId: ViewId } | null {
  if (!handle.startsWith("tab:")) return null;
  const rest = handle.slice(4);
  const colonIdx = rest.indexOf(":");
  if (colonIdx < 0) return null;
  return {
    leafId: rest.slice(0, colonIdx),
    viewId: rest.slice(colonIdx + 1) as ViewId,
  };
}

// ─── Tab slot drop targets ────────────────────────────────────────

/** Identifier for a tab-strip drop slot (the gap between two tabs). */
export type TabSlotHandle = `tab-slot:${string}:${number}`;

export function tabSlotHandle(leafId: WorkspaceLeafId, index: number): TabSlotHandle {
  return `tab-slot:${leafId}:${index}` as TabSlotHandle;
}

export function parseTabSlotHandle(
  handle: string,
): { leafId: WorkspaceLeafId; index: number } | null {
  if (!handle.startsWith("tab-slot:")) return null;
  const rest = handle.slice(9);
  const colonIdx = rest.lastIndexOf(":");
  if (colonIdx < 0) return null;
  const idx = Number(rest.slice(colonIdx + 1));
  if (!Number.isFinite(idx)) return null;
  return {
    leafId: rest.slice(0, colonIdx),
    index: idx,
  };
}

// ─── Leaf drop handles ───────────────────────────────────────────

/** Identifier for a whole-leaf drop target (used by cross-leaf drops). */
export type LeafDropHandle = `leaf-drop:${string}`;

export function leafDropHandle(leafId: WorkspaceLeafId): LeafDropHandle {
  return `leaf-drop:${leafId}` as LeafDropHandle;
}

export function parseLeafDropHandle(handle: string): WorkspaceLeafId | null {
  if (!handle.startsWith("leaf-drop:")) return null;
  return handle.slice(10);
}
