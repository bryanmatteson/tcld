// ─── Annotation Persistence ──────────────────────────────────────
// Bookmarks and inline comments, persisted to localStorage keyed
// by a fingerprint of the loaded bytecode file.

export interface Annotations {
  bookmarks: Set<number>;
  comments: Map<number, string>;
}

const STORAGE_PREFIX = "tcld-annotations-";

function fileHash(filename: string, instructionCount: number, codeBytes: number): string {
  return `${filename}:${instructionCount}:${codeBytes}`;
}

function storageKey(hash: string): string {
  return `${STORAGE_PREFIX}${hash}`;
}

export function loadAnnotations(filename: string, instructionCount: number, codeBytes: number): Annotations {
  const key = storageKey(fileHash(filename, instructionCount, codeBytes));
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return { bookmarks: new Set(), comments: new Map() };
    const parsed = JSON.parse(raw);
    return {
      bookmarks: new Set(parsed.bookmarks ?? []),
      comments: new Map(
        Object.entries(parsed.comments ?? {}).map(([k, v]) => [Number(k), v as string]),
      ),
    };
  } catch {
    return { bookmarks: new Set(), comments: new Map() };
  }
}

export function saveAnnotations(
  filename: string,
  instructionCount: number,
  codeBytes: number,
  annotations: Annotations,
): void {
  const key = storageKey(fileHash(filename, instructionCount, codeBytes));
  try {
    const data = {
      bookmarks: [...annotations.bookmarks],
      comments: Object.fromEntries(annotations.comments),
    };
    localStorage.setItem(key, JSON.stringify(data));
  } catch {
    // localStorage unavailable
  }
}

export function toggleBookmark(annotations: Annotations, idx: number): Annotations {
  const bookmarks = new Set(annotations.bookmarks);
  if (bookmarks.has(idx)) bookmarks.delete(idx);
  else bookmarks.add(idx);
  return { ...annotations, bookmarks };
}

export function setComment(annotations: Annotations, idx: number, text: string): Annotations {
  const comments = new Map(annotations.comments);
  if (text.trim() === "") comments.delete(idx);
  else comments.set(idx, text.trim());
  return { ...annotations, comments };
}

export const EMPTY_ANNOTATIONS: Annotations = {
  bookmarks: new Set(),
  comments: new Map(),
};
