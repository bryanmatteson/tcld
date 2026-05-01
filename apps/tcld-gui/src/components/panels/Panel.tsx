"use client";

import React from "react";
import { SortableContext, horizontalListSortingStrategy, useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { X, Maximize2, Minimize2 } from "lucide-react";
import clsx from "clsx";
import { usePanelMeta, type PanelActionItem } from "./PanelContext";
import { PanelHeaderMenu } from "./PanelHeaderMenu";
import { TabOverflowMenu, type OverflowTab } from "./TabOverflowMenu";
import { TabSlotDrop } from "./TabSlotDrop";
import { fitAdaptiveTabs, renderAdaptiveTabLabel, type AdaptiveTabMode } from "@/lib/adaptive-tabs";
import { TabBadge } from "@/components/primitives/TabBadge";
import { tabHandle } from "@/lib/ids";
import type { ViewId, WorkspaceLeafId } from "@/types";

interface PanelProps {
  id: string;
  title: string;
  badge?: string;
  icon?: React.ReactNode;
  keyHint?: string;
  focused?: boolean;
  accent?: boolean;
  expanded?: boolean;
  subtitle?: string;
  actions?: React.ReactNode;
  designSurface?: string;
  onClose?: () => void;
  onFocus?: () => void;
  onExpand?: () => void;
  children: React.ReactNode;
}

interface PanelTabButtonProps {
  tab: {
    id: ViewId;
    label: string;
    icon?: React.ReactNode;
    badge?: string | number | null;
    dot?: boolean;
    keyHint?: string;
  };
  active: boolean;
  mode: AdaptiveTabMode;
  onSelect?: (tabId: ViewId) => void;
}

const PanelTabButton = React.memo(function PanelTabButton({
  tab,
  active,
  mode,
  onSelect,
}: PanelTabButtonProps) {
  const showLabel = mode === "full" || mode === "ellipsis";
  const showBadge = (mode === "full" || mode === "ellipsis" || mode === "iconCount") && tab.badge != null;

  return (
    <button
      type="button"
      onPointerDownCapture={(e) => {
        e.stopPropagation();
        onSelect?.(tab.id);
      }}
      onClick={(e) => {
        e.stopPropagation();
        onSelect?.(tab.id);
      }}
      // Inline-tab interior: glyph + label + badge. The wrapper <div class="tcld-inline-tab">
      // owns the active border + box-shadow accent stripe; this button is the
      // semantic action and the screen-reader-friendly target.
      className="flex items-center gap-1.5 min-w-0 pointer-events-none bg-transparent border-0 p-0"
      aria-label={tab.label}
      title={tab.keyHint ? `${tab.label} (${tab.keyHint})` : tab.label}
    >
      {tab.dot && (
        <span className="h-1.5 w-1.5 flex-shrink-0 rounded-full bg-tcld-accent" aria-hidden />
      )}
      {tab.icon && (
        <span
          className="tcld-pt-glyph flex h-3 w-3 flex-shrink-0 items-center justify-center"
          aria-hidden
        >
          {tab.icon}
        </span>
      )}
      {showLabel && (
        <span className="truncate max-w-[14rem]">
          {renderAdaptiveTabLabel(tab.label, mode)}
        </span>
      )}
      {showBadge && tab.badge != null && (
        <TabBadge value={tab.badge} active={active} className="ml-0.5" />
      )}
    </button>
  );
});

const EMPTY_PANEL_TABS: Array<{
  id: ViewId;
  label: string;
  icon?: React.ReactNode;
  badge?: string | number | null;
  dot?: boolean;
  keyHint?: string;
}> = [];

interface AdaptivePanelTabProps {
  leafId?: WorkspaceLeafId;
  tab: {
    id: ViewId;
    label: string;
    icon?: React.ReactNode;
    badge?: string | number | null;
    dot?: boolean;
    keyHint?: string;
  };
  active: boolean;
  mode: AdaptiveTabMode;
  onSelect?: (tabId: ViewId) => void;
  onClose?: (tabId: ViewId) => void;
}

const AdaptivePanelTab = React.memo(function AdaptivePanelTab({
  leafId,
  tab,
  active,
  mode,
  onSelect,
  onClose,
}: AdaptivePanelTabProps) {
  // useSortable instead of useDraggable so other tabs in the same strip
  // animate to make room as this tab is dragged. The id matches the
  // SortableContext items array maintained by Panel below.
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: tabHandle(leafId ?? "panel", tab.id),
    data: leafId ? { kind: "tab", leafId, viewId: tab.id } : undefined,
    disabled: !leafId,
  });
  const onDragPointerDown = listeners?.onPointerDown;

  return (
    <div
      ref={setNodeRef}
      {...attributes}
      {...listeners}
      data-design-surface="singleton-tab"
      data-tab-mode={mode}
      data-tab-view-id={tab.id}
      data-tab-dragging={isDragging ? "true" : undefined}
      data-tab-key-hint={tab.keyHint ?? undefined}
      style={{
        // Sortable transform/transition produce the slide animation as the
        // user drags one tab past another. The transform here moves the
        // tab to its in-flight position; SortableContext + the FLIP
        // strategy slides every other tab into its new resting place.
        transform: CSS.Transform.toString(transform),
        transition,
      }}
      // IMPORTANT: do NOT stopPropagation in onPointerDownCapture — calling
      // stopPropagation in the capture phase also stops the bubble phase
      // on this element, which prevents dnd-kit's onPointerDown handler
      // (re-invoked below) from running. The result was tabs that looked
      // clickable but couldn't be dragged.
      //
      // Instead, the activation tap (selecting the tab on click) runs in
      // onClick, and pointer events are forwarded to dnd-kit cleanly so the
      // PointerSensor can detect drag activation past its 8px threshold.
      onPointerDown={(e) => {
        onDragPointerDown?.(e);
      }}
      onClick={(e) => {
        e.stopPropagation();
        onSelect?.(tab.id);
      }}
      // Canonical workbench inline tab. The .tcld-inline-tab CSS owns the
      // 28px height, the asymmetric top-rounded shape, the hover wash, and
      // the accent inset top-stripe on .is-active. We intentionally render
      // the workbench-mock markup verbatim so panels can be eyeballed
      // against ui_kits/workbench/index.html without a vocab mismatch.
      className={clsx(
        "tcld-inline-tab",
        active && "is-active",
        isDragging && "opacity-40",
      )}
    >
      <button
        type="button"
        data-panel-interactive="true"
        // Screen-reader proxy. pointer-events:none keeps drag/click flowing
        // to the wrapper div (the actual dnd-kit drag handle).
        className="pointer-events-none absolute inset-0 z-30 cursor-[inherit] rounded border-0 bg-transparent text-transparent"
        title={tab.label === "Symbols" ? "View Symbols" : tab.label}
        aria-label={tab.label}
        tabIndex={-1}
      >
        {tab.label}
      </button>
      <span className="relative z-10 pointer-events-none">
        <PanelTabButton
          tab={tab}
          active={active}
          mode={mode}
          onSelect={onSelect}
        />
      </span>
      {(mode === "full" || mode === "ellipsis") && (
        <button
          type="button"
          data-panel-interactive="true"
          data-panel-close={tab.id}
          onClick={(e) => {
            e.stopPropagation();
            onClose?.(tab.id);
          }}
          // Workbench tab-close button: 14×14 hit area, hidden until hover/
          // focus on the tab, always-on for the active tab. Class shape
          // matches .tcld-inline-tab .tcld-tab-close in globals.css.
          className="tcld-tab-close relative z-40"
          aria-label={`Close ${tab.label}`}
          title={`Close ${tab.label}`}
        >
          <X size={10} aria-hidden />
        </button>
      )}
    </div>
  );
});

export const Panel = React.memo(React.forwardRef<HTMLDivElement, PanelProps>(
  (
    {
      id,
      title,
      badge,
      icon: iconProp,
      keyHint: keyHintProp,
      focused = false,
      accent = false,
      expanded: expandedProp,
      subtitle,
      actions: actionsProp,
      designSurface,
      onClose,
      onFocus,
      onExpand: onExpandProp,
      children,
    },
    ref
  ) => {
    // Merge context (from MosaicWorkspace) with direct props
    const meta = usePanelMeta();
    const icon = iconProp ?? meta.icon;
    const keyHint = keyHintProp ?? meta.keyHint;
    const expanded = expandedProp ?? meta.expanded ?? false;
    const onExpand = onExpandProp ?? meta.onExpand;
    const actions = actionsProp ?? meta.actions;
    const tabs = meta.tabs ?? EMPTY_PANEL_TABS;
    const hasTabs = tabs.length > 1;
    const tabStripRef = React.useRef<HTMLDivElement | null>(null);
    const [tabStripWidth, setTabStripWidth] = React.useState(4096);
    // Compute which tabs render inline at full/ellipsis mode and which spill
    // into the +N overflow menu. Strategy:
    //   1. Always keep the active tab inline.
    //   2. Walk the remaining tabs in order; keep adding inline as long as
    //      fitAdaptiveTabs reports `full` for every kept tab.
    //   3. Anything that would degrade to iconCount/iconOnly goes to overflow.
    // Reserves OVERFLOW_BUTTON_WIDTH at the end so the +N button itself fits.
    const OVERFLOW_BUTTON_WIDTH = 56;
    const { inlineModes, overflowTabs } = React.useMemo(() => {
      const buildInputs = (subset: typeof tabs) => subset.map((tab) => ({
        label: tab.label,
        active: tab.id === meta.activeTabId,
        badge: tab.badge ?? null,
        dot: tab.dot,
      }));

      // Start with all tabs inline at full; if everyone fits, no overflow.
      const allInputs = buildInputs(tabs);
      const allModes = fitAdaptiveTabs(allInputs, Math.max(0, tabStripWidth - 4));
      if (allModes.every((mode) => mode === "full" || mode === "ellipsis")) {
        return {
          inlineModes: allModes.map((mode, i) => ({ tab: tabs[i], mode })),
          overflowTabs: [] as typeof tabs,
        };
      }

      // At least one would degrade — start removing the *last* non-active tab
      // until the remaining set fits at full mode. Reserves space for the
      // +N overflow button after the inline strip.
      const reserved = OVERFLOW_BUTTON_WIDTH;
      const inline: typeof tabs = [];
      const overflow: typeof tabs = [];
      const activeId = meta.activeTabId;
      // Active tab is always inline (first if not already at front).
      const activeTab = tabs.find((t) => t.id === activeId);
      const others = tabs.filter((t) => t.id !== activeId);
      if (activeTab) inline.push(activeTab);

      for (const tab of others) {
        const candidate = [...inline, tab];
        const modes = fitAdaptiveTabs(buildInputs(candidate), Math.max(0, tabStripWidth - 4 - reserved));
        const fitsFull = modes.every((mode) => mode === "full" || mode === "ellipsis");
        if (fitsFull) {
          inline.push(tab);
        } else {
          overflow.push(tab);
        }
      }

      // Restore original tab order in `inline` so the active tab doesn't jump.
      const inlineSet = new Set(inline.map((t) => t.id));
      const orderedInline = tabs.filter((t) => inlineSet.has(t.id));
      const inlineModes = fitAdaptiveTabs(buildInputs(orderedInline), Math.max(0, tabStripWidth - 4 - reserved));
      return {
        inlineModes: orderedInline.map((tab, i) => ({ tab, mode: inlineModes[i] ?? "full" as AdaptiveTabMode })),
        overflowTabs: overflow,
      };
    }, [meta.activeTabId, tabStripWidth, tabs]);

    React.useEffect(() => {
      const node = tabStripRef.current;
      if (!node) return;
      const update = () => setTabStripWidth(Math.max(0, Math.floor(node.getBoundingClientRect().width)));
      update();
      if (typeof ResizeObserver === "undefined") {
        window.addEventListener("resize", update);
        return () => window.removeEventListener("resize", update);
      }
      const observer = new ResizeObserver(update);
      observer.observe(node);
      return () => observer.disconnect();
    }, [hasTabs]);

    // Build the typed menu list shown in the unified ⋯ panel-header dropdown.
    // Order: upstream `actionItems` (e.g. swap view, add view) → separator →
    // maximize/restore → close. Anything that's not provided is omitted.
    const menuItems = React.useMemo<PanelActionItem[]>(() => {
      const items: PanelActionItem[] = [];
      if (meta.actionItems) items.push(...meta.actionItems);
      if (onExpand) {
        if (items.length > 0 && !items[items.length - 1].separatorAfter) {
          items[items.length - 1] = { ...items[items.length - 1], separatorAfter: true };
        }
        items.push({
          id: "panel-toggle-expand",
          label: expanded ? "Restore panel" : "Maximize panel",
          icon: expanded ? <Minimize2 size={12} /> : <Maximize2 size={12} />,
          detail: "z",
          onSelect: () => { onFocus?.(); onExpand(); },
        });
      }
      if (onClose) {
        items.push({
          id: "panel-close",
          label: "Close panel",
          icon: <X size={12} />,
          detail: "⌘W",
          danger: true,
          onSelect: onClose,
        });
      }
      return items;
    }, [meta.actionItems, expanded, onClose, onExpand, onFocus]);

    const guardedDragHandleProps = React.useMemo(() => {
      const props = meta.dragHandleProps as React.HTMLAttributes<HTMLDivElement> | undefined;
      if (!props) return undefined;
      const isInteractive = (target: EventTarget | null) =>
        target instanceof HTMLElement && Boolean(target.closest("[data-panel-interactive='true']"));
      return {
        ...props,
        onPointerDown: (event: React.PointerEvent<HTMLDivElement>) => {
          if (isInteractive(event.target)) return;
          props.onPointerDown?.(event);
        },
        onMouseDown: (event: React.MouseEvent<HTMLDivElement>) => {
          if (isInteractive(event.target)) return;
          props.onMouseDown?.(event);
        },
      };
    }, [meta.dragHandleProps]);

    /*
     * Canonical workbench panel titlebar. Two layouts:
     *
     *   - Single-view (D pattern):  <div class="tcld-panel-titlebar">
     *       <span class="tcld-pt-glyph">{icon}</span>
     *       <span class="tcld-pt-name">{title}</span>
     *       <span class="tcld-pt-crumb">{subtitle}</span>
     *       <span class="tcld-pt-actions">{keyhint}{menu}</span>
     *
     *   - Multi-view (A pattern):   <div class="tcld-panel-titlebar tcld-panel-titlebar--container">
     *       <span class="tcld-inline-tabs is-bare">{tab*}</span>
     *       <span class="tcld-pt-actions">{keyhint}{menu}</span>
     *
     * Focused-state styling (cue 2 + 3 of the three-cue stack — header
     * wash + accent name color) lives in globals.css under
     * `.tcld-panel.is-focused > .tcld-panel-titlebar`. We just toggle
     * `is-focused` on the panel root.
     */
    const header = (
      <div
        {...guardedDragHandleProps}
        data-design-surface="panel-header"
        className={clsx(
          "tcld-panel-titlebar panel-header",
          hasTabs && "tcld-panel-titlebar--container",
          guardedDragHandleProps && "cursor-grab active:cursor-grabbing",
        )}
      >
        {hasTabs ? (
          <span
            ref={tabStripRef}
            data-design-surface="adaptive-tab-bar"
            className="tcld-inline-tabs is-bare"
          >
            {/*
             * Sortable context for in-strip reorder. items[] must be the
             * sortable ids in current order so dnd-kit/sortable can compute
             * the FLIP-style slide animation as the user drags one tab past
             * another. Cross-leaf moves still go through the tab-slot drop
             * targets and the leaf-level handlers.
             */}
            <SortableContext
              items={inlineModes.map(({ tab }) => tabHandle(meta.leafId ?? "panel", tab.id))}
              strategy={horizontalListSortingStrategy}
            >
              {/* Slot at index 0 — lets users drop a tab at the very start. */}
              {meta.leafId && (
                <TabSlotDrop
                  leafId={meta.leafId}
                  index={0}
                  visible={!!meta.isAnyTabDragging}
                />
              )}
              {inlineModes.map(({ tab, mode }, i) => (
                <React.Fragment key={tab.id}>
                  <AdaptivePanelTab
                    leafId={meta.leafId}
                    tab={tab}
                    active={tab.id === meta.activeTabId}
                    mode={mode}
                    onSelect={meta.onSelectTab}
                    onClose={meta.onCloseTab}
                  />
                  {meta.leafId && (
                    <TabSlotDrop
                      leafId={meta.leafId}
                      index={i + 1}
                      visible={!!meta.isAnyTabDragging}
                    />
                  )}
                </React.Fragment>
              ))}
            </SortableContext>
            {overflowTabs.length > 0 && (
              <TabOverflowMenu
                tabs={overflowTabs.map<OverflowTab>((tab) => ({
                  id: tab.id,
                  label: tab.label,
                  icon: tab.icon,
                  badge: tab.badge,
                  active: tab.id === meta.activeTabId,
                }))}
                onSelect={(tabId) => meta.onSelectTab?.(tabId)}
                onClose={meta.onCloseTab ? (tabId) => meta.onCloseTab?.(tabId) : undefined}
              />
            )}
          </span>
        ) : (
          <>
            {icon && (
              <span className="tcld-pt-glyph" aria-hidden>
                {icon}
              </span>
            )}
            <span className="tcld-pt-name">{title}</span>
            {/* Subtitle becomes the breadcrumb slot — context like
                "::tcl::clock::scan" or "i0148 · invokeWide". Workbench
                mock pattern: dim, no letter-spacing, ellipsizes. */}
            {subtitle && <span className="tcld-pt-crumb">{subtitle}</span>}
            {badge && (
              <TabBadge
                value={badge}
                active={focused}
                variant={accent ? "group" : "default"}
                className="flex-shrink-0"
              />
            )}
          </>
        )}

        {/*
         * Right cluster: Keyhint chip + the single ⋯ menu. Sits in
         * .tcld-pt-actions so it's right-aligned and gap'd correctly.
         * Typed menu items come from meta.actionItems; legacy actions
         * remain a back-compat fallback for callers mid-migration.
         */}
        <span data-design-surface="panel-header-actions" className="tcld-pt-actions">
          {keyHint && (
            <span
              aria-label={`Keyboard shortcut: ${keyHint}`}
              title={`Keyboard shortcut: ${keyHint}`}
              className={clsx(
                "panel-keyhint",
                "font-mono text-label leading-none",
                "px-1 py-0.5 rounded-[var(--radius-badge)]",
                "bg-tcld-bg-3 border border-tcld-border-dim",
                "text-tcld-text-keyhint",
                "select-none",
              )}
            >
              {keyHint}
            </span>
          )}
          <PanelHeaderMenu
            panelId={id}
            items={menuItems}
            legacyActions={actions && !meta.actionItems ? actions : undefined}
          />
        </span>
      </div>
    );

    return (
      <div
        ref={ref}
        data-panel-id={id}
        data-panel-focused={focused ? "true" : undefined}
        data-design-surface={designSurface}
        onClick={onFocus}
        className={clsx(
          // Both class names: panel-root for legacy CSS hooks, tcld-panel
          // for the canonical workbench design-system selectors that key
          // off the focused state (header wash, name accent, left rail).
          "panel-root tcld-panel",
          // Use `relative` so the focused-edge stripe (an absolutely-
          // positioned ::before pseudo-element from .panel-root.is-focused)
          // can anchor to the panel's left edge.
          "relative flex flex-col h-full rounded-[var(--radius-panel)]",
          "border transition-colors duration-150",
          "bg-tcld-bg-2",
          "shadow-[var(--shadow-panel)]",
          "group/panel",
          focused && "is-focused",
          meta.isDropTarget
            // Dashed accent border = drop target preview during cross-leaf
            // drag. Beats focus styling because the user is mid-drag.
            ? "border-tcld-border-focus border-dashed"
            : focused
              // Focused = third cue (rail + header tint + border). The
              // border emphasis catches any glance even when the panel
              // header is scrolled off-screen.
              ? "border-tcld-border-focus"
              // Static dim border for unfocused panels. Subtle but present
              // so the panel always reads as a separate surface.
              : "border-tcld-border-dim"
        )}
      >
        {header}

        {/* Body */}
        <div className="flex-1 flex flex-col overflow-y-auto overflow-x-hidden tcld-panel-body">
          {children}
        </div>
      </div>
    );
  }
));

Panel.displayName = "Panel";
