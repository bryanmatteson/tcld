"use client";

import { useState, useRef, useEffect, useId, memo } from "react";
import { createPortal } from "react-dom";
import { Check, Plus, Search } from "lucide-react";
import { VIEW_DEFINITIONS, VIEW_DESCRIPTIONS } from "@/lib/view-registry";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";
import { useModalFocus } from "@/hooks/use-modal-focus";
import { ViewIcon } from "@/components/ViewIcon";
import type { ViewId } from "@/types";

interface PanelPickerProps {
  openPanels: ViewId[];
  onAddPanel: (type: ViewId) => void;
}

export const PanelPicker = memo(function PanelPicker({ openPanels, onAddPanel }: PanelPickerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [query, setQuery] = useState("");
  const menuRef = useModalFocus(isOpen, () => setIsOpen(false));
  const buttonRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const menuId = useId();
  const menuPosition = useAnchoredFloatingPosition(isOpen, buttonRef, menuRef, { placement: "bottom-end" });
  useEffect(() => {
    if (isOpen && menuPosition.ready) searchRef.current?.focus();
  }, [isOpen, menuPosition.ready]);
  const words = query.toLowerCase().trim().split(/\s+/);
  const views = VIEW_DEFINITIONS.filter((view) => words.every((word) =>
    `${view.label} ${VIEW_DESCRIPTIONS[view.id]}`.toLowerCase().includes(word)));

  useEffect(() => {
    if (!isOpen) return;
    const dismiss = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node) && !buttonRef.current?.contains(event.target as Node)) setIsOpen(false);
    };
    document.addEventListener("mousedown", dismiss);
    return () => document.removeEventListener("mousedown", dismiss);
  }, [isOpen, menuRef]);

  return (
    <div className="relative">
      <button
        ref={buttonRef}
        type="button"
        data-testid="panel-picker-trigger"
        onClick={() => { setQuery(""); setIsOpen(!isOpen); }}
        className="tcld-panel-picker-trigger"
        title="Open or focus a panel"
        aria-label="Add panel"
        aria-haspopup="dialog"
        aria-expanded={isOpen}
        aria-controls={isOpen ? menuId : undefined}
      >
        <Plus size={16} aria-hidden />
        <span className="tcld-chrome-label">Panels</span>
      </button>
      {isOpen && typeof document !== "undefined" && createPortal(
        <div
          id={menuId}
          ref={menuRef}
          tabIndex={-1}
          role="dialog"
          aria-modal="true"
          aria-label="Choose a panel"
          className="tcld-panel-picker fixed z-[var(--z-context-menu)]"
          style={{ left: menuPosition.left, top: menuPosition.top, maxHeight: "min(560px, calc(100dvh - 64px))", visibility: menuPosition.ready ? "visible" : "hidden" }}
          onKeyDown={(event) => {
            if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
            event.preventDefault();
            event.stopPropagation();
            const buttons = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[data-panel-picker-option]') ?? []);
            const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
            if (event.key === "ArrowUp" && index <= 0) searchRef.current?.focus();
            else buttons[event.key === "ArrowDown" ? Math.min(index + 1, buttons.length - 1) : index - 1]?.focus();
          }}
        >
          <div className="tcld-panel-picker-search">
            <Search size={16} aria-hidden />
            <input ref={searchRef} aria-label="Find a panel" placeholder="Find a panel…" value={query} onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && views[0]) { event.preventDefault(); onAddPanel(views[0].id); setIsOpen(false); }
              }} />
            <kbd>Esc</kbd>
          </div>
          <div className="tcld-panel-picker-list">
            {views.map((view) => {
              const alreadyOpen = openPanels.includes(view.id);
              return (
                <button key={view.id} type="button" data-panel-picker-option={view.id}
                  className="tcld-panel-picker-option"
                  title={`${alreadyOpen ? "Focus" : "Open"} ${view.label}`}
                  onClick={() => { onAddPanel(view.id); setIsOpen(false); }}>
                  <ViewIcon viewId={view.id} size={16} />
                  <span><strong>{view.label}</strong><small>{VIEW_DESCRIPTIONS[view.id]}</small></span>
                  <span className="tcld-panel-picker-state">{alreadyOpen ? <><Check size={12} aria-hidden /> Open</> : "Add"}</span>
                </button>
              );
            })}
            {views.length === 0 && <p className="tcld-picker-empty" role="status">No panels match “{query}”. Try source, variables, or flow.</p>}
          </div>
          <div className="tcld-palette-footer">Select a panel to open it or bring it into focus.</div>
        </div>, document.body,
      )}
    </div>
  );
});
