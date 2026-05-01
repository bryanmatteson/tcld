"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useTcldStore } from "@/lib/store";
import { useDebugStore } from "@/lib/debug-store";
import { Panel } from "./Panel";
import clsx from "clsx";
import type { DebugStackFrame, DebugVariable, InstructionDetail, ViewId } from "@/types";
import { Field, FieldList } from "@/components/primitives/Field";
import { SectionEyebrow } from "@/components/primitives/SectionEyebrow";
import { Subpanels, Subpanel } from "@/components/primitives/Subpanel";
import { useLocationStore } from "@/lib/location-store";
import {
  buildInspectorDiagnosticsSummaryModel,
  buildInspectorLiteralSummaryModel,
} from "@/lib/panel-models";

interface DetailPanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
  onOpenView?: (viewId: ViewId) => void;
}

/**
 * Inspector renders its sections as a stacked sub-panel accordion — the
 * canonical workbench mock pattern. Each section has its own ALL-CAPS
 * letterspaced head bar with a meta count, and multiple sections can be
 * open at once. The "active" section (the one that owns the current
 * cursor/scope) gets the accent border-top + accent name color via the
 * Subpanel primitive.
 *
 *   - Detail      — selected instruction identity, operands, literals preview
 *   - Variables   — debugger locals/namespace/global snapshot
 *   - Stack       — debugger call stack
 *   - Diagnostics — file-level diagnostics summary
 *
 * History: previous iterations used tabs (one section at a time), and
 * before that, ad-hoc Collapsibles. The accordion shape from the design
 * system mock wins on density — the user can see Detail + Variables
 * simultaneously when stopped on a breakpoint, instead of flipping a tab
 * every time they want to cross-reference.
 *
 * Backward-compat markers preserved:
 *   - data-inspector-summary="true"     (root, parity test)
 *   - data-inspector-literals-summary   (Detail body)
 *   - data-inspector-diagnostics-summary (Diagnostics body)
 *   - data-inspector-debug-summary      (Variables / Stack bodies)
 *   - data-inspector-action             (show-all-literals / show-diagnostics)
 *   - data-inspector-section / -toggle  (still on each section + its head)
 */
type InspectorTabId = "detail" | "variables" | "stack" | "diagnostics";

export function DetailPanel({ id, focused, onClose, onFocus, onOpenView }: DetailPanelProps) {
  const store = useTcldStore();
  const debug = useDebugStore();
  const { cursor } = useLocationStore();
  const selected = cursor.instIdx;

  // Each subpanel tracks its own open/closed state. Detail opens by default
  // (it's always meaningful when an instruction is selected); the others
  // open contextually — Variables/Stack auto-open when a debug session is
  // paused, Diagnostics auto-opens when there are issues.
  const [openSections, setOpenSections] = useState<Record<InspectorTabId, boolean>>(() => ({
    detail: true,
    variables: false,
    stack: false,
    diagnostics: false,
  }));
  const toggle = (id: InspectorTabId) =>
    setOpenSections((prev) => ({ ...prev, [id]: !prev[id] }));

  // Auto-open Variables + Stack on every transition INTO "paused". Step-over
  // / step-into / continue-then-breakpoint all pass through a non-paused
  // state on the way to paused (running, pausing, …), so this fires once
  // per pause event rather than on every render. Manual user closes survive
  // until the next pause transition; we never auto-close.
  const prevSessionStateRef = useRef(debug.sessionState);
  useEffect(() => {
    const prev = prevSessionStateRef.current;
    const curr = debug.sessionState;
    if (prev !== "paused" && curr === "paused") {
      setOpenSections((sections) => ({
        ...sections,
        variables: true,
        stack: true,
      }));
    }
    prevSessionStateRef.current = curr;
  }, [debug.sessionState]);

  const data = store.data;
  const literalSummary = useMemo(
    () => buildInspectorLiteralSummaryModel(data),
    [data],
  );
  const diagnosticsSummary = useMemo(
    () => buildInspectorDiagnosticsSummaryModel(data),
    [data],
  );
  const debugActive = debug.sessionState === "paused"
    || debug.sessionState === "running"
    || debug.sessionState === "pausing"
    || debug.sessionState === "stopping";
  const hasVariables = debugActive && debug.variables.length > 0;
  const hasStack = debugActive && debug.stack.length > 0;

  // Detail is derived synchronously from the cursor + the loaded TcldData.
  // The previous useEffect+Promise+setState cycle re-rendered the panel
  // every time the cursor moved (even on cache hits) because the async
  // Promise resolution triggered a state update on the next tick. Reading
  // through getDetailSync avoids that round-trip entirely; the result is
  // memoized on `selected` and the data identity, so cursor changes that
  // map to the same row don't cause work.
  const detail = useMemo<InstructionDetail | null>(() => {
    if (!store.loaded || selected == null) return null;
    return store.getDetailSync(selected);
    // store.getDetailSync is stable across renders (useCallback in the
    // store) but data is what actually invalidates the result.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected, store.loaded, data]);

  type FieldRow = { label: string; value: string; tone: Parameters<typeof Field>[0]["tone"] };
  const fields: FieldRow[] = detail ? [
    { label: "instruction index", value: String(detail.instructionIndex), tone: "number" },
    { label: "opcode", value: detail.opcode, tone: "keyword" },
    { label: "pc", value: String(detail.pc), tone: "number" },
    { label: "size", value: String(detail.size), tone: "number" },
    { label: "argc", value: String(detail.argc), tone: "number" },
    { label: "operands", value: JSON.stringify(detail.operands), tone: "default" },
    { label: "command_group", value: detail.commandGroup, tone: detail.commandGroup === "n/a" ? "muted" : "keyword" },
    { label: "command_pc_range", value: detail.commandPcRange, tone: "default" },
    {
      label: "jumps",
      value: `cond=${detail.jumps.conditional} · uncond=${detail.jumps.unconditional}`,
      tone: "keyword",
    },
    { label: "jump_target", value: detail.jumpTarget, tone: "default" },
    { label: "fallthrough", value: detail.fallthrough, tone: "number" },
  ] : [];

  // Per-tab badge: short, glanceable count or status. The chip renders inside
  // the tab button so the user sees variable/stack/issue counts without
  // flipping tabs, mirroring how panel-header tabs surface counts elsewhere.
  const tabBadge = (tabId: InspectorTabId): { value: number | string | null; tone: "neutral" | "warn" | "accent" } => {
    switch (tabId) {
      case "detail":
        return { value: selected != null ? `i${String(selected).padStart(3, "0")}` : null, tone: "neutral" };
      case "variables":
        return debugActive ? { value: debug.variables.length, tone: "neutral" } : { value: null, tone: "neutral" };
      case "stack":
        return debugActive ? { value: debug.stack.length, tone: "neutral" } : { value: null, tone: "neutral" };
      case "diagnostics":
        return diagnosticsSummary.status === "issues"
          ? { value: diagnosticsSummary.badge, tone: "warn" }
          : { value: diagnosticsSummary.badge, tone: "neutral" };
    }
  };

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
      <div
        className="flex h-full flex-col font-mono text-xs"
        data-inspector-summary="true"
      >
        {/*
         * Stacked sub-panel accordion — the canonical workbench Inspector
         * pattern from the design system mock. Each sub-panel has its own
         * 22px ALL-CAPS letterspaced head with optional meta count, and
         * any number can be open at once. The "active" sub-panel (the one
         * matching the cursor) gets the accent border-top + accent name
         * via .tcld-subpanel.is-active in globals.css.
         */}
        <Subpanels>
          {/* ── Detail ───────────────────────────────────────────────── */}
          {(() => {
            const id: InspectorTabId = "detail";
            const badge = tabBadge(id);
            return (
              <Subpanel
                id={id}
                title="Detail"
                meta={badge.value ?? undefined}
                open={openSections.detail}
                onToggle={() => toggle(id)}
                active={selected != null}
                sectionAttrs={{ "data-inspector-section": id }}
                headAttrs={{ "data-inspector-section-toggle": id }}
              >
                <SubpanelSection sectionId={id}>
                  <div className="px-3 py-2">
                    {detail ? (
                      <>
                        {/* Identity block — stays at the top so the user always
                         * knows which instruction the rest of the section is
                         * describing. Uses FieldList for label/value rhythm. */}
                        <FieldList>
                          {fields.map((row) => (
                            <Field
                              key={row.label}
                              label={row.label + ":"}
                              value={row.value}
                              tone={row.tone}
                              labelWidth={150}
                            />
                          ))}
                        </FieldList>

                        {detail.value && (
                          <>
                            <div className="h-2" />
                            <div className="text-meta text-tcld-fg-muted leading-5">value:</div>
                            <div className="whitespace-pre-wrap pt-1 pl-3 text-meta leading-5 text-tcld-string">
                              {detail.value}
                            </div>
                          </>
                        )}
                      </>
                    ) : (
                      <div className="text-meta text-tcld-fg-dim">
                        {store.loaded ? "select an instruction" : "no file loaded"}
                      </div>
                    )}

                    {/* Literals preview — scoped to the file (same TcldData as
                     * the instruction). "show all" hands off to the Literals view. */}
                    <LiteralsPreview
                      count={literalSummary.count}
                      previewRows={literalSummary.previewRows}
                      hiddenCount={literalSummary.hiddenCount}
                      statusText={literalSummary.statusText}
                      onOpenView={onOpenView}
                    />
                  </div>
                </SubpanelSection>
              </Subpanel>
            );
          })()}

          {/* ── Variables ────────────────────────────────────────────── */}
          {(() => {
            const id: InspectorTabId = "variables";
            const badge = tabBadge(id);
            return (
              <Subpanel
                id={id}
                title="Variables"
                meta={badge.value ?? undefined}
                open={openSections.variables}
                onToggle={() => toggle(id)}
                active={hasVariables}
                sectionAttrs={{ "data-inspector-section": id }}
                headAttrs={{ "data-inspector-section-toggle": id }}
              >
                <SubpanelSection sectionId={id}>
                  {!debugActive ? (
                    <InspectorEmpty>No debug session active</InspectorEmpty>
                  ) : !hasVariables ? (
                    <InspectorEmpty>No variables in scope</InspectorEmpty>
                  ) : (
                    <DebugVariablesSummary variables={debug.variables} />
                  )}
                </SubpanelSection>
              </Subpanel>
            );
          })()}

          {/* ── Stack ────────────────────────────────────────────────── */}
          {(() => {
            const id: InspectorTabId = "stack";
            const badge = tabBadge(id);
            return (
              <Subpanel
                id={id}
                title="Stack"
                meta={badge.value ?? undefined}
                open={openSections.stack}
                onToggle={() => toggle(id)}
                active={hasStack}
                sectionAttrs={{ "data-inspector-section": id }}
                headAttrs={{ "data-inspector-section-toggle": id }}
              >
                <SubpanelSection sectionId={id}>
                  {!debugActive ? (
                    <InspectorEmpty>No debug session active</InspectorEmpty>
                  ) : !hasStack ? (
                    <InspectorEmpty>No stack frames</InspectorEmpty>
                  ) : (
                    <DebugStackSummary stack={debug.stack} />
                  )}
                </SubpanelSection>
              </Subpanel>
            );
          })()}

          {/* ── Diagnostics ──────────────────────────────────────────── */}
          {(() => {
            const id: InspectorTabId = "diagnostics";
            const badge = tabBadge(id);
            return (
              <Subpanel
                id={id}
                title="Diagnostics"
                meta={badge.value ?? undefined}
                open={openSections.diagnostics}
                onToggle={() => toggle(id)}
                active={diagnosticsSummary.status === "issues"}
                sectionAttrs={{ "data-inspector-section": id }}
                headAttrs={{ "data-inspector-section-toggle": id }}
              >
                <SubpanelSection sectionId={id}>
                  <DiagnosticsTabBody
                    summary={diagnosticsSummary}
                    onOpenView={onOpenView}
                  />
                </SubpanelSection>
              </Subpanel>
            );
          })()}
        </Subpanels>
      </div>
    </Panel>
  );
}

/**
 * Wraps each sub-panel's body in a <section> that carries the
 * `data-inspector-section-content` marker so parity selectors can assert
 * "this section's body is rendered". The open/closed contract lives on the
 * Subpanel root (`data-inspector-section` + `data-open` via sectionAttrs),
 * which stays in the DOM even while collapsed.
 */
function SubpanelSection({
  sectionId,
  children,
}: {
  sectionId: InspectorTabId;
  children: React.ReactNode;
}) {
  return (
    <section
      data-inspector-section-content={sectionId}
      className="flex flex-col"
    >
      {children}
    </section>
  );
}

/**
 * Smallest possible inline empty state for inspector tabs. The full
 * EmptyState primitive is overkill at this density — a centered, dim line
 * matches the rest of the inspector typography.
 */
function InspectorEmpty({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-full items-center justify-center px-3 py-6 text-meta text-tcld-fg-dim">
      {children}
    </div>
  );
}

/**
 * Literals preview block — pulled out so the Detail tab body stays readable.
 * Identical wire-format to the previous Collapsible body so existing
 * `data-inspector-action="show-all-literals"` and `-literals-summary`
 * selectors keep resolving.
 */
function LiteralsPreview({
  count,
  previewRows,
  hiddenCount,
  statusText,
  onOpenView,
}: {
  count: number;
  previewRows: ReturnType<typeof buildInspectorLiteralSummaryModel>["previewRows"];
  hiddenCount: number;
  statusText: string;
  onOpenView?: (viewId: ViewId) => void;
}) {
  return (
    <div
      className="mt-4 border-t border-tcld-border-dim pt-3"
      data-inspector-literals-summary="true"
    >
      <SectionEyebrow trailing={
        <span className="text-label tabular-nums text-tcld-fg-muted">{count}</span>
      }>
        Literals
      </SectionEyebrow>
      {previewRows.length > 0 ? (
        <div className="space-y-0.5">
          {previewRows.map((literal) => (
            <div
              key={`${literal.index}-${literal.kind}-${literal.value}`}
              className="grid h-5 items-center gap-2 leading-5"
              style={{ gridTemplateColumns: "24px 60px minmax(0, 1fr)" }}
            >
              <span className="text-right text-tcld-fg-dim">{literal.index}</span>
              <span className="truncate font-medium text-tcld-keyword">{literal.kind}</span>
              <span className="truncate text-tcld-string">{literal.value}</span>
            </div>
          ))}
        </div>
      ) : (
        <div className="text-tcld-fg-dim">{statusText}</div>
      )}
      <div className="mt-2 flex items-center justify-between gap-2 text-meta text-tcld-fg-muted">
        <span>
          {hiddenCount > 0 ? `${hiddenCount} more` : statusText}
        </span>
        <button
          type="button"
          data-panel-interactive="true"
          data-inspector-action="show-all-literals"
          className="text-tcld-accent hover:underline disabled:text-tcld-fg-dim disabled:no-underline"
          disabled={!onOpenView}
          onClick={(event) => {
            event.stopPropagation();
            onOpenView?.("literals");
          }}
        >
          show all
        </button>
      </div>
    </div>
  );
}

function DiagnosticsTabBody({
  summary,
  onOpenView,
}: {
  summary: ReturnType<typeof buildInspectorDiagnosticsSummaryModel>;
  onOpenView?: (viewId: ViewId) => void;
}) {
  return (
    <div className="px-3 py-3" data-inspector-diagnostics-summary="true">
      <div className={clsx(
        "mb-2 text-meta",
        summary.status === "issues" ? "text-tcld-warning" : "text-tcld-fg-dim",
      )}>
        {summary.message}
      </div>
      {summary.rows.length > 0 && (
        <div className="space-y-0.5">
          {summary.rows.map((row) => (
            <div
              key={row.kind}
              data-inspector-diagnostics-row={row.kind}
              className="grid h-5 items-center gap-2 leading-5 text-meta"
              style={{ gridTemplateColumns: "96px 32px minmax(0, 1fr)" }}
            >
              <span className="truncate text-tcld-fg-muted">{row.label}</span>
              <span className="text-right text-tcld-number">{row.count}</span>
              <span className="truncate text-tcld-fg">{row.status}</span>
            </div>
          ))}
        </div>
      )}
      <div className="mt-3 text-right">
        <button
          type="button"
          data-panel-interactive="true"
          data-inspector-action="show-diagnostics"
          className="text-meta text-tcld-accent hover:underline disabled:text-tcld-fg-dim disabled:no-underline"
          disabled={!onOpenView}
          onClick={(event) => {
            event.stopPropagation();
            onOpenView?.("diagnostics");
          }}
        >
          open diagnostics
        </button>
      </div>
    </div>
  );
}

function DebugVariablesSummary({ variables }: { variables: DebugVariable[] }) {
  return (
    <div
      className="space-y-0.5 px-3 py-2 text-meta"
      data-inspector-debug-summary="variables"
    >
      {variables.map((variable) => (
        <div
          key={`${variable.scope}-${variable.name}`}
          className="grid h-5 items-center gap-2 leading-5"
          style={{ gridTemplateColumns: "minmax(0, 96px) minmax(0, 1fr) 56px" }}
        >
          <span className="truncate text-tcld-fg-muted">{variable.name}</span>
          <span className="truncate text-tcld-string">{variable.value}</span>
          <span className="truncate text-right text-tcld-fg-dim">{variable.scope}</span>
        </div>
      ))}
    </div>
  );
}

function DebugStackSummary({ stack }: { stack: DebugStackFrame[] }) {
  return (
    <div
      className="space-y-0.5 px-3 py-2 text-meta"
      data-inspector-debug-summary="stack"
    >
      {stack.map((frame, index) => (
        <div
          key={`${frame.level}-${frame.file}-${frame.line}-${index}`}
          className="grid h-5 items-center gap-2 leading-5"
          style={{ gridTemplateColumns: "32px minmax(0, 1fr) 64px" }}
        >
          <span className="text-tcld-fg-dim">#{index}</span>
          <span className="truncate text-tcld-fg">{frame.proc ?? `<${frame.type}>`}</span>
          <span className="truncate text-right text-tcld-fg-muted">
            {frame.line != null ? `:${frame.line}` : `L${frame.level}`}
          </span>
        </div>
      ))}
    </div>
  );
}
