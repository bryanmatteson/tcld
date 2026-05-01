"use client";

import React from "react";
import { Panel } from "./Panel";
import { useTcldStore } from "@/lib/store";
import { useLocationStore } from "@/lib/location-store";
import { buildDiagnosticsPanelModel, sourceMapSelectionTarget } from "@/lib/panel-models";
import { ViewIcon } from "@/components/ViewIcon";
import { EmptyState } from "@/components/EmptyState";
import { Chip, SectionEyebrow } from "@/components/primitives";

interface DiagnosticsPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const DiagnosticsPanel = React.memo(function DiagnosticsPanel({ focused, id, onClose, onFocus }: DiagnosticsPanelProps) {
  const data = useTcldStore().data;
  const { cursor, setCursor } = useLocationStore();
  const model = buildDiagnosticsPanelModel(data, cursor);
  const diagnostics = model.diagnostics;
  const counts = model.issueCounts;

  return (
    <Panel id={id} title="Diagnostics" icon={<ViewIcon viewId="diagnostics" />} badge={model.reconstruction} focused={focused} designSurface="view-issues" onClose={onClose} onFocus={onFocus}>
      <div className="h-full overflow-auto p-3 font-mono text-xs text-tcld-fg" data-diagnostics-state={model.status}>
        {!diagnostics ? (
          // data-diagnostics-empty-state is the selector contract for the
          // Capability-aware no-report state. Keep it on the wrapper so the parity
          // tests can distinguish unavailable from clean.
          <div className="h-full" data-diagnostics-empty-state="unavailable">
            <EmptyState
              size="panel"
              title="Diagnostics unavailable"
              message="No diagnostics report was produced for this document."
            />
          </div>
        ) : (
          <div className="space-y-3">
            {/*
             * Severity-count summary strip. Uses the canonical chip family
             * with three semantic tones (err / warn / info). The "total" pill
             * is borderless (xs) so the count reads as a quiet label rather
             * than competing with the per-severity tonal pills.
             */}
            <div
              className="flex flex-wrap items-center gap-1.5"
              data-diagnostics-summary="severity-counts"
            >
              <Chip size="xs" tone="neutral" data-diagnostics-count="total">
                {`${counts.total} issue${counts.total === 1 ? "" : "s"}`}
              </Chip>
              <Chip size="sm" tone="err" data-diagnostics-count="err">
                {`${counts.err} err`}
              </Chip>
              <Chip size="sm" tone="warn" data-diagnostics-count="warn">
                {`${counts.warn} warn`}
              </Chip>
              <Chip size="sm" tone="info" data-diagnostics-count="info">
                {`${counts.info} info`}
              </Chip>
            </div>

            {model.issueRows.length === 0 ? (
              <div data-diagnostics-empty-state="clean">
                <EmptyState
                  size="panel"
                  title="No issues"
                  message={`Reconstruction: ${model.reconstruction}`}
                />
              </div>
            ) : (
              <section data-diagnostics-issue-list="true">
                {model.issueRows.map((row) => (
                  <button
                    key={row.id}
                    type="button"
                    data-diagnostics-issue-row={row.kind}
                    data-issue-severity={row.severity}
                    data-source-map-index={row.sourceMapIndex ?? undefined}
                    data-selected={row.selected ? "true" : undefined}
                    disabled={row.sourceMapIndex == null}
                    className="diagnostics-issue-row"
                    onClick={() => {
                      if (data && row.sourceMapIndex != null) {
                        const target = sourceMapSelectionTarget(data, row.sourceMapIndex);
                        if (target) setCursor(target, { source: "user" });
                      }
                    }}
                  >
                    <span className={`diagnostics-severity-pill diagnostics-severity-pill-${row.severity}`} data-diagnostics-severity-pill={row.severity}>{row.severityLabel}</span>
                    <span className="min-w-0">
                      <span className="diagnostics-issue-message">{row.message}</span>
                      <span className="diagnostics-issue-meta" data-diagnostics-issue-meta="true">{row.code} · {row.location}</span>
                    </span>
                  </button>
                ))}
              </section>
            )}

            <details className="diagnostics-technical-details">
              <summary className="cursor-pointer">
                <SectionEyebrow as="span" inline>Technical details</SectionEyebrow>
              </summary>
              <div className="mt-2 space-y-3">
                <Section title="Approximations" rows={diagnostics.approximations.map((approximation) => `${approximation.kind} · ${approximation.bytecodeRange ? `${approximation.bytecodeRange[0]}..${approximation.bytecodeRange[1]}` : "nested procedure"} · ${approximation.reason}`)} />
                <Section title="Unsupported instructions" rows={diagnostics.unsupportedInstructions} />
                <Section title="Unreduced opcodes" rows={diagnostics.unreducedOpcodes} />
                <Section title="Suppressed fast paths" rows={diagnostics.suppressedFastPathRanges.map((range) => `${range[0]}..${range[1]}`)} />
                <Section title="Pass decisions" rows={diagnostics.passDecisions.map((decision) => `${decision.name} iteration=${decision.iteration} enabled=${decision.enabled} changed=${decision.changed}`)} />
                <Section title="Recognized constructs" rows={diagnostics.recognizedConstructs} />
              </div>
            </details>

            <section>
              <SectionEyebrow trailing={
                model.linkedRows.length > 0
                  ? <Chip size="xs" tone="neutral">{model.linkedRows.length}</Chip>
                  : null
              }>
                Linked evidence
              </SectionEyebrow>
              {model.linkedRows.length > 0 ? model.linkedRows.map((row, index) => (
                <button
                  key={`${row.kind}-${index}`}
                  type="button"
                  data-source-map-index={row.sourceMapIndex ?? undefined}
                  data-selected={row.selected ? "true" : undefined}
                  className={`block w-full px-1 py-0.5 text-left ${row.selected ? "bg-tcld-bg-highlight text-tcld-accent" : "text-tcld-fg-dim hover:bg-tcld-bg-highlight/60"}`}
                  onClick={() => {
                    if (data && row.sourceMapIndex != null) {
                      const target = sourceMapSelectionTarget(data, row.sourceMapIndex);
                      if (target) setCursor(target, { source: "user" });
                    }
                  }}
                >
                  {row.kind}: {row.label}{row.sourceMapIndex != null ? ` (#${row.sourceMapIndex})` : " (unmapped)"}
                </button>
              )) : <div className="text-tcld-fg-dim">none</div>}
            </section>
          </div>
        )}
      </div>
    </Panel>
  );
});

function Section({ title, rows }: { title: string; rows: string[] }) {
  return (
    <section>
      <SectionEyebrow trailing={rows.length > 0 ? <Chip size="xs" tone="neutral">{rows.length}</Chip> : null}>
        {title}
      </SectionEyebrow>
      {rows.length > 0
        ? rows.map((row, index) => <div key={`${title}-${index}`}>{row}</div>)
        : <div className="text-tcld-fg-dim">none</div>}
    </section>
  );
}
