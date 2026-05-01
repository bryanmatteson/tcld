"use client";

import { useTcldStore } from "@/lib/store";
import { Panel } from "./Panel";
import { ViewIcon } from "@/components/ViewIcon";
import { EmptyState } from "@/components/EmptyState";

interface SummaryPanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
}

export function SummaryPanel({ id, focused, onClose, onFocus }: SummaryPanelProps) {
  const { data } = useTcldStore();
  const text = data?.summaryText ?? "";

  return (
    <Panel id={id} title="Summary" icon={<ViewIcon viewId="summary" />} focused={focused} onClose={onClose} onFocus={onFocus}>
      {text ? (
        <div className="py-2 px-4 font-mono text-disasm leading-5 text-tcld-fg whitespace-pre-wrap break-words">
          {text}
        </div>
      ) : (
        <EmptyState size="panel" title="No file loaded" message="Press ⌘O to open a .tbc file." />
      )}
    </Panel>
  );
}
