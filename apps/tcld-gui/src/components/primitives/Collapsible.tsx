"use client";

import React from "react";
import clsx from "clsx";
import { ChevronDown, ChevronRight } from "lucide-react";

interface CollapsibleProps {
  /** Stable id used for ARIA wiring + data attributes (testing hook). */
  id: string;
  title: string;
  /** Right-aligned subtitle (count, status, …). */
  sub?: React.ReactNode;
  /** Optional leading icon. */
  icon?: React.ReactNode;
  /** Controlled expand state. */
  open: boolean;
  onToggle: () => void;
  /** Visual density. "compact" matches inspector rows; "comfy" matches detail dialogs. */
  density?: "compact" | "comfy";
  /** Render the body without padding so callers can lay out their own grids. */
  flushBody?: boolean;
  /** Optional secondary actions placed inside the header right slot. */
  actions?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}

/**
 * One collapsible-section primitive used by every inspector / sidebar that
 * shows expandable groups (Inspector, Provenance, Diagnostics, Detail).
 * Keeps disclosure-caret semantics, ARIA wiring, and density consistent.
 */
export function Collapsible({
  id,
  title,
  sub,
  icon,
  open,
  onToggle,
  density = "compact",
  flushBody = false,
  actions,
  className,
  children,
}: CollapsibleProps) {
  const contentId = `collapsible-${id}`;
  const headerHeight = density === "compact" ? "h-7" : "h-8";

  return (
    <section
      data-collapsible={id}
      data-open={open ? "true" : "false"}
      className={clsx(
        "border-b border-tcld-border-dim last:border-b-0",
        className,
      )}
    >
      <div className="flex items-stretch">
        <button
          type="button"
          data-panel-interactive="true"
          data-collapsible-toggle={id}
          aria-expanded={open}
          aria-controls={contentId}
          onClick={onToggle}
          className={clsx(
            "flex flex-1 items-center gap-2 px-3 text-left text-tcld-fg",
            "transition-colors hover:bg-tcld-bg-2 focus:bg-tcld-bg-2 focus:outline-none",
            headerHeight,
          )}
        >
          <span
            className="flex h-3.5 w-3.5 items-center justify-center text-tcld-fg-dim"
            aria-hidden
          >
            {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </span>
          {icon && (
            <span className="flex h-3.5 w-3.5 items-center justify-center text-tcld-fg-dim" aria-hidden>
              {icon}
            </span>
          )}
          <span className="flex-1 truncate text-meta font-medium">{title}</span>
          {sub != null && (
            <span className="text-label text-tcld-fg-muted whitespace-nowrap tabular-nums">{sub}</span>
          )}
        </button>
        {actions && (
          <div
            data-panel-interactive="true"
            className="flex shrink-0 items-center gap-1 pr-2"
          >
            {actions}
          </div>
        )}
      </div>
      {open && (
        <div
          id={contentId}
          data-collapsible-content={id}
          className={clsx(!flushBody && "px-3 pb-2")}
        >
          {children}
        </div>
      )}
    </section>
  );
}
