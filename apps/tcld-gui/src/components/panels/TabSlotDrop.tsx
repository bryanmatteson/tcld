"use client";

import React from "react";
import { useDroppable } from "@dnd-kit/core";
import clsx from "clsx";
import { tabSlotHandle } from "@/lib/ids";
import type { WorkspaceLeafId } from "@/types";

interface TabSlotDropProps {
  leafId: WorkspaceLeafId;
  index: number;
  /** True when any tab is currently being dragged anywhere in the workspace. */
  visible: boolean;
}

/**
 * A thin drop target rendered between/around tabs in a panel's tab strip.
 * Drops here trigger `reorderTabInLeaf(viewId, sourceLeafId, leafId, index)`,
 * giving precise insertion order — both for in-strip reorder and for
 * inserting a tab from another panel at a specific position.
 *
 * The slot is invisible at rest (4px wide, transparent) so the strip looks
 * unchanged. While a drag is in progress, the slot widens to 8px and shows
 * a subtle accent line; on hover it lights up to a solid 2px accent
 * indicator so the user can see exactly where the tab will land.
 */
export function TabSlotDrop({ leafId, index, visible }: TabSlotDropProps) {
  const { isOver, setNodeRef } = useDroppable({
    id: tabSlotHandle(leafId, index),
    data: { kind: "tab-slot", leafId, index },
  });

  return (
    <div
      ref={setNodeRef}
      data-tab-slot-leaf={leafId}
      data-tab-slot-index={index}
      data-tab-slot-active={isOver ? "true" : undefined}
      className={clsx(
        "relative self-stretch flex-shrink-0 transition-all duration-100",
        visible ? "w-2" : "w-1",
      )}
      aria-hidden
    >
      {/* Inner indicator — shows only when a drag is in progress so the
          empty strip stays visually quiet. */}
      <span
        className={clsx(
          "absolute top-1 bottom-1 left-1/2 -translate-x-1/2 rounded-full",
          "transition-all duration-100",
          visible
            ? isOver
              ? "w-[3px] bg-tcld-accent shadow-[0_0_6px_var(--accent)]"
              : "w-px bg-tcld-text-keyhint/40"
            : "w-0 bg-transparent",
        )}
      />
    </div>
  );
}
