"use client";

import React, { useCallback, useState } from "react";
import clsx from "clsx";
import { Panel } from "./Panel";
import { EmptyState, PanelEmpty } from "@/components/EmptyState";
import { useDebugStore } from "@/lib/debug-store";
import { useTcldData } from "@/lib/store";
import { useLocationStore } from "@/lib/location-store";
import { useShallow } from "zustand/react/shallow";
import { resolveBreakpointTarget } from "@/lib/workflow-contracts";
import { DEBUGGER_UNAVAILABLE_MESSAGE } from "@/lib/capabilities";
import { Plus, X } from "lucide-react";
import { ViewIcon } from "@/components/ViewIcon";
import { Chip } from "@/components/primitives/Chip";
import type { DebugBreakpoint } from "@/types";

// ─── Panel ──────────────────────────────────────────────────────

interface BreakpointsPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

function isSourceBreakpoint(bp: DebugBreakpoint): bp is DebugBreakpoint & { file: string; line: number } {
  return bp.kind !== "instruction" && typeof bp.file === "string" && typeof bp.line === "number";
}

function breakpointKey(bp: DebugBreakpoint, index: number): string {
  if (bp.kind === "instruction" || bp.pc != null) return `instruction:${bp.pc ?? index}`;
  return `source:${bp.file ?? ""}:${bp.line ?? index}:${bp.offset ?? "line"}`;
}

function formatPc(pc: number | undefined): string {
  return pc == null ? "pc ?" : `pc 0x${pc.toString(16).padStart(4, "0")}`;
}

export const BreakpointsPanel = React.memo(function BreakpointsPanel({
  focused,
  id,
  onClose,
  onFocus,
}: BreakpointsPanelProps) {
  const debug = useDebugStore();
  // Reads breakpoints directly from the store; the store keeps this list in
  // sync via Tauri events (debugger:breakpoint:set/cleared/changed). Panels
  // should never re-fetch on mount — the previous refreshBreakpoints() in
  // useEffect was a redundant round-trip that lost a race with the event
  // stream.
  const { breakpoints, clearBreakpoint, clearAllBreakpoints, toggleBreakpoint, setBreakpoint, debuggerAvailable } = debug;
  const { data } = useTcldData();
  const { cursor } = useLocationStore(useShallow((s) => ({ cursor: s.cursor })));
  // Same target the toolbar's toggle uses: the selected instruction's source
  // line, else where execution stopped.
  const addTarget = resolveBreakpointTarget({
    data,
    selectedInstructionIndex: cursor.instIdx,
    stoppedAt: debug.stoppedAt,
    scriptPath: debug.scriptPath,
  });

  const [editingBp, setEditingBp] = useState<{
    file: string;
    line: number;
    offset?: number;
    len?: number;
  } | null>(null);
  const [conditionValue, setConditionValue] = useState("");

  const handleRemove = useCallback(
    (file: string, line: number, offset?: number, len?: number) => {
      clearBreakpoint(file, line, offset == null || len == null ? undefined : { offset, len });
    },
    [clearBreakpoint],
  );

  const handleToggleEnabled = useCallback(
    (file: string, line: number, offset?: number, len?: number) => {
      toggleBreakpoint(file, line, offset == null || len == null ? undefined : { offset, len });
    },
    [toggleBreakpoint],
  );

  const startEditCondition = (
    file: string,
    line: number,
    currentCondition?: string,
    offset?: number,
    len?: number,
  ) => {
    setEditingBp({ file, line, offset, len });
    setConditionValue(currentCondition || "");
  };

  const saveCondition = () => {
    if (!editingBp) return;
    setBreakpoint(
      editingBp.file,
      editingBp.line,
      conditionValue || undefined,
      editingBp.offset == null || editingBp.len == null
        ? undefined
        : { offset: editingBp.offset, len: editingBp.len },
    );
    setEditingBp(null);
  };

  const cancelEdit = () => {
    setEditingBp(null);
  };

  const basename = (path: string) => {
    const parts = path.split("/");
    return parts[parts.length - 1] || path;
  };

  const isEditing = (file: string, line: number, offset?: number) =>
    editingBp?.file === file
    && editingBp?.line === line
    && editingBp?.offset === offset;
  const activeCount = breakpoints.filter((bp) => bp.enabled).length;

  return (
    <Panel
      designSurface="debug-breakpoints"
      id={id}
      title="Breakpoints"
      icon={<ViewIcon viewId="breakpoints" />}
      badge={breakpoints.length > 0 ? String(breakpoints.length) : undefined}
      focused={focused}
      onClose={onClose}
      onFocus={onFocus}
    >
      {!debuggerAvailable ? (
        // Capability-aware empty state. Uses the same selector contract as the
        // other debug panels (data-debug-empty-state="unavailable").
        <div className="h-full" data-debug-empty-state="unavailable">
          <EmptyState
            size="panel"
            title="Debugger unavailable"
            message={`${DEBUGGER_UNAVAILABLE_MESSAGE}.`}
          />
        </div>
      ) : breakpoints.length === 0 ? (
        <PanelEmpty>No breakpoints set</PanelEmpty>
      ) : (
        <>
          {/*
           * Toolbar — sits above the row stack. Counts on the left, add /
           * clear-all on the right. Uses the panel-toolbar token vocabulary
           * (tcld-toolbar-pill / tcld-toolbar-meta) shared with the other
           * workbench surfaces.
           */}
          <div className="flex h-8 min-w-0 items-center gap-2 border-b border-tcld-border-dim/60 bg-tcld-bg-1/35 px-2">
            <span className="tcld-toolbar-meta">
              {activeCount} active · {breakpoints.length} total
            </span>
            <div className="flex-1" />
            <button
              type="button"
              className={clsx("tcld-toolbar-pill gap-1", !addTarget && "is-disabled")}
              disabled={!addTarget}
              onClick={() => {
                if (addTarget) void setBreakpoint(addTarget.file, addTarget.line);
              }}
              title={addTarget
                ? `Add a breakpoint at ${addTarget.file.split("/").pop()}:${addTarget.line}`
                : "Select an instruction with a source line to add a breakpoint"}
            >
              <Plus size={12} aria-hidden /> Add
            </button>
            <button
              type="button"
              className={clsx("tcld-toolbar-pill", breakpoints.length === 0 && "is-disabled")}
              disabled={breakpoints.length === 0}
              onClick={clearAllBreakpoints}
              title="Clear all breakpoints"
            >
              Clear all
            </button>
          </div>

          {/*
           * Rows — workbench-mock `.bp-row` shape: dot (toggle) + name +
           * loc (line · hits). The dot is the toggle button itself, matching
           * the mock's `dot.off` for disabled. The condition is editable
           * inline on the same row when the user activates it; the row
           * height stays bp-row-fixed so the list keeps its rhythm.
           */}
          {breakpoints.map((bp, index) => {
            const sourceBp = isSourceBreakpoint(bp);
            const instructionBp = bp.kind === "instruction" || bp.pc != null;
            const editing = sourceBp && isEditing(bp.file, bp.line, bp.offset);
            const displayName = instructionBp
              ? "instruction breakpoint"
              : basename(bp.file ?? "source breakpoint");
            const locationLabel = instructionBp
              ? formatPc(bp.pc)
              : `:${bp.line ?? "?"}${bp.offset == null ? "" : ` · @${bp.offset}`}`;
            return (
              <div
                key={breakpointKey(bp, index)}
                className="bp-row"
                data-bp-enabled={bp.enabled ? "true" : "false"}
                data-bp-kind={instructionBp ? "instruction" : "source"}
              >
                <button
                  type="button"
                  className={clsx("dot", !bp.enabled && "off")}
                  onClick={() => sourceBp && handleToggleEnabled(bp.file, bp.line, bp.offset, bp.len)}
                  disabled={!sourceBp}
                  aria-label={bp.enabled ? "Disable breakpoint" : "Enable breakpoint"}
                  title={sourceBp
                    ? (bp.enabled ? "Disable breakpoint" : "Enable breakpoint")
                    : "Instruction breakpoint toggling is managed by the opcode runtime"}
                />

                <span className="name">
                  <span className="file">{displayName}</span>
                  {editing ? (
                    <input
                      autoFocus
                      className="rounded border border-tcld-accent/40 bg-tcld-bg-1 px-1 py-0.5 text-meta font-mono text-tcld-fg outline-none"
                      placeholder="condition…"
                      value={conditionValue}
                      onChange={(e) => setConditionValue(e.target.value)}
                      onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") saveCondition();
                        if (e.key === "Escape") cancelEdit();
                      }}
                      onBlur={saveCondition}
                    />
                  ) : bp.condition ? (
                    <button
                      type="button"
                      onClick={() =>
                        sourceBp
                        && startEditCondition(
                          bp.file,
                          bp.line,
                          bp.condition,
                          bp.offset,
                          bp.len,
                        )}
                      disabled={!sourceBp}
                      className="cursor-text"
                      title="Edit condition"
                    >
                      <Chip size="xs" tone="info">{bp.condition}</Chip>
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={() =>
                        sourceBp
                        && startEditCondition(
                          bp.file,
                          bp.line,
                          undefined,
                          bp.offset,
                          bp.len,
                        )}
                      disabled={!sourceBp}
                      className="text-tcld-fg-dim hover:text-tcld-accent text-meta"
                      title={sourceBp ? "Add condition" : "Instruction breakpoint conditions are not editable here"}
                    >
                      cond+
                    </button>
                  )}
                </span>

                <span className="loc">
                  <span>{locationLabel}</span>
                  {bp.hitCount != null && bp.hitCount > 0 && (
                    <span title={`${bp.hitCount} hit${bp.hitCount === 1 ? "" : "s"}`}>· {bp.hitCount}h</span>
                  )}
                  <button
                    type="button"
                    className="remove"
                    onClick={() => sourceBp && handleRemove(bp.file, bp.line, bp.offset, bp.len)}
                    disabled={!sourceBp}
                    aria-label="Remove breakpoint"
                    title={sourceBp ? "Remove breakpoint" : "Instruction breakpoint removal is managed by the opcode runtime"}
                  >
                    <X size={11} aria-hidden />
                  </button>
                </span>
              </div>
            );
          })}
        </>
      )}
    </Panel>
  );
});
