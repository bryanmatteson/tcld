"use client";

import { useState, useRef, useEffect } from "react";
import { createPortal } from "react-dom";
import { Check, Plus } from "lucide-react";
import clsx from "clsx";
import { VIEW_DEFINITIONS } from "@/lib/view-registry";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";
import { ViewIcon } from "@/components/ViewIcon";
import type { ViewId } from "@/types";

interface PanelPickerProps {
  openPanels: ViewId[];
  onAddPanel: (type: ViewId) => void;
}

export function PanelPicker({ openPanels, onAddPanel }: PanelPickerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuPosition = useAnchoredFloatingPosition(isOpen, buttonRef, menuRef, { placement: "bottom-end" });

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (
        menuRef.current &&
        !menuRef.current.contains(event.target as Node) &&
        buttonRef.current &&
        !buttonRef.current.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setIsOpen(false);
      }
    }

    if (isOpen) {
      document.addEventListener("mousedown", handleClickOutside);
      document.addEventListener("keydown", handleKeyDown);
      return () => {
        document.removeEventListener("mousedown", handleClickOutside);
        document.removeEventListener("keydown", handleKeyDown);
      };
    }
  }, [isOpen]);

  const handleAddPanel = (type: ViewId) => {
    onAddPanel(type);
    setIsOpen(false);
  };

  const menu = (
    <div
      ref={menuRef}
      className={clsx(
        "fixed",
        "min-w-[280px]",
        "z-[var(--z-context-menu)]",
        "bg-tcld-bg-1 border border-tcld-border rounded-lg shadow-[var(--shadow-popover)]",
        "overflow-hidden"
      )}
      style={{
        left: menuPosition.left,
        top: menuPosition.top,
        maxHeight: menuPosition.maxHeight || undefined,
        visibility: menuPosition.ready ? "visible" : "hidden",
      }}
    >
      {VIEW_DEFINITIONS.map((view) => {
        const isOpen = openPanels.includes(view.id);
        return (
          <button
            key={view.id}
            data-panel-picker-option={view.id}
            onClick={() => handleAddPanel(view.id)}
            className={clsx(
              "w-full px-4 py-2.5 text-left",
              "flex items-center justify-between",
              "font-mono text-sm",
              "border-b border-tcld-border-dim last:border-b-0",
              "transition-colors duration-75",
              isOpen
                ? "bg-tcld-bg-2 text-tcld-fg-dim"
                : "bg-tcld-bg-1 text-tcld-fg hover:bg-tcld-bg-highlight"
            )}
          >
            <div className="flex items-center gap-2">
              <span className="text-tcld-accent"><ViewIcon viewId={view.id} size={13} /></span>
              <span>{view.label}</span>
            </div>
            <div className="flex items-center gap-3">
              {isOpen && (
                <Check size={12} className="text-tcld-success" aria-hidden />
              )}
              <span className="text-tcld-fg-muted text-xs opacity-70">
                {view.key}
              </span>
            </div>
          </button>
        );
      })}
    </div>
  );

  return (
    <div className="relative">
      <button
        ref={buttonRef}
        data-testid="panel-picker-trigger"
        onClick={() => setIsOpen(!isOpen)}
        className={clsx(
          "flex items-center justify-center",
          "w-8 h-8 rounded-[var(--radius-button)]",
          "bg-tcld-bg-2 border border-tcld-border",
          "text-tcld-accent hover:border-tcld-border-focus hover:bg-tcld-bg-highlight",
          "transition-colors duration-100 ease-out",
          "cursor-pointer"
        )}
        title="Add panel"
        aria-label="Add panel"
      >
        <Plus size={16} aria-hidden />
      </button>

      {isOpen && typeof document !== "undefined" && createPortal(menu, document.body)}
    </div>
  );
}
