// Layout Persistence
// Save and load workspace layouts from localStorage.

import type { ViewStacks, WorkspaceLayoutSnapshot, WorkspaceTree } from "@/types";
import {
  normalizeLeafStack,
  pruneStacksToTree,
  workspaceFromViewTree,
} from "@/lib/default-layouts";
import { isViewId } from "@/lib/view-registry";

const STORAGE_KEY_V3 = "tcld-layout-v3";
const STORAGE_KEY_V2 = "tcld-layout-v2";
const NAMED_STORAGE_KEY = "tcld-named-layouts-v1";

export interface NamedWorkspaceLayout {
  id: string;
  name: string;
  layout: WorkspaceLayoutSnapshot;
  savedAt: number;
}

export function saveCurrentLayout(layout: WorkspaceLayoutSnapshot): void {
  try {
    localStorage.setItem(STORAGE_KEY_V3, JSON.stringify(layout));
  } catch {
    // localStorage not available (e.g., SSR)
  }
}

export function loadCurrentLayout(): WorkspaceLayoutSnapshot | null {
  try {
    const rawV3 = localStorage.getItem(STORAGE_KEY_V3);
    if (rawV3) {
      const parsed = JSON.parse(rawV3) as {
        tree?: unknown;
        stacks?: unknown;
      };
      if (isWorkspaceTree(parsed.tree) && isRecord(parsed.stacks)) {
        const stacks = sanitizeStacks(parsed.stacks);
        const prunedStacks = pruneStacksToTree(parsed.tree ?? null, stacks);
        if (parsed.tree === null) {
          return {
            tree: null,
            stacks: {},
          };
        }
        if (Object.keys(prunedStacks).length > 0) {
          return {
            tree: parsed.tree ?? null,
            stacks: prunedStacks,
          };
        }
      }
    }

    // Backward compatibility: v2 stored a view tree directly.
    const rawV2 = localStorage.getItem(STORAGE_KEY_V2);
    if (!rawV2) {
      return null;
    }
    const parsedV2 = JSON.parse(rawV2);
    if (isViewTree(parsedV2)) {
      return workspaceFromViewTree(parsedV2);
    }
    return null;
  } catch {
    return null;
  }
}

export function clearSavedLayout(): void {
  try {
    localStorage.removeItem(STORAGE_KEY_V3);
    localStorage.removeItem(STORAGE_KEY_V2);
  } catch {
    // ignore
  }
}

export function saveNamedLayout(name: string, layout: WorkspaceLayoutSnapshot): NamedWorkspaceLayout {
  const entry: NamedWorkspaceLayout = {
    id: slugifyLayoutName(name),
    name,
    layout,
    savedAt: Date.now(),
  };
  try {
    const existing = loadNamedLayouts().filter((candidate) => candidate.id !== entry.id);
    localStorage.setItem(NAMED_STORAGE_KEY, JSON.stringify([entry, ...existing]));
  } catch {
    // localStorage may be unavailable during SSR or privacy-restricted tests.
  }
  return entry;
}

export function loadNamedLayouts(): NamedWorkspaceLayout[] {
  try {
    const raw = localStorage.getItem(NAMED_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((entry) => {
        if (!isRecord(entry) || typeof entry.id !== "string" || typeof entry.name !== "string" || typeof entry.savedAt !== "number") {
          return null;
        }
        const layout = entry.layout;
        if (!isRecord(layout) || !isWorkspaceTree(layout.tree) || !isRecord(layout.stacks)) {
          return null;
        }
        const stacks = pruneStacksToTree(layout.tree ?? null, sanitizeStacks(layout.stacks));
        if (layout.tree !== null && Object.keys(stacks).length === 0) {
          return null;
        }
        return {
          id: entry.id,
          name: entry.name,
          savedAt: entry.savedAt,
          layout: {
            tree: layout.tree ?? null,
            stacks,
          },
        } satisfies NamedWorkspaceLayout;
      })
      .filter((entry): entry is NamedWorkspaceLayout => entry !== null);
  } catch {
    return [];
  }
}

function slugifyLayoutName(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
  return slug ? `saved-${slug}` : "saved-layout";
}

function sanitizeStacks(value: unknown): ViewStacks {
  if (!isRecord(value)) {
    return {};
  }

  const next: ViewStacks = {};
  for (const [leafId, rawStack] of Object.entries(value)) {
    if (!isRecord(rawStack) || !Array.isArray(rawStack.views)) {
      continue;
    }
    const views = rawStack.views.filter(isViewId);
    const activeView = isViewId(rawStack.activeView)
      ? rawStack.activeView
      : views[0];
    if (!activeView) {
      continue;
    }
    const normalized = normalizeLeafStack({
      leafId,
      views,
      activeView,
    });
    if (!normalized) {
      continue;
    }
    next[leafId] = normalized;
  }

  return next;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}

function isWorkspaceTree(value: unknown): value is WorkspaceTree | null {
  return value === null || isMosaicNode(value);
}

function isViewTree(value: unknown): value is import("@/types").ViewLayoutTree {
  return isMosaicNode(value);
}

function isMosaicNode(value: unknown): boolean {
  if (typeof value === "string") {
    return true;
  }
  if (!isRecord(value)) {
    return false;
  }
  return "direction" in value && "first" in value && "second" in value;
}
