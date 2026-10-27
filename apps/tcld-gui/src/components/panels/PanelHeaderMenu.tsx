"use client";

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ChevronRight, MoreHorizontal } from "lucide-react";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";
import type { PanelActionItem } from "./PanelContext";

interface PanelHeaderMenuProps {
  items: PanelActionItem[];
  /** Optional id used as a data-attribute on the trigger for tests / panel discovery. */
  panelId?: string;
  /**
   * Back-compat slot rendered as the first section of the dropdown body
   * (above the typed items). Used to host the existing per-view leaf menus
   * (swap view, add view) until they're migrated to typed `PanelActionItem`s.
   */
  legacyActions?: React.ReactNode;
}

const SUBMENU_HOVER_OPEN_DELAY = 80;
const SUBMENU_HOVER_CLOSE_DELAY = 140;

/**
 * Single ⋯ button that opens a dropdown menu containing every panel-header
 * action (add view, swap view, maximize/restore, close, …). Replaces the
 * old inline cluster so the header reserves its space for tabs.
 *
 * Items with a `submenu` field render a `▸` glyph. Hovering the row (or
 * focusing it via keyboard + ArrowRight) opens a flyout to the right, like
 * a standard menu-bar submenu. The flyout follows the hovered row as the
 * user moves the cursor through the parent menu.
 */
export const PanelHeaderMenu = React.memo(function PanelHeaderMenu({ items, panelId, legacyActions }: PanelHeaderMenuProps) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const pos = useAnchoredFloatingPosition(open, triggerRef, popRef, { placement: "bottom-end", offset: 6 });

  // ─── Submenu state ────────────────────────────────────────────
  // openSubmenuId tracks which item's submenu is open; openSubmenuTrigger
  // holds its DOM button so the flyout can anchor to it. Both are state
  // (not refs) so React's ref-during-render rule is satisfied.
  const [openSubmenuId, setOpenSubmenuId] = useState<string | null>(null);
  const [openSubmenuTrigger, setOpenSubmenuTrigger] = useState<HTMLButtonElement | null>(null);
  const submenuTriggerRefs = useRef<Map<string, HTMLButtonElement>>(new Map());
  const submenuOpenTimerRef = useRef<number | null>(null);
  const submenuCloseTimerRef = useRef<number | null>(null);

  const openSubmenu = useCallback((id: string) => {
    setOpenSubmenuId(id);
    setOpenSubmenuTrigger(submenuTriggerRefs.current.get(id) ?? null);
  }, []);

  const clearSubmenuTimers = useCallback(() => {
    if (submenuOpenTimerRef.current != null) {
      window.clearTimeout(submenuOpenTimerRef.current);
      submenuOpenTimerRef.current = null;
    }
    if (submenuCloseTimerRef.current != null) {
      window.clearTimeout(submenuCloseTimerRef.current);
      submenuCloseTimerRef.current = null;
    }
  }, []);

  const scheduleSubmenuOpen = useCallback((id: string) => {
    clearSubmenuTimers();
    submenuOpenTimerRef.current = window.setTimeout(() => {
      openSubmenu(id);
    }, SUBMENU_HOVER_OPEN_DELAY);
  }, [clearSubmenuTimers, openSubmenu]);

  const scheduleSubmenuClose = useCallback(() => {
    clearSubmenuTimers();
    submenuCloseTimerRef.current = window.setTimeout(() => {
      setOpenSubmenuId(null);
      setOpenSubmenuTrigger(null);
    }, SUBMENU_HOVER_CLOSE_DELAY);
  }, [clearSubmenuTimers]);

  const cancelSubmenuClose = useCallback(() => {
    if (submenuCloseTimerRef.current != null) {
      window.clearTimeout(submenuCloseTimerRef.current);
      submenuCloseTimerRef.current = null;
    }
  }, []);

  const closeMenu = useCallback(() => {
    clearSubmenuTimers();
    setOpen(false);
    setOpenSubmenuId(null);
    setOpenSubmenuTrigger(null);
  }, [clearSubmenuTimers]);

  useEffect(() => () => clearSubmenuTimers(), [clearSubmenuTimers]);

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Node;
      if (wrapRef.current?.contains(target)) return;
      // The menu itself is portaled to document.body (outside wrapRef):
      // pointerdown on a menu item must NOT count as an outside click, or
      // the menu unmounts before the item's click event can fire.
      if (popRef.current?.contains(target)) return;
      // The submenu portal lives outside wrapRef too — don't close when
      // clicks land inside it.
      if (popRef.current?.querySelector('[data-submenu-portal="true"]')?.contains(target)) return;
      const submenuRoot = document.querySelector(`[data-panel-submenu-popover="${panelId ?? "default"}"]`);
      if (submenuRoot?.contains(target)) return;
      closeMenu();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (openSubmenuId) {
          // Focus the parent trigger row before clearing state so the user
          // doesn't lose keyboard context when collapsing the submenu.
          openSubmenuTrigger?.focus();
          setOpenSubmenuId(null);
          setOpenSubmenuTrigger(null);
        } else {
          closeMenu();
          triggerRef.current?.focus();
        }
      }
    };
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, openSubmenuId, openSubmenuTrigger, closeMenu, panelId]);

  const focusItem = useCallback((index: number) => {
    // rAF avoids the focus-loss race the setTimeout(0) version had: if the
    // popover unmounts between the timeout schedule and its callback, focus
    // would land on document.body. rAF runs before the next paint, so the
    // popRef is either present (we focus) or already gone (we no-op). Either
    // way we never strand keyboard focus.
    requestAnimationFrame(() => {
      const buttons = popRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]');
      buttons?.[index]?.focus();
    });
  }, []);

  const handleTriggerKey = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    setOpen(true);
    focusItem(0);
  };

  const handleMenuKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
    const idx = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const move = (offset: number) => {
      if (buttons.length === 0) return;
      const base = idx >= 0 ? idx : 0;
      buttons[(base + offset + buttons.length) % buttons.length]?.focus();
    };
    if (event.key === "ArrowDown") { event.preventDefault(); move(1); }
    else if (event.key === "ArrowUp") { event.preventDefault(); move(-1); }
    else if (event.key === "Home") { event.preventDefault(); buttons[0]?.focus(); }
    else if (event.key === "End") { event.preventDefault(); buttons[buttons.length - 1]?.focus(); }
  };

  // ─── Submenu flyout (memoized parent) ─────────────────────────
  // NOTE: This `useMemo` must be hoisted above any early `return` so it's
  // called unconditionally on every render — React Hooks rule.
  const openSubmenuItem = useMemo(
    () => (openSubmenuId ? items.find((item) => item.id === openSubmenuId) ?? null : null),
    [items, openSubmenuId],
  );

  if (items.length === 0 && !legacyActions) return null;

  const handleSelect = (item: PanelActionItem) => {
    if (item.submenu) {
      openSubmenu(item.id);
      return;
    }
    item.onSelect?.();
    closeMenu();
    triggerRef.current?.focus();
  };

  const renderRow = (item: PanelActionItem) => {
    const hasSubmenu = !!item.submenu;
    const isSubmenuOpen = openSubmenuId === item.id;

    return (
      <React.Fragment key={item.id}>
        <button
          ref={hasSubmenu
            ? (node) => {
                if (node) submenuTriggerRefs.current.set(item.id, node);
                else submenuTriggerRefs.current.delete(item.id);
              }
            : undefined}
          type="button"
          role="menuitem"
          data-panel-menu-item={item.id}
          data-has-submenu={hasSubmenu ? "true" : undefined}
          data-submenu-open={isSubmenuOpen ? "true" : undefined}
          aria-haspopup={hasSubmenu ? "menu" : undefined}
          aria-expanded={hasSubmenu ? isSubmenuOpen : undefined}
          disabled={item.disabled}
          // stopPropagation: the menu is portaled, so React bubbles this
          // click up the COMPONENT tree into the Panel root's
          // onClick={onFocus}, which re-activates the panel's current tab
          // and silently undoes selections that changed the active view
          // (same bug TabOverflowMenu fixed for its items).
          onClick={(event) => {
            event.stopPropagation();
            handleSelect(item);
          }}
          onKeyDown={(event) => {
            if (hasSubmenu && (event.key === "ArrowRight" || event.key === "Enter" || event.key === " ")) {
              event.preventDefault();
              openSubmenu(item.id);
              // Defer focus into submenu after it mounts.
              window.setTimeout(() => {
                const sub = document.querySelector<HTMLButtonElement>(
                  `[data-panel-submenu-popover="${panelId ?? "default"}"][data-submenu-of="${item.id}"] [role="menuitem"]`,
                );
                sub?.focus();
              }, 0);
            }
          }}
          onMouseEnter={() => {
            if (hasSubmenu) {
              cancelSubmenuClose();
              if (openSubmenuId !== item.id) scheduleSubmenuOpen(item.id);
            } else {
              // Hovering a non-submenu row dismisses any open submenu so the
              // user isn't visually distracted.
              if (openSubmenuId) scheduleSubmenuClose();
            }
          }}
          onMouseLeave={() => {
            if (hasSubmenu) scheduleSubmenuClose();
          }}
          className={clsx(
            "flex w-full items-center gap-2 px-3 h-8 text-left",
            "font-ui text-xs",
            isSubmenuOpen && "bg-tcld-bg-highlight text-tcld-fg",
            item.disabled
              ? "text-tcld-text-inactive opacity-50 cursor-not-allowed"
              : item.danger
                ? "text-tcld-error hover:bg-tcld-error/10"
                : "text-tcld-fg hover:bg-tcld-bg-highlight",
          )}
        >
          {item.icon && (
            <span className="flex h-3.5 w-3.5 flex-shrink-0 items-center justify-center">
              {item.icon}
            </span>
          )}
          <span className="flex-1 truncate">{item.label}</span>
          {item.detail && !hasSubmenu && (
            <span className="text-label text-tcld-text-keyhint">{item.detail}</span>
          )}
          {hasSubmenu && (
            <ChevronRight size={12} aria-hidden className="text-tcld-text-inactive" />
          )}
        </button>
        {item.separatorAfter && (
          <div className="my-1 border-t border-tcld-border-dim" aria-hidden />
        )}
      </React.Fragment>
    );
  };

  const menu = (
    <div
      ref={popRef}
      role="menu"
      data-panel-header-menu="true"
      data-panel-id={panelId}
      onKeyDown={handleMenuKey}
      onMouseLeave={() => {
        // Closing the parent → close the submenu after a beat too, so a
        // user moving from menu to submenu has time to cross the gap.
        if (openSubmenuId) scheduleSubmenuClose();
      }}
      onMouseEnter={cancelSubmenuClose}
      className="fixed z-[var(--z-context-menu)] min-w-[220px] overflow-visible rounded-md border border-tcld-border bg-tcld-bg-1 py-1 shadow-[var(--shadow-popover)]"
      style={{
        left: pos.left,
        top: pos.top,
        maxHeight: pos.maxHeight || undefined,
        visibility: pos.ready ? "visible" : "hidden",
      }}
    >
      {legacyActions && (
        <>
          <div className="px-2 py-1 flex flex-col gap-0.5" onClick={(e) => e.stopPropagation()}>
            {legacyActions}
          </div>
          {items.length > 0 && <div className="my-1 border-t border-tcld-border-dim" aria-hidden />}
        </>
      )}
      {items.map(renderRow)}
    </div>
  );

  return (
    <div ref={wrapRef} className="relative">
      <button
        ref={triggerRef}
        type="button"
        data-panel-header-menu-trigger="true"
        data-panel-interactive="true"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Panel actions"
        title="Panel actions"
        onClick={(event) => {
          event.stopPropagation();
          if (open) {
            closeMenu();
          } else {
            setOpen(true);
          }
        }}
        onPointerDown={(event) => event.stopPropagation()}
        onKeyDown={handleTriggerKey}
        // Render as the canonical tcld-pt-btn so the panel header's
        // ⋯ menu button matches the workbench mock — 22×22, accent
        // wash on hover/open, rounded-3.
        data-pressed={open ? "true" : undefined}
        className={clsx("tcld-pt-btn", open && "is-pressed")}
      >
        <MoreHorizontal size={12} aria-hidden />
      </button>
      {open && typeof document !== "undefined" && createPortal(menu, document.body)}
      {open && openSubmenuItem && openSubmenuTrigger && typeof document !== "undefined" &&
        createPortal(
          <SubmenuFlyout
            panelId={panelId ?? "default"}
            parent={openSubmenuItem}
            anchor={openSubmenuTrigger}
            onSelect={(child) => {
              child.onSelect?.();
              closeMenu();
              triggerRef.current?.focus();
            }}
            onClose={() => {
              openSubmenuTrigger.focus();
              setOpenSubmenuId(null);
              setOpenSubmenuTrigger(null);
            }}
            onMouseEnter={cancelSubmenuClose}
            onMouseLeave={scheduleSubmenuClose}
          />,
          document.body,
        )}
    </div>
  );
});

// ─── Submenu flyout (separate portal anchored to the hovered row) ───────

interface SubmenuFlyoutProps {
  panelId: string;
  parent: PanelActionItem;
  anchor: HTMLButtonElement;
  onSelect: (child: PanelActionItem) => void;
  onClose: () => void;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}

function SubmenuFlyout({
  panelId,
  parent,
  anchor,
  onSelect,
  onClose,
  onMouseEnter,
  onMouseLeave,
}: SubmenuFlyoutProps) {
  const items = parent.submenu ?? [];
  const popRef = useRef<HTMLDivElement>(null);

  // Position the flyout adjacent to the parent row. Default placement is
  // to the right; flip to the left if there's not enough room.
  const [position, setPosition] = useState<{ left: number; top: number; ready: boolean }>(
    { left: 0, top: 0, ready: false },
  );
  useEffect(() => {
    const measure = () => {
      const rect = anchor.getBoundingClientRect();
      const popWidth = popRef.current?.offsetWidth ?? 220;
      const popHeight = popRef.current?.offsetHeight ?? 200;
      const margin = 4;
      const maxLeft = window.innerWidth - popWidth - margin;
      const preferRightLeft = rect.right + margin;
      const fallbackLeftLeft = Math.max(margin, rect.left - popWidth - margin);
      const left = preferRightLeft + popWidth > window.innerWidth - margin
        ? fallbackLeftLeft
        : preferRightLeft;
      const top = Math.min(
        Math.max(margin, rect.top - 4),
        Math.max(margin, window.innerHeight - popHeight - margin),
      );
      setPosition({ left: Math.min(left, maxLeft), top, ready: true });
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [anchor, items.length]);

  const handleKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
    const idx = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "ArrowDown") {
      event.preventDefault();
      buttons[(Math.max(0, idx) + 1) % buttons.length]?.focus();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      buttons[(Math.max(0, idx) - 1 + buttons.length) % buttons.length]?.focus();
    } else if (event.key === "ArrowLeft" || event.key === "Escape") {
      event.preventDefault();
      onClose();
    } else if (event.key === "Home") {
      event.preventDefault();
      buttons[0]?.focus();
    } else if (event.key === "End") {
      event.preventDefault();
      buttons[buttons.length - 1]?.focus();
    }
  };

  return (
    <div
      ref={popRef}
      role="menu"
      data-panel-submenu-popover={panelId}
      data-submenu-of={parent.id}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      onKeyDown={handleKey}
      className="fixed z-[calc(var(--z-context-menu)+1)] min-w-[200px] max-w-[300px] overflow-auto rounded-md border border-tcld-border bg-tcld-bg-1 py-1 shadow-[var(--shadow-popover)]"
      style={{
        left: position.left,
        top: position.top,
        maxHeight: Math.max(160, window.innerHeight - position.top - 16),
        visibility: position.ready ? "visible" : "hidden",
      }}
    >
      {items.length === 0 ? (
        <div className="px-3 py-2 font-ui text-meta text-tcld-text-inactive">
          {parent.submenuEmptyLabel ?? "No options available"}
        </div>
      ) : (
        items.map((item) => (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            data-panel-submenu-item={item.id}
            disabled={item.disabled}
            // stopPropagation for the same portal-bubbling reason as the
            // parent menu rows: without it the Panel root's onClick={onFocus}
            // re-activates the previous tab and overrides this selection.
            onClick={(event) => {
              event.stopPropagation();
              onSelect(item);
            }}
            className={clsx(
              "flex w-full items-center gap-2 px-3 h-8 text-left",
              "font-ui text-xs",
              item.disabled
                ? "text-tcld-text-inactive opacity-50 cursor-not-allowed"
                : item.danger
                  ? "text-tcld-error hover:bg-tcld-error/10"
                  : "text-tcld-fg hover:bg-tcld-bg-highlight",
            )}
          >
            {item.icon && (
              <span className="flex h-3.5 w-3.5 flex-shrink-0 items-center justify-center">
                {item.icon}
              </span>
            )}
            <span className="flex-1 truncate">{item.label}</span>
            {item.detail && (
              <span className="text-label text-tcld-text-keyhint">{item.detail}</span>
            )}
          </button>
        ))
      )}
    </div>
  );
}
