"use client";

export interface RowMetrics {
  rowHeight: number;
  overscan: number;
}

const DEFAULT_ROW_HEIGHT = 24;
const DEFAULT_DISASSEMBLY_ROW_HEIGHT = 26;
const DEFAULT_OVERSCAN = 10;

function parsePxValue(raw: string): number | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  const pxMatch = trimmed.match(/(-?\d+(?:\.\d+)?)px/);
  if (pxMatch) {
    const value = Number(pxMatch[1]);
    return Number.isFinite(value) ? value : null;
  }

  const asNumber = Number.parseFloat(trimmed);
  return Number.isFinite(asNumber) ? asNumber : null;
}

export function getRowMetrics(options: { rowVar?: string; defaultRowHeight?: number } = {}): RowMetrics {
  const defaultRowHeight = options.defaultRowHeight ?? DEFAULT_ROW_HEIGHT;
  if (typeof window === "undefined") {
    return { rowHeight: defaultRowHeight, overscan: DEFAULT_OVERSCAN };
  }

  const computed = window.getComputedStyle(document.documentElement);
  const parsed = parsePxValue(computed.getPropertyValue(options.rowVar ?? "--code-row-height"));

  return {
    rowHeight: parsed ?? defaultRowHeight,
    overscan: DEFAULT_OVERSCAN,
  };
}

/** `prev` when `next` measures the same, so state setters can bail out. */
export function sameRowMetrics(prev: RowMetrics, next: RowMetrics): RowMetrics {
  return prev.rowHeight === next.rowHeight && prev.overscan === next.overscan ? prev : next;
}

export function getDisassemblyRowMetrics(): RowMetrics {
  return getRowMetrics({ rowVar: "--disassembly-row-height", defaultRowHeight: DEFAULT_DISASSEMBLY_ROW_HEIGHT });
}

/** Rows of context kept between the cursor and a list edge while stepping. */
const REVEAL_CONTEXT_ROWS = 3;

/**
 * Scroll a fixed-row-height list so row `index` is visible with a few rows of
 * context. Does nothing when the row is already comfortably in view (the
 * common case while stepping), nudges by the minimum when it is near an edge,
 * and centers rows that are a screen or more away so long jumps land with
 * surrounding code visible. Writes `scrollTop` directly, which avoids the
 * virtualizer's multi-frame scrollToIndex reconcile loop on every cursor move.
 */
export function revealRow(
  el: HTMLElement,
  index: number,
  rowHeight: number,
  options: { paddingStart?: number; contextRows?: number; center?: boolean } = {},
): void {
  const paddingStart = options.paddingStart ?? 0;
  const viewHeight = el.clientHeight - paddingStart;
  if (viewHeight <= 0 || rowHeight <= 0) return;
  const top = index * rowHeight;
  const bottom = top + rowHeight;
  const viewTop = el.scrollTop + paddingStart;
  const viewBottom = el.scrollTop + el.clientHeight;
  const context = Math.max(0, Math.min(
    (options.contextRows ?? REVEAL_CONTEXT_ROWS) * rowHeight,
    (viewHeight - rowHeight) / 2,
  ));
  const centered = top - paddingStart - (viewHeight - rowHeight) / 2;

  let next: number;
  if (options.center) {
    if (top >= viewTop + context && bottom <= viewBottom - context) return;
    next = centered;
  } else if (top >= viewTop + context && bottom <= viewBottom - context) {
    return;
  } else if (bottom < viewTop - viewHeight || top > viewBottom + viewHeight) {
    next = centered;
  } else if (top < viewTop + context) {
    next = top - paddingStart - context;
  } else {
    next = bottom - el.clientHeight + context;
  }
  const max = Math.max(0, el.scrollHeight - el.clientHeight);
  el.scrollTop = Math.round(Math.max(0, Math.min(next, max)));
}
