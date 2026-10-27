"use client";

import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ChevronDown, X } from "lucide-react";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";
import { TabBadge } from "@/components/primitives/TabBadge";
import type { ViewId } from "@/types";

export interface OverflowTab {
  id: ViewId;
  label: string;
  icon?: React.ReactNode;
  badge?: string | number | null;
  active?: boolean;
}

interface TabOverflowMenuProps {
  tabs: OverflowTab[];
  onSelect: (tabId: ViewId) => void;
  onClose?: (tabId: ViewId) => void;
}

/**
 * "+N" button shown after the inline tab strip when one or more tabs can't
 * fit. Clicking opens a dropdown listing the overflowed tabs with their
 * icon, label, and badge — clicking a row activates that tab.
 */
export function TabOverflowMenu({ tabs, onSelect, onClose }: TabOverflowMenuProps) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const pos = useAnchoredFloatingPosition(open, triggerRef, popRef, { placement: "bottom-end", offset: 6 });

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Node;
      // The menu is portaled to document.body so it isn't a descendant of
      // wrapRef. Without this popRef containment check, pointerdown inside
      // the menu was treated as an outside click — the menu unmounted
      // before the menuitem's onClick could fire and selection silently
      // no-op'd.
      if (wrapRef.current?.contains(target)) return;
      if (popRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (tabs.length === 0) return null;

  const menu = (
    <div
      ref={popRef}
      role="menu"
      data-tab-overflow-menu="true"
      className="fixed z-[var(--z-context-menu)] min-w-[220px] max-w-[320px] overflow-auto rounded-md border border-tcld-border bg-tcld-bg-1 py-1 shadow-[var(--shadow-popover)]"
      style={{
        left: pos.left,
        top: pos.top,
        maxHeight: pos.maxHeight || 320,
        visibility: pos.ready ? "visible" : "hidden",
      }}
    >
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className="group relative flex items-stretch"
          data-overflow-tab-id={tab.id}
        >
          {/*
           * The menu item is a real <button> so onClick is reliable across
           * browsers and accessibility tree. stopPropagation on click and
           * mousedown both — the parent panel's onFocus handler was racing
           * with selection and overriding the active tab back.
           */}
          <button
            type="button"
            role="menuitem"
            onClick={(event) => {
              event.stopPropagation();
              onSelect(tab.id);
              setOpen(false);
            }}
            onMouseDown={(event) => {
              event.stopPropagation();
            }}
            className={clsx(
              "flex flex-1 items-center gap-2 px-3 h-8 text-left",
              "font-ui text-xs cursor-pointer",
              tab.active
                ? "bg-tcld-bg-highlight text-tcld-fg"
                : "text-tcld-text-inactive hover:text-tcld-fg hover:bg-tcld-bg-highlight",
            )}
          >
            {tab.icon && (
              <span className="flex h-3.5 w-3.5 flex-shrink-0 items-center justify-center">
                {tab.icon}
              </span>
            )}
            <span className="flex-1 truncate">{tab.label}</span>
            {tab.badge != null && tab.badge !== "" && (
              <TabBadge value={tab.badge} active={!!tab.active} />
            )}
          </button>
          {onClose && (
            <button
              type="button"
              aria-label={`Close ${tab.label}`}
              title={`Close ${tab.label}`}
              onClick={(event) => {
                event.stopPropagation();
                onClose(tab.id);
              }}
              onMouseDown={(event) => event.stopPropagation()}
              className={clsx(
                "inline-flex items-center justify-center w-6 self-center mr-1.5 h-4 rounded",
                "text-tcld-text-inactive hover:text-tcld-fg hover:bg-tcld-bg-highlight",
                "opacity-0 group-hover:opacity-80 focus:opacity-100",
              )}
            >
              <X size={11} aria-hidden />
            </button>
          )}
        </div>
      ))}
    </div>
  );

  return (
    <div ref={wrapRef} className="relative">
      <button
        ref={triggerRef}
        type="button"
        data-tab-overflow-trigger="true"
        data-panel-interactive="true"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Show ${tabs.length} more tab${tabs.length === 1 ? "" : "s"}`}
        title={`${tabs.length} more tab${tabs.length === 1 ? "" : "s"}`}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((prev) => !prev);
        }}
        onPointerDown={(event) => event.stopPropagation()}
        className={clsx(
          "tcld-view-tab-overflow inline-flex items-center gap-1 px-2 h-7 rounded",
          "font-ui text-meta",
          "border border-tcld-border-dim bg-tcld-bg-2",
          "text-tcld-text-inactive hover:text-tcld-fg hover:border-tcld-border",
          "transition-colors duration-150 flex-shrink-0",
          open && "text-tcld-fg border-tcld-border bg-tcld-bg-highlight",
        )}
      >
        <span className="tabular-nums">+{tabs.length}</span>
        <ChevronDown size={11} aria-hidden />
      </button>
      {open && typeof document !== "undefined" && createPortal(menu, document.body)}
    </div>
  );
}
