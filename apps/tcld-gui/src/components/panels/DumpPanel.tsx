"use client";

import { useTcldData } from "@/lib/store";
import { useDebugStore } from "@/lib/debug-store";
import { Panel } from "./Panel";
import { PanelEmpty } from "@/components/EmptyState";

interface DumpPanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
}

export function DumpPanel({ id, focused, onClose, onFocus }: DumpPanelProps) {
  const { data } = useTcldData();
  const debug = useDebugStore();
  const liveText = debug.disassemblyHeader
    ? `${debug.disassemblyHeader}\n\n${debug.disassemblyText ?? ""}`
    : debug.disassemblyText;
  const text = data?.dumpText || liveText || "";
  const badge = data?.dumpText ? "raw" : liveText ? "live" : "—";

  return (
    <Panel id={id} title="Dump" badge={badge} focused={focused} onClose={onClose} onFocus={onFocus}>
      {text ? (
        <div className="py-2 px-4 font-mono text-disasm leading-5 text-tcld-fg whitespace-pre-wrap break-words">
          {text}
        </div>
      ) : (
        <PanelEmpty>No dump available</PanelEmpty>
      )}
    </Panel>
  );
}
