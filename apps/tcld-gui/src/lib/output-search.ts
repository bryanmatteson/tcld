import type { OutputLine } from "@/lib/debug-store";

export interface OutputSearchState {
  query: string;
  activeMatchIndex: number;
  filterToMatches: boolean;
}

export interface OutputMatch {
  lineIndex: number;
  start: number;
  end: number;
}

export function createOutputSearchState(
  query = "",
  options: { activeMatchIndex?: number; filterToMatches?: boolean } = {},
): OutputSearchState {
  return {
    query,
    activeMatchIndex: options.activeMatchIndex ?? 0,
    filterToMatches: options.filterToMatches ?? false,
  };
}

export function searchOutput(lines: OutputLine[], state: OutputSearchState): {
  matches: OutputMatch[];
  visibleLines: OutputLine[];
} {
  const needle = state.query.trim().toLocaleLowerCase();
  if (!needle) {
    return { matches: [], visibleLines: lines };
  }

  const matches: OutputMatch[] = [];
  const matchedLineIndexes = new Set<number>();
  lines.forEach((line, lineIndex) => {
    const haystack = line.text.toLocaleLowerCase();
    let start = haystack.indexOf(needle);
    while (start >= 0) {
      matches.push({ lineIndex, start, end: start + needle.length });
      matchedLineIndexes.add(lineIndex);
      start = haystack.indexOf(needle, start + needle.length);
    }
  });

  return {
    matches,
    visibleLines: state.filterToMatches
      ? lines.filter((_, index) => matchedLineIndexes.has(index))
      : lines,
  };
}

export function formatOutputForCopy(lines: OutputLine[]): string {
  return lines.map((line) => `[${line.stream}] ${line.text}`).join("\n");
}
