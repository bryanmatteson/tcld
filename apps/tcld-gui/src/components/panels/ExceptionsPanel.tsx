"use client";

import { useTcldData } from "@/lib/store";
import { Panel } from "./Panel";
import { PanelEmpty } from "@/components/EmptyState";
import { Chip } from "@/components/primitives/Chip";
import clsx from "clsx";

interface ExceptionsPanelProps {
  id: string;
  focused: boolean;
  onClose?: () => void;
  onFocus?: () => void;
}

export function ExceptionsPanel({ id, focused, onClose, onFocus }: ExceptionsPanelProps) {
  const { data } = useTcldData();
  const exceptions = data?.exceptions ?? [];

  if (exceptions.length === 0) {
    return (
      <Panel id={id} title="Exceptions" badge="0" focused={focused} onClose={onClose} onFocus={onFocus}>
        <PanelEmpty>no exception ranges</PanelEmpty>
      </Panel>
    );
  }

  return (
    <Panel id={id} title="Exceptions" badge={String(exceptions.length)} focused={focused} onClose={onClose} onFocus={onFocus}>
      <div className="py-1 px-0">
        {exceptions.map((ex) => (
          <div
            key={ex.idx}
            data-exception-idx={ex.idx}
            data-exception-kind={ex.kind}
            className={clsx(
              "flex items-center gap-2 px-3",
              "font-mono text-meta",
              "h-[var(--code-row-height)] leading-[var(--code-row-height)]",
              "transition-colors duration-[60ms] ease-out",
              "hover:bg-tcld-bg-2",
            )}
          >
            <span className="w-9 flex-shrink-0 pr-2.5 text-right text-tcld-fg-dim">{ex.idx}</span>
            <Chip size="xs" tone={ex.kind === "Catch" ? "err" : "group"}>
              {ex.kind}
            </Chip>
            <span className="w-9 flex-shrink-0 text-tcld-fg-dim">n={ex.nestingLevel}</span>
            <span className="w-24 flex-shrink-0 text-tcld-number">
              code {ex.codeRange[0]}..{ex.codeRange[1]}
            </span>
            <span className="flex-1 text-label text-tcld-fg-muted">
              brk={ex.breakOffset} cont={ex.continueOffset} catch={ex.catchOffset}
            </span>
          </div>
        ))}
      </div>
    </Panel>
  );
}
