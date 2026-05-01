// ─── Recent Files Persistence ───────────────────────────────────────

export interface RecentFile {
  path: string;
  filename: string;
  mode: "open" | "debug";
  timestamp: number;
  instructionCount?: number;
  tclVersion?: string;
}

const STORAGE_KEY = "tcld-recent-files";
const MAX_RECENT = 5;

export function loadRecentFiles(): RecentFile[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const normalized = parsed
      .slice(0, MAX_RECENT)
      .map((entry) => {
        if (
          entry
          && typeof entry.path === "string"
          && typeof entry.filename === "string"
          && (entry.mode === "open" || entry.mode === "debug")
          && typeof entry.timestamp === "number"
        ) {
          const recent: RecentFile = {
            path: entry.path,
            filename: entry.filename,
            mode: entry.mode,
            timestamp: entry.timestamp,
          };
          if (typeof entry.instructionCount === "number" && Number.isFinite(entry.instructionCount)) {
            recent.instructionCount = entry.instructionCount;
          }
          if (typeof entry.tclVersion === "string" && entry.tclVersion.trim()) {
            recent.tclVersion = entry.tclVersion;
          }
          return recent;
        }
        return null;
      })
      .filter((entry): entry is RecentFile => entry !== null);
    return normalized;
  } catch {
    return [];
  }
}

export function addRecentFile(entry: Omit<RecentFile, "timestamp">): void {
  if (typeof window === "undefined") return;
  const existing = loadRecentFiles();
  // Remove any duplicate path+mode combo
  const filtered = existing.filter(
    (f) => !(f.path === entry.path && f.mode === entry.mode),
  );
  const next: RecentFile[] = [
    { ...entry, timestamp: Date.now() },
    ...filtered,
  ].slice(0, MAX_RECENT);
  localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
}

export function clearRecentFiles(): void {
  if (typeof window === "undefined") return;
  localStorage.removeItem(STORAGE_KEY);
}
