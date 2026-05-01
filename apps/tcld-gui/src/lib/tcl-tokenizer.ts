import { T } from "./theme";
import type { SyntaxToken } from "@/types";

export interface TclToken {
  text: string;
  color: string;
  bold?: boolean;
}

/** Map a tree-sitter capture name to a theme color + bold flag. */
function captureStyle(capture: string): { color: string; bold?: boolean } {
  if (capture.includes("keyword")) return { color: T.keyword, bold: true };
  if (capture.includes("function")) return { color: T.keyword, bold: true };
  if (capture.includes("variable")) return { color: T.variable, bold: true };
  if (capture.includes("string")) return { color: T.string };
  if (capture.includes("number")) return { color: T.number };
  if (capture.includes("boolean")) return { color: T.number };
  if (capture.includes("comment")) return { color: T.comment };
  if (capture.includes("operator")) return { color: T.fg };
  if (capture.includes("punctuation")) return { color: T.fgDim };
  return { color: T.fg };
}

/**
 * Build TclToken[] for a single line from pre-computed tree-sitter tokens.
 *
 * @param lineRow   0-based row index of the line in the full source
 * @param lineText  the text content of the line
 * @param allTokens the full document's syntax tokens (sorted by startByte)
 * @param lineStartByte  byte offset of this line within the full source
 */
export function tokenizeTclLine(
  lineRow: number,
  lineText: string,
  allTokens: SyntaxToken[],
  lineStartByte: number,
): TclToken[] {
  if (!lineText) return [];

  const lineEndByte = lineStartByte + new TextEncoder().encode(lineText).length;

  // Collect tokens that overlap this line, preferring the most specific
  // (innermost) capture when ranges overlap.
  const lineTokens: { startByte: number; endByte: number; capture: string }[] = [];
  for (const tok of allTokens) {
    if (tok.startByte >= lineEndByte) break;
    if (tok.endByte <= lineStartByte) continue;
    // Clamp to line boundaries
    lineTokens.push({
      startByte: Math.max(tok.startByte, lineStartByte),
      endByte: Math.min(tok.endByte, lineEndByte),
      capture: tok.capture,
    });
  }

  // Deduplicate overlapping ranges: later entries in the highlights query
  // are more specific, so for overlapping byte ranges prefer the last one.
  const spans: { start: number; end: number; capture: string }[] = [];
  for (const lt of lineTokens) {
    const s = lt.startByte - lineStartByte;
    const e = lt.endByte - lineStartByte;
    // If this span is entirely within the previous span, replace it (more specific).
    // If it partially overlaps, just push it.
    if (spans.length > 0) {
      const prev = spans[spans.length - 1];
      if (s >= prev.start && e <= prev.end) {
        // Split the previous span around this more-specific span
        const saved = { ...prev };
        spans.pop();
        if (s > saved.start) spans.push({ start: saved.start, end: s, capture: saved.capture });
        spans.push({ start: s, end: e, capture: lt.capture });
        if (e < saved.end) spans.push({ start: e, end: saved.end, capture: saved.capture });
        continue;
      }
    }
    spans.push({ start: s, end: e, capture: lt.capture });
  }

  // Build tokens, filling gaps with default color
  const encoder = new TextEncoder();
  const bytes = encoder.encode(lineText);
  const decoder = new TextDecoder();
  const result: TclToken[] = [];
  let cursor = 0;

  for (const span of spans) {
    if (span.start > cursor) {
      result.push({ text: decoder.decode(bytes.slice(cursor, span.start)), color: T.fg });
    }
    const style = captureStyle(span.capture);
    result.push({
      text: decoder.decode(bytes.slice(span.start, span.end)),
      color: style.color,
      bold: style.bold,
    });
    cursor = span.end;
  }

  if (cursor < bytes.length) {
    result.push({ text: decoder.decode(bytes.slice(cursor)), color: T.fg });
  }

  return result;
}
