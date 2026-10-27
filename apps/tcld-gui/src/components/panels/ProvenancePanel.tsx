"use client";

import React from "react";
import { Check, Search, X } from "lucide-react";
import { Panel } from "./Panel";
import { useTcldData } from "@/lib/store";
import { useLocationStore } from "@/lib/location-store";
import { useShallow } from "zustand/react/shallow";
import {
  buildTracePanelModel,
  sourceMapSelectionTarget,
  type TraceCommandRow,
  type TraceEvidenceRow,
  type TracePassRow,
  type TracePassStatus,
} from "@/lib/panel-models";
import { ViewIcon } from "@/components/ViewIcon";
import { EmptyState } from "@/components/EmptyState";
import { Chip } from "@/components/primitives/Chip";
import { SectionEyebrow } from "@/components/primitives/SectionEyebrow";

interface ProvenancePanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

const PASS_STATUS_LABEL: Record<TracePassStatus, string> = {
  matched: "matched",
  fixed: "fixed",
  unmapped: "unmapped",
  skipped: "skipped",
};

export const ProvenancePanel = React.memo(function ProvenancePanel({ focused, id, onClose, onFocus }: ProvenancePanelProps) {
  const data = useTcldData().data;
  const { cursor, setCursor } = useLocationStore(useShallow((s) => ({ cursor: s.cursor, setCursor: s.setCursor })));
  const [filter, setFilter] = React.useState("");

  const model = React.useMemo(() => buildTracePanelModel(data, cursor, filter), [data, cursor, filter]);
  const detail = model.detail;
  const totals = model.totals;

  const handleSelectCommand = React.useCallback((row: TraceCommandRow) => {
    if (!data) return;
    const target = sourceMapSelectionTarget(data, row.sourceMapIndex);
    if (target) setCursor(target, { source: "user" });
  }, [data, setCursor]);

  const handleEvidenceNav = React.useCallback((evidence: TraceEvidenceRow) => {
    if (!data || !evidence.target) return;
    if (evidence.target.instIdx != null) {
      setCursor({ instIdx: evidence.target.instIdx }, { source: "user" });
      return;
    }
    if (evidence.target.sourceLine != null) {
      setCursor({ sourceLine: evidence.target.sourceLine }, { source: "user" });
    }
  }, [data, setCursor]);

  return (
    <Panel
      id={id}
      title="Provenance"
      icon={<ViewIcon viewId="provenance" />}
      badge={String(totals.commands)}
      focused={focused}
      designSurface="view-trace"
      onClose={onClose}
      onFocus={onFocus}
    >
      <div className="tcld-trace-shell" data-trace-shell="true" data-trace-status={model.status}>
        <aside className="tcld-trace-pane tcld-trace-pane-left" data-trace-pane="left">
          <header className="tcld-trace-pane-header" data-trace-section="commands-header">
            <span>commands · {totals.commands}</span>
            <span className="tcld-trace-meta">{totals.changed} changed · {totals.hot} hot</span>
          </header>
          <label className="tcld-trace-filter" data-trace-section="commands-filter">
            <Search size={10} aria-hidden />
            <input
              type="text"
              value={filter}
              placeholder="Filter commands…"
              data-trace-filter-input="true"
              onChange={(event) => setFilter(event.target.value)}
            />
          </label>
          <div className="tcld-trace-command-list" role="list" data-trace-section="commands-list">
            {model.unavailableMessage ? (
              <div data-trace-empty="true">
                <EmptyState size="panel" title={model.unavailableMessage} />
              </div>
            ) : model.noMatchText ? (
              <div data-trace-empty="filter">
                <EmptyState size="panel" title={model.noMatchText} />
              </div>
            ) : model.visibleCommands.map((row) => (
              <button
                type="button"
                key={row.sourceMapIndex}
                role="listitem"
                data-trace-row="true"
                data-trace-row-id={row.id}
                data-trace-row-selected={row.selected ? "true" : "false"}
                data-trace-row-changed={row.changed ? "true" : "false"}
                data-trace-row-hot={row.hot ? "true" : "false"}
                data-trace-source-map-index={row.sourceMapIndex}
                className={`tcld-trace-command-row${row.selected ? " is-active" : ""}`}
                onClick={() => handleSelectCommand(row)}
              >
                <span className="tcld-trace-command-id">{row.id}</span>
                <span className="tcld-trace-command-body">
                  <span className="tcld-trace-command-label">{row.label}</span>
                  <span className="tcld-trace-command-meta">pc {row.pcRangeText} · {row.matched}</span>
                </span>
                <span className="tcld-trace-command-trail">
                  {row.hot ? <span className="tcld-trace-hot-dot" data-trace-hot-dot="true" aria-label="hot" /> : null}
                  <Chip
                    size="xs"
                    tone={row.changed ? "warn" : "neutral"}
                    data-trace-pass-pill={row.changed ? "changed" : "stable"}
                  >
                    {`${row.passes}p`}
                  </Chip>
                </span>
              </button>
            ))}
          </div>
          <footer className="tcld-trace-pane-footer" data-trace-section="commands-footer">
            <span>{totals.commands} commands</span>
            <span data-trace-footer-changed="true">{totals.changed} changed</span>
            <span data-trace-footer-hot="true">{totals.hot} hot</span>
          </footer>
        </aside>
        <section className="tcld-trace-pane tcld-trace-pane-right" data-trace-pane="right" data-trace-detail-available={detail.available ? "true" : "false"}>
          {!detail.available ? (
            <div data-trace-detail-empty="true">
              <EmptyState size="panel" title={detail.unavailableReason ?? "Select a command to inspect its trace."} />
            </div>
          ) : (
            <>
              <header className="tcld-trace-detail-header" data-trace-section="detail-header">
                <span className="tcld-trace-detail-id" data-trace-detail-id={detail.command.id}>{detail.command.id}</span>
                <span className="tcld-trace-detail-label">{detail.command.label}</span>
              </header>
              <dl className="tcld-trace-stats" data-trace-section="detail-stats">
                <Stat label="construct" value={detail.construct || "command"} />
                <Stat label="reconstruction" value={detail.reconstruction} />
                <Stat label="passes ran" value={String(detail.passesRan)} />
                <Stat label="source span" value={detail.sourceSpan} />
              </dl>
              <DetailSection title="Source span" data-trace-section="source-span">
                <pre className="tcld-trace-source-pre" data-trace-source-preview="true">{detail.sourcePreview}</pre>
              </DetailSection>
              <DetailSection title={`Pass chain · ${detail.passChain.length} steps`} data-trace-section="pass-chain">
                <div className="tcld-trace-chain" role="list" data-trace-chain="true">
                  {detail.passChain.length === 0 ? (
                    <div className="tcld-trace-empty" data-trace-chain-empty="true">No pass decisions recorded.</div>
                  ) : detail.passChain.map((row) => (
                    <PassChainRow key={`${row.name}-${row.iteration}-${row.index}`} row={row} />
                  ))}
                </div>
              </DetailSection>
              <DetailSection title="Linked evidence" data-trace-section="evidence">
                {detail.evidence.length === 0 ? (
                  <div className="tcld-trace-empty" data-trace-evidence-empty="true">No evidence rows for this command.</div>
                ) : (
                  <div className="tcld-trace-evidence" role="list" data-trace-evidence="true">
                    {detail.evidence.map((row, index) => (
                      <button
                        type="button"
                        key={`${row.kind}-${index}`}
                        role="listitem"
                        className="tcld-trace-evidence-row"
                        data-trace-evidence-kind={row.kind}
                        data-trace-evidence-target={row.target ? "true" : "false"}
                        onClick={() => handleEvidenceNav(row)}
                        disabled={!row.target}
                      >
                        <span className={`tcld-trace-evidence-badge tcld-trace-evidence-badge-${row.kind}`}>{row.badge}</span>
                        <span className="tcld-trace-evidence-label">{row.label}</span>
                        <span className="tcld-trace-evidence-detail">{row.detail}</span>
                      </button>
                    ))}
                  </div>
                )}
              </DetailSection>
              <DetailSection title="Decisions" data-trace-section="decisions">
                {detail.decisions.length === 0 ? (
                  <div className="tcld-trace-empty" data-trace-decisions-empty="true">No decisions recorded.</div>
                ) : (
                  <ul className="tcld-trace-decisions" data-trace-decisions="true">
                    {detail.decisions.map((row, index) => (
                      <li key={index} className={`tcld-trace-decision tcld-trace-decision-${row.outcome}`} data-trace-decision-outcome={row.outcome}>
                        <span aria-hidden className="tcld-trace-decision-icon">
                          {row.outcome === "yes" ? <Check size={10} /> : <X size={10} />}
                        </span>
                        <span>{row.text}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </DetailSection>
            </>
          )}
        </section>
      </div>
    </Panel>
  );
});

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="tcld-trace-stat" data-trace-stat={label.replace(/\s+/g, "-")}>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

interface DetailSectionProps {
  title: string;
  children: React.ReactNode;
  "data-trace-section"?: string;
}

function DetailSection({ title, children, ...rest }: DetailSectionProps) {
  return (
    <section className="tcld-trace-detail-section" {...rest}>
      <SectionEyebrow as="h4" className="tcld-trace-detail-section-title">{title}</SectionEyebrow>
      <div className="tcld-trace-detail-section-body">{children}</div>
    </section>
  );
}

function PassChainRow({ row }: { row: TracePassRow }) {
  return (
    <div
      role="listitem"
      className={`tcld-trace-chain-row tcld-trace-chain-${row.status}`}
      data-trace-chain-row="true"
      data-trace-pass-name={row.name}
      data-trace-pass-iteration={row.iteration}
      data-trace-pass-status={row.status}
      data-trace-pass-changed={row.changed ? "true" : "false"}
    >
      <span className="tcld-trace-chain-index">{row.index + 1}</span>
      <span className="tcld-trace-chain-pass">{row.name}</span>
      <span className="tcld-trace-chain-iter">iter {row.iteration}</span>
      <span className="tcld-trace-chain-status">{PASS_STATUS_LABEL[row.status]}</span>
      <span className="tcld-trace-chain-note">{row.note}</span>
    </div>
  );
}
