"use client";

import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { computeFloatingPosition, type FloatingPosition } from "@/lib/floating-position";

// ─── Types ──────────────────────────────────────────────────────

export interface ContextMenuItem {
  label: string;
  icon?: React.ReactNode;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
  onClick?: () => void;
}

interface ContextMenuProps {
  items: ContextMenuItem[];
  position: { x: number; y: number };
  onClose: () => void;
}

// ─── Component ──────────────────────────────────────────────────

export function ContextMenu({ items, position, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [focusedIndex, setFocusedIndex] = useState(-1);
  const [adjustedPosition, setAdjustedPosition] = useState<FloatingPosition>({ left: position.x, top: position.y, maxHeight: 0, ready: false });

  // Actionable item indices (skip separators + disabled)
  const actionableIndices = items
    .map((item, i) => (!item.separator && !item.disabled ? i : -1))
    .filter((i) => i >= 0);

  // Close on click-outside
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [onClose]);

  // Keyboard navigation
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setFocusedIndex((prev) => {
          const curPos = actionableIndices.indexOf(prev);
          const nextPos = curPos < actionableIndices.length - 1 ? curPos + 1 : 0;
          return actionableIndices[nextPos] ?? -1;
        });
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setFocusedIndex((prev) => {
          const curPos = actionableIndices.indexOf(prev);
          const nextPos = curPos > 0 ? curPos - 1 : actionableIndices.length - 1;
          return actionableIndices[nextPos] ?? -1;
        });
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        if (focusedIndex >= 0) {
          const item = items[focusedIndex];
          if (item && !item.disabled && !item.separator && item.onClick) {
            item.onClick();
            onClose();
          }
        }
        return;
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [onClose, focusedIndex, items, actionableIndices]);

  useLayoutEffect(() => {
    const update = () => {
      const el = menuRef.current;
      if (!el) return;
      setAdjustedPosition(computeFloatingPosition(
        { left: position.x, top: position.y, width: 0, height: 0 },
        el.getBoundingClientRect(),
        { width: window.innerWidth, height: window.innerHeight },
        { placement: "bottom-start", offset: 2 },
      ));
    };

    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [position]);

  // Portal to document.body so position: fixed actually positions relative
  // to the viewport. Without this, ancestor `transform` properties on the
  // panel-tile wrapper (set by react-mosaic / DnD scale animations) create
  // a containing block for fixed-position descendants, so the menu lands
  // at an offset from the cursor.
  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      // Position is computed dynamically (collision-aware), so it stays an
      // inline style. Everything else lives in tokens via tcld-popover-surface.
      style={{
        position: "fixed",
        left: adjustedPosition.left,
        top: adjustedPosition.top,
        zIndex: 9999,
        visibility: adjustedPosition.ready ? "visible" : "hidden",
        maxHeight: adjustedPosition.maxHeight || undefined,
      }}
      className="tcld-popover-surface min-w-[180px] overflow-y-auto py-1 font-mono text-meta"
    >
      {items.map((item, i) => {
        if (item.separator) {
          return (
            <div
              key={i}
              role="separator"
              className="my-1 mx-2 h-px bg-tcld-border-dim"
            />
          );
        }

        const isFocused = focusedIndex === i;

        return (
          <div
            key={i}
            role="menuitem"
            aria-disabled={item.disabled || undefined}
            className={clsx(
              "flex items-center gap-2 px-3 py-1 transition-colors duration-75",
              item.disabled
                ? "cursor-default opacity-35"
                : "cursor-pointer",
              isFocused && !item.disabled && "bg-tcld-bg-highlight",
              item.danger ? "text-tcld-error" : "text-tcld-fg",
            )}
            onMouseEnter={() => !item.disabled && setFocusedIndex(i)}
            onMouseLeave={() => setFocusedIndex(-1)}
            onClick={() => {
              if (!item.disabled && item.onClick) {
                item.onClick();
                onClose();
              }
            }}
          >
            {/* Icon column */}
            <span
              aria-hidden
              className="flex h-3.5 w-3.5 shrink-0 items-center justify-center text-meta"
            >
              {item.icon ?? ""}
            </span>

            {/* Label */}
            <span className="flex-1 truncate">{item.label}</span>

            {/* Shortcut hint */}
            {item.shortcut && (
              <span className="shrink-0 text-label text-tcld-fg-dim">
                {item.shortcut}
              </span>
            )}
          </div>
        );
      })}
    </div>,
    document.body,
  );
}
