import type { Instruction, LiteralEntry } from "@/types";

/**
 * What the Disassembly shows in an instruction's operand cell: the value the
 * operand stands for, not its encoding. The raw operand stays available in the
 * row tooltip.
 */
export type OperandPresentation =
  | { kind: "literal"; value: string; quoted: boolean; index: number }
  | { kind: "jump"; target: number; backward: boolean }
  | { kind: "words"; count: number }
  | { kind: "raw"; text: string }
  | { kind: "none" };

const NUMERIC_LITERAL_KINDS = new Set(["Int", "WideInt", "Double", "Boolean", "Bignum"]);
const NUMERIC_TEXT = /^-?(?:\d+(?:\.\d+)?|0x[0-9a-f]+)$/i;

/** Literal table keyed by literal index. Payloads spell the index `idx` or `index`. */
export function indexLiterals(literals: readonly LiteralEntry[]): Map<number, LiteralEntry> {
  const map = new Map<number, LiteralEntry>();
  literals.forEach((literal, position) => {
    const candidate = literal.idx ?? (literal as LiteralEntry & { index?: number }).index;
    map.set(typeof candidate === "number" && Number.isFinite(candidate) ? candidate : position, literal);
  });
  return map;
}

export function presentOperand(
  inst: Instruction,
  literals: ReadonlyMap<number, LiteralEntry>,
): OperandPresentation {
  if (inst.isJump && inst.jumpTarget != null) {
    return { kind: "jump", target: inst.jumpTarget, backward: inst.jumpTarget <= inst.pc };
  }
  if (/^push[14]$/.test(inst.op) && inst.operands.length > 0) {
    const index = inst.operands[0];
    const literal = literals.get(index);
    // The payload's comment carries the literal text when the table lacks the entry.
    const value = literal?.value ?? (inst.comment || null);
    if (value != null) {
      const numeric = literal?.kind
        ? NUMERIC_LITERAL_KINDS.has(literal.kind)
        : NUMERIC_TEXT.test(value);
      return { kind: "literal", value, quoted: !numeric, index };
    }
  }
  // invokeStk counts words on the stack, the command name included.
  if (inst.op.startsWith("invokeStk") && inst.operands.length > 0) {
    return { kind: "words", count: inst.operands[0] };
  }
  const text = inst.args.trim();
  return text ? { kind: "raw", text } : { kind: "none" };
}

/** True when the instruction's payload comment only repeats what the operand cell shows. */
export function operandCoversComment(inst: Instruction, presentation: OperandPresentation): boolean {
  const comment = inst.comment.trim();
  if (!comment) return true;
  switch (presentation.kind) {
    case "literal":
      return comment === presentation.value;
    case "jump":
      return /^pc \d+$/.test(comment);
    case "words":
      return comment === String(presentation.count);
    case "raw":
      return comment === presentation.text;
    default:
      return false;
  }
}

/** Address text as the Disassembly's address column spells it. */
export function formatAddress(pc: number, digits = 4): string {
  return pc.toString(16).padStart(digits, "0");
}
