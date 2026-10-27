// ─── Number / count formatters ─────────────────────────────────
//
// These live in lib/ rather than in any individual component because
// MosaicWorkspace, Panel headers, file tabs, status bar, and welcome
// recents all need to display the same compact counts in the same way.

/**
 * Compact count for badges and chips: returns a string for any non-negative
 * finite value, abbreviated past 999 ("1.7k", "24k", "1.2M") so large files
 * still show their size at badge width, or `undefined` so the caller can
 * skip rendering the badge entirely.
 */
export function compactCount(value: number | null | undefined): string | undefined {
  if (value == null) return undefined;
  if (!Number.isFinite(value)) return undefined;
  if (value < 0) return undefined;
  const whole = Math.trunc(value);
  if (whole <= 999) return String(whole);
  const abbreviate = (scaled: number) => (scaled < 9.95 ? scaled.toFixed(1).replace(/\.0$/, "") : String(Math.round(scaled)));
  const thousands = whole / 1_000;
  if (Math.round(thousands) < 1_000) return `${abbreviate(thousands)}k`;
  return `${abbreviate(whole / 1_000_000)}M`;
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
