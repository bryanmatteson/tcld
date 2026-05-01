"use client";

import React, { useState } from "react";
import { X } from "lucide-react";
import { Panel } from "./Panel";
import { EmptyState, PanelEmpty } from "@/components/EmptyState";
import { useDebugStore } from "@/lib/debug-store";
import { DEBUGGER_UNAVAILABLE_MESSAGE } from "@/lib/capabilities";
import { ViewIcon } from "@/components/ViewIcon";
import { Chip } from "@/components/primitives/Chip";

interface WatchpointsPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const WatchpointsPanel = React.memo(function WatchpointsPanel({ focused, id, onClose, onFocus }: WatchpointsPanelProps) {
  const debug = useDebugStore();
  const [name, setName] = useState("");

  return (
    <Panel id={id} title="Watchpoints" icon={<ViewIcon viewId="watchpoints" />} badge={String(debug.watchpoints.length)} focused={focused} designSurface="debug-watchpoints" onClose={onClose} onFocus={onFocus}>
      {!debug.debuggerAvailable ? (
        // Capability-aware empty state. Uses the same selector contract as the
        // other debug panels (data-debug-empty-state="unavailable"). The
        // add/refresh toolbar is withheld entirely: every action in it
        // would end at debug IPC the backend can't serve.
        <div className="h-full" data-debug-empty-state="unavailable">
          <EmptyState
            size="panel"
            title="Debugger unavailable"
            message={`${DEBUGGER_UNAVAILABLE_MESSAGE}.`}
          />
        </div>
      ) : (
      <div className="flex h-full min-h-0 flex-col font-mono text-xs">
        {/*
         * Hit banner — the most-recent watchpoint hit. Always-on alert at
         * the top of the panel; uses warn tone since a hit is an event the
         * user wants to notice. The body matches the same vocabulary as
         * console-row's err-line.
         */}
        {debug.watchpointHit && (
          <div className="border-b border-tcld-border-dim bg-tcld-bg-1 px-3 py-2 text-tcld-fg" data-watchpoint-hit="true">
            <span>
              {debug.watchpointHit.name} {debug.watchpointHit.operation}: {debug.watchpointHit.oldValue} → {debug.watchpointHit.newValue}
            </span>
            <div className="text-tcld-fg-muted">
              {debug.watchpointHit.file}:{debug.watchpointHit.line} · frames {debug.watchpointHit.frameCount}
            </div>
          </div>
        )}

        {/* Toolbar — add input + actions. Pill family shared with the rest. */}
        <div className="flex h-7 min-w-0 items-center gap-1 border-b border-tcld-border-dim/60 bg-tcld-bg-1/35 px-2">
          <input
            className="min-w-0 flex-1 rounded border border-tcld-border-dim bg-tcld-bg-2 px-2 py-0.5 text-meta text-tcld-fg outline-none focus:border-tcld-border-focus"
            value={name}
            placeholder="variable name…"
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter" && name.trim()) {
                void debug.setWatchpoint(name.trim());
                setName("");
              }
            }}
          />
          <button
            type="button"
            className="tcld-toolbar-pill"
            onClick={() => {
              if (name.trim()) void debug.setWatchpoint(name.trim());
              setName("");
            }}
            disabled={!name.trim()}
          >
            add
          </button>
          <button
            type="button"
            className="tcld-toolbar-pill"
            onClick={() => void debug.refreshWatchpoints()}
            title="Force-resync watchpoint list"
          >
            refresh
          </button>
        </div>

        {/*
         * Rows — workbench-mock `.bp-row` shape. A watchpoint reads as a
         * dot/name/loc tuple just like a breakpoint: dot tone marks the
         * scope (accent for global, neutral for local), name carries the
         * variable, loc carries the scope chip + remove. The dot is purely
         * decorative here (no toggle action) — watchpoints are
         * always-on once set.
         */}
        <div className="min-h-0 flex-1 overflow-auto">
          {debug.watchpoints.length === 0 ? (
            <PanelEmpty>No watchpoints</PanelEmpty>
          ) : debug.watchpoints.map((watchpoint) => (
            <div key={watchpoint.id} className="bp-row" data-watchpoint-row={watchpoint.name}>
              <span className="dot" aria-hidden style={{ background: "var(--accent)" }} />
              <span className="name">
                <span className="file">{watchpoint.name}</span>
              </span>
              <span className="loc">
                <Chip size="xs" tone="neutral">{watchpoint.scope}</Chip>
                <button
                  type="button"
                  className="remove"
                  onClick={() => void debug.clearWatchpoint(watchpoint.name)}
                  aria-label="Clear watchpoint"
                  title="Clear watchpoint"
                >
                  <X size={11} aria-hidden />
                </button>
              </span>
            </div>
          ))}
        </div>
      </div>
      )}
    </Panel>
  );
});
