"use client";

import React from "react";
import clsx from "clsx";

interface EmptyStateProps {
  icon?: React.ReactNode;
  title: string;
  message?: string;
  action?: {
    label: string;
    onClick: () => void;
  };
  /** Visual size:
   *   - "panel": dense, used inside a panel body when no data is available
   *   - "screen": large, used as a full-window empty state (welcome / no file)
   */
  size?: "panel" | "screen";
  className?: string;
}

/**
 * Single empty-state primitive used by every panel that needs to communicate
 * "no data here yet". Replaces the dozen-plus ad-hoc centered divs scattered
 * across panels (`flex items-center justify-center h-full text-tcld-fg-dim`).
 *
 * Two sizes:
 *   - `panel`: a quiet, low-contrast hint for empty panels. Uses meta type and
 *     sits in the middle of the panel body.
 *   - `screen`: a large empty state for the welcome screen / cold open. Uses
 *     the accent color and includes a CTA.
 */
export function EmptyState({
  icon,
  title,
  message,
  action,
  size = "panel",
  className,
}: EmptyStateProps) {
  if (size === "screen") {
    return (
      <div
        className={clsx(
          "flex h-full w-full items-center justify-center animate-fade-in",
          className,
        )}
      >
        <div className="flex max-w-sm flex-col items-center gap-4 px-6 text-center">
          {icon && (
            <div className="flex items-center justify-center text-5xl text-tcld-accent">
              {icon}
            </div>
          )}
          <h3 className="text-sm font-semibold text-tcld-fg">{title}</h3>
          {message && (
            <p className="text-meta text-tcld-fg-dim leading-relaxed">{message}</p>
          )}
          {action && (
            <button
              type="button"
              onClick={action.onClick}
              className={clsx(
                "mt-2 rounded bg-tcld-accent px-4 py-2 text-sm font-semibold text-tcld-bg",
                "transition-opacity hover:opacity-90 active:opacity-80",
              )}
            >
              {action.label}
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div
      data-empty-state="panel"
      className={clsx(
        "flex h-full w-full flex-1 items-center justify-center px-6",
        className,
      )}
    >
      <div className="flex max-w-xs flex-col items-center gap-2 text-center">
        {icon && (
          <div className="tcld-empty-icon" aria-hidden>
            {icon}
          </div>
        )}
        <div className="font-ui text-body font-medium text-tcld-fg-muted">{title}</div>
        {message && (
          <div className="font-ui text-meta leading-relaxed text-tcld-text-comment">
            {message}
          </div>
        )}
        {action && (
          <button
            type="button"
            onClick={action.onClick}
            className={clsx(
              "mt-1 rounded-[var(--radius-button)] border border-tcld-border-dim px-2.5 py-1",
              "font-ui text-meta font-medium text-tcld-accent",
              "transition-colors hover:border-tcld-border-focus hover:bg-tcld-accent/10",
            )}
          >
            {action.label}
          </button>
        )}
      </div>
    </div>
  );
}

/**
 * Convenience wrapper for the most common case: an inline panel-empty message
 * with no icon and no CTA. Equivalent to
 *   <EmptyState size="panel" title="…" />
 */
export function PanelEmpty({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div
      data-empty-state="inline"
      className={clsx(
        "flex h-full w-full flex-1 items-center justify-center px-6 text-center",
        "font-ui text-control text-tcld-text-comment",
        className,
      )}
    >
      {children}
    </div>
  );
}
