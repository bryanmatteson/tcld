"use client";

import clsx from "clsx";
import { Chip } from "./Chip";

interface TabBadgeProps {
  /** Raw value; `null`/`undefined`/`<0` renders nothing. */
  value: number | string | null | undefined;
  active?: boolean;
  /** Higher-emphasis variant for in-panel "group" tabs. */
  variant?: "default" | "group";
  className?: string;
}

/**
 * Sub-tab count badge. Thin shim over Chip(size="xs") so the visual
 * vocabulary is unified across MetricChip / TabBadge / status chips.
 *
 * Sizing: 16px tall, no border. The 28px tab strip is dense and a bordered
 * pill with its own background reads as a competing chip; an unboxed
 * tabular-numeric label sits inside the tab quietly. Active tabs lift the
 * label color so the active count still pops.
 */
export function TabBadge({ value, active = false, variant = "default", className }: TabBadgeProps) {
  if (value == null || value === "") return null;

  // Map the legacy "default + active" combo to a brighter neutral fg via
  // className override, so the count visually pops on the active tab.
  const activeOverride =
    active && variant === "default" ? "!text-tcld-fg" : undefined;
  const inactiveOverride =
    !active && variant === "default" ? "opacity-80" : undefined;

  return (
    <Chip
      size="xs"
      tone={variant === "group" ? "group" : "neutral"}
      // Parity/test marker: rendered tab badges must be selectable as
      // production badges (distinct from shortcut key hints).
      data-tab-badge="true"
      className={clsx(activeOverride, inactiveOverride, className)}
    >
      {value}
    </Chip>
  );
}
