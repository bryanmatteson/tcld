import type { TcldData } from "@/types";
import type { Location } from "@/lib/location-resolve";

export type CmdClickNavigationResult =
  | { ok: true; target: Partial<Location>; reason: null }
  | { ok: false; target: null; reason: string };

export function resolveCmdClickNavigation(
  data: TcldData,
  hit: { tokenText: string; sourceLine: number; column: number },
): CmdClickNavigationResult {
  const token = hit.tokenText.trim();
  if (!token || token === "unknown") {
    return { ok: false, target: null, reason: `No navigation target for '${hit.tokenText}'.` };
  }

  const anchor = (data.sourceCommandAnchors ?? []).find(
    (candidate) => candidate.sourceLines[0] <= hit.sourceLine && hit.sourceLine < candidate.sourceLines[1]
      && (candidate.snippet.includes(token) || candidate.displayPath.includes(token)),
  );
  if (anchor) {
    return { ok: true, target: targetForSourceMap(data, anchor.sourceMapIndex, anchor.sourceLines[0]), reason: null };
  }

  const sourceMapIndex = (data.sourceMap ?? []).findIndex(
    (entry) => entry.sourceLines[0] <= hit.sourceLine && hit.sourceLine < entry.sourceLines[1] && entry.text.includes(token),
  );
  if (sourceMapIndex >= 0) {
    return { ok: true, target: targetForSourceMap(data, sourceMapIndex, hit.sourceLine), reason: null };
  }

  const decompileLine = data.decompileLines.find((line) => line.text.includes(token));
  if (decompileLine) {
    return {
      ok: true,
      target: {
        decompileLine: decompileLine.lineIndex,
        group: decompileLine.group,
        sourceLine: decompileLine.sourceLineStart ?? hit.sourceLine,
      },
      reason: null,
    };
  }

  return { ok: false, target: null, reason: `No navigation target for '${hit.tokenText}'.` };
}

function targetForSourceMap(data: TcldData, sourceMapIndex: number, sourceLine: number): Partial<Location> {
  const entry = data.sourceMap?.[sourceMapIndex];
  const instruction = entry
    ? data.instructions.find((inst) => inst.group === entry.commandGroup && inst.groupStart)
      ?? data.instructions.find((inst) => entry.bytecodeRange[0] <= inst.pc && inst.pc < entry.bytecodeRange[1])
    : null;
  const block = instruction ? data.blocks.find((candidate) => candidate.start <= instruction.idx && instruction.idx <= candidate.end) : null;
  return {
    sourceLine,
    decompileLine: data.decompileLines.findIndex((line) => line.sourceMapIndex === sourceMapIndex),
    group: entry?.commandGroup ?? instruction?.group ?? null,
    instIdx: instruction?.idx ?? null,
    pc: instruction?.pc ?? entry?.bytecodeRange[0] ?? null,
    blockId: block?.id ?? null,
  };
}
