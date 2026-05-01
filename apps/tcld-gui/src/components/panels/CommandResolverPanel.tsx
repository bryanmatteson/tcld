"use client";

import React, { useState } from "react";
import { Panel } from "./Panel";
import { ViewIcon } from "@/components/ViewIcon";
import { Field, SectionEyebrow } from "@/components/primitives";
import { debugResolveCommand } from "@/lib/tauri-api";
import type { CommandResolution } from "@/types";

interface CommandResolverPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const CommandResolverPanel = React.memo(function CommandResolverPanel({ focused, id, onClose, onFocus }: CommandResolverPanelProps) {
  const [name, setName] = useState("");
  const [namespace, setNamespace] = useState("");
  const [resolution, setResolution] = useState<CommandResolution | null>(null);
  const [error, setError] = useState<string | null>(null);

  const resolve = async () => {
    if (!name.trim()) return;
    try {
      setError(null);
      setResolution(await debugResolveCommand(name.trim(), namespace.trim() || undefined));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <Panel id={id} title="Command Resolver" icon={<ViewIcon viewId="commandResolver" />} badge={resolution?.exists ? "exists" : undefined} focused={focused} onClose={onClose} onFocus={onFocus}>
      <div className="flex h-full min-h-0 flex-col font-mono text-xs">
        <div className="flex gap-2 border-b border-tcld-border-dim p-2">
          <input className="min-w-0 flex-1 bg-tcld-bg-2 px-2 py-1 outline-none" value={name} placeholder="command" onChange={(e) => setName(e.target.value)} />
          <input className="min-w-0 flex-1 bg-tcld-bg-2 px-2 py-1 outline-none" value={namespace} placeholder="namespace" onChange={(e) => setNamespace(e.target.value)} />
          <button className="px-2 text-tcld-accent" onClick={() => void resolve()}>resolve</button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto p-3">
          {error && <div className="text-tcld-error">{error}</div>}
          {resolution && (
            <div className="space-y-1">
              <Field label="original" value={resolution.originalName} labelWidth={140} wrap />
              <Field label="resolved" value={resolution.resolvedName} labelWidth={140} wrap />
              <Field label="fullyQualified" value={resolution.fullyQualifiedName} labelWidth={140} wrap />
              <Field label="namespace" value={resolution.namespace} labelWidth={140} wrap />
              <Field label="type" value={resolution.commandType} labelWidth={140} wrap />
              <Field label="exists" value={String(resolution.exists)} labelWidth={140} tone={resolution.exists ? "default" : "muted"} />
              <Field label="alias" value={resolution.isAlias ? resolution.aliasTarget ?? "yes" : "no"} labelWidth={140} wrap />
              <Field label="import" value={resolution.isImported ? resolution.importSource ?? "yes" : "no"} labelWidth={140} wrap />
              <Field label="ensemble" value={resolution.isEnsemble ? resolution.ensembleNamespace ?? "yes" : "no"} labelWidth={140} wrap />
              <SectionEyebrow className="pt-2">Resolution path</SectionEyebrow>
              {resolution.resolutionPath.map((step, index) => (
                <div key={index} className="text-meta text-tcld-fg-dim">{step.step}: {step.result}</div>
              ))}
            </div>
          )}
        </div>
      </div>
    </Panel>
  );
});
