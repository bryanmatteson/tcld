"use client";

import { useMemo } from "react";
import { ScanSearch } from "lucide-react";
import { useTcldData } from "@/lib/store";
import { Panel } from "./Panel";
import type { Instruction, InstructionDetail, TraceFrame } from "@/types";
import { Field, FieldList } from "@/components/primitives/Field";
import { EmptyState } from "@/components/EmptyState";
import { useLocationStore } from "@/lib/location-store";
import { useShallow } from "zustand/react/shallow";

interface DetailPanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
}

/**
 * The Inspector describes the instruction under the shared cursor — whichever
 * view moved it (Disassembly, Source View, or the graph). It holds only facts
 * no other view shows: identity, control flow, and stack effect. Variables,
 * call stack, diagnostics, and literals each have their own view.
 *
 * Markers: data-inspector-summary (root), data-inspector-identity (header
 * card), data-inspector-field (one per property row).
 */
export function DetailPanel({ id, focused, onClose, onFocus }: DetailPanelProps) {
  const store = useTcldData();
  const { cursor } = useLocationStore(useShallow((s) => ({ cursor: s.cursor })));
  const selected = cursor.instIdx;
  const data = store.data;

  // Derived synchronously from the cursor and the loaded TcldData, so cursor
  // moves never wait on an async round-trip before the panel updates.
  const detail = useMemo<InstructionDetail | null>(() => {
    if (!store.loaded || selected == null) return null;
    return store.getDetailSync(selected);
    // store.getDetailSync is stable across renders (useCallback in the
    // store) but data is what actually invalidates the result.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected, store.loaded, data]);

  const instruction = selected != null ? data?.instructions[selected] ?? null : null;
  const trace = selected != null ? data?.traces[selected] ?? null : null;
  const fields = detail ? buildDetailFields(detail, instruction, trace) : [];

  return (
    <Panel
      designSurface="view-inspector"
      title="Inspector"
      badge={selected != null ? `i${String(selected).padStart(3, "0")}` : "—"}
      focused={focused}
      id={id}
      onClose={onClose}
      onFocus={onFocus}
    >
      <div className="flex h-full flex-col font-ui text-xs" data-inspector-summary="true">
        {detail ? (
          <div className="px-3 py-3">
            <div className="tcld-inspector-identity" data-inspector-identity="true">
              <div className="flex items-baseline gap-2">
                <span className="tcld-inspector-opcode">{detail.opcode}</span>
                <span className="ml-auto font-mono text-meta tabular-nums text-tcld-fg-dim">
                  i{String(detail.instructionIndex).padStart(3, "0")}
                </span>
              </div>
              <div className="tcld-inspector-identity-meta">
                <span className="font-mono text-meta">{hexPc(detail.pc)}</span>
                <span aria-hidden>·</span>
                <span>{detail.size} {detail.size === 1 ? "byte" : "bytes"}</span>
                {detail.commandGroup !== "n/a" && (
                  <>
                    <span aria-hidden>·</span>
                    <span>command <span className="font-mono text-meta text-tcld-keyword">{detail.commandGroup}</span></span>
                  </>
                )}
              </div>
              {instruction?.comment && !instruction.isJump && (
                <div className="tcld-inspector-annotation font-mono text-meta" title={instruction.comment}>
                  {instruction.comment}
                </div>
              )}
            </div>
            <FieldList className="mt-2">
              {fields.map((row) => (
                <Field
                  key={row.id}
                  data-inspector-field={row.id}
                  label={row.label}
                  value={row.value}
                  tone={row.tone}
                  mono={row.mono ?? true}
                  wrap={row.wrap}
                  labelWidth={92}
                />
              ))}
            </FieldList>
          </div>
        ) : (
          <EmptyState
            icon={<ScanSearch aria-hidden />}
            title={store.loaded ? "No instruction selected" : "Nothing to inspect"}
            message={store.loaded
              ? "Select an instruction in Disassembly, Source View, or the graph."
              : "Open Tcl bytecode or source to inspect its instructions."}
          />
        )}
      </div>
    </Panel>
  );
}

type DetailFieldRow = {
  id: string;
  label: string;
  value: React.ReactNode;
  tone: NonNullable<Parameters<typeof Field>[0]["tone"]>;
  mono?: boolean;
  wrap?: boolean;
};

function hexPc(pc: number): string {
  return `0x${pc.toString(16).padStart(4, "0")}`;
}

function quoteValues(values: string[]): string {
  return values.map((value) => JSON.stringify(value)).join(", ");
}

function Address({ pc }: { pc: number }) {
  return <span className="font-mono text-meta">{hexPc(pc)}</span>;
}

function describeControlFlow(detail: InstructionDetail, instruction: Instruction | null): React.ReactNode {
  const target = instruction?.jumpTarget ?? null;
  const next = instruction?.fallthrough ?? null;
  if (detail.jumps.unconditional && target != null) return <>Jumps to <Address pc={target} /></>;
  if (detail.jumps.conditional && target != null) {
    return next != null
      ? <>Branches to <Address pc={target} />, otherwise <Address pc={next} /></>
      : <>Branches to <Address pc={target} /></>;
  }
  if (next != null) return <>Continues at <Address pc={next} /></>;
  return "Exits — no following instruction";
}

/**
 * Inspector rows in reading order. Labels are plain language; values keep
 * bytecode notation (hex addresses, literal indices) so they match the
 * disassembly the user is cross-referencing.
 */
function buildDetailFields(
  detail: InstructionDetail,
  instruction: Instruction | null,
  trace: TraceFrame | null,
): DetailFieldRow[] {
  const rows: DetailFieldRow[] = [
    {
      id: "operands",
      label: "Operands",
      value: detail.operands.length > 0 ? detail.operands.join(", ") : "none",
      tone: detail.operands.length > 0 ? "number" : "muted",
    },
  ];
  if (detail.argc > 0) {
    rows.push({ id: "arguments", label: "Arguments", value: String(detail.argc), tone: "number" });
  }
  if (instruction?.commandPcRange) {
    const [start, end] = instruction.commandPcRange;
    rows.push({ id: "command-span", label: "Command span", value: `${hexPc(start)} – ${hexPc(end)}`, tone: "default" });
  }
  rows.push({
    id: "control-flow",
    label: "Control flow",
    value: describeControlFlow(detail, instruction),
    tone: detail.jumps.conditional || detail.jumps.unconditional ? "keyword" : "default",
    mono: false,
    wrap: true,
  });
  if (trace) {
    const delta = trace.stackDelta > 0 ? `+${trace.stackDelta}` : String(trace.stackDelta);
    rows.push({ id: "stack-depth", label: "Stack depth", value: `${trace.beforeDepth} → ${trace.afterDepth} (${delta})`, tone: "default" });
    if (trace.popped.length > 0) {
      rows.push({ id: "pops", label: "Pops", value: quoteValues(trace.popped), tone: "string", wrap: true });
    }
    if (trace.pushed.length > 0) {
      rows.push({ id: "pushes", label: "Pushes", value: quoteValues(trace.pushed), tone: "string", wrap: true });
    }
  }
  return rows;
}
