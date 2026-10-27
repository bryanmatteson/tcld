// ─── Opcode Category Classification ──────────────────────────────
// Maps Tcl bytecode opcode prefixes to the role they play, for coloring
// opcodes in the disassembly view. Color follows role, not frequency:
// stack plumbing stays quiet so calls, variable access, and control flow
// stand out, and red stays reserved for errors and breakpoints.

import { T } from "./theme";

export type OpcodeCategory =
  | "stack"
  | "load"
  | "store"
  | "flow"
  | "invoke"
  | "arithmetic"
  | "string"
  | "list"
  | "other";

// The first matching prefix wins, so more specific prefixes come first.
// Matching is case-sensitive against the opcodes' own camelCase names.
const CATEGORY_PREFIXES: [string, OpcodeCategory][] = [
  ["push", "stack"],
  ["pop", "stack"],
  ["dup", "stack"],
  ["over", "stack"],
  ["reverse", "stack"],
  ["nop", "stack"],
  ["startCommand", "stack"],
  ["load", "load"],
  ["store", "store"],
  ["append", "store"],
  ["lappend", "store"],
  ["incr", "store"],
  ["unset", "store"],
  ["jump", "flow"],
  ["break", "flow"],
  ["continue", "flow"],
  ["return", "flow"],
  ["done", "flow"],
  ["tailcall", "flow"],
  ["yield", "flow"],
  ["beginCatch", "flow"],
  ["endCatch", "flow"],
  ["invoke", "invoke"],
  ["callBuiltin", "invoke"],
  ["evalStk", "invoke"],
  ["exprStk", "invoke"],
  ["add", "arithmetic"],
  ["sub", "arithmetic"],
  ["mult", "arithmetic"],
  ["div", "arithmetic"],
  ["mod", "arithmetic"],
  ["expon", "arithmetic"],
  ["uminus", "arithmetic"],
  ["uplus", "arithmetic"],
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
  ["tryCvt", "arithmetic"],
  ["str", "string"],
  ["concat", "string"],
  ["list", "list"],
  ["lindex", "list"],
  ["llength", "list"],
  ["lset", "list"],
  ["foreach", "list"],
  ["dict", "list"],
];

export function getOpcodeCategory(opcode: string): OpcodeCategory {
  for (const [prefix, cat] of CATEGORY_PREFIXES) {
    if (opcode.startsWith(prefix)) return cat;
  }
  return "other";
}

export function opcodeCategoryColor(cat: OpcodeCategory): string {
  switch (cat) {
    case "stack": return T.fgMuted;
    case "load": return T.variable;
    case "store": return T.variable;
    case "flow": return T.warning;
    case "invoke": return T.keyword;
    case "arithmetic": return T.fg;
    case "string": return "var(--pink)";
    case "list": return "var(--pink)";
    case "other": return T.fg;
  }
}
