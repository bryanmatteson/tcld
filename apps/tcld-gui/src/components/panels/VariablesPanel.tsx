"use client";

import React, { useState, useMemo } from "react";
import { Panel } from "./Panel";
import { EmptyState, PanelEmpty } from "@/components/EmptyState";
import { useDebugStore } from "@/lib/debug-store";
import { DEBUGGER_UNAVAILABLE_MESSAGE } from "@/lib/capabilities";
import type { DebugVariable } from "@/types";
import { T } from "@/lib/theme";
import { ChevronDown, ChevronRight, Search } from "lucide-react";
import { ViewIcon } from "@/components/ViewIcon";

// ─── Variable row ────────────────────────────────────────────────

const TYPE_COLORS: Record<string, string> = {
  int: T.number,
  double: T.number,
  bool: T.keyword,
  string: T.string,
  list: T.accent,
  dict: T.group,
  array: T.group,
};

function VariableRow({
  v,
  depth = 0,
  onSetVariable,
}: {
  v: DebugVariable;
  depth?: number;
  onSetVariable?: (name: string, value: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState("");
  const hasChildren = v.children && v.children.length > 0;
  const isScalar = !hasChildren;
  const typeColor = TYPE_COLORS[v.type] || T.fgDim;

  return (
    <>
      <div
        className={v.changed ? "tcld-variable-row tcld-variable-row-changed group" : "tcld-variable-row group"}
        data-variable-changed={v.changed ? "true" : undefined}
        style={{ paddingLeft: 10 + depth * 18 }}
        title={hasChildren ? `${expanded ? "Collapse" : "Expand"} ${v.name}` : undefined}
        onClick={() => hasChildren && setExpanded(!expanded)}
      >
        {/* Expand toggle */}
        <span className="flex w-3 flex-shrink-0 items-center justify-center text-tcld-fg-dim" aria-hidden>
          {hasChildren ? (expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />) : null}
        </span>

        {/* Name */}
        <span className="tcld-variable-name">
          {v.name}
        </span>

        {/* Spacer */}
        <span className="flex-1" />

        {/* Value */}
        {editing ? (
          <input
            autoFocus
            className="max-w-[60%] bg-tcld-bg-1 border border-tcld-accent/40 rounded px-1 text-xs font-mono text-tcld-fg outline-none text-right"
            value={editValue}
            onChange={(e) => setEditValue(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") {
                onSetVariable?.(v.name, editValue);
                setEditing(false);
              }
              if (e.key === "Escape") setEditing(false);
            }}
            onBlur={() => setEditing(false)}
          />
        ) : (
          <span
            className="tcld-variable-value"
            onDoubleClick={
              isScalar
                ? () => {
                    setEditing(true);
                    setEditValue(v.value);
                  }
                : undefined
            }
          >
            {v.value}
          </span>
        )}

        <span
          className="tcld-variable-sparkline"
          aria-hidden
          style={{ display: "var(--variable-sparklines, block)" }}
        />

        {/* Type tag — visible on hover only */}
        <span
          className="tcld-variable-type"
          style={{
            color: typeColor,
            background: `color-mix(in srgb, ${typeColor} 12%, transparent)`,
            border: `1px solid color-mix(in srgb, ${typeColor} 20%, transparent)`,
          }}
        >
          {v.type}
        </span>
      </div>

      {/* Children */}
      {expanded &&
        v.children?.map((child, i) => (
          <VariableRow
            key={`${child.name}-${i}`}
            v={child}
            depth={depth + 1}
            onSetVariable={onSetVariable}
          />
        ))}
    </>
  );
}

// ─── Scope section ───────────────────────────────────────────────

const SCOPE_LABELS: Record<string, string> = {
  local: "Locals",
  ns: "Namespace",
  global: "Globals",
  upvar: "Upvars",
};

function ScopeSection({
  scope,
  variables,
  expanded,
  onToggle,
  onSetVariable,
}: {
  scope: string;
  variables: DebugVariable[];
  expanded: boolean;
  onToggle: () => void;
  onSetVariable?: (name: string, value: string) => void;
}) {
  const label = SCOPE_LABELS[scope] || scope;

  return (
    <div>
      <div
        className="tcld-variable-scope"
        title={`${expanded ? "Collapse" : "Expand"} ${label}`}
        onClick={onToggle}
      >
        <span aria-hidden>
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>
        <span className="tcld-variable-scope-badge">
          {label}
        </span>
        <span className="tcld-variable-scope-count">
          {variables.length}
        </span>
      </div>
      {expanded &&
        variables.map((v, i) => (
          <VariableRow
            key={`${v.name}-${i}`}
            v={v}
            onSetVariable={onSetVariable}
          />
        ))}
    </div>
  );
}

// ─── Panel ──────────────────────────────────────────────────────

interface VariablesPanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const VariablesPanel = React.memo(function VariablesPanel({
  focused,
  id,
  onClose,
  onFocus,
}: VariablesPanelProps) {
  const debug = useDebugStore();
  const { variables, variablesLevel, sessionState, setVariable, debuggerAvailable } = debug;

  const isActive = sessionState === "paused" || sessionState === "running" || sessionState === "pausing" || sessionState === "stopping";

  const [expandedScopes, setExpandedScopes] = useState<Set<string>>(
    new Set(["local", "ns", "global", "upvar"]),
  );
  const [activeScopes, setActiveScopes] = useState<Set<string>>(
    new Set(["local", "ns", "global", "upvar"]),
  );
  const [query, setQuery] = useState("");

  const grouped = useMemo(() => {
    const groups: Record<string, DebugVariable[]> = {};
    const normalizedQuery = query.trim().toLowerCase();
    const matches = (variable: DebugVariable): boolean => {
      if (!normalizedQuery) return true;
      return variable.name.toLowerCase().includes(normalizedQuery)
        || variable.value.toLowerCase().includes(normalizedQuery)
        || variable.type.toLowerCase().includes(normalizedQuery)
        || Boolean(variable.children?.some(matches));
    };

    for (const v of variables) {
      const s = v.scope || "local";
      if (["local", "ns", "global", "upvar"].includes(s) && !activeScopes.has(s)) continue;
      if (!matches(v)) continue;
      if (!groups[s]) groups[s] = [];
      groups[s].push(v);
    }
    return groups;
  }, [activeScopes, query, variables]);

  const scopeOrder = ["local", "ns", "global", "upvar"];

  const handleSetVariable = (name: string, value: string) => {
    setVariable(name, value, variablesLevel);
  };

  const toggleScope = (scope: string) => {
    setExpandedScopes((prev) => {
      const next = new Set(prev);
      if (next.has(scope)) next.delete(scope);
      else next.add(scope);
      return next;
    });
  };

  const toggleActiveScope = (scope: string) => {
    setActiveScopes((prev) => {
      const next = new Set(prev);
      if (next.has(scope)) next.delete(scope);
      else next.add(scope);
      return next;
    });
  };

  return (
    <Panel
      designSurface="debug-variables"
      id={id}
      title="Variables"
      icon={<ViewIcon viewId="variables" />}
      badge={isActive ? `L${variablesLevel}` : undefined}
      focused={focused}
      onClose={onClose}
      onFocus={onFocus}
    >
      {!debuggerAvailable ? (
        // data-debug-empty-state is the selector contract for the capability-aware
        // "backend has no debugger" state — distinct from the ordinary
        // no-session empty state (same pattern as DiagnosticsPanel's
        // data-diagnostics-empty-state="unavailable").
        <div className="h-full" data-debug-empty-state="unavailable">
          <EmptyState
            size="panel"
            title="Debugger unavailable"
            message={`${DEBUGGER_UNAVAILABLE_MESSAGE}.`}
          />
        </div>
      ) : !isActive ? (
        <PanelEmpty>No debug session active</PanelEmpty>
      ) : variables.length === 0 ? (
        <PanelEmpty>No variables in scope</PanelEmpty>
      ) : (
        <div className="tcld-variables-view">
          <div className="tcld-variables-toolbar">
            <Search size={11} aria-hidden />
            <input
              placeholder="Filter variables..."
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            {scopeOrder.map((scope) => (
              <button
                key={scope}
                type="button"
                aria-pressed={activeScopes.has(scope)}
                data-scope-active={activeScopes.has(scope) ? "true" : "false"}
                onClick={() => toggleActiveScope(scope)}
              >
                {scope}
              </button>
            ))}
          </div>
          <div className="tcld-variables-list">
            {scopeOrder
            .filter((s) => grouped[s] && grouped[s].length > 0)
            .map((scope) => (
              <ScopeSection
                key={scope}
                scope={scope}
                variables={grouped[scope]}
                expanded={expandedScopes.has(scope)}
                onToggle={() => toggleScope(scope)}
                onSetVariable={handleSetVariable}
              />
            ))}
          {/* Show any unknown scopes */}
          {Object.keys(grouped)
            .filter((s) => !scopeOrder.includes(s))
            .map((scope) => (
              <ScopeSection
                key={scope}
                scope={scope}
                variables={grouped[scope]}
                expanded={expandedScopes.has(scope)}
                onToggle={() => toggleScope(scope)}
                onSetVariable={handleSetVariable}
              />
            ))}
          </div>
        </div>
      )}
    </Panel>
  );
});
