"use client";

import React, { useCallback, useEffect, useMemo, useRef, useState, memo } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { ChevronDown, LayoutDashboard } from "lucide-react";
import { getOpenViews, LAYOUT_PRESETS, workspaceFromViewTree } from "@/lib/default-layouts";
import { VIEW_TITLES } from "@/lib/view-registry";
import { loadNamedLayouts, type NamedWorkspaceLayout } from "@/lib/layout-storage";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";
import type { LayoutPreset, WorkspaceLayoutSnapshot } from "@/types";

interface MainToolbarProps {
  onLoadPreset: (name: string) => void;
  /** Persists the current workspace as a named layout (use-explorer.saveLayout). */
  onSaveLayout?: () => void;
  layoutDirty?: boolean;
  /**
   * The id of the currently-loaded preset (built-in or saved). When `null`,
   * the toolbar shows a generic "Custom" affordance. When set, the trigger
   * label is derived from the preset's label rather than initialized from
   * `LAYOUT_PRESETS[0]`.
   */
  activePresetId?: string | null;
  activePresetLabel?: string | null;
  launchControls?: React.ReactNode;
}

const CUSTOM_LAYOUT_LABEL = "Custom";

export const MainToolbar = memo(function MainToolbar({
  onLoadPreset,
  onSaveLayout,
  layoutDirty = false,
  activePresetId = null,
  activePresetLabel = null,
  launchControls,
}: MainToolbarProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  // Initialize with the saved layouts at first render — avoids the lint
  // warning about calling setState inside a mount-only useEffect, and is
  // cleaner anyway. We re-read on menu open so newly-saved layouts appear.
  const [savedLayouts, setSavedLayouts] = useState<NamedWorkspaceLayout[]>(loadNamedLayouts);
  const menuRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  /**
   * Anchor coordinates for the layout menu. The menu renders into a portal
   * on document.body so it escapes the workspace's transform-induced
   * stacking context (every DndTile has `transform: scale(...)` which
   * traps `position: fixed` descendants in the local containing block;
   * portaling to body restores the viewport-relative positioning the
   * dropdown needs to land above the panel chrome rather than under it).
   */
  const menuFloatingPosition = useAnchoredFloatingPosition(menuOpen, triggerRef, menuRef, {
    placement: "bottom-start",
    offset: 4,
  });

  const refreshSavedLayouts = useCallback(() => {
    setSavedLayouts(loadNamedLayouts());
  }, []);

  // Wire menu-open behavior: refresh the saved-layouts snapshot in the
  // toggle handler (not in an effect) so we don't cascade renders, and
  // keep the Escape listener side-effect inside the effect where it belongs.
  useEffect(() => {
    if (!menuOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMenuOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [menuOpen]);

  const openMenu = useCallback(() => {
    refreshSavedLayouts();
    setMenuOpen(true);
  }, [refreshSavedLayouts]);
  const toggleMenu = useCallback(() => {
    setMenuOpen((prev) => {
      if (!prev) refreshSavedLayouts();
      return !prev;
    });
  }, [refreshSavedLayouts]);

  // Visible label is derived from the active preset id, with a graceful
  // fallback chain. Do not initialize from LAYOUT_PRESETS[0]; that would show
  // a preset that was not loaded.
  const triggerLabel = useMemo(() => {
    if (activePresetLabel) return shortLayoutName(activePresetLabel);
    if (activePresetId) {
      const builtIn = LAYOUT_PRESETS.find((preset) => preset.name === activePresetId);
      if (builtIn) return shortLayoutName(builtIn.label);
      const saved = savedLayouts.find((layout) => layout.id === activePresetId);
      if (saved) return shortLayoutName(saved.name);
    }
    return CUSTOM_LAYOUT_LABEL;
  }, [activePresetId, activePresetLabel, savedLayouts]);

  const applyLayout = (id: string) => {
    onLoadPreset(id);
    setMenuOpen(false);
    refreshSavedLayouts();
    triggerRef.current?.focus();
  };
  const focusMenuItem = useCallback((index: number) => {
    // Use rAF instead of setTimeout(0) so the focus call lands after the
    // browser has flushed the layout that mounted the menu, but before
    // paint — preventing the brief visible "no focus" flash that the
    // setTimeout pattern produced. We still need to defer past the same
    // synchronous tick that calls openMenu(); rAF gives us that for free.
    requestAnimationFrame(() => {
      const items = menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]');
      items?.[index]?.focus();
    });
  }, []);
  const handleTriggerKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    openMenu();
    focusMenuItem(0);
  };
  const handleMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const nextIndex = (offset: number) => {
      if (items.length === 0) return;
      const base = current >= 0 ? current : 0;
      items[(base + offset + items.length) % items.length]?.focus();
    };
    if (event.key === "ArrowDown") {
      event.preventDefault();
      nextIndex(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      nextIndex(-1);
    } else if (event.key === "Home") {
      event.preventDefault();
      items[0]?.focus();
    } else if (event.key === "End") {
      event.preventDefault();
      items[items.length - 1]?.focus();
    }
  };
  return (
    <div className="tcld-main-toolbar tcld-main-toolbar-inline flex items-center gap-2" data-design-surface="action-bar">
      <div className="tcld-actionbar-left">
        <div className="relative" ref={menuRef}>
          <button
            type="button"
            ref={triggerRef}
            data-testid="layout-preset-trigger"
            data-layout-dirty={layoutDirty ? "true" : "false"}
            className={clsx(
              "tcld-control-field tcld-layout-menu-trigger tcld-layout-select flex items-center gap-2 bg-tcld-bg-2 px-2 py-1 text-xs text-tcld-fg outline-none",
              layoutDirty && "border-tcld-accent text-tcld-accent"
            )}
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={toggleMenu}
            onKeyDown={handleTriggerKeyDown}
          >
            <LayoutDashboard size={12} aria-hidden />
            <span className="tcld-layout-menu-label">Layout</span>
            <span className="tcld-layout-menu-separator" aria-hidden />
            <span className="max-w-40 truncate">{triggerLabel}</span>
            {layoutDirty && <span className="tcld-layout-dirty-dot" aria-label="Unsaved layout changes" />}
            <ChevronDown size={12} aria-hidden />
          </button>

          {menuOpen && typeof document !== "undefined" && createPortal(
            <>
              {/* Click-outside backdrop. Sits one tier below the menu in
                  the canonical z-index scale so it intercepts pointer
                  events for any UI behind the menu but stays under the
                  menu itself. */}
              <button
                type="button"
                data-testid="layout-menu-backdrop"
                className="fixed inset-0 z-[calc(var(--z-context-menu)-1)] cursor-default bg-transparent"
                aria-label="Close layout menu"
                onClick={() => setMenuOpen(false)}
              />
              <div
                ref={menuRef}
                data-testid="layout-preset-menu"
                className="tcld-layout-menu fixed z-[var(--z-context-menu)] w-[340px] max-w-[calc(100vw-24px)] overflow-hidden rounded border border-tcld-border bg-tcld-bg-1 py-1 shadow-[var(--shadow-popover)]"
                role="menu"
                onKeyDown={handleMenuKeyDown}
                style={{
                  left: menuFloatingPosition.left,
                  top: menuFloatingPosition.top,
                  maxHeight: menuFloatingPosition.maxHeight || undefined,
                  overflowY: "auto",
                  visibility: menuFloatingPosition.ready ? "visible" : "hidden",
                }}
              >
                <div data-testid="layout-builtins-group" className="tcld-layout-menu-heading">
                  Presets
                </div>
                {LAYOUT_PRESETS.map((preset) => (
                  <LayoutMenuItem
                    key={preset.name}
                    id={preset.name}
                    kind="built-in"
                    active={preset.name === activePresetId && !layoutDirty}
                    label={shortLayoutName(preset.label)}
                    detail={describePreset(preset)}
                    onClick={() => applyLayout(preset.name)}
                  />
                ))}

                {savedLayouts.length > 0 && (
                  <>
                    <div className="mt-1 border-t border-tcld-border-dim tcld-layout-menu-heading">
                      Saved
                    </div>
                    <div data-testid="layout-saved-group">
                      {savedLayouts.map((layout) => (
                        <LayoutMenuItem
                          key={layout.id}
                          id={layout.id}
                          kind="saved"
                          active={layout.id === activePresetId && !layoutDirty}
                          label={shortLayoutName(layout.name)}
                          detail={describeWorkspace(layout.layout)}
                          onClick={() => applyLayout(layout.id)}
                        />
                      ))}
                    </div>
                  </>
                )}

                {/* Save the current (dirty) workspace as a named layout.
                    Lives in the Layout menu since the inline toolbar button
                    was retired with the slimmed action bar. */}
                <div className="mt-1 border-t border-tcld-border-dim" aria-hidden />
                <button
                  type="button"
                  role="menuitem"
                  data-testid="save-layout"
                  data-layout-dirty={layoutDirty ? "true" : "false"}
                  disabled={!layoutDirty || !onSaveLayout}
                  className="tcld-layout-menu-item disabled:cursor-not-allowed disabled:opacity-45"
                  onClick={() => {
                    onSaveLayout?.();
                    setMenuOpen(false);
                  }}
                  onKeyDown={(event) => {
                    if (event.key !== "Enter" && event.key !== " ") return;
                    event.preventDefault();
                    onSaveLayout?.();
                    setMenuOpen(false);
                  }}
                >
                  <span className="tcld-layout-menu-active-dot" data-active={layoutDirty ? "true" : "false"} aria-hidden />
                  <span className="min-w-0">
                    <span className="block truncate">Save layout</span>
                    <span className="block whitespace-normal text-label text-tcld-text-inactive leading-relaxed mt-0.5">
                      {layoutDirty ? "Save this arrangement for next time" : "No unsaved layout changes"}
                    </span>
                  </span>
                </button>
              </div>
            </>,
            document.body,
          )}
        </div>
      </div>

      <div className="flex-1" />

      <div className="tcld-actionbar-launch">
        <span className="tcld-actionbar-label" data-actionbar-launch-item="label">LAUNCH</span>
        {launchControls}
      </div>
    </div>
  );
});

interface LayoutMenuItemProps {
  id: string;
  kind: "built-in" | "saved";
  active: boolean;
  label: string;
  detail: string;
  onClick: () => void;
}

function LayoutMenuItem({ id, kind, active, label, detail, onClick }: LayoutMenuItemProps) {
  const handleKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onClick();
  };
  return (
    <button
      type="button"
      role="menuitem"
      data-layout-menu-kind={kind}
      data-layout-menu-id={id}
      data-layout-active={active ? "true" : "false"}
      className="tcld-layout-menu-item"
      onClick={onClick}
      onKeyDown={handleKeyDown}
    >
      <span className="tcld-layout-menu-active-dot" data-active={active ? "true" : "false"} aria-hidden />
      <span className="min-w-0">
        <span className="block truncate">{label}</span>
        <span className="block whitespace-normal text-label text-tcld-text-inactive leading-relaxed mt-0.5">{detail}</span>
      </span>
    </button>
  );
}

function shortLayoutName(label: string): string {
  return label.split(" — ")[0]?.split(" - ")[0]?.trim() || label;
}

const PRESET_PURPOSES: Record<string, string> = {
  default: "Read source alongside bytecode and instruction details",
  flow: "Follow branches and loops through the control-flow graph",
  trace: "Follow execution with source and console output",
  console: "Keep source and runtime output side by side",
  symbols: "Explore literals, variables, and their references",
  cockpit: "Compare source, bytecode, and control flow at once",
  runbook: "Step through code with runtime state and output",
  analysis: "Investigate instructions, flow, and symbols together",
  wide: "Give long instructions room across the window",
  decompile: "Focus on reconstructed source and its details",
  debug: "Debug with source, variables, call stack, and console",
  overview: "Survey code, symbols, exceptions, and file statistics",
};

function describePreset(preset: LayoutPreset): string {
  return PRESET_PURPOSES[preset.name]
    ?? describeWorkspace(preset.workspace ?? workspaceFromViewTree(preset.tree));
}

function describeWorkspace(workspace: WorkspaceLayoutSnapshot): string {
  const panelIds = getOpenViews(workspace.tree, workspace.stacks);
  const panelLabel = panelIds.length === 1 ? "panel" : "panels";
  return `${panelIds.length} ${panelLabel} · ${panelIds.map((id) => VIEW_TITLES[id]).join(", ")}`;
}
