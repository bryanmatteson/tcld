"use client";

import React from "react";
import { Panel } from "./Panel";
import { PanelEmpty } from "@/components/EmptyState";
import { useTcldStore } from "@/lib/store";
import { ViewIcon } from "@/components/ViewIcon";
import { Chip } from "@/components/primitives/Chip";

interface ProjectPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const ProjectPanel = React.memo(function ProjectPanel({ focused, id, onClose, onFocus }: ProjectPanelProps) {
  const { session, selectDocument } = useTcldStore();

  return (
    <Panel id={id} title="Project" icon={<ViewIcon viewId="project" />} badge={String(session.documents.length)} focused={focused} onClose={onClose} onFocus={onFocus}>
      <div className="h-full overflow-auto font-mono text-meta">
        {session.documents.length === 0 ? (
          <PanelEmpty>No open documents</PanelEmpty>
        ) : session.documents.map((doc) => (
          <button
            key={doc.id}
            type="button"
            className="block w-full border-b border-tcld-border-dim px-3 py-2 text-left hover:bg-tcld-bg-highlight"
            onClick={() => selectDocument(doc.id)}
            data-project-doc={doc.id}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="truncate text-tcld-fg">{doc.label}</span>
              <Chip size="xs" tone="neutral">{doc.role}</Chip>
            </div>
            <div className="truncate text-tcld-fg-dim">
              {doc.status} · {doc.provenance.kind} · original={String(doc.provenance.originalSourceAvailable)}
            </div>
          </button>
        ))}
      </div>
    </Panel>
  );
});
