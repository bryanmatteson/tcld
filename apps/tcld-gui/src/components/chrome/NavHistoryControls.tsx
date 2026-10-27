"use client";

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ChevronDown, ChevronLeft, ChevronRight } from "lucide-react";
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

/** Familiar back/forward navigation with a separate history picker. */
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
    if (showHistoryMenu && historyPosition.ready) {
      historyDropdownRef.current?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
    }
  }, [showHistoryMenu, historyPosition.ready]);

  useEffect(() => {
    if (!showHistoryMenu) return;
    const onDocMouseDown = (event: MouseEvent) => {
      if (!containerRef.current?.contains(event.target as Node) && !historyDropdownRef.current?.contains(event.target as Node)) {
        setShowHistoryMenu(false);
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setShowHistoryMenu(false);
      historyButtonRef.current?.focus();
    };
    document.addEventListener("mousedown", onDocMouseDown);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDocMouseDown);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [showHistoryMenu]);

  return (
    <div
      ref={containerRef}
      className="flex items-center gap-0.5 relative"
      data-design-surface="nav-history-controls"
    >
      <button
        type="button"
        onClick={onGoBack}
        disabled={!canGoBack}
        className={clsx("tcld-nav-icon", !canGoBack && "is-disabled")}
        title="Back (Alt+Left)"
        aria-label="Back"
      >
        <ChevronLeft size={15} aria-hidden />
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
        <ChevronRight size={15} aria-hidden />
      </button>

      <button
        ref={historyButtonRef}
        type="button"
        onClick={() => setShowHistoryMenu((prev) => !prev)}
        disabled={historyItems.length === 0}
        className={clsx("tcld-nav-icon", historyItems.length === 0 && "is-disabled")}
        title="Navigation history"
        aria-label="History"
        aria-expanded={showHistoryMenu}
        aria-haspopup="menu"
      >
        <ChevronDown size={13} aria-hidden />
      </button>

      {showHistoryMenu && historyItems.length > 0 && typeof document !== "undefined" && createPortal(
        <div
          ref={historyDropdownRef}
          role="menu"
          onKeyDown={(event) => {
            event.stopPropagation();
            const items = Array.from(historyDropdownRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
            const index = items.indexOf(document.activeElement as HTMLButtonElement);
            const next = event.key === "ArrowDown" ? (index + 1) % items.length
              : event.key === "ArrowUp" ? (index - 1 + items.length) % items.length
              : event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : null;
            if (next !== null) { event.preventDefault(); event.stopPropagation(); items[next]?.focus(); }
            if (event.key === "Tab") setShowHistoryMenu(false);
          }}
          aria-label="Navigation history"
          className="fixed z-[var(--z-context-menu)] min-w-64 max-w-96 overflow-auto bg-tcld-bg-1 border border-tcld-border rounded shadow-[var(--shadow-popover)]"
          style={{
            left: historyPosition.left,
            top: historyPosition.top,
            maxHeight: historyPosition.maxHeight || 288,
            visibility: historyPosition.ready ? "visible" : "hidden",
          }}
        >
          <div className="tcld-layout-menu-heading">Recent locations</div>
          {historyItems.map((item) => (
            <button
              key={`${item.offset}:${item.label}`}
              role="menuitem"
              type="button"
              onClick={() => {
                onGoBackTo(item.offset);
                setShowHistoryMenu(false);
                historyButtonRef.current?.focus();
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
