"use client";

import { useMemo, useState } from "react";
import clsx from "clsx";
import { Search } from "lucide-react";
import { buildSymbolsPanelModel, type SymbolRow } from "@/lib/panel-models";
import { useTcldData } from "@/lib/store";
import { useLocationStore } from "@/lib/location-store";
import { useShallow } from "zustand/react/shallow";
import { Panel } from "./Panel";
import { EmptyState } from "@/components/EmptyState";

interface LiteralsPanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
}

export function LiteralsPanel({ id, focused, onClose, onFocus }: LiteralsPanelProps) {
  const { data } = useTcldData();
  const { cursor, setCursor } = useLocationStore(useShallow((s) => ({ cursor: s.cursor, setCursor: s.setCursor })));
  const [filter, setFilter] = useState("");
  const literals = data?.literals ?? [];
  const model = useMemo(() => buildSymbolsPanelModel(data, filter), [data, filter]);

  const onRowClick = (row: SymbolRow) => {
    if (!row.target) return;
    setCursor(row.target, { source: "user" });
  };

  const isSelected = (row: SymbolRow) => {
    if (!row.target) return false;
    return (row.target.group != null && row.target.group === cursor.group)
      || (row.target.pc != null && row.target.pc === cursor.pc)
      || (row.target.sourceLine != null && row.target.sourceLine === cursor.sourceLine);
  };

  if (model.status === "unavailable") {
    return (
      <Panel title="Symbols" badge="0" focused={focused} id={id} designSurface="view-symbols" onClose={onClose} onFocus={onFocus}>
        <div className="flex h-7 items-center gap-2 border-b border-tcld-border-dim px-2 font-mono text-meta" data-symbols-toolbar="true">
          <Search size={12} className="text-tcld-fg-dim" aria-hidden="true" />
          <input
            className="min-w-0 flex-1 bg-transparent text-tcld-fg outline-none placeholder:text-tcld-fg-dim"
            value={filter}
            placeholder="filter symbols..."
            data-symbol-filter="true"
            onChange={(event) => setFilter(event.target.value)}
          />
          <span className="text-tcld-fg-dim tabular-nums" data-symbol-count="true">0/0</span>
        </div>
        {/*
         * "Symbols unavailable" empty state. The substitute hint is rendered
         * as a third line in the message body rather than its own block, so
         * the panel reads as one coherent empty state — not three stacked
         * paragraphs.
         */}
        <div data-symbols-unavailable="true">
          <EmptyState
            size="panel"
            title={model.unavailableTitle}
            message={`${model.unavailableExplanation} Literals remain the current production substitute (${model.substituteLiteralCount} loaded).`}
          />
        </div>
      </Panel>
    );
  }

  return (
    <Panel title="Symbols" badge={String(model.totalCount)} focused={focused} id={id} designSurface="view-symbols" onClose={onClose} onFocus={onFocus}>
      <div className="tcld-symbols-toolbar" data-symbols-toolbar="true">
        <Search size={11} aria-hidden="true" />
        <input
          value={filter}
          placeholder="filter symbols…"
          data-symbol-filter="true"
          onChange={(event) => setFilter(event.target.value)}
        />
        <span data-symbol-count="true">{model.visibleCount}/{model.totalCount}</span>
      </div>
      <div className="tcld-symbols-header" data-symbol-header="true">
        <span data-symbol-column="kind">KIND</span>
        <span data-symbol-column="name">NAME</span>
        <span data-symbol-column="addr">ADDR</span>
        <span data-symbol-column="size">SIZE</span>
        <span data-symbol-column="refs">REFS</span>
      </div>
      {model.noMatchText ? (
        <div data-symbol-empty-state="true">
          <EmptyState size="panel" title={model.noMatchText} />
        </div>
      ) : (
        <div className="tcld-symbols-table" data-symbol-table="true">
          {model.visibleRows.map((row) => {
            const clickable = row.target !== null;
            return (
              <button
                key={row.id}
                type="button"
                disabled={!clickable}
                onClick={() => onRowClick(row)}
                className={clsx(
                  "tcld-symbols-row",
                  isSelected(row) && "is-selected",
                  !clickable && "is-disabled",
                )}
                data-symbol-row="true"
                data-symbol-kind={row.kind}
                data-symbol-name={row.name}
                data-symbol-clickable={clickable ? "true" : "false"}
                data-symbol-selected={isSelected(row) ? "true" : "false"}
              >
                <span
                  className={clsx("tcld-symbols-pill", `tcld-symbols-pill-${row.kind}`)}
                  data-symbol-kind-pill={row.kind}
                >
                  {row.kind}
                </span>
                <span className="tcld-symbols-name" data-symbol-cell="name">{row.name}</span>
                <span className="tcld-symbols-addr" data-symbol-cell="addr" title={row.location ?? undefined}>{row.addressLabel}</span>
                <span className="tcld-symbols-size" data-symbol-cell="size">{row.size ?? "—"}</span>
                <span className="tcld-symbols-refs" data-symbol-cell="refs">{row.refs ?? "—"}</span>
              </button>
            );
          })}
        </div>
      )}
      {literals.length > 0 && (
        <div className="tcld-symbols-substitute" data-symbols-literals-substitute="true">
          {literals.length} literal{literals.length === 1 ? "" : "s"} mapped (transitional)
        </div>
      )}
    </Panel>
  );
}
