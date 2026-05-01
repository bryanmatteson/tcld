// ─── Number / count formatters ─────────────────────────────────
//
// These live in lib/ rather than in any individual component because
// MosaicWorkspace, Panel headers, file tabs, status bar, and welcome
// recents all need to display the same compact counts in the same way.

const COMPACT_COUNT_OVERFLOW = "999+";

/**
 * Compact count for badges and chips: returns a string for any non-negative
 * finite value, "999+" for anything >999, or `undefined` so the caller can
 * skip rendering the badge entirely.
 */
export function compactCount(value: number | null | undefined): string | undefined {
  if (value == null) return undefined;
  if (!Number.isFinite(value)) return undefined;
  if (value < 0) return undefined;
  if (value > 999) return COMPACT_COUNT_OVERFLOW;
  return String(Math.trunc(value));
}

/**
 * Locale-formatted count for prose ("1,234 inst") used in welcome cards
 * and file-tab metadata.
 */
export function formatInstructionCount(count: number | null | undefined): string {
  return typeof count === "number" && Number.isFinite(count)
    ? `${count.toLocaleString()} inst`
    : "inst unknown";
}

export function formatTclVersion(version: string | null | undefined): string {
  return version && version.trim() ? `Tcl ${version}` : "Tcl unknown";
}
