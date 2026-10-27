"use client";

import React from "react";
import { Panel } from "./Panel";
import { useTcldData } from "@/lib/store";
import { useLocationStore } from "@/lib/location-store";
import { useShallow } from "zustand/react/shallow";
import { buildComparisonPanelModel, sourceMapSelectionTarget } from "@/lib/panel-models";
import { ViewIcon } from "@/components/ViewIcon";
import { EmptyState } from "@/components/EmptyState";
import type { ComparisonRowStatus } from "@/lib/workflow-contracts";

interface ComparisonPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

const STATUS_LABELS: ComparisonRowStatus[] = ["match", "partial", "stripped"];

export const ComparisonPanel = React.memo(function ComparisonPanel({ focused, id, onClose, onFocus }: ComparisonPanelProps) {
  const data = useTcldData().data;
  const { cursor, setCursor } = useLocationStore(useShallow((s) => ({ cursor: s.cursor, setCursor: s.setCursor })));
  const model = buildComparisonPanelModel(data, cursor);
  const rows = model?.rows ?? [];
  const emptyMessage = data
    ? (model?.original.available ? "No source-map comparison rows available." : "Original source unavailable; comparison rows show stripped source spans only.")
    : "No document loaded.";

  return (
    <Panel id={id} title="Compare" icon={<ViewIcon viewId="comparison" />} badge={model?.original.available ? "linked" : "reconstructed"} focused={focused} designSurface="view-compare" onClose={onClose} onFocus={onFocus}>
      <div className="comparison-view" data-comparison-mode="pair-rows">
        <div className="comparison-toolbar" data-comparison-toolbar="true">
          <div className="comparison-toolbar-title">
            <span>compare</span>
            <span className="comparison-toolbar-separator" aria-hidden />
            <span>source ↔ disasm</span>
          </div>
          <div className="comparison-legend" data-comparison-legend="true">
            {STATUS_LABELS.map((status) => (
              <span key={status} className="comparison-legend-item" data-comparison-legend-status={status} data-comparison-legend-count={model?.legendCounts[status] ?? 0}>
                <span className={`comparison-dot comparison-dot-${status}`} aria-hidden />
                <span>{status}</span>
                <span className="comparison-legend-count">{model?.legendCounts[status] ?? 0}</span>
              </span>
            ))}
          </div>
        </div>
        <div className="comparison-row-header" data-comparison-row-header="true">
          <span>source</span>
          <span>reconstructed / disasm</span>
        </div>
        <div className="comparison-row-list" data-comparison-row-list="true">
          {rows.length > 0 ? rows.map((row) => {
            const selected = model?.selection.sourceMapIndex === row.sourceMapIndex;
            return (
              <button
                key={row.sourceMapIndex}
                type="button"
                data-comparison-row={row.status}
                data-comparison-status={row.status}
                data-source-map-index={row.sourceMapIndex}
                data-selected={selected ? "true" : undefined}
                data-original-available={row.originalAvailable ? "true" : "false"}
                className={`comparison-row comparison-row-${row.status}${selected ? " comparison-row-selected" : ""}`}
                onClick={() => {
                  const target = data ? sourceMapSelectionTarget(data, row.sourceMapIndex) : null;
                  if (target) setCursor(target, { source: "user" });
                }}
              >
                <span className={`comparison-row-rail comparison-row-rail-${row.status}`} aria-hidden />
                <span className="comparison-row-source">
                  <span className="comparison-row-text">{row.sourceText}</span>
                  <span className="comparison-row-meta">
                    {row.originalSpan ? `orig ${row.originalSpan[0]}..${row.originalSpan[1]}` : (row.unavailableReason ?? "orig unavailable")}
                  </span>
                </span>
                <span className="comparison-row-reconstructed">
                  <span className="comparison-row-text">{row.reconstructedText}</span>
                  <span className="comparison-row-meta">{row.disassemblyText}</span>
                  <span className="comparison-row-meta">{row.metadata}</span>
                </span>
              </button>
            );
          }) : (
            <div data-comparison-empty-state={model?.original.available ? "empty" : "no-source"}>
              <EmptyState size="panel" title={emptyMessage} />
            </div>
          )}
        </div>
      </div>
    </Panel>
  );
});
