"use client";

import React from "react";
import clsx from "clsx";
import { Panel } from "./Panel";
import { EmptyState, PanelEmpty } from "@/components/EmptyState";
import { useDebugStore } from "@/lib/debug-store";
import { DEBUGGER_UNAVAILABLE_MESSAGE } from "@/lib/capabilities";
import { useLocationStore } from "@/lib/location-store";
import { ViewIcon } from "@/components/ViewIcon";

interface StackPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const StackPanel = React.memo(function StackPanel({
  focused,
  id,
  onClose,
  onFocus,
}: StackPanelProps) {
  const debug = useDebugStore();
  const { index, setCursor } = useLocationStore();
  const { stack, sessionState, stoppedAt, fetchVariables, debuggerAvailable } = debug;
  const isActive = sessionState === "paused" || sessionState === "running" || sessionState === "pausing" || sessionState === "stopping";

  return (
    <Panel
      designSurface="debug-call-stack"
      id={id}
      title="Call Stack"
      icon={<ViewIcon viewId="stack" />}
      badge={isActive && stack.length > 0 ? `${stack.length}` : undefined}
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
      ) : !isActive ? (
        <PanelEmpty>No debug session active</PanelEmpty>
      ) : stack.length === 0 ? (
        <PanelEmpty>No stack frames</PanelEmpty>
      ) : (
        <div className="tcld-stack-list">
          {stack.map((frame, i) => {
            const isCurrentFrame = stoppedAt && frame.file === stoppedAt.file && frame.line === stoppedAt.line;
            const basename = frame.file?.split("/").pop() ?? "";
            const preview = frame.cmd ?? (frame.proc ? `${frame.proc} ...` : `<${frame.type}>`);

            return (
              <div
                key={i}
                className={clsx("tcld-stack-card", isCurrentFrame && "tcld-stack-card-active")}
                onClick={() => {
                  void fetchVariables(frame.level);
                  if (frame.line != null || frame.file) {
                    setCursor({
                      scopeId: index?.scopeId ?? null,
                      sourceFile: frame.file ?? null,
                      sourceLine: frame.line ?? null,
                    });
                  }
                }}
              >
                <span className="tcld-stack-depth-line" aria-hidden />
                <span className="tcld-stack-depth-dot" aria-hidden />
                <div className="tcld-stack-card-header">
                  <span className="tcld-stack-index">#{i}</span>
                  <span className="tcld-stack-name">{frame.proc ?? `<${frame.type}>`}</span>
                  <span className="tcld-stack-args">{frame.proc ? "(...)" : ""}</span>
                  <span className="tcld-stack-location">{basename}{frame.line != null ? `:${frame.line}` : ""}</span>
                </div>
                <div className="tcld-stack-preview">
                  <span>{frame.line ?? frame.level}</span>
                  <span>{preview}</span>
                  <span>{frame.type}</span>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </Panel>
  );
});
