"use client";

import clsx from "clsx";
import { ViewIcon } from "@/components/ViewIcon";
import type { ViewId, ViewDef } from "@/types";

interface TabBarProps {
  views: ViewDef[];
  activeView: ViewId;
  onViewChange: (view: ViewId) => void;
}

/**
 * Document-level tab bar — uses the same `.tcld-view-tab` browser-tab
 * vocabulary as in-panel view tabs (see Panel.tsx). Pill chips are reserved
 * for secondary segmentation, never primary view-switching.
 *
 * All styling uses Tailwind utilities + the named .tcld-view-tab class —
 * no inline `style={{ T.*: ... }}` per the design-system source-of-truth
 * cleanup.
 */
export function TabBar({ views, activeView, onViewChange }: TabBarProps) {
  return (
    <div
      className="flex items-end px-1 border-b border-tcld-border bg-tcld-bg shrink-0"
      role="tablist"
    >
      {views.map((v) => {
        const active = v.id === activeView;
        return (
          <button
            key={v.id}
            role="tab"
            aria-selected={active}
            onClick={() => onViewChange(v.id)}
            className={clsx("tcld-view-tab", active && "tcld-view-tab-active")}
          >
            <span className={clsx("flex h-3.5 w-3.5 items-center justify-center", !active && "opacity-60")}>
              <ViewIcon viewId={v.id} size={12} />
            </span>
            <span className={clsx(active ? "font-semibold" : "font-medium")}>{v.label}</span>
            {v.key && (
              <span
                className={clsx(
                  "inline-flex items-center px-1 h-4 rounded text-label font-mono leading-none",
                  active
                    ? "bg-tcld-bg-highlight/65 text-tcld-text-keyhint"
                    : "text-tcld-text-keyhint",
                )}
              >
                {v.key}
              </span>
            )}
          </button>
        );
      })}

      <div className="flex-1" />

      <div className="flex items-center gap-2 pr-3 font-mono text-meta text-tcld-text-keyhint">
        <span className="inline-flex items-center px-1.5 h-5 rounded bg-tcld-bg-2 border border-tcld-border-dim">⌘P</span>
        <span className="inline-flex items-center px-1.5 h-5 rounded bg-tcld-bg-2 border border-tcld-border-dim">?</span>
      </div>
    </div>
  );
}
