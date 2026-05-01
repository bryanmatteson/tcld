import type { Instruction, SyntaxToken } from "@/types";

export interface RenderToken {
  text: string;
  role: string;
  themeToken: string;
  className: string;
}

const ROLE_THEME: Record<string, string> = {
  address: "--color-tcld-fg-dim",
  opcode: "--color-tcld-keyword",
  operand: "--color-tcld-number",
  "jump-target": "--color-tcld-accent",
  "group-label": "--color-tcld-group",
  comment: "--color-tcld-comment",
  "runtime-marker": "--color-tcld-success",
  "tcl-command": "--color-tcld-keyword",
  "tcl-variable": "--color-tcld-variable",
  "tcl-string": "--color-tcld-string",
  "tcl-number": "--color-tcld-number",
  "tcl-brace": "--color-tcld-fg-dim",
  "tcl-comment": "--color-tcld-comment",
  text: "--color-tcld-fg",
};

export function tokenizeInstructionRow(
  inst: Instruction,
  options: { groupLabel?: string | null; comment?: string | null; execution?: boolean } = {},
): RenderToken[] {
  const tokens: RenderToken[] = [
    token(`0x${inst.pc.toString(16).padStart(4, "0")}`, "address"),
    token(inst.op, "opcode"),
  ];
  if (inst.args.trim()) {
    tokens.push(token(inst.args.trim(), "operand"));
  }
  if (inst.isJump && inst.jumpTarget != null) {
    tokens.push(token(String(inst.jumpTarget), "jump-target"));
  }
  if (options.groupLabel) {
    tokens.push(token(options.groupLabel, "group-label"));
  }
  if (options.comment) {
    tokens.push(token(options.comment, "comment"));
  }
  if (options.execution) {
    tokens.push(token("execution", "runtime-marker"));
  }
  return tokens;
}

export function tokenizeSourceLine(line: string, syntaxTokens: SyntaxToken[] = []): RenderToken[] {
  if (syntaxTokens.length > 0) {
    const mapped: RenderToken[] = [];
    let cursor = 0;
    for (const syntax of syntaxTokens) {
      if (syntax.startColumn > cursor) {
        mapped.push(token(line.slice(cursor, syntax.startColumn), "text"));
      }
      mapped.push(token(line.slice(syntax.startColumn, syntax.endColumn), roleForCapture(syntax.capture)));
      cursor = Math.max(cursor, syntax.endColumn);
    }
    if (cursor < line.length) mapped.push(token(line.slice(cursor), "text"));
    if (mapped.length > 0) return mapped;
  }
  return fallbackTokenizeTcl(line);
}

function fallbackTokenizeTcl(line: string): RenderToken[] {
  const tokens: RenderToken[] = [];
  const pattern = /(\s+)|(#.*$)|(\$[A-Za-z_][\w:]*)|(\{|\}|\[|\])|("[^"]*"|'[^']*')|(\b\d+(?:\.\d+)?\b)|([A-Za-z_][\w:]*)|(\S+)/g;
  let match: RegExpExecArray | null;
  let commandSeen = false;
  while ((match = pattern.exec(line)) != null) {
    const text = match[0];
    if (match[1]) {
      tokens.push(token(text, "text"));
      continue;
    }
    if (match[2]) {
      tokens.push(token(text, "tcl-comment"));
      break;
    }
    if (match[3]) {
      tokens.push(token(text, "tcl-variable"));
      continue;
    }
    if (match[4]) {
      tokens.push(token(text, "tcl-brace"));
      continue;
    }
    if (match[5]) {
      tokens.push(token(text, "tcl-string"));
      continue;
    }
    if (match[6]) {
      tokens.push(token(text, "tcl-number"));
      continue;
    }
    if (match[7] && !commandSeen) {
      commandSeen = true;
      tokens.push(token(text, "tcl-command"));
      continue;
    }
    if (match[7]) {
      tokens.push(token(text, "tcl-string"));
      continue;
    }
    tokens.push(token(text, "text"));
  }
  return tokens;
}

function roleForCapture(capture: string): string {
  if (capture.includes("function") || capture.includes("keyword")) return "tcl-command";
  if (capture.includes("variable")) return "tcl-variable";
  if (capture.includes("string")) return "tcl-string";
  if (capture.includes("number")) return "tcl-number";
  if (capture.includes("comment")) return "tcl-comment";
  if (capture.includes("punctuation")) return "tcl-brace";
  return "text";
}

function token(text: string, role: string): RenderToken {
  return {
    text,
    role,
    themeToken: ROLE_THEME[role] ?? ROLE_THEME.text,
    className: `syntax-token-${role}`,
  };
}
