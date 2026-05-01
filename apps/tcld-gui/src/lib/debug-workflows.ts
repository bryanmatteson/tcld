import type { DebugBreakpoint, DebugStoppedEvent, TcldData } from "@/types";

/**
 * When paused inside a precompiled bytecode (`.tbc`) target on a runtime that
 * lacks instruction-level stepping, source-line stepping cannot advance the
 * marker: Tcl's `info frame` collapses to line 1 for every command in a
 * precompiled body. When the opcode-runtime hooks are available, instruction
 * stepping can drive the disassembly highlight by PC; otherwise source-line
 * stepping still executes each command but looks frozen at line 1. Returns a
 * one-line explanation for the console, or null when the situation doesn't
 * apply.
 */
export function precompiledStepAdvisory(
  stopped: Pick<DebugStoppedEvent, "file" | "line">,
  scriptPath: string,
  capabilities: Record<string, boolean>,
): string | null {
  const isPrecompiled =
    scriptPath.toLowerCase().endsWith(".tbc") ||
    (stopped.file ?? "").toLowerCase().endsWith(".tbc");
  const canStepInstruction = capabilities.instruction_step === true;
  // Only meaningful while the marker is pinned at the precompiled body's
  // collapsed line (0 or 1); a real line means source stepping is working.
  const pinnedAtStart = (stopped.line ?? 0) <= 1;
  if (!isPrecompiled || !pinnedAtStart) return null;
  const name = (stopped.file || scriptPath).split("/").pop() || "the target";
  if (canStepInstruction) {
    return (
      `${name} is precompiled bytecode; instruction stepping is available, ` +
      `so the disassembly highlight can advance opcode-by-opcode even when ` +
      `Tcl reports source line 1 for the precompiled body.`
    );
  }
  return (
    `${name} is precompiled bytecode — Tcl reports source line 1 for every ` +
    `command in a precompiled body, and this runtime has no instruction-` +
    `stepping support, so the source marker stays at line 1 while stepping ` +
    `(each step still executes the next command). Moving the disassembly ` +
    `highlight opcode-by-opcode needs a Tcl build that exports the tcld ` +
    `opcode-runtime hooks (instruction_step capability), which this ` +
    `interpreter does not provide.`
  );
}

export function ingestLiveLoadSnapshot(
  setTcldData: (data: TcldData) => void,
  payload: TcldData,
): void {
  setTcldData(payload);
}

export function nextBreakpointEnabledState(
  breakpoints: DebugBreakpoint[],
  file: string,
  line: number,
): DebugBreakpoint | null {
  const existing = breakpoints.find((bp) =>
    bp.kind !== "instruction" && bp.file === file && bp.line === line);
  return existing ? { ...existing, enabled: !existing.enabled } : null;
}
