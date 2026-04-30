"use client";

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";

interface HistoryItem {
  offset: number;
  label: string;
  detail?: string;
}

interface NavHistoryControlsProps {
  canGoBack: boolean;
  canGoForward: boolean;
  historyItems: HistoryItem[];
  onGoBack: () => void;
  onGoForward: () => void;
  onGoBackTo: (offset: number) => void;
}

/**
 * Compact history navigation control: back arrow (with click-to-open history
 * popover, double-click for one-step back) and forward arrow. Lives in the
 * file-header titlebar trail, alongside the command palette and panel picker.
 *
 * Replaces the previous NavBreadcrumbs row, which spent its width re-rendering
 * cursor scope (`c001 → i142 → 142/8421`) that's already shown in panel
 * badges and the StatusBar position counter — that duplication was flagged
 * in the UI review and the row paid its cost in vertical real estate.
 *
 * What we kept: the actual nav affordance + the history popover, both still
 * useful and not surfaced anywhere else.
 * What we dropped: the breadcrumb crumbs themselves (group/inst/position).
 */
export function NavHistoryControls({
  canGoBack,
  canGoForward,
  historyItems,
  onGoBack,
  onGoForward,
  onGoBackTo,
}: NavHistoryControlsProps) {
  const [showHistoryMenu, setShowHistoryMenu] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const historyButtonRef = useRef<HTMLButtonElement>(null);
  const historyDropdownRef = useRef<HTMLDivElement>(null);

  const historyPosition = useAnchoredFloatingPosition(
    showHistoryMenu,
    historyButtonRef,
    historyDropdownRef,
    { placement: "bottom-start", offset: 6 },
  );

  useEffect(() => {
    if (!showHistoryMenu) return;
    const onDocMouseDown = (event: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setShowHistoryMenu(false);
      }
    };
    document.addEventListener("mousedown", onDocMouseDown);
    return () => document.removeEventListener("mousedown", onDocMouseDown);
  }, [showHistoryMenu]);

  return (
    <div
      ref={containerRef}
      className="flex items-center gap-0.5 relative"
      data-design-surface="nav-history-controls"
    >
      <button
        ref={historyButtonRef}
        onClick={() => {
          if (!canGoBack) return;
          setShowHistoryMenu((prev) => !prev);
        }}
        onDoubleClick={onGoBack}
        disabled={!canGoBack}
        className={clsx(
          "tcld-nav-icon",
          !canGoBack && "is-disabled",
        )}
        title="History (click) · Back one step (double-click)"
        aria-label="History"
      >
        <ChevronLeft size={12} aria-hidden />
      </button>
      <button
        onClick={onGoForward}
        disabled={!canGoForward}
        className={clsx(
          "tcld-nav-icon",
          !canGoForward && "is-disabled",
        )}
        title="Forward (⌥→)"
        aria-label="Forward"
      >
        <ChevronRight size={12} aria-hidden />
      </button>

      {showHistoryMenu && historyItems.length > 0 && typeof document !== "undefined" && createPortal(
        <div
          ref={historyDropdownRef}
          className="fixed z-[var(--z-context-menu)] min-w-64 max-w-96 overflow-auto bg-tcld-bg-1 border border-tcld-border rounded shadow-[var(--shadow-popover)]"
          style={{
            left: historyPosition.left,
            top: historyPosition.top,
            maxHeight: historyPosition.maxHeight || 288,
            visibility: historyPosition.ready ? "visible" : "hidden",
          }}
        >
          {historyItems.map((item) => (
            <button
              key={`${item.offset}:${item.label}`}
              type="button"
              onClick={() => {
                onGoBackTo(item.offset);
                setShowHistoryMenu(false);
              }}
              className="w-full text-left px-2 py-1.5 hover:bg-tcld-bg-highlight border-b border-tcld-border-dim last:border-b-0"
            >
              <div className="font-mono text-xs text-tcld-fg">{item.label}</div>
              {item.detail && (
                <div className="font-mono text-label text-tcld-fg-dim mt-0.5">{item.detail}</div>
              )}
            </button>
          ))}
        </div>,
        document.body,
      )}
    </div>
  );
}
