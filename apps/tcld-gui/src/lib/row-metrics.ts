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

export function getDisassemblyRowMetrics(): RowMetrics {
  return getRowMetrics({ rowVar: "--disassembly-row-height", defaultRowHeight: DEFAULT_DISASSEMBLY_ROW_HEIGHT });
}
