"use client";

import type { Location } from "@/lib/location-resolve";

export const MAX_HISTORY = 100;

export interface NavigationStacks {
  back: Location[];
  forward: Location[];
}

export interface BackForwardResult {
  target: Location | null;
  stacks: NavigationStacks;
}

function cloneLocation(loc: Location): Location {
  return { ...loc };
}

export function sameLocation(a: Location, b: Location): boolean {
  return (
    a.scopeId === b.scopeId
    && a.instIdx === b.instIdx
    && a.pc === b.pc
    && a.group === b.group
    && a.blockId === b.blockId
    && a.decompileLine === b.decompileLine
    && a.sourceFile === b.sourceFile
    && a.sourceLine === b.sourceLine
  );
}

export function isJumpLocation(from: Location, to: Location): boolean {
  if (sameLocation(from, to)) return false;
  if (from.instIdx == null || to.instIdx == null) return true;
  if (Math.abs(to.instIdx - from.instIdx) > 5) return true;
  if (from.group !== to.group) return true;
  if (from.blockId !== to.blockId) return true;
  return false;
}

export function pushBackLocation(
  stacks: NavigationStacks,
  from: Location,
  maxHistory: number = MAX_HISTORY,
): NavigationStacks {
  return {
    back: [...stacks.back, cloneLocation(from)].slice(-maxHistory),
    forward: [],
  };
}

export function goBack(
  stacks: NavigationStacks,
  current: Location,
  maxHistory: number = MAX_HISTORY,
): BackForwardResult {
  const target = stacks.back[stacks.back.length - 1] ?? null;
  if (!target) {
    return { target: null, stacks };
  }

  return {
    target: cloneLocation(target),
    stacks: {
      back: stacks.back.slice(0, -1),
      forward: [...stacks.forward, cloneLocation(current)].slice(-maxHistory),
    },
  };
}

export function goBackTo(
  stacks: NavigationStacks,
  current: Location,
  offset: number,
  maxHistory: number = MAX_HISTORY,
): BackForwardResult {
  if (!Number.isFinite(offset) || offset <= 0) {
    return { target: null, stacks };
  }

  const safeOffset = Math.floor(offset);
  if (safeOffset > stacks.back.length) {
    return { target: null, stacks };
  }

  const targetIdx = stacks.back.length - safeOffset;
  const target = stacks.back[targetIdx];
  if (!target) {
    return { target: null, stacks };
  }

  const newer = stacks.back.slice(targetIdx + 1);
  return {
    target: cloneLocation(target),
    stacks: {
      back: stacks.back.slice(0, targetIdx),
      forward: [
        ...stacks.forward,
        cloneLocation(current),
        ...newer.map(cloneLocation).reverse(),
      ].slice(-maxHistory),
    },
  };
}

export function goForward(
  stacks: NavigationStacks,
  current: Location,
  maxHistory: number = MAX_HISTORY,
): BackForwardResult {
  const target = stacks.forward[stacks.forward.length - 1] ?? null;
  if (!target) {
    return { target: null, stacks };
  }

  return {
    target: cloneLocation(target),
    stacks: {
      back: [...stacks.back, cloneLocation(current)].slice(-maxHistory),
      forward: stacks.forward.slice(0, -1),
    },
  };
}
