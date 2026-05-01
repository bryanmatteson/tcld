"use client";

import { createContext, useContext, type ReactNode } from "react";
import type { ViewId, WorkspaceLeafId } from "@/types";

interface PanelTabMeta {
  id: ViewId;
  label: string;
  icon?: ReactNode;
  badge?: string | number | null;
  dot?: boolean;
  keyHint?: string;
}

/**
 * Typed action item rendered in the panel-header dropdown menu.
 * Use this instead of the legacy `actions` ReactNode slot — it lets the
 * Panel render consistent, themed menu rows and makes overflow / keyboard
 * navigation behave predictably.
 */
export interface PanelActionItem {
  id: string;
  label: string;
  icon?: ReactNode;
  /** Optional secondary text (shortcut, hint). */
  detail?: string;
  /** Click handler — called with no args. Skip to render a non-interactive separator/label. */
  onSelect?: () => void;
  disabled?: boolean;
  /** Renders this item with the danger color ramp (e.g., "Close panel"). */
  danger?: boolean;
  /** Insert a thin separator after this row. */
  separatorAfter?: boolean;
  /**
   * Nested items. When present, the row shows a `▸` glyph and clicking it
   * drills into the submenu inside the same dropdown (with a Back row at
   * the top). `onSelect` is ignored when `submenu` is set.
   */
  submenu?: PanelActionItem[];
  /** Optional empty-state copy for the submenu when `submenu` is empty. */
  submenuEmptyLabel?: string;
}

interface PanelMeta {
  leafId?: WorkspaceLeafId;
  icon?: ReactNode;
  keyHint?: string;
  /** @deprecated use `actionItems` — kept for back-compat with one or two callers. */
  actions?: ReactNode;
  /** Typed action items shown in the unified panel-header dropdown. */
  actionItems?: PanelActionItem[];
  expanded?: boolean;
  onExpand?: () => void;
  tabs?: PanelTabMeta[];
  activeTabId?: ViewId;
  onSelectTab?: (tabId: ViewId) => void;
  onCloseTab?: (tabId: ViewId) => void;
  /** @dnd-kit drag handle props — spread on the panel header */
  dragHandleProps?: Record<string, unknown>;
  /** True when another panel is being dragged over this one */
  isDropTarget?: boolean;
  /** True when any tab is currently being dragged anywhere — drives the
   *  tab-slot drop indicators in the tab strip. */
  isAnyTabDragging?: boolean;
}

const PanelMetaContext = createContext<PanelMeta>({});

export const PanelMetaProvider = PanelMetaContext.Provider;
export const usePanelMeta = () => useContext(PanelMetaContext);
