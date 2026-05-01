// ─── Opcode Category Classification ──────────────────────────────
// Maps Tcl bytecode opcode prefixes to semantic categories for
// syntax highlighting in the disassembly view.

import { T } from "./theme";

export type OpcodeCategory = "push" | "load" | "store" | "jump" | "invoke" | "arithmetic" | "string" | "list" | "control" | "other";

const CATEGORY_PREFIXES: [string, OpcodeCategory][] = [
  ["push", "push"],
  ["load", "load"],
  ["store", "store"],
  ["append", "store"],
  ["lappend", "store"],
  ["jump", "jump"],
  ["startCommand", "control"],
  ["invoke", "invoke"],
  ["callBuiltin", "invoke"],
  ["add", "arithmetic"],
  ["sub", "arithmetic"],
  ["mult", "arithmetic"],
  ["div", "arithmetic"],
  ["mod", "arithmetic"],
  ["incr", "arithmetic"],
  ["bitnot", "arithmetic"],
  ["bitand", "arithmetic"],
  ["bitor", "arithmetic"],
  ["bitxor", "arithmetic"],
  ["lshift", "arithmetic"],
  ["rshift", "arithmetic"],
  ["eq", "arithmetic"],
  ["neq", "arithmetic"],
  ["lt", "arithmetic"],
  ["gt", "arithmetic"],
  ["le", "arithmetic"],
  ["ge", "arithmetic"],
  ["not", "arithmetic"],
  ["land", "arithmetic"],
  ["lor", "arithmetic"],
  ["str", "string"],
  ["concat", "string"],
  ["list", "list"],
  ["foreach", "list"],
  ["dict", "list"],
  ["return", "control"],
  ["done", "control"],
  ["pop", "control"],
  ["dup", "control"],
  ["nop", "control"],
  ["try", "control"],
  ["begin", "control"],
  ["end", "control"],
];

export function getOpcodeCategory(opcode: string): OpcodeCategory {
  const lower = opcode.toLowerCase();
  for (const [prefix, cat] of CATEGORY_PREFIXES) {
    if (lower.startsWith(prefix)) return cat;
  }
  return "other";
}

export function opcodeCategoryColor(cat: OpcodeCategory): string {
  switch (cat) {
    case "push": return T.string;
    case "load": return T.variable;
    case "store": return T.variable;
    case "jump": return T.error;
    case "invoke": return T.keyword;
    case "arithmetic": return T.number;
    case "string": return T.string;
    case "list": return T.accent;
    case "control": return T.fgMuted;
    case "other": return T.fg;
  }
}
