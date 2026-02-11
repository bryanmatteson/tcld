"use client";

import { type ReactNode } from "react";
import { create } from "zustand";
import type { LocationIndex } from "@/lib/location-index";
import { emptyLocation, resolveLocation, type Location } from "@/lib/location-resolve";
import { MAX_HISTORY, isJumpLocation, sameLocation } from "@/lib/navigation-history";

export interface Selection {
  kind: "set" | "range";
  instIdxs: number[] | { start: number; end: number };
}

interface LocationState {
  index: LocationIndex | null;
  cursor: Location;
  execution: Location | null;
  selection: Selection | null;
  historyBack: Location[];
  historyForward: Location[];
  marks: Record<string, Location>;
  pinA: Location | null;
  pinB: Location | null;

  setIndex: (index: LocationIndex | null) => void;
  setCursor: (
    partial: Partial<Location>,
    opts?: { source?: "user" | "history" | "execution-sync"; pushHistory?: boolean },
  ) => void;
  setExecution: (partial: Partial<Location> | null) => void;
  setSelection: (selection: Selection | null) => void;
  goBack: () => boolean;
  goBackTo: (offset: number) => boolean;
  goForward: () => boolean;
  clearHistory: () => void;
  setMark: (name: string) => boolean;
  goToMark: (name: string) => boolean;
  clearMark: (name: string) => void;
  setPinA: (loc: Location | null) => void;
  setPinB: (loc: Location | null) => void;
  togglePinA: () => boolean;
  togglePinB: () => boolean;

  jumpToInstruction: (
    idx: number,
    opts?: { source?: "user" | "history" | "execution-sync"; pushHistory?: boolean },
  ) => void;
  jumpToGroup: (group: number) => void;
  jumpToBlock: (blockId: number) => void;
  jumpToPc: (pc: number) => void;
}

function cloneLocation(loc: Location): Location {
  return { ...loc };
}

function normalizeMarkName(name: string): string | null {
  const normalized = name.trim().toLowerCase();
  return /^[a-z]$/.test(normalized) ? normalized : null;
}

function resolveIntoIndex(index: LocationIndex, partial: Partial<Location>): Location {
  return resolveLocation(
    {
      ...partial,
      scopeId: partial.scopeId ?? index.scopeId,
    },
    index,
  );
}

function remapForIndex(index: LocationIndex, loc: Location | null): Location | null {
  if (!loc) return null;
  const remapped = resolveIntoIndex(index, loc);
  return remapped.instIdx != null ? remapped : null;
}

const DEFAULT_STATE = {
  index: null,
  cursor: emptyLocation(null),
  execution: null,
  selection: null,
  historyBack: [],
  historyForward: [],
  marks: {},
  pinA: null,
  pinB: null,
};

export const useLocationStore = create<LocationState>((set, get) => ({
  ...DEFAULT_STATE,

  setIndex: (index) => {
    set((prev) => {
      if (!index) {
        return {
          ...DEFAULT_STATE,
          cursor: emptyLocation(null),
        };
      }

      const scopeChanged = prev.index?.scopeId !== index.scopeId;
      const nextCursor = resolveLocation(
        !scopeChanged && prev.cursor.instIdx != null
          ? { ...prev.cursor, scopeId: index.scopeId }
          : { scopeId: index.scopeId, instIdx: 0 },
        index,
      );

      const nextExecution = !scopeChanged
        ? remapForIndex(index, prev.execution)
        : null;

      const nextHistoryBack = scopeChanged
        ? []
        : prev.historyBack
          .map((entry) => remapForIndex(index, entry))
          .filter((entry): entry is Location => entry !== null);

      const nextHistoryForward = scopeChanged
        ? []
        : prev.historyForward
          .map((entry) => remapForIndex(index, entry))
          .filter((entry): entry is Location => entry !== null);

      const nextMarks = scopeChanged
        ? {}
        : Object.fromEntries(
          Object.entries(prev.marks)
            .map(([name, loc]) => {
              const remapped = remapForIndex(index, loc);
              return remapped ? [name, remapped] : null;
            })
            .filter((entry): entry is [string, Location] => entry !== null),
        );

      const nextPinA = scopeChanged ? null : remapForIndex(index, prev.pinA);
      const nextPinB = scopeChanged ? null : remapForIndex(index, prev.pinB);

      return {
        ...prev,
        index,
        cursor: nextCursor,
        execution: nextExecution,
        historyBack: nextHistoryBack,
        historyForward: nextHistoryForward,
        marks: nextMarks,
        pinA: nextPinA,
        pinB: nextPinB,
      };
    });
  },

  setCursor: (partial, opts) => {
    set((prev) => {
      if (!prev.index) {
        const nextCursor = {
          ...prev.cursor,
          ...partial,
        };
        return {
          ...prev,
          cursor: nextCursor,
        };
      }

      const nextCursor = resolveIntoIndex(prev.index, {
        ...prev.cursor,
        ...partial,
      });
      if (sameLocation(prev.cursor, nextCursor)) {
        return prev;
      }

      const shouldPushHistory = opts?.pushHistory ?? opts?.source !== "history";
      let nextHistoryBack = prev.historyBack;
      let nextHistoryForward = prev.historyForward;
      if (shouldPushHistory && isJumpLocation(prev.cursor, nextCursor)) {
        nextHistoryBack = [...prev.historyBack, cloneLocation(prev.cursor)].slice(-MAX_HISTORY);
        nextHistoryForward = [];
      }

      return {
        ...prev,
        cursor: nextCursor,
        historyBack: nextHistoryBack,
        historyForward: nextHistoryForward,
      };
    });
  },

  setExecution: (partial) => {
    set((prev) => {
      if (!partial) {
        return {
          ...prev,
          execution: null,
        };
      }

      if (!prev.index) {
        return {
          ...prev,
          execution: {
            ...emptyLocation(null),
            ...partial,
          },
        };
      }

      return {
        ...prev,
        execution: resolveLocation(
          {
            ...partial,
            scopeId: partial.scopeId ?? prev.index.scopeId,
          },
          prev.index,
        ),
      };
    });
  },

  setSelection: (selection) => {
    set((prev) => ({
      ...prev,
      selection,
    }));
  },

  goBack: () => {
    const snapshot = get();
    const target = snapshot.historyBack[snapshot.historyBack.length - 1];
    if (!target) return false;
    const resolved = snapshot.index ? resolveIntoIndex(snapshot.index, target) : target;
    if (resolved.instIdx == null) return false;

    set((prev) => {
      const prevTarget = prev.historyBack[prev.historyBack.length - 1];
      if (!prevTarget) return prev;
      const prevResolved = prev.index ? resolveIntoIndex(prev.index, prevTarget) : prevTarget;
      if (prevResolved.instIdx == null) return prev;
      return {
        ...prev,
        cursor: prevResolved,
        historyBack: prev.historyBack.slice(0, -1),
        historyForward: [...prev.historyForward, cloneLocation(prev.cursor)].slice(-MAX_HISTORY),
      };
    });
    return true;
  },

  goBackTo: (offset) => {
    if (!Number.isFinite(offset) || offset <= 0) return false;
    const snapshot = get();
    const safeOffset = Math.floor(offset);
    if (safeOffset > snapshot.historyBack.length) return false;
    const targetIdx = snapshot.historyBack.length - safeOffset;
    const target = snapshot.historyBack[targetIdx];
    if (!target) return false;
    const resolved = snapshot.index ? resolveIntoIndex(snapshot.index, target) : target;
    if (resolved.instIdx == null) return false;

    set((prev) => {
      if (safeOffset > prev.historyBack.length) return prev;

      const prevTargetIdx = prev.historyBack.length - safeOffset;
      const prevTarget = prev.historyBack[prevTargetIdx];
      if (!prevTarget) return prev;

      const prevResolved = prev.index ? resolveIntoIndex(prev.index, prevTarget) : prevTarget;
      if (prevResolved.instIdx == null) return prev;

      const newer = prev.historyBack.slice(prevTargetIdx + 1);
      return {
        ...prev,
        cursor: prevResolved,
        historyBack: prev.historyBack.slice(0, prevTargetIdx),
        historyForward: [
          ...prev.historyForward,
          cloneLocation(prev.cursor),
          ...newer.map(cloneLocation).reverse(),
        ].slice(-MAX_HISTORY),
      };
    });
    return true;
  },

  goForward: () => {
    const snapshot = get();
    const target = snapshot.historyForward[snapshot.historyForward.length - 1];
    if (!target) return false;
    const resolved = snapshot.index ? resolveIntoIndex(snapshot.index, target) : target;
    if (resolved.instIdx == null) return false;

    set((prev) => {
      const prevTarget = prev.historyForward[prev.historyForward.length - 1];
      if (!prevTarget) return prev;
      const prevResolved = prev.index ? resolveIntoIndex(prev.index, prevTarget) : prevTarget;
      if (prevResolved.instIdx == null) return prev;
      return {
        ...prev,
        cursor: prevResolved,
        historyBack: [...prev.historyBack, cloneLocation(prev.cursor)].slice(-MAX_HISTORY),
        historyForward: prev.historyForward.slice(0, -1),
      };
    });
    return true;
  },

  clearHistory: () => {
    set((prev) => ({
      ...prev,
      historyBack: [],
      historyForward: [],
    }));
  },

  setMark: (name) => {
    const normalized = normalizeMarkName(name);
    if (!normalized) return false;
    const snapshot = get();
    if (snapshot.cursor.instIdx == null) return false;

    set((prev) => {
      if (prev.cursor.instIdx == null) return prev;
      return {
        ...prev,
        marks: {
          ...prev.marks,
          [normalized]: cloneLocation(prev.cursor),
        },
      };
    });
    return true;
  },

  goToMark: (name) => {
    const normalized = normalizeMarkName(name);
    if (!normalized) return false;
    const snapshot = get();
    const marked = snapshot.marks[normalized];
    if (!marked) return false;
    const target = snapshot.index ? resolveIntoIndex(snapshot.index, marked) : marked;
    if (target.instIdx == null || sameLocation(snapshot.cursor, target)) return false;

    set((prev) => {
      const prevMarked = prev.marks[normalized];
      if (!prevMarked) return prev;
      const prevTarget = prev.index ? resolveIntoIndex(prev.index, prevMarked) : prevMarked;
      if (prevTarget.instIdx == null || sameLocation(prev.cursor, prevTarget)) return prev;
      let nextHistoryBack = prev.historyBack;
      let nextHistoryForward = prev.historyForward;
      if (isJumpLocation(prev.cursor, prevTarget)) {
        nextHistoryBack = [...prev.historyBack, cloneLocation(prev.cursor)].slice(-MAX_HISTORY);
        nextHistoryForward = [];
      }
      return {
        ...prev,
        cursor: prevTarget,
        historyBack: nextHistoryBack,
        historyForward: nextHistoryForward,
      };
    });
    return true;
  },

  clearMark: (name) => {
    const normalized = normalizeMarkName(name);
    if (!normalized) return;
    set((prev) => {
      if (!prev.marks[normalized]) return prev;
      const nextMarks = { ...prev.marks };
      delete nextMarks[normalized];
      return {
        ...prev,
        marks: nextMarks,
      };
    });
  },

  setPinA: (loc) => {
    set((prev) => ({
      ...prev,
      pinA: prev.index ? remapForIndex(prev.index, loc) : (loc ? cloneLocation(loc) : null),
    }));
  },

  setPinB: (loc) => {
    set((prev) => ({
      ...prev,
      pinB: prev.index ? remapForIndex(prev.index, loc) : (loc ? cloneLocation(loc) : null),
    }));
  },

  togglePinA: () => {
    const snapshot = get();
    if (snapshot.cursor.instIdx == null) return false;

    set((prev) => {
      if (prev.cursor.instIdx == null) return prev;
      const nextPin = prev.pinA && sameLocation(prev.pinA, prev.cursor)
        ? null
        : cloneLocation(prev.cursor);
      return {
        ...prev,
        pinA: nextPin,
      };
    });
    return true;
  },

  togglePinB: () => {
    const snapshot = get();
    if (snapshot.cursor.instIdx == null) return false;

    set((prev) => {
      if (prev.cursor.instIdx == null) return prev;
      const nextPin = prev.pinB && sameLocation(prev.pinB, prev.cursor)
        ? null
        : cloneLocation(prev.cursor);
      return {
        ...prev,
        pinB: nextPin,
      };
    });
    return true;
  },

  jumpToInstruction: (idx, opts) => {
    get().setCursor({ instIdx: idx }, opts);
  },

  jumpToGroup: (group) => {
    get().setCursor({ group });
  },

  jumpToBlock: (blockId) => {
    get().setCursor({ blockId });
  },

  jumpToPc: (pc) => {
    get().setCursor({ pc });
  },
}));

export function LocationStoreProvider({ children }: { children: ReactNode }) {
  return <>{children}</>;
}
